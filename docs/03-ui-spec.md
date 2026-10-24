> **SUPERSEDED.** Wrong layout. Replaced by 09-ui-reference.md.
> See `00-FIDELITY-CONTRACT.md` and `09-ui-reference.md`.

# UI Specification — City of Towers

**Version:** 1.1
**Purpose:** This document is precise enough that two independent developers, given
only this file, will produce pixel-identical screens. Every position, size, colour,
font, state, and transition is specified. Where a value is not given, it is not
free to choose — it is a defect in this document.

---

## 0. Global rules

### 0.1 Logical resolution
- The game renders at a fixed logical size of **1280 × 800**.
- The whole app is scaled uniformly to fit the browser window:
  `scale = min(window.innerWidth / 1280, window.innerHeight / 800)`.
- The app is centred; letterbox bars fill the remainder with `#000`.
- All coordinates in this document are in logical pixels, origin top-left.

### 0.2 Design tokens

| Token | Value | Used for |
| --- | --- | --- |
| `--bg` | `#0d0b08` | page background |
| `--panel` | `#2a2016` | panel fill |
| `--panel-hi` | `#3d2f20` | panel top highlight |
| `--panel-lo` | `#1a130c` | panel bottom shadow |
| `--border` | `#c8a050` | gold trim |
| `--border-dim` | `#7a6030` | inner trim |
| `--text` | `#f0e0c0` | primary text |
| `--text-dim` | `#a89070` | secondary text |
| `--text-mute` | `#6a5a40` | disabled text |
| `--accent` | `#ffd040` | selected / active |
| `--danger` | `#e05040` | errors, destructive |
| `--ok` | `#60c060` | success, built |
| `--wood` | `#8a5a2b` | wood chip |
| `--ore` | `#6a6a72` | ore chip |
| `--mercury` | `#b8b8c8` | mercury chip |
| `--sulfur` | `#d8c040` | sulfur chip |
| `--crystal` | `#60c0e0` | crystal chip |
| `--gems` | `#40c060` | gems chip |
| `--gold` | `#e8c040` | gold chip |

### 0.3 Typography

| Role | Font | Size | Weight | Colour |
| --- | --- | --- | --- | --- |
| Screen title | `Georgia, "Times New Roman", serif` | 32px | bold | `--accent` |
| Section heading | same | 18px | bold | `--text` |
| Body | same | 14px | normal | `--text` |
| Small / meta | same | 12px | normal | `--text-dim` |
| Button label | same | 14px | bold | `--text` |
| Resource value | same | 15px | bold | `--text` |
| Error | same | 13px | normal | `--danger` |

Line height is `1.45` for body text. Text is never smaller than 12px.

### 0.4 Panel style
Every panel is drawn as:
1. Fill with a vertical gradient `--panel-hi` → `--panel` → `--panel-lo`.
2. 1px border in `--border`.
3. 1px inner border in `--border-dim`, inset 3px.
4. Corner radius 4px.

### 0.5 Button style

| State | Fill | Border | Text |
| --- | --- | --- | --- |
| Normal | `#4a3418` | `--border` | `--text` |
| Hover | `#6a4a20` | `--accent` | `--text` |
| Active (pressed) | `#2a1a08` | `--border-dim` | `--text` |
| Disabled | `#2a2016` | `#4a3a20` | `--text-mute` |

Buttons are 40px tall unless stated. Horizontal padding is 16px. Minimum width 80px.

### 0.6 Focus and keyboard
- Every interactive element is reachable by **Tab** in reading order.
- The focused element shows a 2px `--accent` outline offset by 2px.
- **Enter** activates the focused button; **Space** activates it too.
- **Escape** closes the topmost modal, or returns to the previous screen.

### 0.7 Toast
- Appears bottom-centre, 24px above the bottom edge.
- Size: auto width, 40px tall, max 600px wide.
- Fill `#1a130c`, border `--border`, text `--text`, 14px.
- Fades in over 150ms, holds 2.5s, fades out over 300ms.
- Only one toast at a time; a new toast replaces the old.

### 0.8 Modal
- Backdrop: `rgba(0,0,0,0.65)` covering the whole viewport.
- Panel centred both axes.
- Clicking the backdrop closes the modal (except where stated).
- Escape closes the modal.

---

## 1. S1 — Landing / Login

