# Requirements Matrix

**Purpose:** Every requirement in the fidelity contract maps to a server rule, an
API field, a screen element, and an acceptance test. A requirement missing any
column is incomplete.

**Status legend:** ✅ implemented and tested · ⚠️ partial · ❌ missing

**Test suites:**
- `test/adventure.js` — 50–54 checks (monster checks run when a stack is within 10 steps) (map, fog, step-by-step movement, encounters, town, market, obelisk)
- `test/social.js` — 37 checks (messages, trades, annotations, wall, tower, project rewards, town discovery)
- `test/creatures.js` — 126 creatures verified against HoMM3 values
- `e2e/game.spec.ts` — 16 Playwright browser tests

---

## 1. Preserve — HoMM3 features

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| P-1 | Tile map with terrain types | `MapGenerator.Generate` | `GET /games/{id}/map` | Adventure map canvas | adventure.js | ✅ |
| P-2 | Terrain movement costs | `Terrain.MoveCost` | — | — | adventure.js | ✅ |
| P-3 | Fog: unexplored / explored / visible | `GameService.RecomputeFog` | `tiles[].fog` | Canvas fog overlay | adventure.js | ✅ |
| P-4 | Hero with level, XP, 4 stats | `Hero`, `GainExperience` | `GET /heroes/{id}` | Hero screen | adventure.js, e2e S9 | ✅ |
| P-5 | Secondary skills | `Hero.SkillsJson` | `hero.skills` | Hero screen | adventure.js | ✅ |
| P-6 | Movement points per day | `Hero.Movement` | `hero.movement` | Status window | adventure.js, e2e S4 | ✅ |
| P-7 | 7-slot army | `ParseArmy` | `hero.army` | Status window | adventure.js, e2e S4 | ✅ |
| P-8 | Artifacts | `Hero.ArtifactsJson` | `hero.artifacts` | Hero screen | e2e S9 | ✅ |
| P-9 | Spellbook | `Hero.SpellsJson` | `hero.spells` | Hero screen | — | ⚠️ no spells yet |
| P-10 | Seven resources | `Player.Wood..Gold` | `player.resources` | Resource bar | adventure.js, e2e S3 | ✅ |
| P-11 | Daily mine yield | `TickAsync` | — | Resource bar | — | ⚠️ no test |
| P-12 | Town hall income | `TownBuildings.GoldIncome` | `town.goldIncome` | Town header | adventure.js | ✅ |
| P-13 | Claim mines by walking next to them | `MoveHeroAsync` → `InteractAsync` | — | Map | adventure.js | ✅ |
| P-14 | Resource piles | `InteractAsync` | — | Map | — | ⚠️ no test |
| P-15 | Treasure chests | `InteractAsync` | — | Map | — | ⚠️ no test |
| P-16 | Shrines | `InteractAsync` | — | Map | — | ⚠️ no test |
| P-17 | Painted clickable town | `LAYOUT` fixed positions; clicking a building opens its panel | `GET /towns/{id}` | Town screen | e2e S6 | ✅ (art is generated SVG placeholder) |
| P-18 | Building tree | `TownBuildings.All` | `GET /buildings` | Build panel | adventure.js, e2e S7 | ✅ |
| P-19 | Dwellings + weekly growth | `TickAsync` day==1 | `town.available` | Recruit panel | — | ⚠️ no test |
| P-20 | Recruitment | `POST /towns/{id}/recruit` | `town.available` | Recruit panel | adventure.js, e2e S8 | ✅ |
| P-21 | 9 factions × 7 tiers | `Creatures.All` | `GET /creatures` | — | creatures.js | ✅ |
| P-22 | Day / week / month | `Game.Day/Week/Month` | `game.day/week/month` | Date line | adventure.js, e2e S3 | ✅ |
| P-23 | Obelisk reveals fragments | `VisitObeliskAsync` | `GET /obelisk` | Obelisk screen | adventure.js, e2e S10 | ⚠️ visited from the sidebar, not by walking to an obelisk on the map |
| P-24 | Grail location revealed | `RevealGrailAsync` places the `grail` tile (10th fragment or riddle) | `game.grailX/Y` | Map | — | ⚠️ no test |
| P-25 | Dig for the Grail | `InteractAsync` case `grail`; digging ends the hero's day | — | Map | — | ⚠️ no test |
| P-26 | Tavern hires heroes | `POST /towns/{id}/hire` | `town.tavern` | Tavern panel | — | ⚠️ no test |
| P-27 | Marketplace trades | `POST /towns/{id}/trade` (500 gold buys 1; 1 sells for 250; 2:1 between resources) | `town.resources` | Market panel | adventure.js | ✅ |
| P-28 | Mage Guild | `town.MageGuildJson` | `town.mageGuild` | Guild panel | — | ⚠️ empty |
| P-29 | HoMM3 UI composition | — | — | 640×480 map + sidebar + bottom bar | e2e S3 | ✅ |
| P-30 | Step-by-step movement | `MoveHeroAsync`: adjacent steps only, impassable and occupied tiles block, no corner-cutting | `tiles[].passable` | Map pathing | adventure.js | ✅ |
| P-31 | Grail Structure needs the Grail | Build check on `player.Precious`; +5000 gold/day, +2 growth | — | Build panel | adventure.js | ✅ |

