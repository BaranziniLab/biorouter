//! Biorouter Copilot's wrapper around its native engine (docs/design/copilot-agent-desktop.md).
//!
//! One [`SessionRuntime`] per chat. A tool call is checked (grant, refusals, reference
//! provenance), turned into one to three engine commands by [`plan`], run by [`runner`], and its
//! answer rendered by [`render`]. Every chat's task gets a private state folder for the engine
//! (AD-09), deleted when the task ends, on Stop and on revoke.
pub mod contract;
pub mod guard;
#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
pub mod manifest;
mod plan;
mod render;
pub mod retired;
mod runner;
#[cfg(windows)]
mod windows_job;

use contract::{Grant, Platform};
use guard::{AppFacts, HostProbe, Identity};
use render::RefRecord;
use rmcp::model::{CallToolResult, Content, ErrorData};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Said when a call needs control and the person allowed looking only (AD-10). No second card
/// is raised in that request.
pub const LOOK_ONLY_REFUSAL: &str = "The person allowed Biorouter Copilot to look only for this \
request. Do not ask again and do not try another tool; describe what you would change so the \
person can do it or allow control in their next message.";

const STALE_REFUSAL: &str = "computer_use_stale_state: observe this app first. Call get_app_state \
(or find_elements) for it in this request, then act on a reference from that observation.";

/// One running app a call targets: every instance that answers to the name, all checked.
#[derive(Clone, Debug)]
struct Target {
    name: String,
    instances: Vec<AppFacts>,
    window_id: Option<String>,
    window_title: Option<String>,
}

impl Target {
    fn key(&self) -> String {
        self.name.to_lowercase()
    }

    fn pids(&self) -> Vec<u32> {
        self.instances.iter().map(|app| app.pid).collect()
    }

    fn any(&self, predicate: impl Fn(&AppFacts) -> bool) -> bool {
        self.instances.iter().any(predicate)
    }

    fn slow_tree(&self) -> bool {
        const CHROMIUM: &[&str] = &[
            "com.google.chrome",
            "com.microsoft.edgemac",
            "com.brave.browser",
            "org.chromium",
            "com.vivaldi",
            "com.operasoftware",
            "com.microsoft.vscode",
            "com.tinyspeck.slackmacgap",
            "notion.id",
            "md.obsidian",
            "com.hnc.discord",
            "com.github.electron",
        ];
        self.any(|app| {
            let bundle = app.bundle_id.as_deref().unwrap_or_default().to_lowercase();
            let name = app.name.to_lowercase();
            CHROMIUM.iter().any(|prefix| bundle.starts_with(prefix))
                || name.contains("chrome")
                || name.contains("electron")
                || name == "code"
                || name == "msedge"
        })
    }
}

#[derive(Clone, Debug)]
struct Owned {
    record: RefRecord,
    target: Target,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    fn from(value: &Value) -> Option<Self> {
        Some(Self {
            x: value.get("x")?.as_f64()?,
            y: value.get("y")?.as_f64()?,
            width: value.get("width")?.as_f64()?,
            height: value.get("height")?.as_f64()?,
        })
    }

    fn same_place(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= 1.0
            && (self.y - other.y).abs() <= 1.0
            && (self.width - other.width).abs() <= 1.0
            && (self.height - other.height).abs() <= 1.0
    }

    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

/// The geometry of the latest screenshot of one app's window, for coordinates (AD-09).
#[derive(Clone, Debug)]
struct Shot {
    window_id: String,
    bounds: Rect,
    width: f64,
    height: f64,
}

impl Shot {
    /// Screenshot pixels to screen points, through the window's bounds when it was captured.
    fn to_screen(&self, x: f64, y: f64) -> Result<(f64, f64), String> {
        if !(0.0..self.width).contains(&x) || !(0.0..self.height).contains(&y) {
            return Err(format!(
                "Those coordinates are outside the latest screenshot ({} by {} pixels).",
                self.width, self.height
            ));
        }
        Ok((
            self.bounds.x + x * self.bounds.width / self.width,
            self.bounds.y + y * self.bounds.height / self.height,
        ))
    }
}

struct Task {
    generation: String,
    folder: PathBuf,
    engine: runner::Engine,
    refs: HashMap<String, Owned>,
    observed: HashMap<String, Target>,
    shots: HashMap<String, Shot>,
}

impl Task {
    /// After an uncertain outcome nothing about the app is trusted: its references and
    /// screenshot go, so the next action needs a new observation (AD-15).
    fn forget(&mut self, key: &str) {
        self.refs.retain(|_, owned| owned.target.key() != key);
        self.observed.remove(key);
        self.shots.remove(key);
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        remove_folder(&self.folder);
    }
}

fn remove_folder(folder: &Path) {
    let _ = std::fs::remove_dir_all(folder);
    if let Some(chat) = folder.parent() {
        let _ = std::fs::remove_dir(chat);
    }
}

/// Where the engine's per task folders live: `<Biorouter state>/copilot/runs`.
fn runs_root() -> Option<PathBuf> {
    if let Some(root) = crate::paths::path_root_override() {
        return Some(
            PathBuf::from(root)
                .join("state")
                .join("copilot")
                .join("runs"),
        );
    }
    let dirs = crate::paths::platform_dirs().ok()?;
    Some(
        dirs.state
            .unwrap_or(dirs.local_data)
            .join("copilot")
            .join("runs"),
    )
}

fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    let pid = Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::new(),
    );
    system.process(pid).is_some()
}

/// Remove task folders whose owning process is gone (a crash or a killed daemon), once per
/// process, so app content never stays at rest after its task.
fn collect_orphans(root: &Path) {
    let Ok(chats) = std::fs::read_dir(root) else {
        return;
    };
    for chat in chats.flatten() {
        let Ok(tasks) = std::fs::read_dir(chat.path()) else {
            continue;
        };
        for task in tasks.flatten() {
            let owner = std::fs::read_to_string(task.path().join("owner"))
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            if owner.is_none_or(|pid| pid != std::process::id() && !process_alive(pid)) {
                remove_folder(&task.path());
            }
        }
    }
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// One chat's Biorouter Copilot state.
pub struct SessionRuntime {
    session_id: Option<String>,
    task: Option<Task>,
    probe: Arc<dyn HostProbe>,
    platform: Platform,
    executable: Option<PathBuf>,
    runs_root: Option<PathBuf>,
}

impl Default for SessionRuntime {
    fn default() -> Self {
        Self {
            session_id: None,
            task: None,
            probe: Arc::new(guard::SystemProbe),
            platform: Platform::current(),
            executable: None,
            runs_root: None,
        }
    }
}

struct RuntimeRegistration {
    generation: String,
    runtime: std::sync::Weak<tokio::sync::Mutex<SessionRuntime>>,
    cancellation: CancellationToken,
}

type RuntimeRegistry = std::sync::Mutex<HashMap<String, Vec<RuntimeRegistration>>>;
// Weak lifecycle handles let the host revoke a task without sharing its observations.
static SESSION_RUNTIMES: once_cell::sync::Lazy<RuntimeRegistry> =
    once_cell::sync::Lazy::new(Default::default);

pub(crate) fn register_session(
    session: &str,
    generation: &str,
    runtime: &Arc<tokio::sync::Mutex<SessionRuntime>>,
    cancellation: CancellationToken,
) {
    let mut registry = SESSION_RUNTIMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.retain(|_, entries| {
        entries.retain(|entry| entry.runtime.strong_count() > 0);
        !entries.is_empty()
    });
    // One registration per runtime: the latest call's generation and token. The runtime's own
    // lock serializes its calls, so an older entry can only be a finished call.
    let entries = registry.entry(session.to_owned()).or_default();
    entries.retain(|entry| !entry.runtime.ptr_eq(&Arc::downgrade(runtime)));
    entries.push(RuntimeRegistration {
        generation: generation.to_owned(),
        runtime: Arc::downgrade(runtime),
        cancellation,
    });
}

/// Stop (AD-15): cancel the running command (its runner sends `SIGTERM`, waits for the release
/// guard, then `SIGKILL`s the group) and delete the task's state folder.
pub async fn stop_session(session_id: &str, generation: &str) {
    let entries = {
        let mut registry = SESSION_RUNTIMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entries) = registry.get_mut(session_id) else {
            return;
        };
        let mut removed = Vec::new();
        let mut index = 0;
        while index < entries.len() {
            if entries[index].generation == generation {
                removed.push(entries.remove(index));
            } else {
                index += 1;
            }
        }
        removed
    };
    for entry in &entries {
        entry.cancellation.cancel();
    }
    for entry in entries {
        if let Some(runtime) = entry.runtime.upgrade() {
            let mut session = runtime.lock().await;
            if session
                .task
                .as_ref()
                .is_some_and(|task| task.generation == generation)
            {
                session.task.take();
            }
        }
    }
}

