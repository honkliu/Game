//! Raw COM event sinks.
//!
//! Everything the probe measures about navigation comes from these handlers,
//! attached to the `ICoreWebView2` that `wry::WebViewExtWindows::webview()` hands
//! back. wry's own callbacks (`with_navigation_handler` and friends) are used
//! nowhere in this crate: they drop the fields under measurement —
//! `NavigationId`, `NavigationKind` — before the handler ever sees them.
//!
//! Handlers run on the UI thread while messages are dispatched, which is the same
//! thread the harness ticks on, so a record appended here is visible to the step
//! waiting for it without any cross-thread wait.

use std::sync::{Arc, Mutex, mpsc};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2NavigationStartingEventArgs,
    ICoreWebView2NavigationStartingEventArgs3, ICoreWebView2_2,
};
use webview2_com::{
    ContentLoadingEventHandler, DOMContentLoadedEventHandler, HistoryChangedEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler, SourceChangedEventHandler,
    WebMessageReceivedEventHandler,
};
use windows::core::{BOOL, Interface, PWSTR};

use crate::win;

/// One event as WebView2 reported it. No interpretation: `id` is the raw
/// `NavigationId`, or `None` when the event carries no such property, and the
/// distinguishing detail (success, navigation kind, frame-ness) goes in `extra`.
#[derive(Clone, Debug)]
pub struct NavRec {
    pub ms: f64,
    pub label: String,
    pub event: &'static str,
    pub id: Option<u64>,
    pub url: String,
    pub extra: String,
}

pub type Sink = Arc<Mutex<Vec<NavRec>>>;

/// A message a page posted through `window.chrome.webview.postMessage`.
///
/// This is WebView2's own page→host channel: one hop, no wry involvement, and the
/// page stamps `performance.now()` into the payload so a renderer clock and the
/// host clock can be related.
#[derive(Clone, Debug)]
pub struct PageMsg {
    /// Host QPC millisecond at the moment the message was received here.
    pub ms: f64,
    pub label: String,
    pub kind: String,
    pub json: serde_json::Value,
}

pub fn new_sink() -> Sink {
    Arc::new(Mutex::new(Vec::new()))
}

/// Append one record and echo it, keeping the raw record for the steps that
/// count or diff later.
pub fn record(
    sink: &Sink,
    label: &str,
    event: &'static str,
    id: Option<u64>,
    url: String,
    extra: String,
) {
    let ms = win::qpc_ms();
    match id {
        Some(v) => println!("[{ms:>11.3}ms] {label:<5} {event:<19} id={v:<12} {extra} {url}"),
        None => println!("[{ms:>11.3}ms] {label:<5} {event:<19} id=-             {extra} {url}"),
    }
    // A poisoned mutex here means a handler panicked; there is nothing left to
    // measure, so the panic propagates instead of being swallowed.
    sink.lock()
        .expect("nav sink mutex poisoned by a panicking handler")
        .push(NavRec {
            ms,
            label: label.to_string(),
            event,
            id,
            url,
            extra,
        });
}

/// Count records for one (webview, event) pair. Steps snapshot this before an
/// action and compare afterwards, which is how "did this produce a
/// NavigationStarting at all" gets answered by observation rather than by
/// inference.
pub fn count(sink: &Sink, label: &str, event: &str) -> usize {
    sink.lock()
        .expect("nav sink mutex poisoned")
        .iter()
        .filter(|r| r.label == label && r.event == event)
        .count()
}

/// The ids seen for one (webview, event) pair, in arrival order.
pub fn ids(sink: &Sink, label: &str, event: &str) -> Vec<Option<u64>> {
    sink.lock()
        .expect("nav sink mutex poisoned")
        .iter()
        .filter(|r| r.label == label && r.event == event)
        .map(|r| r.id)
        .collect()
}

/// Copy out the raw records; callers format them.
pub fn snapshot(sink: &Sink) -> Vec<NavRec> {
    sink.lock().expect("nav sink mutex poisoned").clone()
}

/// Read a `PWSTR` out-param and free it the way the WebView2 ABI requires.
pub fn read_pwstr(get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    // SAFETY: the callee allocates with `CoTaskMemAlloc`, so the string is
    // copied before `CoTaskMemFree` releases it, and the pointer is not used
    // again afterwards.
    unsafe {
        let mut p = PWSTR::null();
        if get(&mut p).is_err() || p.is_null() {
            return String::new();
        }
        let s = p.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s
    }
}

/// `NavigationId` from a navigation-starting argument object.
///
/// # Safety
/// `args` is the live event argument object handed to the callback.
unsafe fn nav_id(args: &ICoreWebView2NavigationStartingEventArgs) -> Option<u64> {
    let mut v = 0u64;
    args.NavigationId(&mut v).ok()?;
    Some(v)
}

