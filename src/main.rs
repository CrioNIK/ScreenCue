#![recursion_limit = "256"]
mod color;
mod model;
mod platform;
mod protocol;
mod renderer;

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
    dpi::{PhysicalPosition, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId, WindowLevel},
};

enum Event {
    Command(protocol::Command),
    Quit,
}
struct App {
    window: Option<Window>,
    presenter: Option<platform::Presenter>,
    renderer: renderer::Renderer,
    frame: Option<renderer::Frame>,
    hint: Option<Hint>,
    expiry: Option<Instant>,
    once: bool,
    initial: Option<Value>,
    failure: Option<String>,
    color_source: String,
}
impl App {
    fn new(once: bool, initial: Option<Value>) -> Result<Self, String> {
        Ok(Self {
            window: None,
            presenter: None,
            renderer: renderer::Renderer::new()?,
            frame: None,
            hint: None,
            expiry: None,
            once,
            initial,
            failure: None,
            color_source: String::new(),
        })
    }
    fn clear(&mut self) {
        if let Some(w) = &self.window {
            w.set_visible(false)
        }
        self.frame = None;
        self.hint = None;
        self.expiry = None;
    }
    fn dispatch(&mut self, event_loop: &ActiveEventLoop, request: Value) -> Value {
        match self.execute(event_loop, request) {
            Ok(value) => value,
            Err(error) => json!({"ok":false,"error":error}),
        }
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
                json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"visible":self.hint.is_some(),"hint":self.hint,"color_source":self.color_source,"pid":std::process::id()}),
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
                self.clear();
                let mut reason = None;
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
                    let (cx, cy) = hint.anchor();
                    let w = size.width.min(128);
                    let h = size.height.min(128);
                    let x = (cx as i32 - w as i32 / 2)
                        .clamp(pos.x, pos.x + size.width as i32 - w as i32);
                    let y = (cy as i32 - h as i32 / 2)
                        .clamp(pos.y, pos.y + size.height as i32 - h as i32);
                    match platform::sample(x, y, w, h) {
                        Ok(pixels) if !pixels.is_empty() => {
                            self.color_source = "local_pixels".into();
                            color::choose(&pixels)
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
                window.set_outer_position(PhysicalPosition::new(frame.x, frame.y));
                let _ = window.request_inner_size(PhysicalSize::new(
                    frame.pixmap.width(),
                    frame.pixmap.height(),
                ));
                self.presenter
                    .as_mut()
                    .ok_or("Presenter is not ready")?
                    .present(
                        window,
                        frame.pixmap.data(),
                        frame.pixmap.width(),
                        frame.pixmap.height(),
                    )?;
                window.set_visible(true);
                // One redraw after exposure; no continuous rendering loop.
                window.request_redraw();
                self.expiry = if hint.duration_ms > 0 {
                    Some(Instant::now() + Duration::from_millis(hint.duration_ms))
                } else {
                    None
                };
                let arrow_tip = frame.arrow_tip;
                self.frame = Some(frame);
                self.hint = Some(hint);
                Ok(
                    json!({"ok":true,"visible":true,"arrow_tip":arrow_tip,"color":format!("#{:02X}{:02X}{:02X}",selected[0],selected[1],selected[2]),"color_source":self.color_source,"color_note":reason}),
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
                    request = json!({"command":"show","x":p.x+s.width as i32/2,"y":p.y+s.height as i32/2,"text":"ScreenCue — one step at a time","duration_ms":10000});
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
        event_loop.set_control_flow(
            self.expiry
                .map(ControlFlow::WaitUntil)
                .unwrap_or(ControlFlow::Wait),
        );
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
            "x" | "y" | "width" | "height" | "duration_ms" | "step" => {
                json!(text.parse::<i64>().map_err(|_| "Expected integer")?)
            }
            "arrow" | "target" => {
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
    if let Err(error) = run() {
        eprintln!("ScreenCue: {error}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("ScreenCue — lightweight cross-platform desktop hints\n\n  screencue --mcp                MCP over stdin/stdout\n  screencue --json               JSON commands over stdin/stdout\n  screencue demo                 10-second demo\n  screencue displays             Physical monitor coordinates\n  screencue show --x 500 --y 300 --text \"Click here\"\n\nOptions: --shape ellipse|rectangle|rounded|polygon, --width, --height,\n--points \"x,y;x,y;x,y\", --arrow true|false, --target true|false,\n--color auto|#RRGGBB, --duration-ms 0|500..300000, --step 1..99.\nCLI show runs until expiry (0: until process exit). MCP/JSON keep one\noverlay per connection. Linux needs X11 + a compositor. No web server.");
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--render-preview") {
        let output = args.get(1).ok_or("--render-preview needs a path")?;
        let hint = Hint::parse(
            json!({"x":240,"y":80,"text":"Откройте настройки","shape":"rounded","width":90,"height":30}),
        )?;
        let frame = renderer::Renderer::new()?.render(
            &hint,
            Rect {
                x: 0.,
                y: 0.,
                w: 500.,
                h: 350.,
            },
            1.,
            [172, 77, 255],
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
