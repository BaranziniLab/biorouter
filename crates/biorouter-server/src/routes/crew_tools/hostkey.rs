//! A server's host key from a chat (the owner's D1, D2 and D6 rulings on AO-12, WI-2): the step a
//! host-key refusal names, the fingerprints the daemon read, and the host-key card a chat shows.
//!
//! **A chat never accepts a host key.** Crew forces `StrictHostKeyChecking yes` on every hop, so
//! a server whose key this computer does not know is refused until the person checks the key with
//! their server's IT team and adds it in a terminal of their own. The card is
//! [`CrewStepRequest::HostKey`], and every field on it comes from the daemon:
//!
//! * the refusal's own words: a refused action's [`SshFailure`] (`detail`, which the transport
//!   summarizes with every fingerprint first, and `host`, the name OpenSSH looked the key up by),
//!   or a failed setup job's [`biorouter::crew::SetupError`] (whose `detail` is OpenSSH's raw
//!   words), read by [`fingerprints`] the way the desktop's `hostKeyFingerprints` and the
//!   transport read them;
//! * for an unknown key whose refusal printed no fingerprint (OpenSSH prints none under strict
//!   checking), the key the daemon reads itself from the same host and port with `ssh-keyscan`
//!   ([`KeyRefusal::read_offered`]), and only when the refused hop is the route's destination
//!   reached straight from this computer ([`reached_directly`]): no jump host, no ProxyCommand,
//!   no HostKeyAlias. Behind a jump host a scan from here could reach another machine, so none is
//!   made, the card shows no fingerprint, and any key the person adds for the host counts. A key
//!   the daemon read is never called the one the server offered: the model, the job's detail and
//!   the refusal say this computer read it ([`READ_HERE`]).
//!
//! Never anything from the model's parameters, which can name a server but not its key.
//!
//! What happens next:
//!
//! * **An unknown key** parks the card and waits ([`KEY_WAIT`], Stop or a dismissal) while the
//!   daemon reads the known-hosts file each second (`ssh-keygen -F`, [`listed_fingerprints`]) for
//!   the key the card shows. Once the person has added it, the card is withdrawn and the step is
//!   tried again, once ([`try_again`]): the action runs as the model called it, and the approval
//!   the person gave earlier in this call is used once more, for the identical card only, while
//!   an approval card would still have been answerable (`APPROVAL_TTL`). Any other card the
//!   retry needs is asked (`retrying_after_a_host_key` and `allowed_for_this_retry` in the
//!   parent module).
//! * **A changed key** never waits and never offers a fix. Its card (the details for IT) is
//!   published and released at once, and the call answers with [`CHANGED_STEP`].
//! * **No card can be shown** (`biorouter serve`, ACP, `POST /agent/call_tool`): the refusal is
//!   said in words, as before.
//!
//! The model is told the fingerprint (it is not a secret) and [`UNKNOWN_STEP`] or
//! [`CHANGED_STEP`], which forbid it to add, scan or accept a key itself. In Fully Automatic mode
//! criterion 7 of `security::sensitive_ops` backs that: an agent shell that would trust a key asks
//! first.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use base64::Engine as _;
use biorouter::conversation::message::CrewStepRequest;
use biorouter::crew::authentication::{HandoffFailed, InvitationRefused, JoinRefused};
use biorouter::crew::{
    Connection, SetupRefused, SetupState, SetupStatus, SshFailure, SshFailureKind,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use super::{catalog, job_answer, job_target, said, tool_error, Ctx, Said, StepEnd};

/// The code of a refusal on a host key this computer does not know yet.
const UNKNOWN: &str = "crew_ssh_host_key_unknown";
/// The code of a refusal on a host key that differs from the one this computer trusts.
const CHANGED: &str = "crew_ssh_host_key_changed";
/// How long an unknown key's card waits for the person to add the key (capped by `park_step`).
const KEY_WAIT: Duration = Duration::from_secs(600);
/// How often the known-hosts file is read while the card waits.
const KEY_POLL: Duration = Duration::from_secs(1);
/// How long one `ssh-keygen -F` or `ssh-keyscan` may take.
const KEY_TOOL_LIMIT: Duration = Duration::from_secs(10);
/// How much of their output is read.
const KEY_TOOL_OUTPUT: u64 = 64 * 1024;
/// The key types `ssh-keyscan -t` is asked for, as OpenSSH names them in "No ED25519 host key is
/// known for …".
const KEY_TYPES: &[&str] = &["ed25519", "ecdsa", "rsa", "dsa"];

/// The step an unknown key's refusal names for the model.
pub(crate) const UNKNOWN_STEP: &str = "Do not add, scan or accept the key yourself (no ssh-keyscan, no StrictHostKeyChecking=accept-new or no, no edits to known_hosts); Crew never accepts a host key for the person. They check the fingerprint with their server's IT team and add the key in a terminal of their own; then try again.";
/// The step a changed key's refusal names for the model: the IT wording, with no fix.
pub(crate) const CHANGED_STEP: &str = "Do not add, scan, remove or accept a key yourself (no ssh-keyscan, no ssh-keygen -R, no StrictHostKeyChecking=accept-new or no, no edits to known_hosts). The server's host key changed since this computer trusted it, so nothing connects until the person's server IT team confirms the change.";
/// How a refusal's detail labels a key the daemon read itself with `ssh-keyscan`
/// ([`KeyRefusal::detail_with_offered`]). Never the transport's "Offered host key fingerprint":
/// no server offered it on the refused connection; this computer asked for it.
const READ_HERE: &str = "Host key fingerprint read by this computer:";

tokio::task_local! {
    /// Set while the step a host-key card waited for is tried again, so that try is the only
    /// one: a second refusal is said, not carded.
    static TRYING_AGAIN: ();
}

/// The step a host-key refusal names for the model ([`super::next_step`]), or `None` when `code`
/// is not a host key's.
pub(super) fn next_step(code: &str, _connection: Option<&str>) -> Option<String> {
    match code {
        UNKNOWN => Some(UNKNOWN_STEP.to_owned()),
        CHANGED => Some(CHANGED_STEP.to_owned()),
        _ => None,
    }
}

/// `text`, the sentence the model is told for a refusal or a failed job's step, with the
/// fingerprints the daemon read added. `code` is the refusal's code; `fields` are the daemon's
/// own words beside it: an admission refusal's fields (`AdmissionRefusal::fields`), or a failed
/// setup job's `detail` and `host`. Called by [`super::tool_error`] and [`super::job_answer`].
/// Only a fingerprint-shaped value and a host-shaped name are ever repeated, never the words
/// around them.
pub(super) fn enrich(text: String, code: Option<&str>, fields: &[(String, Value)]) -> String {
    let changed = match code {
        Some(UNKNOWN) => false,
        Some(CHANGED) => true,
        _ => return text,
    };
    let field = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.as_str())
    };
    let detail = field("detail").unwrap_or_default();
    let (offered, known) = fingerprints(detail);
    let host = field("host")
        .and_then(plain_lookup)
        .or_else(|| named_host(changed, detail));
    match key_sentence(
        changed,
        host.as_deref(),
        (offered.as_deref(), read_by_this_computer(detail)),
        known.as_deref(),
    ) {
        Some(sentence) => format!("{text} {sentence}"),
        None => text,
    }
}

