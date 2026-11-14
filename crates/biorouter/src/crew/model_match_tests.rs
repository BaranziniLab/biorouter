//! Coding-agent parity (design CP-1 to CP-3, `docs/crew/design/coding-agent-parity.md`): Crew
//! admits a model by privacy alone, through [`check`], the same for every provider.
//!
//! ⚠ **Crew admission; a change here needs human review.**

use super::*;
use crate::crew::{provider_binding, refusal, CrewManager, RunPolicy};
use crate::model::ModelConfig;
use crate::privacy::affiliation::InstitutionId;
use crate::privacy::decision_axes::{DecisionAxes, HostLabel, LoweredBy, NameSource};
use crate::providers::claude_code::ClaudeCodeProvider;
use crate::providers::codex::CodexProvider;
use crate::providers::decision_bound::DecisionBoundProvider;
use crate::session::{session_manager::SessionType, SessionManager};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// The code of `result`'s refusal: `None` when it passed, `untyped` for an untyped error.
fn code(result: Result<()>) -> Option<String> {
    let error = result.err()?;
    Some(
        CrewRefusal::find(&error)
            .map_or_else(|| "untyped".to_owned(), |found| found.code().to_owned()),
    )
}

/// Whether a refusal's sentence names a kind of provider rather than a privacy mismatch.
fn names_a_provider_kind(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    [
        "claude code",
        "codex",
        "coding agent",
        "external tools",
        "isolate",
        "tool bridge",
    ]
    .iter()
    .any(|kind| lower.contains(kind))
}

