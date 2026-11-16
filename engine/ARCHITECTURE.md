# `engine` — A Web Rendering Engine Written From Scratch in Rust

Status: design document. Nothing in `Q:\gitroot\Game\engine` exists on disk yet.
Owner decision (recorded, not argued): **everything from scratch.** No Servo crates, no Chromium, no WebView2, no Blink, no WebKit.

---

## 0. Frame

- **What this is:** a Rust HTML/CSS renderer that fetches a page, builds a DOM, computes styles, lays out boxes, and paints pixels into a native Windows window from our own code.
- **Primary user:** the owner, pointing it at arbitrary URLs and observing what renders.
- **Job-to-be-done:** render the static, CSS-driven portion of the web well enough to be useful on a minority of real pages, and be *honest* everywhere it fails.
- **The constraint that governs every decision:** correctness of the CSS/layout core, and a verification system that proves it without a reference browser. Performance matters (budgets in §11) but cannot be validated for quality against other engines; legibility and honest failure reporting matter more than speed at this stage.

**Scope honesty, stated up front (estimate, not measurement — we have no corpus yet):**

| Bucket | Estimate of real-world pages | Typical page type |
|---|---|---|
| Renders essentially correctly (static HTML/CSS, no JS-dependent content) | ~20–25% | blogs, docs, specs, plain news articles, personal sites |
| Renders partially (structure correct, some blocks missing or misplaced) | ~45–50% | most sites once float/flex/positioning interactions are involved |
| Blank or unusable | ~30% | any SPA, most dashboards, most e-commerce, anything whose text lives in JS |

**What is verifiable on this machine (checked during drafting):** `cargo 1.99.1` / `rustc 1.99.1`; `C:\Windows\System32\dxgi.dll` and `d3d11.dll` present; `C:\Windows\Fonts\segoeui.ttf`, `segoeuib.ttf`, `arial.ttf` present. Crate registry cache contains `tao-0.37.2`, `serde-1.0.229`, `url-2.5.8`, `percent-encoding-2.3.2`, `winapi-0.3.9`, `windows-0.62.2`. **Not cached:** `fontdue`, `png`/`image`, `flate2`, `boa_engine`, `ureq`/`rustls` — these need a working crates.io fetch. `node_modules` is present at the repo root, but a headless-browser reference rasterizer is **not** assumed anywhere in this plan.

---

## 1. Owner constraints, restated precisely

1. **From scratch:** our own HTML tokenizer + tree construction, CSS tokenizer + parser + cascade + selector matching, box tree, layout, paint, compositing, text layout (glyph positioning, line breaking), URL/fetch layer.
2. **Best-effort general browsing:** we accept partial rendering, and we surface it. There is a "render report" per page (§10.4) that lists exactly what was dropped or approximated. We do not claim general browsing works.
3. **No engine code borrowed.** No `stylo`, `selectors`, `cssparser`, `html5ever`, `taffy`, `cosmic-text`, `parley`, `swash`, `rustybuzz`, `tiny-skia`, `lyon`.

### 1.1 Allowed external crates and why each is not "part of the engine"

| Crate | Allowed for | Why it is out of scope |
|---|---|---|
| `tao` 0.37 | OS window creation, event loop, mouse/keyboard/wheel events, IME events, HiDPI scale factor | It is a Win32 `HWND` + `WndProc` wrapper. It creates a surface; it never touches a pixel. Nothing web-related. |
| `windows` 0.62 (`windows-sys`) | Raw `decl`-module bindings to DXGI/D3D11/COM/`dwrite`-adjacent Win32 calls | Crate is generated bindings only, no logic. We write all COM/D3D code. |
| `fontdue` | Font file parse (TTF/OTF tables `cmap`/`hmtx`/`head`/`hhea`/`maxp`/`glyf`/`loca`/`CFF`), glyph outline → coverage mask rasterization | "How do I draw a Bézier into an alpha mask" is not a web-standard behavior. Same reasoning browsers use for FreeType/CoreText/DirectWrite. We still write the *text layout*: advances, kerning lookup, clustering, line breaking, baseline placement (§7). |
| `png` | PNG container + DEFLATE decode into an RGBA buffer | File format decode. We write image *display*: decoding to the surface, scaling, `object-fit`, repeat, opacity. |
| `jpeg-decoder` | Baseline JPEG → RGB | Same. |
| `zune-inflate` | Zlib/gzip/deflate for `Content-Encoding` | Same. |
| `boa_engine` | ECMAScript execution | A JS engine is a language implementation, not a rendering engine. |
| `ureq` (rustls) or `reqwest` (rustls) | HTTP/1.1, TLS, redirect transport | Network transport is not rendering. |
| `url` + `percent-encoding` | RFC 3986 URL parse, serialize, resolve | Pure string/grammar library, no policy. We write redirect policy, cookie policy, same-origin check ourselves. |
| `serde` + `serde_json` | Test fixtures, device-pixel PNG manifest | Test infrastructure. |
| `ahash` | Hash maps/sets in cascade + selector caches | Hashing is not rendering. |

Rejected deliberately even though "convenient": `taffy` (layout), `cosmic-text`/`parley` (text layout), `rustybuzz`/`harfbuzz` (shaping), `tiny-skia` (rasterizer), `reqwest`'s cookie jar policy, any `encoding`/`charset` lookup table beyond a static table we own.

---

## 2. Surface decision: **CPU rasterizer blitting into one D3D11 swapchain**

Chosen: **(a) CPU rasterize → upload to a GPU swapchain.** Not pure GPU.

Justification in terms of what is verifiable here:

- **Golden-pixel tests need a deterministic, headless rasterizer.** A CPU rasterizer produces identical bytes on every run and can be called from `#[test]` with no `HWND`, no GPU, no display. A GPU path would require a swapchain, a readback, and GPU-driver-dependent antialiasing — the golden tests would be untestable and flaky. Since this project's whole verification story is "we have no reference browser, so our goldens are our truth" (§10), the rasterizer must be deterministic.
- **A software bitmap is the single most debuggable artifact.** Every stage downstream of layout can be reproduced as a PNG in `test-results/` with no window.
- **GPU cost is confined to one `CopyResource` per frame.** Full-window blit at 1080p, BGRA8, is ~8.3 MB per frame. At 60 Hz that is ~498 MB/s — well within integrated-GPU bandwidth. When only a damage rect changed (§6.2), we upload only that sub-rectangle.
- **D3D11 flip-model swapchain via DXGI**, `DXGI_FORMAT_B8G8R8A8_UNORM`, `DXGI_SWAP_EFFECT_FLIP_DISCARD`. This gives us vsync-paced presentation for free, without writing a compositor.
- Startup fallback path: if `D3D11CreateDevice` fails, present by `StretchDIBits` into the window DC. The renderer is unaware — it produces a `Surface` either way.

**Consequence we accept:** at 4K full-window repaint, CPU rasterization of text and rounded rects will miss a 16.7 ms budget (§11). Mitigation is damage tracking, not GPU work. We do not plan a GPU path; we plan to measure the CPU path and make scrolling incremental.

---

## 3. Crate / module layout

Workspace root `Q:\gitroot\Game\engine` (independent of `native-browser`, which is a separate `[workspace]`).

```
engine/
  Cargo.toml                      # [workspace], resolver 2, edition 2021, members = crates/*
  rust-toolchain.toml             # stable
  ARCHITECTURE.md
  crates/
    engine-core/                  # types shared by every stage; no I/O, no OS calls
    engine-html/                  # HTML tokenizer, tree construction, DOM
    engine-css/                   # CSS tokenizer, parser, stylesheet model
    engine-style/                 # UA sheet install, cascade, specificity, media queries
    engine-select/                # selector matching (consumes engine-css AST)
    engine-dom/                   # live DOM tree with parent links + NodeId; owned here, built by engine-html
    engine-layout/                # box tree + all formatting contexts + text layout
    engine-text/                  # font loading/cache, glyph atlas, shaping-lite, line breaking
    engine-paint/                 # display list construction
    engine-raster/                # software rasterizer: Surface, Renderer, determinism
    engine-platform/              # tao window, D3D11 swapchain, WndProc, timers, PNG encode
    engine-net/                   # URL parse/resolve, HTTP client, redirect policy, cache
    engine-script/                # boa_engine host, DOM bindings
    engine/                       # the binary: wires everything, owns the main loop
  testdata/
    fixtures/                     # hand-built HTML/CSS/SVG-less page fixtures
    goldens/                      # reference PNGs, committed
  tools/
    golden/                       # golden regeneration CLI (cargo run -p engine-golden)
```

