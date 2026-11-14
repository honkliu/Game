// Overlay surface: the command palette (§3.4) and the history panel (§4.5).
//
// Same bridge shape as app.js. The overlay is only focusable when the host has
// sized it to something larger than 0x0; on close the host restores page focus.

(() => {
  'use strict';

  const pending = new Map();
  const listeners = new Map();
  let nextId = 1;

  function invoke(cmd, payload) {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ id, cmd, payload: payload || {} }));
    });
  }

  function on(event, fn) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(fn);
  }

  window.__native = {
    invoke,
    on,
    _dispatch(env) {
      const set = listeners.get(env && env.event);
      if (!set) return;
      for (const fn of set) fn(env.payload);
    },
    _reply(reply) {
      const entry = pending.get(reply.id);
      if (!entry) return;
      pending.delete(reply.id);
      if (reply.ok) entry.resolve(reply.data);
      else entry.reject(new Error(reply.error || 'command failed'));
    },
  };

  const root = document.getElementById('root');

  // ── palette ──────────────────────────────────────────────────────────
  let paletteRows = [];
  let paletteSel = 0;
  let paletteMode = null;

  function openPalette(prefill) {
    root.className = 'overlay-root';
    root.innerHTML = `
      <div class="palette" role="dialog" aria-modal="true" aria-label="Command palette">
        <input id="pal-input" class="palette-input" type="text" spellcheck="false"
               role="combobox" aria-expanded="true" aria-controls="pal-list"
               aria-autocomplete="list" placeholder="Search tabs, history, or run a command" />
        <ul id="pal-list" class="palette-list" role="listbox" aria-label="Results"></ul>
        <div class="palette-footer"><span>↑↓ navigate · Tab complete · Enter run · Esc close</span><span id="pal-mode"></span></div>
      </div>`;
    const input = document.getElementById('pal-input');
    input.value = prefill || '> ';
    input.focus();
    input.select();
    input.addEventListener('input', () => queryPalette(input.value));
    input.addEventListener('keydown', paletteKey);
    queryPalette(input.value);
  }

  function closeOverlay() {
    root.className = 'overlay-root is-transparent';
    root.innerHTML = '';
    paletteRows = [];
  }

  let seq = 0;
  async function queryPalette(text) {
    const mine = ++seq;
    const res = await invoke('omnibox.query', { text, limit: 40 }).catch(() => null);
    if (mine !== seq || !res) return;
    paletteRows = res.rows || [];
    paletteSel = 0;
    const mode = document.getElementById('pal-mode');
    if (mode) mode.textContent = res.chip || '';
    renderPalette();
  }

  function renderPalette() {
    const list = document.getElementById('pal-list');
    if (!list) return;
    list.innerHTML = '';
    const frag = document.createDocumentFragment();
    paletteRows.forEach((r, i) => {
      const li = document.createElement('li');
      li.className = 'palette-row';
      li.setAttribute('role', 'option');
      li.id = `pal-row-${i}`;
      li.setAttribute('aria-selected', String(i === paletteSel));
      const glyph = r.kind === 'search' ? '🔍' : r.kind === 'command' ? '›'
        : r.kind === 'url' ? '↗' : r.kind === 'tab' ? '▭' : '◷';
      li.innerHTML = `<span class="dropdown-glyph">${glyph}</span>
        <span class="dropdown-text"><span class="dropdown-title"></span><span class="dropdown-sub"></span></span>
        <span class="dropdown-hint"></span>`;
      li.querySelector('.dropdown-title').textContent = r.title || r.url || '';
      li.querySelector('.dropdown-sub').textContent = r.hint || r.url || '';
      li.querySelector('.dropdown-hint').textContent = r.kind === 'command' ? 'Enter' : r.kind === 'tab' ? 'switch' : '';
      li.addEventListener('mousedown', (e) => { e.preventDefault(); paletteSel = i; runPaletteRow(); });
      frag.appendChild(li);
    });
    if (!paletteRows.length) {
      const empty = document.createElement('li');
      empty.className = 'overlay-empty';
      empty.textContent = 'No matches';
      frag.appendChild(empty);
    }
    list.appendChild(frag);
    const input = document.getElementById('pal-input');
    if (input) input.setAttribute('aria-activedescendant', `pal-row-${paletteSel}`);
    const sel = list.querySelector('[aria-selected="true"]');
    if (sel) sel.scrollIntoView({ block: 'nearest' });
  }

  function paletteKey(e) {
    if (e.key === 'ArrowDown') { e.preventDefault(); if (paletteRows.length) { paletteSel = (paletteSel + 1) % paletteRows.length; renderPalette(); } }
    else if (e.key === 'ArrowUp') { e.preventDefault(); if (paletteRows.length) { paletteSel = (paletteSel - 1 + paletteRows.length) % paletteRows.length; renderPalette(); } }
    else if (e.key === 'Tab') {
      if (paletteRows.length) { e.preventDefault(); const r = paletteRows[paletteSel]; e.target.value = r.url || r.title || e.target.value; queryPalette(e.target.value); }
    } else if (e.key === 'Enter') { e.preventDefault(); runPaletteRow(); }
    else if (e.key === 'Escape') { e.preventDefault(); invoke('shell.close_top_overlay', {}); }
  }

  function runPaletteRow() {
    const r = paletteRows[paletteSel];
    const text = document.getElementById('pal-input').value;
    if (!r) { if (text.trim()) invoke('omnibox.activate', { text }); return; }
    if (r.kind === 'command') invoke('omnibox.activate', { text, verb: r.verb });
    else if (r.kind === 'tab') invoke('tab.select', { tab_id: r.tab_id });
    else if (r.kind === 'find') invoke('omnibox.activate', { text });
    else invoke('omnibox.activate', { text, url: r.url });
  }

  // ── history panel ────────────────────────────────────────────────────
  let historyRows = [];
  let historySel = 0;

  function openHistory() {
    root.className = 'overlay-root';
    root.innerHTML = `
      <div class="history-overlay" role="dialog" aria-modal="true" aria-label="History">
        <div class="history-head">
          <h2 class="panel-title">History</h2>
          <input id="hist-search" class="history-search" type="text" spellcheck="false"
                 placeholder="Search — site: before:YYYY-MM-DD after: type:typed"
                 aria-label="Search history" />
          <button id="hist-clear" class="text-btn">Clear history</button>
          <button id="hist-close" class="icon-btn" aria-label="Close history">
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 4 8 8m0-8-8 8" stroke="currentColor" stroke-width="1.6" fill="none" stroke-linecap="round"/></svg>
          </button>
        </div>
        <div id="hist-body" class="history-body" role="listbox" aria-label="History entries"></div>
      </div>`;
    const search = document.getElementById('hist-search');
    search.focus();
    search.addEventListener('input', () => queryHistory(search.value));
    search.addEventListener('keydown', historyKey);
    document.getElementById('hist-close').addEventListener('click', () => invoke('shell.close_top_overlay', {}));
    document.getElementById('hist-clear').addEventListener('click', confirmClearAll);
    queryHistory('');
  }

  let histSeq = 0;
  async function queryHistory(tokens) {
    const mine = ++histSeq;
    const res = await invoke('history.query', { tokens, limit: 300 }).catch(() => null);
    if (mine !== histSeq || !res) return;
    historyRows = res.rows || [];
    historySel = 0;
    renderHistory();
  }

  function dayKey(secs) {
    const d = new Date(secs * 1000);
    const today = new Date();
    const yesterday = new Date(today.getTime() - 86400000);
    const same = (a, b) => a.toDateString() === b.toDateString();
    if (same(d, today)) return 'Today';
    if (same(d, yesterday)) return 'Yesterday';
    return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  }

  function hhmm(secs) {
    const d = new Date(secs * 1000);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  }

  function renderHistory() {
    const body = document.getElementById('hist-body');
    if (!body) return;
    body.innerHTML = '';
    if (!historyRows.length) {
      const p = document.createElement('p');
      p.className = 'overlay-empty';
      p.textContent = 'No history entries match.';
      body.appendChild(p);
      return;
    }
    let lastDay = null;
    const frag = document.createDocumentFragment();
    historyRows.forEach((r, i) => {
      const day = dayKey(r.visited_at);
      if (day !== lastDay) {
        lastDay = day;
        const h = document.createElement('div');
        h.className = 'history-day';
        h.textContent = day;
        frag.appendChild(h);
      }
      const row = document.createElement('div');
      row.className = 'history-row';
      row.setAttribute('role', 'option');
      row.id = `hist-row-${i}`;
      row.setAttribute('aria-selected', String(i === historySel));
      row.dataset.i = String(i);
      row.innerHTML = `<span class="history-time"></span><span class="history-title"></span>
        <span class="history-host"></span>
        <button class="history-more" aria-label="More actions">⋯</button>`;
      row.querySelector('.history-time').textContent = hhmm(r.visited_at);
      row.querySelector('.history-title').textContent = r.title;
      row.querySelector('.history-host').textContent = r.host;
      row.querySelector('.history-more').addEventListener('click', (e) => {
        e.stopPropagation();
        historyMoreMenu(r, e.clientX, e.clientY, i);
      });
      row.addEventListener('click', () => openEntry(r, false));
      row.addEventListener('contextmenu', (e) => { e.preventDefault(); historyMoreMenu(r, e.clientX, e.clientY, i); });
      frag.appendChild(row);
    });
    body.appendChild(frag);
  }

  function historyKey(e) {
    const rows = [...document.querySelectorAll('.history-row')];
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (!rows.length) return;
      historySel = Math.min(rows.length - 1, historySel + 1);
      rows[historySel].focus();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      historySel = Math.max(0, historySel - 1);
      rows[historySel].focus();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      invoke('shell.close_top_overlay', {});
    } else if (e.key === 'Enter') {
      if (document.activeElement && document.activeElement.classList.contains('history-row')) {
        e.preventDefault();
        const i = Number(document.activeElement.dataset.i);
        if (historyRows[i]) openEntry(historyRows[i], e.ctrlKey || e.shiftKey);
      }
    } else if (e.key === 'Delete') {
      const active = document.activeElement;
      if (active && active.classList.contains('history-row')) {
        e.preventDefault();
        const i = Number(active.dataset.i);
        if (historyRows[i]) forgetEntry(historyRows[i]);
      }
    }
  }

  function openEntry(r, newTab) {
    invoke('omnibox.activate', {
      text: r.url,
      url: r.url,
      disposition: newTab ? 'new_tab_background' : 'current',
    });
  }

  async function forgetEntry(r) {
    await invoke('history.forget', { entry_ids: [r.id] }).catch(() => {});
    const search = document.getElementById('hist-search');
    queryHistory(search ? search.value : '');
  }

  function confirmClearAll() {
    const body = document.getElementById('hist-body');
    if (!body) return;
    body.innerHTML = `
      <div class="overlay-empty" role="alert">
        <p>Clear all browsing history? This cannot be undone.</p>
        <p><button id="clear-yes" class="chip-btn is-danger">Clear all history</button>
        <button id="clear-no" class="chip-btn">Cancel</button></p>
      </div>`;
    document.getElementById('clear-yes').addEventListener('click', () => {
      invoke('history.clear', { scope: 'all' }).then(() => queryHistory(''));
    });
    document.getElementById('clear-no').addEventListener('click', () => queryHistory(''));
    document.getElementById('clear-yes').focus();
  }

  function historyMoreMenu(r, x, y, _i) {
    const menu = document.createElement('div');
    menu.className = 'menu';
    menu.setAttribute('role', 'menu');
    menu.style.position = 'fixed';
    menu.style.left = `${Math.min(x, window.innerWidth - 220)}px`;
    menu.style.top = `${Math.min(y, window.innerHeight - 160)}px`;
    const items = [
      { label: 'Open in new tab', run: () => openEntry(r, true) },
      { label: 'Copy URL', run: () => navigator.clipboard.writeText(r.url).catch(() => {}) },
      { sep: true },
      { label: 'Forget this page', danger: true, run: () => forgetEntry(r) },
      { label: 'Forget this site', danger: true, run: () => invoke('history.forget', { site: r.host }).then(() => queryHistory(document.getElementById('hist-search').value)) },
    ];
    for (const it of items) {
      if (it.sep) {
        const s = document.createElement('div');
        s.className = 'menu-sep';
        menu.appendChild(s);
        continue;
      }
      const b = document.createElement('button');
      b.className = 'menu-item' + (it.danger ? ' is-danger' : '');
      b.setAttribute('role', 'menuitem');
      b.textContent = it.label;
      b.addEventListener('click', () => { menu.remove(); it.run(); });
      menu.appendChild(b);
    }
    document.body.appendChild(menu);
    const first = menu.querySelector('.menu-item');
    if (first) first.focus();
    const dismiss = (e) => {
      if (!menu.contains(e.target)) { menu.remove(); document.removeEventListener('mousedown', dismiss, true); }
    };
    document.addEventListener('mousedown', dismiss, true);
  }

  // ── host events ──────────────────────────────────────────────────────
  on('overlay.state', (p) => {
    if (!p || !p.open) { closeOverlay(); return; }
    if (p.kind === 'palette') openPalette('> ');
    else if (p.kind === 'full_page') openHistory();
  });

  on('shell.focus_palette', (p) => openPalette((p && p.prefill) || '> '));

  on('shell.shortcut', (p) => {
    if (p && p.name === 'escape') invoke('shell.close_top_overlay', {});
  });

  on('state.settings', (p) => {
    const s = p || {};
    let theme = s.theme || 'system';
    if (theme === 'system') theme = s.system_dark ? 'dark' : 'light';
    document.documentElement.dataset.theme = theme;
  });

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') invoke('shell.close_top_overlay', {});
  });
})();