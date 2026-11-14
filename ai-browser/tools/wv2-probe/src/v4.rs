//! Does the DevTools-Protocol bridge actually answer at runtime.
//!
//! `CallDevToolsProtocolMethod` and `GetDevToolsProtocolEventReceiver` are on the
//! *base* `ICoreWebView2` in webview2-com-sys 0.39.1 (`ICoreWebView2_Vtbl` starts
//! at bindings.rs:1771, the two slots are 1897 and 1915), and the safe wrapper is
//! bindings.rs:1455. Symbol presence is not capability: nothing in this program
//! had ever made the call, and the read-only-extraction ruling rests on whether
//! the runtime answers it. So this mode makes the call and prints whatever came
//! back — the payload summary, or the exact HRESULT.
//!
//! Two methods, chosen because they are the two shapes extraction needs:
//!
//! * `DOMSnapshot.captureSnapshot` — the DOM plus a layout object per node, so
//!   text and geometry can come back in one round trip.
//! * `Accessibility.getFullAXTree` — the computed accessibility tree, which is the
//!   tier-2 path for content a page script could forge or hide.
//!
//! Plus a control: a method name no CDP domain has ever had, so a blanket failure
//! is distinguishable from one method being unavailable.
//!
//! Nothing here opens DevTools, enables a debugging port, or passes a browser
//! argument that wry does not pass anyway — the environment is built with
//! `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` and nothing
//! else (`env.rs:36`). If the calls work under those conditions the bridge is on
//! by default; if they do not, the ruling that permitted CDP is moot.
//!
//! Both calls are queries, and the parameters handed to each are a literal printed
//! in this run's output, so nothing in this file can modify a page.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::Value;
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use windows::core::{BOOL, PCWSTR};

use crate::harness::{build_step, expect_record, mark, sleep_ms, step, Step, Tick};
use crate::win;

const PAGE_VIEW: usize = 0;

/// How long to wait for the completion callback before calling the method dead.
/// `getFullAXTree` on a one-paragraph page answers in tens of ms, so this is not a
/// budget the measurement is expected to approach; if it is ever hit, that is the
/// answer, and it is printed as such.
const CALL_TIMEOUT_MS: f64 = 20_000.0;

/// Payloads up to this size are printed whole. `DOMSnapshot.captureSnapshot` on
/// example.com came back at 5.8 KB and `getFullAXTree` at 15.5 KB, so a page this
/// size is fully on the record; anything bigger gets the 400-character head plus
/// the structural lines, because a megabyte of JSON in a measurement log is
/// unreadable and tells the reader less than the counts do.
const RAW_LIMIT: usize = 64 * 1024;

/// How many laid-out text runs are printed verbatim with their rects. Enough to
/// show the join is real without filling the log with a page.
const TEXT_SAMPLES: usize = 3;

/// One method as the runtime answered it.
#[derive(Clone, Debug)]
struct Row {
    method: &'static str,
    /// What the synchronous `CallDevToolsProtocolMethod` call returned — whether
    /// the bridge accepted the method name at all.
    sync: String,
    /// The completion callback's HRESULT, `None` if it never fired.
    done: Option<String>,
    /// Host-measured ms from the call to the callback.
    ms: f64,
    bytes: usize,
    nodes: usize,
    /// Facts read out of the payload.
    lines: Vec<String>,
}

#[derive(Default)]
struct Acc {
    rows: Vec<Row>,
}

type Shared = Rc<RefCell<Acc>>;

fn row(method: &'static str, sync: String, done: Option<String>, ms: f64, bytes: usize, nodes: usize, lines: Vec<String>) -> Row {
    Row { method, sync, done, ms, bytes, nodes, lines }
}

/// The settings the calls are made under, printed so "it worked with DevTools
/// never opened" stays separable from "it worked because DevTools was on".
fn settings_row() -> Step {
    step(move |h| {
        let Some(wv) = h.com(PAGE_VIEW) else {
            println!("SETTING: no webview to read");
            return Tick::Done;
        };
        let text = match unsafe { wv.Settings() } {
            Ok(s) => {
                let mut v = BOOL(0);
                // SAFETY: live settings object; the out-param is a local.
                match unsafe { s.AreDevToolsEnabled(&mut v) } {
                    Ok(()) => v.as_bool().to_string(),
                    Err(e) => format!("unreadable ({e})"),
                }
            }
            Err(e) => format!("unreadable ({e})"),
        };
        println!(
            "SETTING ICoreWebView2Settings::AreDevToolsEnabled = {text}. Nothing in this \
             crate calls OpenDevTools or ShowDevTools, so DevTools was never opened in \
             this process."
        );
        Tick::Done
    })
}

