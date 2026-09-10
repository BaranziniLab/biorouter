//! Task 30's behavioural gate: the master toggle `BIOROUTER_PRIVACY_TIERS`,
//! asserted in BOTH positions at every enforcement point this crate can reach
//! (issue #56, DR-15).
//!
//! ⚠ **THE ROW LIST IS THE PLAN'S GATE INVENTORY.** It must agree, row for row,
//! with the structural inventory Step 5 diffs against the tree, and Step 5 fails
//! if the two disagree or if the tree contains an enforcement point that appears
//! in neither.
//!
//! ⚠ **Do not collapse this into a loop over closures returning `bool`.** Each
//! row needs a different fixture, and the value of the test is that a compile
//! error appears when a gate this plan adds later is not represented here.
//!
//! ⚠ **Why this is an integration binary and not `--lib`.** `set_privacy_tiers`
//! mutates a PROCESS-GLOBAL atomic, and `cargo test` runs a crate's unit tests
//! in parallel threads of ONE process. A mutex serializes the tests that *take*
//! it, but `crates/biorouter/src/**`'s ~60 existing privacy tests do not take
//! it and would read `false` while this matrix held it there — they would fail
//! spuriously, or worse, pass while asserting nothing. Each `tests/*.rs` file is
//! its own process, so the only tests that can observe this file's writes are
//! this file's own, and they all take the guard. That is a deviation from the
//! plan's `cargo test -p biorouter --lib privacy`; the plan's own ⚠ names the
//! hazard but its remedy only covers callers of the helper.
//!
//! ⚠ **Three rows live elsewhere, and each reason is a visibility rule, not
//! omission.** Every one of them is asserted in BOTH toggle positions beside
//! the code it governs — none is missing, and the file it lives in is named
//! here so this list can be checked rather than believed:
//!
//! | Row | Gate | Where it is asserted | Why not here |
//! |---|---|---|---|
//! | 11 | F1, `check_enable_allowed` | `src/agents/extension_manager_extension.rs`, `the_master_toggle_silences_gate_f1` | An on-tool-call-path gate: its OFF column is a `CallCapability` whose `enforced` is false, and the only constructor for one is `#[cfg(test)] pub(crate)`. Driving it end-to-end would also need `ucsfomopagent` written into the developer's real `config.yaml`. |
//! | 15 | archive, `kb_export` | `biorouter-mcp/tests/privacy_toggle_export.rs` | Inside `biorouter-mcp`; its only cheap fixture is a `KnowledgeServer` built by that crate's own `#[cfg(test)]` helpers. |
//! | — | `/config/upsert`'s gated arm | `biorouter-server/tests/privacy_toggle_config.rs` | Inside `biorouter-server`, which this crate cannot depend on. |

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::{Session, SessionType};
use biorouter::session::SessionManager;
use biorouter_mcp::knowledge::caller::KbCaller;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use tempfile::TempDir;

// ─────────────────────────────────────────────────────────────────────────────
// The one helper every test in this file shares, DEFINED rather than assumed.
// ─────────────────────────────────────────────────────────────────────────────

/// ⚠ Its own mutex, NOT `env_lock`'s. `env_lock`'s surface here is
/// `lock_env(iter) -> EnvGuard`; it exposes no bare process lock, and `EnvGuard`
/// is a sync guard, so reaching for it would mean either locking a variable this
/// has nothing to do with or holding a `std::sync` guard across an `.await` —
/// not `Send` in a `#[tokio::test]`. What the serialization actually requires is
/// only that **every** mutation of the toggle in this test binary goes through
/// one mutex, and this is it. A test that mutates BOTH the toggle and the
/// environment takes both guards, toggle first.
static PRIVACY_TIER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// ⚠ Drop restores the PREVIOUS value, not `true`. Nesting has to unwind, or the
/// last test to run decides what every test scheduled after it asserts.
pub struct PrivacyTierGuard {
    prev: bool,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for PrivacyTierGuard {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.prev);
    }
}

