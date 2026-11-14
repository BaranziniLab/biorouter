//! Issue #117: **the credential must not be anywhere the model can reach.**
//!
//! Every test here asserts an absence. That is deliberate: an implementation
//! that renders the dialog perfectly, stores the value correctly and *also*
//! writes it into `config.yaml` would pass every functional test ever written
//! for this feature. So these look at the four places a value could end up —
//! the plaintext config, a published card, a parked call's outcome, and a
//! diagnostics bundle — and require it in none of them.
//!
//! Runs in its own binary with `BIOROUTER_PATH_ROOT` pointed at a temp tree, so
//! it reads and writes a sandbox rather than the developer's real config.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::collections::HashMap;
use std::sync::OnceLock;

use biorouter::conversation::message::{SecretDestination, SecretKeyRequest};
use biorouter::extension_install::{
    classify_supplied, compose_config, submit_credentials, BrxtEnvVar, BrxtManifest,
    CredentialSpec, SubmitOutcome,
};
use biorouter::pending_user_action::{
    PendingUserActions, ResolveOutcome, SecretsRequest, UserActionOutcome, UserActionRequest,
};

/// The value that must never turn up anywhere below.
const SECRET: &str = "sk-live-please-do-not-leak-this-anywhere";

/// One sandbox for the whole binary. `Config::global()` resolves its paths once,
/// on first use, so this has to be set before any test touches it — and every
/// test calls it first.
fn sandbox() -> &'static std::path::Path {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::env::set_var("BIOROUTER_PATH_ROOT", dir.path());
        // Plaintext `secrets.yaml` instead of the OS store: the assertion is
        // about what reaches `config.yaml`, and a test must never write into the
        // developer's real keychain to make it.
        std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
        for sub in ["config", "data", "state"] {
            std::fs::create_dir_all(dir.path().join(sub)).expect("a sandbox tree");
        }
        // `Config` opens these on first read; an absent file is an error, not an
        // empty config.
        std::fs::write(dir.path().join("config/config.yaml"), "{}\n").expect("a config file");
        std::fs::write(dir.path().join("config/secrets.yaml"), "{}\n").expect("a secrets file");
        dir
    });
    dir.path()
}

fn var(key: &str, required: bool, secret: bool) -> BrxtEnvVar {
    BrxtEnvVar {
        key: key.to_string(),
        required,
        auto_propagate: false,
        default: None,
        description: String::new(),
        secret,
    }
}

fn manifest(env_vars: Vec<BrxtEnvVar>) -> BrxtManifest {
    BrxtManifest {
        name: "spokeagent".to_string(),
        display_name: "SPOKE Agent".to_string(),
        description: "SPOKE knowledge graph".to_string(),
        version: "0.4.1".to_string(),
        entry_point: "main.py".to_string(),
        repository: "https://github.com/BaranziniLab/SPOKEAgent".to_string(),
        tools_count: None,
        env_vars,
    }
}

/// `config.yaml` is a plaintext file. A credential in its `envs` map is a
/// credential in the user's home directory in the clear, readable by every tool
/// on the machine — and by the agent's own shell.
#[test]
fn a_declared_secret_never_reaches_the_plaintext_config_entry() {
    sandbox();
    let m = manifest(vec![
        var("SPOKEAGENT_PASSCODE", true, true),
        var("SPOKE_HOST", false, false),
    ]);
    let supplied = HashMap::from([
        ("SPOKEAGENT_PASSCODE".to_string(), SECRET.to_string()),
        ("SPOKE_HOST".to_string(), "https://spoke.ucsf".to_string()),
    ]);

    let (secret_keys, settings) = classify_supplied(&m, &supplied);
    assert_eq!(secret_keys, vec!["SPOKEAGENT_PASSCODE".to_string()]);
    assert_eq!(
        settings,
        HashMap::from([("SPOKE_HOST".to_string(), "https://spoke.ucsf".to_string())]),
        "an ordinary setting belongs in the config; a credential does not"
    );

    let config = compose_config(
        &m,
        std::path::Path::new("/ext/spokeagent"),
        settings,
        secret_keys,
    );
    let serialised = serde_yaml::to_string(&config).expect("a serialisable config entry");
    assert!(
        !serialised.contains(SECRET),
        "the credential reached config.yaml:\n{serialised}"
    );
    // The NAME is there — that is how the daemon knows to fetch it at launch.
    assert!(serialised.contains("SPOKEAGENT_PASSCODE"));
    assert!(serialised.contains("https://spoke.ucsf"));
}

