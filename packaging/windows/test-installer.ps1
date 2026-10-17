param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$TestRoot
)
$ErrorActionPreference = 'Stop'
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$target = [System.IO.Path]::GetFullPath($TestRoot)
if (Test-Path -LiteralPath $target) { throw 'Installer test requires a fresh, nonexistent directory.' }
$arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS', '/TASKS=', ('/DIR="' + $target + '"'), '/GROUP="ScreenCue Packaging Test"')
$process = Start-Process -FilePath $installerPath -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
$uninstaller = Join-Path $target 'unins000.exe'
try {
    foreach ($name in @('screencue.exe', 'screencue-listen.exe', 'README.md', 'LICENSE', 'conversation\README.md', 'licenses\CONVERSATION-THIRD-PARTY-LICENSES.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $target $name) -PathType Leaf)) { throw "Installed file missing: $name" }
    }
    $overlayVersion = & (Join-Path $target 'screencue.exe') --version
    if ($LASTEXITCODE -ne 0 -or $overlayVersion -notmatch '^\d+\.\d+\.\d+') { throw 'Installed overlay did not start.' }
    $helperVersion = & (Join-Path $target 'screencue-listen.exe') --version
    if ($LASTEXITCODE -ne 0 -or $helperVersion -notmatch 'screencue-listen') { throw 'Installed audio helper did not start.' }
}
finally {
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        $remove = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -WindowStyle Hidden -PassThru -Wait
        if ($remove.ExitCode -ne 0) { throw "Uninstaller failed: $($remove.ExitCode)" }
    }
}
if (Test-Path -LiteralPath (Join-Path $target 'screencue.exe')) { throw 'Uninstall retained the application executable.' }
Write-Output 'PASS: per-user installation, both installed executables, notices, and uninstall.'
