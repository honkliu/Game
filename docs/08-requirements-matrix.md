# Requirements Matrix

**Purpose:** Every requirement in the fidelity contract maps to a server rule, an
API field, a screen element, and an acceptance test. A requirement missing any
column is incomplete.

**Status legend:** ✅ implemented and tested · ⚠️ partial · ❌ missing

**Current action evidence:** [Action checklist and recorded results](10-action-audit.md)
supersedes the older coverage estimates and blanket all-green claims below.
`node test\actions.js --full --record` regenerates that record using isolated databases.
An endpoint existing or a smoke test passing does not establish Heroes III fidelity.

**Test suites:**
- `test/adventure.js` — 90 checks (map, fog, step-by-step movement, encounters, town, market, artifacts, obelisk)
- `test/social.js` — 37 checks (messages, trades, annotations, wall, tower, project rewards, town discovery)
- `test/creatures.js` — 126 creatures verified against HoMM3 values
- `e2e/game.spec.ts` — Playwright browser tests, including two-click planning, native action cursors, all seven resource/mine interactions, shrine pixel alignment, treasure persistence, flight controls, Haste/Cure, survey views, creature Yes/No and rejection errors, native day/week feedback, sleep/wake and cross-day route retention; see the generated report for the latest executed result
- `test/save-migration.js` — old-schema upgrade and cross-process treasure, encounter, equipment, army/dismissal, shared mine/carry, utility counters, votes and queued-level persistence, using only the isolated audit database
- `node test\movement.js` — deterministic client pathfinding and in-memory server collision checks (no running server required)
- `npm test` — executable action checklist, movement regressions and creature checks; intentionally fails while fidelity gaps remain
- `build/*.js` — historical scratch audits, not reliable current acceptance evidence

---

