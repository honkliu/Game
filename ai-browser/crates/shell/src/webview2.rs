//! WebView2 Evergreen runtime gate (DESIGN A5).
//!
//! Detection, not installation. If the runtime is absent we show one clear
//! dialog with the download URL and exit. The Windows installer handles runtime
//! installation; this gate also supports portable executable launches.

use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions,
};
use webview2_com::{CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

const DOWNLOAD_HINT: &str =
    "https://developer.microsoft.com/microsoft-edge/webview2/";

/// `ICoreWebView2Environment` instances this process has constructed.
///
/// R0a-1's assertion is "exactly one, for the process lifetime", so the count is
/// incremented at the one site in this crate that creates an environment and read
/// back by `--proof` ([`environments_constructed`]). It is a measurement at the
/// construction site, not an inference from the process list.
///
/// Its scope, stated plainly: this counts *this function's* constructions. `wry`
/// creates an environment itself when a builder is handed none (its
/// `WebContextImpl` is an empty struct on Windows), and that path is invisible
/// here. It is only reachable when `create_shared_environment` fails, because
/// `app.rs` then passes no environment to the builders — which costs one browser
/// process per webview. That is why R0a-1's other half, the
/// `msedgewebview2` browser-process delta measured in `ai::proof`, is the
/// cross-check on this counter rather than a restatement of it.
static ENVIRONMENTS_CONSTRUCTED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Keep WebView2's HTML drag/drop handling; only the fallback PDF drop sends a disk-backed File.
pub fn install_pdf_drop(webview: &wry::WebView, on_drop: impl Fn(String) + 'static) -> windows::core::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2,
    };
    use webview2_com::WebMessageReceivedEventHandler;
    use windows::core::{Interface, PWSTR};
    use wry::WebViewExtWindows;

    unsafe {
        let core = webview.controller().CoreWebView2()?;
        let mut token = 0;
        core.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                let mut message = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut message).is_err() { return Ok(()) }
                if crate::sys::take_pwstr(message) != "native-browser:pdf-drop" { return Ok(()) }
                let mut source = PWSTR::null();
                args.Source(&mut source)?;
                let source = crate::sys::take_pwstr(source);
                let mut current = PWSTR::null();
                sender.Source(&mut current)?;
                if source != crate::sys::take_pwstr(current) { return Ok(()) }
                let objects = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>()?.AdditionalObjects()?;
                let mut count = 0;
                objects.Count(&mut count)?;
                if count != 1 { return Ok(()) }
                let file = objects.GetValueAtIndex(0)?.cast::<ICoreWebView2File>()?;
                let mut path = PWSTR::null();
                file.Path(&mut path)?;
                let path = std::path::PathBuf::from(crate::sys::take_pwstr(path));
                if let Some(url) = crate::pdf::dropped_file_url(&path) { on_drop(url); }
                Ok(())
            })),
            &mut token,
        )
    }
}

pub fn install_zoom_changed(webview: &wry::WebView, on_zoom: impl Fn(f64) + 'static) -> windows::core::Result<()> {
    use webview2_com::ZoomFactorChangedEventHandler;
    use wry::WebViewExtWindows;

    unsafe {
        let mut token = 0;
        webview.controller().add_ZoomFactorChanged(
            &ZoomFactorChangedEventHandler::create(Box::new(move |controller, _| {
                if let Some(controller) = controller {
                    let mut factor = 1.0;
                    controller.ZoomFactor(&mut factor)?;
                    if factor.is_finite() && factor > 0.0 { on_zoom(factor); }
                }
                Ok(())
            })),
            &mut token,
        )
    }
}

pub fn install_browser_shortcuts(webview: &wry::WebView, on_shortcut: impl Fn(crate::keys::Shortcut) + 'static) -> windows::core::Result<()> {
    use crate::keys::{self, Mods, Outcome, Shortcut};
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    use wry::WebViewExtWindows;

    // WebView2 owns the focused child HWND, so its keys need not enter tao's message hook.
    unsafe {
        let mut token = 0;
        webview.controller().add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut key = 0;
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                args.VirtualKey(&mut key)?;
                args.KeyEventKind(&mut kind)?;
                if !matches!(kind, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN | COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN) {
                    return Ok(());
                }
                let mods = Mods {
                    ctrl: GetKeyState(0x11) < 0,
                    alt: GetKeyState(0x12) < 0,
                    shift: GetKeyState(0x10) < 0,
                };
                let Outcome::Consume(shortcut) = keys::resolve_native(key, mods) else { return Ok(()) };
                args.SetHandled(true)?;
                let mut lparam = 0;
                args.KeyEventLParam(&mut lparam)?;
                if shortcut != Shortcut::OpenDevtools || lparam & (1 << 30) == 0 {
                    // Queue the action: windowed accelerator callbacks block cross-process COM.
                    on_shortcut(shortcut);
                }
                Ok(())
            })),
            &mut token,
        )
    }
}

pub fn zoom_factor(webview: &wry::WebView) -> windows::core::Result<f64> {
    use wry::WebViewExtWindows;
    let mut factor = 1.0;
    unsafe { webview.controller().ZoomFactor(&mut factor)?; }
    Ok(factor)
}