### 3.1 Module tree with ownership, one line each

**`engine-core`** — no I/O, pure types.
- `src/lib.rs` — re-exports; `Length`, `Px`, `Rect`, `Point`, `Size`, `Color`, `Rgba8`.
- `src/length.rs` — `Length` enum: `Px(f32)`, `Pct(f32)`, `Em(f32)`, `Rem(f32)`, `Calc(CalcExpr)`, `Auto`, `None`. Used by layout and cascade identically.
- `src/geom.rs` — `Rect { x, y, w, h }`, `EdgeSizes`, `Point`, `Size`; all `f32`, all finite-checked.
- `src/color.rs` — `Rgba8`, named-color table we own, `currentColor` resolution helper.
- `src/ids.rs` — `NodeId(u32)`, `StyleId(u32)`, `BoxId(u32)`, `FontId(u32)`.
- `src/units.rs` — `CssPx`, `CssDp` (device px), `Scale(f32)` DPI factor.
- `src/error.rs` — `EngineError` enum used at all stage boundaries.

**`engine-html`**
- `src/tokenizer/state.rs` — the ~80-state HTML5 tokenizer state machine (§4.1).
- `src/tokenizer/token.rs` — `Token::Doctype{...}`, `StartTag{name, attrs, self_closing}`, `EndTag`, `Comment`, `Chars`, `Eof`.
- `src/tokenizer/char_ref.rs` — named character reference table (we own the table; generated once into `src/tokenizer/entities.rs`).
- `src/tokenizer/entities.rs` — static `&[(&str, &str)]` named entities (begins with the ~250 most-used; see §4.1 on the full set).
- `src/tree/construction.rs` — insertion modes (`Initial`, `BeforeHtml`, `InHead`, `InBody`, `InTable`, `InTableBody`, `InRow`, `InCell`, `InSelect`, `InTemplate`, `AfterBody`, …).
- `src/tree/open_elements.rs` — stack of open elements + active formatting elements (with the reconstruction algorithm).
- `src/tree/adoption.rs` — the adoption-agency algorithm (misnested formatting elements).
- `src/tree/foster.rs` — foster parenting for text outside table cells.
- `src/tree/implicit_close.rs` — `<p>`/`<li>`/`<td>` implicit end-tag rules, scoping element sets.
- `src/parse.rs` — `pub fn parse(document: &engine_dom::Document, bytes: &[u8], opts: ParseOpts) -> ParseOutcome` (entry point).
- `src/charset.rs` — BOM sniff, `<meta charset>` sniff, UTF-8 assumed otherwise.
- `src/viewsource.rs` — "view as text" fallback for fragments that fail to parse.

**`engine-dom`**
- `src/lib.rs` — `Document` arena: `Vec<Node>`, `Node { kind, parent, first_child, last_child, prev_sibling, next_sibling, dirty_flags }`.
- `src/node.rs` — `NodeKind::Element(ElementData)`, `Text(String)`, `Comment`, `Document`, `Doctype`.
- `src/element.rs` — `ElementData { tag: Tag, attrs: SmallVec<[(LocalName, String)]>, namespace: Namespace }`.
- `src/tree_ops.rs` — `append_child`, `insert_before`, `remove`, `reparent`; maintains the link invariants; used by tree construction **and** by the DOM JS bindings. This is the only place links are mutated.
- `src/serialize.rs` — `innerHTML`/`textContent` for the DOM API surface.
- `src/mutation.rs` — dirty flag propagation used to schedule restyle/relayout.

**`engine-css`**
- `src/tokenizer/state.rs` — CSS Syntax Level 3 tokenizer state machine (§4.1).
- `src/tokenizer/token.rs` — `CssToken::{Ident, Function, AtKeyword, Hash, Delim, Number, Percentage, Dimension, String, Url, Whitespace, Colon, Semicolon, Comma, LCurly, RCurly, LParen, RParen, LBracket, RBracket, Cdo, Cdc, Eof}`.
- `src/parser/top_level.rs` — rule-list parsing: qualified rules, at-rules, `@media`/`@import`/`@font-face`/`@supports`/`@page`/`@keyframes` (parse, mostly discard).
- `src/parser/declaration.rs` — declaration block parsing, `!important`, custom properties (`--x`), var() syntax.
- `src/parser/component_values.rs` — component-value tree (needed for `var()`, `calc()`, `@media` conditions).
- `src/selectors/parse.rs` — selector list → `SelectorList` AST (`Compound`, `Combinator`, `SimpleSelector`).
- `src/selectors/specificity.rs` — `(a,b,c)` triple per selector; `:where()` yields 0, `:is()`/`:not()` take max of args, pseudo-elements count as element, `:nth-child(An+B of S)` max with `S`.
- `src/properties.rs` — flattened `ComputedStyle` struct plus the *longhand* property list (§4.5). One field per longhand.
- `src/shorthand.rs` — expansion table: `margin`, `padding`, `border`, `background`, `font`, `flex`, `grid` (partial), `overflow`, `list-style`, `text-decoration` — expand at parse time into longhands, so the cascade only ever sees longhands.
- `src/values/keyword.rs` — `Keyword` enum for all CSS-wide keywords + `inherit`/`initial`/`unset`/`revert` handling.
- `src/values/length.rs` — CSS length/percentage/calc parsing → `engine_core::Length` and `CalcExpr`.
- `src/values/media.rs` — `MediaQuery { conditions, kind }`, evaluated against `Viewport`.
- `src/ua.css` — the UA stylesheet, embedded `include_str!`, we author it (§4.2).
- `src/lib.rs` — `pub fn parse_stylesheet(src: &str, origin: Origin) -> Stylesheet`.

**`engine-style`**
- `src/lib.rs` — `pub fn compute_styles(doc: &Document, sheets: &[Stylesheet], vp: Viewport) -> StyleTree`.
- `src/origin.rs` — `Origin::UserAgent | User | Author | Inline`; cascade order table (§4.6).
- `src/cascade.rs` — winner selection per longhand: origin/importance, then specificity, then order; inline last; inherited property resolution.
- `src/inherit.rs` — which longhands inherit (a hand-maintained table, cross-checked against `ua.css` by a test).
- `src/media_eval.rs` — `@media` condition evaluation.
- `src/custom_props.rs` — `var()` substitution on the computed style (registered before computed values that reference it).
- `src/animation.rs` — a no-op in v1; kept as the named seam for transitions.

**`engine-select`**
- `src/matcher.rs` — `pub fn matches(sel: &CompoundSelector, el: NodeId, ctx: &MatchCtx) -> bool`, right-to-left.
- `src/index.rs` — per-element precomputed index: `id`, `class_set`, `tag`, `attr_names`, `nth-child index`, `:first/last-child` flags. Computed once per restyle pass.
- `src/cache.rs` — selector → bitset cache: `HashMap<(SelectorId, NodeId), bool>` in a bounded direct-mapped table, invalidated by restyle epoch.
- `src/ancestor.rs` — the ancestor-filter replacement for Bloom filters (§5).
- `src/pseudo.rs` — structural pseudos (`:nth-child(An+B [of S])`, `:first-child`, `:empty`, `:root`, `:not`, `:is`, `:where`, `:has` (single-pass, depth-limited), `:lang`).

