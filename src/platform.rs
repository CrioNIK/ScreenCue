//! Native, on-demand presentation of premultiplied RGBA pixels.
//!
//! No graphics context or redraw loop is owned here. The caller keeps the winit
//! window alive, invokes these methods on its event-loop thread, and hides the
//! previous hint before asking for an optional small background sample.

use winit::window::Window;

/// This verifies an operating-system window policy, not the output of a sharing
/// application. A capture program or capture device can use a different path.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CapturePrivacyStatus {
    pub requested: bool,
    pub supported: bool,
    pub applied: bool,
    pub verified: bool,
    pub backend: &'static str,
    pub capture_guaranteed: bool,
    pub error: Option<String>,
    pub note: &'static str,
}

/// Capability only: no policy is applied until `set_capture_exclusion` is called
/// for a particular top-level window. New windows need their own policy.
pub fn capture_privacy_capability() -> CapturePrivacyStatus {
    #[cfg(target_os = "windows")]
    {
        imp::capture_privacy_capability()
    }
    #[cfg(not(target_os = "windows"))]
    {
        unsupported_capture_privacy(false)
    }
}

/// Call after preparing the native presenter and before making the window
/// visible. Changing this policy does not activate, map, or hide the window.
pub fn set_capture_exclusion(window: &Window, requested: bool) -> CapturePrivacyStatus {
    #[cfg(target_os = "windows")]
    {
        imp::set_capture_exclusion(window, requested)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        unsupported_capture_privacy(requested)
    }
}

#[cfg(not(target_os = "windows"))]
fn unsupported_capture_privacy(requested: bool) -> CapturePrivacyStatus {
    let note = if cfg!(target_os = "macos") {
        "macOS has no supported window opt-out from other applications' ScreenCaptureKit capture. Share a selected application window and verify the receiver's view."
    } else {
        "This display backend has no supported per-window capture exclusion. Share a selected application window and verify the receiver's view."
    };
    CapturePrivacyStatus {
        requested,
        supported: false,
        applied: false,
        verified: false,
        backend: "unsupported",
        capture_guaranteed: false,
        error: requested.then(|| "Capture exclusion is unavailable on this platform".to_owned()),
        note,
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod capture_privacy_tests {
    #[test]
    fn unsupported_platform_never_claims_capture_exclusion() {
        for requested in [false, true] {
            let status = super::unsupported_capture_privacy(requested);
            assert_eq!(status.requested, requested);
            assert!(!status.supported);
            assert!(!status.applied);
            assert!(!status.verified);
            assert!(!status.capture_guaranteed);
            assert_eq!(status.error.is_some(), requested);
        }
    }
}

/// Mapping a native window is not evidence that a game's presentation path
/// composites it. Keep that distinction visible to clients on every platform.
pub fn presentation_status(window_mapped: bool) -> serde_json::Value {
    let backend = if cfg!(target_os = "windows") {
        "win32_layered"
    } else if cfg!(target_os = "macos") {
        "appkit_layer"
    } else if cfg!(target_os = "linux") {
        "x11_argb"
    } else {
        "unsupported"
    };
    serde_json::json!({
        "backend": backend,
        "window_mapped": window_mapped,
        "fullscreen_visibility": "unverified",
        "exclusive_fullscreen_guaranteed": false
    })
}

/// A native top-level window snapshot in the same physical desktop coordinates
/// as the overlay API. `scale` lets callers retain a logical offset across DPI changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub visible: bool,
    pub destroyed: bool,
    pub scale: f64,
}

impl AnchorState {
    fn destroyed() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            visible: false,
            destroyed: true,
            scale: 1.0,
        }
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        x as i64 >= self.x as i64
            && y as i64 >= self.y as i64
            && (x as i64) < self.x as i64 + self.width as i64
            && (y as i64) < self.y as i64 + self.height as i64
    }
}

/// Tracks one captured native window ID, never whichever window later occupies
/// the point. This follows window movement, not controls, DOM or accessibility.
pub struct WindowAnchor {
    inner: imp::WindowAnchor,
}

impl WindowAnchor {
    pub fn capture(x: i32, y: i32, overlay: &Window) -> Result<Option<Self>, String> {
        Ok(imp::WindowAnchor::capture(x, y, overlay)?.map(|inner| Self { inner }))
    }

    pub fn poll(&self) -> Result<AnchorState, String> {
        self.inner.poll()
    }
}

pub fn set_bounds(
    window: &Window,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<(), String> {
    frame_len(width, height)?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("Invalid target display scale".into());
    }
    #[cfg(target_os = "macos")]
    {
        use winit::dpi::{LogicalPosition, LogicalSize};
        // Physical positions in winit are converted through the window's old
        // screen scale. Use the requested monitor's scale when crossing displays.
        let scale = scale as f64;
        let _ = window.request_inner_size(LogicalSize::new(
            width as f64 / scale,
            height as f64 / scale,
        ));
        // Position last so AppKit's resize anchoring cannot shift the requested top edge.
        window.set_outer_position(LogicalPosition::new(x as f64 / scale, y as f64 / scale));
    }
    #[cfg(not(target_os = "macos"))]
    {
        use winit::dpi::{PhysicalPosition, PhysicalSize};
        window.set_outer_position(PhysicalPosition::new(x, y));
        let _ = window.request_inner_size(PhysicalSize::new(width, height));
    }
    Ok(())
}

pub fn set_visible(window: &Window, visible: bool) -> Result<(), String> {
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    {
        imp::set_visible(window, visible)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        window.set_visible(visible);
        Ok(())
    }
}

pub struct Presenter {
    inner: imp::Presenter,
}

impl Presenter {
    pub fn new(window: &Window) -> Result<Self, String> {
        Ok(Self {
            inner: imp::Presenter::new(window)?,
        })
    }

    pub fn present(
        &mut self,
        window: &Window,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let required = frame_len(width, height)?;
        if rgba.len() != required {
            return Err("Premultiplied RGBA buffer does not match its dimensions".into());
        }
        self.inner.present(window, rgba, width, height)
    }
}

