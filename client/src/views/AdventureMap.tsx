import { useEffect, useRef, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { api, ApiError } from '../api';
import type { MapData, GameDetail, HeroDetail, MapTile } from '../types';
import { Btn, Loading, Txt } from '../ui/Layout';
import { toast } from '../ui/Toast';
import { socket } from '../ws';
import { FACTION_COLOR, FACTION_SEAT, HERO_CLASS, HeroPortrait, RES_ORDER, ResIcon } from '../ui/homm';
import { DefImg, Pcx, drawFrame, getDef, groupFrame, onAssetLoaded, useH3, type Sheet } from '../h3/assets';
import {
  MINE_DEF, MOUNTAIN_DEFS, OBJECT_DEF, RESOURCE_DEF, ROCK_DEFS, TERRAIN_DEF, TOWN_MAP_DEF, TREE_DEFS, artifactSeed, borderKind, edgeFrames,
  artifactDef, creaturePortraitFrame, flagDef, heroMapDef, heroPortrait, monsterDef, townListFrame
} from '../h3/catalog';

// Original HoMM3 800x600 adventure-screen geometry (shared by both art modes).
const TILE = 32;
const MAP_X = 7;
const MAP_Y = 7;
const MAP_W = 594;
const MAP_H = 546;
const VIEW_TX = Math.floor(MAP_W / TILE); // 18 whole tiles
const VIEW_TY = Math.floor(MAP_H / TILE); // 17 whole tiles

const TERRAIN_COLOR: Record<string, string> = {
  dirt: '#8a6a44', sand: '#c8b070', grass: '#5a8a3a', snow: '#c8d0d8',
  swamp: '#4a6a4a', rough: '#7a6a50', subterranean: '#4a3a5a',
  lava: '#6a2a1a', water: '#2a4a8a', rock: '#3a3a3a'
};

// Minimap colours of the original game.
const MINIMAP_COLOR: Record<string, string> = {
  dirt: '#523808', sand: '#dece8c', grass: '#004000', snow: '#b5c7c6', swamp: '#4a846b',
  rough: '#846b31', subterranean: '#845a29', lava: '#4a4a4a', water: '#08528c', rock: '#000000'
};

const TERRAIN_NAME: Record<string, string> = {
  dirt: 'Dirt', sand: 'Sand', grass: 'Grass', snow: 'Snow', swamp: 'Swamp',
  rough: 'Rough', subterranean: 'Subterranean', lava: 'Lava', water: 'Water', rock: 'Rock'
};

// Sidebar buttons at their original positions; each original control maps to a social-game feature.
const BUTTONS: { label: string; def: string; x: number; y: number; w: number; tip: string }[] = [
  { label: 'Social', def: 'IAM002', x: 679, y: 196, w: 32, tip: 'Kingdom: messages, trades, friends' },
  { label: 'Projects', def: 'IAM010', x: 711, y: 196, w: 32, tip: 'Shared wonder projects' },
  { label: 'Obelisk', def: 'IAM004', x: 679, y: 228, w: 32, tip: 'Obelisk puzzle and riddles' },
  { label: 'Sleep', def: 'IAM005', x: 711, y: 228, w: 32, tip: 'Put hero to sleep' },
  { label: 'Hero', def: 'IAM006', x: 679, y: 260, w: 32, tip: 'Hero screen' },
  { label: 'Spells', def: 'IAM007', x: 711, y: 260, w: 32, tip: 'Spellbook' },
  { label: 'Town', def: 'IAM008', x: 679, y: 292, w: 32, tip: 'Visit your town' },
  { label: 'Menu', def: 'IAM009', x: 711, y: 292, w: 32, tip: 'System options' },
  { label: 'Next Hero', def: 'IAM000', x: 679, y: 324, w: 64, tip: 'Next hero' },
  { label: 'End Turn', def: 'IAM001', x: 679, y: 356, w: 64, tip: 'End turn' }
];

const RESBAR_X: Record<string, number> = { wood: 37, mercury: 121, ore: 205, sulfur: 289, crystal: 373, gems: 457, gold: 541 };

function hash(x: number, y: number, s: number): number {
  let h = Math.imul(x | 0, 374761393) + Math.imul(y | 0, 668265263) + Math.imul(s | 0, 1274126177);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

function shade(hex: string, amt: number): string {
  const n = parseInt(hex.slice(1), 16);
  let r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  if (amt >= 0) { r += (255 - r) * amt; g += (255 - g) * amt; b += (255 - b) * amt; }
  else { r *= 1 + amt; g *= 1 + amt; b *= 1 + amt; }
  return `rgb(${r | 0},${g | 0},${b | 0})`;
}

function clampCam(map: MapData, x: number, y: number) {
  return {
    x: Math.max(0, Math.min(map.width - VIEW_TX, x)),
    y: Math.max(0, Math.min(map.height - VIEW_TY, y))
  };
}
const centerCam = (map: MapData, x: number, y: number) => clampCam(map, x - Math.floor(VIEW_TX / 2), y - Math.floor(VIEW_TY / 2));

type Facing = { group: number; mirror: boolean };
function facingFor(dx: number, dy: number): Facing {
  const g = dy < 0 ? (dx === 0 ? 0 : 1) : dy > 0 ? (dx === 0 ? 4 : 3) : 2;
  return { group: g, mirror: dx < 0 };
}

export function AdventureMap() {
  const { gameId } = useParams<{ gameId: string }>();
  const navigate = useNavigate();
  const h3 = useH3() === true;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [map, setMap] = useState<MapData | null>(null);
  const [game, setGame] = useState<GameDetail | null>(null);
  const [hero, setHero] = useState<HeroDetail | null>(null);
  const [heroes, setHeroes] = useState<HeroDetail[]>([]);
  const selectedRef = useRef<string | null>(null);
  const [cam, setCam] = useState({ x: 0, y: 0 });
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);
  const [path, setPath] = useState<{ x: number; y: number }[] | null>(null);
  const [now, setNow] = useState(Date.now());
  const [tick, setTick] = useState(0);
  const [assetGen, setAssetGen] = useState(0);
  const [pressed, setPressed] = useState<string | null>(null);
  const [heroScroll, setHeroScroll] = useState(0);
  const [townScroll, setTownScroll] = useState(0);
  const [encounter, setEncounter] = useState<{ creature: string; count: number } | null>(null);
  const [meeting, setMeeting] = useState<{ heroName: string; playerId: string; ownerName: string } | null>(null);
  const [meetingText, setMeetingText] = useState('');
  const movingRef = useRef<string | null>(null);
  const facingRef = useRef(new Map<string, Facing>());
  const myPlayerIdRef = useRef<string | null>(null);
  const playerId = localStorage.getItem('cityoftowers.player') ?? '';

  const refresh = useCallback(async () => {
    if (!gameId) return;
    try {
      const [m, g, hs] = await Promise.all([api.getMap(gameId), api.getGame(gameId), api.getHeroes(gameId)]);
      setMap(m);
      setGame(g);
      setHeroes(hs);
      const cur = hs.find((h) => h.id === selectedRef.current) ?? hs[0];
      if (cur) {
        setHero(cur);
        if (cur.pendingEncounter) {
          try { setEncounter(JSON.parse(cur.pendingEncounter)); } catch { /* ignore */ }
        } else {
          setEncounter(null);
        }
        if (!movingRef.current) {
          setCam((c) => (c.x === 0 && c.y === 0 ? centerCam(m, cur.x, cur.y) : c));
        }
      }
    } catch (e) {
      if (e instanceof ApiError) toast(e.message);
    }
  }, [gameId]);

  useEffect(() => { refresh(); }, [refresh]);

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  // Adventure-map animation (flags, monsters, chests) at the original ~180ms frame rate.
  useEffect(() => {
    if (!h3) return;
    const id = window.setInterval(() => setTick((t) => t + 1), 180);
    const off = onAssetLoaded(() => setAssetGen((n) => n + 1));
    return () => { window.clearInterval(id); off(); };
  }, [h3]);

  useEffect(() => {
    if (!gameId) return;
    socket.connect(gameId);
    const off = socket.on((e) => {
      if (movingRef.current) return;
      if (e.type === 'turnAdvanced' || e.type === 'heroMoved' || e.type === 'encounter' || e.type === 'project') refresh();
      if (e.type === 'message' && e.payload.toPlayerId === myPlayerIdRef.current) toast('A hero has sent you a message.');
      if (e.type === 'trade' && e.payload.toPlayerId === myPlayerIdRef.current) toast('You have a new trade offer.');
    });
    return off;
  }, [gameId, refresh]);

  // ---------------------------------------------------------------- render
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !map) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.imageSmoothingEnabled = false;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, MAP_W, MAP_H);
    if (h3) renderH3(ctx, map, cam, tick, facingRef.current, movingRef.current);
    else renderFallback(ctx, map, cam);

    if (path && path.length > 1) {
      for (let i = 1; i < path.length; i++) {
        const p = path[i];
        const px = (p.x - cam.x) * TILE + TILE / 2;
        const py = (p.y - cam.y) * TILE + TILE / 2;
        ctx.fillStyle = 'rgba(0,0,0,0.6)';
        ctx.beginPath(); ctx.arc(px + 1, py + 1, 4, 0, 7); ctx.fill();
        ctx.fillStyle = i === path.length - 1 ? '#f0e040' : '#40d040';
        ctx.beginPath(); ctx.arc(px, py, 4, 0, 7); ctx.fill();
      }
    }

    if (hover && !h3) {
      const sx = (hover.x - cam.x) * TILE, sy = (hover.y - cam.y) * TILE;
      ctx.strokeStyle = 'rgba(255,255,255,0.6)';
      ctx.lineWidth = 1;
      ctx.strokeRect(sx + 0.5, sy + 0.5, TILE - 1, TILE - 1);
    }
  }, [map, cam, hover, path, h3, tick, assetGen]);

  // ---------------------------------------------------------------- input
  function toTile(e: React.MouseEvent<HTMLCanvasElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const scale = rect.width / MAP_W;
    const px = (e.clientX - rect.left) / scale;
    const py = (e.clientY - rect.top) / scale;
    return { x: Math.floor(px / TILE + cam.x), y: Math.floor(py / TILE + cam.y) };
  }

  function onMove(e: React.MouseEvent<HTMLCanvasElement>) {
    const t = toTile(e);
    if (map && t.x >= 0 && t.y >= 0 && t.x < map.width && t.y < map.height) {
      if (!hover || hover.x !== t.x || hover.y !== t.y) setHover(t);
    } else setHover(null);
  }

  async function onClick(e: React.MouseEvent<HTMLCanvasElement>) {
    if (!map || !hero || !game || movingRef.current || encounter || meeting) return;
    const t = toTile(e);
    if (t.x < 0 || t.y < 0 || t.x >= map.width || t.y >= map.height) return;

    // Your own town opens from anywhere (like the HoMM3 town list).
    const town = map.towns.find((tw) => tw.x === t.x && tw.y === t.y);
    if (town?.isMine) { navigate(`/game/${gameId}/town/${town.id}`); return; }

    // Other towns and heroes are visited by walking up to them.
    const other = map.heroes.find((h) => !h.isMine && h.x === t.x && h.y === t.y);
    const p = findPath(map, hero.x, hero.y, t.x, t.y);
    if (!p || p.length < 2) {
      if (town || other) toast('Your hero cannot find a way there.');
      return;
    }
    const end = await walk(other ? p.slice(0, -1) : p);
    if (!end || Math.max(Math.abs(end.x - t.x), Math.abs(end.y - t.y)) > 1) return;

    if (town) navigate(`/game/${gameId}/town/${town.id}`);
    else if (other) {
      const owner = game.players.find((pl) => pl.heroIds.includes(other.id));
      if (owner) setMeeting({ heroName: other.name, playerId: owner.playerId, ownerName: owner.name });
    }
  }

  async function walk(p: { x: number; y: number }[]): Promise<{ x: number; y: number } | null> {
    if (!hero) return null;
    let pos = { x: hero.x, y: hero.y };
    if (p.length < 2) return pos;
    movingRef.current = hero.id;
    setPath(p);
    try {
      for (let i = 1; i < p.length; i++) {
        facingRef.current.set(hero.id, facingFor(Math.sign(p[i].x - pos.x), Math.sign(p[i].y - pos.y)));
        const updated = await api.moveHero(hero.id, [p[i]]);
        pos = { x: updated.x, y: updated.y };
        setHero(updated);
        setHeroes((hs) => hs.map((h) => (h.id === updated.id ? updated : h)));
        setMap((m) => m ? {
          ...m,
          heroes: m.heroes.map((h) => h.id === updated.id
            ? { ...h, x: updated.x, y: updated.y, movement: updated.movement } : h)
        } : m);
        setCam((c) => {
          const m = map!;
          // keep the walking hero in view, scrolling like the original
          if (updated.x - c.x < 3 || updated.x - c.x > VIEW_TX - 4 || updated.y - c.y < 3 || updated.y - c.y > VIEW_TY - 4)
            return centerCam(m, updated.x, updated.y);
          return c;
        });
        if (updated.pendingEncounter) break;
        if (updated.movement <= 0) break;
        if (updated.x !== p[i].x || updated.y !== p[i].y) break;
        await new Promise((r) => setTimeout(r, 90));
      }
    } catch (e) {
      if (e instanceof ApiError) toast(e.message);
    } finally {
      movingRef.current = null;
      setPath(null);
      await refresh();
    }
    return pos;
  }

  async function sendMeetingMessage() {
    if (!gameId || !meeting || !meetingText.trim()) return;
    try {
      await api.sendMessage(gameId, meeting.playerId, meetingText.trim());
      toast(`Message sent to ${meeting.ownerName}.`);
      setMeetingText('');
      setMeeting(null);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    }
  }

  function onContextMenu(e: React.MouseEvent<HTMLCanvasElement>) {
    e.preventDefault();
    if (!map || !hero) return;
    const t = toTile(e);
    setPath(findPath(map, hero.x, hero.y, t.x, t.y));
  }

  function onWheel(e: React.WheelEvent<HTMLCanvasElement>) {
    if (!map) return;
    const d = Math.sign(e.deltaY);
    setCam((c) => clampCam(map, c.x + d * 2, c.y + d * 2));
  }

  async function resolveEncounter(action: string) {
    if (!hero) return;
    try {
      const r = await api.resolveEncounter(hero.id, action);
      toast(r.message ?? 'Done.');
      setEncounter(null);
      await refresh();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    }
  }

  if (!map || !game) return <Loading />;

  const me = game.players.find((p) => p.name.toLowerCase() === playerId);
  myPlayerIdRef.current = me?.playerId ?? null;
  const res = me?.resources;
  const myTowns = map.towns.filter((t) => t.isMine);
  const seat = me ? FACTION_SEAT[me.faction] : -1;
  const minesOwned = map.tiles.filter((t) => t.objectKind === 'mine' && t.owner === seat && seat >= 0).length;

  function selectHero(h: HeroDetail) {
    selectedRef.current = h.id;
    setHero(h);
    setPath(null);
    setCam(centerCam(map!, h.x, h.y));
    if (h.pendingEncounter) { try { setEncounter(JSON.parse(h.pendingEncounter)); } catch { /* ignore */ } }
    else setEncounter(null);
  }

  const actions: Record<string, () => void> = {
    Social: () => navigate(`/game/${gameId}/social`),
    Projects: () => navigate(`/game/${gameId}/projects`),
    Obelisk: () => navigate(`/game/${gameId}/obelisk`),
    Sleep: () => toast(`${hero?.name ?? 'Your hero'} rests until the next day.`),
    Hero: () => { if (hero) navigate(`/game/${gameId}/hero/${hero.id}`); },
    Spells: () => toast(`Spellbook: ${hero?.spells.length ?? 0} spells.`),
    Town: () => { const t = myTowns[0]; if (t) navigate(`/game/${gameId}/town/${t.id}`); },
    Menu: () => navigate('/lobby'),
    'Next Hero': () => {
      if (heroes.length === 0) return;
      const i = heroes.findIndex((h) => h.id === hero?.id);
      selectHero(heroes[(i + 1) % heroes.length]);
    },
    'End Turn': () => toast(`Turns advance automatically. Next day in ${mm}:${ss}.`)
  };

  const remaining = Math.max(0, Math.floor(
    (new Date(game.turnStartedAt).getTime() + game.turnSeconds * 1000 - now) / 1000));
  const mm = Math.floor(remaining / 60);
  const ss = String(remaining % 60).padStart(2, '0');

  const hoverTile = hover && map ? map.tiles[hover.y * map.width + hover.x] : null;
  const hoverTown = hover ? map.towns.find((t) => t.x === hover.x && t.y === hover.y) : null;
  const hoverHero = hover ? map.heroes.find((h) => h.x === hover.x && h.y === hover.y) : null;
  const rollover = hoverHero ? hoverHero.name
    : hoverTown ? hoverTown.name
      : hoverTile && hoverTile.fog > 0
        ? (hoverTile.objectKind ? objectName(hoverTile) : (hoverTile.terrain ? TERRAIN_NAME[hoverTile.terrain] : ''))
        : '';
  const heroLine = hero
    ? `Lvl ${hero.level} ${HERO_CLASS[hero.faction] ?? 'Hero'} · Move ${hero.movement}/${hero.maxMovement} · Mana ${hero.mana}/${hero.maxMana} · Next day in ${mm}:${ss}`
    : `Next day in ${mm}:${ss}`;
  const visibleHeroes = heroes.slice(heroScroll, heroScroll + 5);
  const visibleTowns = myTowns.slice(townScroll, townScroll + 5);
  const dateText = `Month: ${game.month}, Week: ${game.week}, Day: ${game.day}`;

  return (
    <>
      {/* frame */}
      {h3
        ? <Pcx name="advmap" x={0} y={0} w={800} h={600} />
        : <div className="abs advframe" style={{ left: 0, top: 0, width: 800, height: 600 }} />}

      {/* adventure map */}
      <canvas
        ref={canvasRef}
        width={MAP_W}
        height={MAP_H}
        aria-label="Adventure map"
        style={{ position: 'absolute', left: MAP_X, top: MAP_Y, width: MAP_W, height: MAP_H, cursor: h3 ? 'pointer' : 'crosshair' }}
        onMouseMove={onMove}
        onMouseLeave={() => setHover(null)}
        onClick={onClick}
        onContextMenu={onContextMenu}
        onWheel={onWheel}
      />

      <Minimap map={map} cam={cam} h3={h3} onJump={(x, y) => setCam(centerCam(map, x, y))} />

      {/* hero list */}
      <ListArrow h3={h3} def="IAM012" x={609} y={196} w={64} label="Scroll heroes up" onClick={() => setHeroScroll((s) => Math.max(0, s - 1))} />
      {[0, 1, 2, 3, 4].map((i) => {
        const h = visibleHeroes[i];
        if (!h) return null;
        const sel = h.id === hero?.id;
        return (
          <div key={h.id} className="abs clickable" style={{ left: 610, top: 212 + i * 32, width: 62, height: 32 }}
            role="button" aria-label={h.name} title={`${h.name} — ${h.movement}/${h.maxMovement} move`}
            onClick={() => selectHero(h)} onDoubleClick={() => navigate(`/game/${gameId}/hero/${h.id}`)}>
            {h3 ? (<>
              <DefImg name="imobil" frame={Math.min(25, Math.floor(h.movement / 100))} x={0} y={1} />
              <H3Img name={heroPortrait(h.heroKey || h.name, h.faction, 'small')} x={7} y={0} w={48} h={32}
                fallback={<HeroPortrait faction={h.faction} size={32} />} />
              <DefImg name="imana" frame={Math.min(25, Math.floor(h.mana / 5))} x={56} y={1} />
              {sel && <div className="abs" style={{ left: 7, top: 0, width: 46, height: 30, border: '1px solid #f8e060' }} />}
            </>) : (<>
              <Bar x={0} value={h.movement} max={h.maxMovement} color="#40c040" />
              <div className="abs" style={{ left: 7, top: 0, width: 48, height: 32, overflow: 'hidden', border: sel ? '1px solid #f8e060' : '1px solid var(--gold-0)' }}>
                <HeroPortrait faction={h.faction} size={32} />
              </div>
              <Bar x={56} value={h.mana} max={h.maxMana} color="#4070e0" />
            </>)}
          </div>
        );
      })}
      <ListArrow h3={h3} def="IAM013" x={609} y={372} w={64} label="Scroll heroes down" onClick={() => setHeroScroll((s) => Math.max(0, Math.min(heroes.length - 5, s + 1)))} />

      {/* town list */}
      <ListArrow h3={h3} def="IAM014" x={747} y={196} w={48} label="Scroll towns up" onClick={() => setTownScroll((s) => Math.max(0, s - 1))} />
      {[0, 1, 2, 3, 4].map((i) => {
        const t = visibleTowns[i];
        if (!t) return null;
        return (
          <div key={t.id} className="abs clickable" style={{ left: 748, top: 212 + i * 32, width: 48, height: 32 }}
            role="button" aria-label={t.name} title={t.name}
            onClick={() => setCam(centerCam(map, t.x, t.y))}
            onDoubleClick={() => navigate(`/game/${gameId}/town/${t.id}`)}>
            {h3 ? <DefImg name="ITPA" frame={townListFrame(t.faction)} x={0} y={0} /> : <TownThumb faction={t.faction} />}
          </div>
        );
      })}
      <ListArrow h3={h3} def="IAM015" x={747} y={372} w={48} label="Scroll towns down" onClick={() => setTownScroll((s) => Math.max(0, Math.min(myTowns.length - 5, s + 1)))} />

      {/* command buttons */}
      {BUTTONS.map((b) => (
        <button key={b.label} type="button" aria-label={b.label} title={b.tip}
          className={h3 ? 'h3hit' : 'btn h3btn-fallback'}
          style={{ position: 'absolute', left: b.x, top: b.y, width: b.w, height: 32, fontSize: 9, padding: 0 }}
          onMouseDown={() => setPressed(b.label)} onMouseUp={() => setPressed(null)} onMouseLeave={() => setPressed(null)}
          onClick={actions[b.label]}>
          {h3 ? <DefImg name={b.def} frame={pressed === b.label ? 1 : 0} x={0} y={0} /> : b.label}
        </button>
      ))}

      {/* info bar: selected hero */}
      <div className="abs" style={{ left: 605, top: 389, width: 175, height: 168 }}>
        {h3 && hero ? (<>
          <Pcx name="ADSTATHR" x={0} y={0} />
          <H3Img name={heroPortrait(hero.heroKey || hero.name, hero.faction, 'large')} x={3} y={2} w={58} h={64}
            fallback={<HeroPortrait faction={hero.faction} size={58} />} />
          <div className="abs h3text" style={{ left: 66, top: 3, fontSize: 12 }}>{hero.name}</div>
          {[hero.attack, hero.defense, hero.power, hero.knowledge].map((v, i) => (
            <div key={i} className="abs h3text" style={{ left: [77, 104, 132, 160][i] - 12, top: 52, width: 25, textAlign: 'center', fontSize: 12 }}>{v}</div>
          ))}
          <DefImg name="IMRL22" frame={3} x={5} y={74} />
          <DefImg name="ILCK22" frame={3} x={5} y={91} />
          <div className="abs h3text" style={{ left: 143, top: 94, width: 30, textAlign: 'center', fontSize: 10 }}>{hero.mana}</div>
          {hero.army.slice(0, 7).map((slot, i) => {
            const [sx, sy] = [[36, 73], [72, 73], [108, 73], [18, 122], [54, 122], [90, 122], [126, 122]][i];
            if (!slot.creature) return null;
            return (
              <div key={i}>
                <DefImg name="CPRSMALL" frame={creaturePortraitFrame(slot.creature)} x={sx} y={sy} />
                <div className="abs h3text" style={{ left: sx, top: sy + 34, width: 34, textAlign: 'center', fontSize: 10 }}>{slot.count}</div>
              </div>
            );
          })}
        </>) : hero ? (
          <div className="well" style={{ position: 'absolute', inset: 0 }}>
            <div className="abs" style={{ left: 4, top: 4 }}><HeroPortrait faction={hero.faction} size={56} /></div>
            <Txt x={66} y={4} size={12} bold>{hero.name}</Txt>
            <Txt x={66} y={22} size={10} color="var(--text-dim)">Att {hero.attack} · Def {hero.defense}</Txt>
            <Txt x={66} y={36} size={10} color="var(--text-dim)">Pow {hero.power} · Kno {hero.knowledge}</Txt>
            <Txt x={66} y={50} size={10} color="#80a0ff">Mana {hero.mana}</Txt>
            {hero.army.slice(0, 7).map((slot, i) => {
              const [sx, sy] = [[36, 73], [72, 73], [108, 73], [18, 118], [54, 118], [90, 118], [126, 118]][i];
              return (
                <div key={i} className="slot" style={{ left: sx, top: sy, width: 32, height: 40 }}>
                  {slot.creature && (<>
                    <CreatureIcon creature={slot.creature} size={30} />
                    <div style={{ position: 'absolute', bottom: 0, width: '100%', textAlign: 'center', fontSize: 9, color: '#fff' }}>{slot.count}</div>
                  </>)}
                </div>
              );
            })}
          </div>
        ) : null}
      </div>

      {/* status bar */}
      <div className={'abs' + (h3 ? ' h3text' : '')}
        style={{ left: 8, top: 556, width: 586, height: 18, textAlign: 'center', fontSize: 12, lineHeight: '18px', color: h3 ? undefined : 'var(--text-dim)' }}>
        {rollover || heroLine}
      </div>

      {/* resource bar */}
      {h3 ? <Pcx name="AResBar" x={3} y={575} /> : <div className="abs panel" style={{ left: 3, top: 575, width: 794, height: 22, borderRadius: 0 }} />}
      {RES_ORDER.map((r) => (
        <div key={r} className={'abs' + (h3 ? ' h3text' : '')} title={r}
          style={{ left: 3 + RESBAR_X[r], top: 578, fontSize: 12, display: 'flex', gap: 4, alignItems: 'center' }}>
          {!h3 && <span style={{ marginLeft: -22 }}><ResIcon r={r} size={16} /></span>}
          {res ? String(res[r]) : '—'}
        </div>
      ))}
      <div className={'abs' + (h3 ? ' h3text' : '')} title={`Mines claimed ${minesOwned} · Obelisks ${me?.obeliskVisits ?? 0}/10`}
        style={{ left: 3 + 619, top: 578, fontSize: 12 }}>{dateText}</div>

      {/* encounter dialog */}
      {encounter && (
        <div className="backdrop">
          <div className="modal panel" style={{ width: 400, height: 220 }}>
            <Txt x={0} y={20} w={400} size={16} bold align="center" className="heading">
              A Wandering Band
            </Txt>
            <Txt x={30} y={56} w={340} size={12} align="center" color="var(--text-dim)">
              {encounter.count} {creatureLabel(encounter.creature)} block the way. There is no need to fight.
            </Txt>
            <Btn x={30} y={100} w={340} h={32} label="Observe (gain experience)" size={12}
              onClick={() => resolveEncounter('observe')} />
            <Btn x={30} y={140} w={340} h={32} label="Recruit them (pay gold)" size={12}
              onClick={() => resolveEncounter('recruit')} />
            <Btn x={30} y={180} w={340} h={28} label="Pass by" size={12}
              onClick={() => resolveEncounter('pass')} />
          </div>
        </div>
      )}

      {/* hero meeting: replaces hero-vs-hero battle */}
      {meeting && (
        <div className="backdrop">
          <div className="modal panel" style={{ width: 420, height: 260 }} role="dialog" aria-label="Hero meeting">
            <Txt x={0} y={18} w={420} size={16} bold align="center" className="heading">
              You meet {meeting.heroName}
            </Txt>
            <Txt x={30} y={48} w={360} size={11} align="center" color="var(--text-dim)">
              {meeting.ownerName}&apos;s hero greets you on the road.
            </Txt>
            <textarea
              aria-label="Message"
              value={meetingText}
              maxLength={2000}
              onChange={(e) => setMeetingText(e.target.value)}
              placeholder="Say something…"
              style={{ position: 'absolute', left: 30, top: 74, width: 360, height: 70, resize: 'none' }}
            />
            <Btn x={30} y={156} w={360} h={30} label="Send message" size={12}
              disabled={!meetingText.trim()} onClick={sendMeetingMessage} />
            <Btn x={30} y={192} w={176} h={28} label="Offer a trade…" size={12}
              onClick={() => navigate(`/game/${gameId}/social?to=${meeting.playerId}`)} />
            <Btn x={214} y={192} w={176} h={28} label="Part ways" size={12}
              onClick={() => setMeeting(null)} />
          </div>
        </div>
      )}

    </>
  );
}