pub fn install_pdf_detection(webview: &wry::WebView, on_pdf: impl Fn(String) + 'static) -> windows::core::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use webview2_com::WebResourceResponseReceivedEventHandler;
    use windows::core::{Interface, PWSTR};
    use wry::WebViewExtWindows;

    unsafe {
        let core = webview.controller().CoreWebView2()?.cast::<ICoreWebView2_2>()?;
        let mut token = 0;
        core.add_WebResourceResponseReceived(
            &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut content_type = PWSTR::null();
                if args.Response()?.Headers()?.GetHeader(w!("Content-Type"), &mut content_type).is_err() { return Ok(()) }
                let content_type = crate::sys::take_pwstr(content_type);
                if !content_type.split(';').next().is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/pdf")) {
                    return Ok(());
                }
                let mut url = PWSTR::null();
                args.Request()?.Uri(&mut url)?;
                on_pdf(crate::sys::take_pwstr(url));
                Ok(())
            })),
            &mut token,
        )
    }
}

/// How many environments [`create_shared_environment`] has handed back. Call
/// before the first webview is built and again after; the two must both read the
/// same single increment.
pub fn environments_constructed() -> u64 {
    ENVIRONMENTS_CONSTRUCTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Create the single `ICoreWebView2Environment` every webview shares.
///
/// `wry`'s `WebContext` carries no environment on Windows (its
/// `WebContextImpl` is an empty struct), so without this each `WebViewBuilder`
/// silently creates its own browser process — three webviews meant three
/// browser processes and three renderers. Passing this one environment to every
/// builder via `WebViewBuilderExtWindows::with_environment` makes them share the
/// browser process, the way a real browser does.
///
/// Blocks on the Win32 callback; call before the event loop starts.
pub fn create_shared_environment(
    data_dir: &std::path::Path,
) -> Result<ICoreWebView2Environment, String> {
    // SAFETY: `data_dir` is passed as a NUL-terminated HSTRING that outlives the
    // call. The completed-handler runs on this thread's COM apartment before
    // `CreateCoreWebView2EnvironmentWithOptions` returns (it is asynchronous but
    // the callback is invoked via the message loop we pump below).
    unsafe {
        let (tx, rx) = std::sync::mpsc::channel::<Result<ICoreWebView2Environment, String>>();
        let options = CoreWebView2EnvironmentOptions::default();
        // Same defaults wry would apply, so behaviour does not change.
        let mut args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection".to_string();
        // Developer/test hook: lets automation drive the real pages over CDP.
        if let Some(port) = std::env::var("BROWSER_DEBUG_PORT").ok().filter(|p| p.chars().all(|c| c.is_ascii_digit())) {
            if !port.is_empty() {
                args.push_str(&format!(" --remote-debugging-port={port}"));
            }
        }
        options.set_additional_browser_arguments(args);

        let user_data = HSTRING::from(data_dir.as_os_str());
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &user_data,
            &ICoreWebView2EnvironmentOptions::from(options),
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |err, env| {
                    let r = match (err, env) {
                        (Ok(()), Some(e)) => {
                            // The environment exists from here, so this is the
                            // construction site the R0a-1 counter belongs at —
                            // not the call above, which can fail.
                            ENVIRONMENTS_CONSTRUCTED
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            Ok(e)
                        }
                        (Err(e), _) => Err(format!("{e}")),
                        (Ok(()), None) => Err("environment callback gave null env".into()),
                    };
                    let _ = tx.send(r);
                    Ok(())
                },
            )),
        )
        .map_err(|e| format!("CreateCoreWebView2EnvironmentWithOptions: {e}"))?;

        // The completion handler is dispatched through COM's message pump; pump
        // until it fires. 10s is far longer than a local environment takes.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(r) = rx.try_recv() {
                return r;
            }
            if std::time::Instant::now() > deadline {
                return Err("WebView2 environment creation timed out".into());
            }
            pump_messages();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

fn pump_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    // SAFETY: standard non-blocking message drain on the current thread.
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Returns the runtime version string, or an error explaining the absence.
pub fn runtime_version() -> Result<String, String> {
    // wry's own check: asks the loader for the WebView2 loader and then creates
    // the environment. `webview_version()` is the cheapest documented probe and
    // is what would fail at build() anyway — failing here lets us show a dialog
    // instead of a panic inside tao.
    match wry::webview_version() {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        Ok(_) => Err("WebView2 runtime reported an empty version".to_string()),
        Err(e) => Err(format!("{e}")),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Modal error box. Safe to call before any window exists.
pub fn show_fatal(message: &str) {
    let body = wide(message);
    let caption = w!("River");
    // SAFETY: both strings are NUL-terminated and outlive the call; a null
    // owner HWND means "desktop owner", which is valid.
    unsafe {
        MessageBoxW(None, PCWSTR(body.as_ptr()), caption, MB_OK | MB_ICONERROR);
    }
}

/// Gate start-up. On failure this shows the dialog and returns `Err`, and the
/// caller exits with a non-zero code.
pub fn ensure_runtime() -> Result<String, String> {
    match runtime_version() {
        Ok(v) => Ok(v),
        Err(e) => {
            let detail = if e.contains("ELF_SIGNED_OUT") || e.contains("80070490") {
                "The WebView2 Evergreen Runtime is not installed.".to_string()
            } else {
                format!("WebView2 failed to initialize: {e}")
            };
            show_fatal(&format!(
                "{detail}\n\nRiver renders pages with Microsoft Edge WebView2.\n\
                 Install the Evergreen Runtime (x64) and start the browser again:\n\n\
                 {DOWNLOAD_HINT}"
            ));
            Err(detail)
        }
    }
}