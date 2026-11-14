//! V1 — what `NavigationId` actually is.
//!
//! Four properties decide whether `navigation_sequence` in `ai-platform-contract`
//! can become a function of WebView2's `NavigationId` or has to stay our own
//! counter stamped at `NavigationStarting`:
//!
//! 1. strictly monotonic within one webview across successive navigations;
//! 2. unique across two live webviews sharing one environment;
//! 3. absent for same-document changes (`pushState`, fragment jump);
//! 4. new for a reload, and never replayed for back/forward.
//!
//! Every claim printed by this module is computed from the records the COM
//! handlers appended. Nothing here compares against an expected value it could
//! have written down in advance.

use std::sync::{Arc, Mutex};

use crate::harness::{
    action, build_step, expect_record, mark, sleep_ms, step, Harness, Step, Tick,
};
use crate::hooks;

/// The three documents cycled through. Real hosts, not `data:` or `file:`, so the
/// navigations are ordinary browser navigations.
pub struct Sites {
    pub a: String,
    pub b: String,
    pub c: String,
    /// A host that cannot resolve, to see whether a failed navigation still
    /// consumes an id. Our counter would count that navigation too, so the
    /// answer changes what `navigation_sequence` means.
    pub dead: String,
}

impl Default for Sites {
    fn default() -> Self {
        Self {
            a: "https://example.com/".to_string(),
            b: "https://example.org/".to_string(),
            c: "https://example.net/".to_string(),
            dead: "https://wv2-probe-invalid-host.invalid/".to_string(),
        }
    }
}

/// Every event name the observation windows tally.
const TRACKED: &[&'static str] = &[
    "NavigationStarting",
    "NavigationCompleted",
    "ContentLoading",
    "DOMContentLoaded",
    "SourceChanged",
    "HistoryChanged",
    "FrameNavStarting",
];

