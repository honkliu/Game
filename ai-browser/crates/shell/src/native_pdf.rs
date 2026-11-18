//! Read WebView2's native PDF accessibility provider on a non-UI COM apartment.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::{implement, Interface, Ref, BSTR};
use windows::Win32::Foundation::{E_FAIL, HWND, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use wry::WebViewExtWindows;

type Reply = Box<dyn FnOnce(String) + Send>;
mod selection;

pub enum SelectionUpdate {
    Changed(Option<[f64; 4]>),
    Clear,
}

type Selection = Box<dyn Fn(SelectionUpdate) + Send>;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static WORKER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();

enum Command {
    Open(u64, isize, String, String, Arc<AtomicBool>, Selection),
    Close(u64),
    Read(u64, Reply),
    Find(u64, String, i64, Reply),
    Reveal(u64, String),
    Selection(u64),
}

pub(crate) struct Session {
    pub id: u64,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::Sender<Command>,
    selection_reader: Rc<selection::Reader>,
    selection_request: Cell<u64>,
}

impl Session {
    pub fn new(
        webview: &wry::WebView,
        url: String,
        title: String,
        selection: impl Fn(u64, SelectionUpdate) + Send + 'static,
    ) -> Result<Self, String> {
        let mut hwnd = HWND::default();
        unsafe { webview.controller().ParentWindow(&mut hwnd) }
            .map_err(|error| error.to_string())?;
        let sender = WORKER
            .get_or_init(|| {
                let (sender, receiver) = mpsc::channel();
                let events = sender.clone();
                std::thread::spawn(move || worker(receiver, events));
                sender
            })
            .clone();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        let selection_reader = selection::Reader::new(webview.webview(), cancelled.clone());
        sender
            .send(Command::Open(
                id,
                hwnd.0 as isize,
                url,
                title,
                cancelled.clone(),
                Box::new(move |update| selection(id, update)),
            ))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            id,
            cancelled,
            sender,
            selection_reader,
            selection_request: Cell::new(0),
        })
    }

    pub fn invalidate_selection(&self) {
        self.selection_request
            .set(self.selection_request.get().wrapping_add(1));
    }

    pub fn selection_request(&self) -> u64 {
        self.selection_request.get()
    }

    pub fn read_selection(&self, done: impl FnOnce(u64, Result<String, String>) + 'static) {
        self.invalidate_selection();
        let request = self.selection_request.get();
        self.selection_reader
            .read(move |result| done(request, result));
    }

    pub fn fit_width(&self, done: impl FnOnce(Result<String, String>) + 'static) {
        self.selection_reader.fit_width(done);
    }

    pub fn read(&self, reply: impl FnOnce(String) + Send + 'static) -> Result<(), String> {
        self.sender
            .send(Command::Read(self.id, Box::new(reply)))
            .map_err(|error| error.to_string())
    }

    pub fn find(
        &self,
        query: String,
        index: i64,
        reply: impl FnOnce(String) + Send + 'static,
    ) -> Result<(), String> {
        self.sender
            .send(Command::Find(self.id, query, index, Box::new(reply)))
            .map_err(|error| error.to_string())
    }

    pub fn reveal(&self, text: String) -> Result<(), String> {
        if text.trim().is_empty() {
            return Err("The cited PDF passage is empty.".into());
        }
        self.sender
            .send(Command::Reveal(self.id, text))
            .map_err(|error| error.to_string())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let _ = self.sender.send(Command::Close(self.id));
    }
}

#[implement(IUIAutomationEventHandler)]
struct SelectionHandler {
    id: u64,
    sender: mpsc::Sender<Command>,
    pending: Arc<AtomicBool>,
}

impl IUIAutomationEventHandler_Impl for SelectionHandler_Impl {
    fn HandleAutomationEvent(
        &self,
        _sender: Ref<IUIAutomationElement>,
        _event: UIA_EVENT_ID,
    ) -> windows::core::Result<()> {
        if !self.pending.swap(true, Ordering::Relaxed) {
            let _ = self.sender.send(Command::Selection(self.id));
        }
        Ok(())
    }
}

struct Document {
    root: IUIAutomationElement,
    hwnd: isize,
    url: String,
    title: String,
    cancelled: Arc<AtomicBool>,
    selection: Selection,
    handler: IUIAutomationEventHandler,
    last_selection: String,
    query: String,
    matches: Vec<IUIAutomationTextRange>,
    highlighted: Option<IUIAutomationTextRange>,
    suppress_selection: Instant,
    pending_event: Arc<AtomicBool>,
}

