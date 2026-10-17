//! An interactive, opaque settings window. The hint window remains click-through.
//! Pixels are redrawn only after an input or state change; there is no UI animation timer.

use crate::{
    connections::{AuthMode, Connection, Connections, Lane, ProviderKind},
    settings::{Mode, Settings},
};
use fontdue::{
    layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
    Font, FontSettings,
};
use std::{num::NonZeroU32, rc::Rc};
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, Ime, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Icon, Window, WindowId},
};

const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 730.0;
const RED: [u8; 3] = [255, 57, 70];
const WHITE: [u8; 3] = [243, 241, 238];
const MUTED: [u8; 3] = [155, 157, 164];
const SAMPLE: &str =
    "Lead with the outcome. Explain the problem, the progress so far, and what comes next.";

#[derive(Clone, Debug, PartialEq)]
pub struct UiStatus {
    pub capture_supported: bool,
    pub capture_note: String,
    pub overlay_visible: bool,
    pub conversation_state: String,
    pub notice: String,
    pub shortcut: String,
    pub credential_states: [bool; 4],
    pub credential_backend: String,
    pub credential_error: Option<String>,
}

impl Default for UiStatus {
    fn default() -> Self {
        Self {
            capture_supported: false,
            capture_note: "Screen-share exclusion is unavailable on this platform.".into(),
            overlay_visible: false,
            conversation_state: "Stopped".into(),
            notice: String::new(),
            shortcut: "Ctrl + Alt + X".into(),
            credential_states: [false; 4],
            credential_backend: "System credential store".into(),
            credential_error: None,
        }
    }
}

