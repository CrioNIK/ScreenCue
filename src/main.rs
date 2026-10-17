#![recursion_limit = "256"]
#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]
mod color;
mod model;
mod platform;
mod protocol;
mod renderer;
mod shortcut;
mod tracking;

use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use model::{Hint, Rect};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId, WindowLevel},
};

enum Event {
    Command(protocol::Command),
    HideShortcut(u32),
    Quit,
}
struct Animation {
    monitor: Rect,
    scale: f32,
    color: [u8; 3],
    started: Instant,
    next: Option<Instant>,
    next_color: Option<Instant>,
    background: [Vec<[u8; 3]>; 2],
    sample_index: usize,
}
struct Tracking {
    anchor: platform::WindowAnchor,
    origin: platform::AnchorState,
    last: platform::AnchorState,
    base: Hint,
    next: Option<Instant>,
}
struct App {
    window: Option<Window>,
    presenter: Option<platform::Presenter>,
    renderer: renderer::Renderer,
    frame: Option<renderer::Frame>,
    hint: Option<Hint>,
    render_hint: Option<Hint>,
    tracking: Option<Tracking>,
    tracking_state: String,
    tracking_note: Option<String>,
    expiry: Option<Instant>,
    once: bool,
    initial: Option<Value>,
    failure: Option<String>,
    color_source: String,
    color_note: Option<String>,
    shortcut: Option<shortcut::Shortcut>,
    animation: Option<Animation>,
}
impl App {
    fn new(once: bool, initial: Option<Value>) -> Result<Self, String> {
        Ok(Self {
            window: None,
            presenter: None,
            renderer: renderer::Renderer::new()?,
            frame: None,
            hint: None,
            render_hint: None,
            tracking: None,
            tracking_state: "fixed".into(),
            tracking_note: None,
            expiry: None,
            once,
            initial,
            failure: None,
            color_source: String::new(),
            color_note: None,
            shortcut: None,
            animation: None,
        })
    }
    fn tracking_status(&self) -> Value {
        json!({"state":self.tracking_state,"note":self.tracking_note,
            "window_bounds":self.tracking.as_ref().map(|t|[t.last.x as i64,t.last.y as i64,t.last.width as i64,t.last.height as i64]),
            "window_scale":self.tracking.as_ref().map(|t|t.last.scale)})
    }
    fn clear(&mut self) {
        if let Some(w) = &self.window {
            let _ = platform::set_visible(w, false);
        }
        self.frame = None;
        self.hint = None;
        self.render_hint = None;
        self.tracking = None;
        self.tracking_state = "fixed".into();
        self.tracking_note = None;
        self.expiry = None;
        self.animation = None;
        if let Some(shortcut) = &mut self.shortcut {
            shortcut.unregister();
        }
    }
    fn dispatch(&mut self, event_loop: &ActiveEventLoop, request: Value) -> Value {
        match self.execute(event_loop, request) {
            Ok(value) => value,
            Err(error) => json!({"ok":false,"error":error}),
        }
    }
    fn update_tracking(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let Some(tracker) = self.tracking.as_mut() else {
            return Ok(());
        };
        if !tracker.next.is_some_and(|next| Instant::now() >= next) {
            return Ok(());
        }
        tracker.next = Some(Instant::now() + tracking_interval());
        let current = match tracker.anchor.poll() {
            Ok(state) => state,
            Err(error) => {
                self.tracking_note = Some(error);
                tracker.next = Some(Instant::now() + Duration::from_millis(250));
                return Ok(());
            }
        };
        if same_anchor(tracker.last, current) {
            return Ok(());
        }
        let actual = if current.visible && !current.destroyed {
            tracking::translated(&tracker.base, tracker.origin, current)
        } else {
            self.hint.clone().ok_or("Missing tracked target")?
        };
        let monitors: Vec<_> = event_loop
            .available_monitors()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                (
                    Rect {
                        x: p.x as f32,
                        y: p.y as f32,
                        w: s.width as f32,
                        h: s.height as f32,
                    },
                    m.scale_factor() as f32,
                )
            })
            .collect();
        let fitting = monitors
            .iter()
            .copied()
            .find(|(bounds, _)| tracking::fits(&actual, *bounds));
        let (state, note) = if current.destroyed {
            ("closed", Some("Window closed"))
        } else if !current.visible {
            ("minimized", Some("Window minimized or hidden"))
        } else if tracking::resized(tracker.origin, current) {
            ("needs_refresh", Some("Window resized · refresh target"))
        } else if fitting.is_none() {
            ("offscreen", Some("Target offscreen"))
        } else {
            ("window", None)
        };
        let (monitor, scale) = fitting
            .or_else(|| tracking::nearest(&actual, &monitors))
            .ok_or("No connected display for tracked cue")?;
        let displayed = if let Some(note) = note {
            tracking::edge_hint(&actual, monitor, scale, note)
        } else {
            actual.clone()
        };
        let a = self.animation.as_mut().ok_or("Missing tracked animation")?;
        let window = self.window.as_ref().ok_or("Missing overlay window")?;
        let old_target = self
            .render_hint
            .as_ref()
            .ok_or("Missing displayed target")?;
        let dx = displayed.x.checked_sub(old_target.x);
        let dy = displayed.y.checked_sub(old_target.y);
        // Most drag updates move only the native window. Reuse every pixel;
        // rerender only when an edge, DPI, display or target state changes.
        let mut moved = false;
        if self.tracking_state == "window"
            && state == "window"
            && same_rect(a.monitor, monitor)
            && a.scale == scale
        {
            if let (Some(dx), Some(dy), Some(frame)) = (dx, dy, self.frame.as_mut()) {
                let destination = Rect {
                    x: frame.x as f32 + dx as f32,
                    y: frame.y as f32 + dy as f32,
                    w: frame.pixmap.width() as f32,
                    h: frame.pixmap.height() as f32,
                };
                if destination.x >= monitor.x
                    && destination.y >= monitor.y
                    && destination.right() <= monitor.right()
                    && destination.bottom() <= monitor.bottom()
                {
                    platform::set_bounds(
                        window,
                        frame
                            .x
                            .checked_add(dx)
                            .ok_or("Frame translation overflow")?,
                        frame
                            .y
                            .checked_add(dy)
                            .ok_or("Frame translation overflow")?,
                        frame.pixmap.width(),
                        frame.pixmap.height(),
                        scale,
                    )?;
                    frame.translate(dx, dy)?;
                    debug_assert!(frame.fits(monitor));
                    moved = true;
                }
            }
        }
        if !moved {
            let frame = self.renderer.render_at(
                &displayed,
                monitor,
                scale,
                a.color,
                if displayed.animate {
                    a.started.elapsed().as_secs_f32()
                } else {
                    0.
                },
            )?;
            platform::set_bounds(
                window,
                frame.x,
                frame.y,
                frame.pixmap.width(),
                frame.pixmap.height(),
                scale,
            )?;
            self.presenter
                .as_mut()
                .ok_or("Missing presenter")?
                .present(
                    window,
                    frame.pixmap.data(),
                    frame.pixmap.width(),
                    frame.pixmap.height(),
                )?;
            self.frame = Some(frame);
        }
        a.monitor = monitor;
        a.scale = scale;
        a.background = [Vec::new(), Vec::new()];
        // Do not analyze a screenshot from a different stage of a window drag.
        // Keep the last color while moving, then sample the settled location.
        if a.next_color.is_some() {
            a.next_color = Some(Instant::now() + Duration::from_millis(250));
        }
        self.hint = Some(actual);
        self.render_hint = Some(displayed);
        self.tracking_state = state.into();
        self.tracking_note = note.map(str::to_owned);
        tracker.last = current;
        if current.destroyed {
            tracker.next = None;
        }
        Ok(())
    }
    fn execute(&mut self, event_loop: &ActiveEventLoop, request: Value) -> Result<Value, String> {
        let command = request
            .get("command")
            .and_then(Value::as_str)
            .ok_or("command is required")?;
        match command {
            "displays" => {
                let displays:Vec<Value>=event_loop.available_monitors().map(|m|{let p=m.position();let s=m.size();json!({"name":m.name(),"x":p.x,"y":p.y,"width":s.width,"height":s.height,"scale":m.scale_factor()})}).collect();
                Ok(
                    json!({"ok":true,"coordinate_space":"physical desktop pixels","platform":std::env::consts::OS,"displays":displays}),
                )
            }
            "clear" => {
                self.clear();
                Ok(json!({"ok":true,"visible":false}))
            }
            "status" => Ok(
                json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"visible":self.frame.is_some(),"hint":self.hint,"tracking":self.tracking_status(),"rendered_target":self.render_hint.as_ref().map(Hint::anchor),"arrow_tip":self.frame.as_ref().and_then(|f|f.arrow_tip),"color_source":self.color_source,"color_note":self.color_note,"color":self.animation.as_ref().map(|a|color_hex(a.color)),"hide_shortcut":self.shortcut.as_ref().map(|s|&s.name),"hide_shortcut_id":self.shortcut.as_ref().map(|s|s.id()),"shortcut_active":self.shortcut.as_ref().is_some_and(|s|s.active()),"shortcut_error":self.shortcut.as_ref().and_then(|s|s.error.as_ref()),"pid":std::process::id()}),
            ),
            "show" => {
                let hint = Hint::parse(request)?;
                let monitor = event_loop
                    .available_monitors()
                    .find(|m| {
                        let p = m.position();
                        let s = m.size();
                        Rect {
                            x: p.x as f32,
                            y: p.y as f32,
                            w: s.width as f32,
                            h: s.height as f32,
                        }
                        .contains(hint.x as f32, hint.y as f32)
                    })
                    .ok_or("Target is outside the connected displays; call screen_displays")?;
                let pos = monitor.position();
                let size = monitor.size();
                let bounds = Rect {
                    x: pos.x as f32,
                    y: pos.y as f32,
                    w: size.width as f32,
                    h: size.height as f32,
                };
                let region = hint.bounds();
                if (hint.shape() == "polygon" || hint.width.is_some() || hint.height.is_some())
                    && (region.x < bounds.x
                        || region.y < bounds.y
                        || region.right() > bounds.right()
                        || region.bottom() > bounds.bottom())
                {
                    return Err("The complete target must fit on one display".into());
                }
                let layout = self.renderer.render(
                    &hint,
                    bounds,
                    monitor.scale_factor() as f32,
                    color::FALLBACK,
                )?;
                let mut tracking_note = None;
                let tracked = if hint.follow_window {
                    let (x, y) = hint.anchor();
                    match platform::WindowAnchor::capture(
                        x as i32,
                        y as i32,
                        self.window.as_ref().ok_or("Window is not ready")?,
                    )
                    .and_then(|anchor| {
                        anchor
                            .map(|anchor| anchor.poll().map(|state| (anchor, state)))
                            .transpose()
                    }) {
                        Ok(Some((anchor, state))) if !state.destroyed && state.visible => {
                            Some(Tracking {
                                anchor,
                                origin: state,
                                last: state,
                                base: hint.clone(),
                                next: Some(Instant::now() + tracking_interval()),
                            })
                        }
                        Ok(_) => {
                            tracking_note = Some("No application window under target; using fixed desktop coordinates".into());
                            None
                        }
                        Err(error) => {
                            tracking_note = Some(error);
                            None
                        }
                    }
                } else {
                    None
                };
                self.clear();
                self.tracking_state = if tracked.is_some() {
                    "window"
                } else if hint.follow_window {
                    "unavailable"
                } else {
                    "fixed"
                }
                .into();
                self.tracking_note = tracking_note;
                self.tracking = tracked;
                let mut reason = None;
                let mut background = [Vec::new(), Vec::new()];
                let selected = if hint.color == "auto" && !hint.background_colors.is_empty() {
                    self.color_source = "client_palette".into();
                    color::choose(
                        &hint
                            .background_colors
                            .iter()
                            .map(|c| model::parse_color(c).unwrap())
                            .collect::<Vec<_>>(),
                    )
                } else if hint.color == "auto" {
                    match sample_background(&hint, bounds, Some(&layout), 0) {
                        Ok(pixels) if !pixels.is_empty() => {
                            self.color_source = "local_pixels".into();
                            let selected = color::choose(&pixels);
                            background[0] = pixels;
                            selected
                        }
                        other => {
                            self.color_source = "fallback".into();
                            reason = Some(
                                other
                                    .err()
                                    .unwrap_or("Pixel sampling returned no pixels".into()),
                            );
                            color::FALLBACK
                        }
                    }
                } else {
                    self.color_source = "manual".into();
                    model::parse_color(&hint.color)?
                };
                let frame =
                    self.renderer
                        .render(&hint, bounds, monitor.scale_factor() as f32, selected)?;
                let window = self.window.as_ref().ok_or("Window is not ready")?;
                platform::set_bounds(
                    window,
                    frame.x,
                    frame.y,
                    frame.pixmap.width(),
                    frame.pixmap.height(),
                    monitor.scale_factor() as f32,
                )?;
                self.presenter
                    .as_mut()
                    .ok_or("Presenter is not ready")?
                    .present(
                        window,
                        frame.pixmap.data(),
                        frame.pixmap.width(),
                        frame.pixmap.height(),
                    )?;
                platform::set_visible(window, true)?;
                // Exposure redraw is separate from the bounded visible-only animation.
                window.request_redraw();
                self.expiry = if hint.duration_ms > 0 {
                    Some(Instant::now() + Duration::from_millis(hint.duration_ms))
                } else {
                    None
                };
                let arrow_tip = frame.arrow_tip;
                let now = Instant::now();
                self.animation = Some(Animation {
                    sample_index: usize::from(
                        self.color_source == "local_pixels" && layout.arrow_start.is_some(),
                    ),
                    background,
                    monitor: bounds,
                    scale: monitor.scale_factor() as f32,
                    color: selected,
                    started: now,
                    next: hint.animate.then_some(now + Duration::from_millis(84)),
                    next_color: (self.color_source == "local_pixels"
                        || reason
                            .as_deref()
                            .is_some_and(|r| r.starts_with("Background sample pending")))
                    .then_some(now + Duration::from_secs(1)),
                });
                if let Some(shortcut) = &mut self.shortcut {
                    shortcut.register();
                }
                self.color_note = reason;
                self.frame = Some(frame);
                self.render_hint = Some(hint.clone());
                self.hint = Some(hint);
                Ok(
                    json!({"ok":true,"visible":true,"arrow_tip":arrow_tip,"tracking":self.tracking_status(),"rendered_target":self.render_hint.as_ref().map(Hint::anchor),"color":color_hex(selected),"color_source":self.color_source,"color_note":self.color_note,"hide_shortcut":self.shortcut.as_ref().map(|s|&s.name),"hide_shortcut_id":self.shortcut.as_ref().map(|s|s.id()),"shortcut_active":self.shortcut.as_ref().is_some_and(|s|s.active()),"shortcut_error":self.shortcut.as_ref().and_then(|s|s.error.as_ref())}),
                )
            }
            _ => Err(format!("Unknown command: {command}")),
        }
    }
}
impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("ScreenCue Overlay")
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_active(false)
            .with_visible(false)
            .with_inner_size(PhysicalSize::new(1, 1));
        #[cfg(target_os = "windows")]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes
                .with_skip_taskbar(true)
                .with_undecorated_shadow(false)
        };
        #[cfg(target_os = "macos")]
        let attributes = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attributes.with_has_shadow(false)
        };
        #[cfg(target_os = "linux")]
        let attributes = {
            use winit::platform::x11::WindowAttributesExtX11;
            attributes.with_override_redirect(true)
        };
        let result = (|| {
            let window = event_loop
                .create_window(attributes)
                .map_err(|e| e.to_string())?;
            window
                .set_cursor_hittest(false)
                .map_err(|e| format!("Click-through unavailable: {e}"))?;
            let presenter = platform::Presenter::new(&window)?;
            self.window = Some(window);
            self.presenter = Some(presenter);
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.failure = Some(error);
            event_loop.exit();
            return;
        }
        if let Some(mut request) = self.initial.take() {
            if request["command"] == "demo" {
                if let Some(m) = event_loop
                    .primary_monitor()
                    .or_else(|| event_loop.available_monitors().next())
                {
                    let p = m.position();
                    let s = m.size();
                    let shortcut = self
                        .shortcut
                        .as_ref()
                        .map(|s| s.name.as_str())
                        .unwrap_or(shortcut::DEFAULT);
                    request = json!({"command":"show","x":p.x+s.width as i32/2,"y":p.y+s.height as i32/2,"text":format!("ScreenCue · {shortcut} — hide"),"duration_ms":0,"follow_window":false});
                }
            }
            let result = self.dispatch(event_loop, request);
            println!("{result}");
            if result["ok"] != true {
                self.failure = Some(
                    result["error"]
                        .as_str()
                        .unwrap_or("Command failed")
                        .to_string(),
                );
            }
            if result["ok"] != true || self.hint.is_none() {
                event_loop.exit()
            }
        }
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Command(command) => {
                let result = self.dispatch(event_loop, command.request);
                let _ = command.reply.send(result);
            }
            Event::Quit => {
                self.clear();
                event_loop.exit();
            }
            Event::HideShortcut(id) => {
                if self.shortcut.as_ref().is_some_and(|s| s.matches(id)) {
                    self.clear();
                    if self.once {
                        event_loop.exit();
                    }
                }
            }
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.clear();
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let (Some(window), Some(presenter), Some(frame)) =
                    (&self.window, &mut self.presenter, &self.frame)
                {
                    if let Err(error) = presenter.present(
                        window,
                        frame.pixmap.data(),
                        frame.pixmap.width(),
                        frame.pixmap.height(),
                    ) {
                        eprintln!("ScreenCue: {error}");
                        self.clear();
                    }
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .expiry
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.clear();
            if self.once {
                event_loop.exit();
            }
        }
        if let Err(error) = self.update_tracking(event_loop) {
            self.tracking_note = Some(error);
            if let Some(tracker) = &mut self.tracking {
                tracker.next = Some(Instant::now() + Duration::from_millis(250));
            }
        }
        if self.animation.as_ref().is_some_and(|a| {
            a.next.is_some_and(|n| Instant::now() >= n)
                || a.next_color.is_some_and(|n| Instant::now() >= n)
        }) {
            let result = (|| -> Result<(), String> {
                let a = self.animation.as_mut().ok_or("Missing animation")?;
                let hint = self.render_hint.as_ref().ok_or("Missing hint")?;
                if a.next_color.is_some_and(|n| Instant::now() >= n) {
                    let mut retry = Duration::from_secs(1);
                    match sample_background(hint, a.monitor, self.frame.as_ref(), a.sample_index) {
                        Ok(pixels) => {
                            a.background[a.sample_index] = pixels;
                            let combined =
                                a.background.iter().flatten().copied().collect::<Vec<_>>();
                            a.color = color::choose_stable(&combined, a.color);
                            a.sample_index =
                                if self.frame.as_ref().is_some_and(|f| f.arrow_start.is_some()) {
                                    1 - a.sample_index
                                } else {
                                    0
                                };
                            self.color_source = "local_pixels".into();
                            self.color_note = None;
                        }
                        Err(error) if error.starts_with("Background sample pending") => {
                            retry = Duration::from_millis(250);
                        }
                        Err(error) => {
                            // Stop retrying a denied/unavailable capture path until the next show.
                            if self.color_source == "local_pixels" {
                                self.color_source = "local_pixels_stale".into();
                            }
                            self.color_note = Some(error);
                            a.next_color = None;
                        }
                    }
                    if a.next_color.is_some() {
                        a.next_color = Some(Instant::now() + retry);
                    }
                }
                let frame = self.renderer.render_at(
                    hint,
                    a.monitor,
                    a.scale,
                    a.color,
                    if hint.animate {
                        a.started.elapsed().as_secs_f32()
                    } else {
                        0.
                    },
                )?;
                let window = self.window.as_ref().ok_or("Missing window")?;
                if self.frame.as_ref().is_some_and(|old| {
                    old.x != frame.x
                        || old.y != frame.y
                        || old.pixmap.width() != frame.pixmap.width()
                        || old.pixmap.height() != frame.pixmap.height()
                }) {
                    platform::set_bounds(
                        window,
                        frame.x,
                        frame.y,
                        frame.pixmap.width(),
                        frame.pixmap.height(),
                        a.scale,
                    )?;
                }
                self.presenter
                    .as_mut()
                    .ok_or("Missing presenter")?
                    .present(
                        window,
                        frame.pixmap.data(),
                        frame.pixmap.width(),
                        frame.pixmap.height(),
                    )?;
                self.frame = Some(frame);
                a.next = hint
                    .animate
                    .then_some(Instant::now() + Duration::from_millis(84));
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("ScreenCue animation: {error}");
                self.clear();
                if self.once {
                    event_loop.exit();
                }
            }
        }
        let deadline = self
            .expiry
            .into_iter()
            .chain(self.animation.as_ref().and_then(|a| a.next))
            .chain(self.animation.as_ref().and_then(|a| a.next_color))
            .chain(self.tracking.as_ref().and_then(|t| t.next))
            .min();
        event_loop.set_control_flow(
            deadline
                .map(ControlFlow::WaitUntil)
                .unwrap_or(ControlFlow::Wait),
        );
    }
}

