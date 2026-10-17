param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$TestRoot
)
$ErrorActionPreference = 'Stop'
function Read-InstalledVersion([string]$Path) {
    # Explicit pipes and waiting are needed for a Windows GUI-subsystem binary.
    # PowerShell's implicit native pipeline can close its stdout before exit.
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $Path
    $start.Arguments = '--version'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($start)
    try {
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(10000)) {
            $process.Kill()
            $process.WaitForExit()
            throw 'Installed executable version check timed out.'
        }
        if ($process.ExitCode -ne 0) { throw ('Installed executable failed: ' + $errors.GetAwaiter().GetResult()) }
        return $output.GetAwaiter().GetResult().Trim()
    }
    finally { $process.Dispose() }
}
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
    $overlayVersion = Read-InstalledVersion (Join-Path $target 'screencue.exe')
    if ($overlayVersion -notmatch '^\d+\.\d+\.\d+') { throw 'Installed overlay did not start.' }
    $helperVersion = Read-InstalledVersion (Join-Path $target 'screencue-listen.exe')
    if ($helperVersion -notmatch 'screencue-listen') { throw 'Installed audio helper did not start.' }
}
finally {
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        $remove = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -WindowStyle Hidden -PassThru -Wait
        if ($remove.ExitCode -ne 0) { throw "Uninstaller failed: $($remove.ExitCode)" }
    }
}
if (Test-Path -LiteralPath (Join-Path $target 'screencue.exe')) { throw 'Uninstall retained the application executable.' }
Write-Output 'PASS: per-user installation, both installed executables, notices, and uninstall.'
