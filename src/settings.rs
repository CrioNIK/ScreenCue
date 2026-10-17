//! Local preferences only. Conversation content and provider credentials are not stored here.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Guidance,
    Conversation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub hide_from_capture: bool,
    pub animate: bool,
    pub panel_width: u32,
    pub audio_config: String,
    pub connections: crate::connections::Connections,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Guidance,
            hide_from_capture: false,
            animate: true,
            panel_width: 420,
            audio_config: String::new(),
            connections: crate::connections::Connections::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(260..=800).contains(&self.panel_width) {
            return Err("Panel width must be between 260 and 800 logical pixels".into());
        }
        if self.audio_config.chars().count() > 4096
            || self.audio_config.chars().any(char::is_control)
        {
            return Err("Audio configuration path must be at most 4096 characters without control characters".into());
        }
        self.connections.validate()?;
        if serde_json::to_vec_pretty(self)
            .map_err(|_| "Cannot serialize settings")?
            .len()
            > 32768
        {
            return Err("Settings file exceeds 32 KiB".into());
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(format!("Cannot open settings: {error}")),
        };
        let mut bytes = Vec::new();
        file.take(32769)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 32768 {
            return Err("Settings file exceeds 32 KiB".into());
        }
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid settings: {e}"))?;
        value.validate()?;
        Ok(value)
    }

    /// Write alongside the destination, flush, then atomically replace it.
    /// A failed write never truncates the previous preferences.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create settings directory: {e}"))?;
        let name = path
            .file_name()
            .ok_or("Settings path has no filename")?
            .to_string_lossy();
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let temporary = parent.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .map_err(|e| format!("Cannot create settings draft: {e}"))?;
            let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.write_all(b"\n"))
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("Cannot save settings: {e}"))?;
            drop(file);
            fs::rename(&temporary, path).map_err(|e| format!("Cannot replace settings: {e}"))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn default_path() -> Result<PathBuf, String> {
        default_path()
    }
}

pub fn default_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("APPDATA is unavailable; supply a settings path")?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join("Library/Application Support"))
        .ok_or("HOME is unavailable; supply a settings path")?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or("No configuration directory is available; supply a settings path")?;
    Ok(base.join("ScreenCue").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_or_partial_settings_keep_safe_defaults() {
        let value: Settings = serde_json::from_str(r#"{"mode":"conversation"}"#).unwrap();
        assert_eq!(value.mode, Mode::Conversation);
        assert!(!value.hide_from_capture);
        assert_eq!(value.panel_width, 420);
        assert_eq!(
            value.connections,
            crate::connections::Connections::default()
        );
        assert!(serde_json::from_str::<Settings>(r#"{"api_key":"secret"}"#).is_err());
    }

    #[test]
    fn settings_replace_atomically_and_invalid_write_preserves_previous_file() {
        let dir = std::env::temp_dir().join(format!(
            "screencue-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("settings.json");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let mut settings = Settings::default();
        settings.save(&path).unwrap();
        settings.mode = Mode::Conversation;
        settings.audio_config = "some config.json".into();
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        let old = fs::read(&path).unwrap();
        settings.panel_width = 0;
        assert!(settings.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), old);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn reject_unbounded_or_control_character_preferences() {
        let mut settings = Settings::default();
        settings.audio_config = "test\nfile.json".into();
        assert!(settings.validate().is_err());
        settings.audio_config = "x".repeat(4097);
        assert!(settings.validate().is_err());
    }

    #[test]
    fn combined_preferences_cannot_exceed_the_load_limit() {
        let mut settings = Settings::default();
        settings.audio_config = "🧭".repeat(4096);
        settings.connections.fast.command = vec!["argument".repeat(125); 20];
        settings.connections.validate().unwrap();
        assert_eq!(
            settings.validate().unwrap_err(),
            "Settings file exceeds 32 KiB"
        );
    }
}
