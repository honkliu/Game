# native-browser — DESIGN.md

Rust shell + WebView2 (Chromium via WRY/TAO). Shell UI: plain HTML/CSS/TypeScript, no framework.
Single delivery cycle. Everything below is in-scope unless it appears under **Out of scope**.

---

## 0. Assumptions (locked)

| # | Assumption | Consequence |
|---|---|---|
| A1 | One `tao` window, one **tab webview per tab**, plus one **shell webview** (HTML/CSS/TS) occupying the chrome band and overlays. | Shell overlays (palette, find, downloads) are HTML inside the shell webview. Never composited over page webviews — WebView2 child HWNDs do not alpha-composite over a parent surface. |
| A2 | Tabs = vertical strip **+** horizontal header band. The **top strip is horizontal** (title only) and the **left rail is vertical**. | Both layouts are cheap; switching is a runtime flag. Not two products. |
| A3 | "Canvas/spatial browsing" is **not** delivered. Tabs are a linear list with group nesting. | See §2 trade-offs. A zoomable canvas of live WebView2 surfaces is a multi-cycle project and fails the latency budget in §1. |
| A4 | No remote services, no accounts, no telemetry, no sync, no auto-update. | Network egress = page loads + search query dispatch only. |
| A5 | Target: Windows 10 1809+/Windows 11, WebView2 Evergreen runtime present (verified at startup, otherwise hard error dialog with runtime download hint — no bundled installer). | No runtime install flow in v1. |

---

## 1. Win conditions

Each AXIS is a number or a testable assertion. **Measurement method is named for every one.** No competitor numbers are asserted; only our targets and how they are measured.

| # | Axis | Target (pass/fail) | Measurement method |
|---|---|---|---|
| W1 | Cold start → first shell paint | **≤ 400 ms** p95, warm disk, 8-core/16 GB reference machine. Process start → first `requestAnimationFrame` timestamp in shell reported via `performance.now()` + process `QueryPerformanceCounter` anchor. | 20-run scripted loop; record p50/p95/max in `bench/startup.json`. |
| W2 | Cold start → first *tab* paint (page content visible, blank `about:blank`) | **≤ 900 ms** p95 same machine. | Same harness; shell emits `shell.ts` and tab webview emits `NavigationCompleted`; both correlated to process start. |
| W3 | Keystroke → glyph in omnibox | **≤ 16 ms** p95, **≤ 33 ms** p99, 200-char paste and single-char typing. | Synthetic `SendInput` at 10 Hz; shell records `keydown` → next `requestAnimationFrame` paint; instrument via `PerformanceObserver` long-task + custom marks. Harness in `bench/input_latency.rs`. |
| W4 | Keystroke → command palette results visible | **≤ 50 ms** p95 for history-only search over 50k entries. | Same harness; DB query time included. Gate: DB query p95 ≤ 15 ms. |
| W5 | Memory at N tabs | **≤ 150 MB RSS at 1 tab (about:blank), ≤ 900 MB at 20 tabs, ≤ 2.0 GB at 100 tabs**, measured as sum of the browser process tree minus WebView2 renderer processes for blank tabs. Sleep tabs after 5 min idle. | `bench/memory.rs` spawns, `tasklist`/ETW process-tree RSS sum, exclude `msedgewebview2.exe` renderer children of slept tabs (their RSS target: ≤ 60 MB). Report the tree breakdown, not a single number. |
| W6 | Tab switch | **≤ 8 ms** p95 for a live tab (show/hide HWND), **≤ 120 ms** p95 for a slept tab (full reload acceptable). | Harness: `Ctrl+Alt+Right` rapid switch loop; frame timing from shell. |
| W7 | Zero vendor network calls | **0 requests** to non-page, non-search origins observed on a blank session. Assert: process-started network capture for 60 s idle produces zero HTTP(S) requests until a page is loaded. | `bench/net_audit.ps1` — Windows `netsh trace` + `pktmon` or a loopback proxy set as WebView2 proxy for the session; startup also logs every outbound connection with a tagged origin. **Testable assertion**, not a marketing claim. |
| W8 | Shell render of 1,000 history/palette rows | **≤ 50 ms** first paint of palette with 1,000 rendered rows; **≤ 16 ms** per filter keystroke. Windowed rendering required. | Harness drives the shell UI directly in Edge (same engine) with a fixture DB. |
| W9 | Accessibility: full keyboard operation | Every action reachable by keyboard, all states reachable without pointer, screen reader announces focus changes. **Testable assertion:** a 60-step keyboard-only scripted walkthrough passes with zero pointer events. | `e2e/keyboard_only.spec.ts` (Playwright drives the shell UI; native key events injected by harness). |
| W10 | Process footprint at rest | **≤ 3 OS processes** at 1 tab: `native-browser.exe`, `msedgewebview2.exe` (browser), `msedgewebview2.exe` (renderer). Note: WebView2 will add helper processes (GPU, utility) — the target must be **measured** and reported as the count, not assumed. | `bench/memory.rs` process list. |

Targets are **statements about our build**, published in `bench/report.md` on every CI run. Competitor numbers are not claimed.

---

## 2. Chrome/Edge competitive audit

Observation only. These are the defects worth fixing; one-line fix each.

