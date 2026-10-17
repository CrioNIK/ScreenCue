param(
    [Parameter(Mandatory = $true)][string]$Stage,
    [Parameter(Mandatory = $true)][string]$Output,
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$CompilerCache = (Join-Path ([System.IO.Path]::GetTempPath()) 'ScreenCue-build-tools\inno-6.7.3')
)
$ErrorActionPreference = 'Stop'
$stagePath = (Resolve-Path -LiteralPath $Stage).Path
$repoPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$outputPath = [System.IO.Path]::GetFullPath($Output)
$compilerPath = [System.IO.Path]::GetFullPath($CompilerCache)
if ($Version -notmatch '^\d+\.\d+\.\d+(?:[a-z]|-[a-z])?$') { throw 'Invalid ScreenCue release version.' }
foreach ($name in @('screencue.exe', 'screencue-listen.exe', 'LICENSE', 'README.md')) {
    if (-not (Test-Path -LiteralPath (Join-Path $stagePath $name) -PathType Leaf)) { throw "Missing staged input: $name" }
}
$iscc = Join-Path $compilerPath 'ISCC.exe'
if (-not (Test-Path -LiteralPath $iscc -PathType Leaf)) {
    [void](New-Item -ItemType Directory -Path $compilerPath -Force)
    $download = Join-Path $compilerPath 'innosetup-6.7.3.exe'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $download
    $signature = Get-AuthenticodeSignature -LiteralPath $download
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Pyrsys B\.V\.') {
        throw 'Inno Setup download did not have a valid expected publisher signature.'
    }
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/CURRENTUSER', '/NOICONS', ('/DIR="' + $compilerPath + '"'))
    $install = Start-Process -FilePath $download -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
    if ($install.ExitCode -ne 0) { throw "Inno Setup compiler installation failed: $($install.ExitCode)" }
}
[void](New-Item -ItemType Directory -Path $outputPath -Force)
& $iscc "/DAppVersion=$Version" "/DSourceDir=$stagePath" "/DOutputPath=$outputPath" "/DIconPath=$(Join-Path $repoPath 'assets\app-icon.ico')" (Join-Path $PSScriptRoot 'ScreenCue.iss')
if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed: $LASTEXITCODE" }
$installerPath = Join-Path $outputPath "ScreenCue-$Version-windows-x64-setup.exe"
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) { throw 'Installer output is missing.' }
Write-Output $installerPath
