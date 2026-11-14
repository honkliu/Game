//! Win32 helpers for the probe: monotonic timing, whole-process-tree memory,
//! window style/region control, screen pixel readback, synthetic clicks.
//!
//! Each wrapper is one Win32 call whose result the probe prints. Nothing here
//! decides an experiment outcome; it only produces numbers.

use std::mem::size_of;

use windows::core::BOOL;
use windows::Win32::Foundation::{COLORREF, GetLastError, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, CreateRectRgn, DeleteDC,
    DeleteObject, GetDC, GetPixel, HBITMAP, HDC, HGDIOBJ, HRGN, ReleaseDC, RGN_DIFF, SelectObject,
    SetWindowRgn, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetAncestor, GetClassNameW, GetClientRect, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, SetCursorPos, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WindowFromPoint, GA_PARENT, GA_ROOT, GWL_EXSTYLE, HWND_TOP, PW_RENDERFULLCONTENT,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_REMOTESESSION, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WINDOW_EX_STYLE,
    WNDENUMPROC,
};

// ---------------------------------------------------------------------------
// Monotonic time
// ---------------------------------------------------------------------------

/// Ticks of the query performance counter. Process-global and monotonic; every
/// timestamp in this probe is either a QPC tick or a millisecond derived from
/// one, so host-side numbers are directly comparable with each other.
pub fn qpc_ticks() -> i64 {
    let mut t = 0i64;
    // SAFETY: QPC writes through a valid pointer to an i64 and has no other
    // preconditions; the counter exists on every Windows build we target.
    unsafe {
        let _ = QueryPerformanceCounter(&mut t);
    }
    t
}

/// QPC ticks per second. Cached because the frequency cannot change while the
/// process lives.
pub fn qpc_frequency() -> i64 {
    static FREQ: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        // SAFETY: see `qpc_ticks`.
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
        }
        if f == 0 {
            1_000_000 // never observed; keeps `qpc_ms` from dividing by zero
        } else {
            f
        }
    })
}

/// Milliseconds since an arbitrary process-local origin, at sub-microsecond
/// resolution. Only deltas are meaningful.
pub fn qpc_ms() -> f64 {
    qpc_ticks() as f64 * 1000.0 / qpc_frequency() as f64
}

/// Milliseconds elapsed since a value from [`qpc_ms`].
pub fn ms_since(since_ms: f64) -> f64 {
    qpc_ms() - since_ms
}

/// Microseconds elapsed since a value from [`qpc_ticks`].
pub fn us_since(since_ticks: i64) -> f64 {
    (qpc_ticks() - since_ticks) as f64 * 1_000_000.0 / qpc_frequency() as f64
}

// ---------------------------------------------------------------------------
// Whole-process-tree memory
// ---------------------------------------------------------------------------

/// Summed memory of this process plus everything below it in the child chain.
///
/// WebView2 runs one browser process per environment plus one renderer per
/// document group, so a single-process number hides exactly the cost this probe
/// is measuring: the extra renderer a second controller needs.
///
/// `working_set` is the closest Windows analogue of RSS and double-counts
/// shared pages across processes; `private_bytes` does not. Both are reported
/// because "RSS" is ambiguous on Windows.
#[derive(Clone, Copy, Default, Debug)]
pub struct TreeMemory {
    pub processes: u32,
    pub working_set: u64,
    pub private_bytes: u64,
}

impl TreeMemory {
    /// Working-set delta expressed in mebibytes, one decimal, signed.
    pub fn ws_mib(&self) -> f64 {
        self.working_set as f64 / (1024.0 * 1024.0)
    }
    pub fn priv_mib(&self) -> f64 {
        self.private_bytes as f64 / (1024.0 * 1024.0)
    }
}

struct ProcRow {
    pid: u32,
    parent: u32,
    name: String,
}

