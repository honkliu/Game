import { render } from 'preact';
import { act } from 'preact/test-utils';
import { expect, it, vi } from 'vitest';
import { native, settingsTab } from '../src/store';
import { settingsReady } from '../src/settings_store';
import { SettingsPage } from '../src/components/SettingsPage';
import { RiverMcpSettings } from '../src/components/RiverMcpSettings';

vi.mock('../src/components/Settings', () => ({ SettingsPanel: () => null }));

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
