# Integration Design — City of Towers → KanKan

**Version:** 1.0
**Status:** Design only. **No integration is performed now.**
**Date:** 2026-10-22

---

## 1. Purpose

City of Towers is being built as a standalone game. It will later be integrated
into **KanKan** (`Q:\gitroot\Jiuzhang\KanKan`) as a game reachable from the top
bar, alongside the existing Tetris and Sudoku in `GamesPage`.

This document records the decisions that must be made **now** so that the later
integration is a wiring job, not a rewrite.

---

## 2. What KanKan actually is (verified, not assumed)

| Aspect | KanKan | City of Towers (current) | Conflict? |
| --- | --- | --- | --- |
| Server framework | ASP.NET Core **net10.0** | ASP.NET Core **net10.0** | none |
| Client framework | **React 18.2** | React 19.3 | **yes** |
| Build tool | **Vite 5** | Vite 8 | **yes** |
| Routing | **react-router-dom 6** | hash routing, hand-rolled | **yes** |
| HTTP | **axios** | `fetch` | minor |
| Real-time | **SignalR** (`@microsoft/signalr` 8) | raw WebSocket | **yes** |
| Auth | **JWT** (Bearer) + refresh token, `withCredentials` | cookie session | **yes** |
| Database | **MongoDB** (driver 3.6) | SQLite + EF Core | **yes** |
| Cache | **Redis** | none | no |
| UI kit | **MUI 5.15** + Emotion | plain CSS | **yes** |
| State | **Redux Toolkit** | React state | minor |
| i18n | `LanguageContext` (en/zh) | none | **yes** |
| Games entry | `GamesPage` at `/games`, tabs | standalone app | **yes** |

**Conclusion:** the two codebases share only the server framework. Everything
else differs. Integration must be planned, not assumed.

---

## 3. Decisions to make now

### D-1 — Align the client stack with KanKan

**Decision: adopt KanKan's versions now.**

| Change | From | To | Reason |
| --- | --- | --- | --- |
| React | 19.3 | **18.2** | KanKan pins 18.2; mixing versions in one bundle is not possible |
| Vite | 8 | **5** | KanKan uses 5; plugin compatibility |
| Router | hash, hand-rolled | **react-router-dom 6** | KanKan's router; nested routes under `/games` |
| HTTP | `fetch` | **axios** | KanKan's interceptor pattern (auth header, 401 refresh) |
| Real-time | raw WebSocket | **SignalR** | KanKan already runs a SignalR hub; one transport, one auth path |
| Auth | cookie session | **JWT Bearer** | KanKan issues JWTs; the game must accept them |
| UI kit | plain CSS | **MUI 5.15** | KanKan's theme; the game must not look foreign |

**Cost now:** moderate — the client is ~6 files so far.
**Cost later if deferred:** high — every screen would need rewriting.

### D-2 — Server: separate service, shared auth

**Decision: City of Towers stays a separate ASP.NET Core service, but validates
KanKan's JWTs.**

Rationale:
- KanKan's server is large (MongoDB, Redis, Semantic Kernel, media pipelines).
  Adding a game domain to it couples unrelated concerns.
- A separate service can be deployed, scaled, and tested independently.
- Shared auth is achieved by validating the same JWT signing key.

```
Browser ──► KanKan client (React 18, MUI)
              │
              ├─ /api/*        ──► KanKan.API        (existing)
              ├─ /hubs/chat    ──► KanKan.API SignalR (existing)
              └─ /api/game/*   ──► CityOfTowers.API   (new, validates KanKan JWT)
```

Reverse proxy (nginx / YARP) routes `/api/game` to the game service. The browser
sees one origin, so CORS and CSRF stay simple.

### D-3 — Identity: reuse KanKan users

**Decision: no separate accounts.** A player is a KanKan user.