fn snapshot_processes() -> Vec<ProcRow> {
    let mut rows = Vec::new();
    // SAFETY: the snapshot handle is checked before use and closed on every
    // path; `PROCESSENTRY32W.dwSize` is set before the first call, which is the
    // documented precondition of the Toolhelp enumeration.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return rows;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let name_len = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
                rows.push(ProcRow {
                    pid: entry.th32ProcessID,
                    parent: entry.th32ParentProcessID,
                    name,
                });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
    }
    rows
}

/// `root` plus every process reachable from it through parent-pid links.
fn tree_members(root: u32) -> Vec<ProcRow> {
    let rows = snapshot_processes();
    let mut keep = Vec::new();
    let mut seen: Vec<u32> = vec![root];
    let mut frontier = vec![root];
    while let Some(pid) = frontier.pop() {
        for r in &rows {
            if r.parent == pid && !seen.contains(&r.pid) {
                seen.push(r.pid);
                frontier.push(r.pid);
                keep.push(ProcRow {
                    pid: r.pid,
                    parent: r.parent,
                    name: r.name.clone(),
                });
            }
        }
    }
    if let Some(me) = rows.iter().find(|r| r.pid == root) {
        keep.push(ProcRow {
            pid: me.pid,
            parent: me.parent,
            name: me.name.clone(),
        });
    }
    keep
}

/// This process's id — the root of every tree measurement.
pub fn root_pid() -> u32 {
    // SAFETY: no preconditions; returns the current process id.
    unsafe { GetCurrentProcessId() }
}

/// Working set and private bytes for this process and everything it spawned.
pub fn tree_memory() -> TreeMemory {
    let mut out = TreeMemory::default();
    for m in tree_members(root_pid()) {
        if let Some((ws, pv)) = mem_counters(m.pid) {
            out.processes += 1;
            out.working_set += ws;
            out.private_bytes += pv;
        }
    }
    out
}

/// Per-process breakdown, largest working set first, so the report can name
/// which process the overlay's memory landed in.
pub fn tree_detail() -> Vec<(u32, String, u64, u64)> {
    let mut out = Vec::new();
    for m in tree_members(root_pid()) {
        if let Some((ws, pv)) = mem_counters(m.pid) {
            out.push((m.pid, m.name.clone(), ws, pv));
        }
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.2));
    out
}

