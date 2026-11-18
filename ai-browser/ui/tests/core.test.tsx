import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { render } from 'preact';
import { Markdown } from '../src/markdown';
import { contextSignature, reconcileEdit } from '../src/context';
import { COMMANDS, createAgentCalls, createBridge } from '../src/ipc';
import { fromMessages, newAgentTurn, reduceAgent, toMessages } from '../src/reducers';
import type { AgentContext } from '../src/types';
import type { ConfiguredTool } from '../src/types';
import { ToolsSettings } from '../src/components/ToolsSettings';
import { act } from 'preact/test-utils';
import { modelCommand } from '../src/model_commands';
import type { ModelInfo } from '../src/types';
import { ContextBar } from '../src/components/ContextBar';
import { ctx as liveContext } from '../src/store';

describe('/model command', () => {
  const entries = [
    { provider_id: 'a', model_id: 'org/sample', display_name: 'Sample', available: true, supports_tools: true },
    { provider_id: 'b', model_id: 'org/sample', display_name: 'Backup', available: true, supports_tools: true },
    { provider_id: 'c', model_id: 'offline', available: false, unavailable_reason: 'disabled' },
    { provider_id: 'c', model_id: 'questions', available: true, supports_tools: false },
  ] as ModelInfo[];
  it('handles local listing, exact qualified IDs, display names and default', () => {
    expect(modelCommand('/model', entries, true)).toEqual({ kind: 'list', query: '' });
    expect(modelCommand('/model a/org/sample', entries, true)).toMatchObject({ kind: 'select', id: 'a/org/sample' });
    expect(modelCommand('/model BACKUP', entries, true)).toMatchObject({ kind: 'select', id: 'b/org/sample' });
    expect(modelCommand('/model default', entries, true)).toMatchObject({ kind: 'select', id: '' });
    expect(modelCommand('/models', entries, true)).toBeNull();
    expect(modelCommand('explain /model', entries, true)).toBeNull();
  });
  it('does not silently select ambiguous, unknown or unusable models', () => {
    for (const id of ['org/sample', 'missing', 'offline', 'questions']) {
      expect(modelCommand(`/model ${id}`, entries, true)).toMatchObject({ kind: 'list', error: expect.any(String) });
    }
    expect(modelCommand('/model questions', entries, false)).toMatchObject({ kind: 'select', id: 'c/questions' });
  });
});

const ctx = (over: Partial<AgentContext['page']> & { rev?: number } = {}): AgentContext =>
  ({
    page: { tab_id: 1, url: 'https://a/', title: 'A', selection: '', ...over },
    tabs: [{ tab_id: 1, url: 'https://a/', title: 'A' }],
    content: { rev: over.rev ?? 1, chars: 10 },
  }) as unknown as AgentContext;

