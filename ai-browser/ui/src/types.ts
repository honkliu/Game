// Shapes of the data that crosses the bridge, and of the transcript built from it.

export interface PageInfo {
  tab_id: number;
  title: string;
  url: string;
  selection?: string | null;
}

export interface TabInfo {
  tab_id: number;
  title: string;
  url: string;
  active?: boolean;
}

/** What the shell last read from the active page (see `PageSnapshot` in the shell). */
export interface ContentInfo {
  rev: number;
  chars: number;
  headings: number;
  fields: number;
  filled: number;
  preview: string;
  at: number;
}

export interface AgentContext {
  page: PageInfo | null;
  tabs: TabInfo[];
  content?: ContentInfo | null;
}

export interface Provider {
  base_url?: string;
  model_id?: string;
  key_saved?: boolean;
  configured?: boolean;
  connections?: ProviderConnection[];
}

export interface ProviderModel {
  model_id: string;
  display_name?: string | null;
  context_window?: number;
  supports_tools?: boolean;
  aliases?: string[];
  modalities?: string[];
}

export interface ProviderConnection {
  id: string;
  base_url: string;
  enabled: boolean;
  models: ProviderModel[];
  key_saved?: boolean;
  key_error?: string;
}

export interface Coverage {
  status?: string;
  estimated_completeness?: number;
  inaccessible_frames?: unknown[];
  unloaded_regions?: unknown[];
  unsupported_content?: unknown[];
  has_untranscribed_media?: boolean;
}

export interface AiStatus {
  busy?: boolean;
  enabled: boolean;
  reason?: string;
  askable: boolean;
  tab?: { host?: string; ai_managed?: boolean };
  selection?: { chars: number } | null;
  context?: { coverage?: Coverage; blocks: number } | null;
  other_tabs?: { tab_id: number; title?: string; host?: string }[];
  multi_tab?: boolean;
  cloud_enabled?: boolean;
  provider?: Provider;
  models?: ListedModel[];
}

export interface ModelInfo {
  provider_id: string;
  model_id: string;
  display_name?: string;
  local?: boolean;
  supports_tools?: boolean;
  available?: boolean;
  unavailable_reason?: string;
}

export interface ListedModel {
  manifest: ModelInfo & { execution?: string };
  available: boolean;
  unavailable_reason?: string;
}

export interface Citation {
  citation_id: number;
  ordinal: number;
  quote?: string;
}

export interface Failure {
  kind?: string;
  code?: string;
  detail?: string;
}

export interface Candidate {
  tab_id: number;
  title?: string;
  url?: string;
  reason?: string;
}

// ── agent engine state ───────────────────────────────────────────────────

export type SkillMode = 'always-on' | 'auto' | 'off';

export interface SkillInfo {
  id: string;
  name: string;
  description: string;
  triggers: string[];
  mode: SkillMode;
  builtin?: boolean;
  instructions?: string;
}

export interface PromptInfo {
  name: string;
  help: string;
  text: string;
  default?: string;
  customized: boolean;
}

export interface McpTool {
  name: string;
  description?: string;
  read_only?: boolean;
}

export interface McpServer {
  id: string;
  name: string;
  enabled?: boolean;
  trust?: boolean;
  url?: string;
  command?: string;
  args?: string[];
  has_token?: boolean;
  status: string;
  error?: string;
  tools: McpTool[];
}

export interface HttpToolConfig {
  id: string;
  name?: string;
  title: string;
  description: string;
  method: 'GET' | 'POST';
  url: string;
  parameters: Record<string, unknown>;
  body: unknown;
  response: 'json' | 'text' | 'duckduckgo_html';
  read_only: boolean;
}

export interface ConfiguredTool {
  id: string;
  name: string;
  title: string;
  description: string;
  source: 'browser' | 'local' | 'http' | 'mcp';
  target: string;
  enabled: boolean;
  read_only: boolean;
  parameters: Record<string, unknown>;
  http?: HttpToolConfig;
  model_definition?: Record<string, unknown>;
}

export interface AgentState {
  config: { auto_approve: boolean; custom_instructions: string; max_rounds: number };
  skills: SkillInfo[];
  prompts: PromptInfo[];
  mcp: McpServer[];
  tools?: ConfiguredTool[];
  tools_enabled?: boolean;
  tools_error?: string | null;
}

export interface ConversationRow {
  id: string;
  title?: string;
  updated?: number;
}

// ── transcript ───────────────────────────────────────────────────────────

export interface ToolSummary {
  name: string;
  args?: string;
  ok?: boolean;
  ms?: number;
  preview?: string;
}

export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
  tools?: ToolSummary[];
  meta?: string;
  reasoning?: string;
  thinking?: string[];
}

export interface Approval {
  call_id: string;
  name: string;
  args: string;
  server?: string;
  answered?: 'allowed' | 'denied';
}

export interface ToolRow {
  id: string;
  name: string;
  args: string;
  state: 'running' | 'waiting' | 'done';
  ok?: boolean;
  ms?: number;
  preview?: string;
  approval?: Approval;
}

/** What was sent with a message, shown under it so the context is never a guess. */
export interface ContextEcho {
  title: string;
  url: string;
  selectionChars: number;
  tabs: number;
  contentChars?: number | null;
  contentSent?: number | null;
  headings?: number;
  edited?: boolean;
}

export interface AgentTurn {
  kind: 'agent';
  id: number;
  question: string;
  status: 'running' | 'done' | 'error';
  runId: number;
  thinking: string[];
  reasoning?: string;
  tools: ToolRow[];
  text: string;
  rounds: number;
  error?: string;
  echo?: ContextEcho;
  discussion?: { id: string; turn_id: number; participant: string };
  /** Pages the agent offered in manual navigation mode, to be opened by a click. */
  links?: { id: string; url: string }[];
  /** Restored from a saved conversation: only tool summaries survive. */
  summary?: ToolSummary[];
}

export interface AskTurn {
  reasoning?: string;
  kind: 'ask';
  id: number;
  turnId: number;
  question: string;
  status: 'pending' | 'ok' | 'error';
  streamed: string;
  text: string;
  provider: string;
  model: string;
  elapsedMs: number;
  citations: Citation[];
  failure: Failure | null;
  stale: boolean;
  staleStatus?: string;
  routeLabel?: string;
}

export type Turn = AgentTurn | AskTurn;

export type AgentEvent = { state_seq?: number } & (
  | { type: 'queued'; run_id: number; turn: Omit<AgentTurn, 'id'> }
  | { type: 'start'; run_id: number; model?: string }
  | { type: 'round'; run_id: number; n: number }
  | { type: 'delta'; run_id: number; text: string }
  | { type: 'thinking'; run_id: number; text: string }
  | { type: 'tool_start'; run_id: number; call_id: string; name: string; args?: string }
  | { type: 'approval'; run_id: number; call_id: string; name: string; args?: string; server?: string }
  | { type: 'tool_end'; run_id: number; call_id: string; ok: boolean; ms?: number; preview?: string }
  | { type: 'link'; run_id: number; call_id: string; url: string }
  | { type: 'compacted'; run_id: number }
  | { type: 'context'; run_id: number; content_chars?: number | null; content_sent?: number | null; headings?: number }
  | { type: 'done'; run_id: number; text?: string; rounds?: number; loaded_skills?: string[] }
  | { type: 'error'; run_id: number; message?: string; stopped?: boolean });