/// One sentence with the fingerprints, for the model. `offered` is the fingerprint and whether
/// this computer read it itself ([`READ_HERE`]), which is then said as that, never as the key
/// the server offered.
fn key_sentence(
    changed: bool,
    host: Option<&str>,
    (offered, read_here): (Option<&str>, bool),
    known: Option<&str>,
) -> Option<String> {
    let whose = host.map_or_else(|| "The server's".to_owned(), |host| format!("{host}'s"));
    Some(match (changed, offered, known) {
        (_, None, None) => return None,
        (false, Some(offered), _) if read_here => {
            let whose = host.map_or_else(|| "the server's".to_owned(), |host| format!("{host}'s"));
            format!("This computer read {whose} host key fingerprint as {offered}.")
        }
        (false, Some(offered), _) => format!("{whose} host key fingerprint is {offered}."),
        (true, Some(offered), Some(known)) => {
            format!("{whose} host key fingerprint is now {offered}; this computer trusted {known}.")
        }
        (true, Some(offered), None) => format!("{whose} host key fingerprint is now {offered}."),
        (_, None, Some(known)) => format!("This computer trusted the host key {known}."),
    })
}

/// Called by [`Ctx::wait_for_job`] with every status it reads of the job it waits for, after
/// [`super::signin::on_status`]. A job that failed on an unknown host key parks the card, waits
/// for the person, and answers with the job tried again once they added the key; a changed key
/// shows its card and lets the wait answer as before. `None` lets the wait go on.
pub(super) async fn on_status(ctx: &Ctx, status: &SetupStatus) -> Option<Value> {
    if !matches!(status.state, SetupState::Failed) {
        return None;
    }
    let error = status.error.as_ref()?;
    let mut refusal = KeyRefusal::read(
        &error.code,
        error.detail.as_deref(),
        error.host.as_deref(),
        &status.server.login,
    )?;
    if refusal.changed {
        show_changed(ctx, &refusal);
        return None;
    }
    if trying_again() {
        return None;
    }
    refusal
        .read_offered(Route::of_job(&status.job_id).await.as_ref())
        .await;
    let mut seen = status.clone();
    if let Some(error) = seen.error.as_mut() {
        error.detail = refusal.detail_with_offered();
    }
    match wait_for_key(ctx, &refusal).await {
        Some(StepEnd::Withdrawn) => Some(match try_again(ctx).await {
            Ok(answer) => answer,
            Err(error) => {
                let refused = tool_error(error, ctx.named_connection()).await.to_string();
                let mut answer = job_answer(&seen, false);
                answer["tried_again"] = json!(refused);
                answer
            }
        }),
        _ => Some(job_answer(&seen, false)),
    }
}

/// Called with an action's error before it becomes the refusal the model reads
/// ([`super::tool_error`]). A refusal on an unknown host key parks the card, waits for the
/// person, and answers with the action tried again once they added the key; a changed key shows
/// its card. `Err` passes the error (this one, or the same refusal with the key the daemon read)
/// on to be worded.
pub(super) async fn on_refusal(ctx: &Ctx, error: anyhow::Error) -> Result<Value> {
    let Some(failure) = host_key_failure(&error).cloned() else {
        return Err(error);
    };
    let route = refused_route(ctx).await;
    let login = route
        .as_ref()
        .map(|route| route.login.clone())
        .unwrap_or_default();
    let Some(mut refusal) = KeyRefusal::read(
        failure.api_code(),
        failure.detail.as_deref(),
        failure.host.as_deref(),
        &login,
    ) else {
        return Err(error);
    };
    if refusal.changed {
        show_changed(ctx, &refusal);
        return Err(error);
    }
    if trying_again() {
        return Err(error);
    }
    refusal.read_offered(route.as_ref()).await;
    let error = if refusal.scanned {
        anyhow::Error::new(SshFailure {
            detail: refusal.detail_with_offered(),
            ..failure
        })
    } else {
        error
    };
    match wait_for_key(ctx, &refusal).await {
        Some(StepEnd::Withdrawn) => try_again(ctx).await,
        _ => Err(error),
    }
}

/// The host-key refusal `error` carries, when it is worded as one: an [`SshFailure`] on an
/// unknown or changed key that no refusal of another kind wraps, in the order
/// `routes::crew_authentication::core_refusal` classifies them (an action's own words, a setup
/// refusal, an invitation's, a join's and a handoff's come first).
fn host_key_failure(error: &anyhow::Error) -> Option<&SshFailure> {
    let worded_otherwise = error.chain().any(|cause| {
        cause.is::<Said>()
            || cause.is::<SetupRefused>()
            || cause.is::<InvitationRefused>()
            || cause.is::<JoinRefused>()
            || cause.is::<HandoffFailed>()
    });
    if worded_otherwise {
        return None;
    }
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SshFailure>())
        .filter(|failure| {
            matches!(
                failure.kind,
                SshFailureKind::HostKeyUnknown | SshFailureKind::HostKeyChanged
            )
        })
}