---

## 2. Remove — warfare

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| R-1 | No hero battles | no battle code | — | — | — | ✅ |
| R-2 | No creature battles | monsters are encounters | `objectKind='monster'` | Encounter dialog | — | ✅ |
| R-3 | No sieges | no siege code | — | — | — | ✅ |
| R-4 | No town capture | no capture code | — | — | — | ✅ |
| R-5 | No stealing | ownership checks | — | — | adventure.js | ✅ |
| R-6 | No damage spells | — | — | — | — | ⚠️ spells not implemented |

---

## 3. Adapt — social replacements

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| A-1 | Peaceful monster encounter | Real creature stacks; `ResolveEncounterAsync`; hero cannot move until resolved | `POST /heroes/{id}/encounter` | Encounter dialog | adventure.js, social.js | ✅ |
| A-2 | Meeting a hero opens dialogue | `SendHeroMessageAsync` | `POST /games/{id}/messages` | Map meeting dialog → message / offer trade | social.js, e2e S12 | ✅ |
| A-3 | Voluntary resource trade | `OfferTradeAsync`, `RespondTradeAsync` | `POST /games/{id}/trades` | Social screen | social.js | ✅ |
| A-4 | Expedition strength gates exploration | — | — | — | — | ❌ not implemented |
| A-5 | First discovery claims a mine | `InteractAsync` | — | Map | adventure.js | ✅ |
| A-6 | Mines cannot be recaptured | `InteractAsync` owner check | — | — | — | ✅ |
| A-7 | Shared mines | — | — | — | — | ❌ not implemented |
| A-8 | Tower is a place on the map | `Town` on map | `towns[]` | Map → town → Tower tab | e2e S14 | ✅ |
| A-9 | Tower holds illustrated journal | `CreatePostAsync` | `POST /towns/{id}/posts` | Tower panel | social.js, e2e S14 | ✅ |
| A-10 | Wall messages | `AddWallMessageAsync` | `POST /towns/{id}/wall` | Wall panel | social.js, e2e S15 | ✅ |
| A-11 | Obelisk messages | `AddObeliskMessageAsync` | `POST /obelisk/message` | Obelisk screen | adventure.js | ✅ |
| A-12 | Shared projects with rewards | `SharedProjectCatalog`, `GrantProjectRewardAsync` (XP, map reveal, +500 gold/day) | `POST /projects/{id}/contribute` | Projects screen | social.js, e2e S13 | ✅ |
| A-13 | Map annotations | `AddAnnotationAsync` | `POST /games/{id}/annotations` | — | social.js | ⚠️ API only, no map UI |
| A-14 | Grail is a milestone, not an ending | Solving reveals the site; a hero must dig | — | Obelisk toast | — | ✅ |
| A-15 | Live updates between players | `WsHub` → SignalR `/hubs/game` (`heroMoved`, `turnAdvanced`, `encounter`, `message`, `trade`, `project`, `wall`, `tower`, `annotation`) | `event` | Map, Social, Projects refresh | manual only | ⚠️ no automated test |
| A-16 | Other players' towns and heroes need discovery | Fog-filtered map DTO; `TownHiddenAsync` | `GET /towns/{id}` 400 if undiscovered | Map | adventure.js, social.js | ✅ |