/// A string as it will appear in a log line: on one line, quoted, capped.
fn short(s: &str, cap: usize) -> String {
    let flat = s.replace('\n', " ").replace('\r', " ");
    let mut out: String = flat.chars().take(cap).collect();
    if flat.chars().count() > out.chars().count() {
        out.push('…');
    }
    format!("\"{out}\"")
}

/// `strings[i]` as text. `-1` is the columnar payload's "no string here" sentinel
/// and yields an empty string; an index off the end is reported as such rather
/// than silently becoming empty, because that would look like missing text.
fn string_at(strings: &[Value], i: i64) -> String {
    if i < 0 {
        return String::new();
    }
    match strings.get(i as usize).and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => format!("<string index {i} not in the table>"),
    }
}

/// Slice a string by UTF-16 code-unit offsets, which is what CDP's `start` and
/// `length` count. Counting `char`s instead would be off by one for every
/// non-BMP character, and off-by-one in a measurement is a bug.
fn utf16_slice(s: &str, start: i64, len: i64) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let a = start.max(0) as usize;
    let b = (a + len.max(0) as usize).min(units.len());
    if a >= b {
        return String::new();
    }
    String::from_utf16_lossy(&units[a..b])
}

/// Call one CDP method and print everything about the answer.
fn cdp(method: &'static str, params: &'static str, acc: Shared) -> Step {
    let mut phase = 0u8;
    let mut rx = None::<Receiver<(i32, String, f64)>>;
    let mut sync = String::new();
    let mut started_ms = 0.0f64;
    step(move |h| {
        match phase {
            0 => {
                let Some(wv) = h.com(PAGE_VIEW) else {
                    println!("CDP {method}: NOT ATTEMPTED, the page view has no webview");
                    acc.borrow_mut().rows.push(row(method, "not attempted".to_string(), None, 0.0, 0, 0, vec![]));
                    return Tick::Done;
                };
                let t0 = win::qpc_ticks();
                let (tx, r) = channel();
                // `webview2-com` converts both completion arguments before the
                // closure sees them: the HRESULT arrives as a `Result` and the JSON
                // as a `String`. The raw code is still recoverable from the error,
                // so the failure case prints the number itself.
                let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                    move |res, json| {
                        let code = match &res {
                            Ok(()) => 0,
                            Err(e) => e.code().0,
                        };
                        let _ = tx.send((code, json, win::us_since(t0) / 1000.0));
                        Ok(())
                    },
                ));
                // The method name and parameters are `PCWSTR`s, so both UTF-16
                // buffers are built here and the locals keep them alive across the
                // call.
                let m: Vec<u16> = method.encode_utf16().chain(std::iter::once(0)).collect();
                let p: Vec<u16> = params.encode_utf16().chain(std::iter::once(0)).collect();
                println!("CDP {method} params={params}");
                // SAFETY: `m` and `p` are nul-terminated UTF-16 held by these locals
                // until the call returns; `handler` is a live COM object; the webview
                // belongs to this thread's apartment.
                let called = unsafe {
                    wv.CallDevToolsProtocolMethod(PCWSTR(m.as_ptr()), PCWSTR(p.as_ptr()), &handler)
                };
                sync = match &called {
                    Ok(()) => "S_OK (the bridge accepted the call)".to_string(),
                    Err(e) => format!("{} — HRESULT {:#010x}", e, e.code().0 as u32),
                };
                println!("CDP {method}: CallDevToolsProtocolMethod returned {sync}");
                if called.is_err() {
                    acc.borrow_mut().rows.push(row(method, sync.clone(), None, 0.0, 0, 0, vec![]));
                    return Tick::Done;
                }
                rx = Some(r);
                started_ms = win::qpc_ms();
                phase = 1;
                Tick::Running
            }
            _ => {
                let Some(rx) = &rx else {
                    return Tick::Done;
                };
                match rx.try_recv() {
                    Ok((code, json, ms)) => {
                        let done = if code == 0 {
                            "S_OK".to_string()
                        } else {
                            format!("HRESULT {:#010x}", code as u32)
                        };
                        let lines = analyse(method, &json);
                        let nodes = node_count(method, &json);
                        println!(
                            "CDP {method}: completed in {ms:.1} ms, {done}, {} bytes of JSON \
                             (waited {:.1} ms here for the callback)",
                            json.len(),
                            win::ms_since(started_ms)
                        );
                        for l in &lines {
                            println!("CDP {method}: {l}");
                        }
                        println!("CDP {method}: raw head — {}", head(&json));
                        // The payload itself, when it is small enough to keep. The
                        // first version of this analyzer guessed the modern
                        // `nodeTreeSnapshot` shape and reported zero nodes against
                        // a payload that plainly had geometry in it, so the shape
                        // gets read from the runtime's own answer from here on.
                        if json.len() <= RAW_LIMIT {
                            println!("CDP {method}: RAW {json}");
                        }
                        acc.borrow_mut().rows.push(row(
                            method,
                            sync.clone(),
                            Some(done),
                            ms,
                            json.len(),
                            nodes,
                            lines,
                        ));
                        Tick::Done
                    }
                    Err(TryRecvError::Disconnected) => {
                        println!("CDP {method}: the completion channel closed with no result");
                        acc.borrow_mut().rows.push(row(method, sync.clone(), None, 0.0, 0, 0, vec![]));
                        Tick::Done
                    }
                    Err(TryRecvError::Empty) => {
                        if win::ms_since(started_ms) > CALL_TIMEOUT_MS {
                            println!(
                                "CDP {method}: the bridge accepted the call and the completion \
                                 callback never fired within {} s",
                                CALL_TIMEOUT_MS as i64 / 1000
                            );
                            acc.borrow_mut().rows.push(row(
                                method,
                                sync.clone(),
                                None,
                                CALL_TIMEOUT_MS,
                                0,
                                0,
                                vec!["never completed".to_string()],
                            ));
                            return Tick::Done;
                        }
                        Tick::Running
                    }
                }
            }
        }
    })
}

