import { render } from 'preact';
import { act } from 'preact/test-utils';
import { expect, it, vi } from 'vitest';
import { native, settingsTab } from '../src/store';
import { settingsReady } from '../src/settings_store';
import { SettingsPage } from '../src/components/SettingsPage';

vi.mock('../src/components/Settings', () => ({ SettingsPanel: () => null }));

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
