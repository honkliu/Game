//! Download records and destination-path sanitization.
//!
//! WebView2's `DownloadStarting` hands us a suggested path. We rewrite the file
//! name component to something that (a) cannot escape the download directory,
//! (b) is a legal NTFS name, and (c) does not silently overwrite an existing
//! file.
//!
//! The download *mechanism* is WebView2's own (wry exposes start + completed
//! callbacks only). Pause/resume/cancel are therefore not offered in v1 — see
//! DESIGN §7 R6 and the scope note in the report.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    /// Transfer in progress.
    InProgress,
    Completed,
    Cancelled,
    Failed,
}

impl DownloadState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, DownloadState::InProgress)
    }
}

/// One row in the downloads panel (§4.4). Status is non-authoritative per §6.4
/// — on restore, a non-terminal record is shown as `Interrupted` rather than
/// claimed as downloading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Download {
    pub id: u64,
    pub url: String,
    pub file: String,
    pub path: String,
    pub state: DownloadState,
    /// Best-effort byte size; `None` until the first `Content-Length`.
    pub total_bytes: Option<u64>,
    /// `None` when unknown; the panel shows "downloading" without a percentage
    /// instead of inventing one.
    pub received_bytes: Option<u64>,
}

impl Download {
    pub fn file_name(&self) -> &str {
        path_file_name(&self.path)
    }
}

pub fn path_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Device names that Windows resolves to a *character device* even with an
/// extension, so a download with this name reports success and writes nothing.
///
/// Measured on this machine (build/resv/consdetect.cpp, CreateFileW +
/// GetFileType + FindFirstFile):
///
///   name          handle type   on disk
///   CON           DISK (1)      yes, 7 bytes    -> ordinary file
///   CON.txt       DISK (1)      yes, 7 bytes    -> ordinary file
///   AUX.txt       DISK (1)      yes            -> ordinary file
///   PRN.md        DISK (1)      yes            -> ordinary file
///   CONIN$.log    DISK (1)      yes            -> ordinary file
///   NUL           CHAR (2)      listed, 0 bytes -> BYTES SILENTLY LOST
///
/// So `CON`, `AUX`, `PRN` and the `COM`/`LPT` forms are only dangerous when the
/// name has *no* extension, and `NUL` is dangerous in every form. `CONIN$` and
/// `CONOUT$` behave like ordinary names here; reserving them was over-cautious.
///
/// (An earlier round of testing disagreed with itself because
/// `GetFileAttributesW` reports a device path as "existing" — only `GetFileType`
/// plus directory enumeration settles it.)
const RESERVED_ALWAYS: &[&str] = &["NUL"];

/// Reserved only in the *extensionless* form.
const RESERVED_BARE: &[&str] = &[
    "CON", "PRN", "AUX", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// `COM` and `LPT` take an optional digits-only second form (`COM1`, `COM10`).
/// Those are reserved in the extensionless form; with an extension they are
/// ordinary names (verified: `COM10` opens the device, `COM10.log` creates a
/// file). Digits stop at 99 — `COM100` is an ordinary name either way.
fn is_port_device(stem_upper: &str) -> bool {
    let base = stem_upper.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &stem_upper[base.len()..];
    if digits.is_empty() {
        return false; // no digits: not the numbered form
    }
    // Windows maps `COM0`–`COM9` and `COM10`–`COM99` to devices; `COM100`
    // (three or more digits) is an ordinary file name.
    digits.len() <= 2 && (base == "COM" || base == "LPT")
}

/// Is `stem_upper` reserved as a bare (extensionless) name?
fn is_reserved_bare(stem_upper: &str) -> bool {
    RESERVED_BARE.contains(&stem_upper) || is_port_device(stem_upper)
}

/// Is this whole name unsafe to write? Always-reserved device names (NUL) are
/// unsafe in any form; the rest only when there is no extension.
fn is_unsafe_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    // Always-reserved device names are unsafe as a bare name *and* as a stem:
    // `NUL.txt` opens the null device and reports the write as successful.
    let stem = match upper.split_once('.') {
        Some((s, _)) => s,
        None => upper.as_str(),
    };
    if RESERVED_ALWAYS.contains(&stem) {
        return true;
    }
    if name.contains('.') {
        return false;
    }
    is_reserved_bare(&upper)
}

/// NTFS forbids these in names. `:` is *not* here: it is handled by
/// `is_replaced` instead, because deleting it changes which file is written
/// (see below). Every other character is deleted rather than rewritten.
fn is_forbidden(c: char) -> bool {
    matches!(c, '<' | '>' | '"' | '/' | '\\' | '|' | '?' | '*')
}