/// First 400 characters on one line, so the payload's real shape is on record and
/// not only this file's reading of it.
fn head(json: &str) -> String {
    let mut s: String = json.chars().take(400).collect();
    if json.chars().count() > s.chars().count() {
        s.push('…');
    }
    s.replace('\n', " ")
}

fn keys_of(v: &Value) -> Vec<&str> {
    v.as_object()
        .map(|o| o.keys().map(|k| k.as_str()).collect())
        .unwrap_or_default()
}

fn arr_len(v: &Value, ptr: &str) -> Option<usize> {
    v.pointer(ptr).and_then(Value::as_array).map(|a| a.len())
}

/// Node count in whatever unit the method itself reports: DOM nodes for the
/// snapshot, AX nodes for the tree.
fn node_count(method: &str, json: &str) -> usize {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return 0;
    };
    if method.starts_with("Accessibility") {
        arr_len(&v, "/nodes").unwrap_or(0)
    } else {
        snapshot_nodes(&v)
    }
}

/// The snapshot's DOM node count, in whichever generation this build used.
///
/// The runtime answered with the legacy *columnar* payload, where `documents[0]
/// .nodes` is an object of parallel arrays and the count is the length of any one
/// of them (`nodeName`). The newer spelling nests the same columns under
/// `nodes.nodeTreeSnapshot`. A plain array is tried last in case a build returns
/// one. Guessing wrong here is what made the first run print "0 DOM nodes" about
/// a payload that contained 35.
fn snapshot_nodes(v: &Value) -> usize {
    [
        "/documents/0/nodes/nodeName",
        "/documents/0/nodes/nodeTreeSnapshot/nodeNames",
        "/documents/0/nodes/nodeTreeSnapshot/backendNodeIds",
        "/documents/0/nodes",
    ]
    .into_iter()
    .find_map(|p| arr_len(v, p))
    .unwrap_or(0)
}

