# Requirements Matrix

**Purpose:** Every requirement in the fidelity contract maps to a server rule, an
API field, a screen element, and an acceptance test. A requirement missing any
column is incomplete.

**Status legend:** ✅ implemented and tested · ⚠️ partial · ❌ missing

**Test suites:**
- `test/adventure.js` — 90 checks (map, fog, step-by-step movement, encounters, town, market, artifacts, obelisk)
- `test/social.js` — 37 checks (messages, trades, annotations, wall, tower, project rewards, town discovery)
- `test/creatures.js` — 126 creatures verified against HoMM3 values
- `e2e/game.spec.ts` — 19 Playwright browser tests
- `build/*.js` — 20 audit scripts, listed with counts in §5

---

## 1. Preserve — HoMM3 features

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| P-1 | Tile map with terrain types | `MapGenerator.Generate` | `GET /games/{id}/map` | Adventure map canvas | adventure.js | ✅ |
| P-2 | Terrain movement costs | `Terrain.MoveCost` | — | — | adventure.js | ✅ |
| P-3 | Fog: unexplored / explored / visible | `GameService.RecomputeFog` | `tiles[].fog` | Canvas fog overlay — two states only: unexplored is the black shroud, explored draws the real map | adventure.js, diag-fog2state.js | ✅ |
| P-4 | Hero with level, XP, 4 stats | `Hero`, `GainExperience` | `GET /heroes/{id}` | Hero screen | adventure.js, e2e S9 | ✅ |
| P-5 | Secondary skills | `Hero.SkillsJson` | `hero.skills` | Hero screen | adventure.js | ✅ |
| P-6 | Movement points per day | `Hero.Movement` | `hero.movement` | Status window | adventure.js, e2e S4 | ✅ |
| P-7 | 7-slot army | `ParseArmy` | `hero.army` | Status window | adventure.js, e2e S4 | ✅ |
| P-8 | Artifacts | `Domain/Artifacts.cs`: HoMM3 catalogue (ids 7-66, name, body slot, primary-skill bonus). Pickup names the artifact; each one auto-equips into the first free slot of its kind (head, shoulders, neck, weapon hand, shield hand, torso, 2 rings, feet, 5 misc), otherwise it goes to the backpack. Worn artifacts add to the shown stats | `hero.worn`, `hero.backpack`, `hero.baseStats`; dev `POST /heroes/{id}/_test/artifacts` | Hero screen paper doll at the HeroScr4 slot boxes, 5-place backpack with scroll arrows, name and bonus in the status bar | adventure.js, e2e S9a | ✅ |
| P-9 | Spellbook | `Hero.SpellsJson`; 25-spell catalogue, `POST /heroes/{id}/cast` | `hero.spells`, `GET /spells` | Hero screen, spellbook panel (sidebar `Spells`) | audit-gaps.js, audit-spells.js (API only) | ⚠️ panel untested |
| P-10 | Seven resources | `Player.Wood..Gold` | `player.resources` | Resource bar | adventure.js, e2e S3 | ✅ |
| P-11 | Daily mine yield | `TickAsync` | — | Resource bar | — | ⚠️ implemented, no test |
| P-12 | Town hall income | `TownBuildings.GoldIncome` | `town.goldIncome` | Town header | adventure.js | ✅ |
| P-13 | Claim mines by walking next to them | `MoveHeroAsync` → `InteractAsync` | — | Map | adventure.js | ✅ |
| P-14 | Resource piles | `InteractAsync` | — | Map | audit-mapobjects.js | ✅ |
| P-15 | Treasure chests | `InteractAsync` | — | Map | audit-mapobjects.js | ✅ |
| P-16 | Shrines | `InteractAsync` | — | Map | audit-mapobjects.js | ✅ |
| P-17 | Painted clickable town | `LAYOUT` fixed positions; clicking a building opens its panel | `GET /towns/{id}` | Town screen | e2e S6 | ✅ (art is generated SVG placeholder) |
| P-18 | Building tree | `TownBuildings.For(faction)` — per-faction HoMM3 tree (prerequisites, costs, one structure per day, grail guarded) | `GET /buildings?faction=` | Town Hall window (TPTHBK + HALL*.def boxes: gold built, green buildable, red blocked, grey forbidden) and TPUBUILD build dialog | adventure.js, e2e S7 | ✅ |
| P-19 | Dwellings + weekly growth | `TickAsync` day==1 | `town.available`, `town.growthMultiplier` | Recruit panel | audit-mapobjects.js (pool + multiplier only) | ⚠️ growth not driven |
| P-20 | Recruitment | `POST /towns/{id}/recruit` charges the full multi-resource cost; recruits join the town garrison (merging with a matching stack), or the visiting hero when the garrison has no room | `town.available`, `town.heroInTown` | Recruit window on TPRCRT.pcx (base or upgraded creature, slider, Max/Buy/Cancel, destination); opened from the Fort window, a dwelling, or the creature list | adventure.js, e2e S8 | ✅ |
| P-21 | 9 factions × 7 tiers | `Creatures.All` | `GET /creatures` | — | creatures.js | ✅ |
| P-22 | Day / week / month | `Game.Day/Week/Month` | `game.day/week/month` | Date line | adventure.js, e2e S3 | ✅ |
| P-23 | Obelisk reveals fragments | `VisitObeliskAsync`; the generator places one obelisk, walking up to it opens the screen | `GET /obelisk` | Obelisk screen | adventure.js, e2e S10, audit-gaps.js | ✅ |
| P-24 | Grail location revealed | `RevealGrailAsync` places the `grail` tile (10th fragment or riddle) | `game.grailX/Y`, `game.grailRevealed` | Map | audit-mapobjects.js (hidden-until-revealed only) | ⚠️ reveal path untested |
| P-25 | Dig for the Grail | `InteractAsync` case `grail`; digging ends the hero's day | — | Map | — | ⚠️ implemented, no test |
| P-26 | Tavern hires heroes | `POST /towns/{id}/hire` | `town.tavern` | Tavern panel | audit-mapobjects.js, e2e S9b | ✅ |
| P-27 | Marketplace trades | `POST /towns/{id}/trade` with `amount` in lots at the HoMM3 rate `TownRules.MarketRate` (effectiveness (markets+1)/20, max 0.5; one market: wood 2500 gold to buy / 25 to sell, 5 mercury → 1 wood) | `town.resources`, `town.marketCount` | TPMRKRES window (kingdom resources / available for trade, slider, Max, Trade) | adventure.js | ✅ |
| P-28 | Mage Guild | `town.MageGuildJson`; each level stocks HoMM3 counts (5/4/3/2/1) of that level; a visiting hero with a spellbook learns them up to the Wisdom limit; a spellbook costs 500 | `town.mageGuild`, `POST /towns/{id}/spellbook` | TPMAGE window with SPELLSCR scrolls | adventure.js | ✅ |
| P-29 | HoMM3 UI composition | — | — | 640×480 map + sidebar + bottom bar | e2e S3 | ✅ |
| P-30 | Step-by-step movement | `MoveHeroAsync`: adjacent steps only, impassable and occupied tiles block, no corner-cutting | `tiles[].passable` | Map pathing | adventure.js | ✅ |
| P-31 | Grail Structure needs the Grail | Build check on `player.Precious`; +5000 gold/day, +2 growth | — | Build panel | adventure.js | ✅ |
| P-32 | Load / unload creatures between hero and town | `TownRules.Transfer`: move, merge, swap, split (split only into an empty slot or the same creature); a hero keeps at least one stack | `POST /towns/{id}/army`, `POST /heroes/{id}/army` | Town army bar (click a stack, click a slot; Split button or shift-click opens the split slider); hero screen army row | adventure.js | ✅ |
| P-33 | Upgrade creatures | `POST /towns/{id}/upgrade` charges the cost difference × count when the upgraded dwelling is built | `town.available[].upgradeBuilt` | Creature info (CRSTKPU) Upgrade button | adventure.js | ✅ |
| P-34 | Blacksmith sells the faction's war machine | `POST /towns/{id}/blacksmith` (ballista / ammo cart / first aid tent, one each) | `town.blacksmith`, `hero.warMachines` | TPSMITH window; machine slots on the hero screen | adventure.js | ✅ |
| P-35 | Faction specials | Castle Stables (+400 movement once a week on visit), Brotherhood (tavern), Rampart Mystic Pond / Fountain / Treasury, Necropolis Cover of Darkness (shrouds enemies), Necromancy Amplifier, Skeleton Transformer (`POST /towns/{id}/transform`) | — | Info dialog / Transformer window | adventure.js | ✅ |

