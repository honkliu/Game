//! WebView2 environment + webview construction.
//!
//! One `ICoreWebView2Environment` for the whole probe process, shared by every
//! webview. Without it each webview would create its own browser process, and
//! the V1 cross-webview question ("is `NavigationId` unique across two webviews
//! in the same environment?") would not even be a question — the webviews would
//! be in different processes.

use std::path::Path;

use tao::window::Window;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions,
};
use webview2_com::{CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler};
use windows::core::{HSTRING, PCWSTR};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows, WebContext};

use crate::win;

/// Create the single environment every webview in this process shares.
///
/// Blocks on the Win32 completion callback by pumping messages; call it before
/// the tao event loop starts, exactly as `native-browser` does, so the probe is
/// measuring a warm environment rather than environment creation.
pub fn shared_environment(data_dir: &Path) -> Result<ICoreWebView2Environment, String> {
    // SAFETY: `user_data` is an HSTRING that outlives the call; the completion
    // handler is dispatched on this thread's apartment while we pump below.
    unsafe {
        let (tx, rx) = std::sync::mpsc::channel::<Result<ICoreWebView2Environment, String>>();
        let options = CoreWebView2EnvironmentOptions::default();
        // The defaults wry itself applies, so nothing here differs from what a
        // plain `WebViewBuilder` would have produced.
        let args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
        options.set_additional_browser_arguments(args.to_string());

        let user_data = HSTRING::from(data_dir.as_os_str());
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &user_data,
            &ICoreWebView2EnvironmentOptions::from(options),
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |err, env| {
                    let r = match (err, env) {
                        (Ok(()), Some(e)) => Ok(e),
                        (Err(e), _) => Err(format!("{e}")),
                        (Ok(()), None) => Err("environment callback returned null".to_string()),
                    };
                    let _ = tx.send(r);
                    Ok(())
                },
            )),
        )
        .map_err(|e| format!("CreateCoreWebView2EnvironmentWithOptions: {e}"))?;

        let deadline = win::qpc_ms() + 20_000.0;
        loop {
            if let Ok(r) = rx.try_recv() {
                return r;
            }
            if win::qpc_ms() > deadline {
                return Err("WebView2 environment creation timed out after 20 s".to_string());
            }
            pump();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// Non-blocking message drain. Needed because WebView2 delivers its completion
/// callbacks through the apartment's message queue.
fn pump() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    // SAFETY: standard non-blocking drain on the current thread.
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Runtime version, printed in every report header so a number can be traced
/// back to the exact Edge build that produced it.
pub fn runtime_version() -> String {
    match wry::webview_version() {
        Ok(v) => v,
        Err(e) => format!("unavailable: {e}"),
    }
}

/// `Rect` in logical pixels, which is what wry's bounds API expects.
pub fn logical_rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w.max(1), h.max(1)).into(),
    }
}

/// Build a webview into `window`'s client area and return it.
///
/// `url` may be an `http(s)` URL or an `about:blank`. Pass `None` to build an
/// empty webview and navigate it later, which is what the V1 sequence does so
/// the first navigation id is observed rather than assumed.
///
/// `transparent` maps to wry's `with_transparent`, which is exactly what
/// `native-browser`'s own overlay uses (`app.rs:2396`): on WebView2 it calls
/// `SetDefaultBackgroundColor` with `A = 0`. Measuring the transparent case
/// therefore has to go through the same call the shipping overlay uses, not a
/// hand-rolled `put_DefaultBackgroundColor`.
pub fn build_webview(
    context: &mut WebContext,
    env: &ICoreWebView2Environment,
    window: &Window,
    id: &str,
    bounds: Rect,
    url: Option<&str>,
    transparent: bool,
) -> Result<WebView, String> {
    let mut builder = WebViewBuilder::new_with_web_context(context)
        .with_id(id)
        .with_bounds(bounds)
        .with_browser_accelerator_keys(false)
        .with_transparent(transparent)
        .with_environment(env.clone());
    if let Some(u) = url {
        builder = builder.with_url(u);
    }
    builder
        .build_as_child(window)
        .map_err(|e| format!("build_as_child({id}): {e}"))
}

/// The controller's *effective* default background colour, read back after the
/// build. The alpha channel printed here is what the overlay actually runs with;
/// "we asked for transparent" is not the same claim as "it is transparent".
pub fn default_background_alpha(webview: &WebView) -> String {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_COLOR, ICoreWebView2Controller2,
    };
    use windows::core::Interface;
    let controller = webview.controller();
    match controller.cast::<ICoreWebView2Controller2>() {
        Ok(c2) => {
            let mut color = COREWEBVIEW2_COLOR::default();
            // SAFETY: the controller is alive on this thread's apartment and the
            // out-param is a local struct the callee fills in.
            let _ = unsafe { c2.DefaultBackgroundColor(&mut color) };
            format!("A={} R={} G={} B={}", color.A, color.R, color.G, color.B)
        }
        Err(e) => format!("unreadable ({e})"),
    }
}

/// The raw `ICoreWebView2` behind a wry webview — the entry point to every
/// measurement in this crate.
pub fn com_of(webview: &WebView) -> webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2 {
    webview.webview()
}
