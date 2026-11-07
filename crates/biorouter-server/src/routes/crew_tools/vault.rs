//! Crew's vault from a chat (the owner's D1 ruling on AO-12): `crew__read credentials_status`,
//! `crew__change credentials_unlock`, `credentials_init` and `credentials_lock`, the vault card
//! a change that needs the vault raises before its own approval, and the step a locked vault's
//! refusal names.
//!
//! A passphrase never passes through this module. Unlock and init park a vault card
//! ([`biorouter::conversation::message::CrewStepRequest::Vault`], which carries the action and
//! nothing else). Its button opens the person's own secure prompt: the desktop's OS-native
//! prompt, or a shared-daemon CLI chat's hidden prompt, which posts the passphrase to the
//! daemon's vault route (`POST /crew/credentials/{init,unlock}`) with the person's proof. The
//! card has no Allow, because the person's typing is the act, and nothing can answer it as
//! done: the call reads the vault every [`POLL`] and takes the card down itself once the vault
//! is open. The tool answers with the vault's state only.
//!
//! Where no person can answer a card (`biorouter serve`, ACP, `POST /agent/call_tool`), nothing
//! is parked and the step is said in words, as before.
//!
//! ⚠ **Person authority; needs human review.** Locking the vault cuts off every chat and Crew
//! task using it, so it takes an ordinary proof-backed approval card.

use std::cell::Cell;
use std::time::Duration;

use anyhow::Result;
use biorouter::conversation::message::{CrewStepRequest, CrewVaultAction};
use biorouter::crew::CredentialStatus;
use biorouter::pending_user_action::user_proof_available;
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::{said, Card, Ctx, Run, StepEnd, APPROVAL_TTL};

/// How often a parked vault card reads the vault (the brief's 500 ms).
const POLL: Duration = Duration::from_millis(500);

/// What the vault answers for a key while it is locked (`credentials::require_unlocked` in the
/// core crate). The transfer service matches the same words (this crate's `crew/transfers.rs`).
/// `the_locked_vault_words_are_the_vaults_own` holds the three together, because a reword there
/// only stops the card from being offered, with no other test failing.
const LOCKED_VAULT_TEXT: &str = "Crew credential vault is locked";

/// The vault's own changes: a locked vault is never asked for around them.
const VAULT_ACTIONS: [&str; 3] = ["credentials_unlock", "credentials_init", "credentials_lock"];

/// Changes a locked vault is not asked for before, because they work as well without it: the
/// vault's own actions; what stops or takes back something (a setup job, a connection's bridge,
/// a task, a grant, a transfer), which a card in front of it must never hold up; and what only
/// changes this computer's own records (saved connection settings, whose reconnect after the
/// save is best effort, acknowledged computers, Crew task defaults). A change not listed here
/// asks for a locked vault first; one that never needed it costs the person an unlock, never a
/// second approval.
const NEEDS_NO_VAULT: &[&str] = &[
    VAULT_ACTIONS[0],
    VAULT_ACTIONS[1],
    VAULT_ACTIONS[2],
    "setup_cancel",
    "disconnect",
    "tasks_cancel",
    "grants_revoke",
    "files_pause",
    "files_forget",
    "connections_update",
    "connections_set_access",
    "privacy_set_personal",
    "profile_devices_acknowledge",
    "defaults_set",
    "defaults_clear",
];

/// The lock's approval card (the brief's wording, as the summary and its one line).
const LOCK_CARD: &str = "Lock Crew's vault on this computer";
const LOCK_CARD_LINE: &str = "Every chat and Crew task using it stops until you unlock it.";

tokio::task_local! {
    /// Set inside [`run_unlocked`] once the action reached [`before_approval`]: from then on an
    /// approval card may have been asked, so a refusal is never answered by running the action
    /// again, and a change is never approved twice.
    static APPROACHED_APPROVAL: Cell<bool>;
}

// ---------------------------------------------------------------------------------------------
// The vault, as these actions see it
// ---------------------------------------------------------------------------------------------

/// Where Crew keeps its keys, and whether that store is set up and locked: the daemon's own
/// [`CredentialStatus`], the four fields `GET /crew/credentials` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Status {
    backend: &'static str,
    initialized: bool,
    locked: bool,
    available: bool,
}

impl From<CredentialStatus> for Status {
    fn from(status: CredentialStatus) -> Self {
        Self {
            backend: status.backend,
            initialized: status.initialized,
            locked: status.locked,
            available: status.available,
        }
    }
}

impl Status {
    /// A vault that is set up and locked: every key in it is out of reach.
    fn locked_vault(self) -> bool {
        self.initialized && self.locked
    }

    /// A vault that is set up and unlocked: what both vault cards wait for.
    fn open(self) -> bool {
        self.initialized && !self.locked
    }

    /// The answer the model reads: the state, never a key or a passphrase.
    fn answer(self) -> Value {
        json!({
            "backend": self.backend,
            "initialized": self.initialized,
            "locked": self.locked,
            "available": self.available,
        })
    }

    fn noted(self, note: &str) -> Value {
        let mut answer = self.answer();
        answer["note"] = json!(note);
        answer
    }