// ---------------------------------------------------------------- H3 art renderer

function viewBounds(cam: { x: number; y: number }) {
  const startX = Math.floor(cam.x), startY = Math.floor(cam.y);
  return { startX, startY, cols: Math.ceil(MAP_W / TILE) + 1, rows: Math.ceil(MAP_H / TILE) + 1 };
}

function pick<T>(list: T[], x: number, y: number, s: number): T {
  return list[Math.floor(hash(x, y, s) * list.length) % list.length];
}

function objectSprite(t: MapTile, x: number, y: number): { def: string; ox: number; animated: boolean } | null {
  switch (t.objectKind) {
    case 'tree': return { def: pick(TREE_DEFS[t.terrain ?? ''] ?? TREE_DEFS.default, x, y, 3), ox: 0, animated: false };
    case 'rock': return { def: pick(ROCK_DEFS, x, y, 4), ox: 0, animated: false };
    case 'mountain': return { def: pick(MOUNTAIN_DEFS, x, y, 5), ox: 0, animated: false };
    case 'mine': return { def: MINE_DEF[t.objectData ?? 'gold'] ?? MINE_DEF.gold, ox: 1, animated: true };
    case 'resource': return { def: RESOURCE_DEF[(t.objectData ?? 'gold').split(':')[0]] ?? RESOURCE_DEF.gold, ox: 0, animated: true };
    case 'treasure': return { def: OBJECT_DEF.treasure, ox: 0, animated: true };
    case 'artifact': return { def: artifactDef(artifactSeed(t.objectData)), ox: 0, animated: true };
    case 'shrine': return { def: OBJECT_DEF.shrine, ox: 1, animated: true };
    case 'grail': return { def: OBJECT_DEF.grail, ox: 0, animated: true };
    case 'monster': return { def: monsterDef((t.objectData ?? '').split(':')[0]), ox: 0, animated: true };
    default: return null;
  }
}

