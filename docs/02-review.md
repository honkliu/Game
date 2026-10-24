# Review — Step 2: Design Document

**Reviewed:** `docs/02-design.md` v0.1
**Date:** 2026-10-22

---

## 1. Coverage: spec requirement → design

| Spec | Requirement | Design section | Covered? |
| --- | --- | --- | --- |
| §3.1 | Create/join game, capacity 2–10 | §5.2 | Yes |
| §3.2 | Host starts, cancels; players leave | §5.2 | Yes |
| §3.3 | Turns advance on schedule | §4 | Yes |
| §3.4 | Game ends on puzzle solve | §5.5, §4 | Yes |
| §4 | City with resources, buildings, tower, wall | §2.1 | Yes |
| §4.1 | Owner-only build; visitors read-only | §6.2, §7 | Yes |
| §5 | Seven resources | §2.1, §3.1 | Yes |
| §5.1 | Accrue once per turn | §4 | Yes |
| §6 | Turn length configurable | §2.1 `TurnSeconds` | Yes |
| §7.1 | Building catalogue with cost/prereq/effect | §3 | Yes (resolves G-1) |
| §7.2 | One building per turn | §2.1 `BuiltThisTurn` | Yes |
| §7.3 | Building categories | §3.1–3.5 | Yes |
| §8.1 | Standalone obelisk, 10 fragments | §5.5, §2.1 `ObeliskVisits` | **Partial** — see D-1 |
| §8.2 | Paste message at obelisk | §5.5 | Yes |
| §8.3 | Global puzzle, submit answer | §5.5 | **Partial** — see D-3 |
| §8.4 | Precious unlocks Grand Archive | §2.1 `Precious`, §3.5 | Yes |
| §9.1 | Tower holds blog posts | §2.1 `Post` | Yes |
| §9.2 | Post slots by tower level | §3.3 | Yes |
| §9.3 | Spellbook presentation | §6.1 | Yes |
| §10.1 | City wall, threaded | §2.1 `WallMessage` | Yes |
| §10.2 | Post comments | §2.1 `Comment` | Yes |
| §10.3 | 2000 char limit, plain text | §2.1, §7 | Yes |
| §11 | Visiting read-only | §6.2 | Yes |
| §12 | Accounts, sessions | §2.1, §5.1, §7 | Yes |
| §13 | Non-functional targets | §1, §4 | Yes |
| §17 | ASP.NET Core + TypeScript | §1 | Yes |
| AC-19 | Real-time messages | §5.6 | Yes |

---

## 2. Gaps found

### D-1 — Obelisk is defined twice, inconsistently
The spec (§8.1) says the Obelisk is **standalone**, separate from any city. The
design (§3.4) also makes it a **city building**. These are two different things
sharing a name, which will confuse implementation and players.

**Resolution:** rename the city building to **"Monument of the Obelisk"** — a
city-side structure that grants *access* to the standalone Obelisk. The standalone
Obelisk remains a world object reachable from the game view. Both are needed:
the building gives a reason to spend resources, the world object is where the
puzzle lives.

### D-2 — Turn limit of 200 × 5min is ~16 hours
That is far too long for a social game. A player who joins at 9am cannot wait
until 1am to see a result.

**Resolution:** default `TurnLimit = 60` (5 hours at 5min/turn). Host may set
0 (unlimited) or any value up to 500.

### D-3 — Puzzle answer provenance is undefined
The design says the answer is stored hashed, but never says **who sets it**.
Options:
- (a) The host types the answer when creating the game.
- (b) The server generates a riddle from a built-in list and knows the answer.
- (c) The answer is derived from the fragments themselves.

**Resolution:** (b). The server picks a riddle from a built-in list of 20 at game
creation, stores the answer hashed, and the 10 fragments are the riddle's clues.
This means no player knows the answer in advance, and the host cannot cheat.

### D-4 — Image URLs are external only
No upload means players must host images elsewhere. That is acceptable for v1 but
should be stated in the UI.

**Resolution:** accept it, show a hint in the post editor.

### D-5 — No "rejoin" path beyond the session cookie
If a player clears cookies, they cannot get back into a game they are in.

**Resolution:** `GET /api/games` already lists games; add a `myGames` field so a
logged-in user can see and re-enter any game they are a player in. This is a
one-line addition to the existing endpoint.

### D-6 — Rate limiting is per-user-per-game but not per-endpoint
A player could spam the obelisk solve endpoint to brute-force the answer.

**Resolution:** add a separate limit on `/obelisk/solve`: 5 attempts per minute,
and lock the player out of solving for 1 turn after 10 wrong answers.

### D-7 — No definition of what a "fragment" looks like
The design says `fragments: string[]` but not the content.

**Resolution:** each fragment is a short line of the riddle, e.g.
`"The first of ten: what has keys but opens no locks?"`. Fragments are stored as
a JSON array on the `Game` row.

---

## 3. Contradictions

| # | Contradiction | Resolution |
| --- | --- | --- |
| C-1 | Spec §8.1 "standalone obelisk" vs design §3.4 "obelisk building" | See D-1 |
| C-2 | Spec §6 "default 60 seconds" vs review G-4 "default 5 minutes" | Spec v0.2 already updated to 5 minutes; design matches. No conflict. |

---

## 4. Verdict

**Design is 90% complete.** Every spec requirement has a home. The gaps are
implementation details (D-3, D-6, D-7) and one naming collision (D-1).

**Blocking before implementation:** D-1 (naming), D-3 (puzzle provenance).

**Recommended:** D-2 (turn limit), D-5 (rejoin), D-6 (solve rate limit).

All resolutions above are incorporated into the design document in the next
revision.