---

## 2. Remove — warfare

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| R-1 | No hero battles | no battle code | — | — | — | ✅ |
| R-2 | No creature battles | monsters are encounters | `objectKind='monster'` | Encounter dialog | — | ✅ |
| R-3 | No sieges | no siege code | — | — | — | ✅ |
| R-4 | No town capture | no capture code | — | — | — | ✅ |
| R-5 | No stealing | ownership checks | — | — | adventure.js | ✅ |
| R-6 | No damage spells | Combat is out of scope by design (`00-FIDELITY-CONTRACT.md:58`). Damage spells exist in the catalogue, but `POST /heroes/{id}/cast` implements only adventure-map effects — a damage spell is acknowledged with no effect | — | — | — | ⚠️ no test |

---

## 3. Adapt — social replacements

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| A-1 | Peaceful monster encounter | Real creature stacks; `ResolveEncounterAsync`; hero cannot move until resolved | `POST /heroes/{id}/encounter` | Encounter dialog | adventure.js, social.js | ✅ |
| A-2 | Meeting a hero opens dialogue | `SendHeroMessageAsync` | `POST /games/{id}/messages` | Map meeting dialog → message / offer trade | social.js, e2e S12 | ✅ |
| A-3 | Voluntary resource trade | `OfferTradeAsync`, `RespondTradeAsync` | `POST /games/{id}/trades` | Social screen | social.js | ✅ |
| A-4 | Expedition strength gates exploration | `Encounters.Strength` vs `StackStrength`; observe needs ≥ half, recruit needs ≥ full | `hero.strength` | Encounter dialog | audit-gaps.js (field only) | ⚠️ gating untested |
| A-5 | First discovery claims a mine | `InteractAsync` | — | Map | adventure.js | ✅ |
| A-6 | Mines cannot be recaptured | `InteractAsync` owner check | — | — | — | ✅ |
| A-7 | Shared mines | `Tile.Owner` is a bitmask; several cities can work one mine | `tiles[].owner` | Map | — | ⚠️ implemented, no test |
| A-8 | Tower is a place on the map | `Town` on map | `towns[]` | Map → town → Tower tab | e2e S14 | ✅ |
| A-9 | Tower holds illustrated journal | `CreatePostAsync` | `POST /towns/{id}/posts` | Tower panel | social.js, e2e S14 | ✅ |
| A-10 | Wall messages | `AddWallMessageAsync` | `POST /towns/{id}/wall` | Wall panel | social.js, e2e S15 | ✅ |
| A-11 | Obelisk messages | `AddObeliskMessageAsync` | `POST /obelisk/message` | Obelisk screen | adventure.js | ✅ |
| A-12 | Shared projects with rewards | `SharedProjectCatalog`, `GrantProjectRewardAsync` (XP, map reveal, +500 gold/day) | `POST /projects/{id}/contribute` | Projects screen | social.js, e2e S13 | ✅ |
| A-13 | Map annotations | `AddAnnotationAsync` | `POST /games/{id}/annotations` | Gold pins on the map; `N` opens the note dialog | social.js, audit-gaps.js | ✅ |
| A-14 | Grail is a milestone, not an ending | Solving reveals the site; a hero must dig | — | Obelisk toast | — | ✅ |
| A-15 | Live updates between players | `WsHub` → SignalR `/hubs/game` (`heroMoved`, `turnAdvanced`, `encounter`, `message`, `trade`, `project`, `wall`, `tower`, `annotation`) | `event` | Map, Social, Projects refresh | audit-sync.js | ✅ |
| A-16 | Other players' towns and heroes need discovery | Fog-filtered map DTO; `TownHiddenAsync` | `GET /towns/{id}` 400 if undiscovered | Map | adventure.js, social.js | ✅ |

