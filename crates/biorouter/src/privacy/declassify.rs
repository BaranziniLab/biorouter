//! Declassification (issue #56, §12.4/§12.5) — the one writer in the tree
//! permitted to LOWER `sessions.privacy_tier`.
//!
//! Everywhere else the classification is a permanent ratchet: writes go through
//! [`crate::session::session_manager::SessionUpdateBuilder::raise_privacy`],
//! whose emission is a monotone `CASE WHEN` that physically cannot express a
//! lowering, whatever the caller passes. That is the property the whole feature
//! rests on, so the escape hatch is exactly one function, in its own module,
//! with a type-level proof of user in its signature and a ledger row for every
//! use.
//!
//! # The three proofs, and why there are three (DR-20, Task 55)
//!
//! A chat §12.4 grades onto the strong control needs all of:
//!
//! 1. **`X-User-Action`** (or a terminal), which says the request came from a
//!    surface a human acts at rather than from a tool call.
//! 2. **The typed phrase** ([`confirmation_matches`]), which says *which* chat
//!    the human meant. A password cannot say that.
//! 3. **The operating system's own authentication**
//!    ([`authenticate_declassification`]), which says *who* the human is. The
//!    phrase cannot say that: it is derived from an id the caller already had.
//!
//! Two proofs answering two different questions is not redundancy. A `turn:*`
//! chat has only (1) and a single click — DR-20's cost is spent where the
//! consequence is, and making the common case expensive is how you teach people
//! to stop privatising at all.

use crate::privacy::system_auth::{self, AuthRequest, SystemAuthRefusal, SystemAuthenticator};
use crate::privacy::SessionClassification;
use crate::session::session_manager::SessionManager;
use anyhow::Result;

/// The `privacy_reason` a declassified session carries afterwards.
///
/// It replaces the provenance (`mcp:…`/`turn:…`) rather than clearing it, so a
/// row that reads Public is still distinguishable from one that was born Public
/// — and the storage layer treats a non-`mcp:` reason as displaceable, so a
/// later raise records the new provenance normally
/// (`session_manager.rs`'s `raise_privacy` doc comment spells this out).
pub const DECLASSIFIED_BY_USER: &str = "declassified_by_user";

/// §12.4's graded confirmation, as a predicate over the stored provenance.
///
/// `true` → the user must retype the last six characters of the session id.
/// `false` → a single click, with a five-second undo.
///
/// **Phrased as an exception, not as a match**, and that is the whole design:
/// the weak control is granted only when the provenance says, in as many words,
/// that this chat merely ran a turn against a private endpoint. Everything else
/// gets the strong one.
///
/// The alternative — `starts_with("mcp:")` → strong — is what §12.4 literally
/// says and it fails open twice over. `privacy_reason`'s vocabulary is `turn:*`,
/// `mcp:*`, `inherited:<parent>`, `diverged:<parent>`, `backfill:<provider>`,
/// `imported` and `declassified_by_user`; a chat branched out of an OMOP session
/// carries `diverged:<parent>`, not `mcp:*`, so a match-on-`mcp:` rule hands the
/// single-click control to a copy of exactly the conversation §12.4 wants
/// protected. §12.4 acknowledges this ("or inherited from an `mcp:*` ancestor")
/// and would need an ancestor walk to implement as written; reading the absence
/// of a `turn:` prefix as "unknown, so protect it" gets the same answer for
/// every inherited case without one, and gets it right for a reason that is
/// absent (a projection bug, a future vocabulary entry) too.
///
/// `backfill:*` is turn-like in origin and still lands on the strong control:
/// the migration inferred that tier from a bound provider rather than observing
/// a turn, so what the chat actually reached is unknown.
pub fn requires_typed_confirmation(privacy_reason: Option<&str>) -> bool {
    !privacy_reason.is_some_and(|reason| reason.starts_with("turn:"))
}

/// Does this chat owe DR-20's operating-system authentication?
///
/// **Exactly where §12.4's typed phrase is owed** — one predicate, delegating
/// rather than restating, so the two proofs cannot drift into disagreeing about
/// which chats are protected. Task 55 Step 1 rules the split: the strong control
/// gains the password, and a `turn:*` chat keeps its single click and gains
/// nothing.
///
/// It exists as its own name because the two are separate *questions* — "which
/// chat did you mean" and "who are you" — and a later ruling could grade them
/// apart. `the_grade_that_demands_a_phrase_is_the_grade_that_demands_the_password`
/// is what makes the delegation a checked claim rather than a comment.
pub fn requires_system_authentication(privacy_reason: Option<&str>) -> bool {
    requires_typed_confirmation(privacy_reason)
}

/// **Why this particular chat is on the strong control** — the clause every
/// surface that asks for the typed phrase must say instead of guessing.
///
/// `None` exactly when [`requires_typed_confirmation`] is `false`, so the
/// wording cannot be shown to a chat that is getting the single click and a
/// caller cannot reach the strong copy by accident. Pinned by
/// `a_reason_exists_for_exactly_the_chats_that_owe_the_phrase`.
///
/// ⚠ **This exists because the obvious sentence is false for most private chats
/// on day one.** Every surface used to say "this chat reached a private data
/// source", which is the rationale §12.4's table gives for the strong control —
/// but the implemented grade is an exception on `turn:*`, not a match on `mcp:`,
/// so the strong control also covers `backfill:*` and `imported`. A backfilled
/// chat reached nothing: [`SessionManager`]'s one-time migration inferred its
/// tier from the provider the row was last bound to. On a machine with history,
/// `backfill:*` is the majority of the private rows, so the majority of users
/// meeting this control were being told a reason that did not happen.
///
/// The clause is written to complete BOTH "Session `<id>` …" and "This chat …",
/// which is what lets the CLI, the daemon and the renderer share one wording.
///
/// The catch-all is deliberately a statement about the RECORD rather than about
/// the conversation: it is the only thing that is true of every value the
/// vocabulary does not name — an absent reason (a projection bug), a future
/// entry, and `declassified_by_user`, which cannot reach here while the row is
/// private but must not be able to produce a false sentence if it ever does.
pub fn strong_confirmation_reason(privacy_reason: Option<&str>) -> Option<&'static str> {
    if !requires_typed_confirmation(privacy_reason) {
        return None;
    }
    Some(match privacy_reason {
        Some(r) if r.starts_with("mcp:") => "reached a private data source",
        Some(r) if r.starts_with("inherited:") => "was created inside a private chat",
        Some(r) if r.starts_with("diverged:") => "was branched out of a private chat",
        Some(r) if r.starts_with("backfill:") => {
            "was marked private by the one-time migration, from the model it was last using rather \
             than from anything it reached"
        }
        Some("imported") => "was imported already marked private",
        _ => "does not record an observed turn on a private model as the reason it is private",
    })
}