/// The route the refused call took, as the daemon saved it: the saved connection the call
/// named, whose login the card names. `None` otherwise (the model's own words never reach a
/// card): the card then names the host alone, and no key is read ([`reached_directly`]).
async fn refused_route(ctx: &Ctx) -> Option<Route> {
    ctx.named_connection()?;
    let connection = ctx.connection().await.ok()?;
    Some(Route::of(&connection))
}

/// A step's SSH route, as the daemon saved or started it: its login, its port and its jump
/// hosts. What [`reached_directly`] checks before the daemon reads a key itself.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Route {
    login: String,
    port: Option<u16>,
    proxy_jump: Option<String>,
}

impl Route {
    fn of(connection: &Connection) -> Self {
        Self {
            login: connection.ssh_target.clone(),
            port: connection.port,
            proxy_jump: connection.proxy_jump.clone(),
        }
    }

    /// The route of the setup job `job_id`, as this host started it ([`job_target`]): its saved
    /// connection's, or the login, port and jump hosts it was started with. `None` for a job
    /// started elsewhere (Crew, a terminal), whose route this host never saw.
    async fn of_job(job_id: &str) -> Option<Self> {
        let target = job_target(job_id)?;
        if target["kind"] == "connection" {
            let id = target["connection_id"].as_str()?;
            let connection = biorouter::crew::manager().ok()?.connection(id).await.ok()?;
            return Some(Self::of(&connection));
        }
        Some(Self {
            login: target["ssh_target"].as_str()?.to_owned(),
            port: target["port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok()),
            proxy_jump: target["proxy_jump"].as_str().map(str::to_owned),
        })
    }

    /// Whether the route names a jump host.
    fn jumps(&self) -> bool {
        self.proxy_jump
            .as_deref()
            .map(str::trim)
            .is_some_and(|jump| !jump.is_empty() && jump != "none")
    }
}

/// Whether the hop OpenSSH refused is `route`'s destination, reached straight from this
/// computer, so that `ssh-keyscan` to the name and port the refusal gave reads the very server
/// the step reached. It is not when the route or the person's SSH configuration (`ssh -G`, read
/// as the transport reads it) names a jump host or a ProxyCommand, when a HostKeyAlias makes the
/// name the key is filed under differ from the one connected to, or when the name and port
/// `ssh -G` resolves are not the ones OpenSSH looked the key up by. Anything that can't be read
/// counts as not direct.
async fn reached_directly(route: &Route, refusal: &KeyRefusal) -> bool {
    if route.jumps() || route.login.is_empty() || route.login.starts_with('-') {
        return false;
    }
    let mut command = tokio::process::Command::new("ssh");
    command.arg("-G");
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        command
            .arg("-F")
            .arg(PathBuf::from(profile).join("home/.ssh/config"));
    }
    if let Some(port) = route.port {
        command.arg("-p").arg(port.to_string());
    }
    command.arg(&route.login);
    let Some(resolved) = run_bounded(command).await else {
        return false;
    };
    resolved_directly(&resolved, refusal)
}

/// [`reached_directly`]'s reading of what `ssh -G` printed for the route.
fn resolved_directly(resolved: &str, refusal: &KeyRefusal) -> bool {
    let setting = |key: &str| {
        resolved.lines().find_map(|line| {
            let (name, value) = line.split_once(' ')?;
            (name == key).then(|| value.trim())
        })
    };
    let unset = |key: &str| setting(key).is_none_or(|value| value == "none");
    unset("proxyjump")
        && unset("proxycommand")
        && unset("hostkeyalias")
        && setting("hostname").is_some_and(|name| name.eq_ignore_ascii_case(&refusal.host))
        && setting("port").and_then(|port| port.parse::<u16>().ok()) == Some(refusal.port)
}

/// Whether this is the one time a step a host-key card waited for is tried again.
fn trying_again() -> bool {
    TRYING_AGAIN.try_with(|_| ()).is_ok()
}

/// Run the call's action again, once, as the model called it: after the person added the key
/// the card waited for. The approval the person gave for the identical card in this call stands
/// for it (`Ctx::approve`); any other card is asked. A second host-key refusal is said, not
/// carded.
async fn try_again(ctx: &Ctx) -> Result<Value> {
    let Some(run) = ctx.action.run else {
        return Err(said(None, catalog::NOT_YET));
    };
    TRYING_AGAIN.scope((), run(ctx)).await
}

/// A changed key's card: published for the person (the details for IT) and released at once,
/// since nothing waits on it and nothing can fix it from a chat. No card where none can be shown.
fn show_changed(ctx: &Ctx, refusal: &KeyRefusal) {
    drop(ctx.park_step(refusal.card(), KEY_WAIT));
}