## 1. Preserve — HoMM3 features

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| P-1 | Tile map with terrain types | `MapGenerator.Generate` | `GET /games/{id}/map` | Adventure map canvas | adventure.js | ✅ |
| P-2 | Terrain movement costs | Source terrain, Pathfinding, whole-army native terrain, diagonal rounding and final-step allowance | `hero.nativeTerrain` | Current/future-day path preview uses matching costs | actions: movement-source/diagonal/last-step/exhaustion/native-terrain; movement.js | ✅ focused numerical cases; latest full-run status in generated report |
| P-3 | Fog: unexplored / explored / visible | `GameService.RecomputeFog` | `tiles[].fog` | Canvas fog overlay — two states only: unexplored is the black shroud, explored draws the real map | adventure.js, diag-fog2state.js | ✅ |
| P-4 | Hero with level, XP, 4 stats | Native profiles, XP table, class weights and persisted multi-level advancement | `hero.pendingLevel`, level-choice endpoint | Hero screen and native LVLUPBKG choice | actions: hero-profiles, hero-level-*; e2e S37; save-migration | Implemented; execution status in generated report |
| P-5 | Secondary skills | Native class-weighted choices; eight-skill limit; preserve stored combat skills but filter ineffective new offers | `hero.skills`, `hero.pendingLevel` | Level-up choice and hero skill slots | actions: hero-skill-*; e2e S37; save-migration | Approved peaceful subset; execution status in generated report |
| P-6 | Movement points per day | `MaxMovement` uses the slowest stack and Logistics; next-day refresh expires daily effects | `hero.movement`, `hero.maxMovement` | Status window | actions: movement-army-speed, movement-speed-table, haste-day-* | ✅ tested mixed armies, speed cap, Logistics and refresh |
| P-7 | 7-slot army | `ParseArmy` | `hero.army` | Status window | adventure.js, e2e S4 | ✅ |
| P-8 | Artifacts | `Domain/Artifacts.cs`: HoMM3 catalogue (ids 7-66, name, body slot, primary-skill bonus). Pickup names the artifact; each one auto-equips into the first free slot of its kind (head, shoulders, neck, weapon hand, shield hand, torso, 2 rings, feet, 5 misc), otherwise it goes to the backpack. Worn artifacts add to the shown stats | `hero.worn`, `hero.backpack`, `hero.baseStats`; dev `POST /heroes/{id}/_test/artifacts` | Hero screen paper doll at the HeroScr4 slot boxes, 5-place backpack with scroll arrows, name and bonus in the status bar | adventure.js, e2e S9a | ✅ |
| P-9 | Spellbook | Seventy native metadata rows, eight approved utilities, mastery costs, Portal/Door/Visions rules; combat/no-op casts rejected before payment | `hero.spells`, `GET /spells`, `POST /heroes/{id}/cast` | Native SpelBack book, school filters, target pointer, Portal choices and world view | actions: spell-*, flight-*, haste-day-*, cure-penalty, viewAir/viewEarth-mastery-*; e2e S18–20, S38–40; save-migration | Implemented approved utilities, not the original combat system; execution status in generated report |
| P-10 | Seven resources | `Player.Wood..Gold` | `player.resources` | Resource bar | adventure.js, e2e S3 | ✅ |
| P-11 | Daily mine yield | `TickAsync` | — | Resource bar | actions: mine-gold/wood/ore/mercury/sulfur/crystal/gems | ✅ exact daily yields checked |
| P-12 | Town hall income | `TownBuildings.GoldIncome` | `town.goldIncome` | Town header | adventure.js | ✅ |
| P-13 | Claim mines by walking next to them | `MoveHeroAsync` → `InteractAsync` | — | Map | adventure.js | ✅ |
| P-14 | Resource piles | `InteractAsync` stops at a pickup and credits it once | `X-Move-Events` resource component | Visit cursor; preview/confirm; three-second right info bar, click to dismiss | actions: resource-*; e2e S21, all seven resources | ✅ stated pickup cases; latest run status in generated report |
| P-15 | Treasure chests | Persisted pending choice, one selected reward, concurrency-token replay protection | `hero.pendingTreasure`, `POST /heroes/{id}/treasure` | Gold/experience/leave dialog | actions: treasure-choice/gold/experience/leave/concurrent; e2e S17; save-migration | ✅ mutually exclusive rewards, reload/restart and replay checked |
| P-16 | Shrines | `InteractAsync`; spellbook and Wisdom checks; persistent solid object | Native `sprite` anchor and `X-Move-Events` spell notice | Visit cursor and shrine dialog; body aligns with blocked tile | actions: shrine-book/wisdom/learn; movement.js; e2e S23 levels 1-3 and actual canvas pixels | ✅ stated cases; latest run status in generated report |
| P-17 | Painted clickable town | Fixed native structure positions and clickable building masks | `GET /towns/{id}` | Original town assets when supplied locally; generated art fallback otherwise | e2e S6 | ✅ stated composition and interactions, not pixel-perfect fallback art |
| P-18 | Building tree | `TownBuildings.For(faction)` — per-faction HoMM3 tree (prerequisites, costs, one structure per day, grail guarded) | `GET /buildings?faction=` | Town Hall window (TPTHBK + HALL*.def boxes: gold built, green buildable, red blocked, grey forbidden) and TPUBUILD build dialog | adventure.js, e2e S7 | ✅ |
| P-19 | Dwellings + weekly growth | Native base growth, fortification and horde bonuses; Grail adds half the resulting growth | `town.available`, `town.growthMultiplier` | Recruit panel | actions: native-creature-reference, special-growth-reference, dwelling-growth | ✅ focused three-town/tier/fortification/Grail combinations; latest full-run status in generated report |
| P-20 | Recruitment | `POST /towns/{id}/recruit` charges the full multi-resource cost; recruits join the town garrison (merging with a matching stack), or the visiting hero when the garrison has no room | `town.available`, `town.heroInTown` | TPRCRT window, Max/Buy/Cancel; buying closes the window and updates actual army/stock, without a success toast | actions: recruit-*, native-creature-reference; e2e S8 | Latest execution in generated report |
| P-21 | 9 factions × 7 tiers | `Creatures.All` | `GET /creatures` | — | creatures.js | ✅ |
| P-22 | Day / week / month | `Game.Day/Week/Month` | `game.day/week/month/turn` | Native NEWDAY / NEWWEEK right info bar; end-turn reminder, local sleep/wake; no countdown toast | adventure.js; e2e S26, S30–32 | ✅ stated cases; latest execution in generated report |
| P-23 | Obelisk reveals fragments | `VisitObeliskAsync`; the generator places one obelisk, walking up to it opens the screen | `GET /obelisk` | Obelisk screen | adventure.js, e2e S10, audit-gaps.js | ✅ |
| P-24 | Grail location revealed | `RevealGrailAsync` places the `grail` tile (10th fragment or riddle) | `game.grailX/Y`, `game.grailRevealed` | Map | actions: obelisk-reveal | ✅ ten-fragment reveal tested |
| P-25 | Dig for the Grail | `InteractAsync` case `grail`; digging ends the hero's day | — | Map | actions: grail-dig | ✅ award and remaining-movement debit tested |
| P-26 | Tavern hires heroes | Original 2500-gold fee, unoccupied gate, eight-hero limit; unaffordable attempts preserve wallet/roster | `town.tavern` | Native tavern and actual visiting hero; no hiring success toast | actions: hire-hero/gate/limits; e2e S9b | Latest execution in generated report |
| P-27 | Marketplace trades | `POST /towns/{id}/trade` with `amount` in lots at the HoMM3 rate `TownRules.MarketRate` (effectiveness (markets+1)/20, max 0.5; one market: wood 2500 gold to buy / 25 to sell, 5 mercury → 1 wood) | `town.resources`, `town.marketCount` | TPMRKRES window (kingdom resources / available for trade, slider, Max, Trade) | adventure.js | ✅ |
| P-28 | Mage Guild | `town.MageGuildJson`; each level stocks HoMM3 counts (5/4/3/2/1) of that level; a visiting hero with a spellbook learns them up to the Wisdom limit; a spellbook costs 500 | `town.mageGuild`, `POST /towns/{id}/spellbook` | TPMAGE window with SPELLSCR scrolls | adventure.js | ✅ |
| P-29 | HoMM3 UI composition | — | — | Native 594×546 viewport in the 800×600 frame; right info bar and bottom resources; contract §6.2 | e2e S3 | ✅ stated geometry |
| P-30 | Step-by-step movement | `MoveHeroAsync`: solid objects, footprints and occupied tiles block walking and corner-cutting. Old saves rebuild native ground masks and repair stale passability. Town gates stop walking; creatures/shrines are visited from beside them. Active Fly crosses without interacting and lands legally | `tiles[].passable`, `tiles[].sprite`, `hero.flying`, `hero.nativeTerrain` | Preview/confirm, native action pointers, native ground costs and future-day ground replanning after Fly expires | adventure.js, movement.js, actions: movement-*, flight-*; e2e S5, S18, S21–24 | Focused movement cases passed; latest full-run status in generated report |
| P-31 | Grail Structure needs the Grail | Consumes the Grail; +5000 gold/day and +50% growth after fortification/horde bonuses | — | Build panel | actions: grail-*, special-growth-reference | ✅ focused rules; latest full-run status in generated report |
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
| R-6 | No damage spells | Combat metadata is retained, but combat/no-op casting is rejected before mana or state changes | Cast endpoint | No cast control for combat spells | actions: spell-reject-unavailable; e2e S39 | Implemented; latest execution in generated report |