/// Read one bounded, transient pixel patch. Pixels are never saved or transmitted.
/// Unsupported capture paths return an error; callers can disclose a color fallback.
pub fn sample(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<[u8; 3]>, String> {
    if width == 0 || height == 0 || width > 128 || height > 128 {
        return Err("Background sampling requires dimensions between 1 and 128 pixels".into());
    }
    imp::sample(x, y, width, height)
}

fn frame_len(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > 32767 || height > 32767 {
        return Err("Overlay dimensions must be between 1 and 32767 pixels".into());
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| "Overlay dimensions overflow the buffer size".to_string())?;
    if bytes > 64 * 1024 * 1024 {
        return Err("Overlay raster exceeds the 64 MiB limit; use smaller hint bounds".into());
    }
    Ok(bytes)
}

#[cfg(target_os = "windows")]
mod imp {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError, HWND, POINT, RECT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CAPTUREBLT,
        DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, SRCCOPY,
    };
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetDesktopWindow, GetPropW, GetShellWindow, GetTopWindow,
        GetWindow, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, RemovePropW, SetPropW,
        SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, UpdateLayeredWindow,
        EVENT_OBJECT_REORDER, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GWL_EXSTYLE,
        GW_HWNDNEXT, GW_HWNDPREV, HWND_TOPMOST, OBJID_CLIENT, OBJID_WINDOW, SWP_HIDEWINDOW,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, ULW_ALPHA, WDA_EXCLUDEFROMCAPTURE,
        WDA_NONE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_APPWINDOW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;

    const OVERLAY_PROPERTY: windows_sys::core::PCWSTR =
        windows_sys::core::w!("ScreenCue.Overlay.v1");

    #[repr(C)]
    struct RtlOsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut RtlOsVersionInfo) -> i32;
    }

    fn supports_capture_exclusion(major: u32, build: u32) -> bool {
        major > 10 || (major == 10 && build >= 19041)
    }

    pub(super) fn capture_privacy_capability() -> super::CapturePrivacyStatus {
        let mut status = super::CapturePrivacyStatus {
            requested: false,
            supported: false,
            applied: false,
            verified: false,
            backend: "win32_display_affinity",
            capture_guaranteed: false,
            error: None,
            note: "Exclusion applies to compatible Windows capture APIs. The sharing application's output has not been verified; check the receiver's view before use.",
        };
        // RtlGetVersion avoids the compatibility-manifest version lie. Older
        // Windows maps 0x11 to WDA_MONITOR, which leaves a blank captured window
        // rather than omitting it; never advertise that as capture exclusion.
        let mut version: RtlOsVersionInfo = unsafe { zeroed() };
        version.size = size_of::<RtlOsVersionInfo>() as u32;
        let result = unsafe { RtlGetVersion(&mut version) };
        if result < 0 {
            status.error = Some(format!(
                "RtlGetVersion failed: NTSTATUS 0x{:08X}",
                result as u32
            ));
        } else if supports_capture_exclusion(version.major, version.build) {
            status.supported = true;
        } else {
            status.note = "Capture exclusion requires Windows 10 version 2004 (build 19041) or newer. No blank-window fallback is applied.";
        }
        status
    }

    pub(super) fn set_capture_exclusion(
        window: &Window,
        requested: bool,
    ) -> super::CapturePrivacyStatus {
        let status = capture_privacy_capability();
        match hwnd(window) {
            Ok(handle) => apply_capture_exclusion(handle, requested, status),
            Err(error) => super::CapturePrivacyStatus {
                requested,
                error: Some(error),
                ..status
            },
        }
    }

    fn apply_capture_exclusion(
        window: HWND,
        requested: bool,
        mut status: super::CapturePrivacyStatus,
    ) -> super::CapturePrivacyStatus {
        status.requested = requested;
        if requested && !status.supported {
            if status.error.is_none() {
                status.error =
                    Some("Capture exclusion requires Windows build 19041 or newer".into());
            }
            return status;
        }
        let wanted = if requested {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        let set_error = if unsafe { SetWindowDisplayAffinity(window, wanted) } == 0 {
            Some(failure("SetWindowDisplayAffinity"))
        } else {
            None
        };
        let mut observed = WDA_NONE;
        // A successful GetWindowDisplayAffinity also verifies DWM composition.
        // DwmIsCompositionEnabled can report false for a composed desktop when
        // the executable lacks a Windows-8 compatibility manifest.
        if unsafe { GetWindowDisplayAffinity(window, &mut observed) } == 0 {
            status.error = Some(set_error.unwrap_or_else(|| failure("GetWindowDisplayAffinity")));
            return status;
        }
        status.applied = status.supported && observed == WDA_EXCLUDEFROMCAPTURE;
        status.verified = set_error.is_none() && observed == wanted;
        status.error = set_error.or_else(|| {
            (observed != wanted).then(|| {
                format!("Window display affinity mismatch: requested 0x{wanted:08X}, observed 0x{observed:08X}")
            })
        });
        status
    }

    #[cfg(test)]
    mod capture_tests {
        use super::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetForegroundWindow, WS_POPUP,
        };

        #[test]
        fn exclusion_gate_does_not_accept_legacy_blank_window_behavior() {
            assert!(!supports_capture_exclusion(6, 99999));
            assert!(!supports_capture_exclusion(10, 19040));
            assert!(supports_capture_exclusion(10, 19041));
            assert!(supports_capture_exclusion(10, 26100));
            assert!(supports_capture_exclusion(11, 1));
        }

        #[test]
        fn unsupported_exclusion_does_not_claim_applied_or_verified() {
            let mut capability = capture_privacy_capability();
            capability.supported = false;
            capability.error = None;
            let status = apply_capture_exclusion(null_mut(), true, capability);
            assert!(status.requested);
            assert!(!status.applied);
            assert!(!status.verified);
            assert!(!status.capture_guaranteed);
            assert!(status.error.unwrap().contains("19041"));
        }

        #[test]
        fn invalid_window_reports_native_failure_instead_of_enabled() {
            let mut capability = capture_privacy_capability();
            capability.supported = true;
            let status = apply_capture_exclusion(null_mut(), true, capability);
            assert!(!status.applied);
            assert!(!status.verified);
            assert!(status
                .error
                .unwrap()
                .starts_with("SetWindowDisplayAffinity"));
        }

        #[test]
        fn owned_hidden_layered_window_can_enable_verify_and_disable_exclusion() {
            let capability = capture_privacy_capability();
            if !capability.supported {
                eprintln!(
                    "Native capture policy test unavailable: {}",
                    capability.note
                );
                return;
            }
            let foreground = unsafe { GetForegroundWindow() };
            // A hidden test-owned top-level window: no screen pixels are read,
            // no user window is altered, and no keyboard focus is requested.
            let handle = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    windows_sys::core::w!("STATIC"),
                    windows_sys::core::w!("ScreenCue capture policy test"),
                    WS_POPUP,
                    0,
                    0,
                    64,
                    64,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null(),
                )
            };
            assert!(!handle.is_null(), "{}", failure("CreateWindowExW"));
            struct OwnedWindow(HWND);
            impl Drop for OwnedWindow {
                fn drop(&mut self) {
                    unsafe { DestroyWindow(self.0) };
                }
            }
            let owned = OwnedWindow(handle);
            let enabled = apply_capture_exclusion(owned.0, true, capability.clone());
            assert!(enabled.applied && enabled.verified, "{enabled:?}");
            assert!(enabled.error.is_none());
            assert!(!enabled.capture_guaranteed);
            let mut observed = 0;
            assert_ne!(
                unsafe { GetWindowDisplayAffinity(owned.0, &mut observed) },
                0
            );
            assert_eq!(observed, WDA_EXCLUDEFROMCAPTURE);
            assert_eq!(unsafe { IsWindowVisible(owned.0) }, 0);
            let disabled = apply_capture_exclusion(owned.0, false, capability);
            assert!(!disabled.applied && disabled.verified, "{disabled:?}");
            assert!(disabled.error.is_none());
            assert_ne!(
                unsafe { GetWindowDisplayAffinity(owned.0, &mut observed) },
                0
            );
            assert_eq!(observed, WDA_NONE);
            assert_eq!(unsafe { IsWindowVisible(owned.0) }, 0);
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        }
    }

    fn cloaked(window: HWND) -> bool {
        let mut value = 0u32;
        unsafe {
            DwmGetWindowAttribute(window, 14, (&mut value as *mut u32).cast(), 4) >= 0 && value != 0
        }
    }

    pub(super) struct WindowAnchor {
        window: HWND,
        process: u32,
        thread: u32,
    }

    impl WindowAnchor {
        pub(super) fn capture(x: i32, y: i32, overlay: &Window) -> Result<Option<Self>, String> {
            let own = hwnd(overlay)?;
            unsafe {
                let desktop = GetDesktopWindow();
                let shell = GetShellWindow();
                let mut candidate = GetTopWindow(null_mut());
                // Traverse the actual top-level Z order. WindowFromPoint can
                // return a transparent overlay rather than the application below.
                for _ in 0..16384 {
                    if candidate.is_null() {
                        break;
                    }
                    let window = candidate;
                    candidate = GetWindow(window, GW_HWNDNEXT);
                    if window == own
                        || window == desktop
                        || window == shell
                        || IsWindowVisible(window) == 0
                        || IsIconic(window) != 0
                        || cloaked(window)
                        || GetWindowLongPtrW(window, GWL_EXSTYLE) & WS_EX_TRANSPARENT as isize != 0
                    {
                        continue;
                    }
                    let mut name = [0u16; 128];
                    let len = GetClassNameW(window, name.as_mut_ptr(), name.len() as i32);
                    let name = String::from_utf16_lossy(&name[..len.max(0) as usize]);
                    if matches!(
                        name.as_str(),
                        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
                    ) {
                        continue;
                    }
                    let mut process = 0;
                    let thread = GetWindowThreadProcessId(window, &mut process);
                    if thread == 0 {
                        continue;
                    }
                    let anchor = Self {
                        window,
                        process,
                        thread,
                    };
                    if let Ok(state) = anchor.poll() {
                        if state.visible && !state.destroyed && state.contains(x, y) {
                            return Ok(Some(anchor));
                        }
                    }
                }
            }
            Ok(None)
        }

        pub(super) fn poll(&self) -> Result<super::AnchorState, String> {
            unsafe {
                let mut process = 0;
                if IsWindow(self.window) == 0
                    || GetWindowThreadProcessId(self.window, &mut process) != self.thread
                    || process != self.process
                {
                    return Ok(super::AnchorState::destroyed());
                }
                let mut rect: RECT = zeroed();
                if GetWindowRect(self.window, &mut rect) == 0 {
                    if IsWindow(self.window) == 0 {
                        return Ok(super::AnchorState::destroyed());
                    }
                    return Err(failure("GetWindowRect anchor"));
                }
                let dpi = GetDpiForWindow(self.window);
                Ok(super::AnchorState {
                    x: rect.left,
                    y: rect.top,
                    width: (rect.right as i64 - rect.left as i64).max(0) as u32,
                    height: (rect.bottom as i64 - rect.top as i64).max(0) as u32,
                    visible: IsWindowVisible(self.window) != 0
                        && IsIconic(self.window) == 0
                        && !cloaked(self.window),
                    destroyed: false,
                    scale: if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 },
                })
            }
        }
    }

    fn failure(operation: &str) -> String {
        format!("{operation}: {}", std::io::Error::last_os_error())
    }

    fn hwnd(window: &Window) -> Result<HWND, String> {
        match window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw()
        {
            RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
            _ => Err("ScreenCue requires a Win32 window on Windows".into()),
        }
    }

    unsafe fn set_styles(window: HWND, style: isize) -> Result<(), String> {
        SetLastError(0);
        if SetWindowLongPtrW(window, GWL_EXSTYLE, style) == 0 && GetLastError() != 0 {
            return Err(failure("SetWindowLongPtrW"));
        }
        Ok(())
    }

    unsafe fn prepare_layered(window: HWND) -> Result<(), String> {
        let old = GetWindowLongPtrW(window, GWL_EXSTYLE);
        // Reset any SetLayeredWindowAttributes state installed by winit's
        // hit-testing implementation before switching to per-pixel alpha.
        set_styles(window, old & !(WS_EX_LAYERED as isize))?;
        let style = (old & !(WS_EX_APPWINDOW as isize))
            | (WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT) as isize;
        set_styles(window, style)?;
        // A stable native marker works across independent MCP processes. Peers
        // must not repeatedly restack above each other in response to WinEvents.
        if SetPropW(window, OVERLAY_PROPERTY, 1usize as *mut c_void) == 0 {
            return Err(failure("SetPropW overlay marker"));
        }
        Ok(())
    }

    unsafe fn ensure_input_styles(window: HWND) -> Result<(), String> {
        let old = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let wanted = (old & !(WS_EX_APPWINDOW as isize))
            | (WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT) as isize;
        if old != wanted {
            set_styles(window, wanted)?;
        }
        Ok(())
    }

    struct ForegroundGuard {
        hooks: Vec<HWINEVENTHOOK>,
        window: HWND,
    }
    impl Drop for ForegroundGuard {
        fn drop(&mut self) {
            unsafe {
                for &hook in &self.hooks {
                    UnhookWinEvent(hook);
                }
            }
        }
    }
    thread_local! {
        static FOREGROUND_GUARD: RefCell<Option<ForegroundGuard>> = const { RefCell::new(None) };
    }

    unsafe extern "system" fn foreground_changed(
        hook: HWINEVENTHOOK,
        event: u32,
        changed: HWND,
        object: i32,
        child: i32,
        _thread: u32,
        _time: u32,
    ) {
        match event {
            EVENT_SYSTEM_FOREGROUND => {}
            EVENT_OBJECT_SHOW | EVENT_OBJECT_REORDER => {
                // Ignore caret, controls and other accessibility-object noise.
                // Desktop client reordering describes the top-level Z order.
                let desktop_reorder = event == EVENT_OBJECT_REORDER
                    && changed == GetDesktopWindow()
                    && object == OBJID_CLIENT;
                if changed.is_null()
                    || child != 0
                    || (!desktop_reorder
                        && (object != OBJID_WINDOW || GetAncestor(changed, GA_ROOT) != changed))
                {
                    return;
                }
            }
            _ => return,
        }
        // OUTOFCONTEXT callbacks run on the installing event-loop thread. A
        // failed borrow can only mean reentrancy during hook install/removal.
        let _ = FOREGROUND_GUARD.try_with(|slot| {
            let target = slot.try_borrow().ok().and_then(|guard| {
                guard
                    .as_ref()
                    .filter(|guard| guard.hooks.contains(&hook))
                    .map(|guard| guard.window)
            });
            if let Some(window) = target {
                if changed != window && IsWindowVisible(window) != 0 && covered_in_z_order(window) {
                    // Does not activate the cue, create foreground events, or show
                    // a hidden window. Skip-own-process also prevents feedback.
                    SetWindowPos(
                        window,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    );
                }
            }
        });
    }

    unsafe fn covered_in_z_order(window: HWND) -> bool {
        // Hidden/cloaked windows can stay ahead in the native Z order. Raising
        // for those would add needless work and could feed reorder notifications.
        let mut above = GetWindow(window, GW_HWNDPREV);
        for _ in 0..16384 {
            if above.is_null() || above == window {
                return false;
            }
            if GetPropW(above, OVERLAY_PROPERTY).is_null()
                && IsWindowVisible(above) != 0
                && !cloaked(above)
            {
                return true;
            }
            above = GetWindow(above, GW_HWNDPREV);
        }
        false
    }

    fn start_foreground_guard(window: HWND) -> Result<(), String> {
        FOREGROUND_GUARD.with(|slot| {
            let mut guard = slot.borrow_mut();
            if guard.as_ref().is_some_and(|guard| guard.window == window) {
                return Ok(());
            }
            *guard = None;
            let mut installed = ForegroundGuard {
                hooks: Vec::with_capacity(2),
                window,
            };
            for (first, last) in [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
                (EVENT_OBJECT_SHOW, EVENT_OBJECT_REORDER),
            ] {
                let hook = unsafe {
                    SetWinEventHook(
                        first,
                        last,
                        null_mut(),
                        Some(foreground_changed),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if hook.is_null() {
                    return Err(failure("SetWinEventHook overlay visibility"));
                }
                installed.hooks.push(hook);
            }
            *guard = Some(installed);
            Ok(())
        })
    }

    fn stop_foreground_guard(window: HWND) {
        let _ = FOREGROUND_GUARD.try_with(|slot| {
            if let Ok(mut guard) = slot.try_borrow_mut() {
                if guard.as_ref().is_some_and(|guard| guard.window == window) {
                    *guard = None;
                }
            }
        });
    }

    pub(super) fn set_visible(window: &Window, visible: bool) -> Result<(), String> {
        // winit's repeated show path uses SW_SHOW and regenerates styles. An
        // overlay must never activate, including its second and later hint.
        unsafe {
            let handle = hwnd(window)?;
            ensure_input_styles(handle)?;
            if visible {
                start_foreground_guard(handle)?;
            } else {
                stop_foreground_guard(handle);
            }
            if SetWindowPos(
                handle,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE
                    | SWP_NOMOVE
                    | SWP_NOSIZE
                    | if visible {
                        SWP_SHOWWINDOW
                    } else {
                        SWP_HIDEWINDOW
                    },
            ) == 0
            {
                let error = failure("SetWindowPos overlay visibility");
                stop_foreground_guard(handle);
                return Err(error);
            }
        }
        Ok(())
    }

    struct Dib {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        pixels: *mut u8,
        width: u32,
        height: u32,
    }

    impl Dib {
        fn new(width: u32, height: u32) -> Result<Self, String> {
            super::frame_len(width, height)?;
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                if dc.is_null() {
                    return Err(failure("CreateCompatibleDC"));
                }
                let mut info: BITMAPINFO = zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32), // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..zeroed()
                };
                let mut pixels: *mut c_void = null_mut();
                let bitmap =
                    CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
                if bitmap.is_null() || pixels.is_null() {
                    let error = failure("CreateDIBSection");
                    if !bitmap.is_null() {
                        DeleteObject(bitmap);
                    }
                    DeleteDC(dc);
                    return Err(error);
                }
                let previous = SelectObject(dc, bitmap);
                if previous.is_null() || previous as isize == -1 {
                    let error = failure("SelectObject");
                    DeleteObject(bitmap);
                    DeleteDC(dc);
                    return Err(error);
                }
                Ok(Self {
                    dc,
                    bitmap,
                    previous,
                    pixels: pixels.cast(),
                    width,
                    height,
                })
            }
        }

        fn pixels_mut(&mut self) -> &mut [u8] {
            let length = self.width as usize * self.height as usize * 4;
            unsafe { std::slice::from_raw_parts_mut(self.pixels, length) }
        }
    }

    impl Drop for Dib {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    struct ScreenDc(HDC);
    impl Drop for ScreenDc {
        fn drop(&mut self) {
            unsafe {
                ReleaseDC(null_mut(), self.0);
            }
        }
    }

    pub(super) struct Presenter {
        hwnd: HWND,
        dib: Option<Dib>,
    }

    impl Drop for Presenter {
        fn drop(&mut self) {
            stop_foreground_guard(self.hwnd);
            unsafe {
                RemovePropW(self.hwnd, OVERLAY_PROPERTY);
            }
        }
    }

    impl Presenter {
        pub(super) fn new(window: &Window) -> Result<Self, String> {
            let hwnd = hwnd(window)?;
            unsafe {
                prepare_layered(hwnd)?;
            }
            Ok(Self { hwnd, dib: None })
        }

        pub(super) fn present(
            &mut self,
            window: &Window,
            rgba: &[u8],
            width: u32,
            height: u32,
        ) -> Result<(), String> {
            if hwnd(window)? != self.hwnd {
                return Err("Presenter belongs to another window".into());
            }
            unsafe {
                ensure_input_styles(self.hwnd)?;
            }
            if self.dib.as_ref().map(|dib| (dib.width, dib.height)) != Some((width, height)) {
                self.dib = Some(Dib::new(width, height)?);
            }
            let dib = self.dib.as_mut().ok_or("Missing native pixel buffer")?;
            for (source, destination) in rgba
                .chunks_exact(4)
                .zip(dib.pixels_mut().chunks_exact_mut(4))
            {
                destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
            }
            let size = SIZE {
                cx: width as i32,
                cy: height as i32,
            };
            let origin = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: 0, // AC_SRC_OVER
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: 1, // AC_SRC_ALPHA, requires premultiplied BGRA
            };
            unsafe {
                let update = || {
                    UpdateLayeredWindow(
                        self.hwnd,
                        null_mut(),
                        null(),
                        &size,
                        dib.dc,
                        &origin,
                        0,
                        &blend,
                        ULW_ALPHA,
                    )
                };
                if update() == 0 {
                    // A later winit hit-test change can reintroduce layered opacity
                    // state. Recover once without relying on undocumented ordering.
                    prepare_layered(self.hwnd)?;
                    if update() == 0 {
                        return Err(failure("UpdateLayeredWindow"));
                    }
                }
            }
            Ok(())
        }
    }

    pub(super) fn sample(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<[u8; 3]>, String> {
        let mut dib = Dib::new(width, height)?;
        unsafe {
            let screen = ScreenDc(GetDC(null_mut()));
            if screen.0.is_null() {
                return Err(failure("GetDC"));
            }
            if BitBlt(
                dib.dc,
                0,
                0,
                width as i32,
                height as i32,
                screen.0,
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            ) == 0
            {
                return Err(failure("BitBlt background sample"));
            }
            // Complete batched GDI writes before directly reading DIB memory.
            if GdiFlush() == 0 {
                return Err(failure("GdiFlush background sample"));
            }
        }
        Ok(dib
            .pixels_mut()
            .chunks_exact(4)
            .map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect())
    }
}

