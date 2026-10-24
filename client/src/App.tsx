import { useEffect, useState, lazy, Suspense } from 'react';
import { HashRouter, Routes, Route, Navigate } from 'react-router-dom';
import { ChoosePlayer } from './views/ChoosePlayer';
import { Toast } from './ui/Toast';
import { Loading } from './ui/Layout';

const Lobby = lazy(() => import('./views/Lobby').then(m => ({ default: m.Lobby })));
const AdventureMap = lazy(() => import('./views/AdventureMap').then(m => ({ default: m.AdventureMap })));
const TownScreen = lazy(() => import('./views/TownScreen').then(m => ({ default: m.TownScreen })));
const HeroScreen = lazy(() => import('./views/HeroScreen').then(m => ({ default: m.HeroScreen })));
const Social = lazy(() => import('./views/Social').then(m => ({ default: m.Social })));
const Projects = lazy(() => import('./views/Projects').then(m => ({ default: m.Projects })));
const Obelisk = lazy(() => import('./views/Obelisk').then(m => ({ default: m.Obelisk })));
const GameOver = lazy(() => import('./views/GameOver').then(m => ({ default: m.GameOver })));

function Stage({ children }: { children: React.ReactNode }) {
  const [scale, setScale] = useState(1);
  useEffect(() => {
    const fit = () => setScale(Math.min(1, window.innerWidth / 800, window.innerHeight / 600));
    fit();
    window.addEventListener('resize', fit);
    return () => window.removeEventListener('resize', fit);
  }, []);
  return (
    <div className="stage" style={{ transform: `translate(-50%, -50%) scale(${scale})` }}>
      {children}
    </div>
  );
}

function Shell() {
  const [playerId, setPlayerId] = useState<string | null>(() => localStorage.getItem('cityoftowers.player'));

  // Re-read the chosen player whenever it changes (same tab or another tab).
  useEffect(() => {
    const sync = () => setPlayerId(localStorage.getItem('cityoftowers.player'));
    window.addEventListener('storage', sync);
    window.addEventListener('cityoftowers:player', sync);
    return () => {
      window.removeEventListener('storage', sync);
      window.removeEventListener('cityoftowers:player', sync);
    };
  }, []);

  const selected = playerId === 'alice' || playerId === 'bob' || playerId === 'carol';

  return (
    <Stage>
      <Suspense fallback={<Loading />}>
        <Routes>
          {selected ? (
            <>
              <Route path="/lobby" element={<Lobby />} />
              <Route path="/game/:gameId" element={<AdventureMap />} />
              <Route path="/game/:gameId/town/:townId" element={<TownScreen />} />
              <Route path="/game/:gameId/hero/:heroId" element={<HeroScreen />} />
              <Route path="/game/:gameId/social" element={<Social />} />
              <Route path="/game/:gameId/projects" element={<Projects />} />
              <Route path="/game/:gameId/obelisk" element={<Obelisk />} />
              <Route path="/game/:gameId/over" element={<GameOver />} />
              <Route path="*" element={<Navigate to="/lobby" replace />} />
            </>
          ) : (
            <Route path="*" element={<ChoosePlayer />} />
          )}
        </Routes>
      </Suspense>
      <Toast />
    </Stage>
  );
}

export function App() {
  return (
    <HashRouter>
      <Shell />
    </HashRouter>
  );
}