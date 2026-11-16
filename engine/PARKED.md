# ENGINE — PARKED 2026-11-13

The from-scratch engine is **stopped**. The user decided to use Chromium core
instead. This tree is kept for possible future resumption, not for continued work.

Do not resume work here without an explicit instruction to do so.

---

## Why it was parked

The project needed a browser that renders real sites. After building
`engine-html` → `engine-dom` → `engine-select` → `engine-style` → `engine-text`
(181 passing tests), the remaining layers — `engine-layout`, `engine-paint`, and
the `engine` binary — were still stubs. On 2026-11-13 the user ran
`native-browser` (WebView2, i.e. Chromium) and it loaded sina.com; the
from-scratch engine could not render anything at all. The user's decision was to
stop and use Chromium core.

## Verified state at parking time

Checked by running the build, not by reading notes:

- `cargo build -p engine-layout` → **Finished, no warnings.**
- Green: `engine-html` 90 tests, `engine-select` 24, `engine-style` 36,
  `engine-text` 31. Total **181 passing, 0 failing.**
- Stubs still stubs: `engine-layout/src/lib.rs` and `engine-paint/src/lib.rs` are
  `pub fn placeholder() {}`; `engine/src/main.rs` is `fn main() {}`.
- `engine-layout/src/inline_flow.rs` exists (written 2026-11-13) but is **not
  declared as a module in `lib.rs`**, so it is inert — the crate never references
  it, so it has never been type-checked. It is self-documenting: whitespace
  collapsing, word wrapping, `text-align`, `line-height`, with a
  per-(style, token) measurement cache. See the compile check below for what
  blocks it.
- Also present and **never compiled**: `block.rs`, `box_tree.rs`, `fragments.rs`.

## Compile check performed at parking time (2026-11-13)

I temporarily replaced `engine-layout/src/lib.rs` with
`pub mod box_tree; pub mod inline_flow;` and ran
`cargo build -p engine-layout --message-format=short`. Then I restored `lib.rs`
to its original contents and re-ran the build to confirm `Finished, no errors`.
**12 errors**, so neither module is resumable as-is:

`inline_flow.rs` — 2 errors:
- `E0433`/`E0432`: `use engine_css::...` — `engine-layout/Cargo.toml` does not
  depend on `engine-css`. Add the dependency. (This is the root cause of both.)
- `E0308` at line 173: `measure()` returns `TextMetrics`, but the code assigns it
  to a `f32` width. The call sites in `emit_run` treat `measure(...)` as an `f32`;
  it needs `.width`.

`box_tree.rs` — 10 errors, i.e. it is further from compiling than `inline_flow`:
- `approximation` field referenced at 4 sites but `LayoutBox` has no such field.
- `Builder::note` method does not exist (2 call sites, lines 330 and 395 area).
- `KnownTag::Canvas` variant does not exist in `engine-dom`.
- `engine_css` not linked (same missing dependency as above).

**Conclusion:** of the two abandoned files, `inline_flow.rs` is ~2 small fixes from
compiling (add the `engine-css` dependency, add `.width`), while `box_tree.rs` was
mid-refactor against an older `LayoutBox` definition and needs its struct and
`LayoutParams` reworked. Pick up `inline_flow.rs` first.

## Pending design artifacts

- `docs/SUPPORTED-CSS.md` (333 lines) — per-property capability table with
  file citations.
- `docs/SHELL-UX.md` (606 lines) — shell spec for the engine's own window.

**Correction to `SUPPORTED-CSS.md`:** it states that
`engine-layout/src/{block.rs, box_tree.rs, fragments.rs}` are "gone" and that the
crate "has two files in total". That is **wrong as of parking time** — verified
with `dir`: five files exist (`block.rs`, `box_tree.rs`, `fragments.rs`,
`inline_flow.rs`, `lib.rs`). Treat the document's file inventory as stale; its
per-property analysis of `engine-css`/`engine-style` was not re-checked.

## Recorded limitations that would still bite on resumption

- `var()` / custom properties are rejected at parse time — a themed stylesheet
  loses every declaration that uses one.
- `em`/`rem` folded to px at CSS parse time against a fixed 16 px context, so
  `html { font-size: 20px }` does not rescale `2em` children.
- HTML parsing is quadratic in nesting depth (~5 s for a 10k-deep document).
- No HTTP client is wired to the parser: nothing in `engine/` calls `ureq`.
  Exercising the engine means feeding it a local HTML file.
- Complex scripts (Arabic, Devanagari, Thai) are unshaped.

## What to do if resuming

1. `cd engine && cargo test` — confirm the 181 still pass.
2. `mod inline_flow;` in `engine-layout/src/lib.rs`, then make it compile.
3. Build block flow on top of it using `engine-text` for measurement.
4. Paint through `engine-raster` (it already has the full `DisplayList` /
   `DisplayItem` model, fills, borders, gradients, clip stack, glyph blitting).
   Do not write a second rasterizer.
5. The binary CLI target: `engine <input.html> -o out.png [-w 1280 -h 800]`.