    /// Where Crew keeps its keys when there is no vault, as a person reads it.
    fn keys_live(self) -> &'static str {
        if self.backend == "file" {
            "development files"
        } else {
            "this computer's keyring"
        }
    }
}

/// Crew's vault as the actions here read and lock it: the daemon's own in a chat
/// ([`Manager`]), a stand-in in this file's tests.
trait Store: Sync {
    fn status(&self) -> BoxFuture<'_, Result<Status>>;
    fn lock(&self) -> BoxFuture<'_, Result<()>>;
    /// Whether this computer keeps a saved Crew connection, whose key a new vault would shadow.
    fn holds_connections(&self) -> BoxFuture<'_, bool>;
}

/// The daemon's Crew manager.
struct Manager;

impl Store for Manager {
    fn status(&self) -> BoxFuture<'_, Result<Status>> {
        Box::pin(async {
            Ok(biorouter::crew::manager()?
                .credential_status()
                .await?
                .into())
        })
    }

    fn lock(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { biorouter::crew::manager()?.lock_vault().await })
    }

    fn holds_connections(&self) -> BoxFuture<'_, bool> {
        Box::pin(async {
            match biorouter::crew::manager() {
                Ok(crew) => !crew.list().await.is_empty(),
                // Unknown counts as holding one: a refusal here costs nothing, a passphrase
                // typed for an init the daemon then refuses costs the person.
                Err(_) => true,
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------------------------

/// How a vault card ended.
#[derive(Debug)]
enum Asked {
    /// The vault is open now, whatever took the card down.
    Open(Status),
    /// The vault is still closed: the card was dismissed, the chat stopped, or time ran out.
    Closed(StepEnd),
    /// No card can be put to a person here (no proof, or no surface a person watches).
    NoCard,
}

/// Park the vault card for `action` in this chat and wait: until the vault is open, read every
/// [`POLL`], when the card is taken down, or until the person dismisses it, the chat stops or
/// [`APPROVAL_TTL`] runs out. Whatever ended it, the vault is read once more, so a passphrase
/// typed just as the card ended still counts.
async fn ask(ctx: &Ctx, store: &dyn Store, action: CrewVaultAction) -> Asked {
    let Some(step) = ctx.park_step(CrewStepRequest::Vault { action }, APPROVAL_TTL) else {
        return Asked::NoCard;
    };
    let withdraw = step.withdrawer();
    let end = step.end();
    tokio::pin!(end);
    let how = loop {
        tokio::select! {
            how = &mut end => break how,
            () = tokio::time::sleep(POLL) => {
                if store.status().await.is_ok_and(Status::open) {
                    withdraw.cancel();
                    break (&mut end).await;
                }
            }
        }
    };
    match store.status().await {
        Ok(status) if status.open() => Asked::Open(status),
        _ => Asked::Closed(how),
    }
}

/// Why the vault is still closed after a card for `action` ended `how`.
fn still_closed(action: CrewVaultAction, how: StepEnd) -> String {
    let (doing, state) = match action {
        CrewVaultAction::Unlock => ("unlocking", "still locked"),
        CrewVaultAction::Init => ("setting up", "still not set up"),
    };
    match how {
        StepEnd::Dismissed => {
            format!("The person closed the card without {doing} Crew's vault, so it is {state}.")
        }
        StepEnd::Stopped => format!("The chat was stopped, so Crew's vault is {state}."),
        StepEnd::TimedOut => {
            format!("Nobody finished {doing} Crew's vault in time, so it is {state}.")
        }
        StepEnd::Withdrawn | StepEnd::NotShown => format!("Crew's vault is {state}."),
    }
}

/// Said when no card can be shown: where the person does it themselves, as before.
fn no_card(action: CrewVaultAction) -> &'static str {
    match action {
        CrewVaultAction::Unlock => "This chat can't open the vault prompt here. The person unlocks Crew's vault in Crew, under Keys, or runs biorouter crew credentials unlock in a terminal.",
        CrewVaultAction::Init => "This chat can't open the vault prompt here. The person sets up Crew's vault in Crew, under Keys, or runs biorouter crew credentials init in a terminal.",
    }
}

// ---------------------------------------------------------------------------------------------
// The actions
// ---------------------------------------------------------------------------------------------

/// `crew__read credentials_status`: whether Crew's vault on this computer is set up and
/// locked, as `GET /crew/credentials` answers it. Not workspace content: any chat may read it.
pub(super) fn credentials_status(_ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move { Ok(Manager.status().await?.answer()) })
}

/// `crew__change credentials_unlock`: the vault card, then the vault's state.
pub(super) fn credentials_unlock(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(open_vault(ctx, &Manager, CrewVaultAction::Unlock))
}

/// `crew__change credentials_init`: the vault card set to init, then the vault's state.
pub(super) fn credentials_init(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(open_vault(ctx, &Manager, CrewVaultAction::Init))
}

/// `crew__change credentials_lock`: an ordinary approval card, then the lock.
pub(super) fn credentials_lock(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(lock_vault(ctx, &Manager))
}

/// Unlock or set up the vault: refuse what the daemon would refuse before the person types
/// anything, answer at once when there is nothing to do, else park the vault card.
async fn open_vault(ctx: &Ctx, store: &dyn Store, action: CrewVaultAction) -> Result<Value> {
    let status = store.status().await?;
    if let Some(answer) = before_asking(store, status, action).await {
        return answer;
    }
    match ask(ctx, store, action).await {
        Asked::Open(status) => Ok(status.noted(match action {
            CrewVaultAction::Unlock => "Crew's vault is unlocked.",
            CrewVaultAction::Init => "Crew's vault is set up and unlocked.",
        })),
        Asked::Closed(how) => Err(said(None, still_closed(action, how))),
        Asked::NoCard => Err(said(None, no_card(action))),
    }
}

/// The answer for `action` when no card is needed or none may be asked: the vault is already
/// as asked, there is none to unlock, or the daemon would refuse to set one up here
/// (`CrewManager::init_vault`). `None` when the card is the next step.
async fn before_asking(
    store: &dyn Store,
    status: Status,
    action: CrewVaultAction,
) -> Option<Result<Value>> {
    match action {
        CrewVaultAction::Unlock if !status.initialized => Some(Err(said(
            None,
            format!(
                "Crew has no vault on this computer, so there is nothing to unlock. It keeps its keys in {}.",
                status.keys_live()
            ),
        ))),
        CrewVaultAction::Unlock | CrewVaultAction::Init if status.open() => {
            Some(Ok(status.noted(match action {
                CrewVaultAction::Unlock => "Crew's vault is already unlocked.",
                CrewVaultAction::Init => "Crew's vault is already set up and unlocked.",
            })))
        }
        CrewVaultAction::Unlock => None,
        CrewVaultAction::Init if status.initialized => Some(Err(said(
            None,
            "Crew's vault is already set up on this computer, and it is locked. Unlock it with crew__change action credentials_unlock.",
        ))),
        CrewVaultAction::Init if status.backend == "file" => Some(Err(said(
            None,
            "This Biorouter profile keeps Crew keys in development files, so it can't set up a vault.",
        ))),
        CrewVaultAction::Init if store.holds_connections().await => Some(Err(said(
            None,
            "Crew already keeps a key on this computer for a saved connection, so a vault can't be set up over it. A vault can only be set up before the first connection.",
        ))),
        CrewVaultAction::Init => None,
    }
}

/// Lock the vault, once the person allowed it on an ordinary approval card.
async fn lock_vault(ctx: &Ctx, store: &dyn Store) -> Result<Value> {
    let status = store.status().await?;
    if !status.initialized {
        return Err(said(
            None,
            format!(
                "Crew has no vault on this computer, so there is nothing to lock. It keeps its keys in {}.",
                status.keys_live()
            ),
        ));
    }
    if status.locked {
        return Ok(status.noted("Crew's vault is already locked."));
    }
    ctx.approve(Card::new(LOCK_CARD).line(LOCK_CARD_LINE))
        .await?;
    store.lock().await?;
    Ok(store.status().await?.noted("Crew's vault is locked."))
}

// ---------------------------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------------------------

/// Called first by [`Ctx::approve`], before any change's approval card is parked. A change that
/// will need the vault while it is locked parks the vault card here first, so the person never
/// approves a change twice. An `Err` is the change's refusal: nothing was asked and nothing was
/// changed. Where no card can be shown, or the vault can't be read, nothing is asked here and
/// the change goes on as before.
pub(super) async fn before_approval(ctx: &Ctx) -> Result<()> {
    before_approval_with(ctx, &Manager).await
}

async fn before_approval_with(ctx: &Ctx, store: &dyn Store) -> Result<()> {
    // Inside `run_unlocked`, a refusal from here on is never answered by running again.
    let _ = APPROACHED_APPROVAL.try_with(|approached| approached.set(true));
    if NEEDS_NO_VAULT.contains(&ctx.action.name) {
        return Ok(());
    }
    let Ok(status) = store.status().await else {
        return Ok(());
    };
    if !status.locked_vault() {
        return Ok(());
    }
    match ask(ctx, store, CrewVaultAction::Unlock).await {
        Asked::Open(_) | Asked::NoCard => Ok(()),
        Asked::Closed(how) => Err(said(
            None,
            format!(
                "{} Nothing was changed.",
                still_closed(CrewVaultAction::Unlock, how)
            ),
        )),
    }
}

/// Run `run`, an action of `ctx`, and when it was refused because Crew's vault is locked before
/// it reached an approval (a read, or a change resolving its targets), park the vault card and
/// run it once more after the person unlocked it. A read then asks for its read consent, and a
/// change for its approval, exactly once. Any other refusal, one after an approval was asked,
/// and the vault's own actions are answered as they are; so is a locked vault where no card can
/// be shown, as before. A change [`NEEDS_NO_VAULT`] lists is asked for here too: a refusal by
/// the locked vault before its approval shows it did need the vault.
///
/// The host calls it in place of `run(&ctx)` (`ChatCrewHost::call`).
pub(super) async fn run_unlocked(ctx: &Ctx, run: Run) -> Result<Value> {
    run_unlocked_with(ctx, run, &Manager).await
}

async fn run_unlocked_with(ctx: &Ctx, run: Run, store: &dyn Store) -> Result<Value> {
    let (answer, approached) = APPROACHED_APPROVAL
        .scope(Cell::new(false), async {
            let answer = run(ctx).await;
            (answer, APPROACHED_APPROVAL.with(Cell::get))
        })
        .await;
    let error = match answer {
        Ok(answer) => return Ok(answer),
        Err(error) => error,
    };
    if approached || VAULT_ACTIONS.contains(&ctx.action.name) || !refused_by_locked_vault(&error) {
        return Err(error);
    }
    match store.status().await {
        // Unlocked meanwhile, by the person elsewhere: no card is needed to run it again.
        Ok(status) if status.open() => return run(ctx).await,
        Ok(status) if status.locked_vault() => {}
        _ => return Err(error),
    }
    match ask(ctx, store, CrewVaultAction::Unlock).await {
        Asked::Open(_) => run(ctx).await,
        Asked::Closed(how) => Err(said(
            None,
            format!(
                "{} Nothing was done.",
                still_closed(CrewVaultAction::Unlock, how)
            ),
        )),
        Asked::NoCard => Err(error),
    }
}

/// Whether `error` is the vault refusing a key because it is locked.
fn refused_by_locked_vault(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains(LOCKED_VAULT_TEXT))
}

