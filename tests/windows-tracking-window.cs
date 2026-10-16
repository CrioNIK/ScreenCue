// Test-only raw Win32 window. It never activates or sends input to other apps.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;

public sealed class ScreenCueTrackingWindow : IDisposable
{
    private delegate IntPtr WindowProcedure(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WindowClass
    {
        public uint Style;
        public IntPtr Procedure;
        public int ClassExtra, WindowExtra;
        public IntPtr Instance, Icon, Cursor, Background;
        public string Menu, ClassName;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Message
    {
        public IntPtr Window;
        public uint Id;
        public UIntPtr WParam;
        public IntPtr LParam;
        public uint Time;
        public int X, Y;
        public uint Private;
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr GetModuleHandle(string name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern ushort RegisterClass(ref WindowClass value);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern bool UnregisterClass(string name, IntPtr instance);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern IntPtr CreateWindowEx(uint extended, string className, string title, uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr DefWindowProc(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] private static extern int GetMessage(out Message message, IntPtr window, uint first, uint last);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref Message message);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr DispatchMessage(ref Message message);
    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] private static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern void PostQuitMessage(int code);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    private readonly Thread thread;
    private readonly ManualResetEvent ready = new ManualResetEvent(false);
    private readonly Queue<Action> actions = new Queue<Action>();
    private readonly WindowProcedure procedure;
    private Exception failure;
    public IntPtr Handle { get; private set; }

    public ScreenCueTrackingWindow(int x, int y, int width, int height)
    {
        procedure = ProcessMessage;
        thread = new Thread(delegate()
        {
            var previousDpi = SetThreadDpiAwarenessContext(new IntPtr(-4));
            var instance = GetModuleHandle(null);
            var name = "ScreenCueOwnedTrackingTest_" + Guid.NewGuid().ToString("N");
            try
            {
                var windowClass = new WindowClass {
                    Procedure = Marshal.GetFunctionPointerForDelegate(procedure),
                    Instance = instance, Background = new IntPtr(6), ClassName = name
                };
                if (RegisterClass(ref windowClass) == 0) throw new Win32Exception();
                // NOACTIVATE | TOOLWINDOW | TOPMOST; ordinary resizable window.
                Handle = CreateWindowEx(0x08000088, name, "ScreenCue owned tracking test", 0x00CF0000,
                    x, y, width, height, IntPtr.Zero, IntPtr.Zero, instance, IntPtr.Zero);
                if (Handle == IntPtr.Zero) throw new Win32Exception();
                if (!SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0053)) throw new Win32Exception();
                ready.Set();
                Message message;
                int result;
                while ((result = GetMessage(out message, IntPtr.Zero, 0, 0)) > 0)
                {
                    TranslateMessage(ref message);
                    DispatchMessage(ref message);
                }
                if (result < 0) throw new Win32Exception();
            }
            catch (Exception error) { failure = error; }
            finally
            {
                if (Handle != IntPtr.Zero) DestroyWindow(Handle);
                UnregisterClass(name, instance);
                ready.Set();
                if (previousDpi != IntPtr.Zero) SetThreadDpiAwarenessContext(previousDpi);
            }
        });
        thread.IsBackground = true;
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        if (!ready.WaitOne(5000)) throw new TimeoutException("Owned tracking window did not start.");
        if (failure != null) throw new InvalidOperationException("Owned tracking window failed.", failure);
    }

    private IntPtr ProcessMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam)
    {
        if (message == 0x0021) return new IntPtr(3); // MOUSEACTIVATE -> NOACTIVATE
        if (message == 0x0002) { PostQuitMessage(0); return IntPtr.Zero; }
        if (message == 0x8001)
        {
            while (true)
            {
                Action action;
                lock (actions) { if (actions.Count == 0) break; action = actions.Dequeue(); }
                action();
            }
            return IntPtr.Zero;
        }
        return DefWindowProc(window, message, wParam, lParam);
    }

    private void Invoke(Action action)
    {
        using (var completed = new ManualResetEvent(false))
        {
            Exception error = null;
            lock (actions) actions.Enqueue(delegate
            {
                try { action(); } catch (Exception caught) { error = caught; }
                finally { completed.Set(); }
            });
            if (!PostMessage(Handle, 0x8001, IntPtr.Zero, IntPtr.Zero)) throw new Win32Exception();
            if (!completed.WaitOne(5000)) throw new TimeoutException("Owned native window did not process its command.");
            if (error != null) throw new InvalidOperationException("Owned window command failed.", error);
        }
    }

    public static int[] PhysicalBounds(IntPtr window)
    {
        var previousDpi = SetThreadDpiAwarenessContext(new IntPtr(-4));
        try
        {
            Rect bounds;
            if (!GetWindowRect(window, out bounds)) throw new Win32Exception();
            return new[] { bounds.Left, bounds.Top, bounds.Right, bounds.Bottom };
        }
        finally { if (previousDpi != IntPtr.Zero) SetThreadDpiAwarenessContext(previousDpi); }
    }

    public void Move(int x, int y)
    {
        Invoke(delegate {
            if (!SetWindowPos(Handle, IntPtr.Zero, x, y, 0, 0, 0x0015)) throw new Win32Exception();
        });
    }
    public void Resize(int width, int height)
    {
        Invoke(delegate {
            if (!SetWindowPos(Handle, IntPtr.Zero, 0, 0, width, height, 0x0016)) throw new Win32Exception();
        });
    }
    public void Minimize() { Invoke(delegate { ShowWindow(Handle, 7); }); }
    public void Restore()
    {
        Invoke(delegate {
            ShowWindow(Handle, 4);
            if (!SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0053)) throw new Win32Exception();
        });
    }
    public void Dispose()
    {
        if (thread.IsAlive)
        {
            Invoke(delegate { DestroyWindow(Handle); });
            if (!thread.Join(3000)) throw new TimeoutException("Owned tracking window did not close.");
        }
        ready.Dispose();
    }
}
