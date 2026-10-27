import { useEffect, useState, useCallback } from 'react';
import { useParams, useSearchParams } from 'react-router-dom';
import { useGameNavigate } from '../useGameNavigate';
import { api, ApiError } from '../api';
import type { GameDetail } from '../types';
import { Btn, Loading, Panel, Txt } from '../ui/Layout';
import { toast } from '../ui/Toast';
import { socket } from '../ws';
import { FACTION_COLOR, FACTION_LABEL, HeroPortrait, ResIcon } from '../ui/homm';

const RES = ['wood', 'ore', 'mercury', 'sulfur', 'crystal', 'gems', 'gold'] as const;

export function Social() {
  const { gameId } = useParams<{ gameId: string }>();
  const navigate = useGameNavigate();
  const [search] = useSearchParams();
  const [game, setGame] = useState<GameDetail | null>(null);
  const [messages, setMessages] = useState<{ id: string; from: string; to: string; body: string; mine: boolean }[]>([]);
  const [trades, setTrades] = useState<{ id: string; from: string; to: string; give: string; giveAmount: number; want: string; wantAmount: number; status: string; incoming: boolean }[]>([]);
  const [draft, setDraft] = useState('');
  const [toPlayer, setToPlayer] = useState(search.get('to') ?? '');
  const [give, setGive] = useState('wood');
  const [giveAmt, setGiveAmt] = useState(5);
  const [want, setWant] = useState('ore');
  const [wantAmt, setWantAmt] = useState(5);

  const load = useCallback(async () => {
    if (!gameId) return;
    try {
      const [g, m, t] = await Promise.all([api.getGame(gameId), api.getMessages(gameId), api.getTrades(gameId)]);
      setGame(g);
      setMessages(m);
      setTrades(t);
      if (!toPlayer && g.players.length) {
        const me = localStorage.getItem('cityoftowers.player') ?? '';
        // default to another human player, not a computer seat
        const others = g.players.filter((p) => p.name.toLowerCase() !== me);
        const pick = others.find((p) => !p.isAi) ?? others[0];
        if (pick) setToPlayer(pick.playerId);
      }
    } catch (e) {
      if (e instanceof ApiError) toast(e.message);
    }
  }, [gameId, toPlayer]);

  useEffect(() => { load(); }, [load]);

  useEffect(() => {
    if (!gameId) return;
    socket.connect(gameId);
    return socket.on((e) => { if (e.type === 'message' || e.type === 'trade') load(); });
  }, [gameId, load]);

  if (!game) return <Loading />;

  const me = localStorage.getItem('cityoftowers.player') ?? '';
  const others = game.players.filter((p) => p.name.toLowerCase() !== me);

  async function send() {
    if (!draft.trim() || !toPlayer) return;
    try {
      await api.sendMessage(gameId!, toPlayer, draft.trim());
      setDraft('');
      await load();
    } catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
  }

  async function offer() {
    if (!toPlayer) return;
    try {
      await api.offerTrade(gameId!, toPlayer, give, giveAmt, want, wantAmt);
      toast('Trade offered.');
      await load();
    } catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
  }

  async function respond(id: string, accept: boolean) {
    try {
      await api.respondTrade(id, accept);
      toast(accept ? 'Trade accepted.' : 'Trade declined.');
      await load();
    } catch (e) { toast(e instanceof ApiError ? e.message : 'Something went wrong.'); }
  }

  return (
    <>
      <Panel x={0} y={0} w={800} h={40} style={{ borderRadius: 0, borderLeft: 'none', borderRight: 'none', borderTop: 'none' }}>
        <Txt x={16} y={12} size={15} bold className="heading">SOCIAL</Txt>
        <Btn x={700} y={6} w={90} h={28} label="Back" size={11} onClick={() => navigate(`/game/${gameId}`)} />
      </Panel>

      {/* messages */}
      <Panel x={16} y={52} w={380} h={250}>
        <Txt x={12} y={8} size={12} bold className="heading">MESSAGES</Txt>
        <div className="scroll" style={{ position: 'absolute', left: 8, top: 30, width: 364, height: 150 }}>
          {messages.length === 0 && (
            <Txt x={4} y={4} w={356} size={10} color="var(--text-mute)">No messages yet.</Txt>
          )}
          {messages.map((m) => (
            <div key={m.id} style={{ marginBottom: 6 }}>
              <div style={{ fontSize: 10, color: m.mine ? 'var(--accent)' : 'var(--ok)' }}>
                {m.from} → {m.to}
              </div>
              <div style={{ fontSize: 11, lineHeight: 1.4 }}>{m.body}</div>
            </div>
          ))}
        </div>
        <select className="inp" aria-label="Recipient" style={{ left: 8, top: 188, width: 110, height: 26 }}
          value={toPlayer} onChange={(e) => setToPlayer(e.target.value)}>
          {others.map((p) => <option key={p.playerId} value={p.playerId}>{p.name}</option>)}
        </select>
        <input className="inp" style={{ left: 124, top: 188, width: 180, height: 26 }}
          placeholder="message…" value={draft} maxLength={2000}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') send(); }} />
        <Btn x={310} y={188} w={62} h={26} label="Send" size={10} onClick={send} />
      </Panel>

      {/* trades */}
      <Panel x={404} y={52} w={380} h={250}>
        <Txt x={12} y={8} size={12} bold className="heading">TRADES</Txt>
        <div className="scroll" style={{ position: 'absolute', left: 8, top: 30, width: 364, height: 120 }}>
          {trades.length === 0 && (
            <Txt x={4} y={4} w={356} size={10} color="var(--text-mute)">No trades yet.</Txt>
          )}
          {trades.map((t) => (
            <div key={t.id} style={{ marginBottom: 6, fontSize: 10 }}>
              <span style={{ color: 'var(--accent)' }}>{t.from}</span> gives {t.giveAmount} {t.give} for {t.wantAmount} {t.want}
              <span className="mute"> · {t.status}</span>
              {t.incoming && t.status === 'pending' && (
                <>
                  <span className="link" style={{ marginLeft: 6 }} onClick={() => respond(t.id, true)}>accept</span>
                  <span className="link" style={{ marginLeft: 6 }} onClick={() => respond(t.id, false)}>decline</span>
                </>
              )}
            </div>
          ))}
        </div>
        <select className="inp" aria-label="Give resource" style={{ left: 8, top: 156, width: 70, height: 24 }}
          value={give} onChange={(e) => setGive(e.target.value)}>
          {RES.map((r) => <option key={r} value={r}>{r[0].toUpperCase() + r.slice(1)}</option>)}
        </select>
        <input className="inp" type="number" aria-label="Give amount" style={{ left: 82, top: 156, width: 44, height: 24 }}
          value={giveAmt} min={1} onChange={(e) => setGiveAmt(Number(e.target.value))} />
        <Txt x={130} y={160} size={10} color="var(--text-dim)">for</Txt>
        <select className="inp" aria-label="Want resource" style={{ left: 156, top: 156, width: 70, height: 24 }}
          value={want} onChange={(e) => setWant(e.target.value)}>
          {RES.map((r) => <option key={r} value={r}>{r[0].toUpperCase() + r.slice(1)}</option>)}
        </select>
        <input className="inp" type="number" aria-label="Want amount" style={{ left: 230, top: 156, width: 44, height: 24 }}
          value={wantAmt} min={1} onChange={(e) => setWantAmt(Number(e.target.value))} />
        <Btn x={282} y={156} w={90} h={24} label="Offer" size={10} onClick={offer} />
        <Txt x={8} y={190} w={364} size={9} color="var(--text-mute)">
          Trades are voluntary. No one can take your resources.
        </Txt>
      </Panel>

      {/* players */}
      <Panel x={16} y={314} w={768} h={270}>
        <Txt x={12} y={8} size={12} bold className="heading">PLAYERS</Txt>
        {game.players.map((p, i) => (
          <Panel key={p.playerId} x={12 + i * 250} y={32} w={236} h={220}>
            <div style={{ position: 'absolute', left: 0, top: 0, width: 236, height: 4, background: FACTION_COLOR[p.faction] ?? '#888' }} />
            <div className="well" style={{ position: 'absolute', left: 10, top: 12, width: 52, height: 52 }}>
              <div style={{ position: 'absolute', left: 2, top: 2 }}><HeroPortrait faction={p.faction} size={48} /></div>
            </div>
            <Txt x={72} y={12} w={156} size={13} bold>{p.name}</Txt>
            <Txt x={72} y={30} w={156} size={10} color={FACTION_COLOR[p.faction] ? 'var(--text)' : 'var(--text-dim)'}>{FACTION_LABEL[p.faction] ?? p.faction}</Txt>
            <Txt x={72} y={46} w={156} size={10} color="var(--text-dim)">{p.cityName}</Txt>
            <Txt x={12} y={74} size={10} color="var(--text-dim)">
              Obelisk: {p.obeliskVisits}/10 fragments
            </Txt>
            {p.precious && <Txt x={12} y={92} size={10} color="var(--accent)">Holds the Grail</Txt>}
            {RES.map((r, j) => (
              <div key={r} title={r} style={{ position: 'absolute', left: 12 + (j % 2) * 110, top: 116 + Math.floor(j / 2) * 24,
                display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, fontWeight: 'bold' }}>
                <ResIcon r={r} size={18} />{p.resources[r]}
              </div>
            ))}
          </Panel>
        ))}
      </Panel>
    </>
  );
}