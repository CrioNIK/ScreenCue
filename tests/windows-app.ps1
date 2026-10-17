param([string]$Executable = (Join-Path $PSScriptRoot '..\target\release\screencue.exe'))
# Owns every tested window and preference file. Never starts audio or a model,
# reads screen pixels, switches another app, or sends input to another process.
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path -LiteralPath $Executable).Path
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class ScreenCueAppTest {
    private delegate bool EnumCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder title, int maximum);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetWindowDisplayAffinity(IntPtr window, out uint affinity);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wParam, IntPtr lParam, uint flags, uint timeout, out IntPtr result);
    public static IntPtr Find(uint process, string expected) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != process) return true;
            var title = new StringBuilder(256); GetWindowText(window, title, title.Capacity);
            if (title.ToString() != expected) return true;
            found = window; return false;
        }, IntPtr.Zero);
        return found;
    }
    public static bool HasIcon(IntPtr window) {
        IntPtr icon;
        for (int kind=0; kind<3; kind++) {
            if (SendMessageTimeout(window, 0x7F, new IntPtr(kind), IntPtr.Zero, 2, 1000, out icon) != IntPtr.Zero && icon != IntPtr.Zero) return true;
        }
        return false;
    }
    public static bool SetMinimized(IntPtr window, bool minimized) {
        IntPtr result;
        // Send the same native system command as the window's minimize/restore
        // controls, and wait for the owned window procedure to finish handling it.
        return SendMessageTimeout(window, 0x112, new IntPtr(minimized ? 0xF020 : 0xF120), IntPtr.Zero, 2, 3000, out result) != IntPtr.Zero;
    }
    public static uint Owner(IntPtr window) { uint owner; GetWindowThreadProcessId(window, out owner); return owner; }
    public static string Describe(IntPtr window) {
        uint owner; GetWindowThreadProcessId(window, out owner);
        return "HWND=0x" + window.ToInt64().ToString("X") + ", PID=" + owner;
    }
    public sealed class ForegroundWatch : IDisposable {
        private delegate void WinEventCallback(IntPtr hook, uint kind, IntPtr window, int objectId, int childId, uint eventThread, uint time);
        [StructLayout(LayoutKind.Sequential)] private struct Point { public int x, y; }
        [StructLayout(LayoutKind.Sequential)] private struct Message {
            public IntPtr window; public uint message; public UIntPtr wParam; public IntPtr lParam;
            public uint time; public Point point; public uint privateData;
        }
        [DllImport("user32.dll")] private static extern IntPtr SetWinEventHook(uint first, uint last, IntPtr module, WinEventCallback callback, uint process, uint thread, uint flags);
        [DllImport("user32.dll")] private static extern bool UnhookWinEvent(IntPtr hook);
        [DllImport("user32.dll")] private static extern int GetMessage(out Message message, IntPtr window, uint first, uint last);
        [DllImport("user32.dll")] private static extern bool PeekMessage(out Message message, IntPtr window, uint first, uint last, uint remove);
        [DllImport("user32.dll")] private static extern bool TranslateMessage(ref Message message);
        [DllImport("user32.dll")] private static extern IntPtr DispatchMessage(ref Message message);
        [DllImport("user32.dll")] private static extern bool PostThreadMessage(uint thread, uint message, UIntPtr wParam, IntPtr lParam);
        [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
        private readonly uint process;
        private readonly Thread worker;
        private readonly ManualResetEventSlim ready = new ManualResetEventSlim(false);
        private readonly WinEventCallback callback;
        private uint threadId;
        private IntPtr hook, activatedWindow;
        private int activations, disposed;
        public int Activations { get { return Interlocked.CompareExchange(ref activations, 0, 0); } }
        public string Diagnostic { get { return "foreground events=" + Activations + ", expected PID=" + process + ", last " + Describe(Interlocked.CompareExchange(ref activatedWindow, IntPtr.Zero, IntPtr.Zero)); } }
        public ForegroundWatch(uint process) {
            this.process = process;
            callback = delegate(IntPtr handle, uint kind, IntPtr window, int objectId, int childId, uint eventThread, uint time) {
                Interlocked.Exchange(ref activatedWindow, window);
                Interlocked.Increment(ref activations);
            };
            worker = new Thread(Pump); worker.IsBackground = true; worker.Start();
            if (!ready.Wait(3000) || hook == IntPtr.Zero) {
                Dispose(); throw new InvalidOperationException("Cannot install the owned-process foreground monitor.");
            }
        }
        private void Pump() {
            threadId = GetCurrentThreadId();
            Message message; PeekMessage(out message, IntPtr.Zero, 0, 0, 0);
            // EVENT_SYSTEM_FOREGROUND, filtered to the owned JSON process.
            // Out-of-context callbacks run on this message-pumping thread.
            hook = SetWinEventHook(3, 3, IntPtr.Zero, callback, process, 0, 0);
            ready.Set();
            if (hook == IntPtr.Zero) return;
            try {
                while (GetMessage(out message, IntPtr.Zero, 0, 0) > 0) {
                    TranslateMessage(ref message); DispatchMessage(ref message);
                }
            } finally { UnhookWinEvent(hook); }
        }
        public void Dispose() {
            if (Interlocked.Exchange(ref disposed, 1) != 0) return;
            if (threadId != 0) PostThreadMessage(threadId, 0x12, UIntPtr.Zero, IntPtr.Zero);
            if (!worker.Join(3000)) throw new InvalidOperationException("Foreground monitor did not stop.");
            ready.Dispose();
        }
    }
}
'@
function Assert-That([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Start-Owned([string]$Mode, [string]$Settings) {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = $Mode + ' --settings-file "' + $Settings + '"'
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    [void]$process.Start()
    return $process
}
function Wait-OwnedWindow($Process, [string]$Title) {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        if ($Process.HasExited) { throw ('Owned app exited: ' + $Process.StandardError.ReadToEnd()) }
        $window = [ScreenCueAppTest]::Find([uint32]$Process.Id, $Title)
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 25
    } while ($timer.ElapsedMilliseconds -lt 10000)
    throw "Owned window did not appear: $Title"
}
function Check-Affinity([IntPtr]$Window, [bool]$Enabled) {
    [uint32]$value = 0
    Assert-That ([ScreenCueAppTest]::GetWindowDisplayAffinity($Window, [ref]$value)) 'Native affinity readback failed.'
    $expected = if ($Enabled) { 0x11 } else { 0 }
    Assert-That ($value -eq $expected) "Affinity mismatch: expected $expected, observed $value."
}
function Wait-OwnedVisible($Process, [IntPtr]$Window) {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        if ($Process.HasExited) { throw ('Owned settings app exited before initialization: ' + $Process.StandardError.ReadToEnd()) }
        if ([ScreenCueAppTest]::IsWindowVisible($Window)) { return }
        Start-Sleep -Milliseconds 25
    } while ($timer.ElapsedMilliseconds -lt 10000)
    throw 'Owned settings window was not displayed before the initialization deadline.'
}
function Check-SettingsPolicy([IntPtr]$Window, [bool]$Enabled) {
    Assert-That (([ScreenCueAppTest]::GetWindowLong($Window, -20) -band 0x80000) -ne 0) 'Settings window is not layered for verified capture policy.'
    Check-Affinity $Window $Enabled
}
function Check-NoOwnedActivation($Watch, [uint32]$ProcessId, [IntPtr]$Overlay) {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        $actual = [ScreenCueAppTest]::GetForegroundWindow()
        Assert-That ($Watch.Activations -eq 0 -and [ScreenCueAppTest]::Owner($actual) -ne $ProcessId) ('The text prompt activated its process. ' + $Watch.Diagnostic + '; current ' + [ScreenCueAppTest]::Describe($actual) + '; overlay ' + [ScreenCueAppTest]::Describe($Overlay))
        Start-Sleep -Milliseconds 25
    } while ($timer.ElapsedMilliseconds -lt 250)
}
function Send-Owned($Process, [hashtable]$Request) {
    $Process.StandardInput.WriteLine(($Request | ConvertTo-Json -Depth 10 -Compress))
    $Process.StandardInput.Flush()
    $line = $Process.StandardOutput.ReadLineAsync()
    if (-not $line.Wait(10000)) { throw 'Owned JSON response timed out.' }
    $text = $line.GetAwaiter().GetResult()
    if (-not $text) { throw ('Owned JSON exited: ' + $Process.StandardError.ReadToEnd()) }
    $reply = $text | ConvertFrom-Json
    Assert-That ($reply.ok -eq $true) $text
    return $reply
}
function Stop-Owned($Process) {
    if ($null -eq $Process) { return }
    if (-not $Process.HasExited) {
        $Process.StandardInput.Close()
        if (-not $Process.WaitForExit(2000)) { $Process.Kill(); [void]$Process.WaitForExit(2000) }
    }
    $Process.Dispose()
}
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('ScreenCue-app-test-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $temporary)
$settingsFile = Join-Path $temporary 'settings.json'
$app = $null
$json = $null
$focusWatch = $null
try {
    foreach ($enabled in @($false, $true)) {
        $preferences = @{ hide_from_capture = $enabled; mode = 'conversation'; animate = $false } | ConvertTo-Json
        [System.IO.File]::WriteAllText($settingsFile, $preferences, (New-Object System.Text.UTF8Encoding($false)))
        $app = Start-Owned '--app' $settingsFile
        $window = Wait-OwnedWindow $app 'ScreenCue'
        Wait-OwnedVisible $app $window
        Assert-That ([ScreenCueAppTest]::IsWindowVisible($window)) 'Settings window was not displayed.'
        Assert-That ([ScreenCueAppTest]::HasIcon($window)) 'Settings window has no app icon.'
        Check-SettingsPolicy $window $enabled
        foreach ($minimized in @($true, $false)) {
            Assert-That ([ScreenCueAppTest]::SetMinimized($window, $minimized)) 'Owned settings minimize/restore command timed out.'
            Assert-That ([ScreenCueAppTest]::IsIconic($window) -eq $minimized) 'Owned settings minimize/restore state did not change.'
            Check-SettingsPolicy $window $enabled
        }
        Assert-That ([ScreenCueAppTest]::IsWindowVisible($window)) 'Settings window was not visible after restoring.'
        $json = Start-Owned '--json' $settingsFile
        $status = Send-Owned $json @{ command = 'status' }
        Assert-That ($status.conversation_state -eq 'Stopped') 'Opening the app started conversation capture.'
        Assert-That ($status.capture_privacy.requested -eq $enabled) 'Stored privacy preference did not load.'
        # Observe only our JSON process, including transient activations. No
        # foreground switch is requested and unrelated OS transitions are ignored.
        $focusWatch = New-Object 'ScreenCueAppTest+ForegroundWatch' ([uint32]$json.Id)
        $reply = Send-Owned $json @{ command = 'prompt'; title = 'Owned privacy test'; text = 'A standalone conversation note.' }
        Assert-That ($reply.capture_privacy.verified -and $reply.capture_privacy.applied -eq $enabled) ('Prompt policy was not verified: ' + ($reply | ConvertTo-Json -Compress -Depth 10))
        Assert-That (-not $reply.capture_privacy.capture_guaranteed) 'Prompt claimed universal capture protection.'
        $overlay = Wait-OwnedWindow $json 'ScreenCue Overlay'
        Check-Affinity $overlay $enabled
        Assert-That (([ScreenCueAppTest]::GetWindowLong($overlay, -20) -band 0x08000000) -ne 0) 'The text overlay lacks WS_EX_NOACTIVATE.'
        $status = Send-Owned $json @{ command = 'status' }
        Assert-That (-not $status.hint.arrow -and -not $status.hint.target) 'A conversation prompt unexpectedly included an arrow or target.'
        Check-NoOwnedActivation $focusWatch ([uint32]$json.Id) $overlay
        $focusWatch.Dispose()
        Assert-That ($focusWatch.Activations -eq 0) ('The text prompt activated before monitor shutdown. ' + $focusWatch.Diagnostic)
        $focusWatch = $null
        [void](Send-Owned $json @{ command = 'clear' })
        $json.StandardInput.Close()
        Assert-That ($json.WaitForExit(5000)) 'JSON EOF did not stop the owned overlay.'
        Assert-That ($json.ExitCode -eq 0) 'JSON overlay exited with an error.'
        Stop-Owned $json
        $json = $null
        Assert-That ([ScreenCueAppTest]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)) 'Could not close the owned settings window.'
        Assert-That ($app.WaitForExit(5000)) 'Settings close did not stop its process.'
        Assert-That ($app.ExitCode -eq 0) ('Settings exited with an error: ' + $app.StandardError.ReadToEnd())
        Stop-Owned $app
        $app = $null
    }
    Write-Output 'PASS: native settings visibility/icon/minimize/restore/close, privacy off/on on both owned windows, standalone prompt, overlay does not activate, idle audio and clean EOF.'
}
finally {
    if ($null -ne $focusWatch) { $focusWatch.Dispose() }
    Stop-Owned $app
    Stop-Owned $json
    foreach ($ownedFile in @($settingsFile, [System.IO.Path]::ChangeExtension($settingsFile, 'dismiss'))) {
        if (Test-Path -LiteralPath $ownedFile) { Remove-Item -LiteralPath $ownedFile }
    }
    # Remove only known test files; an unexpected file keeps the directory intact.
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
}