impl Document {
    unsafe fn content(
        &self,
        automation: &IUIAutomation,
    ) -> windows::core::Result<IUIAutomationElement> {
        // A file PDF can also have an empty outer HTML document with no automation
        // id. Wait for the inner text provider instead of treating that as the PDF.
        let document = automation.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_DocumentControlTypeId.0),
        )?;
        let no_id =
            automation.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(""))?;
        let condition = automation.CreateAndCondition(&document, &no_id)?;
        let candidates = self.root.FindAll(TreeScope_Descendants, &condition)?;
        for i in 0..candidates.Length()? {
            let candidate = candidates.GetElement(i)?;
            let Ok(pattern) =
                candidate.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            else {
                continue;
            };
            if pattern
                .DocumentRange()
                .and_then(|range| range.GetText(1))
                .is_ok_and(|text| !text.is_empty())
            {
                return Ok(candidate);
            }
        }
        Err(windows::core::Error::new(
            E_FAIL,
            "The native PDF text provider is not ready.",
        ))
    }

    unsafe fn read(&self, automation: &IUIAutomation) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let content = loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err("The PDF document changed.".into());
            }
            match self.content(automation) {
                Ok(content) => break content,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(error) => return Err(format!("Native PDF text is not available yet: {error}")),
            }
        };
        let pattern: IUIAutomationTextPattern = content
            .GetCurrentPatternAs(UIA_TextPatternId)
            .map_err(|e| e.to_string())?;
        let raw = pattern
            .DocumentRange()
            .and_then(|range| range.GetText(120_002))
            .map_err(|e| e.to_string())?
            .to_string();
        let count = raw.chars().count();
        let text: String = raw.chars().take(60_000).collect();
        if self.cancelled.load(Ordering::Relaxed) {
            return Err("The PDF document changed.".into());
        }
        Ok(json!({
            "url": self.url, "title": self.title, "ready": "complete", "text": text,
            "description": "", "headings": [], "forms": [], "fields": [], "links": [],
            "coverage": { "source": "webview2_native_pdf", "textChars": count,
                "textTruncated": count > 60_000, "textCharsIsLowerBound": count > 60_000,
                "omittedControls": 0, "omittedLinks": 0, "unreadFrames": 0 }
        }))
    }

    unsafe fn find(
        &mut self,
        automation: &IUIAutomation,
        query: &str,
        index: i64,
    ) -> Result<Value, String> {
        self.suppress_selection = Instant::now() + Duration::from_millis(500);
        if let Some(previous) = self.highlighted.take() {
            let caret = previous.Clone().map_err(|e| e.to_string())?;
            caret
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    &caret,
                    TextPatternRangeEndpoint_Start,
                )
                .and_then(|_| caret.Select())
                .map_err(|e| e.to_string())?;
        }
        if query != self.query || self.matches.is_empty() {
            self.matches.clear();
            self.query = query.to_string();
            if !query.is_empty() {
                let content = self.content(automation).map_err(|e| e.to_string())?;
                let pattern: IUIAutomationTextPattern = content
                    .GetCurrentPatternAs(UIA_TextPatternId)
                    .map_err(|e| e.to_string())?;
                let remaining = pattern.DocumentRange().map_err(|e| e.to_string())?;
                let needle = BSTR::from(query);
                let deadline = Instant::now() + Duration::from_secs(2);
                while self.matches.len() < 10_000 && Instant::now() < deadline {
                    if self.cancelled.load(Ordering::Relaxed) {
                        return Err("The PDF document changed.".into());
                    }
                    let Ok(found) = remaining.FindText(&needle, false, true) else {
                        break;
                    };
                    if found
                        .CompareEndpoints(
                            TextPatternRangeEndpoint_Start,
                            &found,
                            TextPatternRangeEndpoint_End,
                        )
                        .unwrap_or(0)
                        == 0
                    {
                        break;
                    }
                    if remaining
                        .CompareEndpoints(
                            TextPatternRangeEndpoint_Start,
                            &found,
                            TextPatternRangeEndpoint_End,
                        )
                        .unwrap_or(0)
                        >= 0
                    {
                        break;
                    }
                    remaining
                        .MoveEndpointByRange(
                            TextPatternRangeEndpoint_Start,
                            &found,
                            TextPatternRangeEndpoint_End,
                        )
                        .map_err(|e| e.to_string())?;
                    self.matches.push(found);
                }
            }
        }
        self.suppress_selection = Instant::now() + Duration::from_millis(500);
        (self.selection)(SelectionUpdate::Clear);
        let total = self.matches.len();
        let active = if total == 0 {
            0
        } else {
            index.rem_euclid(total as i64) as usize + 1
        };
        if active > 0 {
            let range = &self.matches[active - 1];
            range.ScrollIntoView(true).map_err(|e| e.to_string())?;
            range.Select().map_err(|e| e.to_string())?;
            self.highlighted = Some(range.clone());
            self.last_selection = range.GetText(-1).map(|s| s.to_string()).unwrap_or_default();
        } else {
            self.last_selection.clear();
        }
        Ok(json!({ "total": total, "active": active, "supported_flags": [] }))
    }

    unsafe fn update_selection(&mut self, automation: &IUIAutomation) -> windows::core::Result<()> {
        if self.cancelled.load(Ordering::Relaxed) || Instant::now() < self.suppress_selection {
            return Ok(());
        }
        let content = self.content(automation)?;
        let condition = automation
            .CreatePropertyCondition(UIA_IsTextPatternAvailablePropertyId, &VARIANT::from(true))?;
        let elements = content.FindAll(TreeScope_Subtree, &condition)?;
        let mut text = String::new();
        let mut rect = None;
        for i in 0..elements.Length()?.min(4000) {
            let element = elements.GetElement(i)?;
            let pattern: IUIAutomationTextPattern =
                element.GetCurrentPatternAs(UIA_TextPatternId)?;
            let ranges = pattern.GetSelection()?;
            for j in 0..ranges.Length()?.min(100) {
                let range = ranges.GetElement(j)?;
                let part = range.GetText(-1)?.to_string();
                if part.is_empty() {
                    continue;
                }
                if rect.is_none() {
                    rect = selection_rect(&range, self.hwnd);
                }
                text.push_str(&part);
            }
            // Some providers report ranges on the document itself; others only
            // on its text leaves. Stop before counting both representations.
            if !text.is_empty() && element.CurrentControlType()? == UIA_DocumentControlTypeId {
                break;
            }
        }
        if text != self.last_selection && !self.cancelled.load(Ordering::Relaxed) {
            self.last_selection = text;
            (self.selection)(SelectionUpdate::Changed(rect));
        }
        Ok(())
    }
}

