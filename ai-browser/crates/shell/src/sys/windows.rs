//! Win32 glue that wry/tao do not cover: monotonic timing anchored at process
//! start (W1/W2), process-tree enumeration (W10), and small window helpers.
//!
//! Every `unsafe` block here has a `// SAFETY:` note. This module is the only
//! place in the crate allowed to call raw Win32 beyond wry/tao.

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::Shell::{ILCreateFromPathW, SHOpenFolderAndSelectItems, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowThreadProcessId, IsWindow, IsWindowVisible, SW_SHOWNORMAL,
};

/// Process start as a monotonic tick from `GetTickCount64`, 1 ms resolution,
/// captured in `main()` before anything else happens. Every W1/W2 number is
/// measured relative to this.
pub fn process_uptime_ms() -> u64 {
    // SAFETY: GetTickCount64 is total and has no preconditions.
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
}

/// ms since process start. Wraps `process_uptime_ms` minus the anchor.
pub fn elapsed_since(start_tick: u64) -> u64 {
    process_uptime_ms().saturating_sub(start_tick)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    /// Working-set bytes, 0 when the process exited between enumeration and
    /// measurement (not an error).
    pub working_set: u64,
}

/// All processes whose ancestry includes `root_pid`.
///
/// BFS over a full `CreateToolhelp32Snapshot` — a few ms for ~300 processes,
/// which is fine for an on-demand diagnostics call and for the 20-tab sleep
/// check. PID reuse between snapshot and measurement is handled by treating a
/// vanished PID as `working_set == 0`.
pub fn process_tree(root_pid: u32) -> Vec<ProcessEntry> {
    let mut all = snapshot_processes();
    let mut tree = Vec::new();
    let mut frontier = vec![root_pid];
    let mut seen = std::collections::HashSet::new();
    seen.insert(root_pid);
    while let Some(pid) = frontier.pop() {
        for p in all.iter() {
            if p.parent_pid == pid && seen.insert(p.pid) {
                frontier.push(p.pid);
            }
        }
    }
    // Preserve snapshot order (parent before child, usually) for readable output.
    all.retain(|p| seen.contains(&p.pid));
    for mut p in all {
        p.working_set = working_set_bytes(p.pid);
        tree.push(p);
    }
    tree
}

pub fn snapshot_processes() -> Vec<ProcessEntry> {
    let mut out = Vec::new();
    // SAFETY: the snapshot handle is closed below on every path; the struct is
    // initialized with the required `dwSize` before first use.
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return out,
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let name_end = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                out.push(ProcessEntry {
                    pid: entry.th32ProcessID,
                    parent_pid: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..name_end]),
                    working_set: 0,
                });
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Working set in bytes, 0 when unreadable (exited, or insufficient rights —
/// which for our own child processes should not happen).
pub fn working_set_bytes(pid: u32) -> u64 {
    // SAFETY: the handle is closed on every path; `ext` is fully initialized
    // and `cb` is set to the size of the larger struct, which
    // GetProcessMemoryInfo accepts for PROCESS_MEMORY_COUNTERS_EX.
    unsafe {
        let handle: HANDLE = match OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        ) {
            Ok(h) => h,
            Err(_) => return 0,
        };
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let ok = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        );
        let _ = CloseHandle(handle);
        if ok.is_ok() {
            counters.WorkingSetSize as u64
        } else {
            0
        }
    }
}

pub fn current_pid() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetCurrentProcessId() }
}

/// Upper bound on the command line we are willing to copy out of another
/// process. A real WebView2 command line is a few hundred bytes; the bound
/// exists so a partial or torn read of `RTL_USER_PROCESS_PARAMETERS` cannot turn
/// into a huge allocation. A line this long is truncated, not rejected.
const MAX_COMMAND_LINE_BYTES: usize = 1 << 15;

/// What a process's command line says about its role, read from the PEB.
///
/// Deliberately not the whole command line: inside a WebView2 child tree the only
/// thing that separates the browser process from its children is the presence and
/// value of `--type=`, and returning the full line would hand every caller a
/// string containing our user-data path and every browser argument.
pub(crate) enum CommandLineType {
    /// The command line carried no `--type=` argument — for
    /// `msedgewebview2.exe`, the browser process itself.
    NoTypeArg,
    /// `--type=<value>`, the value exactly as written (`renderer`, `gpu-process`,
    /// `utility`, `crashpad-handler`, …).
    TypeArg(String),
    /// The command line could not be read. Not a classification: a caller must
    /// report this separately rather than fold it into either answer, because
    /// "unreadable" and "no `--type=`" both look like an absent argument until a
    /// browser process is miscounted as a renderer or the reverse.
    Unreadable,
}

