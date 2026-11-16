(() => { try {
  const vw = innerWidth, vh = innerHeight;
  const clean = (s) => String(s || '').replace(/\s+/g, ' ').trim();
  const shown = (e) => {
    const r = e.getBoundingClientRect(), s = getComputedStyle(e);
    return r.width > 1 && r.height > 1 && s.display !== 'none' && s.visibility !== 'hidden' && parseFloat(s.opacity) > 0;
  };
  const ident = (e) => `${e.id || ''} ${typeof e.className === 'string' ? e.className : e.getAttribute('class') || ''}`;

  // Something the page itself shows over its content: a dialog, a backdrop, a promo, a cookie
  // banner, an ad. Site chrome (headers, nav bars, search boxes) is never touched.
  const POPUP = /(^|[^a-z])(modal|popup|pop-?up|popwin|popbox|dialog|overlay|mask|lightbox|interstitial|promo|subscribe|newsletter|cookie|consent|gdpr|sponsor|floating|float-?(ad|box|win|layer)|toast|tanchuang|guanggao|app-?(down|banner|guide)|download-?(bar|tip|guide|app)|adv\w*|ads?|adbox|ad-?(wrap|box|layer|slot|banner|float|container)|gg)([^a-z]|$)/i;
  const AD_FRAME = /doubleclick|googlesyndication|googleadservices|adservice|adsystem|adnxs|taboola|outbrain|criteo|pubmatic|2mdn|amazon-adsystem|cpro\.baidu|pos\.baidu|sax\.sina|simba|cbjs|adsrvr|rubiconproject|moatads|mgid|revcontent/i;
  const CHROME = /(^|[^a-z])(header|navbar?|topbar|menubar|toolbar|searchbar|search-?box|masthead|sidebar|player|video)([^a-z]|$)/i;
  const CHROME_ROLE = /^(navigation|banner|contentinfo|search|menubar|toolbar|main|article)$/;
  const CLOSE_TEXT = /^(×|✕|✖|✗|╳|x|close|dismiss|no,? thanks|not now|maybe later|skip|skip ad|got it|reject( all)?|decline|关闭|关 闭|不再提示|以后再说|我知道了|知道了|跳过|不感兴趣|拒绝|暂不)$/i;
  const CLOSE_CLASS = /(^|[^a-z])(close|closer|cls|clos|btn-?close|close-?btn|icon-?close|guanbi|shut|dismiss|cancel|x-?btn|sprite-?close)([^a-z]|$)/i;

  const isChrome = (e) => {
    const tag = e.tagName;
    if (tag === 'BODY' || tag === 'HTML' || tag === 'HEADER' || tag === 'NAV' || tag === 'FOOTER' || tag === 'MAIN' || tag === 'ARTICLE') return true;
    if (CHROME_ROLE.test(e.getAttribute('role') || '')) return true;
    if (e.getAttribute('aria-modal') === 'true' || /dialog/.test(e.getAttribute('role') || '')) return false;
    if (CHROME.test(ident(e))) return true;
    if (e.querySelector('video,main,article,[role=main],input[type=search]')) return true;
    return false;
  };

  const candidates = [];
  const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_ELEMENT);
  for (let e = walker.nextNode(); e; e = walker.nextNode()) {
    if (candidates.some((c) => c.el.contains(e))) continue;
    const s = getComputedStyle(e);
    if (s.position !== 'fixed' && s.position !== 'absolute' && s.position !== 'sticky') continue;
    if (!shown(e)) continue;
    const r = e.getBoundingClientRect();
    if (r.width < 40 || r.height < 30) continue;
    const z = parseInt(s.zIndex, 10) || 0;
    if (s.position === 'absolute' && z < 100) continue;
    if (isChrome(e)) continue;
    const id = ident(e);
    const area = (Math.min(r.right, vw) - Math.max(r.left, 0)) * (Math.min(r.bottom, vh) - Math.max(r.top, 0)) / (vw * vh);
    const modal = e.getAttribute('aria-modal') === 'true' || /dialog/.test(e.getAttribute('role') || '');
    const frame = [...e.querySelectorAll('iframe')].some((f) => AD_FRAME.test(f.src || '')) || (e.tagName === 'IFRAME' && AD_FRAME.test(e.src || ''));
    const backdrop = s.position === 'fixed' && area >= 0.5 && clean(e.innerText).length < 400;
    const named = POPUP.test(id);
    const tiny = r.width < 90 && r.height < 90;
    if (!(modal || frame || backdrop || (named && !tiny))) continue;
    // A fixed element holding most of the page's text is the app itself, not a popup.
    if (clean(e.innerText).length > 2500) continue;
    candidates.push({ el: e, kind: modal ? 'dialog' : frame ? 'ad' : backdrop ? 'backdrop' : /ads?|adv|sponsor|guanggao|gg/i.test(id) ? 'ad' : 'popup', area });
  }

  const closeButton = (root) => {
    const rr = root.getBoundingClientRect();
    const found = [];
    for (const b of root.querySelectorAll('button,a,[role=button],span,i,b,em,div,svg,img,label')) {
      const r = b.getBoundingClientRect();
      if (r.width < 6 || r.height < 6 || r.width > 100 || r.height > 100) continue;
      if (b.children.length > 3) continue;
      const text = clean(b.innerText || b.value || b.getAttribute('aria-label') || b.getAttribute('title') || b.getAttribute('alt'));
      if (CLOSE_TEXT.test(text) || CLOSE_CLASS.test(ident(b))) {
        // Close buttons live in a corner: nearer the top-right ranks first.
        const score = (rr.right - r.right) + (r.top - rr.top);
        found.push({ b, score });
      }
    }
    found.sort((a, b) => a.score - b.score);
    return found.length ? found[0].b : null;
  };

  const fire = (el) => {
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
      el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window }));
    }
  };

  let clicked = 0, hidden = 0;
  const items = [];
  for (const c of candidates) {
    const label = clean(c.el.innerText).slice(0, 40) || (ident(c.el).trim().slice(0, 40)) || c.el.tagName.toLowerCase();
    const b = closeButton(c.el);
    if (b) { fire(b); clicked++; }
    // Hide it regardless: a close handler may remove the popup later (or not at all).
    c.el.style.setProperty('display', 'none', 'important');
    hidden++;
    items.push({ kind: c.kind, label, clicked_close: !!b });
  }

  if (hidden) {
    // A modal usually locks the page's scrolling; give it back.
    for (const el of [document.documentElement, document.body]) {
      if (!el) continue;
      if (getComputedStyle(el).overflow === 'hidden') el.style.setProperty('overflow', 'auto', 'important');
      if (el === document.body && getComputedStyle(el).position === 'fixed') el.style.setProperty('position', 'static', 'important');
    }
  }
  return JSON.stringify({ dismissed: hidden, clicked_close: clicked, items: items.slice(0, 12), url: location.href });
} catch (e) { return JSON.stringify({ error: String(e) }); } })()
