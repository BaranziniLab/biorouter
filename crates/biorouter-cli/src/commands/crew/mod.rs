//! `biorouter crew`: the terminal's view of Crew, through the profile's shared daemon.
//!
//! People, teams, channels and saved connections are named, not numbered ("Selectors and the
//! resolver" in `docs/crew/design/naming-design.md`). Every name is resolved by the
//! daemon's `POST /crew/resolve` against the person's own workspace snapshot, so the CLI and the
//! desktop share one resolver and a candidate can never be something the person could not see.
//! UUID-shaped text is always an ID and never goes to the resolver, so scripts that pass IDs work
//! against any daemon, including one that predates names. A resolution is a lookup, not a
//! permission: the broker authorizes every mutation, and a person-targeted one also carries the
//! `@username` the person typed (`expected_username`), which the broker checks.

mod args;
mod files;
mod output;

use crate::commands::needs_terminal::{self, NeedsTerminal};
use crate::daemon_client::{CrewClient, DaemonRefusal, NotSent, Restated};
use anyhow::{anyhow, bail, ensure, Context, Result};
pub use args::CrewOptions;
use args::*;
use biorouter::crew::observation::{Initial, ObserveEvent, ObserveRequest};
use output::{component, emit, emit_with, read_input, safe_text, Directory, HumanOptions};
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

/// What a daemon that predates the resolver answers, in place of its bare 404. IDs never reach
/// the resolver, so they keep working against it.
const RESTART_FOR_NAMES: &str = "Restart the shared Biorouter daemon to use names; IDs still work.";
/// The same, for the invitation and device-code routes.
const RESTART_FOR_JOINING: &str =
    "Restart the shared Biorouter daemon to invite or join with an invitation.";
/// A revoke that stopped the grant on this device but that the workspace has not confirmed. It
/// is not a success, so the command exits non-zero (RV-D1). The daemon asks the workspace
/// again by itself whenever the connection is back (F3), so no second command is needed.
const STOPPED_ON_THIS_DEVICE: &str = "Stopped on this device. The workspace hasn't confirmed the revocation yet; Biorouter confirms it with the workspace by itself when the connection is back. biorouter crew grants list shows when it has.";
/// How often `crew join` asks where joining stands, as the desktop's join screen does.
const JOIN_POLL: Duration = Duration::from_secs(5);
/// How long a listing waits for a workspace to answer before it says what it could not check.
const LISTING_WAIT: Duration = Duration::from_secs(10);

pub async fn handle(mut options: CrewOptions) -> Result<()> {
    let format = options.output_format;
    let request_id = options
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    options.request_id = Some(request_id.clone());
    // A streamed command's failure is its last line, so it is one JSON value like the rest.
    let format = if streams(&options.command) {
        output::stream_format(format)
    } else {
        format
    };
    let sent = Arc::new(AtomicBool::new(false));
    execute(options, Arc::clone(&sent))
        .await
        .map_err(|error| failure(&error, format, &request_id, sent.load(Ordering::SeqCst)))
}

/// The commands that print as they go, one JSON value per line in both JSON formats (the
/// contract `docs/crew/command-line.md` states): their last value and their failure are one
/// line too.
fn streams(command: &CrewCommand) -> bool {
    matches!(
        command,
        CrewCommand::Watch(_)
            | CrewCommand::Join(_)
            | CrewCommand::Tasks(TaskCommand::Watch { .. })
            | CrewCommand::Files(FileCommand::Watch { .. })
    )
}

/// The error a failed command exits with.
///
/// Each line is made terminal-safe on its own, so a list of candidates stays a list. The request
/// ID is a machine ID, so it appears only where it is useful: after a mutation that carried it
/// was sent and its outcome is unknown, as the one retry that is safe. JSON output always
/// carries it, and the daemon's refusal code when there is one.
///
/// A command that needed a person at a terminal and had none keeps that type
/// ([`NeedsTerminal`]), so it exits with the usage status 2 like every other such refusal.
fn failure(
    error: &anyhow::Error,
    format: OutputFormat,
    request_id: &str,
    sent: bool,
) -> anyhow::Error {
    let message = safe_lines(&error_text(error));
    if let Some(line) = failure_json(error, &message, request_id, format) {
        // A closed standard output (`| head`) must not turn a failure into a panic, so the
        // write's own error is ignored, as it always was here.
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
    if error
        .chain()
        .any(|cause| cause.is::<NeedsTerminal>() || cause.is::<Usage>())
    {
        needs_a_terminal(message)
    } else if sent && outcome_uncertain(error) {
        anyhow!("{message}\n{}", output::retry_hint(request_id))
    } else {
        anyhow!("{message}")
    }
}

/// What a failure prints to standard output in a JSON format: [`failure_body`], indented for
/// `json` and on one line for `stream-json` (the format a streamed command's failure uses).
fn failure_json(
    error: &anyhow::Error,
    message: &str,
    request_id: &str,
    format: OutputFormat,
) -> Option<String> {
    if matches!(format, OutputFormat::Text) {
        return None;
    }
    output::formatted(
        &failure_body(error, message, request_id),
        format,
        &HumanOptions::default(),
    )
    .ok()
}

/// A failure in JSON: the message, the request ID, and the codes a script can match on. A
/// watch the daemon ended is also the observer's error frame (`type`, its `code` and `clear`),
/// so a script reading lines gets one value for it, not a frame and then a second error.
fn failure_body(error: &anyhow::Error, message: &str, request_id: &str) -> Value {
    let mut body = json!({"error": message, "request_id": request_id});
    if let Some(code) = error_code(error) {
        body["code"] = json!(code);
    } else if error.chain().any(|cause| cause.is::<NeedsTerminal>()) {
        body["code"] = json!(NEEDS_TERMINAL_CODE);
    }
    if let Some((broker_code, _)) = error.chain().find_map(broker_refusal) {
        body["broker_code"] = json!(broker_code);
    }
    // OpenSSH's words beside a refused connect, or any other refusal's diagnostic `detail`
    // (the unreadable-registry refusal keeps its parse error there, AGT2-N6): for a script or
    // support, never in text.
    let detail = connect_detail(error).or_else(|| {
        error
            .chain()
            .find_map(|cause| refusal_code_and_detail(cause).and_then(|(_, detail)| detail))
            .map(str::to_owned)
    });
    if let Some(detail) = detail {
        body["detail"] = json!(detail);
    }
    // AGT2-N5: the daemon's other machine-readable fields (`actual_mode`, `expected_mode`,
    // `institution_refusal`, `reason`, `connection_id`, `host`, `candidates`, …), so a script
    // reads which mode or institution failed without parsing the sentence. What the CLI wrote
    // above is never replaced.
    if let Some(fields) = error.chain().find_map(refusal_fields) {
        for (key, value) in fields {
            if body.get(key).is_none() {
                body[key.as_str()] = value.clone();
            }
        }
    }
    if let Some(stopped) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WatchStopped>())
    {
        body["type"] = json!("error");
        body["code"] = json!(stopped.code);
        body["clear"] = json!(stopped.clear);
    }
    body
}

/// A failure already said in words for a person, naming what the command knew about it
/// ([`Api::worded`]). The failure it words is its source, so JSON output keeps the daemon's and
/// the broker's codes; text output prints only the words.
#[derive(Debug)]
struct Worded {
    sentence: String,
    source: anyhow::Error,
}

impl std::fmt::Display for Worded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.sentence)
    }
}

impl std::error::Error for Worded {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// A watch the daemon's observer ended: the sentence for a person, and the observer's code
/// and `clear` for JSON output.
#[derive(Debug)]
struct WatchStopped {
    message: String,
    code: String,
    clear: bool,
}

impl std::fmt::Display for WatchStopped {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WatchStopped {}

/// The JSON code of a command that needed a person at a terminal and had none.
const NEEDS_TERMINAL_CODE: &str = "crew_needs_terminal";

/// A command line refused before anything was sent, with its own code for JSON: several saved
/// connections and no `--connection`, or a message too long to post. It exits with the usage
/// status 2, as the manual promises for a wrong command line (SF2-N6, MSG2-N7).
#[derive(Debug)]
struct Usage {
    sentence: String,
    code: &'static str,
}

impl std::fmt::Display for Usage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.sentence)
    }
}

impl std::error::Error for Usage {}

fn usage(sentence: impl Into<String>, code: &'static str) -> anyhow::Error {
    Usage {
        sentence: sentence.into(),
        code,
    }
    .into()
}

/// The daemon's code for a request that needs a saved connection chosen.
const CONNECTION_REQUIRED: &str = "crew_connection_required";

/// The refusal of a command that needs a person at a terminal and was run without one. It
/// exits with the usage status 2: nothing was attempted.
fn needs_a_terminal(sentence: impl Into<String>) -> anyhow::Error {
    let sentence = sentence.into();
    match needs_terminal::require(false, &sentence) {
        Err(refusal) => refusal.into(),
        Ok(()) => anyhow!(sentence),
    }
}

/// `{error:#}`, except that a broker refusal the daemon forwarded is said in words for a person
/// ([`output::broker_refusal_text`]) instead of as `Daemon returned 400: code: …`.
///
/// A failed connect with one of the daemon's typed codes (`crew_ssh_auth_required`, …) is said
/// the same way, with what to run, then the code the manual's table is keyed on and OpenSSH's
/// own words (CLI-7), instead of the transport's internal text. Any other daemon refusal with a
/// code is the daemon's own sentence ([`daemon_sentence`]); only one without a code keeps the
/// `Daemon returned NNN:` it is displayed with.
fn error_text(error: &anyhow::Error) -> String {
    let mut parts = Vec::new();
    for cause in error.chain() {
        // Already worded with everything the command knew: what it words is not said again.
        if let Some(worded) = cause.downcast_ref::<Worded>() {
            parts.push(worded.sentence.clone());
            break;
        }
        parts.push(if let Some((code, message)) = broker_refusal(cause) {
            output::broker_refusal_text(code, message)
        } else if let Some((code, sentence, detail)) = connect_failure(cause) {
            connect_failure_lines(code, sentence, detail)
        } else if let Some(sentence) = daemon_sentence(cause) {
            sentence
        } else {
            cause.to_string()
        });
    }
    parts.join(": ")
}

/// A refused connect the daemon typed: its code, the sentence for it, and OpenSSH's words.
fn connect_failure<'a>(
    cause: &'a (dyn std::error::Error + 'static),
) -> Option<(&'a str, &'static str, Option<&'a str>)> {
    let (code, detail) = refusal_code_and_detail(cause)?;
    Some((code, output::connect_failure_text(code)?, detail))
}

/// A daemon refusal's code and `detail`.
fn refusal_code_and_detail<'a>(
    cause: &'a (dyn std::error::Error + 'static),
) -> Option<(&'a str, Option<&'a str>)> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return Some((refused.kind.as_deref()?, refused.detail()));
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return Some((refused.code.as_deref()?, refused.detail.as_deref()));
    }
    None
}

fn connect_failure_lines(code: &str, sentence: &str, detail: Option<&str>) -> String {
    let mut lines = vec![sentence.to_owned(), format!("  Code: {code}")];
    if let Some(detail) = detail {
        for (index, line) in detail.lines().enumerate() {
            lines.push(if index == 0 {
                format!("  Details: {line}")
            } else {
                format!("    {line}")
            });
        }
    }
    lines.join("\n")
}

/// OpenSSH's own words beside a failed connect, for JSON output.
fn connect_detail(error: &anyhow::Error) -> Option<String> {
    error
        .chain()
        .find_map(|cause| connect_failure(cause).and_then(|(_, _, detail)| detail))
        .map(str::to_owned)
}

/// A broker refusal the daemon forwarded: the broker's code and its own `code: sentence`.
fn broker_refusal<'a>(cause: &'a (dyn std::error::Error + 'static)) -> Option<(&'a str, &'a str)> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return refused
            .broker_code
            .as_deref()
            .map(|code| (code, refused.message()));
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return refused
            .broker_code
            .as_deref()
            .map(|code| (code, refused.message.as_str()));
    }
    None
}

/// Each line made terminal-safe on its own. The CLI's own isolates around a name survive
/// ([`output::safe_text_keeping_isolates`]): the names in these sentences were made safe when
/// they were written, so a second escape would print the isolates as `\u{2068}` text.
fn safe_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| output::safe_text_keeping_isolates(line.strip_suffix('\r').unwrap_or(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn execute(options: CrewOptions, sent: Arc<AtomicBool>) -> Result<()> {
    // Before any prompt: nothing here can work without the shared daemon's Unix socket.
    crate::daemon_client::require_supported_platform(cfg!(unix))?;
    let CrewOptions {
        connection,
        expected_mode,
        expected_policy_epoch,
        expected_workspace_policy_epoch,
        no_start,
        approval_key_stdin,
        passphrase_stdin,
        output_format,
        show_ids,
        request_id,
        command,
    } = options;
    if let CrewCommand::Daemon(command) = command {
        let action = match command {
            DaemonCommand::Start => "start",
            DaemonCommand::Status => "status",
            DaemonCommand::Stop => "stop",
        };
        return emit_with(
            &crate::daemon_client::daemon_control(action, approval_key_stdin).await?,
            output_format,
            &HumanOptions::new(show_ids),
        );
    }
    if let CrewCommand::Credentials(command) = command {
        let action = match command {
            CredentialCommand::Status => "status",
            CredentialCommand::Init => "init",
            CredentialCommand::Unlock => "unlock",
            CredentialCommand::Lock => "lock",
        };
        return emit_with(
            &crate::daemon_client::credentials_control(action, approval_key_stdin, passphrase_stdin).await?,
            output_format,
            &HumanOptions::new(show_ids),
        );
    }
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    component(&request_id).context("Invalid Crew request ID")?;
    let client = Client {
        daemon: Daemon::Shared(CrewClient::connect_with_input(no_start, approval_key_stdin).await?),
        request_id: request_id.clone(),
        sent,
    };
    let selected = match connection {
        Some(text) => Some(select_connection(&client, &text).await?),
        None => None,
    };
    let api = Api {
        client,
        selected,
        expected_mode,
        expected_policy_epoch,
        expected_workspace_policy_epoch,
        request_id,
        format: output_format,
        show_ids,
        interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        poll: JOIN_POLL,
        connection_id: tokio::sync::OnceCell::new(),
        channel_labels: std::sync::Mutex::default(),
    };
    let reply = match run(&api, command).await {
        Ok(reply) => reply,
        Err(error) => return Err(api.worded_failure(error).await),
    };
    reply.print(api.format)
}

/// The shared daemon, or a scripted stand-in in tests. Every request goes through
/// [`Client::request`], which notes whether one carrying this invocation's request ID was sent.
struct Client {
    daemon: Daemon,
    request_id: String,
    sent: Arc<AtomicBool>,
}

enum Daemon {
    Shared(CrewClient),
    #[cfg(test)]
    Fake(Arc<tests::FakeDaemon>),
}

impl Client {
    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        if carries_request_id(body.as_ref(), &self.request_id) {
            self.sent.store(true, Ordering::SeqCst);
        }
        match &self.daemon {
            Daemon::Shared(client) => client.request(method, path, body).await,
            #[cfg(test)]
            Daemon::Fake(fake) => fake.answer(method, path, body),
        }
    }

    /// The real daemon, for the streaming and terminal routes a test stand-in cannot serve.
    fn shared(&self) -> Result<&CrewClient> {
        match &self.daemon {
            Daemon::Shared(client) => Ok(client),
            #[cfg(test)]
            Daemon::Fake(_) => bail!("The test daemon serves no streams"),
        }
    }
}

/// Whether `body` carries the request ID a retry reuses: the run, transfer and revoke routes'
/// `request_id`, or a broker mutation's `idempotency_key`.
fn carries_request_id(body: Option<&Value>, request_id: &str) -> bool {
    body.is_some_and(|body| {
        body.get("request_id").and_then(Value::as_str) == Some(request_id)
            || body
                .pointer("/params/idempotency_key")
                .and_then(Value::as_str)
                == Some(request_id)
    })
}

/// A refusal the daemon answered with: its HTTP status and code.
struct Refusal {
    status: u16,
    code: Option<String>,
}

fn refusal(error: &anyhow::Error) -> Option<Refusal> {
    error.chain().find_map(|cause| {
        if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
            return Some(Refusal {
                status: refused.status,
                code: refused.kind.clone(),
            });
        }
        #[cfg(test)]
        if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
            return Some(Refusal {
                status: refused.status,
                code: refused.code.clone(),
            });
        }
        None
    })
}

/// The code a script can match on: the daemon's, the one a restated refusal kept, or
/// [`NotSent::CODE`] for a request the connection never took.
fn error_code(error: &anyhow::Error) -> Option<String> {
    error
        .chain()
        .find_map(|cause| {
            cause
                .downcast_ref::<Restated>()
                .and_then(|restated| restated.code.map(str::to_owned))
        })
        .or_else(|| {
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<Usage>())
                .map(|usage| usage.code.to_owned())
        })
        .or_else(|| {
            error
                .chain()
                .any(|cause| cause.is::<NotSent>())
                .then(|| NotSent::CODE.to_owned())
        })
        .or_else(|| refusal(error).and_then(|refused| refused.code))
}

/// The daemon's code for a request whose outcome it could not confirm (W2-DMN-7): the bridge
/// was lost after the request was written, so it may have landed.
const OUTCOME_UNKNOWN: &str = "crew_outcome_unknown";
/// The daemon's code for a request it never wrote to the workspace.
const NOT_SENT: &str = "crew_not_sent";
/// The daemon's code for a request it did not send because it is dialling a broken bridge
/// again (W2-DMN-6): nothing was sent, and the same command works in a moment.
const RECONNECTING: &str = "crew_reconnecting";

/// Whether the change may have landed although the command failed (R-3), so the one safe retry,
/// the same request ID, is offered. A refusal is a definite answer: nothing changed. So is a
/// request that was never sent, by the daemon ([`NOT_SENT`]) or by the connection here
/// ([`NotSent`]). An outcome the daemon could not confirm ([`OUTCOME_UNKNOWN`]), a daemon
/// failure (5xx) and a lost answer are not.
fn outcome_uncertain(error: &anyhow::Error) -> bool {
    if error
        .chain()
        .any(|cause| cause.is::<Restated>() || cause.is::<NotSent>() || cause.is::<Usage>())
    {
        return false;
    }
    // The broker says it could not tell whether a change was saved (W2-BRK-3).
    if error
        .chain()
        .find_map(broker_refusal)
        .is_some_and(|(code, message)| output::storage_outcome_unknown(code, message))
    {
        return true;
    }
    match refusal(error) {
        None => true,
        Some(refused) => match refused.code.as_deref() {
            Some(OUTCOME_UNKNOWN) => true,
            Some(NOT_SENT | RECONNECTING) => false,
            _ => refused.status >= 500,
        },
    }
}

/// A daemon refusal said for a person without the `Daemon returned N:` prefix: an outcome the
/// daemon could not confirm (the retry line follows it), a request it never sent, and the
/// model's institution and privacy-mode refusals (W2-DMN-9), said from their details, naming
/// both sides. Any other institution refusal is the daemon's own sentence ([`is_model_refusal`]).
///
/// A request the daemon never sent ([`NOT_SENT`]) is its own sentence too (W2-DMN-7). Each one
/// says why nothing was sent, and whether trying again can help: a server that could not be
/// reached may answer later, but an SSH key the server refused is refused every time, so a
/// fixed "run it again" would give advice that cannot work and drop the cause.
///
/// Every other refusal with a code is the daemon's own sentence too (FILES2-N4, CLIDOCS-F6):
/// `CrewError.error` is written for a person, and a code says the daemon typed it. Where that
/// sentence names something only the desktop has, the terminal's words stand in: the
/// `--overwrite` flag for an existing file, `connect` for a dropped connection, `grants grant`
/// for a chat's ended access. A refusal with no code, or no sentence, is left to its display,
/// `Daemon returned NNN: …`.
fn daemon_sentence(cause: &(dyn std::error::Error + 'static)) -> Option<String> {
    if is_disconnected_refusal(cause) {
        return Some(NOT_CONNECTED_SENTENCE.to_owned());
    }
    let (code, message) = refusal_code_and_message(cause)?;
    let own = || {
        Some(message.trim())
            .filter(|message| !message.is_empty())
            .map(str::to_owned)
    };
    match code {
        NOT_SENT => Some(own().unwrap_or_else(|| "Nothing was sent; run it again.".to_owned())),
        OUTCOME_UNKNOWN => Some(own().unwrap_or_else(|| {
            "Crew couldn't confirm whether this reached the workspace. Check the channel before you retry."
                .to_owned()
        })),
        INSTITUTION_MISMATCH => {
            let details = refusal_institution_details(cause);
            match details.and_then(|details| details["model"].as_str()) {
                Some(model) if is_model_refusal(message, refusal_connection_institution(cause)) => {
                    Some(output::institution_refusal_text(model, details))
                }
                _ => own(),
            }
        }
        MODE_MISMATCH => match refusal_modes(cause) {
            Some((actual, expected)) => Some(output::mode_mismatch_text(actual, expected)),
            None => own(),
        },
        DESTINATION_EXISTS => own().map(|sentence| overwrite_sentence(&sentence)),
        GRANT_ENDED => Some(grant_ended_sentence(
            refusal_field(cause, "reason") == Some("settings_changed"),
            None,
        )),
        _ => own(),
    }
}

/// The daemon's code for a destination file that exists and was not approved for replacing.
const DESTINATION_EXISTS: &str = "crew_destination_exists";
/// The daemon's code for a chat whose Crew access ended (T3-BE-7), with `reason`
/// `settings_changed` or `ended`.
const GRANT_ENDED: &str = "crew_grant_ended";
/// The daemon's code for a connection that is not connected.
const NOT_CONNECTED: &str = "crew_not_connected";
/// A disconnected connection, when the command could not name it ([`Api::worded_failure`]
/// names it when it can).
const NOT_CONNECTED_SENTENCE: &str =
    "This connection is disconnected. Run biorouter crew connect, then try again.";
/// The daemon's sentence for a disconnected connection, which a daemon before T3-BE-3 sent
/// untyped (`crew_request_refused`, 400) and a current one sends as [`NOT_CONNECTED`]. It is
/// written for the desktop ("connect in Crew"), so the terminal says its own.
const LEGACY_DISCONNECTED: &str =
    "Crew connection is disconnected; authenticate and connect in Crew";

/// Whether `cause` is the daemon's refusal of a request on a disconnected connection: its
/// code, or an older daemon's untyped sentence for it.
fn is_disconnected_refusal(cause: &(dyn std::error::Error + 'static)) -> bool {
    match refusal_code_and_message(cause) {
        Some((NOT_CONNECTED, _)) => true,
        Some((_, message)) => message.trim().trim_end_matches('.') == LEGACY_DISCONNECTED,
        None => refusal_message(cause)
            .is_some_and(|message| message.trim().trim_end_matches('.') == LEGACY_DISCONNECTED),
    }
}

/// Whether `error` is a refusal because its connection is disconnected.
fn disconnected(error: &anyhow::Error) -> bool {
    error.chain().any(is_disconnected_refusal)
}

/// An existing file's refusal with the flag that replaces it (FILES2-N4): the desktop's
/// "Replace it" is a button this terminal does not have.
fn overwrite_sentence(sentence: &str) -> String {
    const DESKTOP: &str = "Replace it, or choose another name.";
    const TERMINAL: &str = "Add --overwrite to replace it, or save it under another name.";
    match sentence.strip_suffix(DESKTOP) {
        Some(lead) => format!("{lead}{TERMINAL}"),
        None => format!("{sentence} Add --overwrite to replace it."),
    }
}

/// A chat whose Crew access ended (SF2-N4): why, when the daemon said, and the command that
/// grants it again, naming the chat and channel when the command knows them.
fn grant_ended_sentence(settings_changed: bool, grant: Option<(&str, &str)>) -> String {
    let why = if settings_changed {
        "This chat's Crew access ended because Crew settings changed."
    } else {
        "This chat's Crew access ended."
    };
    let (session, channel) = grant.unwrap_or(("SESSION", "CHANNEL"));
    format!("{why} Run biorouter crew grants grant {session} {channel} to grant it again.")
}

/// The daemon's code for an institution refusal on any route (W2-DMN-9 a).
const INSTITUTION_MISMATCH: &str = "crew_institution_mismatch";
/// The daemon's code for a request whose `--expected-mode` is not the connection's (W2-DMN-9 e).
const MODE_MISMATCH: &str = "crew_mode_mismatch";

/// An institution refusal's details: the model, who approved it, the workspace and its
/// institution.
fn refusal_institution_details<'a>(
    cause: &'a (dyn std::error::Error + 'static),
) -> Option<&'a Value> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return refused.institution_refusal.as_ref();
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return refused.institution_refusal.as_ref();
    }
    None
}

/// The connection's institution beside an institution refusal about the connection, not the
/// model.
fn refusal_connection_institution<'a>(
    cause: &'a (dyn std::error::Error + 'static),
) -> Option<&'a str> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return refused.connection_institution.as_deref();
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return refused.connection_institution.as_deref();
    }
    None
}

/// Whether an institution refusal is the model refusal (`check_provider`'s "…the model's
/// resolved affiliation…"), which is said from its details, naming the model and whom it is
/// approved for. Admission's refusal of a connection set to another institution ("This
/// connection is for stanford, but lab belongs to ucsf.") carries the same details, but
/// `connection_institution` beside them marks it: it refuses before any model is judged, so a
/// sentence about the model would contradict itself and advise a local model that is refused
/// too. That one is the daemon's own sentence, as the desktop shows it.
fn is_model_refusal(message: &str, connection_institution: Option<&str>) -> bool {
    message.contains(AFFILIATION_REFUSAL) || connection_institution.is_none()
}

/// A privacy-mode refusal's connection mode and the mode the request required.
fn refusal_modes<'a>(cause: &'a (dyn std::error::Error + 'static)) -> Option<(&'a str, &'a str)> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return refused.modes();
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return refused
            .modes
            .as_ref()
            .map(|(actual, expected)| (actual.as_str(), expected.as_str()));
    }
    None
}

/// A daemon refusal's code and its own text.
fn refusal_code_and_message<'a>(
    cause: &'a (dyn std::error::Error + 'static),
) -> Option<(&'a str, &'a str)> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return Some((refused.kind.as_deref()?, refused.message()));
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return Some((refused.code.as_deref()?, refused.message.as_str()));
    }
    None
}

/// A daemon refusal's own text, with a code or without one.
fn refusal_message<'a>(cause: &'a (dyn std::error::Error + 'static)) -> Option<&'a str> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return Some(refused.message());
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return Some(refused.message.as_str());
    }
    None
}

/// The daemon refusal's other fields ([`crate::daemon_client::REFUSAL_FIELDS`]).
fn refusal_fields<'a>(
    cause: &'a (dyn std::error::Error + 'static),
) -> Option<&'a serde_json::Map<String, Value>> {
    if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
        return Some(refused.fields());
    }
    #[cfg(test)]
    if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
        return Some(&refused.fields);
    }
    None
}

/// One of the daemon refusal's other fields as text, such as `reason`.
fn refusal_field<'a>(cause: &'a (dyn std::error::Error + 'static), key: &str) -> Option<&'a str> {
    refusal_fields(cause)?.get(key)?.as_str()
}

/// A refusal said again for a person, keeping the daemon's code for JSON output.
fn restated(message: impl Into<String>, code: Option<&'static str>) -> anyhow::Error {
    Restated::new(message, code).into()
}

/// A route this CLI needs that the running daemon does not have: a bare 404 (or a 405 where the
/// path matched an older route) with no Crew code. A Crew refusal always carries a code, so a
/// missing connection or grant is never mistaken for an old daemon.
fn older_daemon(error: anyhow::Error, message: &'static str) -> anyhow::Error {
    match refusal(&error) {
        Some(Refusal {
            status: 404 | 405,
            code: None,
        }) => anyhow!(message),
        _ => error,
    }
}

/// UUID-shaped text, the same test the daemon's resolver applies.
fn uuid_shaped(text: &str) -> bool {
    uuid::Uuid::try_parse(text).is_ok()
        || (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `--connection`: an ID as it is, anything else through the daemon's resolver (a saved
/// connection's name, compared without letter case, or its SSH target).
async fn select_connection(client: &Client, text: &str) -> Result<String> {
    let text = text.trim();
    ensure!(
        !text.is_empty(),
        "--connection needs a saved connection's name, SSH target or ID"
    );
    if uuid_shaped(text) {
        return Ok(component(text)?.to_owned());
    }
    let answer = client
        .request(
            "POST",
            "/crew/resolve",
            Some(json!({"connection": text, "selectors": []})),
        )
        .await
        .map_err(|error| older_daemon(error, RESTART_FOR_NAMES))?;
    let connection = &answer["connection"];
    match (connection["status"].as_str(), connection["id"].as_str()) {
        (Some("resolved"), Some(id)) => Ok(component(id)?.to_owned()),
        _ => bail!(
            "No saved Crew connection is named “{text}”. Run biorouter crew connections list to see them."
        ),
    }
}

/// What a selector argument names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Person,
    FormerPerson,
    Team,
    Channel,
}

impl Kind {
    fn wire(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::FormerPerson => "former_person",
            Self::Team => "team",
            Self::Channel => "channel",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Person => "member",
            Self::FormerPerson => "former member",
            Self::Team => "team",
            Self::Channel => "channel",
        }
    }

    /// The ID a selector gives as it is: UUID-shaped text after the kind's optional `@` or `#`.
    fn literal_id(self, text: &str) -> Option<&str> {
        let text = text.trim();
        let bare = match self {
            Self::Person | Self::FormerPerson => text.strip_prefix('@').unwrap_or(text),
            Self::Channel => text.strip_prefix('#').unwrap_or(text),
            Self::Team => text,
        };
        uuid_shaped(bare).then_some(bare)
    }

    /// The selector as a person reads it back: `@bob`, `#methods`, “Analysis Lab”.
    fn shown(self, text: &str) -> String {
        let text = text.trim();
        match self {
            Self::Person | Self::FormerPerson => {
                format!("@{}", text.strip_prefix('@').unwrap_or(text))
            }
            Self::Channel if !text.contains('/') => {
                format!("#{}", text.strip_prefix('#').unwrap_or(text))
            }
            Self::Team | Self::Channel => format!("“{text}”"),
        }
    }
}

/// What a selector named.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    id: String,
    /// The daemon's label: `Bob Lee (@bob)`, `Analysis Lab`, `#methods`. None for an ID given as
    /// it is, which was never looked up.
    label: Option<String>,
    /// A person's username, sent as `expected_username` so the broker refuses a mutation whose
    /// target is not the person who was confirmed.
    username: Option<String>,
}

/// Why a selector did not resolve: the code a script matches on (the resolver's own), and the
/// sentence for a person.
type Unresolved = (&'static str, String);

/// The code of a selector the resolver answered with something this command cannot use.
const LOOKUP_FAILED: &str = "crew_lookup_failed";

/// The status the resolver appends to an archived channel's label (`with_archived` in
/// `routes/crew/names.rs`), which a list shows beside the name.
const ARCHIVED_LABEL_SUFFIX: &str = " · archived";

/// A team's or channel's label as a sentence names it (MSG2-N5): `#archive-me`, not
/// `#archive-me · archived`. A sentence that needs the status says it in words ("… is
/// archived"), and the separator is not ASCII, so kept it would also isolate the whole label.
fn bare_label(label: &str) -> &str {
    let label = label.trim();
    label.strip_suffix(ARCHIVED_LABEL_SUFFIX).unwrap_or(label)
}

/// One resolution from `POST /crew/resolve`, or why it did not resolve.
///
/// A name no channel or team of yours has is said neutrally (M12): "No channel you're in is
/// called #x." is as true of a typo as of a channel renamed or left, where "You're not in a
/// channel called #x" read as a membership problem.
fn target_from(kind: Kind, text: &str, result: &Value) -> std::result::Result<Target, Unresolved> {
    let shown = kind.shown(text);
    match result["status"].as_str() {
        Some("resolved") if result["kind"].as_str() == Some(kind.wire()) => {
            let id = result["id"]
                .as_str()
                .and_then(|id| component(id).ok())
                .ok_or_else(|| {
                    (
                        LOOKUP_FAILED,
                        format!("Biorouter couldn't look up {shown}."),
                    )
                })?;
            let is_person = matches!(kind, Kind::Person | Kind::FormerPerson);
            Ok(Target {
                id: id.to_owned(),
                label: result["label"].as_str().map(|label| match kind {
                    Kind::Channel | Kind::Team => bare_label(label).to_owned(),
                    Kind::Person | Kind::FormerPerson => label.to_owned(),
                }),
                username: result["username"]
                    .as_str()
                    .filter(|_| is_person)
                    .map(str::to_owned),
            })
        }
        Some("unknown_name") => {
            let mut message = match kind {
                Kind::Person => format!("There's no member {shown} in this workspace."),
                Kind::FormerPerson => format!("There's no former member {shown} here."),
                Kind::Team => format!("No team you're in is called {shown}."),
                Kind::Channel => format!("No channel you're in is called {shown}."),
            };
            if let Some(suggestion) = result["did_you_mean"].as_str() {
                message.push_str(&format!(
                    " Did you mean {suggestion}? Usernames must match exactly."
                ));
            }
            Err(("unknown_name", message))
        }
        Some("ambiguous_name") => {
            let mut lines = vec![format!("{shown} matches more than one {}:", kind.noun())];
            lines.extend(
                result["candidates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|candidate| format!("  {candidate}")),
            );
            lines.push(match kind {
                Kind::Channel if !text.contains('/') => {
                    "Name its team too, like analysis-lab/methods.".to_owned()
                }
                Kind::Channel => {
                    "Use the channel's ID: biorouter crew channels list --show-ids.".to_owned()
                }
                Kind::Team => "Use the team's ID: biorouter crew teams list --show-ids.".to_owned(),
                Kind::Person => "Use their ID: biorouter crew members --show-ids.".to_owned(),
                Kind::FormerPerson => {
                    "Use their ID: biorouter crew workspace show --show-ids.".to_owned()
                }
            });
            Err(("ambiguous_name", lines.join("\n")))
        }
        _ => Err((
            LOOKUP_FAILED,
            format!("Biorouter couldn't look up {shown}."),
        )),
    }
}

/// The output of one command.
#[derive(Debug)]
enum Reply {
    /// A daemon value, rendered for a person by `output` or printed as JSON.
    Show(Value, Box<HumanOptions>),
    /// Sentences for a person; `Value` for JSON.
    Say(Value, Vec<String>),
    /// Printed as it happened (watchers, joining).
    Streamed,
    /// The last value of a streamed command, printed like the lines before it: one JSON value
    /// per line in both JSON formats.
    Last(Value, Box<HumanOptions>),
}

impl Reply {
    fn print(self, format: OutputFormat) -> Result<()> {
        match self {
            Self::Show(value, options) => emit_with(&value, format, &options),
            Self::Last(value, options) => {
                emit_with(&value, output::stream_format(format), &options)
            }
            Self::Say(value, lines) => match format {
                OutputFormat::Text => print_lines(&lines),
                OutputFormat::Json | OutputFormat::StreamJson => emit(&value, format),
            },
            Self::Streamed => Ok(()),
        }
    }
}

fn print_lines(lines: &[String]) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    for line in lines {
        writeln!(stdout, "{line}")?;
    }
    stdout.flush()?;
    Ok(())
}

async fn run(api: &Api, command: CrewCommand) -> Result<Reply> {
    Ok(match command {
        CrewCommand::Daemon(_) | CrewCommand::Credentials(_) => {
            bail!("Daemon and credential commands run before connecting")
        }
        CrewCommand::Status => api.show(api.connections_with_privacy().await?),
        CrewCommand::Connections(command) => connections(api, command).await?,
        CrewCommand::Auth => {
            let id = api.connection_id().await?;
            api.show(api.client.shared()?.authenticate_ssh(&id).await?)
        }
        CrewCommand::Connect => api.show(api.connect().await?),
        CrewCommand::Disconnect => api.show(api.connection_action("disconnect", json!({})).await?),
        CrewCommand::Join(args) => join(api, args).await?,
        CrewCommand::Workspace(command) => workspace(api, command).await?,
        CrewCommand::Enroll(command) => enrollment(api, command).await?,
        CrewCommand::Members(MembersArgs { command: None }) => {
            let snapshot = api.snapshot().await?;
            let mut people = with_roles(snapshot_field(&snapshot, "principals")?, &snapshot);
            if let Some(list) = people.as_array_mut() {
                let host = snapshot_host_id(&snapshot);
                output::people_in_order(list, host.as_deref(), snapshot["actor"]["id"].as_str());
            }
            api.show_with(people, Directory::from_snapshot(&snapshot))
        }
        CrewCommand::Members(MembersArgs {
            command:
                Some(MembersCommand::Add {
                    person,
                    team,
                    channels,
                }),
        }) => add_member(api, &person, team.as_deref(), &channels).await?,
        CrewCommand::Teams(command) => teams(api, command).await?,
        CrewCommand::Channels(command) => channels(api, command).await?,
        CrewCommand::Invites(command) => invitations(api, command).await?,
        CrewCommand::Profile(ProfileCommand::Show) => {
            let snapshot = api.snapshot().await?;
            let actor = snapshot_field(&snapshot, "actor")?;
            api.show_with(actor, Directory::from_snapshot(&snapshot))
        }
        CrewCommand::Profile(ProfileCommand::Set { nickname, avatar }) => api.show(
            api.broker(
                "profile.update",
                json!({"nickname":nickname,"avatar":avatar}),
                true,
            )
            .await?,
        ),
        CrewCommand::Ownership(command) => ownership(api, command).await?,
        CrewCommand::RemoveMember {
            channel,
            member,
            former,
            yes,
        } => remove_member(api, &channel, &member, former, yes).await?,
        CrewCommand::History(args) => {
            let channel = api.your_channel(&args.channel).await?;
            let mut page = api
                .broker(
                    "messages.history",
                    history_params(&channel.id, &args),
                    false,
                )
                .await?;
            AttachmentNames::default().name_page(api, &mut page).await;
            api.show_with(page, api.names().await)
        }
        CrewCommand::Search {
            channel,
            query,
            limit,
            after,
        } => {
            let channel = api.your_channel(&channel).await?;
            let mut params = json!({"channel_id":channel.id,"query":query,"limit":limit});
            if let Some(after) = after {
                params["after"] = json!(after);
            }
            let mut page = api.broker("messages.search", params, false).await?;
            AttachmentNames::default().name_page(api, &mut page).await;
            api.show_with(page, api.names().await)
        }
        CrewCommand::Watch(args) => watch(api, args).await?,
        CrewCommand::Send(args) => send_message(api, args).await?,
        CrewCommand::Context { session } => context(api, &session).await?,
        CrewCommand::Files(command) => file_command(api, command).await?,
        CrewCommand::Tasks(command) => tasks(api, command).await?,
        CrewCommand::Grants(command) => grants(api, command).await?,
        CrewCommand::Privacy(command) => privacy(api, command).await?,
    })
}

struct Api {
    client: Client,
    /// The selected connection's ID, resolved from `--connection` before any command runs.
    selected: Option<String>,
    expected_mode: Option<PrivacyMode>,
    expected_policy_epoch: Option<u64>,
    expected_workspace_policy_epoch: Option<u64>,
    request_id: String,
    format: OutputFormat,
    show_ids: bool,
    /// Whether a terminal can answer a question: stdin and stderr are both terminals.
    interactive: bool,
    poll: Duration,
    connection_id: tokio::sync::OnceCell<String>,
    /// Each channel a selector named, by ID, as the resolver labelled it (`#methods`): what a
    /// refusal about that channel names.
    channel_labels: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl Api {
    fn with_expected_mode(&self, body: Value) -> Value {
        add_expected_mode(body, self.expected_mode)
    }

    fn with_run_policy(&self, body: Value) -> Value {
        let mut body = self.with_expected_mode(body);
        if let Some(epoch) = self.expected_policy_epoch {
            body["expected_policy_epoch"] = json!(epoch);
        }
        if let Some(epoch) = self.expected_workspace_policy_epoch {
            body["expected_workspace_policy_epoch"] = json!(epoch);
        }
        body
    }

    fn text(&self) -> bool {
        matches!(self.format, OutputFormat::Text)
    }

    fn human(&self, directory: Directory) -> HumanOptions {
        HumanOptions::new(self.show_ids).with_directory(directory)
    }

    fn show(&self, value: Value) -> Reply {
        self.show_with(value, Directory::default())
    }

    fn show_with(&self, value: Value, directory: Directory) -> Reply {
        Reply::Show(value, Box::new(self.human(directory)))
    }

    fn say(&self, value: Value, lines: Vec<String>) -> Reply {
        Reply::Say(value, lines)
    }

    /// `text`, then the ID when `--show-ids` asked for it.
    fn with_id(&self, text: String, what: &str, id: &str) -> String {
        if self.show_ids {
            format!("{text} [{what} {}]", safe_text(id))
        } else {
            text
        }
    }

    /// A team's or channel's name for a sentence: the daemon's label, else `fallback` for an ID
    /// that was given as it is.
    fn label(&self, target: &Target, fallback: &str, what: &str) -> String {
        let text = target
            .label
            .as_deref()
            .map_or_else(|| fallback.to_owned(), name_text);
        self.with_id(text, what, &target.id)
    }

    /// Print one step of a streamed command: sentences in text, the value as a JSON line.
    fn stream(&self, value: &Value, lines: &[String]) -> Result<()> {
        match self.format {
            OutputFormat::Text => print_lines(lines),
            OutputFormat::Json | OutputFormat::StreamJson => {
                emit(value, output::stream_format(self.format))
            }
        }
    }

    async fn connections(&self) -> Result<Value> {
        self.client.request("GET", "/crew/connections", None).await
    }

    /// The saved connections, each with its effective privacy ([`Self::with_effective_privacy`]).
    async fn connections_with_privacy(&self) -> Result<Value> {
        let mut listed = self.connections().await?;
        if let Some(connections) = listed["connections"].as_array_mut() {
            for connection in connections.iter_mut() {
                *connection = self.with_effective_privacy(connection.take()).await;
            }
        }
        Ok(listed)
    }

    /// `connection` with its effective privacy (SF-F1): `effective_mode` (null when it cannot
    /// be read), and the workspace's `workspace_mode` and `workspace_name` when read. Privacy is
    /// Public only when the connection is Public and the workspace allows it, so the workspace
    /// is asked only about a Public connection, and only while it is connected.
    ///
    /// Two more facts are read for a connected connection, as the desktop's status row reads
    /// them. Whether this computer has joined (`joined`, SC2-N5): a connection the host has not
    /// let in yet is connected but not a member, and reads "Not joined yet", never "Connected".
    /// And, when the daemon says the workspace server has stopped saving (`server_storage`,
    /// RES2-N2), whether the person hosts it (`you_host`), since only the host can fix it.
    async fn with_effective_privacy(&self, connection: Value) -> Value {
        let connected = connection["status"].as_str() == Some("connected");
        let Some(id) = connection["id"]
            .as_str()
            .and_then(|id| component(id).ok())
            .map(str::to_owned)
            .filter(|_| connected)
        else {
            return with_privacy_of(connection, None);
        };
        let mut connection = connection;
        let joined = self.joined(&id).await;
        if let Some(joined) = joined {
            connection["joined"] = json!(joined);
        }
        let stopped_saving = connection["server_storage"].is_object();
        let public = connection["mode"].as_str() == Some("public");
        // A person who has not joined cannot read the workspace, and nothing else needs it.
        if joined == Some(false) || !(public || stopped_saving) {
            return with_privacy_of(connection, None);
        }
        // A listing must not wait on a workspace that is slow to answer: after a few seconds
        // its privacy reads as not checked, as it does offline.
        let snapshot = tokio::time::timeout(
            LISTING_WAIT,
            self.client.request(
                "POST",
                &format!("/crew/connections/{id}/request"),
                Some(json!({"method": "workspace.snapshot", "params": {}, "request_id": null})),
            ),
        )
        .await
        .ok()
        .and_then(Result::ok);
        if stopped_saving {
            let standing = snapshot.as_ref().map_or(HostStanding::Unknown, |snapshot| {
                host_standing(snapshot, None)
            });
            match standing {
                HostStanding::NotHost => connection["you_host"] = json!(false),
                HostStanding::Unknown => {}
                _ => connection["you_host"] = json!(true),
            }
        }
        if public {
            with_privacy_of(connection, snapshot.as_ref())
        } else {
            with_privacy_of(connection, None)
        }
    }

    /// Whether this computer has joined the connected workspace `id`, from `GET …/join` (the
    /// desktop's "Not joined yet" rule): `false` while the host has not let it in, `true` once
    /// it is a member, and `None` when the daemon could not say in time, or the workspace's
    /// server cannot tell (`unsupported`).
    async fn joined(&self, id: &str) -> Option<bool> {
        let status = tokio::time::timeout(
            LISTING_WAIT,
            self.client
                .request("GET", &format!("/crew/connections/{id}/join"), None),
        )
        .await
        .ok()?
        .ok()?;
        match status["status"].as_str()? {
            "joined" => Some(true),
            "invited" | "approved" | "code_mismatch" | "not_invited" | "expired" => Some(false),
            _ => None,
        }
    }

    async fn connection(&self) -> Result<Value> {
        let response = self.connections().await?;
        let connections = response["connections"]
            .as_array()
            .context("Daemon returned an invalid connection list")?;
        if let Some(id) = &self.selected {
            return connections
                .iter()
                .find(|item| item["id"].as_str() == Some(id.as_str()))
                .cloned()
                .context("The selected Crew connection isn't saved on this computer; run biorouter crew connections list");
        }
        match connections.as_slice() {
            [only] => Ok(only.clone()),
            [] => bail!("No Crew connection is saved on this computer. Save the invitation your host sent with biorouter crew connections join-invitation -"),
            // SF2-N6: a wrong command line, exit 2, with the daemon's own code for the case.
            _ => Err(usage(
                "Several Crew connections are saved; choose one with --connection NAME. Run biorouter crew connections list to see them.",
                CONNECTION_REQUIRED,
            )),
        }
    }

    async fn connection_id(&self) -> Result<String> {
        self.connection_id
            .get_or_try_init(|| async {
                let value = self.connection().await?;
                Ok::<_, anyhow::Error>(
                    component(
                        value["id"]
                            .as_str()
                            .context("Daemon returned a connection without its ID")?,
                    )?
                    .to_owned(),
                )
            })
            .await
            .cloned()
    }

    async fn path(&self, suffix: &str) -> Result<String> {
        Ok(format!(
            "/crew/connections/{}{suffix}",
            self.connection_id().await?
        ))
    }

    async fn connection_action(&self, action: &str, body: Value) -> Result<Value> {
        self.client
            .request("POST", &self.path(&format!("/{action}")).await?, Some(body))
            .await
    }

    /// `POST …/connect`. A refused connect to a login on this machine (`crew_iris@localhost`)
    /// is said in the same-host words ([`output::same_host_connect_failure_text`], SETUPHPC2-F-A),
    /// with the code and OpenSSH's words as for any other connect; JSON keeps both.
    async fn connect(&self) -> Result<Value> {
        let error = match self.connection_action("connect", json!({})).await {
            Ok(connected) => return Ok(connected),
            Err(error) => error,
        };
        let Some((code, detail)) = error.chain().find_map(|cause| {
            connect_failure(cause)
                .map(|(code, _, detail)| (code.to_owned(), detail.map(str::to_owned)))
        }) else {
            return Err(error);
        };
        let Ok(connection) = self.connection().await else {
            return Err(error);
        };
        let Some(sentence) = connection["ssh_target"]
            .as_str()
            .and_then(|target| output::same_host_connect_failure_text(&code, target))
        else {
            return Err(error);
        };
        Err(Worded {
            sentence: connect_failure_lines(&code, &sentence, detail.as_deref()),
            source: error,
        }
        .into())
    }

    async fn broker(&self, method: &str, mut params: Value, mutation: bool) -> Result<Value> {
        add_personal_mode(method, &mut params, self.expected_mode);
        if mutation {
            params["idempotency_key"] = json!(self.request_id);
        }
        let body = json!({"method":method,"params":params,"request_id":if mutation {Some(&self.request_id)} else {None}});
        let answer = self.connection_action("request", body).await;
        self.worded(answer, &params).await
    }

    /// A broker read that is only a lookup for a refusal's words: never worded itself.
    async fn lookup(&self, method: &str, params: Value) -> Option<Value> {
        let body = json!({"method":method,"params":params,"request_id":null});
        self.connection_action("request", body).await.ok()
    }

    /// A refusal that can name more than its own text (DW-11, M20, R-2, FILES-F9), said with
    /// what this command knows: the channel it acted on, where a shared file came from, and for
    /// a server that can no longer save, whether the person hosts it. The refusal stays behind
    /// the words, so JSON output keeps its codes. Any other answer is returned as it is.
    async fn worded(&self, answer: Result<Value>, params: &Value) -> Result<Value> {
        let error = match answer {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some((code, message)) = error
            .chain()
            .find_map(broker_refusal)
            .map(|(code, message)| (code.to_owned(), message.to_owned()))
        else {
            return Err(error);
        };
        let Some(subject) = output::refusal_subject(&code, &message) else {
            return Err(error);
        };
        let place = self.refusal_place(subject, params).await;
        if place == output::RefusalPlace::default() {
            return Err(error);
        }
        Err(Worded {
            sentence: output::broker_refusal_text_in(&code, &message, &place),
            source: error,
        }
        .into())
    }

    /// What `subject` needs named, as far as this command can read it.
    async fn refusal_place(
        &self,
        subject: output::RefusalSubject,
        params: &Value,
    ) -> output::RefusalPlace {
        let mut place = output::RefusalPlace::default();
        match subject {
            output::RefusalSubject::Channel => {
                if let Some(id) = params["channel_id"].as_str() {
                    place.channel = self.channel_label(id).await;
                }
            }
            output::RefusalSubject::SharedIn(kind) => {
                place.shared_in = self.shared_in(kind, params).await;
            }
            output::RefusalSubject::Storage => {
                // Reading still works on a server that can no longer save.
                if let Some(snapshot) = self.lookup("workspace.snapshot", json!({})).await {
                    let directory = Directory::from_snapshot(&snapshot);
                    place.host = match host_standing(&snapshot, None) {
                        HostStanding::NotHost => Some(false),
                        HostStanding::Unknown => None,
                        _ => Some(true),
                    };
                    place.host_label = output::host_label(&directory);
                }
                if let Ok(connection) = self.connection().await {
                    // The server, not the login: `bob@hpc.ucsf.edu` is on `hpc.ucsf.edu`.
                    place.server = connection["server_label"]
                        .as_str()
                        .or_else(|| {
                            connection["ssh_target"]
                                .as_str()
                                .and_then(|target| target.rsplit('@').next())
                        })
                        .map(str::trim)
                        .filter(|server| !server.is_empty())
                        .map(safe_text);
                }
            }
        }
        place
    }

    /// `#methods` for a channel ID: the resolver's label, else the person's snapshot. `None`
    /// when neither names it.
    async fn channel_label(&self, id: &str) -> Option<String> {
        let known = self
            .channel_labels
            .lock()
            .ok()
            .and_then(|labels| labels.get(id).cloned());
        if let Some(label) = known {
            return Some(name_text(&label));
        }
        let snapshot = self.lookup("workspace.snapshot", json!({})).await?;
        Some(Directory::from_snapshot(&snapshot).channel_label(id))
            .filter(|label| label.starts_with('#'))
    }

    /// The channel the first attachment or reference in `params` from another channel was
    /// shared in (FILES-F9), named from the person's snapshot.
    async fn shared_in(&self, kind: output::SharedKind, params: &Value) -> Option<String> {
        let (key, method, field) = match kind {
            output::SharedKind::Attachment => ("attachments", "blob.status", "blob_id"),
            output::SharedKind::Reference => ("references", "reference.get", "reference_id"),
        };
        let destination = params["channel_id"].as_str();
        for id in params[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let Some(shared) = self.lookup(method, json!({ field: id })).await else {
                continue;
            };
            match shared["channel_id"].as_str() {
                Some(channel) if Some(channel) != destination => {
                    return self.channel_label(channel).await;
                }
                _ => {}
            }
        }
        None
    }

    /// Step `step` of a command that makes several mutations: each gets its own idempotency
    /// key derived from the one request ID, so a retry with `--request-id` replays every step
    /// that already landed instead of colliding with the first. Step 0 is [`Self::broker`].
    async fn broker_step(&self, method: &str, mut params: Value, step: usize) -> Result<Value> {
        if step == 0 {
            return self.broker(method, params, true).await;
        }
        add_personal_mode(method, &mut params, self.expected_mode);
        let key = format!("{}:{step}", self.request_id);
        params["idempotency_key"] = json!(key);
        let body = json!({"method":method,"params":params,"request_id":key});
        let answer = self.connection_action("request", body).await;
        self.worded(answer, &params).await
    }

    async fn snapshot(&self) -> Result<Value> {
        self.broker("workspace.snapshot", json!({}), false).await
    }

    /// Names for text output: the workspace snapshot, when it can be read. JSON output needs
    /// none. When the snapshot cannot be read, a list is still printed, and every ID it cannot
    /// name says so, with the workspace and the ID (R-8, AG-F17).
    async fn names(&self) -> Directory {
        if !self.text() {
            return Directory::default();
        }
        match self.snapshot().await {
            Ok(snapshot) => Directory::from_snapshot(&snapshot),
            Err(error) => {
                let workspace = match self.connection().await {
                    Ok(connection) => connection_name(&connection),
                    Err(_) => "this workspace".to_owned(),
                };
                // W2-CLI-9: an older daemon's untyped refusal is the same fact.
                Directory::names_unavailable(&workspace, disconnected(&error))
            }
        }
    }

    /// A failed command's error, said with what this command knows that the daemon's refusal
    /// does not (CLIDOCS-F6): a disconnected connection is named, with the command that
    /// connects it, in place of the daemon's "authenticate and connect in Crew", which is
    /// written for the desktop. The refusal stays its source, so JSON keeps the daemon's code.
    async fn worded_failure(&self, error: anyhow::Error) -> anyhow::Error {
        if !disconnected(&error) || error.chain().any(|cause| cause.is::<Worded>()) {
            return error;
        }
        let Ok(connection) = self.connection().await else {
            return error;
        };
        let name = connection_name(&connection);
        let connect = if self.selected.is_some() {
            format!(
                "biorouter crew --connection {} connect",
                safe_text(&shell_word(&name))
            )
        } else {
            "biorouter crew connect".to_owned()
        };
        Worded {
            sentence: format!(
                "{} is disconnected. Run {connect}, then try again.",
                name_text(&name)
            ),
            source: error,
        }
        .into()
    }

    /// `"Bob Lee" (@bob)`, always both, for a decision about a person; from the snapshot, else
    /// the resolver's label, else `@username`. Never fails: it only labels.
    async fn authority_label(&self, target: &Target) -> String {
        if let Ok(snapshot) = self.snapshot().await {
            let label = output::authority_label(
                &Directory::from_snapshot(&snapshot),
                &target.id,
                self.show_ids,
            );
            if !label.starts_with("Unknown member") {
                return label;
            }
        }
        let text = match (&target.label, &target.username) {
            (Some(label), _) => safe_text(label),
            (None, Some(username)) => format!("@{}", safe_text(username)),
            (None, None) => "this member".to_owned(),
        };
        self.with_id(text, "ID", &target.id)
    }

    async fn session_get(&self, session: &str, action: &str) -> Result<Value> {
        let path = self
            .path(&format!("/sessions/{}/{action}", component(session)?))
            .await?;
        self.client.request("GET", &path, None).await
    }

    /// Resolve selectors in order, in one request. UUID-shaped text is taken as an ID without
    /// asking; anything else goes to the daemon's resolver. Every selector that does not resolve
    /// is reported, and nothing is guessed.
    async fn resolve(&self, selectors: &[(Kind, &str)]) -> Result<Vec<Target>> {
        let mut targets = Vec::with_capacity(selectors.len());
        let mut lookups = Vec::new();
        for (index, (kind, text)) in selectors.iter().enumerate() {
            match kind.literal_id(text) {
                Some(id) => targets.push(Some(Target {
                    id: component(id)?.to_owned(),
                    label: None,
                    username: None,
                })),
                None => {
                    ensure!(
                        !text.trim().is_empty(),
                        "Name the {} this command is for.",
                        kind.noun()
                    );
                    targets.push(None);
                    lookups.push(index);
                }
            }
        }
        if !lookups.is_empty() {
            let body = json!({
                "connection": self.connection_id().await?,
                "selectors": lookups
                    .iter()
                    .map(|&index| json!({"kind": selectors[index].0.wire(), "text": selectors[index].1}))
                    .collect::<Vec<_>>(),
            });
            let answer = self
                .client
                .request("POST", "/crew/resolve", Some(body))
                .await
                .map_err(|error| older_daemon(error, RESTART_FOR_NAMES))?;
            let results = answer["results"]
                .as_array()
                .filter(|results| results.len() == lookups.len())
                .context("The daemon's name lookup answered with the wrong number of results")?;
            let mut problems = Vec::new();
            for (&index, result) in lookups.iter().zip(results) {
                let (kind, text) = selectors[index];
                match target_from(kind, text, result) {
                    Ok(target) => {
                        if let (Kind::Channel, Some(label), Ok(mut labels)) =
                            (kind, target.label.as_deref(), self.channel_labels.lock())
                        {
                            labels.insert(target.id.clone(), label.to_owned());
                        }
                        targets[index] = Some(target);
                    }
                    Err(problem) => problems.push(problem),
                }
            }
            if let Some(&(code, _)) = problems.first() {
                // DW-10: a code, like every refusal the daemon types, so a script can tell an
                // unknown name from an ambiguous one without matching the words. The first
                // problem's code stands for the list.
                let sentences: Vec<String> = problems.into_iter().map(|(_, text)| text).collect();
                return Err(restated(sentences.join("\n"), Some(code)));
            }
        }
        targets
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .context("A name was left unresolved")
    }

    async fn target(&self, kind: Kind, text: &str) -> Result<Target> {
        let mut targets = self.resolve(&[(kind, text)]).await?;
        targets.pop().context("A name was left unresolved")
    }

    /// [`Self::resolve`] for a command only a member of each channel can run (history, search,
    /// watch, send, mark-read, files, tasks, grants). A channel given by ID is checked against
    /// the person's own channels too (AG-F13), so an ID from another workspace, or of a channel
    /// they left, is refused here, as a name would be, rather than reaching the daemon's
    /// "refresh" refusal, which no refresh can help.
    ///
    /// Only a snapshot that lists every channel of theirs can prove an ID absent: one that
    /// leaves some out (`totals.channels`) or cannot be read leaves the ID to the broker, which
    /// authorizes every request either way. Commands a host may run on a channel they are not
    /// in (archive, rename, remove-member, members add) never come here.
    async fn resolve_yours(&self, selectors: &[(Kind, &str)]) -> Result<Vec<Target>> {
        let targets = self.resolve(selectors).await?;
        let by_id: Vec<usize> = selectors
            .iter()
            .enumerate()
            .filter(|(_, (kind, text))| *kind == Kind::Channel && kind.literal_id(text).is_some())
            .map(|(index, _)| index)
            .collect();
        if by_id.is_empty() {
            return Ok(targets);
        }
        let Ok(snapshot) = self.snapshot().await else {
            return Ok(targets);
        };
        let Some(channels) = snapshot["channels"].as_array() else {
            return Ok(targets);
        };
        let listed: std::collections::HashSet<&str> = channels
            .iter()
            .filter_map(|channel| channel["id"].as_str())
            .collect();
        let complete = snapshot["totals"]["channels"]
            .as_u64()
            .is_none_or(|total| usize::try_from(total).is_ok_and(|total| total <= listed.len()));
        if !complete {
            return Ok(targets);
        }
        let problems: Vec<String> = by_id
            .iter()
            .filter(|&&index| !listed.contains(targets[index].id.as_str()))
            .map(|&index| {
                format!(
                    "No channel you're in has the ID {}.",
                    safe_text(&targets[index].id)
                )
            })
            .collect();
        if problems.is_empty() {
            Ok(targets)
        } else {
            Err(restated(problems.join("\n"), Some("unknown_name")))
        }
    }

    /// [`Self::target`] for a channel only its members can use ([`Self::resolve_yours`]).
    async fn your_channel(&self, text: &str) -> Result<Target> {
        let mut targets = self.resolve_yours(&[(Kind::Channel, text)]).await?;
        targets.pop().context("A name was left unresolved")
    }
}

/// `connection` with `effective_mode` from its own mode and `snapshot`'s workspace (SF-F1), and
/// `workspace_mode` and `workspace_name` when the snapshot was read.
fn with_privacy_of(mut connection: Value, snapshot: Option<&Value>) -> Value {
    if !connection.is_object() {
        return connection;
    }
    let workspace_mode = snapshot.and_then(|snapshot| snapshot["workspace"]["mode"].as_str());
    connection["effective_mode"] = json!(output::effective_mode(
        connection["mode"].as_str(),
        workspace_mode
    ));
    if let Some(mode) = workspace_mode {
        connection["workspace_mode"] = json!(mode);
    }
    if let Some(name) = snapshot.and_then(workspace_name_in) {
        connection["workspace_name"] = json!(name);
    }
    connection
}

/// The host's principal ID: projected by newer brokers, else the active principal holding the
/// host's UID.
fn snapshot_host_id(snapshot: &Value) -> Option<String> {
    snapshot["workspace"]["host_principal_id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            let uid = snapshot["workspace"]["host_uid"].as_u64()?;
            snapshot["principals"]
                .as_array()?
                .iter()
                .find(|person| {
                    person["uid"].as_u64() == Some(uid) && person["active"].as_bool() != Some(false)
                })
                .and_then(|person| person["id"].as_str())
                .map(str::to_owned)
        })
}

/// Each person in `people` with the two facts the text list shows beside them (CLI-15):
/// `is_you` and `is_host`, from the snapshot's actor and host.
fn with_roles(mut people: Value, snapshot: &Value) -> Value {
    let actor = snapshot["actor"]["id"].as_str();
    // An older broker names only the host's UID; one active principal holds it.
    let host = snapshot_host_id(snapshot);
    // Presence (W2-BRK-6), when the broker reports it; an older one reports none, and nothing
    // is invented.
    let online: Option<Vec<&str>> = snapshot["online_principal_ids"]
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_str).collect());
    if let Some(people) = people.as_array_mut() {
        for person in people.iter_mut().filter(|person| person.is_object()) {
            let id = person["id"].as_str().map(str::to_owned);
            person["is_you"] = json!(id.is_some() && id.as_deref() == actor);
            person["is_host"] = json!(id.is_some() && id == host);
            if let Some(online) = &online {
                person["is_online"] = json!(id.as_deref().is_some_and(|id| online.contains(&id)));
            }
        }
    }
    people
}

/// Each channel in `channels` with its `unread` count from the snapshot's separate map, which
/// the text list shows (CLI-15). A snapshot without the map (an older broker) adds nothing.
fn with_unread(mut channels: Value, snapshot: &Value) -> Value {
    let Some(unread) = snapshot["unread"].as_object() else {
        return channels;
    };
    if let Some(channels) = channels.as_array_mut() {
        for channel in channels.iter_mut().filter(|channel| channel.is_object()) {
            let count = channel["id"]
                .as_str()
                .and_then(|id| unread.get(id))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            channel["unread"] = json!(count);
        }
    }
    channels
}

fn snapshot_field(snapshot: &Value, field: &str) -> Result<Value> {
    snapshot
        .get(field)
        .cloned()
        .context("Daemon returned an incomplete workspace snapshot")
}

fn add_expected_mode(mut body: Value, expected_mode: Option<PrivacyMode>) -> Value {
    let Some(mode) = expected_mode else {
        return body;
    };
    body["expected_mode"] = Value::from(mode.as_str());
    body
}

fn add_personal_mode(method: &str, params: &mut Value, expected_mode: Option<PrivacyMode>) {
    let ("message.post" | "blob.begin", Some(mode)) = (method, expected_mode) else {
        return;
    };
    params["personal_mode"] = Value::from(mode.as_str());
}

/// A team, file or workspace name: escaped, and isolated (U+2068 … U+2069) when it could hold
/// right-to-left text, so it cannot reorder the words around it. ASCII stays bare, so a copied
/// name holds no invisible characters.
fn name_text(name: &str) -> String {
    let text = safe_text(name.trim());
    if text.is_ascii() {
        text
    } else {
        format!("\u{2068}{text}\u{2069}")
    }
}

/// `#methods`, from a stored channel name.
fn channel_text(name: &str) -> String {
    format!("#{}", name_text(name.trim().trim_start_matches('#')))
}

/// `"Bob Lee" (@bob)`, or `@bob` when the display name is the username: the display rule,
/// applied by `output` so the two never drift.
fn person_text(username: &str, display_name: Option<&str>) -> String {
    let directory = Directory::from_snapshot(&json!({
        "principals": [{"id": "person", "username": username, "display_name": display_name}]
    }));
    output::person_label(&directory, "person", false)
}

/// "Bob" for "Send Bob this code": the first word of a display name, or `@username` when the
/// person set none.
fn first_name(person: &Value) -> String {
    let username = person["username"].as_str().unwrap_or_default();
    let display = person["display_name"]
        .as_str()
        .map(str::trim)
        .filter(|display| !display.is_empty() && display.to_lowercase() != username.to_lowercase());
    match display.and_then(|display| display.split_whitespace().next()) {
        Some(first) => name_text(first),
        None if username.is_empty() => "the host".to_owned(),
        None => format!("@{}", safe_text(username)),
    }
}

/// A username typed after `@` for someone who is not a member yet (an invitation to join): one
/// leading `@` removed. The broker canonicalizes it against the server's accounts.
fn account_name(typed: &str) -> Result<&str> {
    let typed = typed.trim();
    let name = typed.strip_prefix('@').unwrap_or(typed);
    ensure!(
        !name.is_empty() && !name.chars().any(|c| c.is_whitespace() || c.is_control()),
        "Type the person's username on the server, like @bob."
    );
    Ok(name)
}

fn same_username(typed: &str, username: &str) -> bool {
    let typed = typed.trim();
    typed.strip_prefix('@').unwrap_or(typed) == username
}

/// A command a shell can run as printed: the word bare when it is plain, else single-quoted.
fn shell_word(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@/:".contains(&b))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// "in 23 hours", for an invitation's expiry.
fn time_left(seconds: i64) -> String {
    let minutes = (seconds + 59) / 60;
    if minutes < 60 {
        return format!("in {minutes} minute{}", if minutes == 1 { "" } else { "s" });
    }
    let hours = (seconds + 1800) / 3600;
    format!("in {hours} hour{}", if hours == 1 { "" } else { "s" })
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Ask a question on the terminal and return the answer, trimmed.
async fn ask(question: String) -> Result<String> {
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut stderr = std::io::stderr().lock();
        write!(stderr, "{question} ")?;
        stderr.flush()?;
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        Ok(answer.trim().to_owned())
    })
    .await
    .context("The question could not be asked")?
}

/// How a consequential command is confirmed (CLI-1, CLI-10), matching the desktop's friction:
/// a plain question for archiving a channel or removing someone from it, the name typed again
/// for removing a connection or making anything public.
#[derive(Debug, PartialEq, Eq)]
enum Consent {
    /// `--yes`, or the name given again with `--confirm`.
    Given,
    /// Ask this on the terminal.
    Ask(String),
}

/// A yes-or-no decision: `--yes`, else a question on the terminal, else the terminal refusal
/// (exit status 2, nothing sent).
fn yes_or_ask(
    yes: bool,
    interactive: bool,
    question: String,
    no_terminal: String,
) -> Result<Consent> {
    if yes {
        Ok(Consent::Given)
    } else if interactive {
        Ok(Consent::Ask(question))
    } else {
        Err(needs_a_terminal(no_terminal))
    }
}

async fn ask_yes(question: String, declined: &str) -> Result<()> {
    let answer = ask(format!("{question} [y/N]")).await?;
    ensure!(
        matches!(answer.to_lowercase().as_str(), "y" | "yes"),
        "{declined}"
    );
    Ok(())
}

/// A decision confirmed by typing `phrase` again, as the desktop's typed confirmations are
/// (letter case aside): `--confirm PHRASE`, else a question on the terminal, else the terminal
/// refusal.
fn typed_or_ask(
    phrase: &str,
    confirm: Option<&str>,
    interactive: bool,
    no_terminal: String,
) -> Result<Consent> {
    match confirm {
        Some(typed) => {
            ensure!(
                same_phrase(typed, phrase),
                "Not done: --confirm {} doesn't match {}.",
                safe_text(typed.trim()),
                name_text(phrase)
            );
            Ok(Consent::Given)
        }
        None if interactive => Ok(Consent::Ask(format!(
            "Type {} to confirm:",
            name_text(phrase)
        ))),
        None => Err(needs_a_terminal(no_terminal)),
    }
}

fn same_phrase(typed: &str, phrase: &str) -> bool {
    typed.trim().to_lowercase() == phrase.trim().to_lowercase()
}

/// Ask for `phrase` on the terminal after `lines`, when [`typed_or_ask`] says to.
async fn typed_consent(
    phrase: &str,
    confirm: Option<&str>,
    interactive: bool,
    lines: &[String],
    no_terminal: String,
) -> Result<()> {
    if let Consent::Ask(question) = typed_or_ask(phrase, confirm, interactive, no_terminal)? {
        let mut stderr = std::io::stderr().lock();
        for line in lines {
            writeln!(stderr, "{line}")?;
        }
        drop(stderr);
        let answer = ask(question).await?;
        ensure!(
            same_phrase(&answer, phrase),
            "Not done: that isn't {}.",
            name_text(phrase)
        );
    }
    Ok(())
}

/// A public setting, confirmed by typing the workspace's name.
async fn confirm_public(
    api: &Api,
    workspace: &str,
    confirm: Option<&str>,
    lines: Vec<String>,
    no_terminal: String,
) -> Result<()> {
    typed_consent(workspace, confirm, api.interactive, &lines, no_terminal).await
}

/// The name a typed confirmation asks for: the workspace's own name, else the caller falls
/// back to the saved connection's (`workspacePhraseFor` on the desktop).
fn workspace_name_in(snapshot: &Value) -> Option<String> {
    snapshot["workspace"]["name"]
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !uuid_shaped(name))
        .map(str::to_owned)
}

fn connection_name(connection: &Value) -> String {
    connection["name"]
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("this connection")
        .to_owned()
}

/// Whether the person hosts the workspace, and whether another of their computers could still
/// act as its host, read from their snapshot.
#[derive(Debug, PartialEq, Eq)]
enum HostStanding {
    NotHost,
    /// The host, and the workspace lists more than one computer enrolled as them. `others` are
    /// the listed computers other than this one. The list is not proof that any of them can still
    /// act as host: a computer stays on it after its connection is removed there, because the
    /// broker drops a device only when its whole member is revoked.
    HostElsewhereToo {
        others: Vec<Value>,
    },
    /// The host, and no other computer of theirs is known: removing this one ends the host
    /// controls for good.
    OnlyHostComputer {
        username: String,
    },
    /// The snapshot could not be read or does not say.
    Unknown,
}

/// The first 16 hex digits of a device ID or a snapshot fingerprint, uppercased: the broker's
/// fingerprint is those digits of the device ID, grouped.
fn fingerprint_digits(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_hexdigit)
        .take(16)
        .map(|digit| digit.to_ascii_uppercase())
        .collect()
}

/// `own_device_id` is this connection's device ID, left out of `others` when it is known.
fn host_standing(snapshot: &Value, own_device_id: Option<&str>) -> HostStanding {
    let actor = &snapshot["actor"];
    let Some(actor_id) = actor["id"].as_str() else {
        return HostStanding::Unknown;
    };
    let workspace = &snapshot["workspace"];
    let hosts = match workspace["host_principal_id"].as_str() {
        Some(host) => host == actor_id,
        None => match (workspace["host_uid"].as_u64(), actor["uid"].as_u64()) {
            (Some(host), Some(uid)) => host == uid,
            _ => return HostStanding::Unknown,
        },
    };
    if !hosts {
        return HostStanding::NotHost;
    }
    let own = own_device_id
        .map(fingerprint_digits)
        .filter(|own| !own.is_empty());
    let others: Vec<Value> = match actor["devices"].as_array() {
        // One listed computer is this one, or the only one there is.
        Some(devices) if devices.len() > 1 => devices
            .iter()
            .filter(|device| {
                own.as_deref().is_none_or(|own| {
                    device["fingerprint"]
                        .as_str()
                        .map(fingerprint_digits)
                        .as_deref()
                        != Some(own)
                })
            })
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    if others.is_empty() {
        HostStanding::OnlyHostComputer {
            username: actor["username"].as_str().unwrap_or_default().to_owned(),
        }
    } else {
        HostStanding::HostElsewhereToo { others }
    }
}

/// The device ID of the computer a saved connection belongs to, when the daemon says.
fn own_device_id(connection: &Value) -> Option<&str> {
    connection["device_id"]
        .as_str()
        .filter(|device| !fingerprint_digits(device).is_empty())
}

/// What `connections remove` says about hosting before it asks, or its refusal (CLI-1).
/// `workspace` is already escaped for display, and `own_device_known` says whether `others`
/// has this computer left out.
fn host_removal_notice(
    standing: &HostStanding,
    workspace: &str,
    own_device_known: bool,
    give_up_host_controls: bool,
    options: &HumanOptions,
) -> Result<Option<String>> {
    Ok(match standing {
        HostStanding::NotHost => None,
        HostStanding::OnlyHostComputer { username } if !give_up_host_controls => {
            let invite = if username.is_empty() {
                "biorouter crew enroll invite @YOUR_USERNAME --add-device".to_owned()
            } else {
                format!(
                    "biorouter crew enroll invite @{} --add-device",
                    safe_text(username)
                )
            };
            return Err(restated(
                format!(
                    "Not removed. You host {workspace}, and no other computer of yours can act as its host. Removing it here ends the host controls for good: nobody could let people in, change its privacy or remove anyone again.\nTo keep them, first add another computer with {invite}.\nTo remove it anyway, add --give-up-host-controls."
                ),
                Some("crew_host_controls_would_end"),
            ));
        }
        HostStanding::OnlyHostComputer { .. } => Some(format!(
            "You host {workspace} and no other computer of yours can act as its host, so its host controls end for good."
        )),
        HostStanding::HostElsewhereToo { others } => {
            let listed = if own_device_known {
                format!(
                    "You host {workspace}. Its host controls continue only if one of your other enrolled computers still has {workspace} saved:"
                )
            } else {
                format!(
                    "You host {workspace}. Its host controls continue only if another of your enrolled computers still has {workspace} saved. These computers are enrolled as you, this one among them:"
                )
            };
            let mut notice = vec![listed];
            notice.extend(
                others
                    .iter()
                    .map(|device| format!("  {}", output::device_text(device, options))),
            );
            notice.push(format!(
                "A computer stays on this list after its connection is removed there. If none of them still has {workspace} saved, removing it here ends the host controls for good."
            ));
            Some(notice.join("\n"))
        }
        HostStanding::Unknown => Some(format!(
            "Biorouter couldn't check whether you host {workspace}. If you do and this is your only computer in it, removing it ends the host controls for good."
        )),
    })
}

/// `connections remove` (CLI-1): delete the saved connection and this computer's device key.
/// For a host's only computer that ends the host controls for good, so it is refused unless
/// `--give-up-host-controls` says so; every removal is confirmed by typing the name, and what
/// removing it means for the host controls is said first, on stderr when nothing is asked.
async fn remove_connection(
    api: &Api,
    confirm: Option<&str>,
    give_up_host_controls: bool,
) -> Result<Reply> {
    let connection = api.connection().await?;
    let name = connection_name(&connection);
    let shown = name_text(&name);
    let own_device = own_device_id(&connection);
    let snapshot = api.snapshot().await.ok();
    let standing = snapshot.as_ref().map_or(HostStanding::Unknown, |snapshot| {
        host_standing(snapshot, own_device)
    });
    let workspace = snapshot
        .as_ref()
        .and_then(workspace_name_in)
        .map_or_else(|| shown.clone(), |workspace| name_text(&workspace));
    let mut lines = vec![format!(
        "Remove {shown} from this computer? It disconnects, ends every chat's access through it and deletes this computer's device key for the workspace. Your messages stay on the server."
    )];
    if let Some(notice) = host_removal_notice(
        &standing,
        &workspace,
        own_device.is_some(),
        give_up_host_controls,
        &HumanOptions::new(api.show_ids),
    )? {
        if confirm.is_some() || !api.interactive {
            eprintln!("{notice}");
        }
        lines.push(notice);
    }
    typed_consent(
        &name,
        confirm,
        api.interactive,
        &lines,
        format!(
            "Removing {shown} deletes this computer's device key for the workspace. There is no terminal to ask in, so confirm with --confirm {}.",
            safe_text(&shell_word(&name))
        ),
    )
    .await?;
    let result = api
        .client
        .request("DELETE", &api.path("").await?, None)
        .await?;
    Ok(api.say(result, vec![format!("Removed {shown} from this computer.")]))
}

async fn connections(api: &Api, command: ConnectionCommand) -> Result<Reply> {
    Ok(match command {
        ConnectionCommand::List => api.show(api.connections_with_privacy().await?),
        ConnectionCommand::Show => {
            let connection = api.connection().await?;
            api.show(api.with_effective_privacy(connection).await)
        }
        // SF-F3: the host's first step and a joiner's are this one command, the same as
        // `enroll prepare`, so the key is described for both uses in one set of words.
        ConnectionCommand::Prepare => api.show(prepare(api).await?),
        ConnectionCommand::Save { input } => {
            let body: Value = serde_json::from_str(&read_input(&input)?)
                .context("Connection input must be a JSON descriptor")?;
            api.show(
                api.client
                    .request("POST", "/crew/connections", Some(body))
                    .await?,
            )
        }
        ConnectionCommand::Update { input } => {
            let body: Value = serde_json::from_str(&read_input(&input)?)
                .context("Connection input must be a JSON descriptor")?;
            api.show(
                api.client
                    .request("PATCH", &api.path("").await?, Some(body))
                    .await?,
            )
        }
        ConnectionCommand::Remove {
            confirm,
            give_up_host_controls,
        } => remove_connection(api, confirm.as_deref(), give_up_host_controls).await?,
        ConnectionCommand::JoinInvitation(args) => join_invitation(api, *args).await?,
        ConnectionCommand::Invitation { invitee } => {
            let invitation = fetch_invitation(api, invitee.as_deref()).await?;
            let lines = invitation_lines(&invitation);
            api.say(invitation, lines)
        }
    })
}

async fn prepare(api: &Api) -> Result<Value> {
    api.client
        .request("POST", "/crew/devices/prepare", Some(json!({})))
        .await
}

/// The host's invitation message (`GET …/invitation`), naming `invitee` when given.
async fn fetch_invitation(api: &Api, invitee: Option<&str>) -> Result<Value> {
    let mut path = api.path("/invitation").await?;
    if let Some(invitee) = invitee.map(str::trim).filter(|invitee| !invitee.is_empty()) {
        path.push_str("?invitee=");
        path.push_str(&urlencoding::encode(invitee));
    }
    api.client
        .request("GET", &path, None)
        .await
        .map_err(|error| older_daemon(error, RESTART_FOR_JOINING))
}

/// The invitation message, line by line, exactly as the person copies it.
fn invitation_lines(invitation: &Value) -> Vec<String> {
    let text = invitation["message"]
        .as_str()
        .or_else(|| invitation["line"].as_str())
        .unwrap_or_default();
    text.lines().map(safe_text).collect()
}

async fn join_invitation(api: &Api, args: JoinInvitationArgs) -> Result<Reply> {
    let from_stdin = args.input == Path::new("-");
    let pasted = read_input(&args.input)?;
    ensure!(
        !pasted.trim().is_empty(),
        "Paste the whole invitation your host sent, or the brcrew1: line in it."
    );
    let mut request = invitation_request(&pasted, &args)?;
    let answer = from_invitation(api, &request, true).await?;
    let preview = answer
        .get("preview")
        .filter(|preview| preview.is_object())
        .context("The daemon did not describe the invitation")?;
    // SC2-N1: a connection saved from someone else's invitation, never connected, is offered
    // for replacement, and replaced only when the person asks with --replace.
    let replaceable = preview["replaceable_connection_id"].as_str();
    if args.replace && replaceable.is_none() {
        return Err(usage(
            "There's no saved connection for this workspace that this invitation can replace. Run it again without --replace.",
            NOTHING_TO_REPLACE,
        ));
    }
    let saved_names = saved_connection_names(api).await;
    let mut summary = invitation_summary(preview, args.preparation_id.is_some());
    summary.extend(saved_connection_lines(preview, &saved_names, args.replace));
    if args.replace {
        request["replace"] = json!(replaceable);
    }
    if args.preview {
        return Ok(api.say(answer.clone(), summary));
    }
    let missing = missing_choices(preview);
    if !missing.is_empty() {
        if api.text() {
            print_lines(&summary)?;
        }
        bail!("Saving this invitation needs {}.", missing.join(", "));
    }
    let mut asked = false;
    if !args.yes {
        if from_stdin {
            return Err(needs_a_terminal(
                "Add --yes to save this connection: the invitation came from stdin, so there is no terminal to ask in. Run with --preview to check it first.",
            ));
        }
        if !api.interactive {
            return Err(needs_a_terminal(
                "Add --yes to save this connection; there is no terminal to ask in. Run with --preview to check it first.",
            ));
        }
        let mut stderr = std::io::stderr().lock();
        for line in &summary {
            writeln!(stderr, "{line}")?;
        }
        drop(stderr);
        let answer = ask("Save this connection? [y/N]".into()).await?;
        ensure!(
            matches!(answer.to_lowercase().as_str(), "y" | "yes"),
            "Nothing was saved."
        );
        asked = true;
    }
    let saved = match from_invitation(api, &request, false).await {
        Ok(saved) => saved,
        Err(error) => return Err(saved_conflict(error, preview, &saved_names)),
    };
    let connection = saved
        .get("connection")
        .filter(|connection| connection.is_object())
        .context("The daemon did not return the saved connection")?;
    let name = connection["name"].as_str().unwrap_or("the connection");
    let mut lines = if asked || !api.text() {
        Vec::new()
    } else {
        summary
    };
    lines.push(api.with_id(
        format!("Saved {}.", name_text(name)),
        "connection ID",
        connection["id"].as_str().unwrap_or_default(),
    ));
    lines.extend(next_steps(
        name,
        args.preparation_id.is_some(),
        connection["status"].as_str() == Some("connected"),
    ));
    Ok(api.say(saved, lines))
}

/// What to run after saving a connection (CLI-9). A host saving their own workspace
/// (`--preparation-id`) signs in and then sets the workspace up; anyone else signs in and then
/// joins. Signing in is skipped when the connection is already up.
fn next_steps(name: &str, hosting: bool, connected: bool) -> Vec<String> {
    let command = |verb: &str| {
        format!(
            "biorouter crew --connection {} {verb}",
            safe_text(&shell_word(name))
        )
    };
    let then = if hosting {
        "workspace bootstrap"
    } else {
        "join"
    };
    if connected {
        vec![format!("Next: {}", command(then))]
    } else {
        vec![
            format!("Next: {}", command("auth")),
            format!("Then: {}", command(then)),
        ]
    }
}

/// `POST /crew/connections/from-invitation`'s body: the pasted text and every choice made.
fn invitation_request(pasted: &str, args: &JoinInvitationArgs) -> Result<Value> {
    let mut body = json!({"invitation": pasted});
    if let Some(username) = &args.username {
        let username = username.trim();
        body["username"] = json!(username.strip_prefix('@').unwrap_or(username));
    }
    if let Some(mode) = args.mode {
        body["mode"] = json!(mode.as_str());
    }
    if let Some(institution) = &args.institution_id {
        body["institution_id"] = json!(institution);
    }
    let mut advanced = serde_json::Map::new();
    let identity_file = args
        .identity_file
        .as_deref()
        .map(|path| {
            absolute_local_path(path, etcetera::home_dir().ok(), || {
                Ok(std::env::current_dir()?)
            })
        })
        .transpose()?;
    for (key, value) in [
        ("ssh_target", &args.ssh_target),
        ("identity_file", &identity_file),
        ("proxy_jump", &args.proxy_jump),
        ("name", &args.name),
        ("remote_root", &args.remote_root),
        ("preparation_id", &args.preparation_id),
    ] {
        if let Some(value) = value {
            advanced.insert(key.into(), json!(value));
        }
    }
    if let Some(port) = args.port {
        advanced.insert("port".into(), json!(port));
    }
    if args.remote_execution {
        advanced.insert("remote_execution".into(), json!(true));
    }
    if !advanced.is_empty() {
        body["advanced"] = Value::Object(advanced);
    }
    Ok(body)
}

/// A local file path as the daemon needs it, absolute (CLI-19): a leading `~/` (or `~\` on
/// Windows) is the home folder, since a shell leaves `--identity-file=~/.ssh/key` as it is,
/// and a relative path is taken from the current folder, as the files commands take theirs.
/// An empty value is left for the daemon to judge.
fn absolute_local_path(
    text: &str,
    home: Option<std::path::PathBuf>,
    current_dir: impl FnOnce() -> Result<std::path::PathBuf>,
) -> Result<String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    // What follows `~` and a separator, or "" for a bare `~`. `~bob/key` is not in it.
    let under_home = match text.strip_prefix('~') {
        Some("") => Some(""),
        Some(rest) => rest.strip_prefix(std::path::is_separator),
        None => None,
    };
    let path = if let Some(rest) = under_home {
        let home = home.context("Biorouter couldn't find your home folder for the ~ in --identity-file; give the full path")?;
        if rest.is_empty() {
            home
        } else {
            home.join(rest)
        }
    } else if Path::new(text).is_absolute() {
        return Ok(text.to_owned());
    } else {
        current_dir()?.join(text)
    };
    path.to_str()
        .map(str::to_owned)
        .context("Crew local paths must be valid Unicode")
}

async fn from_invitation(api: &Api, request: &Value, preview: bool) -> Result<Value> {
    let mut body = request.clone();
    body["preview"] = json!(preview);
    api.client
        .request("POST", "/crew/connections/from-invitation", Some(body))
        .await
        .map_err(|error| older_daemon(error, RESTART_FOR_JOINING))
}

/// "Private · ucsf", "Private" or "Public".
fn privacy_badge(mode: Option<&str>, institution: Option<&str>) -> String {
    match (mode, institution.filter(|id| !id.is_empty())) {
        (Some("private"), Some(institution)) => format!("Private · {}", safe_text(institution)),
        (Some("private"), None) => "Private".into(),
        (Some("public"), _) => "Public".into(),
        _ => "not stated".into(),
    }
}

/// What the invitation says and what saving it would do, as the Join screen shows it. A host
/// saving their own workspace (`--preparation-id`, what `biorouter-crew start` printed) is not
/// invited and does not join: it is their workspace, and they host it (SF-F3).
fn invitation_summary(preview: &Value, hosting: bool) -> Vec<String> {
    let field = |key: &str| preview[key].as_str().filter(|value| !value.is_empty());
    let workspace = field("workspace_label")
        .or_else(|| field("workspace_name"))
        .map_or_else(|| "a workspace".to_owned(), name_text);
    let mut lines = vec![if hosting {
        format!("Your workspace {workspace}")
    } else {
        format!("Invitation to {workspace}")
    }];
    let host = field("host_username").map(|host| person_text(host, field("host_display_name")));
    // SC2-N5: the server by the name the desktop shows (`server_label`: the person's own SSH
    // alias for it, else its host), not the address the invitation resolved.
    let server = field("server_label")
        .or_else(|| field("server"))
        .or_else(|| field("ssh_host"))
        .map(safe_text);
    match (&host, &server) {
        (Some(host), Some(server)) => lines.push(format!("  Hosted by {host} on {server}")),
        (Some(host), None) => lines.push(format!("  Hosted by {host}")),
        (None, Some(server)) => lines.push(format!("  On {server}")),
        (None, None) => {}
    }
    // SETUPHPC2-F-C: a member on the workspace's own server signs in to this machine.
    if field("proxy_jump").is_none() {
        if let Some((_, here)) = field("ssh_target").and_then(output::loopback_login) {
            lines.push(format!(
                "  Crew will connect to this machine ({}).",
                safe_text(here)
            ));
        }
    }
    let workspace_privacy =
        privacy_badge(field("workspace_mode"), field("workspace_institution_id"));
    lines.push(format!("  Workspace privacy: {workspace_privacy}"));
    if let Some(fingerprint) = field("fingerprint") {
        lines.push(format!("  Fingerprint: {}", safe_text(fingerprint)));
    }
    if let Some(username) = field("username") {
        let server = server.as_deref().unwrap_or("the server");
        lines.push(format!(
            "  Your username on {server}: {}",
            safe_text(username)
        ));
    }
    // SC2-N1: the invitation names one account, and this computer's SSH settings sign in to the
    // server as another: someone else's invitation, which the host must replace.
    let mismatch = &preview["login_mismatch"];
    if let (Some(config_user), Some(invitee)) = (
        mismatch["config_user"]
            .as_str()
            .filter(|user| !user.is_empty()),
        mismatch["invitee"].as_str().filter(|user| !user.is_empty()),
    ) {
        lines.push(format!(
            "  This invitation is for @{}, but this computer signs in to {} as {}. Ask your host for your own invitation.",
            safe_text(invitee),
            server.as_deref().unwrap_or("the server"),
            safe_text(config_user)
        ));
    }
    let choice = privacy_badge(field("mode"), field("institution_id"));
    lines.push(if hosting {
        format!("  You'll host it as {choice}.")
    } else {
        format!("  You'll join as {choice}.")
    });
    if let (Some(workspace_institution), Some(chosen)) =
        (field("workspace_institution_id"), field("institution_id"))
    {
        if workspace_institution != chosen {
            lines.push(format!(
                "  {workspace} uses {}; you chose {}.",
                safe_text(workspace_institution),
                safe_text(chosen)
            ));
        }
    }
    if let Some(conflict) = field("institution_conflict") {
        lines.push(format!("  {}", safe_text(conflict)));
    }
    if preview["mode_differs"].as_bool() == Some(true) {
        lines.push(format!(
            "  {workspace} is {workspace_privacy}. Your connection will be {choice}."
        ));
    }
    if let Some(name) = field("name") {
        lines.push(format!(
            "  Connection name on this computer: {}",
            name_text(name)
        ));
    }
    lines
}

/// The code of `join-invitation --replace` when nothing saved can be replaced.
const NOTHING_TO_REPLACE: &str = "crew_nothing_to_replace";

/// Each saved connection's name by ID, for sentences about one this computer already has.
/// Empty when the list cannot be read: those sentences then say "a connection".
async fn saved_connection_names(api: &Api) -> std::collections::HashMap<String, String> {
    let Ok(listed) = api.connections().await else {
        return std::collections::HashMap::new();
    };
    listed["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|connection| {
            Some((
                connection["id"].as_str()?.to_owned(),
                connection_name(connection),
            ))
        })
        .collect()
}

/// A saved connection's name for a sentence: `chen-lab`, isolated when it could hold
/// right-to-left text, or "a connection" when it could not be read.
fn saved_name(names: &std::collections::HashMap<String, String>, id: &str) -> String {
    names
        .get(id)
        .map_or_else(|| "a connection".to_owned(), |name| name_text(name))
}

/// What the invitation summary says about a connection this computer already has for the same
/// workspace (SC2-N1, CLIDOCS-F6): the one saving keeps, or one saved from someone else's
/// invitation that `--replace` replaces.
fn saved_connection_lines(
    preview: &Value,
    names: &std::collections::HashMap<String, String>,
    replace: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(id) = preview["existing_connection_id"].as_str() {
        lines.push(format!(
            "  This computer already has this workspace as {}; saving again keeps that connection if its settings match.",
            saved_name(names, id)
        ));
    }
    if let Some(id) = preview["replaceable_connection_id"].as_str() {
        let name = saved_name(names, id);
        lines.push(if replace {
            format!("  Saving replaces {name}, which signs in as another account and has never connected. Its key is deleted.")
        } else {
            format!("  This computer already has {name} for this workspace, signing in as another account, and it has never connected. Add --replace to save this invitation in its place.")
        });
    }
    lines
}

/// A save refused because this computer already has the workspace (`crew_connection_exists`),
/// said with the command that acts on it (SC2-N1, CLIDOCS-F6): the daemon's "Replace it … or
/// change it in its connection settings" names the desktop's controls. JSON keeps its code.
fn saved_conflict(
    error: anyhow::Error,
    preview: &Value,
    names: &std::collections::HashMap<String, String>,
) -> anyhow::Error {
    let conflict = error.chain().find_map(|cause| {
        let (code, _) = refusal_code_and_message(cause)?;
        (code == "crew_connection_exists")
            .then(|| refusal_field(cause, "connection_id").map(str::to_owned))
    });
    let Some(id) = conflict else {
        return error;
    };
    let replaceable = preview["replaceable_connection_id"].as_str();
    let id = id.or_else(|| replaceable.map(str::to_owned));
    let raw = id
        .as_deref()
        .and_then(|id| names.get(id))
        .map_or("NAME", String::as_str);
    let name = id
        .as_deref()
        .map_or_else(|| "a connection".to_owned(), |id| saved_name(names, id));
    let command = |verb: &str| {
        format!(
            "biorouter crew --connection {} {verb}",
            safe_text(&shell_word(raw))
        )
    };
    let sentence = if id.is_some() && id.as_deref() == replaceable {
        format!(
            "This computer already has {name} for this workspace, signing in as another account, and it has never connected. Run it again with --replace, or remove it with {}.",
            command("connections remove")
        )
    } else {
        // T3-CLI-4: the daemon refuses an existing connection only when a setting it compares
        // differs (`same_settings`: the login, port, key file, jump host, mode and
        // institution); a matching save succeeds. So the saved one is not what was asked for,
        // and nothing was saved: joining with it is only one of the ways on.
        [
            format!("This workspace is already saved as {name} with a different server login, port, key file, jump host or privacy, so nothing was saved."),
            format!("  To compare them: {}", command("connections show")),
            format!(
                "  To give it this invitation's privacy: {}",
                command(&set_personal_words(preview))
            ),
            format!(
                "  To change its server login, port, key file or jump host: {}",
                command("connections update FILE")
            ),
            format!(
                "  If it has not joined yet, remove it and save this invitation again: {}",
                command("connections remove")
            ),
            format!("  To use it as it is: {}", command("join")),
        ]
        .join("\n")
    };
    Worded {
        sentence,
        source: error,
    }
    .into()
}

/// `privacy set-personal private --institution ucsf`: the privacy this invitation would save,
/// as the command that gives it to a connection already saved. The preview's words are used
/// only when they are a mode this command takes; anything else is left as `MODE`.
fn set_personal_words(preview: &Value) -> String {
    match preview["mode"].as_str() {
        Some("private") => match preview["institution_id"]
            .as_str()
            .map(str::trim)
            .filter(|institution| !institution.is_empty())
        {
            Some(institution) => format!(
                "privacy set-personal private --institution {}",
                safe_text(&shell_word(institution))
            ),
            None => "privacy set-personal private".to_owned(),
        },
        Some("public") => "privacy set-personal public".to_owned(),
        _ => "privacy set-personal MODE".to_owned(),
    }
}

/// What saving still needs, with the option that supplies each.
fn missing_choices(preview: &Value) -> Vec<&'static str> {
    preview["missing"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|missing| match missing {
            "username" => "your username on the server (--username)",
            "server" => "the server's address (--ssh-target)",
            "institution" => "an institution for a Private connection (--institution)",
            _ => "a choice this version of the command can't make; use the desktop app",
        })
        .collect()
}

/// `crew join`: show this computer's code and wait until the host lets it in. The code comes
/// from the daemon's `GET …/join`, computed there from this computer's own key and the pinned
/// workspace key; nothing the workspace sends can change it.
async fn join(api: &Api, args: JoinArgs) -> Result<Reply> {
    join_until(api, args, tokio::signal::ctrl_c()).await
}

/// [`join`], stopped by `interrupt` (Ctrl-C). One listener serves the whole wait (CLI-18): a
/// fresh `ctrl_c()` per sleep was not listening while the status was re-read or a claim was
/// made (either can wait on a slow SSH connect), and tokio's handler, installed by the first
/// one, then swallowed that press.
async fn join_until(
    api: &Api,
    args: JoinArgs,
    interrupt: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<Reply> {
    tokio::pin!(interrupt);
    let stopped = || {
        eprintln!("Stopped waiting. Run biorouter crew join again to continue.");
        Ok(Reply::Streamed)
    };
    let path = api.path("/join").await?;
    let mut shown: Option<(String, Option<String>)> = None;
    // Whether the status is being read again right after a claim was refused.
    let mut rechecking = false;
    loop {
        let status = tokio::select! {
            biased;
            signal = &mut interrupt => { signal?; return stopped(); }
            status = join_status(api, &path) => status?,
        };
        let state = status["status"].as_str().unwrap_or_default().to_owned();
        let code = status["code"].as_str().map(str::to_owned);
        let changed = shown.as_ref() != Some(&(state.clone(), code.clone()));
        match state.as_str() {
            "joined" => {
                api.stream(&status, &joined_lines(&status))?;
                return Ok(Reply::Streamed);
            }
            "approved" => {
                // Claim first and speak after (T-13): "approved" only means the host saved a
                // code, and a claim refused because it isn't this computer's must never have
                // been announced as "Joining…". The status the refusal leaves (usually
                // `code_mismatch`) is read once more, straight away, and that is what is said.
                if !rechecking {
                    let claimed = tokio::select! {
                        biased;
                        signal = &mut interrupt => { signal?; return stopped(); }
                        claimed = claim(api, &path) => claimed?,
                    };
                    if let Some(joined) = claimed {
                        api.stream(&joined, &joined_lines(&joined))?;
                        return Ok(Reply::Streamed);
                    }
                    rechecking = true;
                    continue;
                }
                if changed {
                    api.stream(&status, &join_lines(&status, !args.no_wait))?;
                }
            }
            "invited" | "code_mismatch" | "not_invited" => {
                if changed {
                    api.stream(&status, &join_lines(&status, !args.no_wait))?;
                }
            }
            "expired" => {
                let who = status
                    .get("inviter")
                    .filter(|inviter| inviter.is_object())
                    .map_or_else(
                        || "the host".to_owned(),
                        |inviter| {
                            person_text(
                                inviter["username"].as_str().unwrap_or_default(),
                                inviter["display_name"].as_str(),
                            )
                        },
                    );
                bail!("This invitation expired. Ask {who} to invite you again.");
            }
            "unsupported" => bail!(
                "This workspace's server can't let people join with a code yet. Ask the host for an enrollment token instead."
            ),
            _ => bail!(
                "Biorouter reported a join status this version of the command doesn't know. Update Biorouter and try again."
            ),
        }
        rechecking = false;
        if args.no_wait {
            return Ok(Reply::Streamed);
        }
        shown = Some((state, code));
        tokio::select! {
            biased;
            signal = &mut interrupt => { signal?; return stopped(); }
            () = tokio::time::sleep(api.poll) => {}
        }
    }
}

/// `GET …/join`, connecting first when the connection is not up.
async fn join_status(api: &Api, path: &str) -> Result<Value> {
    match api.client.request("GET", path, None).await {
        Err(error) if disconnected(&error) => {
            // A typed connect failure already says what to run; anything else is pointed at
            // `auth`, which signs in and connects.
            api.connect().await.map_err(|error| {
                if error.chain().any(|cause| connect_failure(cause).is_some()) {
                    error
                } else {
                    error.context("Connect to the workspace first: biorouter crew auth")
                }
            })?;
            api.client
                .request("GET", path, None)
                .await
                .map_err(|error| older_daemon(error, RESTART_FOR_JOINING))
        }
        answer => answer.map_err(|error| older_daemon(error, RESTART_FOR_JOINING)),
    }
}

/// `POST …/join` once the host approved a code. A claim refused because the approval does not
/// match this computer (or is not there yet) is not the end: the status says what to do next.
async fn claim(api: &Api, path: &str) -> Result<Option<Value>> {
    match api.client.request("POST", path, None).await {
        Ok(joined) => Ok(Some(joined)),
        Err(error) => match refusal(&error) {
            Some(Refusal {
                status: 409,
                code: Some(code),
            }) if matches!(
                code.as_str(),
                "crew_join_code_mismatch" | "crew_join_not_approved"
            ) =>
            {
                Ok(None)
            }
            _ => Err(older_daemon(error, RESTART_FOR_JOINING)),
        },
    }
}

/// What the join screen says for each state while waiting.
fn join_lines(status: &Value, waiting: bool) -> Vec<String> {
    let workspace = status["workspace_name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map_or_else(|| "the workspace".to_owned(), name_text);
    let inviter = status.get("inviter").filter(|inviter| inviter.is_object());
    let person = inviter.map(|inviter| {
        person_text(
            inviter["username"].as_str().unwrap_or_default(),
            inviter["display_name"].as_str(),
        )
    });
    let first = inviter.map_or_else(|| "the host".to_owned(), first_name);
    let code = status["code"].as_str().map_or_else(
        || "(run biorouter crew join again to see it)".to_owned(),
        safe_text,
    );
    let mut lines = match status["status"].as_str() {
        Some("invited") => vec![
            format!(
                "{} invited you to {workspace}.",
                person.as_deref().unwrap_or("The host")
            ),
            format!("Send {first} this code: {code}"),
        ],
        Some("code_mismatch") => vec![format!(
            "The code {first} entered doesn't match this computer. Send it again: {code}"
        )],
        Some("not_invited") => vec![format!(
            "You're not in {workspace} yet. Ask {} to invite your account on the server.",
            person.as_deref().unwrap_or("the host")
        )],
        Some("approved") if waiting => vec![format!("Joining {workspace}…")],
        Some("approved") => vec![format!(
            "{first} saved a code for you, but this computer hasn't joined {workspace} yet. Run biorouter crew join to finish."
        )],
        _ => Vec::new(),
    };
    if waiting && matches!(status["status"].as_str(), Some("invited" | "code_mismatch")) {
        lines.push(format!(
            "Waiting for {first} to let you in… Ctrl-C stops waiting; run this command again to continue."
        ));
    } else if waiting && status["status"].as_str() == Some("not_invited") {
        lines.push("Waiting for an invitation… Ctrl-C stops waiting.".into());
    }
    lines
}

fn joined_lines(status: &Value) -> Vec<String> {
    let workspace = status["workspace_name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map_or_else(|| "the workspace".to_owned(), name_text);
    let mut lines = vec![format!("You're in {workspace}.")];
    if status["add_device"].as_bool() == Some(true) {
        lines.push("This computer was added to your account.".into());
    }
    lines
}

async fn workspace(api: &Api, command: WorkspaceCommand) -> Result<Reply> {
    Ok(match command {
        WorkspaceCommand::Show => {
            let snapshot = api.snapshot().await?;
            let names = Directory::from_snapshot(&snapshot);
            api.show_with(snapshot, names)
        }
        WorkspaceCommand::Bootstrap => {
            let connection = api.connection().await?;
            api.show(
                api.broker(
                    "auth.bootstrap",
                    json!({"public_key": connection["public_key"]}),
                    true,
                )
                .await?,
            )
        }
        WorkspaceCommand::Rename { name } => {
            let result = api
                .broker("workspace.rename", json!({"name": name}), true)
                .await?;
            let stored = result["name"].as_str().unwrap_or(&name).to_owned();
            api.say(
                result,
                vec![format!("Renamed the workspace to {}.", name_text(&stored))],
            )
        }
    })
}

async fn enrollment(api: &Api, command: EnrollmentCommand) -> Result<Reply> {
    Ok(match command {
        EnrollmentCommand::Prepare => api.show(prepare(api).await?),
        EnrollmentCommand::Invite(args) => enroll_invite(api, args).await?,
        EnrollmentCommand::Pending => {
            let snapshot = api.snapshot().await?;
            let joins = snapshot
                .get("pending_joins")
                .cloned()
                .ok_or_else(|| no_pending_joins(&snapshot))?;
            let lines = pending_lines(&joins, now());
            api.say(joins, lines)
        }
        EnrollmentCommand::Approve {
            person,
            code,
            replace,
        } => {
            let username = account_name(&person)?;
            check_device_code(&code)?;
            let mut params = json!({"username": username, "code": code.trim()});
            if replace {
                params["replace"] = json!(true);
            }
            let result = api.broker("enrollment.approve", params, true).await?;
            let who = result["username"].as_str().unwrap_or(username).to_owned();
            api.say(
                result,
                vec![format!(
                    "Code saved. @{} joins when their computer confirms the same code.",
                    safe_text(&who)
                )],
            )
        }
        EnrollmentCommand::Cancel { person } => {
            let username = account_name(&person)?;
            let result = api
                .broker("enrollment.cancel", json!({"username": username}), true)
                .await?;
            api.say(
                result,
                vec![format!(
                    "Cancelled @{}'s invitation to join.",
                    safe_text(username)
                )],
            )
        }
        EnrollmentCommand::Accept(input) => {
            eprintln!("biorouter crew enroll accept is deprecated. Paste the invitation your host sent into biorouter crew connections join-invitation -, then run biorouter crew join.");
            let connection = api.connection().await?;
            let secret = read_secret(input).await?;
            api.show(
                api.broker(
                    "auth.enroll",
                    json!({"invitation":secret.as_str(),"public_key":connection["public_key"]}),
                    true,
                )
                .await?,
            )
        }
        EnrollmentCommand::Revoke { member, confirm } => {
            revoke_member(api, &member, confirm.as_deref()).await?
        }
    })
}

/// [`args::device_code`] again, for a caller that did not come through the command line.
fn check_device_code(code: &str) -> Result<()> {
    args::device_code(code)
        .map(drop)
        .map_err(anyhow::Error::msg)
}

async fn enroll_invite(api: &Api, args: EnrollInviteArgs) -> Result<Reply> {
    if let Some(uid) = args.uid {
        eprintln!("enroll invite --uid --public-key is deprecated. Invite by username instead: biorouter crew enroll invite @bob");
        let public_key = args
            .public_key
            .context("--public-key is required with --uid")?;
        let mut params = json!({"uid":uid,"public_key":public_key});
        if let Some(principal) = args.existing_principal {
            params["existing_principal_id"] = json!(principal);
        }
        return Ok(api.show(api.broker("enrollment.invite", params, true).await?));
    }
    let typed = args
        .username
        .context("Name the person to invite, like @bob")?;
    let username = account_name(&typed)?;
    let mut params = json!({"username": username});
    if args.add_device {
        params["add_device"] = json!(true);
    }
    let mut result = api.broker("enrollment.invite", params, true).await?;
    let canonical = result["username"].as_str().unwrap_or(username).to_owned();
    // The message is what the person needs next. The invitation stands without it, so a
    // failure here only changes what is printed.
    let invitation = fetch_invitation(api, Some(&canonical)).await.ok();
    let lines = invite_lines(&result, &canonical, invitation.as_ref());
    if let Some(invitation) = invitation {
        result["invitation"] = invitation;
    }
    Ok(api.say(result, lines))
}

fn invite_lines(result: &Value, username: &str, invitation: Option<&Value>) -> Vec<String> {
    let handle = format!("@{}", safe_text(username));
    let mut lines = vec![if result["add_device"].as_bool() == Some(true) {
        format!("Invited {handle} to add another computer.")
    } else {
        match result["full_name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
        {
            Some(full_name) => format!(
                "Invited {handle} · {} (name on the server account).",
                name_text(full_name)
            ),
            None => format!("Invited {handle}."),
        }
    }];
    match invitation.map(invitation_lines).filter(|lines| !lines.is_empty()) {
        Some(message) => {
            lines.push(format!("Send {handle} this invitation:"));
            lines.push(String::new());
            lines.extend(message);
            lines.push(String::new());
        }
        None => lines.push(format!(
            "Print the invitation to send them with: biorouter crew connections invitation --for {handle}"
        )),
    }
    lines.push(format!(
        "When {handle} sends you a code, let them in with: biorouter crew enroll approve {handle} CODE"
    ));
    lines
}

/// Why a snapshot lists no one waiting to join, when it has no `pending_joins` at all (DW-13).
/// The broker lists them only to the host, so a member is refused as every other `enroll`
/// command refuses them; a host's server that leaves them out cannot let people join by name.
/// Either way it is not an empty queue, which is what "No one is waiting to join." said.
fn no_pending_joins(snapshot: &Value) -> anyhow::Error {
    match host_standing(snapshot, None) {
        HostStanding::HostElsewhereToo { .. } | HostStanding::OnlyHostComputer { .. } => restated(
            "This workspace's server doesn't support joining by name.",
            Some("crew_join_by_name_unsupported"),
        ),
        HostStanding::NotHost | HostStanding::Unknown => restated(
            "Only the workspace host can see who is waiting to join.",
            Some("crew_host_required"),
        ),
    }
}

fn pending_lines(joins: &Value, now: i64) -> Vec<String> {
    let rows = joins.as_array().map(Vec::as_slice).unwrap_or_default();
    if rows.is_empty() {
        return vec!["No one is waiting to join.".into()];
    }
    let mut lines = vec![format!("Waiting to join ({}):", rows.len())];
    for join in rows {
        let username = safe_text(join["username"].as_str().unwrap_or("unknown"));
        let mut row = format!("  @{username}");
        if let Some(full_name) = join["full_name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
        {
            row.push_str(&format!(
                " · {} (name on the server account)",
                name_text(full_name)
            ));
        }
        if join["add_device"].as_bool() == Some(true) {
            row.push_str(" · another computer");
        }
        row.push_str(if join["approved"].as_bool() == Some(true) {
            " · code saved; joins when their computer confirms the same code"
        } else {
            " · waiting for their code"
        });
        let expires = join["expires_at"].as_i64();
        row.push_str(&match expires {
            _ if join["expired"].as_bool() == Some(true) => " · expired".to_owned(),
            Some(at) if at > now => format!(" · expires {}", time_left(at - now)),
            Some(_) => " · expired".to_owned(),
            None => String::new(),
        });
        lines.push(row);
        if join["mismatched_attempts"].as_u64().is_some_and(|n| n > 0) {
            lines.push(format!(
                "    A computer trying to join as @{username} showed a different code. Check the code @{username} sent you; if you typed it wrong, run enroll approve again with --replace. Don't approve a code you didn't get from @{username}."
            ));
        }
    }
    lines.push("Let someone in with: biorouter crew enroll approve @USERNAME CODE".into());
    lines
}

/// How `enroll revoke @bob` is confirmed.
#[derive(Debug, PartialEq, Eq)]
enum Confirmation {
    Confirmed,
    /// Ask this on the terminal; the answer must be `@username`.
    Ask(String),
}

/// A revoke by name is a decision about a person, so it is confirmed by typing their
/// `@username` again: on the terminal, or with `--confirm` when there is none to ask in
/// (a script's stdin is not one). A revoke by ID is for scripts and never asks.
fn revoke_confirmation(
    username: &str,
    label: &str,
    confirm: Option<&str>,
    interactive: bool,
) -> Result<Confirmation> {
    match confirm {
        Some(typed) => {
            ensure!(
                same_username(typed, username),
                "Not revoked: --confirm {} doesn't match @{username}.",
                typed.trim()
            );
            Ok(Confirmation::Confirmed)
        }
        None if interactive => Ok(Confirmation::Ask(format!(
            "Revoke {label}? Their membership, devices and agent grants stop working. Type @{} to confirm:",
            safe_text(username)
        ))),
        None => Err(needs_a_terminal(format!(
            "Revoking @{username} removes their membership, devices and agent grants. There is no terminal to ask in, so confirm with --confirm @{username}."
        ))),
    }
}

async fn revoke_member(api: &Api, member: &str, confirm: Option<&str>) -> Result<Reply> {
    let target = api.target(Kind::Person, member).await?;
    let asks = target.username.is_some() && confirm.is_none() && api.interactive;
    let label = if api.text() || asks {
        api.authority_label(&target).await
    } else {
        String::new()
    };
    let mut params = json!({"principal_id": target.id});
    if let Some(username) = &target.username {
        if let Confirmation::Ask(question) =
            revoke_confirmation(username, &label, confirm, api.interactive)?
        {
            let answer = ask(question).await?;
            ensure!(
                same_username(&answer, username),
                "Not revoked: that isn't @{username}."
            );
        }
        params["expected_username"] = json!(username);
    }
    let result = api.broker("enrollment.revoke", params, true).await?;
    Ok(api.say(
        result,
        vec![format!(
            "Revoked {label}. Their membership, devices and agent grants no longer work."
        )],
    ))
}

async fn read_secret(input: SecretInput) -> Result<Zeroizing<String>> {
    tokio::task::spawn_blocking(move || {
        let mut secret = if input.token_stdin {
            read_secret_line(std::io::stdin().lock())?
        } else if let Some(fd) = input.token_fd {
            ensure!(fd >= 0, "Secret file descriptor must be nonnegative");
            #[cfg(unix)]
            {
                read_secret_line(
                    std::fs::File::open(format!("/dev/fd/{fd}"))
                        .context("Could not read the supplied secret descriptor")?,
                )?
            }
            #[cfg(not(unix))]
            {
                return Err(anyhow!(
                    "Secret descriptors are unavailable on this platform; use --token-stdin"
                ));
            }
        } else {
            ensure!(
                std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
                "Enrollment requires a hidden terminal prompt, --token-stdin, or --token-fd"
            );
            eprintln!("Enrollment token (input hidden):");
            Zeroizing::new(console::Term::stderr().read_secure_line()?)
        };
        while secret.ends_with('\n') || secret.ends_with('\r') {
            secret.pop();
        }
        ensure!(
            !secret.is_empty() && secret.len() <= 8192,
            "Enrollment token must contain 1–8192 bytes"
        );
        Ok(secret)
    })
    .await
    .context("Enrollment token input failed")?
}

fn read_secret_line(mut source: impl Read) -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    let mut byte = [0];
    while source.read(&mut byte)? != 0 {
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
        ensure!(bytes.len() <= 8192, "Enrollment token exceeds 8192 bytes");
    }
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes)
            .context("Enrollment token must be UTF-8")?
            .to_string(),
    ))
}

async fn teams(api: &Api, command: TeamCommand) -> Result<Reply> {
    Ok(match command {
        TeamCommand::List => {
            let snapshot = api.snapshot().await?;
            let mut teams = snapshot_field(&snapshot, "teams")?;
            let listed = teams.as_array().map_or(0, Vec::len);
            if let Some(list) = teams.as_array_mut() {
                output::teams_in_order(list);
            }
            let note =
                output::partial_list_note("teams", listed, snapshot["totals"]["teams"].as_u64());
            Reply::Show(
                teams,
                Box::new(
                    api.human(Directory::from_snapshot(&snapshot))
                        .with_view(output::View::Teams)
                        .with_notes(note),
                ),
            )
        }
        // The result names the team as the broker stored it, which `output` prints.
        TeamCommand::Create { name } => api.show(
            api.broker("team.create", json!({"name":name}), true)
                .await?,
        ),
        TeamCommand::Rename { team, name } => {
            let team = api.target(Kind::Team, &team).await?;
            let result = api
                .broker(
                    "team.rename",
                    json!({"team_id": team.id, "name": name}),
                    true,
                )
                .await?;
            let stored = result["display_name"]
                .as_str()
                .or_else(|| result["name"].as_str())
                .unwrap_or(&name)
                .to_owned();
            api.say(
                result,
                vec![format!(
                    "Renamed {} to {}.",
                    api.label(&team, "the team", "team ID"),
                    name_text(&stored)
                )],
            )
        }
    })
}

/// `channels list [--team T]`: the person's channels in a reader's order, with their unread
/// counts, and a note when the snapshot left some out.
async fn list_channels(api: &Api, team: Option<&str>) -> Result<Reply> {
    let snapshot = api.snapshot().await?;
    let mut channels = snapshot_field(&snapshot, "channels")?;
    if let Some(team) = team {
        let team = api.target(Kind::Team, team).await?;
        channels = json!(channels
            .as_array()
            .context("Invalid channel list")?
            .iter()
            .filter(|item| item["team_id"].as_str() == Some(team.id.as_str()))
            .collect::<Vec<_>>());
    }
    let mut channels = with_unread(channels, &snapshot);
    if let Some(list) = channels.as_array_mut() {
        output::channels_in_order(
            list,
            snapshot["teams"].as_array().map_or(&[], Vec::as_slice),
        );
    }
    // A partial snapshot lists some of the person's channels (wave 1's bounded
    // snapshot): say so, rather than let the list read as all of them.
    let listed = snapshot["channels"].as_array().map_or(0, Vec::len);
    let note =
        output::partial_list_note("channels", listed, snapshot["totals"]["channels"].as_u64());
    Ok(Reply::Show(
        channels,
        Box::new(
            api.human(Directory::from_snapshot(&snapshot))
                .with_view(output::View::Channels)
                .with_notes(note),
        ),
    ))
}

async fn channels(api: &Api, command: ChannelCommand) -> Result<Reply> {
    Ok(match command {
        ChannelCommand::List { team } => list_channels(api, team.as_deref()).await?,
        ChannelCommand::Create {
            name,
            team,
            classification,
        } => {
            let team = api.target(Kind::Team, &team).await?;
            let result = api
                .broker(
                    "channel.create",
                    json!({"name":name,"team_id":team.id,"classification":classification.as_str()}),
                    true,
                )
                .await?;
            let stored = result["name"].as_str().unwrap_or(&name).to_owned();
            let mut line = api.with_id(
                format!(
                    "Created {} in {}.",
                    channel_text(&stored),
                    api.label(&team, "the team", "team ID")
                ),
                "channel ID",
                result["id"].as_str().unwrap_or_default(),
            );
            if stored != name.trim().trim_start_matches('#') {
                line.push_str(&format!(
                    " Channel names are lowercase with dashes, so “{}” was saved as {}.",
                    safe_text(name.trim()),
                    channel_text(&stored)
                ));
            }
            api.say(result, vec![line])
        }
        ChannelCommand::Archive { channel, yes } => {
            let channel = api.target(Kind::Channel, &channel).await?;
            let label = api.label(&channel, "the channel", "channel ID");
            // The broker has no way back: archiving closes the channel to posts for everyone.
            if let Consent::Ask(question) = yes_or_ask(
                yes,
                api.interactive,
                format!("Archive {label} for everyone? Nobody can post in it after this, and it can't be undone."),
                format!("Archiving {label} is permanent for everyone. There is no terminal to ask in, so add --yes."),
            )? {
                ask_yes(question, "Nothing was archived.").await?;
            }
            let result = api
                .broker("channel.archive", json!({"channel_id":channel.id}), true)
                .await?;
            let archived = result["name"].as_str().map_or_else(
                || api.label(&channel, "the channel", "channel ID"),
                channel_text,
            );
            api.say(result, vec![format!("Archived {archived}.")])
        }
        ChannelCommand::MarkRead { channel, cursor } => mark_read(api, &channel, cursor).await?,
        ChannelCommand::Rename { channel, name } => {
            let channel = api.target(Kind::Channel, &channel).await?;
            let result = api
                .broker(
                    "channel.rename",
                    json!({"channel_id":channel.id,"name":name}),
                    true,
                )
                .await?;
            let stored = result["name"].as_str().unwrap_or(&name).to_owned();
            api.say(
                result,
                vec![format!(
                    "Renamed {} to {}.",
                    api.label(&channel, "the channel", "channel ID"),
                    channel_text(&stored)
                )],
            )
        }
    })
}

/// `channels mark-read CHANNEL [CURSOR]`: up to `cursor`, or to the newest message.
async fn mark_read(api: &Api, channel: &str, cursor: Option<String>) -> Result<Reply> {
    let channel = api.your_channel(channel).await?;
    let label = api.label(&channel, "the channel", "channel ID");
    let cursor = match cursor {
        Some(cursor) => cursor,
        None => {
            let newest = api
                .broker(
                    "messages.history",
                    json!({"channel_id":channel.id,"latest":true,"limit":1}),
                    false,
                )
                .await?;
            match newest["cursor"].as_str() {
                Some(cursor) => cursor.to_owned(),
                None => {
                    return Ok(api.say(
                        json!({"channel_id":channel.id,"sequence":null}),
                        vec![format!("{label} has no messages to mark as read.")],
                    ))
                }
            }
        }
    };
    let result = api
        .broker(
            "channel.read",
            json!({"channel_id":channel.id,"sequence":cursor}),
            true,
        )
        .await?;
    Ok(api.say(result, vec![format!("Marked {label} as read.")]))
}

async fn invitations(api: &Api, command: InvitationCommand) -> Result<Reply> {
    Ok(match command {
        InvitationCommand::List => {
            let snapshot = api.snapshot().await?;
            let invitations = snapshot_field(&snapshot, "invitations")?;
            // CLIDOCS-F12: each list says its own empty sentence, never "No items.".
            Reply::Show(
                invitations,
                Box::new(
                    api.human(Directory::from_snapshot(&snapshot))
                        .with_view(output::View::Invitations),
                ),
            )
        }
        InvitationCommand::Create {
            person,
            team,
            channel,
        } => {
            let (kind, target_kind, target_text) = match (team, channel) {
                (Some(team), None) => ("team", Kind::Team, team),
                (None, Some(channel)) => ("channel", Kind::Channel, channel),
                _ => bail!("Choose exactly one --team or --channel"),
            };
            let targets = api
                .resolve(&[(Kind::Person, &person), (target_kind, &target_text)])
                .await?;
            let (who, target) = (&targets[0], &targets[1]);
            let mut params = json!({"kind":kind,"target_id":target.id,"principal_id":who.id});
            if let Some(username) = &who.username {
                params["expected_username"] = json!(username);
            }
            let result = api.broker("invitation.create", params, true).await?;
            let what = if kind == "team" {
                "team ID"
            } else {
                "channel ID"
            };
            let line = if api.text() {
                format!(
                    "Invited {} to {}. They accept with biorouter crew invites accept.",
                    api.authority_label(who).await,
                    api.label(target, &format!("the {kind}"), what)
                )
            } else {
                String::new()
            };
            api.say(result, vec![line])
        }
        InvitationCommand::Accept { invitation } => {
            accept_invitation(api, invitation.as_deref()).await?
        }
    })
}

/// `invites accept [NAME|ID]`. With no argument the only pending invitation is accepted; a name
/// picks the one for that team or channel among the caller's own invitations.
async fn accept_invitation(api: &Api, selector: Option<&str>) -> Result<Reply> {
    let selector = selector.map(str::trim).filter(|text| !text.is_empty());
    let (id, label) = match selector {
        Some(text) if uuid_shaped(text) => (component(text)?.to_owned(), None),
        _ => {
            let snapshot = api.snapshot().await?;
            let pending = pending_invitations(&snapshot, now());
            let chosen = choose_invitation(&pending, selector)?;
            let id = chosen["id"]
                .as_str()
                .context("The daemon listed an invitation without its ID")?;
            (component(id)?.to_owned(), Some(invitation_label(chosen)))
        }
    };
    let result = api
        .broker("invitation.accept", json!({"invitation_id":id}), true)
        .await?;
    let line = match label {
        Some(label) => api.with_id(format!("Joined {label}."), "invitation ID", &id),
        None => "Invitation accepted.".to_owned(),
    };
    Ok(api.say(result, vec![line]))
}

/// The caller's own invitations that have not expired.
fn pending_invitations(snapshot: &Value, now: i64) -> Vec<&Value> {
    let actor = snapshot["actor"]["id"].as_str();
    snapshot["invitations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|invitation| actor.is_some() && invitation["principal_id"].as_str() == actor)
        .filter(|invitation| {
            invitation["expired"].as_bool() != Some(true)
                && invitation["expires_at"]
                    .as_i64()
                    .is_none_or(|expires| expires > now)
        })
        .collect()
}

/// "Analysis Lab", or "#methods in Analysis Lab".
fn invitation_label(invitation: &Value) -> String {
    let target = invitation["target_name"]
        .as_str()
        .filter(|name| !name.is_empty());
    let team = invitation["team_name"]
        .as_str()
        .filter(|name| !name.is_empty());
    match (invitation["kind"].as_str(), target, team) {
        (Some("channel"), Some(channel), Some(team)) => {
            format!("{} in {}", channel_text(channel), name_text(team))
        }
        (Some("channel"), Some(channel), None) => channel_text(channel),
        (Some("channel"), None, _) => "a channel".to_owned(),
        (_, Some(team), _) => name_text(team),
        (_, None, _) => "a team".to_owned(),
    }
}

/// How to accept one invitation: `biorouter crew invites accept analysis-lab` (or
/// `analysis-lab/methods`). A workspace too old to name its invitations names nothing, and its
/// ID is a machine ID, so the person is pointed at `--show-ids` instead.
fn invitation_selector(invitation: &Value) -> String {
    let target = invitation["target_name"].as_str().map(loose_key);
    let team = invitation["team_name"].as_str().map(loose_key);
    match (invitation["kind"].as_str(), target, team) {
        (Some("channel"), Some(channel), Some(team)) => {
            format!("biorouter crew invites accept {team}/{channel}")
        }
        (_, Some(target), _) => format!("biorouter crew invites accept {target}"),
        _ => "its ID is in biorouter crew invites list --show-ids".to_owned(),
    }
}

/// A forgiving comparison key for the names on the caller's own invitations: lower case, a
/// leading `#` dropped, and every run of spaces or punctuation one `-`. It only chooses among
/// invitations already addressed to the caller; the broker authorizes the acceptance.
fn loose_key(text: &str) -> String {
    let lowered = text.trim().trim_start_matches('#').to_lowercase();
    let mut key = String::with_capacity(lowered.len());
    let mut gap = false;
    for c in lowered.chars() {
        if c.is_alphanumeric() {
            if gap && !key.is_empty() {
                key.push('-');
            }
            gap = false;
            key.push(c);
        } else {
            gap = true;
        }
    }
    key
}

fn invitation_matches(invitation: &Value, text: &str) -> bool {
    let target = invitation["target_name"].as_str().map(loose_key);
    let team = invitation["team_name"].as_str().map(loose_key);
    if invitation["id"].as_str() == Some(text) {
        return true;
    }
    match (invitation["kind"].as_str(), text.split_once('/')) {
        (Some("channel"), Some((team_part, channel_part))) => {
            team.as_deref() == Some(loose_key(team_part).as_str())
                && target.as_deref() == Some(loose_key(channel_part).as_str())
        }
        (_, Some(_)) => false,
        (_, None) => target.as_deref() == Some(loose_key(text).as_str()),
    }
}

fn choose_invitation<'a>(pending: &[&'a Value], selector: Option<&str>) -> Result<&'a Value> {
    let (matches, named): (Vec<&Value>, Option<&str>) = match selector {
        None => (pending.to_vec(), None),
        Some(text) => (
            pending
                .iter()
                .copied()
                .filter(|invitation| invitation_matches(invitation, text))
                .collect(),
            Some(text),
        ),
    };
    match (matches.as_slice(), named) {
        ([one], _) => Ok(*one),
        ([], None) => bail!("You have no pending invitations."),
        ([], Some(text)) => bail!("You have no pending invitation to “{text}”. Run biorouter crew invites list to see yours."),
        (many, _) => {
            let mut lines = vec![match named {
                None => format!(
                    "You have {} invitations. Name the one to accept:",
                    many.len()
                ),
                Some(text) => format!("“{text}” matches more than one of your invitations:"),
            }];
            lines.extend(many.iter().map(|invitation| {
                format!(
                    "  {} — {}",
                    invitation_label(invitation),
                    invitation_selector(invitation)
                )
            }));
            bail!("{}", lines.join("\n"))
        }
    }
}

async fn ownership(api: &Api, command: OwnershipCommand) -> Result<Reply> {
    Ok(match command {
        OwnershipCommand::Offer { channel, successor } => {
            let targets = api
                .resolve(&[(Kind::Channel, &channel), (Kind::Person, &successor)])
                .await?;
            let (channel, who) = (&targets[0], &targets[1]);
            let mut params = json!({"channel_id":channel.id,"successor_id":who.id});
            if let Some(username) = &who.username {
                params["expected_username"] = json!(username);
            }
            let result = api.broker("channel.transfer", params, true).await?;
            let line = if api.text() {
                format!(
                    "Offered {} to {}. When they accept, they own it and you leave the channel.",
                    api.label(channel, "the channel", "channel ID"),
                    api.authority_label(who).await
                )
            } else {
                String::new()
            };
            api.say(result, vec![line])
        }
        OwnershipCommand::Accept { channel } => {
            let channel = api.target(Kind::Channel, &channel).await?;
            let result = api
                .broker("transfer.accept", json!({"channel_id":channel.id}), true)
                .await?;
            api.say(
                result,
                vec![format!(
                    "You now own {}.",
                    api.label(&channel, "the channel", "channel ID")
                )],
            )
        }
    })
}

/// A channel selector confined to `team` when both are given by name: `methods` with `--team
/// Lab` is `Lab/methods`, so a same-named channel of another team is never chosen. A qualified
/// selector, or either one given as an ID, is kept as it is (the broker refuses a channel
/// outside the team anyway).
fn channel_in_team(team: Option<&str>, channel: &str) -> String {
    match team.map(str::trim) {
        Some(team)
            if !team.is_empty()
                && !team.contains('/')
                && !channel.contains('/')
                && Kind::Team.literal_id(team).is_none()
                && Kind::Channel.literal_id(channel).is_none() =>
        {
            format!("{team}/{}", channel.trim())
        }
        _ => channel.to_owned(),
    }
}

/// `#general`, `#general and #methods`, `#a, #b and #c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `members add @bob --team T [--channel C …]` and `members add @bob --channel C …`: direct add
/// (`team.add_member` and `channel.add_member`, the broker's `direct_add_v1`). The person
/// already joined the workspace, so nothing waits for them to accept; the broker checks that
/// the caller owns the team (or channel) or hosts the workspace, and that `@bob` is still the
/// member who was named.
async fn add_member(
    api: &Api,
    person: &str,
    team: Option<&str>,
    channels: &[String],
) -> Result<Reply> {
    ensure!(
        team.is_some() || !channels.is_empty(),
        "Choose where to add them: --team for a team you own, or --channel for a channel you own."
    );
    let mut selectors = vec![(Kind::Person, person.to_owned())];
    if let Some(team) = team {
        selectors.push((Kind::Team, team.to_owned()));
    }
    selectors.extend(
        channels
            .iter()
            .map(|channel| (Kind::Channel, channel_in_team(team, channel))),
    );
    let borrowed: Vec<(Kind, &str)> = selectors
        .iter()
        .map(|(kind, text)| (*kind, text.as_str()))
        .collect();
    let targets = api.resolve(&borrowed).await?;
    let (who, places) = targets
        .split_first()
        .context("A name was left unresolved")?;
    let username = member_username(api, who).await?;
    if team.is_none() {
        // One channel.add_member per channel cannot be taken back, so what the person's own
        // view already shows would be refused stops the command before the first one.
        if let Ok(snapshot) = api.snapshot().await {
            let problems = channel_add_problems(&snapshot, &who.id, &username, places);
            if !problems.is_empty() {
                return Err(restated(
                    format!("{}\nNothing was added.", problems.join("\n")),
                    Some("crew_request_refused"),
                ));
            }
        }
    }
    let (result, added) = match team {
        Some(_) => {
            let (team, channels) = places.split_first().context("A name was left unresolved")?;
            add_to_team(api, who, &username, team, channels).await?
        }
        None => add_to_channels(api, who, &username, places).await?,
    };
    let lines = if api.text() {
        let team = team.and_then(|_| places.first());
        added_lines(api, who, &username, team, &added, places).await
    } else {
        Vec::new()
    };
    Ok(api.say(result, lines))
}

/// The username a direct add confirms. An ID given as it is was never looked up, so it is read
/// from the workspace, never guessed.
async fn member_username(api: &Api, who: &Target) -> Result<String> {
    if let Some(username) = &who.username {
        return Ok(username.clone());
    }
    let snapshot = api.snapshot().await?;
    snapshot["principals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|principal| principal["id"].as_str() == Some(who.id.as_str()))
        .and_then(|principal| principal["username"].as_str())
        .map(str::to_owned)
        .context("Name the person as @username; that ID isn't a member of this workspace.")
}

/// A broker that predates direct add answers `unsupported`; say so, and point at invitations.
fn direct_add_unsupported(error: anyhow::Error, username: &str) -> anyhow::Error {
    if error_code(&error).as_deref() == Some("unsupported")
        || format!("{error:#}").contains("unsupported: operation is not supported")
    {
        error.context(format!(
            "This workspace's server can't add people directly yet. Invite them instead: biorouter crew invites create @{} --team <team>",
            safe_text(username)
        ))
    } else {
        error
    }
}

/// `team.add_member`: the broker's answer and the channels Bob was newly added to.
async fn add_to_team(
    api: &Api,
    who: &Target,
    username: &str,
    team: &Target,
    channels: &[Target],
) -> Result<(Value, Vec<String>)> {
    let channel_ids: Vec<&str> = channels.iter().map(|c| c.id.as_str()).collect();
    let result = api
        .broker(
            "team.add_member",
            json!({
                "team_id": team.id,
                "principal_id": who.id,
                "expected_username": username,
                "channel_ids": channel_ids,
            }),
            true,
        )
        .await
        .map_err(|error| direct_add_unsupported(error, username))?;
    let added = result["added_channels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Ok((result, added))
}

/// One `channel.add_member` per channel, each with its own idempotency key.
async fn add_to_channels(
    api: &Api,
    who: &Target,
    username: &str,
    channels: &[Target],
) -> Result<(Value, Vec<String>)> {
    let mut results = Vec::new();
    let mut added = Vec::new();
    for (step, channel) in channels.iter().enumerate() {
        let answer = api
            .broker_step(
                "channel.add_member",
                json!({
                    "channel_id": channel.id,
                    "principal_id": who.id,
                    "expected_username": username,
                }),
                step,
            )
            .await
            .map_err(|error| direct_add_unsupported(error, username));
        let result = match answer {
            Ok(result) => result,
            // Say what already changed. Whether a retry with --request-id is offered is the
            // failure path's to decide: only for an outcome that is uncertain, never for a
            // refusal, which the same command would meet again.
            Err(error) if !added.is_empty() => {
                let done: Vec<String> = channels
                    .iter()
                    .filter(|target| added.contains(&target.id))
                    .map(|target| api.label(target, "a channel", "channel ID"))
                    .collect();
                return Err(error.context(format!(
                    "Added @{} to {}, then stopped at {}",
                    safe_text(username),
                    and_list(&done),
                    api.label(channel, "the next channel", "channel ID")
                )));
            }
            Err(error) => return Err(error),
        };
        if result["already_member"].as_bool() == Some(false) {
            added.push(channel.id.clone());
        }
        results.push(result);
    }
    Ok((Value::Array(results), added))
}

/// What the person's snapshot already shows the broker would refuse for `channel.add_member`
/// (CLI-11): a channel they neither own nor host, an archived one, or one whose team the person
/// being added is not in. A channel the snapshot does not list (a host adding to a channel they
/// are not in) is left to the broker, which authorizes every add either way.
fn channel_add_problems(
    snapshot: &Value,
    principal_id: &str,
    username: &str,
    channels: &[Target],
) -> Vec<String> {
    let actor = snapshot["actor"]["id"].as_str();
    let host = matches!(
        host_standing(snapshot, None),
        HostStanding::HostElsewhereToo { .. } | HostStanding::OnlyHostComputer { .. }
    );
    let listed = |key: &str, id: &str| {
        snapshot[key]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["id"].as_str() == Some(id))
            .cloned()
    };
    let mut problems = Vec::new();
    for target in channels {
        let Some(channel) = listed("channels", &target.id) else {
            continue;
        };
        let name = channel["name"]
            .as_str()
            .map_or_else(|| "this channel".to_owned(), channel_text);
        if channel["archived"].as_bool() == Some(true) {
            problems.push(format!("{name} is archived, so no one can be added to it."));
        } else if !host
            && channel["owner_id"].as_str().is_some()
            && channel["owner_id"].as_str() != actor
        {
            problems.push(format!(
                "You don't own {name}. Only its owner or the workspace host can add people to it."
            ));
        } else if let Some(members) = channel["team_id"]
            .as_str()
            .and_then(|team| listed("teams", team))
            .and_then(|team| team["members"].as_array().cloned())
        {
            if !members
                .iter()
                .any(|member| member.as_str() == Some(principal_id))
            {
                problems.push(format!(
                    "@{} isn't in the team {name} belongs to yet. Add them to the team first.",
                    safe_text(username)
                ));
            }
        }
    }
    problems
}

/// What a direct add changed (M11), naming the person as every decision about a person does,
/// `"Carol Nguyen" (@crew_carol)`, and the places so they cannot be mistaken:
///
/// - into a team: `Added "Dan Ro" (@dan) to Analysis Lab. They can now see #general.`;
/// - into channels: `Added "Dan Ro" (@dan) to #methods.`, with the team in front of a channel
///   whose name another of the person's teams also has: `Chen Lab / #general`, since every team
///   has a `#general`.
///
/// Or that nothing needed adding.
async fn added_lines(
    api: &Api,
    who: &Target,
    username: &str,
    team: Option<&Target>,
    added: &[String],
    places: &[Target],
) -> Vec<String> {
    let names = api.names().await;
    let person = {
        let label = output::authority_label(&names, &who.id, api.show_ids);
        if label.starts_with("Unknown member") {
            api.with_id(format!("@{}", safe_text(username)), "ID", &who.id)
        } else {
            label
        }
    };
    if added.is_empty() {
        return vec![format!("{person} is already in everything you chose.")];
    }
    let place = |id: &String, across_teams: bool| {
        let known = if across_teams {
            output::channel_across_teams(&names, id)
        } else {
            Some(names.channel_label(id)).filter(|label| label.starts_with('#'))
        };
        known.unwrap_or_else(|| {
            places
                .iter()
                .find(|target| target.id == *id)
                .and_then(|target| target.label.as_deref())
                .map_or_else(|| "a channel".to_owned(), name_text)
        })
    };
    match team {
        // The channels a team add gave are all that team's, so it names them once.
        Some(team) => {
            let seen: Vec<String> = added.iter().map(|id| place(id, false)).collect();
            vec![format!(
                "Added {person} to {}. They can now see {}.",
                api.label(team, "the team", "team ID"),
                and_list(&seen)
            )]
        }
        None => {
            let seen: Vec<String> = added.iter().map(|id| place(id, true)).collect();
            vec![format!("Added {person} to {}.", and_list(&seen))]
        }
    }
}

async fn remove_member(
    api: &Api,
    channel: &str,
    member: &str,
    former: bool,
    yes: bool,
) -> Result<Reply> {
    let kind = if former {
        Kind::FormerPerson
    } else {
        Kind::Person
    };
    let targets = api
        .resolve(&[(Kind::Channel, channel), (kind, member)])
        .await?;
    let (channel, who) = (&targets[0], &targets[1]);
    let place = api.label(channel, "the channel", "channel ID");
    let handle = who.username.as_deref().map_or_else(
        || "this member".to_owned(),
        |username| format!("@{}", safe_text(username)),
    );
    let question = if !yes && api.interactive {
        format!(
            "Remove {} from {place}? They lose access to its messages and files.",
            api.authority_label(who).await
        )
    } else {
        String::new()
    };
    if let Consent::Ask(question) = yes_or_ask(
        yes,
        api.interactive,
        question,
        format!("Removing {handle} from {place} needs a yes. There is no terminal to ask in, so add --yes."),
    )? {
        ask_yes(question, "Nothing was changed.").await?;
    }
    let mut params = json!({"channel_id":channel.id,"principal_id":who.id});
    if let Some(username) = &who.username {
        params["expected_username"] = json!(username);
    }
    let result = api.broker("membership.revoke", params, true).await?;
    let line = if api.text() {
        format!(
            "Removed {} from {}.",
            api.authority_label(who).await,
            api.label(channel, "the channel", "channel ID")
        )
    } else {
        String::new()
    };
    Ok(api.say(result, vec![line]))
}

fn history_params(channel_id: &str, args: &HistoryArgs) -> Value {
    let mut params = json!({"channel_id":channel_id,"limit":args.limit,"latest":args.latest});
    if let Some(before) = &args.before {
        params["before"] = json!(before);
    }
    if let Some(after) = &args.after {
        params["after"] = json!(after);
    }
    params
}

fn text_input(input: TextInput) -> Result<String> {
    match (input.text, input.input) {
        (Some(text), None) => Ok(text),
        (None, Some(path)) => read_input(&path),
        _ => Err(anyhow!("Choose exactly one --text or --input")),
    }
}

/// The code of a message refused here for its size, before anything was sent.
const MESSAGE_TOO_LONG_CODE: &str = "crew_message_too_long";

/// A message the workspace would refuse for its size is refused here first (MSG2-N7), as a
/// usage error: the broker takes at most 64 KB of text, and at most twice that once
/// JSON-escaped (`message.post` in `broker.rs`).
fn check_message_size(body: &str) -> Result<()> {
    const MAX: usize = biorouter::crew::source_line::MAX_POSTED_BYTES;
    let escaped = serde_json::to_string(body)?.len().saturating_sub(2);
    if body.len() > MAX || escaped > 2 * MAX {
        return Err(usage(output::MESSAGE_TOO_LONG, MESSAGE_TOO_LONG_CODE));
    }
    Ok(())
}

async fn send_message(api: &Api, args: SendArgs) -> Result<Reply> {
    let body = if args.text.is_none() && args.input.is_none() {
        ensure!(
            !args.attachments.is_empty() || !args.references.is_empty(),
            "Choose --text, --input, --attachment, or --reference"
        );
        String::new()
    } else {
        text_input(TextInput {
            text: args.text,
            input: args.input,
        })?
    };
    check_message_size(&body)?;
    let channel = api.your_channel(&args.channel).await?;
    let result = api
        .broker(
            "message.post",
            json!({"channel_id":channel.id,"body":body,"attachments":args.attachments,"references":args.references}),
            true,
        )
        .await?;
    let line = api.with_id(
        format!(
            "Posted to {}.",
            api.label(&channel, "the channel", "channel ID")
        ),
        "message ID",
        result["id"].as_str().unwrap_or_default(),
    );
    Ok(api.say(result, vec![line]))
}

async fn watch(api: &Api, args: WatchArgs) -> Result<Reply> {
    let channel = api.your_channel(&args.channel).await?;
    let (mut cursor, initial) = watch_start(api, &channel.id, &args).await?;
    let path = api.path("/observe").await?;
    let client = api.client.shared()?;
    let state = std::sync::Mutex::new(WatchState::default());
    let watched = api.label(&channel, "the channel", "channel ID");
    loop {
        let request = ObserveRequest {
            channel_id: Some(channel.id.clone()),
            after: cursor.clone(),
            initial: match &initial {
                Initial::Latest => Initial::Latest,
                Initial::All => Initial::All,
            },
        };
        cursor = tokio::select! {
            result = client.observe(&path, &request, |event| watch_event(api, &state, &watched, event)) => result?,
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(Reply::Streamed); }
        };
    }
}

/// Where `crew watch` starts (CLI-6): the channel's newest page, as the desktop's channel
/// view does; with `--new-only`, after the newest message, so only new posts print; with
/// `--from-start`, the oldest message; with `--after`, that cursor. It used to replay the whole
/// channel from its oldest message before following it.
async fn watch_start(
    api: &Api,
    channel_id: &str,
    args: &WatchArgs,
) -> Result<(Option<String>, Initial)> {
    if let Some(after) = &args.after {
        return Ok((Some(after.clone()), Initial::All));
    }
    if args.from_start {
        return Ok((None, Initial::All));
    }
    if !args.new_only {
        return Ok((None, Initial::Latest));
    }
    let newest = api
        .broker(
            "messages.history",
            json!({"channel_id": channel_id, "latest": true, "limit": 1}),
            false,
        )
        .await?;
    // An empty channel has no cursor: everything posted to it from now on is new.
    Ok((newest["cursor"].as_str().map(str::to_owned), Initial::All))
}

/// What `crew watch` carries from frame to frame: the names the last state frame gave, and the
/// attachments already looked up.
#[derive(Default)]
struct WatchState {
    names: Directory,
    attachments: AttachmentNames,
}

async fn watch_event(
    api: &Api,
    state: &std::sync::Mutex<WatchState>,
    watched: &str,
    event: ObserveEvent,
) -> Result<std::ops::ControlFlow<Option<String>>> {
    let locked = || {
        state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    match event {
        // The frame's snapshot names the authors of the messages that follow.
        ObserveEvent::State { snapshot, .. } => {
            if api.text() {
                locked().names = Directory::from_snapshot(&snapshot);
            }
        }
        ObserveEvent::Messages { mut messages, .. } => {
            // The lookups wait on the daemon, so the state is not held across them.
            let mut attachments = std::mem::take(&mut locked().attachments);
            attachments.name_in(api, &mut messages).await;
            let names = {
                let mut state = locked();
                state.attachments = attachments;
                state.names.clone()
            };
            let options = api.human(names);
            for message in messages {
                emit_with(&message, output::stream_format(api.format), &options)?;
            }
        }
        ObserveEvent::Reconnect { cursor } => return Ok(std::ops::ControlFlow::Break(cursor)),
        // Printed once, by the failure path: a sentence in text, and in JSON one line that is
        // both the observer's error frame and the command's error.
        ObserveEvent::Error { code, error, clear } => {
            return Err(WatchStopped {
                message: watch_stopped(watched, &error),
                code: safe_text(&code),
                clear,
            }
            .into());
        }
    }
    Ok(std::ops::ControlFlow::Continue(()))
}

/// Each attachment's name, size and type, looked up with `blob.status` (DW-17), so history and
/// watch can say which file is which: a message carries only its attachments' IDs, and
/// `files download ID` needs the right one. Each ID is asked about once per command, and at most
/// [`Self::MOST`] are asked about in all, so a long page never waits long; an attachment past
/// that, or one the person cannot see, is shown by its ID alone.
#[derive(Debug, Default)]
struct AttachmentNames {
    known: std::collections::HashMap<String, Option<Value>>,
}

impl AttachmentNames {
    const MOST: usize = 40;

    /// [`Self::name_in`] for a history or search page's `messages`.
    async fn name_page(&mut self, api: &Api, page: &mut Value) {
        if let Some(messages) = page["messages"].as_array_mut() {
            self.name_in(api, messages).await;
        }
    }

    /// Add `attachment_details` ({ID: {name, size, media_type}}) beside each message's
    /// unchanged `attachments`, for every attachment that could be looked up.
    async fn name_in(&mut self, api: &Api, messages: &mut [Value]) {
        for message in messages.iter_mut() {
            let ids: Vec<String> = message["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let mut details = serde_json::Map::new();
            for id in ids {
                if let Some(found) = self.look_up(api, &id).await {
                    details.insert(id, found);
                }
            }
            if !details.is_empty() {
                message["attachment_details"] = Value::Object(details);
            }
        }
    }

    async fn look_up(&mut self, api: &Api, id: &str) -> Option<Value> {
        if let Some(known) = self.known.get(id) {
            return known.clone();
        }
        if self.known.len() >= Self::MOST || component(id).is_err() {
            return None;
        }
        let found = api
            .lookup("blob.status", json!({"blob_id": id}))
            .await
            .map(|blob| json!({"name": blob["name"], "size": blob["size"], "media_type": blob["media_type"]}))
            .filter(|details| details["name"].is_string());
        self.known.insert(id.to_owned(), found.clone());
        found
    }
}

/// Why `crew watch` stopped, for a person (Q2-76): the channel, then the observer's own plain
/// sentence. The code stays in the JSON error frame; the cursor is an internal, never named.
/// `watched` is the channel's label as [`Api::label`] made it, already safe and isolated.
fn watch_stopped(watched: &str, error: &str) -> String {
    let sentence = error.trim().trim_end_matches(['.', '!', '?']);
    if sentence.is_empty() {
        format!("Stopped watching {watched}.")
    } else {
        format!("Stopped watching {watched}: {}.", safe_text(sentence))
    }
}

async fn file_command(api: &Api, mut command: FileCommand) -> Result<Reply> {
    if let FileCommand::Upload { channel, .. } | FileCommand::Reference { channel, .. } =
        &mut command
    {
        *channel = api.your_channel(channel).await?.id;
    }
    let watching = matches!(command, FileCommand::Watch { .. });
    // CLIDOCS-F10: `forget` is said in the terminal's words; the daemon's "Receipt removed
    // after authorized partial cleanup…" is its internal account. JSON keeps the daemon's.
    let forgetting = match &command {
        FileCommand::Forget { file, .. } => Some(file.is_some()),
        _ => None,
    };
    let result = files::handle(api, command).await?;
    if let Some(partial_deleted) = forgetting {
        let sentence = if partial_deleted {
            "Removed from your list, and deleted the partial download. Files already shared stay in the channel."
        } else {
            "Removed from your list. Files already shared stay in the channel."
        };
        return Ok(api.say(result, vec![sentence.to_owned()]));
    }
    let names = api.names().await;
    Ok(if watching {
        // The summary sentence ("Transfer Saved."), not one more transfer row.
        let options = api.human(names).with_view(output::View::Result);
        Reply::Last(result, Box::new(options))
    } else {
        api.show_with(result, names)
    })
}

async fn tasks(api: &Api, command: TaskCommand) -> Result<Reply> {
    Ok(match command {
        TaskCommand::Start {
            channel,
            prompt,
            provider,
            model,
            context_channels,
            allow_posting,
        } => {
            ensure!(
                allow_posting,
                "Starting a Crew task requires --allow-posting for its destination channel"
            );
            let prompt = text_input(prompt)?;
            let mut selectors = vec![(Kind::Channel, channel.as_str())];
            selectors.extend(
                context_channels
                    .iter()
                    .map(|channel| (Kind::Channel, channel.as_str())),
            );
            let targets = api.resolve_yours(&selectors).await?;
            let (destination, context) = targets
                .split_first()
                .context("The task's channel was left unresolved")?;
            let context: Vec<&str> = context.iter().map(|target| target.id.as_str()).collect();
            let started = api.connection_action("runs", api.with_run_policy(json!({"request_id":api.request_id,"channel_id":destination.id,"prompt":prompt,"provider":provider,"model":model,"context_channels":context,"posting_grant":allow_posting}))).await;
            let started = started.map_err(|error| institution_refusal(error, &model))?;
            // The task is named by the channel it posts to, as `tasks list` names it.
            api.show_with(started, api.names().await)
        }
        TaskCommand::List => {
            let runs = api
                .client
                .request("GET", &api.path("/runs").await?, None)
                .await?;
            api.show_with(runs, api.names().await)
        }
        TaskCommand::Show { run } => {
            let run = task(api, &run).await?;
            api.show_with(run, api.names().await)
        }
        TaskCommand::Watch { run } => watch_task(api, &run).await?,
        // The route takes no body, so no request ID is sent and no `--request-id` retry is
        // offered: running `tasks cancel` again is the retry (CLI-20).
        TaskCommand::Cancel { run } => api.show(
            api.client
                .request(
                    "POST",
                    &api.path(&format!("/runs/{}/cancel", component(&run)?))
                        .await?,
                    None,
                )
                .await?,
        ),
    })
}

/// `tasks start`'s institution refusal ("…the model's resolved affiliation…") in the desktop's
/// words (Q2-76), keeping the daemon's code for JSON output. Anything else is left as it is.
fn institution_refusal(error: anyhow::Error, requested_model: &str) -> anyhow::Error {
    let found = error.chain().find_map(|cause| {
        if let Some(refused) = cause.downcast_ref::<DaemonRefusal>() {
            return Some((
                refused.message().to_owned(),
                refused.institution_refusal.clone(),
            ));
        }
        #[cfg(test)]
        if let Some(refused) = cause.downcast_ref::<tests::FakeRefusal>() {
            return Some((refused.message.clone(), refused.institution_refusal.clone()));
        }
        None
    });
    // A daemon that types the refusal (`crew_institution_mismatch`) is said by the failure path
    // from its details, keeping that code; this rewords only an older daemon's sentence.
    let typed =
        refusal(&error).and_then(|refused| refused.code).as_deref() == Some(INSTITUTION_MISMATCH);
    match found {
        Some((message, details)) if !typed && message.contains(AFFILIATION_REFUSAL) => restated(
            output::institution_refusal_text(requested_model, details.as_ref()),
            Some("crew_request_refused"),
        ),
        _ => error,
    }
}

/// The words that mark the daemon's institution refusal (`crew/institution.rs`).
const AFFILIATION_REFUSAL: &str = "the model's resolved affiliation";

async fn task(api: &Api, id: &str) -> Result<Value> {
    component(id)?;
    let result = api
        .client
        .request("GET", &api.path("/runs").await?, None)
        .await?;
    result["runs"]
        .as_array()
        .context("Daemon returned an invalid task list")?
        .iter()
        .find(|run| run["run_id"].as_str() == Some(id))
        .cloned()
        .context("Task was not found on this device and connection")
}

async fn watch_task(api: &Api, id: &str) -> Result<Reply> {
    let options = api.human(api.names().await);
    let mut previous = Value::Null;
    loop {
        let current = tokio::select! {
            result = task(api, id) => result?,
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(Reply::Streamed); }
        };
        if current != previous {
            emit_with(&current, output::stream_format(api.format), &options)?;
        }
        if matches!(
            current["status"].as_str(),
            Some(
                "completed"
                    | "failed"
                    | "cancelled"
                    | "interrupted"
                    | "outcome_not_durable"
                    | "cancellation_unconfirmed"
            )
        ) {
            return Ok(Reply::Streamed);
        }
        previous = current;
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {},
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(Reply::Streamed); }
        }
    }
}

/// `crew context SESSION` (AG-F7): what the help promises, the grant's scope first, then the
/// messages the chat can read, oldest first. The daemon's manifest names the channels the chat
/// reads but not the one it posts in, which the grants list has. JSON is the manifest as it is.
async fn context(api: &Api, session: &str) -> Result<Reply> {
    let manifest = match api.session_get(session, "context").await {
        Ok(manifest) => manifest,
        Err(error) => return Err(access_ended(api, error, session).await),
    };
    if !api.text() {
        return Ok(api.show(manifest));
    }
    let destination = grant_channel(api, session).await;
    let lines = output::context_lines(
        &manifest,
        destination.as_deref(),
        &api.human(api.names().await),
    );
    Ok(api.say(manifest, lines))
}

/// The channel `session`'s grant posts in, from the grants list, when it can be read.
async fn grant_channel(api: &Api, session: &str) -> Option<String> {
    let path = api.path("/grants").await.ok()?;
    let grants = api.client.request("GET", &path, None).await.ok()?;
    grants["grants"]
        .as_array()?
        .iter()
        .find(|grant| grant["session_id"].as_str() == Some(session))
        .and_then(|grant| grant["channel_id"].as_str())
        .map(str::to_owned)
}

/// The daemon's sentence for a grant that ended because Crew's settings changed, which a daemon
/// before T3-BE-7 sent as `crew_profile_refused`, a code it also gives unrelated refusals.
const LEGACY_SETTINGS_CHANGED: &str = "Crew settings changed since access was granted";

/// `context` refused because the chat's Crew access ended (SF2-N4): why, and the
/// `grants grant` that grants it again, with the chat and the channel its grant posted in (by
/// name when the workspace can be read, else by ID). The daemon says "Grant access again from
/// Crew", a desktop control. Any other refusal is left as it is; JSON keeps the daemon's code.
async fn access_ended(api: &Api, error: anyhow::Error, session: &str) -> anyhow::Error {
    let ended = error.chain().find_map(|cause| {
        let (code, message) = refusal_code_and_message(cause)?;
        match code {
            GRANT_ENDED => Some(refusal_field(cause, "reason") == Some("settings_changed")),
            "crew_profile_refused" if message.trim().starts_with(LEGACY_SETTINGS_CHANGED) => {
                Some(true)
            }
            _ => None,
        }
    });
    let Some(settings_changed) = ended else {
        return error;
    };
    let channel = match grant_channel(api, session).await {
        Some(id) => {
            let label = api.names().await.channel_label(&id);
            match label.strip_prefix('#') {
                Some(name) => shell_word(name),
                None => id,
            }
        }
        None => "CHANNEL".to_owned(),
    };
    Worded {
        sentence: grant_ended_sentence(
            settings_changed,
            Some((&shell_word(session), &safe_text(&channel))),
        ),
        source: error,
    }
    .into()
}

async fn grants(api: &Api, command: GrantCommand) -> Result<Reply> {
    Ok(match command {
        GrantCommand::List => {
            let mut grants = api
                .client
                .request("GET", &api.path("/grants").await?, None)
                .await?;
            with_task_statuses(api, &mut grants).await;
            api.show_with(grants, api.names().await)
        }
        GrantCommand::Grant {
            session,
            channel,
            context_channels,
        } => {
            component(&session)?;
            let mut selectors = vec![(Kind::Channel, channel.as_str())];
            selectors.extend(
                context_channels
                    .iter()
                    .map(|channel| (Kind::Channel, channel.as_str())),
            );
            let targets = api.resolve_yours(&selectors).await?;
            let (destination, context) = targets
                .split_first()
                .context("The grant's channel was left unresolved")?;
            let context: Vec<&str> = context.iter().map(|target| target.id.as_str()).collect();
            api.show(
                api.connection_action(
                    &format!("sessions/{session}/grant"),
                    api.with_run_policy(
                        json!({"channel_id":destination.id,"context_channels":context}),
                    ),
                )
                .await?,
            )
        }
        GrantCommand::Revoke { session } => revoke_grant(api, &session).await?,
    })
}

/// Each task's grant in `grants` with its task's status from the task list (`task_status`,
/// AGT2-N3), so a task the person stopped reads Stopped and one that finished reads Ended, as
/// the channel's card says it: the grant itself cannot tell them apart, since a finished task's
/// grant is revoked as a stopped one's is. Nothing is added when the task list cannot be read.
async fn with_task_statuses(api: &Api, grants: &mut Value) {
    let is_task = |grant: &Value| grant["kind"].as_str() == Some("task");
    let has_task = ["grants", "replaced_grants"]
        .iter()
        .filter_map(|key| grants[*key].as_array())
        .flatten()
        .any(is_task);
    if !has_task {
        return;
    }
    let Ok(path) = api.path("/runs").await else {
        return;
    };
    let Ok(runs) = api.client.request("GET", &path, None).await else {
        return;
    };
    let statuses: std::collections::HashMap<&str, &str> = runs["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| Some((run["run_id"].as_str()?, run["status"].as_str()?)))
        .collect();
    for key in ["grants", "replaced_grants"] {
        let Some(list) = grants[key].as_array_mut() else {
            continue;
        };
        for grant in list.iter_mut().filter(|grant| is_task(grant)) {
            let status = grant["run_id"]
                .as_str()
                .and_then(|run| statuses.get(run))
                .map(|status| (*status).to_owned());
            if let (Some(status), None) = (status, grant.get("task_status")) {
                grant["task_status"] = json!(status);
            }
        }
    }
}

/// `grants revoke`. Success is only the workspace's confirmation (`200` with `revoked: true`
/// and the revocation confirmed); a grant stopped here but not confirmed there (`503
/// crew_revocation_unconfirmed`, or a `2xx` saying so) is printed as that and exits non-zero,
/// as `tasks cancel` does.
async fn revoke_grant(api: &Api, session: &str) -> Result<Reply> {
    let path = api
        .path(&format!("/sessions/{}/revoke", component(session)?))
        .await?;
    let answer = api
        .client
        .request("POST", &path, None)
        .await
        .map_err(|error| match refusal(&error) {
            Some(Refusal {
                status: 503,
                code: Some(code),
            }) if code == "crew_revocation_unconfirmed" => {
                restated(STOPPED_ON_THIS_DEVICE, Some("crew_revocation_unconfirmed"))
            }
            _ => error,
        })?;
    let lines = revoked_lines(session, &answer)?;
    Ok(api.say(answer, lines))
}

fn revoked_lines(session: &str, answer: &Value) -> Result<Vec<String>> {
    let revoked = answer["revoked"].as_bool() == Some(true);
    let confirmed = answer["remote_revocation_confirmed"].as_bool() != Some(false);
    if revoked && !confirmed {
        return Err(restated(
            STOPPED_ON_THIS_DEVICE,
            Some("crew_revocation_unconfirmed"),
        ));
    }
    if !revoked {
        return Err(restated(
            "Not revoked. This chat can still read and post. Check biorouter crew grants list, then try again.",
            None,
        ));
    }
    let mut lines = vec![format!(
        "Access revoked. Chat {} can't use Crew until you grant access again, or start a new chat.",
        safe_text(session)
    )];
    match answer["task_status"].as_str() {
        Some("cancelled") => lines.push("Its task was stopped.".into()),
        Some(status) => lines.push(format!(
            "Its task's status: {}.",
            output::run_status_word(status)
        )),
        None => {}
    }
    Ok(lines)
}

/// `privacy set-personal MODE`: how this computer treats the workspace. Going public is
/// confirmed by typing the workspace's name (CLI-10), and in a workspace that is Private for
/// everyone it changes nothing until the host allows Public, which is said (SF-F1).
async fn set_personal(
    api: &Api,
    mode: PrivacyMode,
    institution_id: Option<String>,
    confirm: Option<&str>,
) -> Result<Reply> {
    let connection = api.connection().await?;
    // SF-F1: in a workspace that is Private for everyone, a public connection changes
    // nothing until the host allows Public, and the person is told so. Going Private
    // waits on nothing it does not need.
    let snapshot = match mode {
        PrivacyMode::Public => api.snapshot().await.ok(),
        PrivacyMode::Private => None,
    };
    let workspace_private = snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot["workspace"]["mode"] == "private");
    let workspace = snapshot
        .as_ref()
        .and_then(workspace_name_in)
        .unwrap_or_else(|| connection_name(&connection));
    let shown = name_text(&workspace);
    if matches!(mode, PrivacyMode::Public) && connection["mode"].as_str() != Some("public") {
        let effect = if workspace_private {
            format!("Make your {shown} connection public? Nothing changes while {shown} is Private for everyone; if the host allows Public, public models will be able to read public-safe work you can see there. Restricted content stays private.")
        } else {
            format!("Make your {shown} connection public? Public models will be able to read public-safe work you can see here. Restricted content stays private.")
        };
        confirm_public(
            api,
            &workspace,
            confirm,
            vec![effect],
            format!("Making your {shown} connection public needs its name typed. There is no terminal to ask in, so confirm with --confirm {}.", safe_text(&shell_word(&workspace))),
        )
        .await?;
    }
    let mut input = serde_json::Map::new();
    for key in [
        "name",
        "ssh_target",
        "port",
        "identity_file",
        "proxy_jump",
        "socket_path",
        "owner_uid",
        "workspace_id",
        "workspace_public_key",
        "remote_root",
        "remote_execution",
        "cluster_connection_id",
        "institution_id",
    ] {
        if let Some(value) = connection.get(key) {
            input.insert(key.into(), value.clone());
        }
    }
    input.insert("mode".into(), json!(mode.as_str()));
    if let Some(institution_id) = institution_id {
        input.insert("institution_id".into(), json!(institution_id));
    }
    let saved = api
        .client
        .request("PATCH", &api.path("").await?, Some(Value::Object(input)))
        .await?;
    let saved = with_privacy_of(saved, snapshot.as_ref());
    let note = (matches!(mode, PrivacyMode::Public) && workspace_private)
        .then(|| format!("Nothing changes while {shown} is Private for everyone."));
    Ok(Reply::Show(
        saved,
        Box::new(api.human(Directory::default()).with_notes(note)),
    ))
}

/// `privacy show` (SF2-N3, CLIDOCS-F5): the privacy in force (`effective_mode`, the rule
/// `status` uses), the connection's own setting, the workspace's, and each channel's
/// classification in the channels-list order. While disconnected the connection's own setting
/// is still printed, and the workspace's is said to be unreadable (`workspace` is `null`, and
/// `effective_mode` too unless the connection is Private, which is Private whatever the
/// workspace says).
async fn privacy_show(api: &Api) -> Result<Reply> {
    let connection = api.connection().await?;
    let snapshot = match api.snapshot().await {
        Ok(snapshot) => Some(snapshot),
        Err(error) if disconnected(&error) => None,
        Err(error) => return Err(error),
    };
    let workspace = snapshot
        .as_ref()
        .map_or(Value::Null, |snapshot| snapshot["workspace"].clone());
    let mut channels = snapshot
        .as_ref()
        .map_or(Value::Null, |snapshot| snapshot["channels"].clone());
    if let (Some(list), Some(snapshot)) = (channels.as_array_mut(), snapshot.as_ref()) {
        output::channels_in_order(
            list,
            snapshot["teams"].as_array().map_or(&[], Vec::as_slice),
        );
    }
    let effective = output::effective_mode(connection["mode"].as_str(), workspace["mode"].as_str());
    let value = json!({
        "connection_id": connection["id"],
        "personal_mode": connection["mode"],
        "institution_id": connection["institution_id"],
        "connection_policy_epoch": connection["policy_epoch"],
        "effective_mode": effective,
        "workspace": workspace,
        "channels": channels,
    });
    let directory = snapshot
        .as_ref()
        .map(Directory::from_snapshot)
        .unwrap_or_default();
    Ok(Reply::Show(
        value,
        Box::new(api.human(directory).with_view(output::View::Privacy)),
    ))
}

async fn privacy(api: &Api, command: PrivacyCommand) -> Result<Reply> {
    Ok(match command {
        PrivacyCommand::Show => privacy_show(api).await?,
        PrivacyCommand::SetPersonal {
            mode,
            institution_id,
            confirm,
        } => set_personal(api, mode, institution_id, confirm.as_deref()).await?,
        PrivacyCommand::SetWorkspace {
            mode,
            institution_id,
            confirm,
        } => {
            if matches!(mode, PrivacyMode::Public) {
                let connection = api.connection().await?;
                let snapshot = api.snapshot().await.ok();
                let already = snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot["workspace"]["mode"] == "public");
                if !already {
                    let workspace = snapshot
                        .as_ref()
                        .and_then(workspace_name_in)
                        .unwrap_or_else(|| connection_name(&connection));
                    let shown = name_text(&workspace);
                    confirm_public(
                        api,
                        &workspace,
                        confirm.as_deref(),
                        vec![format!("Allow Public in {shown}? Members will be able to choose Public. Agents with access will need permission again.")],
                        format!("Allowing Public in {shown} needs its name typed. There is no terminal to ask in, so confirm with --confirm {}.", safe_text(&shell_word(&workspace))),
                    )
                    .await?;
                }
            }
            let mut policy = json!({"mode":mode.as_str()});
            if let Some(institution_id) = institution_id {
                policy["institution_id"] = json!(institution_id);
            }
            api.show(api.broker("policy.set", policy, true).await?)
        }
    })
}

#[cfg(test)]
mod expected_mode_tests {
    use super::{add_expected_mode, add_personal_mode};
    use crate::commands::crew::args::PrivacyMode;
    use serde_json::json;

    #[test]
    fn expected_mode_is_added_only_when_requested() {
        assert_eq!(
            add_expected_mode(json!({"request_id":"r"}), None),
            json!({"request_id":"r"})
        );
        assert_eq!(
            add_expected_mode(json!({"request_id":"r"}), Some(PrivacyMode::Private)),
            json!({"request_id":"r","expected_mode":"private"})
        );
        assert_eq!(
            add_expected_mode(json!({"request_id":"r"}), Some(PrivacyMode::Public)),
            json!({"request_id":"r","expected_mode":"public"})
        );
    }

    #[test]
    fn personal_mode_is_guarded_to_message_and_blob_methods() {
        let mut message = json!({"body":"hello"});
        add_personal_mode("message.post", &mut message, Some(PrivacyMode::Private));
        assert_eq!(message["personal_mode"], "private");
        let mut blob = json!({"blob_id":"b"});
        add_personal_mode("blob.begin", &mut blob, Some(PrivacyMode::Public));
        assert_eq!(blob["personal_mode"], "public");
        let mut unrelated = json!({"body":"hello"});
        add_personal_mode(
            "workspace.snapshot",
            &mut unrelated,
            Some(PrivacyMode::Private),
        );
        assert!(unrelated.get("personal_mode").is_none());
    }
}

/// The command layer against a scripted daemon: which requests each command sends, and what it
/// prints or refuses. The daemon's own resolver is tested in `routes/crew/names_tests.rs`.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    type Handler = dyn Fn(&str, &str, Option<&Value>) -> Result<Value> + Send + Sync;

    /// A daemon that answers from a closure and records every request.
    pub(super) struct FakeDaemon {
        handler: Box<Handler>,
        requests: Mutex<Vec<Sent>>,
    }

    #[derive(Clone, Debug)]
    struct Sent {
        method: String,
        path: String,
        body: Option<Value>,
    }

    impl FakeDaemon {
        pub(super) fn answer(
            &self,
            method: &str,
            path: &str,
            body: Option<Value>,
        ) -> Result<Value> {
            let answer = (self.handler)(method, path, body.as_ref());
            self.requests.lock().expect("requests").push(Sent {
                method: method.to_owned(),
                path: path.to_owned(),
                body,
            });
            answer
        }

        fn sent(&self) -> Vec<Sent> {
            self.requests.lock().expect("requests").clone()
        }

        /// `(method, params)` of every broker request, in order.
        fn broker_calls(&self) -> Vec<(String, Value)> {
            self.sent()
                .into_iter()
                .filter(|sent| sent.path.ends_with("/request"))
                .filter_map(|sent| sent.body)
                .map(|body| {
                    (
                        body["method"].as_str().unwrap_or_default().to_owned(),
                        body["params"].clone(),
                    )
                })
                .collect()
        }

        fn broker_call(&self, method: &str) -> Option<Value> {
            self.broker_calls()
                .into_iter()
                .find(|(sent, _)| sent == method)
                .map(|(_, params)| params)
        }

        fn resolve_bodies(&self) -> Vec<Value> {
            self.sent()
                .into_iter()
                .filter(|sent| sent.path == "/crew/resolve")
                .filter_map(|sent| sent.body)
                .collect()
        }
    }

    /// A daemon refusal, as `daemon_client` reports one.
    #[derive(Debug)]
    pub(super) struct FakeRefusal {
        pub(super) status: u16,
        pub(super) code: Option<String>,
        pub(super) broker_code: Option<String>,
        pub(super) institution_refusal: Option<Value>,
        pub(super) connection_institution: Option<String>,
        pub(super) message: String,
        pub(super) detail: Option<String>,
        pub(super) modes: Option<(String, String)>,
        /// The refusal's other fields, as `DaemonRefusal::fields` keeps them.
        pub(super) fields: serde_json::Map<String, Value>,
    }

    impl std::fmt::Display for FakeRefusal {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                formatter,
                "Daemon returned {}: {}",
                self.status, self.message
            )
        }
    }

    impl std::error::Error for FakeRefusal {}

    fn refuse(status: u16, code: Option<&str>, message: &str) -> anyhow::Error {
        FakeRefusal {
            status,
            code: code.map(str::to_owned),
            broker_code: None,
            institution_refusal: None,
            connection_institution: None,
            message: message.to_owned(),
            detail: None,
            modes: None,
            fields: Default::default(),
        }
        .into()
    }

    /// A broker refusal as a daemon with `broker_code` forwards it: `crew_request_refused`, the
    /// broker's code, and the broker's own `code: sentence`.
    fn refuse_broker(broker_code: &str, message: &str) -> anyhow::Error {
        FakeRefusal {
            status: 400,
            code: Some("crew_request_refused".into()),
            broker_code: Some(broker_code.into()),
            institution_refusal: None,
            connection_institution: None,
            message: message.to_owned(),
            detail: None,
            modes: None,
            fields: Default::default(),
        }
        .into()
    }

    const CONNECTION: &str = "c0ffee00-0000-4000-8000-000000000001";
    const METHODS: &str = "c4a77e10-0000-4000-8000-000000000002";
    const GENERAL: &str = "c4a77e10-0000-4000-8000-000000000003";
    const TEAM: &str = "7ea30000-0000-4000-8000-000000000004";
    const ALICE: &str = "a11ce000-0000-4000-8000-000000000005";
    const BOB: &str = "b0b00000-0000-4000-8000-000000000006";
    const CAROL: &str = "ca401000-0000-4000-8000-000000000007";
    const SESSION: &str = "20260924_2";

    fn snapshot() -> Value {
        json!({
            "workspace": {"id": "w", "name": "lab", "host_uid": 1000, "mode": "private", "institution_id": "ucsf", "policy_epoch": 3},
            "actor": {"id": ALICE, "username": "alice", "display_name": "Alice Chen", "uid": 1000},
            "principals": [
                {"id": ALICE, "username": "alice", "display_name": "Alice Chen", "uid": 1000},
                {"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001}
            ],
            "former_principals": [
                {"id": CAROL, "username": "carol", "display_name": "Carol Diaz", "active": false}
            ],
            "teams": [{"id": TEAM, "name": "Analysis Lab", "general_channel_id": GENERAL}],
            "channels": [
                {"id": GENERAL, "team_id": TEAM, "name": "general", "classification": "restricted"},
                {"id": METHODS, "team_id": TEAM, "name": "methods", "classification": "restricted"}
            ],
            "invitations": []
        })
    }

    /// What the daemon's resolver answers for the names these tests use.
    fn resolution(kind: &str, text: &str) -> Value {
        let bare = text.trim_start_matches(['@', '#']);
        let resolved = |id: &str, label: &str, username: Option<&str>| json!({"status": "resolved", "kind": kind, "text": text, "id": id, "label": label, "username": username});
        match (kind, bare) {
            ("person", "bob") => resolved(BOB, "Bob Lee (@bob)", Some("bob")),
            ("former_person", "carol") => {
                resolved(CAROL, "Carol Diaz (@carol) · former member", Some("carol"))
            }
            ("person", "Bob") => {
                json!({"status": "unknown_name", "kind": kind, "text": text, "did_you_mean": "@bob"})
            }
            ("channel", "methods" | "analysis-lab/methods" | "Analysis Lab/#methods") => {
                resolved(METHODS, "#methods", None)
            }
            ("channel", "analysis-lab/general") => resolved(GENERAL, "#general", None),
            ("channel", "general") => json!({
                "status": "ambiguous_name", "kind": kind, "text": text,
                "candidates": ["Analysis Lab / #general", "Methods Team / #general"]
            }),
            ("team", "analysis-lab" | "Analysis Lab") => resolved(TEAM, "Analysis Lab", None),
            _ => json!({"status": "unknown_name", "kind": kind, "text": text}),
        }
    }

    fn resolve_answer(body: &Value) -> Value {
        let results: Vec<Value> = body["selectors"]
            .as_array()
            .expect("selectors")
            .iter()
            .map(|selector| {
                resolution(
                    selector["kind"].as_str().expect("kind"),
                    selector["text"].as_str().expect("text"),
                )
            })
            .collect();
        let connection = match body["connection"].as_str() {
            Some("UCSF HPC") => {
                json!({"status": "resolved", "kind": "connection", "text": "UCSF HPC", "id": CONNECTION, "label": "UCSF HPC"})
            }
            _ => Value::Null,
        };
        json!({"connection": connection, "results": results})
    }

    /// The daemon every test starts from: one saved connection, the resolver, the snapshot and a
    /// broker that echoes each mutation's parameters.
    fn standard(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        match (method, path) {
            ("GET", "/crew/connections") => Ok(json!({"connections": [{
                "id": CONNECTION, "name": "UCSF HPC", "ssh_target": "bob@hpc",
                "workspace_id": "w", "status": "connected", "public_key": "k"
            }]})),
            ("POST", "/crew/resolve") => Ok(resolve_answer(body.expect("resolve body"))),
            (_, path) if path.ends_with("/request") => {
                let body = body.expect("broker body");
                match body["method"].as_str() {
                    Some("workspace.snapshot") => Ok(snapshot()),
                    Some("messages.history") => {
                        Ok(json!({"messages": [], "cursor": "m-9", "people": {}}))
                    }
                    Some(_) => Ok(json!({"echo": body["params"]})),
                    None => bail!("no method"),
                }
            }
            _ => bail!("unexpected {method} {path}"),
        }
    }

    fn api_with(
        format: OutputFormat,
        handler: impl Fn(&str, &str, Option<&Value>) -> Result<Value> + Send + Sync + 'static,
    ) -> (Api, Arc<FakeDaemon>) {
        let fake = Arc::new(FakeDaemon {
            handler: Box::new(handler),
            requests: Mutex::default(),
        });
        let api = Api {
            client: Client {
                daemon: Daemon::Fake(Arc::clone(&fake)),
                request_id: "req-1".into(),
                sent: Arc::new(AtomicBool::new(false)),
            },
            selected: None,
            expected_mode: None,
            expected_policy_epoch: None,
            expected_workspace_policy_epoch: None,
            request_id: "req-1".into(),
            format,
            show_ids: false,
            interactive: false,
            poll: Duration::ZERO,
            connection_id: tokio::sync::OnceCell::new(),
            channel_labels: std::sync::Mutex::default(),
        };
        (api, fake)
    }

    fn said(reply: Reply) -> Vec<String> {
        match reply {
            Reply::Say(_, lines) => lines,
            Reply::Show(value, options) | Reply::Last(value, options) => {
                vec![output::render_text(&value, &options)]
            }
            Reply::Streamed => Vec::new(),
        }
    }

    fn history(channel: &str) -> CrewCommand {
        CrewCommand::History(HistoryArgs {
            channel: channel.into(),
            before: None,
            after: None,
            limit: 20,
            latest: true,
        })
    }

    fn message(error: &anyhow::Error) -> String {
        format!("{error:#}")
    }

    #[tokio::test]
    async fn uuid_input_never_goes_to_the_resolver() {
        let (api, fake) = api_with(OutputFormat::Json, standard);
        run(&api, history(&format!("#{METHODS}")))
            .await
            .expect("history by ID");
        run(
            &api,
            CrewCommand::RemoveMember {
                channel: METHODS.into(),
                member: format!("@{BOB}"),
                former: false,
                yes: true,
            },
        )
        .await
        .expect("remove-member by IDs");
        run(
            &api,
            CrewCommand::Enroll(EnrollmentCommand::Revoke {
                member: BOB.into(),
                confirm: None,
            }),
        )
        .await
        .expect("revoke by ID needs no confirmation");

        assert!(
            fake.resolve_bodies().is_empty(),
            "no resolver call for IDs: {:?}",
            fake.sent()
        );
        assert_eq!(
            fake.broker_call("messages.history").expect("history")["channel_id"],
            METHODS
        );
        assert_eq!(
            fake.broker_call("membership.revoke").expect("remove"),
            json!({"channel_id": METHODS, "principal_id": BOB, "idempotency_key": "req-1"})
        );
        let revoke = fake.broker_call("enrollment.revoke").expect("revoke");
        assert_eq!(revoke["principal_id"], BOB);
        assert!(revoke.get("expected_username").is_none());
    }

    #[tokio::test]
    async fn names_are_resolved_by_the_daemon_and_carry_the_username_confirmed() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let lines = said(
            run(
                &api,
                CrewCommand::RemoveMember {
                    channel: "#methods".into(),
                    member: "@bob".into(),
                    former: false,
                    yes: true,
                },
            )
            .await
            .expect("remove-member by name"),
        );
        assert_eq!(
            fake.resolve_bodies()[0],
            json!({"connection": CONNECTION, "selectors": [
                {"kind": "channel", "text": "#methods"},
                {"kind": "person", "text": "@bob"}
            ]}),
            "the # is sent as typed; the daemon strips it"
        );
        assert_eq!(
            fake.broker_call("membership.revoke").expect("remove"),
            json!({"channel_id": METHODS, "principal_id": BOB, "expected_username": "bob", "idempotency_key": "req-1"})
        );
        assert_eq!(lines, ["Removed \"Bob Lee\" (@bob) from #methods."]);
        assert!(
            !lines.iter().any(|line| line.contains(BOB)),
            "no ID by default"
        );

        run(
            &api,
            CrewCommand::Ownership(OwnershipCommand::Offer {
                channel: "methods".into(),
                successor: "bob".into(),
            }),
        )
        .await
        .expect("offer");
        assert_eq!(
            fake.broker_call("channel.transfer").expect("offer"),
            json!({"channel_id": METHODS, "successor_id": BOB, "expected_username": "bob", "idempotency_key": "req-1"})
        );

        for (team, channel, kind, target) in [
            (Some("analysis-lab"), None, "team", TEAM),
            (None, Some("analysis-lab/methods"), "channel", METHODS),
        ] {
            let (api, fake) = api_with(OutputFormat::Text, standard);
            let lines = said(
                run(
                    &api,
                    CrewCommand::Invites(InvitationCommand::Create {
                        person: "@bob".into(),
                        team: team.map(str::to_owned),
                        channel: channel.map(str::to_owned),
                    }),
                )
                .await
                .expect("invite by name"),
            );
            assert_eq!(
                fake.broker_call("invitation.create").expect("invite"),
                json!({"kind": kind, "target_id": target, "principal_id": BOB, "expected_username": "bob", "idempotency_key": "req-1"})
            );
            assert!(lines[0].starts_with("Invited \"Bob Lee\" (@bob) to "));
        }

        let (api, fake) = api_with(OutputFormat::Json, standard);
        run(
            &api,
            CrewCommand::RemoveMember {
                channel: "methods".into(),
                member: "@carol".into(),
                former: true,
                yes: true,
            },
        )
        .await
        .expect("a former member by name");
        assert_eq!(
            fake.resolve_bodies()[0]["selectors"][1],
            json!({"kind": "former_person", "text": "@carol"})
        );
        assert_eq!(
            fake.broker_call("membership.revoke").expect("remove")["expected_username"],
            "carol"
        );
    }

    #[tokio::test]
    async fn an_ambiguous_name_prints_its_candidates_and_changes_nothing() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(
            &api,
            CrewCommand::Channels(ChannelCommand::Archive {
                channel: "general".into(),
                yes: true,
            }),
        )
        .await
        .expect_err("ambiguous names are refused");
        assert_eq!(
            message(&error),
            "#general matches more than one channel:\n  Analysis Lab / #general\n  Methods Team / #general\nName its team too, like analysis-lab/methods."
        );
        assert!(fake.broker_calls().is_empty(), "nothing was archived");

        // The candidates stay one per line on the way out, with no request ID: nothing was sent.
        let shown = failure(&error, OutputFormat::Text, "req-1", false).to_string();
        assert!(shown.contains("\n  Analysis Lab / #general\n  Methods Team / #general\n"));
        assert!(!shown.contains("req-1"));
        assert!(!api.client.sent.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn every_unresolved_name_is_reported_and_a_case_slip_is_never_resolved() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(
            &api,
            CrewCommand::RemoveMember {
                channel: "raw-data".into(),
                member: "@Bob".into(),
                former: false,
                yes: true,
            },
        )
        .await
        .expect_err("unknown names are refused");
        assert_eq!(
            message(&error),
            "No channel you're in is called #raw-data.\nThere's no member @Bob in this workspace. Did you mean @bob? Usernames must match exactly."
        );
        assert!(fake.broker_calls().is_empty());
        // DW-10: the JSON error carries the resolver's code, as a daemon refusal does.
        assert_eq!(error_code(&error).as_deref(), Some("unknown_name"));
        assert_eq!(
            failure_body(&error, &message(&error), "req-1")["code"],
            "unknown_name"
        );
        let (api, _) = api_with(OutputFormat::Json, standard);
        let error = run(&api, history("general"))
            .await
            .expect_err("two teams have a #general");
        assert_eq!(error_code(&error).as_deref(), Some("ambiguous_name"));
        assert!(!failure(&error, OutputFormat::Json, "req-1", false)
            .to_string()
            .contains("req-1"));
    }

    /// AG-F13: a channel ID from another workspace, or of a channel the person left, is refused
    /// before the daemon is asked, as a name would be. A snapshot that leaves channels out
    /// cannot prove an ID absent, so it leaves the ID to the broker, and so do commands a host
    /// may run on a channel they are not in.
    #[tokio::test]
    async fn a_channel_id_you_are_not_in_is_refused_like_a_name() {
        const FOREIGN: &str = "f0e1d2c3-0000-4000-8000-00000000000f";
        let grant = |context: &str| {
            CrewCommand::Grants(GrantCommand::Grant {
                session: SESSION.into(),
                channel: "methods".into(),
                context_channels: vec![context.to_owned()],
            })
        };
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(&api, grant(FOREIGN))
            .await
            .expect_err("not one of yours");
        assert_eq!(
            message(&error),
            format!("No channel you're in has the ID {FOREIGN}.")
        );
        assert_eq!(error_code(&error).as_deref(), Some("unknown_name"));
        assert!(
            !fake.sent().iter().any(|sent| sent.path.ends_with("/grant")),
            "nothing was asked of the daemon"
        );

        // An ID of one of their channels goes through.
        let granting = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/grant") {
                return Ok(json!({"session_id": SESSION, "run_id": "r"}));
            }
            standard(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Text, granting);
        run(&api, grant(GENERAL)).await.expect("one of theirs");
        assert!(fake.sent().iter().any(|sent| sent.path.ends_with("/grant")));

        // A snapshot that leaves channels out cannot prove the ID absent.
        let partial = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["totals"] = json!({"channels": 3});
                return Ok(snapshot);
            }
            granting(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Text, partial);
        run(&api, grant(FOREIGN)).await.expect("left to the broker");
        assert!(fake.sent().iter().any(|sent| sent.path.ends_with("/grant")));

        // A host may archive a channel they are not in, by its ID.
        let (api, fake) = api_with(OutputFormat::Text, standard);
        run(
            &api,
            CrewCommand::Channels(ChannelCommand::Archive {
                channel: FOREIGN.into(),
                yes: true,
            }),
        )
        .await
        .expect("left to the broker");
        assert!(fake.broker_call("channel.archive").is_some());
    }

    #[tokio::test]
    async fn a_daemon_without_the_resolver_is_told_to_restart_and_ids_still_work() {
        let old = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/resolve" {
                return Err(refuse(404, None, "Request refused"));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, old);
        let error = run(&api, history("methods"))
            .await
            .expect_err("names need the resolver");
        assert_eq!(message(&error), RESTART_FOR_NAMES);
        run(&api, history(METHODS))
            .await
            .expect("an ID needs no resolver");

        let error = select_connection(&api.client, "UCSF HPC")
            .await
            .expect_err("a connection name needs the resolver");
        assert_eq!(message(&error), RESTART_FOR_NAMES);
        assert_eq!(
            select_connection(&api.client, CONNECTION)
                .await
                .expect("a connection ID needs no resolver"),
            CONNECTION
        );
    }

    #[tokio::test]
    async fn a_connection_name_is_resolved_by_the_daemon() {
        let (api, fake) = api_with(OutputFormat::Json, standard);
        assert_eq!(
            select_connection(&api.client, " UCSF HPC ")
                .await
                .expect("a saved connection's name"),
            CONNECTION
        );
        assert_eq!(
            fake.resolve_bodies(),
            [json!({"connection": "UCSF HPC", "selectors": []})]
        );
        let error = select_connection(&api.client, "Elsewhere")
            .await
            .expect_err("an unknown connection");
        assert!(message(&error).contains("No saved Crew connection is named “Elsewhere”"));
    }

    #[tokio::test]
    async fn a_crew_refusal_is_never_mistaken_for_an_old_daemon() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/invitation") {
                return Err(refuse(
                    404,
                    Some("crew_connection_not_found"),
                    "This computer has no saved Crew connection with that ID.",
                ));
            }
            if path == "/crew/connections/from-invitation" {
                return Err(refuse(405, None, "Request refused"));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let error = run(
            &api,
            CrewCommand::Connections(ConnectionCommand::Invitation { invitee: None }),
        )
        .await
        .expect_err("refused");
        assert_eq!(
            message(&error),
            "Daemon returned 404: This computer has no saved Crew connection with that ID."
        );

        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), "brcrew1:abc").expect("write invitation");
        let error = run(
            &api,
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(join_args(
                file.path(),
            )))),
        )
        .await
        .expect_err("an old daemon");
        assert_eq!(message(&error), RESTART_FOR_JOINING);
    }

    fn revoke_with(answer: fn() -> Result<Value>) -> (Api, Arc<FakeDaemon>) {
        api_with(
            OutputFormat::Text,
            move |method: &str, path: &str, body: Option<&Value>| {
                if path.ends_with(&format!("/sessions/{SESSION}/revoke")) {
                    return answer();
                }
                standard(method, path, body)
            },
        )
    }

    fn revoke_command() -> CrewCommand {
        CrewCommand::Grants(GrantCommand::Revoke {
            session: SESSION.into(),
        })
    }

    #[tokio::test]
    async fn an_unconfirmed_revoke_says_so_and_exits_non_zero() {
        let (api, fake) = revoke_with(|| {
            Err(refuse(
                503,
                Some("crew_revocation_unconfirmed"),
                "Stopped on this device. The workspace has not confirmed yet; reconnect and retry.",
            ))
        });
        let error = run(&api, revoke_command())
            .await
            .expect_err("a 503 is not success");
        assert!(message(&error).starts_with("Stopped on this device."));
        assert_eq!(
            error_code(&error).as_deref(),
            Some("crew_revocation_unconfirmed")
        );
        let revoke = fake
            .sent()
            .into_iter()
            .find(|sent| sent.path.ends_with("/revoke"))
            .expect("revoke sent");
        assert_eq!(
            (revoke.method.as_str(), revoke.body),
            ("POST", None),
            "the route takes no body"
        );
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert!(
            !shown.contains("req-1"),
            "the retry is the same command: {shown}"
        );

        // A 2xx is success only when it says the workspace confirmed.
        let (api, _) = revoke_with(|| {
            Ok(
                json!({"revoked": true, "session_id": SESSION, "remote_revocation_confirmed": false}),
            )
        });
        let error = run(&api, revoke_command()).await.expect_err("unconfirmed");
        assert!(message(&error).starts_with("Stopped on this device."));

        let (api, _) = revoke_with(|| Ok(json!({})));
        let error = run(&api, revoke_command()).await.expect_err("not revoked");
        assert!(message(&error).starts_with("Not revoked. This chat can still read and post."));

        let (api, _) = revoke_with(|| {
            Err(refuse(
                404,
                Some("crew_grant_not_found"),
                "No Crew grant for this session.",
            ))
        });
        let error = run(&api, revoke_command()).await.expect_err("no grant");
        assert_eq!(
            message(&error),
            "Daemon returned 404: No Crew grant for this session."
        );
    }

    #[tokio::test]
    async fn a_confirmed_revoke_succeeds() {
        let (api, _) = revoke_with(|| {
            Ok(
                json!({"revoked": true, "session_id": SESSION, "run_id": "r", "remote_revocation_confirmed": true, "run": {}, "task_status": "cancelled"}),
            )
        });
        assert_eq!(
            said(run(&api, revoke_command()).await.expect("revoked")),
            [
                "Access revoked. Chat 20260924_2 can't use Crew until you grant access again, or start a new chat.",
                "Its task was stopped."
            ]
        );
    }

    #[tokio::test]
    async fn revoking_by_name_needs_the_username_again_without_a_terminal() {
        let revoke = |confirm: Option<&str>| {
            CrewCommand::Enroll(EnrollmentCommand::Revoke {
                member: "@bob".into(),
                confirm: confirm.map(str::to_owned),
            })
        };
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(&api, revoke(None))
            .await
            .expect_err("no terminal, no --confirm");
        assert!(message(&error).contains("confirm with --confirm @bob"));
        let error = run(&api, revoke(Some("@carol")))
            .await
            .expect_err("the wrong person");
        assert_eq!(
            message(&error),
            "Not revoked: --confirm @carol doesn't match @bob."
        );
        assert!(fake.broker_call("enrollment.revoke").is_none());

        let lines = said(
            run(&api, revoke(Some("bob")))
                .await
                .expect("confirmed without the @"),
        );
        assert_eq!(
            fake.broker_call("enrollment.revoke").expect("revoked"),
            json!({"principal_id": BOB, "expected_username": "bob", "idempotency_key": "req-1"})
        );
        assert_eq!(
            lines,
            ["Revoked \"Bob Lee\" (@bob). Their membership, devices and agent grants no longer work."]
        );

        assert_eq!(
            revoke_confirmation("bob", "Bob Lee (@bob)", None, true).expect("asks"),
            Confirmation::Ask(
                "Revoke Bob Lee (@bob)? Their membership, devices and agent grants stop working. Type @bob to confirm:".into()
            )
        );
        assert_eq!(
            revoke_confirmation("bob", "x", Some(" @bob "), false).expect("confirmed"),
            Confirmation::Confirmed
        );
    }

    #[tokio::test]
    async fn mark_read_without_a_cursor_uses_the_newest_message() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let lines = said(
            run(
                &api,
                CrewCommand::Channels(ChannelCommand::MarkRead {
                    channel: "#methods".into(),
                    cursor: None,
                }),
            )
            .await
            .expect("mark-read"),
        );
        assert_eq!(
            fake.broker_call("messages.history").expect("newest"),
            json!({"channel_id": METHODS, "latest": true, "limit": 1})
        );
        assert_eq!(
            fake.broker_call("channel.read").expect("read"),
            json!({"channel_id": METHODS, "sequence": "m-9", "idempotency_key": "req-1"})
        );
        assert_eq!(lines, ["Marked #methods as read."]);
    }

    /// MSG2-N5, CLIDOCS-F1: the resolver labels an archived channel `#archive-me · archived`
    /// for lists; a sentence names it bare, never repeating its status, and never isolated
    /// because of the separator: in text and in JSON's `error`.
    #[tokio::test]
    async fn an_archived_channel_is_named_bare_in_sentences() {
        const ARCHIVED: &str = "c4a77e10-0000-4000-8000-00000000000a";
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/resolve" {
                let body = body.expect("resolve body");
                let results: Vec<Value> = body["selectors"]
                    .as_array()
                    .expect("selectors")
                    .iter()
                    .map(|selector| json!({"status": "resolved", "kind": "channel", "text": selector["text"], "id": ARCHIVED, "label": "#archive-me · archived"}))
                    .collect();
                return Ok(json!({"connection": null, "results": results}));
            }
            match body.and_then(|body| body["method"].as_str()) {
                Some("workspace.snapshot") => {
                    let mut snapshot = snapshot();
                    snapshot["channels"].as_array_mut().expect("channels").push(json!({"id": ARCHIVED, "team_id": TEAM, "name": "archive-me", "classification": "restricted", "archived": true}));
                    Ok(snapshot)
                }
                Some("message.post" | "channel.rename") => Err(refuse_broker(
                    "channel_archived",
                    "channel_archived: channel is read-only",
                )),
                _ => standard(method, path, body),
            }
        };
        let send = CrewCommand::Send(SendArgs {
            channel: "archive-me".into(),
            text: Some("hi".into()),
            input: None,
            attachments: Vec::new(),
            references: Vec::new(),
        });
        let rename = CrewCommand::Channels(ChannelCommand::Rename {
            channel: "archive-me".into(),
            name: "renamed".into(),
        });
        for (name, command) in [("send", send), ("rename", rename)] {
            let (api, _) = api_with(OutputFormat::Json, handler);
            let error = run(&api, command).await.expect_err(name);
            let text = safe_lines(&error_text(&error));
            assert_eq!(
                text, "#archive-me is archived, so it's read-only.",
                "{name}"
            );
            assert!(!text.contains('\u{2068}'), "{name}: {text}");
            let body = failure_body(&error, &text, "req-1");
            assert_eq!(body["error"], "#archive-me is archived, so it's read-only.");
        }
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                CrewCommand::Channels(ChannelCommand::MarkRead {
                    channel: "archive-me".into(),
                    cursor: None,
                }),
            )
            .await
            .expect("mark-read"),
        );
        assert_eq!(lines, ["Marked #archive-me as read."]);
    }

    #[tokio::test]
    async fn create_and_rename_echo_the_name_the_workspace_stored() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("channel.create") => Ok(
                    json!({"id": "c-new", "team_id": TEAM, "name": "data-analysis", "classification": "restricted"}),
                ),
                Some("team.rename") => {
                    Ok(json!({"id": TEAM, "name": "Analysis Lab 2", "general_channel_id": GENERAL}))
                }
                Some("workspace.rename") => Ok(json!({"id": "w", "name": "lab2"})),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                CrewCommand::Channels(ChannelCommand::Create {
                    name: "Data Analysis".into(),
                    team: "analysis-lab".into(),
                    classification: Classification::Restricted,
                }),
            )
            .await
            .expect("create"),
        );
        assert_eq!(
            fake.broker_call("channel.create").expect("create")["team_id"],
            TEAM
        );
        assert_eq!(
            lines,
            ["Created #data-analysis in Analysis Lab. Channel names are lowercase with dashes, so “Data Analysis” was saved as #data-analysis."]
        );
        let lines = said(
            run(
                &api,
                CrewCommand::Teams(TeamCommand::Rename {
                    team: "Analysis Lab".into(),
                    name: "Analysis Lab 2".into(),
                }),
            )
            .await
            .expect("rename team"),
        );
        assert_eq!(lines, ["Renamed Analysis Lab to Analysis Lab 2."]);
        let lines = said(
            run(
                &api,
                CrewCommand::Workspace(WorkspaceCommand::Rename {
                    name: "lab2".into(),
                }),
            )
            .await
            .expect("rename workspace"),
        );
        assert_eq!(lines, ["Renamed the workspace to lab2."]);
    }

    fn invitation(id: &str, kind: &str, target: &str, team: Option<&str>, expired: bool) -> Value {
        json!({
            "id": id, "kind": kind, "target_id": "t", "principal_id": ALICE, "inviter_id": BOB,
            "expires_at": 4_000_000_000_i64, "expired": expired,
            "target_name": target, "team_name": team,
            "inviter": {"username": "bob", "display_name": "Bob Lee"}
        })
    }

    fn with_invitations(
        invitations: Value,
    ) -> impl Fn(&str, &str, Option<&Value>) -> Result<Value> {
        move |method: &str, path: &str, body: Option<&Value>| {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["invitations"] = invitations.clone();
                return Ok(snapshot);
            }
            standard(method, path, body)
        }
    }

    #[tokio::test]
    async fn accepting_needs_no_argument_when_one_invitation_is_pending() {
        let accept = |invitation: Option<&str>| {
            CrewCommand::Invites(InvitationCommand::Accept {
                invitation: invitation.map(str::to_owned),
            })
        };
        let pending = json!([
            invitation("i-1", "team", "Analysis Lab", None, false),
            invitation("i-old", "team", "Old Lab", None, true),
            // Someone else's invitation, which the host's snapshot also lists.
            {"id": "i-other", "kind": "team", "target_id": "t", "principal_id": BOB, "inviter_id": ALICE, "expires_at": 4_000_000_000_i64}
        ]);
        let (api, fake) = api_with(OutputFormat::Text, with_invitations(pending));
        let lines = said(run(&api, accept(None)).await.expect("the only one"));
        assert_eq!(
            fake.broker_call("invitation.accept").expect("accepted"),
            json!({"invitation_id": "i-1", "idempotency_key": "req-1"})
        );
        assert_eq!(lines, ["Joined Analysis Lab."]);

        let two = json!([
            invitation("i-1", "team", "Analysis Lab", None, false),
            invitation("i-2", "channel", "methods", Some("Analysis Lab"), false)
        ]);
        let (api, fake) = api_with(OutputFormat::Text, with_invitations(two.clone()));
        let error = run(&api, accept(None)).await.expect_err("which one?");
        assert_eq!(
            message(&error),
            "You have 2 invitations. Name the one to accept:\n  Analysis Lab — biorouter crew invites accept analysis-lab\n  #methods in Analysis Lab — biorouter crew invites accept analysis-lab/methods"
        );
        assert!(fake.broker_call("invitation.accept").is_none());

        let (api, fake) = api_with(OutputFormat::Text, with_invitations(two));
        run(&api, accept(Some("analysis-lab/#methods")))
            .await
            .expect("by name");
        assert_eq!(
            fake.broker_call("invitation.accept").expect("accepted")["invitation_id"],
            "i-2"
        );

        let (api, _) = api_with(OutputFormat::Text, with_invitations(json!([])));
        let error = run(&api, accept(None)).await.expect_err("none");
        assert_eq!(message(&error), "You have no pending invitations.");
    }

    fn join_args(path: &Path) -> JoinInvitationArgs {
        JoinInvitationArgs {
            input: path.to_path_buf(),
            preview: false,
            yes: true,
            username: Some("@bob".into()),
            mode: None,
            institution_id: None,
            name: None,
            ssh_target: None,
            port: None,
            identity_file: None,
            proxy_jump: None,
            remote_root: None,
            remote_execution: false,
            preparation_id: None,
            replace: false,
        }
    }

    fn preview(missing: &[&str]) -> Value {
        json!({"preview": {
            "source": "invitation", "workspace_id": "w", "workspace_label": "lab",
            "workspace_name": "lab", "host_username": "alice", "host_display_name": "Alice Chen",
            "workspace_mode": "private", "workspace_institution_id": "ucsf",
            "server": "hpc.ucsf.edu", "fingerprint": "3F2A 9C1E 77B0 D4E1", "username": "bob",
            "mode": "private", "institution_id": "ucsf", "mode_differs": false, "name": "lab",
            "missing": missing
        }})
    }

    #[tokio::test]
    async fn joining_by_invitation_previews_then_saves_with_the_choices_made() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/connections/from-invitation" {
                return Ok(if body.expect("body")["preview"] == true {
                    preview(&[])
                } else {
                    json!({"connection": {"id": CONNECTION, "name": "lab"}})
                });
            }
            standard(method, path, body)
        };
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(
            file.path(),
            "Join lab on Crew.\nIn Biorouter, open Crew …\nbrcrew1:abc\n",
        )
        .expect("write invitation");
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(join_args(
                    file.path(),
                )))),
            )
            .await
            .expect("saved"),
        );
        let bodies: Vec<Value> = fake
            .sent()
            .into_iter()
            .filter(|sent| sent.path == "/crew/connections/from-invitation")
            .filter_map(|sent| sent.body)
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["preview"], true);
        assert_eq!(bodies[1]["preview"], false);
        assert_eq!(bodies[1]["username"], "bob");
        assert!(bodies[1]["invitation"]
            .as_str()
            .expect("pasted")
            .contains("brcrew1:abc"));
        assert!(bodies[1].get("advanced").is_none());
        assert!(lines.contains(&"  Hosted by \"Alice Chen\" (@alice) on hpc.ucsf.edu".to_owned()));
        assert!(lines.contains(&"  Workspace privacy: Private · ucsf".to_owned()));
        assert!(lines.contains(&"  You'll join as Private · ucsf.".to_owned()));
        assert_eq!(
            &lines[lines.len() - 3..],
            [
                "Saved lab.",
                "Next: biorouter crew --connection lab auth",
                "Then: biorouter crew --connection lab join"
            ]
        );

        // Without --yes and without a terminal to ask in, nothing is saved.
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let mut args = join_args(file.path());
        args.yes = false;
        let error = run(
            &api,
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(args))),
        )
        .await
        .expect_err("no confirmation");
        assert!(message(&error).starts_with("Add --yes"));
        assert_eq!(
            fake.sent()
                .iter()
                .filter(|sent| sent.path == "/crew/connections/from-invitation")
                .count(),
            1,
            "only the preview"
        );

        let missing = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/connections/from-invitation" {
                return Ok(preview(&["username", "institution"]));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, missing);
        let error = run(
            &api,
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(join_args(
                file.path(),
            )))),
        )
        .await
        .expect_err("missing choices");
        assert_eq!(
            message(&error),
            "Saving this invitation needs your username on the server (--username), an institution for a Private connection (--institution)."
        );
    }

    /// SC2-N1: someone else's invitation is flagged in the preview and on a `--yes` save, a
    /// connection it left behind is replaced only with `--replace`, and a save refused because
    /// the workspace is already saved names the terminal's commands, keeping the daemon's code.
    #[tokio::test]
    async fn join_invitation_warns_about_another_account_and_replaces_only_when_asked() {
        const OLD: &str = "c0ffee00-0000-4000-8000-00000000000b";
        let saves = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let handler = {
            let saves = std::sync::Arc::clone(&saves);
            move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if path == "/crew/connections/from-invitation" {
                    let body = body.expect("body");
                    if body["preview"] == true {
                        let mut answer = preview(&[]);
                        answer["preview"]["server"] = json!("44.246.189.59");
                        answer["preview"]["server_label"] = json!("lab-ubuntu");
                        answer["preview"]["login_mismatch"] =
                            json!({"config_user": "crew_trent", "invitee": "crew_bob"});
                        answer["preview"]["replaceable_connection_id"] = json!(OLD);
                        return Ok(answer);
                    }
                    saves.lock().expect("saves").push(body.clone());
                    if body.get("replace").is_none() {
                        return Err(FakeRefusal {
                            status: 409,
                            code: Some("crew_connection_exists".into()),
                            broker_code: None,
                            institution_refusal: None,
                            connection_institution: None,
                            message: "This computer already has “chen-lab” for this workspace, signing in as another account, and it has never connected. Replace it with this invitation, or change it in its connection settings.".into(),
                            detail: None,
                            modes: None,
                            fields: json!({"connection_id": OLD}).as_object().cloned().expect("object"),
                        }
                        .into());
                    }
                    return Ok(json!({"connection": {"id": CONNECTION, "name": "lab"}}));
                }
                if (method, path) == ("GET", "/crew/connections") {
                    return Ok(
                        json!({"connections": [{"id": OLD, "name": "chen-lab", "ssh_target": "crew_bob@44.246.189.59", "workspace_id": "w", "status": "disconnected"}]}),
                    );
                }
                standard(method, path, body)
            }
        };
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), "brcrew1:abc\n").expect("write invitation");
        let join = |replace: bool| {
            let mut args = join_args(file.path());
            args.username = None;
            args.replace = replace;
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(args)))
        };

        // Without --replace the save is refused, in the terminal's words.
        let (api, _) = api_with(OutputFormat::Text, handler.clone());
        let error = run(&api, join(false)).await.expect_err("already saved");
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert_eq!(
            shown,
            "This computer already has chen-lab for this workspace, signing in as another account, and it has never connected. Run it again with --replace, or remove it with biorouter crew --connection chen-lab connections remove."
        );
        assert!(!shown.contains("connection settings"), "{shown}");
        assert_eq!(
            error_code(&error).as_deref(),
            Some("crew_connection_exists")
        );
        assert!(saves.lock().expect("saves")[0].get("replace").is_none());

        // With --replace, the preview's connection is named, and the save replaces it.
        let (api, _) = api_with(OutputFormat::Text, handler.clone());
        let lines = said(run(&api, join(true)).await.expect("replaced"));
        assert_eq!(saves.lock().expect("saves")[1]["replace"], OLD);
        assert!(
            lines.contains(&"  Hosted by \"Alice Chen\" (@alice) on lab-ubuntu".to_owned()),
            "{lines:#?}"
        );
        assert!(
            lines.contains(&"  This invitation is for @crew_bob, but this computer signs in to lab-ubuntu as crew_trent. Ask your host for your own invitation.".to_owned()),
            "--yes still prints the warning: {lines:#?}"
        );
        assert!(lines.contains(&"  Saving replaces chen-lab, which signs in as another account and has never connected. Its key is deleted.".to_owned()), "{lines:#?}");
        assert!(
            !lines.iter().any(|line| line.contains("44.246.189.59")),
            "{lines:#?}"
        );

        // A preview without --replace says it can be replaced.
        let (api, _) = api_with(OutputFormat::Text, handler.clone());
        let mut args = join_args(file.path());
        args.yes = false;
        args.preview = true;
        args.username = None;
        let lines = said(
            run(
                &api,
                CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(args))),
            )
            .await
            .expect("previewed"),
        );
        assert!(lines.contains(&"  This computer already has chen-lab for this workspace, signing in as another account, and it has never connected. Add --replace to save this invitation in its place.".to_owned()), "{lines:#?}");

        // --replace with nothing to replace is a usage refusal, and nothing is saved.
        let (api, fake) = api_with(
            OutputFormat::Text,
            |method: &str, path: &str, body: Option<&Value>| {
                if path == "/crew/connections/from-invitation" {
                    return Ok(preview(&[]));
                }
                standard(method, path, body)
            },
        );
        let error = run(&api, join(true)).await.expect_err("nothing to replace");
        assert_eq!(error_code(&error).as_deref(), Some(NOTHING_TO_REPLACE));
        assert!(failure(&error, OutputFormat::Text, "req-1", false)
            .downcast_ref::<NeedsTerminal>()
            .is_some());
        assert_eq!(
            fake.sent()
                .iter()
                .filter(|sent| sent.path == "/crew/connections/from-invitation")
                .count(),
            1,
            "only the preview"
        );

        // The workspace already saved under the login saving would use.
        let existing = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/connections/from-invitation" {
                if body.expect("body")["preview"] == true {
                    let mut answer = preview(&[]);
                    answer["preview"]["existing_connection_id"] = json!(CONNECTION);
                    return Ok(answer);
                }
                return Err(FakeRefusal {
                    status: 409,
                    code: Some("crew_connection_exists".into()),
                    broker_code: None,
                    institution_refusal: None,
                    connection_institution: None,
                    message: "This computer already has “UCSF HPC” for this workspace. Change it in its connection settings instead.".into(),
                    detail: None,
                    modes: None,
                    fields: json!({"connection_id": CONNECTION}).as_object().cloned().expect("object"),
                }
                .into());
            }
            standard(method, path, body)
        };
        // T3-CLI-4: the daemon refuses it only because a compared setting differs, so the
        // terminal says nothing was saved and names every way on, `join` only as one of them.
        let (api, _) = api_with(OutputFormat::Text, existing);
        let error = run(&api, join(false)).await.expect_err("already saved");
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert_eq!(
            shown,
            "This workspace is already saved as UCSF HPC with a different server login, port, key file, jump host or privacy, so nothing was saved.\n  To compare them: biorouter crew --connection 'UCSF HPC' connections show\n  To give it this invitation's privacy: biorouter crew --connection 'UCSF HPC' privacy set-personal private --institution ucsf\n  To change its server login, port, key file or jump host: biorouter crew --connection 'UCSF HPC' connections update FILE\n  If it has not joined yet, remove it and save this invitation again: biorouter crew --connection 'UCSF HPC' connections remove\n  To use it as it is: biorouter crew --connection 'UCSF HPC' join"
        );
        assert!(!shown.contains("finish joining"), "{shown}");
        assert!(!shown.contains("connection settings"), "{shown}");
        assert_eq!(
            error_code(&error).as_deref(),
            Some("crew_connection_exists")
        );
        // JSON keeps the daemon's code and the connection it named beside the same words.
        let (api, _) = api_with(OutputFormat::Json, existing);
        let error = run(&api, join(false)).await.expect_err("already saved");
        let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
        assert_eq!(body["code"], "crew_connection_exists");
        assert_eq!(body["connection_id"], CONNECTION);
        assert_eq!(body["error"], shown);

        // The privacy line names the privacy the invitation would save: a Public choice, and a
        // hostile institution quoted for the shell with its controls escaped.
        let with_privacy = |mode: &str, institution: Option<&str>| {
            let mut answer = preview(&[]);
            answer["preview"]["mode"] = json!(mode);
            answer["preview"]["institution_id"] = json!(institution);
            set_personal_words(&answer["preview"])
        };
        assert_eq!(with_privacy("public", None), "privacy set-personal public");
        assert_eq!(
            with_privacy("private", Some("  ")),
            "privacy set-personal private"
        );
        assert_eq!(
            with_privacy("private", Some("u c\u{1b}[31m")),
            "privacy set-personal private --institution 'u c\\u{1b}[31m'"
        );
        assert_eq!(with_privacy("secret", None), "privacy set-personal MODE");
        let (api, _) = api_with(OutputFormat::Text, existing);
        let mut args = join_args(file.path());
        args.yes = false;
        args.preview = true;
        let lines = said(
            run(
                &api,
                CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(args))),
            )
            .await
            .expect("previewed"),
        );
        assert!(lines.contains(&"  This computer already has this workspace as UCSF HPC; saving again keeps that connection if its settings match.".to_owned()), "{lines:#?}");
    }

    /// SETUPHPC2-F-C: a route to this machine says so in the preview; one through a jump host
    /// or to another server does not.
    #[test]
    fn a_same_host_route_is_said_in_the_preview() {
        let mut answer = preview(&[]);
        answer["preview"]["ssh_target"] = json!("crew_iris@localhost");
        answer["preview"]["server_label"] = json!("localhost");
        let lines = invitation_summary(&answer["preview"], false);
        assert!(
            lines.contains(&"  Crew will connect to this machine (localhost).".to_owned()),
            "{lines:#?}"
        );
        answer["preview"]["proxy_jump"] = json!("bastion");
        assert!(!invitation_summary(&answer["preview"], false)
            .iter()
            .any(|line| line.contains("this machine")));
        answer["preview"]["proxy_jump"] = Value::Null;
        answer["preview"]["ssh_target"] = json!("crew_iris@hpc.ucsf.edu");
        assert!(!invitation_summary(&answer["preview"], false)
            .iter()
            .any(|line| line.contains("this machine")));
    }

    /// SF2-N3, CLIDOCS-F5, CLIDOCS-F12: `privacy show` says the privacy in force (`effective_mode`
    /// in JSON), lists channels in the channels-list order, and while disconnected prints the
    /// connection's own setting and says the workspace's cannot be checked, rather than failing.
    #[tokio::test]
    async fn privacy_show_says_the_privacy_in_force_online_and_offline() {
        let online = |own: &'static str| {
            move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if (method, path) == ("GET", "/crew/connections") {
                    return Ok(
                        json!({"connections": [{"id": CONNECTION, "name": "UCSF HPC", "ssh_target": "bob@hpc", "workspace_id": "w", "status": "connected", "mode": own, "institution_id": "ucsf", "policy_epoch": 2}]}),
                    );
                }
                if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                    let mut snapshot = snapshot();
                    // The broker lists them in no order a reader expects.
                    snapshot["channels"] = json!([
                        {"id": METHODS, "team_id": TEAM, "name": "methods", "classification": "restricted"},
                        {"id": "c4a77e10-0000-4000-8000-00000000000c", "team_id": TEAM, "name": "random", "classification": "public_safe"},
                        {"id": GENERAL, "team_id": TEAM, "name": "general", "classification": "restricted"}
                    ]);
                    return Ok(snapshot);
                }
                standard(method, path, body)
            }
        };
        let (api, _) = api_with(OutputFormat::Json, online("public"));
        let Reply::Show(value, options) = run(&api, CrewCommand::Privacy(PrivacyCommand::Show))
            .await
            .expect("shown")
        else {
            panic!("a value");
        };
        assert_eq!(
            value["effective_mode"], "private",
            "the workspace is Private"
        );
        let order: Vec<&str> = value["channels"]
            .as_array()
            .expect("channels")
            .iter()
            .filter_map(|channel| channel["name"].as_str())
            .collect();
        assert_eq!(order, ["general", "methods", "random"]);
        let text = output::render_text(&value, &options);
        assert!(
            text.starts_with("Privacy: Private (lab is Private for everyone)"),
            "{text}"
        );

        for (own, effective, first) in [
            ("private", json!("private"), "Privacy: Private"),
            (
                "public",
                Value::Null,
                "Privacy: can't be checked while disconnected",
            ),
        ] {
            let offline = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if (method, path) == ("GET", "/crew/connections") {
                    return Ok(
                        json!({"connections": [{"id": CONNECTION, "name": "UCSF HPC", "ssh_target": "bob@hpc", "workspace_id": "w", "status": "disconnected", "mode": own, "institution_id": "ucsf", "policy_epoch": 2}]}),
                    );
                }
                if path.ends_with("/request") {
                    return Err(refuse(409, Some(NOT_CONNECTED), LEGACY_DISCONNECTED));
                }
                standard(method, path, body)
            };
            let (api, _) = api_with(OutputFormat::Json, offline);
            let Reply::Show(value, options) = run(&api, CrewCommand::Privacy(PrivacyCommand::Show))
                .await
                .expect("shown offline")
            else {
                panic!("a value");
            };
            assert_eq!(value["effective_mode"], effective, "{own}");
            assert_eq!(value["personal_mode"], own);
            assert!(value["workspace"].is_null(), "{value}");
            let text = output::render_text(&value, &options);
            assert!(text.starts_with(first), "{own}: {text}");
            assert!(
                text.contains("Workspace: can't be checked while disconnected"),
                "{own}: {text}"
            );
            assert!(text.contains("Your connection: "), "{text}");
        }
        // An older daemon's untyped refusal is the same fact.
        let older = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/request") {
                return Err(refuse(
                    400,
                    Some("crew_request_refused"),
                    LEGACY_DISCONNECTED,
                ));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, older);
        run(&api, CrewCommand::Privacy(PrivacyCommand::Show))
            .await
            .expect("shown offline");
    }

    /// SC2-N5: a connected connection the host has not let in reads "Not joined yet", in
    /// the list and in `connections show`; a member's reads Connected. The JSON says `joined`.
    #[tokio::test]
    async fn a_connection_that_has_not_joined_reads_not_joined_yet() {
        for (state, word) in [("invited", "Not joined yet"), ("joined", "Connected")] {
            let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if path.ends_with("/join") && method == "GET" {
                    return Ok(json!({"status": state}));
                }
                standard(method, path, body)
            };
            let (api, fake) = api_with(OutputFormat::Text, handler);
            let listed = said(run(&api, CrewCommand::Status).await.expect("status"));
            assert!(
                listed[0].contains(&format!("bob@hpc · {word} · ")),
                "{listed:#?}"
            );
            if state == "invited" {
                assert!(
                    fake.broker_calls().is_empty(),
                    "a non-member's workspace is not read"
                );
            }
            let (api, _) = api_with(OutputFormat::Json, handler);
            let Reply::Show(value, _) = run(&api, CrewCommand::Status).await.expect("status")
            else {
                panic!("a value");
            };
            assert_eq!(value["connections"][0]["joined"], state == "joined");
            let (api, _) = api_with(OutputFormat::Text, handler);
            let shown = said(
                run(&api, CrewCommand::Connections(ConnectionCommand::Show))
                    .await
                    .expect("shown"),
            );
            assert!(
                shown[0].contains(&format!("  Status: {word}")),
                "{shown:#?}"
            );
        }
    }

    /// RES2-N2: a connection whose workspace server has stopped saving says so in `status` and
    /// `connections show`, and tells its host what to run; nothing is said while it saves.
    #[tokio::test]
    async fn a_server_that_stopped_saving_is_said_before_anyone_writes() {
        let stopped = |host: bool| {
            move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                match (method, path) {
                    ("GET", "/crew/connections") => {
                        return Ok(json!({"connections": [{
                            "id": CONNECTION, "name": "UCSF HPC", "ssh_target": "bob@hpc",
                            "server_label": "hpc", "workspace_id": "w", "status": "connected",
                            "mode": "private",
                            "server_storage": {"state": "storage_failed", "code": "storage_full", "since": 1_700_000_000}
                        }]}));
                    }
                    (_, path) if path.ends_with("/join") => return Ok(json!({"status": "joined"})),
                    _ => {}
                }
                if !host
                    && body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot")
                {
                    let mut snapshot = snapshot();
                    snapshot["actor"] = json!({"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001});
                    return Ok(snapshot);
                }
                standard(method, path, body)
            }
        };
        let (api, _) = api_with(OutputFormat::Text, stopped(true));
        let shown = said(run(&api, CrewCommand::Status).await.expect("status")).join("\n");
        assert!(
            shown.contains(&format!("\n  {}", output::SERVER_STOPPED_SAVING)),
            "{shown}"
        );
        assert!(shown.contains("  You host this workspace. Free space on hpc, then restart Crew there: biorouter-crew stop, then biorouter-crew start, each with this workspace's --state-dir."), "{shown}");
        let (api, _) = api_with(OutputFormat::Text, stopped(false));
        let shown = said(
            run(&api, CrewCommand::Connections(ConnectionCommand::Show))
                .await
                .expect("shown"),
        )
        .join("\n");
        assert!(shown.contains(output::SERVER_STOPPED_SAVING), "{shown}");
        assert!(!shown.contains("You host"), "a member: {shown}");
        // A server that saves (or an older daemon that does not say) adds nothing.
        let (api, _) = api_with(OutputFormat::Text, standard);
        let shown = said(run(&api, CrewCommand::Status).await.expect("status")).join("\n");
        assert!(!shown.contains("stopped saving"), "{shown}");
    }

    /// SETUPHPC2-F-A: a refused connect to a login on this machine says what a member on the
    /// server itself must fix, with the code and OpenSSH's words, in `connect` and `join`; a
    /// login elsewhere keeps the ordinary sentences.
    #[tokio::test]
    async fn a_same_host_connect_failure_says_what_to_fix_on_this_machine() {
        let refusing = |target: &'static str, code: &'static str| {
            move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                match (method, path) {
                    ("GET", "/crew/connections") => {
                        return Ok(
                            json!({"connections": [{"id": CONNECTION, "name": "patel-hpc", "ssh_target": target, "workspace_id": "w", "status": "disconnected"}]}),
                        );
                    }
                    (_, path) if path.ends_with("/connect") => {
                        return Err(FakeRefusal {
                            status: 400,
                            code: Some(code.into()),
                            broker_code: None,
                            institution_refusal: None,
                            connection_institution: None,
                            message: "Couldn't sign in as crew_iris on this machine.".into(),
                            detail: Some(
                                "crew_iris@localhost: Permission denied (publickey).".into(),
                            ),
                            modes: None,
                            fields: Default::default(),
                        }
                        .into());
                    }
                    (_, path) if path.ends_with("/join") => {
                        return Err(refuse(409, Some(NOT_CONNECTED), LEGACY_DISCONNECTED));
                    }
                    _ => {}
                }
                standard(method, path, body)
            }
        };
        let shown =
            |error: &anyhow::Error| failure(error, OutputFormat::Text, "req-1", false).to_string();
        let (api, _) = api_with(
            OutputFormat::Text,
            refusing("crew_iris@localhost", "crew_ssh_key_refused"),
        );
        let error = run(&api, CrewCommand::Connect).await.expect_err("refused");
        assert_eq!(
            shown(&error),
            "Couldn't sign in as crew_iris on this machine: add your public SSH key to your own ~/.ssh/authorized_keys, then connect again.\n  Code: crew_ssh_key_refused\n  Details: crew_iris@localhost: Permission denied (publickey)."
        );
        let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
        assert_eq!(body["code"], "crew_ssh_key_refused");
        assert_eq!(
            body["detail"],
            "crew_iris@localhost: Permission denied (publickey)."
        );

        let (api, _) = api_with(
            OutputFormat::Text,
            refusing("crew_iris@127.0.0.1", "crew_ssh_host_key_unknown"),
        );
        let error = run(&api, CrewCommand::Join(JoinArgs { no_wait: true }))
            .await
            .expect_err("refused");
        assert!(shown(&error).starts_with("127.0.0.1's host key isn't in your ~/.ssh/known_hosts yet. Add this server's own key (from /etc/ssh/ssh_host_ed25519_key.pub) to it, then connect again.\n  Code: crew_ssh_host_key_unknown"), "{}", shown(&error));

        // Another server keeps the sentence for a server someone else runs.
        let (api, _) = api_with(
            OutputFormat::Text,
            refusing("crew_iris@hpc.ucsf.edu", "crew_ssh_key_refused"),
        );
        let error = run(&api, CrewCommand::Connect).await.expect_err("refused");
        assert!(
            shown(&error).starts_with("The server refused this computer's SSH key."),
            "{}",
            shown(&error)
        );
        for target in ["localhost", "bob@LOCALHOST", "bob@[::1]", "bob@127.0.0.2"] {
            assert!(output::loopback_login(target).is_some(), "{target}");
        }
        for target in [
            "hpc",
            "bob@hpc.ucsf.edu",
            "bob@10.0.0.1",
            "localhost.example.org",
        ] {
            assert!(output::loopback_login(target).is_none(), "{target}");
        }
    }

    /// SF-F3: a host saving their own workspace reads host words in the preview, not an
    /// invitation to join; anyone else reads the invitation as before.
    #[test]
    fn a_host_saving_their_own_workspace_reads_host_words() {
        let preview = preview(&[]);
        let hosting = invitation_summary(&preview["preview"], true);
        assert_eq!(hosting[0], "Your workspace lab");
        assert!(hosting.contains(&"  You'll host it as Private · ucsf.".to_owned()));
        assert!(!hosting
            .iter()
            .any(|line| line.contains("Invitation to") || line.contains("You'll join")));
        let joining = invitation_summary(&preview["preview"], false);
        assert_eq!(joining[0], "Invitation to lab");
        assert!(joining.contains(&"  You'll join as Private · ucsf.".to_owned()));
    }

    /// SF-F3: `connections prepare` and `enroll prepare` are one command, used by a host before
    /// `biorouter-crew start` and by a joiner before `connections save`. Both print the same
    /// words, and those words tell each reader where the key and the preparation ID go,
    /// without sending either of them down the other's path.
    #[tokio::test]
    async fn both_prepare_commands_describe_the_key_for_a_host_and_a_joiner() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if (method, path) == ("POST", "/crew/devices/prepare") {
                return Ok(json!({
                    "preparation_id": "9e7a0000-0000-4000-8000-000000000009",
                    "public_key": "ab".repeat(32),
                    "device_id": "de71ce00-0000-4000-8000-00000000000a",
                }));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let connections = said(
            run(&api, CrewCommand::Connections(ConnectionCommand::Prepare))
                .await
                .expect("connections prepare"),
        );
        let enroll = said(
            run(&api, CrewCommand::Enroll(EnrollmentCommand::Prepare))
                .await
                .expect("enroll prepare"),
        );
        assert_eq!(connections, enroll, "the two names are one command");
        let text = connections.join("\n");
        for (use_, words) in [
            (
                "host: the key",
                "pass this key to biorouter-crew start --bootstrap-key",
            ),
            (
                "joiner: the key",
                "To join one, send it to the workspace's host.",
            ),
            (
                "host: the ID",
                "connections join-invitation --preparation-id when you host",
            ),
            (
                "joiner: the ID",
                "the descriptor for connections save when you join",
            ),
        ] {
            assert!(text.contains(words), "{use_}: {text}");
        }
        assert!(!text.contains("for hosting"), "{text}");
    }

    /// CLI-9: the next steps after saving a connection: a host signs in, then sets the
    /// workspace up; a joiner signs in, then joins; a connection already up skips signing in.
    #[test]
    fn the_next_step_after_saving_depends_on_hosting_and_connecting() {
        assert_eq!(
            next_steps("lab", true, false),
            [
                "Next: biorouter crew --connection lab auth",
                "Then: biorouter crew --connection lab workspace bootstrap"
            ]
        );
        assert_eq!(
            next_steps("UCSF HPC", false, false),
            [
                "Next: biorouter crew --connection 'UCSF HPC' auth",
                "Then: biorouter crew --connection 'UCSF HPC' join"
            ]
        );
        assert_eq!(
            next_steps("lab", false, true),
            ["Next: biorouter crew --connection lab join"]
        );
    }

    /// CLI-19: `--identity-file` is sent absolute, whatever the shell left of it.
    ///
    /// Every expected path is built with `Path::join` and compared as a `Path` (component by
    /// component), and the folders are real absolute ones, so the test reads the same on
    /// Windows, where `join` writes `\` and `/etc/key` is not absolute.
    #[test]
    fn an_identity_file_is_made_absolute_before_it_is_sent() {
        let base = std::env::temp_dir();
        let home_dir = base.join("home").join("bob");
        let work = base.join("work");
        let home = Some(home_dir.clone());
        let cwd = || Ok(work.clone());
        let made = |text: &str, home: Option<std::path::PathBuf>| {
            let made = absolute_local_path(text, home, cwd).expect(text);
            assert!(Path::new(&made).is_absolute(), "{text} -> {made}");
            std::path::PathBuf::from(made)
        };

        assert_eq!(
            made("~/.ssh/lab_ed25519", home.clone()),
            home_dir.join(".ssh").join("lab_ed25519")
        );
        assert_eq!(made("~", home.clone()), home_dir);
        // The platform's own separator after `~` is the home folder too: `~\key` on Windows.
        assert_eq!(
            made(&format!("~{}key", std::path::MAIN_SEPARATOR), home.clone()),
            home_dir.join("key")
        );
        assert_eq!(
            made("./keys/lab", home.clone()),
            work.join("keys").join("lab")
        );
        assert_eq!(
            made("keys/lab", home.clone()),
            work.join("keys").join("lab")
        );
        // `~bob/key` is another user's home, which only a shell can read; it stays relative.
        assert_eq!(
            made("~bob/key", home.clone()),
            work.join("~bob").join("key")
        );
        assert!(absolute_local_path("~/key", None, cwd).is_err());

        // An absolute path is sent exactly as it was given.
        let absolute = base.join("keys").join("lab_ed25519");
        let absolute = absolute.to_str().expect("a Unicode temp folder");
        assert_eq!(
            absolute_local_path(absolute, home.clone(), || bail!("not read")).unwrap(),
            absolute
        );
        assert_eq!(absolute_local_path("", home, cwd).unwrap(), "");

        let mut args = join_args(Path::new("-"));
        args.identity_file = Some("keys/lab".into());
        let body = invitation_request("brcrew1:abc", &args).expect("request");
        let sent = body["advanced"]["identity_file"]
            .as_str()
            .expect("identity file");
        assert!(Path::new(sent).is_absolute(), "{sent}");
        assert!(
            Path::new(sent).ends_with(Path::new("keys").join("lab")),
            "{sent}"
        );
    }

    #[tokio::test]
    async fn join_shows_the_code_then_claims_once_the_host_approves() {
        let statuses = Mutex::new(vec![
            json!({"status": "approved", "code": "7QK2-M9XA-3JTP-WZ4D", "workspace_name": "lab", "add_device": false}),
            json!({"status": "invited", "code": "7QK2-M9XA-3JTP-WZ4D", "workspace_name": "lab", "add_device": false,
                   "inviter": {"username": "alice", "display_name": "Alice Chen"}}),
        ]);
        let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match (method, path.ends_with("/join")) {
                ("GET", true) => Ok(statuses.lock().expect("statuses").pop().expect("a status")),
                ("POST", true) => Ok(
                    json!({"joined": true, "status": "joined", "workspace_name": "lab", "add_device": false}),
                ),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::StreamJson, handler);
        run(&api, CrewCommand::Join(JoinArgs { no_wait: false }))
            .await
            .expect("joined");
        let joins: Vec<&str> = fake
            .sent()
            .iter()
            .filter(|sent| sent.path.ends_with("/join"))
            .map(|sent| {
                if sent.method == "GET" {
                    "status"
                } else {
                    "claim"
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .collect();
        assert_eq!(joins, ["status", "status", "claim"]);

        let status = json!({"status": "invited", "code": "7QK2-M9XA-3JTP-WZ4D", "workspace_name": "lab",
                            "inviter": {"username": "alice", "display_name": "Alice Chen"}});
        assert_eq!(
            join_lines(&status, false),
            [
                "\"Alice Chen\" (@alice) invited you to lab.",
                "Send Alice this code: 7QK2-M9XA-3JTP-WZ4D"
            ]
        );
        let mismatch = json!({"status": "code_mismatch", "code": "7QK2-M9XA-3JTP-WZ4D", "inviter": {"username": "alice", "display_name": "alice"}});
        assert_eq!(
            join_lines(&mismatch, false),
            ["The code @alice entered doesn't match this computer. Send it again: 7QK2-M9XA-3JTP-WZ4D"]
        );

        let expired = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/join") {
                return Ok(
                    json!({"status": "expired", "inviter": {"username": "alice", "display_name": "Alice Chén"}}),
                );
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, expired);
        let error = run(&api, CrewCommand::Join(JoinArgs { no_wait: false }))
            .await
            .expect_err("expired");
        assert!(message(&error).starts_with("This invitation expired. Ask "));

        // CLI-4: the isolates the CLI put around a name that is not ASCII reach the terminal
        // as isolates, and the JSON error carries the same sentence, never the escape text.
        let shown = failure(&error, OutputFormat::Text, "req-1", false).to_string();
        assert_eq!(
            shown,
            "This invitation expired. Ask \"\u{2068}Alice Chén\u{2069}\" (@alice) to invite you again."
        );
        let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
        assert!(!body["error"].as_str().unwrap().contains("\\u{"), "{body}");
    }

    #[tokio::test]
    async fn the_host_invites_by_username_and_gets_the_message_to_send() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.contains("/invitation?") {
                return Ok(
                    json!({"message": "Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this whole message.\nbrcrew1:abc", "line": "brcrew1:abc"}),
                );
            }
            match body.and_then(|body| body["method"].as_str()) {
                Some("enrollment.invite") => Ok(
                    json!({"username": "bob", "full_name": "Bob Lee", "add_device": false, "expires_at": 4_000_000_000_i64}),
                ),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let invite = |username: Option<&str>, uid: Option<u32>| {
            CrewCommand::Enroll(EnrollmentCommand::Invite(EnrollInviteArgs {
                username: username.map(str::to_owned),
                add_device: false,
                uid,
                public_key: uid.map(|_| "abcd".to_owned()),
                existing_principal: uid.map(|_| BOB.to_owned()),
            }))
        };
        let lines = said(
            run(&api, invite(Some("@bob"), None))
                .await
                .expect("invited"),
        );
        assert_eq!(
            fake.broker_call("enrollment.invite").expect("invite"),
            json!({"username": "bob", "idempotency_key": "req-1"})
        );
        assert!(fake
            .sent()
            .iter()
            .any(|sent| sent.path
                == format!("/crew/connections/{CONNECTION}/invitation?invitee=bob")));
        assert_eq!(
            lines[0],
            "Invited @bob · Bob Lee (name on the server account)."
        );
        assert!(lines.contains(&"brcrew1:abc".to_owned()));
        assert_eq!(
            lines.last().map(String::as_str),
            Some("When @bob sends you a code, let them in with: biorouter crew enroll approve @bob CODE")
        );

        // The deprecated form still sends the legacy parameters.
        let (api, fake) = api_with(OutputFormat::Json, handler);
        run(&api, invite(None, Some(12345)))
            .await
            .expect("legacy invite");
        assert_eq!(
            fake.broker_call("enrollment.invite").expect("invite"),
            json!({"uid": 12345, "public_key": "abcd", "existing_principal_id": BOB, "idempotency_key": "req-1"})
        );
    }

    /// T-13: a host who saves a code is told only that: the broker can't compare it with the
    /// joiner's until their computer claims, so "Approved." promised too much.
    #[tokio::test]
    async fn approving_says_the_code_is_saved_not_that_they_joined() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("enrollment.approve") => Ok(json!({"approved": true, "username": "bob"})),
                _ => standard(method, path, body),
            }
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                CrewCommand::Enroll(EnrollmentCommand::Approve {
                    person: "@bob".into(),
                    code: "7QK2-M9XA-3JTP-WZ4D".into(),
                    replace: false,
                }),
            )
            .await
            .expect("approved"),
        );
        assert_eq!(
            lines,
            ["Code saved. @bob joins when their computer confirms the same code."]
        );
    }

    /// T-13/T-14: `join --no-wait` claims as soon as the host saved a code, and when the claim
    /// is refused it reads the status again and reports that (the code doesn't match), never
    /// "Joining…". No second claim is made in the same run.
    #[tokio::test]
    async fn join_no_wait_reports_a_refused_code_and_never_says_joining() {
        let statuses = Mutex::new(vec![
            json!({"status": "code_mismatch", "code": "7QK2-M9XA-3JTP-WZ4D", "workspace_name": "lab",
                   "inviter": {"username": "alice", "display_name": "Alice Chen"}}),
            json!({"status": "approved", "code": "7QK2-M9XA-3JTP-WZ4D", "workspace_name": "lab",
                   "inviter": {"username": "alice", "display_name": "Alice Chen"}}),
        ]);
        let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match (method, path.ends_with("/join")) {
                ("GET", true) => Ok(statuses.lock().expect("statuses").pop().expect("a status")),
                ("POST", true) => Err(refuse(
                    409,
                    Some("crew_join_code_mismatch"),
                    "The host hasn't let this device in.",
                )),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::StreamJson, handler);
        run(&api, CrewCommand::Join(JoinArgs { no_wait: true }))
            .await
            .expect("a refused code is a status, not an error");
        let joins: Vec<&str> = fake
            .sent()
            .iter()
            .filter(|sent| sent.path.ends_with("/join"))
            .map(|sent| {
                if sent.method == "GET" {
                    "status"
                } else {
                    "claim"
                }
            })
            .collect();
        assert_eq!(joins, ["status", "claim", "status"]);

        // What a status still reading `approved` says when the command is not waiting.
        let approved = json!({"status": "approved", "workspace_name": "lab",
                              "inviter": {"username": "alice", "display_name": "Alice Chen"}});
        let lines = join_lines(&approved, false);
        assert!(
            lines.iter().all(|line| !line.contains("Joining")),
            "{lines:?}"
        );
        assert_eq!(
            lines,
            ["Alice saved a code for you, but this computer hasn't joined lab yet. Run biorouter crew join to finish."]
        );
        assert_eq!(join_lines(&approved, true), ["Joining lab…"]);
    }

    fn add_member_command(team: Option<&str>, channels: &[&str]) -> CrewCommand {
        CrewCommand::Members(MembersArgs {
            command: Some(MembersCommand::Add {
                person: "@bob".into(),
                team: team.map(str::to_owned),
                channels: channels.iter().map(|c| (*c).to_owned()).collect(),
            }),
        })
    }

    /// Direct add: `members add @bob --team T --channel C` sends one `team.add_member` with the
    /// confirmed username and the team's channel, the channel confined to that team, and says
    /// what Bob can now see.
    #[tokio::test]
    async fn members_add_puts_a_member_straight_into_a_team_and_its_channels() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("team.add_member") => Ok(json!({
                    "team_id": TEAM, "principal_id": BOB,
                    "added_channels": [GENERAL, METHODS], "already_member": false
                })),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                add_member_command(Some("Analysis Lab"), &["#methods"]),
            )
            .await
            .expect("added"),
        );
        assert_eq!(
            fake.broker_call("team.add_member")
                .expect("team.add_member"),
            json!({"team_id": TEAM, "principal_id": BOB, "expected_username": "bob",
                   "channel_ids": [METHODS], "idempotency_key": "req-1"})
        );
        assert_eq!(
            lines,
            ["Added \"Bob Lee\" (@bob) to Analysis Lab. They can now see #general and #methods."]
        );
        let selectors = &fake.resolve_bodies()[0]["selectors"];
        assert!(
            selectors
                .as_array()
                .unwrap()
                .contains(&json!({"kind": "channel", "text": "Analysis Lab/#methods"})),
            "{selectors}"
        );

        // Already in everything: a success that says so.
        let already = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("team.add_member") => Ok(json!({
                    "team_id": TEAM, "principal_id": BOB, "added_channels": [], "already_member": true
                })),
                _ => standard(method, path, body),
            }
        };
        let (api, _) = api_with(OutputFormat::Text, already);
        let lines = said(
            run(&api, add_member_command(Some("Analysis Lab"), &[]))
                .await
                .expect("a no-op add"),
        );
        assert_eq!(
            lines,
            ["\"Bob Lee\" (@bob) is already in everything you chose."]
        );
    }

    /// M11: every team has a `#general`, so a channel added on its own is named with its team
    /// when another of the person's teams has a channel of that name.
    #[tokio::test]
    async fn an_added_channel_names_its_team_when_another_team_shares_its_name() {
        const OTHER_TEAM: &str = "7ea30000-0000-4000-8000-00000000000e";
        const OTHER_GENERAL: &str = "c4a77e10-0000-4000-8000-00000000000e";
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            let body_value = body.cloned().unwrap_or_default();
            match body_value["method"].as_str() {
                Some("workspace.snapshot") => {
                    let mut snapshot = snapshot();
                    snapshot["teams"][0]["members"] = json!([ALICE, BOB]);
                    snapshot["teams"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"id": OTHER_TEAM, "name": "Methods Team", "general_channel_id": OTHER_GENERAL, "members": [ALICE]}));
                    snapshot["channels"].as_array_mut().unwrap().push(
                        json!({"id": OTHER_GENERAL, "team_id": OTHER_TEAM, "name": "general", "classification": "restricted"}),
                    );
                    Ok(snapshot)
                }
                Some("channel.add_member") => Ok(json!({
                    "channel_id": body_value["params"]["channel_id"],
                    "principal_id": BOB,
                    "already_member": false,
                })),
                _ => standard(method, path, body),
            }
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                add_member_command(None, &["analysis-lab/general", "#methods"]),
            )
            .await
            .expect("added"),
        );
        assert_eq!(
            lines,
            ["Added \"Bob Lee\" (@bob) to Analysis Lab / #general and #methods."]
        );
    }

    /// Without `--team`, each channel is its own `channel.add_member`, each with its own
    /// idempotency key derived from the one request ID, so a retry replays them all.
    #[tokio::test]
    async fn members_add_to_channels_gives_each_step_its_own_retry_key() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            let body_value = body.cloned().unwrap_or_default();
            match body_value["method"].as_str() {
                Some("channel.add_member") => Ok(json!({
                    "channel_id": body_value["params"]["channel_id"],
                    "principal_id": BOB,
                    "already_member": body_value["params"]["channel_id"] == GENERAL,
                })),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                add_member_command(None, &["#methods", "analysis-lab/general"]),
            )
            .await
            .expect("added"),
        );
        let calls: Vec<Value> = fake
            .sent()
            .into_iter()
            .filter(|sent| sent.path.ends_with("/request"))
            .filter_map(|sent| sent.body)
            .filter(|body| body["method"] == "channel.add_member")
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["params"]["channel_id"], METHODS);
        assert_eq!(calls[0]["params"]["idempotency_key"], "req-1");
        assert_eq!(calls[0]["params"]["expected_username"], "bob");
        assert_eq!(calls[1]["params"]["channel_id"], GENERAL);
        assert_eq!(calls[1]["params"]["idempotency_key"], "req-1:1");
        assert_eq!(calls[1]["request_id"], "req-1:1");
        assert_eq!(lines, ["Added \"Bob Lee\" (@bob) to #methods."]);

        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(&api, add_member_command(None, &[]))
            .await
            .expect_err("nowhere to add them");
        assert!(message(&error).starts_with("Choose where to add them"));
        assert!(fake.broker_calls().is_empty());
    }

    /// A snapshot in which Alice owns #methods but not #general, and Bob is in their team.
    fn owned_snapshot() -> Value {
        let mut snapshot = snapshot();
        snapshot["workspace"]["host_principal_id"] = json!(CAROL);
        snapshot["teams"][0]["members"] = json!([ALICE, BOB]);
        snapshot["channels"][0]["owner_id"] = json!(CAROL);
        snapshot["channels"][1]["owner_id"] = json!(ALICE);
        snapshot
    }

    /// CLI-11: without --team each channel is its own add, so what the person's own view
    /// shows would be refused stops the command before anything is added.
    #[tokio::test]
    async fn members_add_to_channels_checks_every_channel_before_adding_any() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("workspace.snapshot") => Ok(owned_snapshot()),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let error = run(
            &api,
            add_member_command(None, &["#methods", "analysis-lab/general"]),
        )
        .await
        .expect_err("#general is not Alice's");
        assert_eq!(
            message(&error),
            "You don't own #general. Only its owner or the workspace host can add people to it.\nNothing was added."
        );
        assert!(fake.broker_call("channel.add_member").is_none());
        assert!(!failure(&error, OutputFormat::Text, "req-1", true)
            .to_string()
            .contains("--request-id"));

        let mut snapshot = owned_snapshot();
        snapshot["channels"][1]["archived"] = json!(true);
        snapshot["teams"][0]["members"] = json!([ALICE]);
        let targets = [Target {
            id: METHODS.into(),
            label: None,
            username: None,
        }];
        assert_eq!(
            channel_add_problems(&snapshot, BOB, "bob", &targets),
            ["#methods is archived, so no one can be added to it."]
        );
        snapshot["channels"][1]["archived"] = json!(false);
        assert_eq!(
            channel_add_problems(&snapshot, BOB, "bob", &targets),
            ["@bob isn't in the team #methods belongs to yet. Add them to the team first."]
        );
        // The host may add anyone to any channel, listed or not.
        snapshot["workspace"]["host_principal_id"] = json!(ALICE);
        snapshot["teams"][0]["members"] = json!([ALICE, BOB]);
        snapshot["channels"][1]["owner_id"] = json!(CAROL);
        assert!(channel_add_problems(&snapshot, BOB, "bob", &targets).is_empty());
    }

    /// CLI-11: a channel that fails after another was added says what changed; a refusal
    /// offers no retry, and a lost answer does.
    #[tokio::test]
    async fn a_partial_add_says_what_changed_and_offers_a_retry_only_when_uncertain() {
        for (refused, retry) in [(true, false), (false, true)] {
            let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                let body_value = body.cloned().unwrap_or_default();
                match body_value["method"].as_str() {
                    Some("channel.add_member") if body_value["params"]["channel_id"] == GENERAL => {
                        Err(if refused {
                            refuse_broker(
                                "forbidden",
                                "forbidden: Only the channel's owner or the workspace host can add people to it.",
                            )
                        } else {
                            anyhow!("connection reset")
                        })
                    }
                    Some("channel.add_member") => Ok(json!({
                        "channel_id": body_value["params"]["channel_id"],
                        "principal_id": BOB,
                        "already_member": false,
                    })),
                    _ => standard(method, path, body),
                }
            };
            let (api, _) = api_with(OutputFormat::Text, handler);
            let error = run(
                &api,
                add_member_command(None, &["#methods", "analysis-lab/general"]),
            )
            .await
            .expect_err("the second channel failed");
            let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
            assert!(
                shown.starts_with("Added @bob to #methods, then stopped at #general: "),
                "{shown}"
            );
            assert_eq!(
                shown.contains("Retry safely with --request-id req-1"),
                retry,
                "{shown}"
            );
        }
    }

    /// CLI-15: JSON output of `members` and `channels list` carries what the text shows.
    #[tokio::test]
    async fn json_lists_carry_who_you_are_the_host_and_unread_counts() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["unread"] = json!({METHODS: 3});
                return Ok(snapshot);
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, handler);
        let Reply::Show(people, _) = run(&api, CrewCommand::Members(MembersArgs { command: None }))
            .await
            .expect("members")
        else {
            panic!("members is a list")
        };
        assert_eq!(people[0]["id"], ALICE);
        assert_eq!(
            (people[0]["is_you"].clone(), people[0]["is_host"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(
            (people[1]["is_you"].clone(), people[1]["is_host"].clone()),
            (json!(false), json!(false))
        );

        let Reply::Show(channels, _) = run(
            &api,
            CrewCommand::Channels(ChannelCommand::List { team: None }),
        )
        .await
        .expect("channels") else {
            panic!("channels is a list")
        };
        assert_eq!(channels[0]["id"], GENERAL);
        assert_eq!(channels[0]["unread"], 0);
        assert_eq!(channels[1]["unread"], 3);

        // Without the map (an older broker), nothing is invented.
        assert!(with_unread(json!([{"id": METHODS}]), &snapshot())[0]
            .get("unread")
            .is_none());
    }

    /// M17, F6, SF-F10: `members` and `channels list` come out in a reader's order in text and
    /// JSON alike, whatever order the broker's IDs put them in, and a partial list says so.
    #[tokio::test]
    async fn lists_come_out_in_a_readers_order_and_say_when_they_are_partial() {
        let shuffled = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["actor"] =
                    json!({"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001});
                snapshot["principals"] = json!([
                    {"id": CAROL, "username": "carol", "display_name": "Carol Diaz", "uid": 1002},
                    {"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001},
                    {"id": ALICE, "username": "alice", "display_name": "Alice Chen", "uid": 1000}
                ]);
                snapshot["channels"] = json!([
                    {"id": METHODS, "team_id": TEAM, "name": "methods", "classification": "restricted"},
                    {"id": GENERAL, "team_id": TEAM, "name": "general", "classification": "restricted"}
                ]);
                snapshot["totals"] = json!({"channels": 4, "teams": 1});
                return Ok(snapshot);
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, shuffled);
        let Reply::Show(people, _) = run(&api, CrewCommand::Members(MembersArgs { command: None }))
            .await
            .expect("members")
        else {
            panic!("members is a list")
        };
        let ids: Vec<&str> = people
            .as_array()
            .unwrap()
            .iter()
            .map(|person| person["id"].as_str().unwrap())
            .collect();
        // Alice hosts, Bob is you, then Carol by name.
        assert_eq!(ids, [ALICE, BOB, CAROL]);
        // An older broker reports no presence, and none is invented.
        assert!(people[0].get("is_online").is_none());
        let mut present = snapshot();
        present["online_principal_ids"] = json!([BOB]);
        let marked = with_roles(present["principals"].clone(), &present);
        assert_eq!(marked[0]["is_online"], false);
        assert_eq!(marked[1]["is_online"], true);

        let (api, _) = api_with(OutputFormat::Text, shuffled);
        let reply = run(
            &api,
            CrewCommand::Channels(ChannelCommand::List { team: None }),
        )
        .await
        .expect("channels");
        let Reply::Show(ref channels, _) = reply else {
            panic!("channels is a list")
        };
        assert_eq!(channels[0]["id"], GENERAL, "#general first");
        let text = said(reply);
        assert!(
            text[0].ends_with("Showing 2 of your 4 channels."),
            "{text:?}"
        );
    }

    /// CLI-16: a started task is named by the channel the person typed, and a revoked task's
    /// status is the word every other surface uses.
    #[tokio::test]
    async fn a_started_task_names_its_channel_and_statuses_use_the_shared_words() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if method == "POST" && path.ends_with("/runs") {
                return Ok(json!({
                    "run_id": "r-1", "connection_id": CONNECTION, "channel_id": METHODS,
                    "session_id": "20260927_3", "status": "running"
                }));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                CrewCommand::Tasks(TaskCommand::Start {
                    channel: "methods".into(),
                    prompt: TextInput {
                        text: Some("Sum the counts".into()),
                        input: None,
                    },
                    provider: "p".into(),
                    model: "m".into(),
                    context_channels: Vec::new(),
                    allow_posting: true,
                }),
            )
            .await
            .expect("started"),
        );
        assert!(
            lines[0].starts_with("Task posting to #methods in Analysis Lab · Working…"),
            "{lines:?}"
        );

        let lines = revoked_lines(
            SESSION,
            &json!({"revoked": true, "remote_revocation_confirmed": true, "task_status": "completed"}),
        )
        .expect("revoked");
        assert_eq!(lines[1], "Its task's status: Done.");
    }

    /// CLI-20: `tasks cancel` sends no body, so a failure offers no `--request-id` that the
    /// route would ignore; running it again is the retry.
    #[tokio::test]
    async fn tasks_cancel_offers_no_request_id_the_route_ignores() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/runs/r-1/cancel") {
                return Err(refuse(
                    503,
                    Some("crew_revocation_unconfirmed"),
                    "Local cancellation requested; retry cancellation to confirm revocation.",
                ));
            }
            standard(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let error = run(
            &api,
            CrewCommand::Tasks(TaskCommand::Cancel { run: "r-1".into() }),
        )
        .await
        .expect_err("unconfirmed");
        let cancel = fake
            .sent()
            .into_iter()
            .find(|sent| sent.path.ends_with("/cancel"))
            .expect("cancel sent");
        assert_eq!((cancel.method.as_str(), cancel.body), ("POST", None));
        let shown = failure(
            &error,
            OutputFormat::Text,
            "req-1",
            api.client.sent.load(Ordering::SeqCst),
        )
        .to_string();
        assert!(!shown.contains("--request-id"), "{shown}");
    }

    /// `(status checks, claims)` join sent: `GET …/join` and `POST …/join`.
    fn join_requests(fake: &FakeDaemon) -> (usize, usize) {
        let sent = fake.sent();
        let count = |method: &str| {
            sent.iter()
                .filter(|sent| sent.method == method && sent.path.ends_with("/join"))
                .count()
        };
        (count("GET"), count("POST"))
    }

    /// Runs `join_until` with `interrupt` standing in for Ctrl-C, and fails rather than hangs
    /// when the press is not heard.
    async fn join_interrupted(
        api: &Api,
        interrupt: impl std::future::Future<Output = std::io::Result<()>>,
    ) -> Reply {
        tokio::time::timeout(
            Duration::from_secs(5),
            join_until(api, JoinArgs { no_wait: false }, interrupt),
        )
        .await
        .expect("the press stops the wait")
        .expect("stopping is not an error")
    }

    /// CLI-18. The fake daemon answers within one poll, so a request can never be pending when
    /// the press lands; each of these tests reaches one `select!` in `join_until` instead, and
    /// fails if that select stops listening for the press.
    ///
    /// The status check: a press already made when join reaches it wins the biased select, so
    /// `GET …/join` (which can wait on a slow SSH connect) is never sent.
    #[tokio::test]
    async fn a_ctrl_c_already_pressed_stops_join_before_the_status_check() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match (method, path.ends_with("/join")) {
                ("GET", true) => Ok(json!({"status": "approved", "code": "7QK2-M9XA-3JTP-WZ4D",
                                           "workspace_name": "lab", "add_device": false})),
                ("POST", true) => Ok(
                    json!({"joined": true, "status": "joined", "workspace_name": "lab", "add_device": false}),
                ),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::StreamJson, handler);
        let reply = join_interrupted(&api, async { Ok(()) }).await;
        assert!(matches!(reply, Reply::Streamed));
        assert_eq!(join_requests(&fake), (0, 0), "nothing sent after the press");
    }

    /// The claim: a press made while the status was read (the fake answers "approved" and
    /// presses in the same call) is heard before the claim, so `POST …/join` is never sent.
    #[tokio::test]
    async fn a_ctrl_c_during_the_status_check_stops_join_before_it_claims() {
        let notify = Arc::new(tokio::sync::Notify::new());
        let pressed = Arc::clone(&notify);
        let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match (method, path.ends_with("/join")) {
                ("GET", true) => {
                    pressed.notify_one();
                    Ok(json!({"status": "approved", "code": "7QK2-M9XA-3JTP-WZ4D",
                              "workspace_name": "lab", "add_device": false}))
                }
                ("POST", true) => Ok(
                    json!({"joined": true, "status": "joined", "workspace_name": "lab", "add_device": false}),
                ),
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::StreamJson, handler);
        let interrupt = async move {
            notify.notified().await;
            Ok(())
        };
        let reply = join_interrupted(&api, interrupt).await;
        assert!(matches!(reply, Reply::Streamed));
        assert_eq!(join_requests(&fake), (1, 0), "no claim after the press");
    }

    /// The wait: a press made while the status was read is kept by the one listener and ends
    /// the sleep that follows. The poll is an hour, so only the sleep's own select can hear it
    /// in time; with a fresh listener per sleep, as before CLI-18, the press was lost.
    #[tokio::test]
    async fn a_ctrl_c_during_the_status_check_ends_the_wait_that_follows() {
        let notify = Arc::new(tokio::sync::Notify::new());
        let pressed = Arc::clone(&notify);
        let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if method == "GET" && path.ends_with("/join") {
                pressed.notify_one();
                return Ok(json!({"status": "invited", "code": "7QK2-M9XA-3JTP-WZ4D",
                                 "workspace_name": "lab"}));
            }
            standard(method, path, body)
        };
        let (mut api, fake) = api_with(OutputFormat::StreamJson, handler);
        api.poll = Duration::from_secs(60 * 60);
        let interrupt = async move {
            notify.notified().await;
            Ok(())
        };
        let reply = join_interrupted(&api, interrupt).await;
        assert!(matches!(reply, Reply::Streamed));
        assert_eq!(
            join_requests(&fake),
            (1, 0),
            "no status check after the press"
        );
    }

    /// A broker that predates direct add says so, and points at the invitation that works.
    #[tokio::test]
    async fn members_add_on_an_older_broker_points_to_an_invitation() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("team.add_member") => Err(refuse_broker(
                    "unsupported",
                    "unsupported: operation is not supported by this broker",
                )),
                _ => standard(method, path, body),
            }
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let error = run(&api, add_member_command(Some("Analysis Lab"), &[]))
            .await
            .expect_err("unsupported");
        assert!(
            message(&error).contains("can't add people directly yet"),
            "{}",
            message(&error)
        );
    }

    #[tokio::test]
    async fn approve_and_cancel_send_the_username_and_check_the_code_shape() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(
            &api,
            CrewCommand::Enroll(EnrollmentCommand::Approve {
                person: "@bob".into(),
                code: "7QK2-M9XA".into(),
                replace: false,
            }),
        )
        .await
        .expect_err("too short");
        assert!(message(&error).starts_with("A code has 16 letters and digits"));
        assert!(fake.broker_calls().is_empty());

        run(
            &api,
            CrewCommand::Enroll(EnrollmentCommand::Approve {
                person: "@bob".into(),
                code: " 7qk2 m9xa 3jtp wz4d ".into(),
                replace: true,
            }),
        )
        .await
        .expect("approved");
        assert_eq!(
            fake.broker_call("enrollment.approve").expect("approve"),
            json!({"username": "bob", "code": "7qk2 m9xa 3jtp wz4d", "replace": true, "idempotency_key": "req-1"})
        );
        run(
            &api,
            CrewCommand::Enroll(EnrollmentCommand::Cancel {
                person: "bob".into(),
            }),
        )
        .await
        .expect("cancelled");
        assert_eq!(
            fake.broker_call("enrollment.cancel").expect("cancel")["username"],
            "bob"
        );
    }

    #[tokio::test]
    async fn tasks_start_says_the_institution_refusal_in_the_desktops_words() {
        const DAEMON: &str = "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution";
        for (details, expected) in [
            (
                Some(json!({"model": "gpt-5.5", "approved_for": ["ucsf"], "workspace": "foreign-lab", "workspace_institution": "stanford"})),
                "gpt-5.5 is approved for UCSF. foreign-lab uses stanford. Choose a model approved for stanford, or a local model.",
            ),
            (
                None,
                "gpt-5.5 isn't approved for this workspace's institution. Choose a model approved for it, or a local model.",
            ),
        ] {
            let (api, _) = api_with(OutputFormat::Text, move |method, path, body| {
                if path.ends_with("/runs") && method == "POST" {
                    return Err(FakeRefusal {
                        status: 400,
                        code: Some("crew_request_refused".into()),
                        broker_code: None,
                        institution_refusal: details.clone(),
                        connection_institution: None,
                        message: DAEMON.into(),
                        detail: None,
                        modes: None,
                        fields: Default::default(),
                    }
                    .into());
                }
                standard(method, path, body)
            });
            let error = run(
                &api,
                CrewCommand::Tasks(TaskCommand::Start {
                    channel: "#methods".into(),
                    prompt: TextInput {
                        text: Some("Summarize".into()),
                        input: None,
                    },
                    provider: "versa_azure".into(),
                    model: "gpt-5.5".into(),
                    context_channels: Vec::new(),
                    allow_posting: true,
                }),
            )
            .await
            .expect_err("refused");
            let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
            assert_eq!(shown, expected);
            assert!(!shown.contains("resolved affiliation"), "{shown}");
            assert!(!shown.contains("Daemon returned"), "{shown}");
            // A refusal is a definite answer: no retry line, and scripts keep the code.
            assert_eq!(error_code(&error).as_deref(), Some("crew_request_refused"));
        }
    }

    /// CLI-5: a streamed command ends on one JSON value per line, its last value and its
    /// failure included, and a finished download ends on the word the rows used.
    #[tokio::test]
    async fn streamed_commands_end_on_one_json_line_and_a_download_ends_saved() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/transfers/t-1" {
                return Ok(json!({
                    "id": "t-1", "connection_id": CONNECTION, "channel_id": METHODS,
                    "direction": "download", "state": "completed", "name": "counts.csv",
                    "size": 10, "offset": 10
                }));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, handler);
        let reply = run(
            &api,
            CrewCommand::Files(FileCommand::Watch {
                transfer: "t-1".into(),
            }),
        )
        .await
        .expect("watched");
        let Reply::Last(last, options) = reply else {
            panic!("files watch ends on its last value: {reply:?}")
        };
        assert_eq!(last["direction"], "download");
        assert_eq!(output::render_text(&last, &options), "Transfer Saved.");
        for format in [OutputFormat::Json, OutputFormat::StreamJson] {
            let line = output::formatted(&last, output::stream_format(format), &options)
                .expect("formatted");
            assert!(!line.contains('\n'), "{line}");
        }

        for command in [
            CrewCommand::Watch(WatchArgs {
                channel: "methods".into(),
                after: None,
                from_start: false,
                new_only: false,
            }),
            CrewCommand::Join(JoinArgs { no_wait: false }),
            CrewCommand::Tasks(TaskCommand::Watch { run: "r".into() }),
            CrewCommand::Files(FileCommand::Watch {
                transfer: "t".into(),
            }),
        ] {
            assert!(streams(&command));
        }
        assert!(!streams(&CrewCommand::Status));
        let lost = anyhow!("connection reset");
        let line = failure_json(&lost, "connection reset", "req-1", OutputFormat::StreamJson)
            .expect("json");
        assert!(!line.contains('\n'), "{line}");
        assert!(failure_json(&lost, "x", "req-1", OutputFormat::Json)
            .expect("json")
            .contains('\n'));
        assert!(failure_json(&lost, "x", "req-1", OutputFormat::Text).is_none());
    }

    /// DW-17: history names each attachment, in text and in JSON (`attachment_details`, beside
    /// the unchanged `attachments`), asking about each ID once; `files show ID` says which file
    /// an ID is.
    #[tokio::test]
    async fn history_names_attachments_and_files_show_says_which_file_an_id_is() {
        const CORRECTED: &str = "a1a1a1a1-0000-4000-8000-000000000001";
        const ORIGINAL: &str = "a1a1a1a1-0000-4000-8000-000000000002";
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            let body_value = body.cloned().unwrap_or_default();
            match body_value["method"].as_str() {
                Some("messages.history") => Ok(json!({"messages": [
                    {"id": "m1", "channel_id": METHODS, "actor_id": BOB, "body": "Both files.",
                     "created_at": 0, "attachments": [CORRECTED, ORIGINAL]},
                    {"id": "m2", "channel_id": METHODS, "actor_id": BOB, "body": "Again.",
                     "created_at": 0, "attachments": [CORRECTED]}
                ], "cursor": "m2", "people": {}})),
                Some("blob.status") => {
                    let id = body_value["params"]["blob_id"].as_str().unwrap_or_default();
                    let name = if id == CORRECTED {
                        "counts-fixed.csv"
                    } else {
                        "counts.csv"
                    };
                    Ok(
                        json!({"id": id, "channel_id": METHODS, "owner_id": BOB, "name": name,
                        "size": 55, "media_type": "text/csv", "complete": true}),
                    )
                }
                _ => standard(method, path, body),
            }
        };
        let (api, fake) = api_with(OutputFormat::Json, handler);
        let Reply::Show(page, _) = run(&api, history("methods")).await.expect("history") else {
            panic!("history shows a page")
        };
        assert_eq!(
            page["messages"][0]["attachments"],
            json!([CORRECTED, ORIGINAL])
        );
        assert_eq!(
            page["messages"][0]["attachment_details"][ORIGINAL],
            json!({"name": "counts.csv", "size": 55, "media_type": "text/csv"})
        );
        let lookups = fake
            .broker_calls()
            .into_iter()
            .filter(|(method, _)| method == "blob.status")
            .count();
        assert_eq!(lookups, 2, "each ID is asked about once");

        let (mut api, _) = api_with(OutputFormat::Text, handler);
        api.show_ids = true;
        let text = said(run(&api, history("methods")).await.expect("history")).join("\n");
        assert!(
            text.contains(&format!("    Attachment: counts-fixed.csv (55 B) [attachment ID {CORRECTED}]\n    Attachment: counts.csv (55 B) [attachment ID {ORIGINAL}]")),
            "{text}"
        );

        let (api, _) = api_with(OutputFormat::Text, handler);
        let shown = said(
            run(
                &api,
                CrewCommand::Files(FileCommand::Show {
                    attachment: ORIGINAL.into(),
                }),
            )
            .await
            .expect("files show"),
        );
        assert_eq!(
            shown,
            ["counts.csv · 55 B · text/csv · in #methods · shared by \"Bob Lee\" (@bob)"]
        );
    }

    /// AG-F7: `crew context` leads with the grant's scope in text, taking the channel it posts
    /// in from the grants list, and prints the manifest as it is in JSON.
    #[tokio::test]
    async fn crew_context_leads_with_the_grants_scope() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with(&format!("/sessions/{SESSION}/context")) {
                return Ok(json!({"run_id": "r", "source_channels": [METHODS, GENERAL],
                    "messages": [], "people": {}, "channel_names": {}}));
            }
            if path.ends_with("/grants") {
                return Ok(
                    json!({"grants": [{"session_id": SESSION, "channel_id": METHODS,
                    "source_channels": [METHODS, GENERAL], "expired": false}]}),
                );
            }
            standard(method, path, body)
        };
        let command = || CrewCommand::Context {
            session: SESSION.into(),
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(run(&api, command()).await.expect("context"));
        assert_eq!(
            lines,
            [
                "Access: #methods in Analysis Lab · also reads #general in Analysis Lab",
                "No messages."
            ]
        );
        let (api, _) = api_with(OutputFormat::Json, handler);
        let Reply::Show(manifest, _) = run(&api, command()).await.expect("context") else {
            panic!("JSON is the manifest")
        };
        assert_eq!(manifest["source_channels"], json!([METHODS, GENERAL]));
    }

    /// AGT2-N3: `grants list` reads each task's status, so a task the person stopped reads
    /// Stopped and one that finished reads Ended; JSON carries it as `task_status`.
    #[tokio::test]
    async fn a_stopped_task_reads_stopped_in_the_grants_list() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/grants") {
                return Ok(json!({"grants": [
                    {"session_id": "20260924_4", "run_id": "run-stopped", "kind": "task", "channel_id": METHODS, "source_channels": [METHODS], "expired": true, "revocation": "confirmed"},
                    {"session_id": "20260924_5", "run_id": "run-done", "kind": "task", "channel_id": METHODS, "source_channels": [METHODS], "expired": true, "revocation": "confirmed"},
                    {"session_id": SESSION, "run_id": "run-chat", "kind": "chat", "channel_id": METHODS, "source_channels": [METHODS], "expired": true, "revocation": "confirmed"}
                ], "replaced_grants": []}));
            }
            if path.ends_with("/runs") {
                return Ok(json!({"runs": [
                    {"run_id": "run-stopped", "status": "cancelled", "session_id": "20260924_4"},
                    {"run_id": "run-done", "status": "completed", "session_id": "20260924_5"}
                ]}));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(&api, CrewCommand::Grants(GrantCommand::List))
                .await
                .expect("listed"),
        )
        .join("\n");
        assert!(
            lines.contains("Task 20260924_4 → #methods in Analysis Lab · Stopped"),
            "{lines}"
        );
        assert!(
            lines.contains("Task 20260924_5 → #methods in Analysis Lab · Ended"),
            "{lines}"
        );
        assert!(
            lines.contains("Chat 20260924_2 → #methods in Analysis Lab · Revoked"),
            "{lines}"
        );
        let (api, _) = api_with(OutputFormat::Json, handler);
        let Reply::Show(value, _) = run(&api, CrewCommand::Grants(GrantCommand::List))
            .await
            .expect("listed")
        else {
            panic!("a value");
        };
        assert_eq!(value["grants"][0]["task_status"], "cancelled");
        assert_eq!(value["grants"][1]["task_status"], "completed");
        assert!(
            value["grants"][2].get("task_status").is_none(),
            "a chat has no task"
        );
    }

    /// CLIDOCS-F12: an empty list says what it lists, never "No items.".
    #[tokio::test]
    async fn an_empty_list_names_what_it_lists() {
        let empty = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["teams"] = json!([]);
                snapshot["channels"] = json!([]);
                return Ok(snapshot);
            }
            standard(method, path, body)
        };
        for (command, sentence) in [
            (
                CrewCommand::Invites(InvitationCommand::List),
                "No pending invitations.",
            ),
            (CrewCommand::Teams(TeamCommand::List), "No teams."),
            (
                CrewCommand::Channels(ChannelCommand::List { team: None }),
                "No channels.",
            ),
        ] {
            let (api, _) = api_with(OutputFormat::Text, empty);
            let lines = said(run(&api, command).await.expect("listed"));
            assert_eq!(lines, [sentence]);
        }
    }

    /// CLIDOCS-F10, CLIDOCS-F6: `files forget` says what it did in the terminal's words, and an
    /// ID this computer has no receipt for says so and where to look; JSON keeps the daemon's.
    #[tokio::test]
    async fn forgetting_and_an_unknown_transfer_are_said_for_a_person() {
        const TRANSFER: &str = "7ea55000-0000-4000-8000-00000000000d";
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match (method, path) {
                ("GET", path) if path == format!("/crew/transfers/{TRANSFER}") => Ok(
                    json!({"id": TRANSFER, "connection_id": CONNECTION, "channel_id": METHODS,
                        "direction": "upload", "state": "needs_file_selection", "name": "counts.csv"}),
                ),
                ("DELETE", path) if path == format!("/crew/transfers/{TRANSFER}") => Ok(json!({
                    "forgotten": true,
                    "message": "Receipt removed after authorized partial cleanup. Remote attachments and published downloads are not deleted."
                })),
                ("GET", path) if path.starts_with("/crew/transfers/") => Err(refuse(
                    400,
                    Some("crew_request_refused"),
                    "Unknown transfer",
                )),
                _ => standard(method, path, body),
            }
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let lines = said(
            run(
                &api,
                CrewCommand::Files(FileCommand::Forget {
                    transfer: TRANSFER.into(),
                    file: None,
                }),
            )
            .await
            .expect("forgotten"),
        );
        assert_eq!(
            lines,
            ["Removed from your list. Files already shared stay in the channel."]
        );
        let (api, _) = api_with(OutputFormat::Json, handler);
        let Reply::Say(value, _) = run(
            &api,
            CrewCommand::Files(FileCommand::Forget {
                transfer: TRANSFER.into(),
                file: None,
            }),
        )
        .await
        .expect("forgotten") else {
            panic!("a sentence");
        };
        assert_eq!(value["forgotten"], true, "JSON is the daemon's answer");

        for command in [
            FileCommand::Status {
                transfer: "7ea55000-0000-4000-8000-0000000000ff".into(),
            },
            FileCommand::Pause {
                transfer: "7ea55000-0000-4000-8000-0000000000ff".into(),
            },
        ] {
            let (api, _) = api_with(OutputFormat::Text, handler);
            let error = run(&api, CrewCommand::Files(command))
                .await
                .expect_err("unknown");
            assert_eq!(
                failure(&error, OutputFormat::Text, "req-1", false).to_string(),
                files::UNKNOWN_TRANSFER
            );
            assert_eq!(error_code(&error).as_deref(), Some("crew_request_refused"));
        }
    }

    /// SF2-N4: `context` for a chat whose access ended says why and the command that grants it
    /// again, naming the chat and its channel, under the daemon's `crew_grant_ended`; an older
    /// daemon's untyped settings-changed refusal is said the same way.
    #[tokio::test]
    async fn context_after_access_ended_says_how_to_grant_it_again() {
        const SETTINGS_CHANGED: &str =
            "Crew settings changed since access was granted. Grant access again from Crew.";
        let ended = |code: &'static str, reason: Option<&'static str>| {
            move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if path.ends_with(&format!("/sessions/{SESSION}/context")) {
                    let mut fields = serde_json::Map::new();
                    if let Some(reason) = reason {
                        fields.insert("reason".into(), json!(reason));
                    }
                    return Err(FakeRefusal {
                        status: 409,
                        code: Some(code.into()),
                        broker_code: None,
                        institution_refusal: None,
                        connection_institution: None,
                        message: SETTINGS_CHANGED.into(),
                        detail: None,
                        modes: None,
                        fields,
                    }
                    .into());
                }
                if path.ends_with("/grants") {
                    return Ok(
                        json!({"grants": [{"session_id": SESSION, "channel_id": METHODS,
                        "source_channels": [METHODS], "expired": true}]}),
                    );
                }
                standard(method, path, body)
            }
        };
        let context = || CrewCommand::Context {
            session: SESSION.into(),
        };
        for (code, reason, sentence) in [
            (GRANT_ENDED, Some("settings_changed"), "This chat's Crew access ended because Crew settings changed. Run biorouter crew grants grant 20260924_2 methods to grant it again."),
            (GRANT_ENDED, Some("ended"), "This chat's Crew access ended. Run biorouter crew grants grant 20260924_2 methods to grant it again."),
            ("crew_profile_refused", None, "This chat's Crew access ended because Crew settings changed. Run biorouter crew grants grant 20260924_2 methods to grant it again."),
        ] {
            let (api, _) = api_with(OutputFormat::Text, ended(code, reason));
            let error = run(&api, context()).await.expect_err("ended");
            let shown = failure(&error, OutputFormat::Text, "req-1", false).to_string();
            assert_eq!(shown, sentence, "{code} {reason:?}");
            assert!(!shown.contains("from Crew"), "{shown}");
            assert_eq!(error_code(&error).as_deref(), Some(code));
        }
        // A settings refusal that is not this one keeps its own sentence.
        let (api, _) = api_with(
            OutputFormat::Text,
            |method: &str, path: &str, body: Option<&Value>| {
                if path.ends_with("/context") {
                    return Err(refuse(
                        400,
                        Some("crew_profile_refused"),
                        "Crew couldn't read its saved settings.",
                    ));
                }
                standard(method, path, body)
            },
        );
        let error = run(&api, context()).await.expect_err("refused");
        assert_eq!(
            failure(&error, OutputFormat::Text, "req-1", false).to_string(),
            "Crew couldn't read its saved settings."
        );
    }

    /// DW-07: `files watch` names each receipt's channel as `files status` does, and shows
    /// its IDs with `--show-ids`.
    #[tokio::test]
    async fn files_watch_rows_are_named_like_files_status() {
        let (mut api, _) = api_with(OutputFormat::Text, standard);
        api.show_ids = true;
        let options = files::watch_options(&api).await;
        let receipt = json!({"id": "t-1", "connection_id": CONNECTION, "channel_id": METHODS,
            "direction": "download", "state": "downloading", "name": "counts.csv",
            "size": 10, "offset": 5});
        let row = output::render_text(&receipt, &options);
        assert!(
            row.starts_with(&format!(
                "counts.csv · download from #methods [channel ID {METHODS}] · Downloading 50%"
            )),
            "{row}"
        );
        assert!(row.contains("[transfer ID t-1]"), "{row}");
    }

    /// R-8, AG-F17: offline, `grants list` and `files pending` say the names are unavailable
    /// and show each channel's ID, rather than "this channel" on every row.
    #[tokio::test]
    async fn offline_lists_say_names_are_unavailable() {
        let offline = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                return Err(refuse(409, Some("crew_not_connected"), "Not connected."));
            }
            if path.ends_with("/grants") {
                return Ok(json!({"grants": [{"session_id": SESSION, "run_id": "r",
                    "connection_id": CONNECTION, "channel_id": METHODS,
                    "source_channels": [METHODS], "expired": false}]}));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, offline);
        let lines = said(
            run(&api, CrewCommand::Grants(GrantCommand::List))
                .await
                .expect("listed offline"),
        );
        assert_eq!(
            lines,
            [format!("Chat 20260924_2 → a channel in UCSF HPC (names unavailable while disconnected) [channel ID {METHODS}] · Active")]
        );

        // W2-CLI-9, CLIDOCS-F6: an older daemon's untyped refusal is the same fact.
        let older = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                return Err(refuse(
                    400,
                    Some("crew_request_refused"),
                    LEGACY_DISCONNECTED,
                ));
            }
            offline(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, older);
        let lines = said(
            run(&api, CrewCommand::Grants(GrantCommand::List))
                .await
                .expect("listed offline"),
        );
        assert!(
            lines[0].contains("(names unavailable while disconnected)"),
            "{lines:?}"
        );
    }

    /// CLIDOCS-F6: a request on a disconnected connection names the connection and the
    /// command that connects it, whether the daemon typed the refusal (`crew_not_connected`)
    /// or sent an older daemon's untyped sentence; JSON keeps the daemon's code.
    #[tokio::test]
    async fn a_disconnected_connection_is_named_with_the_command_that_connects_it() {
        for (status, code) in [(409, NOT_CONNECTED), (400, "crew_request_refused")] {
            let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if path.ends_with("/request") {
                    return Err(refuse(status, Some(code), LEGACY_DISCONNECTED));
                }
                standard(method, path, body)
            };
            let (api, _) = api_with(OutputFormat::Text, handler);
            let error = run(&api, history("methods"))
                .await
                .expect_err("disconnected");
            let error = api.worded_failure(error).await;
            let shown = failure(&error, OutputFormat::Text, "req-1", false).to_string();
            assert_eq!(
                shown, "UCSF HPC is disconnected. Run biorouter crew connect, then try again.",
                "{code}"
            );
            assert!(!shown.contains("Daemon returned"), "{shown}");
            assert!(!shown.contains("in Crew"), "{shown}");
            assert_eq!(error_code(&error).as_deref(), Some(code));

            // With --connection, the command to run names it too.
            let (mut api, _) = api_with(OutputFormat::Text, handler);
            api.selected = Some(CONNECTION.into());
            let error = run(&api, history("methods"))
                .await
                .expect_err("disconnected");
            let shown = failure(
                &api.worded_failure(error).await,
                OutputFormat::Text,
                "req-1",
                false,
            )
            .to_string();
            assert_eq!(
                shown,
                "UCSF HPC is disconnected. Run biorouter crew --connection 'UCSF HPC' connect, then try again."
            );
        }
        // Where the command could not name the connection, the sentence still says what to do.
        let bare = refuse(409, Some(NOT_CONNECTED), LEGACY_DISCONNECTED);
        assert_eq!(
            failure(&bare, OutputFormat::Text, "req-1", false).to_string(),
            NOT_CONNECTED_SENTENCE
        );
    }

    /// CLI-6: a watch starts at the newest page by default, after the newest message with
    /// `--new-only`, at the oldest with `--from-start`, and at a cursor with `--after`.
    #[tokio::test]
    async fn a_watch_starts_at_the_newest_messages_unless_asked_otherwise() {
        let start = |after: Option<&str>, from_start: bool, new_only: bool| WatchArgs {
            channel: "methods".into(),
            after: after.map(str::to_owned),
            from_start,
            new_only,
        };
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let shape = |(cursor, initial): (Option<String>, Initial)| {
            (cursor, serde_json::to_value(initial).expect("initial"))
        };
        assert_eq!(
            shape(
                watch_start(&api, METHODS, &start(None, false, false))
                    .await
                    .unwrap()
            ),
            (None, json!("latest"))
        );
        assert_eq!(
            shape(
                watch_start(&api, METHODS, &start(None, true, false))
                    .await
                    .unwrap()
            ),
            (None, json!("all"))
        );
        assert_eq!(
            shape(
                watch_start(&api, METHODS, &start(Some("m-3"), false, false))
                    .await
                    .unwrap()
            ),
            (Some("m-3".into()), json!("all"))
        );
        assert!(fake.broker_calls().is_empty(), "no history read so far");
        assert_eq!(
            shape(
                watch_start(&api, METHODS, &start(None, false, true))
                    .await
                    .unwrap()
            ),
            (Some("m-9".into()), json!("all"))
        );
        assert_eq!(
            fake.broker_call("messages.history").expect("newest"),
            json!({"channel_id": METHODS, "latest": true, "limit": 1})
        );
    }

    /// CLI-5: a watch the daemon ended prints one error value, the frame and the failure in
    /// one, not a frame followed by a second error object.
    #[tokio::test]
    async fn a_stopped_watch_is_one_error_value() {
        let (api, _) = api_with(OutputFormat::StreamJson, standard);
        let state = std::sync::Mutex::new(WatchState::default());
        let error = match watch_event(
            &api,
            &state,
            "#methods",
            ObserveEvent::Error {
                code: "channel_access_changed".into(),
                error: "You no longer have access to this channel".into(),
                clear: true,
            },
        )
        .await
        {
            Ok(_) => panic!("an error frame ends the watch"),
            Err(error) => error,
        };
        let message = safe_lines(&error_text(&error));
        assert_eq!(
            message,
            "Stopped watching #methods: You no longer have access to this channel."
        );
        let body = failure_body(&error, &message, "req-1");
        assert_eq!(
            body,
            json!({"type": "error", "code": "channel_access_changed", "clear": true,
                   "error": message, "request_id": "req-1"})
        );
    }

    #[test]
    fn crew_watch_stops_in_a_sentence_without_a_code_or_the_cursor() {
        assert_eq!(
            watch_stopped("#general", "You no longer have access to this channel"),
            "Stopped watching #general: You no longer have access to this channel."
        );
        assert_eq!(
            watch_stopped(
                "#general",
                "Your access to a channel in this workspace changed."
            ),
            "Stopped watching #general: Your access to a channel in this workspace changed."
        );
        assert_eq!(watch_stopped("#general", " "), "Stopped watching #general.");
        // CLI-4: a label the CLI isolated is printed once, not escaped again.
        let stopped = watch_stopped("\u{2068}#données\u{2069}", "Live updates stopped");
        assert_eq!(
            stopped,
            "Stopped watching \u{2068}#données\u{2069}: Live updates stopped."
        );
        assert_eq!(safe_lines(&stopped), stopped);
        let text = watch_stopped("#general", "Live updates stopped");
        assert!(!text.contains('['), "{text}");
        assert!(!text.to_ascii_lowercase().contains("cursor"), "{text}");
    }

    #[tokio::test]
    async fn a_broker_refusal_is_said_in_its_own_words_with_the_flag_that_acts_on_it() {
        let approved = "already_approved: You already entered a code for @bob. If it didn't match their computer, enter the code they sent and choose Replace.";
        let (api, _) = api_with(OutputFormat::Text, move |method, path, body| {
            if path.ends_with("/request")
                && body.and_then(|body| body["method"].as_str()) == Some("enrollment.approve")
            {
                return Err(refuse_broker("already_approved", approved));
            }
            standard(method, path, body)
        });
        let error = run(
            &api,
            CrewCommand::Enroll(EnrollmentCommand::Approve {
                person: "@bob".into(),
                code: "7QK2-M9XA-3JTP-WZ4D".into(),
                replace: false,
            }),
        )
        .await
        .expect_err("already approved");
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert_eq!(
            shown,
            "You already entered a code for @bob.\nIf it didn't match their computer, run: biorouter crew enroll approve @bob <code> --replace"
        );
        assert!(!shown.contains("Daemon returned"), "{shown}");
        assert!(!shown.contains("already_approved:"), "{shown}");
        // A refusal is a definite answer, so no retry is offered, and scripts keep both codes.
        assert!(!shown.contains("req-1"));
        assert_eq!(error_code(&error).as_deref(), Some("crew_request_refused"));
        assert_eq!(
            error.chain().find_map(broker_refusal).map(|(code, _)| code),
            Some("already_approved")
        );

        // Without a broker code (another route's refusal), the daemon's own sentence is
        // printed as it is, without the status (FILES2-N4).
        let other = refuse(
            404,
            Some("crew_grant_not_found"),
            "No Crew grant for this session.",
        );
        assert_eq!(
            failure(&other, OutputFormat::Text, "req-1", false).to_string(),
            "No Crew grant for this session."
        );
    }

    /// FILES2-N4, CLIDOCS-F6: a daemon refusal with a code and a sentence of its own prints
    /// that sentence alone, whatever the code; only a refusal with no code keeps the
    /// `Daemon returned NNN:` prefix. JSON keeps the code, and the sentence is its `error`.
    #[test]
    fn a_typed_daemon_refusal_prints_its_own_sentence_without_the_status() {
        let shown =
            |error: &anyhow::Error| failure(error, OutputFormat::Text, "req-1", false).to_string();
        for (code, sentence) in [
            ("crew_folder_shared", "Choose a folder owned by your account that other accounts can't change."),
            ("crew_file_name_hidden", "“.Rprofile” starts with a dot, which Crew doesn't save into your home. Choose a name without the leading dot."),
            ("crew_file_is_program", "“run.sh” is a program. Choose another name, or save it in another folder."),
            ("crew_file_is_credential", "“id_ed25519” looks like a credential file. Crew doesn't share credential files."),
            ("crew_profile_refused", "Crew couldn't read its saved settings."),
            ("crew_transfer_refused", "That file selection expired. Select the file again."),
            ("crew_invitation_conflict", "This invitation doesn't match “lab”, which this computer already has for the same workspace. Ask your host to send it again, and compare the fingerprint."),
            ("crew_grant_not_found", "No Crew grant for this session."),
            ("crew_connection_not_found", "No such connection."),
        ] {
            let error = refuse(400, Some(code), sentence);
            let text = shown(&error);
            assert_eq!(text, sentence, "{code}");
            assert!(!text.contains("Daemon returned"), "{code}: {text}");
            let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
            assert_eq!(body["code"], code);
            assert_eq!(body["error"], sentence);
        }
        // An existing file names the flag that replaces it, not the desktop's Replace button.
        let exists = refuse(
            400,
            Some("crew_destination_exists"),
            "A file named “counts.csv” already exists. Replace it, or choose another name.",
        );
        assert_eq!(
            shown(&exists),
            "A file named “counts.csv” already exists. Add --overwrite to replace it, or save it under another name."
        );
        // A refusal with no code has no sentence of its own the CLI can vouch for.
        let untyped = refuse(400, None, "Something went wrong");
        assert_eq!(shown(&untyped), "Daemon returned 400: Something went wrong");
        // An empty sentence keeps the status too.
        let empty = refuse(409, Some("crew_request_refused"), "  ");
        assert!(shown(&empty).starts_with("Daemon returned 409:"));
    }

    /// SF2-N6, CLIDOCS-F8: a command with several saved connections and no `--connection`, and
    /// `auth` with no terminal, are usage refusals: exit status 2 (`NeedsTerminal` is what
    /// `main` exits 2 on), a code in JSON, and nothing sent.
    #[tokio::test]
    async fn a_missing_connection_choice_and_auth_without_a_terminal_exit_with_the_usage_status() {
        let two = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if (method, path) == ("GET", "/crew/connections") {
                return Ok(json!({"connections": [
                    {"id": CONNECTION, "name": "UCSF HPC", "ssh_target": "bob@hpc", "workspace_id": "w", "status": "connected"},
                    {"id": "c0ffee00-0000-4000-8000-000000000009", "name": "Stanford", "ssh_target": "bob@su", "workspace_id": "v", "status": "connected"}
                ]}));
            }
            standard(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Json, two);
        let error = run(&api, history("methods")).await.expect_err("which one?");
        assert!(fake.broker_calls().is_empty(), "nothing was sent");
        assert_eq!(error_code(&error).as_deref(), Some(CONNECTION_REQUIRED));
        let exit = failure(&error, OutputFormat::Json, "req-1", false);
        assert!(exit.downcast_ref::<NeedsTerminal>().is_some(), "exit 2");
        assert!(exit.to_string().contains("--connection NAME"), "{exit}");
        let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
        assert_eq!(body["code"], CONNECTION_REQUIRED);

        let auth: anyhow::Error =
            needs_terminal::require(false, crate::daemon_client::AUTH_NEEDS_A_TERMINAL)
                .unwrap_err()
                .into();
        let exit = failure(&auth, OutputFormat::Json, "req-1", false);
        assert!(exit.downcast_ref::<NeedsTerminal>().is_some(), "exit 2");
        let body = failure_body(&auth, &safe_lines(&error_text(&auth)), "req-1");
        assert_eq!(body["code"], NEEDS_TERMINAL_CODE);
        assert!(body["error"]
            .as_str()
            .is_some_and(|text| text.contains("biorouter crew connect")));
    }

    /// AGT2-N5: a JSON failure carries the daemon's structured refusal fields, so a script
    /// reads the modes or institutions that failed without parsing the sentence. What the CLI
    /// writes itself (`error`, `request_id`, `code`) is never replaced by the daemon's.
    #[test]
    fn a_json_failure_keeps_the_daemons_structured_refusal_fields() {
        let mode: anyhow::Error = DaemonRefusal::for_test(
            400,
            json!({
                "code": "crew_mode_mismatch",
                "error": "Crew refused this.",
                "actual_mode": "private",
                "expected_mode": "public",
                "request_id": "the-daemons-own",
            }),
        )
        .into();
        let body = failure_body(&mode, &safe_lines(&error_text(&mode)), "req-1");
        assert_eq!(body["code"], "crew_mode_mismatch");
        assert_eq!(body["actual_mode"], "private");
        assert_eq!(body["expected_mode"], "public");
        assert_eq!(body["request_id"], "req-1", "the CLI's own request ID");
        assert_eq!(
            body["error"],
            "Your connection is Private, but this request required Public. Nothing was sent."
        );

        let details = json!({"model": "gpt-5.5", "approved_for": ["ucsf"], "workspace": "okafor-lab", "workspace_institution": "stanford-synthetic"});
        let institution: anyhow::Error = DaemonRefusal::for_test(
            400,
            json!({
                "code": "crew_institution_mismatch",
                "error": "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution",
                "institution_refusal": details,
                "workspace": "okafor-lab",
            }),
        )
        .into();
        // Worded by a command, the refusal is still its source, so its fields still come out.
        let worded = anyhow::Error::from(Worded {
            sentence: "Said for a person.".into(),
            source: institution,
        });
        let body = failure_body(&worded, &safe_lines(&error_text(&worded)), "req-1");
        assert_eq!(body["institution_refusal"], details);
        assert_eq!(body["workspace"], "okafor-lab");
        assert_eq!(body["code"], "crew_institution_mismatch");
        assert_eq!(body["error"], "Said for a person.");

        // T3-CLI-10: a save refused because its institution is not the workspace's own
        // (T3-BE-4) keeps `workspace_institution`, the one institution a script can retry with.
        let save: anyhow::Error = DaemonRefusal::for_test(
            400,
            json!({
                "code": "crew_institution_mismatch",
                "error": "This connection is for ucsf, but okafor-lab belongs to stanford-synthetic. Use stanford-synthetic here.",
                "connection_institution": "ucsf",
                "workspace_institution": "stanford-synthetic",
                "workspace": "okafor-lab",
            }),
        )
        .into();
        let text = safe_lines(&error_text(&save));
        assert_eq!(
            text,
            "This connection is for ucsf, but okafor-lab belongs to stanford-synthetic. Use stanford-synthetic here."
        );
        let body = failure_body(&save, &text, "req-1");
        assert_eq!(body["code"], "crew_institution_mismatch");
        assert_eq!(body["workspace_institution"], "stanford-synthetic");
        assert_eq!(body["connection_institution"], "ucsf");
        assert_eq!(body["workspace"], "okafor-lab");

        // AGT2-N6: a refusal's diagnostic detail reaches JSON, and never the sentence.
        let unreadable: anyhow::Error = DaemonRefusal::for_test(
            409,
            json!({
                "code": "crew_registry_unreadable",
                "error": "Crew's saved settings can't be read, so nothing was changed. Update Biorouter, then try again.",
                "detail": "unknown variant `bogus_future_variant`, expected one of `chat`, `task`",
            }),
        )
        .into();
        let text = safe_lines(&error_text(&unreadable));
        assert_eq!(
            text,
            "Crew's saved settings can't be read, so nothing was changed. Update Biorouter, then try again."
        );
        let body = failure_body(&unreadable, &text, "req-1");
        assert_eq!(body["code"], "crew_registry_unreadable");
        assert_eq!(
            body["detail"],
            "unknown variant `bogus_future_variant`, expected one of `chat`, `task`"
        );
    }

    /// T3-CLI-10: every field of the daemon's `CrewError`, as the generated OpenAPI document
    /// describes it, is either copied into a JSON failure (`REFUSAL_FIELDS`) or written by the
    /// CLI itself (`CLI_WRITTEN_FIELDS`). A field the daemon adds then fails here instead of
    /// silently leaving `--output-format json`, as `workspace_institution` did. The reverse
    /// holds too, so a misspelt entry that could never match is caught.
    #[test]
    fn every_crew_error_field_reaches_json_output_or_is_written_by_the_cli() {
        use crate::daemon_client::{CLI_WRITTEN_FIELDS, REFUSAL_FIELDS};
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/desktop/openapi.json");
        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
        )
        .expect("openapi.json is JSON");
        let properties = document["components"]["schemas"]["CrewError"]["properties"]
            .as_object()
            .expect("openapi.json describes CrewError's properties");
        assert!(
            properties.len() > CLI_WRITTEN_FIELDS.len(),
            "CrewError has only {} properties",
            properties.len()
        );
        let dropped: Vec<&str> = properties
            .keys()
            .map(String::as_str)
            .filter(|key| !REFUSAL_FIELDS.contains(key) && !CLI_WRITTEN_FIELDS.contains(key))
            .collect();
        assert!(
            dropped.is_empty(),
            "CrewError fields a JSON failure would drop: {dropped:?}. Add each to REFUSAL_FIELDS in daemon_client.rs, or write it in failure_body and list it in CLI_WRITTEN_FIELDS."
        );
        let unknown: Vec<&str> = REFUSAL_FIELDS
            .iter()
            .chain(CLI_WRITTEN_FIELDS)
            .copied()
            .filter(|key| !properties.contains_key(*key))
            .collect();
        assert!(
            unknown.is_empty(),
            "fields the CLI keeps that CrewError does not have: {unknown:?}"
        );
        for key in REFUSAL_FIELDS {
            assert!(
                !CLI_WRITTEN_FIELDS.contains(key),
                "{key} is both copied and written by the CLI"
            );
        }
    }

    /// CLI-7: a connect the server refused is said in words with what to run, then the code
    /// the manual's table is keyed on and OpenSSH's own words; JSON keeps the code and detail.
    #[tokio::test]
    async fn a_refused_connect_says_what_to_run_with_the_code_and_details() {
        const TRANSPORT: &str = "Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: ssh exited; reconnect. Submitted operation outcome may be unknown; inspect history before retrying";
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/connect") {
                return Err(FakeRefusal {
                    status: 400,
                    code: Some("crew_ssh_auth_required".into()),
                    broker_code: None,
                    institution_refusal: None,
                    connection_institution: None,
                    message: TRANSPORT.into(),
                    detail: Some(
                        "bob@hpc: Permission denied (publickey,password).\nsecond line".into(),
                    ),
                    modes: None,
                    fields: Default::default(),
                }
                .into());
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, handler);
        let error = run(&api, CrewCommand::Connect).await.expect_err("refused");
        let shown = failure(&error, OutputFormat::Text, "req-1", false).to_string();
        assert_eq!(
            shown,
            "The server wants your password or a verification code. Run biorouter crew auth to sign in.\n  Code: crew_ssh_auth_required\n  Details: bob@hpc: Permission denied (publickey,password).\n    second line"
        );
        assert!(!shown.contains("Submitted operation"), "{shown}");
        let body = failure_body(&error, &shown, "req-1");
        assert_eq!(body["code"], "crew_ssh_auth_required");
        assert_eq!(
            body["detail"],
            "bob@hpc: Permission denied (publickey,password).\nsecond line"
        );

        for code in [
            "crew_ssh_auth_required",
            "crew_ssh_key_refused",
            "crew_broker_not_running",
            "crew_ssh_host_key_unknown",
            "crew_ssh_host_key_changed",
            "crew_ssh_unreachable",
            "crew_bridge_missing",
            "crew_ssh_failed",
            "crew_workspace_identity_mismatch",
        ] {
            let text = output::connect_failure_text(code).expect(code);
            assert!(text.ends_with('.'), "{code}: {text}");
        }
        assert!(output::connect_failure_text("crew_request_refused").is_none());
        // Another refusal keeps its own sentence.
        let other = refuse(
            404,
            Some("crew_connection_not_found"),
            "No such connection.",
        );
        assert_eq!(
            failure(&other, OutputFormat::Text, "req-1", false).to_string(),
            "No such connection."
        );
    }

    /// A daemon whose broker refuses `method` with `code: text`, as a daemon with `broker_code`
    /// forwards it; `blob.status` places every attachment in #general.
    fn refusing(
        method: &'static str,
        code: &'static str,
        text: &'static str,
    ) -> impl Fn(&str, &str, Option<&Value>) -> Result<Value> {
        move |method_: &str, path: &str, body: Option<&Value>| match body
            .and_then(|body| body["method"].as_str())
        {
            Some(called) if called == method => Err(refuse_broker(code, text)),
            Some("blob.status") => {
                Ok(json!({"id": "b", "channel_id": GENERAL, "name": "counts.csv"}))
            }
            _ => standard(method_, path, body),
        }
    }

    /// DW-11, M20, FILES-F9: a refusal names the channel the command acted on, and where a
    /// file came from, in text; JSON keeps both codes.
    #[tokio::test]
    async fn a_refusal_names_the_channel_the_command_acted_on() {
        let shown =
            |error: &anyhow::Error| failure(error, OutputFormat::Text, "req-1", true).to_string();
        let (api, _) = api_with(
            OutputFormat::Text,
            refusing(
                "channel.rename",
                "forbidden",
                "forbidden: current owner required",
            ),
        );
        let error = run(
            &api,
            CrewCommand::Channels(ChannelCommand::Rename {
                channel: "methods".into(),
                name: "methods-2".into(),
            }),
        )
        .await
        .expect_err("not the owner");
        assert_eq!(shown(&error), "Only #methods's owner can do this.");
        let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
        assert_eq!(body["code"], "crew_request_refused");
        assert_eq!(body["broker_code"], "forbidden");
        assert_eq!(body["error"], "Only #methods's owner can do this.");

        // An ID the resolver never saw is named from the person's snapshot.
        let (api, _) = api_with(
            OutputFormat::Text,
            refusing(
                "message.post",
                "channel_archived",
                "channel_archived: channel is read-only",
            ),
        );
        let send = |channel: &str, attachments: Vec<String>| {
            CrewCommand::Send(SendArgs {
                channel: channel.into(),
                text: Some("hi".into()),
                input: None,
                attachments,
                references: Vec::new(),
            })
        };
        let error = run(&api, send(METHODS, Vec::new()))
            .await
            .expect_err("archived");
        assert_eq!(shown(&error), "#methods is archived, so it's read-only.");

        let (api, _) = api_with(
            OutputFormat::Text,
            refusing(
                "message.post",
                "forbidden",
                "forbidden: attachment provenance cannot be dropped",
            ),
        );
        let error = run(&api, send("methods", vec!["b".into()]))
            .await
            .expect_err("shared elsewhere");
        assert_eq!(
            shown(&error),
            "That file was shared in #general. Share it there, or upload it again here."
        );
    }

    /// R-2: a server that can no longer save tells its host what to do, and a member whom to
    /// ask; neither is told to "restart and recover".
    #[tokio::test]
    async fn a_server_that_cannot_save_tells_the_host_what_to_do_and_members_whom_to_ask() {
        const WEDGED: &str = "storage_failed: restart and recover before further mutations";
        let (api, _) = api_with(
            OutputFormat::Text,
            refusing("message.post", "storage_failed", WEDGED),
        );
        let post = || {
            CrewCommand::Send(SendArgs {
                channel: "methods".into(),
                text: Some("hi".into()),
                input: None,
                attachments: Vec::new(),
                references: Vec::new(),
            })
        };
        let error = run(&api, post()).await.expect_err("wedged");
        // Alice hosts lab (her UID is the host's), on the server her login names.
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert!(
            shown.starts_with("The workspace server can't save changes right now.\nYou host this workspace. Free space on hpc if it is full, then restart Crew there"),
            "{shown}"
        );
        assert!(!shown.contains("--request-id"), "a refusal: {shown}");

        // W2-BRK-3: a change that may have been saved offers the one safe retry.
        const MAYBE: &str = "storage_full: The workspace server ran out of disk space while saving this change, so it may not have been saved. Reading still works. Ask the host to free space on the server and restart Crew.";
        let (api, _) = api_with(
            OutputFormat::Text,
            refusing("message.post", "storage_full", MAYBE),
        );
        let error = run(&api, post()).await.expect_err("maybe saved");
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert!(
            shown.ends_with("Retry safely with --request-id req-1"),
            "{shown}"
        );

        let member = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            match body.and_then(|body| body["method"].as_str()) {
                Some("workspace.snapshot") => {
                    let mut snapshot = snapshot();
                    snapshot["actor"] = json!({"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001});
                    Ok(snapshot)
                }
                Some("message.post") => Err(refuse_broker("storage_failed", WEDGED)),
                _ => standard(method, path, body),
            }
        };
        let (api, _) = api_with(OutputFormat::Text, member);
        let error = run(&api, post()).await.expect_err("wedged");
        assert_eq!(
            failure(&error, OutputFormat::Text, "req-1", true).to_string(),
            "The workspace server can't save changes right now. Ask \"Alice Chen\" (@alice) to restart Crew."
        );
    }

    /// MSG2-N7: a message over the workspace's 64 KB limit is refused before anything is sent,
    /// as a usage error with the size and what to do instead; the broker's own refusal of one
    /// is said in the same words.
    #[tokio::test]
    async fn a_message_too_long_is_refused_before_it_is_sent() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(
            &api,
            CrewCommand::Send(SendArgs {
                channel: "methods".into(),
                text: Some("x".repeat(65_537)),
                input: None,
                attachments: Vec::new(),
                references: Vec::new(),
            }),
        )
        .await
        .expect_err("too long");
        assert!(fake.sent().is_empty(), "nothing was sent");
        let exit = failure(&error, OutputFormat::Text, "req-1", false);
        assert!(exit.downcast_ref::<NeedsTerminal>().is_some(), "exit 2");
        assert_eq!(exit.to_string(), output::MESSAGE_TOO_LONG);
        assert_eq!(error_code(&error).as_deref(), Some(MESSAGE_TOO_LONG_CODE));
        // Control characters count as the broker counts them, escaped.
        let (api, fake) = api_with(OutputFormat::Text, standard);
        run(
            &api,
            CrewCommand::Send(SendArgs {
                channel: "methods".into(),
                text: Some("\u{1}".repeat(30_000)),
                input: None,
                attachments: Vec::new(),
                references: Vec::new(),
            }),
        )
        .await
        .expect_err("too long escaped");
        assert!(fake.sent().is_empty());
        // At the limit, it is posted.
        let (api, fake) = api_with(OutputFormat::Text, standard);
        run(
            &api,
            CrewCommand::Send(SendArgs {
                channel: "methods".into(),
                text: Some("x".repeat(65_536)),
                input: None,
                attachments: Vec::new(),
                references: Vec::new(),
            }),
        )
        .await
        .expect("posted");
        assert!(fake.broker_call("message.post").is_some());
        // The broker's refusal of one says the same.
        let refused = refuse_broker("invalid_params", "invalid_params: message too long");
        assert_eq!(
            failure(&refused, OutputFormat::Text, "req-1", false).to_string(),
            output::MESSAGE_TOO_LONG
        );
    }

    /// MSG2-N6: a full workspace tells its host what the host can do, never to "ask the host";
    /// a member still reads the member's words.
    #[tokio::test]
    async fn a_full_workspace_tells_its_host_what_to_do() {
        let post = || {
            CrewCommand::Send(SendArgs {
                channel: "methods".into(),
                text: Some("hi".into()),
                input: None,
                attachments: Vec::new(),
                references: Vec::new(),
            })
        };
        for (text, host_says) in [
            (
                "quota_exceeded: workspace logical state is full; reads remain available and the host can still remove members and change policy, but further changes require a new workspace; in-place pruning of history is not supported",
                "This workspace is full. Reading still works, and you can still remove members and change its privacy. To keep posting, start a new workspace.",
            ),
            (
                "quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported",
                "This workspace has grown past the size Crew supports and cannot take more changes. To keep working together, start a new workspace.",
            ),
        ] {
            // Alice hosts lab.
            let (api, _) = api_with(
                OutputFormat::Text,
                move |method: &str, path: &str, body: Option<&Value>| match body
                    .and_then(|body| body["method"].as_str())
                {
                    Some("message.post") => Err(refuse_broker("quota_exceeded", text)),
                    _ => standard(method, path, body),
                },
            );
            let error = run(&api, post()).await.expect_err("full");
            let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
            assert_eq!(shown, host_says);
            assert!(!shown.contains("Ask the host"), "{shown}");
            let body = failure_body(&error, &safe_lines(&error_text(&error)), "req-1");
            assert_eq!(body["broker_code"], "quota_exceeded");

            // Bob is a member.
            let (api, _) = api_with(
                OutputFormat::Text,
                move |method: &str, path: &str, body: Option<&Value>| match body
                    .and_then(|body| body["method"].as_str())
                {
                    Some("message.post") => Err(refuse_broker("quota_exceeded", text)),
                    Some("workspace.snapshot") => {
                        let mut snapshot = snapshot();
                        snapshot["actor"] = json!({"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001});
                        Ok(snapshot)
                    }
                    _ => standard(method, path, body),
                },
            );
            let error = run(&api, post()).await.expect_err("full");
            let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
            assert!(shown.ends_with("Ask the host about starting a new workspace."), "{shown}");
        }
    }

    /// SF-F4, DW-12: the daemon's typed institution and privacy-mode refusals are said from
    /// their details, naming both sides, and keep their codes.
    #[tokio::test]
    async fn institution_and_mode_refusals_name_both_sides() {
        let typed = |code: &'static str,
                     message: &'static str,
                     details: Option<Value>,
                     modes: Option<(&str, &str)>| {
            let modes = modes.map(|(actual, expected)| (actual.to_owned(), expected.to_owned()));
            move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
                if path.ends_with("/grant")
                    || path.ends_with("/request")
                        && body.and_then(|body| body["method"].as_str()) == Some("message.post")
                {
                    return Err(FakeRefusal {
                        status: 400,
                        code: Some(code.into()),
                        broker_code: None,
                        institution_refusal: details.clone(),
                        connection_institution: None,
                        message: message.into(),
                        detail: None,
                        modes: modes.clone(),
                        fields: Default::default(),
                    }
                    .into());
                }
                standard(method, path, body)
            }
        };
        let details = json!({"model": "gpt-5.5", "approved_for": ["ucsf"], "workspace": "okafor-lab", "workspace_institution": "stanford"});
        // The daemon's model refusal as `check_provider` words it (`crew/institution.rs`).
        let (api, _) = api_with(
            OutputFormat::Text,
            typed(
                "crew_institution_mismatch",
                "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution",
                Some(details),
                None,
            ),
        );
        let error = run(
            &api,
            CrewCommand::Grants(GrantCommand::Grant {
                session: SESSION.into(),
                channel: "methods".into(),
                context_channels: Vec::new(),
            }),
        )
        .await
        .expect_err("another institution");
        assert_eq!(
            failure(&error, OutputFormat::Text, "req-1", true).to_string(),
            "gpt-5.5 is approved for UCSF. okafor-lab uses stanford. Choose a model approved for stanford, or a local model."
        );
        assert_eq!(
            error_code(&error).as_deref(),
            Some("crew_institution_mismatch")
        );

        let (api, _) = api_with(
            OutputFormat::Text,
            typed(
                "crew_mode_mismatch",
                "Crew refused this.",
                None,
                Some(("private", "public")),
            ),
        );
        let error = run(
            &api,
            CrewCommand::Send(SendArgs {
                channel: "methods".into(),
                text: Some("hi".into()),
                input: None,
                attachments: Vec::new(),
                references: Vec::new(),
            }),
        )
        .await
        .expect_err("the other mode");
        let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
        assert_eq!(
            shown,
            "Your connection is Private, but this request required Public. Nothing was sent."
        );
        assert!(!shown.contains("privacy changed"), "{shown}");
        assert_eq!(error_code(&error).as_deref(), Some("crew_mode_mismatch"));
    }

    /// W2-CLI-7: admission refuses a connection set to another institution than the
    /// workspace's before it judges any model, and its refusal carries the model's details as
    /// well as `connection_institution`. That is the daemon's own sentence, as the desktop
    /// prints it, never the model sentence: "gpt-5.5 is approved for ucsf. lab uses ucsf."
    /// contradicts itself, and its advice (a local model) is refused too.
    #[tokio::test]
    async fn a_connection_set_to_another_institution_keeps_the_daemons_sentence() {
        // `crew/institution.rs` admission, for a connection set to stanford in lab (ucsf).
        const SENTENCE: &str = "This connection is for stanford, but lab belongs to ucsf.";
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path.ends_with("/grant") || path.ends_with("/runs") && method == "POST" {
                return Err(FakeRefusal {
                    status: 400,
                    code: Some(INSTITUTION_MISMATCH.into()),
                    broker_code: None,
                    institution_refusal: Some(json!({
                        "model": "gpt-5.5",
                        "approved_for": ["ucsf"],
                        "workspace": "lab",
                        "workspace_institution": "ucsf",
                    })),
                    connection_institution: Some("stanford".into()),
                    message: SENTENCE.into(),
                    detail: None,
                    modes: None,
                    fields: Default::default(),
                }
                .into());
            }
            standard(method, path, body)
        };
        let grant = CrewCommand::Grants(GrantCommand::Grant {
            session: SESSION.into(),
            channel: "methods".into(),
            context_channels: Vec::new(),
        });
        let start = CrewCommand::Tasks(TaskCommand::Start {
            channel: "#methods".into(),
            prompt: TextInput {
                text: Some("Summarize".into()),
                input: None,
            },
            provider: "versa_azure".into(),
            model: "gpt-5.5".into(),
            context_channels: Vec::new(),
            allow_posting: true,
        });
        for (name, command) in [("grants grant", grant), ("tasks start", start)] {
            let (api, _) = api_with(OutputFormat::Text, handler);
            let error = run(&api, command).await.expect_err(name);
            let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
            assert_eq!(shown, SENTENCE, "{name}");
            assert!(!shown.contains("approved for"), "{name}: {shown}");
            assert!(!shown.contains("local model"), "{name}: {shown}");
            assert_eq!(
                error_code(&error).as_deref(),
                Some(INSTITUTION_MISMATCH),
                "{name}"
            );
        }
        // The same details under the model refusal's words are still the model sentence, and
        // so is a daemon's model refusal that names no connection institution.
        for message in [
            "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution",
            "Crew refused this model.",
        ] {
            assert!(is_model_refusal(message, None), "{message}");
        }
        assert!(is_model_refusal(
            "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution",
            Some("stanford"),
        ));
        assert!(!is_model_refusal(SENTENCE, Some("stanford")));
    }

    /// W2-CLI-14: a request the daemon never sent is said in the daemon's own sentence, which
    /// names the cause and whether trying again can help, in text and in JSON's `error`, under
    /// the code `crew_not_sent` and with no retry line. These are `lost_request`'s sentences in
    /// `crates/biorouter/src/crew/mod.rs`, for workspace lab and `bob@hpc`.
    #[tokio::test]
    async fn a_request_the_daemon_never_sent_keeps_the_daemons_sentence() {
        const SIGN_IN_REFUSED: &str = "Couldn't sign in to hpc as bob: the server refused this computer's SSH key. Nothing was sent.";
        const SERVER_UNREACHABLE: &str =
            "lab's server couldn't be reached, so nothing was sent. Try again once Crew reconnects.";
        const DROPPED: &str =
            "The connection to lab dropped before it answered. Nothing changed; try again.";
        const UNREACHABLE: &str = "Biorouter couldn't reach lab, so nothing was sent.";
        for sentence in [SIGN_IN_REFUSED, SERVER_UNREACHABLE, DROPPED, UNREACHABLE] {
            let (api, fake) = api_with(
                OutputFormat::Text,
                move |method: &str, path: &str, body: Option<&Value>| {
                    if body.and_then(|body| body["method"].as_str()) == Some("message.post") {
                        return Err(FakeRefusal {
                            status: 503,
                            code: Some(NOT_SENT.into()),
                            broker_code: None,
                            institution_refusal: None,
                            connection_institution: None,
                            message: sentence.into(),
                            detail: None,
                            modes: None,
                            fields: Default::default(),
                        }
                        .into());
                    }
                    standard(method, path, body)
                },
            );
            let error = run(
                &api,
                CrewCommand::Send(SendArgs {
                    channel: "methods".into(),
                    text: Some("hi".into()),
                    input: None,
                    attachments: Vec::new(),
                    references: Vec::new(),
                }),
            )
            .await
            .expect_err(sentence);
            assert!(
                fake.broker_calls()
                    .iter()
                    .any(|(method, _)| method == "message.post"),
                "the post reached the daemon"
            );
            // The post carried the request ID, so a retry line would be offered if the daemon's
            // answer were uncertain. It is not: nothing was sent.
            let shown = failure(&error, OutputFormat::Text, "req-1", true).to_string();
            assert_eq!(shown, sentence);
            assert!(!shown.contains("--request-id"), "{shown}");
            let line = failure_json(
                &error,
                &safe_lines(&error_text(&error)),
                "req-1",
                OutputFormat::Json,
            )
            .expect("JSON output");
            let body: Value = serde_json::from_str(&line).expect("one JSON value");
            assert_eq!(body["error"], sentence);
            assert_eq!(body["code"], NOT_SENT);
        }
        // A key the server refuses is refused every time: nothing tells the person to run the
        // same command again.
        let refused = refuse(503, Some(NOT_SENT), SIGN_IN_REFUSED);
        let shown = failure(&refused, OutputFormat::Text, "req-1", true).to_string();
        assert!(shown.contains("refused this computer's SSH key"), "{shown}");
        assert!(!shown.contains("run it again"), "{shown}");
    }

    #[test]
    fn a_broker_refusal_keeps_terminal_controls_escaped() {
        let error = refuse_broker(
            "not_invited",
            "not_invited: @eve has no pending invitation.\u{1b}[2J Invite them first.",
        );
        let shown = failure(&error, OutputFormat::Text, "req-1", false).to_string();
        assert!(!shown.contains('\u{1b}'), "{shown}");
        assert!(shown.contains("\\u{1b}[2J"), "{shown}");
    }

    /// CLI-14: a confirmation with no terminal to ask in is the usage refusal, status 2, in
    /// text and JSON alike, and nothing is sent.
    #[tokio::test]
    async fn a_confirmation_without_a_terminal_exits_with_the_usage_status() {
        let revoke = CrewCommand::Enroll(EnrollmentCommand::Revoke {
            member: "@bob".into(),
            confirm: None,
        });
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(&api, revoke).await.expect_err("no terminal");
        let shown = failure(&error, OutputFormat::Text, "req-1", false);
        assert!(shown.downcast_ref::<NeedsTerminal>().is_some(), "{shown:?}");
        assert!(shown.to_string().contains("--confirm @bob"), "{shown}");
        assert_eq!(
            failure_body(&error, "m", "req-1")["code"],
            NEEDS_TERMINAL_CODE
        );
        assert!(fake.broker_call("enrollment.revoke").is_none());

        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), "brcrew1:abc").expect("write invitation");
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if path == "/crew/connections/from-invitation" {
                return Ok(preview(&[]));
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, handler);
        let mut args = join_args(file.path());
        args.yes = false;
        let error = run(
            &api,
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(Box::new(args))),
        )
        .await
        .expect_err("no terminal");
        let shown = failure(&error, OutputFormat::Json, "req-1", false);
        assert!(shown.downcast_ref::<NeedsTerminal>().is_some(), "{shown:?}");

        // Any other failure keeps the ordinary status.
        let refused = refuse(404, Some("crew_grant_not_found"), "No Crew grant.");
        assert!(failure(&refused, OutputFormat::Text, "req-1", false)
            .downcast_ref::<NeedsTerminal>()
            .is_none());
    }

    /// CLI-10: archiving a channel and removing someone from it ask first, as the desktop
    /// does; without a terminal they need --yes, and nothing is sent without it.
    #[tokio::test]
    async fn archive_and_remove_member_ask_first_and_need_yes_without_a_terminal() {
        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(
            &api,
            CrewCommand::Channels(ChannelCommand::Archive {
                channel: "methods".into(),
                yes: false,
            }),
        )
        .await
        .expect_err("no terminal to ask in");
        assert!(error.downcast_ref::<NeedsTerminal>().is_some(), "{error:?}");
        assert_eq!(
            message(&error),
            "Archiving #methods is permanent for everyone. There is no terminal to ask in, so add --yes."
        );
        let error = run(
            &api,
            CrewCommand::RemoveMember {
                channel: "methods".into(),
                member: "@bob".into(),
                former: false,
                yes: false,
            },
        )
        .await
        .expect_err("no terminal to ask in");
        assert!(error.downcast_ref::<NeedsTerminal>().is_some(), "{error:?}");
        assert!(message(&error).starts_with("Removing @bob from #methods needs a yes."));
        assert!(fake.broker_call("channel.archive").is_none());
        assert!(fake.broker_call("membership.revoke").is_none());

        assert_eq!(
            yes_or_ask(false, true, "Archive?".into(), "no".into()).expect("asks"),
            Consent::Ask("Archive?".into())
        );
        assert_eq!(
            yes_or_ask(true, false, "Archive?".into(), "no".into()).expect("given"),
            Consent::Given
        );
    }

    /// A daemon with one saved connection whose snapshot is `snapshot`, and which removes it.
    fn removing(snapshot: Value) -> impl Fn(&str, &str, Option<&Value>) -> Result<Value> {
        move |method: &str, path: &str, body: Option<&Value>| {
            if method == "DELETE" && path == format!("/crew/connections/{CONNECTION}") {
                return Ok(json!({"removed": true}));
            }
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                return Ok(snapshot.clone());
            }
            standard(method, path, body)
        }
    }

    fn remove_connection_command(confirm: Option<&str>, give_up: bool) -> CrewCommand {
        CrewCommand::Connections(ConnectionCommand::Remove {
            confirm: confirm.map(str::to_owned),
            give_up_host_controls: give_up,
        })
    }

    fn deleted(fake: &FakeDaemon) -> bool {
        fake.sent().iter().any(|sent| sent.method == "DELETE")
    }

    /// CLI-1: removing a connection deletes this computer's device key, so the name is typed
    /// again (or given with --confirm), and a host's only computer is refused unless
    /// --give-up-host-controls says so.
    #[tokio::test]
    async fn removing_a_connection_is_confirmed_and_never_silently_ends_the_host_controls() {
        // Alice hosts lab (her UID is the host's) and no other computer of hers is enrolled.
        let (api, fake) = api_with(OutputFormat::Text, removing(snapshot()));
        let error = run(&api, remove_connection_command(Some("UCSF HPC"), false))
            .await
            .expect_err("the host's only computer");
        assert_eq!(
            error_code(&error).as_deref(),
            Some("crew_host_controls_would_end")
        );
        let shown = message(&error);
        assert!(shown.contains("You host lab"), "{shown}");
        assert!(
            shown.contains("biorouter crew enroll invite @alice --add-device"),
            "{shown}"
        );
        assert!(shown.contains("--give-up-host-controls"), "{shown}");
        assert!(!deleted(&fake));

        let lines = said(
            run(&api, remove_connection_command(Some(" ucsf hpc "), true))
                .await
                .expect("given up on purpose"),
        );
        assert!(deleted(&fake));
        assert_eq!(lines, ["Removed UCSF HPC from this computer."]);

        // Bob is a member, not the host.
        let mut member = snapshot();
        member["actor"] =
            json!({"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001});
        let (api, fake) = api_with(OutputFormat::Text, removing(member.clone()));
        let error = run(&api, remove_connection_command(None, false))
            .await
            .expect_err("no terminal, no --confirm");
        assert!(error.downcast_ref::<NeedsTerminal>().is_some(), "{error:?}");
        assert!(
            message(&error).ends_with("so confirm with --confirm 'UCSF HPC'."),
            "{}",
            message(&error)
        );
        let error = run(&api, remove_connection_command(Some("Elsewhere"), false))
            .await
            .expect_err("the wrong name");
        assert_eq!(
            message(&error),
            "Not done: --confirm Elsewhere doesn't match UCSF HPC."
        );
        assert!(!deleted(&fake));
        run(&api, remove_connection_command(Some("UCSF HPC"), false))
            .await
            .expect("a member's confirmed removal");
        assert!(deleted(&fake));

        let mut named = snapshot();
        named["workspace"]["host_principal_id"] = json!(BOB);
        assert_eq!(host_standing(&named, None), HostStanding::NotHost);
        assert_eq!(host_standing(&json!({}), None), HostStanding::Unknown);
        assert_eq!(
            typed_or_ask("UCSF HPC", None, true, "no".into()).expect("asks"),
            Consent::Ask("Type UCSF HPC to confirm:".into())
        );
    }

    /// CLI-1: the snapshot's list of a host's computers never shrinks when one of them removes
    /// its connection (the broker drops a device only when its whole member is revoked), so two
    /// listed computers do not prove the host controls survive removing this one. The removal
    /// goes ahead, but only after saying which other computers must still have the workspace
    /// saved, and this computer is never among them.
    #[tokio::test]
    async fn a_host_with_another_listed_computer_is_told_the_host_controls_depend_on_it() {
        const HERE: &str = "70dcffaf1751a59b0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut two = snapshot();
        two["actor"]["devices"] = json!([
            {"fingerprint": "70DC FFAF 1751 A59B", "added_at": 1_789_000_000, "added_via": "bootstrap"},
            {"fingerprint": "3F2A 9C1E 77B0 D4E1", "added_at": 1_790_200_000, "added_via": "invitation_code"}
        ]);
        let laptop = two["actor"]["devices"][1].clone();
        // Every listed entry being this computer leaves no other one to name.
        let mut twice = two.clone();
        twice["actor"]["devices"][1]["fingerprint"] = json!("70dc-ffaf-1751-a59b");
        assert_eq!(
            host_standing(&twice, Some(HERE)),
            HostStanding::OnlyHostComputer {
                username: "alice".into()
            }
        );
        let connection = json!({"id": CONNECTION, "name": "UCSF HPC", "device_id": HERE});
        assert_eq!(own_device_id(&connection), Some(HERE));
        assert_eq!(own_device_id(&json!({"device_id": ""})), None);
        let standing = host_standing(&two, own_device_id(&connection));
        assert_eq!(
            standing,
            HostStanding::HostElsewhereToo {
                others: vec![laptop.clone()]
            }
        );

        // Both refusal flags aside, it is said, never refused, and names only the laptop.
        let options = HumanOptions::in_utc_at(1_790_214_655);
        for give_up in [false, true] {
            let notice = host_removal_notice(&standing, "lab", true, give_up, &options)
                .expect("not refused: another computer is listed")
                .expect("hosting is always mentioned");
            assert_eq!(
                notice,
                "You host lab. Its host controls continue only if one of your other enrolled computers still has lab saved:\n  3F2A 9C1E 77B0 D4E1 · added Sep 23, 2026 · via invitation code\nA computer stays on this list after its connection is removed there. If none of them still has lab saved, removing it here ends the host controls for good."
            );
            assert!(!notice.contains("70DC"), "{notice}");
        }

        // Without this computer's device ID nothing can be left out, and the notice says so.
        let unknown = host_standing(&two, None);
        assert_eq!(
            unknown,
            HostStanding::HostElsewhereToo {
                others: two["actor"]["devices"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            }
        );
        let notice = host_removal_notice(&unknown, "lab", false, false, &options)
            .expect("not refused")
            .expect("said");
        assert!(
            notice.contains("These computers are enrolled as you, this one among them:"),
            "{notice}"
        );
        assert!(notice.contains("70DC FFAF 1751 A59B"), "{notice}");

        // A member is told nothing about hosting.
        let mut member = two.clone();
        member["workspace"]["host_uid"] = json!(1001);
        assert_eq!(
            host_removal_notice(
                &host_standing(&member, Some(HERE)),
                "lab",
                true,
                false,
                &options
            )
            .expect("not refused"),
            None
        );

        // End to end: without a terminal it still needs the name, and with it the removal
        // goes ahead without --give-up-host-controls.
        let snapshot = two.clone();
        let handler = move |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if method == "GET" && path == "/crew/connections" {
                return Ok(json!({"connections": [{
                    "id": CONNECTION, "name": "UCSF HPC", "ssh_target": "bob@hpc",
                    "workspace_id": "w", "status": "connected", "public_key": "k",
                    "device_id": HERE
                }]}));
            }
            removing(snapshot.clone())(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let error = run(&api, remove_connection_command(None, false))
            .await
            .expect_err("no terminal, no --confirm");
        assert!(error.downcast_ref::<NeedsTerminal>().is_some(), "{error:?}");
        assert!(!deleted(&fake));
        let lines = said(
            run(&api, remove_connection_command(Some("UCSF HPC"), false))
                .await
                .expect("another listed computer may still hold the host controls"),
        );
        assert!(deleted(&fake));
        assert_eq!(lines, ["Removed UCSF HPC from this computer."]);
    }

    /// CLI-10: making the connection or the workspace public is confirmed by typing the
    /// workspace's name, as the desktop's typed confirmation is; making it private is not.
    #[tokio::test]
    async fn going_public_needs_the_workspace_name_typed() {
        let handler = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if method == "PATCH" {
                return Ok(json!({"id": CONNECTION, "mode": "public"}));
            }
            standard(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let workspace = |mode: PrivacyMode, confirm: Option<&str>| {
            CrewCommand::Privacy(PrivacyCommand::SetWorkspace {
                mode,
                institution_id: None,
                confirm: confirm.map(str::to_owned),
            })
        };
        let error = run(&api, workspace(PrivacyMode::Public, None))
            .await
            .expect_err("no terminal");
        assert!(error.downcast_ref::<NeedsTerminal>().is_some(), "{error:?}");
        assert!(message(&error).starts_with("Allowing Public in lab needs its name typed."));
        let error = run(&api, workspace(PrivacyMode::Public, Some("lab2")))
            .await
            .expect_err("the wrong name");
        assert_eq!(
            message(&error),
            "Not done: --confirm lab2 doesn't match lab."
        );
        assert!(fake.broker_call("policy.set").is_none());
        run(&api, workspace(PrivacyMode::Public, Some("LAB")))
            .await
            .expect("confirmed");
        assert_eq!(
            fake.broker_call("policy.set").expect("set")["mode"],
            "public"
        );
        let (api, fake) = api_with(OutputFormat::Text, handler);
        run(&api, workspace(PrivacyMode::Private, None))
            .await
            .expect("private needs no typed name");
        assert!(fake.broker_call("policy.set").is_some());

        let personal = |confirm: Option<&str>| {
            CrewCommand::Privacy(PrivacyCommand::SetPersonal {
                mode: PrivacyMode::Public,
                institution_id: None,
                confirm: confirm.map(str::to_owned),
            })
        };
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let error = run(&api, personal(None)).await.expect_err("no terminal");
        // SF-F1: lab is Private for everyone, so the prompt says nothing changes yet.
        assert!(
            message(&error).starts_with("Making your lab connection public needs its name typed."),
            "{}",
            message(&error)
        );
        assert!(
            message(&error).starts_with("Making your lab connection public needs its name typed.")
        );
        assert!(!fake.sent().iter().any(|sent| sent.method == "PATCH"));
        let saved = said(run(&api, personal(Some("lab"))).await.expect("confirmed"));
        assert!(fake.sent().iter().any(|sent| sent.method == "PATCH"));
        // SF-F1: saved, and said to change nothing while lab is Private for everyone.
        assert!(
            saved[0].ends_with("Nothing changes while lab is Private for everyone."),
            "{saved:?}"
        );
    }

    /// SF-F1: `status` and `connections list` ask a connected Public connection's workspace
    /// for its privacy, and give the effective privacy; a Private one is Private without
    /// asking, and an offline Public one says its workspace's can't be read.
    #[tokio::test]
    async fn connection_lists_give_the_effective_privacy() {
        let listing = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if method == "GET" && path == "/crew/connections" {
                return Ok(json!({"connections": [
                    {"id": CONNECTION, "name": "lab", "ssh_target": "bob@hpc", "status": "connected",
                     "mode": "public", "institution_id": "ucsf"},
                    {"id": "c0ffee00-0000-4000-8000-000000000009", "name": "other", "ssh_target": "bob@other",
                     "status": "disconnected", "mode": "public"},
                    {"id": "c0ffee00-0000-4000-8000-00000000000a", "name": "clinic", "ssh_target": "bob@clinic",
                     "status": "connected", "mode": "private", "institution_id": "ucsf"}
                ]}));
            }
            standard(method, path, body)
        };
        let (api, fake) = api_with(OutputFormat::Json, listing);
        let Reply::Show(listed, _) = run(&api, CrewCommand::Status).await.expect("status") else {
            panic!("status shows the list")
        };
        let connections = listed["connections"].as_array().unwrap();
        assert_eq!(connections[0]["effective_mode"], "private");
        assert_eq!(connections[0]["workspace_mode"], "private");
        assert_eq!(connections[0]["workspace_name"], "lab");
        assert_eq!(connections[1]["effective_mode"], Value::Null);
        assert_eq!(connections[2]["effective_mode"], "private");
        let asked = fake
            .broker_calls()
            .into_iter()
            .filter(|(method, _)| method == "workspace.snapshot")
            .count();
        assert_eq!(
            asked, 1,
            "only the connected Public connection's workspace is asked"
        );
        let (api, _) = api_with(OutputFormat::Text, listing);
        let text = said(run(&api, CrewCommand::Status).await.expect("status")).join("\n");
        assert!(
            text.contains("lab · bob@hpc · Connected · Private because lab is Private for everyone · your connection: Public"),
            "{text}"
        );
    }

    #[test]
    fn a_device_code_is_checked_for_its_length_whatever_separates_it() {
        for code in [
            "7QK2-M9XA-3JTP-WZ4D",
            "7qk2m9xa3jtpwz4d",
            "7QK2 M9XA 3JTP WZ4D",
            "7QK2\u{2013}M9XA\u{2013}3JTP\u{2013}WZ4D",
        ] {
            check_device_code(code).unwrap_or_else(|error| panic!("{code}: {error}"));
        }
        for code in [
            "7QK2-M9XA-3JTP",
            "7QK2-M9XA-3JTP-WZ4D-1",
            "7QK2-M9XA-3JTP-WZ4\u{0414}",
        ] {
            assert!(check_device_code(code).is_err(), "{code}");
        }
    }

    #[test]
    fn pending_joins_show_names_and_warnings_but_never_a_code() {
        let lines = pending_lines(
            &json!([
                {"username": "bob", "full_name": "Bob Lee", "add_device": false, "approved": false,
                 "created_at": 0, "expires_at": 7200, "expired": false, "mismatched_attempts": 1},
                {"username": "carol", "full_name": null, "add_device": true, "approved": true,
                 "created_at": 0, "expires_at": 10, "expired": false, "mismatched_attempts": 0}
            ]),
            100,
        );
        assert_eq!(
            lines,
            [
                "Waiting to join (2):",
                "  @bob · Bob Lee (name on the server account) · waiting for their code · expires in 2 hours",
                "    A computer trying to join as @bob showed a different code. Check the code @bob sent you; if you typed it wrong, run enroll approve again with --replace. Don't approve a code you didn't get from @bob.",
                "  @carol · another computer · code saved; joins when their computer confirms the same code · expired",
                "Let someone in with: biorouter crew enroll approve @USERNAME CODE"
            ]
        );
        assert_eq!(pending_lines(&json!([]), 0), ["No one is waiting to join."]);
    }

    /// DW-13: a snapshot without `pending_joins` is not an empty queue. The broker leaves it
    /// out for anyone but the host, who is refused as every other `enroll` command refuses
    /// them; a host whose server cannot join by name is told so. Both exit non-zero.
    #[tokio::test]
    async fn enroll_pending_without_the_list_says_why_instead_of_an_empty_queue() {
        let pending = || CrewCommand::Enroll(EnrollmentCommand::Pending);
        let as_member = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["actor"] =
                    json!({"id": BOB, "username": "bob", "display_name": "Bob Lee", "uid": 1001});
                return Ok(snapshot);
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, as_member);
        let error = run(&api, pending()).await.expect_err("not the host");
        assert_eq!(
            message(&error),
            "Only the workspace host can see who is waiting to join."
        );
        assert_eq!(error_code(&error).as_deref(), Some("crew_host_required"));

        // Alice hosts lab, and her server's snapshot has no list.
        let (api, _) = api_with(OutputFormat::Text, standard);
        let error = run(&api, pending()).await.expect_err("no joining by name");
        assert_eq!(
            message(&error),
            "This workspace's server doesn't support joining by name."
        );

        // The host's own empty list is an empty queue.
        let empty = |method: &str, path: &str, body: Option<&Value>| -> Result<Value> {
            if body.and_then(|body| body["method"].as_str()) == Some("workspace.snapshot") {
                let mut snapshot = snapshot();
                snapshot["pending_joins"] = json!([]);
                return Ok(snapshot);
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Text, empty);
        assert_eq!(
            said(run(&api, pending()).await.expect("an empty queue")),
            ["No one is waiting to join."]
        );
    }

    #[tokio::test]
    async fn a_request_id_is_offered_only_after_an_uncertain_mutation() {
        let (api, _) = api_with(OutputFormat::Text, standard);
        run(&api, history("methods")).await.expect("a read");
        assert!(
            !api.client.sent.load(Ordering::SeqCst),
            "reads and name lookups carry no request ID"
        );
        run(
            &api,
            CrewCommand::Channels(ChannelCommand::Archive {
                channel: "methods".into(),
                yes: true,
            }),
        )
        .await
        .expect("a mutation");
        assert!(api.client.sent.load(Ordering::SeqCst));

        let lost = anyhow!("connection reset");
        let hint = "Retry safely with --request-id req-1";
        assert!(failure(&lost, OutputFormat::Text, "req-1", true)
            .to_string()
            .ends_with(hint));
        assert!(!failure(&lost, OutputFormat::Text, "req-1", false)
            .to_string()
            .contains("req-1"));
        let refused = refuse(409, Some("crew_request_refused"), "forbidden");
        assert!(!failure(&refused, OutputFormat::Text, "req-1", true)
            .to_string()
            .contains("req-1"));
        let failed = refuse(503, Some("crew_cancel_persistence_failed"), "ledger");
        assert!(failure(&failed, OutputFormat::Text, "req-1", true)
            .to_string()
            .ends_with(hint));

        // R-3: an outcome the daemon could not confirm may have landed, whatever its status, so
        // the retry is offered, after the daemon's own sentence and with its own code.
        const UNCONFIRMED: &str = "Crew couldn't confirm whether this reached lab. Check the channel, then retry with the same request ID.";
        for status in [400, 503] {
            let unknown = refuse(status, Some("crew_outcome_unknown"), UNCONFIRMED);
            let shown = failure(&unknown, OutputFormat::Text, "req-1", true).to_string();
            assert_eq!(shown, format!("{UNCONFIRMED}\n{hint}"));
            assert_eq!(
                failure_body(&unknown, UNCONFIRMED, "req-1")["code"],
                "crew_outcome_unknown"
            );
        }
        // A request the daemon never sent is a definite answer: no retry ID, and it says so in
        // the daemon's own words (a_request_the_daemon_never_sent_keeps_the_daemons_sentence),
        // or, from a daemon that sent none, in these.
        let not_sent = refuse(503, Some("crew_not_sent"), " ");
        assert_eq!(
            failure(&not_sent, OutputFormat::Text, "req-1", true).to_string(),
            "Nothing was sent; run it again."
        );
        // Nor did one it held back while dialling a broken bridge again (W2-DMN-6).
        const RECONNECTING_TEXT: &str =
            "Reconnecting to lab. Nothing was sent; try again in a moment.";
        let reconnecting = refuse(503, Some("crew_reconnecting"), RECONNECTING_TEXT);
        assert_eq!(
            failure(&reconnecting, OutputFormat::Text, "req-1", true).to_string(),
            RECONNECTING_TEXT
        );
        // The daemon's other typed refusals are its own sentence, without "Daemon returned".
        let fixed = refuse(
            409,
            Some("crew_model_fixed"),
            "This chat's model is fixed by its Crew access. Start a new chat to use another model.",
        );
        assert_eq!(
            failure(&fixed, OutputFormat::Text, "req-1", true).to_string(),
            "This chat's model is fixed by its Crew access. Start a new chat to use another model."
        );
        // M16: nor did one the connection here handed back unsent, though it carried the ID.
        let unsent: anyhow::Error = crate::daemon_client::NotSent::for_test().into();
        let shown = failure(&unsent, OutputFormat::Text, "req-1", true).to_string();
        assert_eq!(
            shown,
            "The daemon connection wasn't ready. Nothing was sent; run it again."
        );
        assert_eq!(error_code(&unsent).as_deref(), Some("crew_not_sent"));

        assert_eq!(
            failure(
                &anyhow!("one\u{1b}[2J\ntwo"),
                OutputFormat::Text,
                "r",
                false
            )
            .to_string(),
            "one\\u{1b}[2J\ntwo"
        );
        assert!(carries_request_id(
            Some(&json!({"params": {"idempotency_key": "r"}})),
            "r"
        ));
        assert!(carries_request_id(Some(&json!({"request_id": "r"})), "r"));
        assert!(!carries_request_id(Some(&json!({"request_id": null})), "r"));
        assert!(!carries_request_id(None, "r"));
    }

    #[test]
    fn a_selector_is_an_id_only_when_it_is_uuid_shaped() {
        assert_eq!(
            Kind::Channel.literal_id(&format!("#{METHODS}")),
            Some(METHODS)
        );
        assert_eq!(Kind::Person.literal_id(&format!("@{BOB}")), Some(BOB));
        assert_eq!(Kind::Team.literal_id(TEAM), Some(TEAM));
        assert_eq!(Kind::Team.literal_id(&format!("#{TEAM}")), None);
        let hex = "ab".repeat(32);
        assert_eq!(Kind::Person.literal_id(&hex), Some(hex.as_str()));
        for name in [
            "methods",
            "#methods",
            "analysis-lab/methods",
            "@bob",
            "bob",
            "lab",
        ] {
            assert_eq!(Kind::Channel.literal_id(name), None);
            assert_eq!(Kind::Person.literal_id(name), None);
        }
    }

    #[test]
    fn names_in_sentences_are_escaped_and_isolated() {
        assert_eq!(name_text("Analysis Lab"), "Analysis Lab");
        assert_eq!(name_text("مختبر"), "\u{2068}مختبر\u{2069}");
        assert_eq!(name_text("a\u{1b}b"), "a\\u{1b}b");
        assert_eq!(channel_text("#methods"), "#methods");
        // F10: a name that holds no right-to-left text is left bare.
        assert_eq!(person_text("bob", Some("Bob Lee")), "\"Bob Lee\" (@bob)");
        assert_eq!(
            person_text("dana", Some("דנה לוי")),
            "\"\u{2068}דנה לוי\u{2069}\" (@dana)"
        );
        assert_eq!(person_text("bob", Some("BOB")), "@bob");
        assert_eq!(shell_word("UCSF HPC"), "'UCSF HPC'");
        assert_eq!(shell_word("lab"), "lab");
        assert_eq!(loose_key("#Data Analysis"), "data-analysis");
        assert_eq!(loose_key("Analysis_Lab."), "analysis-lab");
    }
}
