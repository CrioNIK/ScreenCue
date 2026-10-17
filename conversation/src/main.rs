mod audio;
mod config;
mod providers;

use audio::AudioChunk;
use config::Config;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const MAX_CONFIG: usize = 128 * 1024;
const MAX_INPUT: usize = 128 * 1024;

struct ExternalTranscript {
    source: String,
    segments: Vec<providers::Transcript>,
}

#[derive(Default)]
struct QuickState {
    generation: u64,
    latest: Option<(u64, String)>,
}

type QuickQueue = Arc<(Mutex<QuickState>, Condvar)>;

#[derive(Clone)]
struct Events {
    output: Arc<Mutex<io::Stdout>>,
    stopped: Arc<AtomicBool>,
}

impl Events {
    fn new(stopped: Arc<AtomicBool>) -> Self {
        Self {
            output: Arc::new(Mutex::new(io::stdout())),
            stopped,
        }
    }

    fn send(&self, value: Value) {
        let mut output = self.output.lock().unwrap();
        if !self.stopped.load(Ordering::Acquire) {
            let _ = writeln!(output, "{value}");
            let _ = output.flush();
        }
    }

    fn error(&self, stage: &str, message: &str) {
        self.send(json!({"event":"error","stage":stage,"message":message}));
    }

    fn finish(&self) {
        let mut output = self.output.lock().unwrap();
        self.stopped.store(true, Ordering::Release);
        let _ = writeln!(output, "{}", json!({"event":"status","state":"stopped"}));
        let _ = output.flush();
    }
}

fn load(path: &str) -> Result<Config, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "Could not open conversation configuration")?
        .take(MAX_CONFIG as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read conversation configuration")?;
    if bytes.len() > MAX_CONFIG {
        return Err("Conversation configuration exceeds 128 KiB".into());
    }
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|_| "Conversation configuration is not valid or has unknown fields")?;
    config.validate()?;
    Ok(config)
}

fn bounded_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let read = reader
        .take(MAX_INPUT as u64 + 1)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_INPUT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Input exceeds 128 KiB",
        ));
    }
    Ok(Some(bytes))
}

fn history_append(history: &mut VecDeque<String>, source: &str, text: &str) -> String {
    history.push_back(format!("{source}: {text}"));
    while history
        .iter()
        .map(|line| line.chars().count())
        .sum::<usize>()
        > 16000
        && history.len() > 1
    {
        history.pop_front();
    }
    history.iter().cloned().collect::<Vec<_>>().join("\n")
}

fn process_transcripts(
    config: &Config,
    source: &str,
    transcripts: &[providers::Transcript],
    history: &mut VecDeque<String>,
    events: &Events,
    quick: &QuickQueue,
) {
    if transcripts.is_empty() || events.stopped.load(Ordering::Acquire) {
        return;
    }
    let mut pending = quick.0.lock().unwrap();
    pending.generation = pending.generation.wrapping_add(1);
    let mut recent = String::new();
    for transcript in transcripts {
        let role = transcript
            .speaker_id
            .as_ref()
            .and_then(|id| {
                config
                    .speaker_roles
                    .get(&format!("{source}:{id}"))
                    .or_else(|| config.speaker_roles.get(id))
            })
            .or_else(|| config.source_roles.get(source));
        events.send(json!({"event":"transcript","source":source,"text":transcript.text,"speaker_id":transcript.speaker_id,"speaker_role":role}));
        let label = match (&transcript.speaker_id, role) {
            (Some(id), Some(role)) => format!("{source}/{id} ({role})"),
            (Some(id), None) => format!("{source}/{id}"),
            (None, Some(role)) => format!("{source} ({role})"),
            (None, None) => source.into(),
        };
        recent = history_append(history, &label, &transcript.text);
    }
    if config.suggestion.is_some() {
        pending.latest = Some((pending.generation, recent));
        quick.1.notify_one();
    }
}