| # | Browser | Defect | One-line fix |
|---|---|---|---|
| D1 | Chrome | Omnibox is a URL bar with search bolted on; commands need a separate, undiscoverable page (`chrome://commands` behavior is inconsistent across versions). | One input field; `> ` prefix flips to command mode; the same field always accepts URL, search, or command — mode shown by an inline chip. |
| D2 | Edge | Context menus are 3+ levels deep and position-dependent; right-clicking a tab vs. a page vs. a link returns different top-10 items with no consistency. | Single `Space` context menu per *item kind*, max 8 items on level one, everything else behind "More". |
| D3 | Chrome | Tab strip overflow degrades to a horizontal scroll-shuffle of shrinking pills; beyond ~25 tabs the close target is under 8 px. | Vertical tab rail: fixed row height 28 px, close target 20×20 px always, never shrinks; scrolls instead of compressing. |
| D4 | Edge | Sidebar (Copilot/Discover) is always present in the chrome band and costs vertical space even when unused. | The rail is content-only and collapses to a 44 px icon bar; **zero** vendor content, ever. |
| D5 | Chrome | Downloads panel is a bottom bubble with an implicit "open vs showin folder" ambiguity and no keyboard path to reveal the file. | Downloads is a docked panel with an explicit per-row action list; `Ctrl+J` focuses it, `Enter` opens, `Alt+Enter` reveals. |
| D6 | Edge | Sleeping tabs restore with a visible content flash and lose scroll position inconsistently across versions. | Sleep only after 5 min idle **and** never the pinned/audible/foreground tab; restore captures scroll offset in the session record. |
| D7 | Chrome | Find-in-page is a floating box that occludes the page and has no count unless a match is selected; no regex, no highlight-all toggle in the UI. | Docked find bar in the chrome band, live `n/N` count always shown, `Alt+A` highlight-all, `Alt+R` regex toggle. |
| D8 | Chrome | History has no fast path from omnibox to "all history for this site" or "all history this hour". | Palette filters `site:`, `before:`, `after:`, `type:` as first-class tokens; history panel reuses the same query syntax. |
| D9 | Edge | Vertical tabs are implemented as a mode that hides the horizontal strip but keeps all the horizontal-strip semantics (including the shrunken pill ordering). | Rail is the primary surface: ordered, pinnable to top, collapsible groups, and independent of the header band. |
| D10 | Chrome | Error pages ("This site can't be reached") give one retry button and no diagnostics the user can act on; the URL is not selectable/copyable as text. | Error page: URL in a read-only selectable field, error code, `Retry` `Copy URL` `Open in new tab` `Details` (code only, no requests made). |

---

## 3. Interaction model

### 3.1 Window layout

```
┌──────────────────────────────────────────────────────────────────┐
│ HEADER BAND (36 px)                                              │
│ ┌────────┬────────────────────────────────────────────┬─────────┐│
│ │ rail   │ [◀][▶][⟳]  omnibox (flex)  [⇩ 3][☰]       │ — □ ×  ││  <- row is the omnibox row
│ │toggle  │                                            │         ││
│ └────────┴────────────────────────────────────────────┴─────────┘│
├──────────┬───────────────────────────────────────────────────────┤
│ RAIL     │                                                       │
│ 220 px   │  page webview (one child HWND per tab, only the       │
│ collaps- │  active one visible)                                  │
│ to 44 px │                                                       │
│          │                                                       │
│ [pinned] │                                                       │
│ [group]  │                                                       │
│ [tabs]   │                                                       │
│          │                                                       │
└──────────┴───────────────────────────────────────────────────────┘

Overlay surfaces (HTML in shell webview, sized to a rect inside the page area but drawn above it):
  - omnibox dropdown: anchored under omnibox, max-height 480 px
  - command palette: centered, 640 px wide
  - downloads panel: right docked, 360 px
  - history panel: full page-area overlay
  - find bar: docked row at TOP of page area (between header and page), 40 px
```

**Critical constraint:** a child webview is an OS-level surface. Overlays **cannot** be drawn over the page area by CSS alone. Overlays are drawn by the shell webview in the region *above* the page webview, plus in an **overlay webview** — a second, transparent-background `WebView2` child placed on top of the page webview, sized to the overlay rect only (0×0 when no overlay is open). This is the only reliable mechanism under WRY/TAO; the alternative (resizing the page webview to make room) is used for the **find bar** and **docked panels**, which are in normal layout flow.

Rule: any surface that *must* float over page content → overlay webview. Any surface that can push page content → normal flow in the shell/page-area rect.

### 3.2 Tab strip (rail)

- Rows, top to bottom: pinned tabs zone → groups → ungrouped tabs → "new tab" row.
- Row height 28 px; compact mode 24 px. Icon 16×16, title single line, ellipsis. Close button appears on hover/focus, 20×20 hit target, always present in DOM (opacity 0 → 1) so it is keyboard reachable.
- Groups: collapsible header row (24 px), chevron, group color dot, name. Indented children by 12 px. One level of nesting only.
- Overflow: rail scrolls; no shrinking. Scrollbar appears on hover, 8 px wide.
- Drag: reorder, drag into/out of group (drop indicator 2 px accent line). Drag to the header band = tear-off is **out of scope**.
- Pinned tabs render in a 3-column icon grid (16 px icons, 44 px cells) above the tab list when the rail is collapsed; when expanded, pinned render as full rows at the top with a separating rule.