fn tracking_interval() -> Duration {
    Duration::from_millis(if cfg!(target_os = "windows") { 16 } else { 33 })
}
fn same_rect(a: Rect, b: Rect) -> bool {
    a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h
}
fn same_anchor(a: platform::AnchorState, b: platform::AnchorState) -> bool {
    a.x == b.x
        && a.y == b.y
        && a.width == b.width
        && a.height == b.height
        && a.scale == b.scale
        && a.visible == b.visible
        && a.destroyed == b.destroyed
}

fn color_hex(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

fn sample_background(
    hint: &Hint,
    monitor: Rect,
    frame: Option<&renderer::Frame>,
    sample_index: usize,
) -> Result<Vec<[u8; 3]>, String> {
    let (mut cx, mut cy) = hint.anchor();
    if sample_index == 1 {
        if let Some(((sx, sy), (tx, ty))) = frame.and_then(|f| f.arrow_start.zip(f.arrow_tip)) {
            cx = (sx + tx) * 0.5;
            cy = (sy + ty) * 0.5;
        }
    }
    let width = (monitor.w as u32).min(64);
    let height = (monitor.h as u32).min(64);
    let x = (cx as i32 - width as i32 / 2)
        .clamp(monitor.x as i32, monitor.right() as i32 - width as i32);
    let y = (cy as i32 - height as i32 / 2)
        .clamp(monitor.y as i32, monitor.bottom() as i32 - height as i32);
    let pixels = platform::sample(x, y, width, height)?;
    let pixels = if let Some(frame) = frame {
        // The shared union mask includes every arrow frame, so a delayed macOS
        // screenshot cannot leak an older phase into our background colors.
        // Target/card pixels retain their small dilation; the cue stays visible.
        pixels
            .into_iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let px = x + (i as u32 % width) as i32;
                let py = y + (i as u32 / width) as i32;
                (!frame.occludes_sample(px, py)).then_some(p)
            })
            .collect::<Vec<_>>()
    } else {
        pixels
    };
    if pixels.is_empty() {
        Err("No unobscured background pixels available".into())
    } else {
        Ok(pixels)
    }
}

