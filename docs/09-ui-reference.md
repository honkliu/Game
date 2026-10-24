# UI Reference — HoMM3 Composition

**Status:** Authoritative. Supersedes `03-ui-spec.md` and §2 of `00-BUILD-SPEC.md`.
**Reference:** the published HoMM3 manual's screen diagrams.

---

## 0. The correction

Earlier drafts specified a **top resource bar** and a **bottom-right minimap**.
That was a redesign. HoMM3's actual composition is:

- **Adventure map:** the map fills the left; a **right-hand sidebar** holds the
  minimap and control buttons; the **resource bar runs along the bottom**.
- **Town screen:** an **integrated painted scene**, not a grid of tiles.

This document specifies the correct composition.

---

## 0.1 Art source: H3 data mode vs built-in art

The client has two art modes, chosen automatically at startup via `GET /h3/status`.

- **H3 data mode.** The player supplies their own copy of the original game data:
  `H3sprite.lod`, `H3bitmap.lod`, or the files extracted from them.
  - Default folder: `%LOCALAPPDATA%\CityOfTowers\h3data`. Override it with the config
    key `H3:DataPath`.
  - The server (`Services/H3Assets.cs`) decodes DEF/PCX at runtime and serves them under
    `/h3/*`, **to loopback clients only**. Nothing is bundled or redistributed.
  - The map, town and hero screens then use the original sprites, using the original
    window geometry:
    - Map: 594×546 viewport, terrain transitions and shroud.
    - Town: 800×374 scene with `townscrn` panel.
    - Hero: `HeroScr4`.
  - Data mappings live in `client/src/h3/catalog.ts` and `client/src/h3/town.ts`.
- **Built-in art.** If no data is present, or the request is not from loopback, the
  procedurally drawn art is used. Screen positions are the same in both modes.

Known gaps in H3 data mode:
- No player-colour recolouring of flags.
- Rampart borrows Castle's background when `TBRMBACK` is missing (as in the demo).
- Artifacts use generic icons.
- No roads or rivers.

## 1. Global

