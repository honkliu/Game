import { useEffect, useState, useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { api, ApiError } from '../api';
import type { GameSummary, SeededPlayer } from '../types';
import { Btn, Loading, Modal, Panel, Txt } from '../ui/Layout';
import { toast } from '../ui/Toast';

export function Lobby() {
  const navigate = useNavigate();
  const [games, setGames] = useState<GameSummary[]>([]);
  const [player, setPlayer] = useState<SeededPlayer | null>(null);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [list, players] = await Promise.all([api.listGames(), api.players()]);
      setGames(list);
      setPlayer(players.find(p => p.id === localStorage.getItem('cityoftowers.player')) ?? null);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Could not load games.');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(refresh, 5000);
    return () => window.clearInterval(id);
  }, [refresh]);

  if (loading) return <Loading />;

  return (
    <>
      <Panel x={0} y={0} w={800} h={48}>
        <Txt x={20} y={14} size={18} bold className="heading">CITY OF TOWERS</Txt>
        <Txt x={420} y={16} size={13}>Playing as: {player?.name}</Txt>
        <Btn x={650} y={8} w={130} h={32} label="Change player" size={12} onClick={() => {
          localStorage.removeItem('cityoftowers.player');
          window.dispatchEvent(new Event('cityoftowers:player'));
          navigate('/');
        }} />
      </Panel>
      <Txt x={24} y={78} size={20} bold className="heading">GAMES</Txt>
      <Btn x={630} y={66} w={146} h={36} label="New Game" onClick={() => setCreating(true)} />
      <Panel x={24} y={118} w={752} h={458}>
        <div className="scroll" style={{ position: 'absolute', inset: 6 }}>
          {games.length === 0 ? (
            <div className="muted" style={{ padding: 16 }}>No games yet. Create a world to explore together.</div>
          ) : (
            <table className="tbl">
              <thead><tr><th>Name</th><th>Players</th><th>Day</th><th>Status</th><th /></tr></thead>
              <tbody>
                {games.map(game => (
                  <tr key={game.id}>
                    <td>{game.name}</td>
                    <td>{game.players}</td>
                    <td>D{game.day} W{game.week} M{game.month}</td>
                    <td>{game.state === 'InProgress' ? 'In progress' : game.state}</td>
                    <td style={{ width: 100 }}>
                      <button className="btn" style={{ position: 'relative', width: 84, height: 30 }}
                        onClick={async () => {
                          // joining is idempotent: the server keeps the existing seat if there is one
                          try { await api.joinGame(game.id); } catch { /* already seated */ }
                          navigate(`/game/${game.id}`);
                        }}>Enter</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </Panel>
      {creating && <CreateGameModal onClose={() => setCreating(false)}
        onCreated={id => navigate(`/game/${id}`)} />}
    </>
  );
}

function CreateGameModal({ onClose, onCreated }: {
  onClose: () => void; onCreated: (id: string) => void;
}) {
  const [name, setName] = useState('The Three Cities');
  const [turnSeconds, setTurnSeconds] = useState(300);
  const [turnLimit, setTurnLimit] = useState(0);
  const [aiPlayers, setAiPlayers] = useState(0);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    try {
      const result = await api.createGame(name.trim(), 3, turnSeconds, turnLimit, aiPlayers);
      onCreated(result.gameId);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Could not create the game.');
      setBusy(false);
    }
  }

  return (
    <Modal w={480} h={360} onClose={onClose}>
      <Txt x={0} y={20} w={480} size={20} bold align="center" className="heading">NEW GAME</Txt>
      <Txt x={32} y={70}>Name</Txt>
      <input className="inp" aria-label="Name" style={{ left: 32, top: 94, width: 416, height: 36 }}
        value={name} maxLength={60} onChange={e => setName(e.target.value)} />
      <Txt x={32} y={150}>Turn length</Txt>
      <select className="inp" aria-label="Turn length" style={{ left: 32, top: 174, width: 194, height: 36 }}
        value={turnSeconds} onChange={e => setTurnSeconds(Number(e.target.value))}>
        <option value={30}>30 seconds</option><option value={60}>1 minute</option>
        <option value={300}>5 minutes</option><option value={86400}>1 day</option>
      </select>
      <Txt x={254} y={150}>Turn limit (0 = unlimited)</Txt>
      <input className="inp" type="number" aria-label="Turn limit" min={0} max={500}
        style={{ left: 254, top: 174, width: 194, height: 36 }}
        value={turnLimit} onChange={e => setTurnLimit(Number(e.target.value))} />
      <Txt x={32} y={242}>Computer opponents</Txt>
      <select className="inp" aria-label="Computer opponents" style={{ left: 32, top: 266, width: 194, height: 36 }}
        value={aiPlayers} onChange={e => setAiPlayers(Number(e.target.value))}>
        <option value={0}>None (human players)</option>
        <option value={1}>1 computer</option>
        <option value={2}>2 computers</option>
      </select>
      <Txt x={254} y={250} w={194} size={11} color="var(--text-dim)">
        Computers build, recruit and explore each day.
      </Txt>
      <Btn x={176} y={330} w={128} h={38} label={busy ? 'Creating...' : 'Create'}
        disabled={busy || !name.trim()} onClick={submit} />
      <Btn x={320} y={330} w={128} h={38} label="Cancel" onClick={onClose} />
    </Modal>
  );
}
