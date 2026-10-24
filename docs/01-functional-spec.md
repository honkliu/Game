> **SUPERSEDED.** Early feature list, replaced by the fidelity contract.
> See `00-FIDELITY-CONTRACT.md` and `09-ui-reference.md`.

# Functional Specification — City of Towers

**Version:** 0.3 (detailed UI specification)
**Status:** Awaiting review
**Date:** 2026-10-22

---

## 1. Purpose

A browser-based, turn-based social city-building game for up to 10 players. Each
player owns one city and develops it by gathering resources and constructing
buildings. There is **no combat**. The social layer is the core: players visit each
other's cities, read each other's writing, and leave messages.

---

## 2. Actors

| Actor | Description |
| --- | --- |
| **Guest** | Unauthenticated visitor. Sees the landing page and the list of open games. |
| **Player** | Authenticated user who has joined a game. Owns exactly one city. |
| **Host** | The player who created the game. Same powers as a Player, plus start/cancel. |
| **System** | The server. Advances turns, accrues resources, enforces rules. |

---

## 3. Screen inventory

The game has **seven screens**. Every screen is a full-page view at a fixed
**1280 × 800** logical resolution, scaled to fit the browser window.

| # | Screen | Route | Who can see it |
| --- | --- | --- | --- |
| S1 | Landing / Login | `/` | Guest |
| S2 | Lobby | `/lobby` | Player |
| S3 | Game Room (waiting) | `/game/{id}` | Player in that game |
| S4 | City View | `/game/{id}/city/{playerId}` | Any player in the game |
| S5 | Magic Tower | `/game/{id}/city/{playerId}/tower` | Any player in the game |
| S6 | Obelisk | `/game/{id}/obelisk` | Any player in the game |
| S7 | Game Over | `/game/{id}/over` | Any player in the game |

---

## 4. S1 — Landing / Login

### 4.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│                                                                      │
│                    C I T Y   O F   T O W E R S                       │
│                    ─────────────────────────────                     │
│                    a social city-building game                       │
│                                                                      │
│              ┌────────────────────────────────────┐                  │
│              │  Username  [____________________]  │                  │
│              │  Password  [____________________]  │                  │
│              │                                    │                  │
│              │   [   Log In   ]  [  Register  ]   │                  │
│              └────────────────────────────────────┘                  │
│                                                                      │
│              error message appears here (red)                        │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

### 4.2 Elements

| Element | Position (x, y, w, h) | Behaviour |
| --- | --- | --- |
| Title "CITY OF TOWERS" | centred, y=180, 48px serif, gold | static |
| Subtitle | centred, y=220, 16px, muted | static |
| Login panel | 440, 300, 400, 220 | stone panel with gold trim |
| Username field | 470, 340, 340, 32 | autofocus; max 20 chars |
| Password field | 470, 390, 340, 32 | masked; max 64 chars |
| Log In button | 470, 450, 160, 40 | submits login |
| Register button | 650, 450, 160, 40 | submits registration |
| Error text | 470, 500, 340, 20 | red, hidden when empty |

### 4.3 Behaviour

- **Enter** in either field submits Log In.
- On success: navigate to `/lobby`.
- On failure: show the server's message in the error line; do not clear fields.
- Registration validates: username 3–20 chars `[A-Za-z0-9_]`, password ≥ 8 chars.
  Validation errors appear in the same error line.
- If already logged in, `/` redirects to `/lobby`.

---

## 5. S2 — Lobby

### 5.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│  City of Towers          Logged in as: alice            [ Log Out ]  │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  OPEN GAMES                                          [ + Create Game ]│
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ Name            Host      Players   Turn    Status             │  │
│  │ ────────────────────────────────────────────────────────────── │  │
│  │ The Long Road   alice     3 / 10    5 min   Waiting   [ Join ] │  │
│  │ Quiet Valley    bob       10 / 10   5 min   Waiting   (full)   │  │
│  │ Old Harbour     carol     6 / 6     1 min   In progress        │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                                                                      │
│  MY GAMES                                                            │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ The Long Road   alice     3 / 10    Waiting   [ Enter ]        │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

