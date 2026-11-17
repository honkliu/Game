// Minimal PDF viewer on pdf.js: pages fill the window width, text is selectable DOM
// (so the browser's selection/AI features see it), links work. The browser's page
// zoom drives `window.__pdfZoom`.
import * as pdfjs from './pdf.min.mjs';

pdfjs.GlobalWorkerOptions.workerSrc = new URL('./pdf.worker.min.mjs', import.meta.url).href;

const pagesEl = document.getElementById('pages');
const statusEl = document.getElementById('status');
const source = new URLSearchParams(location.search).get('src') || '';
const MARGIN = 0;
const EAGER_TEXT_PAGES = 400;

let doc = null;
let slots = [];
let zoom = 1;
let fit = 1;
let generation = 0;
let loadingTask = null;
let findGeneration = 0;
const fetchController = new AbortController();
const renderTasks = new Set();
const textTasks = new Set();

function cancelRendering() {
  for (const task of renderTasks) task.cancel();
  for (const task of textTasks) task.cancel();
  renderTasks.clear(); textTasks.clear();
}

window.addEventListener('pagehide', () => {
  generation++; findGeneration++;
  fetchController.abort();
  cancelRendering();
  if (observer) observer.disconnect();
  if (loadingTask) loadingTask.destroy();
}, { once: true });

const nextFrame = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));

function titleFromSource() {
  try {
    const path = decodeURIComponent(new URL(source).pathname);
    return path.split('/').filter(Boolean).pop() || 'PDF';
  } catch (e) {
    return 'PDF';
  }
}

async function load() {
  document.title = titleFromSource();
  document.getElementById('name').textContent = document.title;
  try {
    const response = await fetch('data?u=' + encodeURIComponent(source), { signal: fetchController.signal });
    if (!response.ok) throw new Error(await response.text());
    const bytes = new Uint8Array(await response.arrayBuffer());
    loadingTask = pdfjs.getDocument({ data: bytes, isEvalSupported: false });
    doc = await loadingTask.promise;
  } catch (e) {
    statusEl.textContent = 'This PDF could not be opened: ' + (e && e.message ? e.message : e);
    return;
  }
  try {
    const meta = await doc.getMetadata();
    if (meta && meta.info && meta.info.Title) document.title = meta.info.Title;
  } catch (e) { /* the file name stays as the title */ }
  const first = await doc.getPage(1);
  const base = first.getViewport({ scale: 1 });
  slots = [];
  for (let n = 1; n <= doc.numPages; n++) {
    const el = document.createElement('div');
    el.className = 'page';
    el.dataset.page = String(n);
    pagesEl.appendChild(el);
    slots.push({ n, el, page: n === 1 ? first : null, base: n === 1 ? base : null, canvas: false, text: false });
  }
  statusEl.textContent = '';
  layout();
}

function layout() {
  if (!doc) return;
  generation++;
  cancelRendering();
  const first = slots[0].base;
  fit = Math.max(0.2, (document.documentElement.clientWidth - MARGIN) / first.width);
  const scale = fit * zoom;
  for (const slot of slots) {
    const b = slot.base || first;
    slot.el.style.width = Math.floor(b.width * scale) + 'px';
    slot.el.style.height = Math.floor(b.height * scale) + 'px';
    slot.el.replaceChildren();
    slot.canvas = false;
    slot.text = false;
    slot.textPromise = null;
  }
  observe();
  buildText(generation);
}

let observer = null;
function observe() {
  if (observer) observer.disconnect();
  observer = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      const slot = slots[Number(e.target.dataset.page) - 1];
      draw(slot, generation);
    }
  }, { rootMargin: '1500px 0px' });
  for (const slot of slots) observer.observe(slot.el);
}

async function pageOf(slot) {
  if (!slot.page) {
    slot.page = await doc.getPage(slot.n);
    slot.base = slot.page.getViewport({ scale: 1 });
  }
  return slot.page;
}

async function draw(slot, gen) {
  if (slot.canvas) return;
  slot.canvas = true;
  const page = await pageOf(slot);
  if (gen !== generation) return;
  const viewport = page.getViewport({ scale: fit * zoom });
  slot.el.style.width = Math.floor(viewport.width) + 'px';
  slot.el.style.height = Math.floor(viewport.height) + 'px';
  const ratio = Math.min(window.devicePixelRatio || 1, 2,
    Math.sqrt(16000000 / Math.max(1, viewport.width * viewport.height)));
  const canvas = document.createElement('canvas');
  canvas.width = Math.floor(viewport.width * ratio);
  canvas.height = Math.floor(viewport.height * ratio);
  slot.el.prepend(canvas);
  try {
    const task = page.render({
      canvas,
      canvasContext: canvas.getContext('2d'),
      viewport,
      transform: ratio === 1 ? null : [ratio, 0, 0, ratio, 0, 0],
    });
    renderTasks.add(task);
    try { await task.promise; } finally { renderTasks.delete(task); }
  } catch (e) {
    if (gen === generation && e.name !== 'RenderingCancelledException') {
      slot.canvas = false;
      statusEl.textContent = 'Page ' + slot.n + ' could not be rendered: ' + e.message;
    }
  }
  await text(slot, gen);
}

function text(slot, gen) {
  if (slot.text) return slot.textPromise;
  slot.text = true;
  slot.textPromise = renderText(slot, gen);
  return slot.textPromise;
}

