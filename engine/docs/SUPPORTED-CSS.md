# Supported CSS — capability table

**Status:** capability snapshot of the code on disk, 2026-11-13. Every row cites the
file that *implements* the behaviour or the file that *omits* it. Where the code was
not read for a given claim, the row says `verify`.

**Reading the "State" column**

| State | Meaning |
|---|---|
| **Parser only** | `engine-css`/`engine-style` produce a computed value; **no layout stage exists to consume it.** This is the honest state of every box-model and text row today. |
| **Yes** | Implemented and reachable end-to-end; a test exercises the happy path. |
| **Partial** | Implemented with a named, user-visible limitation (listed in "Notes"). |
| **Parses, not used** | The parser accepts the value; no downstream stage reads the field. |
| **Stub** | Planned to be lowered to block flow with `Approximation::Stubbed(...)` — the lowering code is **not on disk**, see gap 3. |
| **No** | Rejected at parse (declaration dropped, error recorded) or the file that should implement it does not exist. |
| `verify` | Behaviour not confirmed by reading the file. |

> **The state of the tree changed during drafting.** Before the first pass, the file
> table showed `engine-layout/src/{block.rs, box_tree.rs, fragments.rs}`. They are now
> **gone**; `engine-layout/src/lib.rs` is `pub fn placeholder() {}` and the crate has
> two files in total. `engine/*` is also **entirely untracked by git** (`git ls-files`
> returns nothing under `engine/`), so there is no history to recover a previous copy
> from. Everything below that cites `block.rs` or `box_tree.rs` is cited **as design
> intent recorded in the source I read during this session**, and must be re-read
> against whatever re-lands before it is trusted. Where a claim depended on layout
> running, it now says **Parser only**.

**Known global gaps that shape every row below**

1. `var()` / custom properties are rejected at parse time. `engine-css::declaration`
   falls through to `"unsupported property"` for any `--x` declaration, and `var()`
   is not a recognised function in `engine-css::values`. A declaration whose *value*
   contains `var(...)` therefore fails to parse. Consequence: a stylesheet that
   themes itself with custom properties loses those declarations, and all of them,
   not one. Recorded in `PROGRESS.md`.
2. `em`/`rem` are folded to px **at CSS parse time** against a fixed 16 px context
   (`engine-css::values::parse_length`, `LengthCtx::default()`), including inside
   `style="..."` attributes (`engine-style::cascade::InlineStyles::collect` uses
   `LengthCtx::default()`). `html { font-size: 20px }` does **not** rescale
   `h1 { font-size: 2em }` (still 32 px). Percentages *are* resolved against the
   real parent size (`engine-style::cascade::resolve_font_size`).
3. `engine-layout` is a **stub**: `src/lib.rs` is `pub fn placeholder() {}` and no
   other source file exists. The formatting-context modules the design names
   (`flow/block.rs`, `flow/inline.rs`, `flow/float.rs`, `flow/flex.rs`,
   `flow/grid.rs`, `flow/table.rs`, `flow/positioned.rs`) and the box/fragment types
   are absent. Consequences: `engine-paint` has no `LayoutTree` to consume, and
   `engine` (the binary) cannot complete the pipeline. **Every "Parser only" row
   below is a parser claim, not a rendering claim.**
4. `engine-paint/src/lib.rs` is a stub. **`engine-raster`, however, is real**: it owns
   `DisplayList`/`DisplayItem` (moved there from `engine-paint` to break a dependency
   cycle, recorded in `engine-raster/src/list.rs`), `render(surface, list, clip,
   images, glyphs)`, `fill_rect`/`fill_rounded_rect`/`fill_linear_gradient`/
   `stroke_border`/`stroke_outline`/`blit_image`, and `verify_determinism`. So the
   *painting primitives* exist and are byte-deterministic; there is simply nothing on
   disk yet that produces a display list from a document.
5. `engine-text::script` flags Hebrew, Arabic, Devanagari, Bengali, Tamil, Thai,
   Myanmar, Khmer as unshaped (`script_warning`). `engine-text::line_break`
   implements four break rules and states the rest of UAX #14 is absent.
   `engine-text::context::measure` and `layout_runs` are implemented and tested, which
   makes text **measurement** the one box-adjacent capability that does run today.

