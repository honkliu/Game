// Typed REST client. All commands go through here; the server is authoritative.

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      headers: {
        'X-Player-Id': localStorage.getItem('cityoftowers.player') ?? '',
        ...(body === undefined ? {} : { 'Content-Type': 'application/json' })
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: 'same-origin'
    });
  } catch {
    throw new ApiError(0, 'Cannot reach the server. Please try again.');
  }

  if (res.status === 204) return undefined as T;

  const text = await res.text();
  let json: unknown = null;
  if (text) {
    try { json = JSON.parse(text); } catch { json = text; }
  }

  if (!res.ok) {
    const msg = (json && typeof json === 'object' && 'error' in json)
      ? String((json as { error: unknown }).error)
      : 'Request failed (' + res.status + ').';
    throw new ApiError(res.status, msg);
  }
  return json as T;
}

export const api = {
  players: () => request<import('./types').SeededPlayer[]>('GET', '/api/players'),
  // auth
  register: (username: string, password: string) =>
    request<{ userId: string; username: string }>('POST', '/api/auth/register', { username, password }),
  login: (username: string, password: string) =>
    request<{ userId: string; username: string }>('POST', '/api/auth/login', { username, password }),
  logout: () => request<void>('POST', '/api/auth/logout'),
  me: () => request<{ userId: string; username: string }>('GET', '/api/auth/me'),

  // games
  listGames: () => request<import('./types').GameSummary[]>('GET', '/api/games'),
  createGame: (name: string, capacity: number, turnSeconds: number, turnLimit: number) =>
    request<{ gameId: string }>('POST', '/api/games', { name, capacity, turnSeconds, turnLimit }),
  joinGame: (id: string) => request<{ playerId: string }>('POST', `/api/games/${id}/join`),
  leaveGame: (id: string) => request<void>('POST', `/api/games/${id}/leave`),
  startGame: (id: string) => request<void>('POST', `/api/games/${id}/start`),
  cancelGame: (id: string) => request<void>('POST', `/api/games/${id}/cancel`),
  getGame: (id: string) => request<import('./types').GameDetail>('GET', `/api/games/${id}`),
  getMap: (id: string) => request<import('./types').MapData>('GET', `/api/games/${id}/map`),
  getHeroes: (id: string) => request<import('./types').HeroDetail[]>('GET', `/api/games/${id}/heroes`),
  getHero: (id: string) => request<import('./types').HeroDetail>('GET', `/api/heroes/${id}`),
  moveHero: (id: string, path: { x: number; y: number }[]) =>
    request<import('./types').HeroDetail>('POST', `/api/heroes/${id}/move`, { path }),
  resolveEncounter: (id: string, action: string) =>
    request<{ message: string }>('POST', `/api/heroes/${id}/encounter`, { action }),
  getTown: (id: string) => request<import('./types').TownDetail>('GET', `/api/towns/${id}`),
  recruit: (id: string, tier: number, count: number) =>
    request<{ recruited: number; creature: string }>('POST', `/api/towns/${id}/recruit`, { tier, count }),
  trade: (id: string, from: string, to: string, amount: number) =>
    request<void>('POST', `/api/towns/${id}/trade`, { from, to, amount }),
  hire: (id: string, heroKey: string) =>
    request<{ heroId: string }>('POST', `/api/towns/${id}/hire`, { heroKey }),

  // city
  getCity: (playerId: string) => request<import('./types').CityView>('GET', `/api/cities/${playerId}`),
  build: (playerId: string, buildingKey: string) =>
    request<void>('POST', `/api/towns/${playerId}/build`, { buildingKey }),
  renameCity: (playerId: string, name: string) =>
    request<void>('PUT', `/api/cities/${playerId}/name`, { name }),

  // posts
  getPosts: (playerId: string) => request<import('./types').Post[]>('GET', `/api/towns/${playerId}/posts`),
  createPost: (playerId: string, title: string, body: string, imageUrl: string | null) =>
    request<{ postId: string }>('POST', `/api/towns/${playerId}/posts`, { title, body, imageUrl }),
  updatePost: (postId: string, title: string, body: string, imageUrl: string | null) =>
    request<void>('PUT', `/api/posts/${postId}`, { title, body, imageUrl }),
  deletePost: (postId: string) => request<void>('DELETE', `/api/posts/${postId}`),

  // comments
  getComments: (postId: string) => request<import('./types').Comment[]>('GET', `/api/posts/${postId}/comments`),
  addComment: (postId: string, body: string) =>
    request<{ commentId: string }>('POST', `/api/posts/${postId}/comments`, { body }),

  // wall
  getWall: (playerId: string) => request<import('./types').WallMessage[]>('GET', `/api/towns/${playerId}/wall`),
  addWallMessage: (playerId: string, body: string, parentId: string | null) =>
    request<{ messageId: string }>('POST', `/api/towns/${playerId}/wall`, { body, parentId }),
  deleteWallMessage: (messageId: string) => request<void>('DELETE', `/api/wall/${messageId}`),

  // tower guestbook
  getTowerMessages: (playerId: string) => request<import('./types').TowerMessage[]>('GET', `/api/towns/${playerId}/tower`),
  addTowerMessage: (playerId: string, body: string) =>
    request<{ messageId: string }>('POST', `/api/towns/${playerId}/tower`, { body }),

  // obelisk
  getObelisk: (gameId: string) => request<import('./types').ObeliskState>('GET', `/api/games/${gameId}/obelisk`),
  visitObelisk: (gameId: string) =>
    request<{ visits: number; fragments: string[] }>('POST', `/api/games/${gameId}/obelisk/visit`),
  addObeliskMessage: (gameId: string, body: string) =>
    request<{ messageId: string }>('POST', `/api/games/${gameId}/obelisk/message`, { body }),
  solveObelisk: (gameId: string, answer: string) =>
    request<{ correct: boolean }>('POST', `/api/games/${gameId}/obelisk/solve`, { body: answer }),

  // catalogue
  getBuildings: () => request<import('./types').BuildingDef[]>('GET', '/api/buildings'),
  getCreatures: () => request<import('./types').CreatureDef[]>('GET', '/api/creatures'),
  getMessages: (id: string) => request<HeroMessageDto[]>('GET', `/api/games/${id}/messages`),
  sendMessage: (id: string, toPlayerId: string, body: string) =>
    request<void>('POST', `/api/games/${id}/messages`, { toPlayerId, body }),
  getTrades: (id: string) => request<TradeDto[]>('GET', `/api/games/${id}/trades`),
  offerTrade: (id: string, toPlayerId: string, give: string, giveAmount: number, want: string, wantAmount: number) =>
    request<void>('POST', `/api/games/${id}/trades`, { toPlayerId, give, giveAmount, want, wantAmount }),
  respondTrade: (id: string, accept: boolean) => request<void>('POST', `/api/trades/${id}/respond`, { accept }),
  getProjects: (id: string) => request<ProjectDto[]>('GET', `/api/games/${id}/projects`),
  contribute: (id: string, gold: number, wood: number, ore: number) =>
    request<void>('POST', `/api/projects/${id}/contribute`, { gold, wood, ore })
};

export interface HeroMessageDto {
  id: string; from: string; to: string; body: string; createdAt: string; mine: boolean;
}

export interface TradeDto {
  id: string; from: string; to: string; give: string; giveAmount: number;
  want: string; wantAmount: number; status: string; incoming: boolean; createdAt: string;
}

export interface ProjectDto {
  id: string; name: string; description: string;
  requiredGold: number; requiredWood: number; requiredOre: number;
  contributedGold: number; contributedWood: number; contributedOre: number;
  completed: boolean; contributors: string[];
}