### 1.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Title | 0 | 150 | 1280 | 48 |
| Subtitle | 0 | 206 | 1280 | 24 |
| Panel | 440 | 280 | 400 | 240 |
| "Username" label | 470 | 306 | 100 | 20 |
| Username input | 470 | 330 | 340 | 36 |
| "Password" label | 470 | 380 | 100 | 20 |
| Password input | 470 | 404 | 340 | 36 |
| Log In button | 470 | 460 | 160 | 40 |
| Register button | 650 | 460 | 160 | 40 |
| Error line | 470 | 508 | 340 | 20 |

Title and subtitle are horizontally centred (`text-align: center`).

### 1.2 Input style
- Fill `#120d08`, border 1px `--border-dim`, text `--text`, 14px.
- Padding 8px horizontal.
- Focus: border becomes `--accent`.
- Placeholder colour `--text-mute`.

### 1.3 States

| State | Trigger | Appearance |
| --- | --- | --- |
| Idle | default | as above |
| Submitting | button pressed | both buttons disabled, label becomes `…` |
| Error | server rejects | error line shows message in `--danger`; inputs keep values |
| Success | server accepts | navigate to `/lobby` |

### 1.4 Validation messages (exact strings)

| Condition | Message |
| --- | --- |
| Username < 3 chars | `Username must be at least 3 characters.` |
| Username > 20 chars | `Username must be at most 20 characters.` |
| Username has bad chars | `Username may only contain letters, digits and underscore.` |
| Password < 8 chars | `Password must be at least 8 characters.` |
| Username taken | `That username is already taken.` |
| Bad credentials | `Incorrect username or password.` |
| Network failure | `Cannot reach the server. Please try again.` |

---

## 2. S2 — Lobby

### 2.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Header bar | 0 | 0 | 1280 | 48 |
| "City of Towers" | 24 | 14 | 300 | 20 |
| "Logged in as: {name}" | 900 | 14 | 260 | 20 |
| Log Out button | 1170 | 8 | 90 | 32 |
| "OPEN GAMES" heading | 40 | 76 | 300 | 24 |
| Create Game button | 1080 | 68 | 160 | 36 |
| Open games table | 40 | 112 | 1200 | auto |
| "MY GAMES" heading | 40 | (below table + 32) | 300 | 24 |
| My games table | 40 | (heading + 36) | 1200 | auto |

### 2.2 Table style
- Header row: 32px tall, fill `#1a130c`, text `--text-dim`, 12px, uppercase.
- Data row: 44px tall, alternating fill `#221a10` / `#1a130c`.
- Row hover: fill `#2e2416`.
- 1px separator in `#3a2c1c` between rows.

### 2.3 Column widths

| Column | x offset | Width | Align |
| --- | --- | --- | --- |
| Name | 0 | 300 | left |
| Host | 300 | 160 | left |
| Players | 460 | 120 | centre |
| Turn | 580 | 100 | centre |
| Status | 680 | 160 | left |
| Action | 840 | 160 | right |

### 2.4 Row states

| Game state | Status text | Action cell |
| --- | --- | --- |
| Waiting, not full, not joined | `Waiting` | `[ Join ]` button |
| Waiting, full | `Waiting` | text `(full)` in `--text-mute` |
| Waiting, already joined | `Waiting` | `[ Enter ]` button |
| In progress, joined | `In progress` | `[ Enter ]` button |
| In progress, not joined | `In progress` | text `(in progress)` in `--text-mute` |
| Finished | `Finished` | text `(finished)` in `--text-mute` |

### 2.5 Create Game modal

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Panel | 400 | 200 | 480 | 380 |
| Title "Create Game" | 400 | 216 | 480 | 28 |
| "Game name" label | 432 | 264 | 200 | 20 |
| Name input | 432 | 288 | 416 | 36 |
| "Capacity" label | 432 | 340 | 200 | 20 |
| Capacity select | 432 | 364 | 200 | 36 |
| "Turn length" label | 432 | 416 | 200 | 20 |
| Turn select | 432 | 440 | 200 | 36 |
| "Turn limit" label | 432 | 492 | 200 | 20 |
| Turn limit input | 432 | 516 | 200 | 36 |
| Create button | 560 | 560 | 140 | 40 |
| Cancel button | 712 | 560 | 140 | 40 |

Select options:
- Capacity: `2, 3, 4, 5, 6, 7, 8, 9, 10` (default `10`)
- Turn length: `30 seconds`, `1 minute`, `5 minutes`, `1 day` (default `5 minutes`)
- Turn limit: number input, default `60`, hint below: `0 = unlimited`

### 2.6 Behaviour
- The table refreshes every 5 seconds and on `lobbyChanged`.
- Clicking `[ Join ]` posts to the server; on success navigate to `/game/{id}`.
- Clicking `[ Enter ]` navigates to `/game/{id}`.
- Errors appear as a toast with the exact strings in §2.7.

