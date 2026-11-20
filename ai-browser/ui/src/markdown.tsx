import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { useMemo } from 'preact/hooks';
import { bridge } from './mcp';

const SAFE_URL = /^https?:\/\/[^\s<>"']+$/i;
const purifier = DOMPurify(window);
purifier.addHook('uponSanitizeAttribute', (_node, data) => {
  if (data.attrName === 'href' && !SAFE_URL.test(data.attrValue)) data.keepAttr = false;
});

function openLink(event: MouseEvent) {
  if (event.button !== 0 && event.button !== 1) return;
  const link = event.target instanceof Element ? event.target.closest('a[href]') : null;
  if (!link) return;
  event.preventDefault();
  const href = link.getAttribute('href') ?? '';
  if (!SAFE_URL.test(href)) return;
  bridge().invoke('shell_tab', { operation: 'open', url: href, background: false }).catch(() => undefined);
}

export function Markdown({ text }: { text: string }) {
  const html = useMemo(() => purifier.sanitize(
    marked.parse(text, { async: false, gfm: true, breaks: true }),
    {
      USE_PROFILES: { html: true },
      FORBID_TAGS: ['style', 'img', 'audio', 'video', 'source', 'form', 'button', 'textarea', 'select'],
      FORBID_ATTR: ['style', 'src', 'srcset', 'poster', 'action', 'formaction'],
    },
  ), [text]);
  return <div class="rich" onClick={openLink} onAuxClick={openLink} dangerouslySetInnerHTML={{ __html: html }} />;
}
