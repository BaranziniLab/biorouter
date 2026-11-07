//! Signing in from a chat (the owner's D1 ruling on AO-12): the `crew__change auth` action, the
//! sign-in card a waited-on setup job or a connect raises, and the step a sign-in refusal names.
//!
//! What a person types to sign in goes from the Sign in window (the desktop's ssh pty, or the
//! raw-mode terminal of a shared-daemon CLI chat) to ssh on the server, never through this
//! module: a card here carries the daemon's own `server` and `login` and nothing a value could
//! sit in ([`CrewStepRequest::SignIn`]), and the tool answers with the job's or the
//! connection's state only.
//!
//! Nothing here can mark a sign-in done. A card is taken down when the daemon itself says so:
//! the setup job no longer waits for a sign-in, or a sign-in to the saved connection was adopted
//! ([`biorouter::crew::authentication::wait_adopted`]). The person can only dismiss it, and a
//! forged answer is refused by the registry (`UserActionRequest::CrewStep` accepts no success).
//!
//! Where no card can be answered (`biorouter serve`, `POST /agent/call_tool`, a stopped call),
//! nothing is parked and the step is said in words, as before.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::Result;
use biorouter::conversation::message::{CrewSignInTarget, CrewStepRequest};
use biorouter::crew::authentication::{wait_adopted, AdoptionWait};
use biorouter::crew::{Connection, CrewManager, SetupState, SetupStatus, SshFailure};
use biorouter::pending_user_action::{user_proof_available, PendingUserActions};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{
    job_answer, job_target, park_step_in, said, saved_connection, Ctx, PendingStep, Progress,
    StepEnd, APPROVAL_TTL,
};

/// The code of a server that asks for a password or a verification code.
pub(super) const AUTH_REQUIRED: &str = "crew_ssh_auth_required";
/// The code of a server that refused this computer's SSH key. A key with a passphrase and no
/// agent holding it is refused this way, and the Sign in window asks for its passphrase.
pub(super) const KEY_REFUSED: &str = "crew_ssh_key_refused";
/// How often a parked sign-in card reads the setup job it waits for.
const SIGN_IN_POLL: Duration = Duration::from_millis(250);
/// Said when no Sign in window can be opened for this call.
const NO_WINDOW: &str = "No Sign in window can open for this chat.";
/// Said by `auth` for a setup job that does not wait for a sign-in.
const JOB_NOTHING_WAITS: &str = "Nothing waits for a sign-in: this setup isn't waiting for one.";
/// Said by `auth` for a connected saved connection.
const CONNECTED_NOTHING_WAITS: &str = "Nothing waits for a sign-in: it is connected.";
/// Said by `auth` for a saved connection whose server has not asked for a sign-in.
const CONNECTION_NOTHING_WAITS: &str = "Nothing waits for a sign-in. Connect it with crew__change action connect, which shows a Sign in card if its server asks for one.";

/// `crew__change auth`: sign the person in to a setup job's server (`job_id`) or a saved
/// connection (`connection`), in the card the chat shows. It asks for no approval of its own:
/// the sign-in is the person's own act, in their own window. Answers the job's state (waited on
/// as the action that started it waits) or the connection's, and that nothing waits when the
/// target is not waiting for a sign-in.
pub(super) fn auth(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        match (ctx.text("job_id"), ctx.text("connection")) {
            (Some(_), Some(_)) => Err(said(
                None,
                "Name a setup job or a saved connection to sign in to, not both.",
            )),
            (Some(job), None) => auth_job(ctx, &job).await,
            (None, _) => auth_connection(ctx).await,
        }
    })
}

async fn auth_job(ctx: &Ctx, job: &str) -> Result<Value> {
    let status =
        biorouter::crew::setup_status(job).ok_or_else(|| said(None, super::setup::NO_JOB))?;
    if status.state != SetupState::NeedsSignIn {
        let mut answer = job_answer(&status, false);
        answer["note"] = json!(JOB_NOTHING_WAITS);
        return Ok(answer);
    }
    match SignInGate::of(ctx, &Progress::default())
        .setup_job(&status)
        .await
    {
        SignIn::Done => Ok(ctx.wait_for_job(job).await),
        SignIn::Ended(how) => Ok(job_left(job, how)),
        SignIn::NoCard => Err(said(
            None,
            format!(
                "{NO_WINDOW} The person signs in from the Crew view of the desktop app, or runs biorouter crew auth --setup-job {job} in a terminal."
            ),
        )),
    }
}