/// A key the manifest never declared is a setting, not a credential. Guessing
/// the other way would silently hide a value the user expected to see.
#[test]
fn an_undeclared_key_is_treated_as_a_setting() {
    sandbox();
    let m = manifest(vec![var("SPOKEAGENT_PASSCODE", true, true)]);
    let supplied = HashMap::from([("SPOKE_TIMEOUT".to_string(), "30".to_string())]);
    let (secret_keys, settings) = classify_supplied(&m, &supplied);
    assert!(secret_keys.is_empty());
    assert_eq!(
        settings.get("SPOKE_TIMEOUT").map(String::as_str),
        Some("30")
    );
}

/// The card is published into the session's action-required queue and rendered
/// in the chat. It asks for key names; it must have no shape a value fits in.
#[test]
fn the_published_card_carries_no_value() {
    sandbox();
    let content = biorouter::conversation::message::MessageContent::action_required_secrets(
        "card-1",
        "SPOKE Agent needs 1 value before it can run.".to_string(),
        vec![SecretKeyRequest {
            key: "SPOKEAGENT_PASSCODE".to_string(),
            label: "SPOKEAGENT_PASSCODE".to_string(),
            description: Some("From the UCSF wiki credentials page".to_string()),
            required: true,
        }],
        SecretDestination::ExtensionEnv {
            extension_name: "spokeagent".to_string(),
        },
    );
    let json = serde_json::to_string(&content).expect("a serialisable card");
    assert!(json.contains("SPOKEAGENT_PASSCODE"));
    assert!(!json.contains(SECRET));
    // Not merely absent for this input — there is no field for one.
    assert!(!json.contains("\"value\""));
}

/// The registry refuses a value-bearing answer to a credential card, and the
/// caller stays parked rather than receiving one. This is the guarantee that
/// holds for surfaces `pending_user_action` has never heard of.
#[tokio::test]
async fn a_value_bearing_answer_to_a_credential_card_is_refused() {
    sandbox();
    let registry = PendingUserActions::global();
    let parked = registry.park(
        Some("s-refuse"),
        None,
        UserActionRequest::Secrets(SecretsRequest {
            prompt: "Configure".to_string(),
            keys: vec![SecretKeyRequest {
                key: "SPOKEAGENT_PASSCODE".to_string(),
                label: "SPOKEAGENT_PASSCODE".to_string(),
                description: None,
                required: true,
            }],
            destination: SecretDestination::Keyring,
        }),
    );
    let id = parked.id().to_string();

    let outcome = registry.resolve_in_session(
        "s-refuse",
        &id,
        UserActionOutcome::Provided {
            data: serde_json::json!({ "SPOKEAGENT_PASSCODE": SECRET }),
        },
        biorouter::pending_user_action::DecisionAuthority::unproven(),
    );
    assert_eq!(
        outcome,
        ResolveOutcome::Rejected,
        "a credential card must not accept a data-bearing outcome"
    );
    assert!(
        registry.is_pending(&id),
        "a refused answer must leave the caller parked, not release it with the value"
    );

    registry.resolve_in_session(
        "s-refuse",
        &id,
        UserActionOutcome::Cancelled,
        biorouter::pending_user_action::DecisionAuthority::unproven(),
    );
    drop(parked);
}