- Logical resolution: **800 × 600** (HoMM3's native size).
- Scaled uniformly to fit the window; centred; black letterbox.
- All coordinates below are logical pixels, origin top-left.

### 1.1 Design tokens

| Token | Value |
| --- | --- |
| `--stone-0` | `#0a0806` |
| `--stone-1` | `#1a130c` |
| `--stone-2` | `#2a2016` |
| `--stone-3` | `#3d2f20` |
| `--gold-0` | `#6a4a18` |
| `--gold-1` | `#a07a30` |
| `--gold-2` | `#c8a050` |
| `--gold-3` | `#e8c878` |
| `--text` | `#f0e0c0` |
| `--text-dim` | `#b8a078` |
| `--text-mute` | `#7a6a50` |
| `--accent` | `#ffd040` |

Font: `Georgia, "Times New Roman", serif`.

---

## 2. Adventure map screen

```
┌───────────────────────────────────────────────┬──────────────────┐
│                                               │  ┌────────────┐  │
│                                               │  │  MINIMAP   │  │
│                                               │  │  144x108   │  │
│                                               │  └────────────┘  │
│                                               │                  │
│              ADVENTURE MAP                    │  [Hero] [Spells] │
│                640 x 480                      │  [Town] [End]    │
│                                               │  [Obelisk][Menu] │
│                                               │                  │
│                                               │                  │
├───────────────────────────────────────────────┴──────────────────┤
│ rollover bar (640 x 24)                                          │
├──────────────────────────────────────────────────────────────────┤
│ [portrait] [army slots]  name / level / move / mana              │
├──────────────────────────────────────────────────────────────────┤
│ WOOD 120 │ ORE 88 │ MERC 4 │ SULF 3 │ CRYS 6 │ GEMS 2 │ GOLD 4200 │
└──────────────────────────────────────────────────────────────────┘
```

### 2.1 Regions

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Adventure map view | 0 | 0 | 640 | 480 |
| Right sidebar | 640 | 0 | 160 | 600 |
| Minimap frame | 648 | 8 | 144 | 108 |
| Minimap view | 652 | 12 | 136 | 100 |
| Control buttons | 648 | 124 | 144 | 200 |
| Rollover bar | 0 | 480 | 640 | 24 |
| Status window | 0 | 504 | 640 | 72 |
| Resource bar | 0 | 576 | 640 | 24 |

### 2.2 Adventure map view

- Tile size **32 × 32**.
- Visible area: **20 × 15 tiles**.
- Camera follows the selected hero, clamped to map bounds.
- Scrolling: cursor at a screen edge, or drag the minimap box.

### 2.3 Minimap

- 136 × 100 view inside a 144 × 108 stone frame.
- Each map tile is drawn as a 1–2px square in its terrain colour.
- Towns: 4 × 4 in the owner's colour.
- Heroes: 3 × 3 white.
- Camera viewport: 1px red rectangle.
- Clicking centres the camera.

### 2.4 Control buttons

Six buttons, 68 × 28, two per row, starting at (648, 124):

| Button | x | y |
| --- | --- | --- |
| Hero | 648 | 124 |
| Spells | 724 | 124 |
| Town | 648 | 160 |
| End Turn | 724 | 160 |
| Obelisk | 648 | 196 |
| Menu | 724 | 196 |

### 2.5 Rollover bar

- 640 × 24 at (0, 480).
- Shows the name of whatever the cursor is over.
- Text 11px `--text-dim` at (8, 4).

### 2.6 Status window

- 640 × 72 at (0, 504).
- Hero portrait 56 × 56 at (8, 8).
- Seven army slots, 36 × 36, starting at (72, 8), gap 2px.
- Hero name 13px bold at (72, 50).
- Level 11px at (200, 50).
- Movement 11px at (280, 50).
- Mana 11px at (400, 50).

### 2.7 Resource bar

- 640 × 24 at (0, 576).
- Seven chips, each 88px wide, starting at x = 4.
- Chip: icon 16 × 16 at (x+2, 4); value 12px bold at (x+22, 6).
- Date at (560, 580), 11px.

### 2.8 Map rendering order

1. Terrain base colour.
2. Noise texture (4 × 4 blocks, ±10% brightness).
3. Terrain detail (grass tufts, water ripples, snow flecks).
4. Object sprite.
5. Fog overlay: unexplored `#07070a`; explored `rgba(0,0,0,0.5)`; visible none.

### 2.9 Hero rendering

- 32 × 32 sprite at the tile origin.
- Faction-coloured flag at the tile's top-right.
- Selected hero: 2px `--accent` outline.
- Path preview: dashed white line.

### 2.10 Interaction

| Input | Effect |
| --- | --- |
| Left-click a tile | Move along the shortest path |
| Right-click a tile | Preview the path |
| Left-click a town | Enter the town screen |
| Left-click the minimap | Centre the camera |
| Cursor at screen edge | Scroll |
| Mouse wheel | Scroll |

---

## 3. Town screen

```
┌──────────────────────────────────────────────────────────────────┐
│                                                                  │
│                                                                  │
│              PAINTED TOWN SCENE  (800 x 480)                     │
│                                                                  │
│     [hall]      [dwelling]     [tower]      [tavern]             │
│                                                                  │
│                                                                  │
├──────────────────────────────────────────────────────────────────┤
│  [Build] [Recruit] [Market] [Guild] [Tavern] [Garrison] [Exit]   │
└──────────────────────────────────────────────────────────────────┘
```

### 3.1 Regions

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Painted scene | 0 | 0 | 800 | 480 |
| Tab bar | 0 | 480 | 800 | 40 |
| Tabs | 8 + i×112 | 486 | 104 | 28 |

### 3.2 The painted scene

**This is not a grid.** The scene is a single painted backdrop in the faction's
palette, with buildings placed at **fixed positions** defined per faction.

Each faction has a layout table:

```
castle:
  villageHall      (120, 300)  96x96
  townHall         (120, 300)  96x96   (replaces villageHall)
  dwelling1        (240, 320)  96x96
  dwelling2        (340, 300)  96x96
  ...
```

- Buildings are drawn at their fixed position, not in a grid.
- Unbuilt buildings are not drawn.
- Hovering a building shows its name in a tooltip.
- Clicking a building opens its detail popover.

### 3.3 Tab bar

Seven tabs, 104 × 28, starting at x = 8:

`Build` · `Recruit` · `Market` · `Guild` · `Tavern` · `Garrison` · `Exit`

Active tab: `--gold-3` text, raised background.

### 3.4 Build panel

A modal 700 × 500 listing the faction's building tree, grouped by category.
Each row: state marker, name, cost. Selecting a row shows its description and a
Build button.

### 3.5 Recruit panel

Seven rows, one per tier. Each: creature sprite, name, available count, cost,
and `[1] [10] [Max]` buttons.

### 3.6 Market / Guild / Tavern / Garrison

As specified in `00-BUILD-SPEC.md` §2.6.4–2.6.7, restyled to 800 × 600.

---

## 4. Hero screen

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Portrait | 24 | 24 | 120 | 120 |
| Name | 160 | 24 | 300 | 24 |
| Level / XP | 160 | 52 | 300 | 18 |
| Attack | 160 | 84 | 140 | 22 |
| Defense | 160 | 110 | 140 | 22 |
| Power | 160 | 136 | 140 | 22 |
| Knowledge | 160 | 162 | 140 | 22 |
| Mana | 160 | 194 | 200 | 18 |
| Movement | 160 | 214 | 200 | 18 |
| Skills panel | 24 | 260 | 300 | 240 |
| Army panel | 340 | 260 | 420 | 160 |
| Artifacts panel | 24 | 512 | 736 | 64 |

---

## 5. Obelisk screen

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Obelisk sprite | 340 | 24 | 120 | 120 |
| Progress text | 0 | 150 | 800 | 18 |
| Visit button | 320 | 172 | 160 | 28 |
| Fragment list | 24 | 210 | 752 | 150 |
| Solve panel | 24 | 370 | 752 | 80 |
| Messages panel | 24 | 460 | 752 | 120 |

---

## 6. Cursors

| Cursor | When |
| --- | --- |
| Arrow | default |
| Green arrow | over a passable tile with movement remaining |
| Red cross | over impassable terrain |
| Gold arrow | over a town or an object you can enter |

---

## 7. Animation

| Action | Duration |
| --- | --- |
| Hero step | 60 ms per tile |
| Panel open | 120 ms fade |
| Toast | 150 ms in, 2500 ms hold, 300 ms out |
| Turn banner | 400 ms |

---

## 8. Conformance

A build conforms when:

1. The adventure map occupies 640 × 480 with a 160px right sidebar.
2. The resource bar is at the bottom, 640 × 24.
3. The minimap is in the sidebar, 144 × 108.
4. The town screen is a painted scene with fixed building positions.
5. Every element in §2–§5 appears at the stated coordinates.
6. Cursors follow §6.