fn parse_cli(args: &[String]) -> Result<Value, String> {
    if args.is_empty() || args[0] == "demo" {
        return Ok(json!({"command":"demo"}));
    }
    if args[0] == "displays" {
        return Ok(json!({"command":"displays"}));
    }
    if args[0] != "show" {
        return Err("Use --help for commands".into());
    }
    let mut obj = serde_json::Map::new();
    obj.insert("command".into(), json!("show"));
    let mut i = 1;
    while i < args.len() {
        let key = args[i]
            .strip_prefix("--")
            .ok_or("Expected --option value")?
            .replace('-', "_");
        let text = args.get(i + 1).ok_or("Missing option value")?;
        let value = match key.as_str() {
            "arrow_scale" => json!(text
                .parse::<f32>()
                .map_err(|_| "Expected arrow scale number")?),
            "x" | "y" | "width" | "height" | "duration_ms" | "step" => {
                json!(text.parse::<i64>().map_err(|_| "Expected integer")?)
            }
            "arrow" | "target" | "animate" | "follow_window" => {
                json!(text.parse::<bool>().map_err(|_| "Expected true or false")?)
            }
            "background_colors" => json!(text.split(',').map(str::trim).collect::<Vec<_>>()),
            "points" => {
                let mut points = Vec::new();
                for pair in text.split(';') {
                    let (x, y) = pair.split_once(',').ok_or("points must be x,y;x,y;x,y")?;
                    points.push(json!({"x":x.trim().parse::<i32>().map_err(|_|"Invalid point x")?,"y":y.trim().parse::<i32>().map_err(|_|"Invalid point y")?}));
                }
                json!(points)
            }
            "text" | "style" | "shape" | "color" => json!(text),
            _ => return Err(format!("Unknown option: {key}")),
        };
        if obj.insert(key, value).is_some() {
            return Err("Duplicate option".into());
        }
        i += 2;
    }
    Ok(Value::Object(obj))
}
fn main() {
    #[cfg(target_os = "windows")]
    attach_parent_console();
    if let Err(error) = run() {
        eprintln!("ScreenCue: {error}");
        std::process::exit(1)
    }
}