### 5.2 Elements

| Element | Position | Behaviour |
| --- | --- | --- |
| Header bar | 0, 0, 1280, 48 | stone; shows username and Log Out |
| "OPEN GAMES" heading | 40, 80 | static |
| Create Game button | 1080, 72, 160, 36 | opens the create dialog (§5.4) |
| Open games table | 40, 120, 1200, auto | one row per game in `Waiting` state |
| "MY GAMES" heading | 40, y+40 | games the user is a player in |
| My games table | 40, y+80, 1200, auto | includes `InProgress` games |

### 5.3 Table columns

| Column | Width | Content |
| --- | --- | --- |
| Name | 300 | game name |
| Host | 160 | host username |
| Players | 120 | `joined / capacity` |
| Turn | 100 | `5 min` / `1 min` / `1 day` |
| Status | 160 | `Waiting` / `In progress` / `Finished` |
| Action | 160 | `[ Join ]` / `[ Enter ]` / `(full)` |

### 5.4 Create Game dialog

A modal over the lobby, 480 × 360, centred.

| Field | Type | Default | Validation |
| --- | --- | --- | --- |
| Game name | text | `"My Game"` | 1–60 chars |
| Capacity | select | 10 | 2–10 |
| Turn length | select | 5 minutes | 30s / 1min / 5min / 1day |
| Turn limit | number | 60 | 0 (unlimited) or 10–500 |

Buttons: `[ Create ]` (primary), `[ Cancel ]`.

On create: the user becomes Host, a `Player` row is created, and the client
navigates to `/game/{id}` (S3).

### 5.5 Behaviour

- The list refreshes every 5 seconds and on any WebSocket `lobbyChanged` event.
- Joining a full game is rejected with a toast: "That game is full."
- Joining a game already in progress is rejected: "That game has already started."
- A user may be in at most one game at a time. Attempting to join a second shows:
  "You are already in a game. Leave it first."

---

## 6. S3 — Game Room (waiting)

### 6.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│  The Long Road          3 / 10 players          [ Start ] [ Cancel ] │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  WAITING FOR PLAYERS                                                 │
│                                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐                │
│  │   [avatar]   │  │   [avatar]   │  │   [avatar]   │                │
│  │    alice     │  │     bob      │  │    carol     │                │
│  │    (host)    │  │              │  │              │                │
│  └──────────────┘  └──────────────┘  └──────────────┘                │
│                                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐                │
│  │  empty slot  │  │  empty slot  │  │  empty slot  │                │
│  └──────────────┘  └──────────────┘  └──────────────┘                │
│                                                                      │
│  Game settings: 10 players · 5 min turns · 60 turn limit             │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

### 6.2 Elements

| Element | Position | Behaviour |
| --- | --- | --- |
| Title | 40, 24 | game name |
| Player count | 400, 24 | `3 / 10 players` |
| Start button | 1000, 16, 120, 36 | host only; disabled until ≥ 2 players |
| Cancel button | 1130, 16, 120, 36 | host only; confirms then returns all to lobby |
| Player cards | grid from 40, 120 | 5 per row, 200 × 120 each |
| Settings line | 40, bottom | read-only summary |

### 6.3 Behaviour

- Non-hosts see no Start/Cancel buttons.
- A non-host sees a `[ Leave ]` button instead.
- When the host starts, all connected clients receive `gameStarted` and navigate
  to S4 (their own city).
- Player cards update live via WebSocket `playerJoined` / `playerLeft`.

---

## 7. S4 — City View

This is the main screen. It is the same layout for the owner and for visitors;
only the available actions differ.

