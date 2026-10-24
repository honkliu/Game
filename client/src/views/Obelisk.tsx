import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { api, ApiError } from '../api';
import type { ObeliskState } from '../types';
import { t } from '../i18n';
import { Box, Btn, Loading, Panel, Txt } from '../ui/Layout';
import { toast } from '../ui/Toast';

export function Obelisk() {
  const { gameId } = useParams<{ gameId: string }>();
  const navigate = useNavigate();
  const [state, setState] = useState<ObeliskState | null>(null);
  const [answer, setAnswer] = useState('');
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [cooldown, setCooldown] = useState(0);

  const refresh = useCallback(async () => {
    if (!gameId) return;
    try {
      setState(await api.getObelisk(gameId));
    } catch (e) {
      if (e instanceof ApiError) toast(e.message);
    }
  }, [gameId]);

  useEffect(() => { refresh(); }, [refresh]);

  useEffect(() => {
    if (cooldown <= 0) return;
    const id = window.setInterval(() => setCooldown((c) => Math.max(0, c - 1)), 1000);
    return () => window.clearInterval(id);
  }, [cooldown]);

  if (!state) return <Loading />;

  const allSeen = state.visits >= state.total;
  const locked = state.wrongAnswers >= 10;

  async function visit() {
    if (!gameId) return;
    setBusy(true);
    try {
      const r = await api.visitObelisk(gameId);
      toast(t('obelisk.revealed', { n: r.visits, total: state!.total }));
      await refresh();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    } finally {
      setBusy(false);
    }
  }

  async function solve() {
    if (!gameId || !answer.trim()) return;
    setBusy(true);
    try {
      const r = await api.solveObelisk(gameId, answer.trim());
      if (r.correct) {
        toast('Solved! The Grail site is marked on your map. Send a hero to dig.');
        setAnswer('');
        await refresh();
      } else {
        toast(t('obelisk.wrong'));
        setCooldown(60);
        await refresh();
      }
    } catch (e) {
      if (e instanceof ApiError) {
        toast(e.message);
        if (/Locked/.test(e.message)) await refresh();
        else setCooldown(60);
      }
    } finally {
      setBusy(false);
    }
  }

  async function postMessage() {
    if (!gameId || !draft.trim()) return;
    try {
      await api.addObeliskMessage(gameId, draft.trim());
      setDraft('');
      await refresh();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : 'Something went wrong.');
    }
  }

  return (
    <>
      <Panel x={0} y={0} w={800} h={40} style={{ borderRadius: 0, borderLeft: 'none', borderRight: 'none', borderTop: 'none' }}>
        <Txt x={16} y={12} size={15} bold className="heading">{t('obelisk.title')}</Txt>
        <Btn x={700} y={6} w={90} h={28} label="Back" size={11} onClick={() => navigate(`/game/${gameId}`)} />
      </Panel>

      {/* obelisk */}
      <Panel x={8} y={48} w={240} h={244}>
        <Box x={40} y={8} w={160} h={160}>
          <img src="/assets/obelisk.png" alt="" width={160} height={160}
            onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = 'none'; }} />
          <svg width={160} height={160} viewBox="0 0 160 160" style={{ position: 'absolute', left: 0, top: 0 }}>
            <defs>
              <linearGradient id="ob" x1="0" y1="0" x2="1" y2="0">
                <stop offset="0%" stopColor="#8a8a96" />
                <stop offset="50%" stopColor="#c8c8d4" />
                <stop offset="100%" stopColor="#5a5a66" />
              </linearGradient>
            </defs>
            <path d="M60 150 L72 20 L88 20 L100 150 Z" fill="url(#ob)" stroke="#2a2a32" strokeWidth="1.5" />
            <path d="M72 20 L80 8 L88 20 Z" fill="#e8e8f0" stroke="#2a2a32" strokeWidth="1.5" />
            <rect x="76" y="50" width="8" height="60" fill="#60c0ff" opacity="0.75" />
            <ellipse cx="80" cy="152" rx="46" ry="8" fill="rgba(0,0,0,0.5)" />
          </svg>
        </Box>
        <Txt x={0} y={174} w={240} size={12} align="center">
          {t('obelisk.seen', { n: state.visits, total: state.total })}
        </Txt>
        <Btn x={30} y={200} w={180} h={32}
          label={allSeen ? t('obelisk.allSeen') : t('obelisk.visit')}
          disabled={allSeen || busy} onClick={visit} />
      </Panel>

      {/* fragments */}
      <Panel x={256} y={48} w={536} h={244}>
        <Txt x={14} y={8} size={13} bold className="heading">{t('obelisk.fragments')}</Txt>
        <Box x={14} y={30} w={508} h={204} className="scroll">
          {state.visits === 0 ? (
            <div style={{ fontSize: 12, color: 'var(--text-mute)' }}>{t('obelisk.notVisited')}</div>
          ) : Array.from({ length: state.total }).map((_, i) => {
            const revealed = i < state.fragments.length;
            return (
              <div key={i} style={{ fontSize: 12, lineHeight: '18px', color: revealed ? 'var(--text)' : 'var(--text-mute)' }}>
                {i + 1}. {revealed ? state.fragments[i] : '░'.repeat(40)}
              </div>
            );
          })}
        </Box>
      </Panel>

      {/* solve */}
      <Panel x={8} y={300} w={784} h={96}>
        <Txt x={14} y={8} size={13} bold className="heading">{t('obelisk.solve')}</Txt>
        <input className="inp" style={{ left: 14, top: 32, width: 520, height: 32 }}
          value={answer} maxLength={200} disabled={locked || cooldown > 0}
          onChange={(e) => setAnswer(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') solve(); }} />
        <Btn x={546} y={32} w={120} h={32}
          label={locked ? t('obelisk.locked') : cooldown > 0 ? t('obelisk.wait', { s: cooldown }) : t('obelisk.submit')}
          disabled={locked || cooldown > 0 || busy} onClick={solve} />
        <Txt x={14} y={72} w={760} size={11} color="var(--text-dim)">
          Solving reveals where the Grail lies. A hero must still walk there and dig.
        </Txt>
      </Panel>

      {/* messages */}
      <Panel x={8} y={404} w={784} h={188}>
        <Txt x={14} y={8} size={13} bold className="heading">{t('obelisk.messages')}</Txt>
        <Box x={14} y={30} w={756} h={108} className="scroll">
          {state.messages.length === 0 && (
            <Txt x={0} y={4} w={756} size={12} color="var(--text-mute)">{t('obelisk.noMessages')}</Txt>
          )}
          {state.messages.map((m) => (
            <div key={m.id} style={{ fontSize: 12, marginBottom: 4 }}>
              <span style={{ color: 'var(--accent)' }}>{m.author}: </span>{m.body}
            </div>
          ))}
        </Box>
        <input className="inp" style={{ left: 14, top: 146, width: 640, height: 32 }}
          placeholder={t('obelisk.leaveMessage')} value={draft} maxLength={2000}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') postMessage(); }} />
        <Btn x={666} y={146} w={104} h={32} label={t('obelisk.post')} size={12} onClick={postMessage} />
      </Panel>
    </>
  );
}