**`engine-layout`**
- `src/lib.rs` — `pub fn layout(tree: &StyleTree, vp: Viewport, fonts: &mut FontContext) -> LayoutTree`.
- `src/box_tree/build.rs` — `StyleTree` → `BoxTree`; `display`/`position` decide the box type; anonymous box generation for block-in-inline.
- `src/box_tree/box.rs` — `BoxKind::Block | InlineBox | Text | Replaced | Flex | Grid | Table | TableRow | TableCell | Anonymous`.
- `src/flow/block.rs` — block formatting context, margin collapsing.
- `src/flow/inline.rs` — inline formatting context, line box construction.
- `src/flow/float.rs` — float placement, `clear`, available-width band computation.
- `src/flow/flex.rs` — single-line flex first, then wrap; `flex-direction`, `justify-content`, `align-items`, `flex-grow/shrink/basis`.
- `src/flow/grid.rs` — **stub in v1**; `display:grid` boxes fall back to block (§8).
- `src/flow/table.rs` — **stub in v1**; tables render as block with cell borders (§8).
- `src/flow/positioned.rs` — absolute/fixed: containing-block resolution, static-position fallback.
- `src/fragments.rs` — `LayoutTree { boxes: Vec<Fragment>, scroll_containers: Vec<ScrollArea> }`. Fragment = laid-out box with absolute rect, margin/border/padding/content edges, and children indices. Absolute coordinates, **not** relative — see §3.2.

**`engine-text`**
- `src/font_dir.rs` — enumerate `C:\Windows\Fonts`; map family → file; parse `Segoe UI`, `Arial`, `Times New Roman`, `Consolas`, `Georgia`, `Courier New`, then any `.ttf`/`.otf` on demand.
- `src/font_file.rs` — thin wrapper over `fontdue::Font`: `metrics(px)` → `FontMetrics { ascent, descent, line_gap, units_per_em }`, `has_glyph(c)`, `glyph_index(c)`.
- `src/font_cache.rs` — `FontContext` with `HashMap<FontKey, Arc<FontFace>>`; `FontFace { family, weight, style, data: fontdue::Font, advances: Vec<u16> }`.
- `src/fallback.rs` — missing glyph → per-character closest-font walk (Segoe UI → Arial → any font with a glyph, in a fixed list).
- `src/shaping.rs` — **latin-only** advance-summed shaping with kerning from `kern` table if present, else no kerning. Emits one cluster per character for Latin, one cluster per code point for everything else.
- `src/line_break.rs` — UAX #14 subset: mandatory breaks at newline; break opportunities at space (non-`nowrap`), after hyphen, around CJK ideographs; no hyphenation.
- `src/glyph_atlas.rs` — `Atlas { pages: Vec<Surface> }`, LRU-glyph cache keyed by `(FontId, glyph_id, size_px_quantized)`.
- `src/measure.rs` — `pub fn measure(&self, text: &str, style: &ComputedText) -> Size` — what layout calls.
- `src/paint_text.rs` — glyph run → display-list items or direct blit into atlas.

**`engine-paint`**
- `src/display_list.rs` — `DisplayItem` enum (§6.1), `DisplayList { items: Vec<DisplayItem>, root_scroll: Rect }`.
- `src/build.rs` — `pub fn build(layout: &LayoutTree, fonts: &FontContext, images: &ImageCache) -> DisplayList`.
- `src/backgrounds.rs` — background-color, corner radii, simple linear gradients (2-stop only).
- `src/borders.rs` — solid borders; `dashed`/`dotted` drawn as solid fallback (§8).
- `src/controls.rs` — form control painting (§6.4).
- `src/image.rs` — `ImageCache` (decoded RGBA + mip-less), scaling, `object-fit: cover|contain|fill`, `background-repeat` tile.
- `src/damage.rs` — `damage_rects(old: &LayoutTree, new: &LayoutTree, scroll: &ScrollState) -> SmallVec<[Rect; 4]>`.

**`engine-raster`**
- `src/surface.rs` — `Surface { width: u32, height: u32, stride: u32, pixels: Vec<u8> }` (BGRA8 premultiplied).
- `src/render.rs` — `pub fn render(surface: &mut Surface, list: &DisplayList, clip: Rect)`.
- `src/rect.rs`, `src/rrect.rs`, `src/path.rs` — coverage-based anti-aliased scanline fill; 4×4 supersampled.
- `src/blend.rs` — src-over, multiply (for image opacity), clip mask compositing.
- `src/clip.rs` — rectangular clip stack (no rounded clips in v1; `border-radius` clipping is approximated by rectangular).
- `src/font_raster.rs` — glyph bitmap cache → surface blit with subpixel horizontal positioning quantized to 1/4 px.
- `src/determinism.rs` — pins: no threads in raster in v1, no floating-point reordering, `f32` only, fixed AA sample positions. Test enforces byte-identical output across two runs.

**`engine-platform`**
- `src/window.rs` — `tao` window + event loop; translates `tao` events to `InputEvent` (see boundary statement below).
- `src/gpu.rs` — D3D11 device, DXGI flip-model swapchain, staging texture, `CopyResource` per frame.
- `src/surface_upload.rs` — dirty-rect `Map`/`Write`/`Unmap` of the staging texture when only a region changed.
- `src/timers.rs` — `requestAnimationFrame` clock, coalesced frame scheduling.
- `src/clipboard.rs`, `src/ime.rs` — WM_CHAR / WM_IME events forwarded to `engine-script` when a text input is focused.
- `src/png_out.rs` — PNG encode a `Surface` (uses `png` crate) for golden capture.

**`engine-net`**
- `src/url.rs` — `Url` parse/resolve via `url` crate; we add `is_same_origin`, redirect policy, and scheme allow-list.
- `src/fetch.rs` — `pub fn fetch(req: Request) -> Result<Response, NetError>`; blocking on a worker thread; no HTTP/2 (verify whether `ureq` 3.x pulls it in — if yes, disable).
- `src/redirect.rs` — max 20 hops, method rewrite per RFC 9110, cross-origin header stripping (Authorization, Cookie on downgrade).
- `src/cache.rs` — page-scoped cache keyed by URL; memory only; no disk cache in v1.
- `src/decompress.rs` — gzip/deflate via `zune-inflate`.
- `src/cookies.rs` — **we implement** cookie storage (domain/path/expiry, `SameSite` ignored in v1); no third-party partition.

**`engine-script`**
- `src/lib.rs` — `pub fn run(doc: &Document, win: &WindowHandle) -> ScriptOutcome`; installs a `boa_engine::Context`.
- `src/bindings/mod.rs` — registration of all DOM globals.
- `src/bindings/document.rs`, `element.rs`, `event.rs`, `timers.rs`, `console.rs`, `location.rs`, `window.rs`.
- `src/unsupported.rs` — records every queried-but-missing global so the render report can list them (§10.4).

**`engine` (binary)**
- `src/main.rs` — arg parse (`engine <url>`), window, loop.
- `src/app.rs` — the load pipeline driver: fetch → parse → style → layout → paint → present, and the invalidation scheduler on input.
- `src/session.rs` — sessions: one per window; scroll state and URL.

### 3.2 Key shared data-type decisions

- **All layout coordinates are f32 absolute device pixels** after the root DP scale is applied. `px` in CSS becomes `px * device_pixel_ratio` at computed-value time. There is no per-element transform in v1, so absolute beating relative is correct and makes damage tracking (§6.2) trivial.
- **Arenas, not `Rc`.** Every stage owns its output in a `Vec<T>` indexed by a newtype id. Nodes, styles, boxes, and fragments all live in their own arena. This makes JS/DOM mutations safe to express as "mark dirty, rebuild the tail of the pipeline" without reference cycles.
- **Pipeline stages are pure functions on arenas**, so each is unit-testable by constructing the arena by hand. `engine-layout` tests never need a DOM; `engine-select` tests never need a stylesheet.
- **`tao` boundary (precise):** `engine-platform` is the *only* crate that depends on `tao`. `tao` supplies (i) an `HWND` and `RawWindowHandle`, (ii) window resize / DPI / focus / close events, (iii) keyboard, mouse, wheel, IME events. It supplies **zero pixels**. Coordinates on the window are fed to `engine-platform::gpu`, which creates the swapchain from `HWND` + HWND's client rect. `engine` never draws through a web widget because there is none in the dependency tree.

