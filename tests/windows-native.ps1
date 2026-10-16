param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\release\screencue.exe'),
    [ValidateRange(2, 30)][int]$SampleSeconds = 5
)

# Windows PowerShell 5.1. Owns one --json process and closes it on every exit.
# Does not save screenshots, synthesize input, or change other applications.
# The explicit auto-color phase samples transient pixels through ScreenCue.
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
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr GetProp(IntPtr window, string name);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
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
    public static IntPtr FindOwnedHotkeyWindow(uint processId) {
        IntPtr result = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var name = new StringBuilder(256); GetClassName(window, name, name.Capacity);
            if (name.ToString() != "global_hotkey_app") return true;
            result = window; return false;
        }, IntPtr.Zero);
        return result;
    }
    public static bool IsAbove(IntPtr upper, IntPtr lower) {
        int order = 0, upperOrder = -1, lowerOrder = -1;
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            if (window == upper) upperOrder = order;
            if (window == lower) lowerOrder = order;
            order++;
            return upperOrder < 0 || lowerOrder < 0;
        }, IntPtr.Zero);
        return upperOrder >= 0 && lowerOrder >= 0 && upperOrder < lowerOrder;
    }
}
'@

Add-Type -Path (Join-Path $PSScriptRoot 'windows-tracking-window.cs')

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Send-Command([hashtable]$Command, [bool]$ExpectedOk = $true) {
    # Unrelated test cases use stable desktop coordinates. Only the dedicated
    # owned-window test enables tracking.
    if ($Command.command -eq 'show' -and -not $Command.ContainsKey('follow_window')) { $Command.follow_window = $false }
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

function Wait-Tracking([scriptblock]$Condition, [string]$Failure) {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        $reply = Send-Command @{ command = 'status' }
        if (& $Condition $reply) { return $reply }
        Start-Sleep -Milliseconds 20
    } while ($timer.ElapsedMilliseconds -lt 1500)
    throw ($Failure + ': ' + ($reply | ConvertTo-Json -Depth 12 -Compress))
}

