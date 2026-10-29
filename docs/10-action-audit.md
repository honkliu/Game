# Action audit: checklist and latest evidence

Generated: 2026-10-29T17:22:00.131Z  
Revision: `d2e6689059924cab8187fb84156b6f473c5291f5` (worktree changes listed below)  
Command: `node test\actions.js --full --record`  
Overall: **INCOMPLETE — the recorded peaceful-ruleset checklist, not a complete Heroes III implementation.**

## Scope and ruleset

Current public mutation routes and action rules in docs/00-FIDELITY-CONTRACT.md; not all mechanics in the original game. Query/catalogue and development-only routes are excluded from mutation inventory. PASS only describes the stated assertion, not exhaustive conformance.

Keep the peaceful adaptations: no battles, hostile recapture, stealing or town capture. Mines require owner consent to share; creature joining uses a gold component and native Yes/No, while non-joining stacks remain solid after a native acknowledgment. The legacy observe API is retained but no longer offered as an invented adventure menu. Expedition-strength eligibility is an adaptation, not original diplomacy. The Grail remains a milestone.

The checklist below is executable: every action row comes from the actual in-memory HTTP audit, with fresh SQLite state per case. Movement checks execute the real client pathfinder and server movement service. Integration/browser suites use a separate temporary database and ephemeral port, never the normal play database.

**187 action cases passed; 0 failed; 0 additional coverage/implementation gaps.** All 35 current public mutation routes are inventoried; 0 have no evidence mapping. Route evidence is deliberately narrower than complete behavioral coverage.

PASS means only the written assertions passed. FAIL is a reproducible mismatch. NOT RUN means required current-run evidence is missing. All 19 original obligations remain below; their status is derived from named assertions, never deleted or manually marked complete. The native reference case compares every retained creature and every supported purchasable town building. Browser cases cover real controls and persistence is tested with actual process restarts.

## Map-object and action-pointer inventory

This is the acceptance inventory, not an additional set of automatically passing cases.
Use the execution record below to determine whether its cited tests actually passed.
Native ground masks are numerical metadata decoded from the installed game's
`OBJECTS.TXT`; artwork stays in the player's local installation.

| Object / destination | Physical and interaction rule | Pointer / feedback | Evidence and limits |
| --- | --- | --- | --- |
| Open ground | First click plans without moving; second click or Move Hero executes. | Walking horse; later-day pointer and red path beyond the movement budget. | movement client cursor/source/diagonal/native/last-step/Fly-expiry tests; browser S5, S21 and S26 (actual red-cross pixels). |
| Resources: wood, ore, mercury, sulfur, crystal, gems, gold | Collect once and stop at the pile. | Visiting horse; native resource component in the right info bar for 3 seconds or until dismissed. | actions resource-*; browser S21 for every resource, real cursor image pixels, HTTP event, wallet debit/credit and dismissal. |
| Treasure chest | Stop for an explicit, persisted gold/XP/leave choice; select then confirm. | Visiting horse; native choice dialog. | actions treasure-*; browser S17 and S24; save-migration restart checks. |
| Artifact | Collect without disturbing manual equipment; equip, unequip, rearrange or exchange on the hero screens. | Visiting horse; native information dialog with artifact icon. | actions artifact-*; browser S24, S33, S34; cross-process persistence. |
| Creature | Solid; stop beside it; declined/non-joining stacks remain solid, accepted stacks merge and charge the quoted gold. | Visiting horse; native gold component and Yes/No; non-joining OK is a no-combat adaptation. No Observe/Pass menu or success toast. | actions encounter-* including quote/legacy/full-army merge; browser S24, S27-29 accept/decline/reload/weak/gold/full errors. Original diplomacy and battle/flee are not implemented. |
| Mines: all seven types | Native ground shapes; claim from beside the entry; owner alone grants or revokes half-yield sharing. | Visiting horse; daily-yield component and native notice. | actions mine-*; native-body movement checks; browser S22 and S41; fractional carry and permissions survive restart. |
| Shrines: levels 1-3 | Native one-tile ground body; stay adjacent; spellbook/Wisdom/known-spell checks. | Visiting horse; native shrine/spell information dialog. | actions shrine-*; movement; actual renderer; browser S23 compares original base pixels with the map canvas and checks learn/revisit. |
| Obelisk | Native one-tile ground body; visit from beside it. | Visiting horse; walking up reads it once and opens the puzzle map. | actions obelisk-visit; client anchor; movement; browser S10, S24. |
| Own / other town | Native 13-tile ground mask, not the old 15-tile rectangle; own gate may be occupied, body is not transit. | Visiting horse; town pointer when standing in own gate; opens town on arrival. | actions town-*; movement body/gate/legacy-save tests; client cursor tests; browser S25 clicks own town body and enters the gate only after confirmation. Other-faction town pixel matrix not established. |
| Other hero | Occupied tile; approach without walking through it. | Native meeting pointer; native owned-hero exchange or approved peaceful social meeting. | movement occupancy/cursor tests; browser S33 and S42; split/merge/swap, artifacts, Scholar, messages, trade and map-marker linkage. |
| Trees, rocks, mountains | Choose artwork whose native solid cells fit existing obstacles; preserve roads and pickups in old saves. | No visit pointer for an inaccessible obstacle; Fly remains the explicit transit exception. | movement fit/corner tests; client dirt-tree anchors; flight-cross-*; browser S18. |
| Grail site | A marked digging site, not an obelisk-shaped building; digging spends the remaining movement. | Visiting pointer; Grail notice; approved milestone adaptation. | actions grail-*; client cursor tests; browser S24 visits and checks the digging movement debit. Dedicated site-pixel case not established. |

## Turn and movement feedback inventory

Browser S26 checks that an unaffordable first step makes no move request, opens no
popup and retains its red path; the same route can execute after the new day.
S30 checks partial route exhaustion and continuation. S31 covers end-turn
confirmation/cancellation, native NEWDAY/NEWWEEK graphics, three-second dismissal
and automatic server turn events. S32 checks persisted local sleep/wake, no date
or movement mutation on sleep, and waking on actual movement. Daily feedback uses
a 1.5-second animation in the native info bar, not a countdown or success toast.
The movement suite additionally verifies source terrain, diagonal rounding, army-native terrain, last-step exhaustion and ground replanning after Fly expires.

## Reproduce

`npm test` runs deterministic actions, client/server movement and creature checks.  
`node test\actions.js --full --record` also builds both projects, runs adventure/social/browser and save-migration suites, and refreshes this file.

Results and individual suite logs: `test-results\action-audit\results.json` and sibling `*.log` files (ignored generated artifacts). The process exits nonzero for failures or unresolved gaps; a green smoke suite must not mask a red fidelity audit. Dependencies must already be restored/installed.

