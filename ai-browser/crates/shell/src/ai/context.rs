//! What the page says, read without touching the page.
//!
//! One CDP method is used: `DOMSnapshot.captureSnapshot`. It is read-only, it is
//! the only DevTools domain this build calls, and the measured cost on this
//! runtime is 2.2 ms for a real page (tools/wv2-probe/RESULTS.md), which is why
//! the composer can afford to capture on every question rather than cache a
//! guess. Nothing here mutates a page, evaluates script, or opens an execution
//! context — extraction is the whole reason the bridge is allowed at all (see the
//! read-only policy note in `ai::mod`'s "deliberately absent" prose).
//!
//! ## The shape the runtime actually returns
//!
//! `DOMSnapshot.captureSnapshot` on the shipping WebView2 runtime returns the
//! *legacy columnar* payload, not the `nodeTreeSnapshot`/`layoutTreeSnapshot`
//! shape current CDP documentation describes:
//!
//! ```text
//! { "strings": ["…", "…"],
//!   "documents": [ {
//!       "nodes": { "nodeType": [1,3,…], "nodeName": [0,2,…], "nodeValue": [-1,…],
//!                  "parentIndex": [-1,0,…], "backendNodeId": [1,2,…] },
//!       "layout": { "nodeIndex": [0,4,…], "text": [7,-1,…],
//!                   "bounds": [[x,y,w,h], …],
//!                   "textBoxes": [{"layoutIndex":0,"start":0,"length":12}, …] } } ] }
//! ```
//!
//! Every string is an index into `strings`, and `-1` is that table's "no string"
//! sentinel. This was measured, not read from documentation: an analyzer written
//! against the modern shape reports 0 nodes against a payload that contains 35
//! (tools/wv2-probe/src/v4.rs). So a payload carrying `nodeTreeSnapshot` is
//! refused with `context.snapshot_shape_unsupported` rather than parsed into an
//! empty block list — an empty context would answer "the page says nothing",
//! which is a wrong answer rather than a missing one.
//!
//! ## What a block is
//!
//! A block is one citable passage: a block-level text element (or a bare text
//! node) with the rect it occupies in document coordinates. Containers are
//! deliberately not blocks — a `<div>` that holds three `<p>`s would otherwise be
//! cited *and* its children cited, and the citation's quote would be a fragment
//! of a larger quoted string. Reading order is `(y, x)`, so `b0` is the first
//! passage a reader meets.
//!
//! Block ids are stable only within one snapshot. That is not a shortcut: a
//! citation already carries its `snapshot_version`, and [`super::answer`] refuses
//! a citation whose document has moved on, so an id never has to outlive the
//! capture that minted it.

use serde_json::Value;
#[cfg(windows)]
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;
#[cfg(windows)]
use windows::core::PCWSTR;

use ai_platform_contract::failure::Failure;
use ai_platform_contract::identity::{DocumentToken, SnapshotVersion};

/// The only CDP method this crate calls.
pub const SNAPSHOT_METHOD: &str = "DOMSnapshot.captureSnapshot";

/// `computedStyles: []` asks for no computed styles, which is what keeps the
/// payload small; extraction needs text and geometry, and nothing else.
pub const SNAPSHOT_PARAMS: &str = r#"{"computedStyles":[]}"#;

/// Shorter than this is navigation chrome, not a passage worth citing.
const MIN_BLOCK_CHARS: usize = 24;

/// Blocks past this are dropped, oldest-last: a 20k-block page is a page the user
/// is not going to scroll to with a chip, and an unbounded payload here is
/// unbounded memory in the shell process. The bound is reported, not hidden —
/// see [`PageContext::dropped_blocks`].
const MAX_BLOCKS: usize = 4_000;

/// How far above the target rect the scroll lands, so the quoted passage is not
/// flush against the omnibox. Public because [`super::answer`] produces the same
/// offset for a citation click and the two must not drift apart.
pub const SCROLL_MARGIN: f64 = 24.0;