### 7.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ WOOD 120  ORE  88  MERC  4  SULF  3  CRYS  6  GEMS  2  GOLD 4200     │  ← resource bar (48px)
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   ┌────────────────────────────────────────────────────────────┐     │
│   │                                                            │     │
│   │                  CITY SCENE (canvas)                       │     │
│   │                                                            │     │
│   │   [town hall]   [woodcutter]   [ore pit]   [tower]         │     │
│   │                                                            │     │
│   │   [obelisk monument]   [granary]   [empty plot]            │     │
│   │                                                            │     │
│   └────────────────────────────────────────────────────────────┘     │
│                                                                      │
├──────────────────────────────────────────────────────────────────────┤
│  alice's city          Turn 12 · next in 3:42      [Build] [Tower]   │  ← action bar (56px)
│                                                     [Wall]  [Obelisk] │
└──────────────────────────────────────────────────────────────────────┘
```

### 7.2 Elements

| Element | Position (x, y, w, h) | Behaviour |
| --- | --- | --- |
| Resource bar | 0, 0, 1280, 48 | seven resource chips + gold |
| City scene | 40, 64, 1200, 600 | canvas; buildings drawn as sprites |
| City name | 40, 680 | owner's city name |
| Turn indicator | 400, 680 | `Turn 12 · next in 3:42` (live countdown) |
| Build button | 900, 672, 100, 40 | owner only |
| Tower button | 1010, 672, 100, 40 | always |
| Wall button | 1120, 672, 100, 40 | always |
| Obelisk button | 900, 720, 100, 40 | always |
| Back to lobby | 1120, 720, 100, 40 | always |

### 7.3 Resource bar

Seven chips, each 150px wide, starting at x=20:

```
[icon] WOOD   120
```

| Chip | Icon | Colour |
| --- | --- | --- |
| Wood | log | `#8a5a2b` |
| Ore | rock | `#6a6a72` |
| Mercury | droplet | `#b8b8c8` |
| Sulfur | crystal | `#d8c040` |
| Crystal | shard | `#60c0e0` |
| Gems | gem | `#40c060` |
| Gold | coin | `#e8c040` |

Visitors see the owner's resources only if the owner has built a **Marketplace**;
otherwise the chips show `—`.

### 7.4 City scene

- Canvas 1200 × 600, drawn at 2× for crispness.
- Buildings are placed on a **6 × 3 grid** of 200 × 200 cells.
- Each built building occupies one cell, drawn as a 160 × 160 sprite centred in it.
- Empty cells show a faint plot outline.
- Hovering a building shows a tooltip with its name and description.
- Clicking a building opens a detail popover (name, description, built on turn).

### 7.5 Owner vs visitor

| Action | Owner | Visitor |
| --- | --- | --- |
| See resources | yes | only with Marketplace |
| Build button | visible | **hidden** |
| Rename city | yes (via city name click) | no |
| Read tower | yes | yes |
| Post to tower | yes | no |
| Leave wall message | yes | yes |
| Delete wall messages | yes | no |
| Visit obelisk | yes | yes |

### 7.6 Build dialog

Opened by the Build button. Modal 900 × 600, centred.

```
┌──────────────────────────────────────────────────────────────────────┐
│  BUILD                                          [ × ]                │
├──────────────────────────────────────────────────────────────────────┤
│  You may build one structure per turn.                               │
│                                                                      │
│  ┌────────────────────────────┐  ┌────────────────────────────────┐  │
│  │ INCOME                     │  │  Woodcutter's Lodge            │  │
│  │  Woodcutter's Lodge        │  │  ─────────────────────────     │  │
│  │  Ore Pit                   │  │  +2 wood per turn.             │  │
│  │  Alchemist's Lab           │  │                                │  │
│  │  Sulfur Dune               │  │  Cost:  500 gold, 5 wood       │  │
│  │  Crystal Cavern            │  │  Requires: —                   │  │
│  │  Gem Pond                  │  │                                │  │
│  │ CIVIC                      │  │  [ Build ]                     │  │
│  │  Marketplace               │  │                                │  │
│  │  Tavern                    │  │                                │  │
│  │  Granary                   │  │                                │  │
│  │  Treasury                  │  │                                │  │
│  │  City Hall                 │  │                                │  │
│  │ MAGIC                      │  │                                │  │
│  │  Magic Tower I–V           │  │                                │  │
│  │ MONUMENT                   │  │                                │  │
│  │  Monument of the Obelisk   │  │                                │  │
│  │ ULTIMATE                   │  │                                │  │
│  │  The Grand Archive         │  │                                │  │
│  └────────────────────────────┘  └────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
```

