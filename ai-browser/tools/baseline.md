# Release 0 baseline — measured on one machine, four commands

Records the numbers R0a-1's assertion is a delta against (gap 5 in `../docs/RELEASE-0.md`), and the
measured verdict of the `--proof` harness that produces them. Every number below was read out of a
running `browser.exe`; nothing here is estimated. Where a budget could not be measured in a given
mode, the harness prints `NOT MEASURED` and this file says why instead of quoting a number for it.

The harness now measures all eight checks; **C3 and C5 — the two that were unmeasurable in the first
cut — are measurable in the modes that turn off the shell behaviour that hid them** (§The four
modes). Run `--proof=+no_tab_sleep+no_overlay` and all eight go green:
`RESULT 0 of 8 checks failed`, exit 0.

## Read this first: the profile wipe is mandatory for comparability, not cleanup

WebView2 persists session state in the user-data-folder and the shell restores it, so sample 0 is
"however many tabs the last run left", not "one new tab page". The verdicts are deltas and survive
that. **The absolute MB figures do not.** Measured on this machine, same binary, same scenario, one
run wiping the profile and one not:

| | wiped profile | profile left from a previous run | drift |
|---|--:|--:|--:|
| tabs at step 0 → last | **1 → 5** | **9 → 13** | restored session added 8 tabs |
| whole process tree at the last sample | **591.6 MB** | **822.6 MB** | **+231 MB (+39 %)** |
| shell process working set | **38.5 MB** | **72.2 MB** | **+33.7 MB (+88 %)** |
| check verdicts | C0/C1/C2/C4 PASS, C3/C5 not measurable | **identical** | none |

So: two runs that agree perfectly on every verdict disagreed by 39 % on the headline memory number,
and only the header row distinguished them.

The harness now enforces this rather than documenting it:

1. `main.rs::wipe_proof_profile()` deletes the private profile before the first sample, and prints
   `proof: profile wiped (…)` (or, if the delete fails — usually because another proof instance
   still holds that folder — `proof: WARNING could not wipe …`, which is also the condition that
   makes check C0 fail).
2. Check **C7** fails if step 0 is not one tab with one live webview, i.e. if a restore happened
   anyway, and the header prints `PROFILE NOT WIPED — session state was restored, see C7` instead of
   the `profile was wiped` line. A report whose C7 is red must not have its absolute MB quoted.
3. The header also prints the mode (`tab_sleep=`, `overlay_at_startup=`), because absolute MB are
   comparable only against a run **in the same mode** — the overlay alone is ~66 MB and each
   background tab kept alive is ~97 MB (§Pricing).

### What is portable and what is not

| Portable across runs and machines | Only comparable within one run |
|---|---|
| Process *counts*: 1 browser process, +1 renderer per live page, 4 non-renderer children per browser process, 0 unclassified | — |
| Per-role working set of one sample (browser process ≈ 140–165 MB, one renderer ≈ 54–73 MB, shell 28–39 MB) | absolute tree MB, unless both runs started from a wiped profile *and* the same mode |
| Intra-run deltas: overlay +66 MB, kept-alive background tab +97 MB, 4 tabs +390 MB | anything compared against a run whose C7 was not green |

Run-to-run spread on the same mode and a wiped profile, measured 6 times today: whole tree ±7 MB,
overlay delta ±2.2 MB, per-kept-alive-tab delta ±0.6 MB. Treat differences smaller than that as
noise, not as a regression.

## Machine

| | |
|---|---|
| CPU | `Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz`, 16 logical CPUs |
| RAM | 63.95 GiB visible (`TotalVisibleMemorySize` = 67 056 476 KiB); the harness reports "64 GiB" |
| OS | Microsoft Windows 11 Enterprise, build 26300 |
| WebView2 runtime | Evergreen 157.0.4341.57, the one `webview2::ensure_runtime()` accepts on this box |
| Build measured | `cargo` **dev** profile (`unoptimized + debuginfo`) — a release build is a different baseline |
| Measured | 2026-11-13: 14 `--proof` runs, plus two runs of the two-host probe in §Workspace isolation |

CPU brand / OS / RAM came from:

```bat
powershell -NoProfile -Command "(Get-CimInstance Win32_Processor | Select-Object -First 1).Name; (Get-CimInstance Win32_OperatingSystem).Caption; [System.Environment]::OSVersion.Version.Build; (Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize"
```

Logical CPU count and total RAM in the harness itself come from `GetSystemInfo` and
`GlobalMemoryStatusEx` (`ai/proof.rs::machine_line`), not from the shell. The CPU *brand string* has
no dependency-free Win32 call, so the harness does not print it and the command above is the source
of record.

## The four modes

