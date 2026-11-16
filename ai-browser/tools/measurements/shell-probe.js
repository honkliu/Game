// Throwaway diagnostic: load the real shell HTML/CSS in headless Edge and report
// what the browser actually computed. Verification tool, not product code.
const { chromium } = require('playwright');
const path = require('path');

const uiDir = 'Q:\\gitroot\\Game\\native-browser\\crates\\browser\\ui';

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });

  await page.goto('file:///' + path.join(uiDir, 'index.html').replace(/\\/g, '/'));
  await page.waitForTimeout(1200);

  const report = await page.evaluate(() => {
    const q = (s) => document.querySelector(s);
    const box = (s) => { const e = q(s); if (!e) return null; const r = e.getBoundingClientRect();
      return { w: Math.round(r.width), h: Math.round(r.height), x: Math.round(r.x), y: Math.round(r.y) }; };
    const cs = (s, p) => { const e = q(s); return e ? getComputedStyle(e)[p] : null; };
    const visible = (s) => { const e = q(s); if (!e) return false; const r = e.getBoundingClientRect();
      const st = getComputedStyle(e); return r.width > 0 && r.height > 0 && st.visibility !== 'hidden' && st.display !== 'none'; };
    const text = (s) => (q(s) ? q(s).innerText.trim().slice(0, 120) : null);

    return {
      bodyBg: cs('body', 'backgroundColor'),
      bodyColor: cs('body', 'color'),
      bodyFont: cs('body', 'fontFamily'),
      railBg: cs('.rail', 'backgroundColor') || cs('[data-rail]', 'backgroundColor'),
      elements: [...document.querySelectorAll('[id], .rail, .toolbar, .omnibox, .tab, header, main, nav')]
        .map((e) => ({ tag: e.tagName.toLowerCase(), id: e.id, cls: e.className && String(e.className).slice(0, 40),
                       box: (() => { const r = e.getBoundingClientRect(); return `${Math.round(r.x)},${Math.round(r.y)} ${Math.round(r.width)}x${Math.round(r.height)}`; })(),
                       text: (e.innerText || '').trim().slice(0, 40) }))
        .filter((e) => e.box.split(' ')[1] !== '0x0')
        .slice(0, 40),
      hasBridgeStub: typeof window.__native,
      deadControls: [...document.querySelectorAll('button, a[href], input')].length,
      stylesheets: [...document.styleSheets].map((s) => s.href || 'inline'),
    };
  });

  const shot = 'Q:\\gitroot\\Game\\native-browser\\build\\shell-headless.png';
  await page.screenshot({ path: shot });
  console.log(JSON.stringify({ report, errors, shot }, null, 2));
  await browser.close();
})();