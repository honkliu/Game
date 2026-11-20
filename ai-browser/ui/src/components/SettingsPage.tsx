import { useEffect } from 'preact/hooks';
import { native, settingsTab, status, type SettingsTab } from '../store';
import { preferences, settingsError } from '../settings_store';
import { SettingsPanel } from './Settings';
import { BrowserPreferences } from './BrowserPreferences';
import { BrowserHistory } from './BrowserHistory';
import { Downloads } from './Downloads';
import { History } from './History';

const sections: [SettingsTab, string][] = [
  ['general', 'General'], ['model', 'Models'], ['tools', 'Tools'], ['skills', 'Skills'],
  ['mcp', 'MCP'], ['prompts', 'Prompts'], ['instructions', 'Instructions'],
  ['history', 'History'], ['downloads', 'Downloads'], ['chats', 'Saved chats'],
];

export function SettingsPage() {
  const section = settingsTab.value;
  useEffect(() => {
    void native.invoke('shell_settings', { operation: 'section_set', section }).catch(error => { settingsError.value = String(error); });
  }, [section]);
  return <div class="settings-page">
    <nav class="settings-nav" aria-label="Settings"><h1>Settings</h1><div role="tablist" aria-label="Settings sections" aria-orientation="vertical">
      {sections.map(([id, label]) => <button type="button" key={id} role="tab" aria-selected={section === id}
        aria-controls="settings-content" id={`settings-${id}`} onClick={() => { settingsTab.value = id; status.value = { text: '' }; }}>{label}</button>)}
    </div></nav>
    <main><div id="settings-content" role="tabpanel" aria-labelledby={`settings-${section}`}>
      <h1>{sections.find(([id]) => id === section)?.[1]}</h1>
      {settingsError.value ? <p role="alert" class="ag-error">{settingsError.value}</p> : null}
      {status.value.kind === 'error' ? <p role="alert" class="ag-error">{status.value.text}</p> : null}
      {section === 'general' ? <>
        {preferences.value ? <BrowserPreferences settings={preferences.value} /> : <p>Loading preferences...</p>}
        <h2>River</h2><SettingsPanel key={section} />
      </> : section === 'history' ? <BrowserHistory /> : section === 'downloads' ? <Downloads /> : section === 'chats' ? <History /> : <SettingsPanel key={section} />}
    </div></main>
  </div>;
}