unsafe fn selection_rect(range: &IUIAutomationTextRange, hwnd: isize) -> Option<[f64; 4]> {
    let array = range.GetBoundingRectangles().ok()?;
    let result = (|| {
        let length = SafeArrayGetUBound(array, 1).ok()? + 1;
        if length < 4 {
            return None;
        }
        let mut data = std::ptr::null_mut();
        SafeArrayAccessData(array, &mut data).ok()?;
        let values = std::slice::from_raw_parts(data.cast::<f64>(), 4);
        let mut rect = [values[0], values[1], values[2], values[3]];
        let _ = SafeArrayUnaccessData(array);
        let hwnd = HWND(hwnd as _);
        let mut bounds = RECT::default();
        GetWindowRect(hwnd, &mut bounds).ok()?;
        let scale = (GetDpiForWindow(hwnd) as f64 / 96.0).max(1.0);
        rect[0] -= bounds.left as f64;
        rect[1] -= bounds.top as f64;
        for value in &mut rect {
            *value /= scale;
        }
        (rect.iter().all(|v| v.is_finite()) && rect[2] > 0.0 && rect[3] > 0.0).then_some(rect)
    })();
    let _ = SafeArrayDestroy(array);
    result
}

fn encoded(result: Result<Value, String>) -> String {
    // Match evaluate_script_with_callback: a JSON-encoded script string.
    let value = result.unwrap_or_else(|error| json!({ "error": error }));
    json!(value.to_string()).to_string()
}