// Actions can carry a newly entered key. Never derive Debug or serialize them.
#[derive(Clone)]
pub enum Action {
    Changed(Settings),
    ShowPreview {
        title: String,
        text: String,
    },
    HideAll,
    StartConversation,
    StopConversation,
    CreateAudioConfig,
    OpenAudioConfig,
    SaveConnections(Connections),
    StoreConnectionKey {
        lane: Lane,
        connection: Connection,
        secret: String,
    },
    DeleteConnectionKey(Lane),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Appearance,
    Conversation,
    Connections,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionsPage {
    Models,
    Runtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Appearance,
    Conversation,
    Connections,
    ConnectionModels,
    ConnectionRuntime,
    Lane(Lane),
    Provider,
    Endpoint,
    Model,
    Bridge,
    Auth,
    Key,
    KeyEnvironment,
    StoreKey,
    DeleteKey,
    SaveConnections,
    ConnectionsEnabled,
    Microphone,
    SystemAudio,
    Diarization,
    Python,
    Guidance,
    ConversationMode,
    Capture,
    Animation,
    Width,
    Narrower,
    Wider,
    Prompt,
    Show,
    Hide,
    Config,
    CreateConfig,
    OpenConfig,
    Start,
    Stop,
}

#[derive(Clone, Copy)]
struct Box2 {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}
impl Box2 {
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
fn area(x: f32, y: f32, w: f32, h: f32) -> Box2 {
    Box2 { x, y, w, h }
}

#[derive(Clone)]
struct Editor {
    value: String,
    cursor: usize,
    all: bool,
    limit: usize,
    multiline: bool,
}
impl Editor {
    fn new(value: String, limit: usize, multiline: bool) -> Self {
        let cursor = value.len();
        Self {
            value,
            cursor,
            all: false,
            limit,
            multiline,
        }
    }
    fn insert(&mut self, text: &str) -> bool {
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || (*c == '\n' && self.multiline))
            .collect();
        if text.is_empty() {
            return true;
        }
        let current = if self.all {
            0
        } else {
            self.value.chars().count()
        };
        // A paste is either accepted in full or rejected, never silently truncated.
        if current + text.chars().count() > self.limit {
            return false;
        }
        if self.all {
            self.value.clear();
            self.cursor = 0;
            self.all = false;
        }
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
        true
    }
    fn backspace(&mut self) {
        if self.all {
            self.value.clear();
            self.cursor = 0;
            self.all = false;
        } else if self.cursor > 0 {
            let previous = self.value[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.value.replace_range(previous..self.cursor, "");
            self.cursor = previous;
        }
    }
    fn delete(&mut self) {
        if self.all {
            self.backspace();
        } else if self.cursor < self.value.len() {
            let next = self.cursor + self.value[self.cursor..].chars().next().unwrap().len_utf8();
            self.value.replace_range(self.cursor..next, "");
        }
    }
    fn left(&mut self) {
        self.all = false;
        self.cursor = self.value[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }
    fn right(&mut self) {
        self.all = false;
        if let Some(c) = self.value[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }
}

struct View {
    settings: Settings,
    status: UiStatus,
    tab: Tab,
    focus: Option<Control>,
    hover: Option<Control>,
    prompt: Editor,
    config: Editor,
    connections: Connections,
    connections_page: ConnectionsPage,
    lane: Lane,
    endpoint: Editor,
    model: Editor,
    bridge: Editor,
    key: Editor,
    key_environment: Editor,
    python: Editor,
    modifiers: ModifiersState,
}

impl View {
    fn new(settings: Settings, status: UiStatus) -> Self {
        let config = Editor::new(settings.audio_config.clone(), 4096, false);
        let connections = settings.connections.clone();
        let mut view = Self {
            settings,
            status,
            tab: Tab::Appearance,
            focus: None,
            hover: None,
            prompt: Editor::new(SAMPLE.into(), 16000, true),
            config,
            connections,
            connections_page: ConnectionsPage::Models,
            lane: Lane::Transcription,
            endpoint: Editor::new(String::new(), 2048, false),
            model: Editor::new(String::new(), 200, false),
            bridge: Editor::new(String::new(), 8192, true),
            key: Editor::new(String::new(), 8192, false),
            key_environment: Editor::new(String::new(), 128, false),
            python: Editor::new(String::new(), 4096, false),
            modifiers: ModifiersState::empty(),
        };
        view.load_connection_editors();
        view.python = Editor::new(view.connections.python_executable.clone(), 4096, false);
        view
    }
    fn controls(&self) -> Vec<(Control, Box2)> {
        let mut controls = vec![
            (Control::Appearance, area(18., 153., 140., 42.)),
            (Control::Conversation, area(18., 202., 140., 42.)),
            (Control::Connections, area(18., 251., 140., 42.)),
        ];
        match self.tab {
            Tab::Appearance => {
                controls.extend([
                    (Control::Guidance, area(194., 155., 258., 76.)),
                    (Control::ConversationMode, area(466., 155., 258., 76.)),
                    (Control::Animation, area(194., 330., 530., 56.)),
                    (Control::Width, area(194., 440., 422., 25.)),
                    (Control::Narrower, area(632., 431., 40., 40.)),
                    (Control::Wider, area(684., 431., 40., 40.)),
                    (Control::Prompt, area(194., 514., 530., 91.)),
                    (Control::Show, area(194., 627., 258., 44.)),
                    (Control::Hide, area(466., 627., 258., 44.)),
                ]);
                if self.status.capture_supported {
                    controls.insert(4, (Control::Capture, area(194., 252., 530., 68.)));
                }
            }
            Tab::Conversation => controls.extend([
                (Control::Config, area(194., 347., 530., 54.)),
                (Control::CreateConfig, area(194., 417., 258., 42.)),
                (Control::OpenConfig, area(466., 417., 258., 42.)),
                (Control::Start, area(194., 627., 258., 44.)),
                (Control::Stop, area(466., 627., 258., 44.)),
            ]),
            Tab::Connections => {
                controls.extend([
                    (Control::ConnectionModels, area(194., 153., 258., 40.)),
                    (Control::ConnectionRuntime, area(466., 153., 258., 40.)),
                ]);
                match self.connections_page {
                    ConnectionsPage::Models => {
                        for (index, lane) in Lane::ALL.into_iter().enumerate() {
                            controls.push((
                                Control::Lane(lane),
                                area(194. + index as f32 * 136., 207., 122., 39.),
                            ));
                        }
                        controls.push((Control::Provider, area(325., 266., 399., 40.)));
                        let connection = self.connections.get(self.lane);
                        if connection.kind != ProviderKind::Disabled {
                            if connection.kind == ProviderKind::Command {
                                controls.push((Control::Bridge, area(194., 338., 530., 105.)));
                            } else {
                                controls.extend([
                                    (Control::Endpoint, area(194., 338., 530., 42.)),
                                    (Control::Model, area(194., 405., 530., 42.)),
                                ]);
                            }
                            controls.push((Control::Auth, area(325., 460., 399., 40.)));
                            match connection.auth {
                                AuthMode::Vault => controls.extend([
                                    (Control::Key, area(194., 532., 530., 42.)),
                                    (Control::StoreKey, area(194., 583., 258., 32.)),
                                    (Control::DeleteKey, area(466., 583., 258., 32.)),
                                ]),
                                AuthMode::Environment => controls
                                    .push((Control::KeyEnvironment, area(194., 532., 530., 42.))),
                                AuthMode::None => {}
                            }
                        }
                    }
                    ConnectionsPage::Runtime => controls.extend([
                        (Control::ConnectionsEnabled, area(194., 215., 530., 61.)),
                        (Control::Microphone, area(194., 292., 530., 46.)),
                        (Control::SystemAudio, area(194., 348., 530., 46.)),
                        (Control::Diarization, area(194., 404., 530., 48.)),
                        (Control::Python, area(194., 494., 530., 44.)),
                    ]),
                }
                controls.push((Control::SaveConnections, area(194., 627., 530., 44.)));
            }
        }
        controls
    }
    fn hit(&self, x: f32, y: f32) -> Option<Control> {
        self.controls()
            .into_iter()
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(control, _)| control)
    }
    fn load_connection_editors(&mut self) {
        self.clear_key();
        let connection = self.connections.get(self.lane);
        self.endpoint = Editor::new(connection.endpoint.clone(), 2048, false);
        self.model = Editor::new(connection.model.clone(), 200, false);
        self.bridge = Editor::new(
            serde_json::to_string(&connection.command).unwrap_or_else(|_| "[]".into()),
            8192,
            true,
        );
        self.key_environment = Editor::new(connection.key_env.clone(), 128, false);
    }
    fn flush_connection(&mut self) -> bool {
        let connection = self.connections.get_mut(self.lane);
        connection.endpoint = self.endpoint.value.clone();
        connection.model = self.model.value.clone();
        connection.key_env = self.key_environment.value.clone();
        if connection.kind == ProviderKind::Command {
            match serde_json::from_str::<Vec<String>>(&self.bridge.value) {
                Ok(command) => connection.command = command,
                Err(_) => {
                    self.status.notice = "Bridge must be a JSON array of strings, such as [\"/path/to/bridge\", \"--json\"].".into();
                    return false;
                }
            }
        }
        true
    }
    fn clear_key(&mut self) {
        self.key = Editor::new(String::new(), 8192, false);
    }
    fn may_copy(&self) -> bool {
        self.focus != Some(Control::Key)
    }
    fn has_unsaved_connections(&self) -> bool {
        self.connections != self.settings.connections
            || (self.connections.get(self.lane).kind == ProviderKind::Command
                && serde_json::from_str::<Vec<String>>(&self.bridge.value)
                    .map(|command| command != self.connections.get(self.lane).command)
                    .unwrap_or(true))
    }
    fn activate(&mut self, control: Control, x: Option<f32>) -> Vec<Action> {
        let mut changed = false;
        match control {
            Control::Appearance => self.tab = Tab::Appearance,
            Control::Conversation => self.tab = Tab::Conversation,
            Control::Connections => self.tab = Tab::Connections,
            Control::ConnectionModels => self.connections_page = ConnectionsPage::Models,
            Control::ConnectionRuntime => self.connections_page = ConnectionsPage::Runtime,
            Control::Lane(lane) => {
                if !self.flush_connection() {
                    return vec![];
                }
                if self.lane != lane {
                    self.clear_key();
                    self.lane = lane;
                    self.load_connection_editors();
                }
            }
            Control::Provider => {
                // Changing kind is also the escape hatch from an invalid bridge draft.
                let choices = providers(self.lane);
                let connection = self.connections.get_mut(self.lane);
                let index = choices
                    .iter()
                    .position(|kind| *kind == connection.kind)
                    .unwrap_or(0);
                connection.kind = choices[(index + 1) % choices.len()];
                self.clear_key();
            }
            Control::Auth => {
                let connection = self.connections.get_mut(self.lane);
                connection.auth = match connection.auth {
                    AuthMode::None => AuthMode::Vault,
                    AuthMode::Vault => AuthMode::Environment,
                    AuthMode::Environment => AuthMode::None,
                };
                if connection.auth == AuthMode::Environment && connection.key_env.is_empty() {
                    connection.key_env = self.lane.env_name().into();
                    self.key_environment = Editor::new(connection.key_env.clone(), 128, false);
                }
                self.clear_key();
            }
            Control::Endpoint
            | Control::Model
            | Control::Bridge
            | Control::Key
            | Control::KeyEnvironment
            | Control::Python => {
                self.focus = Some(control);
                if let Some(editor) = self.editor() {
                    editor.cursor = editor.value.len();
                    editor.all = false;
                }
            }
            Control::SaveConnections => {
                if self.flush_connection() {
                    return vec![Action::SaveConnections(self.connections.clone())];
                }
            }
            Control::StoreKey => {
                if self.key.value.is_empty() {
                    self.status.notice =
                        "Paste a key before saving it to the credential store.".into();
                } else if self.key.value.len() > 2560 || self.key.value.trim() != self.key.value {
                    self.status.notice = "Keys must fit 2560 UTF-8 bytes without surrounding spaces. The draft was kept for editing.".into();
                } else if self.flush_connection() {
                    let secret = std::mem::take(&mut self.key.value);
                    self.clear_key();
                    return vec![Action::StoreConnectionKey {
                        lane: self.lane,
                        connection: self.connections.get(self.lane).clone(),
                        secret,
                    }];
                }
            }
            Control::DeleteKey => {
                self.clear_key();
                return vec![Action::DeleteConnectionKey(self.lane)];
            }
            Control::ConnectionsEnabled => self.connections.enabled = !self.connections.enabled,
            Control::Microphone => self.connections.microphone = !self.connections.microphone,
            Control::SystemAudio => self.connections.system_audio = !self.connections.system_audio,
            Control::Diarization => self.connections.diarization = !self.connections.diarization,
            Control::Guidance => {
                self.settings.mode = Mode::Guidance;
                changed = true;
            }
            Control::ConversationMode => {
                self.settings.mode = Mode::Conversation;
                changed = true;
            }
            Control::Capture if self.status.capture_supported => {
                self.settings.hide_from_capture = !self.settings.hide_from_capture;
                changed = true;
            }
            Control::Animation => {
                self.settings.animate = !self.settings.animate;
                changed = true;
            }
            Control::Width => {
                if let Some(x) = x {
                    self.settings.panel_width =
                        (260. + ((x - 194.) / 422.).clamp(0., 1.) * 540.).round() as u32;
                    changed = true;
                }
            }
            Control::Narrower => {
                self.settings.panel_width = self.settings.panel_width.saturating_sub(20).max(260);
                changed = true;
            }
            Control::Wider => {
                self.settings.panel_width = (self.settings.panel_width + 20).min(800);
                changed = true;
            }
            Control::Prompt => {
                self.prompt.cursor = self.prompt.value.len();
                self.prompt.all = false;
            }
            Control::Config => {
                self.config.cursor = self.config.value.len();
                self.config.all = false;
            }
            Control::Show => {
                return vec![Action::ShowPreview {
                    title: if self.settings.mode == Mode::Conversation {
                        "Your next thought"
                    } else {
                        "A little direction"
                    }
                    .into(),
                    text: self.prompt.value.clone(),
                }]
            }
            Control::Hide => return vec![Action::HideAll],
            Control::Start => return vec![Action::StartConversation],
            Control::Stop => return vec![Action::StopConversation],
            Control::CreateConfig => return vec![Action::CreateAudioConfig],
            Control::OpenConfig => return vec![Action::OpenAudioConfig],
            _ => {}
        }
        self.focus = Some(control);
        if changed {
            vec![Action::Changed(self.settings.clone())]
        } else {
            vec![]
        }
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        match self.focus {
            Some(Control::Prompt) => Some(&mut self.prompt),
            Some(Control::Config) => Some(&mut self.config),
            Some(Control::Endpoint) => Some(&mut self.endpoint),
            Some(Control::Model) => Some(&mut self.model),
            Some(Control::Bridge) => Some(&mut self.bridge),
            Some(Control::Key) => Some(&mut self.key),
            Some(Control::KeyEnvironment) => Some(&mut self.key_environment),
            Some(Control::Python) => Some(&mut self.python),
            _ => None,
        }
    }
    fn after_edit(&mut self) -> Vec<Action> {
        match self.focus {
            Some(Control::Endpoint) => {
                self.connections.get_mut(self.lane).endpoint = self.endpoint.value.clone()
            }
            Some(Control::Model) => {
                self.connections.get_mut(self.lane).model = self.model.value.clone()
            }
            Some(Control::KeyEnvironment) => {
                self.connections.get_mut(self.lane).key_env = self.key_environment.value.clone()
            }
            Some(Control::Python) => self.connections.python_executable = self.python.value.clone(),
            Some(Control::Bridge) => {
                if let Ok(command) = serde_json::from_str::<Vec<String>>(&self.bridge.value) {
                    self.connections.get_mut(self.lane).command = command;
                }
            }
            _ => {}
        }
        if self.focus == Some(Control::Config) && self.settings.audio_config != self.config.value {
            self.settings.audio_config = self.config.value.clone();
            vec![Action::Changed(self.settings.clone())]
        } else {
            vec![]
        }
    }
    fn insert_text(&mut self, text: &str) -> Vec<Action> {
        let rejected = self
            .editor()
            .and_then(|editor| (!editor.insert(text)).then_some(editor.limit));
        if let Some(limit) = rejected {
            self.status.notice =
                format!("Text was not inserted: this field accepts at most {limit} characters.");
        }
        self.after_edit()
    }
    fn advance_focus(&mut self, backwards: bool) {
        let controls = self.controls();
        let count = controls.len();
        let index = self
            .focus
            .and_then(|f| controls.iter().position(|(c, _)| *c == f));
        let next = match (index, backwards) {
            (Some(i), true) => (i + count - 1) % count,
            (Some(i), false) => (i + 1) % count,
            (None, true) => count - 1,
            (None, false) => 0,
        };
        self.focus = Some(controls[next].0);
    }
}

fn providers(lane: Lane) -> &'static [ProviderKind] {
    match lane {
        Lane::Decider => &[
            ProviderKind::Disabled,
            ProviderKind::Typesafe,
            ProviderKind::OpenaiDecisions,
            ProviderKind::Ollama,
            ProviderKind::Command,
        ],
        _ => &[
            ProviderKind::Disabled,
            ProviderKind::OpenaiCompatible,
            ProviderKind::Command,
        ],
    }
}
fn provider_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Disabled => "Off  >",
        ProviderKind::OpenaiCompatible => "OpenAI-compatible API  >",
        ProviderKind::Typesafe => "Typesafe / Jev  >",
        ProviderKind::OpenaiDecisions => "OpenAI Decisions  >",
        ProviderKind::Ollama => "Ollama (local)  >",
        ProviderKind::Command => "Command bridge  >",
    }
}
fn lane_label(lane: Lane) -> &'static str {
    match lane {
        Lane::Transcription => "Audio",
        Lane::Fast => "Fast model",
        Lane::Planner => "Big model",
        Lane::Decider => "System 1",
    }
}
fn lane_index(lane: Lane) -> usize {
    Lane::ALL
        .iter()
        .position(|candidate| *candidate == lane)
        .unwrap_or(0)
}
fn masked_editor(editor: &Editor) -> Editor {
    let mask = '\u{2022}';
    Editor {
        value: std::iter::repeat_n(mask, editor.value.chars().count()).collect(),
        cursor: editor.value[..editor.cursor].chars().count() * mask.len_utf8(),
        all: editor.all,
        limit: editor.limit,
        multiline: false,
    }
}

