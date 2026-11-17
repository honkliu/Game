// WKWebView extractor, evaluated in a host-owned WKContentWorld. Normalize to the
// shared columnar parser; this is DOM-derived context, not a Chromium CDP snapshot.
(() => {
  const strings = [];
  const indices = new Map();
  function intern(value) {
    if (!indices.has(value)) { indices.set(value, strings.length); strings.push(value); }
    return indices.get(value);
  }
  const nodes = { nodeType: [], nodeName: [], nodeValue: [], parentIndex: [], attributes: [] };
  const layout = { nodeIndex: [], text: [], bounds: [] };
  function node(type, name, value, parent, attributes) {
    const index = nodes.nodeType.length;
    nodes.nodeType.push(type);
    nodes.nodeName.push(intern(name));
    nodes.nodeValue.push(intern(value));
    nodes.parentIndex.push(parent);
    nodes.attributes.push(attributes || []);
    return index;
  }
  const blockTags = new Set(['P', 'LI', 'DD', 'DT', 'TD', 'TH', 'H1', 'H2', 'H3',
    'H4', 'H5', 'H6', 'BLOCKQUOTE', 'PRE', 'FIGCAPTION', 'CAPTION']);
  const skipTags = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'TITLE']);
  const visibility = new Map();
  function visible(element) {
    if (!element) return true;
    if (visibility.has(element)) return visibility.get(element);
    const style = getComputedStyle(element);
    const result = !skipTags.has(element.tagName) && !element.hasAttribute('data-ai-probe') &&
      style.display !== 'none' && style.visibility !== 'hidden' && style.visibility !== 'collapse' &&
      Number(style.opacity) !== 0 && visible(element.parentElement);
    visibility.set(element, result);
    return result;
  }
  const groups = new Map();
  const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
  let current;
  let visited = 0;
  let totalChars = 0;
  let truncated = false;
  while ((current = walker.nextNode())) {
    if (++visited > 50000 || totalChars >= 2000000) { truncated = true; break; }
    const text = current.textContent.replace(/\s+/g, ' ').trim();
    if (!text || !visible(current.parentElement)) continue;
    let owner = current.parentElement;
    while (owner && !blockTags.has(owner.tagName)) owner = owner.parentElement;
    owner = owner || current;
    const range = document.createRange();
    range.selectNodeContents(current);
    const textRect = range.getBoundingClientRect();
    if (textRect.width <= 0 || textRect.height <= 0) continue;
    let group = groups.get(owner);
    if (!group) {
      const rect = owner.nodeType === 1 ? owner.getBoundingClientRect() : textRect;
      group = { text: [], rect: [rect.left + scrollX, rect.top + scrollY, rect.width, rect.height] };
      groups.set(owner, group);
    }
    const part = text.slice(0, 2000000 - totalChars);
    group.text.push(part);
    totalChars += part.length;
  }
  let droppedBlocks = truncated ? 1 : 0;
  for (const [owner, group] of groups) {
    const text = group.text.join(' ');
    if (text.length < 24) continue;
    if (layout.nodeIndex.length >= 4000) { droppedBlocks++; continue; }
    const index = node(owner.nodeType === 1 ? 1 : 3, owner.nodeName, text, -1);
    layout.nodeIndex.push(index);
    layout.text.push(intern(text));
    layout.bounds.push(group.rect);
  }
  let links = 0;
  for (const anchor of document.querySelectorAll('a[href]')) {
    if (links >= 300) break;
    if (!visible(anchor)) continue;
    const href = anchor.getAttribute('href');
    let url;
    try { url = new URL(href, document.baseURI); } catch { continue; }
    if (!/^https?:$/.test(url.protocol) || href.startsWith('#')) continue;
    const text = (anchor.innerText || '').replace(/\s+/g, ' ').trim().slice(0, 160);
    if (!text) continue;
    const index = node(1, 'A', '', -1, [intern('href'), intern(url.href)]);
    node(3, '#text', text, index);
    links++;
  }
  const documents = [{ nodes, layout }];
  for (const _ of document.querySelectorAll('iframe,frame')) documents.push({});
  return JSON.stringify({ strings, documents, droppedBlocks, source: 'webkit-dom' });
})()
