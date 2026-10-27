import { useEffect, useState, useCallback, useMemo, useRef } from 'react';
import { useParams } from 'react-router-dom';
import { useGameNavigate } from '../useGameNavigate';
import { api, ApiError } from '../api';
import type { TownDetail, BuildingDef, CreatureDef, HeroDef, ArmySlot, SpellDef } from '../types';
import { Btn, Loading, Modal, Panel, Txt } from '../ui/Layout';
import { toast } from '../ui/Toast';
import { FACTION_LABEL, RES_ORDER, ResIcon, buildingName } from '../ui/homm';
import { DefImg, Pcx, ScreenBackdrop, alphaAt, drawFrame, getDef, onAssetLoaded, useH3 } from '../h3/assets';
import { creaturePortraitFrame, heroPortrait, townListFrame } from '../h3/catalog';
import { TOWN_BACK, townStructures } from '../h3/town';

const HERO_COST = 2000;

const SKILL_NAMES: Record<string, string> = {
  pathfinding: 'Pathfinding', archery: 'Archery', logistics: 'Logistics', scouting: 'Scouting',
  diplomacy: 'Diplomacy', navigation: 'Navigation', leadership: 'Leadership', luck: 'Luck',
  ballistics: 'Ballistics', eagleEye: 'Eagle Eye', estates: 'Estates', wisdom: 'Wisdom',
  mysticism: 'Mysticism', intelligence: 'Intelligence', sorcery: 'Sorcery', resistance: 'Resistance',
  firstAid: 'First Aid', artillery: 'Artillery', learning: 'Learning', tactics: 'Tactics',
  armorer: 'Armorer', air: 'Air Magic', earth: 'Earth Magic', fire: 'Fire Magic', water: 'Water Magic'
};
const skillName = (k: string) => SKILL_NAMES[k] ?? k;

/** Asset file for a building key; Mage Guild levels reuse the magic tower art. */
function assetFor(key: string): string {
  const m = /^mageGuild(\d)$/.exec(key);
  if (m) return `/assets/buildings/magicTower${m[1]}.svg`;
  const u = /^upg(\d)$/.exec(key);
  return `/assets/buildings/${u ? 'dwelling' + u[1] : key}.svg`;
}

type Tab = 'build' | 'recruit' | 'market' | 'guild' | 'tavern' | 'garrison' | 'tower' | 'wall';

export function TownScreen() {
  const { gameId, townId } = useParams<{ gameId: string; townId: string }>();
  const navigate = useGameNavigate();
  const [town, setTown] = useState<TownDetail | null>(null);
  const [buildings, setBuildings] = useState<BuildingDef[]>([]);
  const [creatures, setCreatures] = useState<Record<string, CreatureDef>>({});
  const [tab, setTab] = useState<Tab | null>(null);

  const refresh = useCallback(async () => {
    if (!townId) return;
    try { setTown(await api.getTown(townId)); }
    catch (e) { if (e instanceof ApiError) toast(e.message); }
  }, [townId]);

  useEffect(() => {
    refresh();
    api.getBuildings().then(setBuildings).catch(() => {});
    api.getCreatures().then((cs) => {
      const m: Record<string, CreatureDef> = {};
      for (const c of cs) m[c.key] = c;
      setCreatures(m);
    }).catch(() => {});
  }, [refresh]);

  if (!town) return <Loading />;
  return (
    <>
      <TownH3 town={town} buildings={buildings} creatures={creatures}
        onTab={setTab} onExit={() => navigate(`/game/${gameId}`)} />
      {panels(town, buildings, creatures, tab, setTab, refresh)}
    </>
  );
}

function panels(town: TownDetail, buildings: BuildingDef[], creatures: Record<string, CreatureDef>,
  tab: Tab | null, setTab: (t: Tab | null) => void, refresh: () => void) {
  const built = new Set(town.buildings);
  return (
    <>
      {tab === 'build' && <BuildPanel town={town} buildings={buildings} built={built} onBuilt={refresh} onClose={() => setTab(null)} />}
      {tab === 'recruit' && <RecruitPanel town={town} creatures={creatures} onRecruited={refresh} onClose={() => setTab(null)} />}
      {tab === 'market' && <MarketPanel town={town} onTraded={refresh} onClose={() => setTab(null)} />}
      {tab === 'guild' && <GuildPanel town={town} onClose={() => setTab(null)} />}
      {tab === 'tavern' && <TavernPanel town={town} onHired={refresh} onClose={() => setTab(null)} />}
      {tab === 'garrison' && <GarrisonPanel town={town} creatures={creatures} onChanged={refresh} onClose={() => setTab(null)} />}
      {tab === 'tower' && <TowerPanel town={town} onClose={() => setTab(null)} />}
      {tab === 'wall' && <WallPanel town={town} onClose={() => setTab(null)} />}
    </>
  );
}

const lastBuilt = (chain: string[], built: Set<string>) => chain.reduce((acc, k, i) => (built.has(k) ? i : acc), -1);
const RES_X: Record<string, number> = { wood: 37, mercury: 121, ore: 205, sulfur: 289, crystal: 373, gems: 457, gold: 541 };

/** Which panel a clicked structure opens. */
const KEY_TAB: Record<string, Tab> = {
  fort: 'garrison', citadel: 'garrison', castle: 'garrison',
  dwelling1: 'recruit', dwelling2: 'recruit', dwelling3: 'recruit', dwelling4: 'recruit',
  dwelling5: 'recruit', dwelling6: 'recruit', dwelling7: 'recruit',
  grail: 'build', resourceSilo: 'market', marketplace: 'market',
  mageGuild1: 'guild', mageGuild2: 'guild', mageGuild3: 'guild', mageGuild4: 'guild', mageGuild5: 'guild',
  blacksmith: 'build', tavern: 'tavern',
  villageHall: 'build', townHall: 'build', cityHall: 'build', capitol: 'build'
};
const tabFor = (key: string): Tab => KEY_TAB[key] ?? KEY_TAB[key.replace(/^upg/, 'dwelling')] ?? 'build';

