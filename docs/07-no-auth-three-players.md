# Design Change — Remove Authentication, Pre-seed Three Players

**Version:** 3.0
**Supersedes:** §12 (accounts) of `01-functional-spec.md`, §5.1 (auth API) of `02-design.md`
**Date:** 2026-10-22

---

## 1. Decision

**Authentication is removed.** There is no registration, no login, no password,
no session, no JWT.

The game starts with **three pre-seeded players**:

| Player | Faction | Role |
| --- | --- | --- |
| **Alice** | Castle | human |
| **Bob** | Rampart | human |
| **Carol** | Necropolis | human |

A player is selected by picking a name on the landing screen. That is the entire
"login".

---

## 2. Why

The original design spent effort on password hashing, session cookies, CSRF,
JWT validation, and account recovery. None of that contributes to the game's
look, feel, or play. For a three-player local game it is pure overhead.

Removing it:
- deletes ~400 lines of server code
- deletes the login screen, the lobby's auth state, and the logout flow
- removes the CSRF middleware and the session table
- makes the first screen the **game itself**, not a form

---

## 3. What replaces it

### 3.1 Landing screen (S1, revised)

```
┌──────────────────────────────────────────────────────────────────────┐
│                                                                      │
│                    C I T Y   O F   T O W E R S                       │
│                    ─────────────────────────────                     │
│                                                                      │
│                        Choose your city                              │
│                                                                      │
│   ┌──────────────┐   ┌──────────────┐   ┌──────────────┐             │
│   │   [castle]   │   │  [rampart]   │   │ [necropolis] │             │
│   │    Alice     │   │     Bob      │   │    Carol     │             │
│   │    Castle    │   │   Rampart    │   │  Necropolis  │             │
│   │  [ Play ]    │   │  [ Play ]    │   │  [ Play ]    │             │
│   └──────────────┘   └──────────────┘   └──────────────┘             │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

- Three cards, 240 × 280, at x = 220 / 520 / 820, y = 300.
- Each card shows the faction's town sprite, the player name, and the faction name.
- Clicking **Play** enters the game as that player.
- The choice is stored in `localStorage` under `cityoftowers.player`.
- Opening the game again resumes as the same player.

### 3.2 Identity model

```
Player
  Id            Guid (fixed, seeded)
  Name          "Alice" | "Bob" | "Carol"
  Faction       castle | rampart | necropolis
```

There is no `User` table and no `Session` table.

### 3.3 API changes

| Removed | Replaced by |
| --- | --- |
| `POST /api/auth/register` | — |
| `POST /api/auth/login` | — |
| `POST /api/auth/logout` | — |
| `GET /api/auth/me` | `GET /api/players` (returns the three seeded players) |
| Cookie session | `X-Player-Id` header on every request |

Every mutating endpoint reads `X-Player-Id` and resolves it to a seeded player.
If the header is missing or unknown, the request is rejected with 400.

### 3.4 Security posture

This is a **local, single-machine game**. There is no authentication, so any
client can act as any player. That is acceptable and intentional. The README
must state it plainly.

---

## 4. What stays

- The three players each own one city and one hero.
- The adventure map, hero movement, mines, resources, fog of war.
- The Magic Tower (blog), the wall, the Obelisk, the Grail.
- The turn engine (day / week / month).
- The SignalR hub for live updates.

---

## 5. What is deleted

| File / area | Action |
| --- | --- |
| `Services/AuthService.cs` | delete |
| `Domain/Entities.cs` → `User`, `Session` | delete |
| `Data/AppDb.cs` → `Users`, `Sessions` | delete |
| `Api/Endpoints.cs` → `/api/auth/*` | delete |
| `Program.cs` → CSRF middleware | delete |
| `client/src/views/Login.tsx` | replace with `ChoosePlayer.tsx` |
| `client/src/api.ts` → auth calls | delete |
| `client/src/App.tsx` → session bootstrap | replace with player selection |

---

## 6. Acceptance criteria (revised)

| # | Criterion |
| --- | --- |
| AC-1 | The landing screen shows exactly three players: Alice, Bob, Carol. |
| AC-2 | Clicking Play enters the game as that player. |
| AC-3 | The chosen player persists across a page reload. |
| AC-4 | Requests without a valid `X-Player-Id` are rejected. |
| AC-5 | Alice, Bob and Carol each have a city and a hero on the map. |

AC-1..AC-24 from the original spec are otherwise unchanged, with "user" read as
"player".

---

## 7. Effort reallocation

The effort freed by removing auth goes to:

1. **Adventure map rendering** — terrain tiles, objects, fog of war, hero sprite.
2. **Town screen** — a painted view with clickable buildings, not a grid.
3. **Hero screen** — portrait, stats, army, skills.
4. **Art** — building sprites, creature sprites, UI chrome.

These are what make the game feel like HoMM3.