function anchorDraw(ctx: CanvasRenderingContext2D, s: Sheet, frame: number, px: number, py: number, ox: number, mirror = false) {
  drawFrame(ctx, s, frame, px + (1 + ox) * TILE - s.w, py + TILE - s.h, mirror);
}

const SEAT_FACTION = Object.fromEntries(Object.entries(FACTION_SEAT).map(([f, s]) => [s, f])) as Record<number, string>;

const FLIPS: [number, number][] = [[1, 1], [-1, 1], [1, -1], [-1, -1]];

// Shroud-edge lookup from VCMI MapRenderer.cpp (NeighborTilesInfo); ids >= 34 are left-right mirrors of SHROUD_ROT.
const SHROUD_ROT = [22, 15, 2, 13, 12, 16, 28, 17, 20, 19, 7, 24, 26, 25, 30, 32, 27];
const SHROUD_IDS = [
  -1, 34, 4, 4, 22, 23, 4, 4, 36, 36, 38, 38, 47, 47, 38, 38, 3, 25, 12, 12, 3, 25, 12, 12, 9, 9, 6, 6, 9, 9, 6, 6,
  35, 39, 48, 48, 41, 43, 48, 48, 36, 36, 38, 38, 47, 47, 38, 38, 26, 49, 28, 28, 26, 49, 28, 28, 9, 9, 6, 6, 9, 9, 6, 6,
  0, 45, 29, 29, 24, 33, 29, 29, 37, 37, 7, 7, 50, 50, 7, 7, 13, 27, 44, 44, 13, 27, 44, 44, 8, 8, 10, 10, 8, 8, 10, 10,
  0, 45, 29, 29, 24, 33, 29, 29, 37, 37, 7, 7, 50, 50, 7, 7, 13, 27, 44, 44, 13, 27, 44, 44, 8, 8, 10, 10, 8, 8, 10, 10,
  15, 17, 30, 30, 16, 19, 30, 30, 46, 46, 40, 40, 32, 32, 40, 40, 2, 25, 12, 12, 2, 25, 12, 12, 9, 9, 6, 6, 9, 9, 6, 6,
  18, 42, 31, 31, 20, 21, 31, 31, 46, 46, 40, 40, 32, 32, 40, 40, 26, 49, 28, 28, 26, 49, 28, 28, 9, 9, 6, 6, 9, 9, 6, 6,
  0, 45, 29, 29, 24, 33, 29, 29, 37, 37, 7, 7, 50, 50, 7, 7, 13, 27, 44, 44, 13, 27, 44, 44, 8, 8, 10, 10, 8, 8, 10, 10,
  0, 45, 29, 29, 24, 33, 29, 29, 37, 37, 7, 7, 50, 50, 7, 7, 13, 27, 44, 44, 13, 27, 44, 44, 8, 8, 10, 10, 8, 8, 10, 10
];

