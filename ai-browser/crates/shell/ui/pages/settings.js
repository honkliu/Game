// Settings page (§4.8), rendered in an unprivileged page webview.
//
// Only the embedded settings document can send these bounded settings messages.
// It never receives the privileged shell command bridge.

(() => {
  'use strict';

  function readFragment() {
    const out = {};
    try {
      for (const [k, v] of new URLSearchParams(location.hash.replace(/^#/, ''))) out[k] = v;
    } catch { /* ignore */ }
    return out;
  }

  const frag = readFragment();

  // The host appends the current settings to the fragment before navigation,
  // so a reload still shows the true state; without it the defaults are shown
  // and the first write corrects them.
  const initial = (() => {
    try {
      const raw = frag.state;
      if (!raw) return null;
      const b64 = raw.replace(/-/g, '+').replace(/_/g, '/');
      return JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))));
    } catch {
      return null;
    }
  })();

  function applyInitial(s) {
    if (!s) return;
    for (const r of document.querySelectorAll('input[name="theme"]')) {
      r.checked = r.value === (s.theme || 'system');
    }
    const set = (id, v) => { const e = document.getElementById(id); if (e) e.checked = !!v; };
    set('rail_collapsed', s.rail_collapsed);
    set('compact_tab_rows', s.compact_tab_rows);
    set('clear_history_on_exit', s.clear_history_on_exit);
    set('clear_cookies_on_exit', s.clear_cookies_on_exit);
    set('clear_cache_on_exit', s.clear_cache_on_exit);
    set('always_show_full_url', s.always_show_full_url);
    const density = document.getElementById('density');
    if (density) density.value = s.density || 'comfortable';
    const tplEl = document.getElementById('search_template');
    if (tplEl) tplEl.value = s.search_template || tplEl.value;
    const cap = document.getElementById('history_cap');
    if (cap) cap.value = s.history_cap ?? '';
    const sleep = document.getElementById('sleep_after_minutes');
    if (sleep) sleep.value = s.sleep_after_minutes ?? '';
    const dir = document.getElementById('download_dir');
    if (dir) dir.value = s.download_dir || '';
    const count = document.getElementById('history_count');
    if (count) count.textContent = s.history_count >= 0 ? String(s.history_count) : 'unavailable';
    const build = document.getElementById('build');
    if (build) build.textContent = `${s.engine_name || 'WebView2'} runtime: ${s.webview2_version || 'unknown'}`;
    const data = document.getElementById('data_note');
    if (data) data.textContent = s.data_dir || '%LOCALAPPDATA%\\native-browser';
    document.documentElement.dataset.theme =
      (s.theme === 'system' ? (s.system_dark ? 'dark' : 'light') : s.theme) || 'light';
  }

  applyInitial(initial);

  const resultEl = document.createElement('p');
  resultEl.setAttribute('role', 'status');
  document.querySelector('main').appendChild(resultEl);
  window.__settingsResult = (result) => {
    resultEl.textContent = result.ok ? 'Saved.' : (result.error || 'Could not save setting.');
    if (result.ok) {
      applyInitial(result.settings);
      const bytes = new TextEncoder().encode(JSON.stringify(result.settings));
      const encoded = btoa(Array.from(bytes, c => String.fromCharCode(c)).join(''))
        .replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
      history.replaceState(null, '', '#state=' + encoded);
    }
  };

  function send(key, value) {
    try {
      resultEl.textContent = 'Saving...';
      window.ipc.postMessage(JSON.stringify({ ai: 'settings', key, value }));
    } catch (error) {
      resultEl.textContent = 'Settings bridge is unavailable. Reopen Settings and try again.';
    }
  }

  for (const r of document.querySelectorAll('input[name="theme"]')) {
    r.addEventListener('change', () => { if (r.checked) send('theme', r.value); });
  }
  for (const id of [
    'rail_collapsed',
    'compact_tab_rows',
    'clear_history_on_exit',
    'clear_cookies_on_exit',
    'clear_cache_on_exit',
    'always_show_full_url',
  ]) {
    const e = document.getElementById(id);
    e.addEventListener('change', () => send(id, e.checked));
  }
  document.getElementById('density').addEventListener('change', (e) => send('density', e.target.value));

  const tplInput = document.getElementById('search_template');
  tplInput.addEventListener('change', () => {
    if (!tplInput.value.includes('%s')) {
      tplInput.setAttribute('aria-invalid', 'true');
      tplInput.title = 'The template must contain %s';
      return;
    }
    tplInput.removeAttribute('aria-invalid');
    send('search_template', tplInput.value);
  });

  for (const id of ['history_cap', 'sleep_after_minutes']) {
    const e = document.getElementById(id);
    e.addEventListener('change', () => send(id, Number(e.value)));
  }
  document.getElementById('download_dir').addEventListener('change', (e) => send('download_dir', e.target.value));

  document.getElementById('open_folder').addEventListener('click', () => {
    send('open_folder', true);
  });
})();