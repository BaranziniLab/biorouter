//! Gate H (issue #56): the alternate-provider barrier.
//!
//! Three production paths hand a **session's** content to a provider the session
//! row never records, and none of them passes `Agent::update_provider` or
//! `Agent::reply` — so Gates A–F are all blind to them:
//!
//! 1. **CLI plan mode** — `get_reasoner` (`biorouter-cli/src/session/mod.rs`)
//!    clones the whole message list and completes it on a provider named by
//!    `BIOROUTER_PLANNER_PROVIDER`, or, failing that, the global default.
//! 2. **Prompt hooks** — `resolve_prompt_provider` (`hooks/mod.rs`) builds a
//!    provider from a hook definition's `provider:`/`model:` pair, and the Stop
//!    hook's payload carries `transcript_tail(&conversation)`, i.e. real
//!    transcript text, at the end of every turn.
//! 3. **The knowledge default model** — `build_model_ref_completer`
//!    (`agents/knowledge_tool.rs`) prefers a target KB's `default_model` over
//!    the agent's own provider for scheduled jobs.
//!
//! Site 3 is reached through exactly one function, `build_model_ref_provider`,
//! by BOTH knowledge paths: `platform__ingest_source`'s `model` argument (a
//! provider name the MODEL wrote) and a base's stored `default_model` on a
//! scheduled digest. That is deliberate — they end in the same knowledge-base
//! ratchet, so a third knowledge path cannot be added without passing the gate.
//!
//! A fourth site, the HTTP knowledge macros (`routes/knowledge.rs`'s
//! `build_completer`), is deliberately **not** here, and neither is its CLI twin
//! (`biorouter-cli/src/commands/knowledge.rs`'s `build_completer`, whose
//! provider comes from `--provider`): both have a knowledge-base id and no
//! session at all, so the predicate they need is the KB-keyed one Task 10C
//! installs (`assert_macro_target_reachable` → `knowledge::tier::assert_reachable`),
//! not this session-keyed one. Putting a `SessionClassification` check there
//! would be a type error, not merely a misfiling. The person who typed the
//! provider name is also the one DR-16 rules may raise a tier, which is the
//! substantive half of the distinction rather than the typing one.
//!
//! # How to re-derive this list rather than trust it
//!
//! Grep the workspace for `providers::create`, `create_from_persisted` and
//! `create_with_named_model`, then keep only the calls where BOTH hold: the
//! provider is *not* the one the session row records, and a session's content
//! reaches it. Everything else falls out for a nameable reason, and the reasons
//! are worth knowing because each looks like a Gate H site from the grep alone:
//!
//! * **It binds, so Gate A owns it** — `Agent::update_provider` and the turn
//!   barrier, the CLI session builder, `routes/agent.rs`, `commands/web.rs`,
//!   `biorouter-acp`, `scheduler.rs`, and `workspace_set_tools`' provider switch
//!   (which additionally asks `tool_bind_allowed`, DR-16's half — see
//!   [`assert_alt_provider_matches_session`], which asks the same predicate).
//! * **It is the spawn**, whose own gate already refuses BOTH directions
//!   (`subagent_tool::apply_settings_overrides`).
//! * **No session content reaches it** — `config_management.rs`'s provider
//!   validation and `auto_detect.rs`'s availability probe send a canned request.
//! * **It is the app bind**, which DR-21 confines to `app_provider_bind` (see
//!   the ⚠ note where `routes/apps.rs` deliberately does *not* import `create`).
//!
//! Re-derived this way on 2026-09-12: the three sites above, plus the two
//! KB-keyed exemptions, are the whole list.
//!
//! # What this gate does NOT do: it never raises the floor
//!
//! [`assert_alt_provider_allowed`] only *checks*; it never calls `raise_privacy`.
//! So the reverse flow is unclassified: CLI plan mode pushes the planner's
//! answer back into `self.messages` (`plan_with_reasoner_model`), and if that
//! planner was PRIVATE while the session is public, the session absorbs private
//! output and stays public. The same holds for a private prompt-hook model whose
//! text reaches the transcript.
//!
//! That is under-classification, never over-disclosure — the dangerous direction
//! is refused above — and it is the same class as the one-turn window
//! `Agent::reply` documents at its ratchet (O5). Recorded here because Task 19
//! did not close it and nothing else in this module would say so.
//!
//! # Two halves, because "harmless" depends on where the tier lands
//!
//! [`assert_alt_provider_allowed`] refuses only the DOWNWARD choice, because
//! that is the only one that discloses anything: a MORE private provider cannot
//! leak the session it is handed. Sites 1 and 2 above take that half, and it is
//! the right one for them — plan mode's answer and a hook's output land in
//! `self.messages`, where the residual is the under-classification recorded
//! above and the provider was named by a person's own configuration.
//!
//! [`assert_alt_provider_matches_session`] is the STRICTER half, and site 3
//! needs it: a knowledge ingest does not merely read the session, it writes into
//! a **knowledge base**, whose tier is a permanent, monotone ratchet over the
//! callers that write to it (`biorouter-mcp`'s `knowledge::tier`). The chosen
//! provider's tier — not the session's classification — is what
//! `SourceIngestArgs::caller_capability` carries into that ratchet, so the
//! "harmless" upward choice permanently privatises a base a **public** chat
//! owns, and that chat is then refused at every KB read choke point. The user
//! loses a base to a decision nobody asked them about.
//!
//! So on that half the tiers must MATCH. It is the same ruling the spawn gate
//! reached for the same reason (`subagent_tool::apply_settings_overrides`
//! refuses `spawn_upgrade` AND `spawn_downgrade`): R4 says a public parent may
//! not gain private reach, DR-19 supplies the initiator R4 never named, and a
//! tool call has no person on the other end to consent. DR-16 rules the raise
//! the user's decision, and SD-8 rules that a control which can never work here
//! says so rather than asking — `is_user_action` is false for every tool call,
//! so a proof check here could only ever refuse.