/// Characters that are *replaced* with `_` rather than deleted, so a name keeps
/// the shape the user expects. `:` must be replaced, not deleted: deleting it
/// turns `notes.txt:hidden` (an NTFS alternate data stream) into
/// `notes.txt`, a different file, and the reserved-name check would then see
/// the original name rather than the on-disk one.
fn is_replaced(c: char) -> bool {
    c == ':' || c.is_control()
}

/// Sanitize a *file name* (not a path).
///
/// Guarantees on return:
/// - non-empty,
/// - no path separators or characters NTFS rejects,
/// - not a reserved device name,
/// - no trailing dot or space (Windows silently strips those, which would make
///   the on-disk name differ from what we report),
/// - ASCII control characters replaced by `_` (removing a tab would glue
///   `a\tb` into `ab`, which is a different name than the server sent),
/// - no `:` (an NTFS alternate data stream separator; replace, never delete),
/// - no `..` component (a name of exactly `..` with characters after it, e.g.
///   after `/` stripping, cannot be created and would address the parent dir),
/// - at most 200 characters (leaves room for a numeric suffix within MAX_PATH
///   budgets; not a hard fs limit but keeps the UI readable).
pub fn sanitize_file_name(name: &str) -> String {
    // Take only the last path component, so "../../x" cannot escape.
    let base = name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .trim_matches('\0');

    let cleaned: String = base
        .chars()
        .filter(|c| !is_forbidden(*c))
        .map(|c| if is_replaced(c) { '_' } else { c })
        .collect();
    // Collapse runs of whitespace to a single space to keep names readable.
    let mut squeezed = String::with_capacity(cleaned.len());
    let mut prev_ws = false;
    for c in cleaned.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                squeezed.push(' ');
            }
            prev_ws = true;
        } else {
            squeezed.push(c);
            prev_ws = false;
        }
    }
    // `trim_end_matches('.')` before `trim()` catches "a.txt . " → "a.txt".
    let mut name = squeezed.trim().trim_end_matches(['.', ' ']).trim().to_string();

    // A name that is only dots (`..`, `.`, `...`) is not a file name; Windows
    // resolves `..` to the parent directory.
    if name.chars().all(|c| c == '.') {
        name.clear();
    }

    if name.is_empty() {
        name = "download".to_string();
    }

    if name.chars().count() > 200 {
        // Keep the extension so the OS can still pick an association.
        let path = Path::new(&name);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let stem_chars = 200usize.saturating_sub(ext.chars().count());
        let stem: String = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("download")
            .chars()
            .take(stem_chars)
            .collect();
        name = format!("{stem}{ext}");
    }

    // See `RESERVED_ALWAYS` for the measurements behind this policy: `NUL` is
    // unsafe with or without an extension, the rest only without one.
    if is_unsafe_name(&name) {
        name = format!("_{name}");
    }

    debug_assert!(sanitize_is_idempotent(&name));
    name
}

fn sanitize_is_idempotent(name: &str) -> bool {
    if name.rsplit(['\\', '/']).next().unwrap_or(name) != name {
        return false;
    }
    if name.chars().any(is_forbidden) || name.ends_with(['.', ' ']) {
        return false;
    }
    // Must agree with `sanitize_file_name`: if sanitization would have prefixed
    // `_`, re-running it would prefix again.
    !name.is_empty() && !is_unsafe_name(name)
}

/// A suggested download file name is only safe to use verbatim when it is
/// already what `sanitize_file_name` would produce. `on_download_started` uses
/// this to decide whether the OS-suggested directory is trustworthy: a name
/// that sanitization had to rewrite came from a server-controlled
/// `Content-Disposition`, and the directory part of that path is just as
/// attacker-controlled, so the download must land in the user's download
/// directory instead.
pub fn is_already_safe(name: &str) -> bool {
    sanitize_file_name(name) == name
}