/// The step for `crew_credential_store_unavailable` where no card can be shown: the person's
/// own, as before the vault could be opened from a chat.
const STORE_STEP_WITHOUT_A_CARD: &str =
    "The person unlocks it in Crew, under Keys, or runs biorouter crew credentials unlock.";

/// The step for `crew_credential_store_unavailable` where a card can be shown: the vault's
/// actions, each through its card, then the person's own way as before.
const STORE_STEP: &str = "Read Crew's vault with crew__read action credentials_status. If it is locked, unlock it with crew__change action credentials_unlock; if this computer has no vault yet, set one up with credentials_init. The person types the passphrase in their own secure prompt, never in the chat. They can also unlock it in Crew, under Keys, or run biorouter crew credentials unlock.";

/// The step a refusal with `crew_credential_store_unavailable` names for the model
/// ([`super::next_step`]), or `None` when `code` is not the credential store's: check the vault,
/// then unlock it, or set one up where there is none, each through its card. Where no person's
/// proof can be had (`biorouter serve`), no card can be shown, so the step is today's.
pub(super) fn next_step(code: &str, _connection: Option<&str>) -> Option<String> {
    (code == "crew_credential_store_unavailable")
        .then(|| store_step(user_proof_available()).to_owned())
}

/// [`next_step`]'s words for the credential store, with or without a person's proof.
fn store_step(proof: bool) -> &'static str {
    if proof {
        STORE_STEP
    } else {
        STORE_STEP_WITHOUT_A_CARD
    }
}