- Left list: 300px wide, grouped by category, scrollable.
- Right panel: details of the selected building.
- Each list row shows a state marker:
  - `✓` built
  - `●` available (prerequisites met, affordable)
  - `○` available but unaffordable (cost shown in red)
  - `✕` locked (prerequisites unmet; the missing prerequisite is named)
- The Build button is disabled with a reason line when:
  - already built this turn → "You have already built this turn."
  - unaffordable → "Not enough resources."
  - locked → "Requires: Granary."
- On success: the dialog closes, the city scene updates, a toast shows
  "Built Woodcutter's Lodge."

---

## 8. S5 — Magic Tower

### 8.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│  ← Back to city        alice's Magic Tower       3 / 6 posts         │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   ┌──────────────────────┐   ┌───────────────────────────────────┐   │
│   │  SPELLBOOK           │   │  The Long Road Home               │   │
│   │  ──────────────      │   │  ─────────────────────────────    │   │
│   │  ▸ The Long Road     │   │  posted 2 days ago                │   │
│   │  ▸ On Building       │   │                                   │   │
│   │  ▸ A Quiet Morning   │   │  [image if present]               │   │
│   │                      │   │                                   │   │
│   │  [+ New Post]        │   │  Body text of the post, wrapped   │   │
│   │                      │   │  at 70 characters per line...     │   │
│   │                      │   │                                   │   │
│   │                      │   │  ── Comments (2) ──               │   │
│   │                      │   │  bob: Lovely writing.             │   │
│   │                      │   │  carol: Agreed!                   │   │
│   │                      │   │  [ write a comment... ]  [ Post ] │   │
│   └──────────────────────┘   └───────────────────────────────────┘   │
│                                                                      │
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │  TOWER GUESTBOOK                                             │   │
│   │  dave: Thanks for the tour!                                  │   │
│   │  [ leave a message... ]                            [ Post ]  │   │
│   └──────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘
```

### 8.2 Elements

| Element | Position | Behaviour |
| --- | --- | --- |
| Back link | 40, 20 | returns to S4 |
| Title | 200, 20 | `{owner}'s Magic Tower` |
| Post counter | 1000, 20 | `3 / 6 posts` |
| Spellbook list | 40, 64, 300, 500 | one row per post; selected row highlighted |
| New Post button | 40, 580, 300, 40 | owner only; disabled at slot limit |
| Post reader | 360, 64, 880, 500 | title, date, image, body |
| Comment list | inside reader, bottom | newest last |
| Comment box | inside reader | any player |
| Guestbook | 40, 600, 1200, 160 | tower-level messages |

### 8.3 Behaviour

- If the owner has no Magic Tower, the screen shows:
  "This city has no Magic Tower yet." and only the guestbook.
- Selecting a post loads it into the reader.
- The New Post button is disabled at the slot limit with the tooltip
  "Your tower holds 6 posts. Upgrade it to write more."
- Post editor is a modal: Title (max 120), Body (max 20000, plain text),
  Image URL (optional, max 500). Buttons `[ Save ]` `[ Cancel ]`.
- Only the owner sees `[ Edit ]` and `[ Delete ]` on their own posts.
- Comments and guestbook messages are plain text, max 2000 chars.