---

## 1. `display`

| Value | State | Implemented in | Notes |
|---|---|---|---|
| `block` | Parser only | `engine-css/src/declaration.rs::parse_display`; UA sheet sets it (`engine-css/src/ua.css`). The `engine-layout` lowering (`Display::Block` → block box) **is gone from disk** | Computed value correct. No box is created. |
| `inline` (initial) | Parser only | `engine-css/src/properties.rs` `ComputedStyle::default` | `display: inline` is the initial value, as in CSS. |
| `none` | Parser only | `apply.rs::set_display` stores `Display::None` | Subtree *would* be dropped by the gone `box_tree.rs`. Nothing on disk consumes it (`engine-paint` stub). |
| `inline-block` | Parser only | `apply.rs::set_display` | Whether it becomes a block box flagged inline-level is decided by the absent layout crate. |
| `list-item` | Parser only | `apply.rs` | `list-style-*` fields exist on `ComputedStyle`; no marker is generated or drawn. |
| `flex` / `inline-flex` | Parser only (stub planned) | `apply.rs::set_display`; `properties.rs` `Display::Flex` | `flex-direction/wrap/grow/shrink/basis/justify-content/align-items/gap` all compute correctly and then have **no consumer**. The design's `Approximation::Stubbed("flex")` lowering was in the deleted `block.rs`. When layout re-lands, this is the row to re-check first: **placeholder flex distance beats correct-but-vertical stacking, and vertical stacking is what the design admits.** |
| `grid` / `inline-grid` | Parser only (stub planned) | `apply.rs` | `grid-template-*`, `grid-auto-*`, `grid-column`, `grid-row` are **not parsed at all** — no arm in `declaration.rs`. |
| `table` / `table-row` / `table-cell` | Parser only (stub planned) | `apply.rs`; `border-collapse`/`table-layout`/`border-spacing` → `Unsupported` in `apply.rs` | `UA.css` gives cells 1 px padding and `th` centre + bold. No column layout. |
| `contents` | Parser only | `apply.rs` stores `Display::Contents` | |
| `flow-root` | No | `parse_display` has no arm | Declaration dropped with a parse error. The design's fallback is `overflow: hidden` for a new BFC. |

## 2. Box model

> **Every row in this section is now Parser only.** The computed values are right;
> nothing resolves them to geometry because `engine-layout` is a stub (global gap 3).
> Files named `block.rs` / `box_tree.rs` are the implementations that existed when
> this session began and were deleted during it.

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `margin` (4 longhands, 1–4 value shorthand) | Parser only | `declaration.rs::expand_four`; `apply.rs::set_px` | Percentages on margin are **rejected** (`set_px` returns `Unsupported` for `Length::Pct`). `margin: auto` resolves to 0 there; horizontal centring needed a separate `Length::Auto` path in the deleted `block.rs::auto_margins`. `margin: 5%` does not work and will not when layout returns until `set_px` changes. |
| `padding` (4 longhands) | Parser only | `declaration.rs`; `apply.rs::set_px` | Same percentage rejection. `padding: auto` is a parse error (correct per CSS). |
| `border-width` / `border-style` / `border-color`, per side | Parser only | `declaration.rs::expand_border`; `apply.rs` | `border-style: none` zeroing used width was in the deleted `block.rs::edges`; re-verify on re-land. |
| `border-style: dashed / dotted / double` | Partial | `apply.rs::border_style_at` accepts all three | `properties.rs` documents `Double` as "drawn as solid". `engine-raster::stroke_border` is a real solid stroke, so dashed/dotted would need a new primitive or an approximation. |
| `border-radius` (4 corners, single radius) | Partial | `declaration.rs` expands; `apply.rs` stores into `CornerRadii` | **`engine-raster::fill_rounded_rect` exists**, and `DisplayItem::FillRect` carries `radii: CornerRadii` — so rounded *fills* are real. Elliptical radii (`10px / 5px`) are not representable. Whether the *clip stack* honours radius (`PushClip` carries only a `Rect`) is **No**: rounded clipping is not possible with the current `DisplayItem`. |
| `width` / `height` / `min-*` / `max-*` | Parser only | `apply.rs::set_len` | The deleted `block.rs::resolve(l, base)` used one base for both axes; on re-land, check whether a percentage `height` is measured against the parent's *width*. |
| `box-sizing: content-box / border-box` | Parser only | `apply.rs` | |
| Negative margins | Parser only | — | The deleted `block.rs::collapse_margins` used `max(pos) - max(|neg|)`. Its module doc admitted: no collapsing through chained empty blocks, no float interaction. |
| Margin collapsing parent↔first/last child | Parser only | — | Same module doc; chains longer than one sibling pair were never implemented. |
| `outline` | No | `declaration.rs` produces `outline-width`/`outline-color`; `apply.rs` returns `Unsupported` for both | Note `DisplayItem::Outline` **does** exist in `engine-raster`, so the shell can draw a focus ring directly (see `SHELL-UX.md` §6.1). Content CSS still cannot. |