fn connection(mode: ClusterMode) -> Connection {
    Connection {
        id: "parity-connection".into(),
        node_id: None,
        name: "methods".into(),
        ssh_target: "crew@crew.invalid".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/parity.sock".into(),
        owner_uid: 10001,
        workspace_id: "workspace-methods".into(),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        remote_access: Default::default(),
        cluster_connection_id: "cluster-methods".into(),
        mode,
        institution_id: None,
        policy_epoch: 1,
        status: "disconnected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    }
}

fn scope(
    binding: String,
    public_provider: bool,
    origin_restricted: bool,
    institution_ids: &[&str],
    incarnation: Option<i64>,
    store: Option<String>,
) -> Scope {
    Scope {
        connection_id: "parity-connection".into(),
        run_id: "parity-run".into(),
        channel_id: "channel-methods".into(),
        source_channels: vec!["channel-methods".into()],
        epoch: 1,
        provider_binding: binding,
        public_provider,
        origin_restricted,
        institution_ids: institution_ids.iter().map(|id| (*id).to_owned()).collect(),
        institution_policy: true,
        expired: false,
        expires_at: None,
        labels: None,
        session_incarnation: incarnation,
        session_store: store,
        revocation: None,
    }
}

fn public() -> ModelPrivacy {
    ModelPrivacy::new(ProviderTier::Public, None)
}

fn ucsf() -> ModelPrivacy {
    ModelPrivacy::new(
        ProviderTier::Private,
        Some(ModelAffiliation::institution(InstitutionId::new("ucsf"))),
    )
}

/// CP-1, the table: one rule for every model, whatever provider it is. A private place refuses
/// a public model; a public one admits it unless what it reads is restricted or the chat was
/// already private; a private model reads what its affiliation covers.
#[test]
fn one_rule_decides_every_crew_door() {
    let private_connection = connection(ClusterMode::Private);
    let public_connection = connection(ClusterMode::Public);
    let none = BTreeSet::new();
    let owned_by_ucsf: BTreeSet<String> = BTreeSet::from(["ucsf".to_owned()]);
    let refused = Some(refusal::PUBLIC_MODEL_REFUSED.to_owned());
    let mismatch = Some(refusal::INSTITUTION_MISMATCH.to_owned());

    // A standing grant.
    let open_grant = scope(String::new(), true, false, &[], None, None);
    let private_origin = scope(String::new(), true, true, &[], None, None);
    let private_grant = scope(String::new(), false, false, &["ucsf"], None, None);
    let grant = |model: &ModelPrivacy, scope: &Scope, connection: &Connection| {
        code(check(model, CrewPlace::Grant { scope, connection }))
    };
    assert_eq!(grant(&public(), &open_grant, &public_connection), None);
    assert_eq!(grant(&public(), &open_grant, &private_connection), refused);
    assert_eq!(
        grant(&public(), &private_origin, &public_connection),
        refused
    );
    assert_eq!(
        grant(&public(), &private_grant, &public_connection),
        refused
    );
    assert_eq!(grant(&ucsf(), &private_grant, &private_connection), None);
    assert_eq!(grant(&ucsf(), &open_grant, &public_connection), None);
    let other = ModelPrivacy::new(
        ProviderTier::Private,
        Some(ModelAffiliation::institution(InstitutionId::new(
            "stanford",
        ))),
    );
    assert_eq!(grant(&other, &private_grant, &private_connection), mismatch);

    // A run asked on a connection.
    let asked = |model: &ModelPrivacy, connection: &Connection, origin_restricted: bool| {
        code(check(
            model,
            CrewPlace::Connection {
                connection,
                workspace: "methods",
                origin_restricted,
            },
        ))
    };
    assert_eq!(asked(&public(), &public_connection, false), None);
    assert_eq!(asked(&public(), &private_connection, false), refused);
    assert_eq!(asked(&public(), &public_connection, true), refused);
    assert_eq!(asked(&ucsf(), &private_connection, true), None);

    // The workspace's answer.
    let answered = |model: &ModelPrivacy, mode: ClusterMode, restricted: bool| {
        code(check(
            model,
            CrewPlace::Workspace {
                label: "methods",
                mode,
                restricted,
            },
        ))
    };
    assert_eq!(answered(&public(), ClusterMode::Public, false), None);
    assert_eq!(answered(&public(), ClusterMode::Private, false), refused);
    assert_eq!(answered(&public(), ClusterMode::Public, true), refused);
    assert_eq!(answered(&ucsf(), ClusterMode::Private, true), None);

    // Ownership.
    let owned = |model: &ModelPrivacy, ids: &BTreeSet<String>| {
        code(check(
            model,
            CrewPlace::Owned {
                institution_ids: ids,
            },
        ))
    };
    assert_eq!(owned(&public(), &none), None);
    assert_eq!(owned(&public(), &owned_by_ucsf), refused);
    assert_eq!(owned(&ucsf(), &owned_by_ucsf), None);
    assert_eq!(owned(&other, &owned_by_ucsf), mismatch);
}

/// CP-3: every refusal is said as a privacy mismatch, never as a kind of provider.
#[test]
fn no_refusal_names_a_provider_kind() {
    let private_connection = connection(ClusterMode::Private);
    let public_connection = connection(ClusterMode::Public);
    let private_grant = scope(String::new(), false, true, &["ucsf"], None, None);
    let owned = BTreeSet::from(["ucsf".to_owned()]);
    let refusals = [
        check(
            &public(),
            CrewPlace::Grant {
                scope: &private_grant,
                connection: &public_connection,
            },
        ),
        check(
            &public(),
            CrewPlace::Connection {
                connection: &private_connection,
                workspace: "methods",
                origin_restricted: false,
            },
        ),
        check(
            &public(),
            CrewPlace::Connection {
                connection: &public_connection,
                workspace: "methods",
                origin_restricted: true,
            },
        ),
        check(
            &public(),
            CrewPlace::Workspace {
                label: "methods",
                mode: ClusterMode::Private,
                restricted: false,
            },
        ),
        check(
            &public(),
            CrewPlace::Workspace {
                label: "methods",
                mode: ClusterMode::Public,
                restricted: true,
            },
        ),
        check(
            &public(),
            CrewPlace::Owned {
                institution_ids: &owned,
            },
        ),
    ];
    for refusal in refusals {
        let sentence = refusal.expect_err("refused").to_string();
        assert!(sentence.contains("public model"), "{sentence}");
        assert!(!names_a_provider_kind(&sentence), "{sentence}");
    }
}

/// One device: a session store, a saved chat in it, a Crew registry that resolves chats against
/// it, and one connection of `mode`.
struct Device {
    _data: TempDir,
    _crew_root: TempDir,
    crew: CrewManager,
    chat: String,
    incarnation: i64,
}

impl Device {
    async fn new(mode: ClusterMode) -> Self {
        let data = TempDir::new().unwrap();
        let crew_root = TempDir::new().unwrap();
        let store = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let crew = CrewManager::new(crew_root.path().to_path_buf()).unwrap();
        crew.use_session_store(store.clone());
        let chat = store
            .create_session(data.path().to_path_buf(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let incarnation = store.session_incarnation(&chat).await.unwrap().unwrap();
        crew.registry
            .lock()
            .await
            .connections
            .push(connection(mode));
        Self {
            _data: data,
            _crew_root: crew_root,
            crew,
            chat,
            incarnation,
        }
    }

    /// Grant the chat access bound to `provider`, as the workspace admitted it: a public model
    /// is admitted (`public_provider`) only where the connection is Public.
    async fn grant(&self, provider: &dyn Provider, institutions: &[&str]) {
        let public = ModelPrivacy::of(provider).is_public();
        let store = self.crew.own_store();
        self.crew.registry.lock().await.scopes.insert(
            self.chat.clone(),
            scope(
                provider_binding(provider),
                public,
                false,
                institutions,
                Some(self.incarnation),
                store,
            ),
        );
    }
}

fn claude_code() -> Arc<dyn Provider> {
    Arc::new(ClaudeCodeProvider::for_tests(
        PathBuf::from("/nonexistent/claude"),
        "claude-sonnet-4-6",
    ))
}

fn codex() -> Arc<dyn Provider> {
    Arc::new(CodexProvider::for_tests(
        PathBuf::from("/nonexistent/codex"),
        "gpt-5.5",
    ))
}

fn versa() -> Arc<dyn Provider> {
    use crate::providers::provider_binding::{SecretFreeEndpoint, VersaAzureCredentialSource};
    Arc::new(
        crate::providers::versa_azure::VersaAzureProvider::from_resolved(
            ModelConfig::new_or_fail("gpt-5.5"),
            SecretFreeEndpoint::new(crate::providers::versa_azure::VERSA_AZURE_ENDPOINT.into())
                .unwrap(),
            "gpt-5.5".into(),
            "2025-04-01-preview".into(),
            VersaAzureCredentialSource::ApiKey,
        )
        .unwrap(),
    )
}

/// Versa with a public decision model (a Baseten monitor): DR-33 makes the chat public.
fn versa_with_a_public_decision_model() -> Arc<dyn Provider> {
    DecisionBoundProvider::bind(
        versa(),
        DecisionAxes::public_for_test(LoweredBy {
            source: NameSource::Monitor,
            label: "Done".to_string(),
            model: "joey-1".to_string(),
            host: HostLabel::Baseten,
            others: 0,
        }),
    )
}

/// The owner's ruling, at the doors a chat's grant passes (CP-1): a public workspace admits
/// Claude Code and Codex like any public model, and a private one refuses them with the
/// sentence any public model reads, never one about the provider. Before parity both doors
/// refused the two providers by kind (`uses_tool_bridge`) whatever the workspace.
#[tokio::test]
async fn claude_code_and_codex_meet_crew_as_public_models() {
    for provider in [claude_code(), codex()] {
        let name = provider.get_name().to_owned();
        assert!(
            provider.uses_tool_bridge(),
            "{name}: precondition, a bridged provider"
        );
        assert!(
            ModelPrivacy::of(provider.as_ref()).is_public(),
            "{name}: precondition, a public provider"
        );

        let public_workspace = Device::new(ClusterMode::Public).await;
        public_workspace.grant(provider.as_ref(), &[]).await;
        public_workspace
            .crew
            .check_provider_binding(&public_workspace.chat, provider.as_ref())
            .await
            .unwrap_or_else(|error| panic!("{name} in a public workspace: {error}"));
        public_workspace
            .crew
            .check_dispatch(
                &public_workspace.chat,
                &CallCapability::for_test(ProviderTier::Public, true),
            )
            .await
            .unwrap_or_else(|error| panic!("{name}'s bridged call in a public workspace: {error}"));
        // A run asked on the public connection goes past the privacy checks to the workspace,
        // which this device cannot reach: refused for that, never for the model.
        let asked = public_workspace
            .crew
            .preflight_run(
                "parity-connection",
                "channel-methods",
                &[],
                provider.as_ref(),
                &RunPolicy::default(),
            )
            .await
            .expect_err("no workspace to ask");
        assert_ne!(
            CrewRefusal::find(&asked).map(CrewRefusal::code),
            Some(refusal::PUBLIC_MODEL_REFUSED),
            "{name}: {asked}"
        );
        assert!(
            !names_a_provider_kind(&asked.to_string()),
            "{name}: {asked}"
        );

        let private_workspace = Device::new(ClusterMode::Private).await;
        private_workspace.grant(provider.as_ref(), &[]).await;
        for refused in [
            private_workspace
                .crew
                .check_provider_binding(&private_workspace.chat, provider.as_ref())
                .await,
            private_workspace
                .crew
                .preflight_run(
                    "parity-connection",
                    "channel-methods",
                    &[],
                    provider.as_ref(),
                    &RunPolicy::default(),
                )
                .await
                .map(|_| ()),
        ] {
            let error = refused.expect_err("a private workspace refuses a public model");
            assert_eq!(
                CrewRefusal::find(&error).map(CrewRefusal::code),
                Some(refusal::PUBLIC_MODEL_REFUSED),
                "{name}: {error}"
            );
            assert!(
                !names_a_provider_kind(&error.to_string()),
                "{name}: {error}"
            );
        }
    }
}

/// The same rule for a private model and for a chat its decision model made public (DR-33):
/// Versa reads a private workspace its affiliation covers and a public one; Versa with a
/// Baseten decision model is a public chat, refused a private workspace and admitted to a
/// public one exactly as Claude Code is.
#[tokio::test]
async fn versa_and_a_chat_made_public_by_its_decision_model_meet_the_same_rule() {
    crate::config::with_config_overrides(
        std::collections::HashMap::from([(
            "VERSA_AZURE_API_KEY".to_string(),
            "parity-key".to_string(),
        )]),
        async {
            let versa = versa();
            assert!(
                !ModelPrivacy::of(versa.as_ref()).is_public(),
                "precondition"
            );
            for mode in [ClusterMode::Private, ClusterMode::Public] {
                let device = Device::new(mode).await;
                device.grant(versa.as_ref(), &["ucsf"]).await;
                device
                    .crew
                    .check_provider_binding(&device.chat, versa.as_ref())
                    .await
                    .unwrap_or_else(|error| panic!("Versa in a {mode:?} workspace: {error}"));
            }

            let lowered = versa_with_a_public_decision_model();
            assert!(
                ModelPrivacy::of(lowered.as_ref()).is_public(),
                "precondition: DR-33 makes the chat public"
            );
            let private_workspace = Device::new(ClusterMode::Private).await;
            let refused = private_workspace
                .crew
                .preflight_run(
                    "parity-connection",
                    "channel-methods",
                    &[],
                    lowered.as_ref(),
                    &RunPolicy::default(),
                )
                .await
                .expect_err("a private workspace refuses a public chat");
            assert_eq!(
                CrewRefusal::find(&refused).map(CrewRefusal::code),
                Some(refusal::PUBLIC_MODEL_REFUSED),
                "{refused}"
            );
            // A private grant asked of the public chat: refused, as Claude Code's is.
            private_workspace.grant(versa.as_ref(), &["ucsf"]).await;
            assert_eq!(
                code(
                    private_workspace
                        .crew
                        .check_dispatch(
                            &private_workspace.chat,
                            &CallCapability::for_test(ProviderTier::Public, true),
                        )
                        .await
                ),
                Some(refusal::PUBLIC_MODEL_REFUSED.to_owned())
            );

            let public_workspace = Device::new(ClusterMode::Public).await;
            public_workspace.grant(lowered.as_ref(), &[]).await;
            public_workspace
                .crew
                .check_provider_binding(&public_workspace.chat, lowered.as_ref())
                .await
                .unwrap_or_else(|error| panic!("a public chat in a public workspace: {error}"));
        },
    )
    .await;
}

/// `text` without its comment lines and its test-only items. Each `#[cfg(test)]` item is
/// dropped whole: a block (a test module, a test helper) to its closing brace, a one-line item
/// (`mod tests;`, a `use`) by itself; the rest of the file is kept. Braces are counted plainly,
/// which [`no_crew_door_asks_the_provider_kind`]'s size floors keep honest: a strip that ate a
/// file's production code would fall below them.
fn without_test_items(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::with_capacity(lines.len());
    let mut at = 0;
    while at < lines.len() {
        if lines[at].trim() != "#[cfg(test)]" {
            if !lines[at].trim_start().starts_with("//") {
                kept.push(lines[at]);
            }
            at += 1;
            continue;
        }
        at += 1;
        while at < lines.len() && lines[at].trim_start().starts_with("#[") {
            at += 1;
        }
        let (mut depth, mut opened) = (0i64, false);
        while at < lines.len() {
            let line = lines[at];
            for c in line.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            at += 1;
            if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                break;
            }
        }
    }
    kept.join("\n")
}

/// The Crew source files whose code may decide who reads Crew: the core, the server's Crew
/// routes and the person tools, and the Crew capability's dispatch. Test files and test
/// modules are left out, and so are comment lines.
fn crew_code() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the repository root")
        .to_path_buf();
    let mut files = Vec::new();
    let mut walk = vec![
        root.join("crates/biorouter/src/crew"),
        root.join("crates/biorouter-server/src/routes/crew_tools"),
    ];
    while let Some(dir) = walk.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    for entry in std::fs::read_dir(root.join("crates/biorouter-server/src/routes")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("crew") && name.ends_with(".rs") {
            files.push(path);
        }
    }
    files.push(root.join("crates/biorouter/src/agents/crew_extension.rs"));
    files
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            !name.ends_with("_tests.rs") && name != "tests.rs"
        })
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let code = without_test_items(&text);
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (relative, code)
        })
        .collect()
}

