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
      const timer = setTimeout(() => {
        if (pending.delete(id)) reject(new Error(cmd + ' timed out'));
      }, 150000);
      pending.set(id, { resolve, reject, timer });
      try {
        window.ipc.postMessage(JSON.stringify({ id, cmd, payload: payload || {} }));
      } catch (e) {
        pending.delete(id);
        clearTimeout(timer);
        reject(e);
      }
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
      clearTimeout(entry.timer);
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

  // ── ask (AI composer) ────────────────────────────────────────────────
  // The host owns every decision here: it captures the page, picks the provider,
  // and decides whether a citation may still be scrolled to. This surface renders
  // what it is told and displays the host's refusal when it rejects one, so there
  // is no path from this file to the page other than asking the host to scroll.
  //
  // Every page- or provider-derived string (question, answer text, quotes, model
  // id, failure detail) goes in through textContent. Nothing built from page text
  // is ever assigned to innerHTML.
  const MAX_ASK_TURNS = 20;
  const askTurns = new Map(); // turn_id → record; survives close so reopen shows them
  let askBusy = null;         // turn_id whose final `ai.answer` has not arrived

  let askStatusInfo = null;   // last `ai.status` payload: context header, models, switches
  let askCard = false;        // inline card next to a selection rather than the composer

  function openAsk(card) {
    const existing = document.getElementById('ask-input');
    if (existing && !card) { existing.focus(); return; } // the shortcut re-focuses, never stacks
    askCard = !!card;
    root.className = 'overlay-root';
    root.innerHTML = `
      <div class="ask${askCard ? ' is-card' : ''}" role="dialog" aria-modal="true" aria-label="Ask about this page">
        <div class="ask-top"><strong>✦ AI</strong><span id="ask-ctx" class="ask-ctx"></span><button id="ask-new" class="chip-btn" type="button" title="Start a new conversation">New chat</button><button id="ask-gear" class="chip-btn" type="button" title="Model endpoint settings">⚙</button></div>
        <div id="ask-set" class="ask-set" hidden>
          <label>Endpoint (OpenAI-compatible)<input id="set-url" type="text" spellcheck="false" placeholder="http://host:8001/v1"/></label>
          <label>Model name<input id="set-model" type="text" spellcheck="false" placeholder="e.g. QWen"/></label>
          <label>API key<input id="set-key" type="password" placeholder="leave empty to keep the saved key"/></label>
          <div><button id="set-save" class="chip-btn is-accent" type="button">Save</button> <span id="set-msg" class="ask-status"></span></div>
        </div>
        <div id="ask-quick" class="ask-quick"></div>
        <div class="ask-head">
          <textarea id="ask-input" class="ask-input" rows="2" spellcheck="false"
            placeholder="${askCard ? 'Follow up about the selection' : 'Ask a question about this page'}"></textarea>
          <button id="ask-send" class="chip-btn is-accent" type="button">Ask</button>
          <button id="ask-stop" class="chip-btn" type="button" hidden>Stop</button>
        </div>
        <div class="ask-opts">
          <select id="ask-intent" aria-label="Task">
            <option value="ask">Ask</option><option value="summarize">Summarize</option>
            <option value="explain">Explain</option><option value="verify">Verify</option>
            <option value="translate">Translate</option><option value="rewrite">Rewrite</option>
            <option value="act">Act (may open tabs)</option>
          </select>
          <select id="ask-model" aria-label="Model"><option value="">Auto route</option></select>
          <label class="ask-opt"><input id="ask-sel" type="checkbox"${askCard ? ' checked' : ''}/> selection</label>
          <label class="ask-opt"><input id="ask-local" type="checkbox"/> local only</label>
        </div>
        <div id="ask-tabs" class="ask-tabs" aria-label="Also read these tabs"></div>
        <div id="ask-cands" class="ask-cands"></div>
        <div id="ask-status" class="ask-status" role="status"></div>
        <div id="ask-log" class="ask-log"></div>
        <div class="palette-footer"><span>Enter ask · Shift+Enter newline · Esc close · click a citation to scroll there</span><span id="ask-source"></span></div>
      </div>`;
    const input = document.getElementById('ask-input');
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submitAsk(); }
    });
    document.getElementById('ask-send').addEventListener('click', () => submitAsk());
    wireSidePane();
    document.getElementById('ask-stop').addEventListener('click', stopAsk);
    for (const t of askTurns.values()) paintTurn(t);
    paintContext();
    refreshStatus();
    invoke('ai.models', {}).catch(() => {});
    setBusy(askBusy !== null);
    if (!askTurns.size) {
      const hint = document.createElement('p');
      hint.className = 'overlay-empty';
      hint.textContent = 'Ask me anything. I can use the page you are reading when it helps, and I cite the passage when I do.';
      document.getElementById('ask-log').appendChild(hint);
    }
    setAskStatus('');
    setAskSource();
    input.disabled = askBusy !== null;
    input.focus();
  }

  function setBusy(busy) {
    const stop = document.getElementById('ask-stop');
    const send = document.getElementById('ask-send');
    const input = document.getElementById('ask-input');
    if (stop) stop.hidden = !busy;
    if (send) send.hidden = busy;
    if (input) input.disabled = busy;
  }

  async function stopAsk() {
    const stop = document.getElementById('ask-stop');
    if (stop) stop.disabled = true;
    try { await invoke('ai.stop', { turn_id: askBusy }); } catch (e) {
      setAskStatus(clip((e && e.message) || 'nothing to stop', 200), 'warn');
    }
    if (stop) stop.disabled = false;
  }

  async function refreshStatus() {
    try { askStatusInfo = await invoke('ai.status', {}); } catch (e) { return; }
    paintContext();
  }

  // The context header: which page the assistant is about to read, how much of it
  // it can see, and what is selected. All values are shown through textContent.
  function paintContext() {
    const el = document.getElementById('ask-ctx');
    const s = askStatusInfo;
    if (!el || !s) return;
    el.textContent = '';
    const bits = [];
    if (!s.enabled) bits.push('AI unavailable: ' + (s.reason || 'off'));
    else if (!s.askable) bits.push('This page cannot be read by the assistant');
    else {
      bits.push((s.tab && s.tab.host) || 'this page');
      if (s.context) {
        const cov = s.context.coverage || {};
        const gaps = (cov.inaccessible_frames || []).length + (cov.unloaded_regions || []).length + (cov.unsupported_content || []).length;
        const pct = typeof cov.estimated_completeness === 'number' ? Math.round(cov.estimated_completeness * 100) : null;
        if (cov.status && cov.status !== 'complete_enough' || gaps) bits.push('△ partial page coverage' + (pct !== null ? ' (' + pct + '%)' : ''));
        if (cov.has_untranscribed_media) bits.push('△ media has no transcript');
        bits.push(s.context.blocks + ' passages');
      } else bits.push('page read on ask');
      if (s.selection) bits.push(`${s.selection.chars} chars selected`);
      bits.push(s.cloud_enabled ? 'cloud models allowed' : 'local only');
      if (s.tab && s.tab.ai_managed) bits.push('AI-managed tab');
    }
    el.textContent = bits.join(' · ');
    const sel = document.getElementById('ask-sel');
    if (sel) sel.disabled = !s.selection;
    paintTabChoices(s);
  }

  // Extra context is opt-in and visible: each other readable tab is a checkbox, and the
  // ticked ones travel with the question as `tabs`.
  const extraTabs = new Set();
  function paintTabChoices(s) {
    const box = document.getElementById('ask-tabs');
    if (!box) return;
    box.textContent = '';
    const others = (s.multi_tab && s.other_tabs) || [];
    const live = new Set(others.map((t) => t.tab_id));
    for (const id of [...extraTabs]) if (!live.has(id)) extraTabs.delete(id);
    if (!others.length) return;
    const lead = document.createElement('span');
    lead.className = 'ask-tabs-lead';
    lead.textContent = 'Also read:';
    box.appendChild(lead);
    for (const t of others) {
      const label = document.createElement('label');
      label.className = 'ask-opt ask-tab-opt';
      const cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.dataset.tab = String(t.tab_id);
      cb.checked = extraTabs.has(t.tab_id);
      cb.addEventListener('change', () => { if (cb.checked) extraTabs.add(t.tab_id); else extraTabs.delete(t.tab_id); });
      label.appendChild(cb);
      label.appendChild(document.createTextNode(' ' + clip(t.title || t.host || 'tab', 28)));
      box.appendChild(label);
    }
  }

  function paintModels(models) {
    const pick = document.getElementById('ask-model');
    if (!pick) return;
    const keep = pick.value;
    pick.textContent = '';
    const auto = document.createElement('option');
    auto.value = ''; auto.textContent = 'Auto route';
    pick.appendChild(auto);
    for (const m of models || []) {
      const id = `${m.provider_id}/${m.model_id}`;
      const o = document.createElement('option');
      o.value = id;
      o.textContent = `${m.display_name || id}${m.local ? ' (local)' : ' (cloud)'}`;
      pick.appendChild(o);
    }
    pick.value = keep;
  }

  function setAskStatus(text, kind) {
    const el = document.getElementById('ask-status');
    if (!el) return;
    el.textContent = text || '';
    el.className = 'ask-status' + (kind ? ' is-' + kind : '');
  }

  // What produced this answer, said plainly: the offline stub must never read
  // like a model, and a model answer must say the text came over the network.
  function provenance(t) {
    if (t.status === 'pending') return '';
    const ms = t.elapsedMs ? ` · ${(t.elapsedMs / 1000).toFixed(1)} s` : '';
    if (t.provider === 'http') return `${t.model || 'remote model'} · over the network${ms}`;
    if (t.model && t.model.startsWith('offline')) return `${t.model} · extractive, no network request${ms}`;
    if (t.model) return `${t.model} · on this device${ms}`;
    return `offline extract · no network request${ms}`;
  }

  function setAskSource() {
    const el = document.getElementById('ask-source');
    if (!el) return;
    let last = null;
    for (const t of askTurns.values()) last = t;
    el.textContent = last ? provenance(last) : '';
  }

  function failureLine(t) {
    const f = t.failure;
    if (!f || !f.code) return '';
    return `${f.kind || 'failure'}/${f.code}${f.detail ? ' — ' + f.detail : ''}`;
  }

  function clip(text, max) {
    const s = String(text);
    return s.length > max ? `${s.slice(0, max - 1)}…` : s;
  }

  function citationChip(t, c) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'ask-chip' + (t.stale ? ' is-stale' : '');
    b.textContent = String(c.ordinal);
    b.title = c.quote
      ? `“${clip(c.quote, 220)}” — click to scroll the page there`
      : 'Click to scroll the page there';
    b.disabled = !!t.stale;
    b.addEventListener('click', () => scrollCitation(t, c, b));
    return b;
  }

  async function scrollCitation(t, c, chip) {
    setAskStatus('');
    try {
      const res = await invoke('ai.citation.scroll', { turn_id: t.turnId, citation_id: c.citation_id });
      if (res && res.scrolled) setAskStatus(`Scrolled to citation ${c.ordinal}.`);
    } catch (e) {
      // The host refuses rather than scroll a document this citation no longer
      // describes. Show which refusal it was; do not scroll anything client-side.
      const msg = clip((e && e.message) || 'citation refused', 200);
      chip.classList.add('is-stale');
      chip.title = msg;
      setAskStatus(msg, 'error');
    }
  }

  // Model text is untrusted: it is turned into elements with textContent only, never
  // innerHTML, and links are not made clickable.
  function inlineRich(parent, text) {
    const re = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/g;
    let last = 0;
    let m;
    while ((m = re.exec(text))) {
      if (m.index > last) parent.appendChild(document.createTextNode(text.slice(last, m.index)));
      const tok = m[0];
      const el = document.createElement(tok.startsWith('`') ? 'code' : 'strong');
      el.textContent = tok.startsWith('`') ? tok.slice(1, -1) : tok.slice(2, -2);
      parent.appendChild(el);
      last = m.index + tok.length;
    }
    if (last < text.length) parent.appendChild(document.createTextNode(text.slice(last)));
  }

  function renderRich(root, text) {
    root.textContent = '';
    const lines = String(text).replace(/\r\n/g, '\n').split('\n');
    let list = null;
    let para = null;
    let fence = null;
    const flush = () => { list = null; para = null; };
    for (const line of lines) {
      if (fence) {
        if (/^\s*```/.test(line)) { fence = null; continue; }
        fence.textContent += (fence.textContent ? '\n' : '') + line;
        continue;
      }
      if (/^\s*```/.test(line)) {
        flush();
        const pre = document.createElement('pre');
        fence = document.createElement('code');
        pre.appendChild(fence);
        root.appendChild(pre);
        continue;
      }
      const item = /^\s*(?:[-*]|\d+[.)])\s+(.*)$/.exec(line);
      const head = /^\s{0,3}#{1,4}\s+(.*)$/.exec(line);
      if (item) {
        const ordered = /^\s*\d/.test(line);
        if (!list || list.tagName !== (ordered ? 'OL' : 'UL')) {
          list = document.createElement(ordered ? 'ol' : 'ul');
          root.appendChild(list);
        }
        para = null;
        const li = document.createElement('li');
        inlineRich(li, item[1]);
        list.appendChild(li);
      } else if (head) {
        flush();
        const h = document.createElement('p');
        h.className = 'rich-h';
        inlineRich(h, head[1]);
        root.appendChild(h);
      } else if (!line.trim()) {
        flush();
      } else {
        list = null;
        if (!para) { para = document.createElement('p'); root.appendChild(para); }
        else para.appendChild(document.createElement('br'));
        inlineRich(para, line);
      }
    }
  }

  function turnNode(t) {
    const card = document.createElement('div');
    card.className = 'ask-turn' + (t.stale ? ' is-stale' : '');

    const q = document.createElement('div');
    q.className = 'ask-question';
    q.textContent = t.question || '';
    card.appendChild(q);

    const a = document.createElement('div');
    a.className = 'ask-answer';
    if (t.status === 'pending') {
      a.classList.add('is-pending');
      if (t.streamed) renderRich(a, t.streamed); else a.textContent = 'Thinking…';
    } else if (t.status === 'error') {
      a.classList.add('is-error');
      a.textContent = t.text || failureLine(t) || 'No answer.';
    } else {
      if (t.text) renderRich(a, t.text); else a.textContent = 'The provider returned no text.';
    }
    card.appendChild(a);

    if (t.citations && t.citations.length) {
      const row = document.createElement('div');
      row.className = 'ask-citations';
      for (const c of t.citations) row.appendChild(citationChip(t, c));
      card.appendChild(row);
    }

    const meta = document.createElement('div');
    meta.className = 'ask-meta';
    const why = t.stale ? `${t.staleStatus || 'stale'} — the page changed after this answer` : provenance(t);
    meta.textContent = why;
    if (t.routeLabel && t.status !== 'pending' && t.status !== 'error') meta.textContent += (meta.textContent ? ' · ' : '') + t.routeLabel;
    card.appendChild(meta);
    return card;
  }

  function paintTurn(t) {
    const log = document.getElementById('ask-log');
    if (!log) return;
    const empty = log.querySelector('.overlay-empty');
    if (empty) empty.remove();
    const node = turnNode(t);
    // `t.el` is stale after a close (closeOverlay empties the root), so a node that
    // is no longer in the log means "append", which is how the log is rebuilt on
    // reopen without re-asking the provider.
    if (t.el && t.el.parentNode === log) log.replaceChild(node, t.el);
    else {
      log.appendChild(node);
      log.scrollTop = log.scrollHeight;
    }
    t.el = node;
  }

  async function submitAsk(over) {
    const input = document.getElementById('ask-input');
    if (!input || askBusy !== null) return;
    over = over && over.intent ? over : null;
    const question = over ? (over.question || '') : input.value.trim();
    const intent = over ? over.intent : document.getElementById('ask-intent').value;
    const useSel = over ? !!over.selection : document.getElementById('ask-sel').checked;
    if (!question && !(useSel && intent !== 'ask' && intent !== 'act')) return;
    let res;
    try {
      res = await invoke('ai.ask', {
        question,
        intent,
        selection: useSel,
        model: document.getElementById('ask-model').value || undefined,
        local_only: document.getElementById('ask-local').checked,
        tabs: [...extraTabs],
      });
    } catch (e) {
      setAskStatus(clip((e && e.message) || 'the host did not accept the question', 200), 'error');
      return;
    }
    if (!res || !res.accepted) {
      setAskStatus(clip((res && res.reason) || 'the host did not accept the question', 200), 'error');
      return;
    }
    // The turn exists from here; the `ai.answer` events carry the same turn_id and
    // fill it in, so one question can never produce two cards.
    let t = askTurns.get(res.turn_id);
    if (!t) {
      t = { turnId: res.turn_id, question, status: 'pending', citations: [] };
      askTurns.set(res.turn_id, t);
      while (askTurns.size > MAX_ASK_TURNS) askTurns.delete(askTurns.keys().next().value);
    }
    t.taskId = res.task_id;
    askBusy = res.turn_id;
    input.value = '';
    setBusy(true);
    setAskStatus('Thinking…');
    paintTurn(t);
  }

  // ── side pane: quick actions and the endpoint form ───────────────────
  const QUICK = [
    ['Summarize page', { intent: 'summarize' }],
    ['Key points', { intent: 'ask', question: 'List the key points of this page.' }],
    ['Explain selection', { intent: 'explain', selection: true }],
    ['Find related pages', { intent: 'act', question: 'Find pages related to this one and open the best two as candidate tabs.' }],
  ];

  function wireSidePane() {
    const quick = document.getElementById('ask-quick');
    if (quick && quick.dataset.quick) {
      for (const [label, over] of QUICK) {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'ask-qbtn';
        b.textContent = label;
        b.addEventListener('click', () => submitAsk(over));
        quick.appendChild(b);
      }
    }
    const fresh = document.getElementById('ask-new');
    if (fresh) fresh.addEventListener('click', () => { invoke('ai.history.clear', {}).catch(() => {}); });
    const gear = document.getElementById('ask-gear');
    if (gear) gear.addEventListener('click', toggleSettings);
    const save = document.getElementById('set-save');
    if (save) save.addEventListener('click', saveSettings);
  }

  async function toggleSettings(force) {
    const box = document.getElementById('ask-set');
    if (!box) return;
    const show = typeof force === 'boolean' ? force : box.hidden;
    box.hidden = !show;
    if (!show) return;
    try { askStatusInfo = await invoke('ai.status', {}); } catch (e) { return; }
    const p = askStatusInfo.provider || {};
    document.getElementById('set-url').value = p.base_url || '';
    document.getElementById('set-model').value = p.model_id || '';
    document.getElementById('set-key').value = '';
    document.getElementById('set-key').placeholder = p.key_saved ? 'saved — leave empty to keep it' : 'API key';
  }

  async function saveSettings() {
    const msg = document.getElementById('set-msg');
    msg.textContent = 'Saving…';
    try {
      askStatusInfo = await invoke('ai.provider.set', {
        base_url: document.getElementById('set-url').value,
        model_id: document.getElementById('set-model').value,
        api_key: document.getElementById('set-key').value,
      });
      msg.textContent = 'Saved. Ready.';
      paintContext();
      toggleSettings(false);
    } catch (e) {
      msg.textContent = clip((e && e.message) || 'could not save', 160);
      msg.className = 'ask-status is-error';
    }
  }

  // ── selection toolbar ────────────────────────────────────────────────
  const TOOLBAR_ACTIONS = [
    ['ask', 'Ask'], ['explain', 'Explain'], ['verify', 'Verify'],
    ['translate', 'Translate'], ['rewrite', 'Rewrite'],
  ];

  function openToolbar() {
    root.className = 'overlay-root';
    root.textContent = '';
    const bar = document.createElement('div');
    bar.className = 'sel-toolbar';
    bar.setAttribute('role', 'toolbar');
    bar.setAttribute('aria-label', 'Selection actions');
    for (const [intent, label] of TOOLBAR_ACTIONS) {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'sel-btn';
      b.textContent = label;
      b.addEventListener('mousedown', (e) => e.preventDefault());
      b.addEventListener('click', () => selectionAct(intent));
      bar.appendChild(b);
    }
    root.appendChild(bar);
  }

  async function selectionAct(intent) {
    // The host turns the toolbar into the card before the task starts.
    let res;
    try {
      res = await invoke('ai.selection.act', { intent, question: intent === 'ask' ? '' : undefined });
    } catch (e) {
      openAsk(true);
      setAskStatus(clip((e && e.message) || 'could not start', 200), 'error');
      return;
    }
    openAsk(true);
    if (res && res.turn_id !== undefined) {
      let t = askTurns.get(res.turn_id);
      if (!t) {
        t = { turnId: res.turn_id, question: intent, status: 'pending', citations: [] };
        askTurns.set(res.turn_id, t);
      }
      askBusy = res.turn_id;
      paintTurn(t);
      setBusy(true);
      setAskStatus('Thinking…');
    }
  }

  // ── host events ──────────────────────────────────────────────────────
  on('overlay.state', (p) => {
    if (!p || !p.open) { closeOverlay(); return; }
    if (p.kind === 'palette') openPalette('> ');
    else if (p.kind === 'full_page') openHistory();
    else if (p.kind === 'ask') openAsk(false);
    else if (p.kind === 'toolbar') openToolbar();
    else if (p.kind === 'card') openAsk(true);
  });

  on('shell.focus_palette', (p) => openPalette((p && p.prefill) || '> '));

  on('shell.shortcut', (p) => {
    if (!p) return;
    if (p.name === 'escape') invoke('shell.close_top_overlay', {});
    else if (p.name === 'ask_ai') openAsk(false);
  });

  // Streamed answer text. It is provisional: the final `ai.answer` replaces it, and
  // only that one carries citations.
  on('ai.delta', (p) => {
    const t = p && askTurns.get(p.turn_id);
    if (!t || t.status !== 'pending') return;
    t.streamed = (t.streamed || '') + String(p.text || '');
    paintTurn(t);
  });

  on('ai.stage', (p) => {
    const t = p && askTurns.get(p.turn_id);
    if (!t) return;
    if (p.stage === 'routed') {
      t.routeLabel = `${p.model || 'model'} · ${p.local ? 'on this device' : 'cloud'}`;
      setAskStatus(`Routed to ${t.routeLabel}`);
    } else if (typeof p.stage === 'string') {
      setAskStatus(p.stage.replace(/_/g, ' ') + '…');
    }
  });

  on('ai.models', (p) => paintModels(p && p.models));
  on('ai.context', (p) => { askStatusInfo = p; paintContext(); });
  for (const ev of ['tab.created', 'tab.closed', 'tab.updated']) on(ev, () => refreshStatus());
  on('ai.link', (p) => {
    if (p && p.state === 'down') setAskStatus('The AI process stopped. The browser is unaffected; ask again to restart it.', 'warn');
  });

  // A page the assistant opened in the background: shown as a candidate the user can
  // switch to. Nothing here navigates; the tab id goes back through the same
  // `tab.select` command the rail uses.
  on('ai.candidate', (p) => {
    const row = document.getElementById('ask-cands');
    if (!row || !p) return;
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'ask-chip ask-cand';
    b.textContent = clip(p.title || p.url || 'candidate', 40);
    b.title = (p.reason ? p.reason + ' — ' : '') + (p.url || '');
    b.addEventListener('click', () => invoke('tab.select', { tab_id: p.tab_id }).catch(() => {}));
    row.appendChild(b);
  });

  // The host sends `pending` and then the final payload for the same turn_id.
  on('ai.answer', (p) => {
    if (!p || typeof p.turn_id !== 'number') return;
    let t = askTurns.get(p.turn_id);
    if (!t) {
      t = { turnId: p.turn_id, status: 'pending', citations: [] };
      askTurns.set(p.turn_id, t);
      while (askTurns.size > MAX_ASK_TURNS) askTurns.delete(askTurns.keys().next().value);
    }
    if (p.question) t.question = p.question;
    t.taskId = p.task_id;
    t.tabId = p.tab_id;
    t.status = p.status || 'ok';
    t.text = p.text || '';
    t.provider = p.provider || 'offline';
    t.model = p.model || '';
    t.egress = !!p.egress;
    t.elapsedMs = p.elapsed_ms || 0;
    t.citations = p.citations || [];
    t.failure = p.failure || null;
    if (p.stale) t.stale = true;
    if (t.status !== 'pending') { askBusy = null; setBusy(false); }
    else { askBusy = p.turn_id; setBusy(true); }
    paintTurn(t);
    if (t.status === 'pending') {
      setAskStatus('Thinking…');
    } else if (t.status === 'error') {
      setAskStatus(failureLine(t) || 'No answer.', 'error');
    } else {
      const n = t.citations.length;
      setAskStatus(n ? `${n} citation${n === 1 ? '' : 's'} — click one to scroll the page there.` : '');
    }
    setAskSource();
    const input = document.getElementById('ask-input');
    if (input && askBusy === null) { input.disabled = false; input.focus(); }
  });

  // A navigation in the cited tab: the answer stays readable, but acting on it is
  // refused by the host, so say so here rather than deleting the turn.
  on('ai.answer.stale', (p) => {
    const ids = (p && p.turn_ids) || [];
    for (const id of ids) {
      const t = askTurns.get(id);
      if (!t) continue;
      t.stale = true;
      t.staleStatus = (p && p.status) || 'stale';
      paintTurn(t);
    }
    if (ids.length) setAskStatus('The page changed; these citations no longer point at it.', 'warn');
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

  on('ai.history.cleared', () => {
    askTurns.clear();
    const log = document.getElementById('ask-log');
    if (log) {
      log.textContent = '';
      const hint = document.createElement('p');
      hint.className = 'overlay-empty';
      hint.textContent = 'New conversation. Ask me anything.';
      log.appendChild(hint);
    }
  });

  on('ai.focus', () => { const i = document.getElementById('ask-input'); if (i) i.focus(); });
})();