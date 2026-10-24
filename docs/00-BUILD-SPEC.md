> **SUPERSEDED.** Its UI section (top resource bar, bottom-right minimap, grid town) is wrong. Kept only for the creature and building tables.
> See `00-FIDELITY-CONTRACT.md` and `09-ui-reference.md`.

# City of Towers — Complete Build Specification

**Version:** 4.0
**Purpose:** A developer with no prior context must be able to build this game
exactly as specified, using only this document.

**Reference:** Heroes of Might and Magic III: Shadow of Death. This game
reproduces HoMM3's **play, roles, and experience** exactly, with **one change:
there is no combat.** Everything else — the adventure map, the hero, the towns,
the resources, the turn structure, the Obelisk, the Grail — follows HoMM3.

---

# PART 1 — THE GAME

## 1.1 What the player does

The player controls a **hero** on a **tile map**. Each turn (one day) the hero
has **movement points**. Moving across the map costs points depending on terrain.
Along the way the hero:

- collects **resource piles** (wood, ore, gold, mercury, sulfur, crystal, gems)
- **captures mines**, which then produce resources every day
- opens **treasure chests** for gold and experience
- picks up **artifacts**
- visits **shrines** to learn spells
- returns to **town** to spend resources on buildings and recruit creatures

The hero gains **experience** and **levels up**, improving primary stats and
secondary skills.

There is **no combat**. Wandering monsters on the map are impassable obstacles;
they never move and never fight.

## 1.2 The three players

The game is for **three players**, pre-seeded. There is no registration or login.

| Player | Faction | Town name | Starting hero |
| --- | --- | --- | --- |
| **Alice** | Castle | Alice's Keep | Christian |
| **Bob** | Rampart | Bob's Grove | Ryland |
| **Carol** | Necropolis | Carol's Crypt | Vidomina |

A player is chosen on the landing screen and remembered in `localStorage`
under the key `cityoftowers.player`.

## 1.3 Win condition

The **Obelisk** reveals 10 fragments of a riddle. When a player has seen all 10,
the **Grail's location** is revealed on the map. Walking a hero to that tile
awards the **Precious**. Holding the Precious allows building the **Grail
Structure**, which ends the game.

A player may also submit the riddle answer directly at the Obelisk; a correct
answer awards the Precious immediately.

---

# PART 2 — SCREEN LAYOUT

All screens render at a fixed logical size of **1280 × 800**, scaled uniformly
to fit the browser window and centred with black letterbox bars.

## 2.1 Design tokens

| Token | Value | Use |
| --- | --- | --- |
| `--stone-0` | `#0a0806` | page background |
| `--stone-1` | `#1a130c` | panel bottom |
| `--stone-2` | `#2a2016` | panel middle |
| `--stone-3` | `#3d2f20` | panel top |
| `--stone-4` | `#4f3d28` | raised surface |
| `--stone-5` | `#6a5236` | hover surface |
| `--gold-0` | `#6a4a18` | inner trim |
| `--gold-1` | `#a07a30` | mid trim |
| `--gold-2` | `#c8a050` | outer border |
| `--gold-3` | `#e8c878` | highlight |
| `--gold-4` | `#fff0b8` | brightest |
| `--text` | `#f0e0c0` | primary text |
| `--text-dim` | `#b8a078` | secondary text |
| `--text-mute` | `#7a6a50` | disabled text |
| `--accent` | `#ffd040` | selected / active |
| `--danger` | `#e05040` | errors |
| `--ok` | `#6ac060` | success |

Font: `Georgia, "Times New Roman", serif` throughout.

## 2.2 Panel style

Every panel is drawn as:
1. Vertical gradient `--stone-3` → `--stone-2` → `--stone-1`.
2. 2px border in `--gold-2`.
3. Inset shadow ring: 1px black, then 3px `--gold-0`, then 1px black.
4. Inner shadow `inset 0 2px 12px rgba(0,0,0,0.55)`.
5. Outer shadow `0 2px 6px rgba(0,0,0,0.7)`.
6. Border radius 3px.