function Assert-RenderedOnDisplay($Status, $Displays, [IntPtr]$Window) {
    Assert-That ($Status.visible -and [ScreenCueRustNativeTest]::IsWindowVisible($Window)) 'Pinned tracking cue disappeared.'
    Assert-That (-not [string]::IsNullOrWhiteSpace($Status.tracking.note)) 'Pinned tracking cue has no explanatory note.'
    Assert-That (@($Status.rendered_target).Count -eq 2) 'Status omitted the rendered target coordinates.'
    $bounds = [ScreenCueTrackingWindow]::PhysicalBounds($Window)
    $fits = @($Displays | Where-Object {
        $Status.rendered_target[0] -ge $_.x -and $Status.rendered_target[0] -lt ($_.x + $_.width) -and
        $Status.rendered_target[1] -ge $_.y -and $Status.rendered_target[1] -lt ($_.y + $_.height) -and
        $bounds[0] -ge $_.x -and $bounds[1] -ge $_.y -and
        $bounds[2] -le ($_.x + $_.width) -and $bounds[3] -le ($_.y + $_.height)
    })
    Assert-That ($fits.Count -gt 0) 'Pinned cue or its native frame extends outside every display.'
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

function Assert-ShortcutOwnership([int]$X, [int]$Y, [bool]$ExpectedActive) {
    # A second owned instance verifies real OS registration, both after the
    # first releases its shortcut and while the first holds it.
    $helper = New-Object System.Diagnostics.Process
    $helper.StartInfo = $start
    $helperStarted = $false
    $helperInput = $null
    $raisedWindow = $null
    try {
        $helperStarted = $helper.Start()
        $helperErrors = $helper.StandardError.ReadToEndAsync()
        $helperInput = New-Object System.IO.StreamWriter($helper.StandardInput.BaseStream, (New-Object System.Text.UTF8Encoding($false)), 1024, $true)
        $request = @{ command = 'show'; x = $X; y = $Y; text = ''; color = '#AC4DFF'; duration_ms = 0; animate = $false; follow_window = $false } | ConvertTo-Json -Compress
        $helperInput.WriteLine($request)
        $helperInput.Flush()
        $pending = $helper.StandardOutput.ReadLineAsync()
        Assert-That ($pending.Wait(10000)) 'Shortcut conflict helper timed out.'
        $reply = $pending.GetAwaiter().GetResult() | ConvertFrom-Json
        Assert-That ($reply.ok -eq $true -and $reply.visible -eq $true) 'A shortcut conflict incorrectly blocked showing the helper cue.'
        Assert-That ($reply.shortcut_active -eq $ExpectedActive) 'Native shortcut ownership disagrees with the primary process state.'
        if ($ExpectedActive) {
            Assert-That ([string]::IsNullOrWhiteSpace($reply.shortcut_error)) ('Released shortcut could not be acquired: ' + $reply.shortcut_error)
        }
        else {
            Assert-That (-not [string]::IsNullOrWhiteSpace($reply.shortcut_error)) 'Shortcut conflict was not explicitly reported.'
            $peerWindow = [ScreenCueRustNativeTest]::FindOwnedOverlay([uint32]$helper.Id)
            Assert-That ($peerWindow -ne [IntPtr]::Zero) 'The second owned overlay was not found.'
            Assert-That ([ScreenCueRustNativeTest]::GetProp($peerWindow, 'ScreenCue.Overlay.v1') -eq [IntPtr]1) 'Peer overlay marker is missing.'
            Start-Sleep -Milliseconds 200
            $peerAbove = [ScreenCueRustNativeTest]::IsAbove($peerWindow, $window)
            for ($i = 0; $i -lt 10; $i++) {
                Start-Sleep -Milliseconds 50
                Assert-That ([ScreenCueRustNativeTest]::IsAbove($peerWindow, $window) -eq $peerAbove) 'Idle peer overlays repeatedly changed their relative Z order.'
            }
            $raisedWindow = New-Object ScreenCueTrackingWindow(($X - 100), ($Y - 80), 240, 180)
            Assert-That ([ScreenCueRustNativeTest]::GetProp($raisedWindow.Handle, 'ScreenCue.Overlay.v1') -eq [IntPtr]::Zero) 'Owned target helper incorrectly has an overlay marker.'
            $restackTimer = [System.Diagnostics.Stopwatch]::StartNew()
            do {
                $bothAbove = [ScreenCueRustNativeTest]::IsAbove($window, $raisedWindow.Handle) -and [ScreenCueRustNativeTest]::IsAbove($peerWindow, $raisedWindow.Handle)
                if ($bothAbove) { break }
                Start-Sleep -Milliseconds 20
            } while ($restackTimer.ElapsedMilliseconds -lt 1500)
            Assert-That $bothAbove 'An explicitly raised owned target remained above an overlay.'
            Assert-That ([ScreenCueRustNativeTest]::GetForegroundWindow() -ne $raisedWindow.Handle) 'Peer restacking activated the owned target helper.'
        }
    }
    finally {
        if ($null -ne $raisedWindow) { try { $raisedWindow.Dispose() } catch { } }
        if ($null -ne $helperInput) { try { $helperInput.Dispose() } catch { } }
        if ($helperStarted -and -not $helper.HasExited) {
            $helper.StandardInput.Close()
            if (-not $helper.WaitForExit(3000)) { $helper.Kill(); [void]$helper.WaitForExit(3000) }
        }
        $helper.Dispose()
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
$trackingWindow = $null
$started = $false
try {
    $started = $script:overlay.Start()
    # .NET Framework ProcessStartInfo has no StandardInputEncoding property.
    $script:inputWriter = New-Object System.IO.StreamWriter($script:overlay.StandardInput.BaseStream, (New-Object System.Text.UTF8Encoding($false)), 1024, $true)
    $errors = $script:overlay.StandardError.ReadToEndAsync()
    $status = Send-Command @{ command = 'status' }
    Assert-That ($status.pid -eq $script:overlay.Id) 'Response belongs to a different process.'
    Assert-That (-not $status.visible) 'An idle JSON connection unexpectedly displayed a hint.'
    Assert-That ($status.shortcut_active -eq $false) 'Hidden process should not hold the global shortcut.'
    $window = [ScreenCueRustNativeTest]::FindOwnedOverlay([uint32]$script:overlay.Id)
    Assert-That ($window -ne [IntPtr]::Zero) 'The owned native overlay window was not found.'
    Assert-That ([ScreenCueRustNativeTest]::GetProp($window, 'ScreenCue.Overlay.v1') -eq [IntPtr]1) 'Owned overlay marker is missing.'
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
    $shown = Send-Command @{ command = 'show'; x = $x; y = $y; text = $unicode; color = '#AC4DFF'; animate = $false }
    Assert-That ($shown.color_source -eq 'manual') 'Explicit color unexpectedly sampled the screen.'
    $status = Send-Command @{ command = 'status' }
    Assert-That ($status.visible -and $status.hint.text -ceq $unicode) 'Unicode JSON round-trip failed.'
    Assert-That ($status.hint.duration_ms -eq 0) 'A default hint unexpectedly has an expiry timer.'
    Assert-That ($status.shortcut_active -eq $true) ('Global shortcut registration failed: ' + $status.shortcut_error)
    Assert-That ([ScreenCueRustNativeTest]::IsWindowVisible($window)) 'show did not display the native window.'
    Assert-That ([ScreenCueRustNativeTest]::GetForegroundWindow() -ne $window) 'show took keyboard focus from the working application.'
    # Layered + transparent + tool window + no activation + topmost.
    $required = 0x080800A8
    $styles = [ScreenCueRustNativeTest]::GetWindowLong($window, -20)
    Assert-That (($styles -band $required) -eq $required) ('Missing native click-through/no-activation/topmost styles: actual 0x{0:X8}, required 0x{1:X8}.' -f $styles, $required)
    $samples += Measure-Idle 'visible_static_fixed_hint'
    # Default hints must survive beyond the former 15-second default.
    if ($SampleSeconds -lt 16) { Start-Sleep -Seconds (16 - $SampleSeconds) }
    Assert-That ((Send-Command @{ command = 'status' }).visible) 'Persistent hint expired.'
    Assert-That ([ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Default hint disappeared without a clear command.'
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = $unicode; color = '#AC4DFF'; duration_ms = 0; animate = $true })
    $samples += Measure-Idle 'visible_animated_fixed_hint'
    $automatic = Send-Command @{ command = 'show'; x = $x; y = $y; text = $unicode; color = 'auto'; duration_ms = 0; animate = $true }
    Assert-That ($automatic.color_source -eq 'local_pixels') ('Windows local color sampling unavailable: ' + $automatic.color_note)
    $samples += Measure-Idle 'visible_animated_fixed_auto_color'
    Assert-That ([ScreenCueRustNativeTest]::GetForegroundWindow() -ne $window) 'Animation or automatic color refresh activated the overlay.'

    foreach ($shape in @('ellipse', 'rectangle', 'rounded')) {
        [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; shape = $shape; width = 60; height = 40; arrow = $false; target = $true; color = '#FFAA33' })
        Assert-That ([ScreenCueRustNativeTest]::IsWindowVisible($window)) ('Marker-only shape is not visible: ' + $shape)
    }
    $points = @(@{ x = $x - 30; y = $y - 20 }, @{ x = $x + 30; y = $y - 20 }, @{ x = $x; y = $y + 30 })
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; shape = 'polygon'; points = $points; color = '#FFAA33' })
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; arrow = $true; target = $false; color = '#FFAA33' })
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; arrow_scale = 2.3; color = '#FFAA33' })
    $palette = Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = 'auto'; background_colors = @('#FAFAFA', '#202124') }
    Assert-That ($palette.color_source -eq 'client_palette') 'Client palette did not bypass local sampling.'

    # A layout failure must not destroy the last successfully displayed cue.
    # This is below the character limit but far too tall to fit on the screen.
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = 'Keep the previous valid cue'; color = '#AC4DFF'; duration_ms = 0; animate = $false })
    $previousStatus = Send-Command @{ command = 'status' }
    $previousHint = $previousStatus.hint | ConvertTo-Json -Depth 12 -Compress
    $oversizedText = 'A' + ("`n" * 200) + 'B'
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = $oversizedText; color = '#FFAA33'; duration_ms = 0; animate = $false } $false)
    $status = Send-Command @{ command = 'status' }
    Assert-That ($status.visible -and $status.shortcut_active) 'Rejected replacement cleared the existing cue or its shortcut.'
    Assert-That (($status.hint | ConvertTo-Json -Depth 12 -Compress) -ceq $previousHint) 'Rejected replacement changed the previous hint.'
    Assert-That ($status.color -ceq $previousStatus.color -and $status.color_source -ceq $previousStatus.color_source) 'Rejected replacement changed the previous color state.'
    Assert-That ([ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Rejected replacement hid the existing native window.'

    # Only this owned, non-activating helper is moved/minimized. No screenshots
    # or synthetic input are needed to verify physical target translation.
    $trackingWindow = New-Object ScreenCueTrackingWindow(($x - 100), ($y - 80), 240, 180)
    Assert-That ([ScreenCueRustNativeTest]::GetForegroundWindow() -ne $trackingWindow.Handle) 'Owned helper unexpectedly activated.'
    Assert-That (([ScreenCueRustNativeTest]::GetWindowLong($trackingWindow.Handle, -20) -band 8) -eq 8) 'Owned helper did not become topmost for reliable target capture.'
    $origin = [ScreenCueTrackingWindow]::PhysicalBounds($trackingWindow.Handle)
    $anchorX = $origin[0] + 100
    $anchorY = $origin[1] + 90
    [void](Send-Command @{ command = 'show'; x = $anchorX; y = $anchorY; text = ''; color = '#EF232D'; animate = $false; follow_window = $true })
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'window' } 'Could not bind to the owned target window'
    $samples += Measure-Idle 'visible_static_window_tracking'
    [void](Send-Command @{ command = 'show'; x = $anchorX; y = $anchorY; text = ''; color = 'auto'; animate = $true; follow_window = $true })
    $samples += Measure-Idle 'visible_animated_window_tracking_auto_color'
    [void](Send-Command @{ command = 'show'; x = $anchorX; y = $anchorY; text = ''; color = '#EF232D'; animate = $false; follow_window = $true })
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'window' } 'Could not reset the owned target tracking fixture'
    $initialFrame = [ScreenCueTrackingWindow]::PhysicalBounds($window)
    $initialTip = @($tracked.arrow_tip)
    Assert-That ($initialTip.Count -eq 2) 'Tracked status omitted arrow_tip.'
    $movement = [System.Diagnostics.Stopwatch]::StartNew()
    $trackingWindow.Move(($origin[0] + 48), ($origin[1] + 32))
    $moved = [ScreenCueTrackingWindow]::PhysicalBounds($trackingWindow.Handle)
    $deltaX = $moved[0] - $origin[0]
    $deltaY = $moved[1] - $origin[1]
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'window' -and $s.hint.x -eq ($anchorX + $deltaX) -and $s.hint.y -eq ($anchorY + $deltaY) } 'Target did not follow the owned window movement'
    $movement.Stop()
    Assert-That ($movement.ElapsedMilliseconds -lt 500) ('Owned window tracking took ' + $movement.ElapsedMilliseconds + 'ms; expected under 500ms locally.')
    $movedFrame = [ScreenCueTrackingWindow]::PhysicalBounds($window)
    Assert-That ($movedFrame[0] - $initialFrame[0] -eq $deltaX -and $movedFrame[1] - $initialFrame[1] -eq $deltaY) 'Native overlay did not translate with the target window.'
    Assert-That ([Math]::Abs($tracked.arrow_tip[0] - $initialTip[0] - $deltaX) -le 1 -and [Math]::Abs($tracked.arrow_tip[1] - $initialTip[1] - $deltaY) -le 1) 'Arrow tip did not translate with the target.'

    $trackingWindow.Minimize()
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'minimized' } 'Minimized window was not reported'
    Assert-RenderedOnDisplay $tracked $displays $window
    $trackingWindow.Restore()
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'window' -and [ScreenCueRustNativeTest]::IsAbove($window, $trackingWindow.Handle) } 'Restored window did not recover tracking above the target window'
    Assert-That ($tracked.hint.x -eq ($anchorX + $deltaX) -and $tracked.hint.y -eq ($anchorY + $deltaY)) 'Restoring changed the target anchor.'

    $trackingWindow.Resize(($origin[2] - $origin[0] + 40), ($origin[3] - $origin[1] + 30))
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'needs_refresh' } 'Resized window did not request a fresh observation'
    Assert-RenderedOnDisplay $tracked $displays $window
    $trackingWindow.Resize(($origin[2] - $origin[0]), ($origin[3] - $origin[1]))
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'window' } 'Original window size did not recover tracking'

    $outsideX = [int](($displays | ForEach-Object { [int]$_.x + [int]$_.width } | Measure-Object -Maximum).Maximum) + 300
    $trackingWindow.Move($outsideX, $origin[1])
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'offscreen' } 'Offscreen target was not reported'
    Assert-RenderedOnDisplay $tracked $displays $window
    $trackingWindow.Move($origin[0], $origin[1])
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'window' -and $s.hint.x -eq $anchorX -and $s.hint.y -eq $anchorY } 'Returning target did not recover its original anchor'
    Assert-That ([ScreenCueRustNativeTest]::GetForegroundWindow() -ne $trackingWindow.Handle -and [ScreenCueRustNativeTest]::GetForegroundWindow() -ne $window) 'Tracking activated an owned window.'
    $trackingWindow.Dispose()
    $trackingWindow = $null
    $tracked = Wait-Tracking { param($s) $s.tracking.state -eq 'closed' } 'Closed target window was not reported'
    Assert-RenderedOnDisplay $tracked $displays $window
    Write-Host ('Owned window move observed in ' + $movement.ElapsedMilliseconds + 'ms; minimize/offscreen/restore/close tracking passed.')

    [void](Send-Command @{ command = 'clear' })
    $status = Send-Command @{ command = 'status' }
    Assert-That (-not $status.visible) 'clear retained logical visibility.'
    Assert-That ($status.shortcut_active -eq $false) 'clear retained the global shortcut registration.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'clear retained native visibility.'
    Assert-ShortcutOwnership $x $y $true
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = '#AC4DFF'; duration_ms = 0 })
    $status = Send-Command @{ command = 'status' }
    Assert-That ($status.shortcut_active -eq $true) 'Shortcut was not registered again after clear.'
    Assert-That ($null -ne $status.hide_shortcut_id) 'status omitted the registered shortcut ID.'
    Assert-ShortcutOwnership $x $y $false
    $hotkeyWindow = [ScreenCueRustNativeTest]::FindOwnedHotkeyWindow([uint32]$script:overlay.Id)
    Assert-That ($hotkeyWindow -ne [IntPtr]::Zero) 'The owned global-hotkey message window was not found.'
    # Deliver only a message to our own process. No global keypress is generated.
    # Ctrl+Alt+X: modifiers 0x0003, VK_X 0x58 in the high word.
    Assert-That ([ScreenCueRustNativeTest]::PostMessage($hotkeyWindow, 0x0312, [IntPtr]([int64]$status.hide_shortcut_id), [IntPtr]0x00580003)) 'Could not post the owned WM_HOTKEY test message.'
    Start-Sleep -Milliseconds 150
    $status = Send-Command @{ command = 'status' }
    Assert-That (-not $status.visible -and $status.shortcut_active -eq $false) 'Native shortcut message did not hide the cue and release registration.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Native shortcut message left the overlay visible.'
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = '#AC4DFF'; duration_ms = 500 })
    Start-Sleep -Milliseconds 900
    Assert-That (-not (Send-Command @{ command = 'status' }).visible) 'Expiry retained logical visibility.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Expiry retained native visibility.'

    foreach ($invalid in @(
        @{ command = 'show'; x = 1.5; y = $y },
        @{ command = 'show'; x = $x; y = $y; duration_ms = 499 },
        @{ command = 'show'; x = $x; y = $y; color = '#GG0000' },
        @{ command = 'show'; x = $x; y = $y; arrow = 'false' },
        @{ command = 'show'; x = $x; y = $y; animate = 'false' },
        @{ command = 'show'; x = $x; y = $y; follow_window = 'true' },
        @{ command = 'show'; x = $x; y = $y; arrow_scale = 0.74 },
        @{ command = 'show'; x = $x; y = $y; arrow_scale = 2.51 },
        @{ command = 'show'; x = $x; y = $y; arrow_scale = '1.6' },
        @{ command = 'show'; x = -2147483648; y = -2147483648 },
        @{ command = 'show'; x = $x; y = $y; shape = 'polygon'; points = @() }
    )) { [void](Send-Command $invalid $false) }
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'Rejected input displayed a hint.'

    # EOF, rather than a separate CLI quit command, ends this overlay session.
    [void](Send-Command @{ command = 'show'; x = $x; y = $y; text = ''; color = '#AC4DFF'; duration_ms = 0 })
    $script:inputWriter.Dispose()
    $script:inputWriter = $null
    $script:overlay.StandardInput.Close()
    Assert-That ($script:overlay.WaitForExit(5000)) 'Process did not exit after stdin EOF.'
    Assert-That ($script:overlay.ExitCode -eq 0) 'Process returned a failure on normal EOF.'
    Assert-That (-not [ScreenCueRustNativeTest]::IsWindowVisible($window)) 'EOF left the native overlay visible.'
    $remaining = $script:overlay.StandardOutput.ReadToEnd()
    Assert-That ([string]::IsNullOrWhiteSpace($remaining)) 'Unexpected stdout after final command.'
    $errorText = $errors.GetAwaiter().GetResult()
    Assert-That ([string]::IsNullOrWhiteSpace($errorText)) ('Unexpected diagnostics: ' + $errorText)
    Write-Host 'PASS: owned JSON lifecycle, native visibility/styles, focus, persistent default, window tracking and pinning, Unicode, shapes, palette, validation, failed-replacement preservation, hotkey delivery/release, expiry and EOF.'
    Write-Host 'Observed process metrics; these are not performance guarantees:'
    $samples | ConvertTo-Json -Depth 3
}
finally {
    if ($null -ne $trackingWindow) { try { $trackingWindow.Dispose() } catch { } }
    if ($null -ne $script:inputWriter) { try { $script:inputWriter.Dispose() } catch { } }
    if ($started -and -not $script:overlay.HasExited) {
        try { $script:overlay.StandardInput.Close() } catch { }
        if (-not $script:overlay.WaitForExit(3000)) { $script:overlay.Kill(); [void]$script:overlay.WaitForExit(3000) }
    }
    $script:overlay.Dispose()
}