/// CP-2 and CP-3, held by the source: no Crew code asks what kind of provider a model is, and
/// only this module compares a tier with Public. A door that needs to know whether a model may
/// read asks [`check`], or [`ModelPrivacy::is_public`] for the person tools' read scope.
#[test]
fn no_crew_door_asks_the_provider_kind() {
    let files = crew_code();
    assert!(
        files.len() > 20
            && files.iter().any(|(path, _)| path.ends_with("crew/mod.rs"))
            && files
                .iter()
                .any(|(path, _)| path.ends_with("routes/crew.rs"))
            && files
                .iter()
                .any(|(path, _)| path.ends_with("crew_tools/tasks.rs")),
        "read only {:?}",
        files.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );
    // Each key file keeps its production code (measured 2026-11-12: crew/mod.rs 6,434 lines,
    // routes/crew.rs 2,486, crew_extension.rs 583), so a strip that ate one fails here rather
    // than scanning nothing.
    for (file, floor) in [
        ("crates/biorouter/src/crew/mod.rs", 5_000),
        ("crates/biorouter-server/src/routes/crew.rs", 1_800),
        ("crates/biorouter/src/agents/crew_extension.rs", 400),
        (
            "crates/biorouter-server/src/routes/crew_tools/tasks.rs",
            300,
        ),
    ] {
        let lines = files
            .iter()
            .find(|(path, _)| path == file)
            .map_or(0, |(_, code)| code.lines().count());
        assert!(lines > floor, "{file}: only {lines} lines were scanned");
    }
    let tier_comparison = regex::Regex::new(
        r"[=!]=\s*ProviderTier::Public|ProviderTier::Public\s*[=!]=|matches!\([^)]*ProviderTier::Public",
    )
    .unwrap();
    let mut offenders = Vec::new();
    for (path, code) in &files {
        for kind in [
            "uses_tool_bridge",
            "provider_uses_bridge",
            "\"claude_code\"",
            "\"codex\"",
            "CODING_AGENT",
        ] {
            if code.contains(kind) {
                offenders.push(format!("{path} asks {kind}"));
            }
        }
        if !path.ends_with("crew/model_match.rs") && tier_comparison.is_match(code) {
            offenders.push(format!("{path} compares a tier with Public"));
        }
    }
    assert!(
        offenders.is_empty(),
        "Crew decides who may read by privacy alone, in crew::model_match \
         (docs/crew/design/coding-agent-parity.md, CP-2): {offenders:#?}"
    );
}