/// The `--type=` argument of `pid`'s command line. See [`CommandLineType`] for why
/// only that one argument is read, and `ai::processes` for what the answer means.
///
/// Cost: one `OpenProcess`, one `NtQueryInformationProcess`, three
/// `ReadProcessMemory` (PEB pointer, process-parameter block, the `--type=`
/// substring window). Microseconds per call; call it for the handful of
/// `msedgewebview2.exe` members of our own tree, not for every row of a snapshot.
///
/// Called only from the `--proof` harness. It reaches into another process's
/// memory through `ReadProcessMemory`, which is not a documented embedder API, so
/// it must never run during normal browsing or on startup.
pub(crate) fn command_line_type(pid: u32) -> CommandLineType {
    // SAFETY: every pointer passed below is either a live local of the exact
    // size declared, or a pointer value that was itself read out of the target's
    // address space and null-checked; the handle is closed on every path. A
    // failed read yields `Unreadable`, never a partially populated answer.
    unsafe {
        let handle = match open_for_command_line(pid) {
            Some(h) => h,
            None => return CommandLineType::Unreadable,
        };
        let out = read_command_line_type(handle);
        let _ = CloseHandle(handle);
        out
    }
}

/// `PROCESS_QUERY_INFORMATION` is what `NtQueryInformationProcess` documents for
/// `ProcessBasicInformation`; `PROCESS_QUERY_LIMITED_INFORMATION` usually works
/// too and is what this crate already uses for memory counters. Try limited
/// first, then the documented one — a WebView2 child that refuses one may accept
/// the other.
fn open_for_command_line(pid: u32) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION};
    [
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
        PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
    ]
    .into_iter()
    .find_map(|access| {
        // SAFETY: `OpenProcess` takes an access mask and a pid and returns either
        // a valid handle or an error; the caller closes it.
        unsafe { OpenProcess(access, false, pid).ok() }
    })
}

/// The PEB walk. Kept separate from [`command_line_type`] so exactly one caller
/// owns the handle.
///
/// # Safety
/// `handle` must be a live handle to a process opened with query and VM read
/// access.
unsafe fn read_command_line_type(handle: windows::Win32::Foundation::HANDLE) -> CommandLineType {
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
    use windows::Win32::System::Threading::{
        PEB, PROCESS_BASIC_INFORMATION, RTL_USER_PROCESS_PARAMETERS,
    };

    let mut basic = PROCESS_BASIC_INFORMATION::default();
    // SAFETY: `basic` is a live local of the exact type and size passed in, and
    // `ProcessBasicInformation` writes only that struct.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            ProcessBasicInformation,
            core::ptr::addr_of_mut!(basic) as *mut core::ffi::c_void,
            std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            core::ptr::null_mut(),
        )
    };
    // NTSTATUS has no `ok()` in `windows` 0.62.2; STATUS_SUCCESS is 0.
    if status.0 != 0 || basic.PebBaseAddress.is_null() {
        return CommandLineType::Unreadable;
    }

    let peb = basic.PebBaseAddress;
    let mut params: *mut RTL_USER_PROCESS_PARAMETERS = core::ptr::null_mut();
    // The offset of `ProcessParameters` comes from the `PEB` type rather than a
    // literal, so it is the target's own offset and not an assumption.
    if !read_raw_at(
        handle,
        (peb as *const u8).add(std::mem::offset_of!(PEB, ProcessParameters))
            as *const core::ffi::c_void,
        core::ptr::addr_of_mut!(params) as *mut core::ffi::c_void,
        std::mem::size_of::<*mut RTL_USER_PROCESS_PARAMETERS>(),
    ) || params.is_null()
    {
        return CommandLineType::Unreadable;
    }

    let mut block = RTL_USER_PROCESS_PARAMETERS::default();
    if !read_raw_at(
        handle,
        params as *const core::ffi::c_void,
        core::ptr::addr_of_mut!(block) as *mut core::ffi::c_void,
        std::mem::size_of::<RTL_USER_PROCESS_PARAMETERS>(),
    ) {
        return CommandLineType::Unreadable;
    }

    let line = block.CommandLine;
    let bytes = (line.Length as usize).min(MAX_COMMAND_LINE_BYTES);
    if bytes < 2 || line.Buffer.is_null() {
        return CommandLineType::Unreadable;
    }
    let units = bytes / 2;
    let mut wide = vec![0u16; units];
    if !read_raw_at(
        handle,
        line.Buffer.0 as *const core::ffi::c_void,
        wide.as_mut_ptr() as *mut core::ffi::c_void,
        units * 2,
    ) {
        return CommandLineType::Unreadable;
    }
    // Only `--type=` is read back out; the rest of the line is dropped here.
    match type_arg_of(&String::from_utf16_lossy(&wide)) {
        Some(value) => CommandLineType::TypeArg(value),
        None => CommandLineType::NoTypeArg,
    }
}