/// What the parked install — and therefore the model — is told.
#[tokio::test]
async fn the_install_learns_key_names_and_nothing_else() {
    sandbox();
    let registry = PendingUserActions::global();
    let parked = biorouter::extension_install::park_credentials(
        Some("s-names"),
        None,
        "Configure".to_string(),
        CredentialSpec {
            destination: SecretDestination::ExtensionEnv {
                extension_name: "spokeagent".to_string(),
            },
            vars: vec![var("SPOKEAGENT_PASSCODE", true, true)],
        },
    );
    let id = parked.id().to_string();

    let outcome = submit_credentials(
        &id,
        HashMap::from([("SPOKEAGENT_PASSCODE".to_string(), SECRET.to_string())]),
    );
    assert_eq!(
        outcome,
        SubmitOutcome::Configured {
            configured_keys: vec!["SPOKEAGENT_PASSCODE".to_string()]
        }
    );

    let (released, settings) = parked.wait(std::time::Duration::from_secs(5), None).await;
    match &released {
        UserActionOutcome::SecretsConfigured { configured_keys } => {
            assert_eq!(configured_keys, &vec!["SPOKEAGENT_PASSCODE".to_string()]);
        }
        other => panic!("expected SecretsConfigured, got {other:?}"),
    }
    // The credential half of the answer never comes back through here.
    assert!(
        !settings.contains_key("SPOKEAGENT_PASSCODE"),
        "a declared secret must not arrive as a plain setting"
    );
    let rendered = format!("{released:?}");
    assert!(
        !rendered.contains(SECRET),
        "the outcome's Debug rendering leaked the value: {rendered}"
    );

    // ...and it did land where it was supposed to.
    let stored: String = biorouter::config::Config::global()
        .get_secret("SPOKEAGENT_PASSCODE")
        .expect("the credential store holds it");
    assert_eq!(stored, SECRET);

    let _ = registry.is_pending(&id);
}

/// Diagnostics bundles are attached to bug reports. `secrets.yaml` (and its
/// keyring equivalent) is where the credential lives, so nothing that collects
/// diagnostics may read it.
///
/// A structural assertion rather than a behavioural one, because the failure it
/// guards against is somebody adding one `read_to_string` to a collector — which
/// no behavioural test of today's bundle would ever fail on.
#[test]
fn nothing_in_the_diagnostics_collector_reads_the_secret_store() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("biorouter-server/src/routes/status.rs");
    let source = std::fs::read_to_string(&root).unwrap_or_default();
    assert!(
        !source.contains("secrets.yaml"),
        "the diagnostics route reads the secret store: {}",
        root.display()
    );
    assert!(
        !source.contains("get_secrets("),
        "the diagnostics route enumerates stored secrets: {}",
        root.display()
    );
}

/// A credential card is a decision prompt, not a record.
///
/// It must not be persisted into the session row: reopening the chat would then
/// show a live-looking dialog for an install that finished long ago, and — worse
/// for this feature — every surface that replays a session (export, share,
/// `chatrecall`, a subagent's flattened transcript) would carry the card
/// forward. It carries no value, but a card that cannot be answered is still a
/// prompt inviting one.
#[test]
fn a_credential_card_is_never_written_into_the_session() {
    sandbox();
    let card = biorouter::conversation::message::Message::assistant().with_content(
        biorouter::conversation::message::MessageContent::action_required_secrets(
            "card-1",
            "SPOKE Agent needs 1 value.".to_string(),
            vec![SecretKeyRequest {
                key: "SPOKEAGENT_PASSCODE".to_string(),
                label: "SPOKEAGENT_PASSCODE".to_string(),
                description: None,
                required: true,
            }],
            SecretDestination::Keyring,
        ),
    );
    assert!(
        biorouter::pending_user_action::is_ephemeral_card(&card),
        "a credential card must be ephemeral, or the drain persists it"
    );
}