### 3.3 Omnibox

Single text input. The left of the field shows a **mode chip**: `URL`, `Search`, `› Command`, `🔎 Find`, `# Tag`. The chip is not clickable to change mode — modes are entered by prefix or by key, so the state is always typeable.

Mode resolution (deterministic, evaluated in order):
1. Starts with `> ` → **command mode**. Slot 1 is always "Run command: &lt;literal&gt;".
2. Starts with `? ` → force **search**.
3. Starts with `/ ` → **find in page** with the rest of the text.
4. Contains a `:` scheme, or is `localhost`/`localhost:port`, or is an IPv4 with optional port → **URL**.
5. No space and contains `.` with a valid TLD-shaped last label → **URL** candidate (still offers search below).
6. Otherwise → **search** (default engine) with URL fallback if resolvable.

Dropdown states, exactly one active:
| State | Rows (max visible 8, scrollable) | Row anatomy |
|---|---|---|
| `empty` | 1: top history (5) + 1: "Search for …" (1) | `[★] title — url` |
| `history-match` | up to 8, fuzzy, ranked by (recency × visit count × match quality) | `[★] title` + `url` dim |
| `search-suggest` | 1: "Search &lt;engine&gt; for &lt;query&gt;" — **no remote suggest API** | `[🔍] Search …` |
| `url` | 1: "Go to &lt;normalized url&gt;" | `[↗] Go to …` |
| `command` | up to 8 commands, fuzzy | `[›] Verb — description` |
| `find` | live match count only, no dropdown (routes to find bar) | — |

Selection: `↑`/`↓` move, `Tab` completes the highlighted row into the field *without* executing, `Enter` executes the highlighted row (default = row 0), `Esc` closes the dropdown and keeps text, second `Esc` reverts to the pre-focus text.

### 3.4 Command palette vs. omnibox

**Decision: one field, two presentations.** `Ctrl+K` opens the **palette** (centered, 640 px, overlay webview) pre-filled with `> `. The omnibox and the palette are the same component and the same state machine; the palette is the omnibox with `mode = command|all` and a larger result list that also includes history and open tabs.

Rationale: two independent surfaces with two ranking engines produce two truths and double the keyboard muscle memory. Trade-off: the palette is not a "power-user-only" surface with distinct ranking weights — accepted.

`Ctrl+K` from the palette when already open = no-op (idempotent). `Esc` closes palette, restores focus to the page. `Enter` in the palette executes and closes.

### 3.5 Keyboard map (complete — every binding)

Modifiers on Windows: `Ctrl`, `Alt`, `Shift`, `Win` (Win not used). `Alt+RMB` is out of scope.

| Key | Action |
|---|---|
| `Ctrl+T` | New tab (about:blank, focused omnibox) |
| `Ctrl+W` | Close current tab |
| `Ctrl+Shift+W` | Close window |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab (MRU order is **not** used; spatial order) |
| `Ctrl+1` … `Ctrl+8` | Jump to tab 1–8 |
| `Ctrl+9` | Jump to last tab |
| `Ctrl+Alt+←` / `Ctrl+Alt+→` | **Rapid switch** (same as Ctrl+Tab but usable while a focused page consumes Ctrl+Tab — reserved page-level fallback) |
| `Ctrl+PgUp` / `Ctrl+PgDn` | Next / previous tab (duplicate of Ctrl+Tab for muscle memory) |
| `Ctrl+Shift+T` | Reopen last closed tab (stack of 25) |
| `Ctrl+L` | Focus + select-all omnibox |
| `Alt+D` | Same as `Ctrl+L` |
| `F6` | Cycle focus: page → omnibox → rail → page |
| `Ctrl+K` | Open command palette (`> ` prefilled) |
| `Ctrl+Shift+K` | Close palette (same as `Esc`) |
| `Ctrl+F` | Find in page |
| `F3` / `Shift+F3` | Find next / previous |
| `Ctrl+G` / `Ctrl+Shift+G` | Find next / previous (duplicate) |
| `Alt+A` | Toggle highlight-all in find |
| `Alt+R` | Toggle regex in find |
| `Ctrl+J` | Open downloads panel |
| `Ctrl+H` | Open history panel |
| `Ctrl+Shift+H` | Open history panel filtered to "today" |
| `Alt+←` / `Alt+→` | Page history back / forward |
| `Alt+Home` | Go to new-tab page |
| `Ctrl+R` / `F5` | Reload |
| `Ctrl+Shift+R` / `Shift+F5` | Reload ignoring cache |
| `Ctrl+D` | Bookmark (star) current page |
| `Ctrl+Shift+D` | Bookmark all tabs into a new group |
| `Ctrl+U` | View source in new tab (`view-source:` where supported; fallback: error page with message) — **verify** WebView2 support at build time; if unsupported, binding is removed from the UI, not silently dead |
| `Ctrl+Plus` / `Ctrl+Minus` / `Ctrl+0` | Page zoom in / out / reset |
| `Ctrl+Shift+T` | Already used (reopen). Page zoom via keypad `+` only. |
| `Ctrl+,` | Settings |
| `Esc` | Close topmost transient surface; when none: stop page load |
| `Space` | When rail is focused: context menu for focused row |
| `Shift+F10` / context-menu key | Same as `Space` for focused row |
| `Enter` | Activate focused control |
| `Delete` | In rail: close focused tab |
| `Ctrl+Shift+E` | Collapse/expand rail |
| `Ctrl+Shift+P` | Pin/unpin current tab |
| `Ctrl+\` | Toggle split view — **out of scope**, reserved, not bound |
| `Ctrl+W` inside find bar | Falls through to close tab only when find input is empty; otherwise deletes word. Rule: inputs with `data-intercept-ctrl-w="word"` consume `Ctrl+W`. |

Reserved-but-unbound conventions: `Ctrl+Shift+[1–9]` group switching, `Ctrl+Alt+T` tear-off. Not in v1.

### 3.6 Focus model

Four focus scopes, in tab order: `page → omnibox → rail → page`. `F6` cycles. `Ctrl+L` always lands on omnibox. `Esc` from omnibox returns to page (last-focused element). The rail is a `role="listbox"` with roving tabindex; arrow keys move, `Enter` activates, `Delete` closes, `Space` opens the item menu.

The overlay webview only takes focus when an overlay is open; on close it returns focus to the shell input that opened it, then to the page on the second `Esc`.

**Page webview focus containment:** when the page webview has OS focus, shell shortcuts do not fire. Therefore every shell shortcut is also registered as a `WebView2` accelerator on the page webviews (WebView2 `AcceleratorKeyPressed` / WRY equivalent — **verify the WRY API surface allows registering global accelerators across child webviews**; if not, the fallback is a low-level keyboard hook limited to this process). `Ctrl+Alt+←/→` exists because a small set of pages (games, editors) swallow `Ctrl+Tab`.

---

## 4. Screen-by-screen spec

Common tokens referenced as `--token` (defined in §5).

### 4.1 New-tab page (`about:blank` shell-rendered variant: internal `native://newtab`)

