//! The parent side of the capture child (DD-73; `docs/dictation/surfaces.md`, "The capture
//! child").
//!
//! The child is this binary started again as `__br-dictation-worker capture`, so nothing is
//! discovered on disk, and a driver crash or ALSA's stderr stays out of the terminal: the
//! child's stderr is read here, the last 4 KiB kept for a diagnosis and never printed raw.
//! It answers a probe as soon as it starts; [`CaptureChild::start`], which takes a
//! [`CaptureConsent`], is the only way to open the microphone. The child leads its own
//! process group on Unix, so a Ctrl-C at the terminal reaches only this process, which then
//! stops the child; a parent that dies closes the child's stdin, which ends it within 2 s,
//! and `kill_on_drop` holds for every other way out.
//!
//! Latency (PR-25): the child is started before the model is resolved and the worker loaded,
//! and frames are handed on as they arrive, so the first frame and the load overlap. The
//! event channel is unbounded so the child is never held up: a recording stops at 600 s,
//! so a parent that stalls holds at most about 38 MB of samples, and every frame's bytes,
//! and then its samples once the recorder holds them, are zeroed (P-1).

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use biorouter_dictation::protocol::{read_frame, to_json, write_frame};
use biorouter_dictation::worker::WORKER_MARKER;
use biorouter_dictation_capture::{
    samples_of, wipe, ErrorCode, FromCapture, Level, Probe, Ready, ToCapture, CAPTURE_ROLE,
};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;

use super::consent::CaptureConsent;

/// How much of the child's stderr is kept.
pub const STDERR_TAIL_BYTES: usize = 4096;
/// How long the probe may take: an audio context comes up in tens of milliseconds, and a
/// PulseAudio server that does not answer is waited out by libpulse itself.
pub const PROBE_DEADLINE: Duration = Duration::from_secs(8);
/// How long the child has to exit after its stdin closes.
pub const EXIT_DEADLINE: Duration = Duration::from_secs(2);

/// What the child said.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureEvent {
    Probe(Probe),
    Ready(Ready),
    Level(Level),
    Audio(Vec<f32>),
    Error(ErrorCode),
    Stopped(u64),
    /// Its stdout ended or held a frame that did not read.
    Exited,
}

/// A running capture child.
pub struct CaptureChild {
    child: Child,
    stdin: Option<ChildStdin>,
    events: mpsc::UnboundedReceiver<CaptureEvent>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
}

/// The binary the child runs from: on Linux `/proc/self/exe`, so an upgrade during a
/// session still starts the build that is running, else this executable.
fn program() -> std::io::Result<std::path::PathBuf> {
    if cfg!(target_os = "linux") {
        return Ok("/proc/self/exe".into());
    }
    std::env::current_exe()
}