/// Elements whose own text is a passage. Anything not listed is either a
/// container (whose children are the passages), or non-text.
const BLOCK_TAGS: [&str; 16] = [
    "P", "LI", "DD", "DT", "TD", "TH", "H1", "H2", "H3", "H4", "H5", "H6",
    "BLOCKQUOTE", "PRE", "FIGCAPTION", "CAPTION",
];

/// Elements that are never a passage even when they hold text.
const SKIP_TAGS: [&str; 7] = ["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "HEAD", "TITLE", "META"];

/// One citable passage of the page.
#[derive(Debug, Clone)]
pub struct Block {
    /// Snapshot-local id, `b0`, `b1`, … in reading order.
    pub block_id: String,
    pub text: String,
    /// Document-space rect, CSS pixels: the same space `window.scrollTo` takes.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Block {
    /// Where `scroll_to` should put this passage.
    pub fn scroll_y(&self) -> f64 {
        (self.y - SCROLL_MARGIN).max(0.0).round()
    }
}

/// One link of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Snapshot-local id, `l0`, `l1`, … in document order.
    pub link_id: String,
    pub text: String,
    /// Absolute `http(s)` URL.
    pub url: String,
}

/// Links past this are dropped; a page with more is a directory, not a document.
const MAX_LINKS: usize = 300;
const MAX_LINK_TEXT_CHARS: usize = 160;

/// A capture, with the identity it was taken of.
#[derive(Debug, Clone)]
pub struct PageContext {
    pub document: DocumentToken,
    pub snapshot_version: SnapshotVersion,
    /// Kept beside the token so a citation can carry the number it must be checked
    /// against without resolving the key again on the click path.
    pub navigation_sequence: u64,
    pub url: String,
    pub title: String,
    pub blocks: Vec<Block>,
    /// Followable `http(s)` links, in document order. Page-provenance: the text and
    /// URL are the page's claim, and the AI may only open one by `link_id`.
    pub links: Vec<Link>,
    /// Sub-frames in the payload that were not parsed. The root frame's text is
    /// `documents[0]`; an out-of-process iframe arrives as another document and
    /// this build does not follow it, which the UI is told rather than hides.
    pub frames_not_read: usize,
    /// Blocks over [`MAX_BLOCKS`] that were dropped.
    pub dropped_blocks: usize,
}

impl PageContext {
    pub fn find(&self, block_id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.block_id == block_id)
    }

    /// Everything the provider is allowed to see, as one string. Bounded so a
    /// 4 MB page cannot become a 4 MB prompt; the bound is the point at which the
    /// answer gets an explicit "the page was longer than this" note.
    pub fn prompt_text(&self, max_chars: usize) -> (String, bool) {
        let mut out = String::new();
        let mut truncated = false;
        for b in &self.blocks {
            if out.len() + b.text.len() > max_chars {
                truncated = true;
                break;
            }
            out.push_str(&b.block_id);
            out.push(' ');
            out.push_str(&b.text);
            out.push('\n');
        }
        (out, truncated)
    }
}

/// Captures kept for the turns that cited them, keyed by
/// `(document token, snapshot version)`.
///
/// Bounded, because a long browsing session asks many questions and a stale
/// capture can never be cited again: the answer that used it is refused once the
/// document is gone, so keeping the payload is keeping dead weight.
#[derive(Default)]
pub struct ContextStore {
    contexts: std::collections::HashMap<(u64, u64), PageContext>,
    order: std::collections::VecDeque<(u64, u64)>,
}

/// Captures retained at once. Ten covers a question, its follow-ups, and the
/// tabs a user flips between while reading.
const MAX_CAPTURES: usize = 10;

impl ContextStore {
    pub fn put(&mut self, context: PageContext) {
        let key = (context.document.0, context.snapshot_version.0);
        if self.contexts.insert(key, context).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_CAPTURES {
            match self.order.pop_front() {
                Some(stale) => {
                    self.contexts.remove(&stale);
                }
                None => break,
            }
        }
    }

    pub fn get(&self, document: DocumentToken, version: SnapshotVersion) -> Option<&PageContext> {
        self.contexts.get(&(document.0, version.0))
    }

