// New-tab page (§4.1). Runs in a page webview with no host bridge, so all data
// arrives through the URL fragment the host sets before loading this page:
//   asset://shell/pages/newtab.html#d=<base64url(json)>
// The fragment keeps the payload out of any request and works without IPC.

(() => {
  'use strict';

  function decode() {
    try {
      const m = location.hash.match(/[#&]d=([^&]+)/);
      if (!m) return null;
      const b64 = m[1].replace(/-/g, '+').replace(/_/g, '/');
      const bin = atob(b64);
      const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
      return JSON.parse(new TextDecoder().decode(bytes));
    } catch {
      return null;
    }
  }

  function tick() {
    const now = new Date();
    document.getElementById('clock').firstChild.nodeValue =
      now.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
    document.getElementById('date').textContent =
      now.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' });
  }

  const data = decode();

  tick();
  setInterval(tick, 15000);

  const tiles = document.getElementById('tiles');
  const empty = document.getElementById('tiles-empty');
  const bookmarks = (data && data.bookmarks) || [];
  if (!bookmarks.length) tiles.hidden = true;
  else {
    empty.hidden = true;
    for (const b of bookmarks.slice(0, 8)) {
      const a = document.createElement('a');
      a.className = 'tile';
      a.href = b.url;
      a.title = b.url;
      const dot = document.createElement('span');
      dot.className = 'dot';
      dot.textContent = (b.title || b.url).trim().charAt(0).toUpperCase() || '·';
      const name = document.createElement('span');
      name.className = 'name';
      name.textContent = b.title || b.url;
      a.append(dot, name);
      tiles.appendChild(a);
    }
  }

  const recent = document.getElementById('recent');
  const entries = (data && data.recent) || [];
  for (const e of entries.slice(0, 6)) {
    const li = document.createElement('li');
    const title = document.createElement('a');
    title.className = 'rt';
    title.href = e.url;
    title.textContent = e.title || e.url;
    title.style.color = 'inherit';
    title.style.textDecoration = 'none';
    title.style.flex = '1 1 auto';
    title.style.overflow = 'hidden';
    title.style.textOverflow = 'ellipsis';
    const host = document.createElement('span');
    host.className = 'rh';
    host.textContent = e.host;
    li.append(title, host);
    recent.appendChild(li);
  }
  if (!entries.length) {
    const li = document.createElement('li');
    li.className = 'empty';
    li.textContent = 'Nothing visited yet.';
    recent.appendChild(li);
  }

  // The "Show all history" link is a real link to the host's history surface.
  // A page webview cannot ask the host to open a panel, so this link points at
  // the shell's own history entry point via the omnibox command prefix shape;
  // without a bridge the honest option is to keep it as a no-op label.
  const showAll = document.getElementById('show-all');
  showAll.addEventListener('click', (e) => e.preventDefault());
  showAll.setAttribute('title', 'Press Ctrl+H to open history');
  showAll.textContent = 'Show all history (Ctrl+H)';
})();