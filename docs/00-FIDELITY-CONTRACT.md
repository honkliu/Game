# Fidelity Contract

**Status:** Authoritative. Every other document defers to this one.
**Reference:** Heroes of Might and Magic III: Shadow of Death (SoD).

---

## 1. The contract

> **HoMM3's world, presentation, exploration, heroes, creatures, and town
> development — with warfare replaced by meaningful social play.**
>
> Not a social website wearing a medieval theme.

Every feature in this game is classified below as **Preserve**, **Remove**, or
**Adapt**. A feature that is not in this table does not exist. A feature that is
in the code but not in this table is a defect.

---

## 2. Classification

### 2.1 Preserve — identical to HoMM3

| Feature | HoMM3 behaviour we keep |
| --- | --- |
| **Adventure map** | Tile map, terrain types, movement costs, fog of war (unexplored / explored / visible) |
| **Hero** | Named hero, portrait, level, experience, four primary stats, secondary skills, movement points, 7-slot army, artifacts, spellbook |
| **Movement** | Points per day from the slowest army stack, terrain cost; first left-click previews the path, a second click on that destination or Move Hero confirms. The action pointer distinguishes walking, visiting/picking up and meeting heroes, including later-day variants. Walking cannot cross solid objects or their footprints, or cut blocked diagonal corners. Only active Fly permits crossing obstacles, with a legal landing. |
| **Resources** | Seven resources, daily mine yield, town hall income |
| **Mines** | Claim by approaching the entrance from an adjacent tile; the structure stays solid; yield daily |
| **Map objects** | Resource piles, treasure chests, artifacts, shrines, wandering monsters (as obstacles) |
| **Town** | Painted clickable town view, building tree, dwellings, weekly growth, recruitment |
| **Creatures** | 9 factions × 7 tiers × base+upgrade, real SoD statistics |
| **Time** | Day / week / month; weekly growth; daily income |
| **Obelisk** | Reveals fragments of a puzzle map; after all fragments, the Grail location is revealed |
| **Grail** | Dig at the revealed location; awards the Grail |
| **Tavern** | Hire additional heroes for 2500 gold, only at an unoccupied town gate; maximum eight owned heroes |
| **Marketplace** | Trade resources at exchange rates |
| **Mage Guild** | Learn spells |
| **UI composition** | Resource bar, minimap, status window, rollover bar, control buttons — in HoMM3's arrangement |

### 2.2 Remove — no warfare

| Removed | Consequence |
| --- | --- |
| Hero-vs-hero battles | Meeting another hero opens **conversation and voluntary trade** instead |
| Creature battles | Wandering monsters are **impassable obstacles**; they never move or fight |
| Sieges | Towns have no walls to breach; entering a town is always peaceful |
| Town capture | You cannot take another player's town. An unowned (neutral) town is claimed by the first hero who enters it, as in HoMM3 |
| Stealing | You cannot take another player's resources or creatures |
| Combat spells | Spells are utility only (see §2.3) |

### 2.3 Adapt — social replacements

| HoMM3 feature | Social adaptation |
| --- | --- |
| **Combat** | **Peaceful encounters.** Joining creatures use a native-style gold component and Yes/No confirmation, not an invented Observe/Recruit/Pass menu. Declining leaves the stack solid. Under the no-combat adaptation, non-joining creatures show a single native-style acknowledgment and remain solid. Recruitment eligibility retains the approved expedition-strength rule, not original diplomacy/attitude calculations. The old observe API remains compatible but is not offered by the adventure UI. |
| **Hero battle** | **Meeting another hero** opens a dialogue: exchange messages, trade resources voluntarily, or share map knowledge. |
| **Creature army** | **Expedition strength.** Your army's total AI value is your *expedition strength*, which gates what you can safely explore (deep terrain, guarded regions) and what you can recruit. Creatures are not combat units; they are capability. |
| **Neutral mines** | **First discovery claims a mine.** Once claimed, it stays claimed. There is no hostile recapture. A mine may be **shared** if the owner grants access (see §4). |
| **Obelisk** | **Puzzle map + social clues.** Walking up to an obelisk reads it (once per player per obelisk). The obelisk screen is the original HoMM3 puzzle map: each obelisk read uncovers its share of the 48 pieces of your faction's puzzle, hiding the terrain around the X. The same screen is a board where players post clues and theories for each other. There is no Visit button or riddle. |
| **Grail** | **Digging.** After all fragments, the Grail tile is revealed. A hero must spend a day digging there. |
| **Town capture** | **Visiting.** You may enter another player's town to read its Tower and leave messages. You may not build there or take it. Neutral towns on the map have no owner; the first hero to enter one claims it for good. |
| **Combat spells** | **Utility spells only:** View Air, View Earth, Visions, Town Portal, Dimension Door, Fly, Haste (movement), Cure (removes negative effects). Original combat spell metadata is retained, but combat casts are rejected before charging mana. |

