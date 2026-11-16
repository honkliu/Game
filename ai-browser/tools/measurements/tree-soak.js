// Tree-scoped measurement for native-browser. Fixes two bugs found in soak.js:
// 1. --self-test exits on its own, so sampling began after the app was gone.
// 2. Machine-wide filters picked up SearchHost's and Teams' WebView2 processes.
// This walks only descendants of the launched browser.exe.
const { execFileSync, spawn } = require('child_process');
const fs = require('fs');

const EXE = 'Q:\\gitroot\\Game\\native-browser\\target\\release\\browser.exe';
const OUT = 'Q:\\gitroot\\Game\\native-browser\\build\\soak.json';

function ps(script) {
  return execFileSync('powershell', ['-NoProfile', '-Command', script], {
    encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
  });
}

function tree(pid) {
  const r = ps(`
$root = ${pid}
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, WorkingSetSize, CommandLine
if (-not ($all | Where-Object { $_.ProcessId -eq $root })) { "gone||"; exit }
$ids = @($root); $changed = $true
while ($changed) { $changed = $false
  foreach ($p in $all) { if ($ids -contains $p.ParentProcessId -and -not ($ids -contains $p.ProcessId)) { $ids += $p.ProcessId; $changed = $true } } }
$sel = $all | Where-Object { $ids -contains $_.ProcessId }
$sum = ($sel | Measure-Object -Property WorkingSetSize -Sum).Sum
$types = $sel | ForEach-Object { if ($_.CommandLine -match '--type=([a-z-]+)') { $matches[1] } else { 'main' } } | Group-Object | ForEach-Object { "$($_.Name):$($_.Count)" }
"$($sel.Count)|$([math]::Round($sum/1MB,1))|$($types -join ',')"`).trim();
  if (r.startsWith('gone')) return { gone: true };
  const [count, rss, types] = r.split('|');
  return { count: +count, rss_mb: +rss, types: Object.fromEntries((types || '').split(',').filter(Boolean).map((s) => s.split(':'))) };
}

const settle = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const scenario = process.argv[2] || 'idle';
  const seconds = +(process.argv[3] || 60);
  const url = process.argv[4] || 'https://example.com';
  const res = { scenario, url, at: new Date().toISOString(), samples: [] };

  // Launch WITHOUT --self-test so the app stays up for the soak duration.
  const t0 = Date.now();
  const p = spawn(EXE, [url], { stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  p.stdout.on('data', (d) => (log += d));
  p.stderr.on('data', (d) => (log += d));

  for (let i = 0; i < 80 && !/tab 1 navigated|shell ready/.test(log); i++) await settle(250);
  res.time_to_first_nav_ms = Date.now() - t0;
  res.startup_log = log.split('\n').filter((l) => /shell ready|navigated|window/.test(l)).slice(0, 8);

  const start = Date.now();
  while (Date.now() - start < seconds * 1000) {
    const s = tree(p.pid);
    if (s.gone) break;
    res.samples.push({ t: Math.round((Date.now() - start) / 1000), ...s });
    await settle(parseInt(process.env.SAMPLE_MS || '5000', 10));
  }

  p.kill();
  await settle(4000);
  res.survivors = tree(p.pid);   // should be {gone:true} if cleanup works
  res.exitCode = p.exitCode;

  const s = res.samples;
  if (s.length) {
    const rss = s.map((x) => x.rss_mb), cnt = s.map((x) => x.count);
    res.summary = {
      samples: s.length,
      rss_first: rss[0], rss_last: rss.at(-1), rss_max: Math.max(...rss),
      rss_growth_mb: +(rss.at(-1) - rss[0]).toFixed(1),
      rss_growth_pct: +(((rss.at(-1) - rss[0]) / rss[0]) * 100).toFixed(1),
      count_first: cnt[0], count_last: cnt.at(-1), count_max: Math.max(...cnt),
      types_first: s[0].types, types_last: s.at(-1).types,
    };
  }
  fs.writeFileSync(OUT, JSON.stringify(res, null, 2));
  console.log(JSON.stringify({ scenario, time_to_first_nav_ms: res.time_to_first_nav_ms, summary: res.summary, survivors: res.survivors }, null, 2));
})();