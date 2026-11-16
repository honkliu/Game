// native-browser shell.
//
// Mirrors the Rust-side envelope (src/ipc.rs) by hand. Comments name the
// exact Rust counterpart for each piece so the two cannot drift silently.
//
// Host → UI:  window.__native._dispatch({"event": "...", "payload": {...}})
// UI → host:  ipc.postMessage({"id": n, "cmd": "...", "payload": {...}})
// Host reply: window.__native._reply({"id": n, "ok": true, "data": ...})

(() => {
  'use strict';

  // ── bridge ───────────────────────────────────────────────────────────
  const pending = new Map();
  let nextId = 1;
  const listeners = new Map();

  function invoke(cmd, payload) {
    const id = nextId++;
    const body = JSON.stringify({ id, cmd, payload: payload || {} });
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      if (window.ipc && typeof window.ipc.postMessage === 'function') {
        window.ipc.postMessage(body);
      } else {
        pending.delete(id);
        reject(new Error('ipc bridge unavailable'));
      }
    });
  }

  function on(event, fn) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(fn);
    return () => listeners.get(event).delete(fn);
  }

  function emitLocal(event, payload) {
    const set = listeners.get(event);
    if (!set) return;
    for (const fn of set) {
      try { fn(payload); } catch (e) { console.error(event, e); }
    }
  }

  window.__native = {
    invoke,
    on,
    _dispatch(env) {
      if (!env || typeof env.event !== 'string') return;
      emitLocal(env.event, env.payload);
    },
    _reply(reply) {
      if (!reply) return;
      const entry = pending.get(reply.id);
      if (!entry) return;
      pending.delete(reply.id);
      if (reply.ok) entry.resolve(reply.data);
      else entry.reject(new Error(reply.error || 'command failed'));
    },
  };

  // ── state (host-owned; never mutated locally except hover/focus) ─────
  const state = {
    tabs: [],
    groups: [],
    activeTabId: null,
    settings: {},
    downloads: [],
    find: null,
    omnibox: { rows: [], selected: 0, open: false },
    railFocus: null,
    canvas: null,
    focusedInput: false,
  };

  const $ = (sel) => document.querySelector(sel);
  const el = {
    app: $('#app'),
    rail: $('#rail'),
    railList: $('#rail-list'),
    railGroups: $('#rail-groups'),
    railPinned: $('#rail-pinned'),
    railPinnedGrid: $('#rail-pinned-grid'),
    railToggle: $('#rail-toggle'),
    newTab: $('#new-tab'),
    newCanvas: $('#new-canvas'),
    canvasLayer: $('#canvas-layer'),
    canvasBar: $('#canvas-bar'),
    omniboxForm: $('#omnibox-form'),
    omnibox: $('#omnibox'),
    chip: $('#mode-chip'),
    dropdown: $('#omnibox-dropdown'),
    back: $('#btn-back'),
    forward: $('#btn-forward'),
    reload: $('#btn-reload'),
    star: $('#bookmark-btn'),
    downloadsBtn: $('#btn-downloads'),
    downloadBadge: $('#download-badge'),
    menu: $('#btn-menu'),
    findBar: $('#find-bar'),
    findInput: $('#find-input'),
    findCount: $('#find-count'),
    findClose: $('#find-close'),
    findNext: $('#find-next'),
    findPrev: $('#find-prev'),
    findCase: $('#find-case'),
    findWord: $('#find-word'),
    findAll: $('#find-all'),
    panel: $('#downloads-panel'),
    downloadsList: $('#downloads-list'),
    downloadsEmpty: $('#downloads-empty'),
    downloadsClear: $('#downloads-clear'),
    downloadsClose: $('#downloads-close'),
    toast: $('#toast'),
    itemMenu: $('#item-menu'),
  };

  let toastTimer = 0;
  function toast(text) {
    el.toast.textContent = text;
    el.toast.hidden = false;
    el.toast.classList.add('is-visible');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      el.toast.classList.remove('is-visible');
      setTimeout(() => { el.toast.hidden = true; }, 140);
    }, 1500);
  }

  // ── SVG helpers (inline, no network) ─────────────────────────────────
  const svg = (d, w = 16) =>
    `<svg viewBox="0 0 ${w} ${w}" aria-hidden="true"><path d="${d}" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

  // ── rail ─────────────────────────────────────────────────────────────
  function activeTab() {
    return state.tabs.find((t) => t.active) || null;
  }

  function renderTabs() {
    const railTabs = state.tabs.filter((t) => t.member_of == null);
    const pinned = railTabs.filter((t) => t.pinned);
    const unpinned = railTabs.filter((t) => !t.pinned);

    const vertical = !!state.settings.vertical_tabs;
    document.documentElement.dataset.tabs = vertical ? 'vertical' : 'horizontal';
    state.settings.rail_collapsed = vertical && !!state.settings.rail_collapsed;
    el.rail.classList.toggle('is-collapsed', state.settings.rail_collapsed);
    el.railToggle.setAttribute('aria-pressed', String(!state.settings.rail_collapsed));
    el.railList.setAttribute('aria-orientation', vertical ? 'vertical' : 'horizontal');

    // Pinned: rows when expanded, 3-col grid when collapsed (§4.2).
    el.railPinned.innerHTML = '';
    el.railPinned.hidden = pinned.length === 0 || !!state.settings.rail_collapsed;
    for (const t of pinned) el.railPinned.appendChild(tabRow(t, false));

    el.railPinnedGrid.innerHTML = '';
    el.railPinnedGrid.hidden = !state.settings.rail_collapsed;
    for (const t of pinned) {
      const b = document.createElement('button');
      b.className = 'pinned-cell';
      b.setAttribute('aria-selected', String(!!t.active));
      b.title = t.title;
      b.innerHTML = iconFor(t);
      b.addEventListener('click', () => invoke('tab.select', { tab_id: t.id }));
      b.addEventListener('contextmenu', (e) => { e.preventDefault(); openItemMenu(t, e.clientX, e.clientY); });
      el.railPinnedGrid.appendChild(b);
    }

    // Group headers (one level of nesting only).
    el.railGroups.innerHTML = '';
    const byGroup = new Map();
    for (const t of unpinned) {
      const g = t.group_id ?? null;
      if (!byGroup.has(g)) byGroup.set(g, []);
      byGroup.get(g).push(t);
    }
    const order = [];
    for (const t of unpinned) {
      if (t.group_id != null && !order.includes(t.group_id)) order.push(t.group_id);
    }
    el.railList.innerHTML = '';
    const frag = document.createDocumentFragment();
    // Ungrouped first (matching the visual order produced by the host).
    for (const t of byGroup.get(null) || []) frag.appendChild(tabRow(t, false));
    for (const gid of order) {
      const g = state.groups.find((x) => x.id === gid);
      if (!g) continue;
      const head = document.createElement('li');
      head.className = 'group-row color-' + g.color;
      head.setAttribute('role', 'option');
      head.setAttribute('aria-expanded', String(!g.collapsed));
      head.tabIndex = -1;
      head.innerHTML = `<svg class="group-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="m3 4.5 3 3 3-3" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round"/></svg>
        <span class="group-dot"></span><span class="group-name"></span><span class="group-count"></span>`;
      head.querySelector('.group-name').textContent = g.name;
      head.querySelector('.group-count').textContent = String((byGroup.get(gid) || []).length);
      head.addEventListener('click', () => invoke('group.collapse', { group_id: gid, collapsed: !g.collapsed }));
      head.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          invoke('group.collapse', { group_id: gid, collapsed: !g.collapsed });
        }
      });
      frag.appendChild(head);
      if (!g.collapsed) {
        for (const t of byGroup.get(gid) || []) frag.appendChild(tabRow(t, true));
      }
    }
    el.railList.appendChild(frag);

    // Roving tabindex: exactly one row is tabbable.
    const rows = [...el.railList.querySelectorAll('.tab-row')];
    const focusRow = state.railFocus != null
      ? rows.find((r) => Number(r.dataset.tabId) === state.railFocus)
      : rows.find((r) => r.getAttribute('aria-selected') === 'true') || rows[0];
    for (const r of rows) r.tabIndex = r === focusRow ? 0 : -1;

    el.canvasBar.hidden = !state.canvas;
    renderCanvas();
    updateChrome();
  }

  // ── canvas: a grid of live pages (frames, title strips, empty cells) ──
  // The pages are native surfaces on top of this layer; only the gaps and the
  // title strips (geometry from the host) are visible, so everything here is
  // positioned in window coordinates.
  let dragging = null;
  function renderCanvas() {
    const cv = state.canvas;
    const layer = el.canvasLayer;
    layer.hidden = !cv;
    for (const node of [...layer.children]) if (node !== (dragging && dragging.el)) node.remove();
    // The chrome scales with page zoom (body zoom); canvas geometry is in window pixels.
    const scale = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--ui-scale')) || 1;
    layer.style.zoom = String(1 / scale);
    if (!cv) return;
    for (const c of cv.cells) {
      const cell = document.createElement('div');
      cell.className = 'cv-cell' + (c.focused ? ' is-focused' : '') + (c.tab_id == null ? ' is-empty' : '');
      cell.dataset.cell = String(c.index);
      cell.hidden = !!c.hidden;
      const box = (r) => `left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px`;
      cell.style.cssText = box(c.frame);
      if (c.tab_id == null) {
        const empty = document.createElement('div');
        empty.className = 'cv-empty';
        empty.style.cssText = `left:${c.body.x - c.frame.x}px;top:${c.body.y - c.frame.y}px;width:${c.body.w}px;height:${c.body.h}px`;
        empty.textContent = '';
        empty.addEventListener('click', () => invoke('canvas.focus', { cell: c.index }));
        cell.appendChild(empty);
      }
      layer.appendChild(cell);
    }
    for (const d of cv.dividers || []) {
      const geom = `left:${d.rect.x}px;top:${d.rect.y}px;width:${d.rect.w}px;height:${d.rect.h}px`;
      // The divider being dragged keeps its element (and pointer capture) across redraws.
      if (dragging && dragging.vertical === d.vertical && dragging.index === d.index && dragging.row === d.row) {
        dragging.el.style.cssText = geom;
        continue;
      }
      const h = document.createElement('div');
      h.className = 'cv-divider ' + (d.vertical ? 'is-vertical' : 'is-horizontal');
      h.style.cssText = geom;
      h.addEventListener('dblclick', () => invoke('canvas.resize', { reset: true }));
      h.addEventListener('pointerdown', (e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        h.setPointerCapture(e.pointerId);
        h.classList.add('is-dragging');
        dragging = { el: h, vertical: d.vertical, index: d.index, row: d.row };
        let pending = null;
        const send = () => {
          const pos = d.vertical ? pending.clientX : pending.clientY;
          pending = null;
          invoke('canvas.resize', { vertical: d.vertical, index: d.index, row: d.row, pos });
        };
        const move = (ev) => { if (!pending) requestAnimationFrame(send); pending = ev; };
        const up = () => {
          h.removeEventListener('pointermove', move);
          h.removeEventListener('pointerup', up);
          h.removeEventListener('pointercancel', up);
          h.classList.remove('is-dragging');
          dragging = null;
        };
        h.addEventListener('pointermove', move);
        h.addEventListener('pointerup', up);
        h.addEventListener('pointercancel', up);
      });
      layer.appendChild(h);
    }
  }

  function iconFor(t) {
    if (t.favicon && /^data:image\//.test(t.favicon)) {
      return `<span class="tab-icon"><img alt="" src="${t.favicon}"></span>`;
    }
    const letter = (t.title || '?').trim().charAt(0).toUpperCase() || '·';
    return `<span class="tab-icon" aria-hidden="true">${escapeHtml(letter)}</span>`;
  }

  function tabRow(t, grouped) {
    const row = document.createElement('li');
    row.className = 'tab-row' + (grouped ? ' is-grouped' : '') + (t.sleeping ? ' is-sleeping' : '');
    row.setAttribute('role', 'option');
    row.setAttribute('aria-selected', String(!!(t.active || t.shown)));
    row.dataset.tabId = String(t.id);
    if (t.canvas) row.classList.add('is-canvas');
    row.style.setProperty('--tab-hue', String(tabHue(t)));
    row.tabIndex = -1;
    const url = t.url || '';
    row.title = url ? `${t.title}\n${url}` : t.title;
    row.innerHTML = `${iconFor(t)}<span class="tab-title"></span>
      <button class="tab-close" tabindex="-1" aria-label="Close tab">${svg('m4 4 8 8m0-8-8 8', 12)}</button>`;
    row.querySelector('.tab-title').textContent = t.title || t.url || 'New tab';
    row.addEventListener('click', (e) => {
      if (e.target.closest('.tab-close')) {
        e.stopPropagation();
        invoke('tab.close', { tab_id: t.id });
        return;
      }
      invoke('tab.select', { tab_id: t.id });
    });
    row.addEventListener('contextmenu', (e) => { e.preventDefault(); openItemMenu(t, e.clientX, e.clientY); });
    row.addEventListener('keydown', (e) => railKey(e, row, t));
    row.querySelector('.tab-close').addEventListener('focus', () => { state.railFocus = t.id; });
    return row;
  }

  function railKey(e, row, t) {
    const rows = [...el.railList.querySelectorAll('.tab-row')];
    const i = rows.indexOf(row);
    switch (e.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        e.preventDefault();
        moveRail(rows, i, 1);
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        e.preventDefault();
        moveRail(rows, i, -1);
        break;
      case 'Home':
        e.preventDefault();
        focusRailRow(rows, 0);
        break;
      case 'End':
        e.preventDefault();
        focusRailRow(rows, rows.length - 1);
        break;
      case 'Enter':
        e.preventDefault();
        invoke('tab.select', { tab_id: t.id });
        break;
      case 'Delete':
        e.preventDefault();
        invoke('tab.close', { tab_id: t.id });
        break;
      case ' ':
      case 'Shift+F10':
      case 'ContextMenu':
        e.preventDefault();
        {
          const r = row.getBoundingClientRect();
          openItemMenu(t, r.left + 24, r.top + r.height);
        }
        break;
      default:
        break;
    }
  }

  function moveRail(rows, from, delta) {
    if (!rows.length) return;
    focusRailRow(rows, Math.min(rows.length - 1, Math.max(0, from + delta)));
  }

  function focusRailRow(rows, index) {
    const row = rows[Math.min(rows.length - 1, Math.max(0, index))];
    if (!row) return;
    state.railFocus = Number(row.dataset.tabId);
    for (const r of rows) r.tabIndex = r === row ? 0 : -1;
    row.focus();
  }

  // ── chrome (omnibox value, nav buttons, star) ────────────────────────
  function updateChrome() {
    const t = activeTab();
    const focused = document.activeElement === el.omnibox;
    if (!focused) {
      el.omnibox.value = t ? (t.url === 'native://newtab' ? '' : t.url) : '';
    }
    el.omnibox.placeholder = t && t.url === 'native://newtab' ? 'Search or type a URL' : (t ? t.url : 'Search or type a URL');
    el.back.disabled = !(t && t.can_go_back);
    el.forward.disabled = !(t && t.can_go_forward);
    $('#zoom-label').textContent = `${Math.round(((t && t.zoom) || 1) * 100)}%`;
    const bookmarked = t && state.bookmarks ? state.bookmarks.has(t.url) : false;
    el.star.setAttribute('aria-pressed', String(!!bookmarked));
    const active = state.downloads.filter((d) => d.state === 'in_progress').length;
    el.downloadBadge.hidden = active === 0;
    el.downloadBadge.textContent = String(active);
    document.title = t ? `${t.title} — native-browser` : 'native-browser';
  }

  // ── omnibox ──────────────────────────────────────────────────────────
  let omniSeq = 0;
  let omniTimer = 0;

  function omniboxChanged() {
    const text = el.omnibox.value;
    updateChip(text);
    clearTimeout(omniTimer);
    omniTimer = setTimeout(() => queryOmnibox(text), 40);
  }

  function updateChip(text) {
    let label = 'Search';
    if (text.startsWith('> ') || text === '>') label = '› Command';
    else if (text.startsWith('/ ')) label = '🔎 Find';
    else if (text.startsWith('? ')) label = 'Search';
    else label = classifyChip(text);
    el.chip.textContent = label;
  }

  // Mirrors omnibox::classify's *mode* decision only — the host owns the real
  // classification and the resulting URL; this is the chip label.
  function classifyChip(text) {
    const t = text.trim();
    if (!t) return 'Search';
    if (/^[a-z][a-z0-9+.-]*:/i.test(t)) return 'URL';
    if (/^(localhost|(\d{1,3}\.){3}\d{1,3})(:\d+)?(\/|$)/i.test(t)) return 'URL';
    if (!/\s/.test(t) && /^[^.]+\.[a-zA-Z]{2,}([/:?#]|$)/.test(t)) return 'URL';
    return 'Search';
  }

  async function queryOmnibox(text) {
    const seq = ++omniSeq;
    try {
      const res = await invoke('omnibox.query', { text, limit: 10 });
      if (seq !== omniSeq) return;
      state.omnibox.rows = (res && res.rows) || [];
      state.omnibox.selected = 0;
      renderDropdown();
    } catch (e) {
      console.error('omnibox.query', e);
    }
  }

  function renderDropdown() {
    const rows = state.omnibox.rows;
    const open = document.activeElement === el.omnibox && rows.length > 0;
    state.omnibox.open = open;
    el.omnibox.setAttribute('aria-expanded', String(open));
    el.dropdown.hidden = !open;
    if (!open) return;
    el.dropdown.innerHTML = '';
    rows.forEach((r, i) => {
      const row = document.createElement('div');
      row.className = 'dropdown-row';
      row.setAttribute('role', 'option');
      row.id = `omni-row-${i}`;
      row.setAttribute('aria-selected', String(i === state.omnibox.selected));
      const glyph = r.kind === 'ask' ? '✨' : r.kind === 'search' ? '🔍' : r.kind === 'command' ? '›' : r.kind === 'url' ? '↗' : r.kind === 'tab' ? '▭' : '◷';
      row.innerHTML = `<span class="dropdown-glyph">${glyph}</span>
        <span class="dropdown-text"><span class="dropdown-title"></span><span class="dropdown-sub"></span></span>
        <span class="dropdown-hint"></span>`;
      row.querySelector('.dropdown-title').textContent = r.title || r.url || '';
      row.querySelector('.dropdown-sub').textContent = r.hint || r.url || '';
      row.querySelector('.dropdown-hint').textContent = r.kind === 'history' ? 'open' : '';
      row.addEventListener('mousedown', (e) => { e.preventDefault(); state.omnibox.selected = i; activateRow(); });
      el.dropdown.appendChild(row);
    });
    const footer = document.createElement('div');
    footer.className = 'dropdown-footer';
    footer.innerHTML = `<span>${el.chip.textContent}</span><span>↑↓ navigate · Tab complete · Enter go · Esc dismiss</span>`;
    el.dropdown.appendChild(footer);
    el.omnibox.setAttribute('aria-activedescendant', `omni-row-${state.omnibox.selected}`);
  }

  function moveSelection(delta) {
    const n = state.omnibox.rows.length;
    if (!n) return;
    state.omnibox.selected = (state.omnibox.selected + delta + n) % n;
    renderDropdown();
    const sel = el.dropdown.querySelector('[aria-selected="true"]');
    if (sel) sel.scrollIntoView({ block: 'nearest' });
  }

  function completeSelection() {
    const r = state.omnibox.rows[state.omnibox.selected];
    if (!r) return;
    el.omnibox.value = r.url || r.title || el.omnibox.value;
    omniboxChanged();
  }

  function activateRow() {
    const r = state.omnibox.rows[state.omnibox.selected];
    if (!r) {
      submitOmnibox();
      return;
    }
    if (r.kind === 'command') {
      invoke('omnibox.activate', { text: el.omnibox.value, verb: r.verb });
    } else if (r.kind === 'tab') {
      invoke('tab.select', { tab_id: r.tab_id });
      closeDropdown();
      focusPage();
    } else if (r.kind === 'find') {
      invoke('omnibox.activate', { text: el.omnibox.value });
    } else {
      invoke('omnibox.activate', { text: el.omnibox.value, url: r.url });
    }
    closeDropdown();
  }

  function submitOmnibox() {
    if (state.omnibox.rows.length) {
      activateRow();
      return;
    }
    const text = el.omnibox.value;
    if (!text.trim()) return;
    invoke('omnibox.activate', { text });
    closeDropdown();
    el.omnibox.blur();
  }

  function closeDropdown() {
    state.omnibox.open = false;
    el.dropdown.hidden = true;
    el.omnibox.setAttribute('aria-expanded', 'false');
  }

  // ── focus model (§3.6) ───────────────────────────────────────────────
  function focusPage() {
    closeDropdown();
    el.omnibox.blur();
    // The host moves OS focus back to the active page webview.
    invoke('shell.focus_page', {}).catch(() => {});
    if (window.__native && window.__native.focusPage) window.__native.focusPage();
  }

  function focusOmnibox(selectAll = true) {
    el.omnibox.focus();
    if (selectAll) el.omnibox.select();
    updateChrome();
  }

  // ── item menu (§3.5 Space, max 8 items, one level) ───────────────────
  // Each site gets its own tab colour (stable per host); a canvas has its own.
  function tabHue(t) {
    // Golden-angle steps keep neighbouring tab ids visibly different; the colour is stable per tab.
    return (Number(t.id) * 137.508 + 20) % 360 | 0;
  }

  const GRIDS = [[2, 1], [1, 2], [2, 2], [3, 1], [3, 2], [4, 1], [4, 2], [4, 3]];
  function openItemMenu(t, x, y, custom) {
    const isCanvas = !!(t && t.canvas);
    const items = custom || (isCanvas ? [
      { label: 'Zoom focused page', run: () => invoke('canvas.zoom', { canvas_id: t.id }) },
      ...GRIDS.map(([c, r]) => ({
        label: `Layout ${c} × ${r}`, run: () => invoke('canvas.layout', { canvas_id: t.id, cols: c, rows: r }),
      })),
      { sep: true },
      { label: 'Close canvas', run: () => invoke('tab.close', { tab_id: t.id }), danger: true },
    ] : [
      { label: 'Reload tab', run: () => invoke('nav.reload', { tab_id: t.id }) },
      { label: t.pinned ? 'Unpin tab' : 'Pin tab', run: () => invoke(t.pinned ? 'tab.unpin' : 'tab.pin', { tab_id: t.id }) },
      { label: 'Duplicate tab', run: () => invoke('tab.duplicate', {}) },
      { label: 'Copy URL', run: () => navigator.clipboard.writeText(t.url).catch(() => {}), },
      { sep: true },
      { label: 'Close tab', run: () => invoke('tab.close', { tab_id: t.id }), danger: true },
      { label: 'Close others', run: () => state.tabs.filter((o) => o.id !== t.id).forEach((o) => invoke('tab.close', { tab_id: o.id })) },
    ]);
    el.itemMenu.innerHTML = '';
    let focusIdx = 0;
    items.forEach((it, i) => {
      if (it.sep) {
        const sep = document.createElement('div');
        sep.className = 'menu-sep';
        sep.setAttribute('role', 'separator');
        el.itemMenu.appendChild(sep);
        return;
      }
      const b = document.createElement('button');
      b.className = 'menu-item' + (it.danger ? ' is-danger' : '');
      b.setAttribute('role', 'menuitem');
      b.tabIndex = i === focusIdx ? 0 : -1;
      b.textContent = it.label;
      b.addEventListener('click', () => { closeItemMenu(); it.run(); });
      el.itemMenu.appendChild(b);
    });
    el.itemMenu.hidden = false;
    invoke('shell.menu', { open: true });
    el.itemMenu.style.left = `${Math.min(x, window.innerWidth - 220)}px`;
    el.itemMenu.style.top = `${Math.min(y, window.innerHeight - 200)}px`;
    const first = el.itemMenu.querySelector('.menu-item');
    if (first) first.focus();
  }

  function closeItemMenu() {
    if (!el.itemMenu.hidden) invoke('shell.menu', { open: false });
    el.itemMenu.hidden = true;
    el.itemMenu.innerHTML = '';
  }

  el.itemMenu.addEventListener('keydown', (e) => {
    const items = [...el.itemMenu.querySelectorAll('.menu-item')];
    const i = items.indexOf(document.activeElement);
    if (e.key === 'Escape') { e.preventDefault(); closeItemMenu(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); items[Math.min(items.length - 1, i + 1)].focus(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); items[Math.max(0, i - 1)].focus(); }
  });
  document.addEventListener('mousedown', (e) => {
    if (!el.itemMenu.hidden && !e.target.closest('#item-menu')) closeItemMenu();
  });

  // ── find bar (§4.6) ──────────────────────────────────────────────────
  let findSeq = 0;
  function findQuery(kind) {
    const q = el.findInput.value;
    const seq = ++findSeq;
    invoke(kind || 'find.query', {
      query: q,
      case_sensitive: el.findCase.getAttribute('aria-pressed') === 'true',
      regex_or_whole_word: el.findWord.getAttribute('aria-pressed') === 'true',
    })
      .then((r) => { if (seq === findSeq && r) renderFind(r); })
      .catch((e) => console.error('find', e));
  }

  function renderFind(r) {
    el.findCount.textContent = r.supported_flags && r.supported_flags.length
      ? `${r.active}/${r.total}`
      : 'n/a';
    if (r.supported_flags && r.supported_flags.length === 0) {
      el.findInput.setAttribute('title', 'Find in page is unavailable on this WebView2 build (wry 0.57 exposes no find API)');
    }
  }

  function openFind(query) {
    el.findBar.hidden = false;
    if (typeof query === 'string') el.findInput.value = query;
    el.findInput.focus();
    el.findInput.select();
    findQuery('find.start');
  }

  function closeFind() {
    el.findBar.hidden = true;
    el.findInput.value = '';
    invoke('find.stop', {}).catch(() => {});
  }

  el.findInput.addEventListener('input', () => findQuery('find.query'));
  el.findInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); findQuery(e.shiftKey ? 'find.prev' : 'find.next'); }
    else if (e.key === 'Escape') { e.preventDefault(); closeFind(); focusPage(); }
  });
  el.findClose.addEventListener('click', closeFind);
  el.findNext.addEventListener('click', () => findQuery('find.next'));
  el.findPrev.addEventListener('click', () => findQuery('find.prev'));
  el.findCase.addEventListener('click', () => togglePressed(el.findCase, () => findQuery('find.query')));
  el.findAll.addEventListener('click', () => togglePressed(el.findAll));
  el.findWord.setAttribute('aria-disabled', 'true');
  el.findWord.addEventListener('click', () => toast('Regex / whole-word find is not available on this WebView2 build'));

  function togglePressed(btn, after) {
    const on = btn.getAttribute('aria-pressed') === 'true';
    btn.setAttribute('aria-pressed', String(!on));
    if (after) after();
  }

  // ── downloads (§4.4) ─────────────────────────────────────────────────
  function renderDownloads() {
    el.downloadsList.innerHTML = '';
    el.downloadsEmpty.hidden = state.downloads.length > 0;
    state.downloads.forEach((d, i) => {
      const li = document.createElement('li');
      li.className = 'download-row' + (i === 0 ? ' is-focused' : '');
      li.tabIndex = i === 0 ? 0 : -1;
      li.dataset.id = String(d.id);
      const pct = d.total_bytes && d.received_bytes != null
        ? ` · ${Math.round((d.received_bytes / d.total_bytes) * 100)}%`
        : '';
      const label = d.state === 'completed' ? 'Ready'
        : d.state === 'failed' ? 'Failed'
        : d.state === 'cancelled' ? 'Cancelled'
        : `Downloading${pct}`;
      const cls = d.state === 'completed' ? 'is-ok' : d.state === 'failed' || d.state === 'cancelled' ? 'is-error' : '';
      li.innerHTML = `<div class="download-name"></div>
        <div class="download-path"></div>
        <div class="download-state ${cls}"></div>
        <div class="download-actions"></div>`;
      li.querySelector('.download-name').textContent = d.file;
      li.querySelector('.download-path').textContent = d.path;
      li.querySelector('.download-path').title = d.path;
      li.querySelector('.download-state').textContent = label;
      const actions = li.querySelector('.download-actions');
      const mk = (text, danger, fn) => {
        const b = document.createElement('button');
        b.className = 'chip-btn' + (danger ? ' is-danger' : '');
        b.textContent = text;
        b.tabIndex = -1;
        b.addEventListener('click', (e) => { e.stopPropagation(); fn(); });
        actions.appendChild(b);
      };
      if (d.state !== 'in_progress') mk('Open', false, () => invoke('download.action', { download_id: d.id, action: 'open' }));
      mk('Show in folder', false, () => invoke('download.action', { download_id: d.id, action: 'reveal' }));
      mk('Remove', true, () => invoke('download.action', { download_id: d.id, action: 'remove' }));
      li.addEventListener('contextmenu', (e) => { e.preventDefault(); openDownloadMenu(d, e.clientX, e.clientY); });
      li.addEventListener('keydown', (e) => downloadKey(e, li, i));
      el.downloadsList.appendChild(li);
    });
    el.downloadsClear.disabled = state.downloads.length === 0;
  }

  function openDownloadMenu(d, x, y) {
    const items = [
      { label: 'Open', run: () => invoke('download.action', { download_id: d.id, action: 'open' }) },
      { label: 'Show in folder', run: () => invoke('download.action', { download_id: d.id, action: 'reveal' }) },
      { sep: true },
      { label: 'Remove from list', danger: true, run: () => invoke('download.action', { download_id: d.id, action: 'remove' }) },
    ];
    el.itemMenu.innerHTML = '';
    items.forEach((it) => {
      if (it.sep) {
        const s = document.createElement('div');
        s.className = 'menu-sep';
        el.itemMenu.appendChild(s);
        return;
      }
      const b = document.createElement('button');
      b.className = 'menu-item' + (it.danger ? ' is-danger' : '');
      b.textContent = it.label;
      b.addEventListener('click', () => { closeItemMenu(); it.run(); });
      el.itemMenu.appendChild(b);
    });
    el.itemMenu.hidden = false;
    invoke('shell.menu', { open: true });
    el.itemMenu.style.left = `${Math.min(x, window.innerWidth - 220)}px`;
    el.itemMenu.style.top = `${Math.min(y, window.innerHeight - 160)}px`;
    const first = el.itemMenu.querySelector('.menu-item');
    if (first) first.focus();
  }

  function downloadKey(e, li, i) {
    const rows = [...el.downloadsList.querySelectorAll('.download-row')];
    if (e.key === 'ArrowDown' && i < rows.length - 1) { e.preventDefault(); rows[i + 1].focus(); }
    else if (e.key === 'ArrowUp' && i > 0) { e.preventDefault(); rows[i - 1].focus(); }
    else if (e.key === 'Enter') {
      e.preventDefault();
      const d = state.downloads.find((x) => String(x.id) === li.dataset.id);
      if (d) {
        invoke('download.action', { download_id: d.id, action: e.altKey ? 'reveal' : 'open' }).catch(() => {});
      }
    } else if (e.key === 'Shift+F10' || e.key === 'ContextMenu') {
      e.preventDefault();
      const d = state.downloads.find((x) => String(x.id) === li.dataset.id);
      const r = li.getBoundingClientRect();
      if (d) openDownloadMenu(d, r.left + 16, r.top + r.height);
    }
  }

  el.downloadsClear.addEventListener('click', () => invoke('download.clear_list', {}));
  el.downloadsClose.addEventListener('click', () => invoke('shortcut', { name: 'escape' }));

  // ── toolbars ─────────────────────────────────────────────────────────
  el.back.addEventListener('click', () => invoke('nav.back', {}));
  el.forward.addEventListener('click', () => invoke('nav.forward', {}));
  el.reload.addEventListener('click', () => invoke('nav.reload', {}));
  el.star.addEventListener('click', () => invoke('tab.bookmark', {}));
  el.downloadsBtn.addEventListener('click', () => invoke('shortcut', { name: 'open_downloads' }));
  document.getElementById('btn-ai').addEventListener('click', () => invoke('shortcut', { name: 'ask_ai' }));
  el.menu.addEventListener('click', () => invoke('shell.open_palette', {}));
  el.newTab.addEventListener('click', () => invoke('tab.new', {}));
  el.newCanvas.addEventListener('click', () => {
    const r = el.newCanvas.getBoundingClientRect();
    openItemMenu(null, r.left, r.bottom + 2, GRIDS.map(([c, r2]) => ({
      label: `${c} × ${r2} canvas  (${c * r2} pages)`, run: () => invoke('canvas.create', { cols: c, rows: r2 }),
    })));
  });
  $('#cv-zoom').addEventListener('click', () => invoke('canvas.zoom', {}));
  $('#cv-close').addEventListener('click', () => state.canvas && invoke('canvas.clear', { cell: state.canvas.focus }));
  $('#cv-layout').addEventListener('click', (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    if (state.canvas) openItemMenu({ canvas: true, id: state.canvas.id }, r.left - 140, r.bottom);
  });
  el.railToggle.addEventListener('click', () => invoke('shell.toggle_rail', {}));
  $('#win-min').addEventListener('click', () => invoke('window.minimize', {}));
  $('#win-max').addEventListener('click', () => invoke('window.toggle_maximize', {}));
  $('#win-close').addEventListener('click', () => invoke('window.close', {}));
  const dragArea = $('#win-drag');
  dragArea.addEventListener('mousedown', (e) => { if (e.button === 0 && e.detail < 2) invoke('window.drag', {}); });
  dragArea.addEventListener('dblclick', () => invoke('window.toggle_maximize', {}));
  for (const edge of document.querySelectorAll('.win-edge')) {
    edge.addEventListener('mousedown', (e) => { if (e.button === 0) invoke('window.resize', { dir: edge.dataset.dir }); });
  }
  for (const [id, name] of [['zoom-in', 'zoom_in'], ['zoom-out', 'zoom_out'], ['zoom-label', 'zoom_reset']]) {
    $('#' + id).addEventListener('click', () => invoke('shortcut', { name }));
  }

  el.omniboxForm.addEventListener('submit', (e) => { e.preventDefault(); submitOmnibox(); });
  el.omnibox.addEventListener('input', omniboxChanged);
  el.omnibox.addEventListener('focus', () => {
    state.focusedInput = true;
    queryOmnibox(el.omnibox.value);
  });
  el.omnibox.addEventListener('blur', () => {
    state.focusedInput = false;
    setTimeout(closeDropdown, 100);
    updateChrome();
  });
  el.omnibox.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); moveSelection(1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); moveSelection(-1); }
    else if (e.key === 'Tab') {
      if (state.omnibox.open) { e.preventDefault(); completeSelection(); }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      submitOmnibox();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      if (state.omnibox.open) closeDropdown();
      else { el.omnibox.blur(); focusPage(); }
    }
  });

  // ── host events ──────────────────────────────────────────────────────
  on('app.ready', () => {
    invoke('diagnostics.self_test', {}).catch(() => {});
  });

  on('state.tabs', (p) => {
    if (!p) return;
    if (p.tabs) state.tabs = p.tabs;
    if (p.groups) state.groups = p.groups;
    if (p.active_tab_id !== undefined) state.activeTabId = p.active_tab_id;
    if (p.canvas !== undefined) state.canvas = p.canvas;
    renderTabs();
  });

  on('state.groups', (p) => {
    if (p) state.groups = p;
    renderTabs();
  });

  on('tab.created', () => { /* full state follows in state.tabs */ });
  on('tab.activated', (p) => {
    if (p && p.tabs) {
      state.tabs = p.tabs.tabs || state.tabs;
      state.groups = p.tabs.groups || state.groups;
    }
    updateChrome();
  });
  on('tab.navigated', (p) => {
    if (!p || p.id == null) return;
    const i = state.tabs.findIndex((t) => t.id === p.id);
    if (i >= 0) {
      state.tabs[i] = { ...state.tabs[i], ...p };
      renderTabs();
    }
  });
  on('tab.loading', () => { /* spinner state arrives via state.tabs */ });
  on('tab.zoom_changed', (p) => {
    if (!p) return;
    const t = state.tabs.find((x) => x.id === p.tab_id);
    if (t) t.zoom = p.factor;
    updateChrome();
  });
  on('state.settings', (p) => {
    state.settings = p || {};
    applyTheme();
    document.documentElement.dataset.compact = String(!!state.settings.compact_tab_rows);
    renderTabs();
  });
  on('state.downloads', (p) => {
    state.downloads = (p && p.downloads) || [];
    renderDownloads();
    updateChrome();
  });
  on('find.result', (r) => {
    if (r && r.closed) { el.findBar.hidden = true; return; }
    renderFind(r || {});
  });
  on('shell.focus_changed', (p) => {
    if (!p) return;
    if (p.scope === 'omnibox') focusOmnibox(!!p.select_all);
    else if (p.scope === 'cycle') cycleFocus();
  });
  on('shell.focus_find', (p) => openFind(p && p.query));
  on('shell.panel', (p) => {
    const panel = p && p.panel;
    el.panel.hidden = panel !== 'downloads';
    if (panel === 'downloads') { renderDownloads(); const first = el.downloadsList.querySelector('.download-row'); if (first) first.focus(); }
    if (panel !== 'find' && !el.findBar.hidden) { /* keep find open until host says otherwise */ }
  });
  on('shell.find_action', (p) => {
    if (!p) return;
    if (p.action === 'find_next') findQuery('find.next');
    else if (p.action === 'find_prev') findQuery('find.prev');
    else if (p.action === 'toggle_highlight_all') togglePressed(el.findAll);
    else if (p.action === 'toggle_regex') toast('Regex find is not available on this WebView2 build');
  });
  on('shell.shortcut', (p) => {
    if (!p) return;
    switch (p.name) {
      case 'escape':
        if (!el.itemMenu.hidden) { closeItemMenu(); return; }
        if (!el.findBar.hidden) { closeFind(); return; }
        break;
      case 'item_menu': {
        const row = document.activeElement && document.activeElement.closest('.tab-row');
        if (row) {
          const t = state.tabs.find((x) => String(x.id) === row.dataset.tabId);
          const r = row.getBoundingClientRect();
          if (t) openItemMenu(t, r.left + 24, r.top + r.height);
        }
        break;
      }
      case 'close_tab_or_word': {
        if (document.activeElement === el.omnibox || document.activeElement === el.findInput) {
          const inp = document.activeElement;
          if (inp.value.length === 0) invoke('shortcut', { name: 'close_tab' });
          // Otherwise the key has already been swallowed; the native edit
          // control did not see it. Delete a word explicitly.
          else deleteWord(inp);
        }
        break;
      }
      default:
        break;
    }
  });
  on('shell.ctrl_w', () => {
    if (document.activeElement === el.omnibox || document.activeElement === el.findInput) {
      if (document.activeElement.value.length === 0) invoke('shortcut', { name: 'close_tab' });
      else deleteWord(document.activeElement);
    }
  });
  on('shell.copy', (p) => {
    if (p && p.text) navigator.clipboard.writeText(p.text).then(() => toast('URL copied')).catch(() => toast('Copy failed'));
  });
  on('shell.escape', () => {
    if (!el.itemMenu.hidden) { closeItemMenu(); return; }
    if (!el.findBar.hidden) { closeFind(); return; }
    if (state.omnibox.open) { closeDropdown(); return; }
  });
  on('shell.toast', (p) => { if (p && p.text) toast(p.text); });
  on('shell.command_done', () => closeDropdown());
  on('shell.ui_scale', () => { renderCanvas(); });
  on('shell.layout', (p) => {
    if (!p) return;
    document.documentElement.style.setProperty('--rail-w', `${p.rail_w}px`);
    el.findBar.hidden = !p.find_open;
    if (p.canvas !== undefined) { state.canvas = p.canvas; renderCanvas(); }
  });
  on('diagnostics.report', (r) => {
    console.info('[diagnostics]', JSON.stringify({ startup_ms: r.startup_ms, shell_ready_ms: r.shell_ready_ms, process_count: r.process_count, rss_mb: r.rss_mb }));
  });

  function deleteWord(inp) {
    const v = inp.value;
    const start = inp.selectionStart ?? v.length;
    const end = inp.selectionEnd ?? v.length;
    if (start !== end) { inp.setRangeText('', start, end, 'end'); inp.dispatchEvent(new Event('input')); return; }
    const before = v.slice(0, start);
    const m = before.match(/(\s*\S+)\s*$/);
    const cut = m ? m[1].length : 1;
    inp.setRangeText('', Math.max(0, start - cut), start, 'end');
    inp.dispatchEvent(new Event('input'));
  }

  // F6 cycles page → omnibox → rail → page (§3.6).
  function cycleFocus() {
    const a = document.activeElement;
    if (a === el.omnibox) {
      const row = el.railList.querySelector('.tab-row');
      if (row) row.focus(); else focusPage();
    } else if (a && a.closest('#rail')) {
      focusPage();
    } else {
      focusOmnibox(true);
    }
  }

  function applyTheme() {
    const s = state.settings || {};
    let theme = s.theme || 'system';
    if (theme === 'system') theme = s.system_dark ? 'dark' : 'light';
    document.documentElement.dataset.theme = theme;
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  }

  // ── boot ─────────────────────────────────────────────────────────────
  applyTheme();
  invoke('shell.ready', {}).then(() => {
    // W1: report the first animation frame after the shell has painted.
    requestAnimationFrame(() => {
      const t = performance.now();
      invoke('diagnostics.self_test', {}).then(() => console.info(`[shell] first paint at ${t.toFixed(1)} ms (document)`)).catch(() => {});
    });
  }).catch((e) => console.error('shell.ready', e));

  window.addEventListener('resize', () => {
    // The host owns geometry; this listener exists only so the dropdown
    // mirror re-measures after a resize.
    if (!el.dropdown.hidden) renderDropdown();
  });
})();