**Approved adventure spell adaptations:** Haste adds 300 movement for the current
day, or 500 with Advanced/Expert Air Magic. It applies once per hero per day;
recasting cannot stack or refill movement. Cure removes negative adventure
movement effects, restoring the daily maximum without refilling remaining
movement or removing beneficial Fly/Haste effects.

Treasure chests offer **gold or experience**, never both. The choice persists
across reloads; leaving pays nothing and retains the chest for later.

Object art and collision use the same native sprite anchor and ground mask;
decorative roofs are not extra ground walls. Resource pickups stop movement and
show their resource component in the adventure info bar for three seconds (or
until dismissed), not a website toast. Shrine, artifact and mine visits use
native-style information dialogs. These requirements do not imply that every
remaining catalogue or movement rule has been implemented.

New-day feedback uses the native NEWDAY / NEWWEEK animation in the right info
bar (1.5-second animation, 3-second display), including automatic turn advancement.
There is no invented countdown or new-day success toast. Ending a turn warns when
an awake hero can still move. Sleep/Wake changes local hero selection state; it
does not advance the day or refill movement. An unaffordable first step keeps the
red planned route without sending a failed move request or opening a popup.
An unfinished route is retained and recalculated after the next day. Future-day
planning must not assume that today's Fly or Haste survives.

**Approved remaining-gap decisions:**
- Keep existing combat skills in saved heroes, but do not offer ineffective
  combat skills in new level-up choices. Retain the native class-weighted choice
  and advancement rules for the supported adventure skills. Fire Magic is also
  retained in saves but not newly offered: none of the approved utility spells
  uses that school.
- Shared mine income accumulates fractional resource units persistently; only
  whole units enter the spendable wallet.
- All human players must agree to end a game. Computer seats follow that
  agreement; a human can withdraw consent before unanimity.

Hero meetings within one player's army use the native exchange window for
split/merge/swap and artifact transfers. Equipment can be rearranged between
valid body slots and backpack positions. Creature dismissal preserves the last
hero stack; troop and hero dismissals require confirmation. Level-up choices,
equipment arrangement and utility-spell state survive a server restart.

Town construction, recruitment, hiring, upgrades, market exchanges and gear
purchases show their result in the native town/army/resource controls, without
invented success toasts. Rejected town actions use a native information dialog.
Buying creatures closes the recruitment window. Hiring uses the original
2500-gold fee (`GameConstants::HERO_GOLD_COST`, VCMI revision
`abe535c4418b66bae8cdb56fee1473b109e8c55e`, `lib/constants/NumericConstants.h`).

---

## 3. The social loop

The game's core loop is:

```
  explore  →  discover another player's town  →  visit their Tower
     ↑                                                    ↓
     │                                          read / respond / exchange
     │                                                    ↓
  develop your town  ←  contribute to a shared project  ←┘
     ↓
  explore further
```

Every social feature must sit **inside** this loop. A feature that is a separate
form (a blog page, a message board) and not reachable by walking is a defect.

### 3.1 What the Tower is

The Tower is **someone's illustrated journal**, entered through their town. It is
not a blog site. It is a place on the map you walk to.

### 3.2 What meeting a hero does

Meeting another player's hero opens a conversation panel:

- send a message (appears in their inbox)
- offer a resource trade (they accept or decline)
- share a map annotation (a marker they can see)

### 3.3 What shared projects are

A **shared project** is a monument or festival that any player may contribute
resources to. When complete, it grants a benefit to all contributors. This is the
cooperative objective that replaces alliance warfare.

---

## 4. Mines: the peaceful rule

