//! The terminal's microphone: the child side of `biorouter __br-dictation-worker capture`
//! (DD-5, DD-73; `docs/dictation/surfaces.md`, "The capture child").
//!
//! `biorouter`'s `main()` calls [`run_if_invoked`] before the engine's own
//! `run_if_invoked`, so this role is answered only in `biorouter`; `biorouterd` never links
//! this crate and answers the role with exit 2 (P-10). The child runs no runtime, reads no
//! configuration and writes no log: one audio context, one capture stream, frames on
//! stdout.
//!
//! The conversation, in the worker protocol's frames (`[u32 LE json_len][json][u32 LE
//! payload_len][payload]`, [`biorouter_dictation::protocol`]):
//!
//! 1. The child writes `probe` at once: the backends compiled in, the one that answered
//!    and its capture devices, listed without opening any, so no permission prompt is
//!    shown. `biorouter dictation devices` and `status` stop here, by closing stdin.
//! 2. The parent sends `start`, only after a person at a terminal asked for it
//!    (`CaptureConsent` in `crates/biorouter-cli/src/dictation/consent.rs`). The child
//!    refuses `start` itself in an agent's shell (`BIOROUTER_TERMINAL` set), so a model
//!    that runs this hidden role from its shell gets no microphone either.
//! 3. The child answers `ready{backend, device, rate}`, then every 100 ms a
//!    `level{peak, rms}` and an `audio` frame whose payload is f32 LE, 16 kHz, mono.
//! 4. `stop`, or stdin's end of file, makes the child write what it holds, then
//!    `stopped{frames}`, and only then close the device, so the parent can start the last
//!    decode while the device is still being torn down. It exits within 2 s.
//!
//! A failure is one `error{code}` frame and an exit. Nothing here writes audio anywhere
//! but stdout: no file, no log line. Every buffer of samples the child is done with is
//! zeroed ([`wipe`]) before it is dropped or reused.
//!
//! In a debug build, [`TEST_SOURCE_ENV`] replaces the microphone with a recording, a tone
//! or exact zeros, so the CLI's tests and latency runs work on a machine with no
//! microphone; a release build ignores it.

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use biorouter_dictation::protocol::{read_frame_blocking, to_json, write_frame_blocking};
use serde::{Deserialize, Serialize};

/// The role this crate answers, after `biorouter_dictation::worker::WORKER_MARKER`.
pub const CAPTURE_ROLE: &str = "capture";
/// The rate of every audio frame.
pub const SAMPLE_RATE: u32 = 16_000;
/// The device period, and the cadence of `level` and `audio` frames.
pub const PERIOD_MS: u32 = 100;
/// One period of samples.
pub const PERIOD_SAMPLES: usize = (SAMPLE_RATE as usize * PERIOD_MS as usize) / 1_000;
/// Set in an agent's shell; the child refuses `start` there.
pub const AGENT_SHELL_ENV: &str = "BIOROUTER_TERMINAL";
/// Debug builds only: `zeros`, `tone`, `quiet`, `none` (no audio system), `nodevice`, or a
/// path to a 16 kHz mono 16 bit WAV file played in real time and followed by quiet noise.
pub const TEST_SOURCE_ENV: &str = "BIOROUTER_DICTATION_CAPTURE_TEST_SOURCE";
/// Linux only, environment only (DD-73): a command that writes raw s16le, 16 kHz, mono
/// samples to its stdout, such as `arecord -q -f S16_LE -r 16000 -c 1 -t raw`, used in
/// place of miniaudio for audio setups it does not reach. It is started only on `start`,
/// which the parent sends only behind `CaptureConsent`, and never in an agent's shell.
pub const RECORDER_ENV: &str = "BIOROUTER_DICTATION_RECORDER";

