import { useState } from 'preact/hooks';
import type { ConfiguredTool, HttpToolConfig } from '../types';

type Props = {
  tools: ConfiguredTool[];
  error?: string | null;
  onChange: (operation: string, payload: Record<string, unknown>) => Promise<void>;
  onEditMcp?: () => void;
};

const querySchema = {
  type: 'object', properties: { query: { type: 'string' } }, required: ['query'],
};

export function ToolsSettings({ tools, error, onChange, onEditMcp }: Props) {
  const [editing, setEditing] = useState<ConfiguredTool | 'new' | null>(null);
  const [busy, setBusy] = useState(false);
  const [pendingToggle, setPendingToggle] = useState<{ id: string; enabled: boolean } | null>(null);
  const [message, setMessage] = useState('');
  const act = async (op: string, payload: Record<string, unknown>) => {
    setBusy(true);
    setMessage('');
    try {
      await onChange(op, payload);
      setMessage('Saved.');
      return true;
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      return false;
    } finally { setBusy(false); setPendingToggle(null); }
  };
  if (editing && editing !== 'new' && !editing.http) return <DefinitionEditor key={editing.id}
    tool={editing} busy={busy} message={message} cancel={() => setEditing(null)}
    configureMcp={onEditMcp}
    save={async definition => {
      if (await act('definition_save', { id: editing.id, definition })) setEditing(null);
    }}
    reset={async () => {
      if (await act('definition_reset', { id: editing.id })) setEditing(null);
    }} />;
  if (editing) return <HttpEditor key={editing === 'new' ? 'new' : editing.id}
    existing={editing === 'new' ? undefined : editing}
    save={async payload => {
      if (await act('http_save', payload)) setEditing(null);
    }} cancel={() => setEditing(null)} busy={busy} message={message} />;
  return <>
    <p class="ag-help">Only enabled tools are sent to the model. Browser operations, network access, skills and MCP tools are all listed here. Existing enabled tools are preserved on upgrade.</p>
    <p class="ag-help">Definitions are sent separately from the System Prompt in the model request's <code>tools</code> field. The model returns <code>tool_calls</code>; the browser runs the selected handler below and sends back its result. Edit changes the model-facing name, description and parameters, not the handler implementation.</p>
    {error ? <p role="alert">Could not load tool configuration: {error}</p> : null}
    <div><button type="button" class="chip-btn is-accent" disabled={busy || !!error}
      onClick={() => { setMessage(''); setEditing('new'); }}>Add HTTP tool</button>{' '}
      <span role="status">{message}</span></div>
    {tools.map(tool => <section class="ag-card" key={tool.id} data-tool-id={tool.id}>
      <div class="ag-card-h"><b>{tool.title}</b><span class="ag-badge">{tool.source}</span></div>
      <code>{tool.name}</code>
      {tool.name !== tool.id ? <small>Handler ID: {tool.id}</small> : null}
      <p class="ag-help">{tool.description}</p>
      <p class="ag-help"><b>Target:</b> <code>{tool.target}</code></p>
      <label class="ag-check"><input type="checkbox" checked={pendingToggle?.id === tool.id ? pendingToggle.enabled : tool.enabled}
        aria-label={`Enable ${tool.id}`} disabled={busy || !!error}
        onChange={event => {
          const enabled = event.currentTarget.checked;
          setPendingToggle({ id: tool.id, enabled });
          void act('enabled_set', { id: tool.id, enabled });
        }} />
        <span>Enabled{tool.read_only ? ' (read-only)' : ' (may change data)'}</span></label>
      <details><summary>Model tool definition (tools field)</summary>
        <pre class="ag-code">{JSON.stringify(tool.model_definition || {
          type: 'function', function: { name: tool.name, description: tool.description, parameters: tool.parameters },
        }, null, 2)}</pre>
        {tool.http ? <><div>Response: {tool.http.response}</div>
          {tool.http.method === 'POST' ? <pre class="ag-code">{JSON.stringify(tool.http.body, null, 2)}</pre> : null}</> : null}
      </details>
      <div>
        <button type="button" class="chip-btn" disabled={busy} aria-label={`Edit ${tool.id}`}
          onClick={() => { setMessage(''); setEditing(tool); }}>Edit</button>{' '}
        {tool.http ? <button type="button" class="chip-btn is-danger" disabled={busy}
          aria-label={`Delete ${tool.id}`} onClick={() => {
            if (confirm(`Delete HTTP tool "${tool.title}"?`)) void act('http_delete', { id: tool.id });
          }}>Delete</button> : null}
      </div>
    </section>)}
  </>;
}

