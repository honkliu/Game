// Shared types mirroring the server's JSON shapes.

export interface Me { userId: string; username: string; }

export interface LegacyGameSummary {
  id: string;
  name: string;
  host: string;
  capacity: number;
  joined: number;
  turnSeconds: number;
  state: 'Waiting' | 'InProgress' | 'Finished';
  joinedByMe: boolean;
}

export interface LegacyGamePlayer {
  playerId: string;
  userId: string;
  username: string;
  cityName: string;
  isHost: boolean;
  precious: boolean;
  obeliskVisits: number;
}

export interface LegacyGameDetail {
  id: string;
  name: string;
  hostUserId: string;
  capacity: number;
  turnSeconds: number;
  turnLimit: number;
  state: 'Waiting' | 'InProgress' | 'Finished';
  currentTurn: number;
  turnStartedAt: string;
  winnerUserId: string | null;
  myPlayerId: string | null;
  players: LegacyGamePlayer[];
}

export interface Resources {
  wood: number; ore: number; mercury: number; sulfur: number;
  crystal: number; gems: number; gold: number;
}

export interface SeededPlayer {
  id: string;
  name: string;
  faction: string;
  cityName: string;
}

export interface GameSummary {
  id: string;
  name: string;
  state: 'Waiting' | 'InProgress' | 'Finished';
  day: number;
  week: number;
  month: number;
  turn: number;
  players: number;
}

export interface GamePlayer {
  playerId: string;
  name: string;
  faction: string;
  cityName: string;
  isAi: boolean;
  precious: boolean;
  obeliskVisits: number;
  resources: Resources;
  townId: string | null;
  heroIds: string[];
}

export interface GameDetail extends Omit<GameSummary, 'players'> {
  turnSeconds: number;
  turnLimit: number;
  turnStartedAt: string;
  mapWidth: number;
  mapHeight: number;
  winnerUserId: string | null;
  grailRevealed: boolean;
  grailX: number | null;
  grailY: number | null;
  players: GamePlayer[];
}

export interface ArmySlot {
  creature: string | null;
  count: number;
}

export interface HeroDetail {
  id: string;
  name: string;
  heroKey: string;
  faction: string;
  level: number;
  experience: number;
  nextLevel: number;
  attack: number;
  defense: number;
  power: number;
  knowledge: number;
  mana: number;
  maxMana: number;
  movement: number;
  maxMovement: number;
  x: number;
  y: number;
  army: ArmySlot[];
  strength: number;
  skills: Record<string, number>;
  spells: string[];
  artifacts: string[];
  /** HoMM3 spellbook: needed to learn and cast spells. */
  spellbook: boolean;
  /** War machines bought at a Blacksmith: ballista, ammoCart, firstAidTent. */
  warMachines: string[];
  pendingEncounter: string | null;
}

export interface MapTile {
  terrain: string | null;
  variant: number;
  objectKind: string | null;
  objectData: string | null;
  owner: number;
  /** False for water, rock, trees, mountains, towns, mines and monster stacks. */
  passable: boolean;
  fog: number;
}

export interface MapData {
  width: number;
  height: number;
  tiles: MapTile[];
  heroes: (Pick<HeroDetail, 'id' | 'name' | 'faction' | 'x' | 'y' | 'level' | 'movement' | 'maxMovement'> & { isMine: boolean })[];
  towns: { id: string; name: string; faction: string; x: number; y: number; isMine: boolean }[];
}

export interface TownDetail {
  id: string;
  gameId: string;
  name: string;
  ownerName: string;
  isOwner: boolean;
  faction: string;
  x: number;
  y: number;
  buildings: string[];
  builtThisTurn: boolean;
  postSlots: number;
  goldIncome: number;
  growthMultiplier: number;
  /** Marketplaces the owner has across all their towns (HoMM3 trade rates improve with each). */
  marketCount: number;
  available: {
    tier: number; creature: string | null; upgraded: string | null; count: number;
    dwellingBuilt: boolean; upgradeBuilt: boolean; growth: number; dwellingName: string;
  }[];
  blacksmith: { machine: string; name: string; gold: number };
  garrison: ArmySlot[];
  heroInTown: {
    id: string; name: string; heroKey: string; army: ArmySlot[];
    spellbook: boolean; warMachines: string[]; spells: string[]; wisdom: number;
  } | null;
  tavern: string[];
  mageGuild: string[];
  resources: Resources;
}

export interface CreatureDef {
  key: string;
  name: string;
  faction: string;
  tier: number;
  attack: number;
  defense: number;
  damageMin: number;
  damageMax: number;
  health: number;
  speed: number;
  growth: number;
  gold: number;
  /** Full HoMM3 price per creature, including rare resources. */
  cost: Resources;
  shots: number;
  aiValue: number;
  abilities: string[];
  upgradeOf: string | null;
}

export interface HeroDef {
  key: string;
  name: string;
  faction: string;
  might: boolean;
  attack: number;
  defense: number;
  power: number;
  knowledge: number;
  skills: string[];
}

export interface SpellDef {
  key: string;
  name: string;
  level: number;
  school: string;
  cost: number;
  description: string;
}

export interface CityView {
  playerId: string;
  ownerName: string;
  cityName: string;
  isOwner: boolean;
  gameId: string;
  gameState: string;
  currentTurn: number;
  turnSeconds: number;
  turnStartedAt: string;
  builtThisTurn: boolean;
  precious: boolean;
  buildings: string[];
  resources: Resources | null;
  postSlots: number;
  postCount: number;
}

export interface Post {
  id: string;
  title: string;
  body: string;
  imageUrl: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface Comment {
  id: string;
  author: string;
  body: string;
  createdAt: string;
}

export interface WallMessage {
  id: string;
  author: string;
  body: string;
  parentId: string | null;
  createdAt: string;
}

export interface TowerMessage {
  id: string;
  author: string;
  body: string;
  createdAt: string;
}

export interface ObeliskState {
  visits: number;
  total: number;
  fragments: string[];
  wrongAnswers: number;
  messages: { id: string; author: string; body: string; createdAt: string }[];
}

export interface BuildingDef {
  key: string;
  name: string;
  category: string;
  cost: { gold: number; wood: number; ore: number; mercury: number; sulfur: number; crystal: number; gems: number };
  requires: string[];
  description: string;
}

/** Realtime event from the SignalR hub at /hubs/game (method name "event"). */
export type WsEventType =
  | 'turnAdvanced' | 'heroMoved' | 'encounter' | 'message' | 'trade' | 'project'
  | 'annotation' | 'wall' | 'tower';

export interface WsEvent {
  type: WsEventType;
  gameId: string;
  payload: Record<string, unknown>;
}