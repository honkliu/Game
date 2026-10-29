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
  createGame: (name: string, capacity: number, turnSeconds: number, turnLimit: number, aiPlayers = 0) =>
    request<{ gameId: string }>('POST', '/api/games', { name, capacity, turnSeconds, turnLimit, aiPlayers }),
  joinGame: (id: string) => request<{ playerId: string }>('POST', `/api/games/${id}/join`),
  leaveGame: (id: string) => request<void>('POST', `/api/games/${id}/leave`),
  startGame: (id: string) => request<void>('POST', `/api/games/${id}/start`),
  cancelGame: (id: string) => request<void>('POST', `/api/games/${id}/cancel`),
  getGame: (id: string) => request<import('./types').GameDetail>('GET', `/api/games/${id}`),
  getMap: (id: string) => request<import('./types').MapData>('GET', `/api/games/${id}/map`),
  getHeroes: (id: string) => request<import('./types').HeroDetail[]>('GET', `/api/games/${id}/heroes`),
  getHero: (id: string) => request<import('./types').HeroDetail>('GET', `/api/heroes/${id}`),
  moveHero: async (id: string, path: { x: number; y: number }[]) => {
    const res = await fetch(`/api/heroes/${id}/move`, {
      method: 'POST',
      headers: {
        'X-Player-Id': localStorage.getItem('cityoftowers.player') ?? '',
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({ path }),
      credentials: 'same-origin'
    });
    const text = await res.text();
    let json: unknown = null;
    if (text) { try { json = JSON.parse(text); } catch { json = text; } }
    if (!res.ok) {
      const msg = json && typeof json === 'object' && 'error' in json ? String((json as { error: unknown }).error) : `Request failed (${res.status}).`;
      throw new ApiError(res.status, msg);
    }
    const notices = JSON.parse(res.headers.get('X-Move-Events') ?? '[]') as import('./types').AdventureNotice[];
    return { hero: json as import('./types').HeroDetail, message: res.headers.get('X-Move-Message'), notices };
  },
  resolveEncounter: (id: string, action: string) =>
    request<{ message: string }>('POST', `/api/heroes/${id}/encounter`, { action }),
  resolveTreasure: (id: string, choice: 'gold' | 'experience' | 'leave') =>
    request<{ message: string; notices: import('./types').AdventureNotice[] }>('POST', `/api/heroes/${id}/treasure`, { choice }),
  castSpell: (id: string, spell: string, x?: number, y?: number, townId?: string) =>
    request<{ message: string; mana: number; x: number; y: number; view: import('./types').WorldView | null }>('POST', `/api/heroes/${id}/cast`, { spell, x, y, townId }),
  getVisions: (id: string) => request<import('./types').VisionTarget[]>('GET', `/api/heroes/${id}/visions`),
  endTurn: (id: string) =>
    request<{ day: number; week: number; month: number; turn: number }>('POST', `/api/games/${id}/endturn`),
  shareMine: (id: string, x: number, y: number, shared: boolean) =>
    request<void>('POST', `/api/games/${id}/mine-sharing`, { x, y, shared }),
  endAgreement: (id: string, agree: boolean) =>
    request<{ state: string; votes: string[] }>('POST', `/api/games/${id}/agreement`, { agree }),
  getTown: (id: string) => request<import('./types').TownDetail>('GET', `/api/towns/${id}`),
  recruit: (id: string, tier: number, count: number, creature?: string) =>
    request<{ recruited: number; creature: string }>('POST', `/api/towns/${id}/recruit`, { tier, count, creature }),
  moveArmy: (id: string, fromArmy: 'hero' | 'garrison', fromSlot: number, toArmy: 'hero' | 'garrison', toSlot: number, count = 0) =>
    request<{ hero: import('./types').ArmySlot[] | null; garrison: import('./types').ArmySlot[] }>(
      'POST', `/api/towns/${id}/army`, { fromArmy, fromSlot, toArmy, toSlot, count }),
  moveHeroArmy: (heroId: string, fromSlot: number, toSlot: number, count = 0) =>
    request<{ army: import('./types').ArmySlot[] }>('POST', `/api/heroes/${heroId}/army`, { fromSlot, toSlot, count }),
  exchangeHeroes: (heroId: string, otherHeroId: string, fromSlot?: number, toSlot?: number, count = 0) =>
    request<{ hero: import('./types').HeroDetail; other: import('./types').HeroDetail }>(
      'POST', `/api/heroes/${heroId}/exchange`, { otherHeroId, fromSlot, toSlot, count }),
  moveArtifact: (heroId: string, from: string, to: string, toHeroId?: string) =>
    request<{ hero: import('./types').HeroDetail; other: import('./types').HeroDetail | null }>(
      'POST', `/api/heroes/${heroId}/artifacts`, { from, to, toHeroId }),
  dismissStack: (heroId: string, slot: number) =>
    request<import('./types').HeroDetail>('POST', `/api/heroes/${heroId}/dismiss-stack`, { slot, confirmed: true }),
  dismissHero: (heroId: string) =>
    request<void>('POST', `/api/heroes/${heroId}/dismiss`, { confirmed: true }),
  chooseLevel: (heroId: string, skill: string | null) =>
    request<import('./types').HeroDetail>('POST', `/api/heroes/${heroId}/level-up`, { skill }),
  upgradeStack: (id: string, army: 'hero' | 'garrison', slot: number) =>
    request<{ creature: string }>('POST', `/api/towns/${id}/upgrade`, { army, slot }),
  buyWarMachine: (id: string) => request<{ machine: string }>('POST', `/api/towns/${id}/blacksmith`),
  buySpellbook: (id: string) => request<{ learned: string[] }>('POST', `/api/towns/${id}/spellbook`),
  transformStack: (id: string, slot: number) =>
    request<{ creature: string; count: number }>('POST', `/api/towns/${id}/transform`, { army: 'hero', slot }),
  trade: (id: string, from: string, to: string, amount: number) =>
    request<void>('POST', `/api/towns/${id}/trade`, { from, to, amount }),
  hire: (id: string, heroKey: string) =>
    request<{ heroId: string }>('POST', `/api/towns/${id}/hire`, { heroKey }),
  moveGarrison: (id: string, direction: 'toGarrison' | 'toHero', slot: number, count: number) =>
    request<{ moved: number; creature: string }>('POST', `/api/towns/${id}/garrison`, { direction, slot, count }),

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
  getBuildings: (faction: string) => request<import('./types').BuildingDef[]>('GET', `/api/buildings?faction=${encodeURIComponent(faction)}`),
  getCreatures: () => request<import('./types').CreatureDef[]>('GET', '/api/creatures'),
  getHeroCatalog: () => request<import('./types').HeroDef[]>('GET', '/api/heroes'),
  getSpells: () => request<import('./types').SpellDef[]>('GET', '/api/spells'),
  getAnnotations: (id: string) =>
    request<{ id: string; x: number; y: number; text: string; author: string; createdAt: string }[]>('GET', `/api/games/${id}/annotations`),
  addAnnotation: (id: string, x: number, y: number, text: string) =>
    request<void>('POST', `/api/games/${id}/annotations`, { x, y, text }),
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