function DefinitionEditor({ tool, save, reset, cancel, configureMcp, busy, message }: {
  tool: ConfiguredTool;
  save: (definition: Record<string, unknown>) => Promise<void>;
  reset: () => Promise<void>;
  cancel: () => void;
  configureMcp?: () => void;
  busy: boolean;
  message: string;
}) {
  const [name, setName] = useState(tool.name);
  const [title, setTitle] = useState(tool.title);
  const [description, setDescription] = useState(tool.description);
  const [parameters, setParameters] = useState(JSON.stringify(tool.parameters, null, 2));
  const [error, setError] = useState('');
  const submit = async () => {
    try {
      setError('');
      await save({ name: name.trim(), title: title.trim(), description: description.trim(), parameters: JSON.parse(parameters) });
    } catch (error) { setError(error instanceof Error ? error.message : String(error)); }
  };
  return <>
    <h3>Edit {tool.id}</h3>
    <p class="ag-help">The model sees the name, description and parameter schema below through the request's <code>tools</code> field. The stable handler ID routes calls to the existing implementation. Parameter names must still match that handler's arguments.</p>
    <label class="ag-field">Handler ID<input value={tool.id} readOnly /></label>
    <label class="ag-field">Model tool name<input value={name} onInput={e => setName(e.currentTarget.value)} /></label>
    <label class="ag-field">Title<input value={title} onInput={e => setTitle(e.currentTarget.value)} /></label>
    <label class="ag-field">Description<textarea value={description} onInput={e => setDescription(e.currentTarget.value)} /></label>
    <label class="ag-field">Parameters (JSON Schema)<textarea class="ag-edit" rows={12} spellcheck={false}
      value={parameters} onInput={e => setParameters(e.currentTarget.value)} /></label>
    <p class="ag-help"><b>Execution target:</b> <code>{tool.target}</code></p>
    {tool.source === 'mcp' && configureMcp ? <button type="button" class="chip-btn"
      onClick={configureMcp}>Edit MCP connection</button> : null}
    <details><summary>Model tool definition preview</summary><pre class="ag-code">{JSON.stringify({
      type: 'function', function: {
        name, description, parameters: (() => { try { return JSON.parse(parameters); } catch { return '(invalid JSON)'; } })(),
      },
    }, null, 2)}</pre></details>
    {error ? <p role="alert">{error}</p> : null}
    <div><button type="button" class="chip-btn is-accent" disabled={busy} onClick={() => void submit()}>Save</button>{' '}
      <button type="button" class="chip-btn" disabled={busy} onClick={cancel}>Cancel</button>{' '}
      <button type="button" class="chip-btn" disabled={busy} onClick={() => void reset()}>Restore source definition</button>{' '}
      <span role="status">{message}</span></div>
  </>;
}