fn quick_worker(config: Config, events: Events, quick: QuickQueue) {
    let Some(provider) = config.suggestion else {
        return;
    };
    while !events.stopped.load(Ordering::Acquire) {
        let mut pending = quick.0.lock().unwrap();
        if pending.latest.is_none() {
            pending = quick
                .1
                .wait_timeout(pending, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        let Some((generation, history)) = pending.latest.take() else {
            continue;
        };
        drop(pending);
        if events.stopped.load(Ordering::Acquire) {
            break;
        }
        let result = providers::suggest(&provider, &config.context, &history, &events.stopped);
        let pending = quick.0.lock().unwrap();
        if generation != pending.generation || events.stopped.load(Ordering::Acquire) {
            continue;
        }
        match result {
            Ok(text) if !text.is_empty() => {
                events.send(json!({"event":"hint","title":"Suggested words","text":text}))
            }
            Ok(_) => {}
            Err(error) => events.error("suggestion", &error),
        }
    }
}

fn run(config: Config) -> Result<(), String> {
    let stopped = Arc::new(AtomicBool::new(false));
    let events = Events::new(stopped.clone());
    let quick: QuickQueue = Arc::new((Mutex::new(QuickState::default()), Condvar::new()));
    let capture_failed = Arc::new(AtomicBool::new(false));
    let (audio_send, audio_receive) = mpsc::sync_channel::<AudioChunk>(2);
    let (text_send, text_receive) = mpsc::sync_channel::<ExternalTranscript>(1);
    let (stop_send, stop_receive) = mpsc::sync_channel::<()>(1);
    let input_events = events.clone();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let bytes = match bounded_line(&mut input) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => break,
                Err(_) => {
                    input_events.error("input", "Input is unreadable or exceeds 128 KiB");
                    break;
                }
            };
            let command: Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_) => {
                    input_events.error("input", "Expected one JSON command per line");
                    continue;
                }
            };
            match command.get("command").and_then(Value::as_str) {
                Some("stop") => break,
                Some("transcript") => {
                    match providers::transcripts(&command) {
                        Ok(segments) if !segments.is_empty() => {
                            let source = command
                                .get("source")
                                .and_then(Value::as_str)
                                .unwrap_or("external");
                            if !["microphone", "system_audio", "external"].contains(&source) {
                                input_events.error("input", "Transcript source must be microphone, system_audio or external");
                            } else if text_send
                                .try_send(ExternalTranscript {
                                    source: source.into(),
                                    segments,
                                })
                                .is_err()
                            {
                                input_events.error("input", "A transcript is already pending; wait for the current response");
                            }
                        }
                        Ok(_) => {}
                        Err(error) => input_events.error("input", &error),
                    }
                }
                _ => input_events.error("input", "Expected stop or transcript command"),
            }
        }
        let _ = stop_send.try_send(());
    });
    let streams = match audio::start(
        &config.capture,
        audio_send,
        stopped.clone(),
        capture_failed.clone(),
    ) {
        Ok(streams) => streams,
        Err(error) => {
            events.error("capture", &error);
            events.finish();
            return Ok(());
        }
    };
    events.send(json!({"event":"status","state":"listening","microphone":config.capture.microphone,"system_audio":config.capture.system_audio}));
    let quick_events = events.clone();
    let quick_config = config.clone();
    let quick_queue = quick.clone();
    let quick_thread = thread::spawn(move || quick_worker(quick_config, quick_events, quick_queue));
    let worker_events = events.clone();
    let worker = thread::spawn(move || {
        let mut history = VecDeque::new();
        while !worker_events.stopped.load(Ordering::Acquire) {
            if let Ok(transcript) = text_receive.try_recv() {
                process_transcripts(
                    &config,
                    &transcript.source,
                    &transcript.segments,
                    &mut history,
                    &worker_events,
                    &quick,
                );
                continue;
            }
            let chunk = match audio_receive.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => chunk,
                Err(_) => {
                    thread::sleep(Duration::from_millis(25));
                    continue;
                }
            };
            // Backpressure favors recent conversation rather than presenting a delayed answer.
            if chunk.captured_at.elapsed() > Duration::from_secs(10) {
                continue;
            }
            if let Some(provider) = &config.transcription {
                if worker_events.stopped.load(Ordering::Acquire) {
                    break;
                }
                match providers::transcribe(provider, &chunk, &worker_events.stopped) {
                    Ok(transcripts) => process_transcripts(
                        &config,
                        chunk.source,
                        &transcripts,
                        &mut history,
                        &worker_events,
                        &quick,
                    ),
                    Err(error) => worker_events.error("transcription", &error),
                }
            }
        }
    });
    loop {
        if stop_receive.recv_timeout(Duration::from_millis(25)).is_ok() {
            break;
        }
        if capture_failed.load(Ordering::Acquire) {
            events.error(
                "capture",
                "An audio device stopped unexpectedly; start again after checking the device",
            );
            break;
        }
    }
    // Drop native streams before acknowledging stop. No audio files are written.
    stopped.store(true, Ordering::Release);
    drop(streams);
    events.finish();
    let _ = worker.join();
    let _ = quick_thread.join();
    Ok(())
}

fn execute(args: &[String]) -> Result<(), String> {
    match args {
        [flag] if flag == "--version" => { println!("screencue-listen {}", env!("CARGO_PKG_VERSION")); Ok(()) }
        [flag] if flag == "--list-devices" => { println!("{}", audio::list_devices()?); Ok(()) }
        [flag, path] if flag == "--validate-config" || flag == "--check-config" => {
            load(path)?;
            println!("{}", json!({"valid":true,"capture_started":false,"provider_contacted":false}));
            Ok(())
        }
        [flag, path] if flag == "--write-example" => {
            let mut example = Config::default();
            example.capture.microphone = true;
            example.capture.system_audio = true;
            example.context = "Add your own accurate facts, goals and preferred answer language. Configure transcription and suggestion providers before starting.".into();
            let mut file = OpenOptions::new().create_new(true).write(true).open(path).map_err(|_| "Could not create example configuration; existing files are never overwritten")?;
            file.write_all(serde_json::to_string_pretty(&example).unwrap().as_bytes()).map_err(|_| "Could not write example configuration")?;
            println!("{}", json!({"created":true,"configured":false,"capture_started":false}));
            Ok(())
        }
        [config_flag, path, start] if config_flag == "--config" && start == "--start" => run(load(path)?),
        _ => Err("Usage: screencue-listen --config PATH --start | --validate-config PATH | --write-example PATH | --list-devices | --version".into()),
    }
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Err(error) = execute(&args) {
        println!(
            "{}",
            json!({"event":"error","stage":"config","message":error})
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_input_does_not_allocate_unlimited_lines() {
        assert!(bounded_line(&mut io::Cursor::new(vec![b'x'; MAX_INPUT + 2])).is_err());
        assert_eq!(
            bounded_line(&mut io::Cursor::new(b"{}\n"))
                .unwrap()
                .unwrap(),
            b"{}\n"
        );
    }

    #[test]
    fn history_drops_old_context_and_keeps_latest_turn() {
        let mut history = VecDeque::new();
        history_append(&mut history, "system_audio", &"a".repeat(12000));
        let result = history_append(&mut history, "microphone", &"b".repeat(6000));
        assert!(!result.contains('a'));
        assert!(result.starts_with("microphone:"));
        assert_eq!(history.len(), 1);
    }
}