/** HoMM3 town screen: the painted 800×374 scene with the original structures, over the townscrn.pcx panel. */
function TownH3(props: {
  town: TownDetail; buildings: BuildingDef[]; creatures: Record<string, CreatureDef>;
  onTab: (t: Tab) => void; onExit: () => void;
}) {
  const { town, buildings, creatures, onTab, onExit } = props;
  const canvas = useRef<HTMLCanvasElement>(null);
  const [tick, setTick] = useState(0);
  const [, setLoaded] = useState(0);
  const [hover, setHover] = useState<string | null>(null);
  const [rollover, setRollover] = useState<string | null>(null);
  const built = useMemo(() => new Set(town.buildings), [town.buildings]);
  const structures = useMemo(() => townStructures(town.faction, built), [town.faction, built]);
  const name = (k: string) => buildingName(k, town.faction, buildings.find((b) => b.key === k)?.name);

  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 150);
    const off = onAssetLoaded(() => setLoaded((n) => n + 1));
    return () => { clearInterval(id); off(); };
  }, []);

  useEffect(() => {
    const ctx = canvas.current?.getContext('2d');
    if (!ctx) return;
    ctx.clearRect(0, 0, 800, 374);
    for (const { key, s } of structures) {
      const sheet = getDef(s[0]);
      if (!sheet) continue;
      ctx.filter = key === hover ? 'brightness(1.35)' : 'none';
      // only the animated extras cycle frames; buildings are static, as in HoMM3
      const frame = key.startsWith('extra') ? tick % sheet.count : 0;
      drawFrame(ctx, sheet, frame, s[1], s[2]);
    }
    ctx.filter = 'none';
  });

  function hit(e: React.MouseEvent): string | null {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - r.left, y = e.clientY - r.top;
    for (let i = structures.length - 1; i >= 0; i--) {
      const { key, s } = structures[i];
      if (key.startsWith('extra')) continue;
      const sheet = getDef(s[0]);
      if (!sheet) continue;
      // must match the frame the renderer drew, or the hit test samples the wrong pixels
      const frame = key.startsWith('extra') ? tick % sheet.count : 0;
      if (alphaAt(sheet, frame, x - s[1], y - s[2]) > 160) return key;
    }
    return null;
  }

  const hallLevel = Math.max(0, lastBuilt(['villageHall', 'townHall', 'cityHall', 'capitol'], built));
  const fortLevel = lastBuilt(['fort', 'citadel', 'castle'], built);
  const growth = town.available.filter((a) => a.creature);
  const garrison = (town.garrison ?? []).slice(0, 7);
  const label = (text: string) => setRollover(text);

  return (
    <>
      <div className="town-scene" style={{ position: 'absolute', left: 0, top: 0, width: 800, height: 374, cursor: hover ? 'pointer' : 'default' }}
        onMouseMove={(e) => { const k = hit(e); setHover(k); setRollover(k ? name(k) : null); }}
        onMouseLeave={() => { setHover(null); setRollover(null); }}
        onClick={(e) => { const k = hit(e); if (k) onTab(town.isOwner ? tabFor(k) : 'tower'); }}>
        <Pcx name={TOWN_BACK[town.faction] ?? 'tbcsback'} x={0} y={0} alt={`${FACTION_LABEL[town.faction]} town`} />
        <canvas ref={canvas} width={800} height={374} style={{ position: 'absolute', left: 0, top: 0 }} />
        {structures.filter((s) => !s.key.startsWith('extra')).map(({ key, s }) => (
          <button key={key} className="abs h3hit" data-building-slot={key} aria-label={`Open ${name(key)}`}
            style={{ left: s[1], top: s[2], width: 1, height: 1, opacity: 0 }}
            onClick={(e) => { e.stopPropagation(); onTab(town.isOwner ? tabFor(key) : 'tower'); }} />
        ))}
      </div>

      <Pcx name="townscrn" x={0} y={374} />
      {/* town icon, name, hall + fort levels, income */}
      <DefImg name="ITPT" frame={townListFrame(town.faction) + (town.builtThisTurn ? 1 : 0)} x={15} y={387} />
      <div className="abs h3text" style={{ left: 79, top: 389, width: 147, textAlign: 'center', fontSize: 13 }}>{town.name}</div>
      <button className="abs h3hit" style={{ left: 80, top: 413, width: 38, height: 38 }} aria-label="Hall"
        onMouseEnter={() => label(name(['villageHall', 'townHall', 'cityHall', 'capitol'][hallLevel]))} onMouseLeave={() => setRollover(null)}
        onClick={() => onTab(town.isOwner ? 'build' : 'tower')}>
        <DefImg name="ITMTL" frame={hallLevel} x={0} y={0} />
      </button>
      <button className="abs h3hit" style={{ left: 122, top: 413, width: 38, height: 38 }} aria-label="Fort"
        onMouseEnter={() => label(fortLevel >= 0 ? name(['fort', 'citadel', 'castle'][fortLevel]) : 'No fort')} onMouseLeave={() => setRollover(null)}
        onClick={() => onTab(town.isOwner ? 'garrison' : 'tower')}>
        <DefImg name="ITMCL" frame={fortLevel >= 0 ? fortLevel : 3} x={0} y={0} />
      </button>
      <div className="abs h3text" title={`Creature growth ×${town.growthMultiplier}`}
        style={{ left: 165, top: 435, width: 60, textAlign: 'center', fontSize: 12 }}>{town.goldIncome}</div>

      {/* available creatures — clicking one opens the recruit window */}
      {growth.map((a, i) => (
        <button key={a.tier} className="abs h3hit" data-creature={a.tier}
          title={`${creatures[a.creature!]?.name ?? a.creature}: ${a.count} available`}
          style={{ left: 14 + 55 * (i % 4), top: 459 + 46 * Math.floor(i / 4), width: 48, height: 32 }}
          onMouseEnter={() => label(`${creatures[a.creature!]?.name ?? a.creature}: ${a.count} available`)} onMouseLeave={() => setRollover(null)}
          onClick={() => onTab(town.isOwner ? 'recruit' : 'tower')}>
          <DefImg name="CPRSMALL" frame={creaturePortraitFrame(a.creature!)} x={8} y={0} />
          <div className="abs h3text" style={{ right: 1, bottom: -1, fontSize: 10 }}>{a.count}</div>
        </button>
      ))}

      {/* garrison row */}
      {garrison.map((g, i) => g && g.creature ? (
        <div key={i} className="abs" style={{ left: 305 + 62 * i, top: 387, width: 58, height: 64 }}
          onMouseEnter={() => label(`${creatures[g.creature!]?.name ?? g.creature} ×${g.count}`)} onMouseLeave={() => setRollover(null)}>
          <DefImg name="TWCRPORT" frame={creaturePortraitFrame(g.creature)} x={0} y={0} />
          <div className="abs h3text" style={{ right: 3, bottom: 1, fontSize: 11 }}>{g.count}</div>
        </div>
      ) : null)}

      {/* town list + exit */}
      <DefImg name="ITPA" frame={townListFrame(town.faction)} x={747} y={430} />
      <button className="abs h3hit" style={{ left: 744, top: 544, width: 48, height: 30 }} aria-label="Exit" onClick={onExit}
        onMouseEnter={() => label('Exit')} onMouseLeave={() => setRollover(null)} />

      {/* the town's visitor board (Tower) and its fortifications (Wall) */}
      <button className="abs h3hit" style={{ left: 747, top: 430, width: 48, height: 48 }} aria-label="Tower"
        title="Magic Tower — the town's visitor board"
        onMouseEnter={() => label('Magic Tower')} onMouseLeave={() => setRollover(null)}
        onClick={() => onTab('tower')} />
      <button className="abs h3hit" style={{ left: 747, top: 482, width: 48, height: 48 }} aria-label="Wall"
        title="City Wall — leave a message"
        onMouseEnter={() => label('City Wall')} onMouseLeave={() => setRollover(null)}
        onClick={() => onTab('wall')} />

      <div className="abs h3text" style={{ left: 7, top: 556, width: 737, height: 18, textAlign: 'center', fontSize: 12, lineHeight: '18px' }}>
        {rollover ?? `${FACTION_LABEL[town.faction]} · ${town.ownerName}${town.isOwner ? '' : ' · visiting'}`}
      </div>
      <Pcx name="AResBar" x={3} y={575} />
      {RES_ORDER.map((r) => (
        <div key={r} className="abs h3text" title={r} style={{ left: 3 + RES_X[r], top: 578, fontSize: 12 }}>{town.resources[r]}</div>
      ))}
    </>
  );
}

