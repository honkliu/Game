# Engine build — live progress (session 2: the missing middle)

Working directory: `Q:\gitroot\Game\engine`. Architecture: `ARCHITECTURE.md`.

## Status of the five stub crates

| # | Crate | Status | Tests | Clippy |
|---|-------|--------|-------|--------|
| 1 | engine-select | DONE | 24 (6 unit + 18 integration) | clean |
| 2 | engine-style  | DONE | 36 (6 unit + 30 integration) | clean |
| 4 | engine-text   | DONE | 31 (16 unit + 15 integration) | clean |
| 3 | engine-layout | IN PROGRESS | — | — |
| 5 | engine-paint  | not started | | |
| 6 | engine binary | not started | | |

Build order deviation: text (step 4) was built before layout (step 3) because
`engine-layout` depends on `engine-text`'s measurement API. Writing layout
first would have meant inventing a measurement trait to delete.

## Real bugs found and fixed (not test edits)

1. **engine-select walk terminator.** `compounds` is stored subject-first, so a
   match finishes at `idx == len - 1`, not `idx == 0`. I had it inverted; every
   multi-compound selector failed. Caught by `descendant_combinator`.
2. **engine-select index was quadratic.** Resolving inherited `lang` by walking
   ancestors per node took 2.28 s for a 10k-deep document. Resolved top-down in
   one pass -> 31 ms.
3. **engine-style rank packing overflowed.** `Specificity::key()` is 60 bits;
   shifting it `<< 24` past the origin field silently dropped ids. `#a` lost to
   `.b`. Replaced the packed `u64` with a `Rank` struct (`Ord` by
   origin/spec/order) — no bit widths, no silent truncation.
4. **engine-style bucketing dropped selector-list rules.** The UA block rule has
   39 selectors; keying only on `selectors[0]` meant `div` never saw
   `display: block`. Now bucketed under every selector's rightmost compound.
5. **engine-style cross-sheet order.** Each sheet's `rule.order` starts at 0, so
   two author sheets tied. Added a global `order_base` offset.
6. **engine-text shaping warning was coverage-based.** Segoe UI *has* Hebrew
   glyphs, so the warning never fired. Now script-range based.
7. **engine-text `layout_runs` run-advance math.** Fallback runs had glyph x
   positions relative to the whole string; they must be relative to their own
   run start.

## Recorded limitations / deviations

- **`em` and `rem` are folded at CSS-parse time** against a fixed 16px context
  (`engine-css::values::parse_length`). So `html { font-size: 20px }` does not
  scale `h1 { font-size: 2em }` (stays 32px) and `0.5em` is 8px, not half the
  parent. Correct fix is a relative-length variant at parse time; recorded, not
  worked around.
- **`var()` is not implemented.** `engine-css` rejects `--x` declarations
  entirely, so a `color: var(--c)` is dropped at parse. Bigger gap than the
  architecture implies.
- **Inline `style` parses with a default 16px length context**, so `em` inside
  an inline style resolves against 16px.
- `engine-html::parse` is **quadratic in nesting depth** (1000->4.4 ms,
  2000->17.6 ms, 4000->71 ms). Terminates, but a 10k-deep hostile page takes ~5 s.
  Pre-existing, outside this task's crates.

## Working-method notes (re-confirmed)
- `write_file` on an existing path sometimes reports success but does not land.
  **Verify by re-reading; when it fails twice, `del` then create.**
- Do NOT run PowerShell text transforms on these sources (cmd.exe escaping
  mangles them). Use `edit`/`write_file`.