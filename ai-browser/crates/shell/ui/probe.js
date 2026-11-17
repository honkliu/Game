// Injected into every page's top frame. It reports the text selection to the shell
// (so the selection toolbar can appear), tells the shell when the page's content
// changed (so the assistant's context stays live) and draws the citation flash. It
// reads nothing else and exposes nothing the page can use to reach the browser: the shell
// treats every message as untrusted, bounded text.
(function () {
  if (window.top !== window || window.__aiProbe) return;
  window.__aiProbe = true;
  var timer = 0;
  var last = '';

  function post(obj) {
    try { window.ipc.postMessage(JSON.stringify(obj)); } catch (e) {}
  }

  var editing = false;
  var lifecycleTimer = 0;
  function lifecycle() {
    var media = document.querySelectorAll('audio,video');
    var playing = false;
    for (var i = 0; i < media.length; i++) {
      if (!media[i].paused && !media[i].ended) { playing = true; break; }
    }
    post({ ai: 'lifecycle', url: location.href, scroll_y: window.scrollY,
      editing: editing, playing: playing, ready: document.readyState === 'complete' });
  }
  function scheduleLifecycle() {
    clearTimeout(lifecycleTimer);
    lifecycleTimer = setTimeout(lifecycle, 200);
  }
  document.addEventListener('input', function () { editing = true; lifecycle(); }, true);
  document.addEventListener('play', lifecycle, true);
  document.addEventListener('pause', lifecycle, true);
  document.addEventListener('ended', lifecycle, true);
  window.addEventListener('scroll', scheduleLifecycle, { passive: true });
  window.addEventListener('load', lifecycle);
  window.addEventListener('pagehide', lifecycle);
  document.addEventListener('DOMContentLoaded', lifecycle);
  scheduleLifecycle();

  // Identify extensionless PDFs too. The host chooses the platform's viewer and
  // extraction bridge; a PDF's text is not part of this page's ordinary DOM.
  if (document.contentType === 'application/pdf' && /^(https?|file):/.test(location.href)) {
    post({ ai: 'pdf', url: location.href });
    return;
  }

  function report() {
    var sel = window.getSelection();
    var text = sel ? String(sel.toString()) : '';
    if (text.trim().length === 0) {
      if (last !== '') { last = ''; post({ ai: 'sel', text: '', rect: null }); }
      return;
    }
    if (text === last) return;
    last = text;
    var r = sel.getRangeAt(0).getBoundingClientRect();
    post({ ai: 'sel', text: text.slice(0, 4000), rect: [r.left, r.top, r.width, r.height] });
  }

  // Many sites call window.open('') and then set the new window's location. The host
  // cannot hand a script-controlled window to a tab, so the blank case returns a stub
  // whose navigation is forwarded to the host as an ordinary "open this URL" request.
  var realOpen = window.open;
  window.open = function (url, name, features) {
    var u = url == null ? '' : String(url);
    if (u !== '' && u !== 'about:blank') return realOpen.call(window, url, name, features);
    var stub = { closed: false, close: function () { stub.closed = true; }, focus: function () {}, blur: function () {},
      document: { write: function () {}, writeln: function () {}, open: function () {}, close: function () {} },
      opener: null };
    function go(target) {
      try { target = new URL(String(target), location.href).href; } catch (e) { return; }
      post({ ai: 'open', url: target });
    }
    var loc = { assign: go, replace: go, reload: function () {}, toString: function () { return 'about:blank'; } };
    Object.defineProperty(loc, 'href', { get: function () { return 'about:blank'; }, set: go });
    Object.defineProperty(stub, 'location', { get: function () { return loc; }, set: go });
    return stub;
  };

  // The toolbar is a native surface above the page: shown while the button is still held
  // it would sit under the drag and swallow it. So a drag reports once, on release.
  var down = false;
  document.addEventListener('mousedown', function (e) { if (e.button === 0) down = true; }, true);
  document.addEventListener('mouseup', function () {
    if (!down) return;
    down = false;
    clearTimeout(timer);
    timer = setTimeout(report, 30);
  }, true);
  document.addEventListener('mousemove', function (e) { if (down && e.buttons === 0) { down = false; report(); } }, true);
  window.addEventListener('blur', function () { down = false; });

  document.addEventListener('selectionchange', function () {
    clearTimeout(timer);
    if (down) return;
    timer = setTimeout(report, 250);
  });

  // Content changes that never touch the URL: typing in a form, an in-page tab or
  // accordion, a client-side route, new items streaming in. The notice carries no page
  // data, only a reason from a fixed set; the shell decides whether to re-read. It is
  // debounced (350 ms after the last change, at most 1.5 s into a burst) so a ticking
  // page cannot flood the host.
  var notifyTimer = 0;
  var burstStart = 0;
  function notify(reason) {
    var now = Date.now();
    if (!burstStart) burstStart = now;
    clearTimeout(notifyTimer);
    var wait = Math.max(0, Math.min(350, burstStart + 1500 - now));
    notifyTimer = setTimeout(function () {
      burstStart = 0;
      post({ ai: 'changed', reason: reason });
    }, wait);
  }
  function ours(node) {
    for (; node; node = node.parentNode) {
      if (node.nodeType === 1 && node.hasAttribute && node.hasAttribute('data-ai-probe')) return true;
    }
    return false;
  }
  var observer = new MutationObserver(function (records) {
    for (var i = 0; i < records.length; i++) {
      if (!ours(records[i].target)) { notify('dom'); return; }
    }
  });
  var observing = false;
  function observe() {
    if (observing || !document.documentElement) return;
    observing = true;
    observer.observe(document.documentElement, {
      childList: true, subtree: true, characterData: true, attributes: true,
      attributeFilter: ['hidden', 'class', 'style', 'open', 'aria-selected', 'aria-expanded', 'aria-hidden', 'checked', 'disabled'],
    });
  }
  observe();
  document.addEventListener('readystatechange', observe);
  ['input', 'change', 'keyup', 'submit'].forEach(function (name) {
    document.addEventListener(name, function () { notify('input'); }, true);
  });
  document.addEventListener('click', function () { notify('click'); }, true);
  // A click or key press in a page tells the host which canvas cell to focus.
  // Only real user input moves the canvas focus; a page focusing itself (autofocus,
  // scripts, loading) must not steal it from the cell the user picked.
  var focusAt = 0;
  function focusHost(e) {
    if (e && e.isTrusted === false) return;
    var now = Date.now();
    if (now - focusAt < 250) return;
    focusAt = now;
    post({ ai: 'focus' });
  }
  document.addEventListener('mousedown', focusHost, true);
  // Ctrl+wheel (and touchpad pinch, which arrives the same way) zooms the page. The
  // runtime's own gesture is off, so the host applies the zoom to this tab.
  window.addEventListener('wheel', function (e) {
    if (!e.ctrlKey) return;
    e.preventDefault();
    post({ ai: 'zoom', dir: e.deltaY < 0 ? 1 : -1 });
  }, { passive: false, capture: true });
  ['pushState', 'replaceState'].forEach(function (name) {
    var original = history[name];
    history[name] = function () {
      var result = original.apply(this, arguments);
      notify('route');
      return result;
    };
  });
  window.addEventListener('popstate', function () { notify('route'); });
  window.addEventListener('hashchange', function () { notify('route'); });
  window.addEventListener('load', function () { notify('load'); });

  // Document-space rectangle flash, in a closed shadow root so page CSS cannot touch it.
  window.__aiFlash = function (x, y, w, h) {
    try {
      var host = document.createElement('div');
      host.setAttribute('data-ai-probe', '');
      host.style.cssText = 'all:initial;position:absolute;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none';
      var root = host.attachShadow({ mode: 'closed' });
      var box = document.createElement('div');
      box.style.cssText = 'position:absolute;pointer-events:none;border-radius:3px;background:rgba(255,213,79,.45);' +
        'outline:2px solid rgba(255,179,0,.9);transition:opacity .6s ease;left:' + x + 'px;top:' + y + 'px;width:' +
        Math.max(w, 4) + 'px;height:' + Math.max(h, 4) + 'px';
      root.appendChild(box);
      (document.body || document.documentElement).appendChild(host);
      setTimeout(function () { box.style.opacity = '0'; }, 1400);
      setTimeout(function () { host.remove(); }, 2200);
    } catch (e) {}
  };
})();