- The game service validates the JWT and reads `sub` (user id) and `name`.
- `Player.UserId` becomes a **string** (KanKan's Mongo ObjectId), not a GUID.
- The game never stores passwords.

**Migration impact:** `User` and `Session` tables are dropped from the game
service. `Player.UserId` changes type. This is a schema change — cheaper now
than after data exists.

### D-4 — Persistence: MongoDB, not SQLite

**Decision: switch the game service to MongoDB.**

Rationale: KanKan already runs MongoDB. Running a second database engine for one
feature is operational debt. The game's documents map naturally:

```
games        { _id, name, hostUserId, capacity, turnSeconds, turnLimit,
               state, currentTurn, turnStartedAt, winnerUserId,
               riddleId, puzzleAnswerHash, fragments[] }
players      { _id, gameId, userId, cityName, precious, obeliskVisits,
               builtThisTurn, wrongAnswers, joinedAt }
cities       { _id, playerId, resources{...} }
buildings    { _id, cityId, buildingKey, builtOnTurn }
posts        { _id, cityId, title, body, imageUrl, createdAt, updatedAt }
comments     { _id, postId, authorUserId, body, createdAt }
wallMessages { _id, cityId, authorUserId, parentId, body, createdAt }
towerMessages{ _id, cityId, authorUserId, body, createdAt }
obeliskMsgs  { _id, gameId, authorUserId, body, createdAt }
```

**Cost now:** rewrite the data layer (~1 file).
**Cost later:** data migration + dual-write period.

### D-5 — Real-time: SignalR hub, not raw WebSocket

**Decision: replace `WsHub` with a SignalR hub `GameHub`.**

- Groups are per game: `Groups.AddToGroupAsync(connId, "game:" + gameId)`.
- Server → client methods mirror the current event names exactly
  (`turnAdvanced`, `wallMessage`, `comment`, `towerMessage`, `obeliskMessage`,
  `gameFinished`), so the client's event handling is unchanged.
- Auth: `[Authorize]` on the hub; the JWT is passed via `access_token` query
  parameter (SignalR's standard mechanism).

**Cost now:** ~1 file.
**Cost later:** rewriting every broadcast call site.

### D-6 — UI: MUI theme, but keep the game's own look

**Decision: the game keeps its own visual identity, implemented with MUI
primitives.**

KanKan is a chat app with a light/dark MUI theme. The game is a stone-and-gold
medieval UI. Forcing the game into KanKan's theme would make it look wrong;
ignoring MUI would make it look foreign.

Resolution: wrap the game in a **nested MUI ThemeProvider** with the game's
palette, and use MUI components (`Button`, `TextField`, `Dialog`, `Tabs`) styled
to the game's tokens. The game looks like itself, but behaves like a KanKan
component (focus rings, keyboard handling, i18n).

### D-7 — Entry point: a tab in GamesPage

**Decision: the game appears as a third tab in `GamesPage`.**

```
/games
  ├─ Tetris
  ├─ Sudoku
  └─ City of Towers   ← new
```

The tab renders `<CityOfTowersApp />`, which owns its own sub-routes
(`/games/city-of-towers/lobby`, `/games/city-of-towers/game/:id`, ...).

### D-8 — i18n

**Decision: all user-facing strings go through a `t()` function from day one.**

KanKan has `LanguageContext` with English and Chinese. The game must not hardcode
English strings. Even before integration, the game uses a local `t()` that
returns the English string; integration swaps the implementation.

---

## 4. What changes in the current codebase

| File | Change | When |
| --- | --- | --- |
| `client/package.json` | React 18, Vite 5, add axios / react-router-dom / MUI / SignalR | **now** |
| `client/src/App.tsx` | react-router-dom routes instead of hash routing | **now** |
| `client/src/api.ts` | axios with auth interceptor instead of `fetch` | **now** |
| `client/src/ws.ts` | SignalR client instead of raw WebSocket | **now** |
| `client/src/ui/*` | MUI components with game theme | **now** |
| `client/src/i18n.ts` | `t()` indirection | **now** |
| `server/.../Data/AppDb.cs` | EF Core → MongoDB | **now** |
| `server/.../Services/WsHub.cs` | → `GameHub` (SignalR) | **now** |
| `server/.../Services/AuthService.cs` | password auth → JWT validation | **now** |
| `server/.../Domain/Entities.cs` | `Guid` → `string` ids; drop `User`/`Session` | **now** |
| `server/.../Program.cs` | JWT bearer, SignalR, Mongo | **now** |

---

## 5. What does NOT change

- The functional spec (`docs/01-functional-spec.md`) — features are unchanged.
- The UI spec (`docs/03-ui-spec.md`) — pixel geometry is unchanged; only the
  implementation technology changes.
- The building catalogue, riddle list, and game rules.
- The acceptance criteria AC-1..AC-24.

---

## 6. Risks

| Risk | Mitigation |
| --- | --- |
| KanKan's JWT signing key is not shared with the game service | Confirm with KanKan's config; if not shareable, the game service calls KanKan's `/api/auth/me` to validate |
| React 18 downgrade breaks something I wrote for 19 | The client is small; `createRoot` and hooks are identical |
| MUI theming fights the pixel-exact UI spec | Use MUI for behaviour, plain CSS for geometry; the spec's coordinates win |
| MongoDB adds a dependency for local dev | Provide a `docker-compose.yml` with Mongo; also allow an in-memory fallback for tests |
| SignalR group naming collides with chat groups | Prefix all game groups with `game:` |

---

## 7. Open questions for KanKan's owner

1. Is the JWT signing key available to a second service, or must the game
   validate tokens by calling KanKan?
2. Should game data live in KanKan's Mongo database (separate collections) or a
   separate database on the same server?
3. Does the top bar need a game icon, or is the `/games` tab sufficient?
4. Should the game respect KanKan's light/dark theme, or always use its own
   dark stone theme?

---

## 8. Decision summary

| # | Decision |
| --- | --- |
| D-1 | Adopt KanKan's client stack now: React 18, Vite 5, react-router-dom 6, axios, SignalR, MUI 5.15 |
| D-2 | Separate game service, reverse-proxied under `/api/game` |
| D-3 | Reuse KanKan users via JWT; no separate accounts |
| D-4 | MongoDB, not SQLite |
| D-5 | SignalR hub, not raw WebSocket |
| D-6 | Game keeps its own visual identity via a nested MUI theme |
| D-7 | Entry as a third tab in `GamesPage` |
| D-8 | All strings through `t()` from day one |