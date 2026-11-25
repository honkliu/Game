import { useEffect } from 'preact/hooks';
import { native, settingsTab, status, type SettingsTab } from '../store';
import { preferences, settingsError, settingsReady } from '../settings_store';
import { SettingsPanel } from './Settings';
import { BrowserPreferences } from './BrowserPreferences';
import { BrowserHistory } from './BrowserHistory';
import { Downloads } from './Downloads';
import { History } from './History';
import { RiverMcpSettings } from './RiverMcpSettings';
import { Markdown } from '../markdown';
import manual from '../manual.md?raw';

const sections: [SettingsTab, string][] = [
  ['general', 'General'], ['model', 'Models'], ['tools', 'Tools'], ['skills', 'Skills'],
  ['mcp', 'MCP'], ['rivermcp', 'RiverMCP'], ['prompts', 'Prompts'], ['instructions', 'Instructions'],
  ['history', 'History'], ['downloads', 'Downloads'], ['chats', 'Saved chats'],
  ['help', 'Help'], ['about', 'About'],
];

export function SettingsPage() {
  const section = settingsTab.value;
  const ready = settingsReady.value;
  useEffect(() => {
    if (!ready) return;
    void native.invoke('shell_settings', { operation: 'section_set', section }).catch(error => { settingsError.value = String(error); });
  }, [section, ready]);
  return <div class="settings-page">
    <nav class="settings-nav" aria-label="Settings"><h1>Settings</h1><div role="tablist" aria-label="Settings sections" aria-orientation="vertical">
      {sections.map(([id, label]) => <button type="button" key={id} role="tab" disabled={!ready} aria-selected={section === id}
        aria-controls="settings-content" id={`settings-${id}`} onClick={() => { settingsTab.value = id; status.value = { text: '' }; }}>{label}</button>)}
    </div></nav>
    <main><div id="settings-content" role="tabpanel" aria-labelledby={`settings-${section}`}>
      {section !== 'help' ? <h1>{sections.find(([id]) => id === section)?.[1]}</h1> : null}
      {settingsError.value ? <p role="alert" class="ag-error">{settingsError.value}</p> : null}
      {status.value.kind === 'error' ? <p role="alert" class="ag-error">{status.value.text}</p> : null}
      {section === 'general' ? <>
        {preferences.value ? <BrowserPreferences settings={preferences.value} /> : <p>Loading preferences...</p>}
        <h2>River</h2><SettingsPanel key={section} />
      </> : section === 'about' ? preferences.value ? <p id="app-version">River {preferences.value.app_version}</p> : <p>Loading...</p>
        : section === 'help' ? <article class="settings-manual"><Markdown text={manual} /></article>
        : section === 'rivermcp' ? <RiverMcpSettings /> : section === 'history' ? <BrowserHistory /> : section === 'downloads' ? <Downloads /> : section === 'chats' ? <History /> : <SettingsPanel key={section} />}
    </div></main>
  </div>;
}