---

## 9. S6 — Obelisk

### 9.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│  ← Back to city              THE OBELISK                             │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │                                                              │   │
│   │                    [obelisk sprite]                          │   │
│   │                                                              │   │
│   │              You have seen 4 of 10 fragments.                │   │
│   │                    [ Visit the Obelisk ]                     │   │
│   └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │  FRAGMENTS                                                   │   │
│   │  1. I speak without a mouth and hear without ears.           │   │
│   │  2. I have no body, but I come alive with wind.              │   │
│   │  3. Shout into a canyon and I answer.                        │   │
│   │  4. I repeat what you say, but never first.                  │   │
│   │  5. ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   │   │
│   │  ...                                                         │   │
│   └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │  SOLVE THE RIDDLE                                            │   │
│   │  [ your answer...                              ]  [ Submit ] │   │
│   │  attempts remaining this minute: 5                           │   │
│   └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │  MESSAGES AT THE OBELISK                                     │   │
│   │  bob: I think it's about sound.                              │   │
│   │  [ leave a message... ]                            [ Post ]  │   │
│   └──────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘
```

### 9.2 Elements

| Element | Position | Behaviour |
| --- | --- | --- |
| Back link | 40, 20 | returns to S4 |
| Title | centred, 20 | `THE OBELISK` |
| Obelisk panel | 40, 64, 1200, 200 | sprite + progress + Visit button |
| Visit button | centred in panel | reveals the next fragment |
| Fragment list | 40, 280, 1200, 200 | revealed fragments in full; unrevealed as `░` |
| Solve panel | 40, 496, 1200, 100 | answer field + Submit |
| Message list | 40, 612, 1200, 160 | all players' messages |

### 9.3 Behaviour

- The Visit button is disabled when all 10 fragments are seen, with the label
  "You have seen every fragment."
- Visiting reveals exactly one new fragment and shows a toast:
  "Fragment 5 of 10 revealed."
- Submitting a wrong answer shows: "That is not the answer. Try again."
  and decrements the attempt counter.
- After 5 wrong answers in a minute, Submit is disabled for 60 seconds with a
  countdown.
- After 10 total wrong answers, the player is locked out until the next turn.
- A correct answer: the screen transitions to S7 (Game Over) for all players.

---

## 10. S7 — Game Over

### 10.1 Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│                                                                      │
│                        THE RIDDLE IS SOLVED                          │
│                                                                      │
│                    alice has claimed the Precious                    │
│                                                                      │
│              The answer was:  ECHO                                   │
│                                                                      │
│              ┌────────────────────────────────────┐                  │
│              │  Final standings                   │                  │
│              │  ─────────────────────────────     │                  │
│              │  1. alice    — Precious            │                  │
│              │  2. bob      — 7 fragments         │                  │
│              │  3. carol    — 4 fragments         │                  │
│              └────────────────────────────────────┘                  │
│                                                                      │
│                        [ Return to Lobby ]                           │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

### 10.2 Behaviour

- Shown to all players when the game ends, whether they won or not.
- If the turn limit expired instead, the title reads `THE GAME HAS ENDED`
  and the subtitle reads `No one solved the riddle.`
- The game becomes read-only: all cities remain viewable, no new content.

---

## 11. Game lifecycle

```
  [Lobby] --create--> [Waiting] --host starts--> [In progress] --solved--> [Finished]
                          ^                            |
                          +------ (host cancels) ------+