async fn auth_connection(ctx: &Ctx) -> Result<Value> {
    let crew = ctx.crew()?;
    let connection = ctx.connection().await?;
    if connection.status == "connected" {
        return Ok(json!({"connection_id": connection.id, "state": "connected",
            "note": CONNECTED_NOTHING_WAITS}));
    }
    if !waits_for_connection_sign_in(&crew, &connection) {
        return Ok(
            json!({"connection_id": connection.id, "state": "disconnected",
            "note": CONNECTION_NOTHING_WAITS}),
        );
    }
    match SignInGate::of(ctx, &Progress::default())
        .connection(&connection)
        .await
    {
        // The adopted sign-in connected it, or kept it for setting Crew up there.
        SignIn::Done => {
            let now = saved_connection(&crew, &connection.id).await?;
            Ok(serde_json::to_value(
                crate::routes::crew::connection_view(&now).await?,
            )?)
        }
        SignIn::Ended(how) => Err(said(None, ended(how))),
        SignIn::NoCard => {
            let name = &connection.name;
            Err(said(
                None,
                format!(
                    "{NO_WINDOW} The person chooses Sign in on {name} in the Crew view of the desktop app, or runs biorouter crew auth --connection {name} in a terminal."
                ),
            ))
        }
    }
}

/// Called by [`Ctx::wait_for_job`] with every status it reads of the job it waits for, before
/// it judges the status. A job that waits for a sign-in gets the sign-in card, and the wait
/// stays here until the job moves on (`None`: the wait reads the job again and carries on,
/// not counting the time spent here), or until the card ends with nothing signed in (the job
/// as it stands, with a note and its next step). Where no card can be shown, `None`, and the
/// job's answer names the step in words.
pub(super) async fn on_status(ctx: &Ctx, status: &SetupStatus) -> Option<Value> {
    if status.state != SetupState::NeedsSignIn {
        return None;
    }
    match SignInGate::of(ctx, &Progress::default())
        .setup_job(status)
        .await
    {
        SignIn::Done | SignIn::NoCard => None,
        SignIn::Ended(how) => Some(job_left(&status.job_id, how)),
    }
}

/// Connect the saved connection `id` as the person, as [`CrewManager::connect`] does, for an
/// action whose approval the person already gave. When its server asks for a password or a
/// code (`crew_ssh_auth_required`), the chat shows them a Sign in card for it. Once a sign-in to
/// it is adopted it connects over that sign-in, at most once more, and asks for no second
/// approval. Any other refusal, and a card that ends with nothing signed in, is the connect's
/// refusal.
pub(super) async fn connect_or_sign_in(
    gate: &SignInGate,
    crew: &CrewManager,
    id: &str,
) -> Result<Connection> {
    let refused = match crew.connect(id).await {
        Ok(connected) => {
            sign_in_refusals().remove(id);
            return Ok(connected);
        }
        Err(error) => error,
    };
    let code = failure_code(&refused);
    if let Some(code @ (AUTH_REQUIRED | KEY_REFUSED)) = code {
        remember_sign_in_refusal(id, code);
    }
    if code != Some(AUTH_REQUIRED) {
        return Err(refused);
    }
    let Ok(connection) = crew.connection(id).await else {
        return Err(refused);
    };
    gate.progress.set("stage", "sign_in");
    match gate.connection(&connection).await {
        SignIn::Done => {}
        SignIn::NoCard => return Err(refused),
        SignIn::Ended(how) => {
            return Err(said(
                Some(AUTH_REQUIRED),
                format!("{} {}", refusal_sentence(&refused), ended(how)),
            ))
        }
    }
    gate.ensure_still_ungranted().await?;
    gate.progress.set("stage", "connect");
    // An adopted sign-in connects as it is adopted, unless the server had no Crew for the
    // account then: only that case connects once more, over the kept sign-in.
    let now = saved_connection(crew, id).await?;
    if now.status == "connected" {
        return Ok(now);
    }
    crew.connect(id).await
}

/// The step a sign-in refusal names for the model ([`super::next_step`]), or `None` when
/// `code` is not a sign-in's. `connection` is the saved connection's name, when the call named
/// one. Where a card can be shown, it offers `crew__change auth`; elsewhere it names Crew and
/// `biorouter crew auth`, as before.
pub(super) fn next_step(code: &str, connection: Option<&str>) -> Option<String> {
    step_for(code, connection, card_can_open())
}