---

## 4. Pipeline, stage by stage

```
bytes ─▶ Tokens ─▶ DOM ─▶ StyleTree ─▶ BoxTree ─▶ LayoutTree ─▶ DisplayList ─▶ Surface
        (html)   (dom)   (style)     (layout)   (layout)      (paint)        (raster)
```

All stage boundaries carry a `RenderReport` (§10.4) so failures are attributable.

### Stage 0 — Fetch
```rust
// engine-net
pub fn fetch(req: Request) -> Result<Response, NetError>;

pub struct Request { pub url: Url, pub method: Method, pub headers: Vec<(String, String)>, pub body: Option<Vec<u8>> }
pub struct Response { pub url: Url, pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
```

### Stage 1 — HTML tokenize
```rust
// engine-html::tokenizer
pub fn tokenize(input: &[u8]) -> Tokens  // iterator; never panics, never allocates per char

pub struct Tokens<'a> { /* state machine */ }
impl<'a> Iterator for Tokens<'a> { type Item = Token; }
```

### Stage 2 — Tree construction
```rust
// engine-html::parse
pub fn parse(doc: &Document, bytes: &[u8], opts: ParseOpts) -> ParseOutcome;

pub struct ParseOutcome { pub errors: Vec<HtmlError>, pub root: NodeId, pub quirks: QuirksMode }
pub struct ParseOpts  { pub fragment_context: Option<(Tag, Namespace)>, pub charset: Option<&'static Encoding> }
```
Note this **mutates** the arena (`&Document` holds `RefCell`/unsafe-free cells internally via a `Cell<Vec<Node>>` pattern; see `engine-dom::Document`). The alternative — returning a raw tree and copying it into the DOM — wastes a full traversal per load and complicates the incremental JS story, so we do it in one place: `engine-dom::tree_ops` is the sole link mutator, and tree construction calls it exclusively. This is the one "tokenizer glue" we write rather than taking `html5ever`; the tree-construction insertion-mode table is ~2,000 lines of our own code and is the single largest fixture-tested module.

### Stage 3 — Stylesheet collection + cascade
```rust
// engine-css
pub fn parse_stylesheet(src: &str, base: &Url, origin: Origin) -> Stylesheet;

// engine-style
pub fn compute_styles(doc: &Document, sheets: &[Stylesheet], vp: Viewport) -> StyleTree;

pub struct StyleTree { pub styles: Vec<ComputedStyle>, pub owners: Vec<NodeId>, pub inline: HashMap<NodeId, ComputedStyle> }
pub struct Viewport  { pub width_css: f32, pub height_css: f32, pub dpr: f32 }
```
`ComputedStyle` is a fixed-layout struct of longhand fields (e.g. `margin_top: Length`, `display: Display`, `color: Rgba8`, `font_size_px: f32`). Fixed layout means one `memcpy` per node per restyle; heap indirection only for `content` and custom properties.

### Stage 4 — Box tree
```rust
// engine-layout::box_tree
pub fn build_box_tree(styles: &StyleTree, doc: &Document) -> BoxTree;

pub struct BoxTree { pub boxes: Vec<Box>, pub root: BoxId }
pub struct Box { pub kind: BoxKind, pub style: StyleId, pub children: Vec<BoxId>, pub content: Option<NodeId> }
pub enum BoxKind { Block, Inline, Text, Replaced(ReplacedKind), Flex, Grid, Table, TableRow, TableCell, Anonymous }
```

### Stage 5 — Layout
```rust
// engine-layout
pub fn layout(tree: &BoxTree, styles: &StyleTree, vp: Viewport, fonts: &mut FontContext, images: &ImageCache) -> LayoutTree;

pub struct LayoutTree { pub fragments: Vec<Fragment>, pub root: FragmentId }
pub struct Fragment { pub box_id: BoxId, pub rect: Rect, pub margin: EdgeSizes, pub border: EdgeSizes, pub padding: EdgeSizes, pub content: Rect, pub scroll: Option<ScrollInfo>, pub children: Vec<FragmentId> }
```

### Stage 6 — Display list
```rust
// engine-paint
pub fn build(layout: &LayoutTree, styles: &StyleTree, fonts: &FontContext, images: &ImageCache) -> DisplayList;

pub struct DisplayList { pub items: Vec<DisplayItem>, pub bounds: Rect }
```

### Stage 7 — Raster + present
```rust
// engine-raster
pub fn render(surface: &mut Surface, list: &DisplayList, clip: Rect);

// engine-platform
pub fn present(swapchain: &mut Swapchain, surface: &Surface, dirty: Option<Rect>) -> PresentResult;
```

### Stage 8 — Input loop (invalidation)
```rust
// engine::app
pub enum Invalidations { Restyle(BitSet), Relayout(BitSet), Repaint(Rect), None }
pub fn handle_input(&mut self, ev: InputEvent) -> Invalidations;
```
Scroll → repaint only (§6.3). Hover → restyle of the affected subtree if any `:hover` selector exists anywhere in the sheet, else none. A restyle epoch counter on the stylesheet lets us skip hover work entirely on pages with no `:hover` rules — a cheap global fast path.

---

## 5. CSS parser design (from scratch)

### 5.1 Tokenizer state machine

Implement CSS Syntax Level 3 §4.3 states as an explicit `enum CssState` + `loop { match state { ... } }` dispatcher:

`Data`, `IdentLike`, `Number`, `Numeric`, `Dimension`, `String`, `StringDouble`, `StringSingle`, `Escape`, `EscapeStart`, `Hash`, `HashEscape`, `Comment`, `CommentEnd`, `CommentDash`, `Url`, `BadUrl`, `UrlEscape`, `UrlBadHost`, `UrlBadPort`, `UrlBadPath`, `UrlBadQuery`, `UrlBadFragment`, `UrlBare`, `UrlBareEscapeSplit`, `UrlBareEscape`, `UrlBareIdle`, `UrlBareInterrupted`, `UrlBareInterruptedSplit`.

Whitespace runs coalesce into one `Whitespace` token (comment handling: comments are discarded, not whitespace — except where a comment splits an ident, which produces two idents, matching spec). Every tokenizer test is a fixture in `testdata/fixtures/css/tokenizer/*.txt` with the token list as a second file. This stage is fully unit-testable and is the first thing built.

### 5.2 Parser

Consumes tokens into rules with a `Parser` struct that can be put in "nested at block" mode (needed by `@media`, `@supports`, style blocks inside HTML). Error recovery: any invalid declaration is dropped and parsing resumes at the next `;` at the current nesting level; invalid rules are dropped and parsing resumes at the next `}`. We never bail out of a stylesheet on first error.