/// `(working set, private bytes)` for one pid, or `None` if it exited or the
/// handle could not be opened.
fn mem_counters(pid: u32) -> Option<(u64, u64)> {
    // SAFETY: the handle is closed on every path; the counters struct's `cb`
    // field is set, which is how the API version-checks the buffer. The cast to
    // `PROCESS_MEMORY_COUNTERS` is sound because `_EX` extends it in place.
    unsafe {
        let Ok(h) = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        ) else {
            return None;
        };
        let mut c = PROCESS_MEMORY_COUNTERS_EX {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let ok = K32GetProcessMemoryInfo(
            h,
            &mut c as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
        .as_bool();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        if ok {
            Some((c.WorkingSetSize as u64, c.PrivateUsage as u64))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// `HWND` from a raw handle value (what tao's `hwnd()` and wry's `hwnd()` give).
pub fn hwnd_of(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

/// The raw handle value of an `HWND`, for logging and for tao APIs.
pub fn hwnd_value(h: HWND) -> isize {
    h.0 as isize
}

/// Screen rect of a whole window, physical pixels.
pub fn window_rect(h: HWND) -> (i32, i32, i32, i32) {
    let mut r = RECT::default();
    // SAFETY: pointer to a local RECT; `h` is a live window.
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// Screen rect of a window's client area, physical pixels. The webview lives in
/// client coordinates, so this is where anchored-rect maths starts from.
pub fn client_rect_on_screen(h: HWND) -> (i32, i32, i32, i32) {
    let mut r = RECT::default();
    // SAFETY: locals only; `h` is a live window.
    unsafe {
        if GetClientRect(h, &mut r).is_err() {
            return (0, 0, 0, 0);
        }
        let mut p = POINT { x: r.left, y: r.top };
        if ClientToScreen(h, &mut p).as_bool() {
            (p.x, p.y, r.right - r.left, r.bottom - r.top)
        } else {
            (0, 0, 0, 0)
        }
    }
}

/// DPI of a window (96 == 100%), compared against the page's own
/// `devicePixelRatio` to validate the CSS-pixel → physical-pixel mapping.
pub fn dpi_of(h: HWND) -> u32 {
    // SAFETY: live window handle; returns 0 only for a bad HWND.
    unsafe { GetDpiForWindow(h) }
}

/// Move/resize a window in physical pixels, returning the microseconds the call
/// blocked. The follow-strategy comparison in V2 turns on whether `SetWindowPos`
/// is itself the bottleneck.
pub fn set_window_pos(h: HWND, x: i32, y: i32, w: i32, hh: i32, show: bool) -> f64 {
    let t0 = qpc_ticks();
    let mut flags = SWP_NOACTIVATE;
    if show {
        flags |= SWP_SHOWWINDOW;
    }
    // SAFETY: HWND from a live window; a null `hwndinsertafter` means "leave the
    // Z-order alone", which is what SWP_NOACTIVATE + no TOPMOST asks for.
    unsafe {
        let _ = SetWindowPos(h, None, x, y, w, hh, flags);
    }
    us_since(t0)
}

/// Show a window without taking foreground focus away from the page beneath it.
pub fn show_no_activate(h: HWND) {
    // SAFETY: live window handle.
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
    }
}

/// Raise a window above its siblings without activating it.
///
/// `WindowFromPoint` answers for whichever window is *on top*, so every hit-test
/// measurement in this crate is only about its own overlay while its window is
/// actually on top. A re-run of `v3` here printed "no probe window" for both probe
/// points and a card pixel of rgb(25,26,27) instead of red, because a
/// `Chrome_WidgetWin_1` from another process — one of 14 live `msedgewebview2.exe`
/// hosts on this desktop — was sitting over the probe. Raising before each
/// measurement is what makes the numbers mean what they say;
/// [`crate::harness::Harness::root_at`] then verifies it and the caller refuses to
/// grade the row when it did not take.
pub fn bring_to_top(h: HWND) {
    // SAFETY: live window handle; HWND_TOP with NOMOVE/NOSIZE/NOACTIVATE changes
    // only the Z-order slot and never the geometry or focus.
    unsafe {
        let _ = SetWindowPos(
            h,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// The top-level window a handle belongs to. Two handles can then be compared as
/// "the same window" even when one is a surface window three levels inside the
/// other, which is exactly what `WindowFromPoint` returns for a WebView2.
pub fn root_of(value: isize) -> isize {
    // SAFETY: GetAncestor with no matching ancestor returns NULL rather than
    // faulting, and a bad handle is reported as 0 by the caller's comparison.
    unsafe { hwnd_value(GetAncestor(hwnd_of(value), GA_ROOT)) }
}

/// OR `add` into and ANDN `remove` out of the window's extended styles,
/// returning the previous mask as printed.
pub fn modify_ex_style(h: HWND, add: WINDOW_EX_STYLE, remove: WINDOW_EX_STYLE) -> isize {
    // SAFETY: GWL_EXSTYLE indexes the window's own memory rather than a pointer;
    // Get/SetWindowLongPtrW on a live HWND is the documented pattern. The style
    // mask is 32 bits, so the isize round-trip loses nothing here.
    unsafe {
        let cur = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let next = (cur & !remove.0) | add.0;
        SetWindowLongPtrW(h, GWL_EXSTYLE, next as isize);
        cur as isize
    }
}

/// Current extended style mask.
pub fn ex_style(h: HWND) -> isize {
    // SAFETY: live window handle.
    unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) }
}

/// Restrict hit-testing to `outer` minus `hole`, in *client* pixels. Pixels
/// outside the region behave as if the window were absent, which is what makes a
/// full-window overlay click-through while the card itself stays clickable.
///
/// `hole == None` restores the whole window as hittable.
pub fn set_region_with_hole(h: HWND, outer: (i32, i32, i32, i32), hole: Option<(i32, i32, i32, i32)>) -> bool {
    let (ox, oy, ow, oh) = outer;
    // SAFETY: every region created here is either handed to `SetWindowRgn`
    // (which takes ownership on success) or deleted on the path that created it.
    unsafe {
        let outer_rgn: HRGN = CreateRectRgn(ox, oy, ox + ow, oy + oh);
        let final_rgn = match hole {
            None => outer_rgn,
            Some((hx, hy, hw, hh)) => {
                let hole_rgn = CreateRectRgn(hx, hy, hx + hw, hy + hh);
                let diff = CreateRectRgn(0, 0, 0, 0);
                combine_rgn_diff(diff, outer_rgn, hole_rgn);
                let _ = DeleteObject(outer_rgn.into());
                let _ = DeleteObject(hole_rgn.into());
                diff
            }
        };
        // third argument true == repaint immediately
        let ok = SetWindowRgn(h, Some(final_rgn), true) != 0;
        if !ok {
            let _ = DeleteObject(final_rgn.into());
        }
        ok
    }
}

/// Subtract `src2` from `src1` into `dst`. Split out so the unsafe block above
/// reads as ownership, not as GDI minutiae.
///
/// # Safety
/// All three regions must be live `HRGN`s created by `CreateRectRgn`.
unsafe fn combine_rgn_diff(dst: HRGN, src1: HRGN, src2: HRGN) {
    let _ = windows::Win32::Graphics::Gdi::CombineRgn(Some(dst), Some(src1), Some(src2), RGN_DIFF);
}

/// Remove any region set by [`set_region_with_hole`].
pub fn clear_region(h: HWND) -> bool {
    // SAFETY: a null region means "no clipping" and transfers no ownership.
    unsafe { SetWindowRgn(h, None, true) != 0 }
}

/// Restrict the window — drawing *and* hit-testing — to exactly one rectangle, in
/// client pixels. This is the opposite shape from [`set_region_with_hole`]: an
/// anchored card wants the window to exist only where the card is, so everything
/// else is not "transparent" but absent, and the OS sends those clicks down the
/// Z-order to the page.
///
/// # Safety note for readers
///
/// `SetWindowRgn` takes ownership of the region on success, so the region is only
/// deleted on the failure path; passing the same `HRGN` twice would be a double free.
pub fn set_region_rect(h: HWND, rect: (i32, i32, i32, i32)) -> bool {
    let (x, y, w, hh) = rect;
    // SAFETY: the region is created here and either transferred to the window by
    // `SetWindowRgn` or deleted when that call fails.
    unsafe {
        let rgn = CreateRectRgn(x, y, x + w, y + hh);
        if SetWindowRgn(h, Some(rgn), true) != 0 {
            true
        } else {
            let _ = DeleteObject(rgn.into());
            false
        }
    }
}

/// Every child HWND of `h` with its class name. WebView2 creates its own child
/// window inside a wry window, and that child is the surface that actually
/// receives mouse input — which HWND wins the hit-test is a V3 measurement.
pub fn child_windows(h: HWND) -> Vec<(isize, String)> {
    // SAFETY: the callback appends to a local Vec whose borrow lives for the
    // whole enumeration; `LPARAM` carries a pointer to it, which is the
    // documented use of the last parameter.
    unsafe {
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let acc = &mut *(lparam.0 as *mut Vec<isize>);
            acc.push(hwnd.0 as isize);
            true.into()
        }
        let mut acc: Vec<isize> = Vec::new();
        let lp = LPARAM(&mut acc as *mut Vec<isize> as isize);
        let cb: WNDENUMPROC = Some(collect);
        let _ = EnumChildWindows(Some(h), cb, lp);
        acc.into_iter()
            .map(|hw| (hw, class_name_of(hwnd_of(hw))))
            .collect()
    }
}

/// Window class name; empty if the handle is stale.
pub fn class_name_of(h: HWND) -> String {
    // SAFETY: the buffer length is passed to the API, which NUL-terminates.
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetClassNameW(h, &mut buf);
        if n <= 0 {
            return String::new();
        }
        let end = buf[..n as usize]
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(n as usize);
        String::from_utf16_lossy(&buf[..end])
    }
}

// ---------------------------------------------------------------------------
// Screen pixel readback
// ---------------------------------------------------------------------------

/// Reads single physical pixels, from whichever of two instruments this machine
/// answers.
///
/// **screen** — a 1×1 `BitBlt` off the whole-screen DC. This is the only observer
/// in the probe that is not inside our own process, so it is what "repainted on
/// screen" is measured against.
///
/// **window surface** — `PrintWindow(root, PW_RENDERFULLCONTENT)` into a
/// screen-sized bitmap, then `GetPixel` out of that bitmap. This second instrument
/// exists because the desktop on this machine is not an interactive one: screen-DC
/// readback fails there, while a window-targeted render can still return pixels.
/// It observes the window's composited surface rather than the scanout buffer, so
/// anything timed with it is printed under that label instead of being passed off
/// as on-screen.
///
/// [`PixelProbe::source`] names which instrument answered the last read and
/// [`PixelProbe::diagnostic`] prints the GDI error and `SM_REMOTESESSION` behind a
/// dead one, so a report states its instrument instead of assuming one.
pub struct PixelProbe {
    screen: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    screen_ok: bool,
    screen_err: u32,
    /// Attempts and failures of the screen instrument. `screen_ok` only says the DCs
    /// were built, which is true on a desktop that hands back no pixels, so the
    /// diagnostic reads these counters instead of that flag.
    screen_reads: u32,
    screen_fail: u32,
    surf_mem: HDC,
    surf_bmp: HBITMAP,
    surf_old: HGDIOBJ,
    surf_w: i32,
    surf_h: i32,
    surf_ok: bool,
    surf_err: u32,
    surf_fail: u32,
    surf_us: f64,
    /// Every `PrintWindow` cost taken, so the resolution quoted in the report is the
    /// median rather than whatever the last call happened to be. A push per read is
    /// free next to the 85 ms the read itself costs.
    surf_all: Vec<f64>,
    /// Every *full* [`PixelProbe::pixel`] cost taken, including the failed screen
    /// attempt that precedes a `PrintWindow` on this machine.
    read_all: Vec<f64>,
    /// `false` renders the deepest window under the point, `true` the whole
    /// top-level. Chosen by the calibration's reference pixels, not assumed.
    use_root: bool,
    last_source: &'static str,
}

impl PixelProbe {
    pub fn new() -> Self {
        // SAFETY: GetDC(None) is the whole-screen DC; the two memory DCs and
        // their bitmaps are released in `Drop`, each with the previously selected
        // object restored before the bitmap is deleted.
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, 1, 1);
            let old = SelectObject(mem, bmp.into());
            let surf_w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let surf_h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            let surf_mem = CreateCompatibleDC(Some(screen));
            let surf_bmp = CreateCompatibleBitmap(screen, surf_w, surf_h);
            let surf_old = SelectObject(surf_mem, surf_bmp.into());
            Self {
                screen,
                mem,
                bmp,
                old,
                screen_ok: !screen.is_invalid() && !mem.is_invalid() && !bmp.is_invalid(),
                screen_err: 0,
                screen_reads: 0,
                screen_fail: 0,
                surf_mem,
                surf_bmp,
                surf_old,
                surf_w,
                surf_h,
                surf_ok: !surf_mem.is_invalid() && !surf_bmp.is_invalid(),
                surf_err: 0,
                surf_fail: 0,
                surf_us: 0.0,
                surf_all: Vec::new(),
                read_all: Vec::new(),
                // Cheapest target first; the calibration falls back to the top-level
                // if the small window does not answer the reference pixels.
                use_root: false,
                last_source: "none",
            }
        }
    }

    /// False when neither instrument can hand back a pixel.
    pub fn available(&self) -> bool {
        self.screen_ok || self.surf_ok
    }

    /// Which instrument answered the last [`PixelProbe::pixel`].
    pub fn source(&self) -> &'static str {
        self.last_source
    }

    /// Median cost of a `PrintWindow` in µs — the poll resolution the window
    /// instrument imposes on anything timed with it, printed rather than assumed.
    /// The median, because one call can be an outlier and this number decides
    /// whether a latency band is measurable at all.
    pub fn surface_cost_us(&self) -> f64 {
        if self.surf_all.is_empty() {
            return self.surf_us;
        }
        let mut v = self.surf_all.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    }

    /// One line naming both instruments and why a dead one is dead, printed once
    /// per run so every pixel-derived number under it carries its own provenance.
    pub fn diagnostic(&self) -> String {
        // SAFETY: a metric query with no preconditions.
        let remote = unsafe { GetSystemMetrics(SM_REMOTESESSION) };
        format!(
            "screen BitBlt {} ({} read(s), {} failure(s), last GDI error {}); PrintWindow {} \
             onto the {} (last GDI error {}, {:.0} µs per call); a full pixel() read costs \
             {:.0} µs median; SM_REMOTESESSION={remote} — on a non-interactive desktop the \
             screen DC yields nothing, which is why the second instrument exists",
            if self.screen_reads == 0 {
                "untested"
            } else if self.screen_fail == self.screen_reads {
                "unusable"
            } else if self.screen_fail > 0 {
                "partly usable"
            } else {
                "ok"
            },
            self.screen_reads,
            self.screen_fail,
            self.screen_err,
            if self.surf_fail == 0 { "ok" } else { "degraded" },
            self.render_target(),
            self.surf_err,
            self.surface_cost_us(),
            self.read_cost_us(),
        )
    }

    /// Which window `PrintWindow` is pointed at: the top-level, or the deepest
    /// window under the point. Naming it matters because the two answer different
    /// questions and cost very different amounts.
    pub fn render_target(&self) -> &'static str {
        if self.use_root {
            "top-level window"
        } else {
            "window under the point"
        }
    }

    /// Point the `PrintWindow` instrument at the whole top-level window instead of
    /// the deepest window under the point. The top-level is the only target that
    /// shows how children were composited together, so it is the fallback when the
    /// window under the point does not answer the reference pixels.
    pub fn render_into_root_window(&mut self) {
        self.use_root = true;
    }

    /// `(r, g, b)` at physical screen coordinates, or `None` when both instruments
    /// fail.
    ///
    /// The screen DC is held for the whole run instead of re-acquired per pixel:
    /// `GetDC`/`ReleaseDC` cost roughly 10 µs each, which would swamp the
    /// sub-millisecond attribution this probe exists to measure. A DC outliving
    /// a display topology change is the trade; it degrades reads to `None`.
    pub fn pixel(&mut self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        // The cost of the whole call is recorded, not the cost of one instrument:
        // whichever of the two answers is the one that sets the resolution of any
        // latency measured with it.
        let t = qpc_ticks();
        let out = self.pixel_inner(x, y);
        self.read_all.push(us_since(t));
        out
    }

    fn pixel_inner(&mut self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        if let Some(p) = self.screen_read(x, y) {
            self.last_source = "screen";
            return Some(p);
        }
        let p = self.surface_read(x, y);
        self.last_source = if p.is_some() { "window surface" } else { "none" };
        p
    }

    /// Median cost of a full [`PixelProbe::pixel`] call in µs: the resolution of any
    /// latency this instrument times, whatever it took to get there.
    pub fn read_cost_us(&self) -> f64 {
        if self.read_all.is_empty() {
            return 0.0;
        }
        let mut v = self.read_all.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    }

    /// Fastest full read taken so far — the best resolution this instrument offered.
    pub fn read_best_us(&self) -> f64 {
        self.read_all
            .iter()
            .min_by(|a, b| a.total_cmp(b))
            .copied()
            .unwrap_or(0.0)
    }

    fn screen_read(&mut self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        if !self.screen_ok {
            return None;
        }
        // SAFETY: a 1x1 BitBlt from the screen DC into our own 1x1 compatible
        // bitmap, both DCs validated in `new`. `GetPixel` then reads our bitmap,
        // not the screen.
        unsafe {
            self.screen_reads += 1;
            if BitBlt(self.mem, 0, 0, 1, 1, Some(self.screen), x, y, SRCCOPY)
                .is_err()
            {
                self.screen_err = GetLastError().0;
                self.screen_fail += 1;
                return None;
            }
            Self::decode(GetPixel(self.mem, 0, 0))
        }
    }

    /// Render the window under `(x, y)` into the reused bitmap and read one pixel
    /// out of it.
    ///
    /// The window rendered is chosen by `use_root`: the deepest one at the point by
    /// default, because that is the window whose pixels the question is about — the
    /// overlay's, or the page's. The size heuristic that motivated preferring it did
    /// not survive measurement: on this software-rendered session a `PrintWindow` of
    /// the overlay's own 450×210 window costs the same ~85 ms as one of the
    /// 1590×990 top-level, so the cost is the synchronous compositor capture and not
    /// the area. That measured cost is why the calibration declines to time a 25 ms
    /// latency band with this instrument instead of reporting one anyway.
    fn surface_read(&mut self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        if !self.surf_ok {
            return None;
        }
        unsafe {
            let hit = WindowFromPoint(POINT { x, y });
            if hit.is_invalid() {
                self.surf_err = GetLastError().0;
                self.surf_fail += 1;
                return None;
            }
            let target = if self.use_root {
                GetAncestor(hit, GA_ROOT)
            } else {
                hit
            };
            let (lx, ly, w, h) = window_rect(target);
            if w <= 0 || h <= 0 || w > self.surf_w || h > self.surf_h {
                // The target is sized to the virtual screen; a window larger than
                // it (a second monitor) is skipped rather than read clipped.
                self.surf_fail += 1;
                return None;
            }
            // SAFETY: `target` is a live window and `surf_mem` has a bitmap at least
            // its size selected, so the render cannot write outside the bitmap.
            let t = qpc_ticks();
            let painted = PrintWindow(
                target,
                self.surf_mem,
                // PW_RENDERFULLCONTENT is what makes a GPU-composited window render
                // its real content instead of its GDI fallback (black).
                PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT),
            )
            .as_bool();
            self.surf_us = us_since(t);
            self.surf_all.push(self.surf_us);
            if !painted {
                self.surf_err = GetLastError().0;
                self.surf_fail += 1;
                return None;
            }
            Self::decode(GetPixel(self.surf_mem, x - lx, y - ly))
        }
    }

    /// COLORREF is 0x00BBGGRR; decoding it by hand avoids depending on the
    /// GetRValue/GetGValue/GetBValue helper spelling in this windows version.
    fn decode(c: COLORREF) -> Option<(u8, u8, u8)> {
        if c.0 == 0xFFFFFFFF {
            // GetPixel's own "out of bounds or no data" marker.
            return None;
        }
        Some((
            (c.0 & 0xFF) as u8,
            ((c.0 >> 8) & 0xFF) as u8,
            ((c.0 >> 16) & 0xFF) as u8,
        ))
    }
}

