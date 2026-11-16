// Model text is untrusted. It is parsed into a block tree and rendered as elements and
// text nodes only: never `innerHTML`. Links are clickable only for http(s) URLs, and a
// click opens the page in a new tab through the shell (the pane itself never navigates).
import { bridge } from './ipc';

export type Inline =
  | { t: 'text' | 'code'; v: string }
  | { t: 'strong' | 'em' | 'del'; c: Inline[] }
  | { t: 'a'; href: string; c: Inline[] };

export interface ListBlock {
  t: 'ul' | 'ol';
  items: { text: Inline[]; sub: ListBlock[] }[];
}
export type Block =
  | { t: 'p'; lines: Inline[][] }
  | { t: 'h'; level: number; inline: Inline[] }
  | ListBlock
  | { t: 'pre'; v: string; lang: string }
  | { t: 'quote'; blocks: Block[] }
  | { t: 'hr' }
  | { t: 'table'; head: Inline[][]; align: string[]; rows: Inline[][][] };

const SAFE_URL = /^https?:\/\/[^\s<>"']+$/i;
const TOKEN = /(`[^`\n]+`)|\[((?:[^\]\n]|\[[^\]\n]*\])+)\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"\n]*")?\s*\)|(\*\*|__)(?=\S)([^\n]+?)(?<=\S)\4|~~(?=\S)([^\n]+?)(?<=\S)~~|(?<![\w*])\*(?=[^\s*])([^*\n]+?)(?<=[^\s*])\*(?![\w*])|(?<![\w_])_(?=[^\s_])([^_\n]+?)(?<=[^\s_])_(?![\w_])|<(https?:\/\/[^\s<>]+)>|(https?:\/\/[^\s<>()"'`]+[^\s<>()"'`.,;:!?*_~\]])/g;

export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let last = 0;
  // Per call: nested parseInline calls would otherwise share and reset lastIndex.
  const re = new RegExp(TOKEN.source, 'g');
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push({ t: 'text', v: text.slice(last, m.index) });
    if (m[1]) out.push({ t: 'code', v: m[1].slice(1, -1) });
    else if (m[2] !== undefined) {
      const href = m[3] ?? '';
      const c = parseInline(m[2]);
      // An unsafe scheme keeps its label as plain text.
      if (SAFE_URL.test(href)) out.push({ t: 'a', href, c });
      else out.push(...c);
    } else if (m[5] !== undefined) out.push({ t: 'strong', c: parseInline(m[5]) });
    else if (m[6] !== undefined) out.push({ t: 'del', c: parseInline(m[6]) });
    else if (m[7] !== undefined) out.push({ t: 'em', c: parseInline(m[7]) });
    else if (m[8] !== undefined) out.push({ t: 'em', c: parseInline(m[8]) });
    else {
      const url = m[9] ?? m[10] ?? '';
      out.push({ t: 'a', href: url, c: [{ t: 'text', v: url }] });
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ t: 'text', v: text.slice(last) });
  return out;
}

const ITEM = /^(\s*)(?:[-*+]|\d+[.)])\s+(.*)$/;
const FENCE = /^\s*(```|~~~)\s*([\w+-]*)/;
const HR = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/;
const SEP = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/;

function cells(line: string): string[] {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  return s.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, '|'));
}

const indentOf = (s: string) => s.replace(/\t/g, '    ').length - s.replace(/\t/g, '    ').trimStart().length;

export function parseMarkdown(text: string): Block[] {
  const lines = String(text).replace(/\r\n/g, '\n').split('\n');
  const blocks: Block[] = [];
  let para: Extract<Block, { t: 'p' }> | null = null;
  let stack: { indent: number; list: ListBlock }[] = [];
  const flush = () => {
    para = null;
    stack = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? '';
    const fence = FENCE.exec(line);
    if (fence) {
      flush();
      const body: string[] = [];
      for (i++; i < lines.length && !(lines[i] ?? '').trimStart().startsWith(fence[1] ?? '```'); i++) body.push(lines[i] ?? '');
      // An unterminated fence (a streaming answer) still shows what has arrived.
      blocks.push({ t: 'pre', v: body.join('\n'), lang: fence[2] ?? '' });
      continue;
    }
    const next = lines[i + 1];
    if (line.includes('|') && next !== undefined && next.includes('-') && SEP.test(next) && cells(line).length > 1) {
      flush();
      const align = cells(next).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : ''));
      const rows: Inline[][][] = [];
      let j = i + 2;
      for (; j < lines.length && (lines[j] ?? '').includes('|') && (lines[j] ?? '').trim(); j++) rows.push(cells(lines[j] ?? '').map(parseInline));
      blocks.push({ t: 'table', head: cells(line).map(parseInline), align, rows });
      i = j - 1;
      continue;
    }
    if (/^\s{0,3}>/.test(line)) {
      flush();
      const body: string[] = [];
      for (; i < lines.length && /^\s{0,3}>/.test(lines[i] ?? ''); i++) body.push((lines[i] ?? '').replace(/^\s{0,3}>\s?/, ''));
      i--;
      blocks.push({ t: 'quote', blocks: parseMarkdown(body.join('\n')) });
      continue;
    }
    if (HR.test(line)) {
      flush();
      blocks.push({ t: 'hr' });
      continue;
    }
    const head = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (head) {
      flush();
      blocks.push({ t: 'h', level: (head[1] ?? '#').length, inline: parseInline(head[2] ?? '') });
      continue;
    }
    const item = ITEM.exec(line);
    if (item) {
      para = null;
      const indent = indentOf(line);
      const kind = /^\s*\d/.test(line) ? 'ol' : 'ul';
      while (stack.length > 1 && indent < (stack[stack.length - 1]?.indent ?? 0)) stack.pop();
      let top = stack[stack.length - 1];
      if (top && indent > top.indent) {
        const owner = top.list.items[top.list.items.length - 1];
        const sub: ListBlock = { t: kind, items: [] };
        owner?.sub.push(sub);
        stack.push((top = { indent, list: sub }));
      } else if (!top || top.list.t !== kind) {
        const list: ListBlock = { t: kind, items: [] };
        if (stack.length > 1) stack[stack.length - 2]?.list.items[stack[stack.length - 2]!.list.items.length - 1]?.sub.push(list);
        else blocks.push(list);
        if (stack.length) stack.pop();
        stack.push((top = { indent, list }));
      }
      top.list.items.push({ text: parseInline(item[2] ?? ''), sub: [] });
      continue;
    }
    if (!line.trim()) {
      para = null;
      // A blank line between list items keeps the list; anything else ends it.
      if (!/^\s*(?:[-*+]|\d+[.)])\s/.test(lines[i + 1] ?? '')) stack = [];
      continue;
    }
    const lastItem = stack.length ? stack[stack.length - 1]?.list.items.at(-1) : undefined;
    if (lastItem && /^\s+/.test(line)) {
      lastItem.text.push({ t: 'text', v: ' ' }, ...parseInline(line.trim()));
      continue;
    }
    stack = [];
    if (!para) {
      para = { t: 'p', lines: [] };
      blocks.push(para);
    }
    para.lines.push(parseInline(line));
  }
  return blocks;
}