All four wipe their profile themselves, so no manual `rmdir` is required; it is still the correct
manual fix when C7 goes red.

```bat
cd /d Q:\gitroot\Game\native-browser

cargo run --offline -p browser -- --proof                                :: A  shipping behaviour
cargo run --offline -p browser -- --proof=4+no_tab_sleep                  :: B  background tabs kept alive
cargo run --offline -p browser -- --proof=+no_overlay                     :: C  start without the overlay
cargo run --offline -p browser -- --proof=+no_tab_sleep+no_overlay        :: D  both (all 8 checks green)
```

Both toggles are `mode.rs` flags, parsed out of the same string the scenario reads, **off by
default**, and reachable only when the binary was started with `--proof` (`mode::active()`). The
tab count is optional and capped at the harness's eight distinct origins; a mistyped flag exits 2
rather than silently running the default mode:

```
proof: unknown flag(s) nosleep in `proof:4+nosleep`; expected --proof[=N][+no_tab_sleep][+no_overlay] — refusing to run the default mode instead
```

| Mode | What it turns off, and why that is what makes a check measurable |
|---|---|
| A | nothing — shipping behaviour. Measures C0/C1/C2/C4/C7; prints C3/C5/C6 as NOT MEASURED **with the flag that measures them** |
| B | `AppState::sleep_background_tabs` (destroys every non-active tab's webview). With it off, 5 tabs really are 5 live pages, so "+4 tabs → +4 renderers" has a referent → **C3**, and the price of that choice becomes a number → **C6** |
| C | the overlay webview at startup. Step 0 is sampled without it and the scenario builds it as step 0 → **C5** measures the overlay as a before/after delta |
| D | both. C3 is then measured from the post-overlay sample, so the overlay's renderer is never charged to a tab |

`--driver=proof…` enters the same door as `--proof` (`main.rs` arms the mode for any driver name
starting with `proof`), so the mode and the scenario cannot disagree.

## Measured — mode A, shipping behaviour, verbatim

```
proof: data directory redirected to C:\Users\shuguanl\AppData\Local\native-browser-proof
proof: profile wiped (C:\Users\shuguanl\AppData\Local\native-browser-proof\native-browser)
proof: mode tabs=4 tab_sleep=on overlay_at_startup=true
proof: 0 environment(s) constructed before the app started
[  594 ms] shell webview ready
[  719 ms] overlay webview ready (0x0)
[  719 ms] window + shell created at 719 ms
[  735 ms] shell ready
[  813 ms] webview for tab 1 -> asset://shell/pages/newtab.html#d=eyJib29rbWFya3MiOltdLCJyZWNlbnQiOltdfQ
[ 2875 ms] webview for tab 2 -> https://example.com/
[ 2922 ms] slept 1 background webview(s)
[ 2953 ms] tab 2 navigated: https://example.com/
[ 3922 ms] webview for tab 3 -> https://example.org/
[ 3969 ms] slept 1 background webview(s)
[ 3985 ms] tab 3 navigated: https://example.org/
[ 4953 ms] webview for tab 4 -> https://example.net/
[ 5000 ms] slept 1 background webview(s)
[ 5016 ms] tab 4 navigated: https://example.net/
[ 5985 ms] webview for tab 5 -> https://www.iana.org/
[ 6032 ms] slept 1 background webview(s)
[ 6047 ms] tab 5 navigated: https://www.iana.org/
DRIVER scenario="proof" steps=4 tabs 2->5 live_webviews 1->1 rss_mb 528.3->505.5
[ 6969 ms] shut down
PROOF  native-browser Release 0 harness (R0a-1)
  machine    16 logical CPUs, 64 GiB RAM
  profile    C:\Users\shuguanl\AppData\Local\native-browser-proof\native-browser\webview2
  mode       tabs=4 tab_sleep=on overlay_at_startup=true (absolute MB figures from this run are comparable only against a run with the same mode)
  command    cargo run -p browser -- --proof   (open 4 distinct-origin tabs)
  samples    5 (one per driver step)
  context    0 other host process(es), 22 msedgewebview2.exe in total (ours and everybody else's)
  start      1 tab(s) at step 0, 1 live webview(s), built overlay; profile was wiped, so absolute MB are comparable
  step  tabs  live_wv  ovl  env  browser  renderer  other  unclass   tree_MB  shell_MB  webview_MB
     0     1        1  yes    1       1        3     4       0     527.5      38.2      489.4
     1     2        1  yes    1       1        3     4       0     544.8      38.3      506.4
     2     3        1  yes    1       1        3     4       0     545.3      38.4      506.9
     3     4        1  yes    1       1        3     4       0     548.2      38.6      509.6
     4     5        1  yes    1       1        3     4       0     595.3      38.6      556.7
  per-role working set at the last sample: browser 142.7 MB, renderer 218.5 MB
  step 0 WebView2 members  31620:browser(p22752) 10080:crashpad-handler(p31620) 12356:gpu-process(p31620) 9292:utility(p31620) 8040:utility(p31620) 26684:renderer(p31620) 21656:renderer(p31620) 24408:renderer(p31620)
  last WebView2 members  31620:browser(p22752) 10080:crashpad-handler(p31620) 12356:gpu-process(p31620) 9292:utility(p31620) 8040:utility(p31620) 26684:renderer(p31620) 21656:renderer(p31620) 18180:renderer(p31620)

PASS  C0 the WebView2 tree counted is the one this run started — 1 browser process(es) in our tree across 5 sample(s); widest tree 9 process(es); 22 msedgewebview2.exe machine-wide and 0 other host process(es) running. An empty tree here means the browser process for this user-data-folder belongs to somebody else, not that the cost is zero.
PASS  C1 exactly one ICoreWebView2Environment per process lifetime — constructed=1 sampled before and after opening 4 tabs (a second one means a webview was built without with_environment)
PASS  C2 zero extra WebView2 browser processes for +N tabs — browser processes 1 -> 1 (peak 1, delta +0) while tabs went 1 -> 5; 0 process(es) with an unreadable --type= [31620:browser(…) 21656:renderer(…) 18180:renderer(…)]
NOT MEASURED  C3 +N tabs add exactly +N renderer processes — renderers 3 -> 3 (peak 3, peak delta +0, final delta +0) while tabs went 1 -> 5 (+4) and live page webviews went 1 -> 1 (peak 1, delta +0), measured from step 0; tab sleeping is on (shipping behaviour): the shell held at most 1 live page webview(s) for 5 tabs, so the runtime was never handed 4 live pages and "+4 renderers" has no referent in this mode. Measure it with `--proof=4+no_tab_sleep`; what this mode does establish is that no sample ever held fewer renderers than live page webviews, i.e. no sharing.
PASS  C4 process-tree working set is measurable — tree 595.3 MB = shell 38.6 + WebView2 children 556.7 + other 0.0, over 9 processes (1 live webviews, 5 tabs)
NOT MEASURED  C5 overlay adds no browser process and a bounded renderer cost — no sample pair differs in overlay presence, so the overlay was built at startup and is inside every sample (1 live webview(s) at step 0 = shell + overlay + one page). Run `--proof=4+no_overlay` to measure its delta.
NOT MEASURED  C6 cost of a background tab kept alive (no budget set; number only) — no live-webview growth to divide by: tab sleeping kept the run at 1 live page webview(s). Run `--proof=4+no_tab_sleep` for the number.
PASS  C7 the run started from a wiped profile (one tab, no restored state) — 1 tab(s) and 1 live webview(s) at step 0 (expected 1 and 1). If this fails, delete the harness profile before the next run — `rmdir /s /q "%LOCALAPPDATA%\native-browser-proof"` — and treat every absolute MB in this report as incomparable; per-role working set and deltas across samples of one run stay valid.

RESULT 0 of 8 checks failed

proof: report written to C:\Users\shuguanl\AppData\Local\native-browser-proof\native-browser\proof.txt
```

`[…(…)]` in the C2 line is the only abbreviation: the harness prints the full
`pid:role(parent-pid)` list, and it is quoted in full for step 0 and the last step just above.

**Every mode-A run today was identical in verdict** (C0/C1/C2/C4/C7 PASS, C3/C5/C6 NOT MEASURED,
exit 0) and within 7 MB on the tree. Note the renderer *identity* changes while the count stays at
3 — pid 24408 → 18180 above — i.e. with tab sleeping on, renderers are **replaced per activation,
not accumulated**; one run also showed `peak 4` renderers mid-run, which is the previous page's
renderer being torn down asynchronously after the new one was already up. That is why C2/C3 report
both the peak and the final count.

## Measured — mode D, the R0a-1 exit gate met, verbatim

```
proof: data directory redirected to C:\Users\shuguanl\AppData\Local\native-browser-proof
proof: profile wiped (C:\Users\shuguanl\AppData\Local\native-browser-proof\native-browser)
proof: mode tabs=4 tab_sleep=off overlay_at_startup=false
proof: 0 environment(s) constructed before the app started
[  610 ms] shell webview ready
[  610 ms] overlay deferred (measurement mode); built by the harness step
[  610 ms] window + shell created at 610 ms
[  688 ms] shell ready
[  766 ms] webview for tab 1 -> asset://shell/pages/newtab.html#d=eyJib29rbWFya3MiOltdLCJyZWNlbnQiOltdfQ
[ 2922 ms] overlay webview ready (0x0)
[ 5516 ms] webview for tab 2 -> https://example.com/
[ 5547 ms] tab 2 navigated: https://example.com/
[ 6516 ms] webview for tab 3 -> https://example.org/
[ 6547 ms] tab 3 navigated: https://example.org/
[ 7516 ms] webview for tab 4 -> https://example.net/
[ 7547 ms] tab 4 navigated: https://example.net/
[ 8516 ms] webview for tab 5 -> https://www.iana.org/
[ 8563 ms] tab 5 navigated: https://www.iana.org/
DRIVER scenario="proof:4+no_tab_sleep+no_overlay" steps=5 tabs 1->5 live_webviews 1->5 rss_mb 520.3->815.1
[ 9485 ms] shut down
PROOF  native-browser Release 0 harness (R0a-1)
  machine    16 logical CPUs, 64 GiB RAM
  profile    C:\Users\shuguanl\AppData\Local\native-browser-proof\native-browser\webview2
  mode       tabs=4 tab_sleep=off overlay_at_startup=false (absolute MB figures from this run are comparable only against a run with the same mode)
  command    cargo run -p browser -- --proof=+no_tab_sleep+no_overlay   (open 4 distinct-origin tabs)
  samples    6 (one per driver step)
  context    0 other host process(es), 26 msedgewebview2.exe in total (ours and everybody else's)
  start      1 tab(s) at step 0, 1 live webview(s), absent overlay; profile was wiped, so absolute MB are comparable
  step  tabs  live_wv  ovl  env  browser  renderer  other  unclass   tree_MB  shell_MB  webview_MB
     0     1        1   no    1       1        2     4       0     459.1      28.4      430.7
     1     1        1  yes    1       1        3     4       0     524.8      28.5      496.2
     2     2        2  yes    1       1        4     4       0       600      28.6      571.4
     3     3        3  yes    1       1        5     4       0     689.5      28.7      660.8
     4     4        4  yes    1       1        6     4       0     778.6      28.9      749.8
     5     5        5  yes    1       1        7     4       0     913.9        29      884.9
  per-role working set at the last sample: browser 163.8 MB, renderer 454.3 MB
  step 0 WebView2 members  30196:browser(p13380) 21092:crashpad-handler(p30196) 28564:gpu-process(p30196) 5012:utility(p30196) 21384:utility(p30196) 27144:renderer(p30196) 17920:renderer(p30196)
  last WebView2 members  30196:browser(p13380) 21092:crashpad-handler(p30196) 28564:gpu-process(p30196) 5012:utility(p30196) 21384:utility(p30196) 27144:renderer(p30196) 17920:renderer(p30196) 31580:renderer(p30196) 21776:renderer(p30196) 25060:renderer(p30196) 26596:renderer(p30196) 24228:renderer(p30196)

PASS  C0 the WebView2 tree counted is the one this run started — 1 browser process(es) in our tree across 6 sample(s); widest tree 13 process(es); 26 msedgewebview2.exe machine-wide and 0 other host process(es) running. An empty tree here means the browser process for this user-data-folder belongs to somebody else, not that the cost is zero.
PASS  C1 exactly one ICoreWebView2Environment per process lifetime — constructed=1 sampled before and after opening 4 tabs (a second one means a webview was built without with_environment)
PASS  C2 zero extra WebView2 browser processes for +N tabs — browser processes 1 -> 1 (peak 1, delta +0) while tabs went 1 -> 5; 0 process(es) with an unreadable --type= [30196:browser(…) … 24228:renderer(…)]
PASS  C3 +N tabs add exactly +N renderer processes — renderers 3 -> 7 (peak 7, peak delta +4, final delta +4) while tabs went 1 -> 5 (+4) and live page webviews went 1 -> 5 (peak 5, delta +4), measured from step 1; +4 live page webview(s) and +4 renderer processes: the plan's assertion holds as written
PASS  C4 process-tree working set is measurable — tree 913.9 MB = shell 29.0 + WebView2 children 884.9 + other 0.0, over 13 processes (5 live webviews, 5 tabs)
PASS  C5 overlay adds no browser process and a bounded renderer cost — overlay built between steps 0 and 1 (1 -> 1 live webview(s) in the shell): renderers 2 -> 3 (delta +1), browser processes 1 -> 1 (delta +0), tree 459.1 -> 524.8 MB (delta +65.7 MB against the ≈60 MB budget / 80 MB ceiling). exactly +1 renderer, as budgeted. Working-set deltas carry page-load noise; the process counts do not.
PASS  C6 cost of a background tab kept alive (no budget set; number only) — live page webviews +4 (1 -> 5) while WebView2 children went 496.2 -> 884.9 MB: 97.2 MB per tab kept alive, +388.7 MB for the run; renderer processes 3 -> 7 holding 162.1 -> 454.3 MB, 73.1 MB per renderer added. This is the price of not sleeping background tabs; nothing in the plan sets a ceiling on it yet.
PASS  C7 the run started from a wiped profile (one tab, no restored state) — 1 tab(s) and 1 live webview(s) at step 0 (expected 1 and 1). If this fails, delete the harness profile before the next run — `rmdir /s /q "%LOCALAPPDATA%\native-browser-proof"` — and treat every absolute MB in this report as incomparable; per-role working set and deltas across samples of one run stay valid.

RESULT 0 of 8 checks failed

proof: report written to C:\Users\shuguanl\AppData\Local\native-browser-proof\native-browser\proof.txt
```

## Measured — modes B and C (same runs, the checks only these modes can produce)

`--proof=4+no_tab_sleep`, sample table and the two checks that need it:

```
  mode       tabs=4 tab_sleep=off overlay_at_startup=true
  start      1 tab(s) at step 0, 1 live webview(s), built overlay; profile was wiped, so absolute MB are comparable
  step  tabs  live_wv  ovl  env  browser  renderer  other  unclass   tree_MB  shell_MB  webview_MB
     0     1        1  yes    1       1        3     4       0     525.2      38.1      487.1
     1     2        2  yes    1       1        4     4       0     599.8      38.4      561.4
     2     3        3  yes    1       1        5     4       0     689.3      38.5      650.8
     3     4        4  yes    1       1        6     4       0     780.7      38.4      742.3
     4     5        5  yes    1       1        7     4       0     917.2      38.5      878.7
  per-role working set at the last sample: browser 154.6 MB, renderer 457.2 MB

PASS  C3 +N tabs add exactly +N renderer processes — renderers 3 -> 7 (peak 7, peak delta +4, final delta +4) while tabs went 1 -> 5 (+4) and live page webviews went 1 -> 5 (peak 5, delta +4), measured from step 0; +4 live page webview(s) and +4 renderer processes: the plan's assertion holds as written
PASS  C6 cost of a background tab kept alive (no budget set; number only) — live page webviews +4 (1 -> 5) while WebView2 children went 487.1 -> 878.7 MB: 97.9 MB per tab kept alive, +391.6 MB for the run; renderer processes 3 -> 7 holding 162.2 -> 457.2 MB, 73.8 MB per renderer added.
RESULT 0 of 8 checks failed          (C0/C1/C2/C3/C4/C6/C7 PASS, C5 NOT MEASURED)
```

`--proof=+no_overlay` — shipping tab-sleeping, overlay deferred to a measured step:

```
  mode       tabs=4 tab_sleep=on overlay_at_startup=false
  step  tabs  live_wv  ovl  env  browser  renderer  other  unclass   tree_MB  shell_MB  webview_MB
     0     1        1   no    1       1        2     4       0     458.1      36.6      421.6
     1     1        1  yes    1       1        3     4       0     525.1      38.3      486.8
     2     2        1  yes    1       1        3     4       0     538.3      38.4      499.9
     3     3        1  yes    1       1        3     4       0     537.8      38.6      499.2
     4     4        1  yes    1       1        3     4       0     541.7      38.6      503.2
     5     5        1  yes    1       1        3     4       0     592.3      38.6      553.7

PASS  C5 overlay adds no browser process and a bounded renderer cost — overlay built between steps 0 and 1 (1 -> 1 live webview(s) in the shell): renderers 2 -> 3 (delta +1), browser processes 1 -> 1 (delta +0), tree 458.1 -> 525.1 MB (delta +67.0 MB against the ≈60 MB budget / 80 MB ceiling). exactly +1 renderer, as budgeted.
RESULT 0 of 8 checks failed          (C0/C1/C2/C4/C5/C7 PASS, C3/C6 NOT MEASURED)
```

All 14 harness runs of 2026-11-13 ended `RESULT 0 of 8 checks failed`, exit 0. The exit-1 path was
verified separately on earlier builds (2 FAILs → exit 1). A mis-typed flag exits **2** before the app
starts, so a command that does not measure anything cannot be mistaken for a gate that passed.

The drift table at the top of this file was measured with the harness as it stood *before* the
toggles existed (6 checks, manual `rmdir`); what the toggles changed is the report text and the two
new deltas, not the measurement code, and C7 now catches that drift by itself.

## The numbers, as a baseline to diff against

Mode A is the shipping baseline; B and D add the two toggles' costs. Range = spread across the runs
taken today in that mode.

| Quantity | Mode A (shipping) | Mode B/D (tabs kept alive) |
|---|--:|--:|
| `ICoreWebView2Environment` constructed per process | **1** | **1** |
| `msedgewebview2.exe` **browser** processes (`--type=` absent), 1 tab → 5 tabs | **1 → 1** | **1 → 1** |
| Renderers (`--type=renderer`), 1 tab → 5 tabs | **3 → 3** (identity changes) | **3 → 7** (+1 per live page) |
| Other WebView2 children | **4** (1 gpu-process, 2 utility, 1 crashpad-handler) | **4** |
| Processes in our tree (incl. the shell host) | **9** | **13** |
| Unclassified WebView2 members (unreadable command line) | **0** | **0** |
| Shell process working set | **28.4–38.6 MB** | **28.4–38.7 MB** |
| Browser process working set | **140.5–143.6 MB** at 1 live page; **154.6–163.8 MB** at 5 |
| Renderer working set (all renderers) | **215.4–218.5 MB** for 3 | **159.1–162.2 MB** for 3 → **452.1–457.2 MB** for 7 |
| WebView2 children working set, 1 tab → 5 tabs | **487–489 → 551–557 MB** | **483–487 → 873–879 MB** |
| Whole process tree, 1 tab → 5 tabs | **522–528 → 588–595 MB** | **456–462 (no overlay) → 911–920 MB** |

Two readings of "one webview" fall out of this, and they are different numbers:

* **a page webview that gets slept**: its renderer is destroyed, so it costs ≈ 0 MB of resident
  working set and the tab's entry/title survive;
* **a page webview kept alive**: **+97.2–97.9 MB** of process-tree working set, **+1 renderer
  process**, **+73.1–73.8 MB of renderer working set**, **+0 browser processes**.

## Pricing the product decision: what keeping background tabs alive costs

This is the number `sleep_background_tabs` was written to avoid paying, and the reason the toggle
exists — it is a price, not a feature proposal.

* **97.0–98.2 MB per background tab kept alive** (six measurements, σ ≈ 0.4 MB), on light pages
  (example.com/.org/.net, iana.org). Those are among the cheapest pages on the internet; the shell's
  own comment in `app.rs` cites **≈120 MB per live page** from an earlier 31-webview/4.5 GB
  observation, so treat 97 MB as a **floor** for real content, not a ceiling.
* Arithmetic from that floor, not a measurement: 8 tabs kept alive ≈ **+780 MB**, 20 tabs ≈
  **+1.95 GB**, on top of a ~590 MB five-tab tree.
* The runtime's own ceiling is unchanged by this: 4 extra tabs cost **0** extra browser processes
  either way (C2 green in both modes), so the keep-alive decision is purely a working-set decision,
  not a process-count decision.
* **Nothing in `RELEASE-0.md` sets a budget for this.** If background tabs are ever kept alive by
  default, R0a-1 needs a line item for it; if they stay slept, this file is the number that justifies
  keeping the behaviour.

## Pricing the overlay: C5 as a delta

Six measurements across modes C and D: **+64.2, +65.2, +65.7, +66.9, +67.0, +68.5 MB** — i.e.
**+1 renderer, +0 browser processes, ≈ 66 MB (range 64–69 MB)**.

* Plan said ≈60 MB budget / 80 MB ceiling (`FORK-OR-CDP.md` §3.1): **inside the ceiling, ~11 % over
  the budget estimate**. The plan's process-count prediction (+1 renderer, +0 browser) is exactly
  right.
* An independent cross-check that does not use a before/after pair: mode B at step 0 (overlay
  present, 1 live page) measured 525–528 MB of tree, mode D at step 0 (overlay absent) measured
  456–462 MB — a ~65 MB gap, agreeing with the delta method to within its own spread.
* The plan's alternative reading — that the overlay would *share* the shell's `asset://` renderer and
  cost 0 renderers — is **wrong**: the overlay is a separate `WebViewBuilder::with_id("overlay")`
  controller and reliably gets its own renderer process (2 → 3 renderers on every flip).
* The single-controller-per-window rule in §3.1 is what keeps this a one-time ~66 MB; the check does
  not detect a second controller, only a second *browser process*, so a second overlay controller
  would show up as C5 +66 MB again, not as a C5 failure.

## Not measured

* **Release-build numbers.** Everything above is `cargo`'s dev profile. `--proof` gates whatever
  `cargo run` builds; a release baseline needs `--release` and its own file before any release
  number is quoted against it.
* **Per-renderer MB by origin.** The harness reports renderer MB summed per role, and now reports the
  delta per renderer *added* (73.1–73.8 MB), but it cannot attribute one renderer process to one
  origin — `--type=renderer` is the whole of the discrimination it was asked to do.
* **A second workspace's cost.** That needs a second `ICoreWebView2Environment` on a second
  user-data-folder, which C1/C2 exist to prevent. See §Workspace isolation: the number is needed and
  this harness deliberately cannot produce it yet.
* **The CPU brand string** (no dependency-free Win32 call; the `Get-CimInstance` command in
  §Machine is the source of record).
* **C3 in mode A, C5 in modes A/B, C6 in modes A/C** — each printed as `NOT MEASURED`, naming the
  flag that measures it, rather than green or red. Not-measured is not a failure and is never a pass.

## Reproducing it (three facts that separate numbers from noise)

1. **The WebView2 browser process is keyed on the user-data-folder, not on the host process.** With
   one `browser.exe` already holding `%LOCALAPPDATA%\native-browser\webview2`, a second
   `browser.exe` launched on that folder gets **no `msedgewebview2.exe` children of its own**
   (§Workspace isolation has the direct measurement). A harness that roots its process walk at its
   own pid then counts zero browser processes and zero renderers, which reads as "cost is zero".
   `--proof` therefore redirects `LOCALAPPDATA` to `%LOCALAPPDATA%\native-browser-proof` before any
   path is resolved (`main.rs::private_proof_profile`) and **C0** fails if the tree it counted has no
   browser process of its own. Any run reporting `FAIL C0` measured nothing; do not read its numbers.
2. **The profile is wiped by the harness, and that is load-bearing** — see the first section. Manual
   fix when C7 is red: `rmdir /s /q "%LOCALAPPDATA%\native-browser-proof"`.
3. **Machine state is printed, not assumed.** The `context` line reports other hosts running this
   image and the machine-wide `msedgewebview2.exe` count (22–26 on this box, from SearchHost and
   other WebView2 apps). None of them enter the counts — the tree walk is by descent from our own
   pid — but they are why C0 and the attribution numbers exist.

The user-data-folder the runtime actually creates is one level below the path we hand it: we pass
`<data>\webview2`, the processes carry `--user-data-dir=…\webview2\EBWebView`. Relevant to anyone
grep-ing process command lines for a profile path.

## Workspace isolation: two hosts, one browser process (measured)

`tools/udf-share-probe.ps1` starts two `browser.exe` on the default profile, groups every
`msedgewebview2.exe` by `--user-data-dir` and `--type=`, prints who parents whom, then kills the two
hosts it started. Result, 2026-11-13:

```
HOST_A=18848 HOST_B=31020 BROWSER_EXE_HOSTS_RUNNING=2
MSEDGEWEBVIEW2_MACHINE_WIDE=24
OURS_ON_DEFAULT_PROFILE=10 BROWSER_PROCESSES=1

  pid  15532  ppid 18848  BROWSER            (the only one, and it belongs to host A)
  pid  12976  ppid 15532  crashpad-handler
  pid  31732  ppid 15532  gpu-process
  pid  14824  ppid 15532  renderer
  pid  22860  ppid 15532  renderer
  pid  25640  ppid 15532  renderer
  pid  25996  ppid 15532  renderer
  pid  30848  ppid 15532  renderer
  pid   8480  ppid 15532  utility
  pid  12532  ppid 15532  utility
SURVIVING_WEBVIEWS_ON_OUR_UDF=0 BROWSER_EXE_HOSTS_LEFT=0
```

Two host processes, ten WebView2 processes, **one browser process**, parented to whichever host got
there first; host B has no WebView2 children at all. The same fact holds inside the proof profile in
every harness run above: 7 webviews (shell UI, overlay, one new-tab page + four sites) hang off one
browser process, and machine-wide `msedgewebview2.exe` count being 22–26 while our tree holds 8–12 is
the other half of the same rule — separate folders, separate browser processes; one folder, one
browser process, whoever asked.

What that means for a workspace as an isolation boundary (§8 data/privacy, and §11's "no
cross-workspace leakage" gate):

1. **There is no process-level isolation today, and the runtime will not give it to us for free.**
   Every webview in every window of this app — every workspace's pages, the shell UI, the overlay —
   shares one browser process: one network stack, one cookie jar, one cache, one IndexedDB root, one
   service-worker registration namespace, one crash. §8's "a remote page never receives another
   tab's contents" is currently enforced *only* by our host-side allowlist and by what the shell
   chooses to paint, exactly as `FORK-OR-CDP.md` §2.4 graded it (A, "ours: snapshots built in the
   shell", not a Blink-level guarantee). The measurement above says the OS-level boundary we would
   have wanted is not there.
2. **The only lever is a separate user-data-folder per workspace.** That is the sole documented way
   to get a second browser process, and it is set at environment-creation time. So "workspace =
   isolation boundary" implies **one `ICoreWebView2Environment` per workspace**, which is directly
   against R0a-1's letter: C1 asserts *exactly one* environment per process lifetime and C2 asserts
   *+0 browser processes*. Those assertions are correct for a single-workspace Release 0 and would
   have to be re-scoped to "one environment per user-data-folder, +1 browser process per workspace
   beyond the first" the moment workspace isolation becomes a requirement rather than a nice idea.
3. **The cost of that has to be measured, not guessed.** A second browser process brings its own
   child set with it: measured here, one browser process plus its non-renderer children (gpu, 2 ×
   utility, crashpad) sit inside a 487–489 MB WebView2-children total at 3 renderers, with the
   browser process alone at 140–164 MB. A per-workspace figure needs a two-environment run, which
   this harness deliberately refuses to produce (C1 would fail by design). That is the gap to close,
   not a number to estimate.
4. **Blast radius is the argument that does not need a budget.** Today, the browser process for a
   user-data-folder serves every workspace in the window, so its exit takes all of them: `wry`
   surfaces `add_ProcessFailed` per webview, but the recovery path would have to rebuild *all*
   workspaces' pages, and Release 0 has no such path. Sharing a browser process with the rest of the
   window does not survive that reading of §8 — and, unlike the MB figures, this one does not depend
   on which machine we measure on.
5. **Related deferrals all land on the same lever.** `RELEASE-0.md` §4 defers profiles/`SetProfileName`
   (SDK 10458), workspace partitioning and `--inprivate` (`SetIsInPrivateModeEnabled`, 10466) to
   R0b. All three, plus per-workspace `AdditionalBrowserArguments` (fixed at environment creation —
   `FORK-OR-CDP.md` §2.4's "cannot be flipped at runtime" note), become *possible* at exactly the
   moment we go to one environment per folder, and impossible before it. If §8 is going to require a
   per-workspace boundary, that decision should be made in R0a, not discovered in R0b.

Recommendation for whoever owns §8: prototype it before the design is frozen — extend `--proof` with
a mode that deliberately creates a second environment on a second user-data-folder (the toggles in
`ai/mode.rs` are the pattern; C1/C2 would need to accept the mode's expectation the way C3/C5/C6 now
do), and record per-workspace browser-process MB, child-set MB, and whether one workspace's renderer
crash stays contained. Until that is measured, "workspace isolation" in the §8 sense is an
application-layer promise over a shared browser process, and should be documented as that.

## What the harness measures with, and what it reuses

Nothing here is a second implementation of an existing capability:

| Part | Source |
|---|---|
| Process-tree enumeration, working-set bytes, MB rounding | existing `src/sys.rs` (`process_tree`, `snapshot_processes`, `working_set_bytes`, `mb`) — **reused, not rewritten**; there is still exactly one Toolhelp32 snapshot in the crate |
| Machine-wide WebView2 count and "other hosts running this image" | `ai/processes.rs::attribution`, a filter over that **same** `sys::snapshot_processes()` — no second enumerator |
| `--type=` role split | **added** to `sys.rs` as `command_line_type` (Toolhelp32 carries no command line): `NtQueryInformationProcess(ProcessBasicInformation)` → PEB → `RTL_USER_PROCESS_PARAMETERS::CommandLine`, then `split_whitespace().find_map(|a| a.strip_prefix("--type="))`. It reads that one argument and nothing else, returns `Unreadable` rather than a guess on any failed read, and is reachable only from `--proof` |
| The single `ICoreWebView2Environment` | existing `webview2.rs::create_shared_environment`; the counter is an `AtomicU64` incremented at the construction site inside its completion handler |
| Driving real navigations | existing `--driver` machinery in `app.rs` (settle window per step); `--proof` is one scenario name in `driver_script`, the logic is in `ai/proof.rs` |
| Building / destroying the overlay for C5 | existing overlay construction path, extracted verbatim into `AppState::build_overlay_webview` (it is the same code startup runs, so C5 measures the shipping overlay); teardown is the same `drop` that tab sleeping already relies on |
| The two toggles | new `ai/mode.rs`: `Config::parse` (tabs, `no_tab_sleep`, `no_overlay`, unknown-flag reporting), `install`/`config`/`active` behind a `OnceLock`, and the two guards `app.rs` reads |
| Action levels, the `blocked`/`unsupported` taxonomy, the audit record type | the path dependency `ai-platform-contract` (`../foundation`) — the gate calls `level_of_action_kind` and `ActionLevel::allowed_in_release_zero()` and never restates the table |
| The two-host probe behind §Workspace isolation | `tools/udf-share-probe.ps1` — `Get-CimInstance Win32_Process`, throwaway, kills only what it starts |