impl CaptureChild {
    /// Starts the child, which probes at once. No device opens until [`Self::start`].
    pub fn spawn() -> std::io::Result<CaptureChild> {
        let mut command = tokio::process::Command::new(program()?);
        command
            .arg(WORKER_MARKER)
            .arg(CAPTURE_ROLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        // Last: the daemon's own credentials never reach a child, and on Windows it gets no
        // console window.
        biorouter::subprocess::prepare_agent_child_command(&mut command);
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("the capture child has no stdout"))?;
        let mut stderr_pipe = child.stderr.take();
        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let event = match read_frame(&mut stdout).await {
                    Ok(mut frame) => {
                        let event = match serde_json::from_slice::<FromCapture>(&frame.json) {
                            Ok(FromCapture::Probe(probe)) => Some(CaptureEvent::Probe(probe)),
                            Ok(FromCapture::Ready(ready)) => Some(CaptureEvent::Ready(ready)),
                            Ok(FromCapture::Level(level)) => Some(CaptureEvent::Level(level)),
                            Ok(FromCapture::Audio) => {
                                Some(CaptureEvent::Audio(samples_of(&frame.payload)))
                            }
                            Ok(FromCapture::Error { code }) => Some(CaptureEvent::Error(code)),
                            Ok(FromCapture::Stopped { frames }) => {
                                Some(CaptureEvent::Stopped(frames))
                            }
                            Ok(FromCapture::Unknown) => None,
                            Err(_) => Some(CaptureEvent::Exited),
                        };
                        // P-1: the frame's bytes are audio; the samples now live in the
                        // event, which the recorder zeroes once it holds them.
                        wipe(&mut frame.payload);
                        match event {
                            Some(event) => event,
                            None => continue,
                        }
                    }
                    Err(_) => CaptureEvent::Exited,
                };
                let last = event == CaptureEvent::Exited;
                if tx.send(event).is_err() || last {
                    return;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES)));
        if let Some(mut pipe) = stderr_pipe.take() {
            let tail = Arc::clone(&stderr);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut buf).await {
                    if n == 0 {
                        return;
                    }
                    let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                    tail.extend(&buf[..n]);
                    while tail.len() > STDERR_TAIL_BYTES {
                        tail.pop_front();
                    }
                }
            });
        }
        Ok(CaptureChild {
            child,
            stdin,
            events,
            stderr,
        })
    }

    /// The probe, the child's first frame, or `None` when the child said nothing in time.
    pub async fn probe(&mut self) -> Option<Probe> {
        match tokio::time::timeout(PROBE_DEADLINE, self.events.recv()).await {
            Ok(Some(CaptureEvent::Probe(probe))) => Some(probe),
            _ => None,
        }
    }

    /// Opens the microphone. Only a person at this terminal holds a [`CaptureConsent`].
    pub async fn start(
        &mut self,
        _consent: &CaptureConsent,
        device: Option<String>,
    ) -> std::io::Result<()> {
        self.send(&ToCapture::Start { device }).await
    }

    /// Asks the child to stop: it sends what it holds, then `stopped`.
    pub async fn stop(&mut self) {
        let _ = self.send(&ToCapture::Stop).await;
    }

    async fn send(&mut self, message: &ToCapture) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the capture child's stdin is closed"))?;
        write_frame(stdin, &to_json(message), &[]).await
    }

    /// The next event; [`CaptureEvent::Exited`] once the child has nothing more to say.
    pub async fn next(&mut self) -> CaptureEvent {
        self.events.recv().await.unwrap_or(CaptureEvent::Exited)
    }

    /// The last of the child's stderr, for a diagnosis; never shown raw.
    pub fn stderr_tail(&self) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&tail.iter().copied().collect::<Vec<u8>>()).into_owned()
    }

    /// Closes the child's stdin and waits up to [`EXIT_DEADLINE`] for it to exit, then kills
    /// it. Returns its exit code when it exited on its own.
    pub async fn close(mut self) -> Option<i32> {
        drop(self.stdin.take());
        match tokio::time::timeout(EXIT_DEADLINE, self.child.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                None
            }
        }
    }

    /// The child's pid, while it runs.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }
}

/// Starts a child only to read its probe, then closes it: what `biorouter dictation
/// devices` and the status's `capture` show. No device is opened, so no permission prompt
/// appears.
pub async fn probe_once() -> Option<Probe> {
    let mut child = CaptureChild::spawn().ok()?;
    let probe = child.probe().await;
    let _ = child.close().await;
    probe
}

/// [`probe_once`] for a caller with no runtime at hand (`status`, which `doctor` also
/// prints): the child's stderr goes to the null device, its stdin is closed at once, so it
/// exits after the probe, and one that says nothing within [`PROBE_DEADLINE`] is killed.
pub fn probe_blocking() -> Option<Probe> {
    use biorouter_dictation::protocol::read_frame_blocking;

    let mut command = std::process::Command::new(program().ok()?);
    command
        .arg(WORKER_MARKER)
        .arg(CAPTURE_ROLE)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let probe = read_frame_blocking(&mut stdout)
            .ok()
            .and_then(|frame| serde_json::from_slice::<FromCapture>(&frame.json).ok());
        let _ = tx.send(probe);
    });
    let probe = rx.recv_timeout(PROBE_DEADLINE).ok().flatten();
    if probe.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    match probe {
        Some(FromCapture::Probe(probe)) => Some(probe),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    /// DD-5: `biorouter`'s `main()` answers the capture role before the engine's
    /// `run_if_invoked`, which would answer it with exit 2, and before the runtime starts.
    #[test]
    fn main_answers_the_capture_role_before_the_worker() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let source = std::fs::read_to_string(&path).unwrap();
        let (_, body) = source
            .split_once("\nfn main()")
            .expect("main.rs has fn main()");
        let at = |needle: &str| {
            body.find(needle)
                .unwrap_or_else(|| panic!("main() does not call {needle}"))
        };
        let capture = at("biorouter_dictation_capture::run_if_invoked();");
        let worker = at("biorouter_dictation::worker::run_if_invoked();");
        let runtime = at("run_on_agent_stack");
        assert!(capture < worker && worker < runtime);
        let (before, _) = body.split_at(capture);
        assert!(
            before
                .trim_end()
                .ends_with("#[cfg(feature = \"dictation\")]"),
            "the capture entry is compiled with the dictation feature only"
        );
    }
}