/// What a provider actually receives.
///
/// The coding-agent providers flatten the whole conversation into one prompt —
/// the widest possible outbound payload, and the one place a stray credential
/// would be hardest to notice. The card is in the conversation here on purpose:
/// even if a future change stopped it being ephemeral, flattening it must still
/// produce no value, because there is none in it to produce.
#[test]
fn flattening_a_conversation_that_contains_the_card_yields_no_value() {
    sandbox();
    use biorouter::conversation::message::{Message, MessageContent};

    let messages = vec![
        Message::user().with_text("Install the SPOKE extension for me"),
        Message::assistant().with_content(MessageContent::action_required_secrets(
            "card-1",
            "SPOKE Agent needs 1 value before it can run.".to_string(),
            vec![SecretKeyRequest {
                key: "SPOKEAGENT_PASSCODE".to_string(),
                label: "SPOKEAGENT_PASSCODE".to_string(),
                description: Some("From the UCSF wiki".to_string()),
                required: true,
            }],
            SecretDestination::ExtensionEnv {
                extension_name: "spokeagent".to_string(),
            },
        )),
        // The redacted audit line the chat is allowed to keep.
        Message::assistant()
            .with_text("Credentials configured for SPOKE Agent: SPOKEAGENT_PASSCODE"),
    ];

    let flattened =
        biorouter::providers::coding_agent::transcript::flatten(&messages).unwrap_or_default();
    assert!(
        !flattened.contains(SECRET),
        "the flattened provider prompt carried the credential:\n{flattened}"
    );
    // The audit line survives — it is the useful, safe half.
    assert!(flattened.contains("SPOKEAGENT_PASSCODE"));
}

// ──────────────────────────────────────────────────────────────────────────────
// The on-disk claim (F-13)
//
// A stopped install used to be recorded only in a process-global map, so the
// one case it existed for — "I closed the app before I had the passcode" — was
// never recoverable. These run the real transaction end to end against the
// sandbox above and look at what is left on disk afterwards.
//
// `#[serial]` because they all assert on the *contents* of one claims
// directory, and the binary's tests otherwise run concurrently.
// ──────────────────────────────────────────────────────────────────────────────

use std::path::PathBuf;

use biorouter::extension_install::brxt::{extensions_root, uv_available};
use biorouter::extension_install::claim::{claims_dir, read_claims};
use biorouter::extension_install::{
    ClaimPhase, CredentialPolicy, ExtensionInstallTransaction, InstallSource, InstallState,
};

/// A minimal `.brxt` on disk, plus the temp dir that owns it.
struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

/// Build a structurally valid bundle. `pyproject` is a parameter because a
/// bundle whose `uv sync` fails is how the failed-install path is reached
/// without a network.
fn bundle(name: &str, env_vars: Vec<BrxtEnvVar>, pyproject: &str) -> Fixture {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join(format!("{name}.brxt"));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::FileOptions::default();
    let mut m = manifest(env_vars);
    m.name = name.to_string();
    for (entry, body) in [
        ("manifest.json", serde_json::to_vec(&m).unwrap()),
        ("README.md", b"# Fixture\n".to_vec()),
        ("pyproject.toml", pyproject.as_bytes().to_vec()),
        ("src/main.py", b"pass\n".to_vec()),
    ] {
        zip.start_file(entry, options).unwrap();
        zip.write_all(&body).unwrap();
    }
    zip.finish().unwrap();
    Fixture { _dir: dir, path }
}

fn working_pyproject(name: &str) -> String {
    format!("[project]\nname = \"{name}\"\nversion = \"0.0.1\"\nrequires-python = \">=3.10\"\ndependencies = []\n")
}