pub struct SettingsWindow {
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    _context: softbuffer::Context<Rc<Window>>,
    window: Rc<Window>,
    fonts: Fonts,
    view: View,
    cursor: (f32, f32),
    clipboard: Option<arboard::Clipboard>,
}

impl SettingsWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        settings: Settings,
        status: UiStatus,
        icon: Option<Icon>,
    ) -> Result<Self, String> {
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("ScreenCue")
                        .with_visible(false)
                        .with_inner_size(LogicalSize::new(WIDTH as f64, HEIGHT as f64))
                        .with_min_inner_size(LogicalSize::new(570., 548.))
                        .with_window_icon(icon),
                )
                .map_err(|e| e.to_string())?,
        );
        window.set_ime_allowed(true);
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface =
            softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        Ok(Self {
            surface,
            _context: context,
            window,
            fonts: Fonts::new()?,
            view: View::new(settings, status),
            cursor: (0., 0.),
            clipboard: None,
        })
    }
    pub fn window(&self) -> &Window {
        &self.window
    }
    pub fn id(&self) -> WindowId {
        self.window.id()
    }
    pub fn update(&mut self, settings: Settings, status: UiStatus) {
        if self.view.settings == settings && self.view.status == status {
            return;
        }
        if self.view.config.value != settings.audio_config {
            self.view.config = Editor::new(settings.audio_config.clone(), 4096, false);
        }
        if settings.connections != self.view.settings.connections
            && (!self.view.has_unsaved_connections()
                || (self.view.connections == settings.connections
                    && (self.view.connections.get(self.view.lane).kind != ProviderKind::Command
                        || serde_json::from_str::<Vec<String>>(&self.view.bridge.value).is_ok())))
        {
            self.view.connections = settings.connections.clone();
            self.view.load_connection_editors();
            self.view.python =
                Editor::new(settings.connections.python_executable.clone(), 4096, false);
        }
        self.view.settings = settings;
        self.view.status = status;
        self.window.request_redraw();
    }
    pub fn handle_event(&mut self, event: &WindowEvent) -> Result<Vec<Action>, String> {
        let mut actions = vec![];
        let mut redraw = true;
        match event {
            WindowEvent::CloseRequested => actions.push(Action::Close),
            WindowEvent::RedrawRequested => {
                self.redraw()?;
                redraw = false;
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {}
            WindowEvent::CursorMoved { position, .. } => {
                let size = self.window.inner_size();
                let (scale, ox, oy) = viewport(size.width, size.height);
                self.cursor = (
                    (position.x as f32 - ox) / scale,
                    (position.y as f32 - oy) / scale,
                );
                let hover = self.view.hit(self.cursor.0, self.cursor.1);
                redraw = hover != self.view.hover;
                self.view.hover = hover;
            }
            WindowEvent::CursorLeft { .. } => self.view.hover = None,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(control) = self.view.hit(self.cursor.0, self.cursor.1) {
                    actions = self.view.activate(control, Some(self.cursor.0));
                } else {
                    self.view.focus = None;
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.view.modifiers = modifiers.state();
                redraw = false;
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                actions = self.view.insert_text(text);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command = self.view.modifiers.control_key() || self.view.modifiers.super_key();
                if command {
                    if let Key::Character(key) = &event.logical_key {
                        match key.to_ascii_lowercase().as_str() {
                            "a" => {
                                if let Some(editor) = self.view.editor() {
                                    editor.all = true;
                                }
                            }
                            "v" => {
                                if self.view.editor().is_some() {
                                    if self.clipboard.is_none() {
                                        self.clipboard = arboard::Clipboard::new().ok();
                                    }
                                    match self
                                        .clipboard
                                        .as_mut()
                                        .ok_or("Clipboard is unavailable")
                                        .and_then(|c| {
                                            c.get_text().map_err(|_| "Clipboard has no text")
                                        }) {
                                        Ok(text) => {
                                            actions = self.view.insert_text(&text);
                                        }
                                        Err(message) => self.view.status.notice = message.into(),
                                    }
                                }
                            }
                            "c" | "x" => {
                                if !self.view.may_copy() {
                                    self.view.status.notice =
                                        "Keys can be pasted but cannot be copied from this field."
                                            .into();
                                } else if let Some(editor) = self.view.editor().filter(|e| e.all) {
                                    let text = editor.value.clone();
                                    if self.clipboard.is_none() {
                                        self.clipboard = arboard::Clipboard::new().ok();
                                    }
                                    if self
                                        .clipboard
                                        .as_mut()
                                        .is_some_and(|c| c.set_text(text).is_ok())
                                        && key.eq_ignore_ascii_case("x")
                                    {
                                        self.view.editor().unwrap().backspace();
                                        actions = self.view.after_edit();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    match &event.logical_key {
                        Key::Named(NamedKey::Escape) => actions.push(Action::Close),
                        Key::Named(NamedKey::Tab) => {
                            self.view.advance_focus(self.view.modifiers.shift_key())
                        }
                        Key::Named(NamedKey::Enter)
                            if self.view.focus == Some(Control::Prompt)
                                && self.view.modifiers.shift_key() =>
                        {
                            self.view.prompt.insert("\n");
                        }
                        Key::Named(NamedKey::Enter) if self.view.focus == Some(Control::Prompt) => {
                            actions = self.view.activate(Control::Show, None)
                        }
                        Key::Named(NamedKey::Enter) if self.view.focus == Some(Control::Config) => {
                            self.view.focus = Some(Control::Start)
                        }
                        Key::Named(NamedKey::Enter) if self.view.focus == Some(Control::Key) => {
                            actions = self.view.activate(Control::StoreKey, None)
                        }
                        Key::Named(NamedKey::Enter)
                            if self.view.focus == Some(Control::Bridge)
                                && self.view.modifiers.shift_key() =>
                        {
                            actions = self.view.insert_text("\n");
                        }
                        Key::Named(NamedKey::Enter) if self.view.editor().is_some() => {
                            self.view.advance_focus(false);
                        }
                        Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)
                            if self.view.editor().is_none() =>
                        {
                            if let Some(control) = self.view.focus {
                                actions = self.view.activate(control, None);
                            }
                        }
                        Key::Named(NamedKey::ArrowLeft)
                            if self.view.focus == Some(Control::Width) =>
                        {
                            actions = self.view.activate(Control::Narrower, None);
                            self.view.focus = Some(Control::Width);
                        }
                        Key::Named(NamedKey::ArrowRight)
                            if self.view.focus == Some(Control::Width) =>
                        {
                            actions = self.view.activate(Control::Wider, None);
                            self.view.focus = Some(Control::Width);
                        }
                        Key::Named(NamedKey::Backspace) => {
                            if let Some(editor) = self.view.editor() {
                                editor.backspace();
                            }
                            actions = self.view.after_edit();
                        }
                        Key::Named(NamedKey::Delete) => {
                            if let Some(editor) = self.view.editor() {
                                editor.delete();
                            }
                            actions = self.view.after_edit();
                        }
                        Key::Named(NamedKey::ArrowLeft) => {
                            if let Some(editor) = self.view.editor() {
                                editor.left();
                            }
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            if let Some(editor) = self.view.editor() {
                                editor.right();
                            }
                        }
                        Key::Named(NamedKey::Home) => {
                            if let Some(editor) = self.view.editor() {
                                editor.cursor = 0;
                                editor.all = false;
                            }
                        }
                        Key::Named(NamedKey::End) => {
                            if let Some(editor) = self.view.editor() {
                                editor.cursor = editor.value.len();
                                editor.all = false;
                            }
                        }
                        _ => {
                            if let Some(text) = &event.text {
                                actions = self.view.insert_text(text);
                            }
                        }
                    }
                }
            }
            _ => redraw = false,
        }
        if redraw {
            self.window.request_redraw();
        }
        Ok(actions)
    }
    fn redraw(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        let pixmap = draw(&self.view, &self.fonts, size.width, size.height)?;
        self.surface
            .resize(width, height)
            .map_err(|e| e.to_string())?;
        let mut buffer = self.surface.buffer_mut().map_err(|e| e.to_string())?;
        for (destination, pixel) in buffer.iter_mut().zip(pixmap.data().chunks_exact(4)) {
            *destination = ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
        }
        buffer.present().map_err(|e| e.to_string())
    }
}

pub fn render_preview(
    settings: &Settings,
    status: &UiStatus,
    width: u32,
    height: u32,
) -> Result<Pixmap, String> {
    draw(
        &View::new(settings.clone(), status.clone()),
        &Fonts::new()?,
        width,
        height,
    )
}
pub fn render_conversation_preview(
    settings: &Settings,
    status: &UiStatus,
    width: u32,
    height: u32,
) -> Result<Pixmap, String> {
    let mut view = View::new(settings.clone(), status.clone());
    view.tab = Tab::Conversation;
    draw(&view, &Fonts::new()?, width, height)
}
pub fn render_connections_preview(
    settings: &Settings,
    status: &UiStatus,
    width: u32,
    height: u32,
) -> Result<Pixmap, String> {
    let mut view = View::new(settings.clone(), status.clone());
    view.tab = Tab::Connections;
    draw(&view, &Fonts::new()?, width, height)
}
pub fn render_runtime_preview(
    settings: &Settings,
    status: &UiStatus,
    width: u32,
    height: u32,
) -> Result<Pixmap, String> {
    let mut view = View::new(settings.clone(), status.clone());
    view.tab = Tab::Connections;
    view.connections_page = ConnectionsPage::Runtime;
    draw(&view, &Fonts::new()?, width, height)
}

struct Fonts {
    regular: Font,
    bold: Font,
}
impl Fonts {
    fn new() -> Result<Self, String> {
        Ok(Self {
            regular: Font::from_bytes(
                include_bytes!("../assets/IBMPlexSans-Regular.ttf") as &[u8],
                FontSettings::default(),
            )
            .map_err(|e| e.to_string())?,
            bold: Font::from_bytes(
                include_bytes!("../assets/IBMPlexSans-Bold.ttf") as &[u8],
                FontSettings::default(),
            )
            .map_err(|e| e.to_string())?,
        })
    }
}
fn viewport(width: u32, height: u32) -> (f32, f32, f32) {
    let scale = (width as f32 / WIDTH).min(height as f32 / HEIGHT).max(0.01);
    (
        scale,
        (width as f32 - WIDTH * scale) * 0.5,
        (height as f32 - HEIGHT * scale) * 0.5,
    )
}
struct Canvas<'a> {
    pixmap: Pixmap,
    fonts: &'a Fonts,
    scale: f32,
    ox: f32,
    oy: f32,
}
impl Canvas<'_> {
    fn rect(&mut self, rect: Box2, color: [u8; 3]) {
        let Some(rect) = Rect::from_xywh(
            self.ox + rect.x * self.scale,
            self.oy + rect.y * self.scale,
            rect.w * self.scale,
            rect.h * self.scale,
        ) else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color[0], color[1], color[2], 255);
        self.pixmap
            .fill_rect(rect, &paint, Transform::identity(), None);
    }
    fn outline(&mut self, rect: Box2, color: [u8; 3]) {
        self.rect(area(rect.x, rect.y, rect.w, 1.), color);
        self.rect(area(rect.x, rect.y + rect.h - 1., rect.w, 1.), color);
        self.rect(area(rect.x, rect.y, 1., rect.h), color);
        self.rect(area(rect.x + rect.w - 1., rect.y, 1., rect.h), color);
    }
    fn text(&mut self, text: &str, rect: Box2, size: f32, bold: bool, color: [u8; 3]) {
        self.text_scrolled(text, rect, size, bold, color, (0., 0.), true);
    }
    fn text_scrolled(
        &mut self,
        text: &str,
        rect: Box2,
        size: f32,
        bold: bool,
        color: [u8; 3],
        scroll: (f32, f32),
        wrap: bool,
    ) {
        let font = if bold {
            &self.fonts.bold
        } else {
            &self.fonts.regular
        };
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            max_width: wrap.then_some(rect.w * self.scale),
            ..LayoutSettings::default()
        });
        layout.append(&[font], &TextStyle::new(text, size * self.scale, 0));
        let left = (self.ox + rect.x * self.scale).round() as i32;
        let top = (self.oy + rect.y * self.scale).round() as i32;
        let right = (self.ox + (rect.x + rect.w) * self.scale).ceil() as i32;
        let bottom = (self.oy + (rect.y + rect.h) * self.scale).ceil() as i32;
        let width = self.pixmap.width() as i32;
        let height = self.pixmap.height() as i32;
        for glyph in layout.glyphs() {
            let gx = left + (glyph.x - scroll.0 * self.scale).round() as i32;
            let gy = top + (glyph.y - scroll.1 * self.scale).round() as i32;
            if gx >= right
                || gy >= bottom
                || gx + glyph.width as i32 <= left
                || gy + glyph.height as i32 <= top
            {
                continue;
            }
            let (metrics, bitmap) = font.rasterize_config(glyph.key);
            for yy in 0..metrics.height as i32 {
                let y = gy + yy;
                if y < top || y >= bottom || y < 0 || y >= height {
                    continue;
                }
                for xx in 0..metrics.width as i32 {
                    let x = gx + xx;
                    if x < left || x >= right || x < 0 || x >= width {
                        continue;
                    }
                    let alpha = bitmap[yy as usize * metrics.width + xx as usize] as u32;
                    if alpha == 0 {
                        continue;
                    }
                    let index = ((y * width + x) * 4) as usize;
                    let pixel = &mut self.pixmap.data_mut()[index..index + 4];
                    for channel in 0..3 {
                        pixel[channel] = ((color[channel] as u32 * alpha
                            + pixel[channel] as u32 * (255 - alpha)
                            + 127)
                            / 255) as u8;
                    }
                    pixel[3] = 255;
                }
            }
        }
    }
    fn accent(&mut self, x: f32, y: f32, length: f32) {
        let mut path = PathBuilder::new();
        path.move_to(x, y + 3.);
        path.cubic_to(
            x + length * 0.3,
            y - 1.,
            x + length * 0.7,
            y + 5.,
            x + length,
            y,
        );
        let mut paint = Paint::default();
        paint.set_color_rgba8(RED[0], RED[1], RED[2], 255);
        if let Some(path) = path.finish() {
            self.pixmap.stroke_path(
                &path,
                &paint,
                &Stroke {
                    width: 5.,
                    ..Stroke::default()
                },
                Transform::from_row(self.scale, 0., 0., self.scale, self.ox, self.oy),
                None,
            );
        }
    }
    fn button(&mut self, view: &View, control: Control, label: &str, rect: Box2, primary: bool) {
        let hovered = view.hover == Some(control);
        let color = if primary {
            if hovered {
                [255, 78, 87]
            } else {
                RED
            }
        } else if hovered {
            [49, 50, 57]
        } else {
            [34, 35, 41]
        };
        self.rect(rect, color);
        if view.focus == Some(control) {
            self.outline(
                area(rect.x - 3., rect.y - 3., rect.w + 6., rect.h + 6.),
                WHITE,
            );
        }
        self.text(
            label,
            area(rect.x + 15., rect.y + 11., rect.w - 30., 24.),
            15.,
            true,
            if primary { [20, 20, 24] } else { WHITE },
        );
    }
    fn toggle(&mut self, x: f32, y: f32, on: bool, enabled: bool) {
        self.rect(
            area(x, y, 46., 25.),
            if on && enabled { RED } else { [53, 54, 61] },
        );
        self.rect(
            area(x + if on { 25. } else { 4. }, y + 4., 17., 17.),
            if enabled { WHITE } else { [117, 118, 123] },
        );
    }
    fn edit(
        &mut self,
        view: &View,
        control: Control,
        editor: &Editor,
        rect: Box2,
        placeholder: &str,
    ) {
        self.rect(rect, [19, 20, 25]);
        self.outline(
            rect,
            if view.focus == Some(control) {
                RED
            } else {
                [57, 58, 64]
            },
        );
        if editor.all && view.focus == Some(control) {
            self.rect(
                area(rect.x + 10., rect.y + 9., rect.w - 20., rect.h - 18.),
                [73, 36, 44],
            );
        }
        let content = if editor.value.is_empty() {
            placeholder
        } else {
            &editor.value
        };
        let text_box = area(rect.x + 13., rect.y + 12., rect.w - 26., rect.h - 22.);
        let focused = view.focus == Some(control);
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            max_width: editor.multiline.then_some(text_box.w),
            ..LayoutSettings::default()
        });
        layout.append(
            &[&self.fonts.regular],
            &TextStyle::new(&editor.value, 14., 0),
        );
        let glyphs = layout.glyphs();
        let (caret_x, caret_y) =
            if let Some(glyph) = glyphs.iter().find(|g| g.byte_offset >= editor.cursor) {
                (glyph.x, glyph.y)
            } else if let Some(glyph) = glyphs.last() {
                if glyph.parent == '\n' {
                    (0., glyph.y + 19.)
                } else {
                    (
                        glyph.x + self.fonts.regular.metrics(glyph.parent, 14.).advance_width,
                        glyph.y,
                    )
                }
            } else {
                (0., 2.)
            };
        let scroll = if focused {
            if editor.multiline {
                (0., (caret_y + 18. - text_box.h).max(0.))
            } else {
                ((caret_x + 2. - text_box.w).max(0.), 0.)
            }
        } else {
            (0., 0.)
        };
        self.text_scrolled(
            content,
            text_box,
            14.,
            false,
            if editor.value.is_empty() {
                MUTED
            } else {
                WHITE
            },
            scroll,
            editor.multiline,
        );
        if focused && !editor.all {
            self.rect(
                area(
                    text_box.x + caret_x - scroll.0,
                    text_box.y + caret_y - scroll.1,
                    1.,
                    15.,
                ),
                WHITE,
            );
        }
    }
}