    /// The newest capture of a document, for the case where the caller has the
    /// token but not the version it captured at.
    pub fn newest(&self, document: DocumentToken) -> Option<&PageContext> {
        self.contexts
            .values()
            .filter(|c| c.document == document)
            .max_by_key(|c| c.snapshot_version.0)
    }

    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    /// Required by the clippy lint that pairs with `len`; this store is genuinely
    /// allowed to be empty.
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }
}

/// Ask the page's DevTools bridge for one snapshot.
///
/// `on_done` runs on the UI thread (the WebView2 completion handler is invoked in
/// the apartment that made the call), so it can send straight into the event
/// loop. It is *the* acknowledgement: the synchronous return value only means the
/// bridge accepted the call — measured, a method name invented on the spot
/// returned `S_OK` synchronously and `0x80070057` in the callback. Nothing may
/// treat `Ok(())` here as "the snapshot arrived".
pub fn capture(
    webview: &wry::WebView,
    on_done: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<(), Failure> {
    #[cfg(windows)]
    {
        use wry::WebViewExtWindows;
        capture_webview2(&webview.webview(), on_done)
    }
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::capture(webview, include_str!("snapshot.js"), on_done);
        Ok(())
    }
}

#[cfg(windows)]
fn capture_webview2(
    webview: &ICoreWebView2,
    on_done: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<(), Failure> {
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
        move |result, json| {
            // The callback's own HRESULT is the only real verdict, and its error
            // text is kept: a page that is still loading fails here with a code
            // the log can name.
            let outcome = match result {
                Ok(()) => Ok(json),
                Err(error) => Err(error.to_string()),
            };
            let _ = on_done(outcome);
            Ok(())
        },
    ));
    // Both arguments are `PCWSTR`, so the UTF-16 buffers are built here and these
    // locals keep them alive across the call.
    let method: Vec<u16> = SNAPSHOT_METHOD.encode_utf16().chain(std::iter::once(0)).collect();
    let params: Vec<u16> = SNAPSHOT_PARAMS.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `method` and `params` are nul-terminated UTF-16 owned by these
    // locals until the call returns; `handler` is a live COM object; the webview
    // belongs to the calling thread's apartment, which is the UI thread.
    let called = unsafe { webview.CallDevToolsProtocolMethod(PCWSTR(method.as_ptr()), PCWSTR(params.as_ptr()), &handler) };
    called.map_err(|error| {
        Failure::unavailable("context.cdp_call_failed")
            .with_detail(format!("{error} (HRESULT {:#010x})", error.code().0 as u32))
    })
}

