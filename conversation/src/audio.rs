use crate::config::Capture;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;

pub struct AudioChunk {
    pub source: &'static str,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
    pub captured_at: Instant,
}

pub struct Segmenter {
    samples: Vec<i16>,
    sample_rate: u32,
    max_samples: usize,
    silence_samples: usize,
    quiet: usize,
    spoken: bool,
    threshold: f32,
}

impl Segmenter {
    pub fn new(sample_rate: u32, settings: &Capture) -> Self {
        let max_samples = (sample_rate as u64 * settings.chunk_ms as u64 / 1000) as usize;
        Self {
            samples: Vec::with_capacity(max_samples),
            sample_rate,
            max_samples,
            silence_samples: (sample_rate as u64 * settings.silence_ms as u64 / 1000) as usize,
            quiet: 0,
            spoken: false,
            threshold: settings.silence_threshold,
        }
    }

    pub fn push(&mut self, value: f32) -> Option<Vec<i16>> {
        let value = if value.is_finite() {
            value.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        if value.abs() >= self.threshold {
            self.spoken = true;
            self.quiet = 0;
        } else {
            self.quiet += 1;
        }
        self.samples.push((value * 32767.0) as i16);
        let utterance_ended = self.spoken
            && self.quiet >= self.silence_samples
            && self.samples.len() >= self.sample_rate as usize / 3;
        if self.samples.len() >= self.max_samples || utterance_ended {
            let spoken = self.spoken;
            self.spoken = false;
            self.quiet = 0;
            let result = std::mem::replace(&mut self.samples, Vec::with_capacity(self.max_samples));
            return spoken.then_some(result);
        }
        None
    }
}

fn device_by_id(host: &cpal::Host, id: &str) -> Result<cpal::Device, String> {
    let id = id.parse().map_err(|_| "Invalid audio device ID")?;
    host.device_by_id(&id)
        .ok_or_else(|| "Configured audio device was not found".into())
}

pub fn list_devices() -> Result<serde_json::Value, String> {
    let host = cpal::default_host();
    let devices = host
        .devices()
        .map_err(|_| "Could not enumerate audio devices")?;
    let mut result = Vec::new();
    for device in devices.take(256) {
        let id = device
            .id()
            .map_err(|_| "Could not read an audio device ID")?
            .to_string();
        let description = device
            .description()
            .map_err(|_| "Could not read an audio device description")?;
        result.push(serde_json::json!({"id":id,"name":description.name(),"input":device.supports_input(),"output":device.supports_output()}));
    }
    Ok(serde_json::json!({"devices":result,"capture_started":false}))
}

pub fn start(
    settings: &Capture,
    send: SyncSender<AudioChunk>,
    stopped: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
) -> Result<Vec<cpal::Stream>, String> {
    let host = cpal::default_host();
    let mut selected = Vec::new();
    if settings.microphone {
        let device = if let Some(id) = &settings.microphone_device {
            device_by_id(&host, id)?
        } else {
            host.default_input_device()
                .ok_or("No default microphone is available")?
        };
        let config = device
            .default_input_config()
            .map_err(|_| "Microphone format is unavailable")?;
        selected.push(("microphone", device, config));
    }
    if settings.system_audio {
        let (device, config) = if let Some(id) = &settings.system_audio_device {
            let device = device_by_id(&host, id)?;
            let config = device
                .default_input_config()
                .map_err(|_| "Configured system-audio input is unavailable")?;
            (device, config)
        } else {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                let device = host
                    .default_output_device()
                    .ok_or("No default audio output is available")?;
                let config = device
                    .default_output_config()
                    .map_err(|_| "System-audio loopback format is unavailable")?;
                (device, config)
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                return Err(
                    "System audio requires a configured monitor input device on this platform"
                        .into(),
                );
            }
        };
        selected.push(("system_audio", device, config));
    }
    let mut streams = Vec::new();
    for (source, device, config) in selected {
        let stream = build_stream(
            &device,
            config,
            settings,
            source,
            send.clone(),
            stopped.clone(),
            failed.clone(),
        )?;
        streams.push(stream);
    }
    // All requested devices must open successfully before either starts.
    for stream in &streams {
        stream.play().map_err(|_| {
            "An audio device could not start; check permissions and device availability"
        })?;
    }
    Ok(streams)
}

