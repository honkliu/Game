import { signal } from '@preact/signals';
import { applyModels, applyStatus, native, refreshAgentState, refreshStatus, settingsTab, skillDraft, type SettingsTab } from './store';

export interface BrowserSettings {
  theme: string;
  system_dark: boolean;
  rail_collapsed: boolean;
  vertical_tabs: boolean;
  compact_tab_rows: boolean;
  density: string;
  search_template: string;
  clear_history_on_exit: boolean;
  clear_cookies_on_exit: boolean;
  clear_cache_on_exit: boolean;
  always_show_full_url: boolean;
  history_cap: number;
  sleep_after_minutes: number;
  download_dir: string;
  history_count: number;
  engine_name: string;
  engine_version: string;
  data_dir: string;
}

export interface Download {
  id: number;
  url: string;
  file: string;
  path: string;
  state: string;
  total_bytes?: number;
  received_bytes: number | null;
}

export const preferences = signal<BrowserSettings | null>(null);
export const downloads = signal<Download[]>([]);
export const settingsError = signal('');

function applyPreferences(next: BrowserSettings) {
  preferences.value = next;
  document.documentElement.dataset.theme = next.theme === 'system' ? (next.system_dark ? 'dark' : 'light') : next.theme;
}

function section(next: { section: SettingsTab; skill_draft?: unknown }) {
  if (next.skill_draft) skillDraft.value = next.skill_draft;
  settingsTab.value = next.section;
}

export function startSettingsStore() {
  native.on('state.settings', applyPreferences);
  native.on('settings.section', section);
  native.on('state.downloads', payload => { downloads.value = payload.downloads ?? []; });
  native.on('ai.context', applyStatus);
  native.on('ai.models', payload => applyModels(payload.models ?? []));
  native.on('agent.event', () => void refreshStatus());
  native.on('ai.answer', () => void refreshStatus());
  window.addEventListener('focus', () => { void refreshStatus(); void refreshAgentState(); });
  void native.invoke<{ section: SettingsTab; settings: BrowserSettings; downloads: Download[]; skill_draft?: unknown }>('settings.state')
    .then(snapshot => {
      applyPreferences(snapshot.settings);
      downloads.value = snapshot.downloads;
      section(snapshot);
    }).catch(error => { settingsError.value = String(error); });
  void refreshStatus();
  void refreshAgentState();
  void native.invoke('ai.models').catch(() => undefined);
}