/// A rect, in whichever spelling this build used: `[x, y, w, h]` or an object.
fn fmt_rect(v: &Value) -> String {
    if let Some(a) = v.as_array() {
        let n: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
        if n.len() == 4 {
            return format!("x={} y={} w={} h={}", n[0], n[1], n[2], n[3]);
        }
    }
    if v.is_object() {
        return format!(
            "x={} y={} w={} h={}",
            v.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            v.get("y").and_then(Value::as_f64).unwrap_or(0.0),
            v.get("width").and_then(Value::as_f64).unwrap_or(0.0),
            v.get("height").and_then(Value::as_f64).unwrap_or(0.0),
        );
    }
    v.to_string()
}

/// For every key of an object, what kind of thing it holds and how big it is.
///
/// Two generations of `DOMSnapshot.captureSnapshot` payload exist: the current
/// one, whose node tree is `nodes.nodeTreeSnapshot` with geometry in
/// `nodes.layoutTreeSnapshot.bounds` as objects, and the one this runtime
/// actually returns, whose head starts `{"documents":[{"baseURL":0,…
/// "layout":{"bounds":[[0,0,1350,900],…`. The first analyzer assumed the former
/// and printed "0 DOM nodes" against a payload that visibly had geometry in it. So
/// the shape is reported from the answer, and only then read.
fn shape_of(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(map) = v.as_object() else {
        return out;
    };
    for (k, val) in map {
        match val {
            Value::Array(a) => {
                let first = a
                    .first()
                    .map(|f| match f {
                        Value::Object(_) => format!("keys {:?}", keys_of(f)),
                        Value::Array(inner) => format!("[{}] {:?}", inner.len(), inner),
                        other => format!("{other}"),
                    })
                    .unwrap_or_else(|| "(empty)".to_string());
                out.push(format!("{k}: array of {}, first → {first}", a.len()));
            }
            Value::Object(o) => out.push(format!("{k}: object with keys {:?}", o.keys().collect::<Vec<_>>())),
            other => {
                let s = other.to_string();
                out.push(format!("{k}: {}", s.chars().take(80).collect::<String>()));
            }
        }
    }
    out
}