## 3. Text

| Property | State | Implemented in | Notes |
|---|---|---|---|
| `font-size` (length, %, keywords) | Yes (computed) | `declaration.rs`; `cascade.rs::resolve_font_size` | Absolute keywords map to fixed factors (`medium`=1.0, `large`=1.2, …). `smaller`/`larger` become percentages, so they *do* track the parent. `em` inside the value is folded at parse time (global gap 2). A non-positive result is clamped to 16 px (`cascade.rs`). This is the input to `FontContext::measure`, which **does** run (global gap 5). |
| `font-weight` | Partial | `apply.rs` | Numeric 1–1000 accepted; only regular (400) and bold (≈600+) faces are distinguished (`engine-text/src/context.rs::FontKey::from_style` — `weight_bold: weight >= 600`). `bolder`/`lighter` map to 700/300 rather than stepping from the parent. |
| `font-style: italic` | Partial | `apply.rs`; `context.rs::select_face` | Selects an italic face if one is loaded; `ARCHITECTURE.md` §9.1 states synthetic oblique is not produced. A page with only a regular face gets upright text. |
| `font-family` | Yes | `declaration.rs`; `engine-text/src/font_dir.rs`, `face.rs` | System fonts only. `@font-face` is skipped (`stylesheet.rs` records `@font-face` in `skipped_at_rules`). Generic families map per `ARCHITECTURE.md` §9.1 (`cursive`/`fantasy` → sans-serif). |
| `line-height` (`normal`, number, px, %) | Yes (computed) | `apply.rs::parse_line_height` | `%` becomes a number (relative to font-size — correct). `normal` uses `ascent+descent+line_gap` from `FontContext::line_metrics`. |
| `letter-spacing` | Partial | `apply.rs`; `context.rs::measure`/`layout_runs` | Applied per character to the advance. `em` folded at parse time. |
| `text-align: start / end / center / justify` | Partial | `apply.rs` maps `left`→`Start`, `right`→`End` | **No RTL**: `direction` and `unicode-bidi` are in the recognised-but-ignored list in `declaration.rs`, so `start` always means left. `justify` has no consumer (no inline flow on disk). |
| `white-space: normal / nowrap / pre / pre-wrap / pre-line` | Yes at the text layer | `apply.rs`; `line_break.rs::{break_opportunities, wrap}` | `preserves_newlines` / `wraps` predicates correctly drive the wrapper, and `wrap` is tested (`tests/text.rs`, 13 unit tests in-file). Nothing on disk calls `wrap` with a real line width yet — that was the deleted inline flow. |
| `word-break`, `overflow-wrap`, `hyphens` | No | No arm in `declaration.rs` | Long words break by character only when a single word exceeds the line (`line_break.rs::wrap`, the "no opportunity fits" branch). |
| `text-decoration: underline / line-through` | Parses, not used | `declaration.rs` (overline/blink silently dropped); `apply.rs` sets `TextDecoration` | No paint stage builds underline lines; `engine-raster` has no dedicated decoration primitive, so it would be a `FillRect` of 1 px height. |
| `text-transform`, `font-variant`, `word-spacing` | No (explicitly ignored) | `declaration.rs` recognised-but-ignored list | Dropped without an error. `text-transform: uppercase` does **not** happen. |
| `vertical-align` (keywords) | Partial | `apply.rs` | `sub`/`super`/`middle`/`top`/`bottom`/`baseline` stored; **length/percentage values are `Unsupported`** (`apply.rs` explicit arm). |
| Bidi / complex scripts | No | `engine-text/src/script.rs`; `ARCHITECTURE.md` §9.2 | Arabic/Hebrew render disjoined and in logical order; Devanagari/Thai/Khmer have no reordering. `FontContext::measure` sets `shaping_warning` for the first such code point per string, so a report is possible. |
| Ligatures, kerning from `kern` only | Partial | `context.rs::measure`, `layout_runs` | Advance-summed; optional `kern` table lookups; no GSUB/GPOS. |
| `ex` / `ch` units | Partial | `values.rs::parse_length` | `ex` = 0.5em, `ch` = 0.5em — **not** real font metrics (`engine_core::EX_FALLBACK_EM`). |

