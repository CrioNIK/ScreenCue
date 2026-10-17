//! Native settings and explicit conversation lifecycle.
use crate::{
    listener, platform,
    settings::{Mode, Settings},
    settings_ui::{Action, SettingsWindow, UiStatus},
    App, Event,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use winit::{event_loop::ActiveEventLoop, window::Icon};

fn icon() -> Option<Icon> {
    let pixmap =
        tiny_skia::Pixmap::decode_png(include_bytes!("../assets/app-icon-256.png")).ok()?;
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let p = p.demultiply();
            [p.red(), p.green(), p.blue(), p.alpha()]
        })
        .collect();
    Icon::from_rgba(rgba, pixmap.width(), pixmap.height()).ok()
}

impl App {
    pub(super) fn ui_status(&self) -> UiStatus {
        let failed = self.capture_privacy.requested && !self.capture_privacy.applied
            || self
                .settings_privacy
                .as_ref()
                .is_some_and(|s| s.requested && !s.applied);
        let note = if !self.capture_privacy.supported {
            "This system cannot exclude ScreenCue from arbitrary screen captures."
        } else if failed {
            "Exclusion failed. Hints may be visible in screen sharing."
        } else if self.capture_privacy.applied {
            "Exclusion is enabled. Verify the receiver's view in your sharing app."
        } else {
            "Windows capture exclusion is available. Verify your sharing app before presenting."
        }
        .to_string();
        UiStatus {
            capture_supported: self.capture_privacy.supported,
            capture_note: note,
            overlay_visible: self.frame.is_some(),
            conversation_state: self.conversation_state.clone(),
            notice: self.notice.clone(),
            shortcut: self
                .shortcut
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| crate::shortcut::DEFAULT.into()),
        }
    }
    pub(super) fn refresh_ui(&mut self) {
        let status = self.ui_status();
        if let Some(window) = &mut self.settings_window {
            window.update(self.preferences.clone(), status);
        }
    }
    pub(super) fn open_settings(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = SettingsWindow::new(
            event_loop,
            self.preferences.clone(),
            self.ui_status(),
            icon(),
        )?;
        prepare_settings_capture(window.window());
        self.settings_privacy = Some(platform::set_capture_exclusion(
            window.window(),
            self.preferences.hide_from_capture,
        ));
        show_settings_window(window.window());
        window.window().request_redraw();
        self.settings_window = Some(window);
        self.refresh_ui();
        Ok(())
    }
    pub(super) fn refresh_preferences(&mut self) {
        match Settings::load(&self.preferences_path) {
            Ok(value) if value != self.preferences => {
                self.preferences = value;
                self.apply_privacy();
                self.refresh_ui();
            }
            Err(error) => {
                self.notice = error;
                self.refresh_ui();
            }
            _ => {}
        }
    }
    fn apply_privacy(&mut self) {
        if let Some(window) = &self.window {
            self.capture_privacy =
                platform::set_capture_exclusion(window, self.preferences.hide_from_capture);
        }
        if let Some(window) = &self.settings_window {
            self.settings_privacy = Some(platform::set_capture_exclusion(
                window.window(),
                self.preferences.hide_from_capture,
            ));
        }
    }
    pub(super) fn stop_conversation(&mut self) {
        self.listener_epoch = self.listener_epoch.wrapping_add(1);
        self.listener.take();
        self.conversation_state = "Stopped".into();
        if self.frame.is_none() {
            if let Some(shortcut) = &mut self.shortcut {
                shortcut.unregister();
            }
        }
    }
    pub(super) fn settings_action(
        &mut self,
        event_loop: &ActiveEventLoop,
        action: Action,
    ) -> Result<(), String> {
        match action {
            Action::Changed(value) => {
                value.save(&self.preferences_path)?;
                self.preferences = value;
                self.apply_privacy();
                if let Some(hint) = self.hint.clone() {
                    let mut request = serde_json::to_value(hint).map_err(|e| e.to_string())?;
                    request["command"] = json!("show");
                    request["panel_width"] = json!(
                        (self.preferences.panel_width as f32
                            * self.animation.as_ref().map(|a| a.scale).unwrap_or(1.))
                            as u32
                    );
                    request["animate"] = json!(self.preferences.animate);
                    if self.preferences.mode == Mode::Conversation {
                        request["arrow"] = json!(false);
                        request["target"] = json!(false);
                        request["animate"] = json!(false);
                        request["follow_window"] = json!(false);
                    }
                    self.execute(event_loop, request)?;
                }
            }
            Action::ShowPreview { title, text } => {
                self.execute(
                    event_loop,
                    json!({"command":"prompt","title":title,"text":text}),
                )?;
            }
            Action::HideAll => {
                self.broadcast_dismiss();
                self.stop_conversation();
                self.clear();
            }
            Action::StartConversation => {
                self.stop_conversation();
                self.dismiss_token = self.read_dismiss();
                let proxy = self
                    .proxy
                    .clone()
                    .ok_or("Desktop event proxy is unavailable")?;
                let epoch = self.listener_epoch;
                let process = listener::Listener::start(
                    Path::new(&self.preferences.audio_config),
                    move |event| {
                        let _ = proxy.send_event(Event::Conversation(epoch, event));
                    },
                )?;
                self.listener = Some(process);
                self.conversation_state = "Starting".into();
                self.notice.clear();
                if let Some(shortcut) = &mut self.shortcut {
                    shortcut.register();
                }
            }
            Action::StopConversation => {
                self.stop_conversation();
                self.clear();
            }
            Action::CreateAudioConfig => {
                let parent = self
                    .preferences_path
                    .parent()
                    .ok_or("Settings directory is unavailable")?;
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                let path = parent.join("conversation.json");
                listener::write_example(&path)?;
                let mut settings = self.preferences.clone();
                settings.audio_config = path.to_string_lossy().into();
                settings.save(&self.preferences_path)?;
                self.preferences = settings;
                self.notice="Example created. Configure transcription and suggestion providers before Start.".into();
            }
            Action::OpenAudioConfig => open_config(Path::new(&self.preferences.audio_config))?,
            Action::Close => {
                self.stop_conversation();
                self.clear();
                event_loop.exit();
            }
        }
        self.schedule_preferences();
        self.refresh_ui();
        Ok(())
    }
    pub(super) fn conversation_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        epoch: u64,
        event: Value,
    ) {
        self.check_dismiss();
        if epoch != self.listener_epoch || self.listener.is_none() {
            return;
        }
        match event["event"].as_str() {
            Some("hint") => {
                let mut request = json!({"command":"prompt","title":event["title"].as_str().unwrap_or("Suggested words"),"text":event["text"]});
                if let Some(ttl) = event["ttl_ms"].as_u64() {
                    request["duration_ms"] = json!(ttl.clamp(500, 300000));
                }
                let result = self.dispatch(event_loop, request);
                if result["ok"] != true {
                    self.notice = result["error"]
                        .as_str()
                        .unwrap_or("Cannot show conversation hint")
                        .into();
                }
            }
            Some("clear") => self.clear(),
            Some("screen_hint") => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let fresh = event["captured_at_ms"].as_u64().is_some_and(|at| {
                    at <= now && now - at <= event["max_age_ms"].as_u64().unwrap_or(2000).min(5000)
                });
                if fresh
                    && event["observation_id"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty())
                    && event["hint"].is_object()
                {
                    let mut request = event["hint"].clone();
                    request["command"] = json!("show");
                    let result = self.dispatch(event_loop, request);
                    if result["ok"] != true {
                        self.notice = result["error"]
                            .as_str()
                            .unwrap_or("Invalid screen hint")
                            .into();
                    }
                } else {
                    self.notice =
                        "Screen hint discarded: a fresh identified screen observation is required."
                            .into();
                }
            }
            Some("status") => {
                self.conversation_state = match event["state"].as_str() {
                    Some("listening") => "Listening",
                    Some("ready") => "Ready",
                    Some("stopped") => "Stopped",
                    _ => "Starting",
                }
                .into();
            }
            Some("error") => {
                self.notice = event["message"]
                    .as_str()
                    .unwrap_or("Conversation helper error")
                    .into();
                self.conversation_state = "Error".into();
            }
            Some("exit") => {
                let failed = self.conversation_state == "Error";
                self.stop_conversation();
                if failed {
                    self.conversation_state = "Error".into();
                }
                if self.frame.is_none() {
                    if let Some(s) = &mut self.shortcut {
                        s.unregister();
                    }
                }
            }
            _ => return,
        }
        self.refresh_ui();
    }
    pub(super) fn prepare_request(
        &self,
        event_loop: &ActiveEventLoop,
        mut request: Value,
    ) -> Result<Value, String> {
        let prompt = request["command"] == "prompt";
        if prompt {
            let object = request.as_object().ok_or("Expected an object")?;
            if object.keys().any(|key| {
                ![
                    "command",
                    "text",
                    "title",
                    "text_page",
                    "panel_width",
                    "panel_x",
                    "panel_y",
                    "duration_ms",
                ]
                .contains(&key.as_str())
            }) {
                return Err("Unknown prompt field".into());
            }
            if !request["text"].is_string() {
                return Err("Prompt text is required".into());
            }
            let monitor = event_loop
                .primary_monitor()
                .or_else(|| event_loop.available_monitors().next())
                .ok_or("No connected display")?;
            let pos = monitor.position();
            let size = monitor.size();
            let scale = monitor.scale_factor();
            request["x"] = json!(pos.x + size.width as i32 / 2);
            request["y"] = json!(pos.y + size.height as i32 / 2);
            let width = (self.preferences.panel_width as f64 * scale) as u32;
            if request.get("panel_width").is_none() {
                request["panel_width"] = json!(width);
            }
            if request.get("panel_x").is_none() && request.get("panel_y").is_none() {
                request["panel_x"] =
                    json!(pos.x + (size.width as i32 - width as i32 - (28. * scale) as i32).max(0));
                request["panel_y"] = json!(pos.y + (48. * scale) as i32);
            }
            request["command"] = json!("show");
        }
        if prompt
            || (self.preferences.mode == Mode::Conversation
                && request.get("arrow").is_none()
                && request.get("target").is_none())
        {
            request["arrow"] = json!(false);
            request["target"] = json!(false);
            request["follow_window"] = json!(false);
            request["animate"] = json!(false);
            request["color"] = json!("#EF232D");
        }
        if request.get("animate").is_none() {
            request["animate"] = json!(self.preferences.animate);
        }
        Ok(request)
    }
    pub(super) fn schedule_preferences(&mut self) {
        if self.frame.is_some() || self.listener.is_some() {
            if self.preferences_check.is_none() {
                self.preferences_check = Some(Instant::now() + Duration::from_millis(250));
            }
        } else {
            self.preferences_check = None;
        }
    }
    pub(super) fn read_dismiss(&self) -> String {
        use std::io::Read;
        let mut token = String::new();
        if let Ok(file) = std::fs::File::open(self.preferences_path.with_extension("dismiss")) {
            let _ = file.take(128).read_to_string(&mut token);
        }
        token
    }
    pub(super) fn broadcast_dismiss(&mut self) {
        self.dismiss_token = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        if let Some(parent) = self.preferences_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(
            self.preferences_path.with_extension("dismiss"),
            &self.dismiss_token,
        )
        .is_err()
        {
            self.notice = "This cue was hidden; other connections could not be notified.".into();
        }
    }
    pub(super) fn check_dismiss(&mut self) {
        let token = self.read_dismiss();
        if !token.is_empty() && token != self.dismiss_token {
            self.dismiss_token = token;
            self.stop_conversation();
            self.clear();
            self.refresh_ui();
        }
    }
}

fn open_config(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("Create or choose a conversation configuration first.".into());
    }
    #[cfg(windows)]
    let mut command = Command::new("notepad.exe");
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("open");
        c.arg("-t");
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .arg(path)
        .spawn()
        .map_err(|e| format!("Cannot open configuration editor: {e}"))?;
    Ok(())
}
fn prepare_settings_capture(window: &winit::window::Window) {
    #[cfg(windows)]
    unsafe {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                let hwnd = handle.hwnd.get() as *mut _;
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

fn show_settings_window(window: &winit::window::Window) {
    // Winit rebuilds extended styles in set_visible and would discard the
    // opaque layered style needed for affinity readback. Map natively only
    // after the capture policy is applied, without changing those styles.
    #[cfg(windows)]
    unsafe {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOW};
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                ShowWindow(handle.hwnd.get() as *mut _, SW_SHOW);
            }
        }
    }
    #[cfg(not(windows))]
    window.set_visible(true);
}
