# Heroes of Might and Magic III — Shadow of Death (Web Reimplementation)

A browser reimplementation of the Heroes III: Shadow of Death experience, built with
plain HTML/CSS/JavaScript and a Canvas renderer. **No build step, no dependencies** —
open `index.html` in a browser and play.

All artwork is generated procedurally at runtime from code. No original game assets
(sprites, sounds, map data) are used or redistributed.

## Running

Open `index.html` directly in Chrome, Edge, or Firefox. That's it.

The canvas is 800×600, matching the original game's resolution, so every UI element
sits at the same pixel position as in Heroes III.

## Loading real .h3m maps

The main menu has a **Load .h3m Map** button. It reads Heroes III map files
(Shadow of Death `0x0e` and Armageddon's Blade `0x15`) and imports the terrain,
towns, monsters, resources, mines, artifacts, and shrines.

**Status: experimental.** The parser is written from the published H3M specification
and verified with a synthetic round-trip test (`test/h3m.js`, 33 checks). It has
**not** been verified against a real map file, because none was available. If a real
map fails to load, the error message names the byte offset and the object class that
could not be read.

What is imported: terrain, towns, monsters, resources, mines, artifacts, shrines.
What is not: roads, rivers, hero objects, quest guards, events, and the original
sprite artwork (which lives in the game's `.lod` archives and is not redistributed).

## Controls

| Action | Input |
| --- | --- |
| Select hero / move | Left click |
| Preview path | Right click |
| Scroll map | Mouse wheel, arrow keys |
| End turn | `E` or the End Turn button |
| Hero screen | `H` |
| Town screen | `T` |
| Close dialog / leave screen | `Esc` |

## What's implemented

**Adventure map**
- 72×72 procedurally generated map with 9 terrain types, lakes, mountain ranges, forests
- A* pathfinding with terrain movement costs and diagonal corner-cutting rules
- Fog of war: unexplored (black), explored-but-not-visible (dimmed), visible
- Objects: towns, mines, resource piles, treasure chests, artifacts, shrines, wandering
  monsters, decorative landmarks
- Hero movement with per-turn movement points, terrain penalties, and path preview

**Towns**
- 10 factions (Castle, Rampart, Tower, Inferno, Necropolis, Dungeon, Stronghold,
  Fortress, Conflux) with their own creature lineups and palettes
- Building tree: village hall → town hall → city hall → capitol, fort → citadel → castle,
  mage guild levels 1–5, 7 creature dwellings plus upgrades, tavern, marketplace,
  blacksmith, grail
- Weekly creature growth, daily gold income, one building per day
- Recruit screen, marketplace (buy/sell), mage guild (learn spells), tavern (hire heroes),
  garrison with two-way troop exchange

**Heroes**
- 18 heroes with might/magic archetypes, primary stats, secondary skills
- Experience and leveling (stat or skill gain per level)
- Army of 7 stacks, artifacts with stat bonuses, spellbook
- Movement points modified by Logistics and artifacts

**Combat**
- Hex battlefield (15×11) with obstacles
- Speed-based turn order, initiative bar
- Melee, ranged, retaliation, double/triple strike, no-retaliation, lifesteal,
  fire shield, poison, paralysis, petrification, death stare, curse, blind
- Damage formula with attack/defense differential, Archery skill, ranged penalty
- 30+ spells across 4 schools with school-level scaling
- AI opponent that targets the best stack and closes to melee
- Hero-vs-hero battles with artifact capture and experience

**Game flow**
- Multiple players (1–4), human vs. AI
- Day/week/month cycle, weekly growth, daily income, mine income
- AI builds, recruits, trades resources at the market, captures mines, and hunts
  enemy heroes when it has the advantage
- Win/lose detection, save/load to `localStorage`

## Project layout

```
index.html          entry point, script order
css/style.css       page shell and canvas scaling
js/util.js          RNG, math, colour helpers
js/data.js          creatures, factions, buildings, heroes, skills, spells, artifacts
js/art.js           base procedural art (tiles, panels, icons, cursors)
js/art2.js          high-detail art (creatures, backdrops, plaques, map objects)
js/chrome.js        HoMM3 UI chrome (resource bar, status window, minimap frame)
js/h3m.js           .h3m map file parser
js/mapgen.js        map generation, A* pathfinding, reachability
js/combat.js        hex grid, battle state, damage, spells, combat AI
js/town.js          town buildings, growth, recruitment, income
js/hero.js          hero stats, army, experience, movement
js/game.js          game state, turn flow, map objects, AI player
js/ui.js            screens, rendering, input
js/main.js          bootstrap
test/               headless Chrome test harnesses (dev only)
```

## Tests

The `test/` directory contains headless-Chrome harnesses driven over the DevTools
protocol. They need `ws` installed (`npm install ws`) and Chrome at the default
Windows path.

```
node test/e2e.js        # full loop: menu → map → town → hero → combat → save/load
node test/sim.js        # 30-turn AI simulation
node test/garrison.js   # troop exchange between hero and garrison
node test/visual.js     # per-screen colour/edge density measurement
node test/hud.js        # HUD element positions and minimap click
node test/h3m.js        # .h3m parser round-trip (synthetic map)
node test/h3m-browser.js # .h3m import in the real page
node test/bench.js      # frame-time benchmark
```

## Known limitations

- Combat is single-battlefield; no siege walls, moats, or war machines yet
- AI is heuristic, not a full planner — it does not use spells in combat
- No sound or music
- Map generation is random rather than hand-authored scenarios
- Art is stylised and procedural, not a pixel-for-pixel reproduction of the original