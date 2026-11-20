// Exercise the WKWebView extraction algorithm on real DOM/layout. The native
// WKContentWorld adapter is covered separately by `cargo test --test webkit`.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { launch, listen, run, startFixtures } from './harness.mjs';

const script = readFileSync(new URL('../crates/shell/src/ai/snapshot.js', import.meta.url), 'utf8');
const fixture = await startFixtures();
const reservation = await listen((_req, res) => res.end());
await new Promise(resolve => reservation.server.close(resolve));
const app = await launch({ debugPort: reservation.port, withProvider: false });
let failed;
try {
  await app.invoke('shell_page_navigate', { url: `http://127.0.0.1:${fixture.port}/` });
  const page = await app.pageWith(/127\.0\.0\.1/);
  await page.waitForSelector('#p1');
  const snapshot = async () => JSON.parse(await page.evaluate(script));
  const text = (result) => result.documents[0].layout.text.map((i) => result.strings[i]);
  failed = await run([
    ['WebKit extractor excludes hidden text and does not duplicate nested inline content', async () => {
      await page.setContent(`<p>A visible paragraph with <strong>nested inline words</strong> and enough text.</p>
        <p hidden>A hidden paragraph must never reach the assistant.</p>
        <div style="opacity:0"><p>A transparent paragraph must never reach the assistant.</p></div>
        <p style="display:none">A display-none paragraph must never reach the assistant.</p>
        <script>const secret = 'Script contents must never reach the assistant';</script>`);
      assert.deepEqual(text(await snapshot()), ['A visible paragraph with nested inline words and enough text.']);
    }],
    ['bare text and PDF-style spans produce citable document-space rectangles', async () => {
      await page.setContent(`<div style="height:1200px"></div>
        <div><span>Selectable PDF-style text with enough characters.</span></div>`);
      const result = await snapshot();
      assert.match(text(result).join('\n'), /Selectable PDF-style text/);
      const rect = result.documents[0].layout.bounds[0];
      assert.ok(rect[1] >= 1200 && rect[2] > 0 && rect[3] > 0, JSON.stringify(rect));
    }],
    ['links resolve against the document base and reject hidden or non-web destinations', async () => {
      await page.setContent(`<base href="https://base.example/docs/">
        <a href="next">Follow next</a><a href="#section">Same page</a>
        <a href="javascript:alert(1)">Script link</a><a hidden href="/secret">Hidden link</a>`);
      const result = await snapshot();
      const nodes = result.documents[0].nodes;
      const urls = nodes.attributes.filter((pairs) => pairs.length).map((pairs) => result.strings[pairs[1]]);
      assert.deepEqual(urls, ['https://base.example/docs/next']);
    }],
    ['large documents bound extraction and report omitted blocks', async () => {
      await page.setContent('<main></main>');
      await page.evaluate(() => {
        const main = document.querySelector('main');
        for (let i = 0; i < 4005; i++) {
          const p = document.createElement('p');
          p.textContent = `Paragraph ${i} contains enough text to become a cited passage.`;
          main.appendChild(p);
        }
      });
      const result = await snapshot();
      assert.equal(result.documents[0].layout.nodeIndex.length, 4000);
      assert.equal(result.droppedBlocks, 5);
    }],
  ]);
} finally {
  await app.close();
  fixture.close();
}
process.exitCode = failed ? 1 : 0;
