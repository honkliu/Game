import { downloads } from '../settings_store';
import { native } from '../store';
import { useState } from 'preact/hooks';

export function Downloads() {
  const [error, setError] = useState('');
  const action = async (id: number, kind: string) => {
    try { await native.invoke('download.action', { download_id: id, action: kind }); }
    catch (e) { setError(String(e)); }
  };
  return <>
    <button type="button" class="chip-btn" onClick={() => void native.invoke('download.clear_list').catch(e => setError(String(e)))}>Clear finished downloads</button>
    {error ? <p class="ag-error" role="alert">{error}</p> : null}
    {!downloads.value.length ? <p class="overlay-empty">No downloads yet.</p> : null}
    {downloads.value.map(download => <section class="ag-card" key={download.id}>
      <b>{download.file}</b><p class="ag-help">{download.path || download.url}</p>
      <p>{download.state.replaceAll('_', ' ')}{download.received_bytes ? ` - ${Math.round(download.received_bytes / 1024)} KB` : ''}</p>
      {download.state === 'in_progress' ? <progress max={download.total_bytes || undefined} value={download.total_bytes ? download.received_bytes ?? undefined : undefined} /> : null}
      <div class="settings-actions">
        <button type="button" class="chip-btn" disabled={download.state !== 'completed'} onClick={() => void action(download.id, 'open')}>Open</button>
        <button type="button" class="chip-btn" disabled={!download.path} onClick={() => void action(download.id, 'reveal')}>Show in folder</button>
        <button type="button" class="chip-btn" onClick={() => void action(download.id, 'remove')}>Remove from list</button>
      </div>
    </section>)}
  </>;
}