/// Named form of `COREWEBVIEW2_NAVIGATION_KIND`.
///
/// The bindings generate a tuple struct, so `{:?}` would print a bare integer
/// and every reader of this report would have to look up the constant. The three
/// values are the complete set this SDK exposes.
fn nav_kind_name(k: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_NAVIGATION_KIND) -> &'static str {
    use webview2_com::Microsoft::Web::WebView2::Win32 as wv;
    if k == wv::COREWEBVIEW2_NAVIGATION_KIND_RELOAD {
        "RELOAD"
    } else if k == wv::COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD {
        "BACK_OR_FORWARD"
    } else if k == wv::COREWEBVIEW2_NAVIGATION_KIND_NEW_DOCUMENT {
        "NEW_DOCUMENT"
    } else {
        "OTHER"
    }
}

/// Attach every navigation event the probe reads.
///
/// Registration tokens are deliberately not retained: a handler registered here
/// is AddRef'd by the runtime, the probe never unhooks, and every webview lives
/// until the process exits.
///
/// # Safety
/// `webview` must be alive and this thread must own the COM apartment it was
/// created in.
pub unsafe fn attach_nav_events(label: &str, webview: &ICoreWebView2, sink: &Sink) -> Result<(), String> {
    // --- NavigationStarting: the property the identity model hinges on.
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = NavigationStartingEventHandler::create(Box::new(move |_s, args| {
            if let Some(a) = args {
                // SAFETY: `a` is the live argument object for this callback.
                let (id, url, extra) = unsafe {
                    let id = nav_id(&a);
                    let url = read_pwstr(|p| a.Uri(p));
                    // `NavigationKind` is on the ...3 interface in this SDK (the
                    // ...2 interface adds only
                    // AdditionalAllowedFrameAncestors). An older runtime would
                    // fail that cast, which is a missing field, not a lost
                    // measurement, so it degrades to `kind=n/a`.
                    let kind = match a.cast::<ICoreWebView2NavigationStartingEventArgs3>() {
                        Ok(a3) => {
                            let mut k = Default::default();
                            a3.NavigationKind(&mut k).ok();
                            format!("kind={}", nav_kind_name(k))
                        }
                        Err(_) => "kind=n/a".to_string(),
                    };
                    (id, url, kind)
                };
                record(&sink, &label, "NavigationStarting", id, url, extra);
            }
            Ok(())
        }));
        let mut token: i64 = 0;
        webview
            .add_NavigationStarting(&handler, &mut token)
            .map_err(|e| format!("add_NavigationStarting: {e}"))?;
    }

    // --- ContentLoading: same navigation, later in the pipeline.
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = ContentLoadingEventHandler::create(Box::new(move |_s, args| {
            if let Some(a) = args {
                // SAFETY: live argument object for this callback.
                let (id, extra) = unsafe {
                    let mut raw = 0u64;
                    let id = a.NavigationId(&mut raw).ok().map(|_| raw);
                    // This argument object carries exactly two properties in the
                    // bound SDK: NavigationId and IsErrorPage. (The .NET surface
                    // also exposes IsProgressing/CanGoBack; the C interface does
                    // not, so they are not printed.)
                    let mut error_page = BOOL(0);
                    a.IsErrorPage(&mut error_page).ok();
                    (id, format!("isErrorPage={}", error_page.as_bool()))
                };
                record(&sink, &label, "ContentLoading", id, String::new(), extra);
            }
            Ok(())
        }));
        let mut token: i64 = 0;
        webview
            .add_ContentLoading(&handler, &mut token)
            .map_err(|e| format!("add_ContentLoading: {e}"))?;
    }

    // --- NavigationCompleted: is the id the one NavigationStarting reported?
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = NavigationCompletedEventHandler::create(Box::new(move |_s, args| {
            if let Some(a) = args {
                // SAFETY: live argument object for this callback.
                let (id, extra) = unsafe {
                    let mut raw = 0u64;
                    let id = a.NavigationId(&mut raw).ok().map(|_| raw);
                    let mut ok = BOOL(0);
                    a.IsSuccess(&mut ok).ok();
                    // This argument object has no NavigationKind in the bound SDK
                    // (only the starting args' ...3 interface does), so the
                    // navigation's kind is read once, at NavigationStarting.
                    let mut status = Default::default();
                    a.WebErrorStatus(&mut status).ok();
                    (
                        id,
                        format!(
                            "success={} {status:?}",
                            ok.as_bool()
                        )
                    )
                };
                record(&sink, &label, "NavigationCompleted", id, String::new(), extra);
            }
            Ok(())
        }));
        let mut token: i64 = 0;
        webview
            .add_NavigationCompleted(&handler, &mut token)
            .map_err(|e| format!("add_NavigationCompleted: {e}"))?;
    }

    // --- SourceChanged: fires for pushState and fragment jumps and carries no
    // NavigationId at all. That absence is one of the answers V1 prints.
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = SourceChangedEventHandler::create(Box::new(move |_s, args| {
            if args.is_some() {
                record(
                    &sink,
                    &label,
                    "SourceChanged",
                    None,
                    String::new(),
                    "interface_has_no_NavigationId".to_string(),
                );
            }
            Ok(())
        }));
        let mut token: i64 = 0;
        webview
            .add_SourceChanged(&handler, &mut token)
            .map_err(|e| format!("add_SourceChanged: {e}"))?;
    }

    // --- HistoryChanged: fires when the back/forward list changes, including on
    // pushState. Also no NavigationId.
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = HistoryChangedEventHandler::create(Box::new(move |_s, _a| {
            record(
                &sink,
                &label,
                "HistoryChanged",
                None,
                String::new(),
                "interface_has_no_NavigationId".to_string(),
            );
            Ok(())
        }));
        let mut token: i64 = 0;
        webview
            .add_HistoryChanged(&handler, &mut token)
            .map_err(|e| format!("add_HistoryChanged: {e}"))?;
    }

    // --- FrameNavigationStarting: a subframe navigation reported on the parent
    // webview. The bound SDK registers the plain navigation-starting handler for
    // it (`add_FrameNavigationStarting` takes
    // `ICoreWebView2NavigationStartingEventHandler`), so its argument object is
    // the same type and does expose `NavigationId`. Whether that id shares the
    // top-level counter is a V1 measurement, not an assumption.
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = NavigationStartingEventHandler::create(Box::new(move |_s, args| {
            if let Some(a) = args {
                // SAFETY: live argument object for this callback.
                let (id, url) = unsafe { (nav_id(&a), read_pwstr(|p| a.Uri(p))) };
                record(&sink, &label, "FrameNavStarting", id, url, "subframe".to_string());
            }
            Ok(())
        }));
        let mut token: i64 = 0;
        webview
            .add_FrameNavigationStarting(&handler, &mut token)
            .map_err(|e| format!("add_FrameNavigationStarting: {e}"))?;
    }

    // --- DOMContentLoaded: third place the id appears. It lives on
    // ICoreWebView2_2, so an old runtime legitimately lacks it.
    {
        let (sink, label) = (sink.clone(), label.to_string());
        let handler = DOMContentLoadedEventHandler::create(Box::new(move |_s, args| {
            if let Some(a) = args {
                // SAFETY: live argument object for this callback.
                let id = unsafe {
                    let mut raw = 0u64;
                    a.NavigationId(&mut raw).ok().map(|_| raw)
                };
                record(&sink, &label, "DOMContentLoaded", id, String::new(), String::new());
            }
            Ok(())
        }));
        match webview.cast::<ICoreWebView2_2>() {
            Ok(wv2) => {
                let mut token: i64 = 0;
                wv2.add_DOMContentLoaded(&handler, &mut token)
                    .map_err(|e| format!("add_DOMContentLoaded: {e}"))?;
            }
            Err(e) => println!("NOTE: no ICoreWebView2_2, DOMContentLoaded not hooked: {e}"),
        }
    }

    Ok(())
}