### 2.7 Error strings

| Condition | Message |
| --- | --- |
| Game full | `That game is full.` |
| Game started | `That game has already started.` |
| Already in a game | `You are already in a game. Leave it first.` |
| Game not found | `That game no longer exists.` |

---

## 3. S3 — Game Room (waiting)

### 3.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Header bar | 0 | 0 | 1280 | 48 |
| Game name | 24 | 14 | 400 | 20 |
| Player count | 440 | 14 | 200 | 20 |
| Start button | 1000 | 8 | 120 | 32 |
| Cancel button | 1130 | 8 | 120 | 32 |
| "WAITING FOR PLAYERS" | 40 | 80 | 400 | 24 |
| Player grid | 40 | 120 | 1200 | auto |
| Settings line | 40 | 720 | 1200 | 20 |

### 3.2 Player card
- Size 200 × 120, gap 24px, 5 per row.
- Fill `--panel`, border `--border`.
- Avatar: 64 × 64 circle at (68, 12) within the card, fill `#4a3418`,
  initials in `--accent` 24px bold.
- Name: centred, y=88, 14px `--text`.
- `(host)` badge: centred, y=106, 12px `--accent`, only for the host.
- Empty slot: dashed 1px `--border-dim` border, no fill, text `empty` centred
  in `--text-mute`.

### 3.3 Behaviour
- Start is disabled until `players >= 2`; tooltip `Need at least 2 players.`
- Cancel shows a confirm modal: `Cancel this game? All players will return to the lobby.`
  with `[ Cancel Game ]` and `[ Keep Playing ]`.
- Non-hosts see `[ Leave ]` at 1130, 8, 120, 32 instead of Start/Cancel.
- On `gameStarted`, navigate to `/game/{id}/city/{myPlayerId}`.

---

## 4. S4 — City View

### 4.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Resource bar | 0 | 0 | 1280 | 48 |
| City scene canvas | 40 | 64 | 1200 | 600 |
| City name | 40 | 680 | 400 | 24 |
| Turn indicator | 460 | 680 | 300 | 24 |
| Build button | 900 | 672 | 100 | 40 |
| Tower button | 1010 | 672 | 100 | 40 |
| Wall button | 1120 | 672 | 100 | 40 |
| Obelisk button | 900 | 720 | 100 | 40 |
| Lobby button | 1120 | 720 | 100 | 40 |

### 4.2 Resource bar
- Fill `#1a130c`, bottom border 1px `--border`.
- Seven chips starting at x=20, each 150px wide, 48px tall.
- Chip layout: icon 24 × 24 at (x+8, 12); label 12px `--text-dim` at (x+40, 18);
  value 15px bold `--text` at (x+40, 36).
- Chip order: Wood, Ore, Mercury, Sulfur, Crystal, Gems, Gold.
- Hidden values (visitor without Marketplace) render as `—` in `--text-mute`.

### 4.3 City scene
- Canvas 1200 × 600, backing store 2400 × 1200 (2× for crispness).
- Grid: 6 columns × 3 rows, cell 200 × 200, origin (0, 0) of the canvas.
- Building sprite: 160 × 160, centred in its cell at `(col*200+20, row*200+20)`.
- Empty cell: 1px dashed `#3a2c1c` rectangle inset 20px, no fill.
- Building placement order: buildings fill cells left-to-right, top-to-bottom,
  in the order they were constructed.
- Hover: the building's cell gets a 2px `--accent` outline.
- Hover tooltip: 12px `--text` on `#1a130c`, 4px padding, follows the cursor
  offset by (12, 12), clamped to stay inside the canvas.

### 4.4 Turn indicator
- Text: `Turn {n} · next in {m}:{ss}`.
- Updates every second.
- When `{m}:{ss}` reaches `0:00`, the text becomes `Turn {n+1} · next in …`
  on the next server tick.

### 4.5 Owner vs visitor

| Element | Owner | Visitor |
| --- | --- | --- |
| Build button | visible | **not rendered** |
| City name | clickable (rename) | plain text |
| Resource values | shown | shown only with Marketplace, else `—` |
| Tower button | visible | visible |
| Wall button | visible | visible |
| Obelisk button | visible | visible |

