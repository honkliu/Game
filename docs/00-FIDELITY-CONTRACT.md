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
| **Movement** | Points per day, terrain cost, path preview, click-to-move |
| **Resources** | Seven resources, daily mine yield, town hall income |
| **Mines** | Capture by walking onto them; yield daily |
| **Map objects** | Resource piles, treasure chests, artifacts, shrines, wandering monsters (as obstacles) |
| **Town** | Painted clickable town view, building tree, dwellings, weekly growth, recruitment |
| **Creatures** | 9 factions × 7 tiers × base+upgrade, real SoD statistics |
| **Time** | Day / week / month; weekly growth; daily income |
| **Obelisk** | Reveals fragments of a puzzle map; after all fragments, the Grail location is revealed |
| **Grail** | Dig at the revealed location; awards the Grail |
| **Tavern** | Hire additional heroes |
| **Marketplace** | Trade resources at exchange rates |
| **Mage Guild** | Learn spells |
| **UI composition** | Resource bar, minimap, status window, rollover bar, control buttons — in HoMM3's arrangement |

### 2.2 Remove — no warfare

| Removed | Consequence |
| --- | --- |
| Hero-vs-hero battles | Meeting another hero opens **conversation and voluntary trade** instead |
| Creature battles | Wandering monsters are **impassable obstacles**; they never move or fight |
| Sieges | Towns have no walls to breach; entering a town is always peaceful |
| Town capture | You cannot take another player's town |
| Stealing | You cannot take another player's resources or creatures |
| Combat spells | Spells are utility only (see §2.3) |

### 2.3 Adapt — social replacements

| HoMM3 feature | Social adaptation |
| --- | --- |
| **Combat** | **Peaceful encounters.** Meeting a wandering monster stack offers a choice: *observe* (gain experience and a journal entry), *recruit* (if your army has room and you pay), or *pass*. No fighting. |
| **Hero battle** | **Meeting another hero** opens a dialogue: exchange messages, trade resources voluntarily, or share map knowledge. |
| **Creature army** | **Expedition strength.** Your army's total AI value is your *expedition strength*, which gates what you can safely explore (deep terrain, guarded regions) and what you can recruit. Creatures are not combat units; they are capability. |
| **Neutral mines** | **First discovery claims a mine.** Once claimed, it stays claimed. There is no hostile recapture. A mine may be **shared** if the owner grants access (see §4). |
| **Obelisk** | **Puzzle map + social clues.** Visiting reveals a fragment of the puzzle map. Players post clues and theories at the Obelisk. |
| **Grail** | **Digging.** After all fragments, the Grail tile is revealed. A hero must spend a day digging there. |
| **Town capture** | **Visiting.** You may enter any town to read its Tower and leave messages. You may not build there. |
| **Combat spells** | **Utility spells only:** View Air, View Earth, Town Portal, Dimension Door, Fly, Haste (movement), Cure (removes negative effects). Damage spells are removed. |

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
| Claim | Walking onto a neutral mine claims it for you |
| Yield | The owner receives the yield every day |
| Recapture | **Not possible.** A claimed mine stays claimed |
| Sharing | The owner may mark a mine as *shared*; then all players receive half its yield, and the owner receives the other half |
| Contested | If two players reach a neutral mine on the same day, the one who arrives first claims it |

---

## 5. The Grail: the peaceful endgame

| Step | Detail |
| --- | --- |
| 1 | Visit the Obelisk. Each visit reveals one of 10 puzzle-map fragments |
| 2 | After 10 fragments, the Grail tile is revealed on your map |
| 3 | Walk a hero to the Grail tile and **dig** (spends the rest of that day's movement) |
| 4 | Digging awards the **Grail** |
| 5 | The Grail unlocks the **Grail Structure** in your town |
| 6 | Building the Grail Structure is a **milestone**, not an ending |

**The game does not end when the Grail is found.** The social world continues.
The Grail is a trophy and a powerful building, not a game-over trigger.

The game ends only when:
- the turn limit is reached, or
- all players agree to end it.

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
| Adventure map view | 0 | 0 | 640 | 480 |
| Right sidebar | 640 | 0 | 160 | 600 |
| Minimap | 648 | 8 | 144 | 108 |
| Control buttons | 648 | 124 | 144 | 200 |
| Status window (hero) | 0 | 480 | 640 | 120 |
| Resource bar | 0 | 480 | 640 | 24 |
| Rollover bar | 0 | 456 | 640 | 24 |
| Date | 560 | 486 | 76 | 16 |

**Town screen:**

| Region | x | y | w | h |
| --- | --- | --- | --- | --- |
| Painted town scene | 0 | 0 | 800 | 480 |
| Building buttons | 0 | 480 | 800 | 120 |

### 6.3 Artwork direction

| Element | Direction |
| --- | --- |
| Terrain | Hand-painted look: soft gradients, visible brush texture, no flat fills |
| Buildings | Isometric-ish painted sprites, 96 × 96, with a ground shadow |
| Hero | 32 × 32 sprite with a faction-coloured flag |
| UI chrome | Carved stone with gold trim, bevelled edges, inner shadow |
| Cursors | Arrow (default), green arrow (move), red cross (blocked), gold arrow (enter) |

### 6.4 Animation and timing

| Action | Duration |
| --- | --- |
| Hero step | 60 ms per tile |
| Panel open | 120 ms fade |
| Toast | 150 ms in, 2500 ms hold, 300 ms out |
| Turn advance | 400 ms banner |

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