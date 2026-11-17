//! Our own process tree, split by WebView2 role (R0a-1's measurement surface).
//!
//! A WebView2 embedder's child tree is several processes that all run the *same*
//! image, `msedgewebview2.exe`: one browser process, one or more renderers, a GPU
//! process, and utility processes. `Toolhelp32` (see [`crate::sys::snapshot_processes`])
//! reports the image name and the parent pid but no command line, so every row
//! looks alike. Counting them all as "browser processes" — the mistake this module
//! exists to make impossible — inflates the browser-process count by roughly the
//! process count itself and makes the per-webview cost look like ~150 MB instead
//! of what it actually is.
//!
//! The role is therefore read from the command line's `--type=` argument, which is
//! the only field that separates them: the browser process has no `--type=` at all
//! ([`sys::CommandLineType::NoTypeArg`]), every child has one. When the command
//! line cannot be read the member lands in [`Role::Unclassified`] and is reported
//! as such; folding it into either count is exactly the silent miscount this
//! split is here to prevent.
//!
//! [`sys::CommandLineType::NoTypeArg`]: crate::sys::CommandLineType::NoTypeArg

use crate::sys;
#[cfg(windows)]
use crate::sys::CommandLineType;

/// Image name shared by every WebView2 process, whatever role it is playing.
const WEBVIEW_IMAGE: &str = "msedgewebview2.exe";

/// The role one `msedgewebview2.exe` in our tree is playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// No `--type=` argument: the WebView2 browser process. One per
    /// `ICoreWebView2Environment`, and the expensive one (of the order of 150 MB).
    Browser,
    /// `--type=renderer`: one per live site instance.
    Renderer,
    /// Any other `--type=<value>`, carrying the value so the report can name it
    /// (`gpu-process`, `utility`, `crashpad-handler`, …).
    Child(String),
    /// The command line could not be read, so the role is unknown. Counted and
    /// printed separately; never folded into [`Role::Browser`] or
    /// [`Role::Renderer`].
    Unclassified,
}

impl Role {
    /// Label for the report line. `Browser` and `Renderer` are spelled the same
    /// way the budget document spells them so the two can be compared by eye.
    pub fn label(&self) -> String {
        match self {
            Role::Browser => "browser".to_string(),
            Role::Renderer => "renderer".to_string(),
            Role::Child(kind) => kind.clone(),
            Role::Unclassified => "UNCLASSIFIED".to_string(),
        }
    }
}

/// One `msedgewebview2.exe` member of the tree.
#[derive(Debug, Clone)]
pub struct Member {
    pub pid: u32,
    pub parent_pid: u32,
    /// Working set in bytes; 0 when the process exited between the snapshot and
    /// the memory read (see [`sys::working_set_bytes`]).
    pub working_set: u64,
    pub role: Role,
}

/// A measurement of the tree at one instant. Every count is a count of processes,
/// not of webviews: the shell's live-webview count is carried beside it by the
/// caller because these two numbers are deliberately not assumed equal.
#[derive(Debug, Clone)]
pub struct Tree {
    /// Every `msedgewebview2.exe` descendant of the root, in snapshot order.
    pub members: Vec<Member>,
    /// Working set of this process alone.
    pub shell_working_set: u64,
    /// Working set of the whole tree, including processes that are not WebView2.
    pub total_working_set: u64,
    /// Working set of the WebView2 members only.
    pub webview_working_set: u64,
    /// Processes in the whole tree, WebView2 or not. A report that says "0.0 MB"
    /// is indistinguishable from a failed enumeration unless the process count is
    /// quoted next to it.
    pub total_processes: usize,
}

impl Tree {
    pub fn browser_processes(&self) -> usize {
        self.count(|role| matches!(role, Role::Browser))
    }

    pub fn renderers(&self) -> usize {
        self.count(|role| matches!(role, Role::Renderer))
    }

    /// WebView2 children that are neither browser nor renderer (GPU, utility…).
    pub fn other_children(&self) -> usize {
        self.count(|role| matches!(role, Role::Child(_)))
    }

