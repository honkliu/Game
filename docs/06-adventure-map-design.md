# Design Update — Adventure Map, Hero, and Factions

**Version:** 2.0
**Supersedes:** the city-only model in `02-design.md`
**Date:** 2026-10-22

---

## 1. What changed and why

The first design was a city-builder with a social layer. That is **not** HoMM3's
play loop. HoMM3's loop is:

```
  move hero across the map  →  capture mines / collect resources
            ↓                              ↓
  return to town  ←──────────────  spend resources on buildings
            ↓
  recruit creatures into the hero's army
            ↓
  explore further (fog of war)
```

The only thing removed is **combat**. Everything else stays.

This document specifies the additions: the adventure map, the hero, and the
nine factions.

---

## 2. The adventure map

### 2.1 Structure
- A square tile map, **72 × 72** by default (configurable 36–144).
- Each tile has: terrain type, an optional object, and a passability flag.
- Terrain types and movement costs:

| Terrain | Move cost |
| --- | --- |
| Dirt | 100 |
| Grass | 100 |
| Sand | 100 |
| Rough | 125 |
| Snow | 150 |
| Swamp | 175 |
| Subterranean | 100 |
| Lava | 100 |
| Water | impassable (no boats in v1) |
| Rock | impassable |

### 2.2 Fog of war
Three states per tile, per player:
- **Unexplored** — never seen. Rendered black.
- **Explored** — seen before, not currently visible. Rendered dimmed.
- **Visible** — within the hero's scouting radius. Rendered full.

Scouting radius = `5 + 2 × Scouting skill level`.

### 2.3 Map objects

| Object | Passable | Effect on entry |
| --- | --- | --- |
| Mine | no | Capture it; owner gains its income each day |
| Resource pile | yes | Collect N of one resource; tile becomes empty |
| Treasure chest | yes | Gain gold and experience |
| Artifact | yes | Add to hero's backpack |
| Shrine | yes | Learn one spell |
| Wandering monster | no | **Blocked** — no combat, so it is an obstacle |
| Town | no | Enter the town screen (own) or view it (other) |
| Tree / rock / mountain | no | Decorative obstacle |

> **Design note:** with combat removed, wandering monsters become impassable
> terrain. They are still placed for map texture, but they never move and never
> fight. This is the one place where removing combat changes the map's feel.

### 2.4 Mines
- Seven mine types, one per resource.
- A mine has an owner (player index, or −1 for neutral).
- Entering a neutral or enemy mine captures it.
- Each owned mine yields its resource **every day**.

| Mine | Yield per day |
| --- | --- |
| Sawmill (wood) | +2 wood |
| Ore Pit (ore) | +2 ore |
| Alchemist's Lab (mercury) | +1 mercury |
| Sulfur Dune (sulfur) | +1 sulfur |
| Crystal Cavern (crystal) | +1 crystal |
| Gem Pond (gems) | +1 gems |
| Gold Mine | +1000 gold |

---

## 3. The hero

### 3.1 Attributes

| Attribute | Meaning |
| --- | --- |
| **Name** | From the faction's hero roster |
| **Faction** | Determines starting army and available heroes |
| **Level** | 1–75, from experience |
| **Experience** | Thresholds follow the HoMM3 table |
| **Attack** | Primary skill; affects army strength |
| **Defense** | Primary skill |
| **Power** | Primary skill; affects spell strength |
| **Knowledge** | Primary skill; affects max mana |
| **Mana** | Current / max spell points |
| **Movement** | Points remaining this turn / max |
| **Army** | 7 slots, each a creature type + count |
| **Skills** | Secondary skills with levels None/Basic/Advanced/Expert |
| **Spells** | Known spells |
| **Artifacts** | Up to 8 equipped |

### 3.2 Movement
- Base movement: **1500 points per day**.
- Each tile costs its terrain's move cost.
- Diagonal moves cost the same as orthogonal.
- Logistics adds `150 × level` points.
- Boots of Speed add 600.
- Movement resets at the start of each day.

### 3.3 Experience and leveling
- Experience thresholds follow the HoMM3 table (1000, 2000, 3200, 4600, ...).
- Each level grants **one** of:
  - +1 to a random primary skill, or
  - +1 level to a random secondary skill (up to Expert).
- Level 1 heroes start with two secondary skills at Basic.

### 3.4 Army
- Seven slots.
- Recruiting adds to an existing stack of the same creature, or fills an empty slot.
- Army strength (for AI comparison) = Σ `creature.aiValue × count`.

### 3.5 Heroes per player
- Each player starts with **one** hero, placed next to their town.
- Additional heroes may be hired at a **Tavern** for 2000 gold.
- Maximum 8 heroes per player.

---

## 4. Factions

Nine factions. Each has:
- a **town type** (determines the town screen art and building tree)
- **seven creature tiers**, each with a base and an upgraded form
- a **hero roster** (might and magic heroes)

| Faction | Terrain | Tier 1 | Tier 7 |
| --- | --- | --- | --- |
| Castle | Grass | Pikeman | Angel |
| Rampart | Grass | Centaur | Green Dragon |
| Tower | Snow | Gremlin | Giant |
| Inferno | Lava | Imp | Devil |
| Necropolis | Dirt | Skeleton | Bone Dragon |
| Dungeon | Subterranean | Troglodyte | Red Dragon |
| Stronghold | Rough | Goblin | Behemoth |
| Fortress | Swamp | Gnoll | Hydra |
| Conflux | Grass | Pixie | Firebird |

