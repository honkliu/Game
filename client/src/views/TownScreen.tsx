import { useEffect, useState, useCallback, useMemo } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { api, ApiError } from '../api';
import type { TownDetail, BuildingDef, CreatureDef } from '../types';
import { Btn, Loading, Modal, Panel, Txt } from '../ui/Layout';
import { toast } from '../ui/Toast';
import { FACTION_LABEL, RES_ORDER, ResIcon, buildingName } from '../ui/homm';
import { DefImg, Pcx, useDef } from '../h3/assets';
import { creaturePortraitFrame, townListFrame } from '../h3/catalog';
import { HALL_DEF, TOWN_BACK } from '../h3/town';

/** Asset file for a building key; Mage Guild levels reuse the magic tower art. */
function assetFor(key: string): string {
  const m = /^mageGuild(\d)$/.exec(key);
  if (m) return `/assets/buildings/magicTower${m[1]}.svg`;
  const u = /^upg(\d)$/.exec(key);
  return `/assets/buildings/${u ? 'dwelling' + u[1] : key}.svg`;
}

/** Painted 800×374 town scene, one per faction. */
function townBackdrop(faction: string): string {
  return `/towns/${faction in TOWN_BACK ? faction : 'castle'}.png`;
}

type Tab = 'build' | 'recruit' | 'market' | 'guild' | 'tavern' | 'garrison' | 'tower' | 'wall';

export function TownScreen() {
  const { gameId, townId } = useParams<{ gameId: string; townId: string }>();
  const navigate = useNavigate();
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
      <TownH3 town={town} buildings={buildings} creatures={creatures} tabs={town.isOwner ? ALL_TABS : ALL_TABS.filter(([id]) => id === 'tower' || id === 'wall')}
        onTab={setTab} onExit={() => navigate(`/game/${gameId}`)} />
      {panels(town, buildings, creatures, tab, setTab, refresh)}
    </>
  );
}

const ALL_TABS: [Tab, string][] = [
  ['build', 'Build'], ['recruit', 'Recruit'], ['market', 'Market'],
  ['guild', 'Guild'], ['tavern', 'Tavern'], ['garrison', 'Garrison'],
  ['tower', 'Tower'], ['wall', 'Wall']
];

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
      {tab === 'garrison' && <GarrisonPanel town={town} creatures={creatures} onClose={() => setTab(null)} />}
      {tab === 'tower' && <TowerPanel town={town} onClose={() => setTab(null)} />}
      {tab === 'wall' && <WallPanel town={town} onClose={() => setTab(null)} />}
    </>
  );
}

// HALL icon (original building id) shown on each lower-panel action slot.
const TAB_ICON: Record<Tab, number> = { build: 10, recruit: 30, market: 14, guild: 0, tavern: 5, garrison: 8, tower: 17, wall: 7 };
const lastBuilt = (chain: string[], built: Set<string>) => chain.reduce((acc, k, i) => (built.has(k) ? i : acc), -1);
const RES_X: Record<string, number> = { wood: 37, mercury: 121, ore: 205, sulfur: 289, crystal: 373, gems: 457, gold: 541 };

