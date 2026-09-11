//! Task 30: `/config/upsert` is the master toggle's second writer, and the ONE
//! key it will not write as an ordinary configuration value (issue #56, DR-15).
//!
//! ⚠ **Why this is an integration binary and not a `#[cfg(test)] mod` beside the
//! route.** The toggle is a PROCESS-GLOBAL atomic and `cargo test` runs a
//! crate's unit tests in parallel threads of one process — and
//! `crates/biorouter-server/src/routes/` is compiled into TWO of them, the lib
//! and the `biorouterd` bin. Written next to the route, these tests' OFF window
//! disabled the barrier under
//! `routes::knowledge::tests::a_public_model_is_refused_another_sessions_private_conversation_with_409`
//! and
//! `routes::apps::tests::privacy_capability::the_app_capability_report_follows_the_manifests_provider_not_the_global_one`,
//! which do not take the fixture and cannot be made to — there are ~550 tests
//! across the two targets. Measured, not predicted: both failed on the first
//! full-workspace run. Each `tests/*.rs` file is its own process, so nothing
//! outside this file can observe its writes.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
// The lib's copy is `#[cfg(test)]` and is NOT compiled into an integration
// binary — every one of these files declares its own. Nothing here builds an
// `AppState` today, so this is a floor rather than a fix; the guard exists
// because the next test added here would not have one.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::Json;
use biorouter::config::Config;
use biorouter_server::routes::config_management::{
    read_all_config, read_config, remove_config, upsert_config, ConfigKeyQuery,
    ConfigValueResponse, UpsertConfigQuery,
};
use http::{HeaderMap, StatusCode};
use serde_json::Value;

/// Points `Config::global()` at a throwaway directory, and restores the path
/// override and process-global toggle on drop — including on the unwind path a
/// failing assertion takes.
///
/// ⚠ **These tests must never write the developer's real `config.yaml`, and an
/// earlier version of them did.** `upsert_config` persists, so the accepting arm
/// really does write a file; capturing and restoring the one key it touches
/// looked sufficient and was not. `Config::global()`'s guard is a mutex *inside
/// one process*, and the same tests were compiled into two targets — the lib and
/// the `biorouterd` bin — which `cargo test` runs as separate processes. Their
/// restores interleaved and left `BIOROUTER_PRIVACY_TIERS: off` in the real
/// config, which then made an unrelated test in another crate fail for a reason
/// that had nothing to do with its own code. Redirecting the whole config root
/// is the fix: there is no shared file left to race over.
///
/// ⚠ **ONE root for the whole process, and it is never dropped.**
/// `Config::global()` is a `OnceCell`: it resolves its path on FIRST access and
/// keeps it. A per-test `TempDir` would therefore work for the first test and
/// then be deleted out from under the second, which would still be writing to
/// it. A `OnceLock<TempDir>` in a static is the right lifetime — statics do not
/// drop, so the directory outlives every test, and the OS reclaims it.
///
/// `set_var` rather than `env_lock`: `EnvGuard` borrows its keys and so cannot
/// be stored beside the value it protects, and there is nothing here to race —
/// this is an integration binary whose only two tests are `#[serial]`.
static CONFIG_ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

struct PrivacyToggleFixture {
    prev_enabled: bool,
    prev_path_root: Option<std::ffi::OsString>,
}

impl PrivacyToggleFixture {
    fn capture() -> Self {
        let prev_path_root = std::env::var_os("BIOROUTER_PATH_ROOT");
        let root = CONFIG_ROOT.get_or_init(|| {
            let dir = tempfile::tempdir().expect("temp config root");
            std::fs::create_dir_all(dir.path().join("config")).expect("config dir");
            dir
        });
        std::env::set_var(
            "BIOROUTER_PATH_ROOT",
            root.path().to_str().expect("utf-8 temp path"),
        );
        // Fail loudly rather than silently writing the developer's real file:
        // if `Config::global()` was somehow initialised before this ran, every
        // assertion below would still pass while the side effect landed in
        // `~/.config/biorouter/config.yaml`.
        assert!(
            Config::global().all_values().is_ok_and(|_| true)
                && biorouter::config::paths::Paths::config_dir().starts_with(root.path()),
            "the config root was not redirected; refusing to write the real config.yaml"
        );
        Self {
            prev_enabled: biorouter::privacy::privacy_tiers_enabled(),
            prev_path_root,
        }
    }
}

impl Drop for PrivacyToggleFixture {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.prev_enabled);
        match self.prev_path_root.take() {
            Some(root) => std::env::set_var("BIOROUTER_PATH_ROOT", root),
            None => std::env::remove_var("BIOROUTER_PATH_ROOT"),
        }
    }
}

/// Arm DR-20's system-authentication seam for the next prompt.
///
/// ⚠ **This compiles only because `biorouter` is a `[dev-dependency]` of this
/// crate with `privacy-test-auth` on.** Dropping that dev-dependency stops this
/// line compiling — a loud failure — rather than leaving a test suite that asks
/// the developer for their password on every run.
fn arm_the_system_prompt(outcome: biorouter::privacy::system_auth::AuthOutcome) {
    biorouter::privacy::system_auth_seam::reset();
    biorouter::privacy::system_auth_seam::answer_next_prompt(outcome);
}

fn upsert(key: &str, value: &str, confirm: Option<&str>) -> UpsertConfigQuery {
    UpsertConfigQuery {
        key: key.to_string(),
        value: Value::String(value.to_string()),
        is_secret: false,
        confirm: confirm.map(str::to_string),
    }
}