/// CP-6: no Crew call a coding agent's child makes over the tool bridge outlasts the bridge's
/// budget for one call, so long Crew work returns a handle and is polled, and the bridge's
/// limit is never what ends one. Each request to the workspace waits at most
/// `transport::REQUEST_DEADLINE`; a file copy settles for at most `TRANSFER_SETTLE_WAIT` and
/// then answers its job, polling at most once more past that; a command on the server answers
/// its job within the broker's own 20-second wait, inside the request's deadline; and the person
/// tools cut every card and job wait to the call's remaining time (`within_call`).
#[test]
fn every_crew_wait_fits_inside_a_bridged_call() {
    let budget = crate::providers::coding_agent::bridge::child_tool_call_budget();
    // The margin the person tools keep (`BRIDGE_MARGIN`) and the parked-card limit both sit
    // 30 seconds under the budget.
    let usable = budget - std::time::Duration::from_secs(30);
    let longest_request = super::super::transport::REQUEST_DEADLINE;
    let longest_transfer = longest_request + super::super::TRANSFER_SETTLE_WAIT + longest_request;
    assert!(
        longest_transfer < usable,
        "a Crew transfer can wait {longest_transfer:?}, past the bridge's {usable:?}"
    );
    assert!(
        super::super::TRANSFER_POLL_EVERY < super::super::TRANSFER_SETTLE_WAIT,
        "a transfer polls before it answers its job"
    );
}