## Action checklist

| ID | Action | Rule | Expected / asserted behavior | Result | Failure detail |
| --- | --- | --- | --- | --- | --- |
| spell-native-metadata | Use original utility spell schools, levels and mana costs | Preserve | The original seventy spell definitions are retained as metadata, but only the eight preserved/adapted utility effects are castable. | PASS |  |
| spell-removed | Reject every unsupported or combat-only spell before payment | Remove | Retained legacy spells cannot become paid no-op casts in a peaceful game. | PASS |  |
| spell-portal-0 | Town Portal at Earth mastery 0 | Preserve | No/Basic chooses the nearest own town; Advanced/Expert requires an explicit own town; movement is 300/200 and mana is 16/12. | PASS |  |
| spell-portal-1 | Town Portal at Earth mastery 1 | Preserve | No/Basic chooses the nearest own town; Advanced/Expert requires an explicit own town; movement is 300/200 and mana is 16/12. | PASS |  |
| spell-portal-2 | Town Portal at Earth mastery 2 | Preserve | No/Basic chooses the nearest own town; Advanced/Expert requires an explicit own town; movement is 300/200 and mana is 16/12. | PASS |  |
| spell-portal-3 | Town Portal at Earth mastery 3 | Preserve | No/Basic chooses the nearest own town; Advanced/Expert requires an explicit own town; movement is 300/200 and mana is 16/12. | PASS |  |
| spell-portal-guards | Reject an occupied nearest town and insufficient portal movement | Preserve | Basic Town Portal does not silently skip an occupied closest town or appear beside its gate; rejection costs nothing. | PASS |  |
| spell-door-limit-0 | Dimension Door daily limit at Air mastery 0 | Preserve | Allow one/two/three/four casts, charge 300/200 movement, reject excess casts without payment and reset tomorrow. | PASS |  |
| spell-door-limit-1 | Dimension Door daily limit at Air mastery 1 | Preserve | Allow one/two/three/four casts, charge 300/200 movement, reject excess casts without payment and reset tomorrow. | PASS |  |
| spell-door-limit-2 | Dimension Door daily limit at Air mastery 2 | Preserve | Allow one/two/three/four casts, charge 300/200 movement, reject excess casts without payment and reset tomorrow. | PASS |  |
| spell-door-limit-3 | Dimension Door daily limit at Air mastery 3 | Preserve | Allow one/two/three/four casts, charge 300/200 movement, reject excess casts without payment and reset tomorrow. | PASS |  |
| spell-door-range | Enforce the original 9-by-8 teleport radius | Preserve | Reject a clear in-bounds point just outside either axis, accept the extreme legal corner, and allow the last positive movement point. | PASS |  |
| spell-visions-0 | Reveal nearby details with Visions mastery 0 | Adapt | Show exact monsters, add hero armies at Advanced and garrisons at Expert; do not reveal unseen land and expire tomorrow. | PASS |  |
| spell-visions-1 | Reveal nearby details with Visions mastery 1 | Adapt | Show exact monsters, add hero armies at Advanced and garrisons at Expert; do not reveal unseen land and expire tomorrow. | PASS |  |
| spell-visions-2 | Reveal nearby details with Visions mastery 2 | Adapt | Show exact monsters, add hero armies at Advanced and garrisons at Expert; do not reveal unseen land and expire tomorrow. | PASS |  |
| spell-visions-3 | Reveal nearby details with Visions mastery 3 | Adapt | Show exact monsters, add hero armies at Advanced and garrisons at Expert; do not reveal unseen land and expire tomorrow. | PASS |  |
| hero-scouting | Apply native Scouting and only equipped vision artifacts | Preserve | Hero sight is five plus one/two/three from Scouting; Speculum and Spyglass add one each only while worn. | PASS |  |
| mine-share | Let a mine owner share and revoke access | Adapt | Sharing leaves ownership unchanged, pays every city half, and revocation restores the owner's full yield. | PASS |  |
| mine-half-yield | Accumulate fractional rare-resource mine income | Adapt | Every two half-yields produce one spendable unit, with no rounding loss across revocation. | PASS |  |
| mine-share-guards | Reject sharing by a visitor or after game end | Adapt | Only the owner can authorize sharing; unclaimed/non-mine coordinates are not authority. | PASS |  |
| endgame-agreement | Require unanimous human agreement and allow withdrawal | Adapt | A single vote cannot end a three-player game; withdrawal persists and only the final unanimous vote ends it. | PASS |  |
| endgame-ai-agreement | Auto-consent computer seats after all humans agree | Adapt | Computers neither block human unanimity nor cast a human vote. | PASS |  |
| hero-level-choice | Persist and explicitly choose a secondary skill | Adapt | The primary point is immediate; an unoffered skill is rejected and only one offered skill advances. | PASS |  |
| hero-level-queue | Resolve a multi-level experience award in order | Preserve | Every level keeps its own primary point and persisted offer; later choices are not skipped. | PASS |  |
| hero-level-cap | Respect eight skills and retain old combat skills | Adapt | A full eight-skill hero only upgrades supported existing skills; legacy combat skills and the inactive Fire school receive no invented offers. | PASS |  |
| hero-native-progression | Use original class profiles and experience thresholds | Preserve | Class-specific starting stats and native low/high probabilities replace Might/Magic placeholders; the exponential experience curve is retained. | PASS |  |
| hero-obligatory-offers | Offer Wisdom and a magic school at the native class interval | Preserve | Knight Wisdom is offered by level six; magic classes receive a school by level three; obligatory offers never bypass a class's zero learning weight. | PASS |  |
| hero-learning-estates | Apply preserved Learning and Estates effects | Preserve | Learning grants five percent per mastery and Estates pays 125/250/500 daily. | PASS |  |
| army-heroes | Move, split, merge and swap armies between meeting heroes | Preserve | Counts are conserved in both directions; both heroes retain an army. | PASS |  |
| army-heroes-guards | Reject remote, foreign and last-stack hero transfers | Preserve | Only adjacent own heroes can exchange; invalid transfers are atomic. | PASS |  |
| hero-scholar | Share spells when owned heroes meet | Preserve | Scholar respects its level, the recipient's Wisdom and spellbooks; learning works both ways. | PASS |  |
| army-dismiss | Confirm troop dismissal without emptying a hero | Preserve | Unconfirmed, foreign and last-stack dismissal fails; confirmed dismissal removes only its stack. | PASS |  |
| hero-dismiss | Confirm hero dismissal and guard a townless final hero | Preserve | A hero may be dismissed while a town remains, but not without confirmation or as the last townless hero. | PASS |  |
| artifact-equipment | Equip, unequip and swap persistent artifact slots | Preserve | Only worn artifacts apply bonuses; duplicates and reserved gear survive; reducing max mana does not erase stored mana. | PASS |  |
| artifact-exchange | Transfer and swap artifacts between meeting heroes | Preserve | Both inventories and bonuses update atomically; remote or foreign heroes cannot receive artifacts. | PASS |  |
| artifact-backpack | Reorder a backpack and preserve manual layout on pickup | Preserve | Backpack positions persist, an unavailable slot is rejected, and a full backpack cannot consume a map artifact. | PASS |  |
| social-endgame | Reject pending economics after the game ends | Adapt | Trade responses, project contributions and pending creature actions cannot change finished games. | PASS |  |
| social-concurrent-trade | Accept one offer concurrently twice | Adapt | Exactly one acceptance transfers resources; a replay cannot duplicate either side. | PASS |  |
| social-concurrent-project | Complete a project with concurrent contributions | Adapt | Only the contribution completing the project is charged; the completed project cannot be funded twice. | PASS |  |
| social-concurrent-claim | Visit one neutral mine concurrently | Adapt | The first visitor claims it; the second cannot overwrite ownership or gain a second full yield. | PASS |  |
| game-join | Enter an existing seeded-player game | Adapt | Repeated entry returns the same seat; unknown players are rejected without adding seats. | PASS |  |
| recruit-base | Recruit a base creature | Preserve | Exact gold cost and weekly-pool debit; creatures join the garrison. | PASS |  |
| recruit-upgraded | Recruit base and upgraded forms | Preserve | An upgraded dwelling offers both forms from the same pool. | PASS |  |
| recruit-locked | Recruit from an unbuilt dwelling | Preserve | Reject unbuilt dwellings and unavailable upgrades without charging. | PASS |  |
| recruit-capacity | Recruit with full armies | Preserve | Use a matching stack or empty slot; reject when neither army has room, with no charge. | PASS |  |
| recruit-fallback | Recruit with a full garrison | Preserve | A visiting hero with capacity receives recruits only when the garrison has no room. | PASS |  |
| recruit-amount | Recruit an exact requested quantity | Preserve | Reject zero, negative, and over-pool quantities; never silently change the purchase. | PASS |  |
| recruit-poor | Recruit without sufficient resources | Preserve | Reject an unaffordable purchase with no pool or army change. | PASS |  |
| town-ownership | Use another player's town | Adapt | Peaceful visitors cannot recruit, build, exchange armies, upgrade or buy the owner's gear. | PASS |  |
| army-move | Army move | Preserve | Move, combine, swap or split stacks without losing or creating creatures. | PASS |  |
| army-merge | Army merge | Preserve | Move, combine, swap or split stacks without losing or creating creatures. | PASS |  |
| army-swap | Army swap | Preserve | Move, combine, swap or split stacks without losing or creating creatures. | PASS |  |
| army-split | Army split | Preserve | Move, combine, swap or split stacks without losing or creating creatures. | PASS |  |
| army-last | Keep a hero's last troop | Preserve | Reject removal of the final troop and leave both armies unchanged. | PASS |  |
| army-reject-atomic | Reject an illegal army transfer atomically | Preserve | The shared transfer rule must not mutate either army when it returns an error. | PASS |  |
| army-away | Exchange troops while away from town | Preserve | A hero must physically be in town to exchange troops with its garrison. | PASS |  |
| army-rearrange | Rearrange a hero's own army | Preserve | Moving or splitting within the seven slots preserves total creatures. | PASS |  |
| upgrade-price | Upgrade a creature stack | Preserve | The upgraded dwelling is required; charge cost difference times the entire stack. | PASS |  |
| upgrade-locked | Upgrade without an upgraded dwelling | Preserve | Reject without consuming creatures or resources. | PASS |  |
| upgrade-army | Select an army to upgrade | Preserve | Invalid army selectors must be rejected, not interpreted as the garrison. | PASS |  |
| market-trade | Trade minerals at a marketplace | Preserve | One market buys one wood for 2500 gold and sells one wood for 25 gold. | PASS |  |
| market-amount | Validate marketplace quantities | Preserve | Zero, negative, and unaffordable large orders must fail with no resource change. | PASS |  |
| build-once | Build one structure per day | Preserve | Prerequisites and costs apply; a second building on the same day is rejected. | PASS |  |
| build-prerequisites | Respect a structure's prerequisites | Preserve | Reject buildings whose prerequisites are absent without charging resources. | PASS |  |
| dwelling-growth | Advance a dwelling's weekly pool | Preserve | An existing Guardhouse gains exactly 14 Pikemen at the week boundary. | PASS |  |
| town-income | Collect daily town income | Preserve | A Village Hall adds exactly 500 gold at the day boundary. | PASS |  |
| hire-hero | Hire a tavern hero | Preserve | Pay the original 2500 gold, add the chosen hero and remove that offer; 2499 is insufficient. | PASS |  |
| hire-gate | Hire while the visiting gate is occupied | Preserve | Only one visiting hero fits in town; hiring must not spawn a hero outside the town. | PASS |  |
| spellbook-buy | Buy and learn from a Mage Guild | Preserve | A visitor pays 500 gold for a spellbook and learns eligible stocked spells. | PASS |  |
| guild-wisdom | Respect Wisdom when learning guild spells | Preserve | Without Wisdom, learn levels one and two, but not level three. | PASS |  |
| guild-rest | Rest overnight at a Mage Guild | Preserve | A visiting hero with a spellbook regains full spell points after a day. | PASS |  |
| blacksmith | Buy a faction's war machine | Preserve | A present hero pays once; duplicate purchases are rejected. | PASS |  |
| resource-gold | Collect a gold pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| resource-wood | Collect a wood pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| resource-ore | Collect a ore pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| resource-mercury | Collect a mercury pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| resource-sulfur | Collect a sulfur pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| resource-crystal | Collect a crystal pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| resource-gems | Collect a gems pile | Preserve | Credit the exact quantity once and remove the pile. | PASS |  |
| mine-gold | Claim and collect from a gold mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| mine-wood | Claim and collect from a wood mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| mine-ore | Claim and collect from a ore mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| mine-mercury | Claim and collect from a mercury mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| mine-sulfur | Claim and collect from a sulfur mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| mine-crystal | Claim and collect from a crystal mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| mine-gems | Claim and collect from a gems mine | Preserve | Claiming preserves the structure; pay its exact daily yield once. | PASS |  |
| resource-route-stop | Stop a walking route at the first pickup | Preserve | Only the first pile is collected and reported; later route objects remain untouched. | PASS |  |
| flight-landing-pickup-stop | Stop after a flight landing pickup | Preserve | Landing collects its pile but cannot also visit the adjacent structure in the same move. | PASS |  |
| mine-owner-consent | Visit another player's mine | Adapt | A visitor cannot grant themselves ownership or sharing of a claimed mine. | PASS |  |
| treasure-choice | Open a gold treasure chest | Preserve | A chest must not automatically grant both gold and experience. | PASS |  |
| artifact-pickup | Pick up an artifact | Preserve | Remove the map object and place the artifact into its matching slot or backpack. | PASS |  |
| shrine-book | Visit a shrine without a spellbook | Preserve | Do not teach a spell, remove the shrine, or move through it. | PASS |  |
| shrine-wisdom | Learn a shrine's higher-level spell | Preserve | A third-level shrine requires Basic Wisdom as well as a spellbook. | PASS |  |
| encounter-observe | Peaceful creature encounter: observe | Adapt | Resolve the pending encounter; observe/pass preserve the obstacle, recruitment charges and removes it. | PASS |  |
| encounter-recruit | Peaceful creature encounter: recruit | Adapt | Resolve the pending encounter; observe/pass preserve the obstacle, recruitment charges and removes it. | PASS |  |
| encounter-pass | Peaceful creature encounter: pass | Adapt | Resolve the pending encounter; observe/pass preserve the obstacle, recruitment charges and removes it. | PASS |  |
| encounter-strength | Reject an expedition that is too weak | Adapt | Observe needs half the stack's strength; recruit needs full strength. | PASS |  |
| encounter-offer | Load the server-authoritative joining offer | Adapt | Pending encounters expose the actual creature, count, gold price and strength eligibility, without changing gold or position. | PASS |  |
| encounter-offer-weak | Load a non-joining creature encounter | Adapt | The offer's eligibility uses the same strength rule as recruitment; acknowledging it leaves the creature solid. | PASS |  |
| encounter-offer-legacy | Read absent or malformed legacy encounter metadata | Adapt | Hero queries remain usable and omit an offer for malformed, empty, unknown or absent encounter state. | PASS |  |
| encounter-merge-full | Accept joining creatures with seven occupied matching stacks | Adapt | A full army can merge joining creatures into its matching stack and pays the quoted price exactly once. | PASS |  |
| obelisk-visit | Read obelisks by walking up to them | Adapt | Each obelisk counts once per player; reading every obelisk marks the Grail tile; the board shows a terrain puzzle centred on the Grail and the notes left there. | PASS |  |
| town-claim-neutral | Enter an unowned town | Adapt | The first hero to enter an unowned town claims it for his player; another player's town can be visited but never taken. | PASS |  |
| grail-dig | Dig the revealed Grail | Adapt | Award the Grail, remove its tile and spend remaining movement; do not end the game. | PASS |  |
| grail-build | Build a Grail structure | Preserve | Require and consume the Grail when constructing its unique structure. | PASS |  |
| native-creature-reference | Compare all 126 creatures with original numerical tables | Preserve | Every retained creature matches native recruitment resources, statistics, growth and its upgrade price difference. | PASS |  |
| native-building-reference | Compare every supported purchasable building with BUILDING.TXT | Preserve | All three towns' guilds, halls, fortifications, dwellings, upgrades and specials use original seven-resource prices. | PASS |  |
| special-growth-reference | Apply every supported horde, fortification and Grail growth bonus | Preserve | The Grail adds half of total growth after fortifications and horde bonuses, including low-growth tier-seven rounding. | PASS |  |
| special-pond | Pay Mystic Pond only at the week boundary | Preserve | Exactly one rare resource gains one to four units; no daily payout or replacement peaceful bonus is invented. | PASS |  |
| special-darkness | Preserve active sight inside Cover of Darkness | Preserve | Re-shroud explored memory within radius twenty, without erasing what a hero currently sees or tiles outside the radius. | PASS |  |
| special-combat-only | Retain combat-only buildings without invented adventure effects | Preserve | Brotherhood, Fountain of Fortune and Necromancy Amplifier do not create gold, creatures, movement or replacement skill bonuses. | PASS |  |
| special-treasury-income | Pay every town's income and opening-balance Treasury interest | Preserve | Two Treasuries each pay ten percent of the same pre-income balance, not compounded interest on today's income. | PASS |  |
| artifact-sight | Refresh visibility immediately when a sight artifact is equipped or removed | Preserve | Only equipped Speculum/Spyglass expands active sight; unequipping retains explored memory but not active visibility. | PASS |  |
| movement-source | Charge the terrain being left, not the destination | Preserve | Leaving snow for grass spends 150; entering snow from grass spends 100. | PASS |  |
| movement-diagonal | Charge native diagonal rounding | Preserve | Diagonal costs are floor(source cost times sqrt(2)): 141/176/212/247. | PASS |  |
| movement-last-step | Allow the final diagonal when a straight move remains | Preserve | 125 movement can pay the last 141-point grass diagonal, consuming the remainder. | PASS |  |
| movement-exhaustion | Exhaust unusable movement after stepping | Preserve | A 50-point remainder on snow cannot fund another straight move and becomes zero. | PASS |  |
| movement-native-terrain | Use army-native terrain, including mixed native factions | Preserve | All stacks must be native to the source terrain; hero class/faction does not grant the exemption. | PASS |  |
| spell-door-water | Apply the original SoD wrong-terrain Dimension Door failure | Preserve | A water target fails to move but spends mana, movement and one daily cast. | PASS |  |
| movement-sand | Move across sand | Preserve | Sand has the same 150-point base terrain cost as snow before Pathfinding. | PASS |  |
| movement-pathfinding | Apply the Pathfinding skill | Preserve | Basic Pathfinding reduces a snow step from 150 to 125 points. | PASS |  |
| hero-level | Gain a hero level | Preserve | Each level grants one primary point and persists its secondary-skill choice rather than choosing for the player. | PASS |  |
| spell-flight | Cast Fly | Preserve | Fly must enable a day-long flight movement mode, not acknowledge an inert spell. | PASS |  |
| spell-door-occupied | Cast Dimension Door onto another hero | Preserve | Reject occupied destinations without spending mana. | PASS |  |
| game-finished | Attempt economic actions after the game ends | Preserve | Building, recruitment, trading, hiring and army actions must not change a finished game. | PASS |  |
| recruit-minerals | Recruit a rare-resource creature | Preserve | An Angel costs 3000 gold and one gem; missing gems reject the purchase. | PASS |  |
| army-invalid | Reject invalid army operations | Preserve | Invalid slots, selectors, negative/oversized counts, and partial swaps leave armies unchanged. | PASS |  |
| army-legacy | Transfer through the garrison shortcut | Preserve | Moving creatures to the garrison and back conserves both stacks. | PASS |  |
| upgrade-garrison | Upgrade the town garrison | Preserve | Upgrading retains count and charges the exact price difference. | PASS |  |
| build-new-pool | Build a new dwelling | Preserve | A new Guardhouse makes one base week's supply immediately available. | PASS |  |
| growth-citadel | Advance a week with a citadel | Preserve | Fortifications increase weekly Pikeman growth without discarding the existing pool. | PASS |  |
| growth-castle | Advance a week with a castle | Preserve | Fortifications increase weekly Pikeman growth without discarding the existing pool. | PASS |  |
| hire-limits | Validate tavern hiring | Preserve | Reject missing taverns, unknown offers, insufficient gold, and an eight-hero roster. | PASS |  |
| mana-capacity | Calculate a hero's spell-point capacity | Preserve | Knowledge grants ten points each; Intelligence adds 25/50/100 percent. | PASS |  |
| mana-daily-0 | Regenerate spell points (Mysticism 0) | Preserve | Outside a Mage Guild, recover the skill's daily amount without exceeding capacity. | PASS |  |
| mana-daily-1 | Regenerate spell points (Mysticism 1) | Preserve | Outside a Mage Guild, recover the skill's daily amount without exceeding capacity. | PASS |  |
| mana-daily-2 | Regenerate spell points (Mysticism 2) | Preserve | Outside a Mage Guild, recover the skill's daily amount without exceeding capacity. | PASS |  |
| mana-daily-3 | Regenerate spell points (Mysticism 3) | Preserve | Outside a Mage Guild, recover the skill's daily amount without exceeding capacity. | PASS |  |
| shrine-learn | Learn from a shrine and revisit it | Preserve | An eligible visitor learns once; the structure persists and the hero remains adjacent. | PASS |  |
| mine-repeat | Revisit your own mine | Adapt | A repeat visit neither pays a second daily yield nor removes the mine. | PASS |  |
| encounter-invalid | Reject invalid or unaffordable encounters | Adapt | Unknown choices and unaffordable recruitment retain the obstacle and pending choice. | PASS |  |
| spell-guards | Validate spell casting | Preserve | Unknown/unlearned spells, missing spellbooks and insufficient mana are rejected without cost. | PASS |  |
| spell-door-land | Dimension Door landing validation | Preserve | Solid and out-of-bounds destinations are rejected without spending mana; unexplored clear land remains legal in SoD. | PASS |  |
| stables-week | Visit Castle Stables | Preserve | Gain 400 movement once per week; daily refresh keeps it until the week ends. | PASS |  |
| transform-stack | Use the Skeleton Transformer | Preserve | A living visitor stack becomes the same number of Skeletons; undead cannot be transformed again. | PASS |  |
| tower-post-comment | Publish a journal entry and comment | Adapt | The owner publishes in a built Tower; a comment is stored under the selected post. | PASS |  |
| tower-capacity | Respect a Tower's journal capacity | Adapt | A first-level Tower accepts three entries, then rejects a fourth without inserting it. | PASS |  |
| movement-army-speed | Derive daily movement from the army | Preserve | Replacing the slowest creature with a faster stack changes the next day's base movement budget. | PASS |  |
| spell-haste | Use peaceful Haste | Adapt | The contract adapts Haste into an adventure movement benefit, not an inert combat cast. | PASS |  |
| spell-contract-viewAir | Provide the contract's viewAir utility | Adapt | The contract's utility must exist and must not be acknowledged as an inert combat spell. | PASS |  |
| spell-contract-viewEarth | Provide the contract's viewEarth utility | Adapt | The contract's utility must exist and must not be acknowledged as an inert combat spell. | PASS |  |
| spell-contract-cure | Provide the contract's cure utility | Adapt | The contract's utility must exist and must not be acknowledged as an inert combat spell. | PASS |  |
| treasure-gold | Choose treasure gold | Preserve | Persist the pending choice; grant only the selected reward and reject replay. | PASS |  |
| treasure-experience | Choose treasure experience | Preserve | Persist the pending choice; grant only the selected reward and reject replay. | PASS |  |
| treasure-leave | Leave and revisit a chest | Preserve | Cancellation pays nothing and retains the chest for a later choice. | PASS |  |
| treasure-concurrent | Submit conflicting chest choices concurrently | Preserve | Exactly one gold/experience request succeeds; both rewards can never be paid. | PASS |  |
| flight-cross-town | Fly across town | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-monster | Fly across monster | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-mine | Fly across mine | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-shrine | Fly across shrine | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-obelisk | Fly across obelisk | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-block | Fly across block | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-tree | Fly across tree | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-resource | Fly across resource | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-cross-treasure | Fly across treasure | Preserve | Crossing does not interact, remove, claim or make the object walkable. | PASS |  |
| flight-landings | Validate flight landings | Preserve | Reject unseen, occupied and obstructed landings without spending movement; retain a legal affordable prefix. | PASS |  |
| flight-visit | Land beside and visit a creature | Preserve | Flight can approach a creature but the hero remains on adjacent ground. | PASS |  |
| flight-invalid-route | Reject malformed flight approaches | Preserve | An out-of-bounds approach to a solid destination returns a rejection, not a server error. | PASS |  |
| shrine-legacy-fly | Learn Fly from an old saved shrine | Preserve | Legacy fly5 keys still teach the canonical spell without making the shrine passable. | PASS |  |
| flight-mastery-0 | Fly at Air mastery 0 | Preserve | Mana and diagonal movement use 20/15 mana and 40/20/0 percent movement penalties. | PASS |  |
| flight-mastery-1 | Fly at Air mastery 1 | Preserve | Mana and diagonal movement use 20/15 mana and 40/20/0 percent movement penalties. | PASS |  |
| flight-mastery-2 | Fly at Air mastery 2 | Preserve | Mana and diagonal movement use 20/15 mana and 40/20/0 percent movement penalties. | PASS |  |
| flight-mastery-3 | Fly at Air mastery 3 | Preserve | Mana and diagonal movement use 20/15 mana and 40/20/0 percent movement penalties. | PASS |  |
| movement-speed-table | Use the slowest stack and Logistics | Preserve | A mixed army uses speed 4 = 1560; speed 12 caps at 2000, with 10/20/30 percent Logistics. | PASS |  |
| haste-day-0 | Apply Haste at Air mastery 0 | Adapt | Grant 300/500 once, persist it, reject recasting without spending mana, and expire tomorrow. | PASS |  |
| haste-day-1 | Apply Haste at Air mastery 1 | Adapt | Grant 300/500 once, persist it, reject recasting without spending mana, and expire tomorrow. | PASS |  |
| haste-day-2 | Apply Haste at Air mastery 2 | Adapt | Grant 300/500 once, persist it, reject recasting without spending mana, and expire tomorrow. | PASS |  |
| haste-day-3 | Apply Haste at Air mastery 3 | Adapt | Grant 300/500 once, persist it, reject recasting without spending mana, and expire tomorrow. | PASS |  |
| cure-penalty | Cure a negative adventure effect | Adapt | Remove the penalty without refilling movement, and preserve beneficial Fly/Haste effects. | PASS |  |
| viewAir-mastery-0 | Survey with viewAir mastery 0 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewAir-mastery-1 | Survey with viewAir mastery 1 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewAir-mastery-2 | Survey with viewAir mastery 2 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewAir-mastery-3 | Survey with viewAir mastery 3 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewEarth-mastery-0 | Survey with viewEarth mastery 0 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewEarth-mastery-1 | Survey with viewEarth mastery 1 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewEarth-mastery-2 | Survey with viewEarth mastery 2 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |
| viewEarth-mastery-3 | Survey with viewEarth mastery 3 | Preserve | Reveal only the mastery-appropriate locations and terrain, without changing exploration. | PASS |  |