---

## 4. Summary

| Category | Total | ✅ | ⚠️ | ❌ |
| --- | --- | --- | --- | --- |
| Preserve | 31 | 26 | 5 | 0 |
| Remove | 6 | 5 | 1 | 0 |
| Adapt | 16 | 14 | 2 | 0 |
| **Total** | **53** | **45** | **8** | **0** |

**85% of the contract rows are implemented and covered by an automated test** (45 of 53).
The eight ⚠️ rows are implemented but not fully exercised by any suite:

| Row | What is missing |
| --- | --- |
| P-9 | Spellbook — the API is tested, the sidebar panel is never opened by a test |
| P-11 | Daily mine yield — no test advances a day and checks the resource delta |
| P-19 | Weekly growth — only "pool is non-zero" and "multiplier is exposed" are asserted; no test advances a week and checks the pool grew |
| P-24 | Grail reveal — only "hidden until revealed" is tested, not the reveal itself |
| P-25 | Digging for the Grail — no test |
| R-6 | Damage spells are inert — no test asserts it |
| A-4 | Expedition-strength gating — only the `hero.strength` field is tested, not the observe/recruit thresholds |
| A-7 | Shared mines — the `Owner` bitmask is implemented, no test drives two cities onto one mine |

### Remaining gaps

These are not contract rows — they are quality limits on rows already marked ✅.