function neighbourTerrain(map: MapData, x: number, y: number): string | null {
  for (const [dx, dy] of [[0, 1], [0, -1], [-1, 0], [1, 0], [-1, -1], [1, -1], [-1, 1], [1, 1]]) {
    const nx = x + dx, ny = y + dy;
    if (nx < 0 || ny < 0 || nx >= map.width || ny >= map.height) continue;
    const n = map.tiles[ny * map.width + nx];
    if (n.fog !== 0 && n.terrain) return n.terrain;
  }
  return null;
}

function shroudFrame(map: MapData, x: number, y: number): number {
  const vis = (dx: number, dy: number) => {
    const nx = x + dx, ny = y + dy;
    if (nx < 0 || ny < 0 || nx >= map.width || ny >= map.height) return false;
    const t = map.tiles[ny * map.width + nx];
    return t.fog !== 0 && !!t.terrain;
  };
  const d1 = vis(0, 1), d3 = vis(-1, 0), d4 = vis(1, 0), d6 = vis(0, -1);
  const d0 = d1 || d3 || vis(-1, 1), d2 = d1 || d4 || vis(1, 1), d5 = d3 || d6 || vis(-1, -1), d7 = d4 || d6 || vis(1, -1);
  const bits = [d0, d1, d2, d3, d4, d5, d6, d7].reduce((a, v, i) => a | (v ? 1 << i : 0), 0);
  return SHROUD_IDS[bits];
}

