import { useEffect, useState } from 'preact/hooks';
import {
  agentOn, aiStatus, contentUpdatedAt, ctx, ctxEdit, ctxNotice, ctxPanel, discardContextEdit, editContext, localOnly,
  sendMessage, setStatus, toggleContextPanel, useSelection, busy, SELECTION_PROMPTS, type Intent,
} from '../store';
import { ageLabel, hostOf } from '../context';
import type { Coverage } from '../types';

// The Ask pipeline's own view of the page: coverage, passages, selection, egress.
function askSummary(): string {
  const s = aiStatus.value;
  if (!s) return '';
  if (!s.enabled) return 'AI unavailable: ' + (s.reason || 'off');
  if (!s.askable) return 'This page cannot be read by the assistant';
  const bits: string[] = [s.tab?.host || 'this page'];
  if (s.context) {
    const cov: Coverage = s.context.coverage ?? {};
    const gaps = (cov.inaccessible_frames?.length ?? 0) + (cov.unloaded_regions?.length ?? 0) + (cov.unsupported_content?.length ?? 0);
    const pct = typeof cov.estimated_completeness === 'number' ? Math.round(cov.estimated_completeness * 100) : null;
    if ((cov.status && cov.status !== 'complete_enough') || gaps) bits.push('△ partial page coverage' + (pct !== null ? ` (${pct}%)` : ''));
    if (cov.has_untranscribed_media) bits.push('△ media has no transcript');
    bits.push(`${s.context.blocks} passages`);
  } else bits.push('page read on ask');
  if (s.selection) bits.push(`${s.selection.chars} chars selected`);
  bits.push(s.cloud_enabled ? 'cloud models allowed' : 'local only');
  if (s.tab?.ai_managed) bits.push('AI-managed tab');
  return bits.join(' · ');
}

const ACTIONS: [Intent, string][] = [
  ['explain', 'Explain'], ['verify', 'Verify'], ['translate', 'Translate'], ['rewrite', 'Rewrite'],
];

function useNow(ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [ms]);
  return now;
}

/** The selected text with the actions that use it: the answer to "I selected something,
 *  now what?" and to "what exactly is being sent?". */
function SelectionCard({ text }: { text: string }) {
  const copy = (event: ClipboardEvent) => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount !== 1) return;
    const range = selection.getRangeAt(0);
    if (!(event.currentTarget as HTMLElement).contains(range.commonAncestorContainer)) return;
    event.preventDefault();
    event.clipboardData!.setData('text/plain', range.toString());
  };
  const act = (intent: Intent) => {
    if (agentOn.value) void sendMessage(SELECTION_PROMPTS[intent] ?? '');
    else void sendMessage('', { intent, selection: true });
  };
  return (
    <div class="sel-card" id="sel-card">
      <div class="sel-card-h">
        <b>✂ Selected text</b>
        <span>{text.length} chars · sent with your next message</span>
      </div>
      <blockquote class="sel-quote" onCopy={copy}>{text}</blockquote>
      <div class="sel-actions">
        {ACTIONS.map(([k, label]) => (
          <button type="button" class="ask-qbtn" key={k} disabled={busy.value} onClick={() => act(k)}>{label}</button>
        ))}
      </div>
    </div>
  );
}

export function ContextBar() {
  const c = ctx.value;
  const p = c?.page;
  const now = useNow(5000);
  const panel = ctxPanel.value;
  const content = c?.content;
  const fresh = content && contentUpdatedAt.value && now - contentUpdatedAt.value < 4000;
  const edit = ctxEdit.value;
  return (
    <div id="ag-ctx" class="ag-ctx" aria-label="Context sent with your message">
      <div id="ask-ctx" class="ask-ctx" hidden={agentOn.value}>{askSummary()}</div>
      <div class="ag-ctx-chips">
        <span class="ag-chip is-static" title={p?.url ?? ''}>{p ? `📄 ${p.title || hostOf(p.url)}`.slice(0, 48) : 'No page open'}</span>
        {content ? (
          <span class={'ag-chip is-static' + (fresh ? ' is-live' : '')} id="ctx-content"
            title={content.preview}>
            {content.chars} chars{content.fields ? ` · ${content.filled}/${content.fields} fields` : ''} · {fresh ? 'page updated' : 'read ' + ageLabel(content.at, now)}
          </span>
        ) : null}
        <span class="ag-chip is-static">{c?.tabs.length ?? 0} tab{(c?.tabs.length ?? 0) === 1 ? '' : 's'} listed</span>
        {edit ? <span class="ag-chip is-sel">✎ edited</span> : null}
        <button type="button" class="ag-chip" id="ctx-toggle" onClick={toggleContextPanel}>
          {panel.open ? 'Hide context' : 'View / edit context'}
        </button>
      </div>
      {p?.selection ? <SelectionCard text={p.selection} /> : null}
      {panel.open ? <ContextPanel /> : null}
    </div>
  );
}

function ContextPanel() {
  const panel = ctxPanel.value;
  const edit = ctxEdit.value;
  const text = edit ? edit.text : panel.context;
  return (
    <div class="ctx-panel" id="ctx-panel">
      <div class="ag-help">
        This is what the model receives about the browser, rebuilt from the page as it changes. You can edit it for your next
        messages; when the page changes the edit is dropped and the context follows the page again. Your skills, prompts and
        instructions are stored separately and are never touched by this.
      </div>
      {ctxNotice.value ? <div class="ag-error" id="ctx-notice">{ctxNotice.value}</div> : null}
      {panel.error ? <div class="ag-error">{panel.error}</div> : null}
      <textarea id="ctx-edit" class="ag-edit ctx-edit" rows={10} spellcheck={false} value={panel.loading && !text ? 'Rendering…' : text}
        disabled={panel.loading && !text} onInput={(e) => editContext((e.currentTarget as HTMLTextAreaElement).value)} />
      <div>
        {edit ? <button type="button" class="chip-btn" onClick={discardContextEdit}>Discard my edit</button> : null}
        {!agentOn.value ? <span class="ag-help"> Edits apply in Agent mode. Ask mode uses the page passages it cites.</span> : null}
      </div>
      <details>
        <summary>Full system prompt</summary>
        <pre class="ag-code ag-full">{panel.system}</pre>
      </details>
    </div>
  );
}

export { localOnly, setStatus, useSelection };