| Rule | Detail |
| --- | --- |
| Claim | Approaching a neutral mine's entrance claims it for you; the hero remains adjacent |
| Yield | The owner receives the yield every day |
| Recapture | **Not possible.** A claimed mine stays claimed |
| Sharing | Only the owner may share or revoke sharing. While shared, the owner and each other player receive half the normal yield; fractional units carry forward until spendable |
| Contested | If two players reach a neutral mine on the same day, the one who arrives first claims it |

---

## 5. The Grail: the peaceful endgame

| Step | Detail |
| --- | --- |
| 1 | Walk up to obelisks. Each one read uncovers part of the puzzle map |
| 2 | When every obelisk on the map has been read, the puzzle is complete and the Grail tile is revealed |
| 3 | Walk a hero to the Grail tile and **dig** (spends the rest of that day's movement) |
| 4 | Digging awards the **Grail** |
| 5 | The Grail unlocks the **Grail Structure** in your town |
| 6 | Building the Grail Structure is a **milestone**, not an ending |

**The game does not end when the Grail is found.** The social world continues.
The Grail is a trophy and a powerful building, not a game-over trigger.

The game ends only when:
- the turn limit is reached, or
- all human players agree to end it; computer seats follow their agreement.

---

## 6. Visual fidelity

### 6.1 The reference

The reference is the **published HoMM3 manual's screen diagrams**, not a
description. The manual shows:

- **Adventure map:** the map fills the screen; the **resource bar runs along the
  bottom**; the **minimap and control buttons sit in a right-hand sidebar**.
- **Town screen:** an **integrated painted scene** with clickable buildings, not
  a grid of tiles.

> **Correction to earlier drafts.** `00-BUILD-SPEC.md` v4.0 specified a top
> resource bar and a bottom-right minimap. That was a redesign, not HoMM3's
> interface. It is superseded by §6.2 below.

### 6.2 Reference geometry (800 × 600, scaled uniformly)

The game renders at **800 × 600** — HoMM3's native resolution — and is scaled
uniformly to fit the window. The composition is never rearranged.

**Adventure map screen:**

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Adventure map view | 7 | 7 | 594 | 546 |
| Minimap | 630 | 26 | 144 | 144 |
| Control buttons | 679 | 196 | 64 | 192 |
| Right info window | 605 | 389 | 175 | 168 |
| Resource bar | 3 | 575 | 794 | 22 |
| Rollover text | 8 | 556 | 586 | 18 |
| Date text | 622 | 578 | — | — |

**Town screen:**

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Painted town scene | 0 | 0 | 800 | 374 |
| Town/army panel | 0 | 374 | 800 | 226 |

These native coordinates supersede the older 640 × 480 draft diagram and region
tables in `09-ui-reference.md`.

### 6.3 Artwork direction

| Element | Direction |
| --- | --- |
| Terrain | Hand-painted look: soft gradients, visible brush texture, no flat fills |
| Buildings | Isometric-ish painted sprites, 96 × 96, with a ground shadow |
| Hero | 32 × 32 sprite with a faction-coloured flag |
| UI chrome | Carved stone with gold trim, bevelled edges, inner shadow |
| Cursors | Native CRADVNTR action frames and hotspots: walking, blocked, visiting/picking up, hero meeting and later-day variants |

### 6.4 Animation and timing

| Action | Duration |
| --- | --- |
| Hero step | 60 ms per tile |
| Panel open | 120 ms fade |
| Adventure resource feedback | Native right info bar, three seconds or until dismissed; no pickup success toast |
| Turn advance | Native NEWDAY / NEWWEEK right info bar, 1.5-second animation and three-second display; no countdown banner |

### 6.5 Sound

Out of scope for v1. Noted so it is not silently omitted.

---

## 7. Enforcement

Every requirement in this contract maps to:

1. a **server rule** (a method or endpoint),
2. an **API field**,
3. a **screen element**,
4. an **acceptance test**.

A requirement missing any of the four is incomplete. The mapping table lives in
`08-requirements-matrix.md`.

---

## 8. Superseded documents

| Document | Status |
| --- | --- |
| `00-BUILD-SPEC.md` | **Superseded** by this contract and `09-ui-reference.md`. Kept for the creature and building tables only. |
| `01-functional-spec.md` | Superseded. |
| `02-design.md` | Superseded. |
| `03-ui-spec.md` | Superseded by `09-ui-reference.md`. |
| `06-adventure-map-design.md` | Partially superseded; its §5 (painted towns) is correct and is now authoritative. |
| `07-no-auth-three-players.md` | **Still valid.** |