/// Park an unknown key's card and wait for it to end: withdrawn once the known-hosts file holds
/// the key the card shows, or dismissed, stopped or out of time. `None` when no card could be
/// shown.
async fn wait_for_key(ctx: &Ctx, refusal: &KeyRefusal) -> Option<StepEnd> {
    let step = ctx.park_step(refusal.card(), KEY_WAIT)?;
    let withdraw = step.withdrawer();
    let files = known_hosts_files();
    let end = step.end();
    tokio::pin!(end);
    loop {
        tokio::select! {
            how = &mut end => return Some(how),
            () = tokio::time::sleep(KEY_POLL) => {
                if key_added(refusal, &files).await {
                    withdraw.cancel();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The refusal, as the daemon read it
// ---------------------------------------------------------------------------------------------

/// A host-key refusal as the daemon read it: everything the card shows comes from here.
#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyRefusal {
    changed: bool,
    /// The name the known-hosts file holds the key under, as OpenSSH looked it up: `host`, or
    /// `[host]:port` off port 22.
    lookup: String,
    host: String,
    port: u16,
    /// The SSH login the step used, when the call named one.
    login: String,
    offered: Option<String>,
    known: Option<String>,
    /// The key type an unknown key's refusal names (`ed25519`), so the scan reads that key.
    key_type: Option<String>,
    /// OpenSSH's words, as the daemon kept them.
    detail: Option<String>,
    /// `offered` is the key the daemon read itself, not one the refusal printed.
    scanned: bool,
}

impl KeyRefusal {
    /// A refusal from its code and the daemon's words: `detail`, `host` (OpenSSH's name for
    /// the hop) and `login`. `None` for any other code, or when no host can be named.
    fn read(code: &str, detail: Option<&str>, host: Option<&str>, login: &str) -> Option<Self> {
        let changed = match code {
            UNKNOWN => false,
            CHANGED => true,
            _ => return None,
        };
        let text = detail.unwrap_or_default();
        let lookup = host
            .and_then(plain_lookup)
            .or_else(|| named_host(changed, text))
            .or_else(|| plain_lookup(login.rsplit_once('@').map_or(login, |(_, host)| host)))?;
        let (host, port) = split_lookup(&lookup)?;
        let (offered, known) = fingerprints(text);
        Some(Self {
            changed,
            lookup,
            host,
            port,
            login: login.to_owned(),
            offered,
            known,
            key_type: if changed { None } else { key_type(text) },
            detail: detail.map(str::to_owned),
            scanned: read_by_this_computer(text),
        })
    }

    /// The card: the daemon's values, nothing else.
    fn card(&self) -> CrewStepRequest {
        CrewStepRequest::HostKey {
            host: self.host.clone(),
            port: self.port,
            login: if self.login.is_empty() {
                self.host.clone()
            } else {
                self.login.clone()
            },
            offered: self.offered.clone(),
            known: self.known.clone(),
            changed: self.changed,
        }
    }

    /// For an unknown key whose refusal printed no fingerprint: the key the host gives this
    /// computer now, read by the daemon with `ssh-keyscan` from the host and port OpenSSH named,
    /// of the type it named, and only when the refused hop is `route`'s destination reached
    /// straight from here ([`reached_directly`]). Nothing when the route is not known or not
    /// direct, the type is not one `ssh-keyscan` reads, or the scan finds no key: the card then
    /// shows no fingerprint, and any key the person adds for the host counts ([`key_added`]).
    async fn read_offered(&mut self, route: Option<&Route>) {
        if self.changed || self.offered.is_some() {
            return;
        }
        let Some(key_type) = self.key_type.clone() else {
            return;
        };
        let Some(route) = route else {
            return;
        };
        if !reached_directly(route, self).await {
            return;
        }
        let port = self.port.to_string();
        let mut command = tokio::process::Command::new("ssh-keyscan");
        command
            .args(["-T", "5", "-t", key_type.as_str(), "-p", port.as_str()])
            .arg(&self.host);
        let Some(output) = run_bounded(command).await else {
            return;
        };
        let offered = output
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .find_map(|line| {
                let mut words = line.split_whitespace();
                let (_host, _type, blob) = (words.next()?, words.next()?, words.next()?);
                fingerprint_of(blob)
            });
        if offered.is_some() {
            self.offered = offered;
            self.scanned = true;
        }
    }

    /// The daemon's words with the key it read itself first, labelled as read by this computer
    /// ([`READ_HERE`]), never as the key the server offered, so every reader of the refusal
    /// finds it ([`fingerprints`]) and says what it is ([`enrich`]).
    fn detail_with_offered(&self) -> Option<String> {
        match (&self.offered, self.scanned, &self.detail) {
            (Some(offered), true, Some(detail)) => {
                Some(format!("{READ_HERE} {offered}\n\n{detail}"))
            }
            (Some(offered), true, None) => Some(format!("{READ_HERE} {offered}")),
            _ => self.detail.clone(),
        }
    }
}

/// A name OpenSSH looks a key up by, when it is only that: letters, digits and `.-_:[]`, at
/// most 255 bytes, not starting with `-`. Anything else is not repeated anywhere.
fn plain_lookup(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches(['.', ',', ';']);
    (!text.is_empty()
        && text.len() <= 255
        && !text.starts_with('-')
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:[]".contains(&b)))
    .then(|| text.to_owned())
}

/// `[host]:port` as its host and port; a plain name is on port 22.
fn split_lookup(lookup: &str) -> Option<(String, u16)> {
    let Some(inner) = lookup.strip_prefix('[') else {
        return Some((lookup.to_owned(), 22));
    };
    let (host, port) = inner.split_once("]:")?;
    let port = port.parse().ok()?;
    (!host.is_empty() && !host.contains(['[', ']'])).then(|| (host.to_owned(), port))
}

/// The host OpenSSH's deciding line names: `No ED25519 host key is known for H and …`, or
/// `Host key for H has changed …`. The last such line wins, as the transport reads it.
fn named_host(changed: bool, text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        let needle = if changed {
            if !lower.contains(" has changed") {
                return None;
            }
            "host key for "
        } else {
            " host key is known for "
        };
        let at = lower.find(needle)? + needle.len();
        plain_lookup(line.get(at..)?.split_whitespace().next()?)
    })
}

/// The key type an unknown key's refusal names (`No ED25519 host key is known for …`), as
/// `ssh-keyscan -t` takes it.
fn key_type(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        let (before, _) = lower.split_once(" host key is known for ")?;
        let named = before.trim().strip_prefix("no ")?.trim();
        KEY_TYPES
            .iter()
            .find(|known| **known == named)
            .map(|known| (*known).to_owned())
    })
}

/// Whether `text` is a SHA-256 host-key fingerprint and nothing else.
fn is_fingerprint(text: &str) -> bool {
    text.strip_prefix("SHA256:").is_some_and(|digest| {
        (32..=64).contains(&digest.len())
            && digest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    })
}

/// Every SHA-256 fingerprint in `line`, as the transport finds them.
fn sha256_values(line: &str) -> Vec<String> {
    const MARKER: &str = "SHA256:";
    line.match_indices(MARKER)
        .filter_map(|(at, _)| {
            let digest: String = line
                .get(at + MARKER.len()..)?
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
                .collect();
            let value = format!("{MARKER}{digest}");
            is_fingerprint(&value).then_some(value)
        })
        .collect()
}

/// The offered and the known fingerprint in a refusal's detail: the transport's summary lines
/// first (`Offered host key fingerprint: …`, exactly as the desktop's `hostKeyFingerprints`
/// reads them), else OpenSSH's raw words, labelled by the words around each fingerprint as the
/// transport labels them before it summarizes. An unlabelled fingerprint stands for the offered
/// one, as on the desktop.
fn fingerprints(detail: &str) -> (Option<String>, Option<String>) {
    let summary = summary_fingerprints(detail);
    if summary != (None, None) {
        return summary;
    }
    raw_fingerprints(detail)
}

fn summary_fingerprints(detail: &str) -> (Option<String>, Option<String>) {
    let summary = Summary::of(detail);
    (
        summary.offered.or(summary.unlabeled).or(summary.read_here),
        summary.known,
    )
}

/// Whether the fingerprint [`fingerprints`] reads from `detail` is one this computer read itself
/// ([`READ_HERE`]): the summary carries no other.
fn read_by_this_computer(detail: &str) -> bool {
    let summary = Summary::of(detail);
    summary.offered.is_none() && summary.unlabeled.is_none() && summary.read_here.is_some()
}

/// The fingerprints of a refusal's summary lines, each under its label.
#[derive(Default)]
struct Summary {
    offered: Option<String>,
    known: Option<String>,
    unlabeled: Option<String>,
    read_here: Option<String>,
}

impl Summary {
    fn of(detail: &str) -> Self {
        const OFFERED: [&str; 2] = [
            "New host key fingerprint (offered by the server):",
            "Offered host key fingerprint:",
        ];
        const KNOWN: &str = "Previously known host key fingerprint:";
        const UNLABELED: &str = "Host key fingerprint:";
        let mut summary = Self::default();
        for line in detail.lines().map(str::trim) {
            let value = |label: &str| {
                line.strip_prefix(label)
                    .map(str::trim)
                    .filter(|value| is_fingerprint(value))
                    .map(str::to_owned)
            };
            if let Some(found) = OFFERED.iter().find_map(|label| value(label)) {
                summary.offered.get_or_insert(found);
            } else if let Some(found) = value(KNOWN) {
                summary.known.get_or_insert(found);
            } else if let Some(found) = value(READ_HERE) {
                summary.read_here.get_or_insert(found);
            } else if let Some(found) = value(UNLABELED) {
                summary.unlabeled.get_or_insert(found);
            }
        }
        summary
    }
}

fn raw_fingerprints(text: &str) -> (Option<String>, Option<String>) {
    const OFFERED: [&str; 4] = [
        "sent by the remote host",
        "key fingerprint is",
        "server host key",
        "offered",
    ];
    const KNOWN: [&str; 4] = ["known_hosts", "known host", "previously", "expected"];
    let lines: Vec<&str> = text.lines().collect();
    let (mut offered, mut known, mut unlabeled) = (None, None, None);
    for (index, line) in lines.iter().enumerate() {
        let previous = index.checked_sub(1).map_or("", |at| lines[at]);
        let context = format!("{previous} {line}").to_lowercase();
        for value in sha256_values(line) {
            if OFFERED.iter().any(|needle| context.contains(needle)) {
                offered.get_or_insert(value);
            } else if KNOWN.iter().any(|needle| context.contains(needle)) {
                known.get_or_insert(value);
            } else {
                unlabeled.get_or_insert(value);
            }
        }
    }
    (offered.or(unlabeled), known)
}

/// `SHA256:…` for an OpenSSH public key blob (the base64 word of a known-hosts or
/// `ssh-keyscan` line), as `ssh-keygen -l` prints it.
fn fingerprint_of(blob: &str) -> Option<String> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .ok()?;
    let digest = Sha256::digest(&key);
    Some(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    ))
}