/// What the payload holds, as facts. Every line is read out of this run's own JSON
/// — a field that is not there prints as `absent` rather than being assumed.
fn analyse(method: &str, json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let v: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            out.push(format!("the payload is not JSON: {e}"));
            return out;
        }
    };
    out.push(format!("top-level keys: {:?}", keys_of(&v)));
    if v.get("exceptionDetails").is_some() {
        out.push(format!("CDP reported exceptionDetails: {}", head(json)));
    }
    if method.starts_with("DOMSnapshot") {
        let Some(docs) = v.pointer("/documents").and_then(Value::as_array) else {
            out.push("no documents array".to_string());
            return out;
        };
        out.push(format!(
            "documents: {}, strings in the shared table: {}",
            docs.len(),
            arr_len(&v, "/strings").unwrap_or(0)
        ));
        let Some(doc) = docs.first() else { return out };
        out.push(format!("document 0 keys: {:?}", keys_of(doc)));
        // Reported, not assumed: the node tree and the geometry live under
        // different names in the two generations of this payload, and this runtime
        // turned out to use the older one.
        for l in shape_of(doc) {
            out.push(format!("document 0.{l}"));
        }
        for key in ["nodes", "layout", "textBoxes"] {
            match doc.get(key) {
                Some(v) if v.is_array() => {
                    if let Some(first) = v.as_array().and_then(|a| a.first()) {
                        for l in shape_of(first) {
                            out.push(format!("document 0.{key}[0].{l}"));
                        }
                    }
                }
                Some(v) if v.is_object() => {
                    for l in shape_of(v) {
                        out.push(format!("document 0.{key}.{l}"));
                    }
                }
                _ => out.push(format!("document 0.{key}: absent")),
            }
        }
        out.extend(snapshot_text_and_geometry(&v, doc));
    }
    if method.starts_with("Accessibility") {
        let Some(nodes) = v.pointer("/nodes").and_then(Value::as_array) else {
            out.push("no nodes array".to_string());
            return out;
        };
        out.push(format!(
            "AX nodes: {}, keys on the first one: {:?}",
            nodes.len(),
            keys_of(&nodes[0])
        ));
        let ignored = nodes
            .iter()
            .filter(|n| n.get("ignored").and_then(Value::as_bool).unwrap_or(false))
            .count();
        out.push(format!("of which ignored: {ignored}"));
        let mut named = 0usize;
        let mut with_bounds = 0usize;
        let mut with_join_key = 0usize;
        let mut wanted = Vec::new();
        let mut no_key = Vec::new();
        let mut samples = Vec::new();
        for n in nodes {
            let role = n
                .get("role")
                .and_then(|r| r.get("value").and_then(Value::as_str).or_else(|| r.as_str()))
                .unwrap_or("?")
                .to_string();
            let name = n
                .get("name")
                .and_then(|x| x.get("value").and_then(Value::as_str).or_else(|| x.as_str()))
                .unwrap_or("")
                .to_string();
            if matches!(
                role.as_str(),
                "paragraph" | "StaticText" | "InlineTextBox" | "heading"
            ) {
                if let Some((_, c)) = wanted.iter_mut().find(|(r, _)| *r == role) {
                    *c += 1;
                } else {
                    wanted.push((role.clone(), 1));
                }
                // The ordinary-paragraph content, verbatim: this is what proves the
                // tree carries the page's words rather than just a role skeleton.
                if !name.is_empty() && samples.len() < TEXT_SAMPLES {
                    samples.push((role.clone(), name.clone()));
                }
            }
            if !name.is_empty() {
                named += 1;
            }
            if n.get("bounds").is_some() {
                with_bounds += 1;
            }
            match n.get("backendDOMNodeId") {
                Some(_) => with_join_key += 1,
                // Which roles lack the key matters more than how many: the per-line
                // fragments are exactly where a sentence's text lives, and they are
                // the ones with nothing to join a rect onto.
                None => {
                    if let Some((_, c)) = no_key.iter_mut().find(|(r, _)| *r == role) {
                        *c += 1;
                    } else {
                        no_key.push((role.clone(), 1));
                    }
                }
            }
        }
        out.push(format!("nodes with a non-empty name: {named}"));
        out.push(format!(
            "paragraph-level roles present: {wanted:?} (InlineTextBox is CDP's per-line \
             fragment of a StaticText, so a wrapped sentence is several nodes)"
        ));
        out.push(format!(
            "AX nodes carrying a `bounds` property: {with_bounds} of {}",
            nodes.len()
        ));
        out.push(format!(
            "AX nodes carrying `backendDOMNodeId`: {with_join_key} of {} — that id is the key \
             a rect has to be joined back on, because the tree itself has none",
            nodes.len()
        ));
        if !no_key.is_empty() {
            out.push(format!(
                "AX nodes WITHOUT `backendDOMNodeId`, by role: {no_key:?} — those cannot be \
                 joined to a DOM rect either, so whatever they carry has no addressable \
                 position at all through this pair of calls"
            ));
        }
        // A bounds-like field could be spelled differently; report what the payload
        // text holds instead of concluding from the one key checked above.
        out.push(format!(
            "does the payload text contain a bounds-like key anywhere: bounds={}, \
             boundingBox={}, backendDOMNodeId={}",
            json.contains("\"bounds\""),
            json.contains("\"boundingBox\""),
            json.contains("\"backendDOMNodeId\"")
        ));
        if samples.is_empty() {
            out.push(
                "no paragraph / StaticText / heading node has a non-empty name on this page"
                    .to_string(),
            );
        } else {
            for (role, name) in samples {
                out.push(format!("AX node text, role={role}: {}", short(&name, 120)));
            }
        }
    }
    out
}