### 4.6 Build modal

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Panel | 190 | 100 | 900 | 600 |
| Title "BUILD" | 190 | 112 | 900 | 28 |
| Close × | 1050 | 108 | 32 | 32 |
| Hint line | 222 | 152 | 836 | 20 |
| Category list | 222 | 184 | 300 | 480 |
| Detail panel | 540 | 184 | 518 | 480 |
| Build button | 900 | 600 | 140 | 40 |

Category list rows: 28px tall, 12px text, category headers in `--accent` bold,
building rows indented 12px.

State markers (prefix, 16px wide):
- `✓` `--ok` — built
- `●` `--text` — available and affordable
- `○` `--text-dim` — available, unaffordable
- `✕` `--text-mute` — locked

Detail panel contents, top to bottom:
1. Building name, 18px bold `--text`, y=200
2. Divider line, y=228
3. Description, 14px `--text`, y=252, wrapped at 60 chars
4. `Cost:` line, 14px, y=300; each resource in its chip colour
5. `Requires:` line, 14px, y=328; `—` if none
6. Build button at (900, 600, 140, 40)

Build button states:

| Condition | Label | Enabled | Reason line (y=580) |
| --- | --- | --- | --- |
| Built | `Built` | no | — |
| Locked | `Locked` | no | `Requires: {name}.` |
| Unaffordable | `Build` | no | `Not enough resources.` |
| Already built this turn | `Build` | no | `You have already built this turn.` |
| OK | `Build` | yes | — |

### 4.7 Rename modal
- Opened by clicking the city name (owner only).
- Panel 400 × 180 centred.
- Input 320 × 36, max 40 chars.
- Buttons `[ Save ]` `[ Cancel ]`.

---

## 5. S5 — Magic Tower

### 5.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Back link | 24 | 16 | 160 | 24 |
| Title | 200 | 16 | 600 | 24 |
| Post counter | 1000 | 16 | 240 | 24 |
| Spellbook panel | 40 | 64 | 300 | 500 |
| New Post button | 40 | 580 | 300 | 40 |
| Reader panel | 360 | 64 | 880 | 500 |
| Guestbook panel | 40 | 600 | 1200 | 160 |

### 5.2 Spellbook list
- Row height 36px, 14px text, padding-left 12px.
- Selected row: fill `#3d2f20`, left border 3px `--accent`.
- Hover row: fill `#2e2416`.
- Empty state: `No posts yet.` centred in `--text-mute`.

### 5.3 Reader panel
- Title: 20px bold `--text` at (380, 88).
- Meta line: 12px `--text-dim` at (380, 116), format `posted {relative time}`.
- Image (if present): max 400 × 240 at (380, 140), object-fit contain.
- Body: 14px `--text`, starting y = 140 (or 396 if an image is present),
  wrapped at 90 chars, line height 1.45.
- Comments heading: 14px bold at the bottom of the body area.
- Comment rows: 12px, author in `--accent`, body in `--text`.
- Comment input: 600 × 32, `[ Post ]` button 80 × 32 to its right.

### 5.4 Post editor modal

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Panel | 340 | 120 | 600 | 560 |
| Title "New Post" | 340 | 132 | 600 | 28 |
| Title label | 372 | 180 | 200 | 20 |
| Title input | 372 | 204 | 536 | 36 |
| Body label | 372 | 256 | 200 | 20 |
| Body textarea | 372 | 280 | 536 | 240 |
| Image label | 372 | 536 | 200 | 20 |
| Image input | 372 | 560 | 536 | 36 |
| Hint | 372 | 604 | 536 | 20 |
| Save button | 700 | 632 | 100 | 40 |
| Cancel button | 808 | 632 | 100 | 40 |

Hint text: `Image must be a public URL. Uploads are not supported.`

### 5.5 Behaviour
- No tower: the spellbook and reader are replaced by a single centred message
  `This city has no Magic Tower yet.` The guestbook remains.
- New Post disabled at the slot limit; tooltip
  `Your tower holds {n} posts. Upgrade it to write more.`
- Owner sees `[ Edit ]` and `[ Delete ]` at the top-right of the reader panel.
- Delete shows a confirm modal: `Delete this post? This cannot be undone.`

---

## 6. S6 — Obelisk

### 6.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Back link | 24 | 16 | 160 | 24 |
| Title | 0 | 16 | 1280 | 24 |
| Obelisk panel | 40 | 64 | 1200 | 200 |
| Obelisk sprite | 560 | 80 | 160 | 160 |
| Progress text | 0 | 200 | 1280 | 20 |
| Visit button | 560 | 224 | 160 | 32 |
| Fragments panel | 40 | 280 | 1200 | 200 |
| Solve panel | 40 | 496 | 1200 | 100 |
| Messages panel | 40 | 612 | 1200 | 160 |