Rendered as a tab page webview like any other page (same origin rules, no privileged API).

```
┌──────────────────────────────────────────────────────┐
│                                                      │
│                 [clock 32/40]                        │  centered column, 720 px max
│                                                      │
│        ┌────────────────────────────────┐            │  omnibox mirror, 560 px, 44 px
│        │ Search or type a URL           │            │  focus transfers to real omnibox on click
│        └────────────────────────────────┘            │
│                                                      │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐         │  pinned/bookmark tiles, 4 per row, 120×96
│  │ tile   │ │ tile   │ │ tile   │ │ tile   │         │
│  └────────┘ └────────┘ └────────┘ └────────┘         │
│                                                      │
│  Recent                                                │  max 6, 40 px rows
│  ○ Title                            site — 2h ago     │
└──────────────────────────────────────────────────────┘
```

Controls: clock (text), search mirror (button → focuses omnibox), tiles (links), recent rows (links), "Show all history" link → history panel.
Keyboard reachability: `Tab` cycles all; the mirror is first. No pointer-only control.
Tokens: background `--bg-page`, card `--bg-surface`, text `--fg-primary` / `--fg-muted`.

### 4.2 Tab rail states

| State | Spec |
|---|---|
| **Empty** | One row: `+ New tab`. Below: nothing. No "no tabs" empty-state illustration. |
| **Normal** | Rows 28 px, icon 16, title 13/18, close 20×20 right-aligned, active row `--bg-surface-active` + 2 px left accent `--accent`. |
| **Overflow** | Vertical scroll only. Scroll shadow 8 px at top/bottom when scrollable. Row height never changes. |
| **Pinned** | Rows above a 1 px divider `--border-subtle`, no close button until hover, load-spinner→favicon replaced permanently by favicon, pin glyph on hover. |
| **Grouped** | Header row 24 px: chevron 12, color dot 8, name 12/16 semibold, count 12/16 muted. Children indented 12. Group drop zone highlights `--accent-15`. |
| **Sleeping** | Row at 0.6 opacity, title italic-free, a 2 px dot `--fg-subtle`; hover restores. |
| **Collapsed rail** | 44 px: pinned grid 44×44 cells (3 per row), then favicons 44×44 for unpinned, vertical scroll. New-tab `+` at bottom fixed. |

Keyboard: `↑/↓` roving, `Home`/`End`, `Enter` switch, `Delete` close, `Space` menu, `Ctrl+Shift+E` collapse.

### 4.3 Omnibox dropdown

- Width = omnibox width, min 480, max 900. Top offset 4 px below field. Surface `--bg-overlay`, radius 8, elevation 3, border `--border-subtle`.
- Row 40 px: 16 px leading glyph, title 13/18 `--fg-primary`, secondary line 12/16 `--fg-muted`, trailing hint 11/14 `--fg-subtle` (e.g. `Tab`, `Enter`).
- Selected row: `--bg-surface-active`, 2 px left bar `--accent`.
- Footer 28 px: mode chip + `↑↓ navigate · Tab complete · Enter go · Esc dismiss`.
- States exactly as §3.3. Empty state is not a separate screen — it is `empty` rows.