/// Text and geometry out of the snapshot, joined the way the payload actually
/// joins them — the number that decides whether a rect-carrying extraction tier is
/// real work or a dead tier.
///
/// What this runtime returned is the legacy columnar payload, decoded from its own
/// `RAW` line rather than from a schema I assumed:
///
/// * `strings` — one top-level table every text reference indexes into.
/// * `documents[0].nodes` — parallel arrays (`nodeName`, `nodeType`,
///   `parentIndex`, `backendNodeId`, …), so node *n* is the n-th entry of each.
///   `nodeName[n]` is itself a string index.
/// * `documents[0].layout` — one entry per laid-out node: `bounds[n]` is
///   `[x, y, w, h]` in CSS px, `nodeIndex[n]` says which node it belongs to, and
///   `text[n]` is a string index or `-1` for "this node has no text".
/// * `documents[0].textBoxes` — per line fragment: `bounds[n]`, plus
///   `layoutIndex[n]`/`start[n]`/`length[n]` locating the run inside its layout
///   object's string.
///
/// So a paragraph's words and its rect are both present and reachable in one call;
/// the lines printed below are that join, not a claim about it.
fn snapshot_text_and_geometry(all: &Value, doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(strings) = all.pointer("/strings").and_then(Value::as_array) else {
        out.push("no top-level strings table, so no text index would resolve".to_string());
        return out;
    };
    let Some(layout) = doc
        .get("layout")
        .or_else(|| doc.pointer("/nodes/layoutTreeSnapshot"))
    else {
        out.push("no layout object: this snapshot carries no geometry at all".to_string());
        return out;
    };
    let Some(bounds) = layout.get("bounds").and_then(Value::as_array) else {
        out.push(format!(
            "layout object has no bounds array, keys: {:?}",
            keys_of(layout)
        ));
        return out;
    };
    let node_index = layout.get("nodeIndex").and_then(Value::as_array);
    let text_col = layout.get("text").and_then(Value::as_array);
    // `nodeName` (legacy) and `nodeTreeSnapshot.nodeName` (modern) are the same
    // column; whichever exists turns a node index back into a tag name.
    let names = doc
        .pointer("/nodes/nodeName")
        .or_else(|| doc.pointer("/nodes/nodeTreeSnapshot/nodeName"))
        .and_then(Value::as_array);
    let tag_of = |node: usize| -> String {
        names
            .and_then(|n| n.get(node))
            .and_then(Value::as_i64)
            .map(|s| string_at(strings, s))
            .unwrap_or_else(|| "?".to_string())
    };
    let with_text = text_col
        .map(|t| {
            t.iter()
                .filter(|v| v.as_i64().map(|i| i >= 0).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);
    out.push(format!(
        "geometry: {} layout objects with a rect, of which {} carry text; first three rects in \
         CSS px: {}",
        bounds.len(),
        with_text,
        bounds
            .iter()
            .take(3)
            .map(fmt_rect)
            .collect::<Vec<_>>()
            .join("; ")
    ));
    match node_index {
        Some(n) => out.push(format!(
            "layout nodeIndex: {} entries, indexing nodes 0..={} of the {}-node table",
            n.len(),
            n.iter().filter_map(Value::as_i64).max().unwrap_or(0),
            snapshot_nodes(all)
        )),
        None => out.push("layout has no nodeIndex column: text could not be traced back to \
                          the node it belongs to"
            .to_string()),
    }
    if let Some(ids) = doc
        .pointer("/nodes/backendNodeId")
        .and_then(Value::as_array)
    {
        out.push(format!(
            "node table carries backendNodeId for {} of {} nodes — the key an \
             `Accessibility` node's backendDOMNodeId joins to, which is how a name gets a rect",
            ids.len(),
            snapshot_nodes(all)
        ));
    }
    // The laid-out text runs, verbatim with their rects. Finding the page's own
    // sentence printed here is the evidence that text came back; finding a plausible
    // rect next to it is the evidence that geometry came back for the same content.
    match (text_col, node_index) {
        (Some(text_col), Some(node_index)) => {
            let mut shown = 0usize;
            for (i, t) in text_col.iter().enumerate() {
                if shown >= TEXT_SAMPLES {
                    break;
                }
                let Some(ti) = t.as_i64() else { continue };
                if ti < 0 {
                    continue;
                }
                let node = node_index.get(i).and_then(Value::as_i64).unwrap_or(-1);
                let rect = bounds
                    .get(i)
                    .map(fmt_rect)
                    .unwrap_or_else(|| "(no rect)".to_string());
                out.push(format!(
                    "laid-out text #{shown}: layout object {i}, node {node} <{}>, {rect}, text \
                     {}",
                    tag_of(node.max(0) as usize),
                    short(&string_at(strings, ti), 120)
                ));
                shown += 1;
            }
            if shown == 0 {
                out.push(
                    "no layout object has text, so this snapshot is geometry without words"
                        .to_string(),
                );
            }
        }
        _ => out.push("cannot print text with geometry: layout lacks a text or nodeIndex column"
            .to_string()),
    }
    // Per-line rects, which is what an overlay would actually anchor to.
    if let Some(tb) = doc
        .get("textBoxes")
        .or_else(|| doc.pointer("/nodes/textBoxSnapshot"))
    {
        let tb_bounds = tb.get("bounds").and_then(Value::as_array);
        let li = tb.get("layoutIndex").and_then(Value::as_array);
        let starts = tb.get("start").and_then(Value::as_array);
        let lens = tb.get("length").and_then(Value::as_array);
        match (tb_bounds, li, starts, lens) {
            (Some(b), Some(li), Some(starts), Some(lens)) => {
                out.push(format!(
                    "textBoxes: {} line fragments, each with a rect plus \
                     (layoutIndex, start, length) into its node's string",
                    b.len()
                ));
                for k in 0..b.len().min(TEXT_SAMPLES) {
                    let l = li.get(k).and_then(Value::as_i64).unwrap_or(-1);
                    let full = layout
                        .get("text")
                        .and_then(Value::as_array)
                        .and_then(|t| t.get(l.max(0) as usize))
                        .and_then(Value::as_i64)
                        .map(|s| string_at(strings, s))
                        .unwrap_or_default();
                    let piece = utf16_slice(
                        &full,
                        starts.get(k).and_then(Value::as_i64).unwrap_or(0),
                        lens.get(k).and_then(Value::as_i64).unwrap_or(0),
                    );
                    out.push(format!(
                        "text box #{k}: layout object {l}, {} — the run {}",
                        fmt_rect(&b[k]),
                        short(&piece, 80)
                    ));
                }
            }
            _ => out.push(format!(
                "textBoxes object is present but not the expected shape, keys: {:?}",
                keys_of(tb)
            )),
        }
    } else {
        out.push("no textBoxes: line-level rects are not in this payload".to_string());
    }
    out
}

fn verdict(a: &Acc) {
    println!("\n=== V4 VERDICTS: the DevTools-Protocol bridge on this runtime ===");
    if a.rows.is_empty() {
        println!("NOT MEASURED (no call ran)");
        return;
    }
    println!(
        "{:<34} {:>22} {:>10} {:>10} {:>9}",
        "method", "completion", "ms", "bytes", "nodes"
    );
    for r in &a.rows {
        println!(
            "{:<34} {:>22} {:>10.1} {:>10} {:>9}",
            r.method,
            r.done.as_deref().unwrap_or("(none)"),
            r.ms,
            r.bytes,
            r.nodes,
        );
    }
    println!();
    for r in &a.rows {
        println!("{}: the synchronous call returned {}", r.method, r.sync);
    }
    let snap = a.rows.iter().find(|r| r.method.starts_with("DOMSnapshot"));
    let ax = a.rows.iter().find(|r| r.method.starts_with("Accessibility"));
    let control = a.rows.iter().find(|r| r.method.starts_with("NotAReal"));
    let answered = |r: Option<&Row>| r.and_then(|r| r.done.as_ref()).map(|d| d == "S_OK").unwrap_or(false);
    println!(
        "\nV4-1 both extraction methods answered: {}. The environment carried only wry's own \
         browser arguments, DevTools was never opened, and the page was a normal https origin, \
         so on this runtime none of a port, a flag, or a setting is a prerequisite. The unknown \
         method behaved as: {}.",
        answered(snap) && answered(ax),
        match control {
            Some(r) => match (&r.done, &r.sync) {
                (Some(d), _) if d == "S_OK" => "it still returned S_OK, so the bridge does not \
                 validate method names — which means a typo'd domain would fail silently"
                    .to_string(),
                (Some(d), _) => format!("rejected, {d} (as documentation says)"),
                (None, s) => format!("rejected synchronously: {s}"),
            },
            None => "not measured".to_string(),
        }
    );
    if let Some(r) = ax {
        let no_bounds = r
            .lines
            .iter()
            .any(|l| l.starts_with("AX nodes carrying a `bounds` property: 0 of"));
        let join_gap = r
            .lines
            .iter()
            .find(|l| l.starts_with("AX nodes WITHOUT `backendDOMNodeId`"))
            .map(|l| format!(" {l}."))
            .unwrap_or_default();
        println!(
            "V4-2 the AX tree: {} nodes, {} bytes, {:.1} ms. {}",
            r.nodes,
            r.bytes,
            r.ms,
            if no_bounds {
                format!(
                    "It carries names but NO geometry: an AX-derived extraction cannot anchor \
                     a rect by itself, so a tier that needs position must join it to the DOM \
                     snapshot on `backendDOMNodeId`/`backendNodeId` or call `DOM.getBoxModel` \
                     per node. The paragraph text itself is in there — see the `AX node text` \
                     lines.{join_gap}"
                )
            } else {
                "See the bounds lines above for what this payload actually held.".to_string()
            }
        );
    }
    if let Some(r) = snap {
        let laid_out = r
            .lines
            .iter()
            .filter(|l| l.starts_with("laid-out text #"))
            .count();
        println!(
            "V4-3 the snapshot: {} DOM nodes, {} bytes, {:.1} ms. {}",
            r.nodes,
            r.bytes,
            r.ms,
            match r.lines.iter().find(|l| l.starts_with("geometry: ")) {
                Some(l) => format!(
                    "Text and geometry came back for the SAME content in one call — {l}. The \
                     {laid_out} laid-out text runs printed above are ordinary paragraph \
                     content with a rect attached to it, so a tier that needs positions has a \
                     real rect source here."
                ),
                None => "no geometry array was found in this payload; see the lines above, and \
                        note a tier that needs rects would have no rect source."
                    .to_string(),
            }
        );
    }
    println!(
        "\nV4-4 what this decides. The ruling under test permits CDP for read-only extraction \
         only: every call this mode makes is a query with a literal parameter list, nothing \
         here touches an `Input.*` domain, opens a remote-debugging port, or exposes a CDP \
         surface to a page. {}. Independently of this measurement, the bridge is Chromium's \
         and can vanish or change shape on a runtime update, so the shipped code must treat \
         every call as optional: on a sync failure, a completion failure, a timeout, or an \
         unparseable payload it falls back to the main-world probe with a typed \
         `unsupported`/`failed` result, and it must never sit on the navigation path where a \
         vanished method could break a page load.",
        if answered(snap) && answered(ax) {
            "Both extraction methods answer today on this runtime, so §4.2's tier-2 is real \
             work rather than a dead tier"
        } else {
            "At least one extraction method does not answer today on this runtime, so the \
             ruling is moot here and the tier must ship reporting itself unavailable"
        }
    );
}

pub fn program(site: String) -> Vec<Step> {
    let acc: Shared = Rc::new(RefCell::new(Acc::default()));
    let mut s: Vec<Step> = Vec::new();
    s.push(mark("V4.0 a live https page, DevTools never opened"));
    s.push(build_step(PAGE_VIEW, "PAGE", Some(&site)));
    s.push(expect_record(
        "PAGE",
        "NavigationCompleted",
        20_000,
        "the page CDP is called on",
    ));
    s.push(sleep_ms(1500));
    s.push(mark("V4.1 the settings the calls are made under"));
    s.push(settings_row());
    s.push(mark("V4.2 DOMSnapshot.captureSnapshot — the DOM plus layout geometry"));
    s.push(cdp("DOMSnapshot.captureSnapshot", "{\"computedStyles\":[]}", acc.clone()));
    s.push(mark("V4.3 Accessibility.getFullAXTree — the computed accessibility tree"));
    s.push(cdp("Accessibility.getFullAXTree", "{}", acc.clone()));
    s.push(mark("V4.4 a method no CDP domain has ever had, as the control"));
    s.push(cdp("NotARealDomain.NotAMethod", "{}", acc.clone()));
    s.push(step(move |h| {
        let _ = h;
        verdict(&acc.borrow());
        Tick::Done
    }));
    s
}