fn draw(view: &View, fonts: &Fonts, width: u32, height: u32) -> Result<Pixmap, String> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || width as u64 * height as u64 > 20_000_000
    {
        return Err("Settings render dimensions are outside the supported range".into());
    }
    let mut pixmap = Pixmap::new(width, height).ok_or("Cannot allocate settings pixels")?;
    pixmap.fill(Color::from_rgba8(23, 24, 29, 255));
    let (scale, ox, oy) = viewport(width, height);
    let mut c = Canvas {
        pixmap,
        fonts,
        scale,
        ox,
        oy,
    };
    c.rect(area(0., 0., 174., HEIGHT), [16, 17, 21]);
    c.text("SCREEN", area(24., 35., 130., 32.), 23., true, WHITE);
    c.text("CUE", area(24., 64., 130., 43.), 34., true, WHITE);
    c.accent(24., 112., 85.);
    for (control, label, y, selected) in [
        (
            Control::Appearance,
            "Appearance",
            153.,
            view.tab == Tab::Appearance,
        ),
        (
            Control::Conversation,
            "Conversation",
            202.,
            view.tab == Tab::Conversation,
        ),
        (
            Control::Connections,
            "Connections",
            251.,
            view.tab == Tab::Connections,
        ),
    ] {
        if selected {
            c.rect(area(18., y, 140., 42.), [42, 28, 34]);
            c.rect(area(18., y, 3., 42.), RED);
        } else if view.hover == Some(control) {
            c.rect(area(18., y, 140., 42.), [28, 29, 35]);
        }
        if view.focus == Some(control) {
            c.outline(area(18., y, 140., 42.), WHITE);
        }
        c.text(
            label,
            area(32., y + 11., 120., 26.),
            15.,
            selected,
            if selected { WHITE } else { MUTED },
        );
    }
    c.text(
        "ALWAYS WITH YOU",
        area(24., 601., 130., 22.),
        10.,
        true,
        MUTED,
    );
    c.text(
        "Hide every cue",
        area(24., 627., 130., 22.),
        13.,
        false,
        WHITE,
    );
    c.text(
        &view.status.shortcut,
        area(24., 652., 130., 42.),
        12.,
        true,
        RED,
    );
    c.text(
        "NATIVE / LIGHTWEIGHT",
        area(24., 706., 145., 16.),
        9.,
        false,
        MUTED,
    );
    match view.tab {
        Tab::Appearance => {
            c.text(
                "MAKE ROOM FOR YOUR NEXT THOUGHT",
                area(194., 32., 530., 22.),
                10.,
                true,
                RED,
            );
            c.text(
                "Your cues. Your screen.",
                area(194., 62., 530., 44.),
                29.,
                true,
                WHITE,
            );
            c.text(
                "A little direction, exactly where you need it.",
                area(194., 112., 530., 26.),
                15.,
                false,
                MUTED,
            );
            for (control, title, body, x, selected) in [
                (
                    Control::Guidance,
                    "Guide me",
                    "Arrows, targets and text",
                    194.,
                    view.settings.mode == Mode::Guidance,
                ),
                (
                    Control::ConversationMode,
                    "Help me speak",
                    "Independent text cues",
                    466.,
                    view.settings.mode == Mode::Conversation,
                ),
            ] {
                c.rect(
                    area(x, 155., 258., 76.),
                    if selected { [43, 29, 35] } else { [31, 32, 38] },
                );
                c.outline(
                    area(x, 155., 258., 76.),
                    if selected { RED } else { [57, 58, 65] },
                );
                if view.focus == Some(control) {
                    c.outline(area(x - 3., 152., 264., 82.), WHITE);
                }
                c.text(title, area(x + 17., 169., 224., 25.), 17., true, WHITE);
                c.text(body, area(x + 17., 197., 224., 22.), 13., false, MUTED);
            }
            c.text(
                "Hide from screen sharing",
                area(194., 254., 455., 25.),
                16.,
                true,
                WHITE,
            );
            c.text(
                &view.status.capture_note,
                area(194., 282., 455., 37.),
                12.,
                false,
                MUTED,
            );
            c.toggle(
                678.,
                258.,
                view.settings.hide_from_capture,
                view.status.capture_supported,
            );
            c.rect(area(194., 323., 530., 1.), [46, 47, 54]);
            c.text(
                "Hand-drawn movement",
                area(194., 338., 455., 24.),
                16.,
                true,
                WHITE,
            );
            c.text(
                "Keep the sketch alive. Text stays steady.",
                area(194., 365., 455., 22.),
                12.,
                false,
                MUTED,
            );
            c.toggle(678., 342., view.settings.animate, true);
            c.rect(area(194., 397., 530., 1.), [46, 47, 54]);
            c.text(
                "Text panel width",
                area(194., 412., 300., 25.),
                16.,
                true,
                WHITE,
            );
            c.text(
                &format!("{} px", view.settings.panel_width),
                area(540., 413., 76., 25.),
                14.,
                true,
                MUTED,
            );
            c.rect(area(194., 451., 422., 4.), [62, 63, 71]);
            let position = (view.settings.panel_width.clamp(260, 800) - 260) as f32 / 540.;
            c.rect(area(194., 451., 422. * position, 4.), RED);
            c.rect(area(194. + 422. * position - 4., 445., 8., 16.), WHITE);
            c.button(
                view,
                Control::Narrower,
                "-",
                area(632., 431., 40., 40.),
                false,
            );
            c.button(view, Control::Wider, "+", area(684., 431., 40., 40.), false);
            c.text("Try a cue", area(194., 487., 280., 25.), 16., true, WHITE);
            c.text(
                "Enter to show / Shift+Enter for a new line",
                area(390., 489., 334., 22.),
                11.,
                false,
                MUTED,
            );
            c.edit(
                view,
                Control::Prompt,
                &view.prompt,
                area(194., 514., 530., 91.),
                "Write a thought to keep in view...",
            );
            c.button(
                view,
                Control::Show,
                "Show on my screen",
                area(194., 627., 258., 44.),
                true,
            );
            c.button(
                view,
                Control::Hide,
                "Hide all cues",
                area(466., 627., 258., 44.),
                false,
            );
        }
        Tab::Conversation => {
            c.text("STAY PRESENT", area(194., 32., 530., 22.), 10., true, RED);
            c.text(
                "A thought when you need it.",
                area(194., 62., 530., 44.),
                27.,
                true,
                WHITE,
            );
            c.text(
                "Keep the conversation flowing. Cues stay on your screen.",
                area(194., 112., 530., 28.),
                14.,
                false,
                MUTED,
            );
            c.rect(area(194., 155., 530., 82.), [31, 32, 38]);
            let listening = view
                .status
                .conversation_state
                .to_lowercase()
                .contains("listening");
            c.rect(
                area(213., 177., 9., 9.),
                if listening { [88, 220, 160] } else { MUTED },
            );
            c.text(
                &view.status.conversation_state,
                area(237., 171., 466., 28.),
                17.,
                true,
                WHITE,
            );
            c.text(
                if view.settings.connections.enabled {
                    "Using your saved Connections profile"
                } else {
                    "Audio sources come from your configuration"
                },
                area(213., 207., 480., 22.),
                12.,
                false,
                MUTED,
            );
            c.text(
                "You choose the moment.",
                area(194., 258., 530., 28.),
                19.,
                true,
                WHITE,
            );
            c.text(if view.settings.connections.enabled { "Listening starts only when you press Start. To use a JSON profile below, turn off the Connections profile in Runtime." } else { "Listening starts only when you press Start. Set up models in Connections, or choose an advanced JSON profile below." },area(194.,291.,530.,42.),13.,false,MUTED);
            c.edit(
                view,
                Control::Config,
                &view.config,
                area(194., 347., 530., 54.),
                "Path to your conversation configuration (.json)",
            );
            c.button(
                view,
                Control::CreateConfig,
                "Create example config",
                area(194., 417., 258., 42.),
                false,
            );
            c.button(
                view,
                Control::OpenConfig,
                "Open audio config",
                area(466., 417., 258., 42.),
                false,
            );
            c.rect(area(194., 490., 530., 101.), [19, 20, 25]);
            c.accent(212., 510., 34.);
            c.text(
                "BRING YOUR OWN MODEL",
                area(266., 504., 430., 22.),
                11.,
                true,
                RED,
            );
            c.text("Choose independent models and keys in Connections. Advanced JSON profiles and direct cues from your agent remain available.",area(213.,538.,487.,47.),13.,false,MUTED);
            c.button(
                view,
                Control::Start,
                "Start conversation",
                area(194., 627., 258., 44.),
                true,
            );
            c.button(
                view,
                Control::Stop,
                "Stop listening",
                area(466., 627., 258., 44.),
                false,
            );
        }
        Tab::Connections => draw_connections(&mut c, view),
    }
    for (control, rect) in view.controls() {
        if view.focus == Some(control)
            && matches!(
                control,
                Control::Capture
                    | Control::Animation
                    | Control::Width
                    | Control::ConnectionsEnabled
                    | Control::Microphone
                    | Control::SystemAudio
                    | Control::Diarization
            )
        {
            c.outline(
                area(rect.x - 3., rect.y - 3., rect.w + 6., rect.h + 6.),
                WHITE,
            );
        }
    }
    let default_status = if view.status.overlay_visible {
        "Cue visible on your screen"
    } else {
        "Ready when you are"
    };
    c.text(
        if view.status.notice.is_empty() {
            default_status
        } else {
            &view.status.notice
        },
        area(194., 692., 530., 27.),
        11.,
        false,
        MUTED,
    );
    Ok(c.pixmap)
}

