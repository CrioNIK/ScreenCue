//! Provider routing preferences. Credentials are referenced by name, never serialized.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Transcription,
    Fast,
    Planner,
    Decider,
}

pub const ALL: [Lane; 4] = [
    Lane::Transcription,
    Lane::Fast,
    Lane::Planner,
    Lane::Decider,
];

impl Lane {
    pub const ALL: [Self; 4] = ALL;

    pub fn label(self) -> &'static str {
        match self {
            Self::Transcription => "Transcription",
            Self::Fast => "Fast model",
            Self::Planner => "Slow planner",
            Self::Decider => "Typed decider",
        }
    }

    pub fn env_name(self) -> &'static str {
        match self {
            Self::Transcription => "SCREENCUE_TRANSCRIPTION_API_KEY",
            Self::Fast => "SCREENCUE_FAST_API_KEY",
            Self::Planner => "SCREENCUE_PLANNER_API_KEY",
            Self::Decider => "SCREENCUE_DECIDER_API_KEY",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    #[default]
    Disabled,
    OpenaiCompatible,
    Typesafe,
    OpenaiDecisions,
    Ollama,
    Command,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    None,
    Vault,
    Environment,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connection {
    pub kind: ProviderKind,
    pub endpoint: String,
    pub model: String,
    pub command: Vec<String>,
    pub auth: AuthMode,
    pub key_env: String,
}

impl Connection {
    fn validate_fields(&self) -> Result<(), String> {
        if !bounded(&self.endpoint, 2048)
            || !bounded(&self.model, 200)
            || !bounded(&self.key_env, 128)
            || self.command.len() > 32
            || self.command.iter().any(|argument| !bounded(argument, 4096))
        {
            return Err(
                "Connection fields exceed supported bounds or contain control characters".into(),
            );
        }
        if self.auth == AuthMode::Environment && !valid_env_name(&self.key_env) {
            return Err("Environment authentication requires a variable name, not a key".into());
        }
        Ok(())
    }

    fn validate_active(&self, lane: Lane) -> Result<(), String> {
        if self.kind == ProviderKind::Disabled {
            return Ok(());
        }
        let allowed = if lane == Lane::Decider {
            matches!(
                self.kind,
                ProviderKind::Typesafe
                    | ProviderKind::OpenaiDecisions
                    | ProviderKind::Ollama
                    | ProviderKind::Command
            )
        } else {
            matches!(
                self.kind,
                ProviderKind::OpenaiCompatible | ProviderKind::Command
            )
        };
        if !allowed {
            return Err("Provider kind is not supported for this lane".into());
        }
        if self.kind == ProviderKind::Command {
            if self.command.is_empty() || self.command.iter().any(|argument| argument.is_empty()) {
                return Err("A command bridge needs an executable and nonempty arguments".into());
            }
            return Ok(());
        }
        validate_endpoint(&self.endpoint)?;
        if self.model.trim().is_empty() {
            return Err("An API connection requires a model name".into());
        }
        if matches!(
            self.kind,
            ProviderKind::Typesafe | ProviderKind::OpenaiDecisions
        ) && self.auth == AuthMode::None
        {
            return Err("This typed decision provider requires explicit authentication".into());
        }
        Ok(())
    }

    fn credential_env(&self, lane: Lane) -> Option<&str> {
        match self.auth {
            AuthMode::None => None,
            AuthMode::Vault => Some(lane.env_name()),
            AuthMode::Environment => Some(&self.key_env),
        }
    }

    fn native_provider(&self, lane: Lane, diarization: bool) -> Value {
        match self.kind {
            ProviderKind::Disabled => Value::Null,
            ProviderKind::Command => json!({"kind":"command", "command":self.command,
                "timeout_ms":if lane == Lane::Fast { 1800 } else { 8000 }}),
            _ => {
                let mut value = json!({"kind":"openai-compatible", "endpoint":self.endpoint,
                    "model":self.model, "key_env":self.credential_env(lane).unwrap_or(""),
                    "timeout_ms":if lane == Lane::Fast { 1800 } else { 8000 }});
                if lane == Lane::Transcription && diarization {
                    value["response_format"] = json!("diarized_json");
                }
                value
            }
        }
    }

    fn advanced_provider(&self, lane: Lane) -> Value {
        if self.kind == ProviderKind::Disabled {
            return Value::Null;
        }
        if self.kind == ProviderKind::Command {
            return json!({"kind":"command", "command":self.command});
        }
        let mut value = json!({"kind":self.kind, "endpoint":self.endpoint,
            "model":self.model, "key_env":self.credential_env(lane)});
        if lane != Lane::Decider {
            value["streaming"] = json!(lane == Lane::Fast);
            value["max_tokens"] = json!(if lane == Lane::Planner { 1200 } else { 600 });
        }
        value
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connections {
    pub enabled: bool,
    pub python_executable: String,
    pub microphone: bool,
    pub system_audio: bool,
    pub diarization: bool,
    pub transcription: Connection,
    pub fast: Connection,
    pub planner: Connection,
    pub decider: Connection,
}

impl Connections {
    pub fn get(&self, lane: Lane) -> &Connection {
        match lane {
            Lane::Transcription => &self.transcription,
            Lane::Fast => &self.fast,
            Lane::Planner => &self.planner,
            Lane::Decider => &self.decider,
        }
    }

    pub fn get_mut(&mut self, lane: Lane) -> &mut Connection {
        match lane {
            Lane::Transcription => &mut self.transcription,
            Lane::Fast => &mut self.fast,
            Lane::Planner => &mut self.planner,
            Lane::Decider => &mut self.decider,
        }
    }

    pub fn advanced(&self) -> bool {
        self.planner.kind != ProviderKind::Disabled || self.decider.kind != ProviderKind::Disabled
    }

    pub fn lane_active(&self, lane: Lane) -> bool {
        self.enabled
            && self.get(lane).kind != ProviderKind::Disabled
            && (lane != Lane::Transcription || self.microphone || self.system_audio)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !bounded(&self.python_executable, 4096) {
            return Err("Python executable path exceeds supported bounds".into());
        }
        for lane in ALL {
            self.get(lane).validate_fields()?;
        }
        if serde_json::to_vec_pretty(self)
            .map_err(|_| "Cannot serialize connection preferences")?
            .len()
            > 24 * 1024
        {
            return Err("Connection preferences exceed 24 KiB".into());
        }
        if !self.enabled {
            return Ok(());
        }
        for lane in ALL {
            self.get(lane).validate_active(lane)?;
        }
        for lane in ALL {
            let connection = self.get(lane);
            if self.lane_active(lane) && connection.auth == AuthMode::Environment {
                for other in ALL {
                    let same_name = if cfg!(windows) {
                        connection.key_env.eq_ignore_ascii_case(other.env_name())
                    } else {
                        connection.key_env == other.env_name()
                    };
                    if lane != other
                        && self.lane_active(other)
                        && self.get(other).auth == AuthMode::Vault
                        && same_name
                    {
                        return Err("An environment credential conflicts with another active role's saved-key variable".into());
                    }
                }
            }
        }
        if (self.microphone || self.system_audio)
            && self.transcription.kind == ProviderKind::Disabled
        {
            return Err("Select a transcription provider before enabling audio capture".into());
        }
        if self.advanced() {
            let python = Path::new(&self.python_executable);
            if !python.is_absolute() || !python.is_file() {
                return Err(
                    "Advanced lanes require an existing Python 3.11+ executable's absolute path"
                        .into(),
                );
            }
        }
        Ok(())
    }

    /// Keys are scoped to this profile, lane and actual connection destination.
    /// Editing the endpoint or command requires an independently stored key.
    pub fn credential_account(&self, settings_path: &Path, lane: Lane) -> String {
        let connection = self.get(lane);
        let path = normalized_path(settings_path);
        let identity = serde_json::to_vec(&(
            path.as_os_str().as_encoded_bytes(),
            lane,
            connection.kind,
            &connection.endpoint,
            &connection.command,
        ))
        .expect("Serializable connection identity");
        format!("connection-{:x}", Sha256::digest(identity))
    }

    /// Publish immutable, content-addressed runtime configurations without secrets.
    /// A draft is fully written before its name becomes visible to a launcher.
    pub fn write_runtime_config(&self, settings_path: &Path) -> Result<PathBuf, String> {
        self.validate()?;
        if !self.enabled {
            return Err(
                "Enable managed connections before creating a runtime configuration".into(),
            );
        }
        let settings_path = normalized_path(settings_path);
        let parent = settings_path
            .parent()
            .ok_or("Settings directory is unavailable")?;
        let identity = Sha256::digest(settings_path.as_os_str().as_encoded_bytes());
        let directory = parent.join(format!(".screencue-runtime-{identity:x}"));
        fs::create_dir_all(&directory)
            .map_err(|_| "Cannot create managed configuration directory")?;
        let advanced = self.advanced();
        let native = json!({"capture":{"microphone":self.microphone, "system_audio":self.system_audio},
            "transcription":self.transcription.native_provider(Lane::Transcription, self.diarization),
            "suggestion":if advanced { Value::Null } else { self.fast.native_provider(Lane::Fast, false) },
            "context":"Offer concise, grounded speaking cues. Do not invent facts about the assisted person."});
        if !advanced {
            return publish(&directory, "native", &native);
        }
        let helper = if self.microphone || self.system_audio {
            Some(publish(&directory, "native", &native)?)
        } else {
            None
        };
        publish(
            &directory,
            "conversation",
            &json!({"engine":"conversation-orchestrator",
            "python_executable":self.python_executable, "helper_config":helper,
            "fast":self.fast.advanced_provider(Lane::Fast),
            "planner":self.planner.advanced_provider(Lane::Planner),
            "decider":self.decider.advanced_provider(Lane::Decider)}),
        )
    }
}

fn bounded(value: &str, bytes: usize) -> bool {
    value.len() <= bytes && !value.chars().any(char::is_control)
}

fn valid_env_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(c) if c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn validate_endpoint(value: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).map_err(|_| "Invalid provider endpoint")?;
    let loopback = match parsed.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if !(parsed.scheme() == "https" || parsed.scheme() == "http" && loopback)
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "Use HTTPS or loopback HTTP without URL credentials, queries or fragments".into(),
        );
    }
    Ok(())
}

