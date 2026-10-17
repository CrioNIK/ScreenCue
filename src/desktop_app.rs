//! Native settings and explicit conversation lifecycle.
use crate::{
    connections::{AuthMode, Connections, Lane, ProviderKind},
    credentials, listener, platform,
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
            credential_states: self.credential_states,
            credential_backend: credentials::backend_name().into(),
            credential_error: self.credential_error.clone(),
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
        self.refresh_credential_status();
        self.refresh_ui();
        Ok(())
    }
    pub(super) fn refresh_preferences(&mut self) {
        match Settings::load(&self.preferences_path) {
            Ok(value) if value != self.preferences => {
                let connections_changed = value.connections != self.preferences.connections;
                self.preferences = value;
                if connections_changed {
                    self.stop_conversation();
                    self.refresh_credential_status();
                }
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
        if let Some(cancelled) = &self.connection_cancel {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        self.listener_epoch = self.listener_epoch.wrapping_add(1);
        self.listener.take();
        self.conversation_state = "Stopped".into();
        if self.frame.is_none() {
            if let Some(shortcut) = &mut self.shortcut {
                shortcut.unregister();
            }
        }
    }
    fn save_connections(&mut self, connections: Connections) -> Result<(), String> {
        let mut settings = self.preferences.clone();
        settings.connections = connections;
        settings.save(&self.preferences_path)?;
        self.stop_conversation();
        self.preferences = settings;
        Ok(())
    }
    pub(super) fn refresh_credential_status(&mut self) {
        self.credential_epoch = self.credential_epoch.wrapping_add(1);
        self.credential_states = [false; 4];
        self.credential_error = None;
        if self.credential_check_busy || self.credential_busy {
            return;
        }
        if self.settings_window.is_none() {
            return;
        }
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
        let epoch = self.credential_epoch;
        self.credential_check_busy = true;
        let connections = self.preferences.connections.clone();
        let path = self.preferences_path.clone();
        std::thread::spawn(move || {
            let mut states = [false; 4];
            let mut error = None;
            for lane in Lane::ALL {
                if connections.get(lane).auth == AuthMode::Vault {
                    match credentials::exists(&connections.credential_account(&path, lane)) {
                        Ok(present) => states[lane as usize] = present,
                        Err(message) => {
                            error = Some(message);
                            break;
                        }
                    }
                }
            }
            let _ = proxy.send_event(Event::CredentialStatus(epoch, states, error));
        });
    }
    pub(super) fn credential_status_event(
        &mut self,
        epoch: u64,
        states: [bool; 4],
        error: Option<String>,
    ) {
        self.credential_check_busy = false;
        if epoch == self.credential_epoch {
            self.credential_states = states;
            self.credential_error = error;
            self.refresh_ui();
        } else if !self.credential_busy {
            // A stale reader must not advance the generation of a pending
            // write. The write completion schedules the next read instead.
            self.refresh_credential_status();
        }
    }
    pub(super) fn credential_changed_event(
        &mut self,
        epoch: u64,
        lane: Lane,
        result: Result<bool, String>,
    ) {
        self.credential_busy = false;
        let current = epoch == self.credential_epoch;
        match result {
            Ok(present) => {
                if current {
                    self.credential_error = None;
                    self.credential_states[lane as usize] = present;
                    self.notice = if present {
                        "Key saved in the operating system credential store."
                    } else {
                        "Saved key removed."
                    }
                    .into();
                } else {
                    // An external settings change can select a new destination
                    // during an OS prompt. Never assign the old key to that role.
                    self.notice = if present {
                        "Key saved for the previously selected connection."
                    } else {
                        "Key removed from the previously selected connection."
                    }
                    .into();
                }
            }
            Err(error) => self.notice = error,
        }
        // Refresh after the write has finished, including a deferred read or a
        // changed connection. This cannot invalidate an in-flight write now.
        self.refresh_credential_status();
        self.refresh_ui();
    }
    pub(super) fn conversation_ready(
        &mut self,
        epoch: u64,
        result: Result<(std::path::PathBuf, Vec<(String, String)>), String>,
    ) {
        self.connection_preparing = false;
        self.connection_cancel = None;
        self.check_dismiss();
        if epoch != self.listener_epoch || self.conversation_state != "Starting" {
            return;
        }
        let result = result.and_then(|(path, environment)| {
            let proxy = self
                .proxy
                .clone()
                .ok_or("Desktop event proxy is unavailable")?;
            listener::Listener::start_with_env(&path, environment, move |event| {
                let _ = proxy.send_event(Event::Conversation(epoch, event));
            })
        });
        match result {
            Ok(listener) => self.listener = Some(listener),
            Err(error) => {
                self.stop_conversation();
                self.conversation_state = "Error".into();
                self.notice = error;
            }
        }
        self.schedule_preferences();
        self.refresh_ui();
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
                if self.connection_preparing {
                    return Err("The credential store is still completing the previous Start request. Stop cancels launch; wait for the store to return before starting again.".into());
                }
                if self.credential_busy {
                    return Err("Wait for the key operation to finish before starting.".into());
                }
                self.stop_conversation();
                self.dismiss_token = self.read_dismiss();
                let proxy = self
                    .proxy
                    .clone()
                    .ok_or("Desktop event proxy is unavailable")?;
                let epoch = self.listener_epoch;
                if self.preferences.connections.enabled {
                    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    self.connection_cancel = Some(cancelled.clone());
                    self.connection_preparing = true;
                    let connections = self.preferences.connections.clone();
                    let path = self.preferences_path.clone();
                    std::thread::spawn(move || {
                        let result = prepare_connections(&connections, &path, &cancelled);
                        let _ = proxy.send_event(Event::ConversationReady(epoch, result));
                    });
                } else {
                    let process = listener::Listener::start(
                        Path::new(&self.preferences.audio_config),
                        move |event| {
                            let _ = proxy.send_event(Event::Conversation(epoch, event));
                        },
                    )?;
                    self.listener = Some(process);
                }
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
            Action::SaveConnections(connections) => {
                if self.credential_busy {
                    return Err(
                        "Wait for the key operation to finish before saving connections.".into(),
                    );
                }
                self.save_connections(connections)?;
                self.refresh_credential_status();
                self.notice = "Connections saved. Changes take effect on the next Start.".into();
            }
            Action::StoreConnectionKey {
                lane,
                connection,
                secret,
            } => {
                if self.credential_busy {
                    return Err("A key operation is already running.".into());
                }
                let mut connections = self.preferences.connections.clone();
                *connections.get_mut(lane) = connection;
                connections.get_mut(lane).auth = AuthMode::Vault;
                self.save_connections(connections)?;
                self.credential_epoch = self.credential_epoch.wrapping_add(1);
                let epoch = self.credential_epoch;
                let account = self
                    .preferences
                    .connections
                    .credential_account(&self.preferences_path, lane);
                let proxy = self
                    .proxy
                    .clone()
                    .ok_or("Desktop event proxy is unavailable")?;
                self.credential_busy = true;
                self.notice = "Saving key in the operating system credential store…".into();
                std::thread::spawn(move || {
                    let result = credentials::set(&account, &secret).map(|_| true);
                    let _ = proxy.send_event(Event::CredentialChanged(epoch, lane, result));
                });
            }
            Action::DeleteConnectionKey(lane) => {
                if self.credential_busy {
                    return Err("A key operation is already running.".into());
                }
                self.stop_conversation();
                self.credential_epoch = self.credential_epoch.wrapping_add(1);
                let epoch = self.credential_epoch;
                let account = self
                    .preferences
                    .connections
                    .credential_account(&self.preferences_path, lane);
                let proxy = self
                    .proxy
                    .clone()
                    .ok_or("Desktop event proxy is unavailable")?;
                self.credential_busy = true;
                self.notice = "Removing saved key…".into();
                std::thread::spawn(move || {
                    let result = credentials::delete(&account).map(|_| false);
                    let _ = proxy.send_event(Event::CredentialChanged(epoch, lane, result));
                });
            }
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
        if self.frame.is_some() || self.listener.is_some() || self.conversation_state == "Starting"
        {
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

fn prepare_connections(
    connections: &Connections,
    path: &Path,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<(std::path::PathBuf, Vec<(String, String)>), String> {
    let check_cancelled = || {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            Err("Connection start was cancelled".to_string())
        } else {
            Ok(())
        }
    };
    check_cancelled()?;
    let config = connections.write_runtime_config(path)?;
    let mut environment = Vec::new();
    for lane in Lane::ALL {
        check_cancelled()?;
        if !connections.lane_active(lane) {
            continue;
        }
        let connection = connections.get(lane);
        if connection.kind == ProviderKind::Disabled {
            continue;
        }
        match connection.auth {
            AuthMode::None => {}
            AuthMode::Environment => {
                if std::env::var(&connection.key_env)
                    .ok()
                    .filter(|key| !key.trim().is_empty())
                    .is_none()
                {
                    return Err(format!("The {} environment credential is unavailable. Set it before opening ScreenCue or save a key here.", lane.label()));
                }
            }
            AuthMode::Vault => {
                let key = credentials::get(&connections.credential_account(path, lane))?
                    .ok_or_else(|| {
                        format!("Save an API key for {} before starting.", lane.label())
                    })?;
                environment.push((lane.env_name().into(), key));
            }
        }
    }
    check_cancelled()?;
    Ok((config, environment))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_credential_read_cannot_invalidate_a_pending_write() {
        let mut app = App::new(false, None).unwrap();
        app.credential_epoch = 2;
        app.credential_busy = true;
        app.credential_check_busy = true;
        app.notice = "Saving key…".into();

        app.credential_status_event(1, [false; 4], None);
        assert_eq!(app.credential_epoch, 2);
        assert!(app.credential_busy);
        assert!(!app.credential_check_busy);
        app.credential_changed_event(2, Lane::Fast, Ok(true));
        assert!(!app.credential_busy);
        assert_eq!(
            app.notice,
            "Key saved in the operating system credential store."
        );
        assert!(app.credential_epoch > 2);
    }

    #[test]
    fn credential_write_for_an_old_destination_does_not_mark_the_new_one_present() {
        let mut app = App::new(false, None).unwrap();
        app.credential_epoch = 2;
        app.credential_busy = true;
        app.credential_check_busy = true;
        app.preferences.connections.fast.endpoint = "https://new.example/api".into();
        // This is the same refresh used for externally changed preferences.
        app.refresh_credential_status();
        let changed_epoch = app.credential_epoch;
        app.credential_status_event(1, [true; 4], None);
        assert_eq!(app.credential_epoch, changed_epoch);
        assert_eq!(app.credential_states, [false; 4]);
        app.credential_changed_event(2, Lane::Fast, Ok(true));
        assert!(!app.credential_busy);
        assert_eq!(app.credential_states, [false; 4]);
        assert_eq!(
            app.notice,
            "Key saved for the previously selected connection."
        );
        assert!(app.credential_epoch > changed_epoch);
    }

    #[test]
    fn a_stopped_or_superseded_connection_preparation_cannot_launch_a_helper() {
        let mut app = App::new(false, None).unwrap();
        app.preferences_path = std::env::temp_dir().join(format!(
            "screencue-no-launch-{}-settings.json",
            std::process::id()
        ));
        app.conversation_state = "Starting".into();
        let epoch = app.listener_epoch;
        app.stop_conversation();
        app.conversation_ready(
            epoch,
            Ok((
                "this-helper-must-never-launch".into(),
                vec![("SYNTHETIC_KEY".into(), "fixture-value".into())],
            )),
        );
        assert!(app.listener.is_none());
        assert_eq!(app.conversation_state, "Stopped");
        assert!(app.notice.is_empty());
        app.conversation_ready(
            app.listener_epoch,
            Err("This stale error must stay hidden".into()),
        );
        assert_eq!(app.conversation_state, "Stopped");
        assert!(app.notice.is_empty());
    }
}