/** Picks an H3 edge-transition frame range + flip for a tile, or null for a plain interior tile. */
function terrainView(map: MapData, x: number, y: number, terrain: string): { range: [number, number]; fx: number; fy: number } | null {
  const edges = edgeFrames(terrain);
  if (!edges) return null;
  const b = (dx: number, dy: number) => {
    const nx = x + dx, ny = y + dy;
    if (nx < 0 || ny < 0 || nx >= map.width || ny >= map.height) return null;
    return borderKind(terrain, map.tiles[ny * map.width + nx].terrain);
  };
  if (![-1, 0, 1].some(dy => [-1, 0, 1].some(dx => (dx || dy) && b(dx, dy)))) return null;
  // pattern checks in pattern space: border expected at left/top
  const patterns: [string, (p: (px: number, py: number) => 'D' | 'S' | null) => 'D' | 'S' | null][] = [
    ['left', p => p(-1, 0) && !p(0, -1) && !p(1, -1) && !p(1, 0) && !p(1, 1) && !p(0, 1) ? p(-1, 0) : null],
    ['top', p => p(0, -1) && !p(-1, 0) && !p(1, 0) && !p(-1, 1) && !p(0, 1) && !p(1, 1) ? p(0, -1) : null],
    ['inner', p => p(1, 1) && !p(-1, -1) && !p(0, -1) && !p(1, -1) && !p(-1, 0) && !p(1, 0) && !p(-1, 1) && !p(0, 1) ? p(1, 1) : null],
    ['outer', p => p(-1, -1) && p(0, -1) && p(-1, 0) && !p(1, 0) && !p(0, 1) && !p(1, 1) ? p(0, -1) : null],
    ['fallback', p => (p(0, -1) || p(-1, 0)) && !p(1, 0) && !p(0, 1) && !p(1, 1) ? (p(0, -1) || p(-1, 0)) : null]
  ];
  for (const [name, test] of patterns) {
    for (const [fx, fy] of FLIPS) {
      const kind = test((px, py) => b(px * fx, py * fy));
      if (kind) return { range: edges[name][kind === 'D' ? 0 : 1], fx, fy };
    }
  }
  return null;
}

function renderH3(ctx: CanvasRenderingContext2D, map: MapData, cam: { x: number; y: number }, tick: number,
  facing: Map<string, Facing>, moving: string | null) {
  const { startX, startY, cols, rows } = viewBounds(cam);
  const ox0 = (cam.x - startX) * TILE, oy0 = (cam.y - startY) * TILE;
  const at = (x: number, y: number) => map.tiles[y * map.width + x];

  // terrain
  for (let ry = 0; ry < rows; ry++) {
    for (let rx = 0; rx < cols; rx++) {
      const mx = startX + rx, my = startY + ry;
      if (mx < 0 || my < 0 || mx >= map.width || my >= map.height) continue;
      const t = at(mx, my);
      const terrain = t.terrain && t.fog !== 0 ? t.terrain : shroudFrame(map, mx, my) >= 0 ? neighbourTerrain(map, mx, my) : null;
      if (!terrain) continue;
      const sx = rx * TILE - ox0, sy = ry * TILE - oy0;
      const td = TERRAIN_DEF[terrain] ?? TERRAIN_DEF.grass;
      const s = getDef(td.def);
      if (s) {
        const view = terrain === t.terrain ? terrainView(map, mx, my, terrain) : null;
        if (view) {
          const [lo, hi] = view.range;
          drawFrame(ctx, s, Math.min(lo + Math.floor(hash(mx, my, 11) * (hi - lo + 1)), s.count - 1), sx, sy, view.fx < 0, view.fy < 0);
        } else {
          const [lo, hi] = td.frames;
          const f = lo + Math.floor(hash(mx, my, 11) * (hi - lo + 1));
          drawFrame(ctx, s, Math.min(f, s.count - 1), sx, sy, hash(mx, my, 12) > 0.5);
        }
      } else {
        ctx.fillStyle = TERRAIN_COLOR[terrain] ?? '#5a8a3a';
        ctx.fillRect(sx, sy, TILE, TILE);
      }
    }
  }

  // objects, towns and heroes sorted by their base row (painter's order)
  type D = { y: number; x: number; draw: () => void };
  const list: D[] = [];
  for (let my = startY - 1; my < startY + rows + 6; my++) {
    for (let mx = startX - 3; mx < startX + cols + 4; mx++) {
      if (mx < 0 || my < 0 || mx >= map.width || my >= map.height) continue;
      const t = at(mx, my);
      if (!t.objectKind || t.fog === 0) continue;
      const spr = objectSprite(t, mx, my);
      if (!spr) continue;
      const s = getDef(spr.def) ?? (t.objectKind === 'monster' ? getDef('AVWMON1') : null);
      if (!s) continue;
      const px = (mx - cam.x) * TILE, py = (my - cam.y) * TILE;
      list.push({
        y: my, x: mx, draw: () => {
          const frame = spr.animated ? groupFrame(s, 0, tick + mx * 3 + my) : 0;
          anchorDraw(ctx, s, frame, px, py, spr.ox, t.objectKind === 'monster' && hash(mx, my, 9) > 0.5);
          if (t.objectKind === 'mine' && t.owner >= 0) {
            const f = getDef(flagDef(SEAT_FACTION[t.owner] ?? 'castle'));
            if (f) drawFrame(ctx, f, groupFrame(f, 2, tick), px - 40, py - 30);
          }
        }
      });
    }
  }
  for (const town of map.towns) {
    const s = getDef(TOWN_MAP_DEF[town.faction] ?? 'AVCcasx0');
    if (!s) continue;
    const px = (town.x - cam.x) * TILE, py = (town.y - cam.y) * TILE;
    if (px < -4 * TILE || py < -TILE || px > MAP_W + 6 * TILE || py > MAP_H + 6 * TILE) continue;
    list.push({ y: town.y, x: town.x + 2, draw: () => anchorDraw(ctx, s, 0, px, py, 2) });
  }
  for (const h of map.heroes) {
    const px = (h.x - cam.x) * TILE, py = (h.y - cam.y) * TILE;
    if (px < -3 * TILE || py < -TILE || px > MAP_W + 3 * TILE || py > MAP_H + 2 * TILE) continue;
    const s = getDef(heroMapDef(h.name, h.faction)) ?? getDef('AH00_');
    const fl = getDef(flagDef(h.faction));
    if (!s) continue;
    const dir = facing.get(h.id) ?? { group: 2, mirror: false };
    const isMoving = moving === h.id;
    list.push({
      y: h.y, x: 1e6 + h.x, draw: () => {
        const g = isMoving ? dir.group + 5 : dir.group;
        anchorDraw(ctx, s, groupFrame(s, g, tick), px, py, 1, dir.mirror);
        if (fl) anchorDraw(ctx, fl, groupFrame(fl, g, tick), px, py, 1, dir.mirror);
      }
    });
  }
  list.sort((a, b) => a.y - b.y || a.x - b.x);
  for (const d of list) d.draw();

  // fog of war
  for (let ry = 0; ry < rows; ry++) {
    for (let rx = 0; rx < cols; rx++) {
      const mx = startX + rx, my = startY + ry;
      const sx = rx * TILE - ox0, sy = ry * TILE - oy0;
      if (mx < 0 || my < 0 || mx >= map.width || my >= map.height) { ctx.fillStyle = '#000'; ctx.fillRect(sx, sy, TILE, TILE); continue; }
      const t = at(mx, my);
      if (t.fog === 0 || !t.terrain) {
        const shroud = shroudFrame(map, mx, my);
        const edge = getDef('TSHRE'), full = getDef('TSHRC');
        if (shroud >= 0 && edge) {
          drawFrame(ctx, edge, shroud < 34 ? shroud : SHROUD_ROT[shroud - 34], sx, sy, shroud >= 34);
        } else if (shroud < 0 && full) {
          drawFrame(ctx, full, (((mx * 997) ^ (my * 1009)) / 101 | 0) % full.count, sx, sy);
        } else { ctx.fillStyle = '#000'; ctx.fillRect(sx, sy, TILE, TILE); }
      }
      else if (t.fog === 1) { ctx.fillStyle = 'rgba(0,0,0,0.45)'; ctx.fillRect(sx, sy, TILE, TILE); }
    }
  }
}

