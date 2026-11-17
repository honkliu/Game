import { useState } from 'preact/hooks';
import type { ProviderConnection, ProviderModel } from '../types';

const newModel = (): ProviderModel => ({ model_id: '', display_name: '', context_window: 32768, supports_tools: true });

export function ModelsSettings({ connections, disabled, onSave, onDelete }: {
  connections: ProviderConnection[];
  disabled?: boolean;
  onSave: (connection: Record<string, unknown>) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
}) {
  const [editing, setEditing] = useState<ProviderConnection | null>(null);
  const [message, setMessage] = useState('');
  const [saving, setSaving] = useState(false);
  const act = async (fn: () => Promise<void>) => {
    setSaving(true);
    setMessage('Saving...');
    try { await fn(); setMessage('Saved.'); setEditing(null); }
    catch (error) { setMessage(error instanceof Error ? error.message : String(error)); }
    finally { setSaving(false); }
  };
  const empty = { id: '', base_url: '', enabled: true, models: [newModel()] };
  return (
    <>
      <p class="ag-help">Add a model name and an OpenAI-compatible endpoint. The API key is optional. Switch models in chat with <code>/model</code>; no routing configuration is needed.</p>
      {editing || !connections.length ? (
        <ConnectionEditor key={editing?.id ?? 'first'} connection={editing ?? empty} disabled={disabled || saving}
          onSave={payload => act(() => onSave(payload))}
          onCancel={connections.length ? () => { setEditing(null); setMessage(''); } : undefined} />
      ) : (
        <>
          <div><button type="button" class="chip-btn is-accent" disabled={disabled || saving}
            onClick={() => { setEditing(empty); setMessage(''); }}>Add model</button></div>
          {connections.map(connection => (
            <section class="ag-card" key={connection.id}>
              <div class="ag-card-h"><b>{connection.models.map(model => model.display_name || model.model_id).join(', ')}</b>{!connection.enabled ? <span class="ag-badge">disabled</span> : null}</div>
              <div class="ag-help">{connection.base_url}</div>
              {connection.models.map(model => <div class="ag-help" key={model.model_id}>
                <b>{model.display_name || model.model_id}</b>{model.display_name && model.display_name !== model.model_id ? ` (${model.model_id})` : ''}
                {model.supports_tools ? ' - tool calling' : ' - questions only'}
              </div>)}
              {connection.key_error ? <p class="ag-error">{connection.key_error}</p> : null}
              <div>
                <button type="button" class="chip-btn" disabled={disabled || saving}
                  onClick={() => { setEditing(connection); setMessage(''); }}>Edit</button>{' '}
                <button type="button" class="chip-btn is-danger" disabled={disabled || saving}
                  onClick={() => { if (confirm('Remove these models and their saved API key?')) void act(() => onDelete(connection.id)); }}>Remove</button>
              </div>
            </section>
          ))}
        </>
      )}
      {disabled ? <p class="ag-help">Stop the current AI task before editing models.</p> : null}
      <p class="ask-status" role="status">{message}</p>
    </>
  );
}

function ConnectionEditor({ connection, disabled, onSave, onCancel }: {
  connection: ProviderConnection;
  disabled?: boolean;
  onSave: (payload: Record<string, unknown>) => Promise<void>;
  onCancel?: () => void;
}) {
  const [id, setId] = useState(connection.id);
  const [url, setUrl] = useState(connection.base_url);
  const [enabled, setEnabled] = useState(connection.enabled);
  const [key, setKey] = useState('');
  const [clearKey, setClearKey] = useState(false);
  const [models, setModels] = useState(connection.models.map(model => ({ ...model })));
  const update = (index: number, patch: Partial<ProviderModel>) => setModels(models.map((model, n) => n === index ? { ...model, ...patch } : model));
  return (
    <fieldset disabled={disabled} class="ag-model-editor">
      <legend>{connection.id ? 'Edit model' : 'Add model'}</legend>
      {models.map((model, index) => <label class="ag-field" key={index}>Model name<input type="text" spellcheck={false}
        value={model.model_id} placeholder="e.g. sample-model" onInput={e => update(index, { model_id: e.currentTarget.value })} /></label>)}
      <label class="ag-field">Endpoint<input type="text" spellcheck={false} value={url}
        placeholder="http://host:8001/v1" onInput={e => setUrl(e.currentTarget.value)} /></label>
      <label class="ag-field">API key<input type="password" value={key} disabled={clearKey}
        placeholder={connection.key_saved ? 'saved - leave empty to keep it' : 'optional'}
        onInput={e => setKey(e.currentTarget.value)} /></label>
      {connection.key_saved ? <label class="ag-check"><input type="checkbox" checked={clearKey}
        onChange={e => setClearKey(e.currentTarget.checked)} /> Remove saved API key on save</label> : null}
      <details>
      <summary>Advanced</summary>
      <label class="ag-field">Connection ID<input type="text" value={id} readOnly={!!connection.id}
        placeholder="generated automatically" onInput={e => setId(e.currentTarget.value)} /></label>
      <label class="ag-check"><input type="checkbox" checked={enabled} onChange={e => setEnabled(e.currentTarget.checked)} /> Enabled</label>
      {models.map((model, index) => (
        <section class="ag-card" key={index}>
          <div class="ag-card-h"><b>Model {index + 1}</b></div>
          <label class="ag-field">Display name<input type="text" value={model.display_name || ''}
            placeholder="optional" onInput={e => update(index, { display_name: e.currentTarget.value })} /></label>
          <label class="ag-field">Context window<input type="number" min={1} value={model.context_window ?? 32768}
            onInput={e => update(index, { context_window: Number(e.currentTarget.value) })} /></label>
          <label class="ag-check"><input type="checkbox" checked={!!model.supports_tools}
            onChange={e => update(index, { supports_tools: e.currentTarget.checked })} /> Supports tool calling (required for Agent)</label>
          <button type="button" class="chip-btn" disabled={models.length === 1}
            onClick={() => setModels(models.filter((_model, n) => n !== index))}>Remove model</button>
        </section>
      ))}
      <div><button type="button" class="chip-btn" onClick={() => setModels([...models, newModel()])}>Add another model on this endpoint</button></div>
      </details>
      <div>
        <button type="button" class="chip-btn is-accent" onClick={() => void onSave({
          id, creating: !connection.id, base_url: url, enabled, api_key: key, clear_key: clearKey,
          models: models.map(model => ({ ...model, model_id: model.model_id.trim(), display_name: model.display_name?.trim() || null })),
        })}>Save model</button>{' '}
        {onCancel ? <button type="button" class="chip-btn" onClick={onCancel}>Cancel</button> : null}
      </div>
    </fieldset>
  );
}
