// Measures the native PDF provider without changing the browser's PDF routing.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { launch, listen } from '../e2e/harness.mjs';

if (process.platform !== 'win32') throw new Error('This probe requires Windows and WebView2.');
const exec = promisify(execFile);

function makePdf(pages) {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${pages.map((_, i) => `${4 + i * 2} 0 R`).join(' ')}] /Count ${pages.length} >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  pages.forEach((text, i) => {
    const stream = `BT /F1 18 Tf 72 700 Td (${text}) Tj ET`;
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents ${5 + i * 2} 0 R /Resources << /Font << /F1 3 0 R >> >> >>`,
      `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    );
  });
  let out = '%PDF-1.4\n';
  const offsets = objects.map((object, i) => {
    const offset = out.length;
    out += `${i + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  out += offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('');
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, 'ascii');
}

const pages = [
  'First native PDF page: quarterly revenue increased.',
  'Second native PDF page: operating costs decreased.',
  'Third native PDF page: future outlook is positive.',
];
const otherText = 'Separate native PDF: canvas isolation sentinel.';
const fixture = await listen((req, res) => {
  const other = req.url.endsWith('-b');
  if (req.url.startsWith('/document-')) {
    res.setHeader('content-type', 'application/pdf');
    res.end(makePdf(other ? [otherText] : pages));
  } else {
    res.setHeader('content-type', 'text/html');
    res.end(`<!doctype html><title>Native PDF probe ${other ? 'B' : 'A'}</title>
      <style>html,body{margin:0;height:100%}embed{width:100%;height:100%}</style>
      <embed src="/document-${other ? 'b' : 'a'}" type="application/pdf">`);
  }
});
const reservation = await listen((_req, res) => res.end());
await new Promise(resolve => reservation.server.close(resolve));
let app;
try {
  app = await launch({ debugPort: reservation.port, withProvider: false });
  const base = `http://127.0.0.1:${fixture.port}`;
  await app.invoke('canvas.create', { cols: 2, rows: 1, urls: [`${base}/native-a`, `${base}/native-b`] });
  const pageA = await app.pageWith(/\/native-a$/);
  const pageB = await app.pageWith(/\/native-b$/);
  assert.ok(pageA && pageB, 'both canvas cells opened');
  await Promise.all([pageA.locator('embed').waitFor(), pageB.locator('embed').waitFor()]);
  const ps = `
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::FromHandle((Get-Process -Id ${app.child.pid}).MainWindowHandle)
$descendants = [System.Windows.Automation.TreeScope]::Descendants
$document = [System.Windows.Automation.ControlType]::Document
$documentCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $document)
$emptyId = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, '')
$nativeCondition = New-Object System.Windows.Automation.AndCondition($documentCondition, $emptyId)
$textCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::IsTextPatternAvailableProperty, $true)

function Read-NativePdf([string]$title, [string]$marker) {
  $deadline = [DateTime]::UtcNow.AddSeconds(12)
  do {
    $name = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $title)
    $scopeCondition = New-Object System.Windows.Automation.AndCondition($documentCondition, $name)
    $scope = $root.FindFirst($descendants, $scopeCondition)
    if ($scope) {
      $native = $scope.FindFirst($descendants, $nativeCondition)
      if ($native) {
        $pattern = $native.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
        $text = $pattern.DocumentRange.GetText(60000)
        if ($text.Contains($marker)) { return @{ element=$native; text=$text } }
      }
    }
    Start-Sleep -Milliseconds 250
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Native PDF text did not become available in $title"
}

$a = Read-NativePdf 'Native PDF probe A' 'quarterly revenue'
$b = Read-NativePdf 'Native PDF probe B' 'canvas isolation sentinel'
$selected = ''
$bounds = @()
foreach ($leaf in $a.element.FindAll($descendants, $textCondition)) {
  $pattern = $leaf.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
  $range = $pattern.DocumentRange.FindText('quarterly revenue', $false, $false)
  if (!$range) { continue }
  $range.Select()
  for ($attempt=0; $attempt -lt 10 -and !$selected; $attempt++) {
    Start-Sleep -Milliseconds 100
    foreach ($selection in $pattern.GetSelection()) {
      $selected += $selection.GetText(4000)
      $bounds += @($selection.GetBoundingRectangles() | ForEach-Object { @{ x=$_.X; y=$_.Y; width=$_.Width; height=$_.Height } })
    }
  }
  if ($selected) { break }
}
@{ textA=$a.text; textB=$b.text; selected=$selected; bounds=$bounds } | ConvertTo-Json -Depth 6 -Compress
`;
  const { stdout } = await exec('powershell', ['-NoProfile', '-NonInteractive', '-Command', ps],
    { timeout: 35000, maxBuffer: 1024 * 1024, encoding: 'utf8' });
  const result = JSON.parse(stdout);
  for (const text of pages) assert.ok(result.textA.includes(text), `native extraction includes ${text}`);
  assert.ok(!result.textA.includes(otherText), 'first cell does not include the other PDF');
  assert.ok(result.textB.includes(otherText), 'second cell has its own document text');
  assert.ok(!result.textB.includes(pages[0]), 'second cell does not include the first PDF');
  assert.equal(result.selected, 'quarterly revenue', 'native selected text is available');
  assert.ok(result.bounds.some(rect => rect.width > 0 && rect.height > 0), 'native selection has geometry');
  assert.ok(!result.textA.includes('Page number') && !result.textA.includes('Rotate'), 'viewer chrome is excluded');
  const session = await app.ctx.newCDPSession(pageA);
  const { targetInfos } = await session.send('Target.getTargets');
  assert.equal(targetInfos.filter(target => target.type === 'webview' && /edge_pdf/.test(target.url)).length, 2,
    'both documents use WebView2 native PDF viewers');
  await session.detach();
  console.log(`WebView2 ${app.browser.version()}`);
  console.log('PASS native text extraction includes all three pages, including offscreen pages');
  console.log('PASS independent native PDF text in two canvas cells, excluding viewer chrome');
  console.log('PASS native selected text and selection rectangles');
} finally {
  if (app) await app.close();
  fixture.server.closeAllConnections();
  await new Promise(resolve => fixture.server.close(resolve));
}