use anyhow::{anyhow, Result};

use super::{bind_allowed, tool_bind_allowed, SessionClassification};
use crate::providers::base::Provider;

/// Refuse to hand a session's content to a provider that is not the one bound to
/// it.
///
/// * `what` names the feature, in the sentence the user reads ("plan mode",
///   "this prompt hook").
/// * `session` is the classification of the session whose content is about to
///   travel — its **stored** tier, not the tier of whatever is bound right now:
///   the whole point of this gate is that nothing is bound to the provider being
///   built here.
/// * `env_key_to_name` is the knob that fixes it, named so the refusal is a
///   redirection rather than a dead end.
///
/// The predicate is [`bind_allowed`], unchanged and unduplicated: "may this
/// provider see this session's contents" is one question, and a second spelling
/// of it is a second answer waiting to drift.
pub fn assert_alt_provider_allowed(
    what: &str,
    provider: &dyn Provider,
    session: SessionClassification,
    env_key_to_name: &str,
) -> Result<()> {
    // DR-15's master opt-out, read INSIDE the gate. A direct read, not a
    // `CallCapability`: this gate fires where nothing is bound, so there is no
    // admitted tool call whose capability it could inherit.
    if !super::privacy_tiers_enabled() {
        return Ok(());
    }
    refuse_downward(what, provider, session, env_key_to_name)
}

