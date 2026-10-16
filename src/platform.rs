//! Native, on-demand presentation of premultiplied RGBA pixels.
//!
//! No graphics context or redraw loop is owned here. The caller keeps the winit
//! window alive, invokes these methods on its event-loop thread, and hides the
//! previous hint before asking for an optional small background sample.

use winit::window::Window;

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
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError, HWND, POINT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CAPTUREBLT,
        DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, SRCCOPY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, UpdateLayeredWindow, GWL_EXSTYLE, ULW_ALPHA,
        WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

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
        set_styles(window, style)
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
    use objc::runtime::{Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::c_void;
    use std::marker::PhantomData;
    use std::ptr::null;
    use std::rc::Rc;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    // Direct CoreGraphics declarations avoid a C/Objective-C build step. All
    // supported macOS Rust targets use 64-bit CGFloat and size_t.
    type CfRef = *const c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
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
    }
    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}

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
                let _: () = msg_send![ns_window, setIgnoresMouseEvents: YES];
                let _: () = msg_send![ns_window, setHidesOnDeactivate: NO];
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

    pub(super) fn sample(
        _x: i32,
        _y: i32,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<[u8; 3]>, String> {
        Err("Automatic background sampling is unavailable on macOS in this build; use an explicit color or supply background_colors. No screen-capture permission was requested.".into())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;
    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::protocol::xproto::{
        ConnectionExt, CreateGCAux, ImageFormat, ImageOrder, VisualClass, Visualtype,
    };
    use x11rb::rust_connection::RustConnection;

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
            let alpha = !(visual.red_mask | visual.green_mask | visual.blue_mask);
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