## 2.3 Button style

| State | Background | Border | Text |
| --- | --- | --- | --- |
| Normal | gradient `#6a5236` → `#4a3820` → `#2e2214` | `--gold-2` | `--text` |
| Hover | same, `filter: brightness(1.25)` | `--gold-3` | `--text` |
| Active | same, `filter: brightness(0.8)` | `--gold-2` | `--text` |
| Disabled | gradient `#2a2016` → `#1a130c` | `#4a3a20` | `--text-mute` |

Buttons have `inset 0 1px 0 rgba(255,240,200,0.35)` for a bright top edge, and
`text-shadow: 0 1px 2px rgba(0,0,0,0.9)`.

---

## 2.4 S1 — Landing (choose player)

```
┌──────────────────────────────────────────────────────────────────────┐
│                                                                      │
│                    C I T Y   O F   T O W E R S                       │
│                    ─────────────────────────────                     │
│                                                                      │
│                        Choose your city                              │
│                                                                      │
│   ┌──────────────┐   ┌──────────────┐   ┌──────────────┐             │
│   │  [town art]  │   │  [town art]  │   │  [town art]  │             │
│   │    Alice     │   │     Bob      │   │    Carol     │             │
│   │    Castle    │   │   Rampart    │   │  Necropolis  │             │
│   │  [  Play  ]  │   │  [  Play  ]  │   │  [  Play  ]  │             │
│   └──────────────┘   └──────────────┘   └──────────────┘             │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Title | 0 | 120 | 1280 | 48 |
| Divider line | 490 | 178 | 300 | 1 |
| Subtitle "Choose your city" | 0 | 200 | 1280 | 24 |
| Card 1 (Alice) | 220 | 300 | 240 | 280 |
| Card 2 (Bob) | 520 | 300 | 240 | 280 |
| Card 3 (Carol) | 820 | 300 | 240 | 280 |

Card contents:
- Town sprite 160 × 160 at (40, 20) within the card.
- Player name, 20px bold, centred, y = 196.
- Faction name, 14px `--text-dim`, centred, y = 222.
- Play button 160 × 40 at (40, 240) within the card.

---

## 2.5 S2 — Adventure map (the main screen)

This is the screen the player spends most time on. It reproduces HoMM3's
adventure map layout.

```
┌──────────────────────────────────────────────────────────────────────┐
│ WOOD 120 │ ORE 88 │ MERC 4 │ SULF 3 │ CRYS 6 │ GEMS 2 │ GOLD 4200    │ 48px
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│                                                                      │
│                    ADVENTURE MAP (tiles)                             │
│                                                                      │
│                                                                      │
│                                                                      │
│                                                                      │
├──────────────────────────────────────────────────────────────────────┤
│ ┌────────┐ ┌──┬──┬──┬──┬──┬──┬──┐  [Hero][Spells]   ┌────────────┐   │
│ │portrait│ │  │  │  │  │  │  │  │  [Town][End Turn] │  MINIMAP   │   │
│ │        │ └──┴──┴──┴──┴──┴──┴──┘                   │            │   │
│ └────────┘  Alice · Lvl 3 · Move 1200/1500          └────────────┘   │
│  Day 3, Week 1, Month 1                                              │
└──────────────────────────────────────────────────────────────────────┘
```

### 2.5.1 Resource bar (top)

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Bar | 0 | 0 | 1280 | 48 |

Seven chips, each 150px wide, starting at x = 20:

| Chip | x | Icon | Label | Colour |
| --- | --- | --- | --- | --- |
| Wood | 20 | log | WOOD | `#8a5a2b` |
| Ore | 170 | rock | ORE | `#7a7a84` |
| Mercury | 320 | droplet | MERC | `#c0c0d0` |
| Sulfur | 470 | crystal | SULF | `#e8d060` |
| Crystal | 620 | shard | CRYS | `#60c0e0` |
| Gems | 770 | gem | GEMS | `#40c060` |
| Gold | 920 | coin | GOLD | `#e8c040` |