// ---------------------------------------------------------------- built-in art renderer

function renderFallback(ctx: CanvasRenderingContext2D, map: MapData, cam: { x: number; y: number }) {
  const { startX, startY, cols, rows } = viewBounds(cam);
  for (let ry = 0; ry < rows; ry++) {
    for (let rx = 0; rx < cols; rx++) {
      const mx = startX + rx, my = startY + ry;
      if (mx < 0 || my < 0 || mx >= map.width || my >= map.height) continue;
      const t = map.tiles[my * map.width + mx];
      const sx = rx * TILE - (cam.x - startX) * TILE;
      const sy = ry * TILE - (cam.y - startY) * TILE;

      if (t.fog === 0 || !t.terrain) {
        ctx.fillStyle = '#07070a';
        ctx.fillRect(sx, sy, TILE, TILE);
        continue;
      }

      const base = TERRAIN_COLOR[t.terrain] ?? '#5a8a3a';
      ctx.fillStyle = base;
      ctx.fillRect(sx, sy, TILE, TILE);
      for (let py = 0; py < TILE; py += 4) {
        for (let px = 0; px < TILE; px += 4) {
          const n = hash(mx * TILE + px, my * TILE + py, t.variant);
          if (n > 0.6) {
            ctx.fillStyle = shade(base, (n - 0.6) * 0.5);
            ctx.fillRect(sx + px, sy + py, 4, 4);
          }
        }
      }
      if (t.objectKind) drawObject(ctx, t, sx, sy);
      if (t.fog === 1) {
        ctx.fillStyle = 'rgba(0,0,0,0.5)';
        ctx.fillRect(sx, sy, TILE, TILE);
      }
    }
  }
  for (const town of map.towns) {
    const sx = (town.x - cam.x) * TILE, sy = (town.y - cam.y) * TILE;
    if (sx < -TILE || sy < -TILE || sx > MAP_W || sy > MAP_H) continue;
    drawTown(ctx, sx, sy, FACTION_COLOR[town.faction] ?? '#888');
  }
  for (const h of map.heroes) {
    const sx = (h.x - cam.x) * TILE, sy = (h.y - cam.y) * TILE;
    if (sx < -TILE || sy < -TILE || sx > MAP_W || sy > MAP_H) continue;
    drawHero(ctx, sx, sy, FACTION_COLOR[h.faction] ?? '#888', h.isMine);
  }
}

// ---------------------------------------------------------------- small widgets

function ListArrow(props: { h3: boolean; def: string; x: number; y: number; w: number; label: string; onClick: () => void }) {
  const { h3, def, x, y, w, label, onClick } = props;
  return (
    <button type="button" aria-label={label} title={label} className={h3 ? 'h3hit' : 'btn'}
      style={{ position: 'absolute', left: x, top: y, width: w, height: 16, padding: 0, fontSize: 9 }} onClick={onClick}>
      {h3 ? <DefImg name={def} frame={0} x={0} y={0} /> : (def === 'IAM012' || def === 'IAM014' ? '▲' : '▼')}
    </button>
  );
}

function Bar(props: { x: number; value: number; max: number; color: string }) {
  const hgt = Math.round(30 * Math.min(1, props.value / Math.max(1, props.max)));
  return (
    <div className="abs" style={{ left: props.x, top: 1, width: 6, height: 30, background: '#000' }}>
      <div style={{ position: 'absolute', bottom: 0, width: 6, height: hgt, background: props.color }} />
    </div>
  );
}

/** An original bitmap with a built-in fallback if this PCX is absent from the player's files. */
function H3Img(props: { name: string; x: number; y: number; w: number; h: number; fallback: React.ReactNode }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [props.name]);
  if (failed) return <div className="abs" style={{ left: props.x, top: props.y, width: props.w, height: props.h, overflow: 'hidden' }}>{props.fallback}</div>;
  return (
    <img src={`/h3/pcx/${props.name}`} alt="" draggable={false} onError={() => setFailed(true)}
      style={{ position: 'absolute', left: props.x, top: props.y, width: props.w, height: props.h, pointerEvents: 'none' }} />
  );
}

// ---------------------------------------------------------------- drawing