function open(e: MouseEvent, href: string) {
  e.preventDefault();
  bridge()
    .invoke('nav.open', { url: href, disposition: 'new_tab' })
    .catch(() => undefined);
}

function Inlines({ parts }: { parts: Inline[] }) {
  return (
    <>
      {parts.map((p, i) => {
        switch (p.t) {
          case 'code':
            return <code key={i}>{p.v}</code>;
          case 'strong':
            return (
              <strong key={i}>
                <Inlines parts={p.c} />
              </strong>
            );
          case 'em':
            return (
              <em key={i}>
                <Inlines parts={p.c} />
              </em>
            );
          case 'del':
            return (
              <del key={i}>
                <Inlines parts={p.c} />
              </del>
            );
          case 'a':
            return (
              <a key={i} href={p.href} title={p.href} onClick={(e) => open(e as unknown as MouseEvent, p.href)}>
                <Inlines parts={p.c} />
              </a>
            );
          default:
            return p.v;
        }
      })}
    </>
  );
}

function List({ b }: { b: ListBlock }) {
  const Tag = b.t;
  return (
    <Tag>
      {b.items.map((it, j) => (
        <li key={j}>
          <Inlines parts={it.text} />
          {it.sub.map((s, k) => (
            <List key={k} b={s} />
          ))}
        </li>
      ))}
    </Tag>
  );
}

function Blocks({ blocks }: { blocks: Block[] }) {
  return (
    <>
      {blocks.map((b, i) => {
        switch (b.t) {
          case 'pre':
            return (
              <pre key={i}>
                <code>{b.v}</code>
              </pre>
            );
          case 'h':
            return (
              <p key={i} class={'rich-h rich-h' + Math.min(b.level, 4)} role="heading" aria-level={b.level}>
                <Inlines parts={b.inline} />
              </p>
            );
          case 'ul':
          case 'ol':
            return <List key={i} b={b} />;
          case 'quote':
            return (
              <blockquote key={i}>
                <Blocks blocks={b.blocks} />
              </blockquote>
            );
          case 'hr':
            return <hr key={i} />;
          case 'table':
            return (
              <div class="rich-table" key={i}>
                <table>
                  <thead>
                    <tr>
                      {b.head.map((c, j) => (
                        <th key={j} style={b.align[j] ? { textAlign: b.align[j] } : undefined}>
                          <Inlines parts={c} />
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {b.rows.map((r, j) => (
                      <tr key={j}>
                        {b.head.map((_, k) => (
                          <td key={k} style={b.align[k] ? { textAlign: b.align[k] } : undefined}>
                            <Inlines parts={r[k] ?? []} />
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          default:
            return (
              <p key={i}>
                {b.lines.map((l, j) => (
                  <>
                    {j > 0 ? <br /> : null}
                    <Inlines parts={l} />
                  </>
                ))}
              </p>
            );
        }
      })}
    </>
  );
}

export function Markdown({ text }: { text: string }) {
  return (
    <div class="rich">
      <Blocks blocks={parseMarkdown(text)} />
    </div>
  );
}