/// ⚠ **THESE ASSERTIONS LOOK CONTRADICTORY AND ARE NOT.** `/config/upsert` MUST
/// be one of the toggle's two writers — it is the channel Settings > Privacy
/// uses — and a BARE upsert of this key MUST be refused. What separates them is
/// the confirmation field, which is what the panel sends and what a tool call
/// composing an ordinary config write does not.
///
/// What this is and what it is not: a **UX guard against an accidental or
/// model-composed config write**, not an authorization boundary. The phrase is a
/// fixed string in the shipped source, so a caller holding the daemon secret
/// replays it. That is acceptable because `check_token` has no principal and
/// because the authorization on this route is `X-User-Action`, not the phrase.
/// ⚠ This used to read "accepted for the same reason AR-15 is … such a caller
/// can raise its own session's capability anyway". **AR-15 was retired on
/// 2026-08-02** (DR-16, commit `0757823f`) and that clause is withdrawn.
/// ⚠ `#[serial]`, and the other test in this file carries it too. Moving these
/// out of the lib stopped them disturbing ~550 unrelated tests; it did not stop
/// them disturbing EACH OTHER. Both mutate the same process-global atomic and
/// both write the same config key, and `cargo test` runs a binary's two tests in
/// two threads.
#[tokio::test]
#[serial_test::serial]
async fn a_bare_config_upsert_cannot_flip_the_key_but_the_confirmed_one_can() {
    let _fixture = PrivacyToggleFixture::capture();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    let headers = HeaderMap::new();

    let (status, body) = upsert_config(
        headers.clone(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "off",
            None,
        )),
    )
    .await
    .expect_err("a bare upsert of the master switch must be refused");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("Settings"),
        "the refusal must name the way in: {body}"
    );
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "a refused request must not have written"
    );

    // A wrong phrase is refused, and the comparison is EXACT — this is the
    // assertion that fails an `eq_ignore_ascii_case` or a `trim()`.
    let (status, _) = upsert_config(
        headers.clone(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "off",
            Some("disable privacy tiers"),
        )),
    )
    .await
    .expect_err("a wrong phrase must be refused");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(biorouter::privacy::privacy_tiers_enabled());

    // …and the confirmed one goes through, and moves the LIVE value rather than
    // only the file: the authoritative copy is in daemon memory, so a handler
    // that wrote config.yaml and stopped would leave every gate enforcing until
    // the next restart.
    //
    // Since Task 55 a disable also raises DR-20's system prompt, which the test
    // seam answers here. `turning_the_tiers_off_needs_the_operating_system_and_turning_them_on_does_not`
    // is what asserts the prompt is really consulted; this arming only keeps the
    // assertion above about the CONFIRMATION FIELD from failing for a second,
    // unrelated reason.
    arm_the_system_prompt(biorouter::privacy::system_auth::AuthOutcome::Approved);
    let _ok = upsert_config(
        headers,
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "off",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("the confirmed flip is the one write this route allows");
    assert!(!biorouter::privacy::privacy_tiers_enabled());
}

/// The two asymmetries review found beside the confirmed arm above. Neither was
/// a hole — both land in the safe direction — and both are closed because the
/// value of "there is exactly one way this changes" is that it is TRUE, not that
/// every way it is false happens to be harmless.
///
/// ⚠ `#[serial]` for the same reason as its two siblings: one process-global
/// atomic, one shared temp config file, and `cargo test` runs a binary's tests
/// in threads.
#[tokio::test]
#[serial_test::serial]
async fn the_master_switch_has_exactly_one_door_and_it_is_not_delete_or_the_secret_store() {
    let _fixture = PrivacyToggleFixture::capture();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    let headers = HeaderMap::new();

    // DELETE. `is_capability_key`'s own doc argues one predicate for both verbs;
    // this key had the guard on `upsert` alone, so a `/config/remove` took the
    // key off disk — the next boot then reads *absent* and resolves to ON while
    // the running daemon keeps whatever its atomic held.
    Config::global()
        .set(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            &Value::String("off".to_string()),
            false,
        )
        .unwrap();
    let (status, body) = remove_config(
        headers.clone(),
        Json(ConfigKeyQuery {
            key: biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY.to_string(),
            is_secret: false,
        }),
    )
    .await
    .expect_err("a delete of the master switch must be refused");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("Settings"),
        "the refusal must name the way in: {body}"
    );
    assert!(
        Config::global()
            .get_param::<Value>(biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY)
            .is_ok(),
        "a refused delete must not have removed the key"
    );

    // THE SECRET STORE. `config.set(.., is_secret)` routes to the OS credential
    // store, which the loader's `all_values()` does not read — so a CONFIRMED
    // secret write would set this process's atomic to `off` and then silently
    // revert to `on` at the next launch. The panel always sends `false`, so this
    // is unreachable today; it is refused so that stays a property of the daemon
    // rather than of one caller.
    let (status, body) = upsert_config(
        headers,
        Json(UpsertConfigQuery {
            key: biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY.to_string(),
            value: Value::String("off".to_string()),
            is_secret: true,
            confirm: Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE.to_string()),
        }),
    )
    .await
    .expect_err("the master switch must not be written to the secret store");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("secret"), "{body}");
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "a refused request must not have moved the live value"
    );
}

/// The renderer mirrors [`biorouter::privacy::privacy_tiers_value_is_on`] in
/// `settings/privacy/privacyTiers.ts`, and the two must agree on surrounding
/// whitespace or a hand-edited `BIOROUTER_PRIVACY_TIERS: " off "` renders as off
/// in Settings → Privacy while the daemon goes on enforcing. Telling the user
/// something false about the control they just used is the one failure the
/// parser's own doc-comment says to avoid.
#[test]
fn the_value_parser_agrees_with_the_renderer_about_whitespace() {
    let is_on = |s: &str| biorouter::privacy::privacy_tiers_value_is_on(&Value::String(s.into()));
    assert_eq!(is_on(" off "), Some(false));
    assert_eq!(is_on("\tFalse\n"), Some(false));
    assert_eq!(is_on(" no "), Some(false));
    assert_eq!(is_on(" on "), Some(true));
    // A shape that is neither a bool nor a string still says "I cannot tell",
    // which every caller resolves to ON.
    assert_eq!(
        biorouter::privacy::privacy_tiers_value_is_on(&Value::Null),
        None
    );
}

