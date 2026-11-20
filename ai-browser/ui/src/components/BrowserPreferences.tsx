import { useEffect, useState } from 'preact/hooks';
import { native } from '../store';
import { preferences, type BrowserSettings } from '../settings_store';

export function BrowserPreferences({ settings }: { settings: BrowserSettings }) {
  const [draft, setDraft] = useState(settings);
  const [message, setMessage] = useState('');
  useEffect(() => setDraft(settings), [settings]);
  const save = async (key: keyof BrowserSettings, value: string | number | boolean) => {
    setDraft(current => ({ ...current, [key]: value }));
    setMessage('Saving...');
    try {
      preferences.value = await native.invoke<BrowserSettings>('shell_settings', { operation: 'set', key, value });
      setMessage('Saved.');
    } catch (error) {
      setDraft(settings);
      setMessage(String(error));
    }
  };
  const check = (key: keyof BrowserSettings, label: string) => <label class="ag-check">
    <input id={key} type="checkbox" checked={!!draft[key]} onChange={event => void save(key, event.currentTarget.checked)} />{label}
  </label>;
  const text = (key: 'search_template' | 'download_dir', label: string) => <label class="ag-field">{label}
    <input id={key} type="text" value={draft[key]} onInput={event => setDraft(current => ({ ...current, [key]: event.currentTarget.value }))}
      onChange={event => void save(key, event.currentTarget.value)} />
  </label>;
  const number = (key: 'history_cap' | 'sleep_after_minutes', label: string) => <label class="ag-field">{label}
    <input id={key} type="number" min={key === 'history_cap' ? 1000 : 1} max={key === 'history_cap' ? 1000000 : 240}
      value={draft[key]} onInput={event => setDraft(current => ({ ...current, [key]: Number(event.currentTarget.value) }))}
      onChange={event => void save(key, Number(event.currentTarget.value))} />
  </label>;
  return <>
    <section class="ag-card"><h2>Appearance</h2>
      <fieldset><legend>Theme</legend>{['system', 'light', 'dark'].map(theme => <label class="ag-check" key={theme}>
        <input type="radio" name="theme" value={theme} checked={draft.theme === theme} onChange={() => void save('theme', theme)} />{theme}
      </label>)}</fieldset>
      {check('vertical_tabs', 'Vertical tabs')}
      {check('rail_collapsed', 'Collapsed tab rail')}
      {check('compact_tab_rows', 'Compact tab rows')}
      <label class="ag-field">Density<select id="density" value={draft.density} onChange={event => void save('density', event.currentTarget.value)}>
        <option value="comfortable">Comfortable</option><option value="compact">Compact</option>
      </select></label>
    </section>
    <section class="ag-card"><h2>Search and address bar</h2>
      {text('search_template', 'Search URL template (%s is replaced with your search)')}
      {check('always_show_full_url', 'Always show full URL')}
    </section>
    <section class="ag-card"><h2>Privacy and storage</h2>
      {check('clear_history_on_exit', 'Clear history on exit')}
      {check('clear_cookies_on_exit', 'Clear cookies on exit')}
      {check('clear_cache_on_exit', 'Clear cache on exit')}
      {number('history_cap', 'Maximum history entries')}
      <p class="ag-help">Stored history entries: <span id="history_count">{settings.history_count}</span></p>
      {number('sleep_after_minutes', 'Sleep inactive tabs after minutes')}
      {text('download_dir', 'Download folder')}
      <button id="open_folder" type="button" class="chip-btn" onClick={() => void native.invoke('shell_settings', { operation: 'profile_folder_open' }).catch(error => setMessage(String(error)))}>Open data folder</button>
      <p id="data_note" class="ag-help">{settings.data_dir}</p>
      <p id="build" class="ag-help">{settings.engine_name} runtime: {settings.engine_version}</p>
    </section>
    <p role="status">{message}</p>
  </>;
}
