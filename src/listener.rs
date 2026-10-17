//! Optional audio helper process. No capture or model calls occur in the renderer.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
static REAPERS: Mutex<Vec<thread::JoinHandle<()>>> = Mutex::new(Vec::new());

/// Called after the UI event loop exits, so a shutdown cannot abandon its helper.
pub fn finish_shutdown() {
    let pending = std::mem::take(&mut *REAPERS.lock().unwrap_or_else(|e| e.into_inner()));
    for reaper in pending {
        let _ = reaper.join();
    }
}

pub fn executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = current.with_file_name(if cfg!(windows) {
        "screencue-listen.exe"
    } else {
        "screencue-listen"
    });
    if !path.is_file() {
        return Err(
            "Conversation helper is missing. Install the full ScreenCue app package.".into(),
        );
    }
    Ok(path)
}
fn command() -> Result<Command, String> {
    let mut command = Command::new(executable()?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}
fn launch_command(path: &Path) -> Result<Command, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Cannot open conversation configuration")?
        .take(131073)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read conversation configuration")?;
    if bytes.len() > 131072 {
        return Err("Conversation configuration exceeds 128 KiB".into());
    }
    let config: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid conversation configuration JSON")?;
    if config["engine"] != "conversation-orchestrator" {
        return command();
    }
    let python=config["python_executable"].as_str().filter(|p|Path::new(p).is_absolute()&&Path::new(p).is_file()).ok_or("Advanced conversation mode needs python_executable set to an installed Python 3.11+ executable's absolute path")?;
    let helper = executable()?;
    let script = helper
        .parent()
        .ok_or("Application directory is unavailable")?
        .join("controller/conversation_orchestrator.py");
    if !script.is_file() {
        return Err("Advanced conversation controller is missing from this app package".into());
    }
    let mut process = Command::new(python);
    process.arg(script).arg("--helper").arg(helper);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(process)
}
pub fn write_example(path: &Path) -> Result<(), String> {
    let output = command()?
        .arg("--write-example")
        .arg(path)
        .output()
        .map_err(|e| format!("Cannot create conversation configuration: {e}"))?;
    if !output.status.success() {
        return Err(
            "Cannot create example configuration. Existing files are never overwritten.".into(),
        );
    }
    Ok(())
}
pub struct Listener {
    child: Option<Child>,
}
impl Listener {
    pub fn start(path: &Path, send: impl Fn(Value) + Send + 'static) -> Result<Self, String> {
        if !path.is_file() {
            return Err("Choose an existing conversation configuration first.".into());
        }
        let mut child = launch_command(path)?
            .arg("--config")
            .arg(path)
            .arg("--start")
            .spawn()
            .map_err(|e| format!("Cannot start conversation helper: {e}"))?;
        let stdout = child.stdout.take().ok_or("Helper output is unavailable")?;
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.by_ref().take(131073).read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) if line.len() <= 131072 && line.last() == Some(&b'\n') => {
                        match serde_json::from_slice(&line) {
                            Ok(value) => send(value),
                            Err(_) => {
                                send(
                                    json!({"event":"error","stage":"input","message":"Conversation helper returned invalid output"}),
                                );
                                break;
                            }
                        }
                    }
                    _ => {
                        send(
                            json!({"event":"error","stage":"input","message":"Conversation helper output exceeded its limit or closed unexpectedly"}),
                        );
                        break;
                    }
                }
            }
            send(json!({"event":"exit"}));
        });
        Ok(Self { child: Some(child) })
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Close input immediately. Reaping never blocks the desktop event loop.
            if let Some(mut input) = child.stdin.take() {
                let _ = input.write_all(b"{\"command\":\"stop\"}\n");
            }
            let reaper = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
            let mut reapers = REAPERS.lock().unwrap_or_else(|e| e.into_inner());
            reapers.retain(|thread| !thread.is_finished());
            reapers.push(reaper);
        }
    }
}