Chip layout: icon 24 × 24 at (x+8, 12); label 11px `--text-dim` at (x+40, 10);
value 15px bold `--text` at (x+40, 26).

### 2.5.2 Adventure map viewport

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Map viewport | 0 | 48 | 1280 | 600 |

- Tile size: **32 × 32** logical pixels.
- Visible area: 40 × 18.75 tiles.
- The camera follows the selected hero, clamped to the map bounds.
- Scrolling: move the cursor to a screen edge, or drag the minimap box.

### 2.5.3 Status window (bottom-left)

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Status panel | 0 | 648 | 420 | 152 |
| Hero portrait | 12 | 660 | 96 | 96 |
| Army slots (7) | 120 | 660 | 7 × 40 | 40 |
| Hero name | 12 | 764 | 200 | 20 |
| Level | 12 | 784 | 100 | 16 |
| Movement | 120 | 764 | 200 | 16 |
| Mana | 120 | 784 | 200 | 16 |

Army slot: 40 × 40, gap 2px, starting at x = 120. Each shows the creature
sprite (32 × 32) and the count in 10px white at the bottom.

### 2.5.4 Minimap (bottom-right)

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Minimap panel | 1060 | 648 | 220 | 152 |
| Minimap view | 1068 | 656 | 204 | 136 |

- Each map tile is drawn as a 1–3px square in its terrain colour.
- Towns: 4 × 4 square in the owner's colour.
- Heroes: 3 × 3 white square.
- The camera viewport is a 1px red rectangle.
- Clicking the minimap centres the camera there.

### 2.5.5 Control buttons

| Button | x | y | w | h |
| --- | --- | --- | --- | --- |
| Hero | 440 | 660 | 100 | 36 |
| Spells | 548 | 660 | 100 | 36 |
| Town | 440 | 704 | 100 | 36 |
| End Turn | 548 | 704 | 100 | 36 |
| Obelisk | 440 | 748 | 100 | 36 |
| Menu | 548 | 748 | 100 | 36 |

### 2.5.6 Date

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Date text | 660 | 764 | 380 | 20 |

Format: `Day 3, Week 1, Month 1`.

### 2.5.7 Rollover bar

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Rollover bar | 0 | 624 | 1280 | 24 |

Shows the name of whatever the cursor is over: terrain name, object name,
creature name, or town name.

### 2.5.8 Map rendering

Each tile is drawn in this order:
1. Terrain base colour with a noise texture.
2. Terrain detail (grass tufts, sand ripples, snow flecks, rock speckles).
3. Object sprite, if any.
4. Fog overlay:
   - **Unexplored**: fill `#07070a` (opaque black).
   - **Explored, not visible**: fill `rgba(0,0,0,0.5)`.
   - **Visible**: no overlay.

Terrain colours:

| Terrain | Colour | Move cost |
| --- | --- | --- |
| Dirt | `#8a6a44` | 100 |
| Sand | `#c8b070` | 100 |
| Grass | `#5a8a3a` | 100 |
| Snow | `#c8d0d8` | 150 |
| Swamp | `#4a6a4a` | 175 |
| Rough | `#7a6a50` | 125 |
| Subterranean | `#4a3a5a` | 100 |
| Lava | `#6a2a1a` | 100 |
| Water | `#2a4a8a` | impassable |
| Rock | `#3a3a3a` | impassable |

### 2.5.9 Hero rendering

- Hero sprite 32 × 32, drawn at the tile's top-left.
- A flag in the player's colour at the tile's top-right.
- The selected hero has a 2px `--accent` outline.
- A path preview (dashed white line) is drawn when the player right-clicks a
  destination.

### 2.5.10 Movement

- Left-click a tile: the hero moves along the shortest path, one tile per
  60ms, stopping when movement runs out or an object is reached.