/// [`assert_alt_provider_allowed`] **plus** the upward half, for an alternate
/// provider whose tier does not stay in this process: it becomes the ratchet
/// input of a knowledge base's permanent classification.
///
/// The arguments and the downward sentence are the sibling's, unchanged and
/// undoubled — this is that gate AND one more rule, not a second spelling of it,
/// so the two can never disagree about the cell they share. What it adds is the
/// raise: a provider more private than the session is refused, because
/// `caller_capability` is what `knowledge::tier::raise_unlocked` ratchets on and
/// a ratchet is not undoable. A public chat that names a private model would
/// mark its own base private for good and then be refused at
/// `knowledge::tier::assert_reachable` — a loss of access to the user's own
/// notes that no one was asked about, delivered by a tool call.
///
/// ⚠ **Do not "simplify" this to the sibling.** The cell that differs —
/// `(session = Public, provider = Private)` — is `Ok` there on purpose and is
/// the whole finding here; `only_a_private_session_on_a_public_provider_is_refused`
/// and [`the_upward_choice_is_refused_only_on_the_ratcheting_half`] pin both
/// answers side by side.
///
/// It is also the same shape as `workspace_set_tools`' provider switch, which
/// asks [`bind_allowed`] and then [`tool_bind_allowed`] for the same two
/// sentences. That is not a coincidence to be tidied away later: both surfaces
/// are a MODEL asking for a tier it was not given, so they must answer with one
/// set of cells. This function therefore *calls* [`tool_bind_allowed`] rather
/// than re-spelling `is_private() == is_private()`, and a change to DR-16's rule
/// lands on both sites or neither.
///
/// [`the_upward_choice_is_refused_only_on_the_ratcheting_half`]: tests::the_upward_choice_is_refused_only_on_the_ratcheting_half
pub fn assert_alt_provider_matches_session(
    what: &str,
    provider: &dyn Provider,
    session: SessionClassification,
    env_key_to_name: &str,
) -> Result<()> {
    // One read of DR-15's switch for both rules, for the same reason
    // `tier::assert_reachable` reads it once for both of its axes.
    //
    // ⚠ It is a DIRECT read, and on one of this gate's two callers that is a
    // departure from the house rule — measured, bounded and recorded rather than
    // half-fixed. The rule (see `privacy::CallCapability`) is that a gate on a
    // TOOL-CALL path asks the once-per-call `cap.enforced()` instead of
    // re-reading the flag, because `privacy_tiers_enabled()` is a runtime
    // `AtomicBool` that `POST /config/upsert`'s gated arm can flip mid-call.
    // `platform__ingest_source` IS a tool call and does hold a sampled
    // `CallCapability` (its `resolve_target_kb` already uses it), so this read is
    // a second one; the scheduled-digest caller threads no capability at all, so
    // it could only ever read directly.
    //
    // Why the residual is not worth the threading: the harmful outcome needs the
    // switch OFF **here** (to admit the raise) and ON again at the write (to
    // ratchet), because `knowledge::tier::raise_unlocked` asks
    // `ratchets_are_live()` for itself. A flip in one direction produces no loss,
    // and both directions inside one ingest means the user turned protection off
    // and back on while a model was resolving a provider. Threading a capability
    // through two callers where only one has one, to close that, is a larger
    // change with a worse failure mode than the race it removes. If a future
    // caller gives this gate a capability on BOTH paths, take it — do not add a
    // parameter only the first path can fill.
    if !super::privacy_tiers_enabled() {
        return Ok(());
    }
    refuse_downward(what, provider, session, env_key_to_name)?;
    // DR-16's half, asked through the predicate that already states it
    // (`privacy::tool_bind_allowed`) instead of a second spelling of its cells.
    // Reached only when `refuse_downward` — i.e. `bind_allowed` — already
    // passed, so the only way this can be false is the RAISE; the sentence for
    // the downward cell belongs to `refuse_downward` and is not repeated here.
    if tool_bind_allowed(provider.tier(), session) {
        return Ok(());
    }
    Err(anyhow!(
        "This chat is public, so {what} cannot run on `{}`, which is a private model: a \
         knowledge base takes the tier of the most private model that writes to it, so this \
         would mark the base private permanently and lock this public chat out of its own \
         notes. Nothing was written. Do not retry with the same model; the answer will not \
         change. Set {env_key_to_name} to a public model, or stop and ask the user to \
         continue this work in a private chat.",
        provider.get_name()
    ))
}