    /// Members whose role could not be determined. A non-zero count here makes
    /// every other count in this struct unreliable, which is why the proof
    /// harness fails rather than reporting a clean number.
    pub fn unclassified(&self) -> usize {
        self.count(|role| matches!(role, Role::Unclassified))
    }

    fn count(&self, matches: impl Fn(&Role) -> bool) -> usize {
        self.members.iter().filter(|m| matches(&m.role)).count()
    }

    /// Working set of every member with this role label, in MB to one decimal —
    /// the number the "≈60 MB per renderer" budget is compared against.
    pub fn working_set_mb_of(&self, label: &str) -> f64 {
        let bytes: u64 = self
            .members
            .iter()
            .filter(|m| m.role.label() == label)
            .map(|m| m.working_set)
            .sum();
        round_mb(sys::mb(bytes))
    }
}

/// Snapshot our process tree and split the WebView2 members by role.
///
/// Cost: one `CreateToolhelp32Snapshot` walk (already measured by the self-test
/// path, a few ms) plus one command-line read per `msedgewebview2.exe` member —
/// typically 5..15 processes, so this stays well under a millisecond for the part
/// that is new. Nothing here runs outside `--proof`.
pub fn measure(root_pid: u32) -> Tree {
    let tree = sys::process_tree(root_pid);
    let mut members = Vec::new();
    let mut shell_working_set = 0u64;
    let mut webview_working_set = 0u64;
    for entry in &tree {
        if entry.name.eq_ignore_ascii_case(WEBVIEW_IMAGE) {
            #[cfg(windows)]
            let role = match sys::command_line_type(entry.pid) {
                CommandLineType::NoTypeArg => Role::Browser,
                CommandLineType::TypeArg(value) if value == "renderer" => Role::Renderer,
                CommandLineType::TypeArg(value) => Role::Child(value),
                CommandLineType::Unreadable => Role::Unclassified,
            };
            #[cfg(target_os = "macos")]
            let role = Role::Unclassified;
            webview_working_set += entry.working_set;
            members.push(Member {
                pid: entry.pid,
                parent_pid: entry.parent_pid,
                working_set: entry.working_set,
                role,
            });
        } else if entry.pid == root_pid {
            shell_working_set = entry.working_set;
        }
    }
    Tree {
        total_working_set: tree.iter().map(|entry| entry.working_set).sum(),
        total_processes: tree.len(),
        shell_working_set,
        webview_working_set,
        members,
    }
}

/// Machine-wide context for one [`measure`] reading, used only to say whether the
/// reading can be trusted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attribution {
    /// Processes running `msedgewebview2.exe` anywhere on the machine. Includes
    /// every other WebView2 app running, so it is never a substitute for the tree
    /// walk — it is the number that distinguishes "WebView2 is not running" from
    /// "WebView2 is running, but not ours".
    pub webviews_machine_wide: usize,
    /// Processes running our own image other than this one. Each of them can own
    /// the browser process for the user-data-folder it was started with, which is
    /// how a proof run ends up with a WebView2 tree it did not create.
    pub other_host_processes: usize,
}

/// Count the machine-wide WebView2 processes and the other host processes.
///
/// Attribution evidence, not a measurement of our tree: a foreign
/// `msedgewebview2.exe` is not one of ours, and folding it into a count would be
/// exactly the misattribution this exists to catch. Reuses
/// [`sys::snapshot_processes`] — one more snapshot per sample, proof-only.
pub fn attribution(own_pid: u32) -> Attribution {
    // The shell's own image name, not a literal: a renamed binary still finds its
    // siblings.
    let own_image = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let mut out = Attribution::default();
    for entry in sys::snapshot_processes() {
        if entry.name.eq_ignore_ascii_case(WEBVIEW_IMAGE) {
            out.webviews_machine_wide += 1;
        } else if entry.pid != own_pid && entry.name.eq_ignore_ascii_case(&own_image) {
            out.other_host_processes += 1;
        }
    }
    out
}