- Right-click a tile: preview the path without moving.
- Movement cost per tile = the terrain's move cost.
- Diagonal moves cost the same as orthogonal.
- A hero cannot enter impassable terrain.

---

## 2.6 S3 — Town screen

Reached by clicking a town on the map, or the Town button.

```
┌──────────────────────────────────────────────────────────────────────┐
│  Alice's Keep            Castle            Day 3, Week 1, Month 1    │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│   ┌────────────────────────────────────────────────────────────┐     │
│   │                                                            │     │
│   │              TOWN VIEW (painted, clickable)                │     │
│   │                                                            │     │
│   │   [village hall]  [dwelling 1]  [dwelling 2]  [tavern]     │     │
│   │                                                            │     │
│   └────────────────────────────────────────────────────────────┘     │
│                                                                      │
├──────────────────────────────────────────────────────────────────────┤
│  [Build] [Recruit] [Market] [Mage Guild] [Tavern] [Garrison] [Exit]  │
└──────────────────────────────────────────────────────────────────────┘
```

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Header bar | 0 | 0 | 1280 | 48 |
| Town name | 24 | 14 | 400 | 20 |
| Faction | 440 | 14 | 200 | 20 |
| Date | 900 | 14 | 360 | 20 |
| Town view | 40 | 64 | 1200 | 560 |
| Tab bar | 0 | 640 | 1280 | 48 |
| Tabs | 20 + i×150 | 648 | 140 | 32 |

### 2.6.1 Town view

- A painted background in the faction's palette.
- Built buildings are drawn as 120 × 120 sprites on a 6 × 3 grid of
  200 × 180 cells, starting at (0, 0) of the view.
- Unbuilt buildings are not drawn.
- Hovering a building shows its name in the rollover bar.
- Clicking a building opens its detail popover.

### 2.6.2 Build tab

A modal 900 × 600 listing every building in the faction's tree, grouped by
category, with cost, prerequisites, and a Build button. One building per day.

### 2.6.3 Recruit tab

Seven rows, one per tier. Each shows the creature sprite, name, available
count, cost, and buttons `[1] [10] [Max]`. Recruiting adds to the hero standing
in the town, or to the garrison if no hero is present.

### 2.6.4 Market tab

Trade resources. Exchange rate: selling 1 unit gives 250 gold; buying 1 unit
costs 500 gold.

### 2.6.5 Mage Guild tab

Lists the spells in the guild. A hero in town may learn them.

### 2.6.6 Tavern tab

Lists up to two heroes for hire at 2000 gold each.

### 2.6.7 Garrison tab

Seven slots. Clicking a garrison stack moves it to the hero; clicking a hero
stack moves it to the garrison.

---

## 2.7 S4 — Hero screen

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Portrait | 40 | 64 | 160 | 160 |
| Name | 220 | 64 | 400 | 28 |
| Level / XP | 220 | 100 | 400 | 20 |
| Attack | 220 | 140 | 200 | 24 |
| Defense | 220 | 172 | 200 | 24 |
| Power | 220 | 204 | 200 | 24 |
| Knowledge | 220 | 236 | 200 | 24 |
| Mana | 220 | 276 | 200 | 20 |
| Movement | 220 | 300 | 200 | 20 |
| Skills panel | 40 | 360 | 400 | 300 |
| Army panel | 460 | 360 | 500 | 200 |
| Artifacts panel | 40 | 680 | 920 | 100 |

---

## 2.8 S5 — Magic Tower (blog)

Unchanged from the earlier spec: a spellbook list on the left, a reader on the
right, a guestbook at the bottom. Post slots by tower level: 3 / 6 / 10 / 15 / 25.

## 2.9 S6 — Obelisk

Reached by walking a hero to the Obelisk tile, or from the Obelisk button.

