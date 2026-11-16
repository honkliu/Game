import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { render } from 'preact';
import { Markdown, parseMarkdown } from '../src/markdown';
import { contextSignature, reconcileEdit } from '../src/context';
import { COMMANDS, createAgentCalls, createBridge } from '../src/ipc';
import { fromMessages, newAgentTurn, reduceAgent, toMessages } from '../src/reducers';
import type { AgentContext } from '../src/types';

const ctx = (over: Partial<AgentContext['page']> & { rev?: number } = {}): AgentContext =>
  ({
    page: { tab_id: 1, url: 'https://a/', title: 'A', selection: '', ...over },
    tabs: [{ tab_id: 1, url: 'https://a/', title: 'A' }],
    content: { rev: over.rev ?? 1, chars: 10 },
  }) as unknown as AgentContext;

describe('markdown', () => {
  it('never produces markup from model text', () => {
    const host = document.createElement('div');
    render(<Markdown text={'<img src=x onerror=alert(1)> **b** [l](javascript:alert(1))'} />, host);
    expect(host.querySelector('img')).toBeNull();
    expect(host.querySelector('a')).toBeNull();
    expect(host.querySelector('strong')?.textContent).toBe('b');
    expect(host.textContent).toContain('<img');
  });
  it('renders http(s) links, bare urls, tables, quotes and nested lists', () => {
    const host = document.createElement('div');
    render(
      <Markdown text={'See [**Docs**](https://a.example/x?y=1) and https://b.example/z.\n\n| A | B |\n|---|:-:|\n| 1 | 2 |\n\n> quoted\n\n- one\n  - two\n\n*it* ~~no~~ ## h'} />,
      host,
    );
    const hrefs = [...host.querySelectorAll('a')].map((a) => a.getAttribute('href'));
    expect(hrefs).toEqual(['https://a.example/x?y=1', 'https://b.example/z']);
    expect(host.querySelector('a strong')?.textContent).toBe('Docs');
    expect(host.querySelectorAll('td').length).toBe(2);
    expect(host.querySelector('blockquote')?.textContent).toContain('quoted');
    expect(host.querySelector('li li')?.textContent).toBe('two');
    expect(host.querySelector('em')?.textContent).toBe('it');
    expect(host.querySelector('del')?.textContent).toBe('no');
  });
  it('a link click opens a new tab through the shell and does not navigate the pane', () => {
    const post = vi.fn();
    (window as unknown as { ipc: unknown }).ipc = { postMessage: post };
    const host = document.createElement('div');
    document.body.appendChild(host);
    render(<Markdown text="[go](https://a.example/)" />, host);
    host.querySelector('a')!.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
    const sent = JSON.parse(post.mock.calls[0]?.[0] as string);
    expect(sent.cmd).toBe('nav.open');
    expect(sent.payload).toEqual({ url: 'https://a.example/', disposition: 'new_tab' });
  });
  it('parses lists and code fences', () => {
    const kinds = parseMarkdown('- a\n- b\n\n```\ncode\n```').map((b) => b.t);
    expect(kinds).toContain('ul');
  });
});

describe('editable context', () => {
  it('changes signature on content, selection, url and tab changes', () => {
    const base = contextSignature(ctx());
    expect(contextSignature(ctx())).toBe(base);
    expect(contextSignature(ctx({ rev: 2 }))).not.toBe(base);
    expect(contextSignature(ctx({ selection: 'x' }))).not.toBe(base);
    expect(contextSignature(ctx({ url: 'https://b/' }))).not.toBe(base);
  });
  it('keeps an edit only while the page is unchanged', () => {
    const sig = contextSignature(ctx());
    const edit = { text: 'mine', sig };
    expect(reconcileEdit(edit, sig)).toEqual({ edit, discarded: false });
    expect(reconcileEdit(edit, contextSignature(ctx({ rev: 2 })))).toEqual({ edit: null, discarded: true });
    expect(reconcileEdit(null, sig).discarded).toBe(false);
  });
});

describe('agent reducer and history', () => {
  it('builds a turn from events and ignores other runs', () => {
    let t = { ...newAgentTurn(1, 'q'), runId: 7 };
    t = reduceAgent(t, { type: 'delta', run_id: 7, text: 'think' } as never);
    t = reduceAgent(t, { type: 'tool_start', run_id: 7, call_id: 'c1', name: 'read_page', args: '{}' } as never);
    expect(t.thinking).toEqual(['think']);
    t = reduceAgent(t, { type: 'tool_end', run_id: 7, call_id: 'c1', ok: true, ms: 5, preview: '' } as never);
    t = reduceAgent(t, { type: 'delta', run_id: 99, text: 'foreign' } as never);
    t = reduceAgent(t, { type: 'done', run_id: 7, text: 'answer', rounds: 2 } as never);
    expect(t).toMatchObject({ status: 'done', text: 'answer', rounds: 2 });
    expect(t.tools[0]).toMatchObject({ state: 'done', ok: true });
  });
  it('round-trips a conversation through messages', () => {
    let id = 0;
    const done = { ...newAgentTurn(1, 'hello'), status: 'done' as const, text: 'hi', rounds: 1 };
    const msgs = toMessages([done]);
    expect(msgs.map((m) => m.role)).toEqual(['user', 'assistant']);
    const back = fromMessages(msgs, () => ++id);
    expect(back[0]).toMatchObject({ question: 'hello', text: 'hi', status: 'done' });
  });
});

describe('bridge', () => {
  it('resolves replies by id, and unsubscribes listeners', async () => {
    const post = vi.fn();
    const b = createBridge(post, 10);
    const p = b.invoke('ai.status');
    expect(JSON.parse(post.mock.calls[0]![0])).toMatchObject({ id: 10, cmd: 'ai.status' });
    b._reply({ id: 10, ok: true, data: { x: 1 } });
    await expect(p).resolves.toEqual({ x: 1 });
    const fn = vi.fn();
    const off = b.on('e', fn);
    b._dispatch({ event: 'e', payload: 1 });
    off();
    b._dispatch({ event: 'e', payload: 2 });
    expect(fn).toHaveBeenCalledTimes(1);
  });
  it('rejects failed commands and pairs agent.call with agent.reply', async () => {
    const b = createBridge((raw) => {
      const { id, payload } = JSON.parse(raw);
      b._reply({ id, ok: true });
      if (payload.op) queueMicrotask(() => b._dispatch({ event: 'agent.reply', payload: { req: payload.req, ok: true, value: payload.op } }));
    });
    const call = createAgentCalls(b);
    await expect(call('state')).resolves.toBe('state');
    const bad = createBridge((raw) => bad._reply({ id: JSON.parse(raw).id, ok: false, error: 'nope' }));
    await expect(bad.invoke('ai.stop')).rejects.toThrow('nope');
  });
});

describe('contract with the Rust dispatcher', () => {
  it('every command the pane uses is handled by the shell', () => {
    const rs = readFileSync(resolve(process.cwd(), '../crates/shell/src/app.rs'), 'utf8');
    for (const c of COMMANDS) expect(rs, c).toContain(`"${c}"`);
  });
});