/// Gate A's rule, and the one sentence that states it. Shared so both public
/// entry points above refuse the downward choice in the same words — and so
/// [`bind_allowed`] keeps exactly one caller in this file, which the guard
/// census counts.
fn refuse_downward(
    what: &str,
    provider: &dyn Provider,
    session: SessionClassification,
    env_key_to_name: &str,
) -> Result<()> {
    if bind_allowed(provider.tier(), session) {
        return Ok(());
    }
    Err(anyhow!(
        "This chat is private, so {what} cannot run on `{}`, which is a public model. \
         Set {env_key_to_name} to a private model, or start this work in a public chat.",
        provider.get_name()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::declarative_providers::{DeclarativeProviderConfig, ProviderEngine};
    use crate::model::ModelConfig;
    use crate::privacy::ProviderTier;
    use crate::providers::ollama::OllamaProvider;

    /// A **real** provider at the requested tier, built the way a declarative
    /// JSON file builds one. Not a mock: `tier()` has to be the production
    /// implementation, or this file asserts something about a stub.
    fn provider_at(tier: ProviderTier) -> OllamaProvider {
        let base_url = match tier {
            ProviderTier::Private => "http://localhost:11434",
            ProviderTier::Public => "https://api.example-saas.com",
        };
        let config = DeclarativeProviderConfig {
            name: "planner".to_string(),
            engine: ProviderEngine::Ollama,
            display_name: "Planner".to_string(),
            description: None,
            api_key_env: "NOT_USED".to_string(),
            base_url: base_url.to_string(),
            models: vec![],
            headers: None,
            timeout_seconds: None,
            supports_streaming: None,
        };
        let provider =
            OllamaProvider::from_custom_config(ModelConfig::new_or_fail("qwen3"), config)
                .expect("a declarative ollama provider must construct");
        assert_eq!(provider.tier(), tier, "the fixture must resolve {tier:?}");
        provider
    }

    #[test]
    fn only_a_private_session_on_a_public_provider_is_refused() {
        use SessionClassification::{Private, Public};

        // The one refused combination, and the message has to carry both the
        // reason and the fix — a refusal that names neither is indistinguishable
        // from a broken feature.
        let err = assert_alt_provider_allowed(
            "plan mode",
            &provider_at(ProviderTier::Public),
            Private,
            "BIOROUTER_PLANNER_PROVIDER",
        )
        .expect_err("a private chat on a public model must be refused")
        .to_string();
        assert!(err.contains("private"), "{err}");
        assert!(err.contains("plan mode"), "{err}");
        assert!(err.contains("planner"), "the provider must be named: {err}");
        assert!(err.contains("BIOROUTER_PLANNER_PROVIDER"), "{err}");

        // ...and the other three are allowed, or this is not a barrier but an
        // outage. A public session is unaffected in BOTH directions HERE: the
        // upward choice discloses nothing, so on a path whose tier stops in this
        // process it is Gate A's business and not this one's. Where that tier
        // becomes a knowledge base's permanent classification it is refused, and
        // the test below is the other half of this pair.
        for (session, tier) in [
            (Private, ProviderTier::Private),
            (Public, ProviderTier::Public),
            (Public, ProviderTier::Private),
        ] {
            assert!(
                assert_alt_provider_allowed("plan mode", &provider_at(tier), session, "KEY")
                    .is_ok(),
                "{session:?} + {tier:?} must be allowed"
            );
        }
    }

    /// The ratcheting half. Three cells agree with the sibling above and the
    /// fourth — a PUBLIC session naming a PRIVATE provider — is the finding:
    /// allowed there, refused here.
    ///
    /// Stated as both the four cells and the rule, so a third tier cannot be
    /// added while satisfying the cells.
    #[test]
    fn the_upward_choice_is_refused_only_on_the_ratcheting_half() {
        use SessionClassification::{Private, Public};

        let err = assert_alt_provider_matches_session(
            "ingesting these sources",
            &provider_at(ProviderTier::Private),
            Public,
            "this tool's `model` argument",
        )
        .expect_err("a public chat may not privatise its base by naming a private model")
        .to_string();
        // A refusal names what it refused, why, and the way out.
        assert!(err.contains("public"), "{err}");
        assert!(err.contains("private model"), "{err}");
        assert!(err.contains("planner"), "the provider must be named: {err}");
        assert!(err.contains("ingesting these sources"), "{err}");
        assert!(err.contains("`model` argument"), "{err}");
        assert!(
            err.contains("Do not retry"),
            "a refusal the model will retry is a loop: {err}"
        );

        // The downward cell is still the sibling's sentence, not a second one:
        // one rule, one wording, whichever entry point asked.
        let downward = assert_alt_provider_matches_session(
            "ingesting these sources",
            &provider_at(ProviderTier::Public),
            Private,
            "KEY",
        )
        .expect_err("a private chat on a public model must still be refused")
        .to_string();
        assert_eq!(
            downward,
            assert_alt_provider_allowed(
                "ingesting these sources",
                &provider_at(ProviderTier::Public),
                Private,
                "KEY"
            )
            .expect_err("the sibling refuses the same cell")
            .to_string(),
            "the two entry points must refuse the downward choice in the same words"
        );

        // Sideways, both directions of the pair: the only choices that remain,
        // and the ones that must keep working.
        for (session, tier) in [
            (Private, ProviderTier::Private),
            (Public, ProviderTier::Public),
        ] {
            assert!(
                assert_alt_provider_matches_session(
                    "ingesting these sources",
                    &provider_at(tier),
                    session,
                    "KEY"
                )
                .is_ok(),
                "{session:?} + {tier:?} is sideways, not a crossing"
            );
        }

        // The rule, not the cells — and stated as the predicate rather than as a
        // second spelling of its comparison. This gate admits exactly what the
        // model-facing BIND predicate admits, because both surfaces are a model
        // asking for a tier nobody granted it; `workspace_set_tools`' provider
        // switch is the other caller. The four cells are pinned above, so this
        // adds the thing the cells cannot say: that the two policies are one.
        for (session, tier) in [
            (Public, ProviderTier::Public),
            (Public, ProviderTier::Private),
            (Private, ProviderTier::Public),
            (Private, ProviderTier::Private),
        ] {
            assert_eq!(
                assert_alt_provider_matches_session("x", &provider_at(tier), session, "KEY")
                    .is_ok(),
                tool_bind_allowed(tier, session),
                "{session:?} + {tier:?}"
            );
        }
    }
}