### 4.4 Downloads panel

Right-docked, 360 px, inside page-area rect (page webview shrinks; no overlay webview).

```
Downloads                                   [Clear all] [✕]
───────────────────────────────────────────────────────────
 file.pdf                                  12.4 MB
  C:\Users\…\Downloads\file.pdf
  ● downloading 62%            [Pause] [Cancel]
───────────────────────────────────────────────────────────
 setup.exe                                  3.1 MB
  Ready                       [Open] [Show in folder] [✕]
───────────────────────────────────────────────────────────
 archive.zip  — cancelled                     [Retry] [✕]
```

Controls per row: name (button: open), size, path (button: reveal), state, actions. Empty state: single line "No downloads." + a `Ctrl+J` hint.
Keyboard: `↓/↑` row focus, `Enter` open, `Shift+F10` menu, `Delete` remove from list (not delete file — confirm inline `Remove from list? [Remove] [Cancel]`).
Requires a `download` contract event from WebView2 with progress — see §6 `download.*`.

### 4.5 History panel

Full page-area overlay (overlay webview), replaces page visually.

```
History                                         [search field]  [Close ✕]
──────────────────────────────────────────────────────────────────────
 Today
  10:42  Title of page                          example.com        ⋯
  09:11  …
 Yesterday
  …
```
Grouped by day. Query field supports the same tokens as the palette: `site:`, `before:2026-11-01`, `after:`, `type:`. Rows 36 px, `⋯` opens menu (Open in new tab, Copy URL, Forget this page, Forget this site — all destructive actions have inline confirmation).
Keyboard: `Ctrl+H` opens with field focused; `↑/↓`, `Enter`, `Delete` = forget (with confirm), `Esc` closes.
Capped at 100,000 entries with FIFO eviction; the count is shown in Settings.

### 4.6 Find in page

Docked 40 px row at the top of the page area (page webview shrinks by 40 px; no overlay).

```
[🔎] [case-sensitive field____________________]  3/17  [Aa] [.*] [All] [↑] [↓] [✕]
```
Controls: input, count `n/N` (always visible, `0/0` when empty), case toggle `Aa` (Alt+C), regex `.*` (Alt+R), highlight-all `All` (Alt+A), prev/next, close.
`Enter` next, `Shift+Enter` previous, `Esc` closes and clears highlight. Behavior with regex: **depends on WebView2 find API** — if the API exposes only literal search, then `.*` renames to "Whole word" and the label is changed in the UI so the control never lies (see risk R3).
Highlight color `--find-highlight` (yellow), active match `--find-active` (orange).

### 4.7 Error / failed-load page

Internal page `native://error`, rendered in the page webview on `NavigationCompleted` failure.

```
[ ⚠ ]

Can't reach this page
ERR_NAME_NOT_RESOLVED                          (11/14 muted, monospace)

[ https://example… ]  (44px read-only selectable field, full URL)

[ Retry ]  [ Copy URL ]  [ Open in new tab ]  [ Details ▾ ]

Details: error code, navigation id, timestamp. No request is made to any server.
```
`Retry` = `Ctrl+R`. `Details` expands to a `<pre>` with the code and the URL only. A `dns-failed` variant shows a hint "Check that the domain exists; no DNS fallback search is performed."
No third-party error page, no vendor content.

### 4.8 Settings (`native://settings`)

Single-scroll page, left nav (4 sections) + right content. Persisted to `settings.json` in a fixed app-data path.

| Section | Controls |
|---|---|
| **Appearance** | Theme: Light / Dark / System (radio). Rail expanded / collapsed (toggle). Compact tab rows (toggle). Density: Comfortable / Compact. |
| **Search** | Search engine: a list of one (DuckDuckGo HTML) plus custom URL template field. **No suggest API.** Default is a plain `GET` to the template with `%s`. |
| **Privacy** | "Clear data on exit": history / cookies / cache (3 toggles). Show full URL always (default on, no truncation). Nothing else is collected — a static statement with the list of what is written to disk. |
| **Advanced** | History cap (number, default 100000). Sleep after N minutes (default 5). Download directory (path, `Browse…` uses a native dialog). `Open data folder` (reveals dir). |

All controls keyboard reachable in DOM order, native `<input type="radio">`/`<select>` styled. Every toggle writes immediately (no Save button); a toast "Saved" appears for 1.5 s.

---

## 5. Design tokens

CSS custom properties. `:root` = light, `[data-theme="dark"]` = dark. System theme follows `prefers-color-scheme`.

### 5.1 Color