/** HoMM3 town screen: the painted 800×374 scene with the original structures, over the townscrn.pcx panel. */
function TownH3(props: {
  town: TownDetail; buildings: BuildingDef[]; creatures: Record<string, CreatureDef>;
  tabs: [Tab, string][]; onTab: (t: Tab) => void; onExit: () => void;
}) {
  const { town, buildings, creatures, tabs, onTab, onExit } = props;
  const [rollover, setRollover] = useState<string | null>(null);
  const built = useMemo(() => new Set(town.buildings), [town.buildings]);
  const name = (k: string) => buildingName(k, town.faction, buildings.find((b) => b.key === k)?.name);
  const hall = HALL_DEF[town.faction] ?? 'HALLCSTL';
  const hallSheet = useDef(hall);

  const hallLevel = Math.max(0, lastBuilt(['villageHall', 'townHall', 'cityHall', 'capitol'], built));
  const fortLevel = lastBuilt(['fort', 'citadel', 'castle'], built);
  const growth = town.available.filter((a) => a.creature);
  const garrison = (town.garrison ?? []).slice(0, 7);
  const label = (text: string) => setRollover(text);

  return (
    <>
      <div className="town-scene" style={{ position: 'absolute', left: 0, top: 0, width: 800, height: 374 }}>
        <img src={townBackdrop(town.faction)} alt={`${FACTION_LABEL[town.faction]} town`} width={800} height={374}
          style={{ position: 'absolute', left: 0, top: 0, pointerEvents: 'none' }} />
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

      {/* available creatures */}
      {growth.map((a, i) => (
        <div key={a.tier} className="abs" title={`${creatures[a.creature!]?.name ?? a.creature}: ${a.count} available`}
          style={{ left: 14 + 55 * (i % 4), top: 459 + 46 * Math.floor(i / 4), width: 48, height: 32 }}
          onMouseEnter={() => label(`${creatures[a.creature!]?.name ?? a.creature}: ${a.count} available`)} onMouseLeave={() => setRollover(null)}>
          <DefImg name="CPRSMALL" frame={creaturePortraitFrame(a.creature!)} x={8} y={0} />
          <div className="abs h3text" style={{ right: 1, bottom: -1, fontSize: 10 }}>{a.count}</div>
        </div>
      ))}

      {/* garrison row */}
      {garrison.map((g, i) => g && g.creature ? (
        <div key={i} className="abs" style={{ left: 305 + 62 * i, top: 387, width: 58, height: 64 }}
          onMouseEnter={() => label(`${creatures[g.creature!]?.name ?? g.creature} ×${g.count}`)} onMouseLeave={() => setRollover(null)}>
          <DefImg name="TWCRPORT" frame={creaturePortraitFrame(g.creature)} x={0} y={0} />
          <div className="abs h3text" style={{ right: 3, bottom: 1, fontSize: 11 }}>{g.count}</div>
        </div>
      ) : null)}

      {/* action row: one original building icon per town window */}
      {tabs.map(([id, text], i) => {
        const frame = hallSheet ? (hallSheet.groups['0']?.[TAB_ICON[id]] ?? 0) : 0;
        return (
          <button key={id} className="abs h3hit h3slot" style={{ left: 241 + 62 * i + (i > 0 ? 2 : 0), top: 483, width: 58, height: 64 }}
            onMouseEnter={() => label(text)} onMouseLeave={() => setRollover(null)} onClick={() => onTab(id)}>
            <div style={{ position: 'absolute', inset: 0, overflow: 'hidden' }}>
              <DefImg name={hall} frame={frame} x={-46} y={-4} />
            </div>
            <span className="h3text" style={{ position: 'absolute', left: 0, right: 0, bottom: 1, fontSize: 11, textAlign: 'center' }}>{text}</span>
          </button>
        );
      })}

      {/* town list + exit */}
      <DefImg name="ITPA" frame={townListFrame(town.faction)} x={747} y={430} />
      <button className="abs h3hit" style={{ left: 744, top: 544, width: 48, height: 30 }} aria-label="Exit" onClick={onExit}
        onMouseEnter={() => label('Exit')} onMouseLeave={() => setRollover(null)} />

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

  const cards = CHAINS
    .map((chain) => chain.map((k) => byKey.get(k)).filter((b): b is BuildingDef => !!b))
    .filter((chain) => chain.length > 0)
    .map((chain) => chain.find((b) => !built.has(b.key)) ?? chain[chain.length - 1]);

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
      <Txt x={0} y={30} w={780} size={10} align="center" color="var(--text-dim)">One structure per turn.</Txt>

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

  async function recruit(tier: number, count: number) {
    setBusy(true);
    try {
      const r = await api.recruit(town.id, tier, count);
      toast(`Recruited ${r.recruited} ${creatures[r.creature]?.name ?? r.creature}.`);
      onRecruited();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    } finally { setBusy(false); }
  }

  return (
    <Modal w={700} h={500} onClose={onClose}>
      <Txt x={0} y={10} w={700} size={16} bold align="center" className="heading">RECRUIT</Txt>
      {town.available.map((t, i) => {
        const c = t.creature ? creatures[t.creature] : null;
        const y = 44 + i * 62;
        return (
          <Panel key={t.tier} x={24} y={y} w={652} h={56}>
            <div style={{ position: 'absolute', left: 8, top: 8, width: 40, height: 40 }}>
              <svg width={40} height={40} viewBox="0 0 40 40">
                <circle cx={20} cy={20} r={15} fill={`hsl(${(t.tier * 40) % 360},45%,45%)`} />
                <circle cx={14} cy={17} r={3} fill="#ffd040" />
                <circle cx={26} cy={17} r={3} fill="#ffd040" />
              </svg>
            </div>
            <Txt x={56} y={8} size={12} bold>{c?.name ?? '—'}</Txt>
            <Txt x={56} y={26} size={10} color="var(--text-dim)">
              Available: {t.count} · {c ? c.gold + 'g each' : ''}
            </Txt>
            <Txt x={56} y={40} size={9} color="var(--text-mute)">
              {c ? `A${c.attack} D${c.defense} Dmg${c.damageMin}-${c.damageMax} HP${c.health} Spd${c.speed}` : ''}
            </Txt>
            {t.dwellingBuilt && t.count > 0 && (
              <>
                <Btn x={440} y={14} w={50} h={26} label="1" size={11} disabled={busy} onClick={() => recruit(t.tier, 1)} />
                <Btn x={496} y={14} w={50} h={26} label="10" size={11} disabled={busy} onClick={() => recruit(t.tier, 10)} />
                <Btn x={552} y={14} w={70} h={26} label="Max" size={11} disabled={busy} onClick={() => recruit(t.tier, t.count)} />
              </>
            )}
            {!t.dwellingBuilt && (
              <Txt x={440} y={20} w={190} size={10} color="var(--text-mute)">Dwelling not built</Txt>
            )}
          </Panel>
        );
      })}
    </Modal>
  );
}

function MarketPanel(props: { town: TownDetail; onTraded: () => void; onClose: () => void }) {
  const { town, onTraded, onClose } = props;
  const res = town.resources;
  const [busy, setBusy] = useState(false);

  async function trade(from: string, to: string) {
    setBusy(true);
    try { await api.trade(town.id, from, to, 1); onTraded(); }
    catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
    finally { setBusy(false); }
  }

  const resources = ['wood', 'ore', 'mercury', 'sulfur', 'crystal', 'gems'] as const;

  return (
    <Modal w={560} h={440} onClose={onClose}>
      <Txt x={0} y={10} w={560} size={16} bold align="center" className="heading">MARKETPLACE</Txt>
      <Txt x={24} y={38} w={512} size={10} color="var(--text-dim)">
        Sell 1 for 250 gold. Buy 1 for 500 gold.
      </Txt>
      {resources.map((r, i) => (
        <Panel key={r} x={24} y={60 + i * 58} w={512} h={50}>
          <Txt x={12} y={14} size={12} bold>{r.toUpperCase()}</Txt>
          <Txt x={110} y={16} size={11} color="var(--text-dim)">have {res[r]}</Txt>
          <Btn x={240} y={10} w={90} h={28} label="Sell 1" size={11} disabled={busy || res[r] < 1} onClick={() => trade(r, 'gold')} />
          <Btn x={340} y={10} w={90} h={28} label="Buy 1" size={11} disabled={busy || res.gold < 500} onClick={() => trade('gold', r)} />
        </Panel>
      ))}
    </Modal>
  );
}

function GuildPanel(props: { town: TownDetail; onClose: () => void }) {
  const { town, onClose } = props;
  return (
    <Modal w={560} h={360} onClose={onClose}>
      <Txt x={0} y={10} w={560} size={16} bold align="center" className="heading">MAGE GUILD</Txt>
      {town.mageGuild.length === 0 ? (
        <Txt x={0} y={160} w={560} size={11} align="center" color="var(--text-mute)">No Mage Guild built yet.</Txt>
      ) : (
        town.mageGuild.map((s, i) => <Txt key={s} x={32} y={60 + i * 24} size={12}>{s}</Txt>)
      )}
    </Modal>
  );
}

function TavernPanel(props: { town: TownDetail; onHired: () => void; onClose: () => void }) {
  const { town, onHired, onClose } = props;
  const [busy, setBusy] = useState(false);

  async function hire(key: string) {
    setBusy(true);
    try { await api.hire(town.id, key); toast('Hired a new hero.'); onHired(); }
    catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
    finally { setBusy(false); }
  }

  return (
    <Modal w={560} h={360} onClose={onClose}>
      <Txt x={0} y={10} w={560} size={16} bold align="center" className="heading">TAVERN</Txt>
      <Txt x={24} y={38} w={512} size={10} color="var(--text-dim)">Heroes for hire: 2000 gold each.</Txt>
      {town.tavern.length === 0 ? (
        <Txt x={0} y={160} w={560} size={11} align="center" color="var(--text-mute)">No heroes available.</Txt>
      ) : (
        town.tavern.map((h, i) => (
          <Panel key={h} x={24} y={60 + i * 70} w={512} h={60}>
            <Txt x={14} y={14} size={13} bold>{h}</Txt>
            <Btn x={400} y={16} w={100} h={28} label="Hire" size={11} disabled={busy} onClick={() => hire(h)} />
          </Panel>
        ))
      )}
    </Modal>
  );
}

function GarrisonPanel(props: { town: TownDetail; creatures: Record<string, CreatureDef>; onClose: () => void }) {
  const { town, creatures, onClose } = props;
  const any = town.garrison.some((s) => s.creature);
  return (
    <Modal w={560} h={360} onClose={onClose}>
      <Txt x={0} y={10} w={560} size={16} bold align="center" className="heading">GARRISON</Txt>
      {!any && <Txt x={0} y={160} w={560} size={11} align="center" color="var(--text-mute)">The garrison is empty.</Txt>}
      {town.garrison.map((s, i) => {
        if (!s.creature) return null;
        return (
          <Panel key={i} x={24 + (i % 4) * 130} y={60 + Math.floor(i / 4) * 80} w={120} h={70}>
            <Txt x={10} y={10} size={11} bold>{creatures[s.creature]?.name ?? s.creature}</Txt>
            <Txt x={10} y={34} size={14} color="var(--accent)">{s.count}</Txt>
          </Panel>
        );
      })}
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