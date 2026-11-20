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

## AI-native browser: canonical shell API

The separate `ai-browser` application shares one canonical API catalog between the
sidebar, built-in agents and external MCP clients. Public operations use `shell_`
names; private native host operations use `wv_`. This is a breaking rewrite:
previous method names, dotted commands and browser-tool aliases are not accepted.
Grouped APIs use `operation`, for example `shell_tab` with `operation: "list"`.
Built-in and external agents use the same MCP endpoint, not private browser RPCs.

While a profile is running, its `shell-api.json` contains the loopback endpoint
`url` and external bearer `token`. Treat this file as a credential: do not commit,
publish or log it. The built-in runtime has a separate, unexported credential.
Each external MCP session has its own caller identity, observed documents and
owned tabs; sharing an endpoint does not share that state.

Send each JSON object below as a separate HTTP POST to the descriptor's URL with
`Authorization: Bearer <TOKEN>`, `Content-Type: application/json` and
`Accept: application/json, text/event-stream`. Initialize first; copy the returned
`Mcp-Session-Id: <SESSION>` header into every subsequent request, including the
initialized notification (which returns HTTP 202):

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"external-agent","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"shell_tab","arguments":{"operation":"list"}}}
```

Use the discovered schemas for other calls. Canvas cells are independent live
pages layered beneath shell/sidebar overlays, not text embedded in a container.
A canvas container is not a page: page APIs default to its focused cell; use a
concrete page `tab_id`, or `shell_canvas` with `operation: "cell_open"` and
`cell` (or `row`/`col`), to target another cell explicitly.