| Token | Light | Dark |
|---|---|---|
| `--bg-page` | `#FFFFFF` | `#111214` |
| `--bg-shell` | `#F3F4F6` | `#1A1B1E` |
| `--bg-surface` | `#FFFFFF` | `#202226` |
| `--bg-surface-hover` | `#E9EAEE` | `#282B30` |
| `--bg-surface-active` | `#DDE3EE` | `#2E3540` |
| `--bg-overlay` | `#FFFFFF` | `#202226` |
| `--fg-primary` | `#14161A` | `#EDEFF2` |
| `--fg-muted` | `#5A6170` | `#9AA1AE` |
| `--fg-subtle` | `#8A91A0` | `#6C7484` |
| `--fg-on-accent` | `#FFFFFF` | `#FFFFFF` |
| `--border-subtle` | `#E2E5EA` | `#2C3036` |
| `--border-strong` | `#C9CED6` | `#3A3F47` |
| `--accent` | `#2F6BFF` | `#5B8CFF` |
| `--accent-hover` | `#1F5AEE` | `#7AA3FF` |
| `--accent-15` | `rgba(47,107,255,0.15)` | `rgba(91,140,255,0.18)` |
| `--danger` | `#C4314B` | `#FF6B81` |
| `--success` | `#1E7F4B` | `#4FD08A` |
| `--warning` | `#9A6400` | `#E8B04B` |
| `--find-highlight` | `#FFE58A` | `#5C4B00` |
| `--find-active` | `#FFAE2B` | `#A87400` |
| `--scrollbar-thumb` | `rgba(20,22,26,0.22)` | `rgba(237,239,242,0.22)` |
| `--shadow` | `0 8px 24px rgba(15,18,24,0.12)` | `0 8px 24px rgba(0,0,0,0.55)` |

Contrast check: `--fg-primary` on `--bg-surface` = 16.6:1 light / 13.9:1 dark; `--fg-muted` on `--bg-surface` = 5.4:1 light / 5.9:1 dark; `--fg-subtle` used only for non-essential hints, never body text. `--accent` on white = 4.7:1 (text OK at ≥13 px semibold only; accent is otherwise used for fills and 2 px bars).

### 5.2 Typography

```
--font-ui: "Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system, sans-serif;
--font-mono: "Cascadia Mono", "Consolas", ui-monospace, monospace;
```
| Token | Size / line-height / weight |
|---|---|
| `--text-xs` | 11 / 14 / 400 |
| `--text-sm` | 12 / 16 / 400 |
| `--text-sm-strong` | 12 / 16 / 600 |
| `--text-base` | 13 / 18 / 400 |
| `--text-base-strong` | 13 / 18 / 600 |
| `--text-lg` | 16 / 22 / 600 |
| `--text-xl` | 22 / 28 / 600 |
| `--text-display` | 32 / 40 / 300 |
Letter-spacing: `0` for all sizes except `--text-display` = `-0.01em`. No font loading from network; system stack only. Tabular numerals (`font-variant-numeric: tabular-nums`) for clock, counts, sizes, percentages.

### 5.3 Spacing / radii / elevation

```
--space-1: 2px  --space-2: 4px  --space-3: 8px  --space-4: 12px
--space-5: 16px --space-6: 24px --space-7: 32px --space-8: 48px

--radius-sm: 4px   (inputs, chips, rows)
--radius-md: 8px   (panels, dropdowns, palette)
--radius-lg: 12px  (overlay panels, new-tab cards)
--radius-pill: 999px (mode chip, status dots)

--elev-1: 0 1px 2px rgba(15,18,24,0.08)
--elev-2: 0 4px 12px rgba(15,18,24,0.10)
--elev-3: var(--shadow)
```
Density: Comfortable = row 28 px / pad `--space-3`; Compact = row 24 px / pad `--space-2`.

### 5.4 Motion

- Durations: `--motion-fast: 120ms`, `--motion-base: 180ms`, `--motion-slow: 240ms`.
- Easing: `--ease-out: cubic-bezier(0.2, 0, 0, 1)`; `--ease-in-out: cubic-bezier(0.4, 0, 0.2, 1)`.
- Motion only communicates **state change** (overlay open/close, row selection, rail collapse). No decorative loops, no parallax, no animated backgrounds.
- `@media (prefers-reduced-motion: reduce)` → all durations 0 ms except opacity crossfade at `--motion-fast`; no transform motion.
- No sound, no haptics (desktop).

### 5.5 Dimensions

`--header-h: 36px`, `--rail-w: 220px`, `--rail-w-collapsed: 44px`, `--tab-row-h: 28px` (`24` compact), `--find-bar-h: 40px`, `--omnibox-h: 28px`, `--focus-ring: 2px` (`outline: 2px solid var(--accent); outline-offset: 1px`).

---

## 6. Rust-side contract (what Bob exposes to the UI layer)

No code. Three surfaces: (a) host → UI events, (b) UI → host commands, (c) persisted state.

### 6.1 Transport

- Shell/overlay webviews call `window.__native.invoke(cmd, payload)` → JSON over the WRY IPC bridge (`ipc_handler`), returns a JSON promise. All payloads are JSON-serializable, snake_case.
- Host → UI: `window.__native.emit(event, payload)` via `evaluate_script`. UI subscribes with `window.__native.on(event, fn)`.
- **One-way state**: the host owns `Tab`, `Group`, `Download`, `History`, `Settings`. The UI never mutates its own copies; it sends a command and waits for the resulting `state.*` event. Optimistic rendering is allowed **only** for hover/focus/selection, never for tab order or existence.
- IPC is plain JSON; no binary. A `state.tabs` payload for 100 tabs must be ≤ 8 KB — send diffs (`state.tab_updated`) in addition to full snapshots.

### 6.2 Commands (UI → host)

