// Reducers: how events from the shell turn into transcript state. Pure functions over
// plain objects, so they are tested without a DOM.

import type { AgentEvent, AgentTurn, AskTurn, ChatMessage, Failure, ToolRow, ToolSummary, Turn } from './types';

// Invisible characters in an approval card could hide what is really being approved.
export function reveal(s: string): string {
  return String(s).replace(
    // eslint-disable-next-line no-control-regex
    /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u2064\ufeff]/g,
    (c) => '\\u' + c.charCodeAt(0).toString(16).padStart(4, '0'),
  );
}

export function newAgentTurn(id: number, question: string): AgentTurn {
  return { kind: 'agent', id, question, status: 'running', runId: 0, thinking: [], tools: [], text: '', rounds: 0 };
}

export function newAskTurn(id: number, turnId: number, question: string): AskTurn {
  return {
    kind: 'ask', id, turnId, question, status: 'pending', streamed: '', text: '', provider: 'offline', model: '',
    elapsedMs: 0, citations: [], failure: null, stale: false,
  };
}

function patchTool(t: AgentTurn, callId: string, f: (r: ToolRow) => ToolRow): AgentTurn {
  return { ...t, tools: t.tools.map((r) => (r.id === callId ? f(r) : r)) };
}

/** Apply one `agent.event` to a turn. Events for another run leave it untouched. */
export function reduceAgent(t: AgentTurn, ev: AgentEvent): AgentTurn {
  if (t.runId && ev.run_id !== t.runId) return t;
  switch (ev.type) {
    case 'round':
      return { ...t, rounds: ev.n };
    case 'delta':
      return { ...t, text: t.text + ev.text };
    case 'tool_start': {
      // Text streamed before a tool call is the model thinking aloud; keep it as a note.
      const thinking = t.text.trim() ? [...t.thinking, t.text.trim()] : t.thinking;
      const row: ToolRow = { id: ev.call_id, name: ev.name, args: ev.args ?? '', state: 'running' };
      return { ...t, thinking, text: '', tools: [...t.tools, row] };
    }
    case 'approval':
      return patchTool(t, ev.call_id, (r) => ({
        ...r,
        state: 'waiting',
        approval: { call_id: ev.call_id, name: ev.name, args: ev.args ?? '', server: ev.server },
      }));
    case 'tool_end':
      return patchTool(t, ev.call_id, (r) => ({ ...r, state: 'done', ok: ev.ok, ms: ev.ms, preview: ev.ok ? '' : ev.preview }));
    case 'link':
      return t.links?.some((l) => l.url === ev.url) ? t : { ...t, links: [...(t.links ?? []), { id: ev.call_id, url: ev.url }] };
    case 'context':
      return t.echo
        ? { ...t, echo: { ...t.echo, contentChars: ev.content_chars ?? t.echo.contentChars, contentSent: ev.content_sent, headings: ev.headings } }
        : t;
    case 'done':
      return { ...t, status: 'done', text: ev.text || '(no answer)', rounds: ev.rounds ?? t.rounds };
    case 'error':
      return { ...t, status: 'error', error: ev.stopped ? 'Stopped.' : ev.message || 'The agent failed.' };
    default:
      return t;
  }
}

/** The line shown in the status strip for an event, or null for "no change". */
export function agentStatusLine(ev: AgentEvent): string | null {
  switch (ev.type) {
    case 'start':
      return `Working… (${ev.model || 'model'})`;
    case 'round':
      return `Round ${ev.n} — thinking…`;
    case 'tool_start':
      return `Running ${ev.name}…`;
    case 'approval':
      return 'Waiting for your approval…';
    case 'compacted':
      return 'Context was compacted to keep going.';
    default:
      return null;
  }
}

export function summarizeTools(t: AgentTurn): ToolSummary[] {
  if (t.summary) return t.summary;
  return t.tools.map((r) => ({ name: r.name, args: r.args, ok: r.ok, ms: r.ms, preview: r.preview }));
}

export function failureLine(f: Failure | null): string {
  if (!f || !f.code) return '';
  return `${f.kind || 'failure'}/${f.code}${f.detail ? ' — ' + f.detail : ''}`;
}

/** Apply one `ai.answer` payload (the host sends `pending`, then the final payload). */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function reduceAnswer(t: AskTurn, p: any): AskTurn {
  return {
    ...t,
    question: p.question || t.question,
    status: p.status || 'ok',
    text: p.text || '',
    provider: p.provider || 'offline',
    model: p.model || '',
    elapsedMs: p.elapsed_ms || 0,
    citations: p.citations || [],
    failure: p.failure || null,
    stale: t.stale || !!p.stale,
  };
}

/** What produced an answer, said plainly: the offline stub must never read like a
 *  model, and a model answer must say the text came over the network. */
export function provenance(t: AskTurn): string {
  if (t.status === 'pending') return '';
  const ms = t.elapsedMs ? ` · ${(t.elapsedMs / 1000).toFixed(1)} s` : '';
  if (t.provider === 'http') return `${t.model || 'remote model'} · over the network${ms}`;
  if (t.model.startsWith('offline')) return `${t.model} · extractive, no network request${ms}`;
  if (t.model) return `${t.model} · on this device${ms}`;
  return `offline extract · no network request${ms}`;
}

/** The conversation as the model should see it, across both modes. */
export function toMessages(turns: Turn[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const t of turns) {
    if (t.kind === 'agent') {
      out.push({ role: 'user', content: t.question });
      if (t.status === 'done') {
        const tools = summarizeTools(t);
        out.push({ role: 'assistant', content: t.text, tools, meta: t.rounds ? `${t.rounds} round${t.rounds === 1 ? '' : 's'}` : '' });
      } else if (t.status === 'error' && t.text.trim()) {
        out.push({ role: 'assistant', content: t.text, tools: summarizeTools(t) });
      }
    } else if (t.status === 'ok' && t.text) {
      out.push({ role: 'user', content: t.question });
      out.push({ role: 'assistant', content: t.text });
    }
  }
  return out;
}

/** Saved conversations come back as finished agent turns (tools as summaries). */
export function fromMessages(messages: ChatMessage[], nextId: () => number): AgentTurn[] {
  const turns: AgentTurn[] = [];
  for (const m of messages) {
    if (m.role === 'user') {
      turns.push({ ...newAgentTurn(nextId(), m.content), status: 'done', text: '' });
    } else {
      const last = turns[turns.length - 1];
      if (last && !last.text) {
        last.text = m.content;
        last.summary = m.tools ?? [];
        last.rounds = Number.parseInt(m.meta ?? '', 10) || 0;
      }
    }
  }
  return turns;
}
