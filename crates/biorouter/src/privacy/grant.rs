//! Cross-affiliation grants (issue #56, DR-26 / Task 49) — the user's explicit,
//! once-per-triple acceptance of one cross-institutional data flow.
//!
//! [`super::affiliation`] decides that a flow crosses an institutional boundary
//! and composes the words it is stated in. It deliberately decides nothing about
//! what happens next, because DR-26's answer to a mismatch is neither "block" nor
//! "allow": it is **warn, and let the user accept the stated risk**. A
//! blocked-outright design is one researchers route around by turning the feature
//! off, and legitimate cross-institutional work under a real DUA exists; a
//! silently-allowed design is not a control. This module is the acceptance.
//!
//! # The shape of the flow, and why it cannot be any other shape
//!
//! ⚠ **A Gate C refusal cannot escalate to a user prompt in-process.** The
//! reusable proof-of-user in this tree is the `X-User-Action` header
//! (`biorouter_server::auth::user_action_proof`), and a tool call has no HTTP
//! request to carry one — recorded twice already, on
//! [`super::PrivacyRefusal::PublicChildOfPrivateParent`] and in
//! `subagent_tool.rs`. So the sequence is forced:
//!
//! 1. dispatch refuses, and the refusal tells the model to ask the human
//!    ([`super::refusal::cross_affiliation_refusal`]);
//! 2. the user grants over `POST /agent/cross_affiliation_grant`, which carries
//!    the header;
//! 3. the next call finds the grant and proceeds.
//!
//! That is DR-19 working as intended: the agent may ask, and only the user may
//! answer.
//!
//! # The scope, and the three ways it is deliberately narrow
//!
//! A grant is keyed on the **triple** (session, extension, model affiliation).
//!
//!  * **Per session, never machine-wide.** The risk statement was about *this*
//!    conversation's data flow. A grant given in one chat must not silently cover
//!    another.
//!  * **Per extension.** A user who accepted one connector's disclosure accepted
//!    that connector, not a category.
//!  * **Per model affiliation.** Re-binding to a different institution's model
//!    changes the triple, so the approved flow no longer exists and the grant
//!    does not match. This is the axis an implementer is most likely to drop,
//!    and dropping it converts a one-time acceptance into a standing permission
//!    that survives a model switch the user never reviewed.
//!
//! It is *not* per turn: a control that fires constantly is one people click
//! through, which is the prompt fatigue DR-19 warns about.
//!
//! ⚠ **What the triple deliberately does NOT pin: the extension's own
//! affiliation.** DR-26 specifies these three components and this implements
//! exactly them, but the consequence should be on someone's list. The extension
//! half is a *name key*, so if the institution behind that name changes — the
//! entry reconfigured, or a different server installed under the same key — the
//! stored grant still matches and Gate C permits a flow whose owning institution
//! the user was never shown. It is not closable here: DR-26's fact 4 records
//! that no stable extension id exists in this tree, so there is nothing finer
//! than the name to key on. The mitigation is Task 47's provenance (or adding
//! the extension's affiliation to the key once provenance can supply it), and it
//! belongs there, at the parse that admits the identity, not at this store.
//!
//! # A subagent inherits and can never exceed
//!
//! [`is_granted`] walks `parent_session_id`, so a child acts within authority its
//! parent already holds. It cannot create one: the proof-of-user has exactly one
//! construction site, in the HTTP handler, and no tool-facing surface can reach
//! it — pinned by [`tests::the_proof_of_user_is_constructed_in_exactly_one_place`].

use anyhow::Result;

use super::affiliation::ModelAffiliation;
use crate::session::SessionManager;

/// Proof that a human asked for this specific cross-institutional flow. A ZST
/// with a **private field**, so the tuple literal `UserCrossAffiliationGrant(())`
/// is unavailable outside this module and the named constructor below is the only
/// door.
///
/// It is the third proof-of-user in this feature and it is a **separate type on
/// purpose**, for the reason `knowledge::tier_user::UserKbTierChange` is separate
/// from `privacy::declassify`'s: one proof must not be spendable on another
/// subject. Declassifying a chat, publicizing a knowledge base and accepting a
/// cross-institutional flow are three different risks, stated in three different
/// sentences, and a token minted for one must not settle another.
///
/// ⚠ **The declassification proof could not be reused here even if that were
/// desirable.** It is capped at exactly two construction sites by a repo walk
/// (`declassify.rs`'s own audit); a third would fail a passing test, and relaxing
/// that audit is the wrong repair.
///
/// ⚠ **What Rust enforces here, stated precisely.** The single caller lives in
/// `biorouter-server`, a different crate, and `pub(in path)` does not cross a
/// crate boundary — so the constructor is `pub`, and the language guarantees only
/// that a caller cannot fabricate the proof by writing the struct literal. What
/// caps the number of call sites at one is
/// [`tests::the_proof_of_user_is_constructed_in_exactly_one_place`], a repo walk
/// asserting the set of files that name this type is exactly
/// `{routes/agent.rs}` and that it constructs one exactly once. An MCP server, a
/// `#[tool]` handler or a CLI subcommand reaching for this would have to name the
/// type, and the build turns red.
pub struct UserCrossAffiliationGrant(());

