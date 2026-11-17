(() => {
  try {
    const clean = s => String(s || '').replace(/[ \t\u00a0]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
    const visible = e => {
      const r = e.getBoundingClientRect(), s = getComputedStyle(e);
      return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
    };
    const secret = e => e.type === 'password' ||
      /(?:^|\s)(?:cc-\S+|one-time-code|new-password|current-password)(?:\s|$)/i.test(e.autocomplete || '');
    const selector = e => {
      if (e.id && document.querySelectorAll('#' + CSS.escape(e.id)).length === 1) return '#' + CSS.escape(e.id);
      const parts = [];
      for (let n = e; n && n.nodeType === 1 && n !== document.body; n = n.parentElement) {
        let part = n.tagName.toLowerCase();
        if (n.parentElement) {
          const siblings = [...n.parentElement.children].filter(c => c.tagName === n.tagName);
          if (siblings.length > 1) part += ':nth-of-type(' + (siblings.indexOf(n) + 1) + ')';
        }
        parts.unshift(part);
      }
      return 'body>' + parts.join('>');
    };
    const label = e => clean(e.labels && e.labels[0] ? e.labels[0].innerText :
      (e.getAttribute('aria-label') || e.placeholder || e.innerText || e.name || e.id));
    const value = e => secret(e) ? '(hidden)' : e.tagName === 'SELECT' ?
      [...e.selectedOptions].map(o => clean(o.text)).join(', ') :
      /^(checkbox|radio)$/.test(e.type) ? (e.checked ? 'checked' : 'unchecked') : String(e.value || '');
    const allText = clean(document.body ? document.body.innerText : '');
    const main = document.querySelector('article,main,[role=main]');
    const mainText = main ? clean(main.innerText) : '';
    const text = mainText.length >= allText.length * 0.3 ? mainText : allText;
    const controls = [...document.querySelectorAll('input,textarea,select,button,[role=button]')]
      .filter(e => !['hidden', 'reset', 'image'].includes(e.type) && visible(e));
    const fields = controls.slice(0, 60).map(e => ({
      tag: e.tagName.toLowerCase(), type: e.type || '', label: label(e).slice(0, 80),
      name: (e.name || '').slice(0, 80), value: value(e).slice(0, 200),
      selector: selector(e), focused: e === document.activeElement
    }));
    const forms = fields.filter(e => ['input', 'textarea', 'select'].includes(e.tag) &&
      !['submit', 'button'].includes(e.type));
    const anchors = [...document.querySelectorAll('a[href]')].filter(visible);
    const links = anchors.slice(0, 80).map(a => ({
      text: label(a).slice(0, 100), href: a.href, selector: selector(a)
    })).filter(a => a.text && /^https?:/.test(a.href));
    const meta = document.querySelector('meta[name=description],meta[property="og:description"]');
    return JSON.stringify({
      title: document.title, url: location.href, ready: document.readyState,
      text: text.slice(0, 60000), description: meta ? clean(meta.content).slice(0, 1000) : '',
      headings: [...document.querySelectorAll('h1,h2,h3')].filter(visible).slice(0, 30)
        .map(h => clean(h.innerText).slice(0, 200)), forms, fields, links,
      coverage: { textChars: text.length, textTruncated: text.length > 60000,
        omittedControls: Math.max(0, controls.length - 60), omittedLinks: Math.max(0, anchors.length - 80),
        unreadFrames: document.querySelectorAll('iframe,frame').length }
    });
  } catch (e) { return JSON.stringify({ error: String(e) }); }
})()
