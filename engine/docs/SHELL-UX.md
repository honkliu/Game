# SHELL-UX — the browser shell for the from-scratch engine

**Status:** v1 spec, 2026-11-13. Written against the code on disk, not against
`ARCHITECTURE.md`'s intentions — where the two disagree, this document follows the
code and names the disagreement. Capability facts come from
`docs/SUPPORTED-CSS.md`.

**Not in scope:** the WebView2 shell at `native-browser/` keeps shipping as it is.
This shell is a separate binary. Do not propose Chromium, WebView2, or Servo.

> **Verified build state, 2026-11-13.** `engine-layout` is a **stub**
> (`src/lib.rs` = `pub fn placeholder() {}`, no other file), so nothing produces
> geometry for a document. `engine-paint` is a stub too. What *is* real:
> `engine-css` → `engine-select` → `engine-style` (correct `ComputedStyle` per node),
> `engine-text` (font loading, measurement, line breaking, glyph runs), `engine-raster`
> (the full `DisplayList`/`DisplayItem` model, `render`, fills, borders, rounded
> rects, two-stop gradients, image blitting, clip stack, glyph blitting, and a
> byte-determinism harness). **The shell in this document is therefore specifiable
> and part-buildable today, but no page can be laid out or displayed until layout
> re-lands.** The shell must be designed so that layout arriving is an *addition*,
> not a rewrite.

---

## 0. Frame

**Goal.** A browser window a person can start, type into, navigate, read, and
recover from errors in — drawn by the engine's own rasterizer, with no web
technology under it.

**Primary user.** The owner, and the people they demo this to. Competent, impatient,
suspicious of spinner-then-blank pages.

**Job-to-be-done (v1).** "Open this URL, let me read it, and tell me the truth about
what the renderer could not do." Everything else — sessions, tabs, history — is
secondary to credibility.

**The constraint that governs every decision.** The engine has **no
`position:absolute`, no `float`, no flex, no grid, no `::before`, and — as of this
session — no layout crate at all** (`SUPPORTED-CSS.md` §1, §5, §11). So the shell is
laid out with **one tool**: a top-to-bottom stack of block boxes with padded,
background-filled rects. That is not a limitation to work around — it is the layout
language the shell is designed in, and designing in it is what keeps the shell
working when the engine changes underneath it.

**Second, stronger constraint (the one that actually matters this week).** Do not
make the shell's first deliverable depend on layout existing. §2.1 defines geometry
as a pure function and §2.2 defines the paint path as `shell_rects → DisplayList →
render → Surface`, which uses **only modules that exist today**. The shell is
therefore buildable and golden-testable now; the moment layout lands, the *content*
region starts being filled by a document instead of by a "no document" message, and
nothing in the header changes.

---

## 1. Survey — what the two existing shells got right, and what to keep

### 1.1 Our own prior art (`native-browser/crates/browser/ui/`)

| Artifact | What it does well | Carry forward |
|---|---|---|
| `index.html` header band | 36 px fixed band; 3 × 24 px icon buttons; omnibox as a form; a `mode-chip` span saying `URL` vs `Search`. (`native-browser/src/layout.rs::HEADER_H = 36`) | The **band order** and the **mode chip** — the honest way to say "I will search, not navigate". |
| `pages/error.html` | `role="alert"`; visible error code; failed URL in its own box; `Retry / Copy URL / Open in new tab / Details ▾`. | Actions minus the tab-specific one; error copy that names the code. |
| `pages/viewsource.html` | When a feature does not exist, it says so instead of going dead. | The principle: **a control that cannot work is never drawn as a live control.** |
| `pages/newtab.html` | Clock, a "mirror" search field that is honestly `aria-hidden`, pinned tiles, recent list. | The mirror's *honesty*; in the new shell the field is **real**, because the shell owns input. |
| `ipc.rs::COMMANDS` (55 verbs) | One exhaustive list; adding a verb forces an implementation. | Same idea, 7 verbs (§4.1), each with an acceptance test. |
| `layout.rs` | Geometry as pure, unit-testable functions. | `fn shell_rects(..) -> ShellLayout`, unit-tested with no window. |
| `overlay.html` / command palette | One palette reaches every verb. | v2 (§5.2). |

**Drop:** the tab rail (a tab strip dressed up — the convention the canvas thesis
rejects), the two-webview split whose geometry must stay in sync, and the 23 IPC
handlers the UI never calls.

### 1.2 Existing browsers — keep as-is, and why