fn worker(receiver: mpsc::Receiver<Command>, sender: mpsc::Sender<Command>) {
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return;
        }
        let Ok(automation): windows::core::Result<IUIAutomation> =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
        else {
            return;
        };
        if let Ok(settings) = automation.cast::<IUIAutomation2>() {
            let _ = settings.SetConnectionTimeout(1000);
            let _ = settings.SetTransactionTimeout(1000);
        }
        let mut documents: HashMap<u64, Document> = HashMap::new();
        let mut pending = HashSet::new();
        loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Command::Open(id, hwnd, url, title, cancelled, selection)) => {
                    if cancelled.load(Ordering::Relaxed) {
                        continue;
                    }
                    if let Ok(root) = automation.ElementFromHandle(HWND(hwnd as _)) {
                        let pending_event = Arc::new(AtomicBool::new(false));
                        let handler: IUIAutomationEventHandler = SelectionHandler {
                            id,
                            sender: sender.clone(),
                            pending: pending_event.clone(),
                        }
                        .into();
                        let _ = automation.AddAutomationEventHandler(
                            UIA_Text_TextSelectionChangedEventId,
                            &root,
                            TreeScope_Subtree,
                            None,
                            &handler,
                        );
                        documents.insert(
                            id,
                            Document {
                                root,
                                hwnd,
                                url,
                                title,
                                cancelled,
                                selection,
                                handler,
                                last_selection: String::new(),
                                query: String::new(),
                                matches: Vec::new(),
                                highlighted: None,
                                suppress_selection: Instant::now(),
                                pending_event,
                            },
                        );
                    }
                }
                Ok(Command::Close(id)) => {
                    if let Some(document) = documents.remove(&id) {
                        let _ = automation.RemoveAutomationEventHandler(
                            UIA_Text_TextSelectionChangedEventId,
                            &document.root,
                            &document.handler,
                        );
                    }
                    pending.remove(&id);
                }
                Ok(Command::Read(id, reply)) => {
                    reply(encoded(
                        documents
                            .get(&id)
                            .ok_or_else(|| "The PDF is no longer loaded.".to_string())
                            .and_then(|document| document.read(&automation)),
                    ));
                }
                Ok(Command::Find(id, query, index, reply)) => {
                    reply(encoded(
                        documents
                            .get_mut(&id)
                            .ok_or_else(|| "The PDF is no longer loaded.".to_string())
                            .and_then(|document| document.find(&automation, &query, index)),
                    ));
                }
                Ok(Command::Reveal(id, text)) => {
                    if let Some(document) = documents
                        .get(&id)
                        .filter(|d| !d.cancelled.load(Ordering::Relaxed))
                    {
                        let _ = document.content(&automation).and_then(|content| {
                            let pattern: IUIAutomationTextPattern =
                                content.GetCurrentPatternAs(UIA_TextPatternId)?;
                            pattern
                                .DocumentRange()?
                                .FindText(&BSTR::from(text), false, false)?
                                .ScrollIntoView(true)
                        });
                    }
                }
                Ok(Command::Selection(id)) => {
                    pending.insert(id);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            // Do not place River's toolbar under an in-progress selection drag.
            if GetAsyncKeyState(VK_LBUTTON.0 as i32) >= 0 {
                let ready: Vec<_> = pending
                    .iter()
                    .copied()
                    .filter(|id| {
                        documents
                            .get(id)
                            .is_none_or(|document| Instant::now() >= document.suppress_selection)
                    })
                    .collect();
                for id in ready {
                    pending.remove(&id);
                    if let Some(document) = documents.get_mut(&id) {
                        document.pending_event.store(false, Ordering::Relaxed);
                        let _ = document.update_selection(&automation);
                    }
                }
            }
        }
    }
}

pub(crate) fn snapshot(raw: &str) -> Result<String, String> {
    let inner: String = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let content: Value = serde_json::from_str(&inner).map_err(|e| e.to_string())?;
    if let Some(error) = content["error"].as_str() {
        return Err(error.to_string());
    }
    let text = content["text"].as_str().unwrap_or("");
    let lines: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    Ok(json!({
        "strings": lines,
        "documents": [{
            "nodes": { "nodeType": vec![3; lines.len()], "nodeName": vec![-1; lines.len()] },
            "layout": { "nodeIndex": (0..lines.len()).collect::<Vec<_>>(),
                "text": (0..lines.len()).collect::<Vec<_>>(), "bounds": vec![[0, 0, 0, 0]; lines.len()] }
        }],
        "droppedBlocks": usize::from(content["coverage"]["textTruncated"] == true)
    }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_platform_contract::identity::{DocumentToken, SnapshotVersion};

    #[test]
    fn native_text_becomes_citable_blocks_without_invented_geometry() {
        let text = "First PDF paragraph contains readable document text.\n\nSecond PDF paragraph belongs to the next page.";
        let payload = snapshot(&encoded(Ok(
            json!({ "text": text, "coverage": { "textTruncated": true } }),
        )))
        .unwrap();
        let context = crate::ai::context::parse(
            &serde_json::from_str(&payload).unwrap(),
            DocumentToken(1),
            SnapshotVersion(1),
            1,
            "file:///C:/document.pdf",
            "Document",
        )
        .unwrap();
        assert_eq!(context.blocks.len(), 2);
        assert_eq!(
            context.blocks[1].text,
            "Second PDF paragraph belongs to the next page."
        );
        assert!(context
            .blocks
            .iter()
            .all(|block| block.h == 0.0 && block.y == 0.0));
        assert_eq!(context.dropped_blocks, 1);
    }

    #[test]
    fn unavailable_native_text_is_not_an_empty_successful_snapshot() {
        assert_eq!(
            snapshot(&encoded(Err("PDF navigation cancelled".into()))).unwrap_err(),
            "PDF navigation cancelled"
        );
    }
}