### 6.2 Fragment list
- 10 rows, 18px tall, starting at (60, 300).
- Revealed: `{n}. {text}` in `--text`.
- Unrevealed: `{n}. ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░` in `--text-mute`.

### 6.3 Solve panel
- Label `SOLVE THE RIDDLE` 14px bold at (60, 512).
- Answer input 600 × 36 at (60, 536).
- Submit button 100 × 36 at (672, 536).
- Attempts line 12px `--text-dim` at (60, 580):
  `attempts remaining this minute: {n}`.

### 6.4 Behaviour
- Visit disabled when `visits >= 10`; label becomes `All fragments seen`.
- On visit: toast `Fragment {n} of 10 revealed.`
- Wrong answer: toast `That is not the answer. Try again.`
- Rate limited: Submit disabled, label `Wait {s}s`.
- Locked out: Submit disabled, label `Locked until next turn`.
- Correct: navigate to `/game/{id}/over`.

---

## 7. S7 — Game Over

### 7.1 Geometry

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Title | 0 | 120 | 1280 | 40 |
| Subtitle | 0 | 176 | 1280 | 24 |
| Answer line | 0 | 220 | 1280 | 24 |
| Standings panel | 440 | 280 | 400 | 240 |
| Return button | 560 | 560 | 160 | 40 |

### 7.2 Content

| Condition | Title | Subtitle | Answer line |
| --- | --- | --- | --- |
| Solved | `THE RIDDLE IS SOLVED` | `{name} has claimed the Precious` | `The answer was: {ANSWER}` |
| Turn limit | `THE GAME HAS ENDED` | `No one solved the riddle.` | `The answer was: {ANSWER}` |

Standings rows: rank, name, and `— Precious` or `— {n} fragments`.

---

## 8. Screen transition map

```
S1 Login ──login──> S2 Lobby
S2 Lobby ──create──> S3 Game Room
S2 Lobby ──join────> S3 Game Room
S2 Lobby ──enter───> S4 City (own)   [if game in progress]
S3 Game Room ──start──> S4 City (own)
S3 Game Room ──leave──> S2 Lobby
S4 City ──Tower───> S5 Magic Tower
S4 City ──Wall────> S4 City (wall panel open)
S4 City ──Obelisk─> S6 Obelisk
S4 City ──visit other city──> S4 City (read-only)
S5 Tower ──back───> S4 City
S6 Obelisk ──back─> S4 City
S6 Obelisk ──correct answer──> S7 Game Over
any ──gameFinished event──> S7 Game Over
S7 Game Over ──return──> S2 Lobby
```

---

## 9. Wall panel (overlay on S4)

The wall is not a separate screen; it is a panel that slides in from the right.

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Panel | 880 | 64 | 360 | 600 |
| Title `WALL` | 900 | 80 | 320 | 24 |
| Close × | 1200 | 76 | 32 | 32 |
| Message list | 900 | 112 | 320 | 420 |
| Input | 900 | 544 | 240 | 36 |
| Post button | 1148 | 544 | 72 | 36 |

- Message row: author 12px `--accent`, body 13px `--text`, time 11px `--text-mute`.
- Replies are indented 16px and prefixed with `↳`.
- Owner sees a `×` on each message to delete it.

---

## 10. Error and empty states

| Screen | Condition | Display |
| --- | --- | --- |
| Any | server unreachable | full-screen message `Cannot reach the server.` with `[ Retry ]` |
| S2 | no open games | `No open games. Create one to get started.` |
| S2 | no my games | `You are not in any game.` |
| S5 | no posts | `No posts yet.` |
| S5 | no tower | `This city has no Magic Tower yet.` |
| S6 | no fragments | `You have not visited the Obelisk yet.` |
| S6 | no messages | `No messages yet.` |
| S9 | no wall messages | `No messages yet. Be the first.` |

---

## 11. Accessibility

- All text meets 4.5:1 contrast against its background.
- All interactive elements have an accessible name.
- Images have `alt` text; decorative sprites use `alt=""`.
- The game is fully operable by keyboard.
- Colour is never the only signal: state markers use both a glyph and a colour.

---

## 12. Conformance checklist

A build conforms to this specification if and only if:

1. Every element in §1–§9 appears at the stated coordinates.
2. Every state in the state tables is reachable and renders as described.
3. Every string in the error tables is reproduced verbatim.
4. Every transition in §8 works.
5. Every empty state in §10 renders.
6. Keyboard navigation follows §0.6.