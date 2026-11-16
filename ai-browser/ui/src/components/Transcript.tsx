import { useEffect, useRef } from 'preact/hooks';
import { Markdown } from '../markdown';
import { approve, call, native, openLink, setStatus, turns, agentOn, busy } from '../store';
import { failureLine, provenance, reveal } from '../reducers';
import { hostOf } from '../context';
import type { AgentTurn, AskTurn, ContextEcho, ToolRow, ToolSummary } from '../types';

function Echo({ e }: { e: ContextEcho }) {
  const bits: string[] = [`📄 ${(e.title || hostOf(e.url)).slice(0, 40)}`];
  if (e.selectionChars) bits.push(`✂ ${e.selectionChars} chars selected`);
  if (e.contentSent != null) bits.push(`${e.contentSent}${e.contentChars && e.contentChars > e.contentSent ? ' of ' + e.contentChars : ''} chars of page text`);
  else if (e.contentChars) bits.push(`${e.contentChars} chars of page text`);
  if (e.headings) bits.push(`${e.headings} headings`);
  bits.push(`${e.tabs} tab${e.tabs === 1 ? '' : 's'}`);
  if (e.edited) bits.push('✎ edited context');
  return (
    <div class="ag-echo" title={e.url}>
      {bits.map((b) => (
        <span class="ag-chip is-static" key={b}>{b}</span>
      ))}
    </div>
  );
}

function ToolLine({ t }: { t: ToolRow }) {
  const cls = t.state === 'waiting' ? 'is-waiting' : t.ok === false ? 'is-bad' : t.ok === true ? 'is-ok' : 'is-running';
  return (
    <>
      <div class={'ag-tool ' + cls}>
        <span class="ag-dot" />
        <b>{t.name}</b>
        <span class="ag-args"> {reveal(t.args)}</span>
        {t.ms != null ? <i> {t.ms}ms</i> : null}
        {t.preview ? <div class="ag-preview">{t.preview}</div> : null}
      </div>
      {t.approval ? <ApprovalCard t={t} /> : null}
    </>
  );
}

function ApprovalCard({ t }: { t: ToolRow }) {
  const a = t.approval!;
  if (a.answered) return <div class="ag-approve"><div class="ag-approve-h">{(a.answered === 'allowed' ? 'Allowed: ' : 'Denied: ') + a.name}</div></div>;
  const title = a.server ? `Allow ${a.name} (MCP: ${a.server})?` : `Allow the agent to ${a.name.replace(/_/g, ' ')}?`;
  return (
    <div class="ag-approve">
      <div class="ag-approve-h">{title}</div>
      <pre class="ag-code">{reveal(a.args)}</pre>
      <div>
        <button type="button" class="chip-btn is-accent" onClick={() => approve(a.call_id, true)}>Allow</button>{' '}
        <button type="button" class="chip-btn is-danger" onClick={() => approve(a.call_id, false)}>Deny</button>
      </div>
    </div>
  );
}

function ToolSummaryList({ rows }: { rows: ToolSummary[] }) {
  if (!rows.length) return null;
  return (
    <details class="ag-tools">
      <summary>{rows.length} tool call{rows.length === 1 ? '' : 's'}</summary>
      {rows.map((t, i) => (
        <div key={i} class={'ag-tool ' + (t.ok === false ? 'is-bad' : '')}>
          <b>{t.name}</b>
          <span class="ag-args"> {reveal(t.args || '')}</span>
          {t.ms != null ? <i> {t.ms}ms</i> : null}
          {t.preview ? <div class="ag-preview">{t.preview}</div> : null}
        </div>
      ))}
    </details>
  );
}

function AgentView({ t }: { t: AgentTurn }) {
  const live = t.status === 'running';
  return (
    <div class="ag-turn">
      <div class="ag-msg is-user">{t.question}</div>
      {t.echo ? <Echo e={t.echo} /> : null}
      <div class={'ag-msg is-assistant' + (live ? ' is-live' : '')}>
        {live ? (
          <div class="ag-live">
            {t.thinking.map((x, i) => <div class="ag-think" key={i}>{x}</div>)}
            {t.tools.map((r) => <ToolLine key={r.id} t={r} />)}
          </div>
        ) : t.summary ? (
          <ToolSummaryList rows={t.summary} />
        ) : (
          <ToolSummaryList rows={t.tools.map((r) => ({ name: r.name, args: r.args, ok: r.ok, ms: r.ms, preview: r.preview }))} />
        )}
        <div class="ag-answer">
          {t.status === 'error' ? <div class="ag-error">{t.error}</div> : null}
          {t.text ? <Markdown text={t.text} /> : null}
        </div>
        {t.links?.length ? (
          <div class="ag-links">
            {t.links.map((l) => (
              <button type="button" class="ag-link" key={l.url} title={l.url} onClick={() => openLink(l.url)}>↗ Open {hostOf(l.url)}</button>
            ))}
          </div>
        ) : null}
        {t.status === 'done' && t.rounds ? <div class="ag-meta">{t.rounds} round{t.rounds === 1 ? '' : 's'}</div> : null}
      </div>
    </div>
  );
}