#[cfg(test)]
mod tests {
    //! Against a stand-in vault ([`Fake`]): the daemon's own manager is shared by every test in
    //! this binary and must never be given a vault. `crew_tools_vault.rs` runs the same paths
    //! against a real vault, through the real host and routes.

    use super::super::test_support::test_ctx;
    use super::*;
    use biorouter::crew::tool_host::PersonTool;
    use biorouter::pending_user_action::{DecisionAuthority, ResolveOutcome, UserActionOutcome};
    use biorouter::permission::Permission;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};

    const LOCKED: Status = Status {
        backend: "encrypted_vault",
        initialized: true,
        locked: true,
        available: true,
    };
    const OPEN: Status = Status {
        locked: false,
        ..LOCKED
    };
    const KEYRING: Status = Status {
        backend: "keyring",
        initialized: false,
        locked: false,
        available: true,
    };
    const FILES: Status = Status {
        backend: "file",
        ..KEYRING
    };

    /// A vault whose state a test sets, counting locks and the runs of [`needs_the_vault`].
    struct Fake {
        status: Mutex<Status>,
        connections: bool,
        locks: AtomicUsize,
        runs: AtomicUsize,
    }

    impl Fake {
        fn new(status: Status) -> Arc<Self> {
            Self::with_connections(status, false)
        }

        fn with_connections(status: Status, connections: bool) -> Arc<Self> {
            Arc::new(Self {
                status: Mutex::new(status),
                connections,
                locks: AtomicUsize::new(0),
                runs: AtomicUsize::new(0),
            })
        }

        fn now(&self) -> Status {
            *self.status.lock().unwrap()
        }

        fn set(&self, status: Status) {
            *self.status.lock().unwrap() = status;
        }
    }

    impl Store for Fake {
        fn status(&self) -> BoxFuture<'_, Result<Status>> {
            Box::pin(async { Ok(self.now()) })
        }

        fn lock(&self) -> BoxFuture<'_, Result<()>> {
            Box::pin(async {
                self.locks.fetch_add(1, Ordering::SeqCst);
                self.set(LOCKED);
                Ok(())
            })
        }

        fn holds_connections(&self) -> BoxFuture<'_, bool> {
            Box::pin(async { self.connections })
        }
    }

    async fn ctx(tool: PersonTool, action: &str) -> Ctx {
        test_ctx(tool, action, json!({})).await
    }

    /// The cards parked in the call's chat, as published: `(id, data)`.
    fn cards(ctx: &Ctx) -> Vec<(String, Value)> {
        ctx.call
            .approvals
            .pending_cards_for_session(&ctx.call.session_id)
            .iter()
            .map(|message| {
                let data = serde_json::to_value(message).unwrap()["content"][0]["data"].clone();
                (data["id"].as_str().unwrap_or_default().to_owned(), data)
            })
            .collect()
    }

    /// The first card the call parks, once it is up. A call that parks none fails the test.
    async fn card(ctx: &Ctx) -> (String, Value) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(card) = cards(ctx).into_iter().next() {
                return card;
            }
            assert!(tokio::time::Instant::now() < deadline, "no card was parked");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A refusal with its whole chain, as the host's log would see it.
    fn refusal(answer: Result<Value>) -> String {
        format!("{:#}", answer.expect_err("refused"))
    }

    /// What the vault answers is its state, under the route's four names, and nothing else.
    #[test]
    fn the_status_answer_is_the_vault_state_and_nothing_else() {
        let answer = LOCKED.answer();
        let mut keys: Vec<&str> = answer
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["available", "backend", "initialized", "locked"]);
        assert_eq!(answer["backend"], "encrypted_vault");
        assert_eq!(answer["locked"], true);
    }

    /// Unlock parks one vault card in the chat and nothing else; a forged Allow is refused and
    /// leaves the card up; once the vault is open, the call takes the card down and answers.
    #[tokio::test]
    async fn unlock_parks_one_vault_card_and_answers_once_the_vault_is_open() {
        let ctx = ctx(PersonTool::Change, "credentials_unlock").await;
        let fake = Fake::new(LOCKED);
        let person = async {
            let (id, data) = card(&ctx).await;
            assert_eq!(data["actionType"], "crewStep");
            assert_eq!(data["step"], json!({"kind": "vault", "action": "unlock"}));
            let forged = ctx.call.approvals.resolve_in_session(
                &ctx.call.session_id,
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce,
                },
                DecisionAuthority::unproven(),
            );
            assert_eq!(forged, ResolveOutcome::Rejected);
            tokio::time::sleep(POLL * 2).await;
            assert!(
                ctx.call.approvals.is_pending(&id),
                "a forged Allow ended it"
            );
            assert_eq!(cards(&ctx).len(), 1);
            fake.set(OPEN);
            id
        };
        let (answer, id) = tokio::join!(open_vault(&ctx, &*fake, CrewVaultAction::Unlock), person);
        let answer = answer.unwrap();
        assert_eq!(answer["locked"], false);
        assert_eq!(answer["note"], "Crew's vault is unlocked.");
        assert!(!ctx.call.approvals.is_pending(&id));
        assert!(
            !ctx.call.cancel.is_cancelled(),
            "taking the card down stopped the chat"
        );
    }

    /// Dismissed or stopped, the vault is still locked and the call says so.
    #[tokio::test]
    async fn a_dismissed_or_stopped_card_answers_still_locked() {
        let ctx = ctx(PersonTool::Change, "credentials_unlock").await;
        let fake = Fake::new(LOCKED);
        let person = async {
            let (id, _) = card(&ctx).await;
            assert_eq!(
                ctx.call
                    .approvals
                    .dismiss_crew_step(&ctx.call.session_id, &id),
                ResolveOutcome::Delivered
            );
        };
        let (answer, ()) = tokio::join!(open_vault(&ctx, &*fake, CrewVaultAction::Unlock), person);
        assert_eq!(
            refusal(answer),
            "The person closed the card without unlocking Crew's vault, so it is still locked."
        );

        let ctx = ctx_for("credentials_init").await;
        let fresh = Fake::new(KEYRING);
        let person = async {
            card(&ctx).await;
            ctx.call.cancel.cancel();
        };
        let (answer, ()) = tokio::join!(open_vault(&ctx, &*fresh, CrewVaultAction::Init), person);
        assert_eq!(
            refusal(answer),
            "The chat was stopped, so Crew's vault is still not set up."
        );
    }

    #[test]
    fn every_way_a_card_ends_without_the_vault_says_so() {
        for (how, said) in [
            (
                StepEnd::TimedOut,
                "Nobody finished unlocking Crew's vault in time, so it is still locked.",
            ),
            (StepEnd::NotShown, "Crew's vault is still locked."),
        ] {
            assert_eq!(still_closed(CrewVaultAction::Unlock, how), said);
        }
    }

    /// Init parks the card set to init on a fresh profile; everything the daemon would refuse,
    /// or that needs nothing, is answered before the person types anything.
    #[tokio::test]
    async fn nothing_is_asked_when_there_is_nothing_to_do_or_the_daemon_would_refuse() {
        let ctx = ctx(PersonTool::Change, "credentials_init").await;
        let open = open_vault(&ctx, &*Fake::new(OPEN), CrewVaultAction::Unlock)
            .await
            .unwrap();
        assert_eq!(open["note"], "Crew's vault is already unlocked.");
        let none = refusal(open_vault(&ctx, &*Fake::new(KEYRING), CrewVaultAction::Unlock).await);
        assert!(none.contains("nothing to unlock"), "{none}");
        assert!(none.contains("this computer's keyring"), "{none}");
        let set_up = refusal(open_vault(&ctx, &*Fake::new(LOCKED), CrewVaultAction::Init).await);
        assert!(set_up.contains("credentials_unlock"), "{set_up}");
        let files = refusal(open_vault(&ctx, &*Fake::new(FILES), CrewVaultAction::Init).await);
        assert!(files.contains("development files"), "{files}");
        let shadowed = refusal(
            open_vault(
                &ctx,
                &*Fake::with_connections(KEYRING, true),
                CrewVaultAction::Init,
            )
            .await,
        );
        assert!(shadowed.contains("saved connection"), "{shadowed}");
        assert!(cards(&ctx).is_empty(), "a card was parked for nothing");

        let fresh = Fake::new(Status {
            available: false,
            ..KEYRING
        });
        let person = async {
            let (_, data) = card(&ctx).await;
            assert_eq!(data["step"], json!({"kind": "vault", "action": "init"}));
            fresh.set(OPEN);
        };
        let (answer, ()) = tokio::join!(open_vault(&ctx, &*fresh, CrewVaultAction::Init), person);
        assert_eq!(
            answer.unwrap()["note"],
            "Crew's vault is set up and unlocked."
        );
    }

    /// Where no person can answer a card, none is parked and the step is said in words.
    #[tokio::test]
    async fn no_card_is_parked_where_no_person_can_answer() {
        let ctx = ctx(PersonTool::Change, "credentials_unlock").await;
        let fake = Fake::new(LOCKED);
        let answer = biorouter::user_surface::without_human_surface(open_vault(
            &ctx,
            &*fake,
            CrewVaultAction::Unlock,
        ))
        .await;
        let said = refusal(answer);
        assert!(said.contains("biorouter crew credentials unlock"), "{said}");
        assert!(cards(&ctx).is_empty());
        let before = biorouter::user_surface::without_human_surface(before_approval_with(
            &ctx_for("send").await,
            &*fake,
        ))
        .await;
        assert!(
            before.is_ok(),
            "the change goes on to say what it says today"
        );
    }

    async fn ctx_for(change: &str) -> Ctx {
        ctx(PersonTool::Change, change).await
    }

    /// A change asks for a locked vault before its approval card, so it is approved once; a
    /// dismissal refuses the change before its card.
    #[tokio::test]
    async fn a_locked_vault_is_asked_for_before_a_changes_approval() {
        let ctx = ctx_for("send").await;
        let fake = Fake::new(LOCKED);
        let person = async {
            let (_, data) = card(&ctx).await;
            assert_eq!(data["actionType"], "crewStep");
            fake.set(OPEN);
        };
        let (before, ()) = tokio::join!(before_approval_with(&ctx, &*fake), person);
        assert!(before.is_ok());
        assert!(cards(&ctx).is_empty());

        fake.set(LOCKED);
        let person = async {
            let (id, _) = card(&ctx).await;
            ctx.call
                .approvals
                .dismiss_crew_step(&ctx.call.session_id, &id);
        };
        let (before, ()) = tokio::join!(before_approval_with(&ctx, &*fake), person);
        let said = before.unwrap_err().to_string();
        assert!(said.ends_with("Nothing was changed."), "{said}");
    }

    /// No card first for the vault's own actions, for leaving a connection, or for a vault that
    /// is open or absent.
    #[tokio::test]
    async fn nothing_is_asked_first_where_the_vault_is_not_needed_or_not_locked() {
        // Each is a change the catalog lists (`test_ctx` refuses any other name), so a renamed
        // action can't fall off the list unnoticed.
        for &change in NEEDS_NO_VAULT {
            let ctx = ctx_for(change).await;
            assert!(before_approval_with(&ctx, &*Fake::new(LOCKED))
                .await
                .is_ok());
            assert!(cards(&ctx).is_empty(), "{change}");
        }
        let ctx = ctx_for("send").await;
        for status in [OPEN, KEYRING, FILES] {
            assert!(before_approval_with(&ctx, &*Fake::new(status))
                .await
                .is_ok());
        }
        assert!(cards(&ctx).is_empty());
    }

    /// Stopping and taking back never wait on a vault card, whatever else is listed.
    #[test]
    fn stopping_or_taking_back_never_waits_for_the_vault() {
        for change in [
            "setup_cancel",
            "disconnect",
            "tasks_cancel",
            "grants_revoke",
            "files_pause",
        ] {
            assert!(NEEDS_NO_VAULT.contains(&change), "{change}");
        }
        for action in VAULT_ACTIONS {
            assert!(NEEDS_NO_VAULT.contains(&action), "{action}");
        }
    }

    /// The words [`refused_by_locked_vault`] looks for are the vault's own, and the transfer
    /// service's: a reword in either would stop the card from being offered with nothing else
    /// failing.
    #[test]
    fn the_locked_vault_words_are_the_vaults_own() {
        for (file, source) in [
            (
                "crates/biorouter/src/crew/credentials.rs",
                include_str!("../../../../biorouter/src/crew/credentials.rs"),
            ),
            (
                "crates/biorouter-server/src/crew/transfers.rs",
                include_str!("../../crew/transfers.rs"),
            ),
        ] {
            assert!(source.contains(LOCKED_VAULT_TEXT), "{file}");
        }
        assert!(refused_by_locked_vault(&locked_vault_error()));
        assert!(!refused_by_locked_vault(&said(None, "No such channel.")));
    }

    /// Lock asks on an ordinary approval card, and a decline locks nothing. (The Allow, which
    /// needs the person's proof, is `crew_tools_vault.rs`'s.)
    #[tokio::test]
    async fn lock_asks_on_an_approval_card_and_a_decline_locks_nothing() {
        let ctx = ctx_for("credentials_lock").await;
        let fake = Fake::new(OPEN);
        let person = async {
            let (id, data) = card(&ctx).await;
            assert_eq!(data["actionType"], "toolConfirmation");
            let prompt = data["prompt"].as_str().unwrap();
            assert!(prompt.starts_with(LOCK_CARD), "{prompt}");
            assert!(prompt.contains(LOCK_CARD_LINE), "{prompt}");
            ctx.call.approvals.resolve_in_session(
                &ctx.call.session_id,
                &id,
                UserActionOutcome::Denied {
                    permission: Permission::DenyOnce,
                },
                DecisionAuthority::unproven(),
            );
        };
        let (answer, ()) = tokio::join!(lock_vault(&ctx, &*fake), person);
        assert_eq!(refusal(answer), "The person declined. Nothing was changed.");
        assert_eq!(fake.locks.load(Ordering::SeqCst), 0);
        assert_eq!(fake.now(), OPEN);

        let locked = lock_vault(&ctx, &*Fake::new(LOCKED)).await.unwrap();
        assert_eq!(locked["note"], "Crew's vault is already locked.");
        let none = refusal(lock_vault(&ctx, &*Fake::new(KEYRING)).await);
        assert!(none.contains("nothing to lock"), "{none}");
        assert!(cards(&ctx).is_empty());
    }

    // ------------------------------------------------------------------------- run_unlocked

    /// The fake each test action reads, by chat: a [`Run`] is a plain `fn`.
    static RUNS: LazyLock<Mutex<HashMap<String, Arc<Fake>>>> = LazyLock::new(Mutex::default);

    fn fake_of(ctx: &Ctx) -> Arc<Fake> {
        Arc::clone(&RUNS.lock().unwrap()[&ctx.call.session_id])
    }

    /// The vault's refusal of a key while it is locked, under the action's own words.
    fn locked_vault_error() -> anyhow::Error {
        anyhow::anyhow!(
            "Crew credential vault is locked; explicitly unlock it for this daemon session"
        )
        .context("Couldn't read the workspace")
    }

    /// One run of an action that needs a key from the vault.
    fn needs_the_vault_once(fake: &Fake) -> Result<Value> {
        fake.runs.fetch_add(1, Ordering::SeqCst);
        if fake.now().locked {
            return Err(locked_vault_error());
        }
        Ok(json!({"read": true}))
    }

    /// An action that needs a key from the vault.
    fn needs_the_vault(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move { needs_the_vault_once(&fake_of(ctx)) })
    }

    /// Refused by the locked vault on its first run, and unlocked by then on its second (the
    /// person unlocked it in Crew meanwhile).
    fn needs_the_vault_then_open(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
            let fake = fake_of(ctx);
            let answer = needs_the_vault_once(&fake);
            fake.set(OPEN);
            answer
        })
    }

    /// Reaches its approval with the vault open, then finds it locked when it needs the key.
    fn approves_then_locks(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
            let fake = fake_of(ctx);
            before_approval_with(ctx, &*fake).await?;
            fake.set(LOCKED);
            needs_the_vault_once(&fake)
        })
    }

    /// An action refused for another reason.
    fn refused_otherwise(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
            fake_of(ctx).runs.fetch_add(1, Ordering::SeqCst);
            Err(said(None, "No such channel."))
        })
    }

    async fn running(tool: PersonTool, action: &str, status: Status) -> (Ctx, Arc<Fake>) {
        let ctx = ctx(tool, action).await;
        let fake = Fake::new(status);
        RUNS.lock()
            .unwrap()
            .insert(ctx.call.session_id.clone(), Arc::clone(&fake));
        (ctx, fake)
    }

    /// A read refused by the locked vault parks the vault card, and runs once more once the
    /// person unlocked it.
    #[tokio::test]
    async fn a_run_refused_by_the_locked_vault_asks_for_it_and_runs_once_more() {
        let (ctx, fake) = running(PersonTool::Read, "history", LOCKED).await;
        let person = async {
            let (_, data) = card(&ctx).await;
            assert_eq!(data["step"]["kind"], "vault");
            fake.set(OPEN);
        };
        let (answer, ()) = tokio::join!(run_unlocked_with(&ctx, needs_the_vault, &*fake), person);
        assert_eq!(answer.unwrap(), json!({"read": true}));
        assert_eq!(fake.runs.load(Ordering::SeqCst), 2);
        assert!(cards(&ctx).is_empty());
    }

    /// Dismissed, the run is refused as still locked and nothing runs again.
    #[tokio::test]
    async fn a_dismissed_card_runs_nothing_more() {
        let (ctx, fake) = running(PersonTool::Read, "history", LOCKED).await;
        let person = async {
            let (id, _) = card(&ctx).await;
            ctx.call
                .approvals
                .dismiss_crew_step(&ctx.call.session_id, &id);
        };
        let (answer, ()) = tokio::join!(run_unlocked_with(&ctx, needs_the_vault, &*fake), person);
        let said = refusal(answer);
        assert!(said.contains("still locked"), "{said}");
        assert!(said.ends_with("Nothing was done."), "{said}");
        assert_eq!(fake.runs.load(Ordering::SeqCst), 1);
    }

    /// Never run again: after the action reached its approval (so a change is never approved
    /// twice), for any other refusal, or for a vault that is not set up and locked. Unlocked
    /// meanwhile, it runs again with no card.
    #[tokio::test]
    async fn only_a_refusal_by_the_locked_vault_before_an_approval_runs_again() {
        let (ctx, fake) = running(PersonTool::Change, "send", OPEN).await;
        let approved_then_locked = run_unlocked_with(&ctx, approves_then_locks, &*fake).await;
        assert!(refusal(approved_then_locked).contains("vault is locked"));
        assert_eq!(fake.runs.load(Ordering::SeqCst), 1);
        assert!(cards(&ctx).is_empty());

        let (ctx, fake) = running(PersonTool::Read, "history", LOCKED).await;
        assert_eq!(
            refusal(run_unlocked_with(&ctx, refused_otherwise, &*fake).await),
            "No such channel."
        );
        assert_eq!(fake.runs.load(Ordering::SeqCst), 1);

        let (ctx, fake) = running(PersonTool::Read, "history", LOCKED).await;
        let answer = run_unlocked_with(&ctx, needs_the_vault_then_open, &*fake).await;
        assert_eq!(answer.unwrap(), json!({"read": true}));
        assert_eq!(fake.runs.load(Ordering::SeqCst), 2);
        assert!(
            cards(&ctx).is_empty(),
            "a card was parked for an open vault"
        );

        let (ctx, fake) = running(PersonTool::Read, "history", LOCKED).await;
        let absent = Fake::new(KEYRING);
        assert!(
            refusal(run_unlocked_with(&ctx, needs_the_vault, &*absent).await)
                .contains("vault is locked")
        );
        assert_eq!(fake.runs.load(Ordering::SeqCst), 1);
        assert!(cards(&ctx).is_empty());
    }

    /// A change that is not asked for the vault before its card is, once the locked vault
    /// refused it before any card: that refusal shows it needed the vault. The vault's own
    /// actions never are.
    #[tokio::test]
    async fn a_listed_change_the_locked_vault_refused_is_asked_for_too() {
        let (ctx, fake) = running(PersonTool::Change, "connections_update", LOCKED).await;
        let person = async {
            let (_, data) = card(&ctx).await;
            assert_eq!(data["step"]["kind"], "vault");
            fake.set(OPEN);
        };
        let (answer, ()) = tokio::join!(run_unlocked_with(&ctx, needs_the_vault, &*fake), person);
        assert_eq!(answer.unwrap(), json!({"read": true}));
        assert_eq!(fake.runs.load(Ordering::SeqCst), 2);

        let (ctx, fake) = running(PersonTool::Change, "credentials_lock", LOCKED).await;
        assert!(
            refusal(run_unlocked_with(&ctx, needs_the_vault, &*fake).await)
                .contains("vault is locked")
        );
        assert_eq!(fake.runs.load(Ordering::SeqCst), 1);
        assert!(cards(&ctx).is_empty());
    }

    #[test]
    fn a_store_refusal_names_the_vault_actions() {
        let step = next_step("crew_credential_store_unavailable", None).unwrap();
        for action in [
            "credentials_status",
            "credentials_unlock",
            "credentials_init",
        ] {
            assert!(step.contains(action), "{step}");
        }
        assert!(step.contains("never in the chat"), "{step}");
        // The person's own way stays named, for where the card is dismissed or can't be shown.
        assert!(step.contains("biorouter crew credentials unlock"), "{step}");
        assert_eq!(step, store_step(true));
        assert!(next_step("crew_ssh_auth_required", None).is_none());
    }

    /// Where no person's proof can be had (`biorouter serve`), no vault card can be shown, so
    /// the step is the one said before the vault could be opened from a chat.
    #[test]
    fn without_proof_the_store_step_is_today_s() {
        let step = store_step(false);
        assert_eq!(
            step,
            "The person unlocks it in Crew, under Keys, or runs biorouter crew credentials unlock."
        );
        assert!(!step.contains("credentials_unlock"), "{step}");
    }

    /// Every sentence here: second person or plain, "computer", no dashes.
    #[test]
    fn the_vault_words_follow_the_copy_rules() {
        let mut words = vec![
            LOCK_CARD.to_owned(),
            LOCK_CARD_LINE.to_owned(),
            store_step(true).to_owned(),
            store_step(false).to_owned(),
        ];
        for action in [CrewVaultAction::Unlock, CrewVaultAction::Init] {
            words.push(no_card(action).to_owned());
            for how in [
                StepEnd::Dismissed,
                StepEnd::Stopped,
                StepEnd::TimedOut,
                StepEnd::NotShown,
            ] {
                words.push(still_closed(action, how));
            }
        }
        for text in words {
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
            assert!(!text.to_lowercase().contains("device"), "{text}");
        }
    }
}