| Feature | Verdict |
|---|---|
| Chrome omnibox combining URL entry with a visible "search for …" alternate | Keep; the mode chip makes the alternate visible *before* Enter. |
| Chrome error page with `ERR_*` code + Retry | Keep, cut to 3 actions. |
| Chrome reload→stop in the **same** button position | Keep; one control slot, not two. |
| Chrome back/forward long-press history | Defer to v2 (needs a popup surface). |
| Edge vertical tab rail; sleeping tabs | Reject; N/A (no background pages). |
| Safari Reader mode | **v1.1.** Our engine renders static documents; Reader converts a *partially* rendered page into a *fully* rendered one. Highest-value feature the engine's limits argue for. |
| Safari energy caution | Adopt: shell motion gated on `prefers-reduced-motion` **and** a shell toggle. |
| `Ctrl+L` focuses the address bar; `Esc` restores the URL | Keep verbatim — muscle memory is a compatibility feature. |

### 1.3 Spatial / canvas prior art — read honestly

| Prior art | Extractable | Why not v1 |
|---|---|---|
| ZUI / Pad++ zoomable UI | A page as a thing you approach. | Needs a scaled compositor; our layout is absolute device px and glyphs blit at one `run.size` (`engine-text/src/context.rs::blit_run`). Zoom-out needs a full re-layout at that size. |
| Opera Speed Dial; Arc spaces; Station/Stack | "Place" as the organising unit. | v2 / v2 / v3. |
| Vivaldi tiling | Two panes in one frame. | v2 — needs two display lists + two clip rects, which `DisplayItem::PushClip` already supports. |
| Game HUD minimaps | Orientation in a large space. | v2; a *document* scroll-overview strip is cheaper and more useful first. |

**Base choice, stated:** own engine, CPU rasterizer, D3D11 blit (`ARCHITECTURE.md`
§2). That **enables** a canvas — we own the display list and can put two clip regions
in one surface — and **constrains** zoom, because a scaled document is a re-layout.
Conclusion: a canvas of *documents* is achievable; a canvas of *scaled* documents is
not, until `engine-paint` gains a display-list transform.

---

## 2. What the shell draws itself

The shell is **not** HTML. It has three kinds of surface:

| Surface | Built how | Why |
|---|---|---|
| **Chrome** (header, status bar, panels) | A shell display list built by `shell-ui` directly, using `DisplayItem::{FillRect, Border, GlyphRun, Control, PushClip}` (`engine-raster`'s enum) and `FontContext::{measure, layout_runs, glyph_items}` | No HTML parser round-trip; predictable layout; testable as pure geometry. |
| **Internal pages** (new tab, error, render report, about:) | Real HTML + CSS, parsed and laid out by the engine, rendered into the page region | Dogfoods the engine on the simplest pages we can author. **Rule: an internal page may use only rows marked Yes in `SUPPORTED-CSS.md`.** A CI test parses the internal pages' CSS and fails if any declaration is `Unsupported`. |
| **Web content** | Engine pipeline, as now | — |

**Veto:** the shell never draws itself with HTML that uses flex, grid, floats, or
`position:absolute`. If the shell needed them, the shell would be lying about what
the engine can do, and would break the moment layout changes.

### 2.1 The shell layout function

```
fn shell_rects(viewport_px: Size, dpr: f32, s: &ShellState) -> ShellLayout
ShellLayout {
    header:  Rect,          // y = 0, h = HEADER_H * dpr
    content: Rect,          // page region; everything else
    status:  Rect,          // bottom band, visible on hover of a link or while loading
    overlay: Option<Rect>,  // mode chooser / report panel — drawn over content
}
```

Header band (CSS px, scaled by DPR once at the boundary):

```
[ 28 ][ back 28 ][ fwd 28 ][ reload/stop 28 ][ ▸ omnibox: flex-equivalent = remaining ][ 28 report ]
  ^                                                                                    ^
  collapse-sidebar                                                                render-report toggle
  (v2: the canvas toggle)
```

Geometry rules, all block-level and therefore implementable today:
- header height **36** (CSS px → ×DPR device px) — same number as `native-browser`'s
  `HEADER_H`, so the two shells feel like the same product.
- every icon button is **28 × 28** with a **4 px** gutter; the glyph is drawn as a
  `GlyphRun` from a 14 px `Segoe UI` (or as line art in `DisplayItem::FillRect`
  strips if no icon font is present). **Do not depend on a private icon font** —
  `verify` whether `Segoe Fluent Icons` exists on the target box; assume it does not.
- the omnibox is a `FillRect` (1 px border via `DisplayItem::Border`) with
  `CornerRadii` — `engine-raster::fill_rounded_rect` **exists and is tested**, so
  rounded omnibox corners are real. The text is a `GlyphRun` and the caret is a
  `DisplayItem::TextCursor { rect, color }`, which **also exists**. No text control
  needs `inline-block`.
- the mode chip is a `FillRect` + `GlyphRun`, left of the text.
- the focus ring is `DisplayItem::Outline { rect, color, width: 2.0 }` — this
  primitive exists and is the correct one, because it draws outside the rect without
  displacing layout.

### 2.2 The exact API surface the shell may use (all verified on disk)

```
engine-raster::DisplayItem::{FillRect, Border, Outline, Image, LinearGradient,
                             GlyphRun, TextCursor, Scrollbar, Control,
                             PushClip, PopClip}
engine-raster::{DisplayList::push, render(surface, list, clip, images, glyphs)}
engine-raster::Surface::new(w, h)                       // BGRA8, premultiplied
engine-raster::verify_determinism(..)                   // golden harness
engine-text::FontContext::{new, measure, line_metrics, select_face, layout_runs,
                           glyph_items}                 // impl GlyphBlitter
engine-core::{Rect, Point, Size, Rgba8, CornerRadii, EdgeSizes}
```

Anything not in that list is not available to the shell. In particular: **no
`LayoutTree`, no display-list builder from a document, no scroll container, no
hit-testing helper.** The shell writes its own geometry (`shell_rects`) and its own
rect hit-test (a handful of rect-contains checks — it does not need a general one).

`render` takes the surface, so the shell renders chrome and content into **one**
surface with two `PushClip` regions. That is the whole compositing story for v1, and
it is why there is no per-region GPU work.

---

## 3. Boot sequence

The shell must exist **before** any page loads, and must be useful when the page
never loads at all. Four states, each with a defined first frame:

### 3.1 State: `Boot` (target: first frame ≤ 150 ms, p95 ≤ 250 ms)

| Step | What is on screen | Budget |
|---|---|---|
| 1. Window shown | Clear colour only (paper-white `#FFFFFF`, or theme background) | Window show ≤ 120 ms after process start |
| 2. Font context loaded | Nothing visible changes | Font enumeration is the risk: `engine-text::FontContext::new()` calls `FaceTable::load()` over `C:\Windows\Fonts`. **Measure it**; if > 60 ms, it must move to a worker thread and the header must paint before fonts arrive. |
| 3. Header painted | Header band with back/forward/reload disabled, omnibox empty with placeholder `Search or type a URL`, report button disabled | ≤ 150 ms total |
| 4. Content region painted | **While `engine-layout` is a stub:** a `FillRect` in `bg.page` plus one `GlyphRun` line — *"No page loaded. Layout is not available in this build."* **Once layout lands:** the `about:newtab` document. The shell code path is identical; only the content display list changes. | ≤ 250 ms |

**Rule:** if step 2 exceeds the budget, the shell still shows the header with a
system-font fallback and repaints when faces land. A shell that blocks on fonts shows
a white rectangle, which is the thing this shell exists to avoid.

**Rule (layout independence):** the content region is a `ContentSource` enum —
`Empty`, `Message(text)`, `InternalPage(id)`, `Document(DisplayList)`. v1 ships with
three of the four. That is deliberate: it lets the shell be built, tested, and
demoed **now**, and it makes the layout-landing step a one-variant addition.

### 3.2 State: `NewTab` (internal page, `about:newtab`)

Content, top to bottom, all block flow:

1. A short mission line: *"This browser renders static documents with a from-scratch
   engine. It does not run applications."* — the honesty gate, on the first screen.
2. A **real** address field (the shell focuses its own omnibox on click; no dead
   mirror), plus the three most recent URLs as block rows (40 px, full width).
3. A capability card: the four site classes from `SUPPORTED-CSS.md` §14, one line
   each, with the *reason*. This is the most-read page in the product; it must not
   be marketing.
4. Key hints: `Ctrl+L` address, `Alt+Left` back, `Ctrl+R` reload, `F12` report.

Not in v1's new tab: clock (nobody opens a browser to read the time), pinned tiles
(needs image decoding + a start page), search suggestions (needs a network service).

### 3.3 State: `Loading`

- The reload button becomes **Stop** in the *same* rectangle (no layout shift).
- The status band shows the request: `GET https://host/path — connecting`,
  then `… — 1.2 MB of N` if `Content-Length` is present.
- The page region keeps the **previous** document visible until the new one has a
  layout tree. Never clear the page to white on navigation — a white flash is a
  worse lie than a stale page. (Chrome does the same; this is not invention.)
- If parsing+layout exceeds **300 ms**, the page region shows the *new* document's
  partial paint at first available frame, not a spinner. Spinners in a static
  renderer are the wrong metaphor; a partially painted page is honest.

### 3.4 State: `Error` / `Unsupported` (internal page, `about:error`)

Two distinct pages, because they are two distinct user problems:

**A. `NetError` — the fetch failed.** Actions: `Retry`, `Copy URL`, `Edit address`.
Shows the numeric status (e.g. `ERR_CERT_DATE_INVALID`, `HTTP 404`, `HTTP 500`) and
the URL verbatim. Modelled on `native-browser/ui/pages/error.html` minus
"Open in new tab" (no tabs in v1).

**B. `RenderUnsupported` — the fetch succeeded and the page is blank anyway.**
This page triggers on the detection in `ARCHITECTURE.md` §10.3 ("large empty
container, script count > 3, no text under body") and on *any* load whose render
report has zero text fragments. Actions: `View source text` (the `viewsource`
fallback from `engine-html`), `Show render report`, `Retry`.

Copy, fixed:

> **This page is built by scripts the engine does not run.**
>
> The download succeeded ({bytes} read, HTTP {status}). The document contains
> {n} script elements and no readable text. This engine renders static HTML and CSS
> only; it has no DOM scripting in v1.

**The render report page** (`about:report`, also `F12`) is a v1 feature, not a
debug screen. It renders `RenderReport` (already specified in `ARCHITECTURE.md`
§10.4) as a table: parse errors, skipped at-rules, stubbed features, unshaped
scripts, missing globals, per-stage milliseconds. Rationale: the engine's value
proposition is *honest partial rendering*; without a visible report it is just
partial rendering.

---

## 4. Navigation model

### 4.1 The minimum viable control set (v1)

| Control | Position | Keys | Behaviour |
|---|---|---|---|
| Address / omnibox | header, after nav buttons | `Ctrl+L`, `Alt+D`, `F6` | Focus selects all. Enter navigates if input parses as a URL with a scheme or a host-like form (`example.com/path`), else searches — **but searching requires a search engine, which is a network dependency; v1 ships with search disabled and the chip saying `URL`.** If the input is not a URL, the chip turns to `Not a URL`, and Enter does nothing rather than silently failing. |
| Back | header | `Alt+Left`, `Backspace` when not in a field | Session history stack (§4.2). Disabled at the first entry. |
| Forward | header | `Alt+Right` | Disabled at the newest entry. New navigation truncates forward entries. |
| Reload / Stop | header (one slot) | `F5`, `Ctrl+R` / `Esc` while loading | Stop aborts the fetch worker; the previous document stays. |
| Render report | header, right edge | `F12` | Toggles `about:report` in the page region; toggling back restores the document **from the retained layout tree** without a refetch. |
| Status band | bottom | — | Link target on hover (from hit-testing the display list), loading state otherwise. |
| Exit | — | `Alt+F4` | — |

**Zero navigation verbs beyond these in v1.** No tabs, no bookmarks, no history UI,
no downloads, no find-in-page. Each is a real feature that needs a real surface; a
half-working one costs more trust than its absence.

### 4.2 Session model

```
Session {
    history: Vec<Entry>,        // Entry { url, scroll: Vec<(ScrollerId, f32, f32)>, title }
    index: usize,               // current entry
    doc: Option<Document>,      // current document, kept across back/forward
    cache: Vec<CachedDoc>,      // LRU, capacity 4, keyed by URL
    shell: ShellState,          // focus, omnibox draft, report open
}
```

- **Back/forward** pops the current entry's scroll offsets into the entry, pushes the
  new entry's URL, and restores: cache hit → reuse the retained layout tree and
  apply the scroll offsets (**no refetch, no reparse**); miss → refetch. Budget for
  the cache-hit path: **≤ 120 ms** to first frame (`ARCHITECTURE.md` §11 already
  promises this number for session restore).
- **Scroll offsets live outside layout** (`ARCHITECTURE.md` §8.3). Back/forward must
  never trigger a relayout just to restore position.
- **History depth** 100 entries in memory; no disk persistence in v1.

### 4.3 The spatial thesis, first instalment: a **history trail**, not a tab strip

This is the one canvas idea that ships in v1, and it ships as a **status band**, not
a canvas. After each successful load, the band shows the last 5 entries as
`● host/path › ● host/path › …` with the current one marked. `Alt+←`/`→` moves
along it; `Alt+1…5` jumps to that entry. It costs no new layout capability (block
rows), and it does three things a tab strip cannot:

1. It shows **where you have been**, not only where you are.
2. It makes back/forward **legible** (today it is an invisible stack).
3. It is the honest v1 of "history as a path", which is the canvas model's
   navigation primitive (`SHELL-UX` §5.2 builds the map on top of it).

What we are **not** doing in v1, and why, in one line each:

| Canvas idea | Why not v1 |
|---|---|
| Pages as regions on an infinite canvas | Needs a scroll/pan transform over a retained display list and a second clip per region. Both are additions to `engine-layout` + `engine-paint`, not to the shell. |
| Zoom out to a card grid ("exposé") | Zoom implies re-layout at a smaller font; our glyphs are rasterised at a fixed `run.size`. A card grid is achievable *as a separate internal page* (each card = a snapshot bitmap) — that is the v2 form. |
| Minimap | Cheap in v2 **if** region boxes exist and a document-level page-colour snapshot can be downsampled. Not before. |
| Quick-travel / teleport by search | Needs an index of session content; v2 after the trail. |
| Game-feel micro-feedback (motion, sound) | Motion only where it communicates state (§5.3). No sounds in v1. |

---

## 5. v1 and v2

### 5.1 v1 — "Trustworthy document viewer"

Scope: everything in §2–§4, plus:

- Keyboard-complete operation (`SHELL-UX` §7).
- Internal pages `about:newtab`, `about:error`, `about:report` authored in the
  supported CSS subset.
- Focus model: the shell owns focus between chrome and page; the page's own focus
  traversal is the engine's (see §6.3).
- DPI: layout in CSS px, scaled by `Viewport.dpr` once; a DPR change repaints the
  header and re-lays out the document.
- Reduced motion: no shell animation at all is the default. There is nothing to
  reduce, so `prefers-reduced-motion` is trivially honoured.

### 5.2 v2 — "Where you have been" (the canvas, honestly)

Four additions, in dependency order. Each is gated on a *specific* engine addition,
and the gate is stated so nobody proposes the shell feature first.

| # | Feature | Engine dependency | What the user sees |
|---|---|---|---|
| 1 | **Session map page** (`about:map`) | Render every retained document to a small `Surface` at 25 % scale (**snapshot bitmap**, no re-layout) and draw them as block-stacked cards with their trail order | A scrollable list of miniatures of every page you visited this session. Clicking one restores it. This is fetch-safe because it uses *images of what we already rendered*. |
| 2 | **Two-pane split** | Two layout trees, two clip rects, hit-test routed by rect | Drag a link to the right half; it opens beside, without leaving the left page. |
| 3 | **Document minimap** in a 12 px right gutter | A downsampled column of the document's background/border colours + the viewport indicator | Orientation inside long documents. |
| 4 | **Timeline scrubbing of back/forward** | Entries already keep scroll offsets; add title + a captured favicon-free glyph | Hold `Alt+←` and scroll to walk history without committing. |

**The infinite canvas with linked regions is v3, and it is explicitly out of scope
until `engine-layout` supports zoom-invariant layout** (a display-list transform
stage in `engine-paint`). Shipping a stuttering zoom would be the failure mode the
canvas thesis is most likely to die of.

### 5.3 Interaction rule (applies from v1)

> **Motion communicates state, never decorates it.**

Allowed in v1: a 2 px caret blink in the omnibox (state: focus); the status band
appearing (state: hover/loading); the Stop glyph replacing Reload (state: loading).
Forbidden: page fades, header slide-ins, anything that animates on a timer without
user input. A static-document renderer that animates its own chrome is claiming an
aliveness the page does not have.

---

## 6. Interaction and input model

### 6.1 Focus ownership

Two focus domains: **chrome** and **content**. Exactly one holds focus.

```
Tab ring (chrome): [back]→[forward]→[reload]→[omnibox]→[report]→[content]
```

- `Tab` in the last chrome control moves focus **into** the page (the browser
  convention; without it, keyboard users cannot reach links in a page whose CSS sets
  no `outline` — which is every page here, since `outline` is `Unsupported`).
- `Shift+Tab` from the first focusable node in the page returns to the chrome.
- The **shell draws the chrome focus ring**: a 2 px `Border` inset in the button's
  rect, colour `--focus` (see §7). Reason: content CSS cannot draw focus rings
  (`SUPPORTED-CSS.md` §2 `outline`), so the shell is the only place a ring can exist.
  **The page's own focus ring is the engine's job and is currently not implemented —
  this is the single most serious accessibility gap before v1 ships (§8).**

### 6.2 Pointer

| Input | Chrome | Content |
|---|---|---|
| Click | Button press/release on the control's rect | Hit-test the display list; the topmost `DisplayItem` with a `BoxId` wins; link activation on release inside the same box |
| Drag | — (no draggable chrome in v1) | Text selection (engine), scrollbar drag when scrollbars exist |
| Wheel | Omnibox horizontal scroll | Scroll the nearest scroll container under the pointer, else the root; coalesced per frame (`ARCHITECTURE.md` §8.3) |
| Middle-click / `Ctrl+Click` | — | v1: nothing (no new tab to open it in). v2: new region. |

### 6.3 Keyboard in content

The engine's `:focus-visible` support is claimed in `ARCHITECTURE.md` §6.4;
**`verify` whether focus state is plumbed from the platform layer to the cascade.**
If it is not, v1 ships with a shell-drawn focus ring **at the shell level only**
(omnibox + buttons), and the content focus model is a *known v1 limitation* printed
on `about:newtab`. Do not paper over it.

### 6.4 IME

Composition in the omnibox must go through `WM_IME` events
(`engine-platform/src/ime.rs` per `ARCHITECTURE.md` §3.1). The shell must render the
pre-edit string **with an underline** (a 1 px `FillRect` under the composition run).
CJK input without a pre-edit indicator is unusable, and CJK is a script class the
engine renders *correctly* — so the omnibox is exactly where the engine should look
good.

---

## 7. Design tokens (shell only)

Drawn with `Rgba8` values in `engine-core::color`. Both sets must meet **WCAG AA**
(4.5:1 body text, 3:1 for large text and UI boundaries) against the surface each
sits on. The numbers are the *contract*; Bob should verify contrast with a
one-off calculation in a unit test, not by eye.

| Token | Light | Dark | Used for |
|---|---|---|---|
| `bg.chrome` | `#F3F3F3` | `#202020` | Header band, status band |
| `bg.surface` | `#FFFFFF` | `#FFFFFF` (page area stays white — page CSS defines its own colours) | Omnibox fill |
| `fg.primary` | `#1A1A1A` | `#E8E8E8` | Omnibox text, labels |
| `fg.muted` | `#5C5C5C` | `#A8A8A8` | Placeholder, status band |
| `border.control` | `#8A8A8A` | `#6E6E6E` | Omnibox border, button edges |
| `accent` | `#0B57D0` | `#7CB0FF` | Mode chip text, trail marker of the current entry |
| `focus` | `#0B57D0` | `#7CB0FF` | Shell focus ring (2 px) |
| `danger` | `#B3261E` | `#FF897D` | Error page headline, `ERR_*` code |

Light mode is the default; `prefers-color-scheme` is evaluated against the OS
setting via `engine-style::media_eval` if the shell reuses that module, else by
reading the Win32 app-theme value — `verify` which is cheaper.

**Type scale (shell only):** 13 px label, 13 px omnibox, 12 px status/code,
20 px error headline. Family: `ui-sans-serif` (→ `segoeui.ttf` per
`ARCHITECTURE.md` §9.1).

---

## 8. Acceptance criteria (Bob-testable)

Each criterion names the observable and the tolerance. "Pass" means the test can be
run without a human in the loop unless marked *manual*.

### 8.1 Shell boot

| ID | Criterion | How to test |
|---|---|---|
| B1 | Header paints by 250 ms p95 on this machine, cold, with the warm font cache disabled | Instrumented start-to-first-frame timestamp written to `RenderReport.counts`; run 20 times, take p95 |
| B2 | If font enumeration exceeds 60 ms, the header still paints with a fallback face | Force a delay in the font loader in a test build; assert the header display list exists before the faces arrive |
| B3 | No window state is ever a white rectangle for > 250 ms after window show | Golden-frame capture at 4 ms intervals for 1 s (manual, once) |

### 8.2 Navigation

| ID | Criterion |
|---|---|
| N1 | `Ctrl+L` focuses the omnibox and selects all; `Esc` restores the current URL and blurs |
| N2 | Enter on `example.com` (no scheme) navigates to `http://example.com/` — `verify` against the URL parser's policy: **write the test first**, because the scheme default is a policy decision, not an accident |
| N3 | Enter on `hello world` shows the mode chip as `Not a URL` and does not navigate; the omnibox keeps focus |
| N4 | Back/forward change nothing in the address field until the new document has a layout tree, then update it |
| N5 | Back to a cached entry paints the first frame in ≤ 120 ms and restores the exact scroll offsets of the departure point |
| N6 | Stop during a load leaves the previous document on screen; the next navigation still works |
| N7 | `F5` on `about:report` re-renders the report for the *current* document without refetching it |

### 8.3 Honest failure

| ID | Criterion |
|---|---|
| F1 | A fetch failure shows `NetError` with the status code visible as text, the URL verbatim, and exactly 3 actions |
| F2 | A JS-only page shows `RenderUnsupported`; the script count in the copy equals the number of `<script>` elements in the parsed DOM |
| F3 | `F12` toggles a report that lists, at minimum, every `Stubbed(feature)` approximation the load produced, and `F12` again returns to the intact document |
| F4 | Every internal page's CSS uses only supported declarations — a CI test parses `about:*` stylesheets and asserts zero `Unsupported` results from `engine-style::apply` |

### 8.4 Accessibility

| ID | Criterion |
|---|---|
| A1 | Every chrome control is reachable and operable with `Tab`/`Shift+Tab`/`Enter`/`Space` alone; the focus ring is drawn by the shell and visible on all 5 controls |
| A2 | Contrast of every token pair in §7 meets AA; a unit test computes the ratio |
| A3 | At 200 % DPI the header is still 36 CSS px tall (not 36 device px) and no text is clipped — assert against `shell_rects` output |
| A4 | With reduced motion on, no shell element animates; assert by inspection of the display list for time-varying items |
| A5 | IME pre-edit text is underlined in the omnibox (manual, one language with a composition step — e.g. Japanese — **note: CJK is a supported shaping class, so this is achievable**) |

### 8.5 Performance gates (v1 ship blockers)

| Metric | Budget | Source |
|---|---|---|
| Start → header first frame | ≤ 250 ms p95 | B1 |
| Start → new-tab first frame | ≤ 400 ms p95 | B1 |
| Input → omnibox caret moves | ≤ 16 ms p95 (measured as input event to presented frame) | new |
| Scroll frame on a 2000-box fixture doc | ≤ 16.7 ms p95 | `ARCHITECTURE.md` §11 scroll-repaint budget (3 ms) plus present |
| Back to cached entry | ≤ 120 ms | `ARCHITECTURE.md` §11 session restore |
| Shell RSS at new tab | ≤ 120 MB | measured baseline; record the number, do not guess |

If **any** gate misses on the cold path, the feature is not shipped. The owner's
standing rule applies: measure before claiming.

---

## 9. What we keep / what we kill / what we invent / what could go wrong

**Keep**
- The 36 px header band, icon order, and mode chip from `native-browser/ui/index.html`.
- The four-action error page, minus the tab-specific action.
- Ctrl+L / Alt+Left / F5 / F12 muscle memory.
- Geometry as pure functions (`layout.rs` style), unit-tested without a window.
- The render report as a *user-visible* artifact, not a debug log.

**Kill**
- The tab strip and its rail. Tabs are a window list; we are not shipping the thing
  the thesis rejects. The history trail replaces it in v1.
- Spinners and skeleton screens. A static renderer pretending to stream is a lie.
- Any control that exists but does nothing (the new-tab "mirror" field is the model
  of what not to do — in the new shell that field is live, and if it were not, it
  would not be drawn).
- The two-webview split and the 55-verb IPC surface. The verbs are internal function
  calls again.

**Invent**
- **The history trail in the status band** — position, not just presence, and
  keyboard-jumpable (`Alt+1…5`).
- **`about:report` as a first-class page**, with the report rendered by the engine
  itself (dogfood: the report page is the best stress test we have).
- **The "Not a URL" chip state** — refuse to guess instead of silently searching.
- **v1.1: Reader mode** — the single highest-value feature the engine's limitations
  argue for, because it converts partially-rendered pages into correctly-rendered ones.

**What could go wrong** (ranked by how much it would cost)

1. **Layout never lands, and the shell becomes a demo of a header bar.** As of this
   session `engine-layout` is a stub and the previously-present `block.rs` /
   `box_tree.rs` / `fragments.rs` are gone and untracked by git. If the layout work
   is not recovered, stages 3–4 of §11 cannot start and the product is a window with
   a nice toolbar and a permanent "no page loaded" message. **This is the risk that
   outranks all others**, and it is why stage 0 is deliberately small and provable.
2. **The shell soaks up work that belongs to layout.** It is tempting to bridge the
   gap by drawing documents with `FillRect`s in the shell. That would produce a
   prototype that cannot ever display a real page and would have to be thrown away.
   The `ContentSource` enum is the boundary; a `DisplayList` of page content may
   **only** come from the layout+paint pipeline.
3. **No focus ring in content.** If `:focus-visible` is not plumbed, keyboard users
   cannot see where they are in a page (every page's `outline` is dropped by
   `apply.rs`). v1 must either plumb focus or say so loudly on the new tab.
4. **Font enumeration stalls the first frame.** If it is 500 ms, the shell's
   credibility is gone before the first page. Budget B2 exists for this. Measure
   `FontContext::new()` before writing the boot sequence code.
5. **Screen readers cannot see the shell at all.** `ARCHITECTURE.md` specifies no
   platform accessibility bridge (UIA on Windows). A shell drawn entirely by our own
   rasterizer is, by default, an opaque bitmap to assistive technology. Until a UIA
   provider exists, keyboard-only operation works and screen-reader operation does
   not. Say that on `about:newtab` rather than letting a user discover it.

---

## 10. Accessibility & inclusivity check

| Interaction | Keyboard-only path | Screen reader | Reduced motion | Contrast | Zoom legibility | Pointer-free |
|---|---|---|---|---|---|---|
| Address entry | `Ctrl+L`, type, Enter | Omnibox exposes its text as a `textbox` role; the shell must publish an accessibility tree (platform accessibility is **not** in `ARCHITECTURE.md` — `verify`; without it the shell is invisible to screen readers) | N/A | `fg.primary` on `bg.surface` | Text at 200 % DPI re-lays out; omnibox is one row with horizontal scroll, so it never clips | Yes |
| Back / forward / reload | Tab + Enter, plus `Alt+←/→`, `F5` | Button role + name + disabled state | N/A | `focus` ring 2 px, 3:1 against `bg.chrome` | Icon 14 px → 28 px device at 2×; hit target 28 CSS px (below the 44 px touch guideline — **document as mouse-targeted**) | Yes |
| History trail | `Alt+1…5` | List role with the current entry marked `aria-current` equivalent | N/A | `accent` marker ≥ 3:1 | Text 12 px min; truncate host, never the path tail | Yes |
| Render report | `F12`, then Tab through the table | Table semantics: header row, one row per approximation | N/A | Row text on `bg.chrome` | Report body scrolls, never clips | Yes |
| Error page actions | Tab + Enter | `alert` role on load; each action a named button | N/A | `danger` for the headline, `fg.primary` for body | Buttons min 32 px CSS | Yes |

**Zoom legibility is the canvas thesis's known death trap.** In v1 there is no
canvas zoom, so this row is satisfied trivially: the only zoom is DPI. The rule for
v2 stands: **no zoom level may render text below 12 device px or reuse a glyph
bitmap across a size change.** That constraint alone is why §5.2 defers the canvas.

---

## 11. Roadmap

> **Ordering is now set by the verified build state, not by ambition.** Stages 0–2
> need only modules that exist. Stage 3 is blocked on `engine-layout`.

| Stage | Deliverable | Exit criteria | Blocked on |
|---|---|---|---|
| **0 — now** | `shell-ui` crate: `shell_rects`, a header display list, and a golden harness (`Surface` + `verify_determinism` + PNG out) | Golden PNG of the header at 96 and 192 DPI; B1–B3, A1–A4 | Nothing |
| **1** | Omnibox editing: caret, selection, horizontal scroll, IME pre-edit underline; `Ctrl+L`/`Esc`; mode chip | N1, N3, A5 | Nothing |
| **2** | Session history + trail band + `ContentSource::Message` for errors; window/DPI/focus events via `engine-platform` | N4–N6, F1, §8.5 input-latency gate | Nothing (fetch may be stubbed) |
| **3** | Real page content: `ContentSource::Document` filled from layout; `about:newtab`/`about:report` authored in the supported CSS subset | N2, N7, F2–F4; §8.5 scroll gate | **`engine-layout` re-landing** |
| **4 — v1 ship** | Reader mode deferred; all gates green | All §8.5 gates on cold path | Stage 3 |
| **v1.1** | Reader mode (article extraction as an internal page) | A JS-heavy news article renders as readable text | Stage 3 |
| **v2** | Session map, split view, document minimap, timeline scrub | Each gated on its named engine addition in §5.2 | Stage 3 + named additions |

**The single v1 feature to build first, if only one were possible:** the **header band
— geometry, display list, golden PNG, and the 28 px focus ring — with no omnibox
editing and no navigation.**

Why this and not the omnibox: the header is the only piece that can be *proven
correct today*, because it exercises the entire chain that already works
(`shell_rects` → `DisplayItem` → `render` → `Surface` → PNG) with zero dependency on
layout, fetch, or input. It produces the golden test that every later feature extends,
and it is the surface every other control lives in — including the omnibox, which is
simply the largest rect in the band. Building the omnibox first would mean building
the band anyway, then lashing an editor onto an untested base. Build the base.