/// The value of the `--type=` argument, or `None` when the line has no such
/// argument. Split out so the classification rule is testable without a second
/// process: a WebView2 browser process must land on `None` and a renderer on
/// `Some("renderer")`.
fn type_arg_of(command_line: &str) -> Option<String> {
    command_line
        .split_whitespace()
        .find_map(|arg| arg.strip_prefix("--type=").map(|v| v.to_string()))
}

/// One bounded `ReadProcessMemory`. False on any failure, including a target that
/// exited mid-walk.
///
/// # Safety
/// `src` must be readable for `len` bytes in the process `handle` refers to,
/// `dst` must be writable for `len` bytes in this process, and `handle` must be
/// open to a live process with VM read access.
unsafe fn read_raw_at(
    handle: windows::Win32::Foundation::HANDLE,
    src: *const core::ffi::c_void,
    dst: *mut core::ffi::c_void,
    len: usize,
) -> bool {
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    // SAFETY: preconditions are those of this function, restated at each call.
    unsafe { ReadProcessMemory(handle, src, dst, len, None).is_ok() }
}

#[cfg(test)]
mod tests {
    use super::type_arg_of;

    /// The rule the whole browser-process-vs-renderer split rests on, checked
    /// against command lines in the shape WebView2 actually uses. Getting this
    /// wrong is the miscount `ai::processes` exists to make impossible: treating a
    /// renderer as a browser process makes one shared environment look like several.
    #[test]
    fn the_browser_process_line_has_no_type_argument_and_the_renderer_has_one() {
        let browser = r#""C:\Program Files (x86)\Microsoft\Edge\Application\msedgewebview2.exe" --embedded-browser-webview=1 --webview-exe-name=native-browser --user-data-dir="C:\Users\me\AppData\Local\native-browser\webview2" --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"#;
        assert_eq!(type_arg_of(browser), None);

        assert_eq!(
            type_arg_of(r#""...\msedgewebview2.exe" --type=renderer --video-capture-use-gpu-memory-buffer --lang=en-US"#)
                .as_deref(),
            Some("renderer")
        );
        assert_eq!(
            type_arg_of(r#""...\msedgewebview2.exe" --type=gpu-process --gpu-preferences=1"#)
                .as_deref(),
            Some("gpu-process")
        );
        // Order within the line is irrelevant, and a value containing a path must
        // not be mistaken for the argument we are looking for.
        assert_eq!(
            type_arg_of(r#""a.exe" --variant=type=renderer --type=utility --mojo-named-platform-channel-pair=1"#).as_deref(),
            Some("utility")
        );
        assert_eq!(type_arg_of(""), None);
    }
}

/// True when `hwnd` is a real, visible child window.
pub fn is_live_window(hwnd: isize) -> bool {
    let h = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: IsWindow/IsWindowVisible accept any handle and return false when
    // it is invalid or foreign — no dereference we control.
    unsafe { IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool() }
}

/// The process that owns `hwnd`, for attributing a webview HWND to a tab.
pub fn window_process_id(hwnd: isize) -> u32 {
    let h = HWND(hwnd as *mut core::ffi::c_void);
    let mut pid = 0u32;
    // SAFETY: pid points to a valid u32 for the duration of the call; an
    // invalid HWND makes the function leave it at 0.
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    pid
}

/// Round the corners of the page window sitting at `rect` (physical pixels, parent client
/// coordinates) inside `parent`, or restore a square one when `radius` is 0. WebView2
/// hosts its page in a child window, so a window region clips the page to the shape.
pub fn round_webview(handle: isize, width: i32, height: i32, radius: i32) {
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    // Wry moves child HWNDs asynchronously; identify the view by handle, not its old geometry.
    unsafe {
        if radius > 0 && width > 2 * radius && height > 2 * radius {
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, 2 * radius, 2 * radius);
            SetWindowRgn(hwnd, Some(region), true);
        } else {
            SetWindowRgn(hwnd, None, true);
        }
    }
}

pub fn round_child_at(parent: isize, rect: (i32, i32, i32, i32), radius: i32) {
    use windows::Win32::Foundation::{LPARAM, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{ClientToScreen, CreateRoundRectRgn, SetWindowRgn};
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetWindowRect};

    struct Probe {
        origin: POINT,
        rect: (i32, i32, i32, i32),
        hits: Vec<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        // SAFETY: lparam is the address of the `Probe` that outlives EnumChildWindows.
        let probe = unsafe { &mut *(lparam.0 as *mut Probe) };
        let mut r = RECT::default();
        // SAFETY: r is a valid out-pointer; a stale HWND makes the call fail harmlessly.
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
            let (x, y) = (r.left - probe.origin.x, r.top - probe.origin.y);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let (rx, ry, rw, rh) = probe.rect;
            if (x - rx).abs() <= 1
                && (y - ry).abs() <= 1
                && (w - rw).abs() <= 1
                && (h - rh).abs() <= 1
            {
                probe.hits.push(hwnd);
            }
        }
        true.into()
    }

    let parent = HWND(parent as *mut core::ffi::c_void);
    let mut probe = Probe {
        origin: POINT { x: 0, y: 0 },
        rect,
        hits: Vec::new(),
    };
    // SAFETY: parent is the app's own window; the probe pointer is valid for the call.
    unsafe {
        if !ClientToScreen(parent, &mut probe.origin).as_bool() {
            return;
        }
        let _ = EnumChildWindows(
            Some(parent),
            Some(visit),
            LPARAM(&mut probe as *mut Probe as isize),
        );
    }
    for hwnd in probe.hits {
        // SAFETY: the region is handed to the window, which owns it afterwards.
        unsafe {
            if radius > 0 && rect.2 > 2 * radius && rect.3 > 2 * radius {
                let rgn = CreateRoundRectRgn(0, 0, rect.2 + 1, rect.3 + 1, 2 * radius, 2 * radius);
                SetWindowRgn(hwnd, Some(rgn), true);
            } else {
                SetWindowRgn(hwnd, None, true);
            }
        }
    }
}

/// Open a file with its default handler. Returns false when the shell refused.
pub fn shell_open(path: &str) -> bool {
    let path_w = wide(path);
    // SAFETY: all strings are NUL-terminated and outlive the call; a null HWND
    // owner is valid.
    unsafe {
        let rc = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        rc.0 as usize > 32
    }
}

/// Reveal a file in Explorer with it selected. Falls back to opening the
/// containing directory when the shell rejects the selection request.
pub fn shell_reveal(path: &str) -> bool {
    let path_w = wide(path);
    // SAFETY: ILCreateFromPathW returns an ITEMIDLIST owned by the shell
    // allocator; we free it exactly once via CoTaskMemFree. The handle may be
    // null, which we check before use.
    unsafe {
        let pidl = ILCreateFromPathW(PCWSTR(path_w.as_ptr()));
        if !pidl.is_null() {
            let ok = SHOpenFolderAndSelectItems(pidl, None, 0).is_ok();
            CoTaskMemFree(Some(pidl as *const core::ffi::c_void));
            if ok {
                return true;
            }
        }
    }
    match std::path::Path::new(path).parent() {
        Some(dir) => shell_open(&dir.to_string_lossy()),
        None => false,
    }
}

/// Reveal a directory itself (settings "Open data folder").
pub fn shell_open_folder(dir: &str) -> bool {
    shell_open(dir)
}

/// Human-readable megabytes with one decimal, for logs.
pub fn mb(bytes: u64) -> f64 {
    (bytes as f64) / (1024.0 * 1024.0)
}

/// A borrowed path to a NUL-terminated wide string.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Take ownership of a shell-allocated wide string and free it.
pub fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller guarantees `p` came from the shell task allocator
    // (SHGetKnownFolderPath), is NUL-terminated, and is freed exactly once here.
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s
    }
}