/// Whether a Sign in card could open for the call being answered: a person's proof can be had
/// in this process (not `biorouter serve`), and the call has a surface a person watches (not
/// `POST /agent/call_tool` or a scheduled run, under `without_human_surface`), as
/// [`SignInGate::can_park`] and [`Ctx::park_step`] ask before they park one. Otherwise a step
/// that offered `auth` would promise a card that `auth` then refuses with [`NO_WINDOW`].
fn card_can_open() -> bool {
    user_proof_available() && !biorouter::user_surface::no_human_surface()
}

/// [`next_step`], where `card` says whether a Sign in card can open in this process.
fn step_for(code: &str, connection: Option<&str>, card: bool) -> Option<String> {
    let named = connection.map_or_else(|| "the connection".to_owned(), str::to_owned);
    let auth_call = match connection {
        Some(name) => format!("call crew__change action auth with connection {name}"),
        None => "call crew__change action auth with the connection".to_owned(),
    };
    let command = match connection {
        Some(name) => format!("biorouter crew auth --connection {name}"),
        None => "biorouter crew auth".to_owned(),
    };
    match code {
        AUTH_REQUIRED if card => Some(format!(
            "Ask the person to sign in: {auth_call}, and they sign in in the card that opens. Or they choose Sign in on {named} in Crew, or run {command}. Then try again."
        )),
        AUTH_REQUIRED => Some(format!(
            "The person signs in: in Crew, choose Sign in on {named}, or run {command}. Then try again."
        )),
        KEY_REFUSED if card => Some(format!(
            "If the key has a passphrase, the person can type it in a Sign in window: {auth_call}, and they sign in in the card that opens. Otherwise they check the login and the key in Crew."
        )),
        _ => None,
    }
}

/// The step a setup job that waits for a sign-in names ([`super::job_answer`]).
pub(super) fn job_step(job: &str) -> String {
    job_step_for(job, card_can_open())
}

/// [`job_step`], where `card` says whether a Sign in card can open in this process.
fn job_step_for(job: &str, card: bool) -> String {
    if card {
        format!(
            "Ask the person to sign in: call crew__change action auth with job_id {job}, and they sign in in the card that opens. Or they open Crew, where this job waits with a Sign in button, or run biorouter crew auth --setup-job {job}. Then follow it with crew__read action setup_status."
        )
    } else {
        format!(
            "The person signs in: open Crew, where this job waits with a Sign in button, or run biorouter crew auth --setup-job {job}. Then follow it with crew__read action setup_status."
        )
    }
}

/// Why a sign-in card ended with nothing signed in, for the model.
pub(super) fn ended(how: StepEnd) -> &'static str {
    match how {
        StepEnd::Dismissed => "The person closed the Sign in card, so nothing was signed in.",
        StepEnd::Stopped => "The chat was stopped before anyone signed in.",
        StepEnd::TimedOut => "Nobody signed in before the Sign in card's time ran out.",
        StepEnd::NotShown | StepEnd::Withdrawn => {
            "The Sign in card couldn't be shown, so nothing was signed in."
        }
    }
}

/// How a sign-in the chat asked for ended.
pub(super) enum SignIn {
    /// The daemon says it is done: the job moved on, or a sign-in to the connection was adopted.
    Done,
    /// The card ended with nothing signed in.
    Ended(StepEnd),
    /// No card can be shown for this call.
    NoCard,
}

/// Where a sign-in card goes for one person-tool call: the chat, its approvals and its Stop,
/// with the work's [`Progress`], so the time the person spends signing in is not counted against
/// the call's wait. Cloned into the work [`Ctx::follow`] runs apart from the call, which does not
/// inherit the call's task-local scope; so whether the call has a person's surface at all is
/// asked here, where the call runs.
#[derive(Clone)]
pub(super) struct SignInGate {
    approvals: Arc<PendingUserActions>,
    session_id: String,
    stop: CancellationToken,
    no_human_surface: bool,
    progress: Progress,
}

impl SignInGate {
    /// The gate of `ctx`'s call, for work that reports through `progress`.
    pub(super) fn of(ctx: &Ctx, progress: &Progress) -> Self {
        Self {
            approvals: Arc::clone(&ctx.call.approvals),
            session_id: ctx.call.session_id.clone(),
            stop: ctx.call.cancel.clone(),
            no_human_surface: biorouter::user_surface::no_human_surface(),
            progress: progress.clone(),
        }
    }

