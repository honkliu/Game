import { bridge } from './sidebar/mcp.js';

(() => {
  'use strict';

  const shell = bridge();
  const invoke = shell.invoke.bind(shell);
  const on = shell.on.bind(shell);
  on('mcp.resync', () => void invoke('shell_ui', { operation: 'ready' }));

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
    window: null,
  };

  const $ = (sel) => document.querySelector(sel);
  const assistantName = /^zh(?:-|$)/i.test(navigator.language) ? '洛' : 'River';
  $('#assistant-name').textContent = assistantName;
  $('#btn-ai').setAttribute('aria-label', assistantName);
  $('#btn-ai').title = assistantName + ' (Ctrl+Shift+A)';
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
    settings: $('#btn-settings'),
    findInput: $('#find-input'),
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
    const selectedTab = state.tabs.find(t => t.shown) || activeTab();
    document.documentElement.style.setProperty('--active-tab-hue', String(selectedTab ? tabHue(selectedTab) : 210));
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
      b.addEventListener('click', () => invoke('shell_tab', { operation: 'activate', tab_id: t.id }));
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
      head.addEventListener('click', () => invoke('shell_tab_group', { operation: 'collapsed_set', group_id: gid, collapsed: !g.collapsed }));
      head.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          invoke('shell_tab_group', { operation: 'collapsed_set', group_id: gid, collapsed: !g.collapsed });
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
        empty.addEventListener('click', () => invoke('shell_canvas', { operation: 'cell_focus', cell: c.index }));
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
      h.addEventListener('dblclick', () => invoke('shell_canvas', { operation: 'resize', reset: true }));
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
          invoke('shell_canvas', { operation: 'resize', vertical: d.vertical, index: d.index, row: d.row, pos });
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
    let dragged = false;
    row.addEventListener('pointerdown', e => {
      dragged = false;
      if (e.button !== 0 || e.target.closest('button') || e.ctrlKey || e.shiftKey || e.altKey || e.metaKey || document.documentElement.dataset.tabs !== 'horizontal') return;
      const { clientX: x, clientY: y, pointerId } = e;
      const cleanup = () => {
        window.removeEventListener('pointermove', move, true);
        window.removeEventListener('pointerup', cleanup, true);
        window.removeEventListener('pointercancel', cleanup, true);
        window.removeEventListener('blur', cleanup);
      };
      const move = event => {
        if (event.pointerId !== pointerId) return;
        if (!(event.buttons & 1)) { cleanup(); return; }
        if (Math.hypot(event.clientX - x, event.clientY - y) < 5) return;
        cleanup();
        dragged = true;
        event.preventDefault();
        invoke('shell_window', { operation: 'drag' });
      };
      window.addEventListener('pointermove', move, true);
      window.addEventListener('pointerup', cleanup, true);
      window.addEventListener('pointercancel', cleanup, true);
      window.addEventListener('blur', cleanup);
    });
    row.addEventListener('dragstart', e => e.preventDefault());
    row.addEventListener('click', (e) => {
      if (dragged) { e.preventDefault(); return; }
      if (e.target.closest('.tab-close')) {
        e.stopPropagation();
        invoke('shell_tab', { operation: 'close', tab_id: t.id });
        return;
      }
      invoke('shell_tab', { operation: 'activate', tab_id: t.id });
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
        invoke('shell_tab', { operation: 'activate', tab_id: t.id });
        break;
      case 'Delete':
        e.preventDefault();
        invoke('shell_tab', { operation: 'close', tab_id: t.id });
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
  let omniComposing = false;
  let omniInline = '';
  let omniTyped = '';

  function omniboxChanged(event) {
    const text = el.omnibox.value;
    const seq = ++omniSeq;
    omniTyped = text;
    omniInline = '';
    state.omnibox.explicitSelection = false;
    state.omnibox.rows = [];
    updateChip(text);
    clearTimeout(omniTimer);
    if (omniComposing || (event && event.isComposing)) { closeDropdown(false); return; }
    const complete = event && event.inputType === 'insertText' &&
      el.omnibox.selectionStart === text.length && el.omnibox.selectionEnd === text.length;
    omniTimer = setTimeout(() => queryOmnibox(text, seq, complete), 60);
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

  async function queryOmnibox(text, seq = ++omniSeq, complete = false) {
    try {
      const res = await invoke('shell_omnibox', { operation: 'query', text, limit: 10 });
      if (seq !== omniSeq || omniComposing || document.activeElement !== el.omnibox) return;
      if (res && res.superseded) return;
      omniTyped = text;
      state.omnibox.explicitSelection = false;
      state.omnibox.rows = (res && res.rows) || [];
      if (res.mode === 'ask') el.chip.textContent = '✨ ' + assistantName;
      state.omnibox.selected = 0;
      if (complete && el.omnibox.value === text && typeof res.completion === 'string' &&
          res.completion.length > text.length && res.completion.toLowerCase().startsWith(text.toLowerCase())) {
        omniInline = res.completion;
        el.omnibox.value = omniInline;
        el.omnibox.setSelectionRange(text.length, omniInline.length);
      }
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
      row.querySelector('.dropdown-sub').textContent = r.kind === 'ask'
        ? (r.hint || '').replace(/^River(?=:|$)/, assistantName) : r.hint || r.url || '';
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
    state.omnibox.explicitSelection = true;
    renderDropdown();
    const sel = el.dropdown.querySelector('[aria-selected="true"]');
    if (sel) sel.scrollIntoView({ block: 'nearest' });
  }

  function completeSelection() {
    const r = state.omnibox.rows[state.omnibox.selected];
    if (!r) return;
    el.omnibox.value = r.completion || r.url || r.title || el.omnibox.value;
    omniboxChanged();
  }

  function activateRow() {
    const r = state.omnibox.rows[state.omnibox.selected];
    if (!r) {
      submitOmnibox();
      return;
    }
    if (r.kind === 'command') {
      invoke('shell_omnibox', { operation: 'activate', text: el.omnibox.value, verb: r.verb });
    } else if (r.kind === 'tab') {
      invoke('shell_tab', { operation: 'activate', tab_id: r.tab_id });
      closeDropdown();
      focusPage();
    } else if (r.kind === 'find') {
      invoke('shell_omnibox', { operation: 'activate', text: el.omnibox.value });
    } else {
      invoke('shell_omnibox', { operation: 'activate', text: el.omnibox.value, url: r.url });
    }
    closeDropdown();
  }

  function submitOmnibox() {
    if (omniInline && el.omnibox.value === omniInline && !state.omnibox.explicitSelection) {
      invoke('shell_omnibox', { operation: 'activate', text: el.omnibox.value });
      closeDropdown();
      el.omnibox.blur();
      return;
    }
    if (state.omnibox.rows.length &&
        (state.omnibox.explicitSelection || el.omnibox.value === omniTyped)) {
      activateRow();
      return;
    }
    const text = el.omnibox.value;
    if (!text.trim()) return;
    invoke('shell_omnibox', { operation: 'activate', text });
    closeDropdown();
    el.omnibox.blur();
  }

  function closeDropdown(invalidate = true) {
    if (invalidate) { ++omniSeq; clearTimeout(omniTimer); }
    state.omnibox.open = false;
    el.dropdown.hidden = true;
    el.omnibox.setAttribute('aria-expanded', 'false');
  }

  // ── focus model (§3.6) ───────────────────────────────────────────────
  function focusPage() {
    closeDropdown();
    el.omnibox.blur();
    // The host moves OS focus back to the active page webview.
    invoke('shell_ui', { operation: 'page_focus' }).catch(() => {});
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
    closeMonitorPicker();
    const isCanvas = !!(t && t.canvas);
    const items = custom || (isCanvas ? [
      { label: 'Zoom focused page', run: () => invoke('shell_canvas', { operation: 'zoom', canvas_id: t.id }) },
      ...GRIDS.map(([c, r]) => ({
        label: `Layout ${c} × ${r}`, run: () => invoke('shell_canvas', { operation: 'layout_set', canvas_id: t.id, cols: c, rows: r }),
      })),
      { sep: true },
      { label: 'Close canvas', run: () => invoke('shell_tab', { operation: 'close', tab_id: t.id }), danger: true },
    ] : [
      { label: 'Reload tab', run: () => invoke('shell_page_navigation', { operation: 'reload', tab_id: t.id }) },
      { label: t.pinned ? 'Unpin tab' : 'Pin tab', run: () => invoke('shell_tab', { operation: t.pinned ? 'unpin' : 'pin', tab_id: t.id }) },
      { label: 'Duplicate tab', run: () => invoke('shell_tab', { operation: 'duplicate' }) },
      { label: 'Copy URL', run: () => navigator.clipboard.writeText(t.url).catch(() => {}), },
      { sep: true },
      { label: 'Close tab', run: () => invoke('shell_tab', { operation: 'close', tab_id: t.id }), danger: true },
      { label: 'Close others', run: () => state.tabs.filter((o) => o.id !== t.id).forEach((o) => invoke('shell_tab', { operation: 'close', tab_id: o.id })) },
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
    invoke('shell_ui', { operation: 'menu_set', open: true });
    el.itemMenu.style.left = `${Math.min(x, window.innerWidth - 220)}px`;
    el.itemMenu.style.top = `${Math.min(y, window.innerHeight - 200)}px`;
    const first = el.itemMenu.querySelector('.menu-item');
    if (first) first.focus();
  }

  function closeItemMenu() {
    if (!el.itemMenu.hidden) invoke('shell_ui', { operation: 'menu_set', open: false });
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

  // Monitor slots are explicit user assignments, not inferred desktop ordering.
  let monitorDraft = null;
  let monitorSaving = false;
  let monitorTopology = '';
  const monitorPicker = $('#monitor-picker');
  function renderMonitorPicker() {
    if (!monitorDraft || monitorPicker.hidden) return;
    const displays = state.window?.displays || [];
    const { cols, rows, display_ids: ids } = monitorDraft;
    $('#monitor-layout').value = `${cols}x${rows}`;
    const grid = $('#monitor-slots');
    const focusedSlot = document.activeElement?.id;
    grid.replaceChildren();
    grid.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
    const labels = rows === 1 ? ['Left', 'Right'] : cols === 1 ? ['Top', 'Bottom'] : ['Top left', 'Top right', 'Bottom left', 'Bottom right'];
    for (let i = 0; i < cols * rows; i++) {
      const label = document.createElement('label');
      label.className = 'monitor-slot' + (ids[i] ? ' is-assigned' : '');
      const title = document.createElement('span');
      title.textContent = labels[i];
      const select = document.createElement('select');
      select.id = `monitor-slot-${i}`;
      select.setAttribute('aria-label', `${labels[i]} monitor`);
      select.appendChild(new Option('Choose monitor', ''));
      for (const display of displays) {
        if (!display.id) continue;
        const option = new Option(`Monitor ${display.number} - ${display.resolution.join(' x ')} (${Math.round(display.scale * 100)}%)`, display.id);
        option.title = display.name;
        select.appendChild(option);
      }
      if (ids[i] && !displays.some(display => display.id === ids[i])) {
        select.appendChild(new Option('Disconnected monitor', ids[i]));
      }
      select.value = ids[i] || '';
      select.disabled = monitorSaving;
      select.addEventListener('change', () => { monitorDraft.display_ids[i] = select.value; renderMonitorPicker(); });
      label.append(title, select);
      grid.appendChild(label);
    }
    const complete = ids.length === cols * rows && ids.every(id => id && displays.some(display => display.id === id));
    const unique = new Set(ids).size === ids.length;
    $('#monitor-picker-error').textContent = !complete ? 'Choose a connected monitor for every slot.' : !unique ? 'Choose a different monitor for each slot.' : '';
    $('#monitor-picker-apply').disabled = !complete || !unique || monitorSaving;
    $('#monitor-layout').disabled = monitorSaving;
    if (focusedSlot?.startsWith('monitor-slot-')) document.getElementById(focusedSlot)?.focus();
  }

  async function openMonitorPicker() {
    if (!monitorPicker.hidden) return;
    closeItemMenu();
    state.window = await invoke('shell_window', { operation: 'state_get' });
    const saved = state.window.selected_layout;
    monitorDraft = saved ? { ...saved, display_ids: [...saved.display_ids] } : { cols: 2, rows: 1, display_ids: ['', ''] };
    monitorSaving = false;
    await invoke('shell_ui', { operation: 'menu_set', open: true });
    monitorPicker.hidden = false;
    $('#monitor-picker-backdrop').hidden = false;
    $('#win-span-menu').setAttribute('aria-expanded', 'true');
    monitorTopology = JSON.stringify(state.window.displays);
    renderMonitorPicker();
    $('#monitor-slot-0').focus();
  }

  function closeMonitorPicker() {
    if (monitorPicker.hidden || monitorSaving) return;
    monitorPicker.hidden = true;
    $('#monitor-picker-backdrop').hidden = true;
    $('#win-span-menu').setAttribute('aria-expanded', 'false');
    invoke('shell_ui', { operation: 'menu_set', open: false });
    $('#win-span-menu').focus();
  }

  $('#monitor-layout').addEventListener('change', e => {
    const [cols, rows] = e.target.value.split('x').map(Number);
    monitorDraft = { cols, rows, display_ids: Array.from({ length: cols * rows }, (_, i) => monitorDraft.display_ids[i] || '') };
    renderMonitorPicker();
  });
  $('#monitor-picker-cancel').addEventListener('click', closeMonitorPicker);
  $('#monitor-picker-backdrop').addEventListener('click', closeMonitorPicker);
  $('#monitor-picker-apply').addEventListener('click', async () => {
    monitorSaving = true;
    renderMonitorPicker();
    try {
      await invoke('shell_window', { ...monitorDraft, operation: 'span_select' });
      monitorSaving = false;
      closeMonitorPicker();
    } catch (error) {
      monitorSaving = false;
      renderMonitorPicker();
      $('#monitor-picker-error').textContent = error.message || String(error);
    }
  });
  monitorPicker.addEventListener('keydown', e => {
    if (e.key === 'Escape') { e.preventDefault(); closeMonitorPicker(); }
    if (e.key === 'Tab') {
      const controls = [...monitorPicker.querySelectorAll('select:not(:disabled),button:not(:disabled)')];
      const next = e.shiftKey ? controls.at(-1) : controls[0];
      if (document.activeElement === (e.shiftKey ? controls[0] : controls.at(-1))) { e.preventDefault(); next?.focus(); }
    }
  });

  // ── inline page search ──────────────────────────────────────────────
  let findSeq = 0;
  let findTarget = '';
  let findComposing = false;
  const searchedTabs = new Set();
  function findQuery(kind) {
    const q = el.findInput.value;
    const seq = ++findSeq;
    const tab = activeTab();
    findTarget = tab ? `${tab.id}:${tab.url}` : '';
    if (tab) {
      if (q) searchedTabs.add(tab.id);
      else searchedTabs.delete(tab.id);
    }
    invoke('shell_page_find', { operation: kind || 'query', query: q, highlight_all: true })
      .then((r) => { if (seq === findSeq && r) renderFind(r); })
      .catch((e) => console.error('find', e));
  }

  function renderFind(r) {
    el.findInput.setAttribute('aria-invalid', String(!!el.findInput.value && !r.total));
    el.findInput.title = r.error || (el.findInput.value ? `${r.active || 0}/${r.total || 0}` : 'Find in page (Ctrl+F)');
  }

  function openFind(query) {
    if (typeof query === 'string') el.findInput.value = query;
    el.findInput.focus();
    el.findInput.select();
    findQuery('start');
  }

  function closeFind() {
    ++findSeq;
    el.findInput.value = '';
    el.findInput.removeAttribute('aria-invalid');
    el.findInput.title = 'Find in page (Ctrl+F)';
    searchedTabs.delete(activeTab()?.id);
    invoke('shell_page_find', { operation: 'stop' }).catch(() => {});
  }

  function refreshFindTarget() {
    const tab = activeTab();
    if (!tab || tab.loading) { findTarget = ''; return; }
    if (findComposing) return;
    if (findTarget !== `${tab.id}:${tab.url}` && (el.findInput.value || searchedTabs.has(tab.id))) findQuery();
  }
  el.findInput.addEventListener('input', () => { if (!findComposing) findQuery(); });
  el.findInput.addEventListener('compositionstart', () => { findComposing = true; });
  el.findInput.addEventListener('compositionend', () => { findComposing = false; findQuery(); });
  el.findInput.addEventListener('keydown', (e) => {
    if (e.isComposing || findComposing) return;
    if (e.key === 'Enter') { e.preventDefault(); findQuery(e.shiftKey ? 'previous' : 'next'); }
    else if (e.key === 'Escape') { e.preventDefault(); closeFind(); focusPage(); }
  });

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
      if (d.state !== 'in_progress') mk('Open', false, () => invoke('shell_download', { download_id: d.id, operation: 'open' }));
      mk('Show in folder', false, () => invoke('shell_download', { download_id: d.id, operation: 'reveal' }));
      mk('Remove', true, () => invoke('shell_download', { download_id: d.id, operation: 'entry_remove' }));
      li.addEventListener('contextmenu', (e) => { e.preventDefault(); openDownloadMenu(d, e.clientX, e.clientY); });
      li.addEventListener('keydown', (e) => downloadKey(e, li, i));
      el.downloadsList.appendChild(li);
    });
    el.downloadsClear.disabled = state.downloads.length === 0;
  }

  function openDownloadMenu(d, x, y) {
    const items = [
      { label: 'Open', run: () => invoke('shell_download', { download_id: d.id, operation: 'open' }) },
      { label: 'Show in folder', run: () => invoke('shell_download', { download_id: d.id, operation: 'reveal' }) },
      { sep: true },
      { label: 'Remove from list', danger: true, run: () => invoke('shell_download', { download_id: d.id, operation: 'entry_remove' }) },
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
    invoke('shell_ui', { operation: 'menu_set', open: true });
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
        invoke('shell_download', { download_id: d.id, operation: e.altKey ? 'reveal' : 'open' }).catch(() => {});
      }
    } else if (e.key === 'Shift+F10' || e.key === 'ContextMenu') {
      e.preventDefault();
      const d = state.downloads.find((x) => String(x.id) === li.dataset.id);
      const r = li.getBoundingClientRect();
      if (d) openDownloadMenu(d, r.left + 16, r.top + r.height);
    }
  }

  el.downloadsClear.addEventListener('click', () => invoke('shell_download', { operation: 'list_clear' }));
  el.downloadsClose.addEventListener('click', () => invoke('shell_ui', { operation: 'shortcut', name: 'escape' }));

  // ── toolbars ─────────────────────────────────────────────────────────
  el.back.addEventListener('click', () => invoke('shell_page_navigation', { operation: 'back' }));
  el.forward.addEventListener('click', () => invoke('shell_page_navigation', { operation: 'forward' }));
  el.reload.addEventListener('click', () => invoke('shell_page_navigation', { operation: 'reload' }));
  el.star.addEventListener('click', () => invoke('shell_bookmark', { operation: 'toggle' }));
  el.downloadsBtn.addEventListener('click', () => invoke('shell_ui', { operation: 'shortcut', name: 'open_downloads' }));
  document.getElementById('btn-ai').addEventListener('click', () => invoke('shell_ui', { operation: 'shortcut', name: 'ask_ai' }));
  el.settings.addEventListener('click', () => invoke('shell_settings', { operation: 'open' }));
  el.newTab.addEventListener('click', () => invoke('shell_tab', { operation: 'open' }));
  el.newCanvas.addEventListener('click', () => {
    const r = el.newCanvas.getBoundingClientRect();
    openItemMenu(null, r.left, r.bottom + 2, GRIDS.map(([c, r2]) => ({
      label: `${c} × ${r2} canvas  (${c * r2} pages)`, run: () => invoke('shell_canvas', { operation: 'open', cols: c, rows: r2 }),
    })));
  });
  $('#cv-zoom').addEventListener('click', () => invoke('shell_canvas', { operation: 'zoom' }));
  $('#cv-close').addEventListener('click', () => state.canvas && invoke('shell_canvas', { operation: 'cell_close', cell: state.canvas.focus }));
  $('#cv-layout').addEventListener('click', (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    if (state.canvas) openItemMenu({ canvas: true, id: state.canvas.id }, r.left - 140, r.bottom);
  });
  el.railToggle.addEventListener('click', () => invoke('shell_ui', { operation: 'tab_rail_toggle' }));
  $('#win-min').addEventListener('click', () => invoke('shell_window', { operation: 'minimize' }));
  $('#win-max').addEventListener('click', () => invoke('shell_window', { operation: 'maximize_toggle' }));
  $('#win-span').addEventListener('click', async () => {
    try {
      const result = await invoke('shell_window', { operation: 'span_toggle' });
      if (result?.needs_selection) await openMonitorPicker();
    } catch (error) { toast(error.message || String(error)); }
  });
  $('#win-span-menu').addEventListener('click', () => openMonitorPicker().catch(error => toast(error.message || String(error))));
  $('#win-close').addEventListener('click', () => invoke('shell_window', { operation: 'close' }));
  const dragArea = $('#win-drag');
  dragArea.addEventListener('mousedown', (e) => { if (e.button === 0 && e.detail < 2) invoke('shell_window', { operation: 'drag' }); });
  dragArea.addEventListener('dblclick', () => invoke('shell_window', { operation: 'maximize_toggle' }));
  for (const edge of document.querySelectorAll('.win-edge')) {
    edge.addEventListener('mousedown', (e) => { if (e.button === 0) invoke('shell_window', { operation: 'resize', dir: edge.dataset.dir }); });
  }
  for (const [id, name] of [['zoom-in', 'zoom_in'], ['zoom-out', 'zoom_out'], ['zoom-label', 'zoom_reset'], ['zoom-fit', 'zoom_fit']]) {
    $('#' + id).addEventListener('click', () => invoke('shell_ui', { operation: 'shortcut', name }));
  }

  el.omniboxForm.addEventListener('submit', (e) => { e.preventDefault(); submitOmnibox(); });
  el.omnibox.addEventListener('input', omniboxChanged);
  el.omnibox.addEventListener('compositionstart', () => {
    omniComposing = true; ++omniSeq; clearTimeout(omniTimer);
  });
  el.omnibox.addEventListener('compositionend', () => {
    omniComposing = false; omniboxChanged({ inputType: 'insertText' });
  });
  el.omnibox.addEventListener('focus', () => {
    state.focusedInput = true;
    queryOmnibox(el.omnibox.value);
  });
  el.omnibox.addEventListener('blur', () => {
    state.focusedInput = false;
    setTimeout(() => { if (document.activeElement !== el.omnibox) closeDropdown(); }, 100);
    updateChrome();
  });
  el.omnibox.addEventListener('keydown', (e) => {
    if (e.isComposing || omniComposing) return;
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
  on('state.window', (p) => {
    if (!p) return;
    state.window = p;
    document.documentElement.style.setProperty('--caption-controls-width', p.span_supported ? '206px' : '140px');
    const span = $('#win-span');
    $('#win-span-group').hidden = !p.span_supported;
    span.disabled = !p.can_span && !p.spanning;
    span.setAttribute('aria-pressed', String(!!p.spanning));
    span.title = p.spanning ? 'Restore previous window' : !p.can_span ? 'Connect a second display to span monitors' : p.needs_selection ? 'Choose monitors to span' : 'Span selected monitors';
    span.setAttribute('aria-label', p.spanning ? 'Restore previous window' : 'Span monitors');
    $('#win-max').setAttribute('aria-label', p.maximized ? 'Restore' : 'Maximize');
    $('#win-max').title = p.maximized ? 'Restore' : 'Maximize';
    const topology = JSON.stringify(p.displays);
    if (!monitorPicker.hidden && topology !== monitorTopology) { monitorTopology = topology; renderMonitorPicker(); }
  });
  on('app.ready', () => {
    invoke('shell_diagnostics', { operation: 'self_test' }).catch(() => {});
  });

  on('state.tabs', (p) => {
    if (!p) return;
    if (p.tabs) state.tabs = p.tabs;
    if (p.groups) state.groups = p.groups;
    if (p.active_tab_id !== undefined) state.activeTabId = p.active_tab_id;
    if (p.canvas !== undefined) state.canvas = p.canvas;
    renderTabs();
    refreshFindTarget();
  });

  on('state.groups', (p) => {
    if (p) state.groups = p;
    renderTabs();
  });

  on('tab.created', () => { /* full state follows in state.tabs */ });
  on('tab.closed', p => searchedTabs.delete(p?.tab_id));
  on('tab.activated', (p) => {
    if (p && p.tabs) {
      state.tabs = p.tabs.tabs || state.tabs;
      state.groups = p.tabs.groups || state.groups;
    }
    updateChrome();
    refreshFindTarget();
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
    if (r && r.closed) { renderFind({}); return; }
    renderFind(r || {});
  });
  on('shell.focus_changed', (p) => {
    if (!p) return;
    if (p.scope === 'omnibox') focusOmnibox(!!p.select_all);
    else if (p.scope === 'find') openFind();
    else if (p.scope === 'cycle') cycleFocus();
  });
  on('shell.focus_find', (p) => openFind(p && p.query));
  on('shell.panel', (p) => {
    const panel = p && p.panel;
    el.panel.hidden = panel !== 'downloads';
    if (panel === 'downloads') { renderDownloads(); const first = el.downloadsList.querySelector('.download-row'); if (first) first.focus(); }
  });
  on('shell.find_action', (p) => {
    if (!p) return;
    if (p.action === 'find_next') findQuery('next');
    else if (p.action === 'find_prev') findQuery('previous');
  });
  on('shell.shortcut', (p) => {
    if (!p) return;
    switch (p.name) {
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
          if (inp.value.length === 0) invoke('shell_ui', { operation: 'shortcut', name: 'close_tab' });
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
      if (document.activeElement.value.length === 0) invoke('shell_ui', { operation: 'shortcut', name: 'close_tab' });
      else deleteWord(document.activeElement);
    }
  });
  on('shell.copy', (p) => {
    if (p && p.text) navigator.clipboard.writeText(p.text).then(() => toast('URL copied')).catch(() => toast('Copy failed'));
  });
  on('shell.escape', () => {
    if (!monitorPicker.hidden) { closeMonitorPicker(); return; }
    if (!el.itemMenu.hidden) { closeItemMenu(); return; }
    if (el.findInput.value) { closeFind(); return; }
    if (state.omnibox.open) { closeDropdown(); return; }
  });
  on('shell.toast', (p) => { if (p && p.text) toast(p.text); });
  on('shell.command_done', () => closeDropdown());
  on('shell.ui_scale', () => { renderCanvas(); });
  on('shell.layout', (p) => {
    if (!p) return;
    document.documentElement.style.setProperty('--rail-w', `${p.rail_w}px`);
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
    if (!document.hasFocus()) {
      invoke('shell_ui', { operation: 'shortcut', name: 'focus_omnibox' }).catch(() => {});
      return;
    }
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
  on('mcp.connected', () => void invoke('shell_ui', { operation: 'ready' }).then(() => {
    // W1: report the first animation frame after the shell has painted.
    requestAnimationFrame(() => {
      const t = performance.now();
      invoke('shell_diagnostics', { operation: 'self_test' }).then(() => console.info(`[shell] first paint at ${t.toFixed(1)} ms (document)`)).catch(() => {});
    });
  }).catch((e) => console.error('shell_ui ready', e)));
  void shell.ready.catch((e) => console.error('shell MCP connection', e));

  window.addEventListener('resize', () => {
    // The host owns geometry; this listener exists only so the dropdown
    // mirror re-measures after a resize.
    if (!el.dropdown.hidden) renderDropdown();
  });
})();