| Element | x | y | w | h |
| --- | --- | --- | --- | --- |
| Obelisk panel | 40 | 64 | 1200 | 200 |
| Obelisk sprite | 560 | 80 | 160 | 160 |
| Progress text | 0 | 200 | 1280 | 20 |
| Visit button | 560 | 224 | 160 | 32 |
| Fragments panel | 40 | 280 | 1200 | 200 |
| Solve panel | 40 | 496 | 1200 | 100 |
| Messages panel | 40 | 612 | 1200 | 160 |

## 2.10 S7 — Game over

Title, winner, the answer, final standings, and a Return button.

---

# PART 3 — GAME RULES

## 3.1 Resources

| Resource | Mine yield/day | Starting amount |
| --- | --- | --- |
| Wood | +2 | 20 |
| Ore | +2 | 20 |
| Mercury | +1 | 5 |
| Sulfur | +1 | 5 |
| Crystal | +1 | 5 |
| Gems | +1 | 5 |
| Gold | +1000 | 20000 |

## 3.2 Time

| Unit | Length |
| --- | --- |
| Day | 1 turn (default 5 minutes) |
| Week | 7 days |
| Month | 4 weeks |

- **Every day:** mines yield, town halls yield gold, hero movement resets.
- **Every week (day 1):** dwellings grow.

## 3.3 Hero

| Attribute | Starting value |
| --- | --- |
| Level | 1 |
| Experience | 0 |
| Attack / Defense / Power / Knowledge | 2/2/1/1 (might) or 1/1/2/2 (magic) |
| Max mana | 10 + Knowledge × 10 |
| Movement | 1500 (+150 × Logistics level) |
| Army | 25 × tier 1, 8 × tier 2 |
| Skills | two at Basic |

### Experience thresholds
1000, 2000, 3200, 4600, 6200, 8000, 10000, 12200, 14700, 17500, 20600, 24320,
28784, 34000, 40000, 47000, 55000, 64000, 74000, 85000, ...

Each level grants **one** of: +1 to a random primary stat, or +1 level to a
random secondary skill (max Expert).

## 3.4 Creatures

126 creatures: 9 factions × 7 tiers × (base + upgrade). Statistics are the
HoMM3 Shadow of Death values. See `server/CityOfTowers/Domain/Creatures.cs`.

| Faction | Tier 1 | Tier 2 | Tier 3 | Tier 4 | Tier 5 | Tier 6 | Tier 7 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Castle | Pikeman | Archer | Griffin | Swordsman | Monk | Cavalier | Angel |
| Rampart | Centaur | Dwarf | Wood Elf | Pegasus | Dendroid | Unicorn | Green Dragon |
| Tower | Gremlin | Stone Gargoyle | Stone Golem | Mage | Genie | Naga | Giant |
| Inferno | Imp | Gog | Hell Hound | Demon | Pit Fiend | Efreeti | Devil |
| Necropolis | Skeleton | Walking Dead | Wight | Vampire | Lich | Black Knight | Bone Dragon |
| Dungeon | Troglodyte | Harpy | Beholder | Medusa | Minotaur | Manticore | Red Dragon |
| Stronghold | Goblin | Wolf Rider | Orc | Ogre | Roc | Cyclops | Behemoth |
| Fortress | Gnoll | Lizardman | Serpent Fly | Basilisk | Gorgon | Wyvern | Hydra |
| Conflux | Pixie | Air Elemental | Water Elemental | Fire Elemental | Earth Elemental | Psychic Elemental | Firebird |

## 3.5 Town buildings

| Category | Buildings |
| --- | --- |
| Hall | Village Hall → Town Hall → City Hall → Capitol |
| Fortification | Fort → Citadel → Castle |
| Dwelling | 7 tiers, each with an upgrade |
| Mage Guild | Levels 1–5 |
| Civic | Marketplace, Tavern, Blacksmith, Resource Silo |
| Grail | Grail Structure |

Gold income: Village Hall 500, Town Hall 1000, City Hall 2000, Capitol 4000.
Growth multiplier: Citadel ×1.5, Castle ×2.0.