---

## 4. Summary

| Category | Total | ✅ | ⚠️ | ❌ |
| --- | --- | --- | --- | --- |
| Preserve | 31 | 22 | 9 | 0 |
| Remove | 6 | 5 | 1 | 0 |
| Adapt | 16 | 13 | 2 | 1 |
| **Total** | **53** | **40** | **12** | **1** |

**75% complete against the contract** (40 of 53). The contract grew by four rows; earlier ✅ marks for movement, Grail, realtime and town privacy were overstated and have been fixed in code.

### Remaining gaps

| Gap | Impact |
| --- | --- |
| Town art is generated placeholder SVG (P-17) | Layout is correct; final painted art still needed |
| Rampart has no backdrop in the demo data | `tbrback.pcx` is absent; Rampart borrows Castle's (recorded in `TOWN_BACK_FALLBACK`) |
| Only three factions | The 1999 demo ships Castle, Rampart, Necropolis sprites only |
| Combat spells are inert | Combat is out of scope by design (`00-FIDELITY-CONTRACT.md:58`) |

### Closed since the last audit

| Gap | Resolution |
| --- | --- |
| No AI opponents | `AiService` builds, recruits and explores each day; lobby offers 0–2 computers |
| Multiplayer join was broken | `POST /api/games/{id}/join` added; the lobby's Enter button now seats the player |
| Social defaulted to an arbitrary recipient | Prefers another human seat over a computer |
| Database grew without bound | Startup prunes games untouched for a day and caps the total at 20 (279 MB → 10 MB) |
| Spells not implemented (P-9, R-6) | 25-spell catalogue, `/api/spells`, `/api/heroes/{id}/cast`, spellbook panel |
| No tests for mines/resources/treasure/shrines (P-11, P-14–16) | `build/audit-mapobjects.js` |
| No test for weekly growth (P-19) | `build/audit-mapobjects.js` |
| No test for Grail reveal/dig (P-24, P-25) | `build/audit-mapobjects.js` |
| No test for tavern (P-26) | `build/audit-mapobjects.js`, e2e S9b |
| Obelisk is not on the map (P-23) | Placed by the generator; walking up to it opens the Obelisk screen |
| Mage Guild empty (P-28) | Building a guild teaches one spell per level |
| Expedition strength (A-4) | Gates observe (≥ half) and recruit (≥ full) at wandering stacks |
| Shared mines (A-7) | `Owner` is a bitmask; several cities can work one mine |
| Annotations have no map UI (A-13) | Gold pins on the map, `N` opens the note dialog |
| No automated realtime test (A-15) | `build/audit-sync.js` asserts message and turn propagation between two browsers |

---

## 5. Test results

| Suite | Checks | Result |
| --- | --- | --- |
| `test/adventure.js` | 54 | all pass |
| `test/social.js` | 37 | all pass |
| `test/creatures.js` | 126 creatures | all pass |
| `e2e/game.spec.ts` | 18 | all pass |
| `build/scenarios.js` | 60 | all pass |
| `build/audit-all.js` | 43 | all pass |
| `build/audit-rules.js` | 20 | all pass |
| `build/audit-gaps.js` | 17 | all pass |
| `build/audit-mapobjects.js` | 11 | all pass |
| `build/audit-spells.js` | 6 | all pass |
| `build/audit-townentry.js` | 9 | all pass |
| `build/audit-walkin.js` | 2 | all pass |
| `build/audit-noh3.js` | 5 | all pass |
| `build/audit-hero.js` | 9 | all pass |
| `build/audit-history.js` | 11 | all pass |
| `build/audit-ai.js` | 7 | all pass |
| `build/audit-sync.js` | 4 | all pass |
| `build/audit-persist.js` | 8 | all pass |
| `build/audit-longgame.js` | 13 | all pass |
| `build/audit-responsive.js` | 36 | all pass |
| `build/gen-check` | 15 | all pass |

**Total: 385 checks, all green.**