| Command | Payload | Result event |
|---|---|---|
| `nav.open` | `{ url, disposition: "current"\|"new_tab"\|"new_tab_background" }` | `tab.created` / `tab.navigated` |
| `nav.back` / `nav.forward` / `nav.reload` / `nav.reload_ignoring_cache` / `nav.stop` | `{ tab_id }` | `tab.navigated` or `tab.loading` |
| `tab.select` | `{ tab_id }` | `tab.activated` |
| `tab.close` | `{ tab_id }` | `tab.closed` |
| `tab.move` | `{ tab_id, before_tab_id \| after_tab_id, group_id \| null }` | `state.tabs` |
| `tab.pin` / `tab.unpin` | `{ tab_id }` | `state.tabs` |
| `tab.reopen_closed` | `{}` | `tab.created` |
| `group.create` | `{ name, color, tab_ids }` | `state.groups` |
| `group.rename` / `group.set_color` / `group.collapse` / `group.delete` | `{ group_id, … }` | `state.groups` |
| `find.start` / `find.query` / `find.next` / `find.prev` / `find.stop` | `{ tab_id, query, case_sensitive, regex_or_whole_word }` | `find.result { active, total, supported_flags }` |
| `zoom.set` / `zoom.reset` | `{ tab_id, factor }` | `tab.zoom_changed` |
| `omnibox.query` | `{ text, limit }` | `omnibox.results { mode, rows[] }` — **host** performs history search and URL normalization so the ranking lives Rust-side and is unit-testable |
| `history.query` | `{ tokens, limit, offset }` | `history.results` |
| `history.forget` | `{ entry_ids[] \| { site } }` | `state.history_meta` |
| `history.clear` | `{ scope: "all"\|"range", range? }` | `state.history_meta` |
| `download.action` | `{ download_id, action: "open"\|"reveal"\|"pause"\|"resume"\|"cancel"\|"retry"\|"remove" }` | `state.downloads` |
| `download.clear_list` | `{}` | `state.downloads` |
| `settings.update` | `{ key, value }` | `state.settings` |
| `shell.focus_omnibox` / `shell.toggle_rail` / `shell.open_palette` / `shell.close_top_overlay` | `{}` | `shell.focus_changed` |
| `window.minimize` / `window.toggle_maximize` / `window.close` | `{}` | — |
| `diagnostics.self_test` | `{}` | `diagnostics.report { startup_ms, tabs, process_list, egress_log }` — backs W7/W10 |

`Ctrl+W`-style shortcuts that originate in the **page** webview are intercepted host-side (accelerator) and delivered to the UI as `shell.shortcut { name }` before the page sees them, except those explicitly allowed to the page (see `data-intercept` rule §3.5).

### 6.3 Events (host → UI)

`tab.created`, `tab.closed`, `tab.activated`, `tab.navigated { url, title, can_go_back, can_go_forward, is_loading }`, `tab.loading`, `tab.crashed`, `tab.sleeping_changed`, `tab.zoom_changed`, `tab.favicon_changed`, `state.tabs`, `state.groups`, `state.downloads`, `state.history_meta`, `state.settings`, `omnibox.results`, `find.result`, `nav.failed { code, url, navigation_id }`, `shell.focus_changed`, `shell.shortcut`, `diagnostics.report`, `app.ready`.

`tab.navigated` is coalesced: at most one per 100 ms per tab; the UI renders the latest.

**Favicons:** host provides `tab.favicon_changed { tab_id, data_url }` with a ≤ 4 KB `data:` URL. UI never fetches favicons itself (no extra network).

### 6.4 Persisted state