---

## 3. Adapt — social replacements

| # | Requirement | Server rule | API | Screen | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| A-1 | Peaceful monster encounter | `ResolveEncounterAsync`; authoritative price and strength eligibility in `GetEncounterOffer`; hero remains adjacent | `hero.encounterOffer`, `POST /heroes/{id}/encounter` | Native-style gold component and Yes/No; no Observe/Pass menu; native error overlay preserves pending offer; non-joining OK is the no-combat adaptation | actions: encounter-*; e2e S24, S27–29 | ⚠️ stated flows covered; original diplomacy/attitude and combat/flee behavior deliberately not claimed |
| A-2 | Meeting a hero opens dialogue | `SendHeroMessageAsync` | `POST /games/{id}/messages` | Map meeting dialog → message / offer trade | social.js, e2e S12 | ✅ |
| A-3 | Voluntary resource trade | `OfferTradeAsync`, `RespondTradeAsync` | `POST /games/{id}/trades` | Social screen | social.js | ✅ |
| A-4 | Expedition strength gates exploration | `Encounters.Strength` vs `StackStrength`; recruit needs ≥ full; legacy observe API needs ≥ half | `hero.strength`, `hero.encounterOffer.canRecruit` | Joining offer or non-joining acknowledgment; no invented numeric strength comparison | actions: encounter-strength, encounter-offer*; e2e S27–29 | ✅ approved adaptation, not original diplomacy |
| A-5 | First discovery claims a mine | `InteractAsync` | — | Map | adventure.js | ✅ |
| A-6 | Mines cannot be recaptured | `InteractAsync` owner check | — | — | — | ✅ |
| A-7 | Shared mines | Owner-only share/revoke, half yield and persistent fractional carry | Mine sharing endpoint, `tiles[].shared` | Mine information dialog with native confirmation | actions: mine-owner-consent, mine-share-*; e2e S41; save-migration | Implemented approved rule; latest execution in generated report |
| A-8 | Tower is a place on the map | `Town` on map | `towns[]` | Map → town → Tower tab | e2e S14 | ✅ |
| A-9 | Tower holds illustrated journal | `CreatePostAsync` | `POST /towns/{id}/posts` | Tower panel | social.js, e2e S14 | ✅ |
| A-10 | Wall messages | `AddWallMessageAsync` | `POST /towns/{id}/wall` | Wall panel | social.js, e2e S15 | ✅ |
| A-11 | Obelisk messages | `AddObeliskMessageAsync` | `POST /obelisk/message` | Obelisk screen | adventure.js | ✅ |
| A-12 | Shared projects with rewards | `SharedProjectCatalog`, `GrantProjectRewardAsync` (XP, map reveal, +500 gold/day) | `POST /projects/{id}/contribute` | Projects screen | social.js, e2e S13 | ✅ |
| A-13 | Map annotations | `AddAnnotationAsync` | `POST /games/{id}/annotations` | Gold pins on the map; `N` opens the note dialog | social.js, audit-gaps.js | ✅ |
| A-14 | Grail is a milestone, not an ending | Solving reveals the site; a hero must dig | — | Obelisk toast | — | ✅ |
| A-15 | Live updates between players | `WsHub` → SignalR `/hubs/game` (`heroMoved`, `turnAdvanced`, `encounter`, `message`, `trade`, `project`, `wall`, `tower`, `annotation`) | `event` | Map, Social, Projects refresh | audit-sync.js | ✅ |
| A-16 | Other players' towns and heroes need discovery | Fog-filtered map DTO; `TownHiddenAsync` | `GET /towns/{id}` 400 if undiscovered | Map | adventure.js, social.js | ✅ |
| A-17 | Unanimous ending | Human votes may be withdrawn; unanimity ends the game, computer seats follow; finished games reject mutations | Game end-agreement endpoint | Social agreement controls and all-viewer game-over navigation | actions: social-end-*, concurrent-*; e2e S43; save-migration | Implemented approved rule; latest execution in generated report |