/// ⚠ NOT reentrant. A test holding one guard must `drop` it before taking
/// another, and must never re-arm the flag by calling this again while one is
/// live — that deadlocks rather than failing, which is the worst way for a test
/// helper to be wrong. Re-arming mid-test is a bare
/// `set_privacy_tiers_enabled(..)` under the guard already held.
pub async fn set_privacy_tiers(on: bool) -> PrivacyTierGuard {
    let _lock = PRIVACY_TIER_LOCK.lock().await;
    let prev = biorouter_mcp::privacy_toggle::privacy_tiers_enabled();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(on);
    PrivacyTierGuard { prev, _lock }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures. Each row needs a different one.
// ─────────────────────────────────────────────────────────────────────────────

/// One of the two extensions the compiled-in BAAM baseline calls private.
const PRIVATE_EXTENSION: &str = "ucsfomopagent";
const PRIVATE_TOOL: &str = "ucsfomopagent__data_sources";

/// The one sentence every turn refusal contains. Spelled as a literal rather
/// than imported, so a change to `turn_refusal`'s wording that silently stopped
/// refusing would still have to get past this test.
const TURN_REFUSAL_MARKER: &str = "this turn was not sent";

/// Row 7's subject, and deliberately a session TITLE. §11.4 classifies a title
/// as CONTENT — in this product it is generated from the conversation — so a
/// LOAD guard placed after the header `format!` would return it. Asserting on
/// the title in both directions is what makes row 7 discriminating: an ON column
/// that merely errored, or an OFF column that merely did not, would pass against
/// an unwired guard.
const PRIVATE_SESSION_TITLE: &str = "OMOP diabetes cohort characterisation";

struct TieredProvider {
    name: &'static str,
    tier: ProviderTier,
}

#[async_trait]
impl Provider for TieredProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new("tiered", "Tiered", "", "tiered-model", vec![], "", vec![])
    }

    fn get_name(&self) -> &str {
        self.name
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    /// ⚠ **A private double must state an affiliation, because every real
    /// private provider does** — issue #56 DR-26, Task 48.
    ///
    /// `Some(..)` exactly while a provider's tier is Private is a property of
    /// this build rather than an accident: both deciders route *through* the
    /// tier predicate (`ucsf_gateway_affiliation`, `self_hosted_affiliation`)
    /// and `LeadWorkerProvider` folds both halves. Leaving this on the trait
    /// default gives the one pairing DR-26's vocabulary says cannot exist —
    /// Private tier, affiliation `None` — which the gate treats as *unstated*
    /// rather than as *unconstrained*. Every ON column below would then refuse
    /// for a reason this matrix is not about, and the OFF column would still
    /// pass, so the row would go on looking green while asserting the wrong
    /// thing.
    ///
    /// `Local` because it is DR-26's identity element: the one model
    /// affiliation compatible with every extension. `self.name` is a provider
    /// NAME and never decides an affiliation — see `Provider::affiliation`'s
    /// doc for why a name-keyed table is wrong.
    fn affiliation(&self) -> Option<biorouter::privacy::ModelAffiliation> {
        match self.tier {
            ProviderTier::Private => Some(biorouter::privacy::ModelAffiliation::Local),
            ProviderTier::Public => None,
        }
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Ok((
            Message::assistant().with_text("ok"),
            ProviderUsage::new("tiered-model".to_string(), Usage::default()),
        ))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("tiered-model")
    }
}

const PUBLIC_DOUBLE: &str = "toggle-matrix-public";
const PRIVATE_DOUBLE: &str = "toggle-matrix-private";

/// ⚠ **A double's NAME must not be a registry entry, or this binary hangs
/// forever.** `update_provider` writes the double's name onto the session row,
/// and Gate B's repair arm (`Agent::rebind_from_row`) hands that name to the
/// REAL `providers::create` — the seam that keeps `agent.rs`'s own unit tests
/// off the factory (`seams::override_rebind_provider`) is `#[cfg(test)]`, so it
/// does not exist for an integration binary, which links the library compiled
/// WITHOUT `cfg(test)`.
///
/// Named `anthropic`, row 2 below therefore reached
/// `AnthropicProvider::from_env` -> `Config::global().get_secret(..)` ->
/// `SecKeychainFindGenericPassword`, and blocked in securityd waiting for an
/// interactive Keychain authorization dialog that nothing answers. A fresh
/// `cargo build` yields a new binary hash with no ACL grant, so the prompt
/// returns on every rebuild — and a hang, unlike a failure, is invisible in
/// `cargo test --workspace`, which reports nothing for a binary that never
/// finishes. Measured: 1.3 s with these names, unbounded with `anthropic`.
///
/// With a name the registry does not know, `create` fails at the lookup —
/// before any constructor, any secret and any host — and Gate B's caller treats
/// that error exactly as it treats "the row names something still public": it
/// refuses the turn, which is what row 2 asserts. Gate B's *repair* arms
/// (`Ok(true)`/`Ok(false)`) are covered hermetically over the seam in
/// `agent.rs`'s own `a_repairable_mismatch_rebinds_silently_and_the_turn_runs`
/// and `an_unrepairable_mismatch_refuses_this_turn_and_leaves_the_row_alone`;
/// what belongs HERE is the toggle governing the barrier, and that is
/// unchanged. `the_matrix_doubles_are_not_registry_providers` fails fast if
/// either name ever collides again.
fn public_provider() -> Arc<dyn Provider> {
    Arc::new(TieredProvider {
        name: PUBLIC_DOUBLE,
        tier: ProviderTier::Public,
    })
}

fn private_provider() -> Arc<dyn Provider> {
    Arc::new(TieredProvider {
        name: PRIVATE_DOUBLE,
        tier: ProviderTier::Private,
    })
}

/// An agent over an isolated session store, already bound to `provider`. The
/// `TempDir` is returned because dropping it deletes the SQLite file the agent
/// still holds.
async fn agent_on(provider: Arc<dyn Provider>) -> (TempDir, Arc<Agent>, Session) {
    let dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    let permission_manager = Arc::new(PermissionManager::new(dir.path().to_path_buf()));
    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        session_manager,
        permission_manager,
        None,
        BioRouterMode::Auto,
    )));
    let session = agent
        .config
        .session_manager
        .create_session(
            PathBuf::from("."),
            "toggle-matrix".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    agent.update_provider(provider, &session.id).await.unwrap();
    (dir, agent, session)
}

/// The same, with the private extension loaded under a private NAME, so the tier
/// the gates read is stamped by the production admission path rather than poked
/// into the record by the fixture.
async fn agent_with_the_private_extension(
    provider: Arc<dyn Provider>,
) -> (TempDir, Arc<Agent>, Session) {
    let (dir, agent, session) = agent_on(provider).await;
    agent
        .extension_manager
        .add_inprocess_server(
            PRIVATE_EXTENSION,
            biorouter_mcp::datasql::server::DataSqlServer::new(std::collections::HashMap::new()),
        )
        .await
        .expect("inject the private extension");
    (dir, agent, session)
}

