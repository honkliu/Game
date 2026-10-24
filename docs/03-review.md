# Review — Step 2b: UI Specification

**Reviewed:** `docs/03-ui-spec.md` v1.0
**Test applied:** "Could two independent developers, given only this document,
produce pixel-identical screens?"

---

## 1. Ambiguities found

### A-1 — "auto" table height is undefined
§2.1 gives the open-games table `h = auto`. Two developers will size it
differently. **Fix:** specify `h = 44 * (rows + 1) + 2`, capped at 400px with
internal scrolling beyond that.

### A-2 — Relative time format is unspecified
§5.3 says `posted {relative time}`. Is that "2 days ago", "2d", "48h"?
**Fix:** specify exactly: `just now` (<60s), `{n} minutes ago`, `{n} hours ago`,
`{n} days ago`, `{n} months ago`.

### A-3 — Text wrapping width is given in characters, not pixels
§4.6 says "wrapped at 60 chars", §5.3 says "90 chars". Character counts depend on
the font. **Fix:** specify pixel widths — 480px for the build detail panel,
820px for the post body — and let the browser wrap.

### A-4 — Building sprite appearance is not specified
§4.3 says "building sprite 160 × 160" but never says what a building looks like.
Two developers will draw completely different cities. **Fix:** this is an art
dependency, not a spec gap — but the spec must state that sprites come from a
named asset set with a fixed file naming convention, and list the 18 building
keys that need art.

### A-5 — "Initials" for the avatar is undefined
§3.2 says "initials in --accent". For `alice` is that `A` or `AL`?
**Fix:** first character of the username, uppercased.

### A-6 — Scrollbar styling is unspecified
Several panels scroll. Default browser scrollbars will differ across platforms.
**Fix:** specify a 8px-wide track `#1a130c` with a `#4a3418` thumb, radius 4px.

### A-7 — Modal close on backdrop click conflicts with destructive modals
§0.8 says backdrop click closes the modal, "except where stated". The delete
confirm and cancel-game confirm are destructive. **Fix:** state explicitly that
destructive modals do **not** close on backdrop click.

### A-8 — Focus order within modals is unspecified
§0.6 says Tab follows reading order, but a modal traps focus. **Fix:** specify
that focus moves to the first input on open, Tab cycles within the modal, and
focus returns to the triggering element on close.

### A-9 — The wall panel "slides in" but no animation is specified
§9 says "slides in from the right". **Fix:** specify 200ms ease-out, from
x=1280 to x=880.

### A-10 — No specification for what happens when a player is disconnected
If a player closes the browser mid-game, their city remains. The spec never says
whether the UI shows them as offline. **Fix:** add an "offline" indicator on
player cards in S3 and on the city view header.

### A-11 — Resource chip overflow
Seven chips × 150px = 1050px, starting at x=20, ending at 1070. The bar is
1280 wide, so there is 210px of unused space. **Fix:** state that the remaining
space is empty (no stretching), and that the gold chip is the last one.

### A-12 — No loading state
Every screen fetches data. The spec never says what shows while loading.
**Fix:** specify a centred 24px spinner in `--accent` and the text `Loading…`
in `--text-dim`, shown after 200ms of waiting.

---

## 2. Contradictions

| # | Contradiction | Resolution |
| --- | --- | --- |
| C-1 | §4.1 puts the Obelisk button at y=720, but the city scene ends at y=664 and the action bar row is at y=672. Two rows of buttons at 672 and 720 overlap the 800px viewport bottom (720+40=760, fine) but the layout is cramped. | Accept; 760 < 800. |
| C-2 | §2.1 "MY GAMES" heading y is expressed relative to the table above it, which is `auto` height. | Resolved by A-1 (fixed height). |

---

## 3. Verdict

**The spec is now buildable but not yet pixel-identical.** 12 ambiguities remain,
all fixable with one-line additions. None are structural.

**Blocking:** A-1, A-2, A-3, A-5, A-7, A-8 (these change rendered output).
**Non-blocking:** A-4 (art dependency), A-6, A-9, A-10, A-11, A-12.

All fixes are applied in v1.1 below.