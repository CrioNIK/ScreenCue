param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\release\screencue.exe'),
    [ValidateRange(2, 30)][int]$SampleSeconds = 5
)

# Windows PowerShell 5.1+. Owns one --json process and closes it on every exit.
# Does not capture the desktop, synthesize input, or change other applications.
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path -LiteralPath $Executable).Path

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ScreenCueRustNativeTest {
    private delegate bool EnumCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] public static extern bool IsWindowVisible(IntPtr window);
    public static IntPtr FindOwnedOverlay(uint processId) {
        IntPtr result = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var title = new StringBuilder(256); GetWindowText(window, title, title.Capacity);
            if (title.ToString() != "ScreenCue Overlay") return true;
            result = window; return false;
        }, IntPtr.Zero);
        return result;
    }
}
'@

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Send-Command([hashtable]$Command, [bool]$ExpectedOk = $true) {
    $line = $Command | ConvertTo-Json -Depth 12 -Compress
    $script:inputWriter.WriteLine($line)
    $script:inputWriter.Flush()
    $pending = $script:overlay.StandardOutput.ReadLineAsync()
    if (-not $pending.Wait(10000)) { throw ('JSON response timeout for ' + $Command.command) }
    $response = $pending.GetAwaiter().GetResult()
    if ($null -eq $response) { throw 'Overlay closed stdout without a response.' }
    $reply = $response | ConvertFrom-Json
    Assert-That ($reply.ok -eq $ExpectedOk) ('Unexpected response: ' + $response)
    return $reply
}

function Measure-Idle([string]$Phase) {
    Start-Sleep -Milliseconds 500
    $script:overlay.Refresh()
    $before = $script:overlay.TotalProcessorTime.TotalMilliseconds
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $SampleSeconds
    $script:overlay.Refresh()
    $timer.Stop()
    $cpu = $script:overlay.TotalProcessorTime.TotalMilliseconds - $before
    return [pscustomobject]@{
        phase = $Phase
        sample_seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 3)
        cpu_milliseconds = [Math]::Round($cpu, 3)
        cpu_one_core_percent = [Math]::Round(100 * $cpu / $timer.Elapsed.TotalMilliseconds, 4)
        working_set_mib = [Math]::Round($script:overlay.WorkingSet64 / 1MB, 2)
        private_bytes_mib = [Math]::Round($script:overlay.PrivateMemorySize64 / 1MB, 2)
        handles = $script:overlay.HandleCount
    }
}