/// The other half of hardening measure (1): the value the daemon boots with
/// comes from a FILE the daemon owns, and an environment variable cannot reach
/// it.
///
/// ⚠ **Task 42 re-pointed the second half of this test, and that is the ruling,
/// not a weakening.** It used to write the retired key into `config.yaml` and
/// assert the loader honoured it — *"the FILE is what the loader reads"*.
/// [DR-22] says that file must no longer be the switch's home, so an assertion
/// that `config.yaml` still steers the loader is now an assertion that the
/// ruling was not implemented. The property it exists to protect — no
/// environment variable can turn protection off, and the boot value comes from
/// disk rather than from the process environment — is asserted unchanged, and
/// now against the store the loader really reads.
#[tokio::test]
#[serial_test::serial]
async fn the_startup_load_reads_the_switch_store_and_not_the_environment() {
    // ⚠ The fixture FIRST — its `set_var` is what redirects the config root, and
    // `Config::global()` is a `OnceCell` that resolves its path on first access,
    // so anything reading config before this line pins the developer's real one.
    //
    // A plain `set_var` for the variable under test, matching the fixture. It is
    // NOT that `env_lock` would deadlock — the fixture holds no such guard, as
    // its own doc says — but that every test in this binary is `#[serial]`, so
    // there is nothing here to race with the one test that does take the lock.
    let _fixture = PrivacyToggleFixture::capture();
    struct EnvVar(&'static str);
    impl Drop for EnvVar {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }
    std::env::set_var(biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY, "off");
    let _unset = EnvVar(biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY);

    // The other tests in this binary write both the key and the store into the
    // shared temp config root, and `#[serial]` orders them but says nothing
    // about which runs first. Clear both so "nothing recorded" means exactly
    // that whichever order they take.
    reset_switch_storage();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "nothing recorded must resolve to ON, and the env var must not be consulted"
    );

    // The record is written here as BYTES rather than through the writer, so
    // this pins the on-disk format a user's backup and a support answer depend
    // on — a round-trip through `write_for` would agree with itself whatever it
    // wrote. `changed_at` is deliberately omitted: it is an audit field, and a
    // record missing it must still read.
    std::fs::write(
        config_dir().join("privacy-tiers.json"),
        r#"{"enabled": false}"#,
    )
    .unwrap();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        !biorouter::privacy::privacy_tiers_enabled(),
        "the switch STORE is what the loader reads"
    );
}