impl UserCrossAffiliationGrant {
    /// Mint the proof. Call this **only** after `auth::user_action_proof` has
    /// returned `Proven`; see the handler, which is the sole call site.
    pub fn from_user_action() -> Self {
        Self(())
    }
}

/// What accepting a cross-affiliation warning actually commits the user to.
///
/// ONE constant, in the spirit of [`super::disclosure::COPY_LONG`]: the sentence
/// exists in the refusal the model reads, in the dialog the user clicks and in
/// the audit line, and four hand-written copies drift within one release.
///
/// ⚠ **It states the scope, not just the risk.** DR-26 requires a warning
/// specific enough to act on, and "how far does my yes reach" is part of what is
/// being decided: a user who believes they are approving one call behaves
/// differently from one who knows they are approving this connector for the rest
/// of the chat. It is also the sentence that makes the *narrowness* legible —
/// this chat only, this connector only, this institution's model only.
pub const GRANT_SCOPE_COPY: &str =
    "Approving records your acceptance for this chat, this extension and this model's institution \
     only. It is not remembered for other chats, for other extensions, or if you switch this chat \
     to a model covered by a different institution's agreements. Each of those is a different \
     data flow and would be asked again.";

/// The mismatch plus what a yes covers, as one sentence pair.
///
/// The ONE composition of those two halves in the tree, so the dialog that asks,
/// the response that confirms and any audit line that records it cannot differ by
/// a word. It takes the warning already composed rather than recomposing it,
/// because the surfaces that hold one got it from the gate that decided — and
/// re-deriving it there would be a second decision about whether there is a
/// mismatch at all.
///
/// ⚠ **This module deliberately has no `model → warning → statement` composer,
/// and one was deleted to get here.** `grant_prompt` took the three model axes
/// and re-implemented `gate_cross_affiliation`'s `Some(model)`/`None` branch
/// without its three guards, which is the second implementation of DR-26's table
/// that `affiliation::compatible` and `CallCapability::cross_affiliation_warning`
/// both warn against — at test scale, since production never called it. The
/// production path is the gate, then this: whatever decided there IS a mismatch
/// already holds the sentence, and Task 49's gate (3) drives that path rather
/// than a parallel one.
pub fn accepted_statement(warning: &str) -> String {
    format!("{warning} {GRANT_SCOPE_COPY}")
}

/// The stored spelling of the triple's third component.
///
/// A stable string rather than a serde encoding of [`ModelAffiliation`], because
/// this value is a **database key**: a row written by one build must still match
/// a lookup from the next, and an enum representation is free to change when a
/// variant is added. The three spellings are disjoint by construction —
/// `institution:` cannot collide with either bare word, and an institution id is
/// `name_to_key`-normalised, so `UCSF` and `ucsf` produce the same key.
///
/// ⚠ **`unstated` is a real key, not a missing one.** A private model that states
/// no affiliation mismatches every claimed extension (see
/// `affiliation::unstated_model`), so the user can be asked about it and must
/// therefore be able to answer. Folding it onto `local` would make a grant given
/// for an unknown endpoint silently cover a local model, which is backwards: it
/// is the *less* trusted of the two.
///
/// ⚠ **A model covered by two or more institutions gets its own prefix, and the
/// single-institution key is untouched.** A grant is approval of *one* flow, so
/// the key must distinguish "covered by ucsf" from "covered by ucsf and
/// stanford" — those are different disclosures, and the second must not inherit
/// the first's approval. `institutions:` with a JSON array is used rather than a
/// joined string because an institution id is only `name_to_key`-normalised
/// (lowercase, whitespace stripped) and may therefore contain any separator
/// character: `{"a+b"}` and `{"a", "b"}` would collide under `+`, which would
/// make one user's approval silently cover a flow they never saw. Keeping
/// `institution:<id>` byte-identical for the singleton is what stops every grant
/// already in a user's database from ceasing to match.
fn model_key(model: Option<ModelAffiliation>) -> String {
    match model {
        Some(ModelAffiliation::Local) => "local".to_string(),
        Some(ModelAffiliation::Institutions(set)) => match set.sole() {
            Some(id) => format!("institution:{id}"),
            None => {
                let ids: Vec<&str> = set.iter().map(|id| id.as_str()).collect();
                let encoded = serde_json::to_string(&ids)
                    .expect("a Vec<&str> always serialises to JSON, so this cannot fail");
                format!("institutions:{encoded}")
            }
        },
        None => "unstated".to_string(),
    }
}

/// The stored spelling of the extension half.
///
/// ⚠ **The normaliser is [`crate::config::extensions::name_to_key`], reused
/// rather than re-derived** — the same discipline
/// [`super::affiliation::InstitutionId::new`] follows. The grant is written from
/// an HTTP request naming an extension however the user's UI spells it
/// (`CDWAgent`) and read at dispatch from the resolved client name (`cdwagent`);
/// two normalisers would make a grant that is recorded and never found.
fn extension_key(extension: &str) -> String {
    crate::config::extensions::name_to_key(extension)
}