/** HoMM3 Hall of the town: one card per building chain, showing the next structure to raise. */
const CHAINS: string[][] = [
  ['villageHall', 'townHall', 'cityHall', 'capitol'],
  ['fort', 'citadel', 'castle'],
  ['mageGuild1', 'mageGuild2', 'mageGuild3', 'mageGuild4', 'mageGuild5'],
  ['tavern'], ['blacksmith'], ['marketplace'], ['resourceSilo'], ['grail'],
  ['dwelling1', 'upg1'], ['dwelling2', 'upg2'], ['dwelling3', 'upg3'], ['dwelling4', 'upg4'],
  ['dwelling5', 'upg5'], ['dwelling6', 'upg6'], ['dwelling7', 'upg7']
];

type CardState = 'built' | 'ready' | 'locked' | 'poor';
const STATE_COLOR: Record<CardState, string> = {
  built: '#8a6c14', ready: '#2a7a2a', locked: '#7a1a14', poor: '#7a4a14'
};

function BuildPanel(props: {
  town: TownDetail; buildings: BuildingDef[]; built: Set<string>;
  onBuilt: () => void; onClose: () => void;
}) {
  const { town, buildings, built, onBuilt, onClose } = props;
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const res = town.resources;
  const byKey = new Map(buildings.map((b) => [b.key, b]));
  const sel = (selected && byKey.get(selected)) || null;
  const name = (b: BuildingDef) => buildingName(b.key, town.faction, b.name);

  const canAfford = (b: BuildingDef) => RES_ORDER.every((r) => res[r] >= b.cost[r]);
  const prereqMet = (b: BuildingDef) => b.requires.every((r) => built.has(r));
  const stateOf = (b: BuildingDef): CardState =>
    built.has(b.key) ? 'built' : !prereqMet(b) ? 'locked' : !canAfford(b) ? 'poor' : 'ready';

  // one card per chain: the next structure to raise, or the top tier once the chain is complete
  const cards = CHAINS
    .map((chain) => chain.map((k) => byKey.get(k)).filter((b): b is BuildingDef => !!b))
    .filter((chain) => chain.length > 0)
    .map((chain) => chain.find((b) => !built.has(b.key)) ?? chain[chain.length - 1]);

  const builtCount = buildings.filter((b) => built.has(b.key)).length;

  async function doBuild() {
    if (!sel) return;
    setBusy(true);
    try {
      await api.build(town.id, sel.key);
      toast('Built ' + name(sel) + '.');
      onBuilt();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    } finally { setBusy(false); }
  }

  return (
    <Modal w={780} h={570} onClose={onClose}>
      <Txt x={0} y={8} w={780} size={16} bold align="center" className="heading">HALL OF {town.name.toUpperCase()}</Txt>
      <Txt x={0} y={30} w={780} size={10} align="center" color="var(--text-dim)">
        {builtCount} structures built · {town.builtThisTurn ? 'already built this turn' : 'one structure per turn'}
      </Txt>

      {cards.map((b, i) => {
        const st = stateOf(b);
        return (
          <div key={b.key} data-building={b.key} className={'card' + (selected === b.key ? ' selected' : '')}
            title={name(b)}
            style={{ left: 14 + (i % 5) * 152, top: 48 + Math.floor(i / 5) * 116, width: 144, height: 110 }}
            onClick={() => setSelected(b.key)}>
            <div className="frame" />
            <img src={assetFor(b.key)} alt="" width={120} height={84}
              style={{ position: 'absolute', left: 12, top: 3, objectFit: 'contain' }}
              onError={(e) => { (e.currentTarget as HTMLImageElement).style.visibility = 'hidden'; }} />
            <div className="bar" style={{ background: STATE_COLOR[st] }}>{name(b)}</div>
          </div>
        );
      })}

      <Panel x={14} y={402} w={752} h={154}>
        {sel ? (
          <>
            <Txt x={14} y={10} w={500} size={14} bold>{name(sel)}</Txt>
            <Txt x={14} y={32} w={500} size={11} color="var(--text-dim)" style={{ lineHeight: 1.4 }}>{sel.description}</Txt>
            <Txt x={14} y={68} w={500} size={11}>
              <span className="muted">Requires: </span>
              {sel.requires.length
                ? sel.requires.map((r) => {
                    const d = byKey.get(r);
                    return <span key={r} style={{ color: built.has(r) ? 'var(--text)' : '#e06050' }}>{d ? name(d) : r} </span>;
                  })
                : '—'}
            </Txt>
            <div style={{ position: 'absolute', left: 14, top: 96, display: 'flex', gap: 14 }}>
              {RES_ORDER.filter((r) => sel.cost[r] > 0).map((r) => (
                <div key={r} style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 12, fontWeight: 'bold',
                  color: res[r] >= sel.cost[r] ? 'var(--text)' : '#e06050' }}>
                  <ResIcon r={r} size={22} />{sel.cost[r]}
                </div>
              ))}
              {RES_ORDER.every((r) => !sel.cost[r]) && <span className="muted">Free</span>}
            </div>
            <Btn x={596} y={100} w={140} h={36}
              label={busy ? '…' : built.has(sel.key) ? 'Built' : 'Build'}
              disabled={busy || built.has(sel.key) || !prereqMet(sel) || !canAfford(sel)}
              onClick={doBuild} />
          </>
        ) : (
          <Txt x={0} y={66} w={752} size={11} align="center" color="var(--text-mute)">
            Choose a structure. Gold bar: built · green: can build · red: needs other buildings · brown: not enough resources.
          </Txt>
        )}
      </Panel>
    </Modal>
  );
}
function RecruitPanel(props: {
  town: TownDetail; creatures: Record<string, CreatureDef>;
  onRecruited: () => void; onClose: () => void;
}) {
  const { town, creatures, onRecruited, onClose } = props;
  const [busy, setBusy] = useState(false);
  const [tier, setTier] = useState(1);
  const [qty, setQty] = useState(1);
  const gold = town.resources.gold;

  const row = town.available.find((a) => a.tier === tier) ?? town.available[0];
  const c = row?.creature ? creatures[row.creature] : null;
  const unit = c?.gold ?? 0;
  const affordable = unit > 0 ? Math.floor(gold / unit) : 0;
  const max = Math.min(row?.count ?? 0, affordable);
  const total = unit * qty;
  const canBuy = !!row?.dwellingBuilt && (row?.count ?? 0) > 0 && qty > 0 && qty <= max;

  useEffect(() => { setQty(1); }, [tier]);

  async function recruit() {
    if (!row) return;
    setBusy(true);
    try {
      const r = await api.recruit(town.id, row.tier, qty);
      toast(`Recruited ${r.recruited} ${creatures[r.creature]?.name ?? r.creature}.`);
      onRecruited();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    } finally { setBusy(false); }
  }

  // The recruit PCX is a dark interior too: light text, buttons on the light band at y≈492.
  const ink = '#f0e0b0';
  const dim = '#c0a878';

  return (
    <Modal w={395} h={521} onClose={onClose}>
      <ScreenBackdrop name="tpubuild" w={395} h={521} alt="Recruit" />

      {/* creature portrait */}
      <div style={{ position: 'absolute', left: 22, top: 18, width: 64, height: 64, overflow: 'hidden' }}>
        {c && <DefImg name="CPRSMALL" frame={creaturePortraitFrame(c.key)} x={0} y={0} />}
      </div>

      <Txt x={96} y={22} w={270} size={14} bold color={ink}>{c?.name ?? '—'}</Txt>
      <Txt x={96} y={44} w={270} size={10} color={dim}>
        {c ? `Tier ${c.tier} · ${c.gold} gold each` : ''}
      </Txt>

      <Txt x={24} y={100} w={120} size={10} color={dim}>Available</Txt>
      <Txt x={150} y={100} w={80} size={11} bold color={ink}>{row?.count ?? 0}</Txt>
      <Txt x={24} y={120} w={120} size={10} color={dim}>Attack</Txt>
      <Txt x={150} y={120} w={80} size={11} bold color={ink}>{c?.attack ?? '—'}</Txt>
      <Txt x={24} y={140} w={120} size={10} color={dim}>Defense</Txt>
      <Txt x={150} y={140} w={80} size={11} bold color={ink}>{c?.defense ?? '—'}</Txt>
      <Txt x={24} y={160} w={120} size={10} color={dim}>Damage</Txt>
      <Txt x={150} y={160} w={80} size={11} bold color={ink}>{c ? `${c.damageMin}-${c.damageMax}` : '—'}</Txt>
      <Txt x={24} y={180} w={120} size={10} color={dim}>Health</Txt>
      <Txt x={150} y={180} w={80} size={11} bold color={ink}>{c?.health ?? '—'}</Txt>
      <Txt x={24} y={200} w={120} size={10} color={dim}>Speed</Txt>
      <Txt x={150} y={200} w={80} size={11} bold color={ink}>{c?.speed ?? '—'}</Txt>

      {/* tier selector */}
      <Txt x={24} y={236} w={340} size={10} color={dim}>Creature</Txt>
      {town.available.map((a, i) => {
        const cc = a.creature ? creatures[a.creature] : null;
        const on = a.tier === tier;
        return (
          <button key={a.tier} data-tier={a.tier}
            className={'abs h3hit' + (on ? ' h3slot' : '')}
            style={{ left: 24 + i * 50, top: 254, width: 44, height: 44, overflow: 'hidden',
              border: on ? '2px solid #f0d060' : '1px solid #6a5030', background: 'rgba(0,0,0,0.45)' }}
            title={cc?.name ?? `Tier ${a.tier}`}
            onClick={() => setTier(a.tier)}>
            {cc && <DefImg name="CPRSMALL" frame={creaturePortraitFrame(cc.key)} x={0} y={0} />}
          </button>
        );
      })}

      {/* quantity */}
      <Txt x={24} y={314} w={340} size={10} color={dim}>Quantity</Txt>
      <Btn x={24} y={332} w={44} h={28} label="1" size={11} disabled={busy} onClick={() => setQty(1)} />
      <Btn x={72} y={332} w={44} h={28} label="10" size={11} disabled={busy} onClick={() => setQty(Math.min(10, Math.max(1, max)))} />
      <Btn x={120} y={332} w={60} h={28} label="Max" size={11} disabled={busy || max < 1} onClick={() => setQty(Math.max(1, max))} />
      <Txt x={192} y={338} w={180} size={11} bold color={ink}>{qty}</Txt>

      {/* cost */}
      <div style={{ position: 'absolute', left: 24, top: 380, display: 'flex', alignItems: 'center', gap: 6 }}>
        <ResIcon r="gold" size={22} />
        <span style={{ fontSize: 14, fontWeight: 'bold', color: total <= gold ? ink : '#e06050' }}>{total}</span>
      </div>

      {!row?.dwellingBuilt && (
        <Txt x={24} y={410} w={340} size={10} color={dim}>Dwelling not built.</Txt>
      )}

      {/* buttons on the light band at y≈492 */}
      <Btn x={24} y={496} w={120} h={34} label={busy ? '…' : 'Buy'} size={12}
        disabled={busy || !canBuy} onClick={recruit} />
      <Btn x={251} y={496} w={120} h={34} label="Exit" size={12} onClick={onClose} />
    </Modal>
  );
}

