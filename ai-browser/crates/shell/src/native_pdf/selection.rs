//! Authoritative selected text from the native engine, not UIA character offsets.
//! The PDF extension's target metadata and message endpoint are runtime-private:
//! feature-detect them and fail closed instead of returning inaccurate UIA text.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2_11};
use windows::core::{Interface, PCWSTR};

type Reply = Box<dyn FnOnce(Result<String, String>)>;
type ProtocolReply = Box<dyn FnOnce(Result<Value, String>)>;

pub(super) struct Reader {
    core: ICoreWebView2,
    cancelled: Arc<AtomicBool>,
    busy: Cell<bool>,
    pending: RefCell<Option<Reply>>,
}

impl Reader {
    pub fn new(core: ICoreWebView2, cancelled: Arc<AtomicBool>) -> Rc<Self> {
        Rc::new(Self {
            core,
            cancelled,
            busy: Cell::new(false),
            pending: RefCell::new(None),
        })
    }

    pub fn read(self: &Rc<Self>, done: impl FnOnce(Result<String, String>) + 'static) {
        let done: Reply = Box::new(done);
        // The native protocol does not echo request IDs. Serialize reads and
        // retain only the newest pending request, rather than misattribute replies.
        if self.busy.replace(true) {
            *self.pending.borrow_mut() = Some(done);
            return;
        }
        Read {
            reader: self.clone(),
            done,
            session: None,
        }
        .call(
            "Target.getTargetInfo",
            json!({}),
            |read, value| match value["targetInfo"]["targetId"].as_str() {
                Some(root) => {
                    let root = root.to_string();
                    read.call(
                        "Target.getTargets",
                        json!({}),
                        move |read, value| match pdf_target(&value, &root) {
                            Ok(target) => read.call(
                                "Target.attachToTarget",
                                json!({ "targetId": target, "flatten": true }),
                                |mut read, value| {
                                    let Some(session) = value["sessionId"].as_str() else {
                                        read.finish(Err("Missing native PDF session.".into()));
                                        return;
                                    };
                                    read.session = Some(session.to_string());
                                    read.call(
                                        "Runtime.evaluate",
                                        json!({
                                            "expression": include_str!("selection.js"),
                                            "returnByValue": true,
                                            "awaitPromise": true,
                                        }),
                                        |read, value| read.finish(selected_text(&value)),
                                    );
                                },
                            ),
                            Err(error) => read.finish(Err(error)),
                        },
                    );
                }
                None => read.finish(Err("Missing PDF WebView identity.".into())),
            },
        );
    }
}

struct Read {
    reader: Rc<Reader>,
    done: Reply,
    session: Option<String>,
}

impl Read {
    fn call(self, method: &str, params: Value, next: impl FnOnce(Self, Value) + 'static) {
        if self.reader.cancelled.load(Ordering::Relaxed) {
            self.finish(Err("The PDF is no longer loaded.".into()));
            return;
        }
        let core = self.reader.core.clone();
        let session = self.session.clone();
        protocol(
            &core,
            session.as_deref(),
            method,
            params,
            move |result| match result {
                Ok(value) => next(self, value),
                Err(error) => self.finish(Err(error)),
            },
        );
    }

    fn finish(self, result: Result<String, String>) {
        if let Some(session) = self.session {
            protocol(
                &self.reader.core,
                None,
                "Target.detachFromTarget",
                json!({ "sessionId": session }),
                |_| {},
            );
        }
        self.reader.busy.set(false);
        (self.done)(result);
        let pending = self.reader.pending.borrow_mut().take();
        if let Some(done) = pending {
            self.reader.read(done);
        }
    }
}

fn protocol(
    core: &ICoreWebView2,
    session: Option<&str>,
    method: &str,
    params: Value,
    done: impl FnOnce(Result<Value, String>) + 'static,
) {
    let done: Rc<RefCell<Option<ProtocolReply>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let callback = done.clone();
    let handler =
        CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, text| {
            if let Some(done) = callback.borrow_mut().take() {
                done(
                    result.map_err(|error| error.to_string()).and_then(|_| {
                        serde_json::from_str(&text).map_err(|error| error.to_string())
                    }),
                );
            }
            Ok(())
        }));
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let method = wide(method);
    let params = wide(&params.to_string());
    let called = unsafe {
        if let Some(session) = session {
            let session = wide(session);
            core.cast::<ICoreWebView2_11>().and_then(|core| {
                core.CallDevToolsProtocolMethodForSession(
                    PCWSTR(session.as_ptr()),
                    PCWSTR(method.as_ptr()),
                    PCWSTR(params.as_ptr()),
                    &handler,
                )
            })
        } else {
            core.CallDevToolsProtocolMethod(
                PCWSTR(method.as_ptr()),
                PCWSTR(params.as_ptr()),
                &handler,
            )
        }
    };
    if let Err(error) = called {
        if let Some(done) = done.borrow_mut().take() {
            done(Err(error.to_string()));
        }
    }
}

fn pdf_target(value: &Value, root: &str) -> Result<String, String> {
    let targets = value["targetInfos"]
        .as_array()
        .ok_or("Missing PDF target list.")?;
    let mut matching = targets.iter().filter(|target| {
        target["parentId"].as_str() == Some(root)
            && target["type"] == "webview"
            && target["url"]
                == "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/edge_pdf/index.html"
    });
    let target = matching
        .next()
        .and_then(|target| target["targetId"].as_str())
        .filter(|id| !id.is_empty())
        .ok_or("Native PDF selection target unavailable.")?;
    if matching.next().is_some() {
        return Err("Ambiguous native PDF selection target.".into());
    }
    Ok(target.to_string())
}

fn selected_text(value: &Value) -> Result<String, String> {
    if value.get("exceptionDetails").is_some() {
        return Err("Native PDF selection protocol unavailable or timed out.".into());
    }
    value["result"]["value"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Invalid native PDF selection result.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_target_is_owned_by_the_specific_webview_not_the_document_url() {
        let value = json!({ "targetInfos": [
            { "targetId": "other", "parentId": "right", "type": "webview",
              "url": "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/edge_pdf/index.html" },
            { "targetId": "spoof", "parentId": "left", "type": "webview",
              "url": "https://example.com/edge_pdf/index.html" },
            { "targetId": "own", "parentId": "left", "type": "webview",
              "url": "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/edge_pdf/index.html" }
        ] });
        assert_eq!(pdf_target(&value, "left").unwrap(), "own");
        assert_eq!(pdf_target(&value, "right").unwrap(), "other");
        assert!(pdf_target(&value, "missing").is_err());
        let duplicate =
            json!({ "targetInfos": [value["targetInfos"][2], value["targetInfos"][2]] });
        assert!(pdf_target(&duplicate, "left").is_err());
    }

    #[test]
    fn native_selection_preserves_partial_multiline_text_and_rejects_failures() {
        let text = "made model workloads\nincreasingly input heavy \u{1f4d6}";
        assert_eq!(
            selected_text(&json!({ "result": { "value": text } })).unwrap(),
            text
        );
        assert_eq!(
            selected_text(&json!({ "result": { "value": "" } })).unwrap(),
            ""
        );
        assert!(selected_text(&json!({ "result": { "type": "undefined" } })).is_err());
        assert!(selected_text(&json!({
            "exceptionDetails": { "text": "Protocol unavailable" },
            "result": { "value": "Do not use a partial or stale result" }
        }))
        .is_err());
        let long = "\u{1f4d6}".repeat(4001);
        assert_eq!(
            selected_text(&json!({ "result": { "value": &long } })).unwrap(),
            long,
        );
    }
}