fn draw_connections(c: &mut Canvas<'_>, view: &View) {
    c.text(
        "YOUR MODELS, WORKING TOGETHER",
        area(194., 32., 530., 22.),
        10.,
        true,
        RED,
    );
    c.text(
        "Every role. Its own connection.",
        area(194., 62., 530., 44.),
        26.,
        true,
        WHITE,
    );
    let description = match view.connections_page {
        ConnectionsPage::Runtime => "Choose the sources and runtime. Listening starts separately.",
        ConnectionsPage::Models => match view.lane {
            Lane::Transcription => "Audio & speakers · transcription with optional speaker labels",
            Lane::Fast => "Fast model · short replies as the conversation unfolds",
            Lane::Planner => "Big model · context, prepared replies and useful next topics",
            Lane::Decider => "System 1 · autonomous, low-latency decisions from ready actions",
        },
    };
    c.text(description, area(194., 112., 530., 28.), 13., false, MUTED);
    for (control, label, x, selected) in [
        (
            Control::ConnectionModels,
            "Models & keys",
            194.,
            view.connections_page == ConnectionsPage::Models,
        ),
        (
            Control::ConnectionRuntime,
            "Runtime & audio",
            466.,
            view.connections_page == ConnectionsPage::Runtime,
        ),
    ] {
        c.button(view, control, label, area(x, 153., 258., 40.), false);
        if selected {
            c.rect(area(x, 190., 258., 3.), RED);
        }
    }
    match view.connections_page {
        ConnectionsPage::Models => {
            for (index, lane) in Lane::ALL.into_iter().enumerate() {
                let x = 194. + index as f32 * 136.;
                c.button(
                    view,
                    Control::Lane(lane),
                    lane_label(lane),
                    area(x, 207., 122., 39.),
                    false,
                );
                if view.lane == lane {
                    c.outline(area(x, 207., 122., 39.), RED);
                }
            }
            let connection = view.connections.get(view.lane);
            c.text("Connection", area(194., 278., 125., 24.), 14., true, WHITE);
            c.button(
                view,
                Control::Provider,
                provider_label(connection.kind),
                area(325., 266., 399., 40.),
                false,
            );
            if connection.kind == ProviderKind::Disabled {
                c.rect(area(194., 331., 530., 167.), [19, 20, 25]);
                c.accent(213., 354., 37.);
                c.text(
                    "THIS ROLE IS OPTIONAL",
                    area(272., 348., 410., 24.),
                    12.,
                    true,
                    RED,
                );
                c.text("Choose an API or a local command bridge above. Each role can use a different provider, model and credential.", area(213., 387., 487., 51.), 15., false, WHITE);
                c.text(
                    "Your existing agent bridges keep working independently.",
                    area(213., 458., 487., 24.),
                    12.,
                    false,
                    MUTED,
                );
            } else {
                if connection.kind == ProviderKind::Command {
                    c.text(
                        "Bridge command · JSON argument array, no shell",
                        area(194., 314., 530., 22.),
                        12.,
                        true,
                        MUTED,
                    );
                    c.edit(
                        view,
                        Control::Bridge,
                        &view.bridge,
                        area(194., 338., 530., 105.),
                        "[\"/path/to/bridge\", \"--json\"]",
                    );
                } else {
                    c.text(
                        "Endpoint · complete API URL",
                        area(194., 314., 530., 22.),
                        12.,
                        true,
                        MUTED,
                    );
                    c.edit(
                        view,
                        Control::Endpoint,
                        &view.endpoint,
                        area(194., 338., 530., 42.),
                        "https://provider.example/v1/...",
                    );
                    c.text("Model", area(194., 383., 530., 22.), 12., true, MUTED);
                    c.edit(
                        view,
                        Control::Model,
                        &view.model,
                        area(194., 405., 530., 42.),
                        "Provider model identifier",
                    );
                }
                c.text("Credential", area(194., 472., 125., 24.), 14., true, WHITE);
                let auth = match connection.auth {
                    AuthMode::None => "No API key  >",
                    AuthMode::Vault => "Saved in system credential store  >",
                    AuthMode::Environment => "Environment variable  >",
                };
                c.button(
                    view,
                    Control::Auth,
                    auth,
                    area(325., 460., 399., 40.),
                    false,
                );
                match connection.auth {
                    AuthMode::Vault => {
                        let stored = view.status.credential_states[lane_index(view.lane)];
                        let label = if view.status.credential_error.is_some() {
                            format!(
                                "Key status unavailable · {}",
                                view.status.credential_backend
                            )
                        } else if stored && connection != view.settings.connections.get(view.lane) {
                            "Key exists for the previously saved role; save a key for this change."
                                .into()
                        } else if stored {
                            format!("Saved key exists · {}", view.status.credential_backend)
                        } else {
                            format!("No saved key · {}", view.status.credential_backend)
                        };
                        c.text(&label, area(194., 508., 530., 22.), 11., false, MUTED);
                        // Only the masked surrogate reaches text layout or glyph rasterization.
                        c.edit(
                            view,
                            Control::Key,
                            &masked_editor(&view.key),
                            area(194., 532., 530., 42.),
                            "Paste a key to save or replace it",
                        );
                        c.button(
                            view,
                            Control::StoreKey,
                            "Save key + this role",
                            area(194., 583., 258., 32.),
                            false,
                        );
                        c.button(
                            view,
                            Control::DeleteKey,
                            "Delete saved key",
                            area(466., 583., 258., 32.),
                            false,
                        );
                    }
                    AuthMode::Environment => {
                        c.text(
                            "Variable name · the value stays outside this profile",
                            area(194., 508., 530., 22.),
                            12.,
                            false,
                            MUTED,
                        );
                        c.edit(
                            view,
                            Control::KeyEnvironment,
                            &view.key_environment,
                            area(194., 532., 530., 42.),
                            view.lane.env_name(),
                        );
                        c.text(
                            "Set the variable before launching ScreenCue.",
                            area(194., 586., 530., 24.),
                            12.,
                            false,
                            MUTED,
                        );
                    }
                    AuthMode::None => {
                        c.text("For local endpoints or bridges that manage their own login. No credential is supplied by ScreenCue.", area(194., 526., 530., 52.), 14., false, MUTED);
                    }
                }
            }
        }
        ConnectionsPage::Runtime => {
            c.text(
                "Use this Connections profile",
                area(194., 217., 455., 27.),
                17.,
                true,
                WHITE,
            );
            c.text(
                "Takes precedence over the advanced JSON profile.",
                area(194., 249., 455., 24.),
                12.,
                false,
                MUTED,
            );
            c.toggle(678., 222., view.connections.enabled, true);
            c.rect(area(194., 279., 530., 1.), [46, 47, 54]);
            for (title, detail, y, on) in [
                (
                    "Microphone",
                    "Your voice",
                    292.,
                    view.connections.microphone,
                ),
                (
                    "System audio",
                    "Other participants on supported devices",
                    348.,
                    view.connections.system_audio,
                ),
                (
                    "Speaker labels (diarization)",
                    "Model must support diarized_json or speaker segments",
                    404.,
                    view.connections.diarization,
                ),
            ] {
                c.text(title, area(194., y, 455., 24.), 15., true, WHITE);
                c.text(detail, area(194., y + 25., 455., 21.), 11., false, MUTED);
                c.toggle(678., y + 5., on, true);
            }
            c.text(
                "Python executable · absolute path for the coordinator",
                area(194., 468., 530., 23.),
                12.,
                true,
                MUTED,
            );
            c.edit(
                view,
                Control::Python,
                &view.python,
                area(194., 494., 530., 44.),
                "Absolute path to Python 3.11 or newer",
            );
            c.text("Speaker labels do not establish identities or roles. Advanced role mappings remain available in the JSON profile.", area(194., 555., 530., 45.), 13., false, MUTED);
        }
    }
    c.button(
        view,
        Control::SaveConnections,
        if view.has_unsaved_connections() {
            "Save connections · unsaved changes"
        } else {
            "Save connections"
        },
        area(194., 627., 530., 44.),
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_capture_is_not_clickable_or_focusable() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        assert_eq!(view.hit(700., 268.), None);
        assert!(view.activate(Control::Capture, None).is_empty());
        assert!(!view.settings.hide_from_capture);
        view.status.capture_supported = true;
        assert_eq!(view.hit(700., 268.), Some(Control::Capture));
        assert!(matches!(
            view.activate(Control::Capture, None).as_slice(),
            [Action::Changed(_)]
        ));
        assert!(view.settings.hide_from_capture);
    }
    #[test]
    fn text_editing_preserves_unicode_and_rejects_oversized_pastes() {
        let mut editor = Editor::new("Hi 🌍".into(), 12, true);
        editor.backspace();
        assert_eq!(editor.value, "Hi ");
        editor.insert("café");
        assert_eq!(editor.value, "Hi café");
        editor.left();
        editor.delete();
        assert_eq!(editor.value, "Hi caf");
        let old = editor.value.clone();
        editor.insert(&"x".repeat(13));
        assert_eq!(editor.value, old);
        editor.all = true;
        editor.insert("New\nthought");
        assert_eq!(editor.value, "New\nthought");
    }
    #[test]
    fn mode_changes_are_preferences_but_preview_text_is_ephemeral() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        assert!(matches!(
            view.activate(Control::ConversationMode, None).as_slice(),
            [Action::Changed(_)]
        ));
        view.prompt.value = "A private thought".into();
        match &view.activate(Control::Show, None)[0] {
            Action::ShowPreview { title, text } => {
                assert_eq!(title, "Your next thought");
                assert_eq!(text, "A private thought");
            }
            _ => panic!("Missing preview"),
        };
        assert!(!serde_json::to_string(&view.settings)
            .unwrap()
            .contains("private"));
    }
    #[test]
    fn width_inputs_clamp_and_tabs_do_not_start_audio() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        view.activate(Control::Width, Some(-200.));
        assert_eq!(view.settings.panel_width, 260);
        view.activate(Control::Narrower, None);
        assert_eq!(view.settings.panel_width, 260);
        view.activate(Control::Width, Some(9000.));
        assert_eq!(view.settings.panel_width, 800);
        assert!(view.activate(Control::Conversation, None).is_empty());
        assert!(matches!(
            view.activate(Control::Start, None).as_slice(),
            [Action::StartConversation]
        ));
    }
    #[test]
    fn render_is_opaque_and_scaled_input_matches_logical_controls() {
        let (scale, ox, oy) = viewport(1140, 1095);
        assert_eq!(scale, 1.5);
        assert_eq!((ox, oy), (0., 0.));
        let image = render_preview(&Settings::default(), &UiStatus::default(), 760, 730).unwrap();
        assert!(image.data().chunks_exact(4).all(|p| p[3] == 255));
        let small =
            render_conversation_preview(&Settings::default(), &UiStatus::default(), 570, 548)
                .unwrap();
        assert_eq!(small.width(), 570);
        assert!(render_preview(&Settings::default(), &UiStatus::default(), 0, 0).is_err());
    }
    #[test]
    fn connection_drafts_are_isolated_and_do_not_start_listening() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        let original = view.settings.clone();
        assert!(view.activate(Control::Connections, None).is_empty());
        assert!(view.activate(Control::Lane(Lane::Fast), None).is_empty());
        view.activate(Control::Provider, None);
        view.activate(Control::Model, None);
        view.model.all = true;
        assert!(view.insert_text("quick-local-model").is_empty());
        view.activate(Control::Lane(Lane::Planner), None);
        assert_ne!(view.model.value, "quick-local-model");
        view.activate(Control::Lane(Lane::Fast), None);
        assert_eq!(view.model.value, "quick-local-model");
        assert_eq!(view.settings, original);
        assert!(matches!(
            view.activate(Control::SaveConnections, None).as_slice(),
            [Action::SaveConnections(_)]
        ));
        view.activate(Control::ConnectionRuntime, None);
        assert!(view.activate(Control::ConnectionsEnabled, None).is_empty());
        assert!(view.activate(Control::Diarization, None).is_empty());
        assert!(view.connections.diarization);
    }
    #[test]
    fn keys_are_masked_not_copyable_not_serialized_and_bound_to_the_selected_role() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        view.activate(Control::Connections, None);
        view.activate(Control::Lane(Lane::Fast), None);
        view.connections.get_mut(Lane::Fast).kind = ProviderKind::OpenaiCompatible;
        view.connections.get_mut(Lane::Fast).auth = AuthMode::Vault;
        view.activate(Control::Key, None);
        view.insert_text("private-key-🌍");
        let masked = masked_editor(&view.key);
        assert!(masked
            .value
            .chars()
            .all(|character| character == '\u{2022}'));
        assert_eq!(masked.value.chars().count(), 13);
        assert_eq!(masked.cursor, masked.value.len());
        assert!(!view.may_copy());
        assert!(!serde_json::to_string(&view.settings)
            .unwrap()
            .contains("private-key"));
        assert!(!serde_json::to_string(&view.connections)
            .unwrap()
            .contains("private-key"));
        let actions = view.activate(Control::StoreKey, None);
        match &actions[0] {
            Action::StoreConnectionKey {
                lane,
                connection,
                secret,
            } => {
                assert_eq!(*lane, Lane::Fast);
                assert_eq!(connection, view.connections.get(Lane::Fast));
                assert_eq!(secret, "private-key-🌍");
            }
            _ => panic!("Expected a scoped key action"),
        }
        assert!(view.key.value.is_empty());
        view.activate(Control::Key, None);
        view.insert_text("another-key");
        view.activate(Control::Lane(Lane::Planner), None);
        assert!(view.key.value.is_empty());
    }
    #[test]
    fn different_key_contents_render_identical_masked_pixels() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        view.tab = Tab::Connections;
        view.connections.get_mut(Lane::Transcription).kind = ProviderKind::OpenaiCompatible;
        view.connections.get_mut(Lane::Transcription).auth = AuthMode::Vault;
        view.key = Editor::new("secret-ALPHA".into(), 8192, false);
        let fonts = Fonts::new().unwrap();
        let first = draw(&view, &fonts, 760, 730).unwrap();
        view.key = Editor::new("hidden-BRAVO".into(), 8192, false);
        let second = draw(&view, &fonts, 760, 730).unwrap();
        assert_eq!(first.data(), second.data());
    }
    #[test]
    fn malformed_bridge_blocks_save_and_role_switch_without_losing_draft() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        view.connections.get_mut(Lane::Transcription).kind = ProviderKind::Command;
        view.bridge = Editor::new("[invalid".into(), 8192, true);
        assert!(view.activate(Control::SaveConnections, None).is_empty());
        view.activate(Control::Lane(Lane::Fast), None);
        assert_eq!(view.lane, Lane::Transcription);
        assert_eq!(view.bridge.value, "[invalid");
        view.bridge = Editor::new("[\"bridge\",\"--json\"]".into(), 8192, true);
        view.activate(Control::Lane(Lane::Fast), None);
        assert_eq!(
            view.connections.get(Lane::Transcription).command,
            vec!["bridge", "--json"]
        );
        assert_eq!(view.lane, Lane::Fast);
    }
    #[test]
    fn connection_controls_fit_the_window_and_hide_irrelevant_fields() {
        let mut view = View::new(Settings::default(), UiStatus::default());
        view.tab = Tab::Connections;
        for page in [ConnectionsPage::Models, ConnectionsPage::Runtime] {
            view.connections_page = page;
            for lane in Lane::ALL {
                view.lane = lane;
                for kind in providers(lane) {
                    view.connections.get_mut(lane).kind = *kind;
                    for auth in [AuthMode::None, AuthMode::Vault, AuthMode::Environment] {
                        view.connections.get_mut(lane).auth = auth;
                        for (_, rect) in view.controls() {
                            assert!(
                                rect.x >= 0.
                                    && rect.y >= 0.
                                    && rect.x + rect.w <= WIDTH
                                    && rect.y + rect.h <= HEIGHT
                            );
                        }
                        if *kind == ProviderKind::Disabled && page == ConnectionsPage::Models {
                            assert!(!view
                                .controls()
                                .iter()
                                .any(|(control, _)| *control == Control::Key));
                        }
                    }
                }
            }
        }
    }
}
