# Throwaway probe: does one WebView2 browser process serve two host processes that
# point at the same user-data-folder? The --proof harness already proves the
# one-host case; this is the two-host case behind the workspace-isolation question
# recorded in baseline.md, §"Workspace isolation: two hosts, one browser process".
# Launches two instances of the dev build on the default profile, groups every
# msedgewebview2.exe by its user-data-dir and --type= token, prints which host each
# one descends from, then kills both hosts it started (and nothing else).
$ErrorActionPreference = 'Continue'
$exe = 'Q:\gitroot\Game\native-browser\target\debug\browser.exe'

function Get-Tree([int[]]$roots) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $front = @($roots)
    $seen = @()
    while ($front.Count -gt 0) {
        $next = @()
        foreach ($p in $all) {
            if ($front -contains $p.ParentProcessId) { $next += $p.ProcessId }
        }
        $seen += $next
        $front = @($next)
    }
    return $seen
}

$a = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 12
$b = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 15

$hosts = @(Get-CimInstance Win32_Process -Filter "Name='browser.exe'" | ForEach-Object { $_.ProcessId })
$tree = Get-Tree @($a.Id, $b.Id)

$rows = Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | ForEach-Object {
    $cl = $_.CommandLine
    $role = 'UNREADABLE'
    $udf = '?'
    if ($cl) {
        if ($cl -match '--type=([A-Za-z0-9\-_]+)') { $role = $Matches[1] } else { $role = 'BROWSER' }
        if ($cl -match '--user-data-dir=("?)([^"\s]+)\1') { $udf = $Matches[2] }
    }
    [pscustomobject]@{
        pid     = $_.ProcessId
        ppid    = $_.ParentProcessId
        role    = $role
        udf     = $udf
        inTree  = ($tree -contains $_.ProcessId)
    }
}

$ours = @($rows | Where-Object { $_.udf -like '*native-browser\webview2*' })
Write-Host ("HOST_A={0} HOST_B={1} BROWSER_EXE_HOSTS_RUNNING={2}" -f $a.Id, $b.Id, $hosts.Count)
Write-Host ("MSEDGEWEBVIEW2_MACHINE_WIDE={0}" -f $rows.Count)
Write-Host ("OURS_ON_DEFAULT_PROFILE={0} BROWSER_PROCESSES={1}" -f $ours.Count, @($ours | Where-Object { $_.role -eq 'BROWSER' }).Count)
$ours | Sort-Object role, pid | Format-Table pid, ppid, role, inTree, udf -AutoSize | Out-String -Width 220 | Write-Host
if ($ours.Count -eq 0) {
    Write-Host '--- no match; every distinct user-data-dir seen, with counts ---'
    $rows | Group-Object udf | Sort-Object Count -Descending | ForEach-Object {
        Write-Host ("{0,3}  {1}" -f $_.Count, $_.Name)
    }
}

Stop-Process -Id $a.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $b.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 5
$left = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -like '*native-browser\webview2*' }).Count
$hostsLeft = @(Get-CimInstance Win32_Process -Filter "Name='browser.exe'").Count
Write-Host ("SURVIVING_WEBVIEWS_ON_OUR_UDF={0} BROWSER_EXE_HOSTS_LEFT={1}" -f $left, $hostsLeft)