/// The last six characters of `session_id`, which is what §12.4 asks the user to
/// retype.
///
/// Characters, not bytes: session ids are `YYYYMMDD_HHMMSS` today, but a slice
/// that can split a multi-byte character is a panic waiting for the first id
/// that is not.
///
/// NOT the session NAME, and the design says why: `is_default_session_name`
/// shows `"New Session"`, `"CLI Session"`, `"Session <N>"` and `"New session
/// <N>"` are all live placeholders, so a name-typed phrase is either a string
/// dozens of rows share — destroying the justification, which is to force the
/// user to look at *which* conversation — or a whole sentence to retype.
pub fn confirmation_phrase(session_id: &str) -> String {
    let chars: Vec<char> = session_id.chars().collect();
    chars[chars.len().saturating_sub(6)..].iter().collect()
}

/// Whitespace-insensitive and case-insensitive. The phrase is a "are you looking
/// at the right row" check, not a secret — it is derived from an id the caller
/// already had to know to address the row at all — so being strict about a
/// trailing space buys nothing and costs a user a retry.
pub fn confirmation_matches(session_id: &str, presented: Option<&str>) -> bool {
    presented.is_some_and(|typed| {
        typed
            .trim()
            .eq_ignore_ascii_case(&confirmation_phrase(session_id))
    })
}

/// Proof that a human confirmed.
///
/// A ZST with a **private field**, so the tuple literal `UserConfirmation(())`
/// is unavailable outside this module and the named constructor below is the
/// only door.
///
/// ⚠ **What Rust enforces here, stated precisely, because the design overstates
/// it.** §12.4 describes the constructor as `pub(in …)`, "invoked in exactly one
/// place". It cannot be: the callers are
/// `biorouter-server::routes::session::declassify_session` and
/// `biorouter-cli::commands::session::declassify_by_id`, both in different
/// crates, and `pub(in path)` does not cross a crate boundary. So the constructor
/// is `pub`, and the language guarantees only that a caller cannot fabricate the
/// proof by writing the struct literal — it does not, by itself, cap the number
/// of call sites.
///
/// What caps it is `the_proof_of_user_is_constructed_in_exactly_two_places`
/// below: a repo walk asserting the set of files outside this one that so much
/// as *name* `UserConfirmation` is exactly `{routes/session.rs,
/// cli/commands/session.rs}`. An MCP server, a `ToolRouter` or a `workspace_*`
/// handler that reached for this would have to name the type, and the build
/// turns red. That is a weaker mechanism than the design claims and a stronger
/// one than a route that is merely undocumented — and it is the strongest
/// available without moving `declassify` into the server crate, where it would
/// lose the `pub(crate)` storage access it needs.
///
/// ⚠ **Two doors, and the second is a terminal, not a header.** Task 31 (R10)
/// added the CLI subcommand because `list_sessions` filters to (`user`,
/// `scheduled`): a private `Hidden`, `SubAgent` or `Terminal` chat has no GUI
/// declassification control at all. Its proof-of-user is a confirmation typed at
/// a tty, so an agent holding `developer__shell` can drive it — and that same
/// agent can already write this column with `sqlite3`, so the store was never
/// protected from the shell. Both doors write the same ledger row, which is the
/// property the audit actually protects.
pub struct UserConfirmation(());

impl UserConfirmation {
    /// Mint the proof. Call this **only** after matching a confirmation the user
    /// typed; see §12.4's grading, which the route implements.
    pub fn from_typed_confirmation() -> Self {
        Self(())
    }

    /// The same proof, for tests that exercise the writer rather than the route.
    #[cfg(test)]
    fn for_test() -> Self {
        Self(())
    }
}

/// Proof that the **operating system** authenticated the user, for the chats it
/// named (DR-20, DR-24).
///
/// ⚠ **Unforgeable, and by the language rather than by an audit.** The field is
/// private and there is no public constructor: the only way to obtain one
/// outside this module is [`authenticate_declassification`], which raises a real
/// prompt and returns `Ok` for [`system_auth::AuthOutcome::Approved`] and
/// nothing else. That is a stronger guarantee than [`UserConfirmation`]'s, whose
/// constructor has to be `pub` because its callers live in other crates — this
/// one does not, because the prompt is raised here.
///
/// ⚠ **Not `Clone` and not `Copy`, deliberately.** DR-20 point 2 admits no
/// cached grant. A value that could be duplicated could be stashed in a static
/// and spent again next week; this one lives on the stack of the operation the
/// user authorised, and dies with it. What it *may* do is cover several chats —
/// that is Task 55 Step 3, and it is why [`covers`](Self::covers) exists instead
/// of the type being a bare ZST.
#[derive(Debug)]
pub struct SystemAuthorization {
    /// Exactly the set the prompt named, in the canonical (sorted, deduplicated)
    /// form [`AuthRequest`] put it in.
    session_ids: Vec<String>,
}

impl SystemAuthorization {
    /// Was `session_id` named by the prompt this authorisation came from?
    ///
    /// DR-20 point 4: one authentication may cover several chats, **but only the
    /// ones it named**. A prompt that said "3 chats" and then declassified a
    /// fourth would make the dialog a lie, which is the one thing an
    /// authorisation dialog cannot be.
    pub fn covers(&self, session_id: &str) -> bool {
        self.session_ids.iter().any(|id| id == session_id)
    }

    /// The set the prompt named. For a caller that wants to report what a batch
    /// covered; the spend check is [`covers`](Self::covers).
    pub fn session_ids(&self) -> &[String] {
        &self.session_ids
    }

    /// The same proof, for tests that exercise the writer rather than a prompt.
    ///
    /// `#[cfg(test)]` and private, so it is absent from every shipped binary and
    /// unnameable outside this file even in a test build.
    #[cfg(test)]
    fn for_test(session_ids: &[String]) -> Self {
        Self {
            session_ids: session_ids.to_vec(),
        }
    }
}

/// Raise DR-20's system-authentication prompt **once** for `session_ids`.
///
/// The whole batch costs one prompt (Task 55 Step 3): DR-20 says a
/// declassification may cover many chats, and one prompt per chat would turn a
/// tidy-up of ten old conversations into ten password dialogs — which is not a
/// stricter control, it is a control people stop using.
///
/// ⚠ **Call this LAST, immediately before the write.** Both doors probe with
/// [`declassify`] first and only prompt when it answers
/// [`DeclassifyOutcome::SystemAuthenticationRequired`] — i.e. when the row is
/// really private, really graded onto the strong control, and the typed phrase
/// has already matched. Prompting earlier would ask a user for their password
/// and then tell them the phrase was wrong.
pub async fn authenticate_declassification(
    session_ids: &[String],
) -> Result<SystemAuthorization, SystemAuthRefusal> {
    authorize_with(system_auth::prompter(), session_ids).await
}