`@media` is parsed into a `MediaQuery` AST and evaluated in `engine-style::media_eval`. We honour `screen`, `all`, and the media features `width`, `height`, `aspect-ratio`, `orientation`, `min-width`, `max-width`, `min-height`, `max-height`, `prefers-color-scheme`, `prefers-reduced-motion`; `and`/`,`/`not`; ranges (`(400px <= width <= 700px)`). We do not implement `@container`.
`@import` is resolved during stylesheet parsing (top-level, before any rule that isn't `@charset`/`@layer`).
`@layer` is parsed and treated as cascade order; we implement a flat layer list per origin.

### 5.3 Shorthand expansion

Expansion happens **at parse time** into longhands, into a `DeclarationBlock { longhands: Vec<(PropertyId, DeclaredValue)> }`. Consequences: the cascade never sees shorthand-syntax errors; a shorthand with one invalid value is either partially expanded (where CSS says so, e.g. `border: 1px solid` leaves color `currentcolor`) or dropped if it has no default. The table of shorthands, their longhands, and their initial values lives in `src/shorthand.rs` as a `const &[ShorthandSpec]` and is checked by a test that every longhand field in `ComputedStyle` appears in at least one shorthand.

### 5.4 Specificity

`Specificity { a: u32, b: u32, c: u32 }`. `:is()`/`:not()`/`:has()` take the max of their argument selectors; `:where()` contributes `(0,0,0)`; `:nth-child(An+B of S)` takes max with `S`. `style` attribute sorts above any author selector of any specificity, but below `!important` in the author origin.

### 5.5 Inheritance

A static `INHERITED: &[PropertyId]` table. Inherited properties copy from parent's computed value; non-inherited take the initial value. `inherit`/`initial`/`unset`/`revert` are resolved in `cascade.rs` after the winner is chosen. `font-size` inheritance is resolved numerically first (computed value, not declared) — this is the classic em-cascade bug and gets a dedicated fixture.

### 5.6 Units

Absolute: `px`, `cm`, `mm`, `in`, `pt`, `pc`, `Q` (96 dpi basis). Relative: `em`, `rem`, `%` (context-dependent — left as `Length::Pct` for layout to resolve against the correct box), `vw`/`vh`/`vmin`/`vmax`, `ex`/`ch` resolved from font metrics as `0.5em`/advance of `0`. `calc()`: parsed to a small tree of `Add/Sub/Mul/Div` + leaves; division by non-zero length, multiplication by a number only.

### 5.7 CSS we will not support (summary; full table at end)

`@container`, `@scope`, `@layer` beyond flat ordering, nesting (`&`), `:has()` beyond depth-2, `@property`, `@counter-style`, animations (`@keyframes`, `transition`, `animation`), `will-change`, `contain`, `filter`, `backdrop-filter`, `transform` (all of them), `clip-path`, `mask`, `mix-blend-mode`, `writing-mode` other than `horizontal-tb`, `aspect-ratio` beyond replaced elements, `grid-*`, `multi-column`, `@font-face` (we use system fonts only), `@supports` evaluation (always true for the declarations we support? **no** — we evaluate `@supports (prop: value)` as true iff `prop` is a longhand we parse), `-webkit-*` properties (ignored), logical properties (`margin-inline-start` etc. are aliased to physical LTR equivalents only).

---

## 6. Selector matcher design (from scratch)

### 6.1 Right-to-left matching

A selector list is compiled once into `CompiledSelector { compounds: Vec<Compound>, combinators: Vec<Combinator> }` in **reverse order**. Matching starts from the candidate element and the rightmost compound; each match attempt moves left across combinators:

```
match_from(el, idx):
  if !match_compound(compounds[idx], el): return false
  if idx == 0: return true
  match combinators[idx-1]:
    Descendant  => walk ancestors, return true if any match_from(a, idx-1)
    Child       => return match_from(parent(el), idx-1)
    AdjacentSibling => return match_from(prev_sibling(el), idx-1)
    GeneralSibling  => walk prev siblings
    Column      => never (unsupported)
```

`match_compound` is a set of cheap checks in fixed order: tag (namespace+local name), id, classes (each class looked up in a per-node `HashSet<&str>` built once per restyle), attributes, then pseudos last (structurally most expensive).

### 6.2 Caching

- **Per-element index** built once per restyle epoch; O(nodes) build, O(1) per compound test.
- **Selector→bitset cache:** for each `(SelectorId, bucket_epoch)` we keep a `BitSet` of nodes that already matched. Since selectors are evaluated against a fixed tree per epoch, a match is reusable for every element tested later.
- **Rightmost-compound index:** map from tag → list of selectors ending in that tag; map from class → selectors containing that class. On restyle of subtree S, only selectors whose rightmost compound could match something in S are tried. This is the closest analogue to Style's rule hash, built at stylesheet-parse time.

### 6.3 Without a bloom-filter ancestor cache

Chromium/Gecko use a bit-array of ancestor class/id/tag hashes to short-circuit descendant combinators. We replace it with two cheaper structures:

1. **Ancestor signature stack.** During restyle we walk the tree depth-first and maintain a stack of `(TagId, has_class: u64 bitset over class-slot ids)` frames. Testing "some ancestor has class `.x`" is then a scan over the stack (depth is typically < 30) checking one bit. This is exact, not probabilistic — no false positives, and it allows *negative* answers to be cached too.
2. **Rightmost-compound typing.** Selectors that end in `.foo` are only ever tried against elements with class `foo`, so the descendant combinator walk happens rarely. The expensive path is generic `div div a`-style selectors; the ancestor-signature stack makes each step one bit test.

Blake-class Bloom filters exist to avoid walking ancestors at all. Our stack is a contiguous `Vec<AncestorFrame>` so the walk is a cache-friendly `for` loop; the constant factor trade is a small slowdown vs. a Bloom hit, and a large win in false-positive avoidance. User-facing consequence: right-to-left matching is correct on every selector we claim to support; performance is the only cost.

### 6.4 Known correctness holes

- `:has()` is evaluated with a depth-2 descendant search only, and any selector using `:has()` is excluded from the selector cache (it can become stale after a subtree mutation).
- `:nth-child(An+B of S)` on a filter `S` is supported only when `S` is a single compound selector.
- `:focus-visible` / `:focus-within` are supported; `:hover` only while the pointer is over the element or a descendant; no `:active` beyond mousedown state.

---

## 7. Layout algorithm

Order of implementation, each stage gated by its own fixtures. A stub means: the box is laid out as `display: block`, children in normal flow, and the render report records `Approximation::Stubbed(feature)`.

### 7.1 Block formatting context

Top-down with intrinsic min/max-content width, then a second pass for percentage resolution against the containing block's final width. Two-phase: **measure** (compute min-content and max-content widths bottom-up) then **arrange** (distribute width, place boxes, collapse margins). Margins collapse between adjacent block siblings and between a block and its first/last child; collapsing is suppressed across a BFC boundary, a float, an out-of-flow box, a border/padding edge, and `overflow` other than `visible`. We implement the spec's collapse rules without `display: flow-root` (unsupported; `overflow:hidden` is the workaround we document).

### 7.2 Float

Floats are placed before inline content is laid out, per `float-left`/`float-right`. We maintain a per-BFC list of float rects and compute, for each line, the available horizontal band by intersecting the float rects with the line's vertical span. `clear` is honoured. Floats do not intrude into a block-level box's border box. This is implemented early because a quarter of "documents" depend on it.

### 7.3 Inline formatting context and line breaking

A line box is built from inline-level items (text runs, inline boxes, replaced elements). For each candidate line: measure the run through `engine-text::measure`, find break opportunities via UAX #14 subset, fill greedily, then apply `text-align` and `vertical-align` (we implement `baseline`, `top`, `bottom`, `middle`, `sub`, `super`, and length/percent values). `white-space` supports `normal`, `nowrap`, `pre`, `pre-wrap`, `pre-line`. Shaping limits in §8 make this correct for Latin and approximate for everything else.

### 7.4 Replaced elements

`<img>`, `<video>`, `<canvas>`, `<iframe>`, `<input>` (some types), `<select>`: intrinsic size from decoded image dimensions or a fixed default (300×150 for images without dimensions), then `width`/`height`/`max-*` constraints, then `object-fit`.

### 7.5 Absolute / fixed positioning

Containing block is the nearest ancestor with `position != static` (or the viewport for `fixed`), which requires a separate pass after normal flow, plus the "static position" fallback when `top`/`left` are `auto`. Absolute boxes are laid out in document order after normal flow, and their height does not contribute to ancestors.

### 7.6 Flexbox

Single-line then multi-line. Algorithm: determine main size of items from `flex-basis`/`width`, resolve `flex-grow`/`flex-shrink` with the min/max clamp loop (spec §9.7 "Resolving Flexible Lengths"), then cross-size and `align-items`/`align-self`. Nested flex is supported. `flex` shorthand expansion is done at parse time. Known gap: `gap` is supported, `flex-wrap: wrap-reverse` is not.

### 7.7 Table (stub in v1)

Rendered as `display: block` with each `<tr>` as a block row and cell backgrounds drawn as boxes. Column widths are not computed; `border-collapse` is ignored. Render report: `Stubbed("table")`.

### 7.8 Grid (stub in v1)

`display: grid` falls back to block. Render report: `Stubbed("grid")`. We believe grid is where most of the modern web is, so this stub is the single biggest driver of "partially rendered" (§0). It is deliberately deferred until block/inline/float/flex fixtures pass.

### 7.9 Scroll containers

Any box with `overflow: auto|scroll` on either axis becomes a `ScrollInfo { content_size, viewport_size, offset }`. Layout is computed at the content's full size; the scroller clips on paint. Root scroller is special (it is the viewport) and never has a visible scrollbar in v1.

---

## 8. Paint and compositing

### 8.1 Display list

Flat, painted in order; each item carries its clip rect and a `BoxId` back-ref (used by damage tracking and hit-testing).

```rust
pub enum DisplayItem {
    FillRect  { rect: Rect, color: Rgba8, radii: CornerRadii },
    Border    { rect: Rect, widths: EdgeSizes, colors: [Rgba8; 4], style: BorderStyle },
    Image     { dst: Rect, src: ImageKey, fit: ObjectFit, opacity: f32, repeat: BackgroundRepeat },
    LinearGrad{ rect: Rect, from: Rgba8, to: Rgba8, angle_deg: f32 },
    GlyphRun  { origin: Point, font: FontKey, clusters: Vec<ShapedCluster>, color: Rgba8, baseline: f32 },
    TextCursor{ rect: Rect },
    Scrollbar { track: Rect, thumb: Rect, axis: Axis },
    Control   { rect: Rect, kind: ControlKind, state: ControlState },
    PushClip  { rect: Rect },
    PopClip,
}
```

### 8.2 Damage tracking

Two sources of damage:
1. **Scroll**: unchanged content shifted by `(dx, dy)` → damage is only the newly exposed strip plus the scrollbar. Implemented by re-rendering only items intersecting `viewport ∩ (band)`.
2. **Layout change** (`damage_rects`): walk old and new `LayoutTree` in parallel by `BoxId`; any fragment whose four edge coordinates changed contributes its union with the old and new rects. Union into at most 4 rectangles (a simple greedy merge; we do not implement a full region algebra).

Full repaint on: load, resize, font load, image decode completion, zoom.

### 8.3 Scroll handling

Scroll offset lives in `ScrollState { offsets: HashMap<ScrollContainerId, (f32, f32)> }`, outside layout. Scrolling **never** re-runs layout for `overflow: auto` boxes; it re-runs paint with a translated display list and a new clip. Wheel events are coalesced per frame. This is the single most important performance decision: without it, scrolling is a full relayout at 60 Hz.

### 8.4 Images

Decoded at natural size into `ImageCache` (BGRA premultiplied). Scaling uses nearest-neighbour without quality expectations? **No** — nearest looks catastrophic on text-in-image. We use a separable box filter for downscale and bilinear for upscale, implemented in `engine-raster::image_scale`. `background-repeat: no-repeat|repeat` are supported. `background-position` and `background-size` support keywords and percentages; no multiple backgrounds.

### 8.5 Form controls

Painted by us, not by the OS: `<button>`, `<input type=text|password|checkbox|radio|submit>`, `<textarea>`, `<select>` (closed state only — no popup; clicking a `<select>` shows a debug report that popups are unsupported). Controls use a small fixed token set (border, background, padding) that mirrors the UA stylesheet so they look reasonable, not native. Documented clearly as approximation.

---

## 9. Text rendering

### 9.1 Loading

`engine-text::font_dir` enumerates `C:\Windows\Fonts\*.ttf|*.otf`. Family resolution is by internal name read with `fontdue::Font::from_bytes` (its `FontSettings` exposes the names), with a hardcoded priority list for the CSS generic families:

| CSS family | Windows file(s) used |
|---|---|
| `ui-sans-serif`, `system-ui` | `segoeui.ttf`, bold `segoeuib.ttf` |
| `sans-serif` | `arial.ttf`, `arialbd.ttf` |
| `serif` | `times.ttf` / `timesbd.ttf` |
| `monospace` | `consola.ttf` |
| `cursive`/`fantasy` | mapped to `sans-serif` (documented approximation) |

`font-weight` selects the nearest available face we parsed (400/700 only in v1; 100–900 are mapped by threshold). `font-style: italic` falls back to the regular face; we do not synthesize an oblique (documented; synthetic oblique is a future item).

### 9.2 Shaping limits (stated plainly)

We implement **advance-summed** positioning with an optional `kern` table lookup and no mark repositioning, no GSUB/GPOS, no bidi reordering.

- **Latin, Cyrillic, Greek, CJK (Han)**: advance-summed is close to correct for horizontal text. CJK is fine because syllables are single glyphs.
- **Arabic, Hebrew, Persian, Urdu**: letters are joined and shaped. Without GSUB, Arabic renders as isolated, disjoined letterforms and **without bidi reversal**. Hebrew renders in logical order (left-to-right on screen) — **wrong**. We mark `dir`/RTL content as `Approximation::RtlUnsupported` in the report and display it as best-effort.
- **Devanagari, Bengali, Tamil, Thai, Khmer**: complex clusters and reordering are not implemented. Text will render with incorrect glyph forms and potentially incorrect order.
- **Combining marks** are placed by summing advances; they will be off on many fonts.

This means: any site not in a Latin/CJK script will look visibly broken. That is accepted and reported.

### 9.3 Measuring without a full shaper

Layout needs `measure(text, style) -> Size` before any glyph rasterization. We compute:
- width = Σ `advance(glyph)` from `hmtx` at the requested px size, plus kerning if the `kern` table is present, plus `letter-spacing`;
- height = `(ascent + descent + line_gap)` from `hhea` scalede, times `line-height` when it is a number or percentage;
- `font-size` px → `units_per_em` scaling done in f32 once per run, with advances precomputed per face at integer px sizes (0.25 px quantization) and linear interpolation for fractional sizes.

Fractional advances accumulate error across a long line; we snap the line's final width, not each advance, so line breaking decisions are stable.

### 9.4 Glyph rasterization

`fontdue` gives us a coverage bitmap per glyph at a requested px size. We cache at quantized sizes (0.25 px steps) in the atlas, and blit with a 1/4-px horizontal subpixel offset by sampling the atlas at an offset. No LCD subpixel AA (RGB fringing) in v1 — grayscale AA only. Consequence: text is slightly softer than ClearType, and identical to what a golden test expects.

---

## 10. JS strategy (`boa_engine`)

### 10.1 Integration

`engine-script` installs a `boa_engine::Context` per document and registers globals backed by handle tables (`Vec<Option<ElementRef>>` slots with an index stable across GC). Scripts run after parse (classic + `defer`), inline and external; `type=module` is **not** supported (no module loader, no import maps). `document.write` during parse is supported via a re-entrant tree-construction callback only in v1 for `defer` scripts at document end; dynamic writes after load are not.

### 10.2 Exposed DOM surface

Exposed: `window` (location, timers, `addEventListener`, `innerWidth/Height`), `document` (`getElementById`, `querySelector(All)` — mapped to our selector engine, `createElement`, `createTextNode`, `body`, `head`, `title`, `cookie` (read-only), `readyState`, `addEventListener`), `Element` (`innerHTML`, `textContent`, `setAttribute`/`getAttribute`/`classList`, `style` on longhands only, `children`, `appendChild`, `removeChild`, `insertBefore`, `addEventListener` for `click`, `submit`, `input`, `load`), `Node` traversal, `Event`/`MouseEvent` basics, `console.log`.

Not exposed: `fetch`/`XMLHttpRequest`, `localStorage`/`sessionStorage`, `WebSocket`, `Worker`, `requestAnimationFrame` (present but fired only from our frame loop; no visual guarantee), `Shadow DOM`, `Custom Elements`, `Web Animations`, `FormData`, `File`, `Notification`, `Geolocation`, `Canvas` 2D context, CSSOM mutation (`insertRule`, `getComputedStyle` returns our computed values but read-only), `IntersectionObserver`, `MutationObserver`.

### 10.3 Consequence, blunt

A page whose content is built by JS — which includes nearly every modern SPA, React/Vue/Svelte app, and most e-commerce and dashboard sites — will show **nothing**, or a shell with a `<div id="root"></div>` and a spinner that never stops. We detect this specific shape (large empty container, script count > 3, no text content under `<body>`) and show a banner: *"This page appears to require JavaScript that the engine does not support."* Any DOM code that calls `fetch` or a missing global throws and is caught; the render report lists the top missing globals in order of frequency.

### 10.4 Render report

Every load produces `RenderReport`:

```rust
pub struct RenderReport {
    pub url: Url,
    pub parse_errors: Vec<HtmlError>,
    pub css_errors: Vec<CssError>,
    pub unsupported_selectors: Vec<String>,
    pub approximations: Vec<Approximation>,     // Stubbed("grid"), RtlUnsupported, MissingGlyphs(script), NoImagesPlaceholder, ...
    pub script_errors: Vec<ScriptError>,
    pub missing_globals: Vec<(String, u32)>,
    pub counts: StageCounts,                    // nodes, boxes, display items, ms per stage
}
```
Shown as an overlay on `F12`-equivalent keystroke, and written to stdout when launched with `--report`.

---

## 11. Performance & quality plan

Budgets are targets to be measured, not claims.

| Metric | Budget | Gate |
|---|---|---|
| CSS tokenizer | ≥ 50 MB/s | `cargo bench` in `engine-css` |
| HTML tokenize+construct, 1 MB page | < 60 ms | fixture benchmark |
| Selector match, full restyle of 5k nodes | < 15 ms | fixture benchmark |
| Block+inline layout, 2k boxes | < 25 ms | fixture benchmark |
| Display list build | < 5 ms | fixture benchmark |
| Full repaint 1280×800 | < 12 ms | raster bench on this machine |
| Scroll repaint (exposed strip) | < 3 ms | raster bench |
| Present (D3D11 blit) | < 1 ms | GPU timing query |
| Session restore (layout+display list, no fetch) | < 120 ms | fixture |
| Memory, 2k-node page | < 80 MB RSS | process measurement |

Instrumentation: per-stage `Instant` timings into `RenderReport::counts`; a `--trace` flag dumping per-stage durations per load; in the app, a frame graph overlay (last 60 frames of each stage). No RUM — this tool runs on the owner's machine. Quality gates: (1) every stage fixture passes, (2) golden PNGs byte-identical on two runs, (3) budgets above met on the 3 representative fixtures, (4) no panics on a 50-URL smoke list (report-only, not correctness).

---

## 12. Networking + URL + security

- **URL**: `url` crate for parse/resolve; we add scheme allow-list (`http`, `https`, `about`), same-origin comparison (scheme + host + port, default port normalized), and a `ResourceKind` (`Document`, `Stylesheet`, `Script`, `Image`, `Font`) used for policy.
- **Fetch**: one worker thread; a `ureq` agent per session. Redirects handled by `engine-net::redirect` (max 20, per-RFC method rewrite, `Authorization` and `Cookie` stripped on cross-origin or scheme downgrade).
- **Subresources**: `<link rel=stylesheet>`, `<script src>`, `<img>`, `<link rel=icon>` (unused). No `<iframe>` document loading (the box is drawn empty with a report note). No `srcset`/`picture`.
- **Response limits**: body capped at 32 MB, image decode capped at 64 MPix, stylesheet cap 8 MB, each with report entries when hit.
- **HTML sanitization**: none — we render what we receive, as a browser does. We do not execute `on*` attributes as script; event handlers are only wired through `addEventListener` bindings. (We deliberately do not implement `eval`-class attribute handlers.)
- **Security, one honest paragraph:** This engine has **no sandbox**. The process runs with the user's full Win32 token: no restricted token, no AppContainer, no seccomp-equivalent, no site isolation, no separate renderer process. A malicious page that finds a memory-safety bug in our parser, rasterizer, image decoder, or `fontdue` gets code execution as the user. Buggy internal navigation is not more dangerous than that. There is also no CSP enforcement, no mixed-content blocking, no TLS certificate pinning beyond rustls defaults, no permission prompts, and cookies are global rather than partitioned by top-level site. **Do not browse the open internet with untrusted content in this engine.** Treat it as a local development tool: fixtures, saved pages, intranet, docs. Any future "point it at the real web" use requires first running it inside an AppContainer or a low-integrity job object, which we have not designed.

---

## 13. Verification plan (no reference browser)

The premise is that nothing on this machine is a trusted oracle for "what the page should look like." Therefore three independent test layers, none of which needs a second browser.

### 13.1 Parser fixtures (exact, hand-authored)

- **HTML tokenizer + tree construction:** fixtures we author from the WHATWG spec's own examples, each as `testdata/fixtures/html/<name>.html` plus `<name>.tree.txt` (a canonical serialization of `Document`: element, attributes in document order, text with escapes). Tree construction failures are reported as a diff against `.tree.txt`. These fixtures are written by hand and are the oracle: if we disagree with the spec text, the fixture is fixed by reading the spec, not by copying a browser.
- **CSS tokenizer:** per-state fixtures `testdata/fixtures/css/tokenizer/*.txt` + `*.tokens` where `*.tokens` is one line per token `Type(value)`.
- **CSS cascade:** fixtures `testdata/fixtures/cascade/<name>.html` (inlinable small sheets) + `<name>.computed.txt` — per-node, sorted longhand `name: value` lines. Written by hand from the cascade spec. This is the highest-value test layer because computed values are where CSS bugs are visible without pixels.

### 13.2 Layout assertions (structural, no pixels)

`testdata/fixtures/layout/<name>.html` + `<name>.layout.txt`: for every `BoxId` with an id-bearing element or a `data-test` attribute, the expected `(x, y, w, h)` of margin/border/content edges, rounded to 0.5 px. These are authored by hand for each fixture from the CSS box model on paper. Flat 5×5px grids of `<div>`s give us the arithmetic; nested/float/flex cases give us the hard ones. Structural assertions catch the bug class that golden pixels hide (a wrong border width compensated by a wrong padding).

### 13.3 Golden pixels

- **Generation**: `cargo run -p engine-golden -- testdata/fixtures/golden/ --out testdata/goldens/` renders each HTML fixture at a fixed 800×600 viewport, DPR 1.0, with a fixed font set (Segoe UI, Arial, Consolas from `C:\Windows\Fonts` — copied into `testdata/fonts/` and version-pinned, because system fonts change between Windows builds and that would silently change every golden), then PNG-encodes the surface.
- **First golden = review artifact.** A new golden is *not* accepted as truth. Each new golden is written to `test-results/goldens-new/` and must be visually reviewed against the fixture's spec-derived test file (§13.2) before it is copied into `testdata/goldens/`. Goldens are only as trustworthy as the first human review; after that they guard against regressions.
- **Comparison**: `engine-raster::determinism` guarantees byte-identical output for the same binary, so the harness compares **exact bytes first**. If bytes differ, the test reports a perceptual diff: per-channel absolute difference, count pixels where `max(|Δr|,|Δg|,|Δb|,|Δa|) > 8`, and fail if `diff_pixels / total_pixels > 0.1%` **or** if any 16×16 tile has more than 25% differing pixels (catches localized text displacement that a global percentage hides). Diff images are written to `test-results/`.
- **Determinism guard**: a test renders the same fixture twice in one process and asserts the two surfaces are byte-equal; runs before the golden tests so a non-determinism failure is never misdiagnosed as a rendering regression.

### 13.4 Smoke list

A committed `testdata/urls.txt` (30 URLs, saved-as-fixtures where possible to avoid network dependence; the live URLs are optional and marked `#[ignore]` by default). The smoke test only asserts: no panic, no infinite loop (stage timeout 10 s), and a non-empty display list or a reported reason. It never asserts visual correctness against another engine.

---

## 14. What we keep / kill / invent / what could go wrong

**Keep:** the CSS box model, cascade, specificity, and block/inline flow as specified — they are the contract every page is written against. The `tao` window and D3D11 present path. System fonts. Fixture-driven verification.

**Kill:** none of the web platform itself; we kill the browser-as-given: we do not build tabs, history UI, extensions, or DevTools beyond the render report. We do not attempt full shaping, bidi, grid, tables, or sandboxing in v1.

**Invent:** nothing novel in the platform semantics — this project's invention is a strict, reported approximation budget: every drop is labeled in `RenderReport`, and the Not-supported table below is the external contract.

**What could go wrong (ranked honestly):**
1. **Grid and table stubs** mean a large fraction of modern layout is wrong from the start, and block fallback can cascade into many bad pages. Highest risk.
2. **Text shaping limits** make every non-Latin/CJK page visibly broken, and bidi is wrong, not just ugly.
3. **Golden-test self-deception:** if a visually wrong golden is accepted early, it freezes the bug. Mitigation is the fixture-review step in §13.3; it depends on the owner actually doing that review.
4. **Security:** no sandbox; a rendering crash or memory bug is a full-token compromise. See §12.
5. **HTML tree construction** is the largest single risk of subtle bugs; the adoption-agency algorithm and insertion modes are where hand-written parsers typically diverge. Fixture coverage must be wide before the smoke list is trusted.
6. **Performance:** CPU rasterization at 4K may miss budget on image-heavy pages; damage tracking covers scroll but not a full repaint.

---

## 15. Consolidated "Not supported" table

| Area | Item | What happens instead |
|---|---|---|
| CSS at-rules | `@container`, `@scope`, `@property`, `@counter-style`, `@font-face`, `@page`, `@layer` nesting beyond flat list | parsed or skipped; report entry `Approximation::SkippedAtRule` |
| CSS at-rules | `@supports` | evaluated as "declaration is a longhand we parse" |
| CSS | nesting (`&`), `@keyframes`, `transition`, `animation` | declarations/rules dropped; no motion |
| CSS values | `calc()` with mixed units beyond Length×number, `min()`, `max()`, `clamp()` | `min/max/clamp` unsupported; calc limited to add/sub/mul/div on lengths |
| CSS layout | `display: grid` | falls back to block; `Stubbed("grid")` |
| CSS layout | `display: table*`, `border-collapse`, column widths | block fallback; `Stubbed("table")` |
| CSS layout | multi-column, `column-*` | ignored |
| CSS layout | `aspect-ratio` on non-replaced boxes | ignored |
| CSS layout | logical properties (`margin-inline-*`, `inset-*`) | aliased to physical top/right/bottom/left only |
| CSS layout | `writing-mode` other than `horizontal-tb`, `direction: rtl` | ignored; RTL renders left-to-right |
| CSS layout | `flex-wrap: wrap-reverse` | treated as `wrap` |
| CSS layout | `subgrid` | n/a (grid stubbed) |
| CSS effects | `transform`, `filter`, `backdrop-filter`, `mask`, `clip-path`, `mix-blend-mode`, `opacity` stacking context semantics | ignored or approximate; `opacity` applied to self only |
| CSS effects | `box-shadow`, `text-shadow`, `outline` beyond a rectangle | shadow ignored; outline drawn as a rectangle |
| CSS effects | `border-style` other than solid + the drawn approximation for dashed/dotted | drawn as solid |
| CSS effects | gradients beyond 2-stop linear | last two stops used |
| CSS effects | `will-change`, `contain`, `content-visibility` | ignored |
| CSS selectors | `:has()` beyond a depth-2 descendant search | may fail to match; excluded from cache |
| CSS selectors | `:nth-child(An+B of S)` where S is not a single compound | unsupported; selector never matches |
| CSS selectors | `::before`, `::after`, `::first-line`, `::first-letter`, `::selection`, `::placeholder` | not generated; pseudo-elements absent |
| CSS selectors | Shadow DOM piercing, `:host`, `::part` | no Shadow DOM at all |
| CSS units | `cqw`, `cqh`, `lh`, `rlh`, `vi`, `vb` | unsupported; value dropped |
| HTML | tree construction full insertion-mode fidelity | implemented, but fidelity is fixture-verified, not exhaustive; unknown elements render as blocks |
| HTML | `<iframe>` document loading, `<object>`, `<embed>`, `<svg>` content, `<canvas>` content | box drawn empty; `<svg>` inline not rendered |
| HTML | `srcset`, `<picture>` | `src` used |
| HTML | `<template>` content instantiation | `<template>` children not rendered |
| HTML | Custom elements, shadow DOM, declarative shadow DOM | ignored |
| HTML | `contenteditable`, selection, drag-and-drop | not supported |
| Forms | `<select>` popup, `<input type=date|color|range|file>` widgets, `<input type=hidden>` is honoured | control painted as a static plain box; only text/checkbox/radio/submit are functional |
| Forms | form submission | not implemented |
| Script | `type=module`, dynamic `import()`, import maps, service workers | scripts not run; report entry |
| Script | Network APIs (`fetch`, `XMLHttpRequest`, `WebSocket`), `Worker` | throw; `missing_globals` entry |
| Script | Storage (`localStorage`, `sessionStorage`, IndexedDB) | absent |
| Script | Canvas 2D/WebGL, Web Animations, `MutationObserver`, `IntersectionObserver`, `ResizeObserver` | absent |
| Script | `getComputedStyle` for non-implemented properties | returns our values for supported longhands only |
| Script | `document.write` after load, `javascript:` URLs | ignored |
| Script | `eval`-class inline handlers (`onclick="..."`) | not compiled; only `addEventListener`-registered handlers run |
| Text | Full OpenType shaping (GSUB/GPOS), ligatures beyond none | advance-summed; ligatures off |
| Text | Bidi (Arabic, Hebrew, Persian, Urdu) | logical order rendered LTR; visibly wrong |
| Text | Complex scripts (Devanagari, Bengali, Tamil, Thai, Khmer, Myanmar) | broken glyph forms/order |
| Text | Hyphenation, `hyphens`, `text-justify: inter-character` | none |
| Text | `font-variant-*`, `font-feature-settings` beyond none | ignored |
| Text | Synthetic bold/italic | not synthesized; nearest real face or regular |
| Text | LCD subpixel antialiasing | grayscale AA only |
| Text | Web fonts (`@font-face`), variable font axes | system fonts only, default instance |
| Rendering | GPU compositing, CSS stacking in the compositor, hardware video decode | CPU raster only |
| Rendering | Smooth sub-pixel scrolling, momentum scrolling, scroll snapping | none |
| Rendering | `position: sticky` | treated as `relative` |
| Rendering | Rounded clipping from `border-radius` on scrolling/overflow boxes | rectangular clip |
| Rendering | Print, PDF, screenshots beyond the debug PNG writer | none |
| Network | HTTP/2, HTTP/3, HTTP caching (disk), `Content-Security-Policy`, mixed-content blocking | HTTP/1.1; in-memory cache; no CSP enforcement |
| Network | Cookies partitioned by top-level site, `SameSite` enforcement | global first-party cookie store |
| Network | Certificate pinning beyond rustls defaults, permission prompts, TLS info UI | none |
| Security | Process sandbox, site isolation, AppContainer, separate renderer process | none — full user token (§12) |
| Platform | macOS/Linux, DPI other than 100/125/150/200% tested, screen readers / accessibility tree, nine-language UI | Windows-only; no a11y tree in v1 |
| Browser shell | Tabs, history, bookmarks, downloads, extensions, devtools, password manager, find-in-page | none |
| Browser shell | Multiple windows, session persistence across runs | single window, in-memory only |
```