## Original 19 requirements: evidence-driven closure

These are not silently counted as passes. Requirements absent from the current contract (for example ships, underground maps, campaign play) are not added as new implementation scope.

| ID | Action / requirement | Rule | Status | Evidence / limitation |
| --- | --- | --- | --- | --- |
| army-hero-exchange | Exchange armies between two friendly heroes | Preserve | PASS | army-heroes, army-heroes-guards, hero-scholar, browser-S33, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| army-dismiss | Dismiss a troop or hero with confirmation | Preserve | PASS | army-dismiss, hero-dismiss, browser-S35, browser-S36, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| artifact-manage | Equip, unequip, exchange and rearrange artifacts | Preserve | PASS | artifact-equipment, artifact-exchange, artifact-backpack, artifact-sight, browser-S33, browser-S34, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| hero-level-choice | Choose secondary skills at level-up | Preserve | PASS | hero-level-choice, hero-level-queue, hero-level-cap, browser-S37, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| hero-skill-fidelity | Apply every preserved secondary skill and class progression | Preserve | PASS | hero-native-progression, hero-obligatory-offers, hero-learning-estates, hero-scholar, hero-scouting, hero-level-cap, movement-native-terrain, movement-speed-table, mana-capacity, mana-daily-0, mana-daily-1, mana-daily-2, mana-daily-3, guild-wisdom. All named evidence passed for the recorded peaceful-ruleset scope. |
| movement-fidelity | Match all terrain and daily movement rules | Preserve | PASS | movement-source, movement-diagonal, movement-last-step, movement-exhaustion, movement-native-terrain, movement-pathfinding, movement-speed-table, flight-mastery-0, flight-mastery-1, flight-mastery-2, flight-mastery-3, movement, browser-S18, browser-S26, browser-S30, browser-S31, browser-S32. All named evidence passed for the recorded peaceful-ruleset scope. |
| mine-sharing | Owner grants or revokes peaceful mine sharing | Adapt | PASS | mine-share, mine-half-yield, mine-share-guards, browser-S41, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| spell-fidelity | Match utility spell definitions and mastery | Preserve | PASS | spell-native-metadata, spell-guards, flight-mastery-0, flight-mastery-1, flight-mastery-2, flight-mastery-3, haste-day-0, haste-day-1, haste-day-2, haste-day-3, cure-penalty, browser-S19, browser-S20, browser-S39. All named evidence passed for the recorded peaceful-ruleset scope. |
| spell-visions | Visions exposes useful creature information | Adapt | PASS | spell-visions-0, spell-visions-1, spell-visions-2, spell-visions-3, browser-S40, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| spell-portal | Town Portal follows mastery-dependent destination/movement rules | Preserve | PASS | spell-portal-0, spell-portal-1, spell-portal-2, spell-portal-3, spell-portal-guards, browser-S38. All named evidence passed for the recorded peaceful-ruleset scope. |
| spell-door | Dimension Door respects range, daily limits and movement costs | Preserve | PASS | spell-door-limit-0, spell-door-limit-1, spell-door-limit-2, spell-door-limit-3, spell-door-range, spell-door-water, spell-door-occupied, spell-door-land, browser-S39, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| spell-removed | Remove damage spells from the peaceful action surface | Remove | PASS | spell-removed, browser-S39. All named evidence passed for the recorded peaceful-ruleset scope. |
| specials-complete | Verify every faction special and all dwelling/upgrade costs | Preserve | PASS | native-creature-reference, native-building-reference, special-growth-reference, special-pond, special-darkness, special-combat-only, special-treasury-income, stables-week, transform-stack, browser-S44. All named evidence passed for the recorded peaceful-ruleset scope. |
| social-meeting | Open conversation/trade/map sharing by meeting another hero | Adapt | PASS | social, browser-S42. All named evidence passed for the recorded peaceful-ruleset scope. |
| social-endgame | Reject resource-changing social actions after game end | Adapt | PASS | social-endgame, game-finished. All named evidence passed for the recorded peaceful-ruleset scope. |
| social-concurrency | Concurrent trade/claim/project requests remain atomic | Adapt | PASS | social-concurrent-trade, social-concurrent-project, social-concurrent-claim, treasure-concurrent. All named evidence passed for the recorded peaceful-ruleset scope. |
| endgame-agreement | End the game by unanimous agreement | Adapt | PASS | endgame-agreement, endgame-ai-agreement, browser-S43, save-migration. All named evidence passed for the recorded peaceful-ruleset scope. |
| ui-actions | Exercise every action through its actual screen | Preserve | PASS | browser, browser-S5, browser-S7, browser-S8, browser-S9b, browser-S10, browser-S12, browser-S15, browser-S17, browser-S21, browser-S22, browser-S23, browser-S24, browser-S27, browser-S28, browser-S29, browser-S31, browser-S32, browser-S33, browser-S34, browser-S35, browser-S36, browser-S37, browser-S38, browser-S39, browser-S40, browser-S41, browser-S42, browser-S43, browser-S44, browser-S45. All named evidence passed for the recorded peaceful-ruleset scope. |
| save-resume | Save/resume every new action state | Preserve | PASS | save-migration, browser-S17, browser-S27, browser-S32, browser-S34, browser-S37. All named evidence passed for the recorded peaceful-ruleset scope. |