// ---------------------------------------------------------------------------------------------
// The known-hosts file
// ---------------------------------------------------------------------------------------------

/// The known-hosts files the daemon's ssh reads: the development profile's (the
/// `UserKnownHostsFile` the transport passes), else OpenSSH's defaults in the home folder.
fn known_hosts_files() -> Vec<PathBuf> {
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        return vec![PathBuf::from(profile).join("home/.ssh/known_hosts")];
    }
    biorouter::config::paths::Paths::home_dir()
        .map(|home| {
            vec![
                home.join(".ssh/known_hosts"),
                home.join(".ssh/known_hosts2"),
            ]
        })
        .unwrap_or_default()
}

/// Whether `files` now hold the key the card shows for the refusal's host: the fingerprint the
/// card shows, or, where it shows none, any key the person added for the host. A revoked key or
/// a certificate authority does not count.
async fn key_added(refusal: &KeyRefusal, files: &[PathBuf]) -> bool {
    let mut listed = Vec::new();
    for file in files.iter().filter(|file| file.is_file()) {
        let mut command = tokio::process::Command::new("ssh-keygen");
        command
            .args(["-l", "-F", refusal.lookup.as_str(), "-f"])
            .arg(file);
        if let Some(output) = run_bounded(command).await {
            listed.extend(listed_fingerprints(&output));
        }
    }
    match &refusal.offered {
        Some(offered) => listed.iter().any(|found| found == offered),
        None => !listed.is_empty(),
    }
}