function MarketPanel(props: { town: TownDetail; onTraded: () => void; onClose: () => void }) {
  const { town, onTraded, onClose } = props;
  const res = town.resources;
  const resources = ['wood', 'ore', 'mercury', 'sulfur', 'crystal', 'gems'] as const;
  const [busy, setBusy] = useState(false);
  const [qty, setQty] = useState(1);
  const [sel, setSel] = useState<typeof resources[number]>('wood');

  async function trade(from: string, to: string) {
    setBusy(true);
    try { await api.trade(town.id, from, to, qty); onTraded(); }
    catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
    finally { setBusy(false); }
  }

  const buyCost = 500 * qty;
  const sellGain = 250 * qty;

  // The trade PCX is a dark interior: light text throughout.
  const ink = '#f0e0b0';
  const dim = '#c0a878';

  return (
    <Modal w={800} h={600} onClose={onClose}>
      <ScreenBackdrop name="trade" w={800} h={600} alt="Marketplace" />

      <Txt x={0} y={16} w={800} size={16} bold align="center" color={ink}>MARKETPLACE</Txt>

      {/* resource grid: pick what to trade */}
      {resources.map((r, i) => {
        const on = sel === r;
        return (
          <button key={r} data-resource={r} className="abs h3hit"
            style={{ left: 60 + i * 112, top: 60, width: 96, height: 96,
              border: on ? '2px solid #f0d060' : '1px solid #6a5030',
              background: on ? 'rgba(200,160,80,0.25)' : 'rgba(0,0,0,0.45)' }}
            title={r} onClick={() => setSel(r)}>
            <div style={{ position: 'absolute', left: 28, top: 8 }}><ResIcon r={r} size={40} /></div>
            <div style={{ position: 'absolute', left: 0, right: 0, bottom: 6, textAlign: 'center', fontSize: 11, fontWeight: 'bold', color: ink }}>
              {res[r]}
            </div>
          </button>
        );
      })}

      {/* quantity */}
      <Txt x={60} y={180} w={200} size={11} color={dim}>Quantity</Txt>
      {[1, 5, 10].map((n, i) => (
        <Btn key={n} x={60 + i * 60} y={200} w={52} h={30} label={String(n)} size={11}
          disabled={busy} onClick={() => setQty(n)} />
      ))}
      <Txt x={250} y={208} w={200} size={12} bold color={ink}>{qty} {sel}</Txt>

      {/* gold on hand */}
      <div style={{ position: 'absolute', left: 60, top: 250, display: 'flex', alignItems: 'center', gap: 8 }}>
        <ResIcon r="gold" size={26} />
        <span style={{ fontSize: 16, fontWeight: 'bold', color: ink }}>{res.gold}</span>
      </div>

      {/* the two trades */}
      <Btn x={60} y={300} w={200} h={40} label={`Sell ${qty} for ${sellGain}g`} size={12}
        disabled={busy || res[sel] < qty} onClick={() => trade(sel, 'gold')} />
      <Btn x={280} y={300} w={200} h={40} label={`Buy ${qty} for ${buyCost}g`} size={12}
        disabled={busy || res.gold < buyCost} onClick={() => trade('gold', sel)} />

      <Txt x={60} y={356} w={420} size={10} color={dim}>
        Sell rate 250 gold · Buy rate 500 gold per unit.
      </Txt>

      <Btn x={640} y={540} w={120} h={34} label="Exit" size={12} onClick={onClose} />
    </Modal>
  );
}