/// Empty the claims directory, having first PROVED it is the sandbox's and not
/// the developer's. `~/.config/biorouter` holds live extensions.
fn clear_claims() {
    let dir = claims_dir();
    assert!(
        dir.starts_with(sandbox()),
        "the fixture resolved to {} — refusing to delete anything there",
        dir.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `uv` builds every bundle's environment; without it there is no install to
/// observe. Reported rather than silently passing.
fn uv_or_skip(test: &str) -> bool {
    if uv_available() {
        return true;
    }
    eprintln!("skipping {test}: `uv` is not installed");
    false
}

/// What F-13 is about. A refused install must leave something on disk that
/// names the extension, its tree, and the key it is still waiting for —
/// otherwise there is nothing for `biorouter extension configure` to find.
#[tokio::test]
#[serial_test::serial]
async fn a_refused_install_leaves_a_parked_claim_on_disk() {
    sandbox();
    if !uv_or_skip("a_refused_install_leaves_a_parked_claim_on_disk") {
        return;
    }
    clear_claims();
    // A key and a name of this test's own. `SPOKEAGENT_PASSCODE` is written to
    // the sandbox's credential store by `the_install_learns_key_names_and_…`,
    // and a stored secret counts as MET — so sharing the name would make this
    // test pass or fail on the order the binary happened to run in.
    let name = "parkedclaimfixture";
    let key = "PARKED_CLAIM_PASSCODE";
    let tree = extensions_root().join(name);
    let _ = std::fs::remove_dir_all(&tree);

    let fixture = bundle(name, vec![var(key, true, true)], &working_pyproject(name));
    let report = ExtensionInstallTransaction::new(InstallSource::LocalFile {
        path: fixture.path.clone(),
    })
    .run(CredentialPolicy::Refuse, None)
    .await;

    assert_eq!(
        report.state,
        InstallState::NeedsCredentials {
            keys: vec![key.to_string()]
        },
        "{report:?}"
    );

    let claims = read_claims();
    assert_eq!(claims.len(), 1, "expected exactly one claim: {claims:?}");
    let claim = &claims[0];
    assert_eq!(claim.phase, ClaimPhase::Parked);
    assert_eq!(claim.extension_name, name);
    assert_eq!(claim.install_dir, tree);
    assert!(
        !claim.existed_before,
        "a first install must not claim it found the tree"
    );
    assert_eq!(claim.pending_keys, vec![key.to_string()]);
    assert_eq!(claim.install_id, report.install_id);
    // The expensive half survives, which is what makes the claim resumable.
    assert!(tree.join("manifest.json").is_file());

    // ...and the plaintext claim carries no value, because there is none in it.
    let file = std::fs::read_dir(claims_dir())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let body = std::fs::read_to_string(&file).unwrap();
    assert!(!body.contains(SECRET), "{body}");

    clear_claims();
    let _ = std::fs::remove_dir_all(&tree);
}

/// A claim that outlives a finished install is a permanent phantom "pending
/// install" on every reader.
#[tokio::test]
#[serial_test::serial]
async fn a_successful_install_leaves_no_claim() {
    sandbox();
    if !uv_or_skip("a_successful_install_leaves_no_claim") {
        return;
    }
    clear_claims();
    let name = "claimfixtureok";
    let tree = extensions_root().join(name);
    let _ = std::fs::remove_dir_all(&tree);

    let fixture = bundle(name, Vec::new(), &working_pyproject(name));
    let report = ExtensionInstallTransaction::new(InstallSource::LocalFile {
        path: fixture.path.clone(),
    })
    .run(CredentialPolicy::Refuse, None)
    .await;

    assert!(report.state.is_success(), "{report:?}");
    assert!(
        read_claims().is_empty(),
        "a finished install left a claim behind: {:?}",
        read_claims()
    );

    biorouter::config::extensions::remove_extension(&biorouter::config::extensions::name_to_key(
        name,
    ));
    let _ = std::fs::remove_dir_all(&tree);
}

/// Same rule on the other side: a run that failed and rolled itself back owns
/// nothing, so it must claim nothing.
#[tokio::test]
#[serial_test::serial]
async fn a_failed_install_leaves_no_claim() {
    sandbox();
    if !uv_or_skip("a_failed_install_leaves_no_claim") {
        return;
    }
    clear_claims();
    let name = "claimfixturebad";
    let tree = extensions_root().join(name);
    let _ = std::fs::remove_dir_all(&tree);

    // Unparseable TOML, so `uv sync` fails immediately and offline.
    let fixture = bundle(name, Vec::new(), "this is not toml at all [[[\n");
    let report = ExtensionInstallTransaction::new(InstallSource::LocalFile {
        path: fixture.path.clone(),
    })
    .run(CredentialPolicy::Refuse, None)
    .await;

    assert!(
        matches!(report.state, InstallState::Failed { .. }),
        "{report:?}"
    );
    assert!(
        read_claims().is_empty(),
        "a rolled-back install left a claim behind: {:?}",
        read_claims()
    );
    assert!(
        !tree.exists(),
        "a rolled-back first install must not leave its tree: {}",
        tree.display()
    );
    // Nothing was written outside the sandbox on the way.
    assert!(extensions_root().starts_with(sandbox()));
}

// ──────────────────────────────────────────────────────────────────────────────
// The operator's switch (#42), in both positions
//
// `privacy::refusal::extension_enable_refusal` reads exactly "the config file
// carries an entry for this extension AND that entry says `enabled: false`" and
// calls it an operator pin: `manage_extensions enable` is then refused without
// a proof-backed grant, and a PRIVATE extension is refused even with one. So an
// install that writes that row has not merely disabled an extension — it has
// taken away the model's ability to undo it, with a message that blames a person.
//
// These drive the real transaction against the sandbox and read `config.yaml`
// back, because the defect was entirely about what reaches that file.
// ──────────────────────────────────────────────────────────────────────────────

/// The `enabled` flag `config.yaml` currently carries, or `None` when it has no
/// entry at all — the same distinction the enable gate's `persisted` argument
/// draws.
fn switch_in_config(name: &str) -> Option<bool> {
    if !biorouter::config::extension_entry_is_persisted(name) {
        return None;
    }
    biorouter::config::get_extension_entry_by_name(name).map(|entry| entry.enabled)
}

/// **An install updates a package. It does not author the operator's switch.**
///
/// Measured before the fix, in this order:
///   1. install `switchfixture` — `config.yaml` says `enabled: true`;
///   2. re-install it with `enable: false` — `config.yaml` said `enabled:
///      false`, and `manage_extensions {"action":"enable"}` was then refused
///      with "The operator turned it off deliberately";
///   3. with the switch genuinely pinned off, install with `enable: true` —
///      #42's own direction, already guarded, and re-asserted here so the fix
///      to (2) cannot be written as "just take the caller's flag".
///
/// Fails an implementation that persists this run's own `enable` flag (the
/// shipped one) on row 2, and one that persists it unconditionally on row 3.
#[tokio::test]
#[serial_test::serial]
async fn an_install_never_rewrites_the_operators_enabled_switch() {
    sandbox();
    if !uv_or_skip("an_install_never_rewrites_the_operators_enabled_switch") {
        return;
    }
    clear_claims();
    let name = "switchfixture";
    let key = biorouter::config::extensions::name_to_key(name);
    let tree = extensions_root().join(name);
    let _ = std::fs::remove_dir_all(&tree);
    biorouter::config::remove_extension(&key);
    assert_eq!(
        switch_in_config(name),
        None,
        "the fixture starts from an unconfigured extension, or it proves nothing"
    );

    let fixture = bundle(name, Vec::new(), &working_pyproject(name));
    let install = |enable: bool| {
        let path = fixture.path.clone();
        async move {
            ExtensionInstallTransaction::new(InstallSource::LocalFile { path })
                .enabled(enable)
                .run(CredentialPolicy::Refuse, None)
                .await
        }
    };

    // 1. A first install authors the row, so the caller's flag stands.
    let first = install(true).await;
    assert!(first.state.is_success(), "{first:?}");
    assert_eq!(
        switch_in_config(name),
        Some(true),
        "the precondition for row 2 is an extension the user has ON"
    );
    assert!(first.enabled, "{first:?}");

    // 2. The mirror case. The user's switch stays where the user left it.
    let second = install(false).await;
    assert!(second.state.is_success(), "{second:?}");
    assert_eq!(
        switch_in_config(name),
        Some(true),
        "an install with `enable: false` switched OFF an extension the user had enabled — and \
         the row it wrote is what `extension_enable_refusal` reads as an operator pin"
    );
    assert!(
        !second.operator_pinned_off,
        "nothing was pinned; reporting a pin here blames the user for the install's own flag"
    );

    // 3. #42's own direction, against a switch that really was set by a person.
    biorouter::config::set_extension_enabled(&key, false);
    assert_eq!(switch_in_config(name), Some(false));
    let third = install(true).await;
    assert!(third.state.is_success(), "{third:?}");
    assert_eq!(
        switch_in_config(name),
        Some(false),
        "#42: an install must not overturn a persisted `enabled: false`"
    );
    assert!(
        third.operator_pinned_off,
        "the install was blocked by a real pin and has to say so: {third:?}"
    );
    assert!(!third.enabled, "{third:?}");

    biorouter::config::remove_extension(&key);
    let _ = std::fs::remove_dir_all(&tree);
}