```

### 11.1 Lobby
- A Guest registers or logs in.
- A Player creates a game (becomes Host) or joins an open game.
- Capacity 2–10, chosen by the Host.

### 11.2 Waiting
- Host sees joined players; may start (≥ 2 players) or cancel.
- Players may leave.

### 11.3 In progress
- Each player has a city.
- Turns advance on schedule (§12).
- Players visit cities and the obelisk.

### 11.4 Finished
- Triggered by a correct answer or by the turn limit.
- Read-only thereafter.

---

## 12. Turns

- One turn = one day.
- **Default 5 minutes**, configurable: 30s / 1min / 5min / 1day.
- At the start of each turn every city accrues income.
- Turns gate **building only**. Reading, writing, and visiting are always allowed.
- The server is authoritative; the client shows a countdown derived from
  `turnStartedAt + turnSeconds`.

---

## 13. Resources

| Resource | Source building | Per turn |
| --- | --- | --- |
| Wood | Woodcutter's Lodge | +2 |
| Ore | Ore Pit | +2 |
| Mercury | Alchemist's Lab | +1 |
| Sulfur | Sulfur Dune | +1 |
| Crystal | Crystal Cavern | +1 |
| Gems | Gem Pond | +1 |
| Gold | Town Hall | +500 |

Granary multiplies wood and ore by 1.5. City Hall and Treasury add gold.
Resources are not tradeable in v1.

---

## 14. Buildings

### 14.1 Income

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `townHall` | Town Hall | — (starting) | — | +500 gold/turn |
| `woodcutter` | Woodcutter's Lodge | 500g, 5w | — | +2 wood/turn |
| `orePit` | Ore Pit | 500g, 5w | — | +2 ore/turn |
| `alchemist` | Alchemist's Lab | 1000g, 5w, 5o | — | +1 mercury/turn |
| `sulfurDune` | Sulfur Dune | 1000g, 5w, 5o | — | +1 sulfur/turn |
| `crystalCavern` | Crystal Cavern | 1000g, 5w, 5o | — | +1 crystal/turn |
| `gemPond` | Gem Pond | 1000g, 5w, 5o | — | +1 gems/turn |

### 14.2 Civic

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `marketplace` | Marketplace | 500g, 5w | townHall | Visitors can see your resources |
| `tavern` | Tavern | 500g, 5w | townHall | Shows visitor count |
| `granary` | Granary | 1000g, 5w, 5o | woodcutter, orePit | +50% wood and ore |
| `treasury` | Treasury | 2000g, 10o | townHall | +250 gold/turn |
| `cityHall` | City Hall | 2000g, 10w, 10o | townHall | +500 gold/turn |

### 14.3 Magic

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `magicTower1` | Magic Tower I | 1000g, 5w, 5o | townHall | 3 post slots |
| `magicTower2` | Magic Tower II | 1500g, 5w, 5o, 4crystal | magicTower1 | 6 post slots |
| `magicTower3` | Magic Tower III | 2000g, 5w, 5o, 6gems | magicTower2 | 10 post slots |
| `magicTower4` | Magic Tower IV | 2500g, 5w, 5o, 6sulfur | magicTower3 | 15 post slots |
| `magicTower5` | Magic Tower V | 3000g, 5w, 5o, 6mercury | magicTower4 | 25 post slots |

### 14.4 Monument

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `obeliskMonument` | Monument of the Obelisk | 2000g, 10o | townHall | Access to the standalone Obelisk |

### 14.5 Ultimate

| Key | Name | Cost | Prereq | Effect |
| --- | --- | --- | --- | --- |
| `grandArchive` | The Grand Archive | 10000g, 20w, 20o, 10 of each rare | obeliskMonument + Precious | Ends the game |

### 14.6 Construction rules
- One building per turn.
- Prerequisites must exist; cost must be affordable.
- Construction is immediate.
- No demolition in v1.

---

## 15. The Obelisk and the Global Puzzle

- One **standalone Obelisk**, reachable from any city view.
- Visiting reveals **one fragment** of the global riddle, sequentially.
- **10 fragments total.** Every player starts at fragment 1.
- After visiting, a player may paste a message at the Obelisk, visible to all.
- The riddle is **global**: one answer for the whole game.
- The server picks the riddle at game creation from a built-in list of 20.
  No player knows the answer in advance.
- A correct answer awards the **Precious** and ends the game.
- Answer comparison: lowercase, trimmed, whitespace collapsed, punctuation stripped.

---

## 16. The Magic Tower (Blog)

- Each city has a Magic Tower holding the owner's blog posts.
- A post has: title, body, optional image URL, timestamp.
- Post slots by tower level: 3 / 6 / 10 / 15 / 25.
- Only the owner may create, edit, or delete posts.
- Visitors may read posts and comment.
- The tower also has a **guestbook** where any player may leave a message.

---

## 17. Messages

### 17.1 City wall
- Every city has a wall.
- Any player may post; messages are threaded (replies).
- The owner may delete messages on their own wall.

### 17.2 Post comments
- Each post has a comment thread; any player may comment.
- The post owner may delete comments on their own posts.

### 17.3 Rules
- Max 2000 characters, plain text, no HTML.
- Timestamped and attributed.
- Rate limit: 10 messages per minute per user per game.

---

## 18. Visiting

- Any player may visit any other player's city at any time.
- Visitors see the city read-only: buildings, tower, wall.
- Visitors may read posts, comment, and leave wall messages.
- Visitors may not build, rename, spend resources, or delete content.

---

## 19. Accounts and sessions

- Username 3–20 chars `[A-Za-z0-9_]`; password ≥ 8 chars.
- Passwords hashed with PBKDF2-SHA256, 100k iterations, per-user salt.
- Cookie sessions, 7-day expiry.
- A user may be in at most one game at a time.

---

## 20. Non-functional requirements

| Requirement | Target |
| --- | --- |
| Concurrent players per game | 10 |
| Concurrent games | 50 |
| Turn tick accuracy | ±2 seconds |
| Page load | < 2s on a local network |
| Persistence | Survives server restart |
| Browser support | Chrome, Edge, Firefox (current) |
| Logical resolution | 1280 × 800, scaled to fit |

---

## 21. Out of scope for v1

- Combat of any kind
- Resource trading between players
- Alliances or teams
- Mobile-native apps
- Chat outside the wall/comment system
- Multiple cities per player
- Building demolition
- Password recovery
- Global moderation

---

## 22. Acceptance criteria

| # | Criterion |
| --- | --- |
| AC-1 | A user can register, log in, and log out. |
| AC-2 | A user can create a game with a chosen capacity (2–10) and turn length. |
| AC-3 | Up to 10 users can join a waiting game; the 11th is rejected. |
| AC-4 | The host can start the game; all players receive a city. |
| AC-5 | Resources accrue once per turn for every city. |
| AC-6 | A player can construct a building if prerequisites and cost are met. |
| AC-7 | A player cannot construct two buildings in one turn. |
| AC-8 | A player cannot construct a building whose prerequisites are unmet. |
| AC-9 | A visitor sees another city read-only and cannot build there. |
| AC-10 | A visitor can leave a message on another city's wall. |
| AC-11 | Messages support replies. |
| AC-12 | A player can create a blog post in their Magic Tower, up to the slot limit. |
| AC-13 | A visitor can read another player's blog posts and comment on them. |
| AC-14 | Visiting the Obelisk reveals the next puzzle fragment. |
| AC-15 | A player can paste a message at the Obelisk, visible to all. |
| AC-16 | Submitting the correct puzzle answer awards the Precious and ends the game. |
| AC-17 | Submitting a wrong answer is rejected without ending the game. |
| AC-18 | Game state survives a server restart. |
| AC-19 | Two browsers logged in as different players see each other's messages in real time. |
| AC-20 | A visitor without a Marketplace cannot see the owner's resources. |
| AC-21 | The Build button is hidden for visitors. |
| AC-22 | The New Post button is disabled at the tower's slot limit. |
| AC-23 | The Visit button is disabled after all 10 fragments are seen. |
| AC-24 | Five wrong answers in a minute disables Submit for 60 seconds. |