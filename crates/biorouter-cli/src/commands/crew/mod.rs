//! `biorouter crew`: the terminal's view of Crew, through the profile's shared daemon.
//!
//! People, teams, channels and saved connections are named, not numbered ("Selectors and the
//! resolver" in `docs/research/biorouter-crew/naming-design.md`). Every name is resolved by the
//! daemon's `POST /crew/resolve` against the person's own workspace snapshot, so the CLI and the
//! desktop share one resolver and a candidate can never be something the person could not see.
//! UUID-shaped text is always an ID and never goes to the resolver, so scripts that pass IDs work
//! against any daemon, including one that predates names. A resolution is a lookup, not a
//! permission: the broker authorizes every mutation, and a person-targeted one also carries the
//! `@username` the person typed (`expected_username`), which the broker checks.

mod args;
mod files;
mod output;

use crate::daemon_client::{CrewClient, DaemonRefusal};
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

pub async fn handle(mut options: CrewOptions) -> Result<()> {
    let format = options.output_format;
    let request_id = options
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    options.request_id = Some(request_id.clone());
    let sent = Arc::new(AtomicBool::new(false));
    execute(options, Arc::clone(&sent))
        .await
        .map_err(|error| failure(&error, format, &request_id, sent.load(Ordering::SeqCst)))
}

/// The error a failed command exits with.
///
/// Each line is made terminal-safe on its own, so a list of candidates stays a list. The request
/// ID is a machine ID, so it appears only where it is useful: after a mutation that carried it
/// was sent and its outcome is unknown, as the one retry that is safe. JSON output always
/// carries it, and the daemon's refusal code when there is one.
fn failure(
    error: &anyhow::Error,
    format: OutputFormat,
    request_id: &str,
    sent: bool,
) -> anyhow::Error {
    let message = safe_lines(&error_text(error));
    if matches!(format, OutputFormat::Json | OutputFormat::StreamJson) {
        let mut body = json!({"error": message, "request_id": request_id});
        if let Some(code) = error_code(error) {
            body["code"] = json!(code);
        }
        if let Some((broker_code, _)) = error.chain().find_map(broker_refusal) {
            body["broker_code"] = json!(broker_code);
        }
        let _ = emit(&body, format);
    }
    if sent && outcome_uncertain(error) {
        anyhow!("{message}\n{}", output::retry_hint(request_id))
    } else {
        anyhow!("{message}")
    }
}

/// `{error:#}`, except that a broker refusal the daemon forwarded is said in words for a person
/// ([`output::broker_refusal_text`]) instead of as `Daemon returned 400: code: …`.
fn error_text(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| match broker_refusal(cause) {
            Some((code, message)) => output::broker_refusal_text(code, message),
            None => cause.to_string(),
        })
        .collect::<Vec<_>>()
        .join(": ")
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