function GuildPanel(props: { town: TownDetail; onClose: () => void }) {
  const { town, onClose } = props;
  const [spells, setSpells] = useState<Record<string, SpellDef>>({});

  useEffect(() => {
    api.getSpells().then((ss) => {
      const m: Record<string, SpellDef> = {};
      for (const s of ss) m[s.key] = s;
      setSpells(m);
    }).catch(() => {});
  }, []);

  const known = town.mageGuild.map((k) => spells[k]).filter((s): s is SpellDef => !!s);

  return (
    <Modal w={600} h={420} onClose={onClose}>
      <Txt x={0} y={10} w={600} size={16} bold align="center" className="heading">MAGE GUILD</Txt>
      {town.mageGuild.length === 0 ? (
        <Txt x={0} y={180} w={600} size={11} align="center" color="var(--text-mute)">
          No Mage Guild built yet.
        </Txt>
      ) : (
        <>
          <Txt x={24} y={40} w={552} size={10} color="var(--text-dim)">
            {known.length} spell{known.length === 1 ? '' : 's'} known
          </Txt>
          {known.map((s, i) => (
            <Panel key={s.key} x={24} y={62 + i * 62} w={552} h={56}>
              <Txt x={12} y={8} size={12} bold>{s.name}</Txt>
              <Txt x={12} y={26} w={400} size={10} color="var(--text-dim)">{s.description}</Txt>
              <Txt x={12} y={42} w={400} size={9} color="var(--text-mute)">
                Level {s.level} · {s.school} · {s.cost} spell points
              </Txt>
            </Panel>
          ))}
        </>
      )}
      <Btn x={456} y={368} w={120} h={34} label="Exit" size={12} onClick={onClose} />
    </Modal>
  );
}