describe('markdown', () => {
  it('does not render arbitrary HTML or unsafe links', () => {
    const host = document.createElement('div');
    render(<Markdown text={'<img src=x onerror=alert(1)> **b** [l](javascript:alert(1))'} />, host);
    expect(host.querySelector('img')).toBeNull();
    expect(host.querySelector('a[href]')).toBeNull();
    expect(host.querySelector('strong')?.textContent).toBe('b');
    expect(host.querySelector('[onerror]')).toBeNull();
  });
  it('renders superscript notes with inline Markdown instead of literal HTML tags', () => {
    const host = document.createElement('div');
    render(<Markdown text={'广喜，念<sup>注1</sup>鬼，曰："此教我先威<SUP>**注2**</sup>众耳。"'} />, host);
    expect([...host.querySelectorAll('sup')].map(note => note.textContent)).toEqual(['注1', '注2']);
    expect(host.querySelector('sup strong')?.textContent).toBe('注2');
    expect(host.textContent?.trim()).toBe('广喜，念注1鬼，曰："此教我先威注2众耳。"');
  });
  it('keeps HTML literal in code while sanitizing rendered HTML', () => {
    const host = document.createElement('div');
    render(<Markdown text={'`<sup>note</sup>`\n\n```\n<sup>code</sup>\n```\n\n<sup onclick="alert(1)">unsafe</sup>\n\n<sup><img src=x onerror=alert(1)>safe note</sup>'} />, host);
    expect([...host.querySelectorAll('code')].map(code => code.textContent)).toEqual(['<sup>note</sup>', '<sup>code</sup>\n']);
    expect([...host.querySelectorAll('sup')].map(note => note.textContent)).toEqual(['unsafe', 'safe note']);
    expect(host.querySelector('img, [onclick], [onerror]')).toBeNull();
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
    expect(host.querySelectorAll('th')[1]?.getAttribute('align')).toBe('center');
    expect(host.querySelector('blockquote')?.textContent).toContain('quoted');
    expect(host.querySelector('li li')?.textContent).toBe('two');
    expect(host.querySelector('em')?.textContent).toBe('it');
    expect(host.querySelector('del')?.textContent).toBe('no');
  });
  it('link clicks on nested labels and bare URLs open native tabs without navigating the pane', () => {
    const post = vi.fn();
    (window as unknown as { ipc: unknown }).ipc = { postMessage: post };
    const host = document.createElement('div');
    document.body.appendChild(host);
    render(<Markdown text="[**go**](https://a.example/?x=1&y=2) or https://b.example/" />, host);
    const click = new MouseEvent('click', { bubbles: true, cancelable: true, ctrlKey: true });
    host.querySelector('a strong')!.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    const sent = JSON.parse(post.mock.calls[0]?.[0] as string);
    expect(sent.cmd).toBe('nav.open');
    expect(sent.payload).toEqual({ url: 'https://a.example/?x=1&y=2', disposition: 'new_tab' });
    const middle = new MouseEvent('auxclick', { button: 1, bubbles: true, cancelable: true });
    host.querySelectorAll('a')[1]!.dispatchEvent(middle);
    expect(middle.defaultPrevented).toBe(true);
    expect(JSON.parse(post.mock.calls[1]?.[0] as string).payload).toEqual({ url: 'https://b.example/', disposition: 'new_tab' });
  });
  it('renders reference links, escaped punctuation and ordered list starts using standard Markdown', () => {
    const host = document.createElement('div');
    render(<Markdown text={'# Heading\n\n[Docs][ref] and \\*literal\\*\n\n3. third\n4. fourth\n\n[ref]: https://a.example/ "Docs"'} />, host);
    expect(host.querySelector('h1')?.textContent).toBe('Heading');
    expect(host.querySelector('a')?.getAttribute('href')).toBe('https://a.example/');
    expect(host.querySelector('a')?.title).toBe('Docs');
    expect(host.textContent).toContain('*literal*');
    expect(host.querySelector('em')).toBeNull();
    expect(host.querySelector('ol')?.start).toBe(3);
  });
  it('updates incomplete streamed code without interpreting its HTML', () => {
    const host = document.createElement('div');
    render(<Markdown text={'```html\n<sup>note'} />, host);
    expect(host.querySelector('pre code')?.textContent).toContain('<sup>note');
    expect(host.querySelector('sup')).toBeNull();
    render(<Markdown text={'```html\n<sup>note</sup>\n```\n\n**Done**'} />, host);
    expect(host.querySelector('pre code')?.textContent).toContain('<sup>note</sup>');
    expect(host.querySelector('strong')?.textContent).toBe('Done');
    expect(host.querySelector('sup')).toBeNull();
  });
});