Creature statistics are the real HoMM3 Shadow of Death values (attack, defense,
damage range, health, speed, growth, cost, shots, abilities).

---

## 5. Town screen

### 5.1 Structure
The town screen is a **painted view** with clickable buildings, not a grid.
Each faction has its own layout of building positions.

### 5.2 Building tree (per faction)

| Category | Buildings |
| --- | --- |
| **Hall** | Village Hall → Town Hall → City Hall → Capitol |
| **Fortifications** | Fort → Citadel → Castle |
| **Dwellings** | 7 tiers, each with an upgrade |
| **Mage Guild** | Levels 1–5 |
| **Marketplace** | Trade resources |
| **Tavern** | Hire heroes |
| **Blacksmith** | Buy war machines (no combat, so cosmetic in v1) |
| **Special** | Faction-specific (e.g. Castle's Griffin Tower) |
| **Grail** | Requires the Grail |

### 5.3 Weekly growth
- Each dwelling accumulates creatures **every week** (day 1 of the week).
- Growth = base growth × (1 + 0.5 if Citadel) × (2 if Castle) + 2 if Grail.
- Unrecruited creatures accumulate.

### 5.4 Recruitment
- Recruiting costs gold (and sometimes resources for tier 7).
- Recruited creatures go to the hero standing in the town, or to the garrison.

---

## 6. Time

| Unit | Length |
| --- | --- |
| Day | 1 turn |
| Week | 7 days |
| Month | 4 weeks |

- **Every day:** mines yield, town halls yield gold, hero movement resets.
- **Every week (day 1):** dwellings grow.
- **Every month:** nothing special in v1.

---

## 7. What the social layer becomes

The social features stay, but they attach to the new structure:

| Feature | Change |
| --- | --- |
| **Magic Tower** | Still a town building; still holds blog posts |
| **City wall** | Still a message board on the town |
| **Obelisk** | Now a **map object** you walk to, not a menu item |
| **Precious / Grail** | The Grail is dug up at the location the Obelisk reveals |

The Obelisk becomes a real place on the map. Visiting it reveals a fragment.
When all 10 fragments are seen, the Grail's location is revealed on the map.
Digging there awards the Precious.

---

## 8. Data model additions

```
Game
  + MapWidth, MapHeight
  + TilesJson          (terrain + object per tile)
  + GrailX, GrailY     (revealed after 10 fragments)

Player
  + Faction            (castle | rampart | ...)
  + Resources          (moved from City)

Hero
  Id, GameId, PlayerId
  Name, Faction, Level, Experience
  Attack, Defense, Power, Knowledge
  Mana, MaxMana
  Movement, MaxMovement
  X, Y
  ArmyJson             (7 slots)
  SkillsJson
  SpellsJson
  ArtifactsJson

Town
  Id, GameId, PlayerId
  Name, Faction, X, Y
  BuildingsJson
  AvailableJson        (creatures available to recruit per tier)
  GarrisonJson
```

---

## 9. API additions

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/games/{id}/map` | Tiles visible to the caller + objects |
| GET | `/api/games/{id}/heroes` | The caller's heroes |
| POST | `/api/heroes/{id}/move` | `{path: [{x,y}...]}` — move along a path |
| GET | `/api/heroes/{id}` | Hero detail |
| GET | `/api/towns/{id}` | Town screen data |
| POST | `/api/towns/{id}/build` | `{buildingKey}` |
| POST | `/api/towns/{id}/recruit` | `{tier, count}` |
| POST | `/api/towns/{id}/hire` | `{heroId}` — tavern |
| POST | `/api/towns/{id}/trade` | `{from, to, amount}` — marketplace |

---

## 10. Client screens (revised)

| # | Screen | Change |
| --- | --- | --- |
| S1 | Login | unchanged |
| S2 | Lobby | unchanged |
| S3 | Game room | unchanged |
| **S4** | **Adventure map** | **new** — replaces the city grid as the main screen |
| S5 | Town screen | rewritten as a painted view |
| S6 | Hero screen | **new** |
| S7 | Magic Tower | unchanged |
| S8 | Obelisk | now reached by walking to it |
| S9 | Game over | unchanged |

---

## 11. Acceptance criteria (additions)

| # | Criterion |
| --- | --- |
| AC-25 | A player starts with one hero adjacent to their town. |
| AC-26 | A hero can move along a path, spending movement points by terrain cost. |
| AC-27 | A hero cannot move onto impassable terrain or beyond its movement. |
| AC-28 | Fog of war reveals tiles within the hero's scouting radius. |
| AC-29 | Entering a neutral mine captures it and it yields resources daily. |
| AC-30 | Entering a resource pile collects it and empties the tile. |
| AC-31 | A hero gains experience from treasure chests and levels up. |
| AC-32 | Dwellings accumulate creatures weekly. |
| AC-33 | Recruiting adds creatures to the hero's army. |
| AC-34 | The town screen shows only buildings available to that faction. |
| AC-35 | The Obelisk is a map object; visiting it reveals a fragment. |
| AC-36 | After 10 fragments, the Grail location is revealed on the map. |