---

## 4. Summary

The previous percentage-based coverage estimate is withdrawn: several tests checked
only that fields or endpoints existed, not that actions matched the reference rules.
The [generated action audit](10-action-audit.md) now records each actual assertion,
failure, missing requirement, route mapping, suite result, revision and worktree.
Its failures and gaps take precedence over older implemented/tested labels here.

### Remaining gaps

These are not contract rows — they are quality limits on rows already marked ✅.

| Gap | Impact |
| --- | --- |
| Built-in fallback art is generated (P-17) | Original town art requires locally supplied game data; the fallback is not pixel-identical |
| Rampart has no backdrop in the demo data | `tbrback.pcx` is absent; Rampart borrows Castle's (recorded in `TOWN_BACK_FALLBACK`) |
| Only three factions have town art | The 1999 demo ships Castle, Rampart, Necropolis sprites only; all nine factions exist in the creature catalogue |
| No original combat system | Combat spells retain metadata only and are rejected before payment; approved Haste/Cure adventure adaptations are separate |

### Historical implementation notes

These notes describe earlier implementation work, not current proof of fidelity.
In particular, a spell catalogue does not establish working utility spells, and
scratch-script success is not a substitute for the recorded action assertions.

| Gap | Resolution |
| --- | --- |
| No AI opponents | `AiService` builds, recruits and explores each day; lobby offers 0–2 computers |
| Multiplayer join was broken | `POST /api/games/{id}/join` added; the lobby's Enter button now seats the player |
| Social defaulted to an arbitrary recipient | Prefers another human seat over a computer |
| Database grew without bound | Startup prunes games untouched for a day and caps the total at 20 (279 MB → 10 MB) |
| Spells not implemented (P-9) | Historical 25-spell implementation superseded by native metadata, eight approved utilities and the executable action/browser/restart record |
| No tests for resource piles / treasure / shrines (P-14–16) | `build/audit-mapobjects.js` |
| No test for weekly growth (P-19) | `build/audit-mapobjects.js` |
| No test for tavern (P-26) | `build/audit-mapobjects.js`, e2e S9b |
| Obelisk is not on the map (P-23) | Placed by the generator; walking up to it opens the Obelisk screen |
| Mage Guild empty (P-28) | Building a guild teaches one spell per level |
| Annotations have no map UI (A-13) | Gold pins on the map, `N` opens the note dialog |
| No automated realtime test (A-15) | `build/audit-sync.js` asserts message and turn propagation between two browsers |

---

## 5. Test results

Use the [latest recorded results](10-action-audit.md), not the removed historical
"401 checks, all green" claim. Some scratch audits are stale or assert less than
their names imply.

Run `node test\actions.js --full --record` to build, execute the checklist and current suites,
and regenerate the record. The runner starts its own server on an ephemeral port,
uses a temporary database, checks readiness, and stops only that owned process.
Do not run development fixture endpoints or scratch restart scripts against the
normal play database.