fn normalized_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn publish(directory: &Path, prefix: &str, value: &Value) -> Result<PathBuf, String> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|_| "Cannot serialize runtime configuration")?;
    bytes.push(b'\n');
    let path = directory.join(format!("{prefix}-{:x}.json", Sha256::digest(&bytes)));
    let matches = |path: &Path| -> Result<bool, String> {
        let mut existing = Vec::new();
        fs::File::open(path)
            .map_err(|_| "Cannot read managed runtime configuration")?
            .take((bytes.len() + 1) as u64)
            .read_to_end(&mut existing)
            .map_err(|_| "Cannot read managed runtime configuration")?;
        Ok(existing == bytes)
    };
    if path.exists() {
        return if matches(&path)? {
            Ok(path)
        } else {
            Err("Managed runtime configuration was modified; it will not be overwritten".into())
        };
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".draft-{}-{}",
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
            .map_err(|_| "Cannot create runtime configuration draft")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot save runtime configuration draft")?;
        drop(file);
        match fs::hard_link(&temporary, &path) {
            Ok(()) => Ok(path.clone()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && matches(&path)? => {
                Ok(path.clone())
            }
            Err(_) => Err("Cannot atomically publish runtime configuration".into()),
        }
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            Self(
                std::env::temp_dir()
                    .join(format!(
                        "screencue-connections-{}-{}-{}",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos(),
                        SEQUENCE.fetch_add(1, Ordering::Relaxed)
                    ))
                    .join("settings.json"),
            )
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0.parent().unwrap());
        }
    }
    fn api() -> Connection {
        Connection {
            kind: ProviderKind::OpenaiCompatible,
            endpoint: "http://127.0.0.1:8080/api".into(),
            model: "configured-model".into(),
            auth: AuthMode::Vault,
            ..Connection::default()
        }
    }
    fn read(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn defaults_are_offline_and_old_preferences_are_compatible() {
        let connections: Connections = serde_json::from_str("{}").unwrap();
        connections.validate().unwrap();
        assert!(
            !connections.enabled
                && !connections.microphone
                && !connections.system_audio
                && !connections.diarization
        );
        assert!(ALL
            .iter()
            .all(|lane| connections.get(*lane).kind == ProviderKind::Disabled));
        assert!(connections.write_runtime_config(&Profile::new().0).is_err());
        assert!(serde_json::from_str::<Connection>(r#"{"api_key":"secret"}"#).is_err());
    }

    #[test]
    fn native_runtime_routes_separate_key_names_and_reuses_immutable_config() {
        let profile = Profile::new();
        let config = Connections {
            enabled: true,
            microphone: true,
            diarization: true,
            transcription: api(),
            fast: api(),
            ..Connections::default()
        };
        let path = config.write_runtime_config(&profile.0).unwrap();
        let value = read(&path);
        assert_eq!(
            value["transcription"]["key_env"],
            Lane::Transcription.env_name()
        );
        assert_eq!(value["suggestion"]["key_env"], Lane::Fast.env_name());
        assert_eq!(value["transcription"]["response_format"], "diarized_json");
        assert_eq!(value["suggestion"]["timeout_ms"], 1800);
        assert!(value.get("engine").is_none());
        assert_eq!(config.write_runtime_config(&profile.0).unwrap(), path);
        fs::write(&path, b"modified by user").unwrap();
        assert!(config.write_runtime_config(&profile.0).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"modified by user");
    }

    #[test]
    fn advanced_runtime_disables_duplicate_suggestions_and_preserves_typed_adapter() {
        let profile = Profile::new();
        let config = Connections {
            enabled: true,
            system_audio: true,
            transcription: api(),
            fast: api(),
            planner: api(),
            python_executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            decider: Connection {
                kind: ProviderKind::Typesafe,
                endpoint: "https://example.com/decide".into(),
                model: "jev".into(),
                auth: AuthMode::Vault,
                ..Connection::default()
            },
            ..Connections::default()
        };
        let value = read(&config.write_runtime_config(&profile.0).unwrap());
        assert_eq!(value["engine"], "conversation-orchestrator");
        assert_eq!(value["fast"]["streaming"], true);
        assert_eq!(value["planner"]["streaming"], false);
        assert_eq!(value["decider"]["kind"], "typesafe");
        assert_eq!(value["decider"]["key_env"], Lane::Decider.env_name());
        let native = read(Path::new(value["helper_config"].as_str().unwrap()));
        assert!(native["suggestion"].is_null());
        assert_eq!(native["capture"]["system_audio"], true);
    }

    #[test]
    fn command_bridges_and_external_transcripts_do_not_enable_capture() {
        let profile = Profile::new();
        let bridge = Connection {
            kind: ProviderKind::Command,
            command: vec!["local-wrapper".into(), "--profile".into(), "test".into()],
            ..Connection::default()
        };
        let config = Connections {
            enabled: true,
            fast: bridge.clone(),
            planner: bridge,
            python_executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            ..Connections::default()
        };
        let value = read(&config.write_runtime_config(&profile.0).unwrap());
        assert!(value["helper_config"].is_null());
        assert_eq!(value["fast"]["command"][0], "local-wrapper");
        assert!(value["fast"].get("key_env").is_none());
    }

    #[test]
    fn rejects_bad_routing_endpoints_audio_and_environment_names_without_echoing_values() {
        let mut config = Connections {
            enabled: true,
            fast: api(),
            ..Connections::default()
        };
        for endpoint in [
            "http://remote.example/api",
            "https://user:secret@example.com/api",
            "https://example.com/api?key=secret",
            "http://127.0.0.1.example/api",
        ] {
            config.fast.endpoint = endpoint.into();
            let error = config.validate().unwrap_err();
            assert!(!error.contains("secret") && !error.contains(endpoint));
        }
        config.fast = api();
        config.fast.kind = ProviderKind::Typesafe;
        assert!(config.validate().is_err());
        config.fast = Connection::default();
        config.microphone = true;
        assert!(config.validate().is_err());
        config.microphone = false;
        config.fast = api();
        config.fast.auth = AuthMode::Environment;
        config.fast.key_env = "SCREENCUE_TEST_NOT_REQUIRED_ON_SAVE".into();
        config.validate().unwrap();
        config.fast.key_env = "literal-key-with-dashes".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn credential_identity_is_stable_and_bound_to_profile_lane_and_destination() {
        let profile = Profile::new();
        let mut config = Connections {
            fast: api(),
            ..Connections::default()
        };
        let account = config.credential_account(&profile.0, Lane::Fast);
        assert!(account.len() <= 128 && account.is_ascii());
        assert_eq!(config.credential_account(&profile.0, Lane::Fast), account);
        config.fast.model = "another-model".into();
        assert_eq!(config.credential_account(&profile.0, Lane::Fast), account);
        assert_ne!(
            config.credential_account(&profile.0.with_file_name("other.json"), Lane::Fast),
            account
        );
        assert_ne!(
            config.credential_account(&profile.0, Lane::Planner),
            account
        );
        config.fast.endpoint = "https://different.example/api".into();
        assert_ne!(config.credential_account(&profile.0, Lane::Fast), account);
        let endpoint_account = config.credential_account(&profile.0, Lane::Fast);
        config.fast.kind = ProviderKind::Command;
        config.fast.command = vec!["bridge".into()];
        assert_ne!(
            config.credential_account(&profile.0, Lane::Fast),
            endpoint_account
        );
        let command_account = config.credential_account(&profile.0, Lane::Fast);
        config.fast.command.push("--other-destination".into());
        assert_ne!(
            config.credential_account(&profile.0, Lane::Fast),
            command_account
        );
    }

    #[test]
    fn environment_credentials_cannot_alias_another_active_vault_role_including_bridges() {
        let bridge = Connection {
            kind: ProviderKind::Command,
            command: vec!["local-wrapper".into()],
            auth: AuthMode::Vault,
            ..Connection::default()
        };
        let mut config = Connections {
            enabled: true,
            fast: bridge.clone(),
            planner: bridge,
            python_executable: std::env::current_exe().unwrap().to_str().unwrap().into(),
            ..Connections::default()
        };
        config.planner.auth = AuthMode::Environment;
        config.planner.key_env = Lane::Fast.env_name().into();
        let error = config.validate().unwrap_err();
        assert!(!error.contains(Lane::Fast.env_name()));
        config.fast.kind = ProviderKind::Disabled;
        config.validate().unwrap();
        config.transcription = api();
        config.planner.key_env = Lane::Transcription.env_name().into();
        assert!(!config.lane_active(Lane::Transcription));
        config.validate().unwrap();
        config.microphone = true;
        assert!(config.lane_active(Lane::Transcription));
        assert!(config.validate().is_err());
    }
}