/// Attach the page→host message channel.
///
/// # Safety
/// `webview` must be alive and this thread must own its COM apartment.
pub unsafe fn attach_page_messages(
    label: &str,
    webview: &ICoreWebView2,
    tx: mpsc::Sender<PageMsg>,
) -> Result<(), String> {
    let label = label.to_string();
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_s, args| {
        if let Some(a) = args {
            // SAFETY: live argument object; the JSON is copied out before the
            // buffer is freed.
            let json = unsafe { read_pwstr(|p| a.WebMessageAsJson(p)) };
            let parsed: serde_json::Value = match serde_json::from_str(&json) {
                // A page that calls `postMessage(JSON.stringify(obj))` arrives as a
                // JSON *string* in `WebMessageAsJson`, one level deeper than a page
                // that calls `postMessage(obj)`. Decode that level here so both
                // spellings land as an object and `kind` is never `"?"`.
                Ok(serde_json::Value::String(s)) => match serde_json::from_str(&s) {
                    Ok(v) => v,
                    Err(_) => serde_json::Value::String(s),
                },
                Ok(v) => v,
                Err(_) => serde_json::Value::String(json),
            };
            let kind = parsed
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let _ = tx.send(PageMsg {
                ms: win::qpc_ms(),
                label: label.clone(),
                kind,
                json: parsed,
            });
        }
        Ok(())
    }));
    let mut token: i64 = 0;
    webview
        .add_WebMessageReceived(&handler, &mut token)
        .map_err(|e| format!("add_WebMessageReceived: {e}"))?;
    Ok(())
}

/// Send a JSON message to a page; the page listens with
/// `window.chrome.webview.addEventListener("message", e => ...)`.
///
/// # Safety
/// `webview` must be alive.
pub unsafe fn post_to_page(webview: &ICoreWebView2, json: &str) -> Result<(), String> {
    let h = windows::core::HSTRING::from(json);
    webview
        .PostWebMessageAsJson(&h)
        .map_err(|e| format!("PostWebMessageAsJson: {e}"))
}
