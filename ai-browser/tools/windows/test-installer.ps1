#Requires -RunAsAdministrator
param(
    [string]$BrowserPath,
    [string]$Target
)

$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if (!$BrowserPath) {
    $buildRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $workspace 'target' }
    if ($Target) { $buildRoot = Join-Path $buildRoot $Target }
    $BrowserPath = Join-Path $buildRoot 'release\browser.exe'
}
$BrowserPath = (Resolve-Path -LiteralPath $BrowserPath).Path
$id = 'honkliu.ai-browser.test.' + [Guid]::NewGuid().ToString('N')
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\target\installer-tests\$id"))
$appName = "AI Browser Installer Test $id"
$install = Join-Path $env:ProgramW6432 $appName
$registry = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appName"
$browser = $null
$uninstalled = $false

function Assert($Condition, [string]$Message) {
    if (!$Condition) { throw $Message }
}

function Run-Setup([string]$File, [string[]]$Arguments) {
    $process = Start-Process -FilePath $File -ArgumentList $Arguments -PassThru
    if (!$process.WaitForExit(120000)) {
        Stop-Process -Id $process.Id -Force
        throw "Installer timed out: $File"
    }
    Assert ($process.ExitCode -eq 0) "Installer failed with exit code $($process.ExitCode): $File"
}

try {
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    Push-Location $workspace
    try {
        $metadata = (& cargo metadata --locked --no-deps --format-version 1) | ConvertFrom-Json
        Assert ($LASTEXITCODE -eq 0) 'Cargo metadata failed.'
        $package = $metadata.packages | Where-Object { $_.name -eq 'browser' }
        $config = $package.metadata.packager
        $shell = Split-Path -Parent $package.manifest_path
        $config.'before-packaging-command' = $null
        $config.'product-name' = $appName
        $config.identifier = $id
        $config.'out-dir' = $root
        $config.icons = @($config.icons | ForEach-Object { Join-Path $shell $_ })
        $config.nsis.'installer-icon' = Join-Path $shell $config.nsis.'installer-icon'
        $config.macos.'info-plist-path' = Join-Path $shell $config.macos.'info-plist-path'
        $config | Add-Member -NotePropertyName version -NotePropertyValue $package.version
        $config | Add-Member -NotePropertyName binariesDir -NotePropertyValue (Split-Path -Parent $BrowserPath)
        $config | Add-Member -NotePropertyName binaries -NotePropertyValue @(@{ path = 'browser'; main = $true })
        $configPath = Join-Path $root 'test-packager.json'
        [IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
        $packageArguments = @('packager', '--config', $configPath, '--formats', 'nsis')
        if ($Target) { $packageArguments += @('--target', $Target) }
        & cargo @packageArguments
        Assert ($LASTEXITCODE -eq 0) 'Cargo packaging failed.'
    } finally { Pop-Location }
    $setups = @(Get-ChildItem -LiteralPath $root -Filter '*-setup.exe')
    Assert ($setups.Count -eq 1) 'Expected exactly one architecture-specific installer.'
    $setup = $setups[0].FullName
    $arguments = @('/S', '/NS')
    Run-Setup $setup $arguments
    Assert (Test-Path -LiteralPath $registry) 'Uninstall registration is missing.'
    Assert ((Get-ItemPropertyValue -LiteralPath $registry -Name InstallLocation).Trim('"').TrimEnd('\') -eq $install) 'Setup did not use the default Program Files directory.'
    $files = @('browser.exe', 'uninstall.exe')
    foreach ($name in $files) {
        Assert (Test-Path -LiteralPath (Join-Path $install $name)) "Missing installed file: $name"
    }
    Assert ((Get-FileHash -LiteralPath (Join-Path $install 'browser.exe')).Hash -eq (Get-FileHash -LiteralPath $BrowserPath).Hash) 'Setup packaged a different browser build.'

    $data = Join-Path $install 'user-data'
    New-Item -ItemType Directory -Path $data | Out-Null
    $marker = Join-Path $data 'settings.json'
    Set-Content -LiteralPath $marker -Value '{"preserve":true}'
    Run-Setup $setup $arguments
    Assert (Test-Path -LiteralPath $marker) 'Reinstallation deleted user data.'

    $previousProfile = $env:AI_BROWSER_DATA_DIR
    $profile = Join-Path $root 'profile'
    try {
        $env:AI_BROWSER_DATA_DIR = $profile
        $browser = Start-Process -FilePath (Join-Path $install 'browser.exe') -ArgumentList '--self-test=native://newtab' -PassThru
    } finally {
        $env:AI_BROWSER_DATA_DIR = $previousProfile
    }
    if (!$browser.WaitForExit(60000)) { throw 'Installed browser did not complete its startup self-test.' }
    Assert ($browser.ExitCode -eq 0) "Installed browser failed with exit code $($browser.ExitCode)."
    Assert (Test-Path -LiteralPath (Join-Path $profile 'self-test.json')) 'Installed browser did not write its self-test report.'

    Run-Setup (Join-Path $install 'uninstall.exe') @('/S', "_?=$install")
    $uninstalled = $true
    Assert (!(Test-Path -LiteralPath $registry)) 'Uninstall registration was not removed.'
    Assert (!(Test-Path -LiteralPath (Join-Path $install 'browser.exe'))) 'Uninstall left the browser executable behind.'
    Assert (Test-Path -LiteralPath $marker) 'Uninstall deleted user-created data.'
    Write-Output 'PASS: default Program Files directory, machine registration, installer payload, reinstall, isolated browser startup, uninstall and data preservation.'
} finally {
    if ($browser -and !$browser.HasExited) { Stop-Process -Id $browser.Id -Force }
    $uninstaller = Join-Path $install 'uninstall.exe'
    if (!$uninstalled -and (Test-Path -LiteralPath $uninstaller)) {
        Run-Setup $uninstaller @('/S', "_?=$install")
    }
    foreach ($path in @($install, $root)) {
        for ($attempt = 0; $attempt -lt 50 -and (Test-Path -LiteralPath $path); $attempt++) {
            try { Remove-Item -LiteralPath $path -Recurse -Force }
            catch {
                if ($attempt -eq 49) { throw }
                Start-Sleep -Milliseconds 200
            }
        }
    }
}