fn safe_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| safe_text(line.strip_suffix('\r').unwrap_or(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn execute(options: CrewOptions, sent: Arc<AtomicBool>) -> Result<()> {
    let CrewOptions {
        connection,
        expected_mode,
        expected_policy_epoch,
        expected_workspace_policy_epoch,
        no_start,
        approval_key_stdin,
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
            &crate::daemon_client::credentials_control(action, approval_key_stdin).await?,
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
    };
    run(&api, command).await?.print(api.format)
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

/// The code a script can match on: the daemon's, or the one a restated refusal kept.
fn error_code(error: &anyhow::Error) -> Option<String> {
    error
        .chain()
        .find_map(|cause| {
            cause
                .downcast_ref::<Restated>()
                .and_then(|restated| restated.code.map(str::to_owned))
        })
        .or_else(|| refusal(error).and_then(|refused| refused.code))
}

/// A refusal is a definite answer: nothing changed. A daemon failure or a lost answer is not.
fn outcome_uncertain(error: &anyhow::Error) -> bool {
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<Restated>().is_some())
    {
        return false;
    }
    refusal(error).is_none_or(|refused| refused.status >= 500)
}

/// A refusal said again for a person, keeping the daemon's code for JSON output.
#[derive(Debug)]
struct Restated {
    message: String,
    code: Option<&'static str>,
}

impl std::fmt::Display for Restated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Restated {}

fn restated(message: impl Into<String>, code: Option<&'static str>) -> anyhow::Error {
    Restated {
        message: message.into(),
        code,
    }
    .into()
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

/// One resolution from `POST /crew/resolve`, or the sentence explaining why it did not resolve.
fn target_from(kind: Kind, text: &str, result: &Value) -> std::result::Result<Target, String> {
    let shown = kind.shown(text);
    match result["status"].as_str() {
        Some("resolved") if result["kind"].as_str() == Some(kind.wire()) => {
            let id = result["id"]
                .as_str()
                .and_then(|id| component(id).ok())
                .ok_or_else(|| format!("Biorouter couldn't look up {shown}."))?;
            let is_person = matches!(kind, Kind::Person | Kind::FormerPerson);
            Ok(Target {
                id: id.to_owned(),
                label: result["label"].as_str().map(str::to_owned),
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
                Kind::Team => format!("You're not in a team called {shown}."),
                Kind::Channel => format!("You're not in a channel called {shown}."),
            };
            if let Some(suggestion) = result["did_you_mean"].as_str() {
                message.push_str(&format!(
                    " Did you mean {suggestion}? Usernames must match exactly."
                ));
            }
            Err(message)
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
            Err(lines.join("\n"))
        }
        _ => Err(format!("Biorouter couldn't look up {shown}.")),
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
}

impl Reply {
    fn print(self, format: OutputFormat) -> Result<()> {
        match self {
            Self::Show(value, options) => emit_with(&value, format, &options),
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
        CrewCommand::Status => api.show(api.connections().await?),
        CrewCommand::Connections(command) => connections(api, command).await?,
        CrewCommand::Auth => {
            let id = api.connection_id().await?;
            api.show(api.client.shared()?.authenticate_ssh(&id).await?)
        }
        CrewCommand::Connect => api.show(api.connection_action("connect", json!({})).await?),
        CrewCommand::Disconnect => api.show(api.connection_action("disconnect", json!({})).await?),
        CrewCommand::Join(args) => join(api, args).await?,
        CrewCommand::Workspace(command) => workspace(api, command).await?,
        CrewCommand::Enroll(command) => enrollment(api, command).await?,
        CrewCommand::Members(MembersArgs { command: None }) => {
            let snapshot = api.snapshot().await?;
            let people = snapshot_field(&snapshot, "principals")?;
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
        } => remove_member(api, &channel, &member, former).await?,
        CrewCommand::History(args) => {
            let channel = api.target(Kind::Channel, &args.channel).await?;
            let page = api
                .broker(
                    "messages.history",
                    history_params(&channel.id, &args),
                    false,
                )
                .await?;
            api.show_with(page, api.names().await)
        }
        CrewCommand::Search {
            channel,
            query,
            limit,
            after,
        } => {
            let channel = api.target(Kind::Channel, &channel).await?;
            let mut params = json!({"channel_id":channel.id,"query":query,"limit":limit});
            if let Some(after) = after {
                params["after"] = json!(after);
            }
            let page = api.broker("messages.search", params, false).await?;
            api.show_with(page, api.names().await)
        }
        CrewCommand::Watch(args) => watch(api, args).await?,
        CrewCommand::Send(args) => send_message(api, args).await?,
        CrewCommand::Context { session } => api.show(api.session_get(&session, "context").await?),
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
            _ => bail!("Several Crew connections are saved; choose one with --connection NAME. Run biorouter crew connections list to see them."),
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

    async fn broker(&self, method: &str, mut params: Value, mutation: bool) -> Result<Value> {
        add_personal_mode(method, &mut params, self.expected_mode);
        if mutation {
            params["idempotency_key"] = json!(self.request_id);
        }
        let body = json!({"method":method,"params":params,"request_id":if mutation {Some(&self.request_id)} else {None}});
        self.connection_action("request", body).await
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
        self.connection_action("request", body).await
    }

    async fn snapshot(&self) -> Result<Value> {
        self.broker("workspace.snapshot", json!({}), false).await
    }

    /// Names for text output: the workspace snapshot, when it can be read. JSON output needs
    /// none, and a list is still printed (with "Unknown member") when the snapshot fails.
    async fn names(&self) -> Directory {
        if !self.text() {
            return Directory::default();
        }
        self.snapshot()
            .await
            .map(|snapshot| Directory::from_snapshot(&snapshot))
            .unwrap_or_default()
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
                    Ok(target) => targets[index] = Some(target),
                    Err(problem) => problems.push(problem),
                }
            }
            if !problems.is_empty() {
                bail!("{}", problems.join("\n"));
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

async fn connections(api: &Api, command: ConnectionCommand) -> Result<Reply> {
    Ok(match command {
        ConnectionCommand::List => api.show(api.connections().await?),
        ConnectionCommand::Show => api.show(api.connection().await?),
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
        ConnectionCommand::Remove => api.show(
            api.client
                .request("DELETE", &api.path("").await?, None)
                .await?,
        ),
        ConnectionCommand::JoinInvitation(args) => join_invitation(api, args).await?,
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
    let request = invitation_request(&pasted, &args);
    let answer = from_invitation(api, &request, true).await?;
    let preview = answer
        .get("preview")
        .filter(|preview| preview.is_object())
        .context("The daemon did not describe the invitation")?;
    let summary = invitation_summary(preview);
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
        ensure!(
            !from_stdin,
            "Add --yes to save this connection: the invitation came from stdin, so there is no terminal to ask in. Run with --preview to check it first."
        );
        ensure!(
            api.interactive,
            "Add --yes to save this connection; there is no terminal to ask in. Run with --preview to check it first."
        );
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
    let saved = from_invitation(api, &request, false).await?;
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
    lines.push(format!(
        "Next: biorouter crew --connection {} join",
        safe_text(&shell_word(name))
    ));
    Ok(api.say(saved, lines))
}

/// `POST /crew/connections/from-invitation`'s body: the pasted text and every choice made.
fn invitation_request(pasted: &str, args: &JoinInvitationArgs) -> Value {
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
    for (key, value) in [
        ("ssh_target", &args.ssh_target),
        ("identity_file", &args.identity_file),
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
    body
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

/// What the invitation says and what saving it would do, as the Join screen shows it.
fn invitation_summary(preview: &Value) -> Vec<String> {
    let field = |key: &str| preview[key].as_str().filter(|value| !value.is_empty());
    let workspace = field("workspace_label")
        .or_else(|| field("workspace_name"))
        .map_or_else(|| "a workspace".to_owned(), name_text);
    let mut lines = vec![format!("Invitation to {workspace}")];
    let host = field("host_username").map(|host| person_text(host, field("host_display_name")));
    let server = field("server").or_else(|| field("ssh_host")).map(safe_text);
    match (&host, &server) {
        (Some(host), Some(server)) => lines.push(format!("  Hosted by {host} on {server}")),
        (Some(host), None) => lines.push(format!("  Hosted by {host}")),
        (None, Some(server)) => lines.push(format!("  On {server}")),
        (None, None) => {}
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
    let choice = privacy_badge(field("mode"), field("institution_id"));
    lines.push(format!("  You'll join as {choice}."));
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
    if field("existing_connection_id").is_some() {
        lines.push(
            "  This computer already has this workspace; saving again keeps that connection."
                .into(),
        );
    }
    lines
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
    let path = api.path("/join").await?;
    let mut shown: Option<(String, Option<String>)> = None;
    // Whether the status is being read again right after a claim was refused.
    let mut rechecking = false;
    loop {
        let status = join_status(api, &path).await?;
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
                    if let Some(joined) = claim(api, &path).await? {
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
            () = tokio::time::sleep(api.poll) => {}
            signal = tokio::signal::ctrl_c() => {
                signal?;
                eprintln!("Stopped waiting. Run biorouter crew join again to continue.");
                return Ok(Reply::Streamed);
            }
        }
    }
}

/// `GET …/join`, connecting first when the connection is not up.
async fn join_status(api: &Api, path: &str) -> Result<Value> {
    match api.client.request("GET", path, None).await {
        Err(error)
            if refusal(&error)
                .is_some_and(|refused| refused.code.as_deref() == Some("crew_not_connected")) =>
        {
            api.connection_action("connect", json!({}))
                .await
                .context("Connect to the workspace first: biorouter crew auth")?;
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
                .unwrap_or_else(|| json!([]));
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

/// A device code as the joiner sent it: 16 letters and digits, whatever separates them. The
/// broker applies the full Crockford normalization and refuses what it cannot read.
fn check_device_code(code: &str) -> Result<()> {
    let bare: Vec<char> = code.chars().filter(|c| c.is_alphanumeric()).collect();
    ensure!(
        bare.len() == 16 && bare.iter().all(char::is_ascii_alphanumeric),
        "A code has 16 letters and digits, like 7QK2-M9XA-3JTP-WZ4D. Copy it exactly as they sent it."
    );
    Ok(())
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
/// (`--approval-key-stdin` has already used stdin). A revoke by ID is for scripts and never asks.
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
        None => bail!(
            "Revoking @{username} removes their membership, devices and agent grants. There is no terminal to ask in, so confirm with --confirm @{username}."
        ),
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
            let teams = snapshot_field(&snapshot, "teams")?;
            api.show_with(teams, Directory::from_snapshot(&snapshot))
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

async fn channels(api: &Api, command: ChannelCommand) -> Result<Reply> {
    Ok(match command {
        ChannelCommand::List { team } => {
            let snapshot = api.snapshot().await?;
            let mut channels = snapshot_field(&snapshot, "channels")?;
            if let Some(team) = team {
                let team = api.target(Kind::Team, &team).await?;
                channels = json!(channels
                    .as_array()
                    .context("Invalid channel list")?
                    .iter()
                    .filter(|item| item["team_id"].as_str() == Some(team.id.as_str()))
                    .collect::<Vec<_>>());
            }
            api.show_with(channels, Directory::from_snapshot(&snapshot))
        }
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
        ChannelCommand::Archive { channel } => {
            let channel = api.target(Kind::Channel, &channel).await?;
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
    let channel = api.target(Kind::Channel, channel).await?;
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
            api.show_with(invitations, Directory::from_snapshot(&snapshot))
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
    let (result, added) = match team {
        Some(_) => {
            let (team, channels) = places.split_first().context("A name was left unresolved")?;
            add_to_team(api, who, &username, team, channels).await?
        }
        None => add_to_channels(api, who, &username, places).await?,
    };
    let lines = if api.text() {
        added_lines(api, &username, &added, places).await
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
            Err(error) if !added.is_empty() => {
                return Err(error.context(format!(
                    "@{} was added to some of the channels before this one failed; run the same command again with --request-id {} to finish.",
                    safe_text(username),
                    api.request_id
                )))
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

/// "Added. @bob can now see #general and #methods.", or that nothing needed adding.
async fn added_lines(
    api: &Api,
    username: &str,
    added: &[String],
    places: &[Target],
) -> Vec<String> {
    let handle = format!("@{}", safe_text(username));
    if added.is_empty() {
        return vec![format!("{handle} is already in everything you chose.")];
    }
    let names = api.names().await;
    let seen: Vec<String> = added
        .iter()
        .map(|id| {
            let known = names.channel_label(id);
            if known.starts_with('#') {
                return known;
            }
            places
                .iter()
                .find(|target| target.id == *id)
                .and_then(|target| target.label.as_deref())
                .map_or(known, name_text)
        })
        .collect();
    vec![format!("Added. {handle} can now see {}.", and_list(&seen))]
}

async fn remove_member(api: &Api, channel: &str, member: &str, former: bool) -> Result<Reply> {
    let kind = if former {
        Kind::FormerPerson
    } else {
        Kind::Person
    };
    let targets = api
        .resolve(&[(Kind::Channel, channel), (kind, member)])
        .await?;
    let (channel, who) = (&targets[0], &targets[1]);
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
    let channel = api.target(Kind::Channel, &args.channel).await?;
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
    let channel = api.target(Kind::Channel, &args.channel).await?;
    let path = api.path("/observe").await?;
    let client = api.client.shared()?;
    let mut cursor = args.after;
    let mut names = Directory::default();
    let watched = api.label(&channel, "the channel", "channel ID");
    loop {
        let request = ObserveRequest {
            channel_id: Some(channel.id.clone()),
            after: cursor.clone(),
            initial: Initial::All,
        };
        cursor = tokio::select! {
            result = client.observe(&path, &request, |event| watch_event(api, &mut names, &watched, event)) => result?,
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(Reply::Streamed); }
        };
    }
}

fn watch_event(
    api: &Api,
    names: &mut Directory,
    watched: &str,
    event: ObserveEvent,
) -> Result<std::ops::ControlFlow<Option<String>>> {
    match event {
        // The frame's snapshot names the authors of the messages that follow.
        ObserveEvent::State { snapshot, .. } => {
            if api.text() {
                *names = Directory::from_snapshot(&snapshot);
            }
        }
        ObserveEvent::Messages { messages, .. } => {
            let options = api.human(names.clone());
            for message in messages {
                emit_with(&message, output::stream_format(api.format), &options)?;
            }
        }
        ObserveEvent::Reconnect { cursor } => return Ok(std::ops::ControlFlow::Break(cursor)),
        ObserveEvent::Error { code, error, clear } => {
            if !api.text() {
                emit(
                    &json!({"type":"error","code":code,"error":error,"clear":clear}),
                    output::stream_format(api.format),
                )?;
            }
            return Err(anyhow!(watch_stopped(watched, &error)));
        }
    }
    Ok(std::ops::ControlFlow::Continue(()))
}

/// Why `crew watch` stopped, for a person (Q2-76): the channel, then the observer's own plain
/// sentence. The code stays in the JSON error frame; the cursor is an internal, never named.
fn watch_stopped(watched: &str, error: &str) -> String {
    let sentence = error.trim().trim_end_matches(['.', '!', '?']);
    if sentence.is_empty() {
        format!("Stopped watching {}.", safe_text(watched))
    } else {
        format!(
            "Stopped watching {}: {}.",
            safe_text(watched),
            safe_text(sentence)
        )
    }
}

async fn file_command(api: &Api, mut command: FileCommand) -> Result<Reply> {
    if let FileCommand::Upload { channel, .. } | FileCommand::Reference { channel, .. } =
        &mut command
    {
        *channel = api.target(Kind::Channel, channel).await?.id;
    }
    let result = files::handle(api, command).await?;
    Ok(api.show_with(result, api.names().await))
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
            let targets = api.resolve(&selectors).await?;
            let (destination, context) = targets
                .split_first()
                .context("The task's channel was left unresolved")?;
            let context: Vec<&str> = context.iter().map(|target| target.id.as_str()).collect();
            let started = api.connection_action("runs", api.with_run_policy(json!({"request_id":api.request_id,"channel_id":destination.id,"prompt":prompt,"provider":provider,"model":model,"context_channels":context,"posting_grant":allow_posting}))).await;
            api.show(started.map_err(|error| institution_refusal(error, &model))?)
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
        TaskCommand::Cancel { run } => api.show(
            api.connection_action(
                &format!("runs/{}/cancel", component(&run)?),
                json!({"request_id":api.request_id}),
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
    match found {
        Some((message, details)) if message.contains(AFFILIATION_REFUSAL) => restated(
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

async fn grants(api: &Api, command: GrantCommand) -> Result<Reply> {
    Ok(match command {
        GrantCommand::List => {
            let grants = api
                .client
                .request("GET", &api.path("/grants").await?, None)
                .await?;
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
            let targets = api.resolve(&selectors).await?;
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
            safe_text(&status.replace('_', " "))
        )),
        None => {}
    }
    Ok(lines)
}

async fn privacy(api: &Api, command: PrivacyCommand) -> Result<Reply> {
    Ok(match command {
        PrivacyCommand::Show => {
            let connection = api.connection().await?;
            let snapshot = api.snapshot().await?;
            let value = json!({"connection_id":connection["id"],"personal_mode":connection["mode"],"institution_id":connection["institution_id"],"connection_policy_epoch":connection["policy_epoch"],"workspace":snapshot["workspace"],"channels":snapshot["channels"]});
            api.show_with(value, Directory::from_snapshot(&snapshot))
        }
        PrivacyCommand::SetPersonal {
            mode,
            institution_id,
        } => {
            let connection = api.connection().await?;
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
            api.show(
                api.client
                    .request("PATCH", &api.path("").await?, Some(Value::Object(input)))
                    .await?,
            )
        }
        PrivacyCommand::SetWorkspace {
            mode,
            institution_id,
        } => {
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
        pub(super) message: String,
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
            message: message.to_owned(),
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
            message: message.to_owned(),
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
        };
        (api, fake)
    }

    fn said(reply: Reply) -> Vec<String> {
        match reply {
            Reply::Say(_, lines) => lines,
            Reply::Show(value, options) => vec![output::render_text(&value, &options)],
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
        assert_eq!(
            lines,
            ["Removed \"\u{2068}Bob Lee\u{2069}\" (@bob) from #methods."]
        );
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
            assert!(lines[0].starts_with("Invited \"\u{2068}Bob Lee\u{2069}\" (@bob) to "));
        }

        let (api, fake) = api_with(OutputFormat::Json, standard);
        run(
            &api,
            CrewCommand::RemoveMember {
                channel: "methods".into(),
                member: "@carol".into(),
                former: true,
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
            },
        )
        .await
        .expect_err("unknown names are refused");
        assert_eq!(
            message(&error),
            "You're not in a channel called #raw-data.\nThere's no member @Bob in this workspace. Did you mean @bob? Usernames must match exactly."
        );
        assert!(fake.broker_calls().is_empty());
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
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(join_args(file.path()))),
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
            ["Revoked \"\u{2068}Bob Lee\u{2069}\" (@bob). Their membership, devices and agent grants no longer work."]
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
                CrewCommand::Connections(ConnectionCommand::JoinInvitation(join_args(file.path()))),
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
        assert!(lines.contains(
            &"  Hosted by \"\u{2068}Alice Chen\u{2069}\" (@alice) on hpc.ucsf.edu".to_owned()
        ));
        assert!(lines.contains(&"  Workspace privacy: Private · ucsf".to_owned()));
        assert!(lines.contains(&"  You'll join as Private · ucsf.".to_owned()));
        assert_eq!(
            &lines[lines.len() - 2..],
            ["Saved lab.", "Next: biorouter crew --connection lab join"]
        );

        // Without --yes and without a terminal to ask in, nothing is saved.
        let (api, fake) = api_with(OutputFormat::Text, handler);
        let mut args = join_args(file.path());
        args.yes = false;
        let error = run(
            &api,
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(args)),
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
            CrewCommand::Connections(ConnectionCommand::JoinInvitation(join_args(file.path()))),
        )
        .await
        .expect_err("missing choices");
        assert_eq!(
            message(&error),
            "Saving this invitation needs your username on the server (--username), an institution for a Private connection (--institution)."
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
                "\"\u{2068}Alice Chen\u{2069}\" (@alice) invited you to lab.",
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
                    json!({"status": "expired", "inviter": {"username": "alice", "display_name": "Alice Chen"}}),
                );
            }
            standard(method, path, body)
        };
        let (api, _) = api_with(OutputFormat::Json, expired);
        let error = run(&api, CrewCommand::Join(JoinArgs { no_wait: false }))
            .await
            .expect_err("expired");
        assert!(message(&error).starts_with("This invitation expired. Ask "));
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
        assert_eq!(lines, ["Added. @bob can now see #general and #methods."]);
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
        assert_eq!(lines, ["@bob is already in everything you chose."]);
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
        assert_eq!(lines, ["Added. @bob can now see #methods."]);

        let (api, fake) = api_with(OutputFormat::Text, standard);
        let error = run(&api, add_member_command(None, &[]))
            .await
            .expect_err("nowhere to add them");
        assert!(message(&error).starts_with("Choose where to add them"));
        assert!(fake.broker_calls().is_empty());
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
                "gpt-5.5 is approved for ucsf. foreign-lab uses stanford. Choose a model approved for it, or a local model.",
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
                        message: DAEMON.into(),
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

        // Without a broker code (another route's refusal), the text is unchanged.
        let other = refuse(
            404,
            Some("crew_grant_not_found"),
            "No Crew grant for this session.",
        );
        assert_eq!(
            failure(&other, OutputFormat::Text, "req-1", false).to_string(),
            "Daemon returned 404: No Crew grant for this session."
        );
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
        assert_eq!(
            person_text("bob", Some("Bob Lee")),
            "\"\u{2068}Bob Lee\u{2069}\" (@bob)"
        );
        assert_eq!(person_text("bob", Some("BOB")), "@bob");
        assert_eq!(shell_word("UCSF HPC"), "'UCSF HPC'");
        assert_eq!(shell_word("lab"), "lab");
        assert_eq!(loose_key("#Data Analysis"), "data-analysis");
        assert_eq!(loose_key("Analysis_Lab."), "analysis-lab");
    }
}