## Browser execution record

| Evidence group | Actual screen assertion | Result |
| --- | --- | --- |
| browser-S1 | S1 landing shows three players | PASS |
| browser-S2 | S2 choosing a player persists across reload | PASS |
| browser-S3 | S3 adventure map uses HoMM3 composition | PASS |
| browser-S4 | S4 hero status window shows army and movement | PASS |
| browser-S5 | S5 first click previews, second click moves the hero | PASS |
| browser-S6 | S6 town screen is the original HoMM3 scene | PASS |
| browser-S7 | S7 build a dwelling from the town screen | PASS |
| browser-S8 | S8 recruit creatures | PASS |
| browser-S9 | S9 hero screen shows stats and army | PASS |
| browser-S9a | S9a artifacts sit in their own paper-doll slot | PASS |
| browser-S9b | S9b tavern shows hero details and hires | PASS |
| browser-S9c | S9c walking into your town opens the town screen | PASS |
| browser-S10 | S10 obelisk board shows the puzzle map and notes | PASS |
| browser-S10b | S10b a hero claims an unowned town by entering it | PASS |
| browser-S11 | S11 social screen shows players and trade form | PASS |
| browser-S12 | S12 send a message to another player | PASS |
| browser-S13 | S13 projects screen renders | PASS |
| browser-S14 | S14 tower shows the journal | PASS |
| browser-S15 | S15 wall accepts a message | PASS |
| browser-S17 | S17 treasure offers exactly one reward and survives reload | PASS |
| browser-S18 | S18 Fly crosses an obstacle using the actual map controls | PASS |
| browser-S19 | S19 Haste does not stack and Cure does not refill movement | PASS |
| browser-S20 | S20 survey spells show hidden locations without exploring them | PASS |
| browser-S21 | S21 wood uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S21 | S21 ore uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S21 | S21 mercury uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S21 | S21 sulfur uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S21 | S21 crystal uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S21 | S21 gems uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S21 | S21 gold uses a visit cursor, previews without collecting, and reports in the info bar | PASS |
| browser-S22 | S22 wood mine visits without entering its physical body | PASS |
| browser-S22 | S22 ore mine visits without entering its physical body | PASS |
| browser-S22 | S22 mercury mine visits without entering its physical body | PASS |
| browser-S22 | S22 sulfur mine visits without entering its physical body | PASS |
| browser-S22 | S22 crystal mine visits without entering its physical body | PASS |
| browser-S22 | S22 gems mine visits without entering its physical body | PASS |
| browser-S22 | S22 gold mine visits without entering its physical body | PASS |
| browser-S23 | S23 shrine magicArrow has its body on the visit tile and never lets the hero walk through | PASS |
| browser-S23 | S23 shrine viewAir has its body on the visit tile and never lets the hero walk through | PASS |
| browser-S23 | S23 shrine visions has its body on the visit tile and never lets the hero walk through | PASS |
| browser-S23 | S23 shrine animateDead has its body on the visit tile and never lets the hero walk through | PASS |
| browser-S24 | S24 artifact previews a visit and executes its own interaction | PASS |
| browser-S24 | S24 treasure previews a visit and executes its own interaction | PASS |
| browser-S24 | S24 monster previews a visit and executes its own interaction | PASS |
| browser-S24 | S24 obelisk previews a visit and executes its own interaction | PASS |
| browser-S24 | S24 grail previews a visit and executes its own interaction | PASS |
| browser-S25 | S25 clicking the solid town body plans a gate visit and enters only after confirmation | PASS |
| browser-S26 | S26 insufficient movement uses next-day visit cursor and the actual red destination cross | PASS |
| browser-S27 | S27 creature offer uses native Yes/No, survives reload and joins | PASS |
| browser-S27 | S27 creature offer uses native Yes/No, survives reload and declines without moving through | PASS |
| browser-S28 | S28 non-joining creatures have a single native acknowledgment and stay solid | PASS |
| browser-S29 | S29 creature offer gold failure uses a native error dialog without consuming the offer | PASS |
| browser-S29 | S29 creature offer army failure uses a native error dialog without consuming the offer | PASS |
| browser-S30 | S30 movement exhaustion preserves the route and continues next day without a popup | PASS |
| browser-S31 | S31 end-turn reminder can cancel; day/week feedback uses the original info-bar animations | PASS |
| browser-S32 | S32 sleep persists without ending the turn; explicit wake and movement wake the hero | PASS |
| browser-S33 | S33 meeting owned heroes opens native exchange and transfers split armies and artifacts | PASS |
| browser-S34 | S34 hero equipment supports unequip, replacement and native incompatible-slot errors | PASS |
| browser-S35 | S35 troop details precede dismissal and confirmations preserve the final stack | PASS |
| browser-S36 | S36 dismissing a hero requires native confirmation | PASS |
| browser-S37 | S37 native level choices survive reload and resolve multiple levels in sequence | PASS |
| browser-S38 | S38 Advanced Town Portal uses the native-style destination selector and retains movement | PASS |
| browser-S39 | S39 native spellbook hides combat casts and Dimension Door uses cancelable map targeting | PASS |
| browser-S40 | S40 Visions reveals real creature information through map right-click | PASS |
| browser-S41 | S41 mine ownership grants and revokes half-yield sharing through native confirmations | PASS |
| browser-S42 | S42 physical other-player meetings link messages, map markers and accepted trades | PASS |
| browser-S43 | S43 human unanimity, withdrawal and AI consent navigate both players to game over | PASS |
| browser-S44 | S44 town screens split, merge, upgrade, trade and buy gear using their real controls | PASS |
| browser-S45 | S45 projects accept an actual screen contribution without changing another resource | PASS |
| browser-S16 | S16 two players see the same game | PASS |