describe('editable context', () => {
  it('renders the complete long selection without a shortened preview', () => {
    const text = 'Selected text with line breaks\n'.repeat(300) + 'END OF SELECTION';
    const previous = liveContext.value;
    const host = document.createElement('div');
    try {
      act(() => {
        liveContext.value = ctx({ selection: text });
        render(<ContextBar />, host);
      });
      expect(host.querySelector('.sel-quote')?.textContent).toBe(text);
    } finally {
      act(() => { render(null, host); liveContext.value = previous; });
    }
  });

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
  it('rejects timed-out requests and releases completed timers', async () => {
    vi.useFakeTimers();
    try {
      const b = createBridge(() => {}, 1, 25);
      const request = b.invoke('ai.status');
      const rejected = expect(request).rejects.toThrow('ai.status timed out');
      await vi.advanceTimersByTimeAsync(25);
      await rejected;
      const next = b.invoke('ai.status');
      b._reply({ id: 2, ok: true, data: 'done' });
      await expect(next).resolves.toBe('done');
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
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

  describe('visible tool configuration', () => {
    const search: ConfiguredTool = {
      id: 'web_search', name: 'web_search', title: 'Search', description: 'Configured search',
      source: 'http', target: 'POST http://localhost:3010/v1/search', enabled: true, read_only: true,
      parameters: { type: 'object', properties: { query: { type: 'string' } } },
      http: {
        id: 'web_search', title: 'Search', description: 'Configured search', method: 'POST',
        url: 'http://localhost:3010/v1/search', response: 'json', read_only: true,
        parameters: { type: 'object', properties: { query: { type: 'string' } } },
        body: { query: '{{query}}' },
      },
    };
    it('shows targets and sends explicit enable changes, reporting save failures', async () => {
      const host = document.createElement('div');
      let rejectSave!: (error: Error) => void;
      const change = vi.fn(() => new Promise<void>((_resolve, reject) => { rejectSave = reject; }));
      await act(() => render(<ToolsSettings tools={[search]} onChange={change} />, host));
      expect(host.textContent).toContain(search.target);
      await act(async () => {
        const checkbox = host.querySelector<HTMLInputElement>('input[aria-label="Enable web_search"]')!;
        checkbox.checked = false;
        checkbox.dispatchEvent(new Event('change', { bubbles: true }));
      });
      expect(change).toHaveBeenCalledWith('tools.enable', { id: 'web_search', enabled: false });
      expect(host.querySelector<HTMLInputElement>('input[aria-label="Enable web_search"]')!.checked).toBe(false);
      await act(async () => rejectSave(new Error('Cannot save configuration')));
      expect(host.querySelector<HTMLInputElement>('input[aria-label="Enable web_search"]')!.checked).toBe(true);
      expect(host.textContent).toContain('Cannot save configuration');
    });
    it('edits browser, local and MCP definitions without changing handler IDs', async () => {
      for (const [source, id] of [['browser', 'read_page'], ['local', 'fetch_url'], ['mcp', 'mcp_fixture__echo']] as const) {
        const host = document.createElement('div');
        const change = vi.fn().mockResolvedValue(undefined);
        const connection = vi.fn();
        const tool: ConfiguredTool = { ...search, id, name: id, source, http: undefined };
        await act(() => render(<ToolsSettings tools={[tool]} onChange={change} onEditMcp={connection} />, host));
        expect(host.textContent).toContain('tools field');
        await act(() => host.querySelector<HTMLButtonElement>(`[aria-label="Edit ${id}"]`)!.click());
        const field = (label: string) => [...host.querySelectorAll('label')]
          .find(field => field.textContent?.startsWith(label))!.querySelector<HTMLInputElement | HTMLTextAreaElement>('input,textarea')!;
        await act(() => {
          for (const [label, value] of [['Model tool name', 'my_tool'], ['Description', 'Edited description']]) {
            const input = field(label!);
            input.value = value!;
            input.dispatchEvent(new Event('input', { bubbles: true }));
          }
        });
        const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === text)!;
        if (source === 'mcp') {
          await act(() => button('Edit MCP connection').click());
          expect(connection).toHaveBeenCalledOnce();
        }
        await act(async () => button('Save').click());
        expect(change).toHaveBeenCalledWith('tools.definition.save', {
          id, definition: { name: 'my_tool', title: search.title, description: 'Edited description', parameters: search.parameters },
        });
      }
    });
    it('edits an existing search tool to MoonShot without silently changing its ID or endpoint', async () => {
      const host = document.createElement('div');
      const change = vi.fn().mockResolvedValue(undefined);
      await act(() => render(<ToolsSettings tools={[search]} onChange={change} />, host));
      await act(() => host.querySelector<HTMLButtonElement>('[aria-label="Edit web_search"]')!.click());
      const buttons = () => [...host.querySelectorAll<HTMLButtonElement>('button')];
      await act(() => buttons().find(button => button.textContent === 'MoonShot search template')!.click());
      const fields = [...host.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>('input,textarea')];
      expect(fields[0]!.value).toBe('web_search');
      const url = fields.find(field => field.getAttribute('placeholder') === 'http://host:3010/v1/search')!;
      expect(url.value).toBe('');
      await act(() => {
        url.value = 'http://localhost:3010/v1/search';
        url.dispatchEvent(new Event('input', { bubbles: true }));
      });
      await act(async () => buttons().find(button => button.textContent === 'Save')!.click());
      expect(change).toHaveBeenCalledWith('tools.http.save', expect.objectContaining({
        editing: true, enabled: true,
        tool: expect.objectContaining({
          id: 'web_search', method: 'POST', url: 'http://localhost:3010/v1/search',
          body: { mode: 'text', query: '{{query}}', offset: 0, limit: 10 },
        }),
      }));
    });
  });
});
