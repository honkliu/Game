import { useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { agentState, aiStatus, busy, call, model, native, refreshAgentState, refreshStatus, setModel, setStatus, settingsTab, skillDraft } from '../store';
import type { AgentState, McpServer, SkillInfo, SkillMode } from '../types';
import { ToolsSettings } from './ToolsSettings';
import { ModelsSettings } from './ModelsSettings';

const MODES: SkillMode[] = ['always-on', 'auto', 'off'];
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Runs a save-style operation and reports its outcome next to the button. */
function useAction() {
  const [msg, setMsg] = useState<{ text: string; kind?: string }>({ text: '' });
  const run = async <T,>(doing: string, fn: () => Promise<T>): Promise<T | undefined> => {
    setMsg({ text: doing });
    try {
      const r = await fn();
      if (r && typeof r === 'object' && 'prompts' in (r as object)) agentState.value = r as unknown as AgentState;
      setMsg({ text: 'Saved.' });
      return r;
    } catch (e) {
      setMsg({ text: errText(e) || 'failed', kind: 'error' });
      return undefined;
    }
  };
  return { msg, run, view: <span class={'ask-status' + (msg.kind ? ' is-' + msg.kind : '')}>{msg.text}</span> };
}

const Field = ({ label, children }: { label: string; children: ComponentChildren }) => (
  <label class="ag-field">{label}{children}</label>
);

function PromptsTab({ s }: { s: AgentState }) {
  return (
    <>
      <p class="ag-help">These are the exact instructions the agent receives. Edit them freely; placeholders in {'{{double braces}}'} are filled in on every run. Reset restores the built-in text.</p>
      {s.prompts.map((p) => <PromptCard key={p.name + p.text} p={p} />)}
    </>
  );
}

function PromptCard({ p }: { p: AgentState['prompts'][number] }) {
  const [text, setText] = useState(p.text);
  const a = useAction();
  return (
    <section class="ag-card">
      <div class="ag-card-h"><b>{p.name}</b>{p.customized ? <span class="ag-badge">customized</span> : null}</div>
      <div class="ag-help">{p.help}</div>
      <textarea class="ag-edit" rows={p.name === 'system' ? 16 : 8} spellcheck={false} value={text} onInput={(e) => setText(e.currentTarget.value)} />
      <div>
        <button type="button" class="chip-btn is-accent" onClick={() => void a.run('Saving…', () => call('prompt.save', { name: p.name, text }))}>Save</button>{' '}
        <button type="button" class="chip-btn" onClick={() => void a.run('Resetting…', () => call('prompt.reset', { name: p.name }))}>Reset to default</button>{' '}
        {a.view}
      </div>
    </section>
  );
}

function InstructionsTab({ s }: { s: AgentState }) {
  const [text, setText] = useState(s.config.custom_instructions || '');
  const a = useAction();
  return (
    <>
      <p class="ag-help">Your own standing instructions. They are added to every agent run (the system prompt's {'{{custom_instructions}}'} placeholder).</p>
      <textarea class="ag-edit" rows={12} spellcheck={false} value={text} placeholder="e.g. Always answer in British English. Prefer primary sources. Keep answers short." onInput={(e) => setText(e.currentTarget.value)} />
      <div><button type="button" class="chip-btn is-accent" onClick={() => void a.run('Saving…', () => call('config.set', { custom_instructions: text }))}>Save</button> {a.view}</div>
    </>
  );
}

function ModelTab() {
  const refresh = async () => {
    await refreshStatus();
    await native.invoke('ai.models', {}).catch(() => undefined);
  };
  return <ModelsSettings connections={aiStatus.value?.provider?.connections ?? []} disabled={busy.value || aiStatus.value?.busy}
    onSave={async payload => { await native.invoke('ai.provider.set', payload); await refresh(); }}
    onDelete={async id => {
      await native.invoke('ai.provider.delete', { id });
      if (model.value.startsWith(id + '/')) setModel('');
      await refresh();
    }} />;
}

function GeneralTab({ s }: { s: AgentState }) {
  const [auto, setAuto] = useState(!!s.config.auto_approve);
  const [rounds, setRounds] = useState(String(s.config.max_rounds));
  const a = useAction();
  return (
    <>
      <label class="ag-check"><input type="checkbox" checked={auto} onChange={(e) => setAuto(e.currentTarget.checked)} /><span> Run actions without asking (click, type, navigate and non-read-only MCP tools). Off is safer.</span></label>
      <label class="ag-check">Max tool rounds per run <input type="number" min={1} max={60} class="ag-num" value={rounds} onInput={(e) => setRounds(e.currentTarget.value)} /></label>
      <div><button type="button" class="chip-btn is-accent" onClick={() => void a.run('Saving…', () => call('config.set', { auto_approve: auto, max_rounds: Number(rounds) || 25 }))}>Save</button> {a.view}</div>
      <p class="ag-help">Model endpoints and keys are on the Models tab.</p>
    </>
  );
}

// ── skills ───────────────────────────────────────────────────────────────

function SkillsTab({ s }: { s: AgentState }) {
  const [editing, setEditing] = useState<{ skill: Partial<SkillInfo>; isNew: boolean } | null>(
    skillDraft.value ? { skill: skillDraft.value, isNew: true } : null,
  );
  if (skillDraft.value && !editing) skillDraft.value = null;
  if (editing) return <SkillEditor e={editing} done={() => { skillDraft.value = null; setEditing(null); }} />;
  return (
    <>
      <p class="ag-help">A skill is a playbook the agent follows. "always-on" is added to every run, "auto" is loaded by the agent when the task matches its description or triggers, "off" is disabled.</p>
      <div><button type="button" class="chip-btn is-accent" onClick={() => setEditing({ skill: { id: '', name: '', description: '', triggers: [], mode: 'auto', instructions: '' }, isNew: true })}>New skill</button></div>
      {s.skills.map((k) => (
        <section class="ag-card" key={k.id}>
          <div class="ag-card-h">
            <b>{k.name}</b>{k.builtin ? <span class="ag-badge">built-in</span> : null}
            <select class="ag-mode" value={k.mode} onChange={(e) => call<AgentState>('skill.mode', { id: k.id, mode: e.currentTarget.value }).then((r) => { agentState.value = r; }).catch((err) => setStatus(errText(err), 'error'))}>
              {MODES.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
          </div>
          <div class="ag-help">{k.description}</div>
          <div>
            <button type="button" class="chip-btn" onClick={() => call<SkillInfo>('skill.get', { id: k.id }).then((skill) => setEditing({ skill, isNew: false })).catch((err) => setStatus(errText(err), 'error'))}>Edit</button>{' '}
            {k.builtin ? null : <button type="button" class="chip-btn is-danger" onClick={() => { if (confirm(`Delete skill "${k.name}"?`)) void call<AgentState>('skill.delete', { id: k.id }).then((r) => { agentState.value = r; }); }}>Delete</button>}
          </div>
        </section>
      ))}
    </>
  );
}

function SkillEditor({ e, done }: { e: { skill: Partial<SkillInfo>; isNew: boolean }; done: () => void }) {
  const k = e.skill;
  const [name, setName] = useState(k.name ?? '');
  const [desc, setDesc] = useState(k.description ?? '');
  const [trig, setTrig] = useState((k.triggers ?? []).join(', '));
  const [mode, setMode] = useState<SkillMode>(k.mode ?? 'auto');
  const [ins, setIns] = useState(k.instructions ?? '');
  const a = useAction();
  const save = async () => {
    const r = await a.run('Saving…', () => call('skill.save', {
      id: k.id || '', name, description: desc, mode, instructions: ins,
      triggers: trig.split(',').map((x) => x.trim()).filter(Boolean),
    }));
    if (r) { await refreshAgentState(); done(); }
  };
  return (
    <>
      <p class="ag-help">{e.isNew ? 'New skill' : `Editing "${k.name}"${k.builtin ? ' — saving creates your own copy that overrides the built-in one.' : ''}`}</p>
      <Field label="Name"><input type="text" value={name} onInput={(ev) => setName(ev.currentTarget.value)} /></Field>
      <Field label="Description (when should the agent use it?)"><input type="text" value={desc} onInput={(ev) => setDesc(ev.currentTarget.value)} /></Field>
      <Field label="Trigger phrases (comma separated)"><input type="text" value={trig} onInput={(ev) => setTrig(ev.currentTarget.value)} /></Field>
      <Field label="Activation"><select value={mode} onChange={(ev) => setMode(ev.currentTarget.value as SkillMode)}>{MODES.map((m) => <option key={m} value={m}>{m}</option>)}</select></Field>
      <Field label="Instructions (markdown)"><textarea class="ag-edit" rows={14} spellcheck={false} value={ins} onInput={(ev) => setIns(ev.currentTarget.value)} /></Field>
      <div>
        <button type="button" class="chip-btn is-accent" onClick={() => void save()}>Save skill</button>{' '}
        <button type="button" class="chip-btn" onClick={done}>Cancel</button> {a.view}
      </div>
    </>
  );
}

// ── MCP ──────────────────────────────────────────────────────────────────

function McpTab({ s }: { s: AgentState }) {
  const [editing, setEditing] = useState<Partial<McpServer> | null>(null);
  if (editing) return <McpEditor m={editing} done={() => setEditing(null)} />;
  const set = (r: AgentState) => { agentState.value = r; };
  return (
    <>
      <p class="ag-help">MCP servers add tools to the agent. Tools that are not read-only ask for approval unless you mark the server trusted.</p>
      <div>
        <button type="button" class="chip-btn is-accent" onClick={() => setEditing({ id: '', name: '', url: '', command: '', args: [], enabled: true, trust: false })}>Add server</button>{' '}
        <button type="button" class="chip-btn" onClick={() => void call<AgentState>('mcp.sync').then(set)}>Reconnect all</button>
      </div>
      {!s.mcp.length ? <p class="overlay-empty">No MCP servers yet.</p> : null}
      {s.mcp.map((m) => (
        <section class="ag-card" key={m.id}>
          <div class="ag-card-h">
            <b>{m.name}</b>
            <span class={'ag-badge ' + (m.status === 'connected' ? 'is-ok' : m.status === 'error' ? 'is-bad' : '')}>{m.status}</span>
            {m.trust ? <span class="ag-badge">trusted</span> : null}
          </div>
          <div class="ag-help">{m.url || [m.command, ...(m.args ?? [])].join(' ')}</div>
          {m.error ? <div class="ag-error">{m.error}</div> : null}
          {m.tools?.length ? (
            <details><summary>{m.tools.length} tools</summary>
              {m.tools.map((t) => <div class="ag-help" key={t.name}><b>{t.name}</b>{(t.read_only ? ' (read-only) ' : ' ') + (t.description || '').slice(0, 120)}</div>)}
            </details>
          ) : null}
          <div>
            <button type="button" class="chip-btn" onClick={() => setEditing(m)}>Edit</button>{' '}
            <button type="button" class="chip-btn" onClick={() => void call<AgentState>('mcp.retry', { id: m.id }).then(set).catch((e) => setStatus(errText(e), 'error'))}>Retry</button>{' '}
            <button type="button" class="chip-btn is-danger" onClick={() => { if (confirm(`Remove server "${m.name}"?`)) void call<AgentState>('mcp.delete', { id: m.id }).then(set); }}>Delete</button>
          </div>
        </section>
      ))}
    </>
  );
}

function McpEditor({ m, done }: { m: Partial<McpServer>; done: () => void }) {
  const [kind, setKind] = useState(m.command ? 'stdio' : 'url');
  const [name, setName] = useState(m.name ?? '');
  const [url, setUrl] = useState(m.url ?? '');
  const [token, setToken] = useState('');
  const [cmd, setCmd] = useState(m.command ?? '');
  const [args, setArgs] = useState((m.args ?? []).join(' '));
  const [trust, setTrust] = useState(!!m.trust);
  const [enabled, setEnabled] = useState(m.enabled !== false);
  const a = useAction();
  const save = async () => {
    const payload: Record<string, unknown> = { id: m.id || '', name, enabled, trust };
    if (kind === 'url') Object.assign(payload, { url, command: '', args: [], ...(token ? { token } : {}) });
    else Object.assign(payload, { url: '', command: cmd, args: args.split(/\s+/).filter(Boolean) });
    if (await a.run('Saving…', () => call('mcp.save', payload))) done();
  };
  return (
    <>
      <p class="ag-help">{m.id ? `Editing "${m.name}"` : 'Add an MCP server'}</p>
      <Field label="Name"><input type="text" value={name} onInput={(e) => setName(e.currentTarget.value)} /></Field>
      <Field label="Type">
        <select value={kind} onChange={(e) => setKind(e.currentTarget.value)}>
          <option value="url">HTTP (streamable)</option><option value="stdio">Local command (stdio)</option>
        </select>
      </Field>
      {kind === 'url' ? (
        <>
          <Field label="URL"><input type="text" value={url} placeholder="https://host/mcp" onInput={(e) => setUrl(e.currentTarget.value)} /></Field>
          <Field label="Bearer token"><input type="password" value={token} placeholder={m.has_token ? 'saved — leave empty to keep it' : 'optional bearer token'} onInput={(e) => setToken(e.currentTarget.value)} /></Field>
        </>
      ) : (
        <>
          <Field label="Command"><input type="text" value={cmd} placeholder="npx" onInput={(e) => setCmd(e.currentTarget.value)} /></Field>
          <Field label="Arguments (space separated)"><input type="text" value={args} placeholder="-y @scope/server" onInput={(e) => setArgs(e.currentTarget.value)} /></Field>
        </>
      )}
      <label class="ag-check"><input type="checkbox" checked={trust} onChange={(e) => setTrust(e.currentTarget.checked)} /><span> Trusted: never ask before running its tools</span></label>
      <label class="ag-check"><input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.currentTarget.checked)} /><span> Enabled</span></label>
      <div>
        <button type="button" class="chip-btn is-accent" onClick={() => void save()}>Save</button>{' '}
        <button type="button" class="chip-btn" onClick={done}>Cancel</button> {a.view}
      </div>
    </>
  );
}

// ── shell of the settings view ───────────────────────────────────────────

export function SettingsPanel() {
  const s = agentState.value;
  const tab = settingsTab.value;
  // MCP connections take a while; keep the status fresh while that tab is open.
  useMcpPolling(tab === 'mcp' || tab === 'tools');
  return (
      <div class="ag-body settings-agent-body">
        {tab === 'model' ? <ModelTab /> : !s ? <p class="overlay-empty">Loading…</p>
          : tab === 'tools' ? <ToolsSettings tools={s.tools || []} error={s.tools_error}
              onEditMcp={() => { settingsTab.value = 'mcp'; }}
              onChange={async (op, payload) => { agentState.value = await call<AgentState>(op, payload); }} />
          : tab === 'prompts' ? <PromptsTab s={s} />
          : tab === 'instructions' ? <InstructionsTab s={s} />
          : tab === 'skills' ? <SkillsTab s={s} />
          : tab === 'mcp' ? <McpTab s={s} />
          : <GeneralTab s={s} />}
      </div>
  );
}

import { useEffect } from 'preact/hooks';
function useMcpPolling(active: boolean) {
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => void refreshAgentState(), 4000);
    return () => clearInterval(id);
  }, [active]);
}