/// MB to one decimal, the precision every budget in the program is quoted at.
pub fn round_mb(mb: f64) -> f64 {
    (mb * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(role: Role, working_set: u64) -> Member {
        Member { pid: 1, parent_pid: 0, working_set, role }
    }

    fn tree(members: Vec<Member>) -> Tree {
        Tree {
            total_processes: members.len(),
            members,
            shell_working_set: 0,
            total_working_set: 0,
            webview_working_set: 0,
        }
    }

    /// The miscount this module exists to prevent: `--type=` absent is the only
    /// thing that marks the browser process, and a `gpu-process` child is not one.
    #[test]
    fn only_the_no_type_member_counts_as_a_browser_process() {
        let t = tree(vec![
            member(Role::Browser, 150 << 20),
            member(Role::Renderer, 60 << 20),
            member(Role::Renderer, 61 << 20),
            member(Role::Child("gpu-process".into()), 40 << 20),
            member(Role::Child("utility".into()), 20 << 20),
        ]);
        assert_eq!(t.browser_processes(), 1);
        assert_eq!(t.renderers(), 2);
        assert_eq!(t.other_children(), 2);
        assert_eq!(t.unclassified(), 0);
    }

    /// An unreadable command line is neither a browser process nor a renderer. If
    /// it silently became a browser process, a runtime that protected its
    /// renderers would look like it had sprouted extra browser processes; if it
    /// became a renderer, an extra browser process would be hidden.
    #[test]
    fn an_unreadable_command_line_is_counted_as_neither_role() {
        let t = tree(vec![
            member(Role::Browser, 0),
            member(Role::Unclassified, 0),
            member(Role::Unclassified, 0),
        ]);
        assert_eq!(t.browser_processes(), 1);
        assert_eq!(t.renderers(), 0);
        assert_eq!(t.other_children(), 0);
        assert_eq!(t.unclassified(), 2);
    }

    /// Per-role MB is what the renderer/overlay budget is quoted against, so the
    /// bucketing has to be by role and not by "everything WebView2".
    #[test]
    fn working_set_is_bucketed_per_role() {
        let t = tree(vec![
            member(Role::Renderer, 60 << 20),
            member(Role::Renderer, 62 << 20),
            member(Role::Browser, 150 << 20),
            member(Role::Unclassified, 1 << 20),
        ]);
        assert_eq!(t.working_set_mb_of("renderer"), 122.0);
        assert_eq!(t.working_set_mb_of("browser"), 150.0);
        assert_eq!(t.working_set_mb_of("UNCLASSIFIED"), 1.0);
        assert_eq!(t.working_set_mb_of("gpu-process"), 0.0);
    }

    #[test]
    fn measurement_of_the_running_test_process_finds_no_webview_tree() {
        // This test runs without a webview, so the WebView2 member list is empty
        // and the shell's own working set is non-zero. A zero here would mean the
        // enumeration silently returned nothing, which is the failure mode that
        // would make every `--proof` number read as zero-cost.
        let t = measure(sys::current_pid());
        assert!(t.members.is_empty(), "unit tests must not spawn WebView2 processes");
        assert!(t.shell_working_set > 0, "our own working set must be readable");
        assert!(t.total_working_set >= t.shell_working_set);
    }

    /// A harness that counted *itself* as a competing host process would report an
    /// attribution problem on every clean run, so the self-exclusion is pinned.
    /// `pid 0` is nobody, so that reading still includes this process — the gap
    /// between the two counts is exactly the process we left out.
    #[test]
    fn our_own_pid_is_never_counted_as_another_host_process() {
        let own = attribution(sys::current_pid());
        let including_ours = attribution(0);
        assert!(
            including_ours.other_host_processes >= own.other_host_processes + 1,
            "self-exclusion broken: own={}, without exclusion={}",
            own.other_host_processes,
            including_ours.other_host_processes
        );
    }
}