## Public mutation inventory

Automatically extracted from all `Api\*.cs` files; adding a route without a mapping makes the audit incomplete. GET/catalogue routes and development-only fixture endpoints are not gameplay mutations and are excluded here. Evidence is named test cases or suite IDs from the next table.

| Method | Route | Evidence | Result |
| --- | --- | --- | --- |
| POST | /api/games | adventure | FAIL |
| POST | /api/games/{id:guid}/join | game-join | PASS |
| POST | /api/heroes/{heroId:guid}/move | movement, movement-pathfinding, resource-gold, shrine-learn, obelisk-visit, town-claim-neutral | PASS |
| POST | /api/heroes/{heroId:guid}/treasure | treasure-gold, treasure-experience, treasure-leave, treasure-concurrent | PASS |
| POST | /api/towns/{townId:guid}/build | build-once, build-prerequisites, build-new-pool, grail-build | PASS |
| POST | /api/towns/{townId:guid}/recruit | recruit-base, recruit-amount, recruit-minerals, recruit-capacity | PASS |
| POST | /api/towns/{townId:guid}/trade | market-trade, market-amount | PASS |
| POST | /api/towns/{townId:guid}/hire | hire-hero, hire-gate, hire-limits | PASS |
| POST | /api/towns/{townId:guid}/garrison | army-legacy | PASS |
| POST | /api/towns/{townId:guid}/army | army-move, army-merge, army-swap, army-split, army-invalid, army-last | PASS |
| POST | /api/heroes/{heroId:guid}/army | army-rearrange | PASS |
| POST | /api/towns/{townId:guid}/upgrade | upgrade-price, upgrade-locked, upgrade-army, upgrade-garrison | PASS |
| POST | /api/towns/{townId:guid}/blacksmith | blacksmith | PASS |
| POST | /api/towns/{townId:guid}/spellbook | spellbook-buy, guild-wisdom | PASS |
| POST | /api/towns/{townId:guid}/transform | transform-stack | PASS |
| POST | /api/heroes/{heroId:guid}/cast | spell-guards, spell-flight, spell-haste, spell-door-occupied, spell-door-land | PASS |
| POST | /api/games/{id:guid}/endturn | dwelling-growth, guild-rest, town-income, game-finished | PASS |
| POST | /api/games/{id:guid}/obelisk/message | obelisk-visit, adventure | FAIL |
| POST | /api/heroes/{heroId:guid}/encounter | encounter-observe, encounter-recruit, encounter-pass, encounter-strength, encounter-invalid | PASS |
| POST | /api/games/{id:guid}/messages | social | PASS |
| POST | /api/games/{id:guid}/trades | social | PASS |
| POST | /api/trades/{offerId:guid}/respond | social | PASS |
| POST | /api/projects/{projectId:guid}/contribute | social | PASS |
| POST | /api/games/{id:guid}/annotations | social | PASS |
| POST | /api/towns/{townId:guid}/posts | tower-post-comment, tower-capacity | PASS |
| POST | /api/posts/{postId:guid}/comments | tower-post-comment | PASS |
| POST | /api/towns/{townId:guid}/wall | social | PASS |
| POST | /api/towns/{townId:guid}/tower | social | PASS |
| POST | /api/heroes/{heroId:guid}/level-up | hero-level-choice, hero-level-queue, hero-level-cap | PASS |
| POST | /api/heroes/{heroId:guid}/exchange | army-heroes, army-heroes-guards, hero-scholar | PASS |
| POST | /api/heroes/{heroId:guid}/dismiss-stack | army-dismiss | PASS |
| POST | /api/heroes/{heroId:guid}/dismiss | hero-dismiss | PASS |
| POST | /api/heroes/{heroId:guid}/artifacts | artifact-equipment, artifact-exchange, artifact-backpack | PASS |
| POST | /api/games/{id:guid}/mine-sharing | mine-share, mine-half-yield, mine-share-guards | PASS |
| POST | /api/games/{id:guid}/agreement | endgame-agreement, endgame-ai-agreement | PASS |