## 3.6 The Obelisk and Grail

- One Obelisk on the map.
- Each visit reveals one of 10 riddle fragments.
- After 10 fragments, the Grail's location is revealed.
- Walking a hero to the Grail tile awards the Precious.
- A correct riddle answer at the Obelisk also awards the Precious.

---

# PART 4 — IMPLEMENTATION

## 4.1 Stack

| Layer | Technology |
| --- | --- |
| Server | ASP.NET Core (.NET 10), C# |
| Database | SQLite via EF Core |
| Client | React 18, TypeScript, Vite 5 |
| Routing | react-router-dom 6 |
| HTTP | axios |
| Real-time | SignalR |
| Styling | plain CSS with the tokens in §2.1 |

## 4.2 Identity

No authentication. Every request carries an `X-Player-Id` header with one of
`alice`, `bob`, `carol`. The server resolves it to a seeded player.

## 4.3 API

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/players` | the three seeded players |
| POST | `/api/games` | create a game |
| GET | `/api/games/{id}` | game state |
| GET | `/api/games/{id}/map` | tiles visible to the caller |
| GET | `/api/games/{id}/heroes` | the caller's heroes |
| POST | `/api/heroes/{id}/move` | `{path:[{x,y}]}` |
| GET | `/api/towns/{id}` | town screen data |
| POST | `/api/towns/{id}/build` | `{buildingKey}` |
| POST | `/api/towns/{id}/recruit` | `{tier, count}` |
| POST | `/api/towns/{id}/trade` | `{from, to, amount}` |
| POST | `/api/towns/{id}/hire` | `{heroKey}` |
| GET/POST | `/api/towns/{id}/posts` | blog |
| GET/POST | `/api/towns/{id}/wall` | wall |
| GET/POST | `/api/games/{id}/obelisk` | obelisk |
| POST | `/api/games/{id}/obelisk/visit` | reveal a fragment |
| POST | `/api/games/{id}/obelisk/solve` | submit an answer |

## 4.4 Real-time events

`turnAdvanced`, `wallMessage`, `comment`, `towerMessage`, `obeliskMessage`,
`gameFinished`.

---

# PART 5 — ACCEPTANCE CRITERIA

| # | Criterion |
| --- | --- |
| AC-1 | The landing screen shows exactly three players: Alice, Bob, Carol. |
| AC-2 | Clicking Play enters the game as that player. |
| AC-3 | The chosen player persists across a page reload. |
| AC-4 | Requests without a valid `X-Player-Id` are rejected. |
| AC-5 | Each player has a town and a hero on the map. |
| AC-6 | The adventure map renders terrain, objects, and fog of war. |
| AC-7 | A hero moves along a path, spending movement by terrain cost. |
| AC-8 | A hero cannot enter impassable terrain or move without points. |
| AC-9 | Fog reveals tiles within the hero's scouting radius. |
| AC-10 | Entering a resource pile collects it and empties the tile. |
| AC-11 | Entering a neutral mine captures it; it yields daily. |
| AC-12 | Entering a treasure chest grants gold and experience. |
| AC-13 | A hero levels up at the experience threshold. |
| AC-14 | Dwellings accumulate creatures weekly. |
| AC-15 | Recruiting adds creatures to the hero's army. |
| AC-16 | The town screen shows only that faction's buildings. |
| AC-17 | The minimap shows terrain, towns, heroes, and the viewport. |
| AC-18 | Clicking the minimap moves the camera. |
| AC-19 | The resource bar shows all seven resources. |
| AC-20 | The status window shows the hero portrait, army, and movement. |
| AC-21 | Visiting the Obelisk reveals one fragment. |
| AC-22 | After 10 fragments the Grail location is revealed. |
| AC-23 | A correct riddle answer awards the Precious and ends the game. |
| AC-24 | A wrong answer is rejected without ending the game. |
| AC-25 | Game state survives a server restart. |
| AC-26 | Two browsers see each other's wall messages in real time. |