function TavernPanel(props: { town: TownDetail; onHired: () => void; onClose: () => void }) {
  const { town, onHired, onClose } = props;
  const h3 = useH3();
  const [busy, setBusy] = useState(false);
  const [heroes, setHeroes] = useState<Record<string, HeroDef>>({});
  const [index, setIndex] = useState(0);

  useEffect(() => {
    api.getHeroCatalog().then((hs) => {
      const m: Record<string, HeroDef> = {};
      for (const h of hs) m[h.key] = h;
      setHeroes(m);
    }).catch(() => {});
  }, []);

  const pool = town.tavern;
  const key = pool[Math.min(index, Math.max(0, pool.length - 1))];
  const h = key ? heroes[key] : null;
  const gold = town.resources.gold;
  const canAfford = gold >= HERO_COST;

  async function hire() {
    if (!key) return;
    setBusy(true);
    try {
      await api.hire(town.id, key);
      toast(`Hired ${heroes[key]?.name ?? key}.`);
      setIndex(0);
      onHired();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    } finally { setBusy(false); }
  }

  // The tavern PCX is a dark wooden interior: text must be light, and the
  // portrait sits in the upper-left window.
  const ink = '#f0e0b0';
  const dim = '#c0a878';

  return (
    <Modal w={395} h={504} onClose={onClose}>
      <ScreenBackdrop name="tptavern" w={395} h={504} alt="Tavern" />

      {pool.length === 0 ? (
        <Txt x={0} y={230} w={395} size={11} align="center" color={dim}>
          No heroes are visiting today.
        </Txt>
      ) : (
        <>
          {/* hero portrait in the upper-left window */}
          <div style={{ position: 'absolute', left: 22, top: 18, width: 58, height: 64, overflow: 'hidden' }}>
            {h3 && <Pcx name={heroPortrait(key, town.faction, 'small')} x={0} y={0} w={58} h={64} alt={h?.name ?? key} />}
          </div>

          <Txt x={90} y={22} w={280} size={14} bold color={ink}>{h?.name ?? key}</Txt>
          <Txt x={90} y={42} w={280} size={10} color={dim}>
            {h ? (h.might ? 'Might' : 'Magic') + ' Hero' : ''}
          </Txt>

          {/* stats block */}
          <Txt x={24} y={100} w={120} size={10} color={dim}>Attack</Txt>
          <Txt x={150} y={100} w={60} size={11} bold color={ink}>{h?.attack ?? '—'}</Txt>
          <Txt x={24} y={120} w={120} size={10} color={dim}>Defense</Txt>
          <Txt x={150} y={120} w={60} size={11} bold color={ink}>{h?.defense ?? '—'}</Txt>
          <Txt x={24} y={140} w={120} size={10} color={dim}>Power</Txt>
          <Txt x={150} y={140} w={60} size={11} bold color={ink}>{h?.power ?? '—'}</Txt>
          <Txt x={24} y={160} w={120} size={10} color={dim}>Knowledge</Txt>
          <Txt x={150} y={160} w={60} size={11} bold color={ink}>{h?.knowledge ?? '—'}</Txt>

          {/* skills */}
          <Txt x={24} y={192} w={340} size={10} color={dim}>Skills</Txt>
          {(h?.skills ?? []).map((s, j) => (
            <Txt key={s} x={24} y={210 + j * 18} w={340} size={11} color={ink}>· {skillName(s)}</Txt>
          ))}

          {/* cost */}
          <div style={{ position: 'absolute', left: 24, top: 300, display: 'flex', alignItems: 'center', gap: 6 }}>
            <ResIcon r="gold" size={22} />
            <span style={{ fontSize: 14, fontWeight: 'bold', color: canAfford ? ink : '#e06050' }}>{HERO_COST}</span>
          </div>

          {/* cycle between the visiting heroes */}
          {pool.length > 1 && (
            <Btn x={24} y={336} w={130} h={28} label={`Hero ${index + 1} of ${pool.length}`} size={10}
              disabled={busy} onClick={() => setIndex((i) => (i + 1) % pool.length)} />
          )}

          {/* buttons sit on the light band at y≈428 */}
          <Btn x={24} y={432} w={120} h={34} label={busy ? '…' : 'Hire'} size={12}
            disabled={busy || !canAfford} onClick={hire} />
        </>
      )}

      <Btn x={251} y={432} w={120} h={34} label="Exit" size={12} onClick={onClose} />
    </Modal>
  );
}

