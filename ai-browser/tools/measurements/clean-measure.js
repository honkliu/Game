// Clean-profile measurement pass for native-browser.
// Measures: baseline footprint, footprint at N tabs, restore-after-sleep cost.
const { execFileSync, spawn } = require('child_process');
const fs = require('fs');

const EXE = 'Q:\\gitroot\\Game\\native-browser\\target\\release\\browser.exe';

function ps(script) {
  return execFileSync('powershell', ['-NoProfile', '-Command', script], {
    encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
  });
}

function tree(pid) {
  const r = ps(`
$root = ${pid}
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, WorkingSetSize, CommandLine
if (-not ($all | Where-Object { $_.ProcessId -eq $root })) { "gone"; exit }
$ids = @($root); $changed = $true
while ($changed) { $changed = $false
  foreach ($p in $all) { if ($ids -contains $p.ParentProcessId -and -not ($ids -contains $p.ProcessId)) { $ids += $p.ProcessId; $changed = $true } } }
$sel = $all | Where-Object { $ids -contains $_.ProcessId }
$sum = ($sel | Measure-Object -Property WorkingSetSize -Sum).Sum
$types = $sel | ForEach-Object { if ($_.CommandLine -match '--type=([a-z-]+)') { $matches[1] } else { 'main' } } | Group-Object | ForEach-Object { "$($_.Name):$($_.Count)" }
"$($sel.Count)|$([math]::Round($sum/1MB,1))|$($types -join ',')"`).trim();
  if (r === 'gone') return { gone: true };
  const [count, rss, types] = r.split('|');
  return { count: +count, rss_mb: +rss, types: (types || '') };
}

const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const out = { at: new Date().toISOString(), steps: [] };

async function runSelfTest(label) {
  const t0 = Date.now();
  const r = spawn(EXE, ['--self-test=https://example.com'], { stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  r.stdout.on('data', (d) => (log += d));
  r.stderr.on('data', (d) => (log += d));
  while (!/SELF-TEST/.test(log) && Date.now() - t0 < 60000) await settle(200);
  const m = log.match(/SELF-TEST process_count=(\d+) rss_mb=([\d.]+) shell_ready_ms=(\d+) tabs=(\d+)/);
  out.steps.push({ label, wall_ms: Date.now() - t0, parsed: m ? { procs: +m[1], rss: +m[2], shell_ready: +m[3], tabs: +m[4] } : null });
  return out.steps.at(-1);
}

(async () => {
  // 1. Baseline: one tab.
  const b = await runSelfTest('baseline 1 tab');

  // 2. Live app, open tabs one at a time and measure growth.
  const p = spawn(EXE, ['https://example.com'], { stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  p.stdout.on('data', (d) => (log += d));
  p.stderr.on('data', (d) => (log += d));
  for (let i = 0; i < 80 && !/tab 1 navigated|shell ready/.test(log); i++) await settle(250);
  await settle(2500);
  out.steps.push({ label: 'live 1 tab', ...tree(p.pid) });

  // Driver-based tab opening is in-process only, so use the real UI path:
  // send Ctrl+T then a URL is not available headless. Instead measure the
  // process tree while the driver opens tabs in a second instance.
  p.kill();
  await settle(4000);

  const d = spawn(EXE, ['--driver=navloop'], { stdio: ['ignore', 'pipe', 'pipe'] });
  let dlog = '';
  d.stdout.on('data', (x) => (dlog += x));
  d.stderr.on('data', (x) => (dlog += x));
  const maxes = [];
  while (d.exitCode === null && !/DRIVER /.test(dlog)) {
    const t = tree(d.pid);
    if (!t.gone) maxes.push(t);
    await settle(1000);
  }
  await settle(2000);
  out.steps.push({ label: 'navloop peak (30 tabs opened)', samples: maxes.length, peak: maxes.length ? maxes.reduce((a, b) => (b.rss_mb > a.rss_mb ? b : a)) : null });
  const dm = dlog.match(/DRIVER .*rss_mb ([\d.]+)->([\d.]+)/);
  out.steps.push({ label: 'navloop driver summary', line: (dlog.match(/DRIVER .*/) || [''])[0] });

  fs.writeFileSync('Q:\\gitroot\\Game\\native-browser\\build\\clean-measure.json', JSON.stringify(out, null, 2));
  console.log(JSON.stringify(out, null, 2));
})();