/// Parse a `DOMSnapshot.captureSnapshot` payload into citable blocks.
///
/// See the module docs for the payload shape. `url` and `title` come from the
/// shell's own tab state rather than from the payload, because the shell's value
/// is the one the address bar shows and the user can check.
pub fn parse(
    payload: &Value,
    document: DocumentToken,
    snapshot_version: SnapshotVersion,
    navigation_sequence: u64,
    url: &str,
    title: &str,
) -> Result<PageContext, Failure> {
    if payload.get("nodeTreeSnapshot").is_some() || payload.get("layoutTreeSnapshot").is_some() {
        return Err(Failure::unsupported("context.snapshot_shape_unsupported").with_detail(
            "the runtime returned the modern DOMSnapshot shape; this parser reads the \
             legacy columnar one it was measured against",
        ));
    }
    let Some(documents) = payload.get("documents").and_then(Value::as_array) else {
        return Err(Failure::unavailable("context.snapshot_unreadable")
            .with_detail("no `documents` array in the payload"));
    };
    let Some(root) = documents.first() else {
        // An empty document array is a real state — a page with no document yet —
        // and it must not be reported as a parse failure the user could retry.
        return Err(Failure::unavailable("context.document_absent"));
    };

    let nodes = root.get("nodes");
    let strings = payload
        .get("strings")
        .and_then(Value::as_array)
        .ok_or_else(|| Failure::unavailable("context.snapshot_unreadable")
            .with_detail("no `strings` table in the payload"))?;
    let layout = root
        .get("layout")
        .and_then(Value::as_object)
        .ok_or_else(|| Failure::unavailable("context.snapshot_unreadable")
            .with_detail("no `layout` in documents[0]"))?;
    let column = |name: &str| -> Option<&Vec<Value>> {
        layout.get(name).and_then(Value::as_array)
    };
    let node_index = column("nodeIndex").ok_or_else(|| {
        Failure::unavailable("context.snapshot_unreadable").with_detail("no layout.nodeIndex")
    })?;
    let layout_text = column("text").ok_or_else(|| {
        Failure::unavailable("context.snapshot_unreadable").with_detail("no layout.text")
    })?;
    let layout_bounds = column("bounds").ok_or_else(|| {
        Failure::unavailable("context.snapshot_unreadable").with_detail("no layout.bounds")
    })?;
    let node_type = nodes
        .as_ref()
        .and_then(|n| n.get("nodeType"))
        .and_then(Value::as_array);
    let node_name = nodes
        .as_ref()
        .and_then(|n| n.get("nodeName"))
        .and_then(Value::as_array);

    let mut blocks: Vec<Block> = Vec::new();
    let mut dropped = payload.get("droppedBlocks").and_then(Value::as_u64).unwrap_or(0) as usize;
    for i in 0..node_index.len() {
        // The columns are parallel, so a short column means the payload is not the
        // shape claimed. That is a parse failure, not a page with less text.
        let Some(node_at) = node_index[i].as_i64() else { continue };
        let Some(text) = string_at(strings, layout_text.get(i).and_then(Value::as_i64)) else {
            return Err(Failure::unavailable("context.snapshot_unreadable")
                .with_detail(format!("layout.text[{i}] is not a string index")));
        };
        let text = text.trim();
        if text.chars().count() < MIN_BLOCK_CHARS {
            continue;
        }
        let name = node_name
            .and_then(|names| names.get(node_at as usize))
            .and_then(Value::as_i64)
            .and_then(|name_index| {
                strings.get(name_index as usize).and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_string();
        if SKIP_TAGS.iter().any(|skip| name.eq_ignore_ascii_case(skip)) {
            continue;
        }
        let kind = node_type
            .and_then(|types| types.get(node_at as usize))
            .and_then(Value::as_i64);
        // Node type 3 is a text node; anything else has to be an element on the
        // block list to count as a passage.
        let citable = kind == Some(3) || BLOCK_TAGS.iter().any(|tag| name.eq_ignore_ascii_case(tag));
        if !citable {
            continue;
        }
        let Some(bounds) = layout_bounds.get(i).and_then(Value::as_array) else {
            continue;
        };
        let mut rect = [0.0f64; 4];
        for (slot, value) in rect.iter_mut().zip(bounds) {
            *slot = value.as_f64().unwrap_or(0.0);
        }
        if blocks.len() >= MAX_BLOCKS {
            dropped += 1;
            continue;
        }
        blocks.push(Block {
            block_id: String::new(),
            text: text.to_string(),
            x: rect[0],
            y: rect[1],
            w: rect[2],
            h: rect[3],
        });
    }

    // Reading order last, so the ids mean "first passage a reader meets" rather
    // than "first row of the payload's column array".
    blocks.sort_by(|a, b| {
        a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x))
    });
    for (index, block) in blocks.iter_mut().enumerate() {
        block.block_id = format!("b{index}");
    }

    let links = extract_links(strings, nodes, url);

    Ok(PageContext {
        document,
        snapshot_version,
        navigation_sequence,
        url: url.to_string(),
        title: title.to_string(),
        blocks,
        links,
        frames_not_read: documents.len().saturating_sub(1),
        dropped_blocks: dropped,
    })
}