async function renderText(slot, gen) {
  const page = await pageOf(slot);
  if (gen !== generation) return;
  const viewport = page.getViewport({ scale: fit * zoom });
  const layer = document.createElement('div');
  layer.className = 'textLayer';
  layer.style.setProperty('--scale-factor', String(viewport.scale));
  layer.style.setProperty('--total-scale-factor', String(viewport.scale));
  slot.el.append(layer);

  try {
    const task = new pdfjs.TextLayer({ textContentSource: page.streamTextContent(), container: layer, viewport });
    textTasks.add(task);
    try { await task.render(); } finally { textTasks.delete(task); }
  } catch (e) { return; }
  if (gen !== generation) return;
  try {
    for (const a of await page.getAnnotations()) {
      if (a.subtype !== 'Link' || !a.url) continue;
      const [x1, y1, x2, y2] = viewport.convertToViewportRectangle(a.rect);
      const link = document.createElement('a');
      link.href = a.url;
      link.style.cssText = `left:${Math.min(x1, x2)}px;top:${Math.min(y1, y2)}px;width:${Math.abs(x2 - x1)}px;height:${Math.abs(y2 - y1)}px`;
      layer.after(link);
      slot.el.append(link);
    }
  } catch (e) { /* links are optional */ }
}

// Text for the whole document is built in the background so the agent and find-in-page
// see every page, not just the ones on screen.
async function buildText(gen) {
  for (const slot of slots.slice(0, EAGER_TEXT_PAGES)) {
    if (gen !== generation) return;
    await text(slot, gen);
    await nextFrame();
  }
}

window.__pdfFind = async (query, matchCase, wholeWord, _highlightAll, index) => {
  const scan = ++findGeneration;
  const flags = ['case_sensitive', 'whole_word'];
  if (window.CSS && CSS.highlights) CSS.highlights.delete('browser-find-active');
  if (!query || !doc) return { total: 0, active: 0, supported_flags: flags };
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp(escaped, matchCase ? 'gu' : 'giu');
  const word = s => /[\p{L}\p{N}_]/u.test(s);
  const hits = [];
  let limited = doc.numPages > 2000;
  const started = Date.now();
  for (let n = 1; n <= Math.min(doc.numPages, 2000); n++) {
    if (scan !== findGeneration) return { pending: true };
    const page = await doc.getPage(n);
    const content = await page.getTextContent();
    const full = content.items.map(item => item.str || '').join('');
    const body = full.slice(0, 2000000);
    limited ||= full.length > body.length;
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(body))) {
      const end = m.index + m[0].length;
      if (wholeWord && (word(body[m.index - 1] || '') || word(body[end] || ''))) continue;
      hits.push({ page: n, from: m.index, to: end });
      if (hits.length >= 5000) { limited = true; break; }
    }
    if (hits.length >= 5000 || Date.now() - started > 15000) { limited = true; break; }
    if (n % 5 === 0) await nextFrame();
  }
  if (scan !== findGeneration) return { pending: true };
  const selected = hits.length ? ((index % hits.length) + hits.length) % hits.length : -1;
  if (selected >= 0) {
    const hit = hits[selected], slot = slots[hit.page - 1];
    await text(slot, generation);
    if (scan !== findGeneration) return { pending: true };
    slot.el.scrollIntoView({ block: 'start' });
    const layer = slot.el.querySelector('.textLayer');
    if (layer) {
      const walker = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT);
      const range = document.createRange();
      let node, offset = 0, found = false;
      while ((node = walker.nextNode())) {
        if (!found && offset + node.length > hit.from) {
          range.setStart(node, hit.from - offset); found = true;
        }
        if (found && offset + node.length >= hit.to) {
          range.setEnd(node, hit.to - offset);
          if (window.CSS && CSS.highlights && window.Highlight) CSS.highlights.set('browser-find-active', new Highlight(range));
          else { const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range); }
          break;
        }
        offset += node.length;
      }
    }
  }
  return { total: hits.length, active: selected + 1, limited, supported_flags: flags };
};

window.__pdfZoom = (factor) => {
  zoom = Math.min(5, Math.max(0.25, Number(factor) || 1));
  const y = window.scrollY / Math.max(1, document.documentElement.scrollHeight);
  layout();
  requestAnimationFrame(() => window.scrollTo(window.scrollX, y * document.documentElement.scrollHeight));
};

let resizeTimer = 0;
window.addEventListener('resize', () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(layout, 120);
});

load();

const post = (obj) => { try { window.ipc.postMessage(JSON.stringify(obj)); } catch (e) { /* no host */ } };
document.getElementById('zin').addEventListener('click', () => post({ ai: 'zoom', dir: 1 }));
document.getElementById('zout').addEventListener('click', () => post({ ai: 'zoom', dir: -1 }));
document.getElementById('zfit').addEventListener('click', () => post({ ai: 'zoom', dir: 0 }));
window.addEventListener('scroll', () => {
  const mid = window.scrollY + window.innerHeight / 3;
  const slot = slots.find((s) => s.el.offsetTop + s.el.offsetHeight > mid) || slots[slots.length - 1];
  if (slot) {
    document.getElementById('cur').textContent = String(slot.n);
    document.getElementById('total').textContent = String(slots.length);
  }
}, { passive: true });
