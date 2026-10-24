import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { api } from '../api';
import type { SeededPlayer } from '../types';
import { Box, Btn, Loading, Panel, Txt } from '../ui/Layout';

const FACTION_LABEL: Record<string, string> = {
  castle: 'Castle', rampart: 'Rampart', necropolis: 'Necropolis'
};

const FACTION_COLOR: Record<string, string> = {
  castle: '#3a6ad0', rampart: '#2a8a4a', necropolis: '#5a4a6a'
};

export function ChoosePlayer() {
  const navigate = useNavigate();
  const [players, setPlayers] = useState<SeededPlayer[] | null>(null);

  useEffect(() => {
    api.players().then(setPlayers).catch(() => setPlayers([]));
  }, []);

  function play(id: string) {
    localStorage.setItem('cityoftowers.player', id);
    // notify the shell so it re-renders with the chosen player
    window.dispatchEvent(new Event('cityoftowers:player'));
    navigate('/lobby');
  }

  if (!players) return <Loading />;

  return (
    <>
      <Txt x={0} y={70} w={800} size={34} bold align="center" className="heading">
        CITY OF TOWERS
      </Txt>
      <Box x={300} y={116} w={200} h={1} style={{ background: 'var(--gold-1)' }} />
      <Txt x={0} y={132} w={800} size={12} align="center" color="var(--text-dim)">
        Choose your city
      </Txt>

      {players.map((p, i) => {
        const x = 100 + i * 210;
        const color = FACTION_COLOR[p.faction] ?? '#3a6ad0';
        return (
          <Panel key={p.id} x={x} y={180} w={190} h={250}>
            <Box x={0} y={0} w={190} h={6} style={{ background: color, borderRadius: '3px 3px 0 0' }} />
            <Box x={35} y={16} w={120} h={120}>
              <svg width={120} height={120} viewBox="0 0 160 160">
                <defs>
                  <linearGradient id={'g' + p.id} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor={color} stopOpacity="0.9" />
                    <stop offset="100%" stopColor="#1a130c" />
                  </linearGradient>
                </defs>
                <ellipse cx={80} cy={146} rx={56} ry={9} fill="rgba(0,0,0,0.5)" />
                <rect x={26} y={86} width={108} height={58} fill="#8a8070" stroke="#2a1a10" strokeWidth={1.5} />
                <path d="M26 86 L26 80 L36 80 L36 86 L46 86 L46 80 L56 80 L56 86 L66 86 L66 80 L76 80 L76 86 L86 86 L86 80 L96 80 L96 86 L106 86 L106 80 L116 80 L116 86 L126 86 L126 80 L134 80 L134 86 Z" fill="#9a9080" stroke="#2a1a10" strokeWidth={1} />
                <rect x={66} y={34} width={28} height={54} fill="#b0a694" stroke="#2a1a10" strokeWidth={1.5} />
                <path d="M60 34 L80 12 L100 34 Z" fill={'url(#g' + p.id + ')'} stroke="#2a1a10" strokeWidth={1.5} />
                <rect x={72} y={112} width={16} height={32} fill="#241a10" stroke="#2a1a10" strokeWidth={1} />
                <path d="M72 112 A8 8 0 0 1 88 112 Z" fill="#241a10" />
                <rect x={36} y={100} width={12} height={16} fill="#1a1410" />
                <rect x={112} y={100} width={12} height={16} fill="#1a1410" />
                <rect x={78} y={20} width={4} height={14} fill={color} />
              </svg>
            </Box>
            <Txt x={0} y={150} w={190} size={16} bold align="center">{p.name}</Txt>
            <Txt x={0} y={172} w={190} size={11} align="center" color="var(--text-dim)">
              {FACTION_LABEL[p.faction] ?? p.faction}
            </Txt>
            <Btn x={30} y={196} w={130} h={34} label="Play" size={13} onClick={() => play(p.id)} />
          </Panel>
        );
      })}
    </>
  );
}