    /// The work's progress, for its stages.
    pub(super) fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Whether a card can be shown for this call at all.
    fn can_park(&self) -> bool {
        !self.no_human_surface && user_proof_available() && !self.stop.is_cancelled()
    }

    fn park(&self, step: CrewStepRequest) -> Option<PendingStep> {
        if !self.can_park() {
            return None;
        }
        park_step_in(
            &self.approvals,
            &self.session_id,
            &self.stop,
            step,
            APPROVAL_TTL,
        )
    }

    /// The sign-in card for the setup job `status` describes, while it waits for one. Its time
    /// is the job's own: a job whose wait for a sign-in runs out fails, and the card goes with it.
    pub(super) async fn setup_job(&self, status: &SetupStatus) -> SignIn {
        if status.state != SetupState::NeedsSignIn {
            return SignIn::Done;
        }
        if !self.can_park() {
            return SignIn::NoCard;
        }
        let job = status.job_id.clone();
        let step = CrewStepRequest::SignIn {
            target: CrewSignInTarget::Setup {
                job_id: job.clone(),
            },
            server: job_server(status).await,
            login: status.server.login.clone(),
        };
        let Some(card) = self.park(step) else {
            return SignIn::NoCard;
        };
        let _awaiting = self.progress.awaiting_person();
        let moved_on = async {
            loop {
                tokio::time::sleep(SIGN_IN_POLL).await;
                if !waits_for_sign_in(&job) {
                    return true;
                }
            }
        };
        match until(card, moved_on).await {
            StepEnd::Withdrawn => SignIn::Done,
            how => SignIn::Ended(how),
        }
    }

    /// The sign-in card for the saved `connection`, until a sign-in to it is adopted.
    pub(super) async fn connection(&self, connection: &Connection) -> SignIn {
        if !self.can_park() {
            return SignIn::NoCard;
        }
        // Counted from before the card is shown, so an adoption that beats the first look at
        // it still counts.
        let adopted = wait_adopted(
            &connection.id,
            tokio::time::Instant::now() + APPROVAL_TTL,
            self.stop.clone(),
        );
        let step = CrewStepRequest::SignIn {
            target: CrewSignInTarget::Connection {
                connection_id: connection.id.clone(),
            },
            server: biorouter::crew::server_label(&connection.ssh_target, connection.port).await,
            login: connection.ssh_target.clone(),
        };
        let Some(card) = self.park(step) else {
            return SignIn::NoCard;
        };
        let _awaiting = self.progress.awaiting_person();
        match until(card, async move { adopted.await == AdoptionWait::Adopted }).await {
            StepEnd::Withdrawn => {
                sign_in_refusals().remove(&connection.id);
                SignIn::Done
            }
            how => SignIn::Ended(how),
        }
    }

    /// AO-12: a chat given a Crew grant while the person signed in no longer acts as them.
    async fn ensure_still_ungranted(&self) -> Result<()> {
        if biorouter::crew::manager()?
            .is_scoped_session(&self.session_id)
            .await
        {
            return Err(said(None, biorouter::crew::tool_host::GRANTED_MEANWHILE));
        }
        Ok(())
    }
}

/// Wait for `card` to end, taking it down as soon as `done` answers `true`.
async fn until(card: PendingStep, done: impl Future<Output = bool>) -> StepEnd {
    let withdraw = card.withdrawer();
    let end = card.end();
    tokio::pin!(end);
    tokio::pin!(done);
    let mut watching = true;
    loop {
        tokio::select! {
            how = &mut end => return how,
            finished = &mut done, if watching => {
                watching = false;
                if finished {
                    withdraw.cancel();
                }
            }
        }
    }
}

/// Saved connections whose connect from a chat was refused for a sign-in
/// (`crew_ssh_auth_required`, or `crew_ssh_key_refused`, which a key with a passphrase gives),
/// with the code. A person's Connect leaves no code on the connection itself, so this is what
/// tells `auth` that a connection's server waits for one. In memory only, and bounded.
static SIGN_IN_REFUSALS: LazyLock<Mutex<HashMap<String, &'static str>>> =
    LazyLock::new(Mutex::default);
/// How many connections [`SIGN_IN_REFUSALS`] remembers at once.
const SIGN_IN_REFUSALS_KEPT: usize = 64;

