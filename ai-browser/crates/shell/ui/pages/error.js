// Error page (§4.7). Data arrives in the fragment; no network of any kind.
(() => {
  'use strict';

  function params() {
    const out = {};
    try {
      for (const [k, v] of new URLSearchParams(location.hash.replace(/^#/, ''))) out[k] = v;
    } catch { /* ignore */ }
    return out;
  }

  const p = params();
  const url = p.url || '';
  const code = p.code || 'ERR_UNKNOWN';
  const navId = p.nav || '0';

  document.getElementById('code').textContent = code;
  const urlField = document.getElementById('url');
  urlField.textContent = url;
  urlField.title = url;

  if (/NAME_NOT_RESOLVED|DNS/.test(code)) {
    const hint = document.getElementById('hint');
    hint.hidden = false;
    hint.textContent = 'Check that the domain exists; no DNS fallback search is performed.';
  }

  // Retry reloads exactly this URL. The page is in a webview with no bridge,
  // so this is a plain navigation — the host records it like any other.
  document.getElementById('retry').addEventListener('click', () => {
    location.href = url || 'asset://shell/pages/newtab.html';
  });

  document.getElementById('copy').addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(url);
      document.getElementById('copy').textContent = 'Copied';
    } catch {
      // Clipboard can be denied without a user gesture in some builds; fall
      // back to selecting the text so the user can copy manually.
      const range = document.createRange();
      range.selectNodeContents(urlField);
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
    }
  });

  document.getElementById('newtab').addEventListener('click', () => {
    // Ctrl+click-equivalent is impossible without the bridge; opening the
    // address in this tab is honest and reversible with Alt+Left.
    location.href = url;
  });

  const details = document.getElementById('details');
  const toggle = document.getElementById('details-toggle');
  toggle.addEventListener('click', () => {
    details.hidden = !details.hidden;
    toggle.setAttribute('aria-expanded', String(!details.hidden));
    toggle.textContent = details.hidden ? 'Details ▾' : 'Details ▴';
  });

  document.getElementById('details-body').textContent =
    `code: ${code}\nurl: ${url}\nnavigation_id: ${navId}\ntimestamp: ${new Date().toISOString()}\n`;

  document.title = `Can't reach ${url}`;
})();