## 4. Color and background

| Property | State | Implemented in | Notes |
|---|---|---|---|
| `color` | Yes (computed) | `declaration.rs::parse_color` | `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, named colors, `transparent`, `rgb()/rgba()/hsl()/hsla()` (comma and space-separated, `/ alpha`). `currentcolor` is accepted as a *name* only if present in the named table — `verify` whether it resolves to the element's `color`. `Rgba8` is what `DisplayItem` carries, so the computed value is directly paintable. |
| `background-color` | Yes (computed) | `apply.rs` → `style.background.color` | `DisplayItem::FillRect { color }` is the primitive it would feed. |
| `background-image`, `background-position` | Parses, not used | `Background` has `image: Option<usize>` and `position`; `declaration.rs::expand_background` is `verify` for image extraction | **`engine-raster::blit_image` and `ImageStore` exist**, so the primitive is ready; no `url()` loader exists in the computed-style → display-list path (that path is `engine-paint`, a stub). |
| `background-repeat` | Parses, not used | `apply.rs`; `DisplayItem::Image { repeat }` supports `NoRepeat/Repeat/RepeatX/RepeatY` | Primitive exists. |
| `background-size` | Partial | `apply.rs` | Only `cover`/`contain` are accepted; **explicit lengths such as `100% 100%` return `Unsupported`** (the parser emits a `StringList` and `apply.rs` only checks keywords). `ObjectFit` would cover `fill`/`contain`/`cover` once mapped. |
| Multiple backgrounds | No | `declaration.rs::expand_background` | Single-layer model only. |
| Gradients | Partial (parse), Yes (paint) | No `linear-gradient()` in `declaration.rs` | `DisplayItem::LinearGradient` **is implemented** (`fill_linear_gradient`, 2-stop) and exercised by the raster determinism test. Only the CSS side is missing. |
| `filter`, `backdrop-filter` | No | No arm in `declaration.rs` | |
| `opacity` | Parses, not used | `apply.rs` stores a clamped number | `DisplayItem::Image` has per-item `opacity: f32`; a group opacity would need the display list to carry it on every item. |
| `box-shadow`, `text-shadow` | No | No arm in `declaration.rs` | |
| `visibility: hidden` | Parses, not used | `apply.rs` sets `visibility_hidden` | Inherits (`properties.rs::inherit::apply_inherited`). Skipping the box at paint needs a consumer. |
| Per-channel alpha blending | Yes | `engine-core/src/color.rs` `premultiplied`, `source_over`; `engine-raster::Surface::blend_premul_at` | Real, tested. |

## 5. Positioning

> **No row in this section has an implementation on disk.** `top/right/bottom/left`
> compute correctly and are consumed by nothing. This is the single most important
> section for the shell spec, which is why `SHELL-UX.md` §2 constrains the shell to
> block-stack layout.

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `position: static` / `relative` | Parser only | `apply.rs` stores `Position`; offsets stored on `ComputedStyle` | `top/right/bottom/left` are parsed (`parse_length`) and stored. No consumer. A `relative` offset has no effect. |
| `position: absolute` / `fixed` | **No** | No `flow/positioned.rs`; no file applies offsets | `ARCHITECTURE.md` §7.5 describes the intended algorithm. Nothing implements containing-block resolution, static-position fallback, or out-of-flow removal. |
| `position: sticky` | No | `apply.rs` maps it to `Position::Sticky`; `properties.rs` comments "`Sticky` is treated as `relative`" | Which is itself a no-op. |
| `z-index` | Parses, not used | `declaration.rs` bounds it to ±16 777 216; `apply.rs` maps `auto`→0 | Paint order is display-list order; `DisplayItem` has no z field. |
| `transform`, `translate`, `scale`, `rotate` | No | No arm in `declaration.rs` | Dropped with a parse error. This is the root reason a canvas zoom is not achievable today (see `SHELL-UX.md` §5.2). |
| `float: left/right` | **No** | `apply.rs` stores `Float` | The deleted `block.rs` module doc said "`float` children are laid out as blocks". `ARCHITECTURE.md` §7.2 describes float bands; nothing implements them. A two-column float layout is a vertical stack. |
| `clear` | Parses, not used | `apply.rs` | Meaningless without float placement. |

## 6. Overflow and scrolling

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `overflow: visible / hidden / scroll / auto` (both axes) | Parses, not used | `declaration.rs`; `apply.rs::overflow_of` (`clip`→`Hidden`) | The design computed a `clip: Option<Rect>` on the fragment; that fragment type is gone. **`DisplayItem::PushClip { rect } / PopClip`** in `engine-raster` *is* implemented and the renderer maintains a real clip stack, so recording clips is a ready primitive — it just has no producer. |
| Any `overflow: auto/scroll` box as a **scroll container** | **No** | — | Nothing computes content size vs viewport size, and no scroll offsets exist outside layout. `DisplayItem::Scrollbar { track, thumb, axis }` exists as a paint primitive with no producer. |
| Root/document scroll | No | — | Described in `ARCHITECTURE.md` §7.9; nothing owns it. The shell cannot scroll a page today. |
| `text-overflow: ellipsis` | Parses, not used | `ComputedStyle::text_overflow: bool` | Nothing paints an ellipsis. |
| `scrollbar-*`, `scrollbar-gutter` | No | No arm in `declaration.rs` | |

## 7. Tables

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `table-layout: auto/fixed` | Parses, not used | `declaration.rs`; `apply.rs` returns `Unsupported` | No column width computation anywhere. |
| `border-collapse` | Parses, not used | `apply.rs` returns `Unsupported` | Collapsed borders are not modelled. |
| `border-spacing` | Parses, not used | same | |
| `<td>`/`<th>` padding, backgrounds | Parses, not used | `ua.css` gives 1 px padding and `th` centre + bold | The design laid a table out as a vertical stack of rows of block cells with `Approximation::Stubbed("table")`. That lowering is gone with `box_tree.rs`. |
| `caption-side`, `empty-cells` | No | `declaration.rs` recognised-but-ignored | |

## 8. Lists

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `list-style-type` (disc, circle, square, decimal, alpha, roman, none) | Parses, not used | `apply.rs` stores `ListStyleType` | No marker box is generated, and `engine-raster` has no list-marker primitive — a marker would be a glyph run plus indentation. |
| `list-style-position: inside/outside` | Parses, not used | `apply.rs` | |
| `list-style-image` | No | No arm | |
| List indentation from the UA sheet | Yes (computed) | `ua.css`: `padding-left: 40px` on `ul`/`ol`, `margin-left: 40px` on `dd` | This is the *only* thing that will make lists look like lists when layout returns — the indent, not the bullet. Expect bullet-less lists initially. |

## 9. Images (replaced elements)

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `<img>` intrinsic size in layout | **No** | — | The design read intrinsic size from decoded dimensions (default 300×150 per `ARCHITECTURE.md` §7.4) inside the deleted `box_tree.rs`/`block.rs`. No consumer. |
| The *paint* half of images | Yes | `engine-raster::blit_image`, `ImageStore`, `DecodedImage`, `DisplayItem::Image { dst, src, fit: ObjectFit, opacity, repeat }` | `ObjectFit::{Fill, Contain, Cover, None, ScaleDown}` is already the CSS value set — only the CSS property mapping is missing. |
| `object-fit` CSS property | No | No field in `ComputedStyle` | Dropped at parse. The raster side is ready. |
| `background-image` on a box | Parses, not used | see §4 | |
| `srcset` / `<picture>` | No | `ARCHITECTURE.md` §12 | Only `src` is considered. |
| `<video>`, `<canvas>` | **No** | — | No `ReplacedKind` exists on disk any more; no video decoder, no 2D canvas context, no WebGL. |
| SVG | No | — | No SVG parser anywhere in `engine-*`. `<img src=*.svg>` cannot decode (only PNG/JPEG decoders are planned). Inline `<svg>` becomes unknown elements. |

## 10. Forms

> Forms are **parse-only** today. The UA sheet gives them a border; `DisplayItem::Control
> { rect, kind: ControlKind, state: ControlState, radius }` exists in `engine-raster`
> with `ControlKind::{Button, TextInput, Checkbox, Radio, Submit}` and
> `ControlState::{Normal, Hover, Active, Focused, Checked, Disabled}` — so control
> *painting* is designed and ready. Nothing produces those items from a document.

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| `<input>` (text, password, checkbox, radio, submit) | Parses, not used | `ua.css` border/background/padding | Value text, caret, selection, and checked boxes have no producer. |
| `<button>` | Parses, not used | `ua.css` styles it inline-block | |
| `<textarea>` | Parses, not used | `ua.css` `white-space: pre-wrap` | No intrinsic rows/cols, no internal scrolling (no scroll containers at all — §6). |
| `<select>` | No | — | No closed-state rendering, no popup. |
| `:focus`, `:focus-visible`, `:focus-within` | `verify` | `ARCHITECTURE.md` §6.4; `engine-select/src/matcher.rs` | `engine-select` **is** implemented and tested (24 tests per `PROGRESS.md`), but whether a focus state is plumbed from the platform layer into `MatchCtx` is `verify`. |
| `<form>` submission | No | — | `engine-script` is a stub; `submit` event wiring is not on disk. |

## 11. Selectors and cascade (needed to judge whether a page's CSS applies at all)

> This is the **most complete** vertical slice in the engine. `engine-css` parses,
> `engine-select` matches (24 tests), `engine-style` cascades (36 tests), and the
> result is a correct `ComputedStyle` per element. It is the one pipeline that can be
> exercised end-to-end today — down to the point where it hands a style to a layout
> stage that does not exist.

| Feature | State | Implemented in | Notes |
|---|---|---|---|
| Type, class, id, attribute, universal | Yes | `engine-css/src/selector_parser.rs`, `engine-select/src/matcher.rs` | Right-to-left matching. |
| Descendant, child, adjacent, general sibling | Yes | `engine-select/src/matcher.rs` | Column combinator never matches (documented). |
| `:not`, `:is`, `:where`, `:nth-child(An+B)` | Yes | `engine-select/src/matcher.rs` and its tests | `PROGRESS.md` records three real matcher/cascade bugs found and fixed (subject-first compound walk, quadratic `lang` resolution, specificity bit packing). |
| `:first-child`, `:empty`, `:root`, `:lang` | Yes | `engine-select/src/index.rs`, `matcher.rs` | |
| `:nth-child(An+B of S)` | Partial | `ARCHITECTURE.md` §6.4 | Only when `S` is a single compound. |
| `:has()` | Partial | `ARCHITECTURE.md` §6.4 | Depth-2 descendant only; excluded from the selector cache. |
| `:hover`, `:active` | `verify` | `ARCHITECTURE.md` §6.4 | `:hover` requires a pointer state from the platform layer; `apply.rs`/`cascade.rs` carry no input state. |
| `::before` / `::after` | No | `apply.rs` documents `content` as `Unsupported` | Pseudo-elements are not generated. **A very large amount of icon and ribbon CSS is `::before`-based**, so this matters beyond text. |
| `@media` | Partial | `stylesheet.rs::parse_media_query`; `engine-style/src/media_eval.rs` | `screen`, `all`, `not`, `and`, comma lists; numeric features (`min-width`, …) and `prefers-color-scheme`, `prefers-reduced-motion` **as keyword features**. **Range syntax `(400px <= width <= 700px)` is `MediaQuery::Unsupported` → never matches** (`parse_media_feature`). |
| `@supports` | Yes (approximate) | `stylesheet.rs::supports_condition` | True iff the inner declaration parses. That is a *weaker* test than a browser's, which checks computed-value support too. |
| `@import` | Partial | `stylesheet.rs` collects URLs into `sheet.imports` | The caller must fetch them; `verify` that the load pipeline does. `@import` after another rule is still honoured (CSS requires it be first). |
| `@layer` | No | `stylesheet.rs` treats it as a no-op | Layer order is not honoured. `@layer a, b;` statements are skipped. |
| `@font-face` | No | `skipped_at_rules` | System fonts only. |
| `@keyframes` / `animation` / `transition` | No | `skipped_at_rules` for `@keyframes`; no `animation*`/`transition*` arms | No motion from page CSS. |
| `@container`, nesting (`&`), `@scope`, `@property` | No | `skipped_at_rules`; a nested rule fails `parse_selector_list` and is dropped with an error | |
| `!important`, origin order, inline style | Yes | `engine-style/src/cascade.rs` (`Rank` struct; origin 1–7) | Inline style ranks 4 (normal) / 5 (important), above author normal. |
| CSS-wide keywords | Partial | `cascade.rs::apply_wide_keyword` | `revert` is approximated as `unset`. |
| `revert-layer` | No | Not in `WideKeyword` | |

## 12. Units and functions

| Item | State | Implemented in | Notes |
|---|---|---|---|
| `px`, `cm`, `mm`, `in`, `pt`, `pc`, `Q` | Yes | `values.rs::parse_length` | 96 dpi basis. |
| `%` | Partial | preserved as `Length::Pct` | Resolved for width, font-size, line-height. **Not** for margin/padding (`set_px` rejects), and height percentages use the width base (see §2). |
| `em`, `rem` | Partial | `values.rs` | Folded at parse against 16 px — global gap 2. |
| `vw`, `vh`, `vmin`, `vmax` | Partial | `values.rs` uses `LengthCtx` | The stylesheet parse context is `LengthCtx::default()` (800×600) unless the caller passes a real one — `verify` that the load pipeline calls `parse_stylesheet_with_ctx` with the real viewport. If it does not, `vw`/`vh` compute against 800×600. |
| `ex`, `ch` | Partial | 0.5 em / 0.5 em | Not font metrics. |
| `calc()` | Partial | `values.rs::parse_calc` | `px`-only arithmetic. `calc(50% - 10px)` is **rejected** (affine mix of % and px). Depth cap 64. |
| `cqw`, `lh`, `vi`, `vb`, `dvh`, `svh`, `lvh` | No | rejected in `parse_length` | |
| `var()` | No | see global gap 1 | |
| `clamp()`, `min()`, `max()` | No | Not in `values.rs` | |

## 13. The UA stylesheet's contribution

`engine-css/src/ua.css` is authored (not copied) and is testually guarded by
`stylesheet.rs::ua_sheet_parses_without_errors` and `ua_sheet_has_expected_rules`.
It supplies: block defaults for the common flow elements; `display: none` for
`head/script/style/meta/link/…`; body 8 px margin; heading sizes and margins;
`p` 1em margins; `ul/ol` 40 px left padding with disc/decimal markers; `a` blue
underline plus an `:visited` colour; `b/strong` bold; `i/em/cite/dfn/var` italic;
`pre/code` monospace + pre; `sub/sup` vertical-align; `mark` yellow; form control
borders; table-cell 1 px padding.

**Wall-clock caveat:** `:visited` styling exists, but whether the shell records
visited state and feeds it into the cascade before a restyle is `verify`
(`engine-style` has no visited-set input in `cascade.rs`).

---

## 14. Sites this predicts

Observation, not aspiration — derived directly from the rows above:

| Site class | Expected outcome **once layout exists** | The row that decides it |
|---|---|---|
| Static docs, specs, plain-text archives, wiki exports, single-column blogs | Renders well | block flow + text + images |
| Anything floated two-column (2005–2014 era, many news CMSs) | Partially: float columns stack vertically | §5 `float` |
| Anything flex-based (most nav bars, cards, hero sections since ~2018) | Partially: nav bars and card grids collapse into vertical stacks | §1 `flex` |
| Anything grid-based (most dashboards, product grids) | Partially: same, plus no column tracks at all | §1 `grid` |
| Tailwind / utility-CSS sites | **Badly**: utility classes are plain classes so they *apply*, but `::before` icons, custom properties, `transform`, and `position:absolute` are all absent — a real site loses its icons, overlays, and dropdowns | §11 `::before`, §5 position, §12 `var()` |
| Google Fonts–only sites | Font falls back to the next family | §3 `@font-face` |
| Anything with `position:fixed` headers | The header scrolls away (once scrolling exists); content is not offset | §5 |
| Arabic/Hebrew/Indic sites | Visibly broken text | §3 bidi |
| JS-rendered SPAs (React/Vue/Svelte) | Blank | `engine-script` is a stub; even with boa, no `fetch`, no `rAF`, no Shadow DOM |

**Today, before layout re-lands:** no predicted outcome is reachable. The table above
is what the *parsed capability set* supports, not what the binary renders.

**Honest summary for user-facing copy:** the engine is being built to render *static
documents*, not *applications*. `ARCHITECTURE.md` §0 estimates 20–25 % of real pages
render essentially correctly; this capability table does not contradict that and the
flex / `::before` / `position` gaps are the main reason behind the "partially
rendered" 45–50 %.

---

## `verify` list (for Bob or the owner to confirm empirically)

**Blocking — answer these first**

1. **Where did `engine-layout/src/{block.rs, box_tree.rs, fragments.rs}` go?** They
   existed earlier in this session and are absent now, untracked by git. Is the
   layout work in a different directory, an unmerged branch, or lost? Until answered,
   every §1/§2/§5/§6/§9/§10 row stays "Parser only".
2. **Is the engine workspace buildable?** `cargo build -p engine-layout` on a
   `placeholder()` crate links fine, but `engine-paint` would have to import a
   `LayoutTree` type that no longer exists. Confirm which crates currently compile.
3. **`vw`/`vh` against the real viewport** — does the load pipeline call
   `parse_stylesheet_with_ctx` with the window's `LengthCtx`, or take the 800×600
   default?

**Correctness — answer when layout re-lands**

4. **`currentcolor`** — does `parse_color` resolve it to the element's `color`, or
   does it drop?
5. **Percentage `height`** — is it measured against the parent's height or width?
6. **`margin: 5%` / `padding: 5%`** — `apply.rs::set_px` rejects percentages. Is that
   recorded in the render report as `Unsupported`, or silently zero?
7. **`<input value>` text** — is the value synthesised as a text node anywhere?
8. **`:visited`** — is there a visited set feeding the cascade?
9. **`:hover`/`:active`** — is pointer state plumbed into `MatchCtx`?
10. **`background-image` extraction** from the `background` shorthand.
11. **Rounded clipping** — `DisplayItem::PushClip` carries only a `Rect`; confirm no
    rounded clip is planned in `engine-raster` before relying on `border-radius` for
    clipping (it works for *fills*).
12. **`justify`** text distribution.

**Performance — measure, do not guess**

13. **`FontContext::new()` wall time** on this machine (enumerates `C:\Windows\Fonts`).
    The shell's first-frame budget in `SHELL-UX.md` §3.1 depends on this number.