fn sign_in_refusals() -> std::sync::MutexGuard<'static, HashMap<String, &'static str>> {
    SIGN_IN_REFUSALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn remember_sign_in_refusal(id: &str, code: &'static str) {
    let mut refusals = sign_in_refusals();
    if refusals.len() >= SIGN_IN_REFUSALS_KEPT && !refusals.contains_key(id) {
        refusals.clear();
    }
    refusals.insert(id.to_owned(), code);
}

/// Whether the saved, disconnected `connection`'s server waits for the person to sign in: its
/// last connect was refused for one, from a chat or as the daemon recorded it.
fn waits_for_connection_sign_in(crew: &CrewManager, connection: &Connection) -> bool {
    connection.status != "connected"
        && (sign_in_refusals().contains_key(&connection.id)
            || matches!(
                crew.last_error_code(connection),
                Some(AUTH_REQUIRED | KEY_REFUSED)
            ))
}

/// Whether the setup job `job` is known and waits for its person to sign in.
fn waits_for_sign_in(job: &str) -> bool {
    biorouter::crew::setup_status(job).is_some_and(|status| status.state == SetupState::NeedsSignIn)
}

/// The server a setup job signs in to, as a person names it: from the job's login, and the
/// port the job was started with (its saved connection's, for a connection's job).
async fn job_server(status: &SetupStatus) -> String {
    let target = job_target(&status.job_id).unwrap_or_default();
    let port = if target["kind"] == "connection" {
        match (biorouter::crew::manager(), target["connection_id"].as_str()) {
            (Ok(crew), Some(id)) => crew.connection(id).await.ok().and_then(|saved| saved.port),
            _ => None,
        }
    } else {
        target["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
    };
    biorouter::crew::server_label(&status.server.login, port).await
}

/// The job `job` as it stands after its sign-in card ended with nothing signed in.
fn job_left(job: &str, how: StepEnd) -> Value {
    let Some(status) = biorouter::crew::setup_status(job) else {
        return json!({"job_id": job, "state": "unknown",
            "note": "That job is not on this computer any more."});
    };
    let mut answer = job_answer(&status, false);
    let mut note = ended(how).to_owned();
    if status.state == SetupState::NeedsSignIn {
        note.push_str(" The job still waits for a sign-in.");
    }
    answer["note"] = json!(note);
    answer
}

/// The code of the SSH failure behind `error`, if one is.
fn failure_code(error: &anyhow::Error) -> Option<&'static str> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SshFailure>())
        .map(SshFailure::api_code)
}

/// The sentence a person reads for the SSH failure behind `error`, as the admission routes say
/// it (`core_refusal`).
fn refusal_sentence(error: &anyhow::Error) -> String {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SshFailure>())
        .map_or_else(|| super::FAILED.to_owned(), ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_ctx;
    use super::*;
    use biorouter::crew::tool_host::PersonTool;

    /// `auth` for a job this computer does not know refuses, and names both or neither as
    /// the action's table says.
    #[tokio::test]
    async fn auth_names_a_job_or_a_connection() {
        let ctx = test_ctx(
            PersonTool::Change,
            "auth",
            json!({"job_id": "no-such-job", "connection": "lab"}),
        )
        .await;
        let refused = auth(&ctx).await.unwrap_err().to_string();
        assert!(refused.contains("not both"), "{refused}");
        let ctx = test_ctx(PersonTool::Change, "auth", json!({"job_id": "no-such-job"})).await;
        let refused = auth(&ctx).await.unwrap_err().to_string();
        assert_eq!(refused, super::super::setup::NO_JOB);
    }

    /// With a card possible, the step offers `auth` and keeps the command line's; without, it
    /// is said as before. A refused key offers the Sign in window only where one can open.
    #[test]
    fn the_sign_in_steps_offer_auth_where_a_card_can_open() {
        let step = step_for(AUTH_REQUIRED, Some("lab"), true).unwrap();
        assert!(
            step.contains("crew__change action auth with connection lab"),
            "{step}"
        );
        assert!(step.contains("the card that opens"), "{step}");
        assert!(
            step.contains("biorouter crew auth --connection lab"),
            "{step}"
        );
        let key = step_for(KEY_REFUSED, Some("lab"), true).unwrap();
        assert!(key.contains("crew__change action auth"), "{key}");
        assert!(key.contains("the card that opens"), "{key}");
        let job = job_step_for("job-1", true);
        assert!(job.contains("auth with job_id job-1"), "{job}");
        assert!(job.contains("--setup-job job-1"), "{job}");
        assert!(step_for("crew_ssh_host_key_unknown", None, true).is_none());
        for text in [step, key, job] {
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
            assert!(!text.to_lowercase().contains("password"), "{text}");
        }
    }

    /// `biorouter serve`: no card can open, so no step offers one, and each is said as before.
    #[test]
    fn without_a_person_s_proof_the_steps_are_said_as_before() {
        let step = step_for(AUTH_REQUIRED, Some("lab"), false).unwrap();
        assert_eq!(
            step,
            "The person signs in: in Crew, choose Sign in on lab, or run biorouter crew auth --connection lab. Then try again."
        );
        assert_eq!(
            step_for(AUTH_REQUIRED, None, false).unwrap(),
            "The person signs in: in Crew, choose Sign in on the connection, or run biorouter crew auth. Then try again."
        );
        assert!(step_for(KEY_REFUSED, Some("lab"), false).is_none());
        let job = job_step_for("job-1", false);
        assert!(!job.contains("crew__change"), "{job}");
        assert_eq!(
            job,
            "The person signs in: open Crew, where this job waits with a Sign in button, or run biorouter crew auth --setup-job job-1. Then follow it with crew__read action setup_status."
        );
    }

    /// A call no person watches (`POST /agent/call_tool`, a scheduled run) is never told a card
    /// will open: `auth` would answer it with "No Sign in window can open".
    #[tokio::test]
    async fn no_step_promises_a_card_to_a_call_nobody_watches() {
        assert!(
            user_proof_available(),
            "this test needs a daemon that can obtain a person's proof"
        );
        let watched = next_step(AUTH_REQUIRED, Some("lab")).unwrap();
        assert!(watched.contains("the card that opens"), "{watched}");
        assert!(job_step("job-1").contains("the card that opens"));
        let (step, key, job) = biorouter::user_surface::without_human_surface(async {
            (
                next_step(AUTH_REQUIRED, Some("lab")),
                next_step(KEY_REFUSED, Some("lab")),
                job_step("job-1"),
            )
        })
        .await;
        assert_eq!(step, step_for(AUTH_REQUIRED, Some("lab"), false));
        assert!(!step.unwrap().contains("crew__change"));
        assert!(key.is_none());
        assert_eq!(job, job_step_for("job-1", false));
    }

    /// No card where no person can answer it, and none for a stopped call.
    #[tokio::test]
    async fn no_card_is_parked_where_nobody_can_answer() {
        let ctx = test_ctx(PersonTool::Change, "auth", json!({})).await;
        let hidden = biorouter::user_surface::without_human_surface(async {
            SignInGate::of(&ctx, &Progress::default()).can_park()
        })
        .await;
        assert!(!hidden);
        let gate = SignInGate::of(&ctx, &Progress::default());
        assert!(gate.can_park());
        ctx.call.cancel.cancel();
        assert!(!gate.can_park());
    }

    /// A card is taken down once the daemon says the step is done, and only then; the time it
    /// was up is counted as the person's.
    #[tokio::test]
    async fn a_card_comes_down_when_the_daemon_says_it_is_done() {
        let ctx = test_ctx(PersonTool::Change, "auth", json!({})).await;
        let progress = Progress::default();
        let gate = SignInGate::of(&ctx, &progress);
        let card = gate
            .park(CrewStepRequest::SignIn {
                target: CrewSignInTarget::Connection {
                    connection_id: "conn-1".into(),
                },
                server: "hpc.example.edu".into(),
                login: "you@hpc.example.edu".into(),
            })
            .unwrap();
        let id = card.id().to_owned();
        let awaiting = progress.awaiting_person();
        let (tell, told) = tokio::sync::oneshot::channel::<bool>();
        let waited = tokio::spawn(until(card, async move { told.await.unwrap_or(false) }));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(ctx.call.approvals.is_pending(&id));
        tell.send(true).unwrap();
        assert_eq!(waited.await.unwrap(), StepEnd::Withdrawn);
        assert!(!ctx.call.approvals.is_pending(&id));
        drop(awaiting);
        assert!(progress.person_time() >= Duration::from_millis(40));
        assert!(!ctx.call.cancel.is_cancelled());
    }

    #[test]
    fn every_ending_is_said_plainly() {
        for how in [
            StepEnd::Dismissed,
            StepEnd::Stopped,
            StepEnd::TimedOut,
            StepEnd::NotShown,
            StepEnd::Withdrawn,
        ] {
            let text = ended(how);
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
            assert!(text.ends_with('.'), "{text}");
        }
    }
}