fn refuse(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_request(message.into(), None)
}

/// A call stops early either with a refusal (an error the host shows the model) or with a tool
/// result (an engine error, which carries its delivery disposition).
enum Halt {
    Error(ErrorData),
    Tool(Box<CallToolResult>),
}

impl From<ErrorData> for Halt {
    fn from(error: ErrorData) -> Self {
        Self::Error(error)
    }
}

type Step<T> = Result<T, Halt>;

fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text.into())])
}

fn failed(text: impl Into<String>) -> Halt {
    Halt::Tool(Box::new(CallToolResult::error(vec![Content::text(
        text.into(),
    )])))
}

fn arg<'a>(arguments: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn arg_f64(arguments: &Map<String, Value>, key: &str) -> Option<f64> {
    arguments.get(key).and_then(Value::as_f64)
}

fn app_matches(app: &AppFacts, query: &str) -> bool {
    let query = query.trim();
    app.name.eq_ignore_ascii_case(query)
        || app.name.eq_ignore_ascii_case(&format!("{query}.exe"))
        || app
            .bundle_id
            .as_deref()
            .is_some_and(|bundle| bundle.eq_ignore_ascii_case(query))
}

fn parse_apps(data: &Value) -> Vec<AppFacts> {
    data.get("apps")
        .and_then(Value::as_array)
        .map(|apps| {
            apps.iter()
                .filter_map(|app| {
                    Some(AppFacts {
                        name: app.get("name")?.as_str()?.to_owned(),
                        bundle_id: app
                            .get("bundle_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        pid: u32::try_from(app.get("pid")?.as_u64()?).ok()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_windows(data: &Value) -> Vec<Value> {
    data.as_array()
        .cloned()
        .or_else(|| data.get("windows").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

fn window_pid(window: &Value) -> Option<u32> {
    window
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
}

fn window_shown(window: &Value) -> bool {
    window.get("visible").and_then(Value::as_bool) != Some(false)
        && window.get("minimized").and_then(Value::as_bool) != Some(true)
}

fn window_id(window: &Value) -> Option<&str> {
    window.get("id").and_then(Value::as_str)
}

/// A PNG scaled so its long side is at most 1,568 pixels, as base64, with its size.
fn downscale_png(bytes: &[u8]) -> Result<(String, u32, u32), String> {
    use base64::Engine as _;
    const LONG_SIDE: u32 = 1568;
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|_| "computer_use_protocol_error: the screenshot could not be read".to_string())?;
    let (width, height) = (image.width(), image.height());
    let image = if width.max(height) > LONG_SIDE {
        let scale = f64::from(LONG_SIDE) / f64::from(width.max(height));
        image.resize(
            ((f64::from(width) * scale).round() as u32).max(1),
            ((f64::from(height) * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageOutputFormat::Png)
        .map_err(|_| {
            "computer_use_protocol_error: the screenshot could not be encoded".to_string()
        })?;
    Ok((
        base64::engine::general_purpose::STANDARD.encode(encoded.into_inner()),
        image.width(),
        image.height(),
    ))
}

impl SessionRuntime {
    /// A runtime with stand-in host probes, engine and state folder, for tests (which run on
    /// Unix: the stand-in engine is a shell script).
    #[cfg(all(test, unix))]
    fn for_tests(
        probe: Arc<dyn HostProbe>,
        platform: Platform,
        executable: PathBuf,
        runs_root: PathBuf,
    ) -> Self {
        Self {
            session_id: None,
            task: None,
            probe,
            platform,
            executable: Some(executable),
            runs_root: Some(runs_root),
        }
    }

    pub(crate) fn ensure_session(&self, session_id: &str) -> Result<(), ErrorData> {
        if self
            .session_id
            .as_deref()
            .is_some_and(|bound| bound != session_id)
        {
            return Err(refuse(
                "computer_use_session_mismatch: observations cannot cross chat connections",
            ));
        }
        Ok(())
    }

    async fn start_task(&mut self, session_id: &str, generation: &str) -> Result<(), ErrorData> {
        if self
            .task
            .as_ref()
            .is_some_and(|task| task.generation == generation)
        {
            return Ok(());
        }
        self.task.take();
        let executable = match &self.executable {
            Some(executable) => executable.clone(),
            None => {
                tokio::task::spawn_blocking(manifest::locate)
                    .await
                    .map_err(|_| {
                        refuse(
                            "computer_use_missing_runtime: the native runtime could not be checked",
                        )
                    })?
                    .map_err(|error| refuse(error.to_string()))?
                    .executable
            }
        };
        let root = self
            .runs_root
            .clone()
            .or_else(runs_root)
            .ok_or_else(|| refuse("computer_use_unsupported_environment: no folder for Biorouter Copilot's working state"))?;
        static COLLECTED: std::sync::Once = std::sync::Once::new();
        let orphan_root = root.clone();
        COLLECTED.call_once(move || collect_orphans(&orphan_root));
        use sha2::Digest;
        let chat: String = format!("{:x}", sha2::Sha256::digest(session_id.as_bytes()))
            .chars()
            .take(32)
            .collect();
        let folder = root.join(chat).join(plan_session(generation));
        let engine = runner::Engine {
            executable,
            home: folder.clone(),
            temp: folder.join("tmp"),
        };
        for path in [&folder, &engine.temp, &folder.join("shots")] {
            create_private_dir(path).map_err(|error| {
                refuse(format!(
                    "computer_use_unsupported_environment: Biorouter Copilot's working folder could not be created ({error})"
                ))
            })?;
        }
        let _ = std::fs::write(folder.join("owner"), std::process::id().to_string());
        self.task = Some(Task {
            generation: generation.to_owned(),
            folder,
            engine,
            refs: HashMap::new(),
            observed: HashMap::new(),
            shots: HashMap::new(),
        });
        Ok(())
    }

    fn ctx(&self) -> plan::Ctx {
        plan::Ctx {
            platform: self.platform,
            session: self
                .task
                .as_ref()
                .map(|task| plan_session(&task.generation))
                .unwrap_or_default(),
        }
    }

    fn task(&mut self) -> &mut Task {
        self.task.as_mut().expect("task started before dispatch")
    }

    /// Run one command, turning a stop, a timeout or a spawn failure into words for the model.
    async fn exec(
        &self,
        invocation: &runner::Invocation,
        cancel: &CancellationToken,
    ) -> Step<runner::Envelope> {
        let engine = self
            .task
            .as_ref()
            .map(|task| task.engine.clone())
            .ok_or_else(|| Halt::Error(refuse("computer_use_cancelled")))?;
        runner::run(&engine, invocation, cancel).await.map_err(|error| {
            Halt::Error(refuse(match error {
                runner::RunError::Cancelled { killed, physical } => {
                    let mut text = "computer_use_cancelled: Biorouter Copilot stopped this operation; its outcome may be uncertain. Observe before any further action.".to_string();
                    if killed && physical {
                        text.push_str(" If the pointer seems stuck, click once in an empty area.");
                    }
                    text
                }
                runner::RunError::TimedOut { .. } => "computer_use_timeout: the operation did not finish in time and may have taken effect. Do not repeat it; observe the app first.".into(),
                runner::RunError::Spawn(text) | runner::RunError::Protocol(text) => text,
            }))
        })
    }

    async fn exec_ok(
        &self,
        invocation: &runner::Invocation,
        cancel: &CancellationToken,
    ) -> Step<Value> {
        let envelope = self.exec(invocation, cancel).await?;
        if envelope.ok {
            Ok(envelope.data)
        } else {
            Err(failed(render::error(&envelope.error)))
        }
    }

    async fn apps(&self, cancel: &CancellationToken) -> Step<Vec<AppFacts>> {
        let data = self.exec_ok(&plan::list_apps(&self.ctx()), cancel).await?;
        Ok(parse_apps(&data))
    }

    /// The running app a call names, with every refusal that applies to any of its instances.
    async fn resolve(
        &self,
        query: &str,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<Target> {
        let instances: Vec<AppFacts> = self
            .apps(cancel)
            .await?
            .into_iter()
            .filter(|app| app_matches(app, query))
            .collect();
        let Some(first) = instances.first() else {
            return Err(Halt::Error(refuse(format!(
                "computer_use_app_not_found: no running app is named \"{}\". Call list_apps and use a name exactly as it shows, or open_app to start it.",
                crate::untrusted_text::framed_label(query, 120)
            ))));
        };
        let target = Target {
            name: first.name.clone(),
            instances: instances.clone(),
            window_id: None,
            window_title: None,
        };
        check_identity(&target, identity)?;
        Ok(target)
    }

    async fn windows(&self, app: Option<&str>, cancel: &CancellationToken) -> Step<Vec<Value>> {
        let data = self
            .exec_ok(&plan::list_windows(&self.ctx(), app), cancel)
            .await?;
        Ok(parse_windows(&data))
    }

    /// A reference from an observation this task ran, with what was recorded about it.
    fn owned(&self, reference: &str) -> Step<Owned> {
        self.task
            .as_ref()
            .and_then(|task| task.refs.get(reference))
            .cloned()
            .ok_or_else(|| {
                Halt::Error(refuse(format!(
                    "computer_use_stale_reference: {} is not from an observation Biorouter Copilot made in this request. Call get_app_state (or find_elements) again and use a reference from the latest result.",
                    crate::untrusted_text::framed_label(reference, 80)
                )))
            })
    }

    /// The privacy settings refusal reads the window titles as they are now (AD-12).
    async fn check_settings(&self, target: &Target, cancel: &CancellationToken) -> Step<()> {
        if !target.any(guard::is_settings_app) {
            return Ok(());
        }
        let pids = target.pids();
        let titles: Vec<String> = self
            .windows(Some(&target.name), cancel)
            .await?
            .iter()
            .filter(|window| window_pid(window).is_some_and(|pid| pids.contains(&pid)))
            .filter_map(|window| {
                window
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        let privacy = titles.is_empty()
            || titles
                .iter()
                .any(|title| target.any(|app| guard::is_privacy_settings(app, Some(title))));
        if privacy {
            return Err(Halt::Error(refuse(guard::PRIVACY_SETTINGS_REFUSAL)));
        }
        Ok(())
    }

    /// Everything an act call into `target` must pass: terminals for input, privacy settings.
    async fn check_act(
        &self,
        target: &Target,
        input: bool,
        cancel: &CancellationToken,
    ) -> Step<()> {
        if input && target.any(guard::is_terminal) {
            return Err(Halt::Error(refuse(guard::TERMINAL_REFUSAL)));
        }
        if input && self.probe.secure_input() {
            return Err(Halt::Error(refuse(guard::SECURE_INPUT_REFUSAL)));
        }
        self.check_settings(target, cancel).await
    }

    /// AD-12 on Windows and Linux, where `secure_input` is always false: `press_key` goes to the
    /// app's focused element, so it is refused while a password field this task recorded in that
    /// app holds the keyboard (any key, as macOS secure input refuses them all). One `is` read per
    /// recorded password field, none when the observation held none. It sees only fields an
    /// observation in this request recorded, and a read that fails (the field is gone, or no
    /// longer proven unique) counts as not focused. `type_text` and `set_value` act on a
    /// reference, which `write` refuses when it is a password field.
    async fn check_password_focus(&self, target: &Target, cancel: &CancellationToken) -> Step<()> {
        if self.platform == Platform::MacOs {
            return Ok(());
        }
        let key = target.key();
        let mut secure: Vec<String> = self
            .task
            .as_ref()
            .map(|task| {
                task.refs
                    .iter()
                    .filter(|(_, owned)| owned.target.key() == key && owned.record.is_secure())
                    .map(|(reference, _)| reference.clone())
                    .collect()
            })
            .unwrap_or_default();
        secure.sort();
        for reference in secure.iter().take(8) {
            match self
                .exec(&plan::is_focused(&self.ctx(), reference), cancel)
                .await
            {
                Ok(envelope)
                    if envelope.ok
                        && envelope.data.get("result").and_then(Value::as_bool) == Some(true) =>
                {
                    return Err(Halt::Error(refuse(guard::FOCUSED_PASSWORD_REFUSAL)));
                }
                Err(halt) if cancel.is_cancelled() => return Err(halt),
                _ => {}
            }
        }
        Ok(())
    }

    /// The window of `target` a screenshot or a coordinate refers to.
    async fn pick_window(
        &self,
        target: &Target,
        requested: Option<&str>,
        cancel: &CancellationToken,
    ) -> Step<Value> {
        let pids = target.pids();
        let windows: Vec<Value> = self
            .windows(Some(&target.name), cancel)
            .await?
            .into_iter()
            .filter(|window| window_pid(window).is_some_and(|pid| pids.contains(&pid)))
            .collect();
        if let Some(requested) = requested {
            return windows
                .into_iter()
                .find(|window| window_id(window) == Some(requested))
                .ok_or_else(|| {
                    Halt::Error(refuse(
                        "computer_use_window_not_found: that window id is not one of this app's windows. Call screen_capture with list_only.",
                    ))
                });
        }
        let shown: Vec<Value> = windows.into_iter().filter(window_shown).collect();
        if shown.len() == 1 {
            return Ok(shown.into_iter().next().unwrap());
        }
        let observed = self
            .task
            .as_ref()
            .and_then(|task| task.observed.get(&target.key()))
            .and_then(|observed| observed.window_id.clone());
        if let Some(observed) = observed {
            if let Some(window) = shown
                .iter()
                .find(|window| window_id(window) == Some(&observed))
            {
                return Ok(window.clone());
            }
        }
        if shown.is_empty() {
            return Err(Halt::Error(refuse(
                "computer_use_window_not_found: that app shows no window. Use open_app to bring it forward, then try again.",
            )));
        }
        let listing: Vec<String> = shown
            .iter()
            .map(|window| {
                format!(
                    "{} \"{}\"",
                    crate::untrusted_text::framed_label(window_id(window).unwrap_or_default(), 200),
                    crate::untrusted_text::framed_label(
                        window
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        200
                    )
                )
            })
            .collect();
        Err(Halt::Error(refuse(format!(
            "computer_use_ambiguous_window: that app has several windows; pass window_id, one of: {}",
            listing.join("; ")
        ))))
    }

    /// Capture one window into the task folder, read it, delete it, and remember its geometry.
    async fn capture_window(
        &mut self,
        target: &Target,
        window: &Value,
        cancel: &CancellationToken,
    ) -> Step<Content> {
        let id = window_id(window).unwrap_or_default().to_owned();
        let bounds = window.get("bounds").and_then(Rect::from);
        let path = self
            .task()
            .folder
            .join("shots")
            .join(format!("{}.png", uuid::Uuid::new_v4()));
        let data = self
            .exec_ok(
                &plan::screenshot_window(&self.ctx(), &id, &path.to_string_lossy()),
                cancel,
            )
            .await;
        let bytes = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        data?;
        let bytes = bytes
            .map_err(|_| failed("computer_use_protocol_error: the screenshot was not written"))?;
        let (encoded, width, height) = downscale_png(&bytes).map_err(failed)?;
        if let Some(bounds) = bounds {
            let key = target.key();
            self.task().shots.insert(
                key,
                Shot {
                    window_id: id,
                    bounds,
                    width: f64::from(width),
                    height: f64::from(height),
                },
            );
        }
        Ok(Content::image(encoded, "image/png"))
    }

    /// Screenshot pixels of `app`'s latest screenshot to screen points, refusing when the window
    /// moved since (AD-09).
    async fn point(
        &self,
        target: &Target,
        x: f64,
        y: f64,
        cancel: &CancellationToken,
    ) -> Step<(f64, f64)> {
        let shot = self
            .task
            .as_ref()
            .and_then(|task| task.shots.get(&target.key()))
            .cloned()
            .ok_or_else(|| {
                Halt::Error(refuse(
                    "computer_use_stale_state: coordinates refer to this app's latest screenshot, and there is none in this request. Call get_app_state with screenshot true (or screen_capture with app) first, or use a reference.",
                ))
            })?;
        let now = self
            .windows(Some(&target.name), cancel)
            .await?
            .into_iter()
            .find(|window| window_id(window) == Some(&shot.window_id))
            .and_then(|window| window.get("bounds").and_then(Rect::from));
        if !now.is_some_and(|bounds| bounds.same_place(&shot.bounds)) {
            return Err(Halt::Error(refuse(
                "computer_use_stale_state: the window moved or closed since the screenshot; take a new screenshot.",
            )));
        }
        shot.to_screen(x, y)
            .map_err(|text| Halt::Error(refuse(text)))
    }

    /// The end of an action: words for the model, and forgetting the app after an uncertain one.
    fn acted(
        &mut self,
        keys: &[String],
        envelope: runner::Envelope,
        what: &str,
        secure: bool,
    ) -> Step<CallToolResult> {
        let disposition = if envelope.ok {
            envelope
                .data
                .get("disposition")
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            envelope
                .error
                .get("disposition")
                .cloned()
                .unwrap_or(Value::Null)
        };
        if render::uncertain(&disposition) {
            for key in keys {
                self.task().forget(key);
            }
        }
        if envelope.ok {
            Ok(text_result(render::action(&envelope.data, what, secure)))
        } else {
            Err(failed(render::error(&envelope.error)))
        }
    }

    /// Run an action, forgetting the app if it was stopped or timed out mid-way.
    async fn act(
        &mut self,
        keys: &[String],
        invocation: runner::Invocation,
        what: &str,
        secure: bool,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        match self.exec(&invocation, cancel).await {
            Ok(envelope) => self.acted(keys, envelope, what, secure),
            Err(halt) => {
                for key in keys {
                    self.task().forget(key);
                }
                Err(halt)
            }
        }
    }

    pub async fn call(
        &mut self,
        session_id: &str,
        generation: &str,
        name: &str,
        arguments: Value,
        grant: Grant,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.ensure_session(session_id)?;
        self.session_id = Some(session_id.to_owned());
        let arguments = arguments.as_object().cloned().unwrap_or_default();
        let needed = contract::grant_for(name, Some(&arguments), self.platform);
        if !grant.covers(needed) {
            return Err(refuse(LOOK_ONLY_REFUSAL));
        }
        if self.probe.screen_locked() {
            return Err(refuse(guard::LOCKED_REFUSAL));
        }
        if self
            .task
            .as_ref()
            .is_some_and(|task| task.generation != generation)
        {
            self.task.take();
        }
        // A reference from another task or chat is refused before the engine is even located.
        for key in ["ref", "from_ref", "to_ref", "root"] {
            if let Some(reference) = arg(&arguments, key) {
                if let Err(Halt::Error(error)) = self.owned(reference) {
                    return Err(error);
                }
            }
        }
        self.start_task(session_id, generation).await?;
        let identity = self.probe.identity();
        let outcome = self
            .dispatch(name, &arguments, &identity, &cancellation)
            .await;
        match outcome {
            Ok(result) => Ok(result),
            Err(Halt::Error(error)) => Err(error),
            Err(Halt::Tool(result)) => Ok(*result),
        }
    }

    async fn dispatch(
        &mut self,
        name: &str,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        match name {
            "list_apps" => self.list_apps(identity, cancel).await,
            "get_app_state" => self.get_app_state(arguments, identity, cancel).await,
            "find_elements" => self.find_elements(arguments, identity, cancel).await,
            "wait_for" => self.wait_for(arguments, identity, cancel).await,
            "screen_capture" => self.screen_capture(arguments, identity, cancel).await,
            "click" => self.click(arguments, identity, cancel).await,
            "perform_secondary_action" => self.secondary(arguments, cancel).await,
            "scroll" => self.scroll(arguments, cancel).await,
            "drag" => self.drag(arguments, identity, cancel).await,
            "type_text" | "set_value" => self.write(name, arguments, cancel).await,
            "press_key" => self.press_key(arguments, identity, cancel).await,
            "open_app" => self.open_app(arguments, identity, cancel).await,
            _ => Err(Halt::Error(ErrorData::invalid_params(
                "Unknown Biorouter Copilot tool",
                None,
            ))),
        }
    }

    async fn list_apps(
        &mut self,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let apps = self.apps(cancel).await?;
        let mut hidden = false;
        let mut lines = Vec::new();
        for app in &apps {
            if identity.owns(app) || guard::is_security_prompt(app) {
                hidden = true;
                continue;
            }
            let bundle = app
                .bundle_id
                .as_deref()
                .map(|bundle| format!(" ({})", crate::untrusted_text::framed_label(bundle, 200)))
                .unwrap_or_default();
            lines.push(format!(
                "{}{bundle}",
                crate::untrusted_text::framed_label(&app.name, 200)
            ));
        }
        let mut body = lines.join("\n");
        body.push('\n');
        if hidden {
            body.push_str("[Biorouter itself and system security prompts are not listed; Biorouter Copilot cannot observe or control them.]\n");
        }
        Ok(text_result(render::frame("running apps", None, &body)))
    }

    async fn get_app_state(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let mut target = self
            .resolve(arg(arguments, "app").unwrap_or_default(), identity, cancel)
            .await?;
        let requested_window = arg(arguments, "window_id");
        if let Some(requested) = requested_window {
            self.pick_window(&target, Some(requested), cancel).await?;
        }
        let root = arg(arguments, "root");
        if let Some(root) = root {
            let owned = self.owned(root)?;
            if owned.target.key() != target.key() {
                return Err(Halt::Error(refuse("computer_use_stale_reference: root must be a reference from this app's latest observation.")));
            }
        }
        let request = plan::SnapshotRequest {
            app: &target.name,
            window_id: requested_window,
            surface: arg(arguments, "surface"),
            root,
            max_depth: arguments.get("max_depth").and_then(Value::as_u64),
            interactive_only: arguments
                .get("interactive_only")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            slow: target.slow_tree(),
            ..Default::default()
        };
        let data = self
            .exec_ok(&plan::snapshot(&self.ctx(), &request), cancel)
            .await?;
        let limit = arguments
            .get("text_limit")
            .and_then(Value::as_u64)
            .map(|limit| (limit as usize).clamp(1000, render::MAX_TEXT_LIMIT))
            .unwrap_or(render::DEFAULT_TEXT_LIMIT);
        let text = self.record_snapshot(&mut target, &data, limit);
        let mut content = vec![Content::text(text)];
        if arguments.get("screenshot").and_then(Value::as_bool) == Some(true) {
            let window = self.pick_window(&target, requested_window, cancel).await?;
            content.push(self.capture_window(&target, &window, cancel).await?);
        }
        Ok(CallToolResult::success(content))
    }

    /// Render a snapshot and remember its references, window and app for this task.
    fn record_snapshot(&mut self, target: &mut Target, data: &Value, limit: usize) -> String {
        target.window_id = data
            .pointer("/window/id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        target.window_title = data
            .pointer("/window/title")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut refs = HashMap::new();
        let text = render::snapshot(data, &target.name, limit, &mut refs);
        self.remember(target, refs);
        text
    }

    fn remember(&mut self, target: &Target, refs: HashMap<String, RefRecord>) {
        let task = self.task();
        for (reference, record) in refs {
            task.refs.insert(
                reference,
                Owned {
                    record,
                    target: target.clone(),
                },
            );
        }
        task.observed.insert(target.key(), target.clone());
    }

    async fn find_elements(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let target = self
            .resolve(arg(arguments, "app").unwrap_or_default(), identity, cancel)
            .await?;
        let window_id = arg(arguments, "window_id");
        if let Some(requested) = window_id {
            self.pick_window(&target, Some(requested), cancel).await?;
        }
        let request = plan::FindRequest {
            app: &target.name,
            window_id,
            role: arg(arguments, "role"),
            name: arg(arguments, "name"),
            text: arg(arguments, "text"),
            value: arg(arguments, "value"),
            slow: target.slow_tree(),
        };
        let data = self
            .exec_ok(&plan::find(&self.ctx(), &request), cancel)
            .await?;
        let mut refs = HashMap::new();
        let text = render::find(&data, &target.name, &mut refs);
        self.remember(&target, refs);
        Ok(text_result(text))
    }

    async fn wait_for(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let mut target = self
            .resolve(arg(arguments, "app").unwrap_or_default(), identity, cancel)
            .await?;
        let given = ["element", "gone", "text", "window_title"]
            .iter()
            .filter(|key| arguments.contains_key(**key))
            .count();
        if given > 1 {
            return Err(Halt::Error(refuse(
                "Give one of element, gone, text or window_title (or only ms).",
            )));
        }
        let ms = arguments.get("ms").and_then(Value::as_u64);
        let wait = Duration::from_millis(ms.unwrap_or(10_000)).min(plan::MAX_WAIT);
        if let (Some(element), _) | (_, Some(element)) =
            (arg(arguments, "element"), arg(arguments, "gone"))
        {
            let appear = arguments.contains_key("element");
            let request = plan::SnapshotRequest {
                app: &target.name,
                interactive_only: true,
                wait_for: appear.then_some(element),
                wait_for_gone: (!appear).then_some(element),
                wait: Some(wait),
                slow: target.slow_tree(),
                ..Default::default()
            };
            let data = self
                .exec_ok(&plan::snapshot(&self.ctx(), &request), cancel)
                .await?;
            let text = self.record_snapshot(&mut target, &data, render::DEFAULT_TEXT_LIMIT);
            let said = if appear {
                "It appeared."
            } else {
                "It is gone."
            };
            return Ok(text_result(format!("{said}\n{text}")));
        }
        let (request, said) = if let Some(text) = arg(arguments, "text") {
            (plan::WaitRequest::Text(text), "The text appeared.")
        } else if let Some(title) = arg(arguments, "window_title") {
            (plan::WaitRequest::Window(title), "The window appeared.")
        } else if ms.is_some() {
            (plan::WaitRequest::Pause, "Waited.")
        } else {
            return Err(Halt::Error(refuse(
                "Give element, gone, text, window_title, or ms to wait.",
            )));
        };
        self.exec_ok(
            &plan::wait(&self.ctx(), &target.name, request, wait),
            cancel,
        )
        .await?;
        Ok(text_result(said))
    }

    async fn screen_capture(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let chosen = ["app", "window_id", "display", "list_only"]
            .iter()
            .filter(|key| arguments.contains_key(**key))
            .count();
        if chosen != 1 {
            return Err(Halt::Error(refuse(
                "Give exactly one of app, window_id, display or list_only.",
            )));
        }
        if arguments
            .get("list_only")
            .and_then(Value::as_bool)
            .is_some()
        {
            return self.list_windows(identity, cancel).await;
        }
        if let Some(display) = arguments.get("display").and_then(Value::as_u64) {
            return self.capture_display(display, identity, cancel).await;
        }
        let (target, window) = if let Some(app) = arg(arguments, "app") {
            let target = self.resolve(app, identity, cancel).await?;
            let window = self.pick_window(&target, None, cancel).await?;
            (target, window)
        } else {
            let requested = arg(arguments, "window_id").unwrap_or_default();
            let window = self
                .windows(None, cancel)
                .await?
                .into_iter()
                .find(|window| window_id(window) == Some(requested))
                .ok_or_else(|| Halt::Error(refuse("computer_use_window_not_found: no window has that id. Call screen_capture with list_only.")))?;
            let pid = window_pid(&window).unwrap_or_default();
            let apps = self.apps(cancel).await?;
            let instances: Vec<AppFacts> = apps.into_iter().filter(|app| app.pid == pid).collect();
            let facts = instances.first().cloned().unwrap_or(AppFacts {
                name: window
                    .get("app_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                bundle_id: None,
                pid,
            });
            let target = Target {
                name: facts.name.clone(),
                instances: if instances.is_empty() {
                    vec![facts]
                } else {
                    instances
                },
                window_id: Some(requested.to_owned()),
                window_title: None,
            };
            check_identity(&target, identity)?;
            (target, window)
        };
        let image = self.capture_window(&target, &window, cancel).await?;
        let title = window.get("title").and_then(Value::as_str);
        Ok(CallToolResult::success(vec![
            Content::text(render::frame(&target.name, title, "[Screenshot of this window follows. Pixel coordinates for click and drag refer to it.]\n")),
            image,
        ]))
    }

    async fn list_windows(
        &mut self,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let apps = self.apps(cancel).await?;
        let hidden_pid = |pid: u32| {
            identity.owns_pid(pid)
                || apps
                    .iter()
                    .filter(|app| app.pid == pid)
                    .any(|app| identity.owns(app) || guard::is_security_prompt(app))
        };
        let mut lines = Vec::new();
        let mut hidden = false;
        for window in self.windows(None, cancel).await? {
            if window_pid(&window).is_none_or(hidden_pid) {
                hidden = true;
                continue;
            }
            let label = |key: &str| {
                crate::untrusted_text::framed_label(
                    window.get(key).and_then(Value::as_str).unwrap_or_default(),
                    200,
                )
            };
            let state = if window_shown(&window) {
                ""
            } else {
                " (hidden or minimized)"
            };
            lines.push(format!(
                "window_id {}: {} \"{}\"{state}",
                label("id"),
                label("app_name"),
                label("title")
            ));
        }
        let displays = self
            .exec_ok(&plan::list_displays(&self.ctx()), cancel)
            .await?;
        for (index, display) in displays
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if let Some(bounds) = display.get("bounds").and_then(Rect::from) {
                lines.push(format!(
                    "display {index}: {} by {} points{}",
                    bounds.width,
                    bounds.height,
                    if display.get("is_primary").and_then(Value::as_bool) == Some(true) {
                        " (main)"
                    } else {
                        ""
                    }
                ));
            }
        }
        let mut body = lines.join("\n");
        body.push('\n');
        if hidden {
            body.push_str(
                "[Biorouter's own windows and system security prompts are not listed.]\n",
            );
        }
        Ok(text_result(render::frame(
            "windows and displays",
            None,
            &body,
        )))
    }

    /// AD-12: a display capture would show Biorouter's own windows, which can hold other
    /// (possibly private) chats, and any system security prompt or password store window on
    /// it, which every other observation refuses; so it is refused while one is on that display.
    async fn capture_display(
        &mut self,
        display: u64,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let displays = self
            .exec_ok(&plan::list_displays(&self.ctx()), cancel)
            .await?;
        let bounds = displays
            .as_array()
            .and_then(|list| list.get(display as usize))
            .and_then(|entry| entry.get("bounds"))
            .and_then(Rect::from)
            .ok_or_else(|| Halt::Error(refuse("computer_use_display_not_found: no display has that index. Call screen_capture with list_only.")))?;
        // A process the guard calls a security prompt (password stores included), by its
        // listed facts or, failing a listing, by the name the window itself reports.
        let prompts: Vec<u32> = self
            .apps(cancel)
            .await?
            .into_iter()
            .filter(guard::is_security_prompt)
            .map(|app| app.pid)
            .collect();
        let is_prompt = |window: &Value| {
            window_pid(window).is_some_and(|pid| prompts.contains(&pid))
                || guard::is_security_prompt(&AppFacts {
                    name: window
                        .get("app_name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    bundle_id: None,
                    pid: 0,
                })
        };
        let on_display = |window: &Value| {
            window
                .get("bounds")
                .and_then(Rect::from)
                .map(|rect| rect.intersects(&bounds))
        };
        let windows = self.windows(None, cancel).await?;
        let covered = windows.iter().any(|window| {
            window_shown(window)
                && window_pid(window).is_some_and(|pid| identity.owns_pid(pid))
                && on_display(window) == Some(true)
        });
        if covered {
            return Err(Halt::Error(refuse(
                "computer_use_refused: a Biorouter window is on that display, and a display screenshot would capture it (it can show other chats). Capture an app window instead (screen_capture with app or window_id).",
            )));
        }
        // A prompt window that reports no bounds could be on any display, so it counts.
        if windows.iter().any(|window| {
            window_shown(window) && is_prompt(window) && on_display(window) != Some(false)
        }) {
            return Err(Halt::Error(refuse(
                "computer_use_refused: a system security prompt or password window is on that display, and a display screenshot would capture it. Capture an app window instead (screen_capture with app or window_id), or ask the person to finish with the prompt first.",
            )));
        }
        let path = self
            .task()
            .folder
            .join("shots")
            .join(format!("{}.png", uuid::Uuid::new_v4()));
        let data = self
            .exec_ok(
                &plan::screenshot_display(&self.ctx(), display, &path.to_string_lossy()),
                cancel,
            )
            .await;
        let bytes = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        data?;
        let bytes = bytes
            .map_err(|_| failed("computer_use_protocol_error: the screenshot was not written"))?;
        let (encoded, _, _) = downscale_png(&bytes).map_err(failed)?;
        Ok(CallToolResult::success(vec![
            Content::text(render::frame("display", None, "[Screenshot of the display follows. Click and drag by reference or by an app window's screenshot, not by display pixels.]\n")),
            Content::image(encoded, "image/png"),
        ]))
    }

    async fn click(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let count = arguments.get("count").and_then(Value::as_u64).unwrap_or(1);
        let right = arg(arguments, "button") == Some("right");
        if let Some(reference) = arg(arguments, "ref") {
            if arguments.contains_key("x") || arguments.contains_key("y") {
                return Err(Halt::Error(refuse(
                    "Give ref, or app with x and y, not both.",
                )));
            }
            let owned = self.owned(reference)?;
            self.check_act(&owned.target, false, cancel).await?;
            let invocation = plan::click_ref(&self.ctx(), reference, count, right)
                .map_err(|t| Halt::Error(refuse(t)))?;
            return self
                .act(&[owned.target.key()], invocation, "Clicked", false, cancel)
                .await;
        }
        let (Some(app), Some(x), Some(y)) = (
            arg(arguments, "app"),
            arg_f64(arguments, "x"),
            arg_f64(arguments, "y"),
        ) else {
            return Err(Halt::Error(refuse("Give ref, or app with x and y.")));
        };
        let target = self.resolve(app, identity, cancel).await?;
        self.require_observed(&target)?;
        self.check_act(&target, false, cancel).await?;
        let (sx, sy) = self.point(&target, x, y, cancel).await?;
        let invocation = plan::click_point(&self.ctx(), sx, sy, count, right)
            .map_err(|t| Halt::Error(refuse(t)))?;
        self.act(
            &[target.key()],
            invocation,
            "Clicked at that point",
            false,
            cancel,
        )
        .await
    }

    fn require_observed(&self, target: &Target) -> Step<()> {
        let observed = self.task.as_ref().is_some_and(|task| {
            task.observed.contains_key(&target.key()) || task.shots.contains_key(&target.key())
        });
        if observed {
            Ok(())
        } else {
            Err(Halt::Error(refuse(STALE_REFUSAL)))
        }
    }

    async fn secondary(
        &mut self,
        arguments: &Map<String, Value>,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let reference = arg(arguments, "ref").unwrap_or_default();
        let action = arg(arguments, "action").unwrap_or_default();
        let owned = self.owned(reference)?;
        if action == "clear" && owned.record.is_secure() {
            return Err(Halt::Error(refuse(guard::PASSWORD_FIELD_REFUSAL)));
        }
        if contract::grant_for("perform_secondary_action", Some(arguments), self.platform)
            == Grant::Act
        {
            self.check_act(&owned.target, false, cancel).await?;
        }
        let invocation = plan::secondary(&self.ctx(), reference, action, arg(arguments, "option"))
            .map_err(|t| Halt::Error(refuse(t)))?;
        let what = format!("Performed {action}");
        self.act(
            &[owned.target.key()],
            invocation,
            &what,
            owned.record.is_secure(),
            cancel,
        )
        .await
    }

    async fn scroll(
        &mut self,
        arguments: &Map<String, Value>,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let reference = arg(arguments, "ref").unwrap_or_default();
        let owned = self.owned(reference)?;
        let direction = arg(arguments, "direction").unwrap_or("down");
        let amount = arguments.get("amount").and_then(Value::as_u64).unwrap_or(3);
        let invocation = plan::scroll(&self.ctx(), reference, direction, amount)
            .map_err(|t| Halt::Error(refuse(t)))?;
        self.act(&[owned.target.key()], invocation, "Scrolled", false, cancel)
            .await
    }

    /// One end of a drag: a reference from this task, or pixels of `app`'s latest screenshot.
    async fn drag_end(
        &self,
        arguments: &Map<String, Value>,
        side: &str,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<(Target, Option<String>, Option<(f64, f64)>)> {
        if let Some(reference) = arg(arguments, &format!("{side}_ref")) {
            if arguments.contains_key(&format!("{side}_x"))
                || arguments.contains_key(&format!("{side}_y"))
            {
                return Err(Halt::Error(refuse(format!(
                    "Give {side}_ref or {side}_x and {side}_y, not both."
                ))));
            }
            let owned = self.owned(reference)?;
            return Ok((owned.target, Some(reference.to_owned()), None));
        }
        let (Some(x), Some(y)) = (
            arg_f64(arguments, &format!("{side}_x")),
            arg_f64(arguments, &format!("{side}_y")),
        ) else {
            return Err(Halt::Error(refuse(format!(
                "Give {side}_ref, or {side}_x and {side}_y with app."
            ))));
        };
        let app = arg(arguments, "app").ok_or_else(|| {
            Halt::Error(refuse(
                "Coordinates need app: the app whose latest screenshot they refer to.",
            ))
        })?;
        let target = self.resolve(app, identity, cancel).await?;
        let point = self.point(&target, x, y, cancel).await?;
        Ok((target, None, Some(point)))
    }

    async fn drag(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let (from_target, from_ref, from_point) =
            self.drag_end(arguments, "from", identity, cancel).await?;
        let (to_target, to_ref, to_point) =
            self.drag_end(arguments, "to", identity, cancel).await?;
        for target in [&from_target, &to_target] {
            // A drop into a terminal types there; either end in one is refused.
            if target.any(guard::is_terminal) {
                return Err(Halt::Error(refuse(guard::TERMINAL_REFUSAL)));
            }
            self.check_settings(target, cancel).await?;
        }
        let verify = arg(arguments, "verify");
        let invocation = plan::drag(
            &self.ctx(),
            drag_end_of(&from_ref, from_point),
            drag_end_of(&to_ref, to_point),
            verify,
        )
        .map_err(|t| Halt::Error(refuse(t)))?;
        let keys = [from_target.key(), to_target.key()];
        let envelope = match self.exec(&invocation, cancel).await {
            Ok(envelope) => envelope,
            Err(halt) => {
                for key in &keys {
                    self.task().forget(key);
                }
                return Err(halt);
            }
        };
        if !envelope.ok
            && verify.is_some()
            && envelope.error.get("code").and_then(Value::as_str) == Some("TIMEOUT")
        {
            for key in &keys {
                self.task().forget(key);
            }
            return Err(failed(format!(
                "The drag was delivered, but \"{}\" did not appear at the destination within {} seconds: the target may not have accepted the drop. Observe both apps before trying again.",
                crate::untrusted_text::framed_label(verify.unwrap_or_default(), 200),
                plan::DRAG_VERIFY.as_secs()
            )));
        }
        let what = if verify.is_some() && envelope.ok {
            "Dragged and dropped; the expected element appeared at the destination"
        } else {
            "Dragged and dropped. Observe the destination before saying it worked"
        };
        self.acted(&keys, envelope, what, false)
    }

    async fn write(
        &mut self,
        name: &str,
        arguments: &Map<String, Value>,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let reference = arg(arguments, "ref").unwrap_or_default();
        let owned = self.owned(reference)?;
        if owned.record.is_secure() {
            return Err(Halt::Error(refuse(guard::PASSWORD_FIELD_REFUSAL)));
        }
        self.check_act(&owned.target, true, cancel).await?;
        let (invocation, what) = if name == "type_text" {
            let text = arg(arguments, "text").unwrap_or_default();
            (
                plan::type_text(&self.ctx(), reference, text),
                "Typed the text",
            )
        } else {
            let value = arg(arguments, "value").unwrap_or_default();
            (
                plan::set_value(&self.ctx(), reference, value),
                "Set the value",
            )
        };
        let invocation = invocation.map_err(|t| Halt::Error(refuse(t)))?;
        self.act(&[owned.target.key()], invocation, what, false, cancel)
            .await
    }

    async fn press_key(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let target = self
            .resolve(arg(arguments, "app").unwrap_or_default(), identity, cancel)
            .await?;
        self.require_observed(&target)?;
        self.check_act(&target, true, cancel).await?;
        self.check_password_focus(&target, cancel).await?;
        let window = arg(arguments, "window_id");
        if let Some(requested) = window {
            self.pick_window(&target, Some(requested), cancel).await?;
        }
        let key = arg(arguments, "key").unwrap_or_default();
        let invocation = plan::press(&self.ctx(), &target.name, key, window)
            .map_err(|t| Halt::Error(refuse(t)))?;
        self.act(
            &[target.key()],
            invocation,
            "Pressed the key",
            false,
            cancel,
        )
        .await
    }

    async fn open_app(
        &mut self,
        arguments: &Map<String, Value>,
        identity: &Identity,
        cancel: &CancellationToken,
    ) -> Step<CallToolResult> {
        let app = arg(arguments, "app").unwrap_or_default().trim().to_owned();
        let named = AppFacts {
            name: app.clone(),
            bundle_id: Some(app.clone()),
            pid: 0,
        };
        let lowered = app.to_lowercase();
        if guard::is_biorouter_bundle(&named)
            || lowered == "biorouter"
            || lowered == "biorouter copilot"
        {
            return Err(Halt::Error(refuse(guard::BIOROUTER_REFUSAL)));
        }
        if guard::is_security_prompt(&named) {
            return Err(Halt::Error(refuse(guard::SECURITY_PROMPT_REFUSAL)));
        }
        let running: Vec<AppFacts> = self
            .apps(cancel)
            .await?
            .into_iter()
            .filter(|facts| app_matches(facts, &app))
            .collect();
        if !running.is_empty() {
            check_identity(
                &Target {
                    name: running[0].name.clone(),
                    instances: running.clone(),
                    window_id: None,
                    window_title: None,
                },
                identity,
            )?;
        } else if matches!(self.platform, Platform::LinuxX11 | Platform::LinuxWayland)
            && !linux_desktop_entry(&app)
        {
            // On Linux the engine also starts bare commands from PATH; Copilot opens
            // applications, so only an installed desktop entry is accepted.
            return Err(Halt::Error(refuse(
                "computer_use_app_not_found: on Linux, open_app takes an installed application's desktop entry name (for example org.gnome.Calculator or gedit).",
            )));
        }
        let invocation = plan::open_app(&self.ctx(), &app).map_err(|t| Halt::Error(refuse(t)))?;
        let data = self.exec_ok(&invocation, cancel).await?;
        let pid = data
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok());
        if pid.is_some_and(|pid| identity.owns_pid(pid)) {
            return Err(Halt::Error(refuse(guard::BIOROUTER_REFUSAL)));
        }
        let title = data.pointer("/window/title").and_then(Value::as_str);
        let opened = data
            .get("app")
            .and_then(Value::as_str)
            .unwrap_or(&app)
            .to_owned();
        Ok(text_result(render::frame(
            &opened,
            title,
            "[Opened and brought forward. Call get_app_state for it before acting.]\n",
        )))
    }
}

fn drag_end_of(reference: &Option<String>, point: Option<(f64, f64)>) -> plan::End<'_> {
    match (reference, point) {
        (Some(reference), _) => plan::End::Ref(reference.as_str()),
        (None, Some((x, y))) => plan::End::Point(x, y),
        (None, None) => plan::End::Point(0.0, 0.0),
    }
}

/// `--session` must be 1 to 64 letters, digits, `-` or `_`.
fn plan_session(generation: &str) -> String {
    let cleaned: String = generation
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "task".into()
    } else {
        cleaned
    }
}

fn check_identity(target: &Target, identity: &Identity) -> Step<()> {
    if target.any(|app| identity.owns(app)) {
        return Err(Halt::Error(refuse(guard::BIOROUTER_REFUSAL)));
    }
    if target.any(guard::is_security_prompt) {
        return Err(Halt::Error(refuse(guard::SECURITY_PROMPT_REFUSAL)));
    }
    Ok(())
}

/// Whether `name` is an installed desktop entry (a `.desktop` file in the XDG data folders).
fn linux_desktop_entry(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return false;
    }
    let file = if name.ends_with(".desktop") {
        name.to_owned()
    } else {
        format!("{name}.desktop")
    };
    let mut folders: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    {
        folders.push(home);
    }
    let dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    folders.extend(
        dirs.split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
    );
    folders.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    folders.iter().any(|folder| {
        let applications = folder.join("applications");
        applications.join(&file).is_file()
            || std::fs::read_dir(&applications).is_ok_and(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&file)
                })
            })
    })
}

/// The app bundle an executable runs from (`…/Biorouter Copilot.app` for
/// `…/Biorouter Copilot.app/Contents/MacOS/<engine>`), which is what macOS System Settings lists
/// and grants Accessibility and Screen Recording to. `None` for an executable outside a bundle.
pub fn app_bundle_of(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_owned())
}

pub fn diagnostics() -> Value {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "backend host".into());
    match manifest::probe() {
        Ok(payload) => {
            // `executable` stays exact for `biorouter doctor --format json` and the payload
            // acceptance check; a person is shown `app_bundle` where there is one.
            let mut value = json!({"status":"probe_pending","runtime_version":manifest::UPSTREAM_VERSION,"target":manifest::target(),"executable":payload.executable,"development_override":payload.development_override,"permissions":"unknown","integrity":"verified_on_start","host":host});
            if let Some(bundle) = app_bundle_of(&payload.executable) {
                value["app_bundle"] = json!(bundle);
            }
            value
        }
        Err(error) => {
            json!({"status":if error.to_string().contains("incompatible_runtime") {"incompatible_runtime"} else {"missing_runtime"},"runtime_version":manifest::UPSTREAM_VERSION,"target":manifest::target(),"permissions":"unknown","integrity":"verified_on_start","host":host,"error":error.to_string()})
        }
    }
}

type ReadinessCache = tokio::sync::Mutex<Option<(std::time::Instant, Value)>>;
static READINESS: once_cell::sync::Lazy<ReadinessCache> =
    once_cell::sync::Lazy::new(Default::default);

/// The first `at-spi2-core` known to carry the bridge fix the engine needs (AD-06). Ubuntu 24.04
/// ships it; the exact first release with the fix is measured before release, so this is the
/// conservative floor until then.
pub const ATSPI_MINIMUM: &str = "2.52.0";

#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn version_at_least(found: &str, minimum: &str) -> bool {
    let parse = |text: &str| -> Vec<u64> {
        text.split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .take(3)
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    let (found, minimum) = (parse(found), parse(minimum));
    !found.is_empty() && found >= minimum
}

/// Run the engine's permission read (it never prompts) through the same disclaimed spawn as
/// every command, so it reports Biorouter Copilot's own grants.
async fn doctor() -> Result<Value, String> {
    let payload = tokio::task::spawn_blocking(manifest::locate)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let session = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
            && (std::env::var_os("DISPLAY").is_some()
                || std::env::var_os("WAYLAND_DISPLAY").is_some());
        if !session {
            return Ok(
                json!({"state":"desktop_unavailable","desktop_available":false,"accessibility":false,"screen_recording":false,"capture_available":false,"message":"No graphical desktop session is reachable from Biorouter's backend (no session bus or display)."}),
            );
        }
        match linux::atspi_version() {
            Some(version) if version_at_least(&version, ATSPI_MINIMUM) => {}
            found => {
                let found = found.unwrap_or_else(|| "not readable".into());
                return Ok(
                    json!({"state":"unsupported_environment","desktop_available":true,"accessibility":false,"screen_recording":false,"capture_available":false,"message":format!("Biorouter Copilot needs the accessibility bridge at-spi2-core {ATSPI_MINIMUM} or newer (Ubuntu 24.04 or later); this computer has {found}. An older bridge can crash the app being controlled.")}),
                );
            }
        }
    }
    let home = tempfile::Builder::new()
        .prefix("biorouter-copilot-probe-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let engine = runner::Engine {
        executable: payload.executable,
        home: home.path().to_owned(),
        temp: home.path().to_owned(),
    };
    let ctx = plan::Ctx {
        platform: Platform::current(),
        session: "readiness".into(),
    };
    let envelope = runner::run(&engine, &plan::permissions(&ctx), &CancellationToken::new())
        .await
        .map_err(|e| e.to_string())?;
    if !envelope.ok {
        return Err(render::error(&envelope.error));
    }
    // A state the engine could not read counts as not granted: "ready" is said only on
    // evidence.
    let state = |key: &str| {
        json!(matches!(
            envelope
                .data
                .pointer(&format!("/{key}/state"))
                .and_then(Value::as_str),
            Some("granted" | "not_required")
        ))
    };
    let accessibility = state("accessibility");
    let screen_recording = state("screen_recording");
    let denied = accessibility == json!(false) || screen_recording == json!(false);
    Ok(json!({
        "state": if denied { "os_permission_required" } else { "ready" },
        "desktop_available": true,
        "accessibility": accessibility,
        "screen_recording": screen_recording,
        "capture_available": screen_recording != json!(false),
        "message": if denied && cfg!(target_os = "macos") {
            "Allow Biorouter Copilot in System Settings (Accessibility, and Screen Recording for screenshots), then check again."
        } else if denied {
            "The desktop's accessibility service did not answer Biorouter Copilot. Make sure accessibility is on for this desktop session, then check again."
        } else {
            "Biorouter Copilot is ready."
        },
    }))
}

/// Ask the operating system to let Biorouter Copilot in (AD-04), then read readiness again.
///
/// On macOS this runs `permissions --request` through the same disclaimed spawn as every command,
/// so the system prompts name Biorouter Copilot and the grants land on its bundle, never on
/// Biorouter. The engine also asks about Automation for System Events, which Copilot does not use;
/// the person may refuse that one. Elsewhere there is nothing to request: Windows needs no grant
/// and Linux reads the session's accessibility bus. Called only from the setup route after a
/// person's press, never from a tool call.
pub async fn request_permissions() -> Value {
    #[cfg(target_os = "macos")]
    {
        let asked = tokio::time::timeout(
            plan::REQUEST_PERMISSIONS + Duration::from_secs(15),
            request_os_permissions(),
        )
        .await;
        let failure = match asked {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("The system's permission prompts did not finish in time.".to_owned()),
        };
        let mut readiness = probe_readiness(true).await;
        if let Some(failure) = failure {
            readiness["request_error"] = json!(failure);
        }
        readiness
    }
    #[cfg(not(target_os = "macos"))]
    {
        probe_readiness(true).await
    }
}

#[cfg(target_os = "macos")]
async fn request_os_permissions() -> Result<(), String> {
    let payload = tokio::task::spawn_blocking(manifest::locate)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let home = tempfile::Builder::new()
        .prefix("biorouter-copilot-request-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let engine = runner::Engine {
        executable: payload.executable,
        home: home.path().to_owned(),
        temp: home.path().to_owned(),
    };
    let ctx = plan::Ctx {
        platform: Platform::current(),
        session: "readiness".into(),
    };
    let envelope = runner::run(
        &engine,
        &plan::request_permissions(&ctx),
        &CancellationToken::new(),
    )
    .await
    .map_err(|e| e.to_string())?;
    if envelope.ok {
        Ok(())
    } else {
        Err(render::error(&envelope.error))
    }
}

/// The readiness probe's own bound. It must EXCEED the bound of the command it supervises (the
/// permission read's deadline plus the runner's grace): an outer bound that does not exceed the
/// inner one always wins, because it starts earlier, and the engine's own answer is never
/// delivered.
const PROBE_BOUND: Duration = Duration::from_secs(45);

pub async fn probe_readiness(refresh: bool) -> Value {
    let mut cache = READINESS.lock().await;
    if !refresh {
        if let Some((observed, value)) = cache.as_ref() {
            if observed.elapsed() < Duration::from_secs(30) {
                return value.clone();
            }
        }
    }
    let mut result = diagnostics();
    if result.get("status").and_then(Value::as_str) == Some("probe_pending") {
        match tokio::time::timeout(PROBE_BOUND, doctor()).await {
            Ok(Ok(probe)) => {
                result["status"] = probe["state"].clone();
                result["permissions"] = json!({"accessibility":probe["accessibility"],"screen_recording":probe["screen_recording"]});
                for key in ["desktop_available", "capture_available", "message"] {
                    result[key] = probe[key].clone();
                }
                result["integrity"] = json!("verified");
            }
            Ok(Err(error)) => {
                result["status"] = json!("probe_failed");
                result["message"] = json!(error);
            }
            Err(_) => {
                result["status"] = json!("probe_failed");
                result["message"] =
                    json!("computer_use_probe_timeout: native readiness probe did not complete");
            }
        }
    }
    *cache = Some((std::time::Instant::now(), result.clone()));
    result
}

#[cfg(all(test, unix))]
mod tests;