One `session.json` (sub-second writes debounced 500 ms, atomic replace) + one `history.sqlite` (or sled — Bob's choice, but query p95 ≤ 15 ms over 50k rows is the gate) + `settings.json`.

| Store | Contents |
|---|---|
| `session.json` | window rect/maximized, rail collapsed, theme override, ordered tab records `{ id, url, title, scroll_y, group_id, pinned, zoom, last_active_at }`, group records, active tab id, closed-tab stack (25), downloads list (non-authoritative status). |
| `history.sqlite` | `(id, url, title, visited_at, visit_count, typed_count)`, index on `visited_at`, FTS index on title+url for the omnibox. |
| `settings.json` | every Settings control in §4.8. |

**Session restore budget:** restore ≤ 250 ms p95 to a visually complete rail (titles from session.json, no page loads). Pages load lazily: active tab eager, others on first activation unless the tab was pinned/audible. Scroll offset is applied after `NavigationCompleted` via script injection `window.scrollTo(0, y)` and must survive; if injection fails, the failure is logged and the UI does not claim restored position.

**Startup path:** read settings → open main window → create shell webview (paint header + rail skeleton from `session.json` before any tab webview is created) → create active tab webview. This ordering is what makes W1 reachable.

**Security:** the shell/overlay webviews load only bundled `asset://` content; no remote navigation is permitted in them (host rejects any navigation whose origin is not the asset origin). The page webviews are unprivileged. IPC is available **only** on shell/overlay webviews; page webviews get no `__native` and no host objects.

---

## 7. What we keep, kill, invent, and what could go wrong

**Keep:** address bar as the single text entry point; `Ctrl+Tab`/`Ctrl+T`/`Ctrl+W`/`Ctrl+L` semantics; a vertical tab rail; pinned tabs; history + downloads as first-class lists; a find bar; WebView2/Chromium rendering and its security model.

**Kill:** horizontal overflowing tab pills; vendor sidebar content and any vendor-origin network call at idle; remote search suggestions; telemetry; per-tab "first run" screens; error pages that only offer one button; a separate, second command surface with distinct ranking.

**Invent (relative to Chrome/Edge):** one field with an explicit, typeable mode chip (URL / Search / Command / Find); palette and omnibox as one state machine with history search performed in Rust; rail rows that never shrink (fixed 28 px, 20 px close target at 100 tabs); `Ctrl+Alt+←/→` page-swallowing fallback; a documented, testable zero-egress assertion in the shipped product (`diagnostics.self_test`); downloads with per-row explicit actions; a local-only search engine default (DuckDuckGo HTML endpoint) with a user-set template.

**Could go wrong:**
- **R1 — Overlay webview focus flicker.** Toggling a transparent overlay webview can steal focus from the page. Mitigation: overlay created once at startup, 0×0, focusable only when sized; explicit focus handoff on close, tested by the keyboard walkthrough (W9).
- **R2 — Page webview swallows shortcuts.** `Ctrl+W`, `Ctrl+Tab` inside a focused page may be consumed by the page before the host accelerator. Mitigation: accelerators registered per webview; the primary path is registered, with `Ctrl+Alt+←/→` as the guaranteed fallback. Even so, some pages will block; this is stated, not hidden.
- **R3 — Find API limits.** WebView2's find supports literal text and case sensitivity; regex and highlight-all may not be reachable. Mitigation: the control's label is generated from `find.result.supported_flags`, so the UI never offers a mode that silently degrades. If unsupported, the toggle reads "Whole word" or is absent.
- **R4 — WebView2 memory floor.** No amount of tab management gets a Chromium multi-process tree under ~200 MB; W5's low-end target may miss. Mitigation: report the actual measurement and the composition, never a headline number. This is the biggest honest risk to the "better than Chrome" claim.
- **R5 — Resizing page webview for find bar/docked panel** triggers a page reflow and can be slow on heavy pages. Mitigation: 40 px shrink only on open/close; measured under W6's harness.
- **R6 — Single delivery cycle scope.** Sleep, groups, regex find, download pause/resume, and session restore are each individually real work. If anything must drop, drop in this order: regex/whole-word find → download pause/resume → groups → sleep. Do not drop: overlay/focus correctness, zero egress, keyboard map.

---

## 8. Accessibility & inclusivity check

| Requirement | Implementation |
|---|---|
| Keyboard-only operation of every action | Every binding in §3.5; W9 script exercises all panels, rail, omnibox, palette, downloads, history, find, settings. |
| Screen reader semantics | Rail = `role="listbox"` with `aria-selected`, `aria-expanded` on groups; overlay = `role="dialog"` + `aria-modal="true"` + `aria-label`; dropdown = `role="listbox"`/`option` with `aria-activedescendant`; toasts `aria-live="polite"`; error page `role="alert"`. |
| Reduced motion | All transitions 0 ms except ≤120 ms opacity; no transform-based motion. |
| Contrast | §5.1 measured ratios; `--fg-subtle` restricted to hints. |
| Zoom legibility | Shell UI at 100 % browser zoom is fixed-px, but `Ctrl+Plus` must also scale the **shell**: shell zoom is mirrored from the page zoom setting (`--ui-scale`, clamped 0.8–2.0), so chrome never becomes unreadable relative to the page. This is the legibility failure mode of canvas/zoomable shells, handled by not having a zoomable canvas and by scaling the chrome with the page. |
| Pointer-free operation | No control requires hover to be *used* (hover only adds affordance); the close button exists at `opacity:0` with `:focus-visible` making it visible. |
| Focus visibility | `outline: 2px solid var(--accent); outline-offset: 1px` on every focusable element; never `outline: none` without a replacement. |
| Screen reader + page content | Browser chrome (shell) and page are separate webviews; the shell exposes `aria-label` on every region so a screen reader user can distinguish "browser chrome" from "web page". |

---

## 9. Out of scope (explicit)

Extensions, profiles, sync, accounts, tabs-as-canvas/ZUI, split view, tear-off tabs, reader mode, print, bookmarks bar (bookmarks exist only as new-tab tiles and `Ctrl+D`), PDF viewer custom UI (WebView2 default), auto-update, crash reporter upload, search suggest API, password manager, incognito (a `Private` group is not shipped in v1), Windows dark-mode auto-detection beyond `prefers-color-scheme`.

---

## 10. Acceptance gates before merge

1. W1, W2, W3 published in `bench/report.md` with the raw run data.
2. W7 passes: 60 s idle session, zero non-page requests in the capture.
3. W9 keyboard walkthrough passes with zero pointer events.
4. `Ctrl+W` inside a page, in a text input, and in the rail behaves per §3.5.
5. 100-tab session: rail renders in ≤ 50 ms, W5 memory reported, overflow scrolling works.
6. Session restore after a forced kill: rail restored, active tab restored, scroll offset either restored or logged as failed — never silently claimed.