#[cfg(target_os = "macos")]
// objc 0.2 macros emit a legacy cargo-clippy feature check in the caller.
#[allow(unexpected_cfgs)]
mod imp {
    use block::ConcreteBlock;
    use objc::runtime::{Class, Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::{c_void, CStr};
    use std::marker::PhantomData;
    use std::ptr::null;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    // Direct CoreGraphics declarations avoid a C/Objective-C build step. All
    // supported macOS Rust targets use 64-bit CGFloat and size_t.
    type CfRef = *const c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> CfRef;
        fn CGRectMakeWithDictionaryRepresentation(dictionary: CfRef, rect: *mut Rect) -> bool;
        static kCGWindowBounds: CfRef;
        static kCGWindowNumber: CfRef;
        static kCGWindowLayer: CfRef;
        static kCGWindowOwnerPID: CfRef;
        static kCGWindowIsOnscreen: CfRef;
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGDisplayBounds(display: u32) -> Rect;
        fn CGDisplayPixelsWide(display: u32) -> usize;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
        fn CGColorSpaceCreateDeviceRGB() -> CfRef;
        fn CGColorSpaceRelease(space: CfRef);
        fn CGDataProviderCreateWithData(
            info: *mut c_void,
            data: *const c_void,
            size: usize,
            release: Option<unsafe extern "C" fn(*mut c_void, *const c_void, usize)>,
        ) -> CfRef;
        fn CGDataProviderRelease(provider: CfRef);
        fn CGImageCreate(
            width: usize,
            height: usize,
            bits_per_component: usize,
            bits_per_pixel: usize,
            bytes_per_row: usize,
            space: CfRef,
            bitmap_info: u32,
            provider: CfRef,
            decode: *const f64,
            interpolate: bool,
            intent: u32,
        ) -> CfRef;
        fn CGImageRelease(image: CfRef);
        fn CGImageGetWidth(image: CfRef) -> usize;
        fn CGImageGetHeight(image: CfRef) -> usize;
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: CfRef,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextDrawImage(context: *mut c_void, rect: Rect, image: CfRef);
        fn CGContextRelease(context: *mut c_void);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: CfRef);
    }
    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}

    struct WindowInfo(CfRef);
    impl Drop for WindowInfo {
        fn drop(&mut self) {
            unsafe {
                CFRelease(self.0);
            }
        }
    }

    unsafe fn window_info(options: u32, id: u32) -> Result<WindowInfo, String> {
        let list = CGWindowListCopyWindowInfo(options, id);
        if list.is_null() {
            Err("macOS window information is unavailable".into())
        } else {
            Ok(WindowInfo(list))
        }
    }

    unsafe fn info_number(info: *mut Object, key: CfRef) -> u32 {
        let number: *mut Object = msg_send![info, objectForKey: key];
        if number.is_null() {
            0
        } else {
            msg_send![number, unsignedIntValue]
        }
    }

    unsafe fn info_bounds(info: *mut Object) -> Option<Rect> {
        let dictionary: *mut Object = msg_send![info, objectForKey: kCGWindowBounds];
        let mut rect = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 0.0,
                height: 0.0,
            },
        };
        if !dictionary.is_null()
            && CGRectMakeWithDictionaryRepresentation(dictionary.cast(), &mut rect)
            && rect.origin.x.is_finite()
            && rect.origin.y.is_finite()
            && rect.size.width.is_finite()
            && rect.size.height.is_finite()
            && rect.size.width > 0.0
            && rect.size.height > 0.0
        {
            Some(rect)
        } else {
            None
        }
    }

    unsafe fn display_scale_at(point: Point, fallback: f64) -> f64 {
        let screens: *mut Object = msg_send![class!(NSScreen), screens];
        let count: usize = msg_send![screens, count];
        let key: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String: b"NSScreenNumber\0".as_ptr()];
        for index in 0..count {
            let screen: *mut Object = msg_send![screens, objectAtIndex: index];
            let description: *mut Object = msg_send![screen, deviceDescription];
            let number: *mut Object = msg_send![description, objectForKey: key];
            if number.is_null() {
                continue;
            }
            let id: u32 = msg_send![number, unsignedIntValue];
            let bounds = CGDisplayBounds(id);
            if point.x >= bounds.origin.x
                && point.y >= bounds.origin.y
                && point.x < bounds.origin.x + bounds.size.width
                && point.y < bounds.origin.y + bounds.size.height
            {
                let scale: f64 = msg_send![screen, backingScaleFactor];
                if scale.is_finite() && scale > 0.0 {
                    return scale;
                }
            }
        }
        fallback
    }

    pub(super) struct WindowAnchor {
        id: u32,
        process: u32,
        // Logical offset of the selected target, used to choose the correct
        // backing scale when a window spans or moves between Retina displays.
        offset: Point,
        initial_scale: f64,
    }

    impl WindowAnchor {
        pub(super) fn capture(x: i32, y: i32, overlay: &Window) -> Result<Option<Self>, String> {
            let _ = view(overlay)?; // also enforces AppKit main-thread access
            unsafe {
                let _pool = Pool::new();
                let point = sample_rect(SampleRequest {
                    x,
                    y,
                    width: 1,
                    height: 1,
                })?;
                let scale = 1.0 / point.size.width;
                // Front-to-back visible windows, excluding desktop elements.
                // Window metadata needs no Screen Recording permission or capture.
                let list = window_info(1 | 16, 0)?;
                let array = list.0 as *mut Object;
                let count: usize = msg_send![array, count];
                for index in 0..count {
                    let info: *mut Object = msg_send![array, objectAtIndex: index];
                    let process = info_number(info, kCGWindowOwnerPID);
                    if process == std::process::id() || info_number(info, kCGWindowLayer) != 0 {
                        continue;
                    }
                    let Some(bounds) = info_bounds(info) else {
                        continue;
                    };
                    if point.origin.x >= bounds.origin.x
                        && point.origin.y >= bounds.origin.y
                        && point.origin.x < bounds.origin.x + bounds.size.width
                        && point.origin.y < bounds.origin.y + bounds.size.height
                    {
                        let id = info_number(info, kCGWindowNumber);
                        if id != 0 {
                            return Ok(Some(Self {
                                id,
                                process,
                                offset: Point {
                                    x: point.origin.x - bounds.origin.x,
                                    y: point.origin.y - bounds.origin.y,
                                },
                                initial_scale: scale,
                            }));
                        }
                    }
                }
            }
            Ok(None)
        }

        pub(super) fn poll(&self) -> Result<super::AnchorState, String> {
            unsafe {
                let main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
                if main_thread == NO {
                    return Err("macOS window tracking must run on the main thread".into());
                }
                let _pool = Pool::new();
                // IncludingWindow requests only this saved ID, including while
                // minimized/offscreen; it does not enumerate the whole desktop.
                let list = window_info(8, self.id)?;
                let array = list.0 as *mut Object;
                let count: usize = msg_send![array, count];
                for index in 0..count {
                    let info: *mut Object = msg_send![array, objectAtIndex: index];
                    if info_number(info, kCGWindowNumber) != self.id
                        || info_number(info, kCGWindowOwnerPID) != self.process
                    {
                        continue;
                    }
                    let Some(bounds) = info_bounds(info) else {
                        return Ok(super::AnchorState::destroyed());
                    };
                    let scale = display_scale_at(
                        Point {
                            x: bounds.origin.x + self.offset.x,
                            y: bounds.origin.y + self.offset.y,
                        },
                        self.initial_scale,
                    );
                    return Ok(super::AnchorState {
                        x: (bounds.origin.x * scale).round() as i32,
                        y: (bounds.origin.y * scale).round() as i32,
                        width: (bounds.size.width * scale).round().max(0.0) as u32,
                        height: (bounds.size.height * scale).round().max(0.0) as u32,
                        visible: info_number(info, kCGWindowIsOnscreen) != 0,
                        destroyed: false,
                        scale,
                    });
                }
                Ok(super::AnchorState::destroyed())
            }
        }
    }
    #[link(name = "ScreenCaptureKit", kind = "framework")]
    extern "C" {}

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct SampleRequest {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    }

    struct CaptureState {
        generation: u64,
        active: Option<(u64, SampleRequest, Instant)>,
        completed: Option<(SampleRequest, Instant, Result<Vec<[u8; 3]>, String>)>,
    }
    static CAPTURE: Mutex<CaptureState> = Mutex::new(CaptureState {
        generation: 0,
        active: None,
        completed: None,
    });

    fn sample_rect(request: SampleRequest) -> Result<Rect, String> {
        unsafe {
            let main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
            if main_thread == NO {
                return Err(
                    "macOS background sampling must be requested on the main thread".into(),
                );
            }
            // Query only when sampling, never at the overlay animation rate. This
            // also picks up display/Retina changes without a stale monitor cache.
            let screens: *mut Object = msg_send![class!(NSScreen), screens];
            let count: usize = msg_send![screens, count];
            let key: *mut Object = msg_send![class!(NSString),
                stringWithUTF8String: b"NSScreenNumber\0".as_ptr()];
            for index in 0..count {
                let screen: *mut Object = msg_send![screens, objectAtIndex: index];
                let description: *mut Object = msg_send![screen, deviceDescription];
                let number: *mut Object = msg_send![description, objectForKey: key];
                if number.is_null() {
                    continue;
                }
                let display: u32 = msg_send![number, unsignedIntValue];
                let scale: f64 = msg_send![screen, backingScaleFactor];
                if !scale.is_finite() || scale <= 0.0 {
                    continue;
                }
                let bounds = CGDisplayBounds(display);
                // Match winit 0.30 MonitorHandle: each display's origin and size
                // are converted from CG coordinates using its backing scale.
                let left = (bounds.origin.x * scale).round() as i64;
                let top = (bounds.origin.y * scale).round() as i64;
                let width = (CGDisplayPixelsWide(display) as f64 * scale).round() as i64;
                let height = (CGDisplayPixelsHigh(display) as f64 * scale).round() as i64;
                if request.x as i64 >= left
                    && request.y as i64 >= top
                    && request.x as i64 + request.width as i64 <= left + width
                    && request.y as i64 + request.height as i64 <= top + height
                {
                    // ScreenshotManager takes global top-left coordinates in points.
                    return Ok(Rect {
                        origin: Point {
                            x: request.x as f64 / scale,
                            y: request.y as f64 / scale,
                        },
                        size: Size {
                            width: request.width as f64 / scale,
                            height: request.height as f64 / scale,
                        },
                    });
                }
            }
        }
        Err("Background sample must fit inside one macOS display".into())
    }

    unsafe fn captured_pixels(
        image: CfRef,
        error: *mut Object,
        request: SampleRequest,
    ) -> Result<Vec<[u8; 3]>, String> {
        if !error.is_null() {
            let description: *mut Object = msg_send![error, localizedDescription];
            let text: *const std::ffi::c_char = msg_send![description, UTF8String];
            let detail = if text.is_null() {
                "unknown error".into()
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            };
            return Err(format!("macOS background capture failed: {detail}"));
        }
        if image.is_null() {
            return Err("macOS background capture returned no image".into());
        }
        let actual_width = CGImageGetWidth(image);
        let actual_height = CGImageGetHeight(image);
        if actual_width == 0 || actual_height == 0 || actual_width > 1024 || actual_height > 1024 {
            return Err("macOS returned unexpected background sample dimensions".into());
        }
        // Normalize the small CGImage into bounded 8-bit RGBA, including when the
        // desktop uses another color space or ScreenCaptureKit returns HiDPI data.
        let mut rgba = vec![0u8; request.width as usize * request.height as usize * 4];
        let space = CGColorSpaceCreateDeviceRGB();
        if space.is_null() {
            return Err("Background sample color space allocation failed".into());
        }
        let context = CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            request.width as usize,
            request.height as usize,
            8,
            request.width as usize * 4,
            space,
            0x4000 | 1,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return Err("Background sample bitmap allocation failed".into());
        }
        CGContextDrawImage(
            context,
            Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width: request.width as f64,
                    height: request.height as f64,
                },
            },
            image,
        );
        CGContextRelease(context);
        Ok(rgba
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect())
    }

    struct Pool(*mut Object);
    impl Pool {
        unsafe fn new() -> Self {
            Self(msg_send![class!(NSAutoreleasePool), new])
        }
    }
    impl Drop for Pool {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![self.0, drain];
            }
        }
    }

    struct Image(CfRef);
    impl Drop for Image {
        fn drop(&mut self) {
            unsafe {
                CGImageRelease(self.0);
            }
        }
    }

    unsafe extern "C" fn release_pixels(info: *mut c_void, _data: *const c_void, _size: usize) {
        drop(Box::from_raw(info.cast::<Vec<u8>>()));
    }

    fn image(rgba: &[u8], width: u32, height: u32) -> Result<Image, String> {
        unsafe {
            let data = Box::new(rgba.to_vec());
            let pixels = data.as_ptr();
            let length = data.len();
            let owner = Box::into_raw(data);
            let provider = CGDataProviderCreateWithData(
                owner.cast(),
                pixels.cast(),
                length,
                Some(release_pixels),
            );
            if provider.is_null() {
                drop(Box::from_raw(owner));
                return Err("CGDataProviderCreateWithData failed".into());
            }
            let space = CGColorSpaceCreateDeviceRGB();
            if space.is_null() {
                CGDataProviderRelease(provider);
                return Err("CGColorSpaceCreateDeviceRGB failed".into());
            }
            // Premultiplied-last, big-endian component order is RGBA bytes.
            let result = CGImageCreate(
                width as usize,
                height as usize,
                8,
                32,
                width as usize * 4,
                space,
                0x4000 | 1,
                provider,
                null(),
                false,
                0,
            );
            CGColorSpaceRelease(space);
            CGDataProviderRelease(provider);
            if result.is_null() {
                return Err("CGImageCreate failed".into());
            }
            Ok(Image(result))
        }
    }

    fn view(window: &Window) -> Result<*mut Object, String> {
        unsafe {
            let main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
            if main_thread == NO {
                return Err("AppKit presentation must run on the main thread".into());
            }
        }
        match window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw()
        {
            RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr().cast()),
            _ => Err("ScreenCue requires an AppKit window on macOS".into()),
        }
    }

    pub(super) fn set_visible(window: &Window, visible: bool) -> Result<(), String> {
        let view = view(window)?;
        unsafe {
            let _pool = Pool::new();
            let ns_window: *mut Object = msg_send![view, window];
            if ns_window.is_null() {
                return Err("AppKit view is not attached to a window".into());
            }
            configure_overlay(ns_window);
            if visible {
                // winit 0.30's set_visible(true) makes the window key. Ordering
                // it without making it key preserves the user's typing focus.
                let _: () = msg_send![ns_window, orderFrontRegardless];
            } else {
                let _: () = msg_send![ns_window, orderOut: null::<Object>()];
            }
        }
        Ok(())
    }

    unsafe fn configure_overlay(ns_window: *mut Object) {
        let _: () = msg_send![ns_window, setIgnoresMouseEvents: YES];
        let _: () = msg_send![ns_window, setHidesOnDeactivate: NO];
        // NSStatusWindowLevel: above ordinary document/floating windows without
        // attempting to outrank protected system UI or secure desktops.
        let _: () = msg_send![ns_window, setLevel: 25isize];
        let behavior: usize = msg_send![ns_window, collectionBehavior];
        let behavior = (behavior & !((1usize << 1) | (1usize << 7) | (1usize << 9)))
            | (1usize << 0)
            | (1usize << 8); // CanJoinAllSpaces | FullScreenAuxiliary
        let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
    }

    pub(super) struct Presenter {
        layer: *mut Object,
        view: *mut Object,
        // AppKit and release of its objects must stay on the event-loop thread.
        _not_send: PhantomData<Rc<()>>,
    }

    impl Presenter {
        pub(super) fn new(window: &Window) -> Result<Self, String> {
            let view = view(window)?;
            unsafe {
                let _pool = Pool::new();
                let ns_window: *mut Object = msg_send![view, window];
                if ns_window.is_null() {
                    return Err("AppKit view is not attached to a window".into());
                }
                let _: () = msg_send![ns_window, setOpaque: NO];
                let _: () = msg_send![ns_window, setHasShadow: NO];
                configure_overlay(ns_window);
                let clear: *mut Object = msg_send![class!(NSColor), clearColor];
                let _: () = msg_send![ns_window, setBackgroundColor: clear];
                let _: () = msg_send![view, setWantsLayer: YES];
                let parent: *mut Object = msg_send![view, layer];
                if parent.is_null() {
                    return Err("AppKit did not create a backing layer".into());
                }
                let layer: *mut Object = msg_send![class!(CALayer), new];
                if layer.is_null() {
                    return Err("CALayer allocation failed".into());
                }
                let _: () = msg_send![layer, setOpaque: NO];
                let _: () = msg_send![parent, setOpaque: NO];
                // Bounds/frame use NSValue to avoid structure-return Objective-C
                // ABI differences between Intel and Apple Silicon.
                let frame_key: *mut Object =
                    msg_send![class!(NSString), stringWithUTF8String: b"frame\0".as_ptr()];
                let bounds_key: *mut Object =
                    msg_send![class!(NSString), stringWithUTF8String: b"bounds\0".as_ptr()];
                let bounds: *mut Object = msg_send![view, valueForKey: bounds_key];
                let _: () = msg_send![layer, setValue: bounds forKey: frame_key];
                let _: () = msg_send![layer, setAutoresizingMask: 18u32]; // width + height sizable
                let _: () = msg_send![layer, setContentsScale: window.scale_factor()];
                let _: () = msg_send![parent, addSublayer: layer];
                Ok(Self {
                    layer,
                    view,
                    _not_send: PhantomData,
                })
            }
        }

        pub(super) fn present(
            &mut self,
            window: &Window,
            rgba: &[u8],
            width: u32,
            height: u32,
        ) -> Result<(), String> {
            if view(window)? != self.view {
                return Err("Presenter belongs to another window".into());
            }
            let image = image(rgba, width, height)?;
            unsafe {
                let _pool = Pool::new();
                let _: () = msg_send![class!(CATransaction), begin];
                let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
                let frame_key: *mut Object =
                    msg_send![class!(NSString), stringWithUTF8String: b"frame\0".as_ptr()];
                let bounds_key: *mut Object =
                    msg_send![class!(NSString), stringWithUTF8String: b"bounds\0".as_ptr()];
                let bounds: *mut Object = msg_send![self.view, valueForKey: bounds_key];
                let _: () = msg_send![self.layer, setValue: bounds forKey: frame_key];
                let _: () = msg_send![self.layer, setContentsScale: window.scale_factor()];
                // CALayer retains the CGImage; its provider owns the copied pixels.
                let _: () = msg_send![self.layer, setContents: image.0];
                let _: () = msg_send![class!(CATransaction), commit];
                let _: () = msg_send![class!(CATransaction), flush];
            }
            Ok(())
        }
    }

    impl Drop for Presenter {
        fn drop(&mut self) {
            unsafe {
                let _pool = Pool::new();
                let _: () = msg_send![self.layer, removeFromSuperlayer];
                let _: () = msg_send![self.layer, setContents: null::<c_void>()];
                let _: () = msg_send![self.layer, release];
            }
        }
    }

    pub(super) fn sample(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<[u8; 3]>, String> {
        unsafe {
            let _pool = Pool::new();
            let manager = Class::get("SCScreenshotManager")
                .ok_or("Automatic background sampling requires macOS 15.2 or later")?;
            let supported: BOOL = msg_send![manager,
                respondsToSelector: sel!(captureImageInRect:completionHandler:)];
            if supported == NO {
                return Err("Automatic background sampling requires macOS 15.2 or later".into());
            }
            // Preflight never asks for consent. We never call a permission-request
            // API or open System Settings; capture needs an existing user grant.
            if !CGPreflightScreenCaptureAccess() {
                return Err("macOS Screen Recording permission is not granted. Enable ScreenCue (or its launching terminal) in System Settings > Privacy & Security > Screen & System Audio Recording, then show the hint again. No permission request was opened.".into());
            }
            let request = SampleRequest {
                x,
                y,
                width,
                height,
            };
            let rect = sample_rect(request)?;
            let mut state = CAPTURE
                .lock()
                .map_err(|_| "macOS capture state is unavailable")?;
            if let Some((_, _, started)) = state.active {
                if started.elapsed() > Duration::from_secs(3) {
                    return Err(
                        "macOS background sample timed out; no additional capture was started"
                            .into(),
                    );
                }
                return Err("Background sample pending: macOS capture is asynchronous".into());
            }
            // Never accept a completed patch after the hint moves elsewhere.
            let completed = state
                .completed
                .take()
                .filter(|(region, captured, _)| {
                    *region == request && captured.elapsed() <= Duration::from_secs(3)
                })
                .map(|(_, _, result)| result);
            if let Some(result) = completed {
                // Let the caller advance to its next patch before starting any
                // further capture. Returning pixels must not queue old coordinates.
                return result;
            }
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.active = Some((generation, request, Instant::now()));
            drop(state);

            let completion = ConcreteBlock::new(move |image: CfRef, error: *mut Object| {
                // The callback may run on any queue, including the main queue.
                // Copy only the small patch while the framework owns the image.
                let _pool = unsafe { Pool::new() };
                let result = unsafe { captured_pixels(image, error, request) };
                if let Ok(mut state) = CAPTURE.lock() {
                    if state.active.as_ref().map(|active| active.0) == Some(generation) {
                        state.active = None;
                        state.completed = Some((request, Instant::now(), result));
                    }
                }
            })
            .copy();
            let _: () =
                msg_send![manager, captureImageInRect: rect completionHandler: &*completion];
            // No synchronous wait or run-loop pumping: winit stays responsive and
            // a later call consumes this result without starting another request.
            Err("Background sample pending: macOS capture is asynchronous".into())
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::cell::RefCell;
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;
    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::errors::ReplyError;
    use x11rb::protocol::render::{ConnectionExt as RenderConnectionExt, PictType};
    use x11rb::protocol::xproto::{
        AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, CreateGCAux,
        EventMask, ImageFormat, ImageOrder, MapState, PropMode, StackMode, VisualClass, Visualtype,
    };
    use x11rb::protocol::{ErrorKind, Event};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

    const OVERLAY_PROPERTY: &[u8] = b"_SCREENCUE_OVERLAY_V1";

    extern "C" {
        fn shutdown(socket: i32, how: i32) -> i32;
    }

    fn anchor_state(
        connection: &RustConnection,
        window: u32,
        root: u32,
    ) -> Result<super::AnchorState, ReplyError> {
        // Queue the three small requests together before waiting for replies.
        let attributes = connection.get_window_attributes(window)?;
        let geometry = connection.get_geometry(window)?;
        let position = connection.translate_coordinates(window, root, 0, 0)?;
        let attributes = attributes.reply()?;
        let geometry = geometry.reply()?;
        let position = position.reply()?;
        Ok(super::AnchorState {
            x: position.dst_x as i32,
            y: position.dst_y as i32,
            width: geometry.width as u32,
            height: geometry.height as u32,
            visible: attributes.map_state == MapState::VIEWABLE && position.same_screen,
            destroyed: false,
            scale: 1.0,
        })
    }

    pub(super) struct WindowAnchor {
        connection: RustConnection,
        window: u32,
        root: u32,
    }

    impl WindowAnchor {
        pub(super) fn capture(x: i32, y: i32, overlay: &Window) -> Result<Option<Self>, String> {
            let own = xid(overlay)?;
            let (connection, _) = x11rb::connect(None).map_err(|error| error.to_string())?;
            let root = connection
                .get_geometry(own)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .root;
            let atom = |name: &[u8]| -> Result<u32, String> {
                Ok(connection
                    .intern_atom(false, name)
                    .map_err(|error| error.to_string())?
                    .reply()
                    .map_err(|error| error.to_string())?
                    .atom)
            };
            let stacking = atom(b"_NET_CLIENT_LIST_STACKING")?;
            let window_type = atom(b"_NET_WM_WINDOW_TYPE")?;
            let desktop = atom(b"_NET_WM_WINDOW_TYPE_DESKTOP")?;
            let dock = atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
            let clients = connection
                .get_property(false, root, stacking, AtomEnum::WINDOW, 0, 16384)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?;
            let mut candidates: Vec<u32> = clients
                .value32()
                .map(|ids| ids.collect())
                .unwrap_or_default();
            if candidates.is_empty() {
                // ICCCM fallback: locate WM_STATE clients inside reparenting
                // frames. No desktop/root window is accepted as an anchor.
                let wm_state = atom(b"WM_STATE")?;
                let children = connection
                    .query_tree(root)
                    .map_err(|error| error.to_string())?
                    .reply()
                    .map_err(|error| error.to_string())?
                    .children;
                for child in children {
                    let mut pending = vec![(child, 0)];
                    for _ in 0..128 {
                        let Some((id, depth)) = pending.pop() else {
                            break;
                        };
                        if id == own {
                            continue;
                        }
                        let state = connection
                            .get_property(false, id, wm_state, AtomEnum::ANY, 0, 2)
                            .map_err(|error| error.to_string())?
                            .reply();
                        if state
                            .as_ref()
                            .is_ok_and(|state| state.format == 32 && !state.value.is_empty())
                        {
                            candidates.push(id);
                        } else if depth < 3 {
                            if let Ok(tree) = connection
                                .query_tree(id)
                                .map_err(|error| error.to_string())?
                                .reply()
                            {
                                pending.extend(tree.children.into_iter().map(|id| (id, depth + 1)));
                            }
                        }
                    }
                }
            }
            // EWMH and QueryTree both list bottom-to-top.
            for window in candidates.into_iter().rev() {
                if window == own || window == root {
                    continue;
                }
                let types = connection
                    .get_property(false, window, window_type, AtomEnum::ATOM, 0, 32)
                    .map_err(|error| error.to_string())?
                    .reply();
                if types
                    .as_ref()
                    .ok()
                    .and_then(|reply| reply.value32())
                    .is_some_and(|mut types| types.any(|kind| kind == desktop || kind == dock))
                {
                    continue;
                }
                if let Ok(state) = anchor_state(&connection, window, root) {
                    if state.visible && state.contains(x, y) {
                        return Ok(Some(Self {
                            connection,
                            window,
                            root,
                        }));
                    }
                }
            }
            Ok(None)
        }

        pub(super) fn poll(&self) -> Result<super::AnchorState, String> {
            match anchor_state(&self.connection, self.window, self.root) {
                Ok(state) => Ok(state),
                Err(ReplyError::X11Error(error))
                    if matches!(error.error_kind, ErrorKind::Window | ErrorKind::Drawable) =>
                {
                    Ok(super::AnchorState::destroyed())
                }
                Err(error) => Err(format!("X11 anchored window: {error}")),
            }
        }
    }

    struct StackGuard {
        window: u32,
        stopped: Arc<AtomicBool>,
        // A duplicate of only the observer socket. shutdown wakes a blocked
        // event read (or reply) without touching winit or the presenter socket.
        wake: OwnedFd,
        finished: mpsc::Receiver<()>,
        worker: Option<JoinHandle<()>>,
    }

    impl Drop for StackGuard {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            unsafe {
                shutdown(self.wake.as_raw_fd(), 2); // SHUT_RDWR on Linux
            }
            // No unbounded UI-thread join if the X server stops responding.
            // Socket shutdown also cancels a worker waiting for an X11 reply.
            if !matches!(
                self.finished.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
            }
        }
    }

    thread_local! {
        static STACK_GUARD: RefCell<Option<StackGuard>> = const { RefCell::new(None) };
    }

    fn raise_if_needed(
        connection: &RustConnection,
        root: u32,
        window: u32,
        marker: u32,
        stopped: &AtomicBool,
    ) -> Result<(), String> {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let attributes = connection
            .get_window_attributes(window)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        if attributes.map_state != MapState::VIEWABLE {
            return Ok(());
        }
        let tree = connection
            .query_tree(root)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        // QueryTree orders siblings bottom-to-top. Ignore other ScreenCue
        // processes and unmapped siblings, so peers never restack in a loop.
        // The usual already-topmost case needs no per-sibling requests.
        let Some(index) = tree.children.iter().position(|&child| child == window) else {
            return Ok(());
        };
        let mut covered = false;
        for &above in tree.children[index + 1..].iter().rev() {
            if stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            let property = connection
                .get_property(false, above, marker, AtomEnum::CARDINAL, 0, 1)
                .map_err(|error| error.to_string())?
                .reply();
            let property = match property {
                Ok(property) => property,
                Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => {
                    continue
                }
                Err(error) => return Err(error.to_string()),
            };
            if property.value32().and_then(|mut values| values.next()) == Some(1) {
                continue;
            }
            let attributes = connection
                .get_window_attributes(above)
                .map_err(|error| error.to_string())?
                .reply();
            match attributes {
                Ok(attributes) if attributes.map_state == MapState::VIEWABLE => {
                    covered = true;
                    break;
                }
                Ok(_) => {}
                Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        if covered && !stopped.load(Ordering::Acquire) {
            connection
                .configure_window(
                    window,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                )
                .map_err(|error| error.to_string())?
                .check()
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn start_stack_guard(window: u32) -> Result<(), String> {
        STACK_GUARD.with(|slot| {
            let mut guard = slot.borrow_mut();
            if guard.as_ref().is_some_and(|guard| {
                guard.window == window
                    && guard
                        .worker
                        .as_ref()
                        .is_some_and(|worker| !worker.is_finished())
            }) {
                return Ok(());
            }
            *guard = None;
            let (connection, _) = x11rb::connect(None).map_err(|error| error.to_string())?;
            let root = connection
                .get_geometry(window)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .root;
            let active_atom = connection
                .intern_atom(false, b"_NET_ACTIVE_WINDOW")
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .atom;
            let stacking_atom = connection
                .intern_atom(false, b"_NET_CLIENT_LIST_STACKING")
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .atom;
            let marker_atom = connection
                .intern_atom(false, OVERLAY_PROPERTY)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .atom;
            // Select notifications only, never SubstructureRedirect: the desktop
            // window manager retains full control over other applications.
            connection
                .change_window_attributes(
                    root,
                    &ChangeWindowAttributesAux::new()
                        .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE),
                )
                .map_err(|error| error.to_string())?
                .check()
                .map_err(|error| error.to_string())?;
            let wake = connection
                .stream()
                .as_fd()
                .try_clone_to_owned()
                .map_err(|error| error.to_string())?;
            let stopped = Arc::new(AtomicBool::new(false));
            let worker_stopped = Arc::clone(&stopped);
            let (done, finished) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("screencue-x11-stack".into())
                .spawn(move || {
                    // The observer sleeps in wait_for_event; static hints need
                    // no redraw timer and no periodic stacking query.
                    while !worker_stopped.load(Ordering::Acquire) {
                        let event = match connection.wait_for_event() {
                            Ok(event) => event,
                            Err(_) => break,
                        };
                        if worker_stopped.load(Ordering::Acquire) {
                            break;
                        }
                        let changed = match event {
                            Event::MapNotify(_)
                            | Event::CirculateNotify(_)
                            | Event::ReparentNotify(_) => true,
                            Event::ConfigureNotify(event) => event.window != window,
                            Event::PropertyNotify(event) => {
                                event.window == root
                                    && (event.atom == active_atom || event.atom == stacking_atom)
                            }
                            Event::DestroyNotify(event) if event.window == window => break,
                            Event::UnmapNotify(event) if event.window == window => break,
                            _ => false,
                        };
                        if changed
                            && raise_if_needed(
                                &connection,
                                root,
                                window,
                                marker_atom,
                                &worker_stopped,
                            )
                            .is_err()
                        {
                            break;
                        }
                    }
                    // Dropping this independent connection removes all event
                    // subscriptions. No X11 resources or listener survive hide.
                    drop(connection);
                    let _ = done.send(());
                })
                .map_err(|error| format!("Cannot start X11 stacking observer: {error}"))?;
            *guard = Some(StackGuard {
                window,
                stopped,
                wake,
                finished,
                worker: Some(worker),
            });
            Ok(())
        })
    }

    fn stop_stack_guard(window: u32) {
        let _ = STACK_GUARD.try_with(|slot| {
            if let Ok(mut guard) = slot.try_borrow_mut() {
                if guard.as_ref().is_some_and(|guard| guard.window == window) {
                    *guard = None;
                }
            }
        });
    }

    pub(super) fn set_visible(window: &Window, visible: bool) -> Result<(), String> {
        let id = xid(window)?;
        if visible {
            start_stack_guard(id)?;
        } else {
            stop_stack_guard(id);
        }
        window.set_visible(visible);
        Ok(())
    }

    fn xid(window: &Window) -> Result<u32, String> {
        match window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw()
        {
            RawWindowHandle::Xlib(handle) => {
                u32::try_from(handle.window).map_err(|_| "Invalid X11 window ID".into())
            }
            RawWindowHandle::Xcb(handle) => Ok(handle.window.get()),
            RawWindowHandle::Wayland(_) => Err(
                "Native Wayland overlay positioning is unsupported; use an X11 desktop session"
                    .into(),
            ),
            _ => Err("ScreenCue requires an X11 window on Linux".into()),
        }
    }

    fn visual(connection: &RustConnection, id: u32) -> Result<Visualtype, String> {
        connection
            .setup()
            .roots
            .iter()
            .flat_map(|screen| screen.allowed_depths.iter())
            .flat_map(|depth| depth.visuals.iter())
            .find(|visual| visual.visual_id == id)
            .cloned()
            .ok_or_else(|| "X11 visual was not found in the display setup".into())
    }

    fn check_mask(mask: u32) -> bool {
        if mask == 0 {
            return false;
        }
        let normalized = mask >> mask.trailing_zeros();
        normalized & normalized.wrapping_add(1) == 0
    }

    fn pack(value: u8, mask: u32) -> u32 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let maximum = mask >> shift;
        (((value as u64 * maximum as u64 + 127) / 255) as u32) << shift
    }

    fn unpack(value: u32, mask: u32) -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let maximum = mask >> shift;
        ((((value & mask) >> shift) as u64 * 255 + maximum as u64 / 2) / maximum as u64) as u8
    }

    pub(super) struct Presenter {
        connection: RustConnection,
        window: u32,
        gc: u32,
        marker: u32,
        red: u32,
        green: u32,
        blue: u32,
        alpha: u32,
        little_endian: bool,
    }

    impl Presenter {
        pub(super) fn new(window: &Window) -> Result<Self, String> {
            let window = xid(window)?;
            let (connection, _) = x11rb::connect(None).map_err(|error| error.to_string())?;
            let geometry = connection
                .get_geometry(window)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?;
            if geometry.depth != 32 {
                return Err("X11 overlay requires a 32-bit ARGB visual; create the winit window with transparency enabled".into());
            }
            let screen_index = connection
                .setup()
                .roots
                .iter()
                .position(|screen| screen.root == geometry.root)
                .ok_or("X11 window does not belong to the connected display")?;
            let compositor_name = format!("_NET_WM_CM_S{screen_index}");
            let compositor = connection
                .intern_atom(false, compositor_name.as_bytes())
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .atom;
            if connection
                .get_selection_owner(compositor)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .owner
                == x11rb::NONE
            {
                return Err(
                    "Transparent X11 hints require an active compositing window manager".into(),
                );
            }
            let attributes = connection
                .get_window_attributes(window)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?;
            let visual = visual(&connection, attributes.visual)?;
            // Depth32 alone can also describe RGBX. Ask Render for this visual's
            // actual alpha mask rather than treating all unused RGB bits as alpha.
            let pictures = connection
                .render_query_pict_formats()
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?;
            let picture_id = pictures
                .screens
                .get(screen_index)
                .and_then(|screen| {
                    screen
                        .depths
                        .iter()
                        .flat_map(|depth| depth.visuals.iter())
                        .find(|picture| picture.visual == attributes.visual)
                })
                .map(|picture| picture.format)
                .ok_or("X11 visual has no Render picture format")?;
            let picture = pictures
                .formats
                .iter()
                .find(|format| format.id == picture_id)
                .ok_or("X11 Render picture format is unavailable")?;
            if picture.type_ != PictType::DIRECT
                || picture.depth != 32
                || picture.direct.alpha_mask == 0
            {
                return Err("X11 overlay visual has no usable alpha channel".into());
            }
            let channel = |mask: u16, shift: u16| -> Result<u32, String> {
                if shift >= 32 {
                    return Err("Unsupported X11 Render channel shift".into());
                }
                let value = (mask as u64) << shift;
                if value > u32::MAX as u64 {
                    return Err("Unsupported X11 Render channel mask".into());
                }
                Ok(value as u32)
            };
            let direct = &picture.direct;
            let alpha = channel(direct.alpha_mask, direct.alpha_shift)?;
            if channel(direct.red_mask, direct.red_shift)? != visual.red_mask
                || channel(direct.green_mask, direct.green_shift)? != visual.green_mask
                || channel(direct.blue_mask, direct.blue_shift)? != visual.blue_mask
                || alpha & (visual.red_mask | visual.green_mask | visual.blue_mask) != 0
            {
                return Err("X11 visual and Render channel layouts disagree".into());
            }
            if visual.class != VisualClass::TRUE_COLOR
                || ![visual.red_mask, visual.green_mask, visual.blue_mask, alpha]
                    .into_iter()
                    .all(check_mask)
            {
                return Err("Unsupported X11 ARGB channel layout".into());
            }
            let format = connection
                .setup()
                .pixmap_formats
                .iter()
                .find(|format| format.depth == 32)
                .ok_or("X11 display has no 32-bit pixmap format")?;
            if format.bits_per_pixel != 32 || format.scanline_pad != 32 {
                return Err("Unsupported X11 32-bit pixel format".into());
            }
            let little_endian = connection.setup().image_byte_order == ImageOrder::LSB_FIRST;
            let marker = connection
                .intern_atom(false, OVERLAY_PROPERTY)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .atom;
            connection
                .change_property32(PropMode::REPLACE, window, marker, AtomEnum::CARDINAL, &[1])
                .map_err(|error| error.to_string())?
                .check()
                .map_err(|error| error.to_string())?;
            // Only annotate our own surface. These EWMH hints ask the compositor
            // to preserve alpha composition; they do not force game redirection.
            for (property, value, property_type) in [
                (
                    b"_NET_WM_BYPASS_COMPOSITOR".as_slice(),
                    2,
                    AtomEnum::CARDINAL,
                ),
                (
                    b"_NET_WM_WINDOW_TYPE".as_slice(),
                    connection
                        .intern_atom(false, b"_NET_WM_WINDOW_TYPE_NOTIFICATION")
                        .map_err(|error| error.to_string())?
                        .reply()
                        .map_err(|error| error.to_string())?
                        .atom,
                    AtomEnum::ATOM,
                ),
            ] {
                let property = connection
                    .intern_atom(false, property)
                    .map_err(|error| error.to_string())?
                    .reply()
                    .map_err(|error| error.to_string())?
                    .atom;
                connection
                    .change_property32(PropMode::REPLACE, window, property, property_type, &[value])
                    .map_err(|error| error.to_string())?
                    .check()
                    .map_err(|error| error.to_string())?;
            }
            let gc = connection
                .generate_id()
                .map_err(|error| error.to_string())?;
            connection
                .create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0u32))
                .map_err(|error| error.to_string())?
                .check()
                .map_err(|error| error.to_string())?;
            Ok(Self {
                connection,
                window,
                gc,
                marker,
                red: visual.red_mask,
                green: visual.green_mask,
                blue: visual.blue_mask,
                alpha,
                little_endian,
            })
        }

        pub(super) fn present(
            &mut self,
            window: &Window,
            rgba: &[u8],
            width: u32,
            height: u32,
        ) -> Result<(), String> {
            if xid(window)? != self.window {
                return Err("Presenter belongs to another window".into());
            }
            // Stacking belongs to the visible-only event observer, including
            // the initial MapNotify. Animation never changes peer window order.
            let mut native = Vec::with_capacity(rgba.len());
            for pixel in rgba.chunks_exact(4) {
                let value = pack(pixel[0], self.red)
                    | pack(pixel[1], self.green)
                    | pack(pixel[2], self.blue)
                    | pack(pixel[3], self.alpha);
                native.extend_from_slice(&if self.little_endian {
                    value.to_le_bytes()
                } else {
                    value.to_be_bytes()
                });
            }
            let row_bytes = width as usize * 4;
            // Reserve more than the PutImage request header, including BigRequests
            // framing. Splitting into row groups avoids server request limits.
            let maximum = self.connection.maximum_request_bytes().saturating_sub(64);
            let rows = (maximum / row_bytes).min(u16::MAX as usize);
            if rows == 0 {
                return Err("X11 request limit is smaller than one overlay row".into());
            }
            let mut row = 0usize;
            while row < height as usize {
                let count = rows.min(height as usize - row);
                let start = row * row_bytes;
                self.connection
                    .put_image(
                        ImageFormat::Z_PIXMAP,
                        self.window,
                        self.gc,
                        width as u16,
                        count as u16,
                        0,
                        row as i16,
                        0,
                        32,
                        &native[start..start + count * row_bytes],
                    )
                    .map_err(|error| error.to_string())?
                    .check()
                    .map_err(|error| error.to_string())?;
                row += count;
            }
            self.connection.flush().map_err(|error| error.to_string())
        }
    }

    impl Drop for Presenter {
        fn drop(&mut self) {
            stop_stack_guard(self.window);
            let _ = self.connection.delete_property(self.window, self.marker);
            let _ = self.connection.free_gc(self.gc);
            let _ = self.connection.flush();
        }
    }

    pub(super) fn sample(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<[u8; 3]>, String> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            return Err("Native Wayland background sampling is unavailable".into());
        }
        let (connection, screen_index) = x11rb::connect(None).map_err(|error| error.to_string())?;
        let screen = &connection.setup().roots[screen_index];
        if x < 0
            || y < 0
            || x > i16::MAX as i32
            || y > i16::MAX as i32
            || x as i64 + width as i64 > screen.width_in_pixels as i64
            || y as i64 + height as i64 > screen.height_in_pixels as i64
        {
            return Err("Background sample lies outside the X11 root window".into());
        }
        let reply = connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                screen.root,
                x as i16,
                y as i16,
                width as u16,
                height as u16,
                u32::MAX,
            )
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        let visual = visual(
            &connection,
            if reply.visual == 0 {
                screen.root_visual
            } else {
                reply.visual
            },
        )?;
        if visual.class != VisualClass::TRUE_COLOR
            || ![visual.red_mask, visual.green_mask, visual.blue_mask]
                .into_iter()
                .all(check_mask)
        {
            return Err("Background sampling requires an X11 TrueColor visual".into());
        }
        let format = connection
            .setup()
            .pixmap_formats
            .iter()
            .find(|format| format.depth == reply.depth)
            .ok_or("X11 sample pixel format is unavailable")?;
        if ![16, 24, 32].contains(&format.bits_per_pixel)
            || ![8, 16, 32].contains(&format.scanline_pad)
        {
            return Err("Unsupported X11 background pixel format".into());
        }
        let bytes_per_pixel = format.bits_per_pixel as usize / 8;
        let pad = format.scanline_pad as usize;
        let row_bytes = (width as usize * format.bits_per_pixel as usize).div_ceil(pad) * pad / 8;
        if reply.data.len() < row_bytes * height as usize {
            return Err("X11 returned an incomplete background image".into());
        }
        let little_endian = connection.setup().image_byte_order == ImageOrder::LSB_FIRST;
        let mut result = Vec::with_capacity(width as usize * height as usize);
        for row in 0..height as usize {
            for column in 0..width as usize {
                let offset = row * row_bytes + column * bytes_per_pixel;
                let bytes = &reply.data[offset..offset + bytes_per_pixel];
                let mut value = 0u32;
                for (index, &byte) in bytes.iter().enumerate() {
                    let shift = if little_endian {
                        index
                    } else {
                        bytes_per_pixel - index - 1
                    };
                    value |= (byte as u32) << (shift * 8);
                }
                result.push([
                    unpack(value, visual.red_mask),
                    unpack(value, visual.green_mask),
                    unpack(value, visual.blue_mask),
                ]);
            }
        }
        Ok(result)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod imp {
    use winit::window::Window;
    pub(super) struct WindowAnchor;
    impl WindowAnchor {
        pub(super) fn capture(_x: i32, _y: i32, _overlay: &Window) -> Result<Option<Self>, String> {
            Ok(None)
        }
        pub(super) fn poll(&self) -> Result<super::AnchorState, String> {
            Ok(super::AnchorState::destroyed())
        }
    }
    pub(super) struct Presenter;
    impl Presenter {
        pub(super) fn new(_window: &Window) -> Result<Self, String> {
            Err("ScreenCue supports Windows, macOS, and Linux/X11".into())
        }
        pub(super) fn present(
            &mut self,
            _window: &Window,
            _rgba: &[u8],
            _width: u32,
            _height: u32,
        ) -> Result<(), String> {
            Err("Native presentation is unavailable on this platform".into())
        }
    }
    pub(super) fn sample(
        _x: i32,
        _y: i32,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<[u8; 3]>, String> {
        Err("Background sampling is unavailable on this platform".into())
    }
}