/// [`authenticate_declassification`] with the prompter given rather than
/// resolved.
///
/// **Private**, and that is the security boundary: a caller that could choose
/// the prompter could choose one that approves. Its only production caller is
/// the function above, which passes [`system_auth::prompter`] — the one resolver
/// in the tree that can reach the test seam, and only in a build where the seam
/// compiles at all. What the split buys is that "one prompt for a batch",
/// "denied refuses" and "unavailable refuses" are assertions about the real
/// decision path, testable on any host, with no password typed.
async fn authorize_with(
    prompter: &dyn SystemAuthenticator,
    session_ids: &[String],
) -> Result<SystemAuthorization, SystemAuthRefusal> {
    // Canonicalised BEFORE the sentence is composed: a caller that passes the
    // same id twice must not be told the prompt covers two chats.
    let mut named: Vec<String> = session_ids.to_vec();
    named.sort();
    named.dedup();
    let Ok(request) = AuthRequest::new(declassification_reason(named.len()), &named) else {
        // An empty set names nothing, so the prompt could not state what it
        // authorises and the proof would be spendable on nothing — which reads
        // at the call site as a successful authentication that does nothing.
        // Refused rather than accepted-and-ignored.
        return Err(SystemAuthRefusal {
            outcome: system_auth::AuthOutcome::Denied,
            message: "No chats were named, so there was nothing to authenticate for and \
                      nothing was changed."
                .to_string(),
        });
    };

    match system_auth::authenticate_or_refuse(prompter, &request).await {
        None => Ok(SystemAuthorization {
            session_ids: request.session_ids,
        }),
        Some(refusal) => Err(refusal),
    }
}

/// The sentence the operating system shows above the password field.
///
/// DR-20 point 4 wants the prompt to state the operation; each platform appends
/// the ids from [`AuthRequest::session_ids`], so this is the verb and the count
/// and the ids are the subject.
fn declassification_reason(count: usize) -> String {
    if count == 1 {
        "Make this private Biorouter chat public.".to_string()
    } else {
        format!("Make {count} private Biorouter chats public.")
    }
}

/// What a declassification actually did. The route turns these into 200 / 200 /
/// 400 / 403 / 404 — a "no such session" must not read as a successful
/// declassification, and an already-public row must not read as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclassifyOutcome {
    /// The row was Private (to the fail-closed reader) and is now Public. One
    /// ledger row was written.
    Declassified,
    /// The row was already exactly `public`. Nothing was written — a
    /// double-clicked confirm button must not leave a second ledger entry
    /// claiming a transition that never happened.
    AlreadyPublic,
    /// The provenance read inside the transaction grades this chat onto §12.4's
    /// typed confirmation, and the confirmation presented did not match. Nothing
    /// was written.
    ConfirmationRequired,
    /// The provenance read inside the transaction grades this chat onto DR-20's
    /// system authentication, and no [`SystemAuthorization`] covering this id
    /// was presented. Nothing was written.
    ///
    /// ⚠ **It is a PROBE result as much as a refusal**, and both doors rely on
    /// that. They call [`declassify`] with no authorisation first; this outcome
    /// is what tells them the prompt is owed — after the row has been found, the
    /// grade taken from the provenance inside the transaction, and the typed
    /// phrase matched. A door that surfaces it to the user has already raised
    /// the prompt and been refused, or has re-probed with an authorisation that
    /// does not name this chat.
    SystemAuthenticationRequired,
    /// No row with that id.
    SessionNotFound,
}

/// The ONLY writer in the tree permitted to lower `privacy_tier`.
///
/// Every other write goes through the session update builder, whose emission is
/// the monotone `CASE WHEN` and physically cannot lower it; this bypasses the
/// builder with its own `UPDATE`.
/// `exactly_one_statement_in_the_tree_assigns_a_public_classification` asserts
/// that this is the only such statement outside the migration.
///
/// **The audit row is written in the SAME transaction, BEFORE the `UPDATE`**, so
/// it observes the pre-change state — which the session row itself will not hold
/// a moment later, because the declassification overwrites `privacy_reason`. A
/// crash between the two leaves either both or neither.
///
/// The bound provider is deliberately untouched. A public chat may run a private
/// model; that direction was never restricted (`bind_allowed` admits any
/// provider on a public session), so clearing it here would break a working chat
/// to enforce a rule that does not exist.
///
/// **§12.4's grade is decided here, from the provenance this transaction reads**,
/// rather than by the caller from an earlier read. A caller that checked the
/// grade first and then called this would be a check-then-act: a `turn:*` chat
/// that reaches an MCP data source in the window between the two would be
/// declassified on the single-click control it no longer qualifies for. The
/// grade and the write are now the same transaction, so the provenance that
/// decided which confirmation was required is exactly the provenance the
/// `UPDATE` overwrites.
///
/// The non-writing outcomes are ordered deliberately: **not found**, then
/// **already public**, then **confirmation required**, then **system
/// authentication required**. An already-public row is a no-op, so there is
/// nothing to confirm and demanding a phrase for it would refuse a second,
/// harmless click of the same button — and after a successful call the
/// provenance reads `declassified_by_user`, which
/// [`requires_typed_confirmation`] grades onto the strong control, so that
/// second click would otherwise be refused over a phrase the single-click path
/// never showed the user.
///
/// The password comes **after** the phrase for the same class of reason: a user
/// who mistyped the phrase must learn that from a form field, not from an
/// operating-system dialog they had to satisfy first. It also makes the probe
/// call the doors make cheap and side-effect-free — see
/// [`DeclassifyOutcome::SystemAuthenticationRequired`].
///
/// ⚠ **Two of these racing on the same row do not both succeed, but not because
/// of anything written here.** The pool is `max_connections(4)` over WAL, so
/// both transactions can hold the same read snapshot showing `private`; the
/// loser's upgrade to a writer then fails `SQLITE_BUSY_SNAPSHOT` *immediately* —
/// a busy handler does not cover a snapshot conflict, which this tree has
/// measured at 0.0000s elsewhere — so the single-ledger-row invariant holds by
/// SQLite's snapshot isolation, and the loser surfaces as a 500 rather than the
/// tidy [`DeclassifyOutcome::AlreadyPublic`] a sequential second call gets. The
/// direction is safe (a refusal, never a double write) and the double-click that
/// motivated `AlreadyPublic` is sequential in practice, because the dialog
/// disables its confirm button while a request is in flight.
///
/// ⚠ **Nothing here stops an in-flight turn from raising the row straight back.**
/// Declassifying a chat that is mid-turn leaves a running agent that may reach a
/// private model or data source a moment later and re-raise through the normal
/// ratchet, writing a second ledger row under the new provenance. The audit stays
/// honest and the direction is fail-safe — private is the protected state — but
/// the user can watch their action undo itself. Preventing it would mean
/// refusing to declassify a busy session, which §12.4 does not ask for.
/// ⚠ **`_ok` is borrowed rather than consumed**, and that is what keeps the two
/// doors at ONE construction site each. Both call this twice for a chat on the
/// strong control — once to probe, once to write — and
/// [`the_proof_of_user_is_constructed_in_exactly_two_places`] counts
/// constructions per file, not calls. One human action, one proof, however many
/// times the writer is asked.
pub async fn declassify(
    sm: &SessionManager,
    session_id: &str,
    confirmation: Option<&str>,
    authorization: Option<&SystemAuthorization>,
    _ok: &UserConfirmation,
) -> Result<DeclassifyOutcome> {
    let pool = sm.storage().pool().await?;
    let mut tx = pool.begin().await?;

    // Inside the transaction, so the state the ledger records is the state the
    // UPDATE below overwrites — not a snapshot from before some concurrent
    // ratchet.
    let Some((raw_tier, reason_before, provider_name)) =
        sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT privacy_tier, privacy_reason, provider_name FROM sessions WHERE id = ?1",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?
    else {
        return Ok(DeclassifyOutcome::SessionNotFound);
    };

    // The tier the READER sees, not the raw bytes. `from_stored` fails closed, so
    // a row holding `PUBLIC` (a hand-edited database, a restored backup) is
    // Private to every gate in the tree and is unassignable through the ratchet.
    // This writer canonicalises it — otherwise the user is shown a private badge
    // above a control that silently does nothing.
    let from = SessionClassification::from_stored(&raw_tier);
    if from == SessionClassification::Public {
        return Ok(DeclassifyOutcome::AlreadyPublic);
    }

    // §12.4's graded confirmation, keyed on the provenance this transaction just
    // read. The client decides which control to SHOW; this decides whether that
    // was the right one, so a caller cannot claim the weak path for a chat that
    // reached a private data source.
    if requires_typed_confirmation(reason_before.as_deref())
        && !confirmation_matches(session_id, confirmation)
    {
        return Ok(DeclassifyOutcome::ConfirmationRequired);
    }

    // DR-20 / Task 55, on the SAME grade and from the SAME provenance. The two
    // proofs answer different questions — the phrase says which chat, the
    // password says who — so both are required and neither substitutes for the
    // other. A `turn:*` chat reaches neither check and keeps its single click.
    //
    // ⚠ The authorisation is checked against the id this transaction is writing,
    // not merely for existence: a batch prompt covers the chats it named and no
    // others (DR-20 point 4), and `is_some_and` fails closed for a caller that
    // presented none.
    if requires_system_authentication(reason_before.as_deref())
        && !authorization.is_some_and(|granted| granted.covers(session_id))
    {
        return Ok(DeclassifyOutcome::SystemAuthenticationRequired);
    }

    let message_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = ?1")
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await?;

    // `actor` and `actor_kind` are both "user" because this daemon has no
    // principal: `check_token` compares one machine-wide bearer, so there is no
    // identity to record (AR-11/AR-15). What the row can say honestly is the
    // KIND of actor, and the kind is the whole point — the route that mints the
    // proof is the only construction site, and it is behind the user-action
    // header. Writing a fabricated username here would be worse than writing
    // none.
    // `session_incarnation` names the ROW declassified, not just its id: the
    // ledger outlives the chat, and the backfill's declassification guard must
    // not read this entry as a later chat's under the same id
    // (`SessionStorage::NOT_DECLASSIFIED_BY_USER`).
    sqlx::query(&format!(
        "INSERT INTO classification_audit ( \
            session_id, from_classification, to_classification, reason, actor, actor_kind, \
            app_version, provider_name_at_change, privacy_reason_before, message_count_at_change, \
            session_incarnation \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, {})",
        crate::session::session_manager::LEDGER_SESSION_INCARNATION
    ))
    .bind(session_id)
    .bind(from.as_sql())
    .bind(SessionClassification::Public.as_sql())
    .bind(DECLASSIFIED_BY_USER)
    .bind("user")
    .bind("user")
    .bind(env!("CARGO_PKG_VERSION"))
    .bind(provider_name)
    .bind(reason_before)
    .bind(message_count)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE sessions \
            SET privacy_tier = 'public', privacy_reason = ?2, updated_at = datetime('now') \
          WHERE id = ?1",
    )
    .bind(session_id)
    .bind(DECLASSIFIED_BY_USER)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    tracing::info!(
        session_id,
        from = from.as_sql(),
        "declassified by the user (issue #56 §12.5)"
    );
    Ok(DeclassifyOutcome::Declassified)
}

