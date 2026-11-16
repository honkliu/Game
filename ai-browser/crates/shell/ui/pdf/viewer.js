// Minimal PDF viewer on pdf.js: pages fill the window width, text is selectable DOM
// (so the browser's selection/AI features see it), links work. The browser's page
// zoom drives `window.__pdfZoom`.
import * as pdfjs from './pdf.min.mjs';

pdfjs.GlobalWorkerOptions.workerSrc = new URL('./pdf.worker.min.mjs', import.meta.url).href;

const pagesEl = document.getElementById('pages');
const statusEl = document.getElementById('status');
const source = new URLSearchParams(location.search).get('src') || '';
const MARGIN = 16;
const EAGER_TEXT_PAGES = 400;

let doc = null;
let slots = [];
let zoom = 1;
let fit = 1;
let generation = 0;

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
  try {
    const response = await fetch('data?u=' + encodeURIComponent(source));
    if (!response.ok) throw new Error(await response.text());
    const bytes = new Uint8Array(await response.arrayBuffer());
    doc = await pdfjs.getDocument({ data: bytes }).promise;
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
  const ratio = window.devicePixelRatio || 1;
  const canvas = document.createElement('canvas');
  canvas.width = Math.floor(viewport.width * ratio);
  canvas.height = Math.floor(viewport.height * ratio);
  slot.el.prepend(canvas);
  try {
    await page.render({
      canvas,
      canvasContext: canvas.getContext('2d'),
      viewport,
      transform: ratio === 1 ? null : [ratio, 0, 0, ratio, 0, 0],
    }).promise;
  } catch (e) { /* a cancelled or broken page stays blank */ }
  await text(slot, gen);
}

async function text(slot, gen) {
  if (slot.text) return;
  slot.text = true;
  const page = await pageOf(slot);
  if (gen !== generation) return;
  const viewport = page.getViewport({ scale: fit * zoom });
  const layer = document.createElement('div');
  layer.className = 'textLayer';
  layer.style.setProperty('--scale-factor', String(viewport.scale));
  layer.style.setProperty('--total-scale-factor', String(viewport.scale));
  slot.el.append(layer);
  try {
    await new pdfjs.TextLayer({ textContentSource: page.streamTextContent(), container: layer, viewport }).render();
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