/// Whether a stored grant belongs to the chat that holds its session id now:
/// that chat already existed when the grant was recorded. A predicate over
/// `cross_affiliation_grants g JOIN sessions s ON s.id = g.session_id`.
///
/// ⚠ **The id is not the chat.** `create_session` minted `<day>_<MAX(N)+1>`, so
/// deleting the newest chat of the day handed its id to the next one, and until
/// `delete_session` took a chat's grants with it that next chat read every flow
/// accepted in the deleted one as accepted in itself — the one thing
/// [`GRANT_SCOPE_COPY`] tells the user cannot happen. The delete is not the
/// only way a grant outlives its chat: every earlier build left them behind, a
/// terminal `biorouter` lagging the desktop app still does while it shares the
/// database, and a restored backup can hold grants for an id that is live
/// again. None of those can be recorded after the chat now holding the id was
/// created, so this test needs no cooperation from any of them.
///
/// Both timestamps come from SQLite's clock (`datetime('now')` and
/// `CURRENT_TIMESTAMP`, or an imported chat's RFC 3339 `created_at`, which
/// `datetime()` reads the same way), at one-second resolution. A grant from the
/// chat's first second still counts — a user cannot be shown a refusal and
/// accept it inside the second the chat was created in, and a stale grant would
/// need a user to accept, delete the chat and start another within one. A row
/// either side cannot read fails closed: the flow is asked about again.
///
/// One definition, read by [`is_granted`] and by the startup sweep in
/// `session_manager` that deletes what it rejects, so the two can never
/// disagree about which grants are live.
pub(crate) const GRANT_IS_THE_CHATS_OWN: &str = "datetime(g.granted_at) >= datetime(s.created_at)";

/// How deep the parent walk goes before giving up.
///
/// A subagent may itself spawn a subagent, so the chain is genuinely longer than
/// one — but `parent_session_id` is an unconstrained TEXT column with no
/// referential integrity, so a hand-edited or restored database can contain a
/// cycle. A bound turns that into "no grant found", which is the fail-closed
/// answer, instead of a hung dispatch.
const MAX_PARENT_DEPTH: usize = 16;

/// Record the user's acceptance of one cross-institutional flow.
///
/// Idempotent: re-approving the same triple refreshes the timestamp rather than
/// erroring, because the user did ask again and the record should say when.
///
/// The row is written with the app version, in the shape `classification_audit`
/// stores a declassification: "when was this accepted, and by which build" is the
/// question anyone auditing a cross-institutional disclosure will ask, and a bare
/// flag cannot answer it.
pub async fn record(
    sm: &SessionManager,
    session_id: &str,
    extension: &str,
    model: Option<ModelAffiliation>,
    _ok: &UserCrossAffiliationGrant,
) -> Result<()> {
    let pool = sm.storage().pool().await?;
    let key = model_key(model);
    let ext = extension_key(extension);
    sqlx::query(
        "INSERT INTO cross_affiliation_grants \
            (session_id, extension, model_affiliation, granted_at, app_version) \
         VALUES (?1, ?2, ?3, datetime('now'), ?4) \
         ON CONFLICT(session_id, extension, model_affiliation) \
         DO UPDATE SET granted_at = datetime('now'), app_version = excluded.app_version",
    )
    .bind(session_id)
    .bind(&ext)
    .bind(&key)
    .bind(env!("CARGO_PKG_VERSION"))
    .execute(pool)
    .await?;
    tracing::info!(
        session_id,
        extension = ext,
        model_affiliation = key,
        "cross-institutional flow accepted by the user (issue #56 DR-26)"
    );
    Ok(())
}

/// Has the user accepted this exact triple, in this session or in an ancestor of
/// it?
///
/// ⚠ **Fail-closed, and it returns a bare `bool` to make that unavoidable.** An
/// unreadable database, a missing table or a query error reads *not granted* — a
/// refusal the user can clear by approving again, rather than a permitted
/// cross-institutional disclosure nobody accepted. A `Result` here would let a
/// caller write `.unwrap_or(true)`, and the one plausible reason to reach for
/// that (a gate that must not fail a call over a database hiccup) is exactly the
/// reasoning this control cannot accept.
///
/// The ancestor walk is what makes "a subagent inherits its parent's grants"
/// true. It reads upward only: a child's own grants are invisible to its parent,
/// and a child has no way to create one regardless.
///
/// Only a grant recorded while the chat holding its id existed is read — see
/// [`GRANT_IS_THE_CHATS_OWN`]. A grant whose chat is gone is not read either,
/// because the lookup joins the chat's row.
pub async fn is_granted(
    sm: &SessionManager,
    session_id: &str,
    extension: &str,
    model: Option<ModelAffiliation>,
) -> bool {
    match granted_inner(sm, session_id, extension, model).await {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(
                session_id,
                %error,
                "could not read cross-affiliation grants; treating the flow as ungranted"
            );
            false
        }
    }
}