/// The fingerprints of the plain entries `ssh-keygen -l -F` printed. Each entry follows a
/// `# Host NAME found: line N` header, which ends in `REVOKED` or `CA` for a marked one.
fn listed_fingerprints(output: &str) -> Vec<String> {
    let mut plain = false;
    let mut found = Vec::new();
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("# Host ") {
            plain = header
                .rsplit_once(" found: line ")
                .is_some_and(|(_, rest)| rest.split_whitespace().nth(1).is_none());
            continue;
        }
        if std::mem::take(&mut plain) {
            found.extend(sha256_values(line));
        }
    }
    found
}

/// Run a key tool (`ssh-keygen`, `ssh-keyscan`) as the daemon's own child: no input, its
/// output bounded, killed past [`KEY_TOOL_LIMIT`]. `None` when it could not run or said
/// nothing readable.
async fn run_bounded(mut command: tokio::process::Command) -> Option<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    biorouter::subprocess::prepare_agent_child_command(&mut command);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let ran = async move {
        let mut bytes = Vec::new();
        stdout
            .take(KEY_TOOL_OUTPUT)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        let _ = child.wait().await;
        Some(bytes)
    };
    let bytes = tokio::time::timeout(KEY_TOOL_LIMIT, ran).await.ok()??;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_ctx;
    use super::*;
    use biorouter::action_required_manager::ActionRequiredManager;
    use biorouter::conversation::message::{ActionRequiredData, MessageContent};
    use biorouter::crew::tool_host::PersonTool;

    /// A key OpenSSH printed as `ssh-ed25519 <blob>`, and `ssh-keygen -l`'s fingerprint of it.
    const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIDVMyBQ3psSwstgc8+Mce5VHIZN5GFuu3xE6WV8pfFkD";
    const BLOB_FINGERPRINT: &str = "SHA256:XA01Ou0eChdPDyZiadGxCZ/g+7RnODImHcZEtuRmV8g";
    const OFFERED: &str = "SHA256:TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmU";
    const KNOWN: &str = "SHA256:Wm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm8";

    fn changed_banner() -> String {
        [
            "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
            "The fingerprint for the ED25519 key sent by the remote host is",
            &format!("{OFFERED}."),
            "Offending ED25519 key in /Users/alice/.ssh/known_hosts:3",
            "Host key for hpc.example.edu has changed and you have requested strict checking.",
            "Host key verification failed.",
        ]
        .join("\n")
    }

    fn failure(kind: SshFailureKind, detail: &str, host: Option<&str>) -> SshFailure {
        SshFailure {
            kind,
            code: "ssh_eof".into(),
            status: "exit_255".into(),
            description: "SSH exited".into(),
            detail: Some(detail.into()),
            host: host.map(str::to_owned),
            outcome_unknown: false,
        }
    }

    #[test]
    fn each_host_key_refusal_names_its_own_step() {
        let unknown = next_step(UNKNOWN, None).unwrap();
        assert!(
            unknown.starts_with("Do not add, scan or accept the key yourself"),
            "{unknown}"
        );
        assert!(unknown.contains("no ssh-keyscan"), "{unknown}");
        assert!(unknown.contains("never accepts a host key"), "{unknown}");
        assert!(unknown.contains("then try again"), "{unknown}");
        let changed = next_step(CHANGED, None).unwrap();
        assert!(changed.contains("IT team confirms the change"), "{changed}");
        assert!(changed.contains("no ssh-keygen -R"), "{changed}");
        assert!(!changed.contains("try again"), "{changed}");
        assert!(next_step("crew_ssh_auth_required", None).is_none());
        for text in [unknown, changed] {
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
        }
    }

    /// The transport's summary, a changed key's raw banner and an unlabelled fingerprint are
    /// each read; a digest too short to be one, or words around a fingerprint, are not.
    #[test]
    fn fingerprints_are_read_from_the_daemon_s_words_only() {
        assert_eq!(
            fingerprints(&format!(
                "Offered host key fingerprint: {OFFERED}\nPreviously known host key fingerprint: {KNOWN}\n\nraw words"
            )),
            (Some(OFFERED.into()), Some(KNOWN.into()))
        );
        assert_eq!(
            fingerprints(&format!(
                "New host key fingerprint (offered by the server): {OFFERED}"
            )),
            (Some(OFFERED.into()), None)
        );
        assert_eq!(
            fingerprints(&changed_banner()),
            (Some(OFFERED.into()), None)
        );
        assert_eq!(
            fingerprints(&format!("Host key fingerprint: {OFFERED}")),
            (Some(OFFERED.into()), None)
        );
        assert_eq!(
            fingerprints(
                "Offered host key fingerprint: SHA256:short\nNo ED25519 host key is known for h"
            ),
            (None, None)
        );
        assert_eq!(
            fingerprints(&format!("Offered host key fingerprint: {OFFERED} <script>")),
            (Some(OFFERED.into()), None),
            "a raw line still yields only the fingerprint itself"
        );
    }

    #[test]
    fn the_host_and_key_type_come_from_openssh_s_deciding_line() {
        let unknown = "No ED25519 host key is known for [127.0.0.1]:2222 and you have requested strict checking.\nHost key verification failed.";
        let refusal = KeyRefusal::read(UNKNOWN, Some(unknown), None, "you@lab").unwrap();
        assert_eq!(refusal.lookup, "[127.0.0.1]:2222");
        assert_eq!((refusal.host.as_str(), refusal.port), ("127.0.0.1", 2222));
        assert_eq!(refusal.key_type.as_deref(), Some("ed25519"));
        assert_eq!(refusal.offered, None);

        let changed = KeyRefusal::read(CHANGED, Some(&changed_banner()), None, "").unwrap();
        assert_eq!(changed.lookup, "hpc.example.edu");
        assert_eq!(changed.offered.as_deref(), Some(OFFERED));
        assert!(changed.key_type.is_none(), "a changed key is never scanned");

        // The refusal's own host field wins; a host that is not a plain name is never used.
        let named = KeyRefusal::read(UNKNOWN, Some(unknown), Some("gate.example.edu"), "").unwrap();
        assert_eq!(named.lookup, "gate.example.edu");
        let spoofed = KeyRefusal::read(
            UNKNOWN,
            Some("No ED25519 host key is known for evil<script> and you have requested strict checking."),
            Some("-oProxyCommand=x"),
            "you@hpc.example.edu",
        )
        .unwrap();
        assert_eq!(
            spoofed.lookup, "hpc.example.edu",
            "falls back to the login's host"
        );
        assert!(KeyRefusal::read("crew_ssh_auth_required", Some(unknown), None, "h").is_none());
    }

    #[test]
    fn the_model_text_carries_the_step_and_the_fingerprints() {
        let fields = vec![
            (
                "detail".to_owned(),
                json!(format!("Offered host key fingerprint: {OFFERED}")),
            ),
            ("host".to_owned(), json!("hpc.example.edu")),
        ];
        let text = enrich("Said.".into(), Some(UNKNOWN), &fields);
        assert_eq!(
            text,
            format!("Said. hpc.example.edu's host key fingerprint is {OFFERED}.")
        );
        let changed = vec![(
            "detail".to_owned(),
            json!(format!(
                "New host key fingerprint (offered by the server): {OFFERED}\nPreviously known host key fingerprint: {KNOWN}"
            )),
        )];
        let text = enrich("Said.".into(), Some(CHANGED), &changed);
        assert!(
            text.contains(&format!("is now {OFFERED}; this computer trusted {KNOWN}")),
            "{text}"
        );
        // Other refusals, and words that are not a fingerprint, pass unchanged.
        assert_eq!(
            enrich("Said.".into(), Some("crew_ssh_auth_required"), &fields),
            "Said."
        );
        let words = vec![(
            "detail".to_owned(),
            json!("Offered host key fingerprint: trust me"),
        )];
        assert_eq!(enrich("Said.".into(), Some(UNKNOWN), &words), "Said.");
    }

    /// A key the daemon read itself is labelled as read by this computer in the refusal's
    /// detail, read back as that, and said to the model as that: never as the key the server
    /// offered.
    #[test]
    fn a_key_this_computer_read_is_never_called_the_server_s_offer() {
        let unknown = "No ED25519 host key is known for hpc.example.edu and you have requested strict checking.";
        let mut refusal =
            KeyRefusal::read(UNKNOWN, Some(unknown), None, "you@hpc.example.edu").unwrap();
        refusal.offered = Some(BLOB_FINGERPRINT.into());
        refusal.scanned = true;
        let detail = refusal.detail_with_offered().unwrap();
        assert!(
            detail.starts_with(&format!("{READ_HERE} {BLOB_FINGERPRINT}\n\n")),
            "{detail}"
        );
        assert!(!detail.to_lowercase().contains("offered"), "{detail}");
        assert_eq!(fingerprints(&detail), (Some(BLOB_FINGERPRINT.into()), None));
        assert!(read_by_this_computer(&detail));
        let again = KeyRefusal::read(UNKNOWN, Some(&detail), None, "you@hpc.example.edu").unwrap();
        assert!(again.scanned);
        assert_eq!(again.offered.as_deref(), Some(BLOB_FINGERPRINT));

        let fields = vec![
            ("detail".to_owned(), json!(detail)),
            ("host".to_owned(), json!("hpc.example.edu")),
        ];
        assert_eq!(
            enrich("Said.".into(), Some(UNKNOWN), &fields),
            format!(
                "Said. This computer read hpc.example.edu's host key fingerprint as {BLOB_FINGERPRINT}."
            )
        );
        let offered = format!("Offered host key fingerprint: {OFFERED}");
        assert!(!read_by_this_computer(&offered));
        // The transport's own label beside it wins: that one the server offered.
        assert!(!read_by_this_computer(&format!(
            "{offered}\n{READ_HERE} {KNOWN}"
        )));
    }

    /// The daemon reads a key itself only for the route's destination reached straight from
    /// this computer: never behind a jump host, a ProxyCommand or a HostKeyAlias, and never when
    /// `ssh -G` resolves another name or port than the one the key was looked up by.
    #[tokio::test]
    async fn a_key_is_read_from_here_only_for_a_direct_hop() {
        let refusal = KeyRefusal::read(
            UNKNOWN,
            Some("No ED25519 host key is known for [hpc.example.edu]:2222 and you have requested strict checking."),
            None,
            "you@hpc.example.edu",
        )
        .unwrap();
        let direct = "user you\nhostname hpc.example.edu\nport 2222\nproxycommand none\n";
        assert!(resolved_directly(direct, &refusal));
        assert!(resolved_directly(
            "hostname HPC.example.edu\nport 2222\nproxyjump none\n",
            &refusal
        ));
        for indirect in [
            format!("{direct}proxyjump gate.example.edu\n"),
            "hostname hpc.example.edu\nport 2222\nproxycommand nc %h %p\n".to_owned(),
            format!("{direct}hostkeyalias lab\n"),
            "hostname 10.0.0.7\nport 2222\n".to_owned(),
            "hostname hpc.example.edu\nport 22\n".to_owned(),
            "port 2222\n".to_owned(),
            String::new(),
        ] {
            assert!(!resolved_directly(&indirect, &refusal), "{indirect}");
        }
        // A route with a jump host, or with no login to resolve, is not direct before `ssh -G`
        // is even asked.
        let jumped = Route {
            login: "you@hpc.example.edu".into(),
            port: Some(2222),
            proxy_jump: Some("gate.example.edu".into()),
        };
        assert!(!reached_directly(&jumped, &refusal).await);
        let unnamed = Route {
            login: String::new(),
            port: None,
            proxy_jump: None,
        };
        assert!(!reached_directly(&unnamed, &refusal).await);
        let option = Route {
            login: "-oProxyCommand=x".into(),
            port: None,
            proxy_jump: None,
        };
        assert!(!reached_directly(&option, &refusal).await);
        // Without a known route, nothing is read and the card shows no fingerprint.
        let mut unread = refusal.clone();
        unread.read_offered(None).await;
        assert_eq!(unread.offered, None);
        assert!(!unread.scanned);
    }

    #[test]
    fn a_key_blob_s_fingerprint_is_the_one_ssh_keygen_prints() {
        assert_eq!(fingerprint_of(BLOB).as_deref(), Some(BLOB_FINGERPRINT));
        assert_eq!(fingerprint_of("not base64!"), None);
    }

    #[test]
    fn only_plain_known_hosts_entries_count() {
        let output = format!(
            "# Host hpc.example.edu found: line 1 REVOKED\nhpc.example.edu ED25519 {KNOWN}\n# Host hpc.example.edu found: line 2 CA\nhpc.example.edu ED25519 {OFFERED}\n# Host hpc.example.edu found: line 3 \nhpc.example.edu ED25519 {BLOB_FINGERPRINT}\n"
        );
        assert_eq!(
            listed_fingerprints(&output),
            vec![BLOB_FINGERPRINT.to_owned()]
        );
    }

    /// The daemon's own reading of a known-hosts file: the key the card shows counts once it is
    /// there, a revoked one never does, and a card that shows none counts any plain entry.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_key_counts_as_added_only_when_the_file_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let mut refusal = KeyRefusal::read(
            UNKNOWN,
            Some("No ED25519 host key is known for [hpc.example.edu]:2222 and you have requested strict checking."),
            None,
            "",
        )
        .unwrap();
        refusal.offered = Some(BLOB_FINGERPRINT.into());
        let files = vec![file.clone()];
        assert!(!key_added(&refusal, &files).await, "no file yet");
        std::fs::write(
            &file,
            format!("@revoked [hpc.example.edu]:2222 ssh-ed25519 {BLOB}\n"),
        )
        .unwrap();
        assert!(
            !key_added(&refusal, &files).await,
            "a revoked key is not trusted"
        );
        std::fs::write(&file, format!("hpc.example.edu ssh-ed25519 {BLOB}\n")).unwrap();
        assert!(
            !key_added(&refusal, &files).await,
            "port 22's entry is not port 2222's"
        );
        std::fs::write(
            &file,
            format!("[hpc.example.edu]:2222 ssh-ed25519 {BLOB}\n"),
        )
        .unwrap();
        assert!(key_added(&refusal, &files).await);
        refusal.offered = Some(OFFERED.into());
        assert!(
            !key_added(&refusal, &files).await,
            "another key than the card's"
        );
        refusal.offered = None;
        assert!(
            key_added(&refusal, &files).await,
            "a card with no fingerprint takes any key"
        );
    }

    /// A changed key never waits: its card is published for the person and released at once,
    /// and the call's own refusal goes on to be worded.
    #[tokio::test]
    async fn a_changed_key_shows_its_card_and_waits_for_nothing() {
        let ctx = test_ctx(
            PersonTool::Change,
            "connect",
            json!({"connection": "SHA256:Zm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm9vZm8"}),
        )
        .await;
        let error = anyhow::Error::new(failure(
            SshFailureKind::HostKeyChanged,
            &changed_banner(),
            Some("hpc.example.edu"),
        ));
        let passed = tokio::time::timeout(Duration::from_secs(5), on_refusal(&ctx, error))
            .await
            .expect("a changed key waits for nothing")
            .unwrap_err();
        assert!(passed.chain().any(|cause| cause.is::<SshFailure>()));
        let cards: Vec<_> = ActionRequiredManager::global()
            .drain_requests(&ctx.call.session_id)
            .into_iter()
            .flat_map(|message| message.content)
            .filter_map(|content| match content {
                MessageContent::ActionRequired(action) => match action.data {
                    ActionRequiredData::CrewStep { id, step } => Some((id, step)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 1, "{cards:?}");
        let (id, step) = &cards[0];
        assert_eq!(
            step,
            &CrewStepRequest::HostKey {
                host: "hpc.example.edu".into(),
                port: 22,
                login: "hpc.example.edu".into(),
                offered: Some(OFFERED.into()),
                known: None,
                changed: true,
            },
            "the card is the daemon's words, never the call's parameters"
        );
        assert!(!ctx.call.approvals.is_pending(id), "nothing waits on it");
    }

    /// Where no person can answer a card, nothing is published and the refusal is said as
    /// before.
    #[tokio::test]
    async fn no_card_where_nobody_can_answer_it() {
        let ctx = test_ctx(PersonTool::Change, "connect", json!({})).await;
        let error = anyhow::Error::new(failure(
            SshFailureKind::HostKeyChanged,
            &changed_banner(),
            None,
        ));
        let passed = biorouter::user_surface::without_human_surface(on_refusal(&ctx, error)).await;
        assert!(passed.is_err());
        assert!(ActionRequiredManager::global()
            .drain_requests(&ctx.call.session_id)
            .is_empty());
    }

    /// Anything that is not a host-key refusal passes through untouched, and so does a host-key
    /// failure another refusal wraps, which is worded as that refusal.
    #[tokio::test]
    async fn other_errors_pass_through() {
        let ctx = test_ctx(PersonTool::Change, "connect", json!({})).await;
        let passed = on_refusal(&ctx, anyhow::anyhow!("refused"))
            .await
            .unwrap_err();
        assert_eq!(passed.to_string(), "refused");
        let wrapped = HandoffFailed::wrap(anyhow::Error::new(failure(
            SshFailureKind::HostKeyUnknown,
            "No ED25519 host key is known for h and you have requested strict checking.",
            None,
        )));
        assert!(host_key_failure(&wrapped).is_none());
        let worded = said(Some(UNKNOWN), "Said.");
        assert!(host_key_failure(&worded).is_none());
        assert!(ActionRequiredManager::global()
            .drain_requests(&ctx.call.session_id)
            .is_empty());
    }
}