/// Declassify through the real writer, for tests that live in **other** modules.
///
/// It exists because the proof-of-user cannot travel. `UserConfirmation` may be
/// *named* only in this file and the two door files —
/// [`the_proof_of_user_is_constructed_in_exactly_two_places`] fails the build for
/// any other file under `crates/` that so much as mentions it — so a test
/// elsewhere in the tree cannot mint the proof for itself. Its only alternative
/// is a hand-rolled `UPDATE`, which is worse twice over: it would duplicate the
/// writer this module exists to keep singular, and because
/// [`exactly_one_statement_in_the_tree_assigns_a_public_classification`] permits
/// exactly one `SET privacy_tier = 'public'` in the whole tree, the copy would
/// have to be composed at runtime to slip past a security audit in order to
/// compile at all.
///
/// `#[cfg(test)]`, so it is absent from every shipped binary.
///
/// Used by `session_manager`'s Task 38 migration tests, which need a genuinely
/// declassified row — tier lowered, `privacy_reason` rewritten, **and
/// `provider_name` deliberately left in place** — to prove the one-time backfill
/// cannot re-privatise it on the next launch.
#[cfg(test)]
pub(crate) async fn declassify_for_test(
    sm: &SessionManager,
    session_id: &str,
) -> Result<DeclassifyOutcome> {
    declassify(
        sm,
        session_id,
        Some(&confirmation_phrase(session_id)),
        Some(&SystemAuthorization::for_test(&[session_id.to_string()])),
        &UserConfirmation::for_test(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelConfig;
    use crate::privacy::system_auth::{AuthOutcome, AuthRequest, NoPrompter, SystemAuthenticator};
    use crate::privacy::SessionClassification;
    use crate::session::session_manager::{SessionManager, SessionType};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One audit row, in the shape §12.5 stores it.
    #[derive(Debug, sqlx::FromRow)]
    struct AuditRow {
        from_classification: String,
        to_classification: String,
        reason: String,
        actor: String,
        actor_kind: String,
        app_version: String,
        provider_name_at_change: Option<String>,
        privacy_reason_before: Option<String>,
        message_count_at_change: Option<i64>,
    }

    /// A private session carrying `reason` as its provenance and `versa_azure`
    /// as its bound provider.
    async fn private_session_with_reason(sm: &SessionManager, reason: &str) -> String {
        let s = sm
            .create_session(
                std::env::temp_dir(),
                "a cohort chat".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(
            &s.id,
            &crate::conversation::message::Message::user().with_text("patient MRN 12345"),
        )
        .await
        .unwrap();
        sm.update(&s.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, reason)
            .apply()
            .await
            .unwrap();
        s.id
    }

    async fn audit_rows(sm: &SessionManager, session_id: &str) -> Vec<AuditRow> {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_as::<_, AuditRow>(
            "SELECT from_classification, to_classification, reason, actor, actor_kind, \
             app_version, provider_name_at_change, privacy_reason_before, \
             message_count_at_change FROM classification_audit WHERE session_id = ?1 \
             ORDER BY id",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn only_a_user_confirmation_can_lower_the_tier() {
        // UserConfirmation is a ZST whose constructor is invoked in exactly two
        // places, both of which are surfaces a human has to act at: the HTTP
        // handler, after it has matched the typed confirmation, and the CLI's
        // `session declassify <id>`, after it has asked at the terminal. No MCP
        // server, no ToolRouter and no workspace_* handler constructs one — the
        // field is private, so the tuple literal is unavailable outside this
        // module, and the named constructor is pinned to those two call sites by
        // [`the_proof_of_user_is_constructed_in_exactly_two_places`].
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = private_session_with_reason(&sm, "mcp:ucsfomopagent").await;

        declassify(
            &sm,
            &id,
            Some(&confirmation_phrase(&id)),
            Some(&SystemAuthorization::for_test(std::slice::from_ref(&id))),
            &UserConfirmation::for_test(),
        )
        .await
        .unwrap();

        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(row.privacy_tier, SessionClassification::Public);
        assert_eq!(row.privacy_reason.as_deref(), Some("declassified_by_user"));
        // A declassified session must never be indistinguishable from one that
        // was always public.
        let rows = audit_rows(&sm, &id).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].actor_kind, "user");
        assert_eq!(rows[0].actor, "user");
        assert_eq!(rows[0].from_classification, "private");
        assert_eq!(rows[0].to_classification, "public");
        assert_eq!(rows[0].reason, "declassified_by_user");
        assert_eq!(rows[0].app_version, env!("CARGO_PKG_VERSION"));
        // The pre-change state, which the row on `sessions` no longer holds: the
        // provenance is overwritten by the declassification itself, so the audit
        // is the ONLY remaining record of what this chat had reached.
        assert_eq!(
            rows[0].privacy_reason_before.as_deref(),
            Some("mcp:ucsfomopagent")
        );
        assert_eq!(
            rows[0].provider_name_at_change.as_deref(),
            Some("versa_azure")
        );
        assert_eq!(rows[0].message_count_at_change, Some(1));

        // The bound provider is left exactly as it was: a public chat may run a
        // private model, and that direction was never restricted.
        assert_eq!(row.provider_name.as_deref(), Some("versa_azure"));
    }

    #[tokio::test]
    async fn declassifying_twice_writes_one_row_and_a_missing_session_is_reported() {
        // A double-click on the confirm button sends two requests. The second
        // must not leave a second ledger entry claiming a private→public
        // transition that never happened.
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = private_session_with_reason(&sm, "turn:versa_azure").await;

        let ok = UserConfirmation::for_test();
        assert_eq!(
            declassify(&sm, &id, None, None, &ok).await.unwrap(),
            DeclassifyOutcome::Declassified
        );
        // The second click carries no confirmation, exactly as the first did.
        // An already-public row is answered BEFORE the grade is consulted —
        // otherwise this arm is unreachable on the single-click path, because
        // the first call rewrote the provenance to `declassified_by_user`, which
        // grades onto the strong control (and, since Task 55, onto the system
        // prompt as well — so a second click would otherwise raise a password
        // dialog for a chat that is already public).
        assert_eq!(
            declassify(&sm, &id, None, None, &ok).await.unwrap(),
            DeclassifyOutcome::AlreadyPublic
        );
        assert_eq!(audit_rows(&sm, &id).await.len(), 1);

        assert_eq!(
            declassify(&sm, "29990101_00000", None, None, &ok)
                .await
                .unwrap(),
            DeclassifyOutcome::SessionNotFound
        );
    }

    #[tokio::test]
    async fn the_grade_is_taken_from_the_provenance_the_writing_transaction_reads() {
        // The check-then-act this closes: the caller reads the row, sees
        // `turn:*`, renders the single-click control, and by the time the write
        // happens the chat has reached an MCP data source. Deciding the grade
        // from an earlier read would declassify it on a control it no longer
        // qualifies for. Here the raise lands between the two, and the answer
        // comes from what the transaction reads, not from what the caller saw.
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = private_session_with_reason(&sm, "turn:versa_azure").await;

        sm.update(&id)
            .raise_privacy(SessionClassification::Private, "mcp:ucsfomopagent")
            .apply()
            .await
            .unwrap();

        let ok = UserConfirmation::for_test();
        let granted = SystemAuthorization::for_test(std::slice::from_ref(&id));
        assert_eq!(
            declassify(&sm, &id, None, Some(&granted), &ok)
                .await
                .unwrap(),
            DeclassifyOutcome::ConfirmationRequired
        );
        // Refused means refused: no ledger row claiming a transition, and the
        // row is untouched.
        assert!(audit_rows(&sm, &id).await.is_empty());
        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(row.privacy_tier, SessionClassification::Private);
        assert_eq!(row.privacy_reason.as_deref(), Some("mcp:ucsfomopagent"));

        // A wrong phrase of the same length is refused too, so this cannot pass
        // merely because something was presented.
        let phrase = confirmation_phrase(&id);
        let wrong: String = phrase.chars().rev().collect();
        if wrong != phrase {
            assert_eq!(
                declassify(&sm, &id, Some(&wrong), Some(&granted), &ok)
                    .await
                    .unwrap(),
                DeclassifyOutcome::ConfirmationRequired
            );
        }

        assert_eq!(
            declassify(&sm, &id, Some(&phrase), Some(&granted), &ok)
                .await
                .unwrap(),
            DeclassifyOutcome::Declassified
        );
        // …and the ledger records the provenance that decided the grade.
        assert_eq!(
            audit_rows(&sm, &id).await[0]
                .privacy_reason_before
                .as_deref(),
            Some("mcp:ucsfomopagent")
        );
    }

    #[test]
    fn the_phrase_is_the_last_six_characters_of_the_id() {
        assert_eq!(confirmation_phrase("abc123def456"), "def456");
        // Shorter than six: the whole id, not a panic.
        assert_eq!(confirmation_phrase("abc"), "abc");
        assert_eq!(confirmation_phrase(""), "");
        // Characters, not bytes. A byte slice would split the last character and
        // panic; ids are ASCII today and this is what keeps that from being a
        // load-bearing assumption.
        assert_eq!(confirmation_phrase("aé漢字漢字漢字"), "漢字漢字漢字");

        assert!(confirmation_matches("abc123def456", Some("def456")));
        assert!(confirmation_matches("abc123def456", Some("  def456 ")));
        assert!(confirmation_matches("abc123DEF456", Some("def456")));
        assert!(!confirmation_matches("abc123def456", Some("ef456")));
        assert!(!confirmation_matches("abc123def456", Some("")));
        assert!(!confirmation_matches("abc123def456", None));
    }

    #[tokio::test]
    async fn a_value_the_reader_refuses_is_repaired_rather_than_treated_as_public() {
        // `from_stored` maps anything that is not exactly `public` to Private, so
        // a row holding `PUBLIC` is private to the whole Rust tree while being
        // unassignable through the ratchet
        // (`a_stored_tier_the_reader_refuses_cannot_be_assigned_away`). This is
        // the one writer that can canonicalise it, and it must — otherwise the
        // user is shown a private badge with a control that silently does
        // nothing.
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = private_session_with_reason(&sm, "turn:versa_azure").await;
        {
            let pool = sm.storage().pool().await.unwrap();
            sqlx::query("UPDATE sessions SET privacy_tier = 'PUBLIC' WHERE id = ?1")
                .bind(&id)
                .execute(pool)
                .await
                .unwrap();
        }
        assert_eq!(
            sm.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Private,
            "the reader fails closed, so this row is Private to every gate"
        );

        assert_eq!(
            declassify(&sm, &id, None, None, &UserConfirmation::for_test())
                .await
                .unwrap(),
            DeclassifyOutcome::Declassified
        );
        assert_eq!(
            sm.get_session(&id, false).await.unwrap().privacy_tier,
            SessionClassification::Public
        );
        // …and the ledger records the tier the READER saw, not the raw bytes,
        // because that is the classification the gates were enforcing.
        assert_eq!(audit_rows(&sm, &id).await[0].from_classification, "private");
    }

    #[test]
    fn the_weak_control_is_granted_only_to_a_chat_that_merely_ran_a_turn() {
        assert!(!requires_typed_confirmation(Some("turn:versa_azure")));
        assert!(requires_typed_confirmation(Some("mcp:ucsfomopagent")));
        // The cases a match-on-`mcp:` rule hands the weak control to. A branch
        // of an OMOP session is the one that matters: it carries the SAME
        // conversation and none of the `mcp:` spelling.
        assert!(requires_typed_confirmation(Some(
            "diverged:20260101_120000"
        )));
        assert!(requires_typed_confirmation(Some(
            "inherited:20260101_120000"
        )));
        assert!(requires_typed_confirmation(Some("imported")));
        assert!(requires_typed_confirmation(Some("backfill:versa_azure")));
        // Absent, and anything a future task adds to the vocabulary.
        assert!(requires_typed_confirmation(None));
        assert!(requires_typed_confirmation(Some("")));
        assert!(requires_typed_confirmation(Some("something_new")));
        // Not a prefix match on a longer word: `turned:` is not `turn:`.
        assert!(requires_typed_confirmation(Some("turned_private")));
    }

    /// The whole §12.4 vocabulary, plus the values that are not in it. Every
    /// case that owes the phrase is listed with the EXACT clause it is owed, so
    /// a wording that regresses to one sentence for all of them — which is what
    /// shipped, and what this fixes — fails here rather than in front of a user.
    #[test]
    fn every_provenance_on_the_strong_control_is_told_its_own_reason() {
        let cases: [(Option<&str>, &str); 10] = [
            (Some("mcp:ucsfomopagent"), "reached a private data source"),
            (
                Some("inherited:20260101_120000"),
                "was created inside a private chat",
            ),
            (
                Some("diverged:20260101_120000"),
                "was branched out of a private chat",
            ),
            (
                Some("backfill:versa_azure"),
                "was marked private by the one-time migration, from the model it was last using \
                 rather than from anything it reached",
            ),
            (Some("imported"), "was imported already marked private"),
            // Not in the vocabulary, and the record is all that can be claimed.
            (
                Some("declassified_by_user"),
                "does not record an observed turn on a private model as the reason it is private",
            ),
            (
                Some("something_new"),
                "does not record an observed turn on a private model as the reason it is private",
            ),
            (
                Some(""),
                "does not record an observed turn on a private model as the reason it is private",
            ),
            (
                None,
                "does not record an observed turn on a private model as the reason it is private",
            ),
            // `turned:` is not `turn:`, so this owes the phrase like any other
            // unrecognised value — and must not be told it ran a turn.
            (
                Some("turned_private"),
                "does not record an observed turn on a private model as the reason it is private",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(
                strong_confirmation_reason(reason),
                Some(expected),
                "the sentence shown for {reason:?}"
            );
        }

        // The specific falsehood this exists to stop. `backfill:*` is the
        // majority of the private rows on a machine with history, and it reached
        // nothing: the migration read the provider the row was last bound to.
        for reason in [
            Some("backfill:versa_azure"),
            Some("imported"),
            Some("inherited:20260101_120000"),
            Some("diverged:20260101_120000"),
            Some("declassified_by_user"),
            Some("something_new"),
            Some(""),
            None,
        ] {
            let said = strong_confirmation_reason(reason).unwrap();
            assert!(
                !said.contains("reached a private data source"),
                "a {reason:?} chat is told it reached a private data source, which it did not: \
                 {said}"
            );
        }

        // …and the one that did keeps the sentence that is true of it.
        assert_eq!(
            strong_confirmation_reason(Some("mcp:ucsfomopagent")),
            Some("reached a private data source")
        );
    }

    /// The wording and the grade are one decision, so the strong copy is
    /// unreachable for a chat that gets the single click. Without this a later
    /// edit could hand the weak path a sentence about a data source — the same
    /// class of bug from the other side.
    #[test]
    fn a_reason_exists_for_exactly_the_chats_that_owe_the_phrase() {
        for reason in [
            Some("turn:versa_azure"),
            Some("turn:ollama"),
            Some("mcp:ucsfomopagent"),
            Some("diverged:20260101_120000"),
            Some("inherited:20260101_120000"),
            Some("imported"),
            Some("backfill:versa_azure"),
            Some("declassified_by_user"),
            Some("turned_private"),
            Some("something_new"),
            Some(""),
            None,
        ] {
            assert_eq!(
                strong_confirmation_reason(reason).is_some(),
                requires_typed_confirmation(reason),
                "the wording and the grade disagree about {reason:?}"
            );
        }
    }

    /// The renderer says the same thing the daemon and the CLI do, and this is
    /// the only mechanical check of that: the three live in different languages,
    /// so nothing else makes a TypeScript edit that drifts from the Rust fail.
    ///
    /// It asserts every clause appears VERBATIM in the dialog's source. Prettier
    /// does not split string literals, so a clause that survives formatting is
    /// one contiguous match; a reworded Rust clause, or a renderer that goes back
    /// to one sentence for every provenance, leaves a clause with no hit here.
    #[test]
    fn the_desktop_dialog_carries_the_same_clauses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let dialog = root.join("ui/desktop/src/components/sessions/DeclassifySessionDialog.tsx");
        assert!(
            dialog.is_file(),
            "the audit reads {}; if that path is wrong the assertions below prove nothing",
            dialog.display()
        );
        let src = std::fs::read_to_string(&dialog).unwrap();

        for reason in [
            Some("mcp:ucsfomopagent"),
            Some("inherited:20260101_120000"),
            Some("diverged:20260101_120000"),
            Some("backfill:versa_azure"),
            Some("imported"),
            None,
        ] {
            let clause = strong_confirmation_reason(reason).unwrap();
            assert!(
                src.contains(clause),
                "the desktop dialog does not say {clause:?} for a {reason:?} chat. The Rust and \
                 the TypeScript wordings have drifted."
            );
        }
    }

    // ------------------------------------------------------------------
    // Task 55 / DR-20: the operating system's own authentication
    // ------------------------------------------------------------------

    /// A prompter that approves and **counts**, so DR-20's "one prompt for the
    /// batch" is an assertion rather than a hope.
    #[derive(Default)]
    struct CountingPrompter {
        prompts: AtomicUsize,
        named: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl SystemAuthenticator for CountingPrompter {
        async fn authenticate(&self, req: &AuthRequest) -> AuthOutcome {
            self.prompts.fetch_add(1, Ordering::Relaxed);
            *self.named.lock().unwrap() = req.session_ids.clone();
            AuthOutcome::Approved
        }

        fn platform(&self) -> &'static str {
            "counting test prompter"
        }
    }

    /// The user pressed Cancel.
    struct AlwaysDenies;

    #[async_trait::async_trait]
    impl SystemAuthenticator for AlwaysDenies {
        async fn authenticate(&self, _req: &AuthRequest) -> AuthOutcome {
            AuthOutcome::Denied
        }

        fn platform(&self) -> &'static str {
            "denying test prompter"
        }
    }

    /// DR-20, Task 55 Step 1. The chats §12.4 grades onto the typed phrase now
    /// need the operating system's authentication **as well**, and the two
    /// proofs answer different questions: the phrase proves *which* chat, the
    /// password proves *who*.
    #[tokio::test]
    async fn a_chat_that_reached_a_private_data_source_needs_the_password_as_well_as_the_phrase() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = private_session_with_reason(&sm, "mcp:ucsfomopagent").await;
        let ok = UserConfirmation::for_test();

        // The right phrase, and no system authentication: refused.
        assert_eq!(
            declassify(&sm, &id, Some(&confirmation_phrase(&id)), None, &ok)
                .await
                .unwrap(),
            DeclassifyOutcome::SystemAuthenticationRequired,
            "the typed phrase alone declassified a chat that reached a private data source"
        );
        // Refused means refused: nothing written, nothing claimed.
        assert!(audit_rows(&sm, &id).await.is_empty());
        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(row.privacy_tier, SessionClassification::Private);
        assert_eq!(row.privacy_reason.as_deref(), Some("mcp:ucsfomopagent"));

        // An authentication that named a DIFFERENT chat is not spendable here —
        // DR-20 point 4: one prompt may cover several chats, but only the ones
        // it named.
        let elsewhere = SystemAuthorization::for_test(&["20990101_000000".to_string()]);
        assert_eq!(
            declassify(
                &sm,
                &id,
                Some(&confirmation_phrase(&id)),
                Some(&elsewhere),
                &ok
            )
            .await
            .unwrap(),
            DeclassifyOutcome::SystemAuthenticationRequired
        );
        assert!(audit_rows(&sm, &id).await.is_empty());

        // Both proofs, and only then.
        let granted = SystemAuthorization::for_test(std::slice::from_ref(&id));
        assert_eq!(
            declassify(
                &sm,
                &id,
                Some(&confirmation_phrase(&id)),
                Some(&granted),
                &ok
            )
            .await
            .unwrap(),
            DeclassifyOutcome::Declassified
        );
        assert_eq!(audit_rows(&sm, &id).await.len(), 1);
    }

    /// Task 55 Step 1's second half, and it is a rule about the COMMON case:
    /// a chat that merely ran a turn against a private endpoint keeps its single
    /// click and is never asked for a password. Making the common case expensive
    /// is how you teach people to stop privatising at all.
    #[tokio::test]
    async fn a_turn_only_chat_keeps_its_single_click_and_is_never_asked_for_a_password() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = private_session_with_reason(&sm, "turn:versa_azure").await;

        assert_eq!(
            declassify(&sm, &id, None, None, &UserConfirmation::for_test())
                .await
                .unwrap(),
            DeclassifyOutcome::Declassified,
            "the weak control now demands a system authentication it never shows the user"
        );
    }

    /// The two proofs are graded by ONE predicate, so they cannot drift into
    /// disagreeing about which chats are protected.
    #[test]
    fn the_grade_that_demands_a_phrase_is_the_grade_that_demands_the_password() {
        for reason in [
            Some("turn:versa_azure"),
            Some("mcp:ucsfomopagent"),
            Some("diverged:20260101_120000"),
            Some("inherited:20260101_120000"),
            Some("imported"),
            Some("backfill:versa_azure"),
            Some("declassified_by_user"),
            Some("something_new"),
            Some(""),
            None,
        ] {
            assert_eq!(
                requires_system_authentication(reason),
                requires_typed_confirmation(reason),
                "the two §12.4 controls disagree about {reason:?}"
            );
        }
    }

    /// Task 55 Step 3. DR-20 says a declassification may cover many chats, and
    /// that it costs **one** prompt — not one per chat.
    #[tokio::test]
    async fn one_prompt_covers_a_batch_and_covers_only_the_chats_it_named() {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let mut ids = vec![];
        for _ in 0..3 {
            ids.push(private_session_with_reason(&sm, "mcp:ucsfomopagent").await);
        }

        let prompter = CountingPrompter::default();
        // The same id twice, and out of order: the prompt names a canonical set,
        // so the sentence cannot claim to cover four chats when it covers three.
        let asked: Vec<String> = ids.iter().rev().cloned().chain([ids[0].clone()]).collect();
        let granted = authorize_with(&prompter, &asked)
            .await
            .expect("an approving prompter yields the authorisation");
        assert_eq!(
            prompter.prompts.load(Ordering::Relaxed),
            1,
            "a batch of three chats raised more than one prompt"
        );
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            *prompter.named.lock().unwrap(),
            sorted,
            "the prompt must name exactly the chats it authorises (DR-20 point 4)"
        );

        // …and the one authorisation is spendable on every chat it named.
        let ok = UserConfirmation::for_test();
        for id in &ids {
            assert_eq!(
                declassify(&sm, id, Some(&confirmation_phrase(id)), Some(&granted), &ok)
                    .await
                    .unwrap(),
                DeclassifyOutcome::Declassified,
                "{id} was not covered by the batch's single prompt"
            );
        }
        // …and on nothing else.
        assert!(!granted.covers("20990101_000000"));
    }

    /// Task 55 Step 4. A refusal from the prompter leaves the chat private and
    /// writes no audit row — and `Unavailable`, the state of every platform with
    /// no prompter, refuses rather than proceeding.
    ///
    /// [`NoPrompter`] is the real fallback this build ships for a target DR-24
    /// does not name, so this exercises the shipped fail-closed path rather than
    /// a mock of it.
    #[tokio::test]
    async fn a_refused_or_unavailable_prompt_never_yields_an_authorisation() {
        let ids = ["20260804_120000".to_string()];

        let denied = authorize_with(&AlwaysDenies, &ids)
            .await
            .expect_err("a denied prompt must not yield an authorisation");
        assert_eq!(denied.outcome, AuthOutcome::Denied);
        assert!(!denied.message.is_empty());

        let unavailable = authorize_with(&NoPrompter, &ids)
            .await
            .expect_err("a platform with no prompter must refuse, not proceed");
        assert_eq!(unavailable.outcome, AuthOutcome::Unavailable);
        assert!(
            unavailable.message.contains(NoPrompter.platform()),
            "an Unavailable refusal must name the platform: {}",
            unavailable.message
        );

        // An empty set names nothing, so the prompt could not say what it
        // authorises and the proof would be spendable on nothing. Refused rather
        // than accepted-and-ignored.
        assert!(authorize_with(&CountingPrompter::default(), &[])
            .await
            .is_err());
    }

    /// The whole audit surface for "can the ratchet be reversed".
    ///
    /// The plan's shell gate greps for any comparison of `privacy_tier` against
    /// the public literal and expects one hit. That spelling is not the property:
    /// this tree already contained FOUR such occurrences before a line of this
    /// task existed — Gate A's `WHERE` clause in `bind_provider_if_allowed` and
    /// chat recall's two visibility filters, all of them READS — so a count over
    /// them says nothing about writes and the gate's stated expectation cannot
    /// hold. What is pinned here instead is the set of files containing an
    /// ASSIGNMENT (a SQL `SET` of that column to that literal), which must be
    /// exactly this one.
    ///
    /// The needle is composed at runtime rather than written out, so this file
    /// does not match its own audit — see below.
    ///
    /// ⚠ **This is a tripwire, not a proof, and the difference matters.** It
    /// matches one SPELLING of the assignment. `SET privacy_tier=?1`, a bind
    /// parameter, a builder that emits the column name from a variable — none of
    /// them are seen by it. What it reliably catches is the realistic case: a
    /// second hand-written bypass, added by someone who copied this one. The
    /// property that actually holds is structural and lives elsewhere —
    /// `SessionUpdateBuilder`'s emission is a monotone `CASE WHEN` that cannot
    /// express a lowering whatever the caller passes, so every write that is not
    /// this function is incapable of it by construction.
    #[test]
    fn exactly_one_statement_in_the_tree_assigns_a_public_classification() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let crates = root.join("crates");
        assert!(
            crates.is_dir(),
            "the audit walks {}; if that path is wrong every assertion below passes \
             for the wrong reason",
            crates.display()
        );

        // Composed rather than written out, so this file does not match its own
        // audit. Spelling the needle literally would give `declassify.rs` two
        // hits — the statement and the scanner — and "2" is then indistinguishable
        // from a real second bypass landing here.
        let needle = format!("SET privacy_tier = '{}'", SessionClassification::PUBLIC_SQL);
        let mut assignments: std::collections::BTreeMap<String, usize> = Default::default();
        let mut scanned = 0usize;
        for entry in walkdir::WalkDir::new(&crates) {
            let entry = entry.expect("the audit must not silently skip an unreadable directory");
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            scanned += 1;
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
            for line in src.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                let hits = code.matches(needle.as_str()).count();
                if hits > 0 {
                    *assignments.entry(rel.clone()).or_default() += hits;
                }
            }
        }
        assert!(
            scanned >= 400,
            "only {scanned} .rs files were scanned. A broken walk reports the same empty \
             set as a clean tree."
        );
        let found: Vec<(String, usize)> = assignments.into_iter().collect();
        assert_eq!(
            found,
            vec![("crates/biorouter/src/privacy/declassify.rs".to_string(), 1)],
            "the set of statements that lower a session's classification changed. Every \
             other write goes through `SessionUpdateBuilder`, whose monotone `CASE WHEN` \
             physically cannot lower the tier; a second bypass is a design change."
        );
    }

    /// The proof-of-user is a cross-crate `pub` constructor, because its callers
    /// live in `biorouter-server` and `biorouter-cli` and `pub(in …)` cannot
    /// cross a crate boundary. Rust therefore cannot restrict it to a fixed set
    /// of call sites on its own — this test is what does.
    ///
    /// It pins two things, and it takes both. The set of FILES that so much as
    /// name the type must be exactly the two below; and within them the number
    /// of times the constructor is actually CALLED must be exactly one each. The
    /// file set alone is not enough: a second handler added to one of those files
    /// could mint the proof with no user check and this audit would not notice,
    /// because the file is already a permitted member.
    /// `auth::tests::the_declassify_route_consults_the_user_action_guard` closes
    /// the loop from the other side for the HTTP door, asserting that its one
    /// call sits inside the body of the handler that consults the guard.
    ///
    /// ⚠ **The set grew to two at Task 31, and the claim it supports shrank.**
    /// Until then this test could be read as "the only way to lower a
    /// classification is an HTTP route behind the user-action header". R10 makes
    /// the CLI a required surface — `list_sessions` filters to (`user`,
    /// `scheduled`), so a private `Hidden`/`SubAgent`/`Terminal` chat has no GUI
    /// declassification control at all — and `biorouter session declassify <id>`
    /// is that surface. Its proof-of-user is a confirmation typed at a terminal
    /// rather than a header, and the honest residual is written down at
    /// `commands/session.rs`'s `declassify_command`: an agent holding
    /// `developer__shell` can drive the CLI, and that same agent can already
    /// write the column directly with `sqlite3`, so the store was never
    /// protected from the shell. What both doors still share is the ledger row.
    ///
    /// Both assertions fail closed. They are exact equalities against a computed
    /// map, so a needle that stopped matching — a renamed constructor, a
    /// reformatted call — yields an empty map and a red build, never a silent
    /// pass.
    #[test]
    fn the_proof_of_user_is_constructed_in_exactly_two_places() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let mut naming: Vec<String> = vec![];
        let mut constructions: std::collections::BTreeMap<String, usize> = Default::default();
        let mut scanned = 0usize;
        for entry in walkdir::WalkDir::new(root.join("crates")) {
            let entry = entry.expect("the audit must not silently skip an unreadable directory");
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            scanned += 1;
            // This file declares the constructor and names it throughout its own
            // prose; counting it would make every number below one larger and
            // indistinguishable from a real second site.
            if rel == "crates/biorouter/src/privacy/declassify.rs" {
                continue;
            }
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
            if src.contains("UserConfirmation") {
                naming.push(rel.clone());
            }
            for line in src.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                let hits = code
                    .matches("UserConfirmation::from_typed_confirmation(")
                    .count();
                if hits > 0 {
                    *constructions.entry(rel.clone()).or_default() += hits;
                }
            }
        }
        assert!(scanned >= 400, "only {scanned} .rs files were scanned");
        naming.sort();
        assert_eq!(
            naming,
            vec![
                // Task 31: `biorouter session declassify <id>`, behind a
                // confirmation typed at the terminal.
                "crates/biorouter-cli/src/commands/session.rs".to_string(),
                // `POST /sessions/{id}/declassify`, behind the user-action header.
                "crates/biorouter-server/src/routes/session.rs".to_string(),
            ],
            "a third file naming the proof-of-user appeared. The claim that a model cannot \
             declassify a chat rests on this set staying closed, and on every member being a \
             surface a human has to act at."
        );
        let called: Vec<(String, usize)> = constructions.into_iter().collect();
        assert_eq!(
            called,
            vec![
                (
                    "crates/biorouter-cli/src/commands/session.rs".to_string(),
                    1
                ),
                (
                    "crates/biorouter-server/src/routes/session.rs".to_string(),
                    1
                )
            ],
            "the proof-of-user is minted somewhere new, or twice in a file that is already a \
             member. Each extra construction site is another way to lower a classification, and \
             only these two are known to sit behind a human's confirmation."
        );
    }
}