/// Followable links from the node table: `<a href>` elements with their text.
///
/// Resolved against `base`; anything that is not `http(s)` after resolution
/// (`javascript:`, `mailto:`, `data:`, fragments of the same page) is dropped, so a
/// link the AI is shown is a link the browser would actually navigate to.
fn extract_links(strings: &[Value], nodes: Option<&Value>, base: &str) -> Vec<Link> {
    let Some(nodes) = nodes else { return Vec::new() };
    let column = |name: &str| nodes.get(name).and_then(Value::as_array);
    let (Some(names), Some(parents), Some(attributes)) =
        (column("nodeName"), column("parentIndex"), column("attributes"))
    else {
        return Vec::new();
    };
    let types = column("nodeType");
    let values = column("nodeValue");
    let string = |index: Option<i64>| -> Option<&str> {
        let i = index?;
        if i < 0 { None } else { strings.get(i as usize).and_then(Value::as_str) }
    };

    let mut anchors: Vec<(usize, String)> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if !string(name.as_i64()).is_some_and(|n| n.eq_ignore_ascii_case("A")) {
            continue;
        }
        let Some(pairs) = attributes.get(i).and_then(Value::as_array) else { continue };
        let mut href = None;
        for pair in pairs.chunks(2) {
            if string(pair.first().and_then(Value::as_i64)).is_some_and(|k| k.eq_ignore_ascii_case("href")) {
                href = string(pair.get(1).and_then(Value::as_i64));
            }
        }
        if let Some(url) = href.and_then(|h| resolve_http(base, h)) {
            anchors.push((i, url));
        }
    }

    let mut texts: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    if let (Some(types), Some(values)) = (types, values) {
        let anchor_set: std::collections::HashSet<usize> = anchors.iter().map(|(i, _)| *i).collect();
        for (j, kind) in types.iter().enumerate() {
            if kind.as_i64() != Some(3) {
                continue;
            }
            let Some(text) = string(values.get(j).and_then(Value::as_i64)) else { continue };
            let mut at = parents.get(j).and_then(Value::as_i64).unwrap_or(-1);
            for _ in 0..8 {
                if at < 0 {
                    break;
                }
                if anchor_set.contains(&(at as usize)) {
                    let entry = texts.entry(at as usize).or_default();
                    if !entry.is_empty() {
                        entry.push(' ');
                    }
                    entry.push_str(text.trim());
                    break;
                }
                at = parents.get(at as usize).and_then(Value::as_i64).unwrap_or(-1);
            }
        }
    }

    let mut links = Vec::new();
    for (index, url) in anchors {
        let text: String = texts.get(&index).map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        if links.len() >= MAX_LINKS {
            break;
        }
        links.push(Link {
            link_id: format!("l{}", links.len()),
            text: text.chars().take(MAX_LINK_TEXT_CHARS).collect(),
            url,
        });
    }
    links
}

/// `href` resolved against `base`, or `None` when the result is not `http(s)`.
/// Deliberately small: the cases pages actually use (absolute, protocol-relative,
/// root-relative, path-relative). Anything it cannot place is refused, not guessed.
pub(crate) fn resolve_http(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(href.to_string());
    }
    let scheme_end = base.find("://")?;
    let scheme = &base[..scheme_end];
    if scheme != "http" && scheme != "https" {
        return None;
    }
    if href.starts_with("//") {
        return Some(format!("{scheme}:{href}"));
    }
    if href.contains(':') && href.find(':') < href.find('/').or(Some(usize::MAX)) {
        // `javascript:`, `mailto:`, `tel:`, `data:` — a scheme that is not http(s).
        return None;
    }
    let after = &base[scheme_end + 3..];
    let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let origin = &base[..scheme_end + 3 + host_end];
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let path = &after[host_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    Some(format!("{origin}{dir}{href}"))
}

/// `strings[i]` as text. `-1` is the payload's "no string here" sentinel, and an
/// index past the end is an error the caller sees rather than an empty passage.
fn string_at(strings: &[Value], index: Option<i64>) -> Option<String> {
    match index? {
        i if i < 0 => Some(String::new()),
        i => match strings.get(i as usize).and_then(Value::as_str) {
            Some(text) => Some(text.to_string()),
            None => None,
        },
    }
}