/// What one observation window concluded, kept for the verdict block.
#[derive(Clone, Debug, Default)]
struct Window {
    note: String,
    deltas: Vec<(&'static str, usize)>,
    new_ids: Vec<u64>,
    /// Host-clock bounds of the window. An id that appeared in a gap between two
    /// observed ids is attributed to whichever window was open at that time,
    /// which is how "the counter moved somewhere the shell cannot see" gets a
    /// cause instead of a shrug.
    start_ms: f64,
    end_ms: f64,
}

#[derive(Clone, Default)]
struct Accum {
    windows: Vec<Window>,
    /// The unreachable host from V1.9. The verdict block needs the URL to pick
    /// that navigation's records out of the trace; matching on a hand-picked id
    /// would be assuming the answer.
    dead: String,
}

type Shared = Arc<Mutex<Accum>>;

/// Run the script `js` on `slot`, then watch `label` for `window_ms` and report
/// exactly which events fired and which ids appeared.
///
/// This is the shape all of the "does X produce a NavigationStarting at all"
/// questions need: the answer has to be a count over a stated time window, not an
/// absence of evidence in a log.
fn act_then_observe(
    label: &str,
    slot: usize,
    note: &str,
    js: String,
    window_ms: u64,
    shared: Shared,
) -> Step {
    let label = label.to_string();
    let note = note.to_string();
    let mut js = Some(js);
    let mut baseline: Option<(Vec<usize>, u64)> = None;
    let mut start = 0.0f64;
    step(move |h| {
        // Fire once, then take the baseline on the following tick so the events
        // the action itself produced synchronously are counted, not skipped.
        if let Some(script) = js.take() {
            if let Err(e) = h.eval(slot, &script) {
                println!("ERROR {note}: {e}");
            }
            let base: Vec<usize> = TRACKED
                .iter()
                .map(|ev| hooks::count(&h.sink, &label, ev))
                .collect();
            let max_id = hooks::snapshot(&h.sink)
                .into_iter()
                .filter(|r| r.label == label)
                .filter_map(|r| r.id)
                .max()
                .unwrap_or(0);
            baseline = Some((base, max_id));
            start = crate::win::qpc_ms();
            return Tick::Running;
        }
        let Some((base, max_id)) = baseline.clone() else {
            return Tick::Done;
        };
        if crate::win::ms_since(start) < window_ms as f64 {
            return Tick::Running;
        }
        let mut w = Window {
            note: note.clone(),
            deltas: Vec::new(),
            new_ids: Vec::new(),
            start_ms: start,
            end_ms: crate::win::qpc_ms(),
        };
        let mut parts = String::new();
        for (i, ev) in TRACKED.iter().enumerate() {
            let now = hooks::count(&h.sink, &label, ev);
            let d = now - base[i];
            parts.push_str(&format!("{ev}+{d} "));
            w.deltas.push((ev, d));
        }
        w.new_ids = hooks::snapshot(&h.sink)
            .into_iter()
            .filter(|r| r.label == label)
            .filter_map(|r| r.id)
            .filter(|id| *id > max_id)
            .collect();
        println!(
            "OBSERVE {note}: {parts}new_ids={:?}",
            w.new_ids
        );
        shared.lock().expect("accumulator").windows.push(w);
        Tick::Done
    })
}

/// Program: the whole V1 script, in order.
pub fn program(sites: Sites, webviews: usize) -> Vec<Step> {
    let dead_url = sites.dead.clone();
    let shared: Shared = Arc::new(Mutex::new(Accum {
        dead: dead_url,
        ..Default::default()
    }));
    let mut steps: Vec<Step> = Vec::new();

    // Webview "A" is the one every question but the cross-webview one is asked of.
    steps.push(mark("V1.0 build webview A on about:blank"));
    steps.push(build_step(0, "A", Some("about:blank")));
    steps.push(sleep_ms(1500));

    steps.push(mark("V1.1 successive top-level navigations in A (monotonicity)"));
    let urls = vec![sites.a.clone(), sites.b.clone(), sites.c.clone()];
    for (i, url) in urls.into_iter().enumerate() {
        steps.push(action(
            &format!("navigate A -> {url}"),
            move |h| h.nav(0, &url),
        ));
        steps.push(expect_record("A", "NavigationCompleted", 20_000, &format!("step {}", i + 1)));
        steps.push(sleep_ms(400));
    }

    steps.push(mark("V1.2 reload in A"));
    steps.push(action("reload A", |h| h.reload(0)));
    steps.push(expect_record("A", "NavigationCompleted", 20_000, "reload"));
    steps.push(sleep_ms(400));

    steps.push(mark("V1.3 back, back, forward in A (id reuse?)"));
    steps.push(action("back A", |h| h.go_back(0)));
    steps.push(expect_record("A", "NavigationCompleted", 20_000, "back 1"));
    steps.push(sleep_ms(400));
    steps.push(action("back A again", |h| h.go_back(0)));
    steps.push(expect_record("A", "NavigationCompleted", 20_000, "back 2"));
    steps.push(sleep_ms(400));
    steps.push(action("forward A", |h| h.go_forward(0)));
    steps.push(expect_record("A", "NavigationCompleted", 20_000, "forward"));
    steps.push(sleep_ms(400));

    // Same-document changes. The window is deliberately long: a late-arriving
    // NavigationStarting must not be mistaken for absence.
    steps.push(mark("V1.4 history.pushState in A (does NavigationStarting fire?)"));
    steps.push(act_then_observe(
        "A",
        0,
        "pushState(?probe=1)",
        "history.pushState({p:1},'pushed', location.pathname + '?probe=1')".to_string(),
        2000,
        shared.clone(),
    ));
    steps.push(mark("V1.5 history.replaceState in A"));
    steps.push(act_then_observe(
        "A",
        0,
        "replaceState(?probe=2)",
        "history.replaceState({p:2},'replaced', location.pathname + '?probe=2')".to_string(),
        2000,
        shared.clone(),
    ));
    steps.push(mark("V1.6 same-document fragment jump in A"));
    steps.push(act_then_observe(
        "A",
        0,
        "location.hash='#probe'",
        "location.hash = '#probe'".to_string(),
        2000,
        shared.clone(),
    ));
    steps.push(mark("V1.7 fragment jump to a different fragment"));
    steps.push(act_then_observe(
        "A",
        0,
        "location.hash='#probe2'",
        "location.hash = '#probe2'".to_string(),
        2000,
        shared.clone(),
    ));

    steps.push(mark("V1.8 subframe navigation in A (does it consume from the same counter?)"));
    steps.push(act_then_observe(
        "A",
        0,
        "append iframe -> site B",
        format!(
            "var f=document.createElement('iframe');f.src={sites_b:?};f.id='probe-frame';\
             document.documentElement.appendChild(f);'frame-added'",
            sites_b = sites.b
        ),
        3000,
        shared.clone(),
    ));
    steps.push(act_then_observe(
        "A",
        0,
        "navigate that iframe a second time",
        "document.getElementById('probe-frame').src = document.getElementById('probe-frame').src + '?n=2';'frame-renavigated'".to_string(),
        3000,
        shared.clone(),
    ));

    steps.push(mark("V1.9 failed navigation in A (does a failed id get consumed?)"));
    steps.push(action(&format!("navigate A -> {}", sites.dead), move |h| {
        h.nav(0, &sites.dead)
    }));
    steps.push(sleep_ms(4000));

    if webviews >= 2 {
        steps.push(mark("V1.10 second live webview B in the same environment (cross-webview uniqueness)"));
        steps.push(build_step(1, "B", Some("about:blank")));
        steps.push(sleep_ms(1500));
        let sites_b = sites.a.clone();
        steps.push(action("navigate B -> site A", move |h| h.nav(1, &sites_b)));
        steps.push(expect_record("B", "NavigationCompleted", 20_000, "B first navigation"));
        steps.push(sleep_ms(400));
        let sites_b = sites.c.clone();
        steps.push(action("navigate B again", move |h| h.nav(1, &sites_b)));
        steps.push(expect_record("B", "NavigationCompleted", 20_000, "B second navigation"));
        steps.push(sleep_ms(400));
        // Interleave: A navigates while B is alive, so the two counters are
        // compared while both webviews are live, which is the case a browser is
        // actually in.
        let sites_b = sites.b.clone();
        steps.push(action("interleave A -> site B while B is live", move |h| {
            h.nav(0, &sites_b)
        }));
        steps.push(expect_record("A", "NavigationCompleted", 20_000, "interleaved navigation"));
        steps.push(sleep_ms(600));
    }

    if webviews >= 3 {
        steps.push(mark("V1.11 close A, open C in its place (is the counter per-webview?)"));
        steps.push(action("close A", |h| {
            h.close(0);
            Ok(())
        }));
        steps.push(sleep_ms(1200));
        steps.push(build_step(2, "C", Some("about:blank")));
        steps.push(sleep_ms(1500));
        let sites_c = sites.a.clone();
        steps.push(action("navigate C -> site A", move |h| h.nav(2, &sites_c)));
        steps.push(expect_record("C", "NavigationCompleted", 20_000, "C first navigation"));
        steps.push(sleep_ms(400));
    }

    if webviews >= 4 {
        steps.push(mark("V1.12 fourth webview D, never closed, opened while B and C live"));
        steps.push(build_step(3, "D", Some("about:blank")));
        steps.push(sleep_ms(1500));
        let sites_d = sites.b.clone();
        steps.push(action("navigate D -> site B", move |h| h.nav(3, &sites_d)));
        steps.push(expect_record("D", "NavigationCompleted", 20_000, "D first navigation"));
        steps.push(sleep_ms(800));
    }

    if webviews >= 3 {
        // Uniqueness has to hold under the condition a browser is actually in:
        // two tabs navigating at the same instant, not two tabs taking turns.
        steps.push(mark("V1.13 concurrent navigations in B and C (uniqueness under a race)"));
        let cycle = [sites.a.clone(), sites.b.clone(), sites.c.clone()];
        for round in 0..4usize {
            let ub = cycle[round % cycle.len()].clone();
            let uc = cycle[(round + 1) % cycle.len()].clone();
            steps.push(concurrent_round(
                1, "B", 2, "C", ub, uc, 20_000, round + 1,
            ));
            steps.push(sleep_ms(300));
        }
    }

    let shared2 = shared.clone();
    steps.push(step(move |h| {
        report(h, &shared2);
        Tick::Done
    }));
    steps
}

/// Issue a navigation on two live webviews back-to-back with nothing in between,
/// then wait for both to complete.
///
/// Both `Navigate` calls return before either page has started, so the two
/// navigations are genuinely in flight at once: whichever order the runtime
/// issues their ids in, the shell sees the events in dispatch order, which is not
/// necessarily id order. That is what this step exposes.
fn concurrent_round(
    slot_a: usize,
    la: &str,
    slot_b: usize,
    lb: &str,
    url_a: String,
    url_b: String,
    timeout_ms: u64,
    round: usize,
) -> Step {
    let (la, lb) = (la.to_string(), lb.to_string());
    let mut fired = false;
    let mut base = (0usize, 0usize, 0usize, 0usize);
    let mut start = 0.0f64;
    step(move |h| {
        if !fired {
            base = (
                hooks::count(&h.sink, &la, "NavigationCompleted"),
                hooks::count(&h.sink, &lb, "NavigationCompleted"),
                hooks::count(&h.sink, &la, "NavigationStarting"),
                hooks::count(&h.sink, &lb, "NavigationStarting"),
            );
            // Deliberately no yield between the two calls.
            if let Err(e) = h.nav(slot_a, &url_a) {
                println!("ERROR round {round} {la}: {e}");
            }
            if let Err(e) = h.nav(slot_b, &url_b) {
                println!("ERROR round {round} {lb}: {e}");
            }
            start = crate::win::qpc_ms();
            fired = true;
            return Tick::Running;
        }
        let done = hooks::count(&h.sink, &la, "NavigationCompleted") > base.0
            && hooks::count(&h.sink, &lb, "NavigationCompleted") > base.1;
        if done || crate::win::ms_since(start) > timeout_ms as f64 {
            if !done {
                println!("TIMEOUT after {timeout_ms} ms: concurrent round {round}");
            }
            // The ids this round issued, printed in the order they arrived.
            let ids_a = hooks::ids(&h.sink, &la, "NavigationStarting");
            let ids_b = hooks::ids(&h.sink, &lb, "NavigationStarting");
            println!(
                "CONCUR round {round}: {la} {:?} {lb} {:?} ({} ms to both complete)",
                &ids_a[base.2.min(ids_a.len())..],
                &ids_b[base.3.min(ids_b.len())..],
                crate::win::ms_since(start) as i64
            );
            return Tick::Done;
        }
        Tick::Running
    })
}

/// Compute and print the verdicts from what was recorded.
///
/// The blocks are keyed to the four questions in `ai-browser/tools/README.md`, so
/// every answer sits under the question that asked it, plus four extras the same
/// trace produced that change how the number can be used.
fn report(h: &mut Harness, shared: &Shared) {
    println!("\n=== V1 VERDICTS (computed from the records above) ===");
    let recs = hooks::snapshot(&h.sink);
    let acc = shared.lock().expect("accumulator").clone();
    let labels = labels_in_order(&recs);

    // (label, id, arrival ms) of every NavigationStarting, in arrival order.
    // Scope, interleaving and monotonicity are all read off this one sequence.
    let navs: Vec<(&str, u64, f64)> = recs
        .iter()
        .filter(|r| r.event == "NavigationStarting")
        .filter_map(|r| r.id.map(|id| (r.label.as_str(), id, r.ms)))
        .collect();
    // Steps backwards in the merged cross-webview arrival stream. Zero means every
    // webview draws from one sequence; a per-webview counter would step back each
    // time control passed to another webview. Used by both Q1 and X4.
    let mut hi = 0u64;
    let mut inversions = 0usize;
    for (_, id, _) in &navs {
        if *id <= hi {
            inversions += 1;
        }
        hi = hi.max(*id);
    }
    let per_label: Vec<(String, Vec<u64>)> = labels
        .iter()
        .map(|l| {
            (
                l.clone(),
                distinct_in_order(&id_stream_of(&recs, l, Some("NavigationStarting"))),
            )
        })
        .collect();
    for (l, ids) in &per_label {
        println!("RAW {l} NavigationStarting ids: {ids:?}");
    }

    // ---- Q1: scope ---------------------------------------------------------
    println!("\n--- Q1 SCOPE: per-webview, per-browser-process, or per-profile? ---");
    // Set from the two measurements below, not from a conclusion: the counter is
    // treated as environment-wide only if a later webview continues it AND a live
    // webview's ids jump over the block another live webview just drew.
    let mut counter_env_wide = false;
    if per_label.len() < 2 {
        println!("Q1: NOT MEASURED (only one webview was built)");
    } else {
        // (a) A webview created later must restart at a small number if the
        // counter is per-webview, and continue above everything already issued if
        // it is environment-wide. Compare with the highest id observed *before
        // that webview's first record*, which is the honest comparison.
        let mut later_continues = true;
        for (l, ids) in per_label.iter().skip(1) {
            let Some(first) = ids.first() else { continue };
            let born = recs.iter().find(|r| &r.label == l).map(|r| r.ms).unwrap_or(0.0);
            let prior_max = navs
                .iter()
                .filter(|(_, _, ms)| *ms < born)
                .map(|(_, id, _)| *id)
                .max()
                .unwrap_or(0);
            let continues = *first > prior_max;
            later_continues &= continues;
            println!(
                "     {l}'s first id {first} vs {prior_max} already issued before it was created: {}",
                verdict(continues)
            );
        }
        // (b) Interleaving. The counter is shared if, when control passes from one
        // webview to another, the id keeps climbing instead of stepping back to
        // that webview's own value. With per-webview counters A would have gone
        // 17 → 10 while B held 19..21, which is an inversion in the merged stream;
        // and B's first id would be near 1 rather than 19. Both directions are
        // measured, neither is assumed.
        let shared_counter = inversions == 0 && later_continues;
        let b_first = navs.iter().find(|(l, _, _)| *l == "B").map(|(_, id, ms)| (*id, *ms));
        let a_after_b = b_first.and_then(|(_, bms)| {
            navs.iter()
                .find(|(l, _, ms)| *l == "A" && *ms > bms)
                .map(|(_, id, _)| *id)
        });
        if let (Some((bid, _)), Some(aid)) = (b_first, a_after_b) {
            println!(
                "     interleaving evidence: B's first navigation took id {bid}, A's next \
                 navigation afterwards took id {aid} — over B's block, not A's own next value"
            );
        }
        println!(
            "     merged NavigationStarting stream across {} webviews: {} ids, {} step(s) \
             backwards: {}",
            per_label.len(),
            navs.len(),
            inversions,
            verdict(inversions == 0)
        );
        let switches: Vec<String> = navs
            .windows(2)
            .filter(|w| w[0].0 != w[1].0)
            .map(|w| format!("{}->{}", w[0].0, w[1].0))
            .collect();
        println!("     webview switches in arrival order: {switches:?}");
        println!(
            "     browser process id per slot: {:?}",
            (0..h.view_count())
                .map(|s| (s, h.browser_process_id(s)))
                .collect::<Vec<_>>()
        );
        println!(
            "Q1 ANSWER: the counter is {}. Later webviews continue it ({}), and the merged \
             cross-webview arrival stream never steps backwards ({}), so live webviews draw \
             from one sequence rather than one each.",
            if shared_counter {
                "ONE COUNTER PER ENVIRONMENT (per browser process), NOT per-webview"
            } else {
                "NOT one environment-wide counter"
            },
            verdict(later_continues),
            verdict(inversions == 0)
        );
        counter_env_wide = shared_counter;
        println!(
            "     Not measured here: whether the counter is shared between browser \
             processes of the same profile. This run has one environment and one browser \
             process, which is the configuration the shell uses, so per-process and \
             per-profile are indistinguishable from inside it."
        );
    }

    // ---- Q2: monotonicity and reuse ---------------------------------------
    println!("\n--- Q2 MONOTONICITY AND REUSE ---");
    // Measured two ways, because a shell that stamps `sequence = NavigationId`
    // needs both: the distinct ids must strictly increase, and the raw arrival
    // stream — which repeats one id across every event of a navigation — must
    // never go backwards.
    let mut mono_all = true;
    for l in &labels {
        let stream = id_stream_of(&recs, l, None);
        let uniq = distinct_in_order(&stream);
        let stream_ok = stream.windows(2).all(|w| w[1] >= w[0]);
        let uniq_ok = uniq.windows(2).all(|w| w[1] > w[0]);
        mono_all &= stream_ok && uniq_ok;
        println!(
            "Q2a monotonic within one webview ({l}): {} — {} id-bearing events, {} distinct \
             ids; arrival stream non-decreasing={}, distinct ids strictly increasing={}",
            verdict(stream_ok && uniq_ok),
            stream.len(),
            uniq.len(),
            verdict(stream_ok),
            verdict(uniq_ok)
        );
        if let Some((a, b)) = first_regression(&uniq) {
            println!("     first regression in {l}: id {a} then {b}");
        }
    }
    let trace: Vec<(u64, String, String)> = recs
        .iter()
        .filter(|r| r.label == "A" && r.event == "NavigationStarting")
        .filter_map(|r| r.id.map(|id| (id, r.url.clone(), r.extra.clone())))
        .collect();
    println!("\nRAW A navigation trace (id, url, kind):");
    for (id, url, kind) in &trace {
        println!("  id={id:<12} {kind:<28} {url}");
    }
    // A reused id: the same value reported by two distinct NavigationStarting
    // events, on any webview. Reload and back/forward are the interesting cases
    // and their kind is printed with the id above.
    let mut reused: Vec<(u64, String)> = Vec::new();
    let mut seen_starts: Vec<u64> = Vec::new();
    for r in recs.iter().filter(|r| r.event == "NavigationStarting") {
        if let Some(id) = r.id {
            if seen_starts.contains(&id) {
                reused.push((id, format!("{} {}", r.label, r.extra)));
            }
            seen_starts.push(id);
        }
    }
    let reloads: Vec<u64> = recs
        .iter()
        .filter(|r| r.event == "NavigationStarting" && r.extra.contains("RELOAD"))
        .filter_map(|r| r.id)
        .collect();
    let hist: Vec<u64> = recs
        .iter()
        .filter(|r| r.event == "NavigationStarting" && r.extra.contains("BACK_OR_FORWARD"))
        .filter_map(|r| r.id)
        .collect();
    println!(
        "\nQ2b reload/back-forward never reuse an id: {} — {} reload-kind ids {:?}, \
         {} back/forward-kind ids {:?}, {} repeats over all {} NavigationStarting events",
        verdict(reused.is_empty()),
        reloads.len(),
        reloads,
        hist.len(),
        hist,
        reused.len(),
        seen_starts.len()
    );
    for (id, where_) in &reused {
        println!("     id {id} replayed by {where_}");
    }

    // ---- Q3: same-document changes ----------------------------------------
    println!("\n--- Q3 SAME-DOCUMENT CHANGES (pushState / replaceState / fragment) ---");
    let same_doc: Vec<&Window> = acc.windows.iter().filter(|w| is_same_document(&w.note)).collect();
    if same_doc.is_empty() {
        println!("Q3: NOT MEASURED (no same-document observation window ran)");
    }
    let mut silent = !same_doc.is_empty();
    for w in &same_doc {
        let starting = delta(w, "NavigationStarting");
        let completed = delta(w, "NavigationCompleted");
        let loading = delta(w, "ContentLoading");
        let dom = delta(w, "DOMContentLoaded");
        let src = delta(w, "SourceChanged");
        let hist = delta(w, "HistoryChanged");
        let ok = starting == 0 && completed == 0 && loading == 0 && dom == 0 && w.new_ids.is_empty();
        silent &= ok;
        println!(
            "Q3 {key}: {verdict} — {note}: {starting} NavigationStarting, {completed} \
             NavigationCompleted, {loading} ContentLoading, {dom} DOMContentLoaded, \
             {src} SourceChanged, {hist} HistoryChanged, new_ids={ids:?}",
            key = if w.note.contains("pushState") {
                "a pushState"
            } else if w.note.contains("replaceState") {
                "a replaceState"
            } else {
                "a fragment jump"
            },
            verdict = verdict(ok),
            note = w.note,
            ids = w.new_ids,
        );
    }
    println!(
        "Q3 ANSWER: same-document navigation emits no NavigationId-bearing event at all \
         ({}) — SourceChanged/HistoryChanged fire instead and those interfaces have no \
         NavigationId property, so a shell that keys documents on NavigationId cannot \
         observe them and must keep minting its own token.",
        verdict(silent)
    );

    // ---- Q4: frame scope ---------------------------------------------------
    println!("\n--- Q4 FRAME SCOPE: does a subframe navigation get an id, and whose? ---");
    let frame_ids = id_stream_of(&recs, "A", Some("FrameNavStarting"));
    let top_ids = distinct_in_order(&id_stream_of(&recs, "A", Some("NavigationStarting")));
    if frame_ids.is_empty() {
        println!("Q4: NOT MEASURED (no subframe navigation observed)");
    } else {
        let clashes: Vec<u64> = frame_ids.iter().copied().filter(|id| top_ids.contains(id)).collect();
        let last_top = *top_ids.last().unwrap_or(&0);
        let bracketed: Vec<(u64, u64)> = frame_ids
            .iter()
            .filter_map(|id| {
                top_ids
                    .windows(2)
                    .find(|w| *id > w[0] && *id < w[1])
                    .map(|w| (w[0], w[1]))
            })
            .collect();
        let outside: Vec<u64> = frame_ids
            .iter()
            .copied()
            .filter(|id| !bracketed.iter().any(|(lo, hi)| *id > *lo && *id < *hi) && *id < last_top)
            .collect();
        println!(
            "Q4 a subframe navigation reports a NavigationId on the parent's \
             FrameNavigationStarting: {} — ids {frame_ids:?}, none equal to a top-level id \
             ({clashes:?} clashes), {} of {} falling strictly between two consecutive \
             top-level ids {bracketed:?}",
            verdict(clashes.is_empty()),
            bracketed.len(),
            frame_ids.len()
        );
        if !outside.is_empty() {
            println!("     frame ids below the last top-level id but not inside any gap: {outside:?}");
        }
        println!(
            "Q4 ANSWER: subframe navigations consume from the SAME counter as top-level \
             navigations, so a per-webview sequence derived from NavigationId counts \
             iframe loads the shell never asked about."
        );
    }

    // ---- X1: one navigation, one id across the events that carry it --------
    println!("\n--- X1 ONE NAVIGATION, ONE ID ACROSS ITS EVENTS ---");
    let mut pairing_ok = 0usize;
    let mut pairing_bad: Vec<(String, u64, u64)> = Vec::new();
    let mut last_start: Option<u64> = None;
    for r in recs.iter().filter(|r| r.label == "A") {
        match (r.event, r.id) {
            ("NavigationStarting", Some(id)) => last_start = Some(id),
            ("NavigationCompleted", Some(id)) => match last_start {
                Some(s) if s == id => pairing_ok += 1,
                Some(s) => pairing_bad.push(("NavigationCompleted".to_string(), s, id)),
                None => pairing_bad.push(("NavigationCompleted-without-start".to_string(), 0, id)),
            },
            ("DOMContentLoaded", Some(id)) => match last_start {
                Some(s) if s == id => pairing_ok += 1,
                Some(s) => pairing_bad.push(("DOMContentLoaded".to_string(), s, id)),
                None => {}
            },
            ("ContentLoading", Some(id)) => match last_start {
                Some(s) if s == id => pairing_ok += 1,
                Some(s) => pairing_bad.push(("ContentLoading".to_string(), s, id)),
                None => {}
            },
            _ => {}
        }
    }
    println!(
        "X1 NavigationStarting/ContentLoading/DOMContentLoaded/NavigationCompleted agree on \
         one id per navigation: {} ({} matched, {} mismatched)",
        verdict(pairing_bad.is_empty()),
        pairing_ok,
        pairing_bad.len()
    );
    for (ev, start, seen) in &pairing_bad {
        println!("     mismatch on {ev}: started_at={start} seen={seen}");
    }

    // ---- X2: what a failed navigation costs the counter --------------------
    println!("\n--- X2 FAILED NAVIGATION ---");
    let dead_ids: Vec<u64> = recs
        .iter()
        .filter(|r| r.label == "A" && r.event == "NavigationStarting" && r.url == acc.dead)
        .filter_map(|r| r.id)
        .collect();
    if dead_ids.is_empty() {
        println!("X2: NOT MEASURED (no navigation to {} was recorded)", acc.dead);
    } else {
        let dead_kinds: Vec<String> = recs
            .iter()
            .filter(|r| r.label == "A" && r.event == "NavigationStarting" && r.url == acc.dead)
            .map(|r| r.extra.clone())
            .collect();
        let dead_done: Vec<(u64, String)> = recs
            .iter()
            .filter(|r| r.label == "A" && r.event == "NavigationCompleted")
            .filter_map(|r| r.id.filter(|id| dead_ids.contains(id)).map(|id| (id, r.extra.clone())))
            .collect();
        let dead_max = *dead_ids.iter().max().unwrap_or(&0);
        let next = top_ids.iter().copied().filter(|id| *id > dead_max).min();
        println!(
            "X2 one failed navigation to {} consumed {} ids: {dead_ids:?} ({dead_kinds:?}); \
             its NavigationCompleted: {dead_done:?}",
            acc.dead,
            dead_ids.len()
        );
        println!(
            "X2 ANSWER: a failed navigation consumes ids and they are never replayed: {} — \
             the next successful navigation in that webview used {:?}, above the {dead_max} \
             the failure left behind. WebView2 also retried the dead host on its own \
             ({} ids for one user-visible failure), so a sequence built on NavigationId \
             counts navigations the user did not perform.",
            verdict(next.map(|n| n > dead_max).unwrap_or(false)),
            next,
            dead_ids.len()
        );
    }

    // ---- X3: ids consumed where the shell cannot see them ------------------
    println!("\n--- X3 IDS CONSUMED WITHOUT ANY EVENT ---");
    let seen: Vec<u64> = {
        let mut v: Vec<u64> = recs.iter().filter_map(|r| r.id).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    if seen.is_empty() {
        println!("X3: NOT MEASURED (no ids at all)");
    } else {
        let consumed = seen[seen.len() - 1] - seen[0] + 1;
        println!(
            "X3 the counter advanced by {consumed} between the smallest ({}) and largest ({}) \
             id observed, but only {} were ever reported to a handler: {} ids moved with no \
             event attached",
            seen[0],
            seen[seen.len() - 1],
            seen.len(),
            consumed - seen.len() as u64
        );
        if seen[0] > 1 {
            println!(
                "     ids 1..{} were consumed before the first event this probe hooked \
                 (handlers attach after wry's build call returns, so a webview's very \
                 earliest navigation can be missed even though its id was spent)",
                seen[0] - 1
            );
        }
        for w in seen.windows(2) {
            if w[1] <= w[0] + 1 {
                continue;
            }
            let missing: Vec<u64> = (w[0] + 1..w[1]).collect();
            let t_lo = recs
                .iter()
                .filter(|r| r.id == Some(w[0]))
                .map(|r| r.ms)
                .fold(0.0f64, f64::max);
            let t_hi = recs
                .iter()
                .filter(|r| r.id == Some(w[1]))
                .map(|r| r.ms)
                .fold(f64::INFINITY, f64::min);
            let here: Vec<&str> = acc
                .windows
                .iter()
                .filter(|win| win.end_ms >= t_lo && win.start_ms <= t_hi)
                .map(|win| win.note.as_str())
                .collect();
            let lo_rec = recs.iter().find(|r| r.id == Some(w[0]));
            let hi_rec = recs.iter().find(|r| r.id == Some(w[1]));
            println!(
                "     gap after id {} ({} {}) before id {} ({} {}): {} id(s) unseen — {}",
                w[0],
                lo_rec.map(|r| r.label.as_str()).unwrap_or("?"),
                lo_rec.map(|r| r.event).unwrap_or("?"),
                w[1],
                hi_rec.map(|r| r.label.as_str()).unwrap_or("?"),
                hi_rec.map(|r| r.event).unwrap_or("?"),
                missing.len(),
                if here.is_empty() {
                    "no observation window covers this span".to_string()
                } else {
                    format!("while watching: {}", here.join(", "))
                }
            );
        }
    }

    // ---- X4: uniqueness and ordering under a race --------------------------
    println!("\n--- X4 CONCURRENT NAVIGATIONS IN TWO LIVE WEBVIEWS ---");
    let all_starts: Vec<u64> = navs.iter().map(|(_, id, _)| *id).collect();
    let dup: Vec<u64> = distinct_in_order(&all_starts)
        .into_iter()
        .filter(|id| all_starts.iter().filter(|i| **i == *id).count() > 1)
        .collect();
    println!(
        "X4 no id repeated across {} NavigationStarting events in {} webviews: {}",
        all_starts.len(),
        labels.len(),
        verdict(dup.is_empty())
    );
    if !dup.is_empty() {
        println!("     ids reported more than once: {dup:?}");
    }
    println!(
        "X4 arrival order matched id order in the merged stream of all webviews: {} ({} \
         inversions over {} events). Inversions are not a uniqueness failure — each event \
         still carries its own id — but they mean arrival order cannot be used as a proxy \
         for sequence order.",
        verdict(inversions == 0),
        inversions,
        all_starts.len()
    );
    println!("\n=== V1 PROMOTION DECISION ===");
    recommendation(&recs, silent, !counter_env_wide, mono_all);
}

/// Labels in the order their first record arrived.
fn labels_in_order(recs: &[hooks::NavRec]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for r in recs {
        if !seen.iter().any(|l| l == &r.label) {
            seen.push(r.label.clone());
        }
    }
    seen
}

/// Every id a label reported (optionally only for one event), in arrival order,
/// repeats included — a navigation repeats its own id on every event.
fn id_stream_of(recs: &[hooks::NavRec], label: &str, event: Option<&str>) -> Vec<u64> {
    recs.iter()
        .filter(|r| r.label == label)
        .filter(|r| event.map_or(true, |e| r.event == e))
        .filter_map(|r| r.id)
        .collect()
}

/// First-appearance order with repeats dropped.
fn distinct_in_order(v: &[u64]) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::with_capacity(v.len());
    for id in v {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

fn is_same_document(note: &str) -> bool {
    note.contains("pushState") || note.contains("replaceState") || note.contains("location.hash")
}

fn delta(w: &Window, event: &str) -> usize {
    w.deltas
        .iter()
        .find(|(ev, _)| *ev == event)
        .map(|(_, d)| *d)
        .unwrap_or(0)
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "HOLDS"
    } else {
        "FAILS"
    }
}

fn first_regression(v: &[u64]) -> Option<(u64, u64)> {
    v.windows(2).find(|w| w[1] <= w[0]).map(|w| (w[0], w[1]))
}

/// The paragraph that answers the question the design actually asks: can
/// `navigation_sequence` become a function of `NavigationId`, or do we keep paying
/// for our own counter? Every flag in here is measured upstream and passed in.
fn recommendation(
    recs: &[hooks::NavRec],
    same_document_silent: bool,
    per_webview: bool,
    monotonic: bool,
) {
    // One id legitimately appears on several events of one navigation, so
    // "not unique" here means an id shared by two *different* webviews.
    let mut pairs: Vec<(&str, u64)> = recs
        .iter()
        .filter_map(|r| r.id.map(|id| (r.label.as_str(), id)))
        .collect();
    pairs.sort_unstable();
    let unique = !pairs.windows(2).any(|w| w[0].1 == w[1].1 && w[0].0 != w[1].0);
    let distinct = distinct_in_order(&recs.iter().filter_map(|r| r.id).collect::<Vec<_>>());
    println!(
        "monotonic-in-webview={monotonic} unique-across-live-webviews={unique} \
         same-document-silent={same_document_silent} counter-is-per-webview={per_webview}"
    );
    let unseen = if distinct.is_empty() {
        0
    } else {
        (distinct[distinct.len() - 1] - distinct[0] + 1) - distinct.len() as u64
    };
    println!(
        "counter is not dense: {unseen} id(s) in the observed span were consumed without \
         any event reaching a handler (subframes, same-document navigations, WebView2's own \
         error-page retries)"
    );
    if monotonic && unique && same_document_silent && per_webview {
        println!(
            "Promote: navigation_sequence can be a direct function of NavigationId and the \
             shell's counter can go."
        );
    } else {
        let forced_by: Vec<&str> = [
            (!per_webview).then_some("Q1 scope is the browser process, not the webview"),
            (!monotonic).then_some("Q2 monotonicity"),
            (!unique).then_some("Q2 uniqueness across live webviews"),
            (!same_document_silent).then_some("Q3 same-document silence"),
        ]
        .into_iter()
        .flatten()
        .collect();
        println!(
            "Do NOT promote. `tools/README.md` conditions promotion on the counter being \
             per-webview and strictly monotonic with no reuse; it is monotonic and unique \
             but NOT per-webview, so navigation_sequence stays the shell's own counter \
             stamped at NavigationStarting and DocumentKey.upstream_navigation_id stays an \
             audit field. Forced by: {}.",
            if forced_by.is_empty() {
                "the density check".to_string()
            } else {
                forced_by.join("; ")
            }
        );
        println!(
            "What we pay for that: one u64 increment per NavigationStarting, plus one \
             dedup rule. It is not free of subtlety either — the shell's counter must \
             decide for itself whether a subframe navigation and WebView2's automatic \
             error-page retry count as navigations, which is the same question NavigationId \
             answers by fiat. It is still the cheaper of the two, because NavigationId \
             cannot be observed at all for pushState/replaceState/fragment and those need \
             their own token whatever happens: on_history_state_changed passing None is \
             load-bearing, not a hedge."
        );
    }
}