/// Issue #56 DR-20 / Task 55 Step 2. Disabling the whole tier system is at least
/// as consequential as declassifying one chat, so it takes the same
/// operating-system authentication.
///
/// ⚠ **Only the OFF direction, and that is a ruling rather than an oversight.**
/// Turning protection back ON is the safe direction, and gating it would mean an
/// [`AuthOutcome::Unavailable`] — the state of every headless host, and of every
/// Linux install until the packaging ships the polkit action — strands a machine
/// with the feature disabled and no way to re-enable it. The same asymmetry
/// Task 55 Step 1 applies to a `turn:*` chat: spend the cost where the
/// consequence is.
///
/// The ON half is also the discriminating half. The seam is left UNARMED for it
/// on purpose: an unarmed seam refuses by default, so if the re-enable path
/// prompted, this test would fail there rather than pass quietly.
///
/// ⚠ **"An unarmed seam refuses" is only true with `BIOROUTER_PRIVACY_TEST_AUTH`
/// unset**, so the guard below is what makes that sentence a property rather
/// than an assumption about the developer's shell. `env_answer` is consulted
/// whenever the one-shot arming is absent, so `BIOROUTER_PRIVACY_TEST_AUTH=approve`
/// exported by anyone driving the dev GUI turns the unarmed seam into an
/// approving one and a regression on the ON path would pass here. Measured on
/// the sibling assertion in `biorouter-cli`, which had the identical hole:
/// mutate the code to prompt where it should not, and the test fails with the
/// variable unset and passes with it set.
///
/// The fixture does **not** hold an `env_lock` guard of its own — it redirects
/// the config root with a plain `set_var`, as its own doc says — so taking one
/// here cannot deadlock against it.
#[tokio::test]
#[serial_test::serial]
async fn turning_the_tiers_off_needs_the_operating_system_and_turning_them_on_does_not() {
    use biorouter::privacy::system_auth::AuthOutcome;

    let _env = env_lock::lock_env([("BIOROUTER_PRIVACY_TEST_AUTH", None::<&str>)]);
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(biorouter::privacy::privacy_tiers_enabled());

    // The user's "no", and a machine that cannot ask at all. Neither may
    // disable the feature, and neither may leave a record on disk that disables
    // it at the NEXT launch — a switch that flips on restart is the divergence
    // Task 30's measure (3) exists to prevent, and the user would be told it
    // was refused.
    for outcome in [AuthOutcome::Denied, AuthOutcome::Unavailable] {
        arm_the_system_prompt(outcome);
        let (status, body) = upsert_config(
            HeaderMap::new(),
            Json(upsert(
                biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
                "off",
                Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
            )),
        )
        .await
        .expect_err("a refused system authentication must not disable the tiers");
        assert_eq!(status, StatusCode::FORBIDDEN, "{outcome:?}: {body}");
        assert!(
            biorouter::privacy::privacy_tiers_enabled(),
            "{outcome:?} disabled the tiers anyway"
        );
        biorouter::privacy::load_privacy_tiers_from_config();
        assert!(
            biorouter::privacy::privacy_tiers_enabled(),
            "{outcome:?} recorded a disable on disk, so the next launch comes up unprotected"
        );
    }

    // DR-20 point 4: the dialog says what it authorises. A prompt reading
    // "BioRouter wants to make changes" satisfies the letter of the ruling and
    // defeats its purpose.
    let asked = biorouter::privacy::system_auth_seam::last_request()
        .expect("the master switch reached the prompter");
    assert!(!asked.reason.is_empty(), "the prompt stated no reason");
    assert_eq!(
        asked.session_ids.len(),
        1,
        "the master switch names one subject, not a list of chats: {asked:?}"
    );

    arm_the_system_prompt(AuthOutcome::Approved);
    let _ok = upsert_config(
        HeaderMap::new(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "off",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("an approved authentication is the one that writes");
    assert!(!biorouter::privacy::privacy_tiers_enabled());

    // …and turning protection back on asks for nothing.
    biorouter::privacy::system_auth_seam::reset();
    let _ok = upsert_config(
        HeaderMap::new(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "on",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("re-enabling protection must not need a password");
    assert!(biorouter::privacy::privacy_tiers_enabled());
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 42 (DR-22): the master switch's storage is not `config.yaml`.
// ─────────────────────────────────────────────────────────────────────────────

/// The directory `Config::global()` resolved its `config.yaml` into — which is
/// also where the switch store lives, by construction rather than by a second
/// env read (see `biorouter::privacy::master_switch`).
fn config_dir() -> std::path::PathBuf {
    std::path::Path::new(&Config::global().path())
        .parent()
        .expect("config.yaml always has a parent directory")
        .to_path_buf()
}

/// Return this install to its pre-migration state: no store, no retired key.
/// The tests below each start from it, because the fixture's temp config root
/// is shared by every test in this binary and `#[serial]` fixes no order.
fn reset_switch_storage() {
    std::fs::remove_file(config_dir().join("privacy-tiers.json")).ok();
    Config::global()
        .delete(biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY)
        .ok();
}

/// Is the retired key present in the config FILE? Deliberately not
/// `get_param`, whose first branch resolves an environment variable — these
/// tests set that variable, and the question here is only what is on disk.
fn retired_key_on_disk() -> Option<Value> {
    Config::global()
        .all_values()
        .ok()
        .and_then(|m| m.get(biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY).cloned())
}

/// **The gate DR-22 names.** Hand-edit `config.yaml`, restart, and the feature
/// is still on.
///
/// This single test fails the most plausible wrong implementation — the
/// compatibility reader that consults the new store *and* the old key — which
/// no amount of testing the happy path would catch. A reader that still
/// consults `config.yaml` has not closed the channel, it has added a second
/// one.
#[tokio::test]
#[serial_test::serial]
async fn a_hand_edited_config_yaml_cannot_turn_the_feature_off() {
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();

    // Bring the install to the state a daemon reaches on its first start after
    // the upgrade: the migration has run and the store exists.
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "an install that never disabled the feature comes up ON"
    );

    // THE HAND EDIT. Exactly the file, the key and the value a user — or an
    // agent holding `developer__text_editor`, which DR-17 leaves able to write
    // this file — would put there.
    Config::global()
        .set(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            &Value::String("off".to_string()),
            false,
        )
        .unwrap();

    // THE RESTART. A fresh process comes up on the fail-safe default and then
    // loads; `load_privacy_tiers_from_config` is that load, and it is the only
    // thing a restart does to this value.
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    biorouter::privacy::load_privacy_tiers_from_config();

    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "a hand-written `config.yaml` key disabled the master switch across a restart. \
         the retired key must be IGNORED, not honoured for compatibility"
    );
}

/// The mirror. Disabling **through** the authenticated path survives a restart —
/// otherwise the gate above would be satisfied by a switch that simply cannot
/// be turned off, and the test above would pass while the feature was broken.
///
/// It also pins the two consequences of the move that the renderer depends on:
/// the value does **not** land in `config.yaml`, and `/config/read` and
/// `/config` still answer for the key — with the LIVE value, so Settings →
/// Privacy cannot show the user something false about the control they just
/// used.
#[tokio::test]
#[serial_test::serial]
async fn disabling_through_the_authenticated_path_survives_a_restart() {
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(biorouter::privacy::privacy_tiers_enabled());

    // Task 55: a disable also raises DR-20's system prompt, answered here by the
    // test seam. What this test is about is what SURVIVES A RESTART, so the
    // authentication is a precondition of reaching that, not its subject.
    arm_the_system_prompt(biorouter::privacy::system_auth::AuthOutcome::Approved);
    let _ok = upsert_config(
        HeaderMap::new(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "off",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("the confirmed flip is the one write this route allows");
    assert!(!biorouter::privacy::privacy_tiers_enabled());

    // The one door writes the store, and NOT `config.yaml`: leaving a copy
    // there would re-create the channel the move exists to close.
    assert_eq!(
        retired_key_on_disk(),
        None,
        "the master switch must not be written into config.yaml"
    );

    // Both config read paths still answer for the key, from the live value.
    // `ConfigContext` reads the whole map and `PrivacyPanel` reads the single
    // key; a blind reader would paint every badge as if the feature were on.
    match read_config(Json(ConfigKeyQuery {
        key: biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY.to_string(),
        is_secret: false,
    }))
    .await
    .expect("reading the master switch must not fail")
    .0
    {
        ConfigValueResponse::Value(v) => assert_eq!(v, Value::String("off".to_string())),
        ConfigValueResponse::MaskedValue(_) => {
            panic!("the master switch is not a secret and must not be masked")
        }
    }
    assert_eq!(
        read_all_config()
            .await
            .expect("reading the config map must not fail")
            .0
            .config
            .get(biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY),
        Some(&Value::String("off".to_string())),
    );

    // THE RESTART.
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        !biorouter::privacy::privacy_tiers_enabled(),
        "the user's own decision must survive a restart, or the switch is not a switch"
    );
}

/// The fourth state, and the one the gate above cannot reach: a store that
/// EXISTS but does not parse, with the retired key present and `off`.
///
/// ⚠ **Why the gate needs this beside it.** Every test above runs the migration
/// first, so the store is always *readable* by the time the hand-edited key is
/// read — which means one wrong implementation slips through all of them: the
/// compatibility reader written as `read_for(..).or_else(|| the key)`, whose
/// fallback arm is simply never taken. This is the state that takes it.
/// `exists_in` holds the migration shut (deliberately — one corrupt byte must
/// not make the retired key live again) while `read_for` answers `None`, so a
/// reader with a fallback reaches the key and honours it. Nothing recorded and
/// nothing readable both mean ON.
#[tokio::test]
#[serial_test::serial]
async fn an_unreadable_store_falls_back_to_enforcing_and_never_to_the_retired_key() {
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();

    std::fs::write(config_dir().join("privacy-tiers.json"), "{ not json").unwrap();
    Config::global()
        .set(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            &Value::String("off".to_string()),
            false,
        )
        .unwrap();

    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
    biorouter::privacy::load_privacy_tiers_from_config();

    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "an unreadable store fell through to the retired `config.yaml` key: a \
         record that does not parse must resolve to ON, never to whatever the \
         key says"
    );
    assert_eq!(
        retired_key_on_disk(),
        Some(Value::String("off".to_string())),
        "the migration ran against a store that already exists: one corrupt \
         byte must not re-open it"
    );
}

/// Step 2: the retired key is read **once**, at migration, and then ignored.
///
/// The first half is what users who set the key before this version are owed —
/// their answer is carried across rather than silently reset to ON. The second
/// half is why the migration is gated on the STORE's existence and not on the
/// key's: a migration that re-runs whenever the key reappears is the
/// compatibility reader wearing a different hat.
#[tokio::test]
#[serial_test::serial]
async fn the_retired_key_is_migrated_once_and_then_ignored() {
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();

    // An install upgrading from a version where the key WAS the switch.
    Config::global()
        .set(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            &Value::String("off".to_string()),
            false,
        )
        .unwrap();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        !biorouter::privacy::privacy_tiers_enabled(),
        "a value the user set before this version must be carried across, not reset"
    );
    assert!(
        config_dir().join("privacy-tiers.json").exists(),
        "the migration must leave the store behind: its existence is what stops \
         the migration running a second time"
    );
    assert_eq!(
        retired_key_on_disk(),
        None,
        "the migration must REMOVE the key, not leave it beside the store to drift"
    );

    // The user turns it back on through the one door…
    let _ok = upsert_config(
        HeaderMap::new(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "on",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("re-enabling goes through the same door");
    assert!(biorouter::privacy::privacy_tiers_enabled());

    // …and writing the key back by hand never migrates again.
    Config::global()
        .set(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            &Value::String("off".to_string()),
            false,
        )
        .unwrap();
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "the retired key was read a second time; 'once, at migration' means once"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// H3 (2026-09-10 security test drive): an OFF switch is announced, and says how
// it got there.
//
// The record is agent-writable — DR-17's accepted risk, unchanged here. What the
// drive measured on top of it was the SILENCE: `{"enabled": false}` written from
// a chat's shell disabled every gate at the next launch and nothing in the app
// said so. The config surface the renderer already reads now carries the record
// beside the switch — where it lives and which door last wrote it — so the
// banner can say "off, and turned off outside the app".
// ─────────────────────────────────────────────────────────────────────────────

/// The wire name, as a literal. `settings/privacy/privacyTiers.ts` mirrors it,
/// and a literal here pins what the renderer reads rather than agreeing with a
/// Rust constant whatever it happens to say.
const RECORD_KEY: &str = "BIOROUTER_PRIVACY_TIERS_RECORD";

/// What BOTH config read paths say about the record — asserted equal, because
/// `ConfigContext` reads the map and a single-key reader must not be told
/// something different.
async fn record_on_the_surface() -> Value {
    let from_map = read_all_config()
        .await
        .expect("reading the config map must not fail")
        .0
        .config
        .get(RECORD_KEY)
        .cloned()
        .unwrap_or(Value::Null);
    let from_key = match read_config(Json(ConfigKeyQuery {
        key: RECORD_KEY.to_string(),
        is_secret: false,
    }))
    .await
    .expect("reading the record must not fail")
    .0
    {
        ConfigValueResponse::Value(v) => v,
        ConfigValueResponse::MaskedValue(_) => panic!("the record is not a secret"),
    };
    assert_eq!(from_map, from_key, "the two config read paths disagree");
    from_map
}

fn record_path() -> String {
    config_dir()
        .join("privacy-tiers.json")
        .display()
        .to_string()
}

/// THE MEASURED WRITE, then a restart: the surface reports OFF, where the record
/// is, and that no door the app records wrote it.
#[tokio::test]
#[serial_test::serial]
async fn a_record_turned_off_outside_the_app_is_reported_as_such_through_the_config_surface() {
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();
    biorouter::privacy::load_privacy_tiers_from_config();

    // Byte for byte what the drive wrote from `developer__shell`.
    std::fs::write(
        config_dir().join("privacy-tiers.json"),
        r#"{"enabled":false}"#,
    )
    .unwrap();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(
        !biorouter::privacy::privacy_tiers_enabled(),
        "the file channel is DR-17's accepted risk: the loader still obeys it"
    );

    let record = record_on_the_surface().await;
    assert_eq!(record["enabled"], Value::Bool(false), "{record}");
    assert_eq!(
        record["origin"],
        Value::String("unrecorded".to_string()),
        "an OFF record no door recorded writing must be flagged: {record}"
    );
    assert_eq!(record["path"], Value::String(record_path()), "{record}");
    assert_eq!(record["last_change"], Value::Null, "{record}");

    // A copy of the key written into `config.yaml` — which `/config/upsert`
    // will do for any key — must not be what the surface serves: the report is
    // the daemon's, not a value the agent can pre-fill.
    Config::global()
        .set(
            RECORD_KEY,
            &serde_json::json!({"enabled": false, "origin": "settings"}),
            false,
        )
        .unwrap();
    assert_eq!(
        record_on_the_surface().await["origin"],
        Value::String("unrecorded".to_string()),
        "a forged copy in config.yaml reached the surface"
    );
    Config::global().delete(RECORD_KEY).ok();
}

/// The mirror: the deliberate path is reported as deliberate — live, the moment
/// it lands, and again after a restart reads it back from disk. Without this,
/// the test above would be satisfied by a surface that calls every OFF
/// "outside the app".
#[tokio::test]
#[serial_test::serial]
async fn a_deliberate_change_is_reported_as_deliberate_live_and_after_a_restart() {
    let _env = env_lock::lock_env([("BIOROUTER_PRIVACY_TEST_AUTH", None::<&str>)]);
    let _fixture = PrivacyToggleFixture::capture();
    install_user_action_key_once();
    reset_switch_storage();
    biorouter::privacy::load_privacy_tiers_from_config();

    arm_the_system_prompt(biorouter::privacy::system_auth::AuthOutcome::Approved);
    let _ok = upsert_config(
        user_action_headers(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "off",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("the confirmed, authenticated flip is the deliberate door");
    assert!(!biorouter::privacy::privacy_tiers_enabled());

    let live = record_on_the_surface().await;
    assert_eq!(live["enabled"], Value::Bool(false), "{live}");
    assert_eq!(
        live["origin"],
        Value::String("settings".to_string()),
        "{live}"
    );
    assert_eq!(live["path"], Value::String(record_path()), "{live}");
    let change = &live["last_change"];
    assert_eq!(
        change["via"],
        Value::String("settings".to_string()),
        "{live}"
    );
    assert_eq!(change["set_to"], Value::Bool(false), "{live}");
    assert_eq!(change["system_authenticated"], Value::Bool(true), "{live}");
    assert_eq!(change["user_action"], Value::Bool(true), "{live}");
    assert!(
        change["at"].as_str().is_some_and(|at| !at.is_empty()),
        "the deliberate change must say when: {live}"
    );

    // THE RESTART reads the stamp back off the disk.
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);
    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(!biorouter::privacy::privacy_tiers_enabled());
    let reloaded = record_on_the_surface().await;
    assert_eq!(
        reloaded["origin"],
        Value::String("settings".to_string()),
        "the deliberate stamp did not survive a restart: {reloaded}"
    );
    assert_eq!(reloaded["last_change"], live["last_change"], "{reloaded}");
}

/// The edit an agent is likeliest to make is not an overwrite but a one-field
/// flip — `jq '.enabled = false'` — which keeps whatever else the record held.
/// A stamp that named only its door would then vouch for a value it never
/// wrote, so the stamp names the value too, and a disagreement is flagged.
#[tokio::test]
#[serial_test::serial]
async fn an_edit_that_flips_only_the_value_is_not_mistaken_for_a_deliberate_change() {
    let _fixture = PrivacyToggleFixture::capture();
    reset_switch_storage();
    biorouter::privacy::load_privacy_tiers_from_config();

    // A deliberate ON, through the door — which needs no system prompt.
    let _ok = upsert_config(
        HeaderMap::new(),
        Json(upsert(
            biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY,
            "on",
            Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE),
        )),
    )
    .await
    .expect("re-enabling goes through the same door");

    // THE ONE-FIELD FLIP, keeping every other byte of the stamped record.
    let path = config_dir().join("privacy-tiers.json");
    let mut on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .expect("the door writes JSON");
    on_disk["enabled"] = Value::Bool(false);
    std::fs::write(&path, serde_json::to_string_pretty(&on_disk).unwrap()).unwrap();

    biorouter::privacy::load_privacy_tiers_from_config();
    assert!(!biorouter::privacy::privacy_tiers_enabled());
    let record = record_on_the_surface().await;
    assert_eq!(
        record["origin"],
        Value::String("unrecorded".to_string()),
        "a stamp that set ON vouched for an OFF it never wrote: {record}"
    );
    assert_eq!(
        record["last_change"]["set_to"],
        Value::Bool(true),
        "the surface keeps what the last recorded change DID set, which is the \
         evidence of the edit: {record}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 52 (DR-27): the cross-institution mixing policy's WRITE DOOR.
//
// ⚠ **These belong here and not beside the route, for the header's reason and
// one more.** The mixing policy is a process-global atomic exactly as the master
// switch is, so a test that moves it inside `biorouter-server`'s lib target would
// be seen by the ~550 tests compiled into that target and into `biorouterd`.
// Beyond that, this file is where the identically-shaped arm of the identical
// handler is already driven against a redirected config root — and Task 52 landed
// the mixing arm with no behavioural test at all, which review found and which
// is what these close: the `X-User-Action` requirement, the closed vocabulary,
// the secret-store refusal, the delete refusal, the direction guard as the HTTP
// caller meets it, and both read paths.
// ─────────────────────────────────────────────────────────────────────────────

/// The user-action key this binary installs, so [`is_user_action`] has something
/// to verify a header against.
///
/// ⚠ `USER_ACTION_DIGEST` is a `OnceLock` and installing is one-shot per process,
/// which is why [`install_user_action_key_once`] exists and why no test may
/// install a different one. Nothing else in this binary sends the header — the
/// master switch's door is the `confirm` phrase — so there is nothing to disturb.
const USER_ACTION_KEY: &str = "task-52-user-action-key";

fn install_user_action_key_once() {
    // Through the slice, exactly as `commands::agent::read_user_action_digest`
    // does with the hex the launcher pipes in: the digest is what the daemon
    // stores, and the raw key is what the header presents.
    let digest = <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes());
    let digest = <[u8; 32]>::try_from(digest.as_slice()).expect("SHA-256 is 32 bytes");
    biorouter_server::auth::install_user_action_digest(Some(digest));
}

/// A request carrying the proof of a human, exactly as Settings > Privacy sends
/// it.
fn user_action_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    headers
}

/// What the mixing policy's RECORD holds — deliberately not
/// `biorouter::privacy::mixing::policy()`, which is the live value. Several
/// assertions below need to tell "refused, and nothing reached the disk" from
/// "refused, and the next launch comes up in the mode the user was refused".
fn mixing_record() -> Option<biorouter::privacy::mixing::MixingPolicy> {
    biorouter::privacy::mixing::read_in(&config_dir())
}

/// Return this install to "no mixing policy recorded", in both places.
fn reset_mixing_storage() {
    std::fs::remove_file(biorouter::privacy::mixing::path_in(&config_dir())).ok();
    biorouter::privacy::load_mixing_policy_from_record();
}

fn mixing_upsert(value: &str, is_secret: bool) -> UpsertConfigQuery {
    UpsertConfigQuery {
        key: biorouter::privacy::mixing::MIXING_POLICY_CONFIG_KEY.to_string(),
        value: Value::String(value.to_string()),
        is_secret,
        confirm: None,
    }
}

/// **Task 52 gate (5), at the door the user actually reaches.**
///
/// The repo walk in `privacy::mixing` proves the proof-of-user is minted in one
/// file and once; it cannot see whether that mint sits BEHIND the header check.
/// Moving `UserMixingPolicyChange::from_user_action()` above the
/// `is_user_action` guard keeps that audit green and hands every agent the
/// setting — so the guard is asserted here, behaviourally, along with the three
/// other ways in that the handler has to refuse.
#[tokio::test]
#[serial_test::serial]
async fn the_mixing_policy_is_user_only_and_takes_one_of_exactly_three_modes() {
    use biorouter::privacy::mixing::MixingPolicy;

    let _fixture = PrivacyToggleFixture::capture();
    install_user_action_key_once();
    reset_mixing_storage();

    // NO PROOF OF A HUMAN. DR-19 applies in every mode, so this is refused
    // whatever the machine is currently set to — and refused with a 409, the
    // status DR-16's arm already uses for "the user decides this".
    let (status, body) = upsert_config(HeaderMap::new(), Json(mixing_upsert("open", false)))
        .await
        .expect_err("an agent may never move the mixing policy");
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body.contains("Do not retry"),
        "the refusal must foreclose the retry, or a model loops on it: {body}"
    );
    assert_eq!(
        mixing_record(),
        None,
        "a refused request must not have written"
    );
    assert_eq!(
        biorouter::privacy::mixing::policy(),
        MixingPolicy::Standard,
        "a refused request must not have moved the live value either"
    );

    // A MODE THAT DOES NOT EXIST. Refused rather than resolved to a default:
    // the two wrong guesses fail in opposite directions, so there is no safe
    // one.
    let (status, body) = upsert_config(
        user_action_headers(),
        Json(mixing_upsert("permissive", false)),
    )
    .await
    .expect_err("the vocabulary is closed");
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("'open'") && body.contains("'standard'") && body.contains("'strict'"),
        "a closed vocabulary can afford to state itself: {body}"
    );
    assert_eq!(mixing_record(), None);

    // THE SECRET STORE. The daemon reads its own record, not the credential
    // store, so a secret write would move this process's value and silently
    // revert at the next launch — the master switch's reason, unchanged.
    let (status, body) = upsert_config(user_action_headers(), Json(mixing_upsert("open", true)))
        .await
        .expect_err("the mixing policy is not a secret");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("secret"), "{body}");
    assert_eq!(mixing_record(), None);

    // DELETE. One predicate, every verb: a delete is not the absence of a write,
    // it is a way to ask for the default back without proving a human or facing
    // the direction guard. Refused even WITH the header, because there is no
    // legitimate caller.
    let (status, body) = remove_config(
        user_action_headers(),
        Json(ConfigKeyQuery {
            key: biorouter::privacy::mixing::MIXING_POLICY_CONFIG_KEY.to_string(),
            is_secret: false,
        }),
    )
    .await
    .expect_err("the mixing policy is set, never deleted");
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.contains("Settings"),
        "the refusal must name the way in: {body}"
    );

    // …and the proven caller, tightening, goes through — moving the RECORD and
    // the LIVE value together. A handler that wrote only the file would leave
    // every gate on the old mode until the next restart.
    let _ok = upsert_config(user_action_headers(), Json(mixing_upsert("strict", false)))
        .await
        .expect("the proven caller is the one this route serves");
    assert_eq!(mixing_record(), Some(MixingPolicy::Strict));
    assert_eq!(biorouter::privacy::mixing::policy(), MixingPolicy::Strict);

    // The value is NOT left in `config.yaml`: a copy there would re-create the
    // agent-writable channel DR-22 moved it out of.
    assert_eq!(
        Config::global().all_values().ok().and_then(|m| m
            .get(biorouter::privacy::mixing::MIXING_POLICY_CONFIG_KEY)
            .cloned()),
        None,
        "the mixing policy must not be written into config.yaml"
    );

    reset_mixing_storage();
}

/// **Task 52 gate (2), through the HTTP door rather than through the seam.**
///
/// `privacy::mixing`'s own table drives all nine cells against a private
/// function with an injected prompter. This asserts the route really reaches
/// that decision: relaxing costs the operating system's approval and tightening
/// costs nothing, measured on a real handler with the real resolver.
///
/// ⚠ The tightening leg leaves the seam **unarmed** on purpose. An unarmed seam
/// refuses, so an implementation that prompted in both directions fails here
/// rather than passing quietly.
#[tokio::test]
#[serial_test::serial]
async fn relaxing_the_mixing_policy_needs_the_operating_system_and_tightening_does_not() {
    use biorouter::privacy::mixing::MixingPolicy;
    use biorouter::privacy::system_auth::AuthOutcome;

    let _env = env_lock::lock_env([("BIOROUTER_PRIVACY_TEST_AUTH", None::<&str>)]);
    let _fixture = PrivacyToggleFixture::capture();
    install_user_action_key_once();
    reset_mixing_storage();

    // Start in `strict`, which is a tightening from the default and therefore
    // free. The seam is unarmed, so a prompt raised here refuses and fails.
    biorouter::privacy::system_auth_seam::reset();
    let _ok = upsert_config(user_action_headers(), Json(mixing_upsert("strict", false)))
        .await
        .expect(
            "entering `strict` must cost nothing: requiring a password to be \
                 careful punishes the careful user",
        );
    assert_eq!(biorouter::privacy::mixing::policy(), MixingPolicy::Strict);

    // Leaving it is the direction that costs. The user's "no" and a machine that
    // cannot ask at all are both refusals, and neither may leave a record that
    // relaxes the machine at the NEXT launch.
    for outcome in [AuthOutcome::Denied, AuthOutcome::Unavailable] {
        arm_the_system_prompt(outcome);
        let (status, body) =
            upsert_config(user_action_headers(), Json(mixing_upsert("open", false)))
                .await
                .expect_err("a refused system authentication must not relax the policy");
        assert_eq!(status, StatusCode::FORBIDDEN, "{outcome:?}: {body}");
        assert_eq!(
            biorouter::privacy::mixing::policy(),
            MixingPolicy::Strict,
            "{outcome:?} relaxed the live policy anyway"
        );
        assert_eq!(
            mixing_record(),
            Some(MixingPolicy::Strict),
            "{outcome:?} recorded a relaxation, so the next launch comes up in `open`"
        );
    }

    // DR-20 point 4: the dialog says what it authorises, and names its own
    // subject rather than one taken off the request.
    let asked = biorouter::privacy::system_auth_seam::last_request()
        .expect("the mixing policy reached the prompter");
    assert!(
        asked.reason.contains("open") && asked.reason.contains("strict"),
        "the prompt did not say which change it authorises: {asked:?}"
    );
    assert_eq!(
        asked.session_ids.len(),
        1,
        "this prompt names one subject, not a list of chats: {asked:?}"
    );

    // …and an approval is what leaving `strict` costs.
    arm_the_system_prompt(AuthOutcome::Approved);
    let _ok = upsert_config(user_action_headers(), Json(mixing_upsert("open", false)))
        .await
        .expect("an approved system authentication is what relaxing costs");
    assert_eq!(biorouter::privacy::mixing::policy(), MixingPolicy::Open);
    assert_eq!(mixing_record(), Some(MixingPolicy::Open));

    // Both config READ paths now report `open`, from the LIVE value. Without
    // these arms `config.get` answers `NotFound` and the panel renders "no mode
    // set" over a control the user has just used.
    match read_config(Json(ConfigKeyQuery {
        key: biorouter::privacy::mixing::MIXING_POLICY_CONFIG_KEY.to_string(),
        is_secret: false,
    }))
    .await
    .expect("reading the mixing policy must not fail")
    .0
    {
        ConfigValueResponse::Value(v) => assert_eq!(v, Value::String("open".to_string())),
        ConfigValueResponse::MaskedValue(_) => {
            panic!("the mixing policy is not a secret and must not be masked")
        }
    }
    assert_eq!(
        read_all_config()
            .await
            .expect("reading the config map must not fail")
            .0
            .config
            .get(biorouter::privacy::mixing::MIXING_POLICY_CONFIG_KEY),
        Some(&Value::String("open".to_string())),
        "a bulk read that skipped the key would report the setting absent on every \
         machine"
    );

    // …and tightening back is free again, from the most permissive mode.
    biorouter::privacy::system_auth_seam::reset();
    let _ok = upsert_config(
        user_action_headers(),
        Json(mixing_upsert("standard", false)),
    )
    .await
    .expect("tightening must never raise a prompt");
    assert_eq!(biorouter::privacy::mixing::policy(), MixingPolicy::Standard);

    // THE RESTART: the user's own decision survives it.
    let _ok = upsert_config(user_action_headers(), Json(mixing_upsert("strict", false)))
        .await
        .expect("tightening is free");
    biorouter::privacy::load_mixing_policy_from_record();
    assert_eq!(
        biorouter::privacy::mixing::policy(),
        MixingPolicy::Strict,
        "the start-up load did not restore the recorded mode, so a `strict` machine \
         comes up as `standard`"
    );

    reset_mixing_storage();
}