function CitationChip({ t, c }: { t: AskTurn; c: AskTurn['citations'][number] }) {
  const ref = useRef<HTMLButtonElement>(null);
  const click = async () => {
    setStatus('');
    try {
      const res = await native.invoke<{ scrolled?: boolean }>('ai.citation.scroll', { turn_id: t.turnId, citation_id: c.citation_id });
      if (res?.scrolled) setStatus(`Scrolled to citation ${c.ordinal}.`);
    } catch (e) {
      // The host refuses to scroll a document this citation no longer describes.
      const msg = (e instanceof Error ? e.message : String(e)).slice(0, 200) || 'citation refused';
      ref.current?.classList.add('is-stale');
      if (ref.current) ref.current.title = msg;
      setStatus(msg, 'error');
    }
  };
  return (
    <button ref={ref} type="button" class={'ask-chip' + (t.stale ? ' is-stale' : '')} disabled={t.stale}
      title={c.quote ? `“${c.quote.slice(0, 220)}” — click to scroll the page there` : 'Click to scroll the page there'} onClick={click}>
      {String(c.ordinal)}
    </button>
  );
}

function AskView({ t }: { t: AskTurn }) {
  const pending = t.status === 'pending';
  const meta = t.stale ? `${t.staleStatus || 'stale'} — the page changed after this answer` : provenance(t);
  const route = t.routeLabel && !pending && t.status !== 'error' ? (meta ? ' · ' : '') + t.routeLabel : '';
  return (
    <div class={'ask-turn' + (t.stale ? ' is-stale' : '')}>
      <div class="ask-question">{t.question}</div>
      <div class={'ask-answer' + (pending ? ' is-pending' : '') + (t.status === 'error' ? ' is-error' : '')}>
        {pending ? (t.streamed ? <Markdown text={t.streamed} /> : 'Thinking…')
          : t.status === 'error' ? t.text || failureLine(t.failure) || 'No answer.'
          : t.text ? <Markdown text={t.text} /> : 'The provider returned no text.'}
      </div>
      {t.citations.length ? <div class="ask-citations">{t.citations.map((c) => <CitationChip key={String(c.citation_id)} t={t} c={c} />)}</div> : null}
      <div class="ask-meta">{meta + route}</div>
    </div>
  );
}

const STARTERS: [string, string][] = [
  ['Research a topic', 'Research this topic on the web and give me a sourced summary: '],
  ['Summarize this page', 'Summarize the page I am reading.'],
  ['Compare my tabs', 'Compare the pages in my open tabs.'],
];

export function Transcript({ onStarter }: { onStarter: (text: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const list = turns.value;
  const last = list[list.length - 1];
  const sig = list.length + ':' + (last ? (last.kind === 'agent' ? last.text.length + last.tools.length + last.status : last.streamed.length + last.status) : '');
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [sig]);
  return (
    <div id="ask-log" class="ask-log ag-log" ref={ref}>
      {!list.length ? (
        <>
          <p class="overlay-empty">
            {agentOn.value
              ? 'Give me a task. I can read your open tabs, browse and search the web, and use your skills and MCP tools. I ask before I click, type or navigate.'
              : 'Ask me anything. I can use the page you are reading when it helps, and I cite the passage when I do.'}
          </p>
          {agentOn.value ? (
            <div class="ask-quick">
              {STARTERS.map(([label, text]) => <button type="button" class="ask-qbtn" key={label} onClick={() => onStarter(text)}>{label}</button>)}
            </div>
          ) : null}
        </>
      ) : null}
      {list.map((t) => (t.kind === 'agent' ? <AgentView key={t.id} t={t} /> : <AskView key={t.id} t={t} />))}
      {busy.value ? null : null}
    </div>
  );
}

export { call };