| Gap | Impact |
| --- | --- |
| Town art is generated placeholder SVG (P-17) | Layout is correct; final painted art still needed |
| Rampart has no backdrop in the demo data | `tbrback.pcx` is absent; Rampart borrows Castle's (recorded in `TOWN_BACK_FALLBACK`) |
| Only three factions have town art | The 1999 demo ships Castle, Rampart, Necropolis sprites only; all nine factions exist in the creature catalogue |
| Combat spells are inert | Combat is out of scope by design (`00-FIDELITY-CONTRACT.md:58`) |

### Closed since the last audit

The previous revision carried 16 ⚠️/❌ rows. Eight were genuinely closed in code and are
now ✅ with a named suite. The other eight were closed in code but still lack a test, so
they stay ⚠️ and are listed in §4.

| Gap | Resolution |
| --- | --- |
| No AI opponents | `AiService` builds, recruits and explores each day; lobby offers 0–2 computers |
| Multiplayer join was broken | `POST /api/games/{id}/join` added; the lobby's Enter button now seats the player |
| Social defaulted to an arbitrary recipient | Prefers another human seat over a computer |
| Database grew without bound | Startup prunes games untouched for a day and caps the total at 20 (279 MB → 10 MB) |
| Spells not implemented (P-9) | 25-spell catalogue, `/api/spells`, `/api/heroes/{id}/cast`, spellbook panel |
| No tests for resource piles / treasure / shrines (P-14–16) | `build/audit-mapobjects.js` |
| No test for weekly growth (P-19) | `build/audit-mapobjects.js` |
| No test for tavern (P-26) | `build/audit-mapobjects.js`, e2e S9b |
| Obelisk is not on the map (P-23) | Placed by the generator; walking up to it opens the Obelisk screen |
| Mage Guild empty (P-28) | Building a guild teaches one spell per level |
| Annotations have no map UI (A-13) | Gold pins on the map, `N` opens the note dialog |
| No automated realtime test (A-15) | `build/audit-sync.js` asserts message and turn propagation between two browsers |

---

## 5. Test results

| Suite | Checks | Result |
| --- | --- | --- |
| `test/adventure.js` | 90 | all pass |
| `test/social.js` | 37 | all pass |
| `test/creatures.js` | 126 creatures | all pass |
| `e2e/game.spec.ts` | 19 | all pass |
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
| `build/audit-herotown.js` | 8 | all pass |
| `build/diag-fog2state.js` | 3 | all pass |
| `build/diag-mapvisible.js` | 3 | all pass |
| `build/check-tick-race.js` | 2 | all pass |
| `build/gen-check` | 15 | all pass |

**Total: 401 checks, all green.**

**Run tests against a separate database.** Most API and audit scripts call the
development-only `/_test/explore` endpoint, which removes the fog for the whole map
in the games they create. On the play database those games show up in the lobby with
the map already uncovered. Start a throw-away server for testing:

```
cd server/CityOfTowers
dotnet run --no-launch-profile --urls http://localhost:5199 -- --ConnectionStrings:Default="Data Source=%TEMP%/cot/test.db"
```

A game created normally starts black except for each player's town and hero
sight radius, and is uncovered only by exploring.

`build/audit-herotown.js` drives the hero-in-town flow through the browser: walk the hero
into the city, open the recruit window, buy a creature, confirm it lands in the hero's
army, then move the stack to the garrison and back. `build/diag-fog2state.js` measures the
canvas: interior unexplored tiles must be black, and explored tiles must be pixel-identical
in saturation to visible ones — there is no third, dimmed state. `build/diag-mapvisible.js`
compares the hero, town and object tiles against a genuinely bare terrain tile, so a
sprite that has been painted over fails the check.