## Suite execution record

| Suite | Result | Duration (ms) | Log / detail |
| --- | --- | --- | --- |
| actions | PASS | 13620 | test-results/action-audit/actions.log |
| movement | PASS | 11611 | test-results/action-audit/movement.log |
| creatures | PASS | 77 | test-results/action-audit/creatures.log |
| creature-sanity | PASS | 72 | test-results/action-audit/creature-sanity.log |
| server-build | PASS | 1807 | test-results/action-audit/server-build.log |
| client-build | PASS | 7696 | test-results/action-audit/client-build.log |
| isolated-server | PASS |  | Ephemeral port, temporary SQLite database, readiness verified. |
| adventure | FAIL | 74 | test-results/action-audit/adventure.log |
| social | PASS | 1207 | test-results/action-audit/social.log |
| browser | PASS | 127672 | test-results/action-audit/browser.log |
| save-migration | PASS | 13461 | test-results/action-audit/save-migration.log |

## References and limits

The authoritative project rules are in `00-FIDELITY-CONTRACT.md`. Original rules were compared with the 1999 player manual: adventure interactions (pp. 13–22), heroes and skills (pp. 23–39), town development/recruiting/guilds (pp. 49–54), and spell reference (pp. 63–80). The manual is a primary reference, not a replacement for SoD-specific executable checks.

Manual distribution and searchable transcription:

```text
https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/297000/manuals/BONUS_Heroes_of_Might_and_Magic_III_HDEdition_OldManual1999_EN.pdf
https://device.report/m/94ee8e7055ce6a4f1f8f920ebd8e230c1fcf319f564890831ff6c5fbbae1f9d8
```

Primary implementation references for pointer frames/hotspots, the ADAG end cross,
and native resource information-bar presentation (not a claim that the full VCMI
ruleset is implemented):

```text
https://github.com/vcmi/vcmi/blob/develop/config/cursors.json
https://github.com/vcmi/vcmi/blob/develop/client/mapView/MapRenderer.cpp
https://github.com/vcmi/vcmi/blob/develop/lib/mapObjects/CGResource.cpp
https://github.com/vcmi/vcmi/blob/develop/client/adventureMap/CInfoBar.cpp
https://github.com/vcmi/vcmi/blob/develop/client/adventureMap/AdventureMapShortcuts.cpp
https://github.com/vcmi/vcmi/blob/develop/client/PlayerLocalState.cpp
https://github.com/vcmi/vcmi/blob/develop/client/HeroMovementController.cpp
https://github.com/vcmi/vcmi/blob/develop/lib/mapObjects/CGCreature.cpp
```

The original 63-case baseline was 47 PASS / 16 FAIL. This expanded record supersedes older blanket all-green claims in the requirements matrix. The baseline identified invalid recruitment/trades, transfer mutation on rejection, invalid upgrade selectors, occupied hiring, missing overnight mana, unauthorized mine sharing, treasure double rewards, shrine Wisdom, unconsumed Grail, sand/Pathfinding costs, incomplete level advancement, inert Fly, occupied teleport landing, and finished-game economics.

## Audited worktree

```text
M client/src/api.ts
 M client/src/i18n.ts
 M client/src/types.ts
 M client/src/views/AdventureMap.tsx
 M client/src/views/Obelisk.tsx
 M client/src/views/Social.tsx
 M docs/08-requirements-matrix.md
 M e2e/game.spec.ts
 M server/CityOfTowers/Api/Endpoints.cs
 M server/CityOfTowers/Domain/AdventureMap.cs
 M server/CityOfTowers/Domain/Entities.cs
 M server/CityOfTowers/Program.cs
 M server/CityOfTowers/Services/GameService.cs
 M test/MovementChecks/ActionChecks.cs
 M test/MovementChecks/Program.cs
 M test/actions.js
 M test/adventure.js
?? client/src/h3/puzzle-map.json
```