$start = New-Object System.Diagnostics.ProcessStartInfo
$start.FileName = $Executable
$start.Arguments = '--json'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.StandardOutputEncoding = New-Object System.Text.UTF8Encoding($false)
$start.StandardErrorEncoding = New-Object System.Text.UTF8Encoding($false)
$script:overlay = New-Object System.Diagnostics.Process
$script:overlay.StartInfo = $start
$script:inputWriter = $null
$started = $false
try {
    $started = $script:overlay.Start()
    # .NET Framework ProcessStartInfo has no StandardInputEncoding property.
    $script:inputWriter = New-Object System.IO.StreamWriter($script:overlay.StandardInput.BaseStream, (New-Object System.Text.UTF8Encoding($false)), 1024, $true)
    $errors = $script:overlay.StandardError.ReadToEndAsync()
    $status = Send-Command @{ command = 'status' }
    Assert-That ($status.pid -eq $script:overlay.Id) 'Response belongs to a different process.'
    Assert-That (-not $status.visible) 'An idle JSON connection unexpectedly displayed a hint.'
    $window = [ScreenCueRustNativeTest]::FindOwnedOverlay([uint32]$script:overlay.Id)
    Assert-That ($window -ne [IntPtr]::Zero) 'The owned native overlay window was not found.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Idle native window is visible.'
    $samples = @()
    $samples += Measure-Idle 'hidden'

    $geometry = Send-Command @{ command = 'displays' }
    $displays = @($geometry.displays)
    Assert-That ($displays.Count -gt 0) 'No monitor geometry reported.'
    $monitor = $displays[0]
    Assert-That ($monitor.width -gt 100 -and $monitor.height -gt 100) 'Test needs a display at least 100 pixels wide/high.'
    $x = [int]$monitor.x + [int][Math]::Floor($monitor.width / 2)
    $y = [int]$monitor.y + [int][Math]::Floor($monitor.height / 2)
    $unicode = ([char]0x041D).ToString() + [char]0x0430 + [char]0x0436 + [char]0x043C + [char]0x0438 + [char]0x0442 + [char]0x0435 + ' "Settings" / ' + [char]0x4E16 + [char]0x754C
    $shown = Send-Command @{ command = 'show'; x = $x; y = $y; text = $unicode; color = '#AC4DFF'; duration_ms = 0 }
    Assert-That ($shown.color_source -eq 'manual') 'Explicit color unexpectedly sampled the screen.'
    $status = Send-Command @{ command = 'status' }
    Assert-That ($status.visible -and $status.hint.text -ceq $unicode) 'Unicode JSON round-trip failed.'
    Assert-That ([ScreenCueRustNativeTest]::IsWindowVisible($window)) 'show did not display the native window.'
    # Layered + transparent + tool window + no activation + topmost.
    $required = 0x080800A8
    $styles = [ScreenCueRustNativeTest]::GetWindowLong($window, -20)
    Assert-That (($styles -band $required) -eq $required) 'Missing native click-through/no-activation/topmost styles.'
    $samples += Measure-Idle 'visible_static_hint'
    Assert-That ((Send-Command @{ command = 'status' }).visible) 'Persistent hint expired.'

    foreach ($shape in @('ellipse', 'rectangle', 'rounded')) {
        [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; shape = $shape; width = 60; height = 40; arrow = $false; target = $true; color = '#FFAA33' })
        Assert-That ([ScreenCueRustNativeTest]::IsWindowVisible($window)) ('Marker-only shape is not visible: ' + $shape)
    }
    $points = @(@{ x = $x - 30; y = $y - 20 }, @{ x = $x + 30; y = $y - 20 }, @{ x = $x; y = $y + 30 })
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; shape = 'polygon'; points = $points; color = '#FFAA33' })
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; arrow = $true; target = $false; color = '#FFAA33' })
    $palette = Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = 'auto'; background_colors = @('#FAFAFA', '#202124') }
    Assert-That ($palette.color_source -eq 'client_palette') 'Client palette did not bypass local sampling.'

    [void](Send-Command @{ command = 'clear' })
    Assert-That (-not (Send-Command @{ command = 'status' }).visible) 'clear retained logical visibility.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'clear retained native visibility.'
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = '#AC4DFF'; duration_ms = 500 })
    Start-Sleep -Milliseconds 900
    Assert-That (-not (Send-Command @{ command = 'status' }).visible) 'Expiry retained logical visibility.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Expiry retained native visibility.'

    foreach ($invalid in @(
        @{ command = 'show'; x = 1.5; y = $y },
        @{ command = 'show'; x = $x; y = $y; duration_ms = 499 },
        @{ command = 'show'; x = $x; y = $y; color = '#GG0000' },
        @{ command = 'show'; x = $x; y = $y; arrow = 'false' },
        @{ command = 'show'; x = -2147483648; y = -2147483648 },
        @{ command = 'show'; x = $x; y = $y; shape = 'polygon'; points = @() }
    )) { [void](Send-Command $invalid $false) }
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Rejected input displayed a hint.'

    # EOF, rather than a separate CLI quit command, ends this overlay session.
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = '#AC4DFF'; duration_ms = 0 })
    $script:inputWriter.Dispose()
    $script:overlay.StandardInput.Close()
    Assert-That ($script:overlay.WaitForExit(5000)) 'Process did not exit after stdin EOF.'
    Assert-That ($script:overlay.ExitCode -eq 0) 'Process returned a failure on normal EOF.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'EOF left the native overlay visible.'
    $remaining = $script:overlay.StandardOutput.ReadToEnd()
    Assert-That ([string]::IsNullOrWhiteSpace($remaining)) 'Unexpected stdout after final command.'
    $errorText = $errors.GetAwaiter().GetResult()
    Assert-That ([string]::IsNullOrWhiteSpace($errorText)) ('Unexpected diagnostics: ' + $errorText)
    Write-Host 'PASS: owned JSON lifecycle, native visibility/styles, Unicode, shapes, palette, validation, expiry and EOF.'
    Write-Host 'Observed process metrics; these are not performance guarantees:'
    $samples | ConvertTo-Json -Depth 3
}
finally {
    if ($null -ne $script:inputWriter) { $script:inputWriter.Dispose() }
    if ($started -and -not $script:overlay.HasExited) {
        try { $script:overlay.StandardInput.Close() } catch { }
        if (-not $script:overlay.WaitForExit(3000)) { $script:overlay.Kill(); [void]$script:overlay.WaitForExit(3000) }
    }
    $script:overlay.Dispose()
}
