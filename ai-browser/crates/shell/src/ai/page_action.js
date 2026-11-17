function (action, selector, text, value) {
  try {
    const visible = e => {
      const r = e.getBoundingClientRect(), s = getComputedStyle(e);
      return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
    };
    const label = e => (e.innerText || e.value || e.getAttribute('aria-label') || '').trim().toLowerCase();
    let matches;
    if (selector) {
      matches = [...document.querySelectorAll(selector)].filter(visible);
      if (text) matches = matches.filter(e => label(e) === text.trim().toLowerCase());
    } else {
      const all = [...document.querySelectorAll('a,button,[role=button],input[type=submit],input[type=button],summary')]
        .filter(visible);
      const want = (text || '').trim().toLowerCase();
      matches = want ? all.filter(e => label(e) === want) : [];
      if (want && matches.length === 0) matches = all.filter(e => label(e).includes(want));
    }
    if (matches.length !== 1) return JSON.stringify({
      error: matches.length ? 'ambiguous target; read_page and choose a unique selector' : 'no visible target matches',
      matches: matches.length, dispatched: false, confirmed: false
    });
    const el = matches[0];
    if (el.disabled || el.readOnly || el.getAttribute('aria-disabled') === 'true' || el.closest('[inert]'))
      return JSON.stringify({ error: 'the target is disabled or read-only', dispatched: false, confirmed: false });
    el.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'instant' });
    const r = el.getBoundingClientRect();
    const hit = document.elementFromPoint(Math.max(0, Math.min(innerWidth - 1, r.left + r.width / 2)),
      Math.max(0, Math.min(innerHeight - 1, r.top + r.height / 2)));
    if (!hit || !(hit === el || el.contains(hit))) return JSON.stringify({
      error: 'the target is covered; dismiss the overlay or choose another target', dispatched: false, confirmed: false
    });
    if (action === 'click') {
      if (typeof el.click !== 'function') return JSON.stringify({ error: 'the target is not clickable' });
      el.click();
      return JSON.stringify({ action, dispatched: true, confirmed: false, outcome: 'dispatched',
        note: 'A click was dispatched, not proof of navigation, saving, or server-side success. Read the page to verify.',
        tag: el.tagName.toLowerCase(), text: label(el).slice(0, 80) });
    }
    const secret = el.type === 'password' ||
      /(?:^|\s)(?:cc-\S+|one-time-code|new-password|current-password)(?:\s|$)/i.test(el.autocomplete || '');
    if (secret) return JSON.stringify({ error: 'sensitive fields must be filled by the user', dispatched: false, confirmed: false });
    if (!['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) ||
      /^(file|hidden|checkbox|radio|button|submit|reset|image)$/.test(el.type))
      return JSON.stringify({ error: 'this field type is not supported', dispatched: false, confirmed: false });
    el.focus();
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype :
      el.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    const confirmed = el.isConnected && String(el.value) === value;
    return JSON.stringify({ action, dispatched: true, confirmed, outcome: confirmed ? 'field_value_confirmed' : 'unconfirmed',
      note: 'Only the current field value is checked; the form has not been submitted or saved.' });
  } catch (e) { return JSON.stringify({ error: String(e), dispatched: false, confirmed: false }); }
}