fn build_stream(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    settings: &Capture,
    source: &'static str,
    send: SyncSender<AudioChunk>,
    stopped: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;
    if !(8000..=192000).contains(&sample_rate) || !(1..=32).contains(&channels) {
        return Err("Audio device format exceeds supported capture bounds".into());
    }
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let mut segmenter = Segmenter::new(sample_rate, settings);
    let mut accept = move |value: f32| {
        if !stopped.load(Ordering::Acquire) {
            if let Some(samples) = segmenter.push(value) {
                // Never block the real-time audio callback on a model or a full queue.
                let _ = send.try_send(AudioChunk {
                    source,
                    sample_rate,
                    samples,
                    captured_at: Instant::now(),
                });
            }
        }
    };
    let error = move |_: cpal::Error| {
        failed.store(true, Ordering::Release);
    };
    macro_rules! stream {
        ($sample:ty, $convert:expr) => {
            device.build_input_stream(
                config,
                move |data: &[$sample], _: &_| {
                    for frame in data.chunks_exact(channels) {
                        let value = frame.iter().map($convert).sum::<f32>() / channels as f32;
                        accept(value);
                    }
                },
                error,
                None,
            )
        };
    }
    let stream = match format {
        cpal::SampleFormat::F32 => stream!(f32, |value: &f32| *value),
        cpal::SampleFormat::F64 => stream!(f64, |value: &f64| *value as f32),
        cpal::SampleFormat::I16 => stream!(i16, |value: &i16| *value as f32 / 32768.0),
        cpal::SampleFormat::I32 => stream!(i32, |value: &i32| *value as f32 / 2147483648.0),
        cpal::SampleFormat::U16 => stream!(u16, |value: &u16| (*value as f32 - 32768.0) / 32768.0),
        _ => {
            return Err(
                "Audio device sample format is not supported; choose a standard PCM input".into(),
            )
        }
    };
    stream.map_err(|_| {
        "Could not open an audio device; check permissions, format and loopback support".into()
    })
}

pub fn wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_length = samples.len() as u32 * 2;
    let mut bytes = Vec::with_capacity(44 + data_length as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_length).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_length.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_pcm_header_and_exact_samples() {
        let bytes = wav(&[-32768, 0, 32767], 48000);
        assert_eq!(bytes.len(), 50);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[24..28], &48000u32.to_le_bytes());
        assert_eq!(&bytes[40..44], &6u32.to_le_bytes());
        assert_eq!(&bytes[44..], &[0, 128, 0, 0, 255, 127]);
    }

    #[test]
    fn silence_is_not_transcribed_and_utterance_ends_after_pause() {
        let mut seg = Segmenter::new(16000, &Capture::default());
        for _ in 0..48000 {
            assert!(seg.push(0.0).is_none());
        }
        for _ in 0..4000 {
            assert!(seg.push(0.1).is_none());
        }
        let mut chunks = Vec::new();
        for _ in 0..6000 {
            if let Some(chunk) = seg.push(0.0) {
                chunks.push(chunk);
            }
        }
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].len() < 16000 * 2);
    }

    #[test]
    fn continuous_speech_is_bounded_and_nonfinite_samples_are_safe() {
        let mut settings = Capture::default();
        settings.chunk_ms = 500;
        let mut seg = Segmenter::new(16000, &settings);
        for _ in 0..7999 {
            assert!(seg.push(0.2).is_none());
        }
        assert_eq!(seg.push(f32::NAN).unwrap().len(), 8000);
    }
}
