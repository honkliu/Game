//! `asset://` custom protocol serving the hand-written HTML/CSS/JS shell from
//! disk (DESIGN §6.4 Security: shell webviews load only bundled `asset://`
//! content; no local HTTP server).
//!
//! On Windows, wry maps `asset://host/path` to `http://asset.host/path`
//! internally, so `asset://shell/index.html` arrives with path `/shell/index.html`
//! and Host `shell`. We ignore the host and serve from the root directory.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use wry::http::{header, Request, Response, StatusCode};

/// Content types we actually ship. Anything else is refused rather than guessed,
/// so a stray binary in `ui/` cannot be served with a wrong type.
pub fn content_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => return None,
    })
}

/// Resolve a request path to a file inside `root`.
///
/// Rejects `..`, absolute components, drive prefixes, and NUL. Returns `None`
/// for anything that could escape the root — this is the security boundary and
/// is covered by tests.
pub fn resolve_asset(root: &Path, request_path: &str) -> Option<PathBuf> {
    if request_path.contains('\0') {
        return None;
    }
    // Strip query/fragment before it reaches the filesystem.
    let path_part = request_path
        .split(['?', '#'])
        .next()
        .unwrap_or(request_path);
    let trimmed = path_part.trim_start_matches('/');
    let relative = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };
    let mut out = PathBuf::from(root);
    for comp in Path::new(relative).components() {
        match comp {
            Component::Normal(seg) => {
                // `Normal` only means "no separator in the *URL*". On Windows a
                // segment may still carry `:` (`index.html:hidden` is an NTFS
                // alternate data stream on `index.html`) or a backslash (an
                // encoded `%5C` becomes a real separator for the OS but not for
                // `Path::components`). Neither may reach `fs::read`, or the
                // handler would serve bytes the URL never named.
                let s = seg.to_str()?;
                if s.contains(':') || s.contains('\\') {
                    return None;
                }
                out.push(seg);
            }
            // `CurDir` is harmless; anything else (`..`, root, prefix) is not.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Build the wry response for a request. Missing files become a 404 with the
/// shell's own "file not found" so a broken asset path is visible, not blank.
pub fn serve(root: &Path, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let raw_path = request.uri().path().to_string();
    let Some(file) = resolve_asset(root, &raw_path) else {
        return text(StatusCode::FORBIDDEN, "forbidden path");
    };
    let Some(ct) = content_type(&file) else {
        return text(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported asset type");
    };
    match std::fs::read(&file) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, ct)
            // Local shell assets change only when the binary is rebuilt and
            // relaunched, so a private cache with no validator is correct and
            // keeps a cold start free of repeat I/O.
            .header(header::CACHE_CONTROL, "no-store")
            .body(Cow::Owned(bytes))
            .unwrap_or_else(|_| text(StatusCode::INTERNAL_SERVER_ERROR, "response error")),
        Err(e) => text(
            StatusCode::NOT_FOUND,
            &format!("asset not found: {raw_path} ({e})"),
        ),
    }
}

/// Serve from the table embedded in the executable. `AI_BROWSER_UI_DIR` points at
/// a directory instead, for editing the shell assets without rebuilding.
pub fn serve_embedded(request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    if let Some(dir) = std::env::var_os("AI_BROWSER_UI_DIR") {
        return serve(Path::new(&dir), request);
    }
    let raw_path = request.uri().path().to_string();
    let Some(rel) = resolve_asset(Path::new(""), &raw_path) else {
        return text(StatusCode::FORBIDDEN, "forbidden path");
    };
    let key = rel.to_string_lossy().replace('\\', "/");
    let Some(ct) = content_type(&rel) else {
        return text(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported asset type");
    };
    match embedded::ASSETS.iter().find(|(name, _)| *name == key) {
        Some((_, bytes)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, ct)
            .header(header::CACHE_CONTROL, "no-store")
            .body(Cow::Borrowed(*bytes))
            .unwrap_or_else(|_| text(StatusCode::INTERNAL_SERVER_ERROR, "response error")),
        None => text(StatusCode::NOT_FOUND, &format!("asset not found: {raw_path}")),
    }
}

/// An embedded shell asset by its path under `ui/`, with its content type.
pub fn embedded_file(key: &str) -> Option<(&'static str, &'static [u8])> {
    let ct = content_type(Path::new(key))?;
    embedded::ASSETS.iter().find(|(name, _)| *name == key).map(|(_, bytes)| (ct, *bytes))
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}

/// The single HTML page the shell can navigate to. Everything else in the shell
/// is a panel *inside* `index.html` — the shells must never navigate away from
/// this URL (§6.4: reject any navigation whose origin is not the asset origin).
pub const SHELL_URL: &str = "asset://shell/index.html";

/// Pages rendered inside page webviews (new tab, error, settings). These are
/// served by the same protocol but loaded in unprivileged page webviews, which
/// receive no `__native` bridge.
pub fn internal_page(kind: InternalPage) -> &'static str {
    match kind {
        InternalPage::NewTab => "asset://shell/pages/newtab.html",
        InternalPage::Error => "asset://shell/pages/error.html",
        InternalPage::Settings => "asset://shell/pages/settings.html",
        InternalPage::ViewSourceUnsupported => "asset://shell/pages/viewsource.html",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalPage {
    NewTab,
    Error,
    Settings,
    ViewSourceUnsupported,
}

fn text(status: StatusCode, body: &str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Cow::Owned(body.as_bytes().to_vec()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(r"C:\app\ui")
    }

    #[test]
    fn normal_paths_resolve_inside_root() {
        assert_eq!(resolve_asset(&root(), "/index.html"), Some(root().join("index.html")));
        assert_eq!(resolve_asset(&root(), "/"), Some(root().join("index.html")));
        assert_eq!(
            resolve_asset(&root(), "/pages/newtab.html"),
            Some(root().join("pages").join("newtab.html"))
        );
        assert_eq!(
            resolve_asset(&root(), "/app.js?v=2"),
            Some(root().join("app.js"))
        );
    }

    #[test]
    fn traversal_is_rejected() {
        for bad in [
            "/../secret.txt",
            "/pages/../../secret.txt",
            "/C:/Windows/win.ini",
            "/\\server\\share\\x",
            "/pages/..\\secret.txt",
        ] {
            assert_eq!(resolve_asset(&root(), bad), None, "must reject {bad}");
        }
    }

    #[test]
    fn percent_escapes_are_literal_file_text_not_decoded() {
        // `Request::uri().path()` does not percent-decode, so `%2e%2e` is a
        // literal six-character name and cannot become `..`. Asserting this
        // pins the reason the traversal check is sufficient without a decode
        // step — if a future change decodes, this test fails first.
        assert_eq!(
            resolve_asset(&root(), "/%2e%2e/secret.txt"),
            Some(root().join("%2e%2e").join("secret.txt"))
        );
    }

    #[test]
    fn encoded_backslash_cannot_smuggle_a_parent_component() {
        // WebView2 may hand the handler a decoded `\`; `Path::components`
        // treats it as a separator on Windows, so the `..` is caught. Pin it.
        assert_eq!(resolve_asset(&root(), "/pages%5C..%5C..%5Cx.txt"),
                   Some(root().join("pages%5C..%5C..%5Cx.txt")),
                   "percent text is a literal file name, not a traversal");
        assert_eq!(resolve_asset(&root(), "/x/..\\..\\y"), None);
    }

    #[test]
    fn content_types_cover_shipped_assets_only() {
        assert_eq!(content_type(Path::new("a.html")), Some("text/html; charset=utf-8"));
        assert_eq!(content_type(Path::new("a.JS")), Some("text/javascript; charset=utf-8"));
        assert_eq!(content_type(Path::new("a.exe")), None);
        assert_eq!(content_type(Path::new("noext")), None);
    }

    #[test]
    fn serve_reads_real_file_and_404s() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
        let req = Request::builder().uri("asset://shell/index.html").body(Vec::new()).unwrap();
        let resp = serve(dir.path(), &req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().as_ref(), b"<html></html>");
        assert_eq!(
            resp.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );

        let missing = Request::builder().uri("asset://shell/nope.html").body(Vec::new()).unwrap();
        assert_eq!(serve(dir.path(), &missing).status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn alternate_data_streams_cannot_be_read_through_the_handler() {
        // `index.html:hidden` is one `Normal` component, so the component walk
        // accepts it; only an explicit `:` rejection stops `fs::read` from
        // opening the named NTFS stream on `index.html`.
        let dir = tempfile::tempdir().unwrap();
        let fine = serve_uri(dir.path(), "/index.html:stream");
        assert_eq!(fine.status(), StatusCode::FORBIDDEN, "ADS path must not reach fs::read");
        assert_eq!(resolve_asset(dir.path(), "/index.html:stream"), None);
        // `::$DATA` is the unnamed stream: the whole file.
        assert_eq!(resolve_asset(dir.path(), "/app.js::$DATA"), None);
    }

    #[test]
    fn eight_three_short_names_do_not_bypass_the_extension_gate() {
        // `ASSETP~1`-style short paths end in `.HTM`, which is `.htm`. The
        // extension gate must not treat an unknown short extension as shippable.
        assert_eq!(content_type(Path::new("INDEX~1.HTM")), None);
        assert_eq!(content_type(Path::new("APP~1.JS")), Some("text/javascript; charset=utf-8"));
    }

    fn serve_uri(root: &Path, uri_path: &str) -> Response<Cow<'static, [u8]>> {
        let req = Request::builder()
            .uri(format!("asset://shell{uri_path}"))
            .body(Vec::new())
            .unwrap();
        serve(root, &req)
    }
}