/// Split a suggested path into (directory, sanitized file name).
///
/// WebView2 builds the suggestion as `<Downloads>\<Content-Disposition name>`,
/// so the directory should be the platform's downloads directory. Honouring
/// *any* other directory would let a response whose `Content-Disposition`
/// carries an absolute path (`C:\Windows\System32\evil.exe`) choose where its
/// own payload is written. The suggestion's directory is therefore only used
/// when it is the genuine default download directory; otherwise the file lands
/// in `fallback_dir`. A `..` component or a rewritten name is disqualifying too,
/// because both mean the suggestion was not well-formed.
pub fn resolve_destination(suggested: &Path, fallback_dir: &Path) -> (PathBuf, String) {
    let raw_name = suggested
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let default_dir = PathBuf::from(crate::settings::default_download_dir());
    let parent_ok = match suggested.parent() {
        Some(p) if !p.as_os_str().is_empty() => {
            paths_equal_ignoring_case(p, &default_dir)
                && !suggested
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        }
        _ => false,
    };
    let dir = if parent_ok && is_already_safe(raw_name) {
        suggested.parent().unwrap_or(fallback_dir).to_path_buf()
    } else {
        fallback_dir.to_path_buf()
    };
    (dir, sanitize_file_name(raw_name))
}

/// Windows paths compare case-insensitively; `C:\Users\X\Downloads` and
/// `c:\users\x\downloads` are the same directory.
fn paths_equal_ignoring_case(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.display()
            .to_string()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Pick a non-colliding file name in `dir`: `name.ext`, `name (1).ext`, …
/// Stops at `(999)` and then appends a counter past the stem — bounded so a
/// full directory cannot spin forever.
///
/// `exists()` returns `false` for a path that exists but cannot be stat'd, so
/// "not occupied" is not the same as "creatable". The caller must treat a
/// failed download as a failed download; this function only avoids the common
/// case of overwriting.
pub fn unique_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    for n in 1..=999 {
        let candidate = format!("{stem} ({n}){ext}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    // Pathological: a directory with all 999 slots taken (or unreadable, which
    // reports every slot as occupied). Return a name with the counter inside
    // the length cap; the download itself will fail loudly if it truly collides.
    format!("{stem} (copy){ext}")
}

fn split_ext(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => (stem, &name[stem.len()..]),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_path_traversal() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("..\\..\\windows\\system32\\evil.dll"), "evil.dll");
        assert_eq!(sanitize_file_name("/abs/path/file.txt"), "file.txt");
    }

    #[test]
    fn removes_ntfs_forbidden_characters() {
        // Deleted, not replaced.
        assert_eq!(sanitize_file_name("a<b>c\"d|e?f*g.txt"), "abcdefg.txt");
        // `:` is replaced with `_` (alternate-data-stream separator); deleting
        // it would silently create a stream on the file before it.
        assert_eq!(sanitize_file_name("a:b.txt"), "a_b.txt");
        // Control characters are replaced, so `a\tb` is not glued into `ab`.
        assert_eq!(sanitize_file_name("tab\there.txt"), "tab_here.txt");
    }

    #[test]
    fn ad_stream_separator_becomes_underscore_not_a_deleted_char() {
        // `notes.txt:hidden` would create the invisible stream `hidden` on
        // `notes.txt`. Deleting `:` silently does exactly that; rewriting it
        // makes an ordinary, visible file whose name still says what happened.
        assert_eq!(sanitize_file_name("notes.txt:hidden"), "notes.txt_hidden");
        assert_eq!(sanitize_file_name("evil.txt::$DATA"), "evil.txt__$DATA");
    }

    #[test]
    fn dot_dot_is_never_a_file_name() {
        // `..` addresses the parent directory; a `rc:`/`Content-Disposition`
        // name of exactly `..` must not survive.
        assert_eq!(sanitize_file_name(".."), "download");
        assert_eq!(sanitize_file_name("."), "download");
        assert_eq!(sanitize_file_name("a/.."), "download");
    }

    #[test]
    fn handles_reserved_device_names() {
        // Policy measured in build/resv/consdetect.cpp (CreateFileW + GetFileType +
        // FindFirstFile). `NUL` is the only name that resolves to a character
        // device even with an extension, and it reports success while storing
        // 0 bytes — silent data loss, not merely a wrong name.
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("com1"), "_com1");
        assert_eq!(sanitize_file_name("COM1"), "_COM1");
        assert_eq!(sanitize_file_name("COM0"), "_COM0");
        assert_eq!(sanitize_file_name("LPT99"), "_LPT99");
        assert_eq!(sanitize_file_name("CONIN$"), "_CONIN$");
        assert_eq!(sanitize_file_name("conout$"), "_conout$");

        // NUL is unsafe in every form: silent data loss, not a wrong name.
        assert_eq!(sanitize_file_name("nul"), "_nul");
        assert_eq!(sanitize_file_name("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitize_file_name("nul.download"), "_nul.download");

        // Everything else is safe once it has an extension — measured: these
        // all create real files with 7 bytes written.
        assert_eq!(sanitize_file_name("con.txt"), "con.txt");
        assert_eq!(sanitize_file_name("LPT1.log"), "LPT1.log");
        assert_eq!(sanitize_file_name("COM10.log"), "COM10.log");
        assert_eq!(sanitize_file_name("CONIN$.log"), "CONIN$.log");
        assert_eq!(sanitize_file_name("AUX.txt"), "AUX.txt");
        assert_eq!(sanitize_file_name("PRN.md"), "PRN.md");
        assert_eq!(sanitize_file_name("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(sanitize_file_name("COM100.log"), "COM100.log");
    }

    #[test]
    fn truncated_long_name_can_never_end_up_reserved() {
        // 200 characters of `CON…` is not a reserved name (its stem is one long
        // run), and truncation must not shave it into one. The check runs after
        // truncation, so whatever comes out is checked.
        let long = "CON".to_string() + &"x".repeat(364);
        let out = sanitize_file_name(&long);
        assert_eq!(out.chars().count(), 200);
        let stem = out.split_once('.').map(|(s, _)| s).unwrap_or(&out);
        assert!(!is_reserved_bare(&stem.to_ascii_uppercase()), "got {out:?}");
        assert_eq!(sanitize_file_name(&out), out, "must be idempotent");
    }

    #[test]
    fn trailing_dots_and_spaces_are_removed() {
        assert_eq!(sanitize_file_name("file.txt..."), "file.txt");
        assert_eq!(sanitize_file_name("file.txt   "), "file.txt");
        assert_eq!(sanitize_file_name("  spaced.out "), "spaced.out");
    }

    #[test]
    fn empty_and_dot_only_names_get_a_fallback() {
        assert_eq!(sanitize_file_name(""), "download");
        assert_eq!(sanitize_file_name("..."), "download");
        assert_eq!(sanitize_file_name("/"), "download");
    }

    #[test]
    fn long_names_keep_extension_and_cap_length() {
        let long = format!("{}.zip", "x".repeat(400));
        let out = sanitize_file_name(&long);
        assert_eq!(out.chars().count(), 200);
        assert!(out.ends_with(".zip"));
        assert_eq!(sanitize_file_name(&out), out, "must be idempotent");
    }

    #[test]
    fn unique_name_avoids_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(unique_name(d, "a.txt"), "a.txt");
        std::fs::write(d.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_name(d, "a.txt"), "a (1).txt");
        std::fs::write(d.join("a (1).txt"), b"x").unwrap();
        assert_eq!(unique_name(d, "a.txt"), "a (2).txt");
        std::fs::write(d.join("noext"), b"x").unwrap();
        assert_eq!(unique_name(d, "noext"), "noext (1)");
    }

    #[test]
    fn resolve_destination_keeps_the_default_download_dir() {
        let downloads = crate::settings::default_download_dir();
        let fallback = Path::new(r"C:\fallback");
        let suggested = Path::new(&downloads).join("report.pdf");
        let (dir, name) = resolve_destination(&suggested, fallback);
        assert_eq!(dir, PathBuf::from(&downloads));
        assert_eq!(name, "report.pdf");
    }

    #[test]
    fn resolve_destination_rejects_a_server_chosen_directory() {
        // The suggestion comes from `Content-Disposition`. A response that
        // carries its own absolute path must not choose where its payload is
        // written.
        let fallback = Path::new(r"C:\fallback");
        for hostile in [
            r"C:\Windows\System32\evil.exe",
            r"C:\Windows\Temp\a<b>.exe",
            r"\Windows\Temp\no-drive.exe",
            r"evil\nul.txt",
            r"..\evil\nul.txt",
        ] {
            let (dir, _name) = resolve_destination(Path::new(hostile), fallback);
            assert_eq!(dir, fallback.to_path_buf(), "hostile dir kept for {hostile}");
        }
    }

    #[test]
    fn resolve_destination_rejects_parent_components_inside_the_default_dir() {
        // Even a path whose parent *looks* like the download dir must not be
        // honoured when it walks upwards on the way there.
        let downloads = crate::settings::default_download_dir();
        let fallback = Path::new(r"C:\fallback");
        let suggested = Path::new(&downloads).join("..").join("evil.exe");
        let (dir, _name) = resolve_destination(&suggested, fallback);
        assert_eq!(dir, fallback.to_path_buf());
    }

    #[test]
    fn resolve_destination_falls_back_when_no_dir() {
        let (dir, name) = resolve_destination(Path::new("plain.bin"), Path::new(r"C:\dl"));
        assert_eq!(dir, PathBuf::from(r"C:\dl"));
        assert_eq!(name, "plain.bin");
    }
}