function drawObject(ctx: CanvasRenderingContext2D, t: MapTile, sx: number, sy: number) {
  const kind = t.objectKind;
  if (kind === 'tree') {
    ctx.fillStyle = 'rgba(0,0,0,0.3)';
    ctx.beginPath(); ctx.ellipse(sx + 16, sy + 28, 9, 3, 0, 0, 7); ctx.fill();
    ctx.fillStyle = '#4a3020'; ctx.fillRect(sx + 14, sy + 18, 4, 10);
    ctx.fillStyle = '#3a6a2a'; ctx.beginPath(); ctx.arc(sx + 16, sy + 14, 10, 0, 7); ctx.fill();
    ctx.fillStyle = '#4a8a3a'; ctx.beginPath(); ctx.arc(sx + 13, sy + 11, 6, 0, 7); ctx.fill();
  } else if (kind === 'rock') {
    ctx.fillStyle = '#7a7a7a';
    ctx.beginPath();
    ctx.moveTo(sx + 6, sy + 26); ctx.lineTo(sx + 10, sy + 12);
    ctx.lineTo(sx + 20, sy + 8); ctx.lineTo(sx + 27, sy + 18);
    ctx.lineTo(sx + 24, sy + 26); ctx.closePath(); ctx.fill();
    ctx.fillStyle = '#9a9a9a';
    ctx.beginPath();
    ctx.moveTo(sx + 10, sy + 12); ctx.lineTo(sx + 20, sy + 8);
    ctx.lineTo(sx + 18, sy + 16); ctx.closePath(); ctx.fill();
  } else if (kind === 'mountain') {
    ctx.fillStyle = '#5a5a62';
    ctx.beginPath();
    ctx.moveTo(sx + 2, sy + 30); ctx.lineTo(sx + 16, sy + 2);
    ctx.lineTo(sx + 30, sy + 30); ctx.closePath(); ctx.fill();
    ctx.fillStyle = '#e8e8f0';
    ctx.beginPath();
    ctx.moveTo(sx + 11, sy + 12); ctx.lineTo(sx + 16, sy + 2);
    ctx.lineTo(sx + 21, sy + 12); ctx.lineTo(sx + 16, sy + 9); ctx.closePath(); ctx.fill();
  } else if (kind === 'mine') {
    ctx.fillStyle = '#5a5048';
    ctx.beginPath();
    ctx.moveTo(sx + 4, sy + 28); ctx.lineTo(sx + 16, sy + 6);
    ctx.lineTo(sx + 28, sy + 28); ctx.closePath(); ctx.fill();
    ctx.fillStyle = '#1a1410';
    ctx.beginPath(); ctx.arc(sx + 16, sy + 24, 6, Math.PI, 0); ctx.fill();
    ctx.fillRect(sx + 10, sy + 24, 12, 4);
    ctx.fillStyle = RES_COLOR[t.objectData ?? 'gold'] ?? '#e8c040';
    ctx.beginPath(); ctx.arc(sx + 16, sy + 10, 3, 0, 7); ctx.fill();
  } else if (kind === 'resource') {
    const type = (t.objectData ?? 'gold').split(':')[0];
    ctx.fillStyle = RES_COLOR[type] ?? '#e8c040';
    ctx.beginPath(); ctx.arc(sx + 16, sy + 18, 7, 0, 7); ctx.fill();
    ctx.fillStyle = 'rgba(255,255,255,0.5)';
    ctx.beginPath(); ctx.arc(sx + 13, sy + 15, 2.5, 0, 7); ctx.fill();
  } else if (kind === 'treasure') {
    ctx.fillStyle = '#8a5a20'; ctx.fillRect(sx + 7, sy + 14, 18, 12);
    ctx.fillStyle = '#a06a28'; ctx.beginPath(); ctx.arc(sx + 16, sy + 14, 9, Math.PI, 0); ctx.fill();
    ctx.fillStyle = '#e0c040'; ctx.fillRect(sx + 14, sy + 14, 4, 6);
  } else if (kind === 'artifact') {
    ctx.fillStyle = '#e0c040';
    ctx.beginPath();
    ctx.moveTo(sx + 16, sy + 6); ctx.lineTo(sx + 22, sy + 16);
    ctx.lineTo(sx + 16, sy + 26); ctx.lineTo(sx + 10, sy + 16);
    ctx.closePath(); ctx.fill();
  } else if (kind === 'shrine') {
    ctx.fillStyle = '#a0a0a8'; ctx.fillRect(sx + 8, sy + 20, 16, 8); ctx.fillRect(sx + 11, sy + 8, 10, 12);
    ctx.fillStyle = '#c0c0c8';
    ctx.beginPath();
    ctx.moveTo(sx + 9, sy + 8); ctx.lineTo(sx + 16, sy + 2);
    ctx.lineTo(sx + 23, sy + 8); ctx.closePath(); ctx.fill();
    ctx.fillStyle = '#60c0ff'; ctx.beginPath(); ctx.arc(sx + 16, sy + 14, 3, 0, 7); ctx.fill();
  } else if (kind === 'monster') {
    ctx.fillStyle = '#8a2a2a'; ctx.beginPath(); ctx.arc(sx + 16, sy + 16, 9, 0, 7); ctx.fill();
    ctx.fillStyle = '#ffd040';
    ctx.beginPath(); ctx.arc(sx + 12, sy + 14, 2, 0, 7); ctx.fill();
    ctx.beginPath(); ctx.arc(sx + 20, sy + 14, 2, 0, 7); ctx.fill();
  } else if (kind === 'grail') {
    ctx.fillStyle = '#e0c040';
    ctx.beginPath();
    ctx.moveTo(sx + 10, sy + 10); ctx.lineTo(sx + 22, sy + 10);
    ctx.lineTo(sx + 19, sy + 24); ctx.lineTo(sx + 13, sy + 24);
    ctx.closePath(); ctx.fill();
    ctx.fillStyle = '#fff8c0'; ctx.fillRect(sx + 9, sy + 8, 14, 3);
  }
}

// Town drawn ~2x2 tiles: gate sits on the town tile, walls extend up and to the sides.
function drawTown(ctx: CanvasRenderingContext2D, sx: number, sy: number, color: string) {
  const L = sx - 16, T = sy - 22;
  ctx.fillStyle = 'rgba(0,0,0,0.45)';
  ctx.beginPath(); ctx.ellipse(sx + 16, sy + 30, 34, 7, 0, 0, 7); ctx.fill();
  // curtain wall
  ctx.fillStyle = '#9a8e78'; ctx.fillRect(L + 4, T + 30, 56, 24);
  ctx.fillStyle = '#7a705e'; ctx.fillRect(L + 4, T + 50, 56, 4);
  for (let i = 0; i < 7; i++) { ctx.fillStyle = '#b0a48c'; ctx.fillRect(L + 4 + i * 8, T + 26, 5, 5); }
  // keep
  ctx.fillStyle = '#b8ac94'; ctx.fillRect(L + 22, T + 8, 20, 30);
  ctx.fillStyle = '#8a7e68'; ctx.fillRect(L + 38, T + 8, 4, 30);
  // side towers
  for (const tx of [L, L + 52]) {
    ctx.fillStyle = '#a89c84'; ctx.fillRect(tx, T + 18, 12, 36);
    ctx.fillStyle = '#7a705e'; ctx.fillRect(tx + 9, T + 18, 3, 36);
    ctx.fillStyle = color;
    ctx.beginPath(); ctx.moveTo(tx - 2, T + 18); ctx.lineTo(tx + 6, T + 4); ctx.lineTo(tx + 14, T + 18); ctx.closePath(); ctx.fill();
  }
  // keep roof + banner
  ctx.fillStyle = color;
  ctx.beginPath(); ctx.moveTo(L + 19, T + 8); ctx.lineTo(L + 32, T - 10); ctx.lineTo(L + 45, T + 8); ctx.closePath(); ctx.fill();
  ctx.fillStyle = '#3a2a18'; ctx.fillRect(L + 31, T - 20, 2, 12);
  ctx.fillStyle = color; ctx.fillRect(L + 33, T - 20, 9, 6);
  // windows + gate
  ctx.fillStyle = '#2a2018'; ctx.fillRect(L + 28, T + 14, 3, 5); ctx.fillRect(L + 34, T + 14, 3, 5);
  ctx.fillStyle = '#241a10';
  ctx.beginPath(); ctx.moveTo(sx + 9, sy + 32); ctx.lineTo(sx + 9, sy + 22); ctx.arc(sx + 16, sy + 22, 7, Math.PI, 0); ctx.lineTo(sx + 23, sy + 32); ctx.closePath(); ctx.fill();
}

// Mounted hero with a faction banner, HoMM3 style.
function drawHero(ctx: CanvasRenderingContext2D, sx: number, sy: number, color: string, mine: boolean) {
  ctx.fillStyle = 'rgba(0,0,0,0.45)';
  ctx.beginPath(); ctx.ellipse(sx + 16, sy + 29, 13, 3.5, 0, 0, 7); ctx.fill();
  // horse
  ctx.fillStyle = '#6a4a2a';
  ctx.beginPath(); ctx.ellipse(sx + 15, sy + 20, 10, 5, 0, 0, 7); ctx.fill();
  ctx.fillRect(sx + 7, sy + 22, 2, 7); ctx.fillRect(sx + 11, sy + 23, 2, 6);
  ctx.fillRect(sx + 18, sy + 23, 2, 6); ctx.fillRect(sx + 22, sy + 22, 2, 7);
  ctx.beginPath(); ctx.moveTo(sx + 22, sy + 18); ctx.lineTo(sx + 29, sy + 11); ctx.lineTo(sx + 31, sy + 14); ctx.lineTo(sx + 25, sy + 21); ctx.closePath(); ctx.fill();
  ctx.fillStyle = '#2a1a0a'; ctx.fillRect(sx + 4, sy + 17, 3, 7);
  // caparison in faction colour
  ctx.fillStyle = color; ctx.fillRect(sx + 9, sy + 17, 12, 6);
  // rider
  ctx.fillStyle = '#c8c8d0'; ctx.fillRect(sx + 12, sy + 8, 6, 10);
  ctx.fillStyle = '#d8b090'; ctx.beginPath(); ctx.arc(sx + 15, sy + 6, 3, 0, 7); ctx.fill();
  ctx.fillStyle = '#909098'; ctx.fillRect(sx + 12, sy + 2, 6, 3);
  // banner pole
  ctx.fillStyle = '#3a2a18'; ctx.fillRect(sx + 9, sy - 4, 2, 22);
  ctx.fillStyle = color;
  ctx.beginPath(); ctx.moveTo(sx + 11, sy - 4); ctx.lineTo(sx + 1, sy - 1); ctx.lineTo(sx + 11, sy + 3); ctx.closePath(); ctx.fill();
  if (mine) {
    ctx.strokeStyle = 'rgba(255,208,64,0.9)'; ctx.lineWidth = 1.5;
    ctx.beginPath(); ctx.ellipse(sx + 16, sy + 29, 14, 4, 0, 0, 7); ctx.stroke();
  }
}

const RES_COLOR: Record<string, string> = {
  wood: '#8a5a2b', ore: '#7a7a84', mercury: '#c0c0d0', sulfur: '#e8d060',
  crystal: '#60c0e0', gems: '#40c060', gold: '#e8c040'
};

