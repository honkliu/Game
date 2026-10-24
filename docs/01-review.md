# Review — Step 1: Functional Specification

**Reviewed:** `docs/01-functional-spec.md` v0.1
**Date:** 2026-10-22

---

## 1. Traceability: your description → spec

| Your words | Spec section | Covered? |
| --- | --- | --- |
| "online game" | §1, §13 | Yes |
| "start with up to 10 people" | §3.1 (capacity 2–10) | Yes |
| "10 people with a city each" | §4 | Yes |
| "every people could build their city" | §7 | Yes |
| "get resources like the hero3" | §5 (7 resources) | Yes |
| "there is no fighting!" | §14 (out of scope) | Yes |
| "Every city could be built just like hero3" | §7.3 | **Partially** — see G-1 |
| "adding one special structure called magic tower" | §9 | Yes |
| "each magic is actually a pic/text representing a blog" | §9.1–9.3 | Yes |
| "each person could visit other's city" | §11 | Yes |
| "can not build but only browse and leave messages" | §11 | Yes |
| "followup as twitter" | §10.1 (threaded) | Yes |
| "visit the 方尖塔 (obelisk)" | §8.1 | Yes |
| "solve the puzzle" | §8.3 | Yes |
| "extract the precious" | §8.4 | Yes |
| "built the best architecture" | §8.4 (Grand Archive) | Yes |
| "in the tower, a visitor could leave messages to others" | §9, §10.2 | **Ambiguous** — see G-2 |
| "the puzzle is global" | §8.3 | Yes |
| "standalone obelisk" | §8.1 | Yes |
| "once they visit, then could paste message on that" | §8.2 | Yes |
| "typescript + .net" | §13 (implied) | **Missing** — see G-3 |
| "people login to a website" | §12 | Yes |
| "start or join a game" | §3.1 | Yes |
| "once the game has enough people, the game could start" | §3.2 | Yes |

---

## 2. Gaps found

### G-1 — "built just like hero3" is under-specified
The spec lists building *categories* but not the actual building list. A player
cannot evaluate whether the build tree feels right without seeing it. **Action:**
the design document must contain the full building table (name, cost, prerequisite,
effect) for every building.

### G-2 — "in the tower, a visitor could leave messages" is ambiguous
Two readings:
- (a) Visitors comment on individual blog posts.
- (b) The tower itself has a message board, separate from posts.

The spec currently covers (a) via §10.2 but not (b). **Action:** support both —
comments on posts, plus a tower-level guestbook. This is cheap and matches the
wording.

### G-3 — Tech stack is not in the spec
You specified TypeScript + .NET. The spec's §13 lists non-functional targets but
never states the stack. **Action:** add a §17 "Implementation constraints" naming
ASP.NET Core + TypeScript, and the deployment shape.

### G-4 — No definition of "turn" for a social game
A 60-second turn is odd for a game whose main activity is reading and writing.
If a player is mid-sentence when the turn flips, nothing breaks, but the pacing
may feel wrong. **Action:** keep configurable turn length, but default to **5
minutes** rather than 60 seconds, and make clear that turns only gate *building*,
not reading or writing.

### G-5 — Puzzle fragment distribution is unspecified
§8.1 says each visit reveals "the next fragment", but not whether fragments are
per-player or shared. If per-player, a player who visits 10 times alone can solve
it without socialising — which defeats the social purpose. **Action:** fragments
are **per-player and sequential**, but the total is 10 and each player starts at
fragment 1, so solving requires 10 visits. Whether that is enough social pressure
is a design question — flagged for the design doc.

### G-6 — No win condition if nobody solves the puzzle
If the game runs forever, players lose interest. **Action:** add an optional
**turn limit** (default 200 turns); when reached, the game ends with no winner.

### G-7 — "Precious" is undefined as an object
Is it a resource, a building, a flag? **Action:** define it as a **trophy flag** on
the player, which unlocks one specific building.

### G-8 — No moderation story
A social game with free-text messages needs at least a delete path. §10.1 gives
owners delete rights on their own wall. There is no global moderation. **Action:**
out of scope for v1, but noted.

### G-9 — Real-time requirement is stated but not scoped
AC-19 requires real-time message delivery. That implies WebSocket or SSE. The spec
should say which. **Action:** design doc must specify the transport.

### G-10 — No account recovery
If a player forgets their password, there is no path back. **Action:** out of scope
for v1, noted explicitly.

---

## 3. Contradictions

None found. The spec is internally consistent.

---

## 4. Verdict

**Spec is 85% complete.** It covers every feature you named. The gaps are all
*detail* gaps (G-1, G-3, G-5, G-7) or *scope* gaps (G-6, G-8, G-10), not missing
features.

**Blocking before design:** G-1 (building table), G-3 (stack), G-5 (fragment
distribution), G-7 (Precious definition).

**Recommended before design:** G-2 (tower guestbook), G-4 (turn default), G-6
(turn limit).