/// The child's exit codes.
pub mod exit {
    /// `stop` or stdin's end of file.
    pub const OK: i32 = 0;
    /// The device could not be opened, or was lost; the `error` frame says which.
    pub const FAILED: i32 = 1;
    /// `start` refused: an agent's shell, or a build with no microphone.
    pub const REFUSED: i32 = 2;
    /// A frame from the parent that could not be read.
    pub const PROTOCOL: i32 = 65;
}

/// Messages from the parent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToCapture {
    /// Open `device` (the system default when `None`) and start streaming.
    Start {
        #[serde(default)]
        device: Option<String>,
    },
    Stop,
    #[serde(other)]
    Unknown,
}

/// Messages from the child.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromCapture {
    Probe(Probe),
    Ready(Ready),
    Level(Level),
    /// The payload is f32 LE samples at [`SAMPLE_RATE`], mono.
    Audio,
    Error {
        code: ErrorCode,
    },
    Stopped {
        frames: u64,
    },
    #[serde(other)]
    Unknown,
}

/// What the child found before opening anything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    /// The backends this build carries, in miniaudio's order of preference.
    pub compiled: Vec<String>,
    /// The backend that answered, `None` when none did.
    pub backend: Option<String>,
    pub devices: Vec<Device>,
    /// `no_backend`, `no_device` or `not_compiled` when the child cannot record here.
    pub error: Option<ErrorCode>,
    /// A debug build's stand in for the microphone answered instead of a device.
    #[serde(default)]
    pub test_source: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub default: bool,
}

/// The stream is open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    pub backend: String,
    pub device: String,
    /// The rate of the audio frames, always [`SAMPLE_RATE`].
    pub rate: u32,
    /// The device's own rate, which miniaudio resamples from.
    pub device_rate: u32,
}

/// One period's level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub peak: f32,
    pub rms: f32,
}

impl Level {
    pub fn of(samples: &[f32]) -> Level {
        if samples.is_empty() {
            return Level::default();
        }
        let mut peak = 0f32;
        let mut sum = 0f64;
        for &s in samples {
            peak = peak.max(s.abs());
            sum += f64::from(s) * f64::from(s);
        }
        Level {
            peak,
            rms: (sum / samples.len() as f64).sqrt() as f32,
        }
    }
}

/// Why the child cannot record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// This build has no microphone code (a target outside macOS, Windows and Linux).
    NotCompiled,
    /// No audio system answered: no PulseAudio, PipeWire or ALSA, or no CoreAudio or
    /// WASAPI service.
    NoBackend,
    /// The audio system answered but has no capture device.
    NoDevice,
    /// The device named by the parent is not there.
    DeviceMissing,
    /// The device would not open: in use, or blocked by the system.
    DeviceBusy,
    /// The device stopped on its own while recording: unplugged, or taken away.
    DeviceLost,
    /// `start` in an agent's shell.
    AgentShell,
    /// Anything else that kept the stream from starting.
    StartFailed,
    #[serde(other)]
    Unknown,
}

/// The f32 LE bytes of an audio frame's payload.
pub fn payload_of(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes
}

