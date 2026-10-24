# Documentation Index

**Read in this order.** Documents marked SUPERSEDED are kept for history only —
do not build from them.

| # | Document | Status | Purpose |
| --- | --- | --- | --- |
| 00 | **`00-FIDELITY-CONTRACT.md`** | **AUTHORITATIVE** | What is preserved, removed, and adapted from HoMM3. Every other document defers to this. |
| 01 | `08-requirements-matrix.md` | **AUTHORITATIVE** | Every requirement mapped to server rule, API, screen, and test. Shows what is done and what is missing. |
| 02 | **`09-ui-reference.md`** | **AUTHORITATIVE** | HoMM3's actual screen composition at 800 × 600. |
| 03 | `07-no-auth-three-players.md` | **VALID** | Three pre-seeded players; no authentication. |
| 04 | `06-adventure-map-design.md` | **PARTIAL** | §5 (painted towns) is authoritative. The rest is superseded. |
| 05 | `00-BUILD-SPEC.md` | **SUPERSEDED** | Kept only for the creature and building tables. Its UI section is wrong. |
| 06 | `01-functional-spec.md` | **SUPERSEDED** | Early feature list. |
| 07 | `02-design.md` | **SUPERSEDED** | Early architecture. |
| 08 | `03-ui-spec.md` | **SUPERSEDED** | Wrong layout (top resource bar). |
| 09 | `05-kankan-integration.md` | **DEFERRED** | Future integration. Not part of the current build. |
| 10 | `01-review.md`, `02-review.md`, `03-review.md`, `04-server-review.md` | **HISTORY** | Review records. |

---

## The one-sentence contract

> **HoMM3's world, presentation, exploration, heroes, creatures, and town
> development — with warfare replaced by meaningful social play.**
>
> Not a social website wearing a medieval theme.

---

## Current state

Per `08-requirements-matrix.md`:

| Category | Total | Done | Partial | Missing |
| --- | --- | --- | --- | --- |
| Preserve | 29 | 20 | 9 | 0 |
| Remove | 6 | 5 | 1 | 0 |
| Adapt | 14 | 12 | 1 | 1 |
| **Total** | **49** | **37** | **11** | **1** |

**76% complete against the contract.**

The largest gaps:

1. **UI composition is wrong** — top resource bar and bottom-right minimap
   instead of HoMM3's bottom bar and right sidebar.
2. **Town screen is a grid**, not a painted scene.
3. **Social adaptations are missing** — peaceful monster encounters, hero
   dialogue, player-to-player trade, shared projects, map annotations.
4. **The Grail ends the game** instead of being a milestone.
5. **No digging** for the Grail.

---

## Implementation order

1. Align client/server contracts — fix the UI composition, make the town painted.
2. Reproduce one complete adventure/town/hero experience end to end.
3. Repair exploration and economy — mines, resources, growth, Grail digging.
4. Integrate Towers and social encounters.
5. Expand content — remaining factions, shared projects, annotations.