async fn ratchet_to_private(sm: &SessionManager, id: &str) {
    sm.update(id)
        .raise_privacy(
            SessionClassification::Private,
            &format!("turn:{PRIVATE_DOUBLE}"),
        )
        .apply()
        .await
        .unwrap();
}

fn cfg(session: &Session) -> SessionConfig {
    SessionConfig {
        id: session.id.clone(),
        schedule_id: None,
        max_turns: Some(2),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    }
}

/// Row 2's subject: the turn barrier, driven through the real `Agent::reply`.
/// Returns the concatenated text of every message event, refusal included.
async fn turn_text(agent: &Agent, session: &Session) -> String {
    let mut stream = agent
        .reply(Message::user().with_text("hi"), cfg(session), None)
        .await
        .unwrap();
    let mut out = String::new();
    while let Some(event) = stream.next().await {
        if let Ok(AgentEvent::Message(m)) = event {
            out.push_str(&m.as_concat_text());
            out.push('\n');
        }
    }
    out
}

/// Row 3's subject: the agent loop's own dispatch, which samples the capability
/// from the bound provider and hands it down. Returns the tool's output text
/// **whatever the outcome**, refusal message included.
async fn call_private_tool_via_agent_loop(agent: &Agent, session: &Session) -> String {
    let (_id, result) = agent
        .dispatch_tool_call(
            CallToolRequestParams {
                task: None,
                name: PRIVATE_TOOL.to_string().into(),
                arguments: Some(rmcp::object!({})),
                meta: None,
            },
            "req-1".to_string(),
            None,
            session,
        )
        .await;
    match result {
        Ok(call) => match call.result.await {
            Ok(ok) => format!("{ok:?}"),
            Err(e) => e.message.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

/// Row 4's subject: Gate C', which is a DIFFERENT function on a DIFFERENT path
/// from Gate C — the resource surface, not tool dispatch — and was covered by
/// nothing until review said so. `assert_extension_reachable` is private;
/// `read_resource` is its production caller and is `pub`, and passing `None` for
/// the admitted capability is what six of the eight real entries do, so the
/// guard samples the toggle live exactly as it does for them.
///
/// Returns the error text, or the success shape, whichever came back.
async fn read_private_resource(agent: &Agent) -> String {
    match agent
        .extension_manager
        .read_resource(
            "ui://cohort",
            PRIVATE_EXTENSION,
            None,
            tokio_util::sync::CancellationToken::default(),
        )
        .await
    {
        Ok(ok) => format!("{ok:?}"),
        Err(e) => e.message.to_string(),
    }
}

/// Row 4b's subject: the same surface's **fan-out** branch, which is a different
/// leak from row 4's refusal and was live on `main` until the 2026-09-10 test
/// drive measured it (finding M6).
///
/// `read_resource_tool` with no `extension_name` probes every installed
/// extension in turn, refusing the private ones one at a time — and then
/// composed its not-found message from `self.extensions.lock().await.keys()`,
/// handing back in one sentence every name the loop had just withheld. Row 4
/// cannot see it: that row asks the NAMED branch, which never reaches this
/// message.
///
/// `None` for the admitted capability, exactly as row 4 does, so the guard
/// samples the toggle live the way the six non-tool-call entries do.
///
/// Returns the error text, or the success shape, whichever came back.
async fn read_unknown_resource_across_extensions(agent: &Agent) -> String {
    match agent
        .extension_manager
        .read_resource_tool(
            serde_json::json!({ "uri": "nope://x" }),
            None,
            tokio_util::sync::CancellationToken::default(),
        )
        .await
    {
        Ok(ok) => format!("{ok:?}"),
        Err(e) => e.message.to_string(),
    }
}

/// Rows 5's subject: what discovery — and therefore the SYSTEM PROMPT — is
/// allowed to name. `get_extensions_info` carries a private server's own
/// instructions, so this covers Gate F2 as well as Gate E.
async fn extension_names_and_instructions(agent: &Agent) -> String {
    agent
        .extension_manager
        .get_extensions_info()
        .await
        .iter()
        .map(|info| format!("{}::{}", info.name, info.instructions))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Row 6's subject: Gate D, over a store this test owns.
async fn search_as(tier: ProviderTier, sm: &SessionManager, query: &str) -> usize {
    sm.search_chat_history(
        query,
        None,
        None,
        None,
        None,
        biorouter::session::chat_history_search::SearchReach::tier_only(tier),
    )
    .await
    .unwrap()
    .results
    .len()
}

/// Rows 7 and 18's subject: `chatrecall`'s LOAD guard, driven through the REAL
/// dispatch path so the capability is sampled by production code.
///
/// This is also the row that carries the visibility predicate. `visible_to` is
/// PURE — it reads no toggle and cannot — so the plan's ⚠ requires its row to be
/// asserted THROUGH a caller, and this is that caller: the guard it sits inside
/// is `cap.enforced() && !visible_to(cap.tier(), loaded.privacy_tier)`, so
/// flipping the master toggle must change what comes back here without
/// `visible_to` itself ever changing its answer.
///
/// Returns the tool's output text **whatever the outcome**, refusal included.
async fn chatrecall_load_as(agent: &Agent, caller: &Session, target_id: &str) -> String {
    let mut args = rmcp::model::JsonObject::new();
    args.insert(
        "session_id".into(),
        serde_json::Value::String(target_id.to_string()),
    );
    let (_id, result) = agent
        .dispatch_tool_call(
            CallToolRequestParams {
                task: None,
                name: "chatrecall__chatrecall".to_string().into(),
                arguments: Some(args),
                meta: None,
            },
            "req-chatrecall".to_string(),
            None,
            caller,
        )
        .await;
    match result {
        Ok(call) => match call.result.await {
            Ok(ok) => format!("{ok:?}"),
            Err(e) => e.message.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

// ── Row 13's fixtures: the spawn matrix ─────────────────────────────────────
//
// `apply_settings_overrides` makes THREE decisions that hang off one read of
// the master toggle — R4's refusal, DR-19's refusal and the private-extension
// filter — and each is asserted in both columns below. The filter is the one a
// flipped sense would ship green on: it is a `partition` predicate, so an
// inverted conjunct drops the wrong half rather than refusing anything.

/// What the spawning model asked for. The requested NAME is `ollama` for both
/// tiered asks, deliberately: §8.2 validates the CONSTRUCTED INSTANCE, and
/// `ollama` is the one registry entry that constructs with no credential and no
/// network *and* reads its tier off the resolved base URL — so one real
/// `providers::create` yields a Private instance pointed at this machine and a
/// Public one pointed off it, under the identical requested name.
fn spawn_params(named_provider: bool) -> biorouter::agents::subagent_tool::SubagentParams {
    let body = if named_provider {
        serde_json::json!({
            "instructions": "do the thing",
            "settings": { "provider": "ollama" }
        })
    } else {
        serde_json::json!({ "instructions": "do the thing" })
    };
    serde_json::from_value(body).unwrap()
}

/// Task-local config, which beats both the environment and the config file and
/// touches neither. BOTH poles are pinned, including the one that agrees with
/// the shipped default: leaving the private pole to the *absence* of
/// `OLLAMA_HOST` would let a developer machine with Ollama pointed off-box turn
/// this row vacuous.
fn ollama_host(private: bool) -> std::collections::HashMap<String, String> {
    let host = if private {
        "http://localhost:11434"
    } else {
        "https://ollama.example.com"
    };
    std::collections::HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())])
}

fn builtin_extension(name: &str) -> biorouter::agents::ExtensionConfig {
    biorouter::agents::ExtensionConfig::Builtin {
        name: name.to_string(),
        display_name: Some(name.to_string()),
        description: String::new(),
        timeout: Some(30),
        bundled: Some(true),
        available_tools: Vec::new(),
    }
}

/// One row of the spawn matrix, through the real resolver.
async fn resolve_child(
    parent: Arc<dyn Provider>,
    named_provider: bool,
    child_is_private: bool,
    extensions: Vec<biorouter::agents::ExtensionConfig>,
) -> anyhow::Result<biorouter::agents::TaskConfig> {
    let task_config = biorouter::agents::TaskConfig::new(
        parent,
        "parent-1",
        std::path::Path::new("."),
        extensions,
    );
    let params = spawn_params(named_provider);
    biorouter::config::with_config_overrides(
        ollama_host(child_is_private),
        biorouter::agents::subagent_tool::apply_settings_overrides(task_config, &params),
    )
    .await
}

// ─────────────────────────────────────────────────────────────────────────────

/// DR-15. Every enforcement point this crate can reach, asserted in BOTH toggle
/// positions, plus the copy INVARIANT asserted identically in both — carrying a
/// parent's stamp is column propagation, not a classification decision, and a
/// copy that laundered a private row to public while the feature was off would
/// do exactly that, durably, because re-enabling does not revisit it (AR-7).
///
/// The `on` column is what every other task already tests, restated here so this
/// test fails when a gate is wired to the toggle but broken, not only when it is
/// unwired.
#[tokio::test]
async fn the_master_toggle_governs_every_gate_in_both_directions() {
    // ---- ON: the shipped default. Every gate refuses. -----------------------
    // ⚠ `set_privacy_tiers` RETURNS AN RAII GUARD and is never called for
    //   effect: a bare setter here would leave the flag true-by-luck rather than
    //   true-by-construction.
    let on = set_privacy_tiers(true).await;

    // 1  A     (Task 12) — the bind lattice, through its production caller.
    let (_d1, agent1, s1) = agent_on(private_provider()).await;
    let sm1 = agent1.config.session_manager.clone();
    ratchet_to_private(&sm1, &s1.id).await;
    assert!(agent1
        .update_provider(public_provider(), &s1.id)
        .await
        .is_err());

    // 2  B     (Task 13) — the turn barrier.
    let (_d2, agent2, s2) = agent_on(public_provider()).await;
    let sm2 = agent2.config.session_manager.clone();
    ratchet_to_private(&sm2, &s2.id).await;
    assert!(turn_text(&agent2, &s2).await.contains(TURN_REFUSAL_MARKER));

    // 3  C     (Task 14) — tool dispatch into a private extension.
    //
    // BOTH markers, and the second is not decoration: the helper collapses every
    // failure into one string, so an unrelated dispatch error ("… not found")
    // would satisfy the ON column's `contains(PRIVATE_EXTENSION)` and the OFF
    // column's `!contains("private extension")` SIMULTANEOUSLY. Naming Gate C's
    // own sentence in the ON column is what makes the pair discriminating.
    let (_d3, agent3, s3) = agent_with_the_private_extension(public_provider()).await;
    let gate_c_on = call_private_tool_via_agent_loop(&agent3, &s3).await;
    assert!(gate_c_on.contains(PRIVATE_EXTENSION), "{gate_c_on}");
    assert!(gate_c_on.contains("private extension"), "{gate_c_on}");

    // 4  C'    (Task 15) — the RESOURCE surface. A different function on a
    //          different path from Gate C above: `dispatch_tool_call` never
    //          reaches `assert_extension_reachable`, so row 3 does not cover it.
    let gate_c_prime_on = read_private_resource(&agent3).await;
    assert!(
        gate_c_prime_on.contains("private extension"),
        "{gate_c_prime_on}"
    );

    // 4b C'    (2026-09-10 test drive, M6) — the SAME surface's fan-out branch.
    //          Row 4 asks the named branch and is blind to this one: the leak is
    //          not the refusal, it is the not-found message the fan-out composes
    //          after every private extension has already been refused.
    //
    //          Asserted against the LIVE map rather than against the constant, so
    //          the row cannot pass by naming the one extension the fixture
    //          happens to load. Every name in this fixture's map is private, so
    //          Gate E's verdict for a public caller is empty and the message may
    //          carry none of them.
    let loaded = agent3.extension_manager.list_extensions().await.unwrap();
    assert!(
        !loaded.is_empty(),
        "the fixture loaded nothing, so this row asserts nothing"
    );
    let fanout_on = read_unknown_resource_across_extensions(&agent3).await;
    for name in &loaded {
        assert!(
            !fanout_on.contains(name.as_str()),
            "a public caller was handed `{name}` in a not-found message — the set Gate E \
             exists to withhold: {fanout_on}"
        );
    }

    // 5+12 E+F2 (Tasks 16, 18) — discovery, and a private server's instructions
    //          in a public system prompt.
    assert!(!extension_names_and_instructions(&agent3)
        .await
        .contains(PRIVATE_EXTENSION));

    // 6  D     (Task 17) — chat history search.
    let (_d6, agent6, s6) = agent_on(private_provider()).await;
    let sm6 = agent6.config.session_manager.clone();
    sm6.add_message(&s6.id, &Message::user().with_text("cohort n=412"))
        .await
        .unwrap();
    ratchet_to_private(&sm6, &s6.id).await;
    assert_eq!(search_as(ProviderTier::Public, &sm6, "cohort").await, 0);

    // 7  LOAD  (Task 10) — `chatrecall`'s LOAD guard, and with it row 18's
    //          visibility predicate, asserted THROUGH a caller.
    //
    // The target's NAME is the assertion's subject in both directions: §11.4
    // classifies a session title as CONTENT (it is LLM-generated from the
    // conversation), so the ON column must not return it and the OFF column
    // must. Asserting on the title rather than on "did it error" is what keeps
    // the OFF column from passing on an unrelated failure.
    let (_d7, agent7, s7) = agent_on(public_provider()).await;
    let sm7 = agent7.config.session_manager.clone();
    agent7
        .extension_manager
        .add_extension(biorouter::agents::ExtensionConfig::Platform {
            name: "chatrecall".to_string(),
            description: biorouter::agents::extension::PLATFORM_EXTENSIONS["chatrecall"]
                .description
                .to_string(),
            bundled: None,
            available_tools: Vec::new(),
        })
        .await
        .expect("load the chatrecall platform extension");
    let target7 = sm7
        .create_session(
            PathBuf::from("."),
            PRIVATE_SESSION_TITLE.to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    // A message, or `handle_chatrecall` short-circuits on "has no messages"
    // BEFORE it builds the header the title lives in — and the OFF column would
    // then fail against a guard that is behaving perfectly. Measured, not
    // assumed: the first run of this row failed exactly there.
    sm7.add_message(&target7.id, &Message::user().with_text("cohort n=412"))
        .await
        .unwrap();
    ratchet_to_private(&sm7, &target7.id).await;
    let load_on = chatrecall_load_as(&agent7, &s7, &target7.id).await;
    assert!(!load_on.contains(PRIVATE_SESSION_TITLE), "{load_on}");
    assert!(load_on.contains("private"), "{load_on}");

    // 8  KB    (Task 10C) — the knowledge-base read barrier.
    let kb_dir = TempDir::new().unwrap();
    let path_root = kb_dir.path();
    let kb_root = kb_root_under(path_root);
    let kb_root = kb_root.as_path();
    // A REAL base, created through the production path that stamps its tier in
    // the same transaction — not a bare directory plus a hand-written tier
    // entry. `Catalog::discover` reads the registry and the manifest, so a
    // hand-made directory would leave row 14 passing vacuously in BOTH columns:
    // an empty catalog contains no private base either.
    biorouter_mcp::knowledge::service::KnowledgeService::new(kb_root.to_path_buf())
        .create_base_as(
            "omop",
            "OMOP",
            None,
            biorouter_mcp::knowledge::types::KbFormat::default(),
            /* caller_is_private */ true,
            // DR-26's axis is not what this row is about; `Unstated` is the
            // restrictive value and adds no owner, so the tier assertions below
            // stay the only thing under test.
            &biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated,
        )
        .expect("create the private base");
    assert!(biorouter_mcp::knowledge::tier::is_private(kb_root, "omop"));
    assert!(biorouter_mcp::knowledge::tier::assert_reachable(
        kb_root,
        "omop",
        false,
        &biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated,
    )
    .is_err());

    // 9  G     (Task 11) — conversation ingest.
    let private_row = sm6.get_session(&s6.id, false).await.unwrap();
    assert!(
        biorouter::knowledge::conversation_ingest::refuses_every_session(
            ProviderTier::Public,
            std::slice::from_ref(&private_row)
        )
    );

    // 10 H     (Task 19) — an alternate provider built outside the session bind.
    assert!(biorouter::privacy::assert_alt_provider_allowed(
        "plan mode",
        public_provider().as_ref(),
        SessionClassification::Private,
        "BIOROUTER_PLANNER_PROVIDER",
    )
    .is_err());

    // 13 spawn (Task 23) — the spawn matrix's THREE decisions, all hanging off
    //          one toggle read inside `apply_settings_overrides`.
    //
    // 13a R4: a public parent may not spawn a private child.
    let up_on = resolve_child(public_provider(), true, true, vec![])
        .await
        .expect_err("a public parent may not spawn a private child");
    assert!(
        matches!(
            up_on.downcast_ref::<biorouter::privacy::PrivacyRefusal>(),
            Some(biorouter::privacy::PrivacyRefusal::PrivateChildOfPublicParent { .. })
        ),
        "expected R4's typed refusal, got: {up_on}"
    );
    // 13b DR-19: a private parent may not spawn a child on a public model it
    //     named.
    let down_on = resolve_child(private_provider(), true, false, vec![])
        .await
        .expect_err("a private parent may not name a public model for its child");
    assert!(
        matches!(
            down_on.downcast_ref::<biorouter::privacy::PrivacyRefusal>(),
            Some(biorouter::privacy::PrivacyRefusal::PublicChildOfPrivateParent { .. })
        ),
        "expected DR-19's typed refusal, got: {down_on}"
    );
    // 13c the private-extension filter — a `partition`, so an inverted conjunct
    //     drops the wrong half rather than refusing anything, and neither column
    //     of 13a/13b would notice.
    let inherited_on = resolve_child(
        public_provider(),
        false,
        false,
        vec![
            builtin_extension(PRIVATE_EXTENSION),
            builtin_extension("developer"),
        ],
    )
    .await
    .expect("an inheriting spawn is permitted");
    assert_eq!(
        inherited_on
            .extensions
            .iter()
            .map(|e| e.name())
            .collect::<Vec<_>>(),
        vec!["developer".to_string()]
    );
    assert_eq!(
        inherited_on.dropped_private_extensions,
        vec![PRIVATE_EXTENSION.to_string()]
    );

    // 14 CP5   (Task 10D) — the Agent Drafter catalog's knowledge bases. The
    //          private base is omitted for a public caller, and the assertion is
    //          NOT vacuous: a private caller sees it in the same tree.
    assert!(catalog_kb_ids(path_root, &private_kb_caller()).contains(&"omop".to_string()));
    assert!(!catalog_kb_ids(path_root, &KbCaller::restricted()).contains(&"omop".to_string()));

    // 16 ratchet (Task 10B) — a private caller's write marks the base private.
    biorouter_mcp::knowledge::tier::raise_unlocked(kb_root, "notes", true).unwrap();
    assert!(biorouter_mcp::knowledge::tier::is_private(kb_root, "notes"));

    // 17 copy  (Task 22) — an INVARIANT, not a gate. Identical in both columns.
    //
    // Its own store: a copy lands in the same database as its original, and Gate
    // D's row above counts rows matching "cohort". Sharing `sm6` would make the
    // OFF column's search find two, which reads as a Gate D regression and is
    // really this row's side effect.
    let (_d17, agent17, s17) = agent_on(private_provider()).await;
    let sm17 = agent17.config.session_manager.clone();
    ratchet_to_private(&sm17, &s17.id).await;
    let copy_on = sm17
        .copy_session(&s17.id, "copy-on".to_string())
        .await
        .unwrap();
    assert_eq!(copy_on.privacy_tier, SessionClassification::Private);

    // 18 VIS   (Task 21) — ASSERTED THROUGH A CALLER, at row 7 above: `visible_to`
    //          is PURE, so the toggle reaches it only through
    //          `cap.enforced() && !visible_to(..)` in `handle_chatrecall`, and it
    //          is row 7's output that must change between the two columns.
    //
    //          The line below is NOT that row. It is a restatement of the
    //          predicate's own invariant — the toggle does not, and must not,
    //          change what `visible_to` answers — and it is written identically
    //          in both columns for exactly that reason. Read on its own it can
    //          never fail for the reason row 18 exists; row 7 is what can.
    assert!(!biorouter::privacy::visible_to(
        ProviderTier::Public,
        SessionClassification::Private
    ));

    // ---- OFF: nothing is refused. ------------------------------------------
    drop(on); // ← restores the previous value; the guard below then owns it
    let _off = set_privacy_tiers(false).await;

    assert!(agent1
        .update_provider(public_provider(), &s1.id)
        .await
        .is_ok());
    assert!(!turn_text(&agent2, &s2).await.contains(TURN_REFUSAL_MARKER));
    assert!(!call_private_tool_via_agent_loop(&agent3, &s3)
        .await
        .contains("private extension"));
    // 4 C': the resource surface stops refusing too. It still fails — the
    //   fixture server serves no resources — but on its OWN terms, not Gate C''s,
    //   which is the distinction this pair exists to make.
    let gate_c_prime_off = read_private_resource(&agent3).await;
    assert!(
        !gate_c_prime_off.contains("private extension"),
        "{gate_c_prime_off}"
    );
    assert!(extension_names_and_instructions(&agent3)
        .await
        .contains(PRIVATE_EXTENSION));
    // 4b: the fan-out's roster stops being filtered too, which is what makes the
    //   ON column above a privacy filter rather than a message that never names
    //   anything. `allowed_extension_keys` returns every key with the toggle
    //   off, so the private fixture is back in the list.
    let fanout_off = read_unknown_resource_across_extensions(&agent3).await;
    assert!(
        fanout_off.contains(PRIVATE_EXTENSION),
        "the fan-out named nothing with privacy tiers OFF, so the ON column would pass \
         against an implementation that simply deleted the roster: {fanout_off}"
    );

    assert_eq!(search_as(ProviderTier::Public, &sm6, "cohort").await, 1);
    // 7 + 18: the same public session now reads the private chat, title and all.
    let load_off = chatrecall_load_as(&agent7, &s7, &target7.id).await;
    assert!(
        load_off.contains(PRIVATE_SESSION_TITLE),
        "the chatrecall LOAD guard was still armed with privacy tiers off: {load_off}"
    );
    assert!(biorouter_mcp::knowledge::tier::assert_reachable(
        kb_root,
        "omop",
        false,
        &biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated,
    )
    .is_ok());
    assert!(
        !biorouter::knowledge::conversation_ingest::refuses_every_session(
            ProviderTier::Public,
            std::slice::from_ref(&private_row)
        )
    );
    assert!(biorouter::privacy::assert_alt_provider_allowed(
        "plan mode",
        public_provider().as_ref(),
        SessionClassification::Private,
        "BIOROUTER_PLANNER_PROVIDER",
    )
    .is_ok());
    // 13 spawn: all three decisions go quiet, and the third is the one a flipped
    //    `partition` sense would hide — the child now inherits the private
    //    extension instead of losing it.
    let up_off = resolve_child(public_provider(), true, true, vec![])
        .await
        .expect("with privacy tiers off, a public parent may spawn a private child");
    assert_eq!(up_off.privacy_tier, SessionClassification::Private);
    let down_off = resolve_child(private_provider(), true, false, vec![])
        .await
        .expect("with privacy tiers off, a private parent may name a public model");
    assert_eq!(down_off.privacy_tier, SessionClassification::Public);
    let inherited_off = resolve_child(
        public_provider(),
        false,
        false,
        vec![
            builtin_extension(PRIVATE_EXTENSION),
            builtin_extension("developer"),
        ],
    )
    .await
    .expect("an inheriting spawn is permitted");
    assert_eq!(
        inherited_off
            .extensions
            .iter()
            .map(|e| e.name())
            .collect::<Vec<_>>(),
        vec![PRIVATE_EXTENSION.to_string(), "developer".to_string()]
    );
    assert!(inherited_off.dropped_private_extensions.is_empty());
    assert!(catalog_kb_ids(path_root, &KbCaller::restricted()).contains(&"omop".to_string()));
    // AR-7: the ratchet stops too.
    biorouter_mcp::knowledge::tier::raise_unlocked(kb_root, "notes2", true).unwrap();
    assert!(!biorouter_mcp::knowledge::tier::is_private(
        kb_root, "notes2"
    ));
    // ⚠ Row 17 is IDENTICAL in both columns, on purpose. The toggle stops
    //   ENFORCEMENT; it does not delete the columns or rewrite the stamps
    //   already written, and a copy that laundered a private row to public while
    //   the feature was off would do exactly that — durably, because re-enabling
    //   does not revisit it (AR-7). Row 16 differs because a ratchet *writes a
    //   new* classification, which is the thing AR-7 says stops happening.
    let copy_off = sm17
        .copy_session(&s17.id, "copy-off".to_string())
        .await
        .unwrap();
    assert_eq!(copy_off.privacy_tier, SessionClassification::Private);
    // …and the pure predicate's own answer is unchanged: the toggle reaches it
    //   through its callers, never inside it.
    assert!(!biorouter::privacy::visible_to(
        ProviderTier::Public,
        SessionClassification::Private
    ));
}

/// AR-7, as an assertion rather than a paragraph: with the toggle off the ratchet
/// does not fire, and turning it back on does not go back and fix it. This is the
/// one behaviour a reader is most likely to assume works the other way, so it is
/// pinned rather than described.
#[tokio::test]
async fn nothing_ratchets_while_the_toggle_is_off_and_re_enabling_does_not_backfill() {
    let off = set_privacy_tiers(false).await;
    let (_dir, agent, s) = agent_with_the_private_extension(private_provider()).await;
    let sm = agent.config.session_manager.clone();
    // DR-4's two triggers: a permitted private-extension dispatch, and a turn.
    let _ = call_private_tool_via_agent_loop(&agent, &s).await;
    let _ = turn_text(&agent, &s).await;
    assert_eq!(
        sm.get_session(&s.id, false).await.unwrap().privacy_tier,
        SessionClassification::Public,
        "DR-4's two triggers must not fire while the feature is off"
    );

    drop(off);
    let _on = set_privacy_tiers(true).await;
    assert_eq!(
        sm.get_session(&s.id, false).await.unwrap().privacy_tier,
        SessionClassification::Public,
        "re-enabling must not retro-classify; there is no content scan (AR-7)"
    );
}

/// The failure mode is an agent disabling its own protection, and
/// `Config::get_param`'s env branch is the easiest lever in the tree. The
/// resolution reads a record on disk instead.
///
/// ⚠ **Run against a SCRATCH configuration directory, and that changed in Task
/// 42.** It used to call `load_privacy_tiers_from_config`, which only read
/// whatever `config.yaml` `Config::global()` had resolved — harmless, and
/// impossible to redirect, since `Config::global()` is a `OnceCell` the matrix
/// above has already initialised. DR-22 made the resolution *write*: it migrates
/// the retired key into its own record on first run. Left as it was, this test
/// would create `privacy-tiers.json` in the developer's real
/// `~/.config/biorouter` and delete a key out of their real `config.yaml` — the
/// exact side effect `privacy_toggle_config.rs`'s fixture doc-comment is a page
/// long about. `resolve_privacy_tiers` takes the `Config` explicitly for this
/// reason, and it is the whole body of the loader.
#[tokio::test]
async fn no_environment_variable_can_turn_protection_off() {
    let _g = set_privacy_tiers(true).await;
    let scratch = TempDir::new().expect("scratch config dir");
    let config = biorouter::config::Config::new_with_file_secrets(
        scratch.path().join("config.yaml"),
        scratch.path().join("secrets.yaml"),
    )
    .expect("scratch config");
    let _env = env_lock::lock_env([("BIOROUTER_PRIVACY_TIERS", Some("off"))]);
    assert!(
        biorouter::privacy::resolve_privacy_tiers(&config),
        "an env var disabled the whole feature"
    );

    // …and the same resolution DOES honour a record on disk, so the assertion
    // above is about the environment rather than about a resolution that can
    // never say `off` at all.
    biorouter::privacy::master_switch::write_for(&config, false).expect("record the switch");
    assert!(
        !biorouter::privacy::resolve_privacy_tiers(&config),
        "the switch's own record is what the resolution reads"
    );
}

/// The guard on the naming rule above, spelled as an assertion because the
/// failure it prevents cannot be seen any other way.
///
/// A double whose name IS a registry entry does not fail this binary — it
/// **hangs** it, inside `providers::create`, on an OS credential prompt. Cargo
/// reports nothing at all for a binary that never finishes and simply waits, so
/// the whole workspace run stalls with no test named and no output; the last
/// occurrence sat behind an unrelated failing binary for as long as that binary
/// kept aborting the run first. This test turns that into a one-line failure in
/// under a second.
///
/// Enumerating the registry does NOT construct anything: `providers()` reads
/// metadata, and the declarative loader it initialises reads provider
/// *definitions* only — no secret, no keyring, no host.
#[tokio::test]
async fn the_matrix_doubles_are_not_registry_providers() {
    let registered: Vec<String> = biorouter::providers::providers()
        .await
        .into_iter()
        .map(|(metadata, _)| metadata.name)
        .collect();
    // Not vacuous: an empty or unbuilt registry would clear every name below
    // while the hang came straight back. `anthropic` is the entry the doubles
    // used to be named after, and the one whose `from_env` reads the keyring.
    assert!(
        registered.iter().any(|name| name == "anthropic"),
        "the registry did not enumerate, so the absence check below proves nothing: {registered:?}"
    );
    for double in [PUBLIC_DOUBLE, PRIVATE_DOUBLE] {
        assert!(
            !registered.contains(&double.to_string()),
            "the test double `{double}` collides with a real provider, so Gate B's \
             repair arm will construct that provider for real and block this binary \
             on the developer's OS keyring; registered: {registered:?}"
        );
    }
}

/// The knowledge bases the Agent Drafter catalog offers a caller.
///
/// `Catalog::discover` resolves its own root through `BIOROUTER_PATH_ROOT`, so
/// the env guard is taken here — inside a SYNCHRONOUS helper — rather than
/// around the `await`s in the matrix: `env_lock`'s guard is a `std::sync` guard
/// and holding one across an `.await` is not `Send` in a `#[tokio::test]`.
fn catalog_kb_ids(path_root: &std::path::Path, caller: &KbCaller) -> Vec<String> {
    let _env = env_lock::lock_env([(
        "BIOROUTER_PATH_ROOT",
        Some(path_root.to_str().expect("utf-8 temp path")),
    )]);
    biorouter_mcp::agent_drafter::catalog::Catalog::discover(caller)
        .knowledge_bases
        .into_iter()
        .map(|kb| kb.id)
        .collect()
}

/// The two callers the CP5 rows use.
///
/// ⚠ The private one is `Local`, not `Unstated`. After audit finding 17 the
/// catalog asks `tier::assert_reachable`, which reads DR-26's affiliation axis
/// as well as the tier — and `Unstated` is that axis's RESTRICTIVE value, so a
/// row that meant "a private caller sees the private base" and spelled it
/// `Unstated` would pass for the wrong reason the day the base gains an owner.
/// `Local` transfers nothing and clears every base, which is what
/// `caller_is_private = true` meant before the axis existed.
fn private_kb_caller() -> KbCaller {
    KbCaller::new(
        true,
        biorouter_mcp::knowledge::affiliation::CallerAffiliation::Local,
    )
}

/// The knowledge root `Catalog::discover` will resolve under `path_root`, so the
/// matrix stamps tiers on the same tree the catalog reads.
fn kb_root_under(path_root: &std::path::Path) -> std::path::PathBuf {
    let _env = env_lock::lock_env([(
        "BIOROUTER_PATH_ROOT",
        Some(path_root.to_str().expect("utf-8 temp path")),
    )]);
    let root = biorouter_mcp::knowledge::paths::knowledge_root().expect("knowledge root");
    std::fs::create_dir_all(&root).expect("create knowledge root");
    root
}