async fn granted_inner(
    sm: &SessionManager,
    session_id: &str,
    extension: &str,
    model: Option<ModelAffiliation>,
) -> Result<bool> {
    let pool = sm.storage().pool().await?;
    let key = model_key(model);
    let ext = extension_key(extension);

    let lookup = format!(
        "SELECT COUNT(*) FROM cross_affiliation_grants g \
           JOIN sessions s ON s.id = g.session_id \
          WHERE g.session_id = ?1 AND g.extension = ?2 AND g.model_affiliation = ?3 \
            AND {GRANT_IS_THE_CHATS_OWN}"
    );
    let mut current = session_id.to_string();
    for _ in 0..MAX_PARENT_DEPTH {
        let found: i64 = sqlx::query_scalar(&lookup)
            .bind(&current)
            .bind(&ext)
            .bind(&key)
            .fetch_one(pool)
            .await?;
        if found > 0 {
            return Ok(true);
        }
        let parent: Option<String> =
            sqlx::query_scalar("SELECT parent_session_id FROM sessions WHERE id = ?1")
                .bind(&current)
                .fetch_optional(pool)
                .await?
                .flatten();
        match parent {
            Some(next) if next != current => current = next,
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// Record a grant through the real writer, for tests that live in **other**
/// modules.
///
/// It exists because the proof-of-user cannot travel: [`tests::the_proof_of_user_is_constructed_in_exactly_one_place`]
/// fails the build for any file under `crates/` outside this one and the HTTP
/// handler that so much as mentions the type, so a test elsewhere in the tree
/// cannot mint one. Its only alternative is a hand-rolled `INSERT`, which would
/// duplicate the writer this module exists to keep singular.
///
/// ⚠ **It is the one hole in "a grant can only be written by something that
/// names the proof", and the hole is deliberate.** Any in-crate test can call
/// this without naming [`UserCrossAffiliationGrant`], so the repo-walk audit
/// does not see it. What keeps that from mattering is `#[cfg(test)]`: it is
/// absent from every shipped binary, so no model-reachable path can call it at
/// run time. If it ever loses that attribute, the audit above stops meaning what
/// it says.
#[cfg(test)]
pub(crate) async fn record_for_test(
    sm: &SessionManager,
    session_id: &str,
    extension: &str,
    model: Option<ModelAffiliation>,
) -> Result<()> {
    record(
        sm,
        session_id,
        extension,
        model,
        &UserCrossAffiliationGrant(()),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::affiliation::{ExtensionAffiliation, InstitutionId};
    use crate::session::session_manager::SessionType;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn ucsf_owned() -> ExtensionAffiliation {
        ExtensionAffiliation::institution(InstitutionId::new("ucsf"))
    }

    fn bound_to(name: &str) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::institution(InstitutionId::new(name)))
    }

    /// The statement a user decides on, assembled the way **production**
    /// assembles it: the gate decides, and [`accepted_statement`] adds the
    /// scope.
    ///
    /// ⚠ **It composes production's pieces rather than standing in for them.**
    /// The route reaches the same two functions by a longer road —
    /// `Agent::cross_affiliation_grant_subject` → `CallCapability::
    /// cross_affiliation` → [`super::affiliation::gate_cross_affiliation`] —
    /// which needs a live `ExtensionManager` and a bound provider to drive. What
    /// this must NOT be is a third spelling of the decision: an earlier
    /// `grant_prompt` re-implemented the gate's `Some(model)`/`None` branch and
    /// omitted its three guards, so a gate asserted against it would keep passing
    /// after the production composition changed underneath. Everything below runs
    /// through the gate.
    fn statement(
        model: Option<ModelAffiliation>,
        extension: &str,
        ext: &ExtensionAffiliation,
    ) -> Option<String> {
        crate::privacy::affiliation::gate_cross_affiliation_warning(
            true,
            crate::privacy::ProviderTier::Private,
            model,
            extension,
            &crate::privacy::ExtensionClassification {
                tier: crate::privacy::ProviderTier::Private,
                affiliation: ext.clone(),
            },
        )
        .map(|warning| accepted_statement(&warning))
    }

    async fn session_manager_with_a_chat() -> (tempfile::TempDir, SessionManager, String) {
        let dir = tempfile::tempdir().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let session = sm
            .create_session(PathBuf::from("."), "grant".to_string(), SessionType::User)
            .await
            .unwrap();
        (dir, sm, session.id)
    }

    /// Task 49 gate (1), at the store rather than at a gate: the triple is the
    /// key, and every one of its three components discriminates.
    #[tokio::test]
    async fn a_grant_matches_its_own_triple_and_no_other() {
        let (_dir, sm, id) = session_manager_with_a_chat().await;

        assert!(
            !is_granted(&sm, &id, "ucsfomopagent", bound_to("stanford")).await,
            "a chat with no grants grants nothing"
        );

        record_for_test(&sm, &id, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();

        assert!(is_granted(&sm, &id, "ucsfomopagent", bound_to("stanford")).await);
        // …and each axis, moved one at a time.
        assert!(
            !is_granted(&sm, &id, "cdwagent", bound_to("stanford")).await,
            "a grant is per extension"
        );
        assert!(
            !is_granted(&sm, &id, "ucsfomopagent", bound_to("mayo")).await,
            "a grant is per model affiliation, so re-binding invalidates it"
        );
        assert!(
            !is_granted(&sm, &id, "ucsfomopagent", Some(ModelAffiliation::Local)).await,
            "a grant given for an institutional model is not a grant for a local one"
        );
        assert!(
            !is_granted(&sm, &id, "ucsfomopagent", None).await,
            "…nor for a model that states no affiliation at all"
        );

        let (_dir2, other_sm, other_id) = session_manager_with_a_chat().await;
        assert!(
            !is_granted(&other_sm, &other_id, "ucsfomopagent", bound_to("stanford")).await,
            "a grant is never machine-wide"
        );
    }

    /// Task 56: a model covered by **two** institutions is a different flow from
    /// either half, and its grant key says so.
    ///
    /// ⚠ **A grant is the user's acceptance of ONE disclosure.** "Covered by
    /// ucsf" and "covered by ucsf and stanford" are not the same disclosure —
    /// the second also sends the extension's inputs and results to Stanford — so
    /// a spanning model inheriting a singleton's approval would clear a flow
    /// nobody was shown. The prefix (`institutions:` with a JSON array, beside
    /// the untouched `institution:<id>`) is what prevents it, and it was
    /// argued in a doc comment and asserted nowhere.
    ///
    /// The last pair is what the JSON array buys over a joined string: an
    /// institution id is only `name_to_key`-normalised (lowercase, whitespace
    /// stripped), so it may contain any separator character, and `{"a+b"}`
    /// would collide with `{"a", "b"}` under `+`.
    #[tokio::test]
    async fn a_model_covered_by_two_institutions_does_not_inherit_either_halfs_grant() {
        let (_dir, sm, id) = session_manager_with_a_chat().await;
        let spanning = |names: &[&str]| {
            Some(
                ModelAffiliation::institutions(names.iter().map(|n| InstitutionId::new(n)))
                    .expect("a fixture names at least one institution"),
            )
        };

        record_for_test(&sm, &id, "ucsfomopagent", bound_to("ucsf"))
            .await
            .unwrap();
        assert!(
            is_granted(&sm, &id, "ucsfomopagent", bound_to("ucsf")).await,
            "the singleton key must be unchanged, or every grant already in a user's \
             database stops matching"
        );
        assert!(
            !is_granted(&sm, &id, "ucsfomopagent", spanning(&["ucsf", "stanford"])).await,
            "a pair spanning two institutions must not inherit one half's approval; it \
             discloses to an endpoint the user was never shown"
        );

        // ...and the other direction: approving the pair does not approve a half.
        record_for_test(&sm, &id, "cdwagent", spanning(&["ucsf", "stanford"]))
            .await
            .unwrap();
        assert!(is_granted(&sm, &id, "cdwagent", spanning(&["ucsf", "stanford"])).await);
        assert!(
            !is_granted(&sm, &id, "cdwagent", bound_to("ucsf")).await,
            "a grant for the pair is not a grant for either institution alone"
        );
        assert!(
            !is_granted(
                &sm,
                &id,
                "cdwagent",
                spanning(&["ucsf", "stanford", "broad"])
            )
            .await,
            "two spanning models that differ in membership are different flows"
        );
    }

    /// The extension half is normalised on both sides, so the spelling the user's
    /// UI sends (`UCSFOMOPAgent`) finds the grant the dispatch looks up under the
    /// resolved client name (`ucsfomopagent`). Two normalisers would produce a
    /// grant that is recorded and never found — a control that silently does
    /// nothing.
    #[tokio::test]
    async fn the_extension_half_is_normalised_on_both_sides() {
        let (_dir, sm, id) = session_manager_with_a_chat().await;
        record_for_test(&sm, &id, "UCSF OMOP Agent", bound_to("stanford"))
            .await
            .unwrap();
        assert!(is_granted(&sm, &id, "ucsfomopagent", bound_to("stanford")).await);
    }

    /// Re-approving the same triple is not an error and does not multiply rows.
    /// The user did ask again, and the record should say when.
    #[tokio::test]
    async fn re_approving_the_same_triple_is_idempotent() {
        let (_dir, sm, id) = session_manager_with_a_chat().await;
        record_for_test(&sm, &id, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();
        record_for_test(&sm, &id, "ucsfomopagent", bound_to("stanford"))
            .await
            .expect("a second approval of the same flow is not a conflict");

        let pool = sm.storage().pool().await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cross_affiliation_grants")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    /// A subagent acts within authority its parent already holds — and only
    /// upward. The parent must NOT inherit from the child, or a chat could gain
    /// reach by spawning one.
    #[tokio::test]
    async fn a_subagent_inherits_its_parents_grants_and_the_parent_inherits_nothing() {
        let (_dir, sm, parent) = session_manager_with_a_chat().await;
        let child = sm
            .create_session(
                PathBuf::from("."),
                "child".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap()
            .id;
        sm.update(&child)
            .parent_session_id(Some(parent.clone()))
            .apply()
            .await
            .unwrap();

        record_for_test(&sm, &parent, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();
        assert!(
            is_granted(&sm, &child, "ucsfomopagent", bound_to("stanford")).await,
            "a subagent inherits its parent's grants"
        );

        // The other direction, which is the one that would be a hole.
        let (_dir2, sm2, parent2) = session_manager_with_a_chat().await;
        let child2 = sm2
            .create_session(
                PathBuf::from("."),
                "child".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap()
            .id;
        sm2.update(&child2)
            .parent_session_id(Some(parent2.clone()))
            .apply()
            .await
            .unwrap();
        record_for_test(&sm2, &child2, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();
        assert!(
            !is_granted(&sm2, &parent2, "ucsfomopagent", bound_to("stanford")).await,
            "a parent must not gain reach by spawning a child that was granted it"
        );
    }

    /// A `parent_session_id` cycle — reachable in a hand-edited or restored
    /// database, since the column carries no referential integrity — terminates
    /// rather than hanging the dispatch that asked, and still answers correctly
    /// on the way round.
    ///
    /// ⚠ **The absence half alone would not discriminate.** Asserting only that
    /// a cycle with no grant in it reads *not granted* passes just as happily
    /// against an implementation that returned `false` without walking anywhere
    /// — the two are indistinguishable from outside. So the cycle carries a real
    /// grant on `b`, one hop up from `a`: finding it proves the walk ran, and
    /// returning at all proves [`MAX_PARENT_DEPTH`] stopped it. Without the
    /// bound this test hangs, which is the failure it is here to catch.
    #[tokio::test]
    async fn a_parent_cycle_terminates_and_still_finds_a_grant_inside_it() {
        let (_dir, sm, a) = session_manager_with_a_chat().await;
        let b = sm
            .create_session(PathBuf::from("."), "b".to_string(), SessionType::User)
            .await
            .unwrap()
            .id;
        sm.update(&a)
            .parent_session_id(Some(b.clone()))
            .apply()
            .await
            .unwrap();
        sm.update(&b)
            .parent_session_id(Some(a.clone()))
            .apply()
            .await
            .unwrap();

        // Nothing granted anywhere in the cycle: terminates, fail-closed.
        assert!(!is_granted(&sm, &a, "ucsfomopagent", bound_to("stanford")).await);

        // One hop up, inside the cycle: terminates, and the walk really walked.
        record_for_test(&sm, &b, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();
        assert!(
            is_granted(&sm, &a, "ucsfomopagent", bound_to("stanford")).await,
            "the ancestor walk did not run: a cycle test with no grant in it \
             cannot tell that apart from a bounded walk that found nothing"
        );
        // …and a triple nobody granted is still not granted, so the assertion
        // above is not passing on a lookup that stopped discriminating.
        assert!(!is_granted(&sm, &a, "ucsfomopagent", bound_to("mayo")).await);
    }

    /// Task 49 gate (3). The statement the user decides on names **both**
    /// institutions, and the display names come from the registry's own map
    /// rather than from a string typed here — a hardcoded `"UCSF"` would keep
    /// passing after `registry.json` renamed it.
    ///
    /// Driven through [`statement`], which is the gate plus
    /// [`accepted_statement`] — the pair production composes. Asserting this
    /// against a composer of its own would leave the property true of a function
    /// nothing calls.
    #[test]
    fn the_prompt_names_both_institutions_from_the_registrys_map() {
        let published = crate::privacy::registry_private::INSTITUTIONS;
        assert!(
            !published.is_empty(),
            "the registry publishes no institutions, so every assertion below is vacuous"
        );

        for (id, display) in published {
            let institution = InstitutionId::new(id);

            // As the institution that OWNS the extension's data.
            let owner_side = statement(
                bound_to("an-unpublished-institution"),
                "ucsfomopagent",
                &ExtensionAffiliation::institution(institution),
            )
            .expect("a foreign model reaching a claimed connector is a mismatch");
            assert!(
                owner_side.contains(display),
                "the owning institution is not named by its published display name: {owner_side}"
            );
            assert!(
                owner_side.contains("an-unpublished-institution"),
                "the bound model's institution is not named: {owner_side}"
            );

            // …and as the institution whose model is BOUND. Both halves, because
            // a composer that names only the extension's owner satisfies half of
            // DR-26 and reads as a complete warning.
            let model_side = statement(
                Some(ModelAffiliation::institution(institution)),
                "someone-elses-connector",
                &ExtensionAffiliation::institution(InstitutionId::new(
                    "an-unpublished-institution",
                )),
            )
            .expect("a published model reaching a foreign connector is a mismatch");
            assert!(
                model_side.contains(display),
                "the bound model's institution is not named by its published display name: \
                 {model_side}"
            );
            assert!(
                model_side.contains("an-unpublished-institution"),
                "the owning institution is not named: {model_side}"
            );
        }
    }

    /// The prompt says what a yes covers, not only what the risk is. A user who
    /// believes they are approving one call behaves differently from one who
    /// knows they are approving this connector for the rest of the chat.
    #[test]
    fn the_prompt_states_the_scope_of_the_approval() {
        let prompt = statement(bound_to("stanford"), "ucsfomopagent", &ucsf_owned())
            .expect("this pair mismatches");
        assert!(prompt.contains(GRANT_SCOPE_COPY), "{prompt}");
        // The three narrowings, each stated. Generic reassurance ("your data is
        // safe") would pass a `contains(GRANT_SCOPE_COPY)` check alone.
        assert!(prompt.contains("this chat"), "{prompt}");
        assert!(prompt.contains("extension"), "{prompt}");
        assert!(prompt.contains("institution"), "{prompt}");

        // …and it does not fire on a flow with no institutional boundary in it,
        // which is the prompt fatigue DR-19 rejects.
        assert!(statement(
            Some(ModelAffiliation::Local),
            "ucsfomopagent",
            &ucsf_owned()
        )
        .is_none());
        assert!(statement(
            bound_to("stanford"),
            "developer",
            &ExtensionAffiliation::Any
        )
        .is_none());
    }

    /// The unstated-model arm is grantable too. It mismatches every claimed
    /// extension (`affiliation::unstated_model`), so a user who is asked must be
    /// able to answer — and the copy must not invent an institution for the model
    /// side, because "we cannot tell whose agreements cover this model" is the
    /// actionable statement and a fabricated name is not.
    #[test]
    fn a_model_that_states_no_affiliation_has_a_prompt_that_names_no_institution_for_it() {
        let owners = BTreeSet::from([InstitutionId::new("ucsf")]);
        let prompt = statement(
            None,
            "ucsfomopagent",
            &ExtensionAffiliation::Institutions(owners),
        )
        .expect("an unstated private model may not reach a claimed connector unasked");
        assert!(
            prompt.contains("does not state whose agreements cover it"),
            "{prompt}"
        );
        assert!(prompt.contains(GRANT_SCOPE_COPY), "{prompt}");
    }

    /// Task 49 gate (2). The tool-facing surface has no path to a grant.
    ///
    /// The proof-of-user is a cross-crate `pub` constructor, because the only
    /// caller lives in `biorouter-server` and `pub(in …)` cannot cross a crate
    /// boundary — Rust therefore cannot restrict it to one call site on its own.
    /// This test is what does, and it is Task 29's
    /// `the_proof_of_user_is_constructed_in_exactly_two_places` for the third
    /// axis.
    ///
    /// It pins two things and it takes both: the set of FILES outside this one
    /// that so much as NAME the type, and the number of times the constructor is
    /// CALLED. A count alone says nothing about which function holds it, and a
    /// file set alone says nothing about a second construction inside a permitted
    /// file.
    #[test]
    fn the_proof_of_user_is_constructed_in_exactly_one_place() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let crates = root.join("crates");
        assert!(
            crates.is_dir(),
            "the audit walks {}; if that path is wrong every assertion below \
             passes for the wrong reason",
            crates.display()
        );

        // Composed rather than written out, so this file does not match its own
        // audit and a reader cannot mistake the needle for a call site.
        let named = concat!("User", "CrossAffiliationGrant");
        let minted = concat!("User", "CrossAffiliationGrant::from_user_action(");

        let mut naming: Vec<String> = vec![];
        let mut constructions: std::collections::BTreeMap<String, usize> = Default::default();
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
            // This file declares the type and names it in its own test door;
            // counting it would make every number below larger and
            // indistinguishable from a real second site.
            if rel == "crates/biorouter/src/privacy/grant.rs" {
                continue;
            }
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
            let mut names_it = false;
            for line in src.lines() {
                let code = line.trim_start();
                // ⚠ The comment skip covers the NAMING set too, and it must:
                // prose cannot hold a proof-of-user and cannot construct one, so
                // a file that only *mentions* the type is not a way to accept a
                // cross-institutional flow. The twin audit
                // (`knowledge::tier_user::tests::the_proof_of_user_is_constructed_in_exactly_one_place`)
                // learned this the expensive way — it scanned whole files, a
                // `///` line in THIS module's header named its type, and the
                // audit went red across a task boundary with nobody's code at
                // fault. This copy is line-wise from the start so the two cannot
                // disagree about what a mention is, and so the next person is
                // not taught to relax an assertion to make prose compile.
                //
                // ⚠ Residual, stated because it is invisible: only `//` is
                // skipped. A `/* … */` block that names the type WILL trip this,
                // and the repair is to write the comment with `//`, never to
                // widen the skip — a skip that swallowed a line starting with
                // `*` could swallow a real construction, which is the one
                // direction an audit must never fail in.
                if code.starts_with("//") {
                    continue;
                }
                names_it |= code.contains(named);
                let hits = code.matches(minted).count();
                if hits > 0 {
                    *constructions.entry(rel.clone()).or_default() += hits;
                }
            }
            if names_it {
                naming.push(rel.clone());
            }
        }
        assert!(
            scanned >= 400,
            "only {scanned} .rs files were scanned. A broken walk reports the same \
             empty set as a clean tree."
        );
        naming.sort();
        assert_eq!(
            naming,
            vec!["crates/biorouter-server/src/routes/agent.rs".to_string()],
            "a new file names the cross-affiliation proof-of-user. The whole claim \
             that an agent cannot accept a cross-institutional risk on the user's \
             behalf rests on this set being exactly the one HTTP handler behind the \
             X-User-Action header."
        );
        let called: Vec<(String, usize)> = constructions.into_iter().collect();
        assert_eq!(
            called,
            vec![("crates/biorouter-server/src/routes/agent.rs".to_string(), 1)],
            "the proof-of-user is minted more than once. A second construction site \
             is a second way to accept a cross-institutional flow, and only the one \
             inside the grant handler is known to sit behind the user-action guard."
        );
    }

    /// The other half of gate (2): exactly one writer reaches the grant table.
    ///
    /// The proof audit above pins who may *ask* for a grant; this pins that there
    /// is no second door into the store that skips the proof entirely. A tripwire
    /// rather than a proof — it matches one spelling — but what it reliably
    /// catches is the realistic case, a second `INSERT` added by someone who
    /// copied this one.
    #[test]
    fn exactly_one_statement_in_the_tree_writes_a_cross_affiliation_grant() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        // Composed, so this test does not count itself.
        let needle = concat!("INSERT INTO ", "cross_affiliation_grants");
        let mut writers: Vec<String> = vec![];
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
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
            if src
                .lines()
                .any(|l| !l.trim_start().starts_with("//") && l.contains(needle))
            {
                writers.push(rel);
            }
        }
        assert!(scanned >= 400, "only {scanned} .rs files were scanned");
        writers.sort();
        assert_eq!(
            writers,
            vec!["crates/biorouter/src/privacy/grant.rs".to_string()],
            "a second module writes the grant store directly, bypassing the \
             proof-of-user this module exists to require."
        );
    }

    /// Task 57. [`GRANT_SCOPE_COPY`]'s doc says this sentence belongs to "the
    /// dialog the user clicks", and Task 57 built that dialog — so the renderer
    /// now needs it BEFORE the press, where the daemon cannot supply it: the
    /// daemon composes it into [`accepted_statement`] and returns it only once
    /// the grant is recorded, which is one press too late for the person
    /// deciding.
    ///
    /// So the renderer mirrors it, and this is the detector the doc's warning
    /// about "four hand-written copies drift within one release" otherwise
    /// lacked. It compares bytes against the real shipped file rather than a
    /// copy, and a reword on either side turns the Rust build red — which is the
    /// only direction that works, since the renderer's own tests cannot see this
    /// constant.
    #[test]
    fn the_scope_copy_the_user_reads_is_the_one_the_daemon_records() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let mirror = root.join("ui/desktop/src/utils/crossAffiliation.ts");
        let src = std::fs::read_to_string(&mirror).unwrap_or_else(|e| {
            panic!(
                "the accept control's module is missing at {} ({e}). Without it the user has no \
                 way to accept a cross-institutional flow, which is the whole of Task 57.",
                mirror.display()
            )
        });
        assert!(
            src.contains(GRANT_SCOPE_COPY),
            "the renderer states a different scope from the one the daemon records. The dialog \
             that asks and the audit row that answers must not differ by a word. Re-mirror \
             GRANT_SCOPE_COPY into {}.",
            mirror.display()
        );
        // …and the marker the same file keys the whole control off. A refusal
        // the renderer cannot recognise renders no button at all, silently.
        assert!(
            src.contains(super::super::refusal::CROSS_AFFILIATION_ACCEPT_MARKER),
            "the renderer no longer recognises the accept frame, so the control it exists to \
             render can never appear"
        );
    }

    /// A grant belongs to the chat it was given in, and "the chat" is not "the
    /// id". `create_session` mints `<day>_<MAX(N)+1>`, so deleting the newest
    /// chat of the day hands its id to the next one — "delete the chat I just
    /// made and start again". Before a delete took the chat's grants with it,
    /// that next chat silently inherited every cross-institutional flow the user
    /// had accepted in the deleted one: a flow nobody accepted in the new chat,
    /// which is exactly what [`GRANT_SCOPE_COPY`] promises cannot happen.
    #[tokio::test]
    async fn a_deleted_chats_grant_is_not_inherited_by_the_next_chat_to_get_its_id() {
        let (_dir, sm, id) = session_manager_with_a_chat().await;
        record_for_test(&sm, &id, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();
        assert!(is_granted(&sm, &id, "ucsfomopagent", bound_to("stanford")).await);

        sm.delete_session(&id).await.unwrap();
        let next = sm
            .create_session(PathBuf::from("."), "next".to_string(), SessionType::User)
            .await
            .unwrap()
            .id;
        assert_eq!(
            next, id,
            "the fixture must reproduce the id reuse, or this test proves nothing"
        );

        assert!(
            !is_granted(&sm, &next, "ucsfomopagent", bound_to("stanford")).await,
            "a new chat inherited a cross-institutional approval given in a chat the user deleted"
        );
    }

    /// ...and a delete is not the only way a grant can outlive its chat. Every
    /// build before this one left grants behind on delete, a terminal
    /// `biorouter` that lags the desktop app still does while it shares this
    /// database, and a restored backup can hold grants for ids that are live
    /// again. So the reader refuses, on its own, a grant recorded before the chat
    /// now holding the id existed — it cannot have been given in that chat — and
    /// it does so at read time, not only when a startup sweep next runs.
    #[tokio::test]
    async fn a_grant_recorded_before_its_chat_existed_is_not_read() {
        let (_dir, sm, id) = session_manager_with_a_chat().await;
        record_for_test(&sm, &id, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();

        // The delete an older build performs: the chat goes, its grants stay.
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query("DELETE FROM sessions WHERE id = ?1")
            .bind(&id)
            .execute(pool)
            .await
            .unwrap();
        // Backdated, because a person takes longer than one clock second to
        // accept a warning, delete the chat and start another; a fixture that
        // did all three inside one second would be testing a tie no user makes.
        sqlx::query("UPDATE cross_affiliation_grants SET granted_at = datetime('now', '-1 hour')")
            .execute(pool)
            .await
            .unwrap();

        let next = sm
            .create_session(PathBuf::from("."), "next".to_string(), SessionType::User)
            .await
            .unwrap()
            .id;
        assert_eq!(
            next, id,
            "the fixture must reproduce the id reuse, or this test proves nothing"
        );
        assert!(
            !is_granted(&sm, &next, "ucsfomopagent", bound_to("stanford")).await,
            "a grant older than the chat holding its id was read as that chat's"
        );

        // ...and the new chat's own acceptance of the same flow is honoured: the
        // upsert refreshes `granted_at`, so the row is this chat's again.
        record_for_test(&sm, &next, "ucsfomopagent", bound_to("stanford"))
            .await
            .unwrap();
        assert!(
            is_granted(&sm, &next, "ucsfomopagent", bound_to("stanford")).await,
            "re-accepting the flow in the new chat must work"
        );
    }
}