/// An audio frame's samples. Trailing bytes short of a sample are ignored.
pub fn samples_of(payload: &[u8]) -> Vec<f32> {
    payload
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Overwrites `buf` with zeros before its memory is freed or reused, with writes the
/// optimizer keeps (P-1: audio does not outlive its use in freed memory). The child, and
/// the parent's recorder and decode copies, call it on every buffer of samples they drop.
pub fn wipe<T: Copy + Default>(buf: &mut [T]) {
    for value in buf.iter_mut() {
        // SAFETY: `value` is a valid, aligned and exclusive reference into `buf`.
        unsafe { std::ptr::write_volatile(value, T::default()) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// The backends this build compiled in, as miniaudio names them.
pub fn compiled_backends() -> Vec<String> {
    #[cfg(not(br_no_capture))]
    {
        // SAFETY: returns a pointer to a static, NUL terminated buffer.
        let names = unsafe { std::ffi::CStr::from_ptr(ffi::br_capture_compiled_backends()) };
        names
            .to_string_lossy()
            .split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect()
    }
    #[cfg(br_no_capture)]
    {
        Vec::new()
    }
}

/// Returns only when argv is not `__br-dictation-worker capture`; otherwise runs the
/// child and exits the process with its code.
pub fn run_if_invoked() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(biorouter_dictation::worker::WORKER_MARKER)) {
        return;
    }
    if args.next().as_deref() != Some(OsStr::new(CAPTURE_ROLE)) {
        return;
    }
    let agent_shell = std::env::var_os(AGENT_SHELL_ENV).is_some_and(|v| !v.is_empty());
    let mut backend = open_backend();
    let code = serve(
        backend.as_mut(),
        io::stdin(),
        &mut io::stdout().lock(),
        agent_shell,
    );
    drop(backend);
    std::process::exit(code);
}

/// The microphone, or in a debug build the test source when [`TEST_SOURCE_ENV`] is set.
fn open_backend() -> Box<dyn Backend> {
    if cfg!(debug_assertions) {
        if let Some(kind) = std::env::var_os(TEST_SOURCE_ENV).filter(|v| !v.is_empty()) {
            return Box::new(TestSource::new(&kind));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(command) = std::env::var_os(RECORDER_ENV).filter(|v| !v.is_empty()) {
            return Box::new(CommandRecorder::new(command));
        }
    }
    #[cfg(not(br_no_capture))]
    {
        Box::new(Miniaudio::open())
    }
    #[cfg(br_no_capture)]
    {
        Box::new(NotCompiled)
    }
}

/// What the serving loop needs of a microphone.
trait Backend {
    fn probe(&mut self) -> Probe;
    /// Opens `device` and starts sending samples to `sink`.
    fn start(
        &mut self,
        device: Option<&str>,
        sink: mpsc::Sender<Event>,
    ) -> Result<Ready, ErrorCode>;
    /// Whether the stream stopped without being asked.
    fn lost(&self) -> bool;
    /// Closes the device; no sample is sent once this returns.
    fn stop(&mut self);
}

/// What the serving loop waits on: samples from the device and frames from the parent.
enum Event {
    Samples(Vec<f32>),
    Command(ToCapture),
    Closed,
    BadFrame,
}

fn write_message<W: Write>(out: &mut W, message: &FromCapture, payload: &[u8]) -> io::Result<()> {
    write_frame_blocking(out, &to_json(message), payload)
}

/// The child: `probe`, then one stream on `start`, until `stop` or end of file. Returns the
/// exit code.
fn serve<R: Read + Send + 'static, W: Write>(
    backend: &mut dyn Backend,
    stdin: R,
    out: &mut W,
    agent_shell: bool,
) -> i32 {
    let (tx, rx) = mpsc::channel::<Event>();
    let commands = tx.clone();
    std::thread::Builder::new()
        .name("br-capture-stdin".into())
        .spawn(move || {
            let mut stdin = stdin;
            loop {
                let event = match read_frame_blocking(&mut stdin) {
                    Ok(frame) => match serde_json::from_slice::<ToCapture>(&frame.json) {
                        Ok(command) => Event::Command(command),
                        Err(_) => Event::BadFrame,
                    },
                    Err(biorouter_dictation::protocol::FrameError::Closed) => Event::Closed,
                    Err(_) => Event::BadFrame,
                };
                let last = !matches!(event, Event::Command(_));
                if commands.send(event).is_err() || last {
                    return;
                }
            }
        })
        .ok();

    if write_message(out, &FromCapture::Probe(backend.probe()), &[]).is_err() {
        return exit::OK;
    }
    // Before `start`: samples cannot arrive, and anything but `start` ends the child.
    let device = loop {
        match rx.recv() {
            Ok(Event::Command(ToCapture::Start { device })) => break device,
            Ok(Event::Command(ToCapture::Unknown)) | Ok(Event::Samples(_)) => continue,
            Ok(Event::Command(ToCapture::Stop)) | Ok(Event::Closed) | Err(_) => return exit::OK,
            Ok(Event::BadFrame) => return exit::PROTOCOL,
        }
    };
    if agent_shell {
        let _ = write_message(
            out,
            &FromCapture::Error {
                code: ErrorCode::AgentShell,
            },
            &[],
        );
        return exit::REFUSED;
    }
    let ready = match backend.start(device.as_deref(), tx.clone()) {
        Ok(ready) => ready,
        Err(code) => {
            let _ = write_message(out, &FromCapture::Error { code }, &[]);
            return if code == ErrorCode::NotCompiled {
                exit::REFUSED
            } else {
                exit::FAILED
            };
        }
    };
    if write_message(out, &FromCapture::Ready(ready), &[]).is_err() {
        backend.stop();
        return exit::OK;
    }
    stream(backend, &rx, out)
}

/// Sends what the device delivers, in periods of [`PERIOD_SAMPLES`] or every
/// [`PERIOD_MS`], whichever comes first, until `stop`, end of file or a lost device.
fn stream<W: Write>(backend: &mut dyn Backend, rx: &mpsc::Receiver<Event>, out: &mut W) -> i32 {
    let period = Duration::from_millis(u64::from(PERIOD_MS));
    let mut pending: Vec<f32> = Vec::with_capacity(PERIOD_SAMPLES * 2);
    let mut frames: u64 = 0;
    let mut last_emit = Instant::now();
    fn emit<W: Write>(pending: &mut Vec<f32>, frames: &mut u64, out: &mut W) -> io::Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        let mut payload = payload_of(pending);
        let written = write_message(out, &FromCapture::Level(Level::of(pending)), &[])
            .and_then(|()| write_message(out, &FromCapture::Audio, &payload));
        wipe(&mut payload);
        if written.is_ok() {
            *frames += pending.len() as u64;
        }
        wipe(pending);
        pending.clear();
        written
    }
    loop {
        let wait = period
            .saturating_sub(last_emit.elapsed())
            .max(Duration::from_millis(5));
        match rx.recv_timeout(wait) {
            Ok(Event::Samples(mut samples)) => {
                pending.extend_from_slice(&samples);
                wipe(&mut samples);
            }
            Ok(Event::Command(ToCapture::Stop))
            | Ok(Event::Closed)
            | Err(mpsc::RecvTimeoutError::Disconnected) => {
                // What the device delivered goes out first, then `stopped`, and only then
                // is the device closed: the parent's last decode need not wait for it.
                let _ = emit(&mut pending, &mut frames, out);
                let _ = write_message(out, &FromCapture::Stopped { frames }, &[]);
                backend.stop();
                return exit::OK;
            }
            Ok(Event::Command(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Event::BadFrame) => {
                backend.stop();
                return exit::PROTOCOL;
            }
        }
        if backend.lost() {
            let _ = emit(&mut pending, &mut frames, out);
            let _ = write_message(
                out,
                &FromCapture::Error {
                    code: ErrorCode::DeviceLost,
                },
                &[],
            );
            backend.stop();
            return exit::FAILED;
        }
        let due = last_emit.elapsed() >= period;
        if pending.len() >= PERIOD_SAMPLES || due {
            last_emit = Instant::now();
            if emit(&mut pending, &mut frames, out).is_err() {
                // The parent is gone: nothing reads the frames any more.
                backend.stop();
                return exit::OK;
            }
        }
    }
}

#[cfg(br_no_capture)]
struct NotCompiled;

#[cfg(br_no_capture)]
impl Backend for NotCompiled {
    fn probe(&mut self) -> Probe {
        Probe {
            error: Some(ErrorCode::NotCompiled),
            ..Probe::default()
        }
    }
    fn start(&mut self, _: Option<&str>, _: mpsc::Sender<Event>) -> Result<Ready, ErrorCode> {
        Err(ErrorCode::NotCompiled)
    }
    fn lost(&self) -> bool {
        false
    }
    fn stop(&mut self) {}
}

#[cfg(not(br_no_capture))]
mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct BrCapture {
        _private: [u8; 0],
    }

    pub type OnSamples = extern "C" fn(user: *mut c_void, samples: *const f32, frames: u32);
    pub type OnDevice = extern "C" fn(user: *mut c_void, name: *const c_char, is_default: c_int);

    extern "C" {
        pub fn br_capture_compiled_backends() -> *const c_char;
        pub fn br_capture_new(result: *mut c_int) -> *mut BrCapture;
        pub fn br_capture_backend_name(capture: *const BrCapture) -> *const c_char;
        pub fn br_capture_devices(
            capture: *mut BrCapture,
            on_device: OnDevice,
            user: *mut c_void,
        ) -> c_int;
        pub fn br_capture_start(
            capture: *mut BrCapture,
            name: *const c_char,
            rate: u32,
            period_ms: u32,
            on_samples: OnSamples,
            user: *mut c_void,
        ) -> c_int;
        pub fn br_capture_device_name(capture: *const BrCapture) -> *const c_char;
        pub fn br_capture_device_rate(capture: *const BrCapture) -> u32;
        pub fn br_capture_lost(capture: *const BrCapture) -> c_int;
        pub fn br_capture_stop(capture: *mut BrCapture);
        pub fn br_capture_free(capture: *mut BrCapture);
    }

    /// miniaudio's `ma_result` values the child tells apart.
    pub const MA_ACCESS_DENIED: c_int = -6;
    pub const MA_DOES_NOT_EXIST: c_int = -7;
    pub const MA_BUSY: c_int = -19;
    pub const MA_DEVICE_TYPE_NOT_SUPPORTED: c_int = -201;
    pub const MA_NO_BACKEND: c_int = -203;
    pub const MA_NO_DEVICE: c_int = -204;
    pub const MA_FAILED_TO_INIT_BACKEND: c_int = -400;
    pub const MA_FAILED_TO_OPEN_BACKEND_DEVICE: c_int = -401;
    pub const MA_FAILED_TO_START_BACKEND_DEVICE: c_int = -402;
}

/// The real microphone, through the shim.
#[cfg(not(br_no_capture))]
struct Miniaudio {
    capture: *mut ffi::BrCapture,
    /// The sender the device's callback holds, freed after the device is closed.
    sink: *mut mpsc::Sender<Event>,
}

#[cfg(not(br_no_capture))]
impl Miniaudio {
    fn open() -> Miniaudio {
        let mut result = 0;
        // SAFETY: `result` is a valid out pointer; a null return means no context.
        let capture = unsafe { ffi::br_capture_new(&mut result) };
        Miniaudio {
            capture,
            sink: std::ptr::null_mut(),
        }
    }
}

#[cfg(not(br_no_capture))]
extern "C" fn on_device(
    user: *mut std::ffi::c_void,
    name: *const std::ffi::c_char,
    is_default: std::ffi::c_int,
) {
    let _ = std::panic::catch_unwind(|| {
        if user.is_null() || name.is_null() {
            return;
        }
        // SAFETY: `user` is the `Vec<Device>` `probe` passed, alive for the call; `name`
        // is miniaudio's NUL terminated device name.
        let devices = unsafe { &mut *(user as *mut Vec<Device>) };
        let name = unsafe { std::ffi::CStr::from_ptr(name) };
        devices.push(Device {
            name: name.to_string_lossy().into_owned(),
            default: is_default != 0,
        });
    });
}

#[cfg(not(br_no_capture))]
extern "C" fn on_samples(user: *mut std::ffi::c_void, samples: *const f32, frames: u32) {
    let _ = std::panic::catch_unwind(|| {
        if user.is_null() || samples.is_null() || frames == 0 {
            return;
        }
        // SAFETY: `user` is the boxed sender `start` passed, freed only after
        // `br_capture_stop` returned; `samples` holds `frames` mono floats for this call.
        let sink = unsafe { &*(user as *const mpsc::Sender<Event>) };
        let samples = unsafe { std::slice::from_raw_parts(samples, frames as usize) };
        let _ = sink.send(Event::Samples(samples.to_vec()));
    });
}

#[cfg(not(br_no_capture))]
impl Backend for Miniaudio {
    fn probe(&mut self) -> Probe {
        let compiled = compiled_backends();
        if self.capture.is_null() {
            return Probe {
                compiled,
                error: Some(ErrorCode::NoBackend),
                ..Probe::default()
            };
        }
        // SAFETY: a live context; the name is a static string.
        let backend =
            unsafe { std::ffi::CStr::from_ptr(ffi::br_capture_backend_name(self.capture)) }
                .to_string_lossy()
                .into_owned();
        let mut devices: Vec<Device> = Vec::new();
        // SAFETY: a live context; `devices` outlives the call, which runs the callback
        // only while it lasts.
        let result = unsafe {
            ffi::br_capture_devices(
                self.capture,
                on_device,
                &mut devices as *mut Vec<Device> as *mut std::ffi::c_void,
            )
        };
        let error = (result != 0 || devices.is_empty()).then_some(ErrorCode::NoDevice);
        Probe {
            compiled,
            backend: Some(backend),
            devices,
            error,
            test_source: false,
        }
    }

    fn start(
        &mut self,
        device: Option<&str>,
        sink: mpsc::Sender<Event>,
    ) -> Result<Ready, ErrorCode> {
        if self.capture.is_null() {
            return Err(ErrorCode::NoBackend);
        }
        let name = match device {
            Some(name) => Some(std::ffi::CString::new(name).map_err(|_| ErrorCode::DeviceMissing)?),
            None => None,
        };
        let sink = Box::into_raw(Box::new(sink));
        // SAFETY: a live context; the name lives for the call; `sink` stays valid until
        // `stop` frees it after the device is closed.
        let result = unsafe {
            ffi::br_capture_start(
                self.capture,
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                SAMPLE_RATE,
                PERIOD_MS,
                on_samples,
                sink as *mut std::ffi::c_void,
            )
        };
        if result != 0 {
            // SAFETY: the device did not open, so no callback holds the sender.
            drop(unsafe { Box::from_raw(sink) });
            return Err(match result {
                ffi::MA_DOES_NOT_EXIST if device.is_some() => ErrorCode::DeviceMissing,
                ffi::MA_DOES_NOT_EXIST | ffi::MA_NO_DEVICE => ErrorCode::NoDevice,
                ffi::MA_NO_BACKEND | ffi::MA_FAILED_TO_INIT_BACKEND => ErrorCode::NoBackend,
                ffi::MA_ACCESS_DENIED
                | ffi::MA_BUSY
                | ffi::MA_DEVICE_TYPE_NOT_SUPPORTED
                | ffi::MA_FAILED_TO_OPEN_BACKEND_DEVICE
                | ffi::MA_FAILED_TO_START_BACKEND_DEVICE => ErrorCode::DeviceBusy,
                _ => ErrorCode::StartFailed,
            });
        }
        self.sink = sink;
        // SAFETY: the device is open; the name points into its state.
        let (backend, device, device_rate) = unsafe {
            (
                std::ffi::CStr::from_ptr(ffi::br_capture_backend_name(self.capture))
                    .to_string_lossy()
                    .into_owned(),
                std::ffi::CStr::from_ptr(ffi::br_capture_device_name(self.capture))
                    .to_string_lossy()
                    .into_owned(),
                ffi::br_capture_device_rate(self.capture),
            )
        };
        Ok(Ready {
            backend,
            device,
            rate: SAMPLE_RATE,
            device_rate,
        })
    }

    fn lost(&self) -> bool {
        // SAFETY: a live context, or null, which is checked first.
        !self.capture.is_null() && unsafe { ffi::br_capture_lost(self.capture) } != 0
    }

    fn stop(&mut self) {
        if !self.capture.is_null() {
            // SAFETY: a live context; no callback runs once this returns.
            unsafe { ffi::br_capture_stop(self.capture) };
        }
        if !self.sink.is_null() {
            // SAFETY: the device is closed, so nothing else holds the sender.
            drop(unsafe { Box::from_raw(self.sink) });
            self.sink = std::ptr::null_mut();
        }
    }
}

#[cfg(not(br_no_capture))]
impl Drop for Miniaudio {
    fn drop(&mut self) {
        self.stop();
        if !self.capture.is_null() {
            // SAFETY: the device is closed; the context is freed once.
            unsafe { ffi::br_capture_free(self.capture) };
            self.capture = std::ptr::null_mut();
        }
    }
}

/// [`RECORDER_ENV`]'s command in place of miniaudio: started on `start` through `/bin/sh -c`,
/// its stdout read as s16le 16 kHz mono, its stderr dropped. Selected on Linux only; the
/// type builds on every Unix so its tests run anywhere.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct CommandRecorder {
    command: std::ffi::OsString,
    child: Option<std::process::Child>,
    reader: Option<std::thread::JoinHandle<()>>,
    ended: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl CommandRecorder {
    fn new(command: std::ffi::OsString) -> CommandRecorder {
        CommandRecorder {
            command,
            child: None,
            reader: None,
            ended: Default::default(),
            stopping: Default::default(),
        }
    }

    /// What the probe names the device: the command's first word.
    fn name(&self) -> String {
        self.command
            .to_string_lossy()
            .split_whitespace()
            .next()
            .unwrap_or("recorder")
            .to_string()
    }
}

#[cfg(unix)]
impl Backend for CommandRecorder {
    fn probe(&mut self) -> Probe {
        Probe {
            compiled: compiled_backends(),
            backend: Some("Recorder".into()),
            devices: vec![Device {
                name: self.name(),
                default: true,
            }],
            error: None,
            test_source: false,
        }
    }

    fn start(
        &mut self,
        device: Option<&str>,
        sink: mpsc::Sender<Event>,
    ) -> Result<Ready, ErrorCode> {
        use std::sync::atomic::Ordering;
        let name = self.name();
        if device.is_some_and(|d| d != name) {
            return Err(ErrorCode::DeviceMissing);
        }
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&self.command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| ErrorCode::StartFailed)?;
        let mut stdout = child.stdout.take().ok_or(ErrorCode::StartFailed)?;
        let (ended, stopping) = (
            std::sync::Arc::clone(&self.ended),
            std::sync::Arc::clone(&self.stopping),
        );
        self.reader = Some(std::thread::spawn(move || {
            let mut bytes = vec![0u8; PERIOD_SAMPLES * 2];
            let mut carry: Option<u8> = None;
            loop {
                let n = match stdout.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let mut samples = Vec::with_capacity(n / 2 + 1);
                let mut at = 0;
                if let Some(low) = carry.take() {
                    samples.push(f32::from(i16::from_le_bytes([low, bytes[0]])) / 32_768.0);
                    at = 1;
                }
                let pairs = bytes.get(at..n).unwrap_or_default();
                let mut chunks = pairs.chunks_exact(2);
                for pair in &mut chunks {
                    samples.push(f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0);
                }
                carry = chunks.remainder().first().copied();
                if sink.send(Event::Samples(samples)).is_err() {
                    break;
                }
            }
            wipe(&mut bytes);
            if !stopping.load(Ordering::SeqCst) {
                ended.store(true, Ordering::SeqCst);
            }
        }));
        self.child = Some(child);
        Ok(Ready {
            backend: "Recorder".into(),
            device: name,
            rate: SAMPLE_RATE,
            device_rate: SAMPLE_RATE,
        })
    }

    fn lost(&self) -> bool {
        self.ended.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn stop(&mut self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(unix)]
impl Drop for CommandRecorder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A debug build's stand in for the microphone ([`TEST_SOURCE_ENV`]).
struct TestSource {
    kind: TestKind,
    running: Option<(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    )>,
}

#[derive(Clone)]
enum TestKind {
    Zeros,
    Tone,
    Quiet,
    NoBackend,
    NoDevice,
    Recording(std::sync::Arc<[f32]>),
}

/// The device name a test source reports.
pub const TEST_DEVICE: &str = "Biorouter test source";

impl TestSource {
    fn new(kind: &OsStr) -> TestSource {
        let kind = match kind.to_str() {
            Some("zeros") => TestKind::Zeros,
            Some("tone") => TestKind::Tone,
            Some("quiet") => TestKind::Quiet,
            Some("none") => TestKind::NoBackend,
            Some("nodevice") => TestKind::NoDevice,
            _ => std::fs::read(kind)
                .ok()
                .and_then(|bytes| biorouter_dictation::audio::parse_wav_s16(&bytes).ok())
                .map(|wav| {
                    let mut floats = Vec::new();
                    biorouter_dictation::audio::s16_to_f32(&wav.to_16k_mono(), &mut floats);
                    TestKind::Recording(floats.into())
                })
                .unwrap_or(TestKind::NoDevice),
        };
        TestSource {
            kind,
            running: None,
        }
    }
}

impl Backend for TestSource {
    fn probe(&mut self) -> Probe {
        match self.kind {
            TestKind::NoBackend => Probe {
                compiled: compiled_backends(),
                error: Some(ErrorCode::NoBackend),
                test_source: true,
                ..Probe::default()
            },
            TestKind::NoDevice => Probe {
                compiled: compiled_backends(),
                backend: Some("Test".into()),
                error: Some(ErrorCode::NoDevice),
                test_source: true,
                ..Probe::default()
            },
            _ => Probe {
                compiled: compiled_backends(),
                backend: Some("Test".into()),
                devices: vec![Device {
                    name: TEST_DEVICE.into(),
                    default: true,
                }],
                error: None,
                test_source: true,
            },
        }
    }

    fn start(
        &mut self,
        device: Option<&str>,
        sink: mpsc::Sender<Event>,
    ) -> Result<Ready, ErrorCode> {
        match self.kind {
            TestKind::NoBackend => return Err(ErrorCode::NoBackend),
            TestKind::NoDevice => return Err(ErrorCode::NoDevice),
            _ => {}
        }
        if device.is_some_and(|d| d != TEST_DEVICE) {
            return Err(ErrorCode::DeviceMissing);
        }
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let kind = self.kind.clone();
        let flag = std::sync::Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let started = Instant::now();
            let mut at = 0usize;
            let mut noise: u32 = 0x9E37_79B9;
            let mut period = 0u64;
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                let mut chunk = Vec::with_capacity(PERIOD_SAMPLES);
                for _ in 0..PERIOD_SAMPLES {
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    let quiet = ((noise % 21) as f32 - 10.0) / 32_768.0;
                    let sample = match &kind {
                        TestKind::Zeros => 0.0,
                        TestKind::Tone => {
                            let t = at as f32 / SAMPLE_RATE as f32;
                            0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                        }
                        TestKind::Recording(pcm) => pcm.get(at).copied().unwrap_or(quiet),
                        _ => quiet,
                    };
                    chunk.push(sample);
                    at += 1;
                }
                if sink.send(Event::Samples(chunk)).is_err() {
                    return;
                }
                period += 1;
                let next = started + Duration::from_millis(period * u64::from(PERIOD_MS));
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
            }
        });
        self.running = Some((stop, handle));
        Ok(Ready {
            backend: "Test".into(),
            device: TEST_DEVICE.into(),
            rate: SAMPLE_RATE,
            device_rate: SAMPLE_RATE,
        })
    }

    fn lost(&self) -> bool {
        false
    }

    fn stop(&mut self) {
        if let Some((flag, handle)) = self.running.take() {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = handle.join();
        }
    }
}

impl Drop for TestSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
