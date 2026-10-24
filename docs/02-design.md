> **SUPERSEDED.** Early architecture, replaced by the fidelity contract.
> See `00-FIDELITY-CONTRACT.md` and `09-ui-reference.md`.

# Design Document — City of Towers

**Version:** 0.1 (draft for review)
**Status:** Awaiting review
**Date:** 2026-10-22
**Implements:** `docs/01-functional-spec.md` v0.2

---

## 1. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Browser (TypeScript, no framework)                         │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌───────────────┐   │
│  │ Lobby    │ │ City     │ │ Tower    │ │ Obelisk       │   │
│  │ view     │ │ view     │ │ view     │ │ view          │   │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └───────┬───────┘   │
│       └────────────┴────────────┴───────────────┘           │
│                        │                                    │
│              ┌─────────┴─────────┐                          │
│              │  api.ts (REST)    │  ws.ts (WebSocket)       │
│              └─────────┬─────────┘                          │
└────────────────────────┼────────────────────────────────────┘
                         │ HTTPS / WSS
┌────────────────────────┼────────────────────────────────────┐
│  ASP.NET Core (.NET 10)│                                    │
│  ┌─────────────────────┴──────────────────────┐             │
│  │ Minimal API endpoints (REST)               │             │
│  │  /api/auth/*  /api/games/*  /api/cities/*  │             │
│  └─────────────────────┬──────────────────────┘             │
│  ┌─────────────────────┴──────────────────────┐             │
│  │ GameService (domain logic, authoritative)  │             │
│  │  - turn ticker (BackgroundService)         │             │
│  │  - building rules, resource accrual        │             │
│  │  - puzzle state                            │             │
│  └─────────────────────┬──────────────────────┘             │
│  ┌─────────────────────┴──────────────────────┐             │
│  │ EF Core + SQLite (single file)             │             │
│  └────────────────────────────────────────────┘             │
│  ┌────────────────────────────────────────────┐             │
│  │ WebSocket hub (/ws) — broadcasts events    │             │
│  └────────────────────────────────────────────┘             │
└─────────────────────────────────────────────────────────────┘
```

**Key decision:** the server is authoritative. The client never computes resource
totals or validates builds; it renders what the server sends and sends commands.

---

## 2. Data model

### 2.1 Entities

```
User
  Id            GUID
  Username      string (unique, 3-20 chars)
  PasswordHash  string (PBKDF2, 100k iterations, 16-byte salt)
  CreatedAt     DateTime

Game
  Id            GUID
  Name          string
  HostUserId    GUID
  Capacity      int (2-10)
  TurnSeconds   int (30 | 60 | 300 | 86400)
  TurnLimit     int (default 200, 0 = unlimited)
  State         enum { Waiting, InProgress, Finished }
  CurrentTurn   int
  TurnStartedAt DateTime
  WinnerUserId  GUID? (null until solved)
  PuzzleAnswer  string (stored hashed, case-insensitive compare)
  CreatedAt     DateTime

Player
  Id            GUID
  GameId        GUID
  UserId        GUID
  CityName      string
  Precious      bool
  ObeliskVisits int (0..10)
  BuiltThisTurn bool
  JoinedAt      DateTime

City
  Id            GUID
  PlayerId      GUID (1:1)
  Wood          int
  Ore           int
  Mercury       int
  Sulfur        int
  Crystal       int
  Gems          int
  Gold          int

Building
  Id            GUID
  CityId        GUID
  BuildingKey   string (e.g. "woodcutter")
  BuiltOnTurn   int

Post
  Id            GUID
  CityId        GUID
  Title         string (max 120)
  Body          string (max 20000)
  ImageUrl      string? (max 500)
  CreatedAt     DateTime
  UpdatedAt     DateTime

Comment
  Id            GUID
  PostId        GUID
  AuthorUserId  GUID
  Body          string (max 2000)
  CreatedAt     DateTime

WallMessage
  Id            GUID
  CityId        GUID
  AuthorUserId  GUID
  ParentId      GUID? (null = top-level, else reply)
  Body          string (max 2000)
  CreatedAt     DateTime

TowerMessage          (tower-level guestbook, G-2)
  Id            GUID
  CityId        GUID
  AuthorUserId  GUID
  Body          string (max 2000)
  CreatedAt     DateTime

ObeliskMessage
  Id            GUID
  GameId        GUID
  AuthorUserId  GUID
  Body          string (max 2000)
  CreatedAt     DateTime

Session
  Id            GUID
  UserId        GUID
  ExpiresAt     DateTime
```

### 2.2 Indexes
- `User.Username` unique
- `Player (GameId, UserId)` unique — one city per player per game
- `Building (CityId, BuildingKey)` unique — no duplicate buildings
- `WallMessage (CityId, CreatedAt)` — wall reads
- `Comment (PostId, CreatedAt)` — comment reads

---

## 3. Building catalogue (resolves G-1)

Costs are in resources. `+N/turn` means per turn income.

### 3.1 Income buildings

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `townHall` | Town Hall | — (starting) | — | +500 gold/turn |
| `woodcutter` | Woodcutter's Lodge | 500g, 5w | — | +2 wood/turn |
| `orePit` | Ore Pit | 500g, 5w | — | +2 ore/turn |
| `alchemist` | Alchemist's Lab | 1000g, 5w, 5o | — | +1 mercury/turn |
| `sulfurDune` | Sulfur Dune | 1000g, 5w, 5o | — | +1 sulfur/turn |
| `crystalCavern` | Crystal Cavern | 1000g, 5w, 5o | — | +1 crystal/turn |
| `gemPond` | Gem Pond | 1000g, 5w, 5o | — | +1 gems/turn |

### 3.2 Civic buildings

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `marketplace` | Marketplace | 500g, 5w | townHall | Unlocks resource display totals |
| `tavern` | Tavern | 500g, 5w | townHall | Cosmetic; shows visitor count |
| `granary` | Granary | 1000g, 5w, 5o | woodcutter, orePit | +50% wood and ore income |
| `treasury` | Treasury | 2000g, 10o | townHall | +250 gold/turn |
| `cityHall` | City Hall | 2000g, 10w, 10o | townHall | +500 gold/turn (replaces Town Hall bonus) |

### 3.3 Magic buildings

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `magicTower1` | Magic Tower I | 1000g, 5w, 5o | townHall | 3 post slots |
| `magicTower2` | Magic Tower II | 1500g, 5w, 5o, 4crystal | magicTower1 | 6 post slots |
| `magicTower3` | Magic Tower III | 2000g, 5w, 5o, 6gems | magicTower2 | 10 post slots |
| `magicTower4` | Magic Tower IV | 2500g, 5w, 5o, 6sulfur | magicTower3 | 15 post slots |
| `magicTower5` | Magic Tower V | 3000g, 5w, 5o, 6mercury | magicTower4 | 25 post slots |

### 3.4 Monument

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `obeliskMonument` | Monument of the Obelisk | 2000g, 10o | townHall | Grants access to the standalone Obelisk |

> **Resolved (D-1):** the city building is named **Monument of the Obelisk** and
> grants *access*. The **standalone Obelisk** is a world object (spec §8.1) where
> the puzzle lives. Two distinct things, two distinct names.

### 3.5 Ultimate

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `grandArchive` | The Grand Archive | 10000g, 20w, 20o, 10 of each rare | obelisk + Precious | Ends the game |

---

## 4. Turn engine

- A `BackgroundService` ticks every second.
- For each `InProgress` game, if `now - TurnStartedAt >= TurnSeconds`:
  1. `CurrentTurn++`
  2. For each player: accrue income from all buildings
  3. Reset `BuiltThisTurn = false`
  4. Broadcast `turnAdvanced` to all connected clients
  5. If `TurnLimit > 0 && CurrentTurn > TurnLimit`, set `State = Finished`
- Turn timing is stored in the database, so a restart resumes correctly.

---

## 5. API

### 5.1 Auth
| Method | Path | Body | Response |
| --- | --- | --- | --- |
| POST | `/api/auth/register` | `{username, password}` | `{userId}` + session cookie |
| POST | `/api/auth/login` | `{username, password}` | `{userId}` + session cookie |
| POST | `/api/auth/logout` | — | 204 |
| GET | `/api/auth/me` | — | `{userId, username}` or 401 |

### 5.2 Games
| Method | Path | Body | Response |
| --- | --- | --- | --- |
| GET | `/api/games` | — | list of `{id, name, host, capacity, joined, state}` |
| POST | `/api/games` | `{name, capacity, turnSeconds, turnLimit}` | `{gameId}` |
| POST | `/api/games/{id}/join` | — | 204 |
| POST | `/api/games/{id}/leave` | — | 204 |
| POST | `/api/games/{id}/start` | — | 204 (host only) |
| POST | `/api/games/{id}/cancel` | — | 204 (host only) |
| GET | `/api/games/{id}` | — | full game state for the caller |

### 5.3 City
| Method | Path | Body | Response |
| --- | --- | --- | --- |
| GET | `/api/cities/{playerId}` | — | city view (read-only if not owner) |
| POST | `/api/cities/{playerId}/build` | `{buildingKey}` | 204 or 400 with reason |
| PUT | `/api/cities/{playerId}/name` | `{name}` | 204 (owner only) |

### 5.4 Content
| Method | Path | Body | Response |
| --- | --- | --- | --- |
| GET | `/api/cities/{playerId}/posts` | — | list of posts |
| POST | `/api/cities/{playerId}/posts` | `{title, body, imageUrl}` | `{postId}` (owner only) |
| PUT | `/api/posts/{postId}` | `{title, body, imageUrl}` | 204 (owner only) |
| DELETE | `/api/posts/{postId}` | — | 204 (owner only) |
| GET | `/api/posts/{postId}/comments` | — | list |
| POST | `/api/posts/{postId}/comments` | `{body}` | `{commentId}` |
| GET | `/api/cities/{playerId}/wall` | — | threaded messages |
| POST | `/api/cities/{playerId}/wall` | `{body, parentId?}` | `{messageId}` |
| DELETE | `/api/wall/{messageId}` | — | 204 (city owner only) |
| GET | `/api/cities/{playerId}/tower` | — | tower guestbook |
| POST | `/api/cities/{playerId}/tower` | `{body}` | `{messageId}` |

### 5.5 Obelisk
| Method | Path | Body | Response |
| --- | --- | --- | --- |
| GET | `/api/games/{id}/obelisk` | — | `{visits, fragments: string[], messages: []}` |
| POST | `/api/games/{id}/obelisk/visit` | — | `{fragment, visits}` |
| POST | `/api/games/{id}/obelisk/message` | `{body}` | `{messageId}` |
| POST | `/api/games/{id}/obelisk/solve` | `{answer}` | `{correct: bool}` |

### 5.6 WebSocket
- Endpoint: `/ws?gameId={id}`
- Server → client events:
  - `turnAdvanced {turn}`
  - `wallMessage {cityId, message}`
  - `comment {postId, comment}`
  - `towerMessage {cityId, message}`
  - `obeliskMessage {message}`
  - `gameFinished {winnerUserId}`
  - `playerJoined {player}`
- Client → server: none (commands go over REST).

---

## 6. Client structure

```
client/
  index.html
  styles.css
  src/
    main.ts          bootstrap, routing
    api.ts           typed REST client
    ws.ts            WebSocket client with reconnect
    state.ts         in-memory app state
    views/
      lobby.ts       game list, create, join
      city.ts        city view (own or visited)
      tower.ts       blog / spellbook
      obelisk.ts     puzzle fragments + messages
      wall.ts        threaded messages
    ui/
      panel.ts       shared panel/button rendering
      toast.ts       notifications
```

### 6.1 Rendering
- Canvas for the city scene (buildings drawn as sprites).
- DOM for text-heavy views (tower, wall, obelisk) — better for text selection
  and accessibility.

### 6.2 Read-only enforcement
- The client hides build controls when `city.ownerId !== me.userId`.
- The server **also** rejects build requests from non-owners (defence in depth).

---

## 7. Security

| Concern | Mitigation |
| --- | --- |
| Password storage | PBKDF2-SHA256, 100k iterations, per-user salt |
| Session fixation | New session ID on login |
| CSRF | SameSite=Strict cookies + `Origin` check on mutating requests |
| XSS | All user text rendered via `textContent`, never `innerHTML` |
| SQL injection | EF Core parameterised queries only |
| Authorisation | Every mutating endpoint checks ownership server-side |
| Rate limiting | 10 messages/minute per user per game |

---

## 8. Testing strategy

| Level | What | Tool |
| --- | --- | --- |
| Unit | Building rules, resource accrual, puzzle compare | xUnit |
| Integration | API endpoints against in-memory SQLite | `WebApplicationFactory` |
| End-to-end | Two browsers, real WebSocket, message propagation | Playwright |
| Acceptance | Each AC-1..AC-19 mapped to a test | xUnit + Playwright |

---

## 9. Design review resolutions

| # | Item | Resolution |
| --- | --- | --- |
| D-1 | Obelisk naming collision | City building renamed **Monument of the Obelisk**; standalone Obelisk is a world object. |
| D-2 | Turn limit too long | Default `TurnLimit = 60` (5h at 5min/turn). Host may set 0 (unlimited) or up to 500. |
| D-3 | Puzzle answer provenance | Server picks a riddle from a built-in list of 20 at game creation, stores the answer hashed, and the 10 fragments are that riddle's clues. No player knows the answer in advance. |
| D-4 | External image URLs | Accepted for v1; the post editor shows a hint. |
| D-5 | Rejoin after cookie loss | `GET /api/games` returns a `myGames` array so a logged-in user can re-enter any game they are in. |
| D-6 | Solve brute-force | `/obelisk/solve` limited to 5 attempts/minute; 10 wrong answers locks the player out for 1 turn. |
| D-7 | Fragment content | Each fragment is one line of the riddle, stored as a JSON array on the `Game` row. |

### 9.1 Riddle storage

```
Game
  RiddleId      int      (index into the built-in riddle list)
  PuzzleAnswer  string   (SHA-256 of the normalised answer)
  Fragments     string   (JSON array of 10 clue lines)
```

Normalisation for comparison: lowercase, trim, collapse internal whitespace,
strip punctuation.