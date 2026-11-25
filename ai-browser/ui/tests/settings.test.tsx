import { render } from 'preact';
import { act } from 'preact/test-utils';
import { expect, it, vi } from 'vitest';
import { native, settingsTab } from '../src/store';
import { preferences, settingsReady } from '../src/settings_store';
import { SettingsPage } from '../src/components/SettingsPage';
import { RiverMcpSettings } from '../src/components/RiverMcpSettings';

vi.mock('../src/components/Settings', () => ({ SettingsPanel: () => null }));

it('shows only the running AI Browser version in About and follows native state updates', async () => {
  const previous = { settings: preferences.value, ready: settingsReady.value, tab: settingsTab.value };
  const host = document.createElement('div');
  const invoke = vi.spyOn(native, 'invoke').mockResolvedValue(null);
  try {
    await act(() => {
      preferences.value = {
        app_version: '9.8.7', theme: 'system', system_dark: false, rail_collapsed: false,
        vertical_tabs: true, compact_tab_rows: false, density: 'comfortable', search_template: '',
        clear_history_on_exit: false, clear_cookies_on_exit: false, clear_cache_on_exit: false,
        always_show_full_url: false, history_cap: 10000, sleep_after_minutes: 30, download_dir: '',
        history_count: 0, engine_name: 'WebView2', engine_version: '123.0.0', data_dir: 'private-profile',
      };
      settingsReady.value = true; settingsTab.value = 'about';
      render(<SettingsPage />, host);
    });
    expect(host.querySelector('#app-version')?.textContent).toBe('AI Browser 9.8.7');
    expect(host.querySelector('#settings-content')?.textContent).toBe('AboutAI Browser 9.8.7');
    expect(invoke).toHaveBeenCalledWith('shell_settings', { operation: 'section_set', section: 'about' });
    await act(() => { preferences.value = { ...preferences.value!, app_version: '9.8.8' }; });
    expect(host.querySelector('#app-version')?.textContent).toBe('AI Browser 9.8.8');
  } finally {
    await act(() => { render(null, host); preferences.value = previous.settings; settingsReady.value = previous.ready; settingsTab.value = previous.tab; });
    invoke.mockRestore();
  }
});

it('opens the Markdown workflow manual with distinct Windows and macOS capabilities', async () => {
  const previous = { ready: settingsReady.value, tab: settingsTab.value };
  const host = document.createElement('div');
  const invoke = vi.spyOn(native, 'invoke').mockResolvedValue(null);
  try {
    await act(() => { settingsReady.value = true; settingsTab.value = 'general'; render(<SettingsPage />, host); });
    await act(() => host.querySelector<HTMLButtonElement>('#settings-help')!.click());
    const manual = host.querySelector('#settings-content article.settings-manual .rich')!;
    expect(manual.querySelector('h1')?.textContent).toBe('Using AI Browser');
    expect(host.querySelectorAll('#settings-content h1')).toHaveLength(1);
    expect(manual.querySelector('ol')?.textContent).toContain('Save model');
    const platforms = [...manual.querySelectorAll('table')].find(table => table.textContent?.includes('macOS'));
    expect(platforms?.textContent).toContain('Windows');
    expect(platforms?.textContent).toContain('Span monitors');
    expect(platforms?.textContent).toContain('not available');
    expect(invoke).toHaveBeenLastCalledWith('shell_settings', { operation: 'section_set', section: 'help' });
  } finally {
    await act(() => { render(null, host); settingsReady.value = previous.ready; settingsTab.value = previous.tab; });
    invoke.mockRestore();
  }
});

it('applies RiverMCP drafts explicitly and displays success or failure beside Apply', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const initial = { port: 18080, token: 'initial-token', active_port: 18080, listen_address: '0.0.0.0' };
  const invoke = vi.spyOn(native, 'invoke').mockResolvedValue(initial);
  try {
    await act(() => render(<RiverMcpSettings />, host));
    const port = host.querySelector<HTMLInputElement>('#rivermcp-port')!;
    const token = host.querySelector<HTMLInputElement>('#rivermcp-token')!;
    const apply = host.querySelector<HTMLButtonElement>('#rivermcp-apply')!;
    const result = host.querySelector<HTMLElement>('#rivermcp-result')!;
    expect(token.type).toBe('password');
    await act(() => { port.value = '18081'; port.dispatchEvent(new Event('input', { bubbles: true })); });
    expect(invoke).toHaveBeenCalledTimes(1);
    invoke.mockRejectedValueOnce(new Error('Port is in use'));
    await act(() => apply.click());
    await vi.waitFor(() => expect(host.querySelector('#rivermcp-result')?.textContent).toBe('Port is in use'));
    expect(invoke).toHaveBeenLastCalledWith('shell_settings', { operation: 'river_mcp_apply', value: { port: 18081, token: initial.token } });
    expect(result.parentElement).toBe(apply.parentElement);
    expect(result.getAttribute('role')).toBe('alert');
    expect(result.textContent).toBe('Port is in use');
    await act(() => { token.value = 'replacement-token'; token.dispatchEvent(new Event('input', { bubbles: true })); });
    expect(result.textContent).toBe('');
    invoke.mockResolvedValueOnce({ ...initial, port: 18081, active_port: 18081, token: 'replacement-token' });
    await act(() => apply.click());
    await vi.waitFor(() => expect(host.querySelector('#rivermcp-result')?.textContent).toBe('Applied. Listening on 0.0.0.0:18081.'));
    expect(result.textContent).toBe('Applied. Listening on 0.0.0.0:18081.');
    expect(result.getAttribute('role')).toBe('status');
  } finally { await act(() => render(null, host)); host.remove(); invoke.mockRestore(); }
});

it('does not overwrite the requested Settings section before its initial snapshot arrives', async () => {
  const host = document.createElement('div');
  const invoke = vi.spyOn(native, 'invoke').mockResolvedValue(null);
  document.body.appendChild(host);
  try {
    settingsTab.value = 'general';
    settingsReady.value = false;
    await act(() => render(<SettingsPage />, host));
    expect(invoke).not.toHaveBeenCalled();
    expect(host.querySelector<HTMLButtonElement>('#settings-model')!.disabled).toBe(true);
    await act(() => { settingsTab.value = 'model'; settingsReady.value = true; });
    expect(invoke).toHaveBeenCalledExactlyOnceWith('shell_settings', { operation: 'section_set', section: 'model' });
    expect(host.querySelector<HTMLButtonElement>('#settings-model')!.disabled).toBe(false);
  } finally {
    await act(() => render(null, host));
    host.remove(); invoke.mockRestore();
    settingsReady.value = false; settingsTab.value = 'general';
  }
});