function TownThumb({ faction }: { faction: string }) {
  const color = FACTION_COLOR[faction] ?? '#888';
  return (
    <svg width={68} height={44} viewBox="0 0 68 44" style={{ position: 'absolute', left: 0, top: 0 }}>
      <rect width={68} height={44} fill="#3a5a2a" />
      <rect y={30} width={68} height={14} fill="#2a4a1e" />
      <rect x={12} y={20} width={44} height={18} fill="#9a8e78" />
      <rect x={26} y={8} width={16} height={30} fill="#b8ac94" />
      <rect x={8} y={14} width={10} height={24} fill="#a89c84" />
      <rect x={50} y={14} width={10} height={24} fill="#a89c84" />
      <path d="M6 14 L13 4 L20 14Z M48 14 L55 4 L62 14Z M23 8 L34 -4 L45 8Z" fill={color} />
      <path d="M30 38 V31 A4 4 0 0 1 38 31 V38Z" fill="#241a10" />
    </svg>
  );
}
function CreatureIcon({ creature, size }: { creature: string; size: number }) {
  const tier = creatureTier(creature);
  const hue = (tier * 40) % 360;
  return (
    <svg width={size} height={size} viewBox="0 0 38 38">
      <circle cx={19} cy={19} r={13} fill={`hsl(${hue},45%,45%)`} />
      <circle cx={14} cy={16} r={2.5} fill="#ffd040" />
      <circle cx={24} cy={16} r={2.5} fill="#ffd040" />
      <path d="M14 24 Q19 28 24 24" stroke="#1a1410" strokeWidth={1.5} fill="none" />
    </svg>
  );
}

function creatureTier(key: string): number {
  const tiers: Record<string, number> = {
    pikeman: 1, halberdier: 1, archer: 2, marksman: 2, griffin: 3, royalGriffin: 3,
    swordsman: 4, crusader: 4, monk: 5, zealot: 5, cavalier: 6, champion: 6,
    angel: 7, archangel: 7, centaur: 1, centaurCaptain: 1, dwarf: 2, battleDwarf: 2,
    woodElf: 3, grandElf: 3, pegasus: 4, silverPegasus: 4, dendroid: 5, dendroidSoldier: 5,
    unicorn: 6, warUnicorn: 6, greenDragon: 7, goldDragon: 7, skeleton: 1, skeletonWarrior: 1,
    walkingDead: 2, zombie: 2, wight: 3, wraith: 3, vampire: 4, vampireLord: 4,
    lich: 5, powerLich: 5, blackKnight: 6, dreadKnight: 6, boneDragon: 7, ghostDragon: 7
  };
  return tiers[key] ?? 1;
}

function creatureLabel(key: string): string {
  return key.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/^./, (c) => c.toUpperCase()) + 's';
}

function objectName(t: MapTile): string {
  switch (t.objectKind) {
    case 'mine': return `${t.objectData} Mine${t.owner >= 0 ? ' (claimed)' : ''}`;
    case 'resource': return `${(t.objectData ?? '').split(':')[1]} ${(t.objectData ?? '').split(':')[0]}`;
    case 'treasure': return 'Treasure Chest';
    case 'artifact': return 'Artifact';
    case 'shrine': return 'Shrine';
    case 'monster': {
      const [c, n] = (t.objectData ?? '').split(':');
      return c && n ? `${n} ${creatureLabel(c)}` : 'Wandering Monsters';
    }
    case 'grail': return 'The Grail';
    case 'tree': return 'Trees';
    case 'rock': return 'Rocks';
    case 'mountain': return 'Mountains';
    case 'town': return 'Town';
    default: return '';
  }
}

function Minimap(props: { map: MapData; cam: { x: number; y: number }; h3: boolean; onJump: (x: number, y: number) => void }) {
  const { map, cam, h3, onJump } = props;
  const ref = useRef<HTMLCanvasElement>(null);
  const W = 144, H = 144;

  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const ctx = c.getContext('2d');
    if (!ctx) return;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, W, H);
    const sx = W / map.width, sy = H / map.height;
    const palette = h3 ? MINIMAP_COLOR : TERRAIN_COLOR;
    for (let y = 0; y < map.height; y++) {
      for (let x = 0; x < map.width; x++) {
        const t = map.tiles[y * map.width + x];
        if (t.fog === 0 || !t.terrain) continue;
        const blocked = !t.passable && t.terrain !== 'water';
        ctx.fillStyle = blocked && h3 ? shade(palette[t.terrain] ?? '#333', -0.45) : (palette[t.terrain] ?? '#333');
        ctx.fillRect(x * sx, y * sy, Math.ceil(sx), Math.ceil(sy));
      }
    }
    for (const t of map.towns) {
      ctx.fillStyle = FACTION_COLOR[t.faction] ?? '#fff';
      ctx.fillRect(t.x * sx - 2, t.y * sy - 2, 5, 5);
    }
    for (const h of map.heroes) {
      ctx.fillStyle = FACTION_COLOR[h.faction] ?? '#fff';
      ctx.fillRect(h.x * sx - 1, h.y * sy - 1, 3, 3);
    }
    ctx.strokeStyle = h3 ? '#ffffff' : '#ff4040';
    ctx.lineWidth = 1;
    ctx.strokeRect(cam.x * sx + 0.5, cam.y * sy + 0.5, (MAP_W / TILE) * sx, (MAP_H / TILE) * sy);
  }, [map, cam, h3]);

  return (
    <canvas
      ref={ref}
      width={W}
      height={H}
      aria-label="Minimap"
      className={h3 ? undefined : 'well'}
      style={{ position: 'absolute', left: 630, top: 26, width: W, height: H, cursor: 'pointer' }}
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        onJump(Math.floor((e.clientX - r.left) / r.width * map.width), Math.floor((e.clientY - r.top) / r.height * map.height));
      }}
    />
  );
}
function findPath(map: MapData, sx: number, sy: number, tx: number, ty: number): { x: number; y: number }[] | null {
  const W = map.width, H = map.height;
  if (sx === tx && sy === ty) return [{ x: sx, y: sy }];
  const key = (x: number, y: number) => y * W + x;
  const start = key(sx, sy), goal = key(tx, ty);
  const g = new Float64Array(W * H).fill(Infinity);
  const came = new Int32Array(W * H).fill(-1);
  const closed = new Uint8Array(W * H);
  g[start] = 0;
  const h = (i: number) => Math.max(Math.abs((i % W) - tx), Math.abs(Math.floor(i / W) - ty));
  const occupied = new Set(map.heroes.map((hh) => key(hh.x, hh.y)));
  const open: { i: number; f: number }[] = [{ i: start, f: h(start) }];
  const dirs = [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 1], [1, -1], [-1, 1], [-1, -1]];
  let iter = 0;
  // mines, monsters, towns and heroes are reached by stepping "into" them from next door
  const visitable = (i: number) => occupied.has(i) ||
    ['mine', 'monster', 'town'].includes(map.tiles[i].objectKind ?? '');

  while (open.length && iter++ < 20000) {
    let bi = 0;
    for (let i = 1; i < open.length; i++) if (open[i].f < open[bi].f) bi = i;
    const cur = open.splice(bi, 1)[0];
    if (cur.i === goal) {
      const path: { x: number; y: number }[] = [];
      let c = goal;
      while (c !== -1) { path.push({ x: c % W, y: Math.floor(c / W) }); c = came[c]; }
      return path.reverse();
    }
    if (closed[cur.i]) continue;
    closed[cur.i] = 1;
    const cx = cur.i % W, cy = Math.floor(cur.i / W);
    for (const [dx, dy] of dirs) {
      const nx = cx + dx, ny = cy + dy;
      if (nx < 0 || ny < 0 || nx >= W || ny >= H) continue;
      const ni = key(nx, ny);
      if (closed[ni]) continue;
      const t = map.tiles[ni];
      if (t.fog === 0) continue;
      if (ni === goal) {
        if (!t.passable && !visitable(ni)) continue;
      } else if (!t.passable || occupied.has(ni)) continue;
      if (dx && dy) {
        const a = map.tiles[key(cx + dx, cy)], b = map.tiles[key(cx, cy + dy)];
        if (!a.passable || !b.passable) continue;
      }
      const ng = g[cur.i] + 1;
      if (ng < g[ni]) {
        g[ni] = ng;
        came[ni] = cur.i;
        open.push({ i: ni, f: ng + h(ni) });
      }
    }
  }
  return null;
}