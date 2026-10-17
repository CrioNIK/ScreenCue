use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub capture: Capture,
    pub transcription: Option<Provider>,
    pub suggestion: Option<Provider>,
    pub context: String,
    pub speaker_roles: BTreeMap<String, String>,
    pub source_roles: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Capture {
    pub microphone: bool,
    pub system_audio: bool,
    /// Optional CPAL input-device ID, including a configured Linux monitor.
    pub microphone_device: Option<String>,
    /// Optional input-device ID instead of native output-device loopback.
    pub system_audio_device: Option<String>,
    pub chunk_ms: u32,
    pub silence_ms: u32,
    pub silence_threshold: f32,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            microphone: false,
            system_audio: false,
            microphone_device: None,
            system_audio_device: None,
            chunk_ms: 2400,
            silence_ms: 350,
            silence_threshold: 0.006,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Provider {
    OpenaiCompatible {
        endpoint: String,
        model: String,
        #[serde(default)]
        key_env: String,
        /// Used only for transcription; diarization requires a compatible model.
        #[serde(default)]
        response_format: Option<String>,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    Command {
        command: Vec<String>,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
}

fn default_timeout() -> u64 {
    8000
}

impl Provider {
    pub fn timeout_ms(&self) -> u64 {
        match self {
            Self::OpenaiCompatible { timeout_ms, .. } | Self::Command { timeout_ms, .. } => {
                *timeout_ms
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(500..=30_000).contains(&self.timeout_ms()) {
            return Err("Provider timeout_ms must be between 500 and 30000".into());
        }
        match self {
            Self::OpenaiCompatible {
                endpoint,
                model,
                key_env,
                response_format,
                ..
            } => {
                validate_endpoint(endpoint)?;
                if model.trim().is_empty()
                    || model.len() > 200
                    || model.chars().any(char::is_control)
                {
                    return Err("Provider model must contain 1 to 200 characters".into());
                }
                if !key_env.is_empty()
                    && (key_env.len() > 128
                        || !key_env
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                        || key_env.as_bytes()[0].is_ascii_digit())
                {
                    return Err("key_env must be an environment variable name, not a key".into());
                }
                if response_format
                    .as_ref()
                    .is_some_and(|format| !["json", "diarized_json"].contains(&format.as_str()))
                {
                    return Err("response_format must be json or diarized_json".into());
                }
            }
            Self::Command { command, .. } => {
                if command.is_empty()
                    || command.len() > 32
                    || command.iter().any(|v| v.len() > 4096 || v.contains('\0'))
                    || command[0].is_empty()
                {
                    return Err("A command requires an executable and at most 31 arguments".into());
                }
            }
        }
        Ok(())
    }
}

pub fn validate_endpoint(value: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).map_err(|_| "Invalid provider endpoint")?;
    let loopback = match parsed.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback))
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
        || value.len() > 2048
    {
        return Err("Use HTTPS or loopback HTTP; endpoint credentials, queries and fragments are not accepted".into());
    }
    Ok(())
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(500..=6000).contains(&self.capture.chunk_ms)
            || !(100..=2000).contains(&self.capture.silence_ms)
            || !self.capture.silence_threshold.is_finite()
            || !(0.0..=0.2).contains(&self.capture.silence_threshold)
        {
            return Err("Capture timing or silence threshold is outside supported bounds".into());
        }
        for device in [
            &self.capture.microphone_device,
            &self.capture.system_audio_device,
        ] {
            if device
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 4096)
            {
                return Err("Device IDs must contain 1 to 4096 characters".into());
            }
        }
        if (self.capture.microphone || self.capture.system_audio) && self.transcription.is_none() {
            return Err("Select a transcription provider before enabling audio capture".into());
        }
        if self.context.chars().count() > 16000 {
            return Err("Conversation context must not exceed 16000 characters".into());
        }
        for mapping in [&self.speaker_roles, &self.source_roles] {
            if mapping.len() > 64
                || mapping.iter().any(|(id, role)| {
                    id.is_empty()
                        || id.len() > 128
                        || role.is_empty()
                        || role.len() > 200
                        || id.chars().any(char::is_control)
                        || role.chars().any(char::is_control)
                })
            {
                return Err("Speaker and source role mappings exceed supported bounds".into());
            }
        }
        if self
            .source_roles
            .keys()
            .any(|source| !["microphone", "system_audio", "external"].contains(&source.as_str()))
        {
            return Err("source_roles accepts microphone, system_audio and external only".into());
        }
        for provider in [&self.transcription, &self.suggestion]
            .into_iter()
            .flatten()
        {
            provider.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_offline_and_do_not_capture() {
        let cfg = Config::default();
        cfg.validate().unwrap();
        assert!(!cfg.capture.microphone && !cfg.capture.system_audio);
        assert!(cfg.transcription.is_none() && cfg.suggestion.is_none());
    }

    #[test]
    fn rejects_unsafe_endpoints_and_inline_credentials() {
        for endpoint in [
            "http://example.com/v1/chat/completions",
            "https://user:password@example.com/api",
            "https://example.com/api?key=private",
            "file:///etc/passwd",
            "http://127.0.0.1.example.com/api",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
        for endpoint in [
            "https://example.com/v1/chat/completions",
            "http://localhost:8080/v1/audio/transcriptions",
            "http://[::1]:8080/v1/chat/completions",
        ] {
            assert!(validate_endpoint(endpoint).is_ok(), "{endpoint}");
        }
    }

    #[test]
    fn audio_requires_transcription_and_unknown_fields_are_rejected() {
        let mut cfg = Config::default();
        cfg.capture.system_audio = true;
        assert!(cfg.validate().is_err());
        assert!(serde_json::from_str::<Config>(r#"{"api_key":"secret"}"#).is_err());
    }
}