#[cfg(target_os = "windows")]
fn attach_parent_console() {
    // GUI subsystem prevents a console from flashing when an agent starts us.
    // Keep inherited MCP/JSON pipes intact; attach only for interactive CLI output.
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_UNKNOWN};
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
    };
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if output.is_null()
            || output == INVALID_HANDLE_VALUE
            || GetFileType(output) == FILE_TYPE_UNKNOWN
        {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut hide_shortcut = shortcut::DEFAULT.to_string();
    if let Some(i) = args.iter().position(|a| a == "--hide-shortcut") {
        hide_shortcut = args
            .get(i + 1)
            .ok_or("--hide-shortcut needs a combination, e.g. Ctrl+Alt+X")?
            .clone();
        shortcut::parse(&hide_shortcut)?;
        args.drain(i..=i + 1);
    }
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("ScreenCue — lightweight cross-platform desktop hints\n\n  screencue --mcp                MCP over stdin/stdout\n  screencue --json               JSON commands over stdin/stdout\n  screencue demo                 Demo until the hide shortcut\n  screencue displays             Physical monitor coordinates\n  screencue show --x 500 --y 300 --text \"Click here\"\n\nOptions: --shape ellipse|rectangle|rounded|polygon, --width, --height,\n--points \"x,y;x,y;x,y\", --arrow true|false, --target true|false,\n--arrow-scale 0.75..2.5, --animate true|false, --color auto|#RRGGBB,\n--background-colors \"#FFFFFF,#202124\", --duration-ms 0|500..300000,\n--step 1..99. Global: --hide-shortcut Ctrl+Alt+X (default; Option on Mac).\nCLI show persists by default until hidden; an explicit duration sets expiry.\nMCP/JSON keep one overlay per connection. Linux needs X11 + a compositor.\nAuto color checks a small background patch about once per second while visible.");
        println!("Window movement follows locally by default; --follow-window false fixes desktop coordinates.\nScrolling/rearranged controls need a fresh hint. Offscreen targets keep a labelled edge cue.");
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--render-preview") {
        let output = args.get(1).ok_or("--render-preview needs a path")?;
        let hint = Hint::parse(
            json!({"x":240,"y":80,"text":"Open settings","shape":"rounded","width":90,"height":30}),
        )?;
        let phase = args
            .get(2)
            .map(|s| s.parse::<f32>())
            .transpose()
            .map_err(|_| "Preview phase must be a number")?
            .unwrap_or(0.);
        let frame = renderer::Renderer::new()?.render_at(
            &hint,
            Rect {
                x: 0.,
                y: 0.,
                w: 500.,
                h: 350.,
            },
            1.,
            color::FALLBACK,
            phase,
        )?;
        frame.pixmap.save_png(output).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mcp = args.first().is_some_and(|a| a == "--mcp");
    let raw = args.first().is_some_and(|a| a == "--json");
    if (mcp || raw) && args.len() != 1 {
        return Err("--mcp and --json do not accept extra arguments".into());
    }
    let initial = if mcp || raw {
        None
    } else {
        Some(parse_cli(&args)?)
    };
    let mut builder = EventLoop::<Event>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false)
            .with_default_menu(false);
    }
    let event_loop = builder
        .build()
        .map_err(|e| format!("Cannot open the desktop (Linux requires X11): {e}"))?;
    let mut app = App::new(!mcp && !raw, initial)?;
    app.shortcut = Some(shortcut::Shortcut::new(hide_shortcut)?);
    let shortcut_proxy = event_loop.create_proxy();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let _ = shortcut_proxy.send_event(Event::HideShortcut(event.id));
        }
    }));
    if mcp || raw {
        let proxy = event_loop.create_proxy();
        thread::spawn(move || {
            let output = proxy.clone();
            let send = move |command| {
                output
                    .send_event(Event::Command(command))
                    .map_err(|_| "Desktop event loop stopped".to_string())
            };
            if mcp {
                protocol::serve(send)
            } else {
                serve_json(send)
            }
            let _ = proxy.send_event(Event::Quit);
        });
    }
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(error) = app.failure {
        return Err(error);
    }
    Ok(())
}
fn serve_json(send: impl Fn(protocol::Command) -> Result<(), String>) {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let mut bytes = Vec::new();
        let mut oversized = false;
        loop {
            let available = match input.fill_buf() {
                Ok(b) => b,
                Err(_) => return,
            };
            if available.is_empty() {
                return;
            }
            let n = available
                .iter()
                .position(|b| *b == b'\n')
                .map(|n| n + 1)
                .unwrap_or(available.len());
            let ends = available[n - 1] == b'\n';
            if bytes.len() + n > 32768 {
                oversized = true
            } else {
                bytes.extend_from_slice(&available[..n]);
            }
            input.consume(n);
            if ends || oversized {
                break;
            }
        }
        if oversized {
            let _ = writeln!(
                output,
                "{}",
                json!({"ok":false,"error":"Message exceeds 32768 bytes"})
            );
            let _ = output.flush();
            return;
        }
        let result = match serde_json::from_slice::<Value>(&bytes) {
            Ok(request) => {
                let (tx, rx) = mpsc::channel();
                match send(protocol::Command { request, reply: tx }) {
                    Ok(()) => rx
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap_or(json!({"ok":false,"error":"Desktop response timed out"})),
                    Err(e) => json!({"ok":false,"error":e}),
                }
            }
            Err(e) => json!({"ok":false,"error":e.to_string()}),
        };
        if writeln!(output, "{result}")
            .and_then(|_| output.flush())
            .is_err()
        {
            return;
        }
    }
}
