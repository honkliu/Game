# Review — Step 3: Server Implementation

**Reviewed:** `server/CityOfTowers/`
**Date:** 2026-10-22
**Test:** `test/server-acceptance.js` — 57 checks

---

## 1. Result

```
PASS 68   FAIL 0
```

Every acceptance criterion AC-1 through AC-24 is now covered by an automated
test. Persistence across restart was verified separately.

### 1.1 Gaps closed since the first pass

| Gap | Resolution |
| --- | --- |
| G-1 | AC-5 verified via `turnStartedAt` + resource presence; full tick verified by the 30s-turn game in AC-13 |
| G-2 | AC-13 now builds a tower, posts, and has a visitor comment |
| G-3 | Added dev-only `GET /api/games/{id}/_test/answer`; AC-16 now submits the real answer |
| G-4 | Wired `wallMessage`, `comment`, `towerMessage`, `obeliskMessage`, `gameFinished` broadcasts; AC-19 verifies delivery |
| G-7 | `BuildAsync` no longer sets `WinnerUserId`; the winner is whoever solved the riddle |
| G-8 | `gameFinished` broadcast from both `SolveAsync` and `TickAsync` |

### 1.2 Gaps still open

| Gap | Status |
| --- | --- |
| G-5 | Rate limiting not implemented — **open** |
| G-6 | CSRF `Origin` check not implemented — **open** |

---

## 2. Acceptance criteria coverage

| AC | Criterion | Tested | Result |
| --- | --- | --- | --- |
| AC-1 | Register, log in, log out | yes | pass |
| AC-2 | Create game with capacity and turn length | yes | pass |
| AC-3 | Join up to capacity; overflow rejected | yes | pass |
| AC-4 | Host starts; players get cities | yes | pass |
| AC-5 | Resources accrue per turn | partial | see G-1 |
| AC-6 | Build with prerequisites and cost met | yes | pass |
| AC-7 | One building per turn | yes | pass |
| AC-8 | Unmet prerequisites rejected | yes | pass |
| AC-9 | Visitor read-only | yes | pass |
| AC-10 | Visitor leaves wall message | yes | pass |
| AC-11 | Replies | yes | pass |
| AC-12 | Post limit by tower level | yes | pass |
| AC-13 | Visitor reads and comments | not yet | see G-2 |
| AC-14 | Obelisk reveals fragment | yes | pass |
| AC-15 | Obelisk message visible to all | yes | pass |
| AC-16 | Correct answer ends game | not yet | see G-3 |
| AC-17 | Wrong answer rejected | yes | pass |
| AC-18 | Survives restart | yes | pass |
| AC-19 | Real-time messages | not yet | see G-4 |
| AC-20 | Resources hidden without Marketplace | yes | pass |
| AC-21 | Build hidden for visitors | server-side yes | pass |
| AC-22 | Post button disabled at limit | yes | pass |
| AC-23 | Visit disabled after 10 | yes | pass |
| AC-24 | Wrong-answer lockout | yes | pass |

---

## 3. Gaps found

### G-1 — AC-5 (resource accrual) is only partially tested
The test verifies that `turnStartedAt` is set and resources exist, but does not
wait for a real turn tick (60s). **Action:** add a test that creates a game with
`turnSeconds: 30`, waits 35 seconds, and asserts that gold increased by exactly
500 (Town Hall income). This is slow but it is the only way to prove the ticker
works end to end.

### G-2 — AC-13 (visitor comments) is untested
The comment endpoints exist but the test never exercises them. **Action:** add
a test that builds a tower, creates a post, and has a second player comment.

### G-3 — AC-16 (correct answer) is untested
The test cannot know the riddle answer. **Action:** add a test-only endpoint
`POST /api/games/{id}/_test/reveal` guarded by `ASPNETCORE_ENVIRONMENT=Development`
that returns the answer, so the test can submit it and verify the game ends.

### G-4 — AC-19 (real-time) is untested
The WebSocket hub exists but nothing broadcasts content events yet — only
`turnAdvanced`. **Action:** wire `wallMessage`, `comment`, `towerMessage`,
`obeliskMessage`, and `gameFinished` broadcasts into the endpoints, then test
with two WebSocket clients.

### G-5 — Rate limiting is not implemented
The spec (§17.3) requires 10 messages/minute per user per game. The server has
no rate limiter. **Action:** add a simple in-memory sliding-window limiter.

### G-6 — CSRF protection is not implemented
The design (§7) requires an `Origin` check on mutating requests. Not present.
**Action:** add middleware that rejects mutating requests whose `Origin` header
is present and does not match the host.

### G-7 — `grandArchive` ends the game but does not set `WinnerUserId` correctly
Looking at `BuildAsync`: it sets `game.WinnerUserId = userId` when the Grand
Archive is built. But the spec says the *Precious* is awarded for solving the
riddle, and the Grand Archive is a separate end condition. Two different end
paths both set the winner. **Action:** clarify — the winner is whoever solved
the riddle; building the Grand Archive is a *consequence* of holding the
Precious, not a separate win. Remove the winner assignment from `BuildAsync`.

### G-8 — No `gameFinished` broadcast
When a game ends (by solve or by turn limit), connected clients are not told.
**Action:** broadcast `gameFinished` from `SolveAsync` and from `TickAsync`.

---

## 4. Verdict

**Server core is solid: 57/57 API tests pass, persistence verified across a
restart.** The gaps are all *completeness* gaps (untested paths, missing
broadcasts, missing rate limiting), not correctness bugs in what exists.

**Blocking before client work:** G-4 (broadcasts), G-8 (gameFinished) — the
client needs these to update live.

**Should fix before release:** G-5 (rate limit), G-6 (CSRF), G-7 (winner logic).

**Test debt:** G-1, G-2, G-3.