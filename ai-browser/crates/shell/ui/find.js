function (query, matchCase, wholeWord, highlightAll, index, sequence) {
  try {
    if (window.__pdfFind) {
      window.__pdfFind(query, matchCase, wholeWord, highlightAll, index).then(result => {
        window.ipc.postMessage(JSON.stringify({ ai: 'find', sequence, result }));
      }).catch(error => {
        window.ipc.postMessage(JSON.stringify({ ai: 'find', sequence,
          result: { total: 0, active: 0, error: String(error), supported_flags: [] } }));
      });
      return JSON.stringify({ pending: true });
    }
    if (window.CSS && CSS.highlights) {
      CSS.highlights.delete('browser-find');
      CSS.highlights.delete('browser-find-active');
    }
    if (!document.getElementById('__browser_find_style')) {
      const style = document.createElement('style');
      style.id = '__browser_find_style';
      style.setAttribute('data-ai-probe', '');
      style.textContent = '::highlight(browser-find){background:#ffe27a;color:#161616}::highlight(browser-find-active){background:#ff9933;color:#161616}';
      (document.head || document.documentElement).append(style);
    }
    const nodes = [], offsets = [];
    let text = '', limited = false, previousBlock = null;
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const el = node.parentElement;
        if (!el || el.closest('script,style,noscript,input,textarea,[hidden],[aria-hidden=true],[data-ai-probe]'))
          return NodeFilter.FILTER_REJECT;
        const s = getComputedStyle(el);
        return s.visibility !== 'hidden' && el.getClientRects().length ?
          NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
      }
    });
    let node;
    while ((node = walker.nextNode())) {
      let block = node.parentElement;
      while (block && getComputedStyle(block).display === 'inline') block = block.parentElement;
      if (previousBlock && block !== previousBlock) text += '\n';
      previousBlock = block;
      offsets.push(text.length); nodes.push(node); text += node.data;
      if (text.length > 2000000 || nodes.length >= 20000) { limited = true; break; }
    }
    const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const re = new RegExp(escaped, matchCase ? 'gu' : 'giu');
    const isWord = s => /[\p{L}\p{N}_]/u.test(s);
    const ranges = [];
    if (query) {
      let m, from = 0;
      while ((m = re.exec(text))) {
        const end = m.index + m[0].length;
        if (wholeWord && (isWord(text.slice(Math.max(0, m.index - 2), m.index).replace(/^.*?([\s\S])$/u, '$1')) ||
            isWord(text.slice(end, end + 2).replace(/^([\s\S]).*$/u, '$1')))) continue;
        while (from + 1 < nodes.length && offsets[from + 1] <= m.index) from++;
        let to = from;
        while (to + 1 < nodes.length && offsets[to] + nodes[to].length < end) to++;
        if (m.index - offsets[from] > nodes[from].length || end - offsets[to] > nodes[to].length) continue;
        const range = document.createRange();
        range.setStart(nodes[from], m.index - offsets[from]);
        range.setEnd(nodes[to], end - offsets[to]);
        ranges.push(range);
        if (ranges.length >= 5000) { limited = true; break; }
      }
    }
    const selected = ranges.length ? ((index % ranges.length) + ranges.length) % ranges.length : -1;
    if (selected >= 0) {
      if (window.CSS && CSS.highlights && window.Highlight) {
        if (highlightAll) CSS.highlights.set('browser-find', new Highlight(...ranges));
        CSS.highlights.set('browser-find-active', new Highlight(ranges[selected]));
      } else {
        const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(ranges[selected]);
      }
      const rect = ranges[selected].getBoundingClientRect();
      window.scrollBy(0, rect.top - innerHeight / 3);
    }
    return JSON.stringify({ total: ranges.length, active: selected + 1, limited,
      supported_flags: ['case_sensitive', 'whole_word'].concat(window.CSS && CSS.highlights ? ['highlight_all'] : []) });
  } catch (e) { return JSON.stringify({ total: 0, active: 0, error: String(e), supported_flags: [] }); }
}
