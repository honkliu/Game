import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';

/**
 * In-game navigation. Panels are views of one game, not pages: opening the town or
 * the hero screen must not add a browser history entry, or Back would rewind the game.
 * Every in-game transition replaces the current entry instead of pushing a new one.
 */
export function useGameNavigate() {
  const navigate = useNavigate();
  return useCallback(
    (to: string) => navigate(to, { replace: true }),
    [navigate]
  );
}