impl Drop for PixelProbe {
    fn drop(&mut self) {
        // SAFETY: mirrors the acquisition in `new`; both run on the owning
        // thread because `PixelProbe` holds raw GDI handles and is not `Send`.
        unsafe {
            if self.screen_ok {
                SelectObject(self.mem, self.old);
                let _ = DeleteObject(self.bmp.into());
                let _ = DeleteDC(self.mem);
                let _ = ReleaseDC(None, self.screen);
            }
            if self.surf_ok {
                SelectObject(self.surf_mem, self.surf_old);
                let _ = DeleteObject(self.surf_bmp.into());
                let _ = DeleteDC(self.surf_mem);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Synthetic input
// ---------------------------------------------------------------------------

/// Real left click at physical screen coordinates, injected through `SendInput`
/// so it takes the same path a user click does: system input queue → window
/// manager hit-test → whichever window wins. `PostMessage` would let the overlay
/// decline the message before the hit-test ran, which is precisely the question
/// V3 asks, so it is not a valid substitute.
///
/// Returns (events the queue accepted, last GDI error, where the cursor actually
/// ended up). A probe that could not report the last two printed "the page got no
/// clicks" for a run in which *no click was ever delivered*: on a non-interactive
/// desktop `SetCursorPos` and `SendInput` both fail, and the failure is the result,
/// not the click count.
pub fn click_at(x: i32, y: i32) -> (usize, u32, (i32, i32)) {
    // SAFETY: virtual-screen metrics are read immediately before the calls and
    // only used to normalise coordinates; 0..65535 across the virtual screen is
    // the documented mapping for MOUSEEVENTF_ABSOLUTE.
    unsafe {
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2);
        let nx = (((x - vx) as f64 / (vw - 1) as f64) * 65535.0).round().clamp(0.0, 65535.0) as i32;
        let ny = (((y - vy) as f64 / (vh - 1) as f64) * 65535.0).round().clamp(0.0, 65535.0) as i32;
        let seq = [
            MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE,
            MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_LEFTUP,
        ];
        let inputs: Vec<INPUT> = seq
            .iter()
            .map(|flags| INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: nx,
                        dy: ny,
                        mouseData: 0,
                        dwFlags: *flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
            .collect();
        let injected = SendInput(&inputs, size_of::<INPUT>() as i32) as usize;
        // The error has to be read *here*: `GetLastError` is per-thread and the
        // next call overwrites it, and "SendInput accepted 0 of 3" is only
        // interpretable with the code that came with it.
        let err = GetLastError().0;
        (injected, err, cursor_pos())
    }
}

/// Current cursor position, physical pixels.
pub fn cursor_pos() -> (i32, i32) {
    // SAFETY: writes through a local POINT.
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Move the cursor, physical screen coordinates. Returns whether the window
/// manager accepted the move — on a desktop with no pointing device it does not,
/// and a probe that ignores the answer measures a click that never happened.
pub fn move_cursor(x: i32, y: i32) -> bool {
    // SAFETY: absolute coordinates; no effect beyond the cursor.
    unsafe { SetCursorPos(x, y).is_ok() }
}

/// The window itself plus every ancestor up to the desktop, nearest first, each
/// with its class name. `WindowFromPoint` returns the innermost child window
/// (`Chrome_RenderWidgetHostHWND` for WebView2), which is not the handle wry hands
/// back for the view, so ownership can only be answered by walking up: the
/// ancestor list is what turns a hit-test number into "the overlay" or "the page".
pub fn ancestor_chain(value: isize) -> Vec<(isize, String)> {
    let mut out = Vec::new();
    let mut h = hwnd_of(value);
    // Bounded walk: the desktop is a few levels above any webview, and a cycle
    // here would be a window-manager bug, not something to follow.
    for _ in 0..16 {
        if h.0.is_null() {
            break;
        }
        out.push((hwnd_value(h), class_name_of(h)));
        let next = unsafe { GetAncestor(h, GA_PARENT) };
        if hwnd_value(next) == hwnd_value(h) {
            break;
        }
        h = next;
    }
    out
}

/// Which window the hit-test would pick for a point, before a click is sent.
pub fn window_from_point(x: i32, y: i32) -> isize {
    // SAFETY: `WindowFromPoint` takes POINT by value; the handle is only printed.
    unsafe {
        let p = POINT { x, y };
        hwnd_value(WindowFromPoint(p))
    }
}