function GarrisonPanel(props: {
  town: TownDetail; creatures: Record<string, CreatureDef>;
  onChanged: () => void; onClose: () => void;
}) {
  const { town, creatures, onChanged, onClose } = props;
  const [busy, setBusy] = useState(false);
  const hero = town.heroInTown;

  async function move(direction: 'toGarrison' | 'toHero', slot: number, count: number) {
    setBusy(true);
    try {
      const r = await api.moveGarrison(town.id, direction, slot, count);
      toast(`Moved ${r.moved} ${creatures[r.creature]?.name ?? r.creature}.`);
      onChanged();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    } finally { setBusy(false); }
  }

  const row = (slots: ArmySlot[], direction: 'toGarrison' | 'toHero', y: number) =>
    slots.map((s, i) => {
      const c = s.creature ? creatures[s.creature] : null;
      const x = 24 + i * 76;
      return (
        <div key={i} data-slot={`${direction}-${i}`}
          style={{ position: 'absolute', left: x, top: y, width: 70, height: 76,
            border: '1px solid var(--gold-0)', background: 'rgba(0,0,0,0.35)' }}>
          {c && <DefImg name="CPRSMALL" frame={creaturePortraitFrame(c.key)} x={12} y={4} />}
          <div style={{ position: 'absolute', left: 0, right: 0, bottom: 20, textAlign: 'center', fontSize: 11, fontWeight: 'bold' }}>
            {s.count > 0 ? s.count : ''}
          </div>
          {s.creature && s.count > 0 && (
            <button className="abs h3hit" disabled={busy}
              style={{ left: 4, bottom: 2, width: 62, height: 16, fontSize: 9 }}
              title={`Move ${c?.name ?? s.creature} ${direction === 'toGarrison' ? 'to garrison' : 'to hero'}`}
              onClick={() => move(direction, i, s.count)}>
              {direction === 'toGarrison' ? '▼' : '▲'}
            </button>
          )}
        </div>
      );
    });

  return (
    <Modal w={600} h={420} onClose={onClose}>
      <Txt x={0} y={10} w={600} size={16} bold align="center" className="heading">GARRISON</Txt>

      <Txt x={24} y={44} w={552} size={11} color="var(--text-dim)">
        {hero ? `${hero.name} in town` : 'No hero in town — troops stay in the garrison.'}
      </Txt>

      {hero && (
        <>
          <Txt x={24} y={68} w={552} size={10} color="var(--text-mute)">Hero army</Txt>
          {row(hero.army, 'toGarrison', 86)}
        </>
      )}

      <Txt x={24} y={hero ? 180 : 86} w={552} size={10} color="var(--text-mute)">Garrison</Txt>
      {row(town.garrison, 'toHero', hero ? 198 : 104)}

      {!hero && (
        <Txt x={24} y={200} w={552} size={10} color="var(--text-mute)">
          Bring a hero into town to move troops between the army and the garrison.
        </Txt>
      )}

      <Btn x={456} y={368} w={120} h={34} label="Exit" size={12} onClick={onClose} />
    </Modal>
  );
}