function HttpEditor({ existing, save, cancel, busy, message }: {
  existing?: ConfiguredTool;
  save: (payload: Record<string, unknown>) => Promise<void>;
  cancel: () => void;
  busy: boolean;
  message: string;
}) {
  const tool = existing?.http;
  const [id, setId] = useState(tool?.id || '');
  const [name, setName] = useState(tool?.name || tool?.id || '');
  const [title, setTitle] = useState(tool?.title || '');
  const [description, setDescription] = useState(tool?.description || '');
  const [method, setMethod] = useState<HttpToolConfig['method']>(tool?.method || 'POST');
  const [url, setUrl] = useState(tool?.url || '');
  const [parameters, setParameters] = useState(JSON.stringify(tool?.parameters || querySchema, null, 2));
  const [body, setBody] = useState(JSON.stringify(tool?.body ?? { query: '{{query}}' }, null, 2));
  const [response, setResponse] = useState<HttpToolConfig['response']>(tool?.response || 'json');
  const [readOnly, setReadOnly] = useState(tool?.read_only ?? false);
  const [enabled, setEnabled] = useState(existing?.enabled ?? true);
  const [error, setError] = useState('');
  const preset = (document: boolean) => {
    if (!existing) {
      const id = document ? 'get_search_document' : 'moonshot_search';
      setId(id);
      setName(id);
    }
    setTitle(document ? 'MoonShot document' : 'MoonShot search');
    setDescription(document ? 'Retrieve complete search document content by document_id.'
      : 'Search MoonShot and return matching documents with document_id.');
    setMethod(document ? 'GET' : 'POST');
    setUrl('');
    setParameters(JSON.stringify(document ? {
      type: 'object', properties: { document_id: { type: 'string' } }, required: ['document_id'],
    } : querySchema, null, 2));
    setBody(JSON.stringify(document ? null : { mode: 'text', query: '{{query}}', offset: 0, limit: 10 }, null, 2));
    setResponse('json');
    setReadOnly(true);
  };
  const submit = async () => {
    try {
      setError('');
      const definition: HttpToolConfig = {
        id: id.trim(), name: name.trim() || id.trim(), title: title.trim(), description: description.trim(), method,
        url: url.trim(), parameters: JSON.parse(parameters),
        body: method === 'POST' ? JSON.parse(body) : null, response, read_only: readOnly,
      };
      await save({ tool: definition, editing: !!existing, enabled });
    } catch (error) { setError(error instanceof Error ? error.message : String(error)); }
  };
  return <>
    <h3>{existing ? `Edit ${existing.id}` : 'Add HTTP tool'}</h3>
    <p class="ag-help">Use {'{{argument}}'} in the URL or JSON body. URL values are encoded; JSON placeholders preserve their types. No endpoint is contacted until the model calls this enabled tool.</p>
    <div><button type="button" class="chip-btn" onClick={() => preset(false)}>MoonShot search template</button>{' '}
      <button type="button" class="chip-btn" onClick={() => preset(true)}>Document lookup template</button></div>
    <label class="ag-field">Tool ID<input value={id} disabled={!!existing} onInput={e => setId(e.currentTarget.value)} /></label>
    <label class="ag-field">Model tool name<input value={name} placeholder={id || 'Defaults to Tool ID'}
      onInput={e => setName(e.currentTarget.value)} /></label>
    <label class="ag-field">Title<input value={title} onInput={e => setTitle(e.currentTarget.value)} /></label>
    <label class="ag-field">Description<textarea value={description} onInput={e => setDescription(e.currentTarget.value)} /></label>
    <label class="ag-field">Method<select value={method} onChange={e => setMethod(e.currentTarget.value as HttpToolConfig['method'])}>
      <option value="GET">GET</option><option value="POST">POST JSON</option></select></label>
    <label class="ag-field">URL<input value={url} spellcheck={false}
      placeholder={method === 'POST' ? 'http://host:3010/v1/search' : 'http://host:3010/v1/documents/{{document_id}}'}
      onInput={e => setUrl(e.currentTarget.value)} /></label>
    <label class="ag-field">Parameters (JSON Schema)<textarea class="ag-edit" rows={8} spellcheck={false}
      value={parameters} onInput={e => setParameters(e.currentTarget.value)} /></label>
    {method === 'POST' ? <label class="ag-field">JSON body<textarea class="ag-edit" rows={7} spellcheck={false}
      value={body} onInput={e => setBody(e.currentTarget.value)} /></label> : null}
    <label class="ag-field">Response format<select value={response}
      onChange={e => setResponse(e.currentTarget.value as HttpToolConfig['response'])}>
      <option value="json">JSON</option><option value="text">Text / HTML</option>
      <option value="duckduckgo_html">DuckDuckGo HTML results</option></select></label>
    <label class="ag-check"><input type="checkbox" checked={readOnly} onChange={e => setReadOnly(e.currentTarget.checked)} /> Read-only tool</label>
    <label class="ag-check"><input type="checkbox" checked={enabled} onChange={e => setEnabled(e.currentTarget.checked)} /> Enabled</label>
    {error ? <p role="alert">{error}</p> : null}
    <div><button type="button" class="chip-btn is-accent" disabled={busy} onClick={() => void submit()}>Save</button>{' '}
      <button type="button" class="chip-btn" disabled={busy} onClick={cancel}>Cancel</button>{' '}
      <span role="status">{message}</span></div>
  </>;
}