function TowerPanel(props: { town: TownDetail; onClose: () => void }) {
  const { town, onClose } = props;
  const [posts, setPosts] = useState<{ id: string; title: string; body: string; createdAt: string }[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [comments, setComments] = useState<{ id: string; author: string; body: string }[]>([]);
  const [draft, setDraft] = useState('');
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [writing, setWriting] = useState(false);

  const load = useCallback(async () => {
    try {
      const p = await api.getPosts(town.id);
      setPosts(p);
      if (!selected && p.length) setSelected(p[0].id);
    } catch { /* ignore */ }
  }, [town.id, selected]);

  useEffect(() => { load(); }, [load]);

  useEffect(() => {
    if (!selected) { setComments([]); return; }
    api.getComments(selected).then(setComments).catch(() => {});
  }, [selected]);

  const post = posts.find((p) => p.id === selected) ?? null;

  async function addComment() {
    if (!selected || !draft.trim()) return;
    try {
      await api.addComment(selected, draft.trim());
      setDraft('');
      setComments(await api.getComments(selected));
    } catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
  }

  async function savePost() {
    try {
      await api.createPost(town.id, title, body, null);
      setWriting(false);
      setTitle('');
      setBody('');
      await load();
      toast('Post published.');
    } catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
  }

  return (
    <Modal w={700} h={500} onClose={onClose}>
      <Txt x={0} y={10} w={700} size={16} bold align="center" className="heading">MAGIC TOWER</Txt>
      <Txt x={24} y={36} w={652} size={10} color="var(--text-dim)">
        {posts.length} posts · a journal of this city
      </Txt>

      <Panel x={24} y={58} w={200} h={400}>
        <div className="scroll" style={{ position: 'absolute', inset: 3 }}>
          {posts.length === 0 && (
            <Txt x={8} y={10} w={180} size={10} color="var(--text-mute)">No posts yet.</Txt>
          )}
          {posts.map((p) => (
            <div key={p.id} onClick={() => setSelected(p.id)}
              style={{
                padding: '6px 8px', fontSize: 11, cursor: 'pointer',
                color: selected === p.id ? 'var(--accent)' : 'var(--text)',
                background: selected === p.id ? 'rgba(200,160,80,0.18)' : 'transparent',
                borderLeft: selected === p.id ? '3px solid var(--accent)' : '3px solid transparent',
                whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'
              }}>
              ▸ {p.title}
            </div>
          ))}
        </div>
      </Panel>

      <Panel x={236} y={58} w={440} h={400}>
        {post ? (
          <div className="scroll" style={{ position: 'absolute', inset: 6, padding: '4px 8px' }}>
            <div style={{ fontSize: 14, fontWeight: 'bold' }}>{post.title}</div>
            <div style={{ fontSize: 10, color: 'var(--text-dim)', marginTop: 2 }}>
              {new Date(post.createdAt).toLocaleDateString()}
            </div>
            <div style={{ fontSize: 11, lineHeight: 1.5, marginTop: 8, whiteSpace: 'pre-wrap' }}>{post.body}</div>
            <div style={{ marginTop: 14, borderTop: '1px solid var(--gold-0)', paddingTop: 6 }}>
              <div style={{ fontSize: 11, fontWeight: 'bold' }}>Comments ({comments.length})</div>
              {comments.map((c) => (
                <div key={c.id} style={{ marginTop: 4, fontSize: 10 }}>
                  <span style={{ color: 'var(--accent)' }}>{c.author}: </span>{c.body}
                </div>
              ))}
              <div style={{ marginTop: 8, display: 'flex', gap: 6 }}>
                <input className="inp" style={{ position: 'relative', width: 300, height: 26 }}
                  placeholder="write a comment…" value={draft} maxLength={2000}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') addComment(); }} />
                <Btn x={0} y={0} w={60} h={26} label="Post" size={10} onClick={addComment} />
              </div>
            </div>
          </div>
        ) : (
          <Txt x={0} y={180} w={440} size={11} align="center" color="var(--text-mute)">No posts yet.</Txt>
        )}
      </Panel>

      {writing ? (
        <Panel x={24} y={58} w={652} h={400}>
          <Txt x={16} y={12} size={12} bold>New Post</Txt>
          <input className="inp" style={{ left: 16, top: 36, width: 620, height: 28 }}
            placeholder="Title" value={title} maxLength={120} onChange={(e) => setTitle(e.target.value)} />
          <textarea className="inp" style={{ left: 16, top: 72, width: 620, height: 240 }}
            placeholder="Write your journal entry…" value={body} maxLength={20000}
            onChange={(e) => setBody(e.target.value)} />
          <Btn x={440} y={330} w={90} h={30} label="Publish" size={11} onClick={savePost} />
          <Btn x={540} y={330} w={90} h={30} label="Cancel" size={11} onClick={() => setWriting(false)} />
        </Panel>
      ) : (
        <Btn x={24} y={466} w={200} h={26} label="+ New Post" size={11} onClick={() => setWriting(true)} />
      )}
    </Modal>
  );
}

function WallPanel(props: { town: TownDetail; onClose: () => void }) {
  const { town, onClose } = props;
  const [msgs, setMsgs] = useState<{ id: string; author: string; body: string; parentId: string | null }[]>([]);
  const [draft, setDraft] = useState('');

  const load = useCallback(async () => {
    try { setMsgs(await api.getWall(town.id)); } catch { /* ignore */ }
  }, [town.id]);

  useEffect(() => { load(); }, [load]);

  async function post() {
    if (!draft.trim()) return;
    try {
      await api.addWallMessage(town.id, draft.trim(), null);
      setDraft('');
      await load();
    } catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
  }

  return (
    <Modal w={560} h={440} onClose={onClose}>
      <Txt x={0} y={10} w={560} size={16} bold align="center" className="heading">CITY WALL</Txt>
      <Panel x={24} y={44} w={512} h={320}>
        <div className="scroll" style={{ position: 'absolute', inset: 6 }}>
          {msgs.length === 0 && (
            <Txt x={8} y={10} w={480} size={10} color="var(--text-mute)">No messages yet. Be the first.</Txt>
          )}
          {msgs.map((m) => (
            <div key={m.id} style={{ marginBottom: 8, marginLeft: m.parentId ? 16 : 0 }}>
              <div style={{ fontSize: 10, color: 'var(--accent)' }}>{m.parentId ? '↳ ' : ''}{m.author}</div>
              <div style={{ fontSize: 11, lineHeight: 1.4 }}>{m.body}</div>
            </div>
          ))}
        </div>
      </Panel>
      <input className="inp" style={{ left: 24, top: 380, width: 400, height: 28 }}
        placeholder="leave a message…" value={draft} maxLength={2000}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') post(); }} />
      <Btn x={432} y={380} w={104} h={28} label="Post" size={11} onClick={post} />
    </Modal>
  );
}