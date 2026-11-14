//! Issue #112: **a write to the extension map is an event, or nothing sees it.**
//!
//! The reported bug was two extensions that installed correctly, showed as
//! enabled in `biorouter extension list`, and could not be attached to the chat
//! that had just asked for them. Everything downstream — the Settings list, the
//! composer's picker, the running agent — refetches when it is told to, so the
//! whole failure was that nobody told it.
//!
//! These tests therefore watch the *choke points*, not the surfaces: if
//! `set_extension` and its two siblings publish, every inventory is reachable;
//! if they do not, no amount of correct rendering downstream can help.
//!
//! Its own binary with `BIOROUTER_PATH_ROOT` on a temp tree — these write
//! `config.yaml`, and must never touch the developer's real one.
//!
//! ⚠ **`#[serial]` is load-bearing, not tidiness.** Every test here mutates two
//! things the whole binary shares: one `config.yaml` and one process-global
//! revision counter. They happen to pass interleaved today because they use
//! distinct extension names — but `a_change_made_outside_this_process_is_
//! detected_exactly_once` deliberately rewinds the watcher's snapshot, which
//! would make any test running beside it see a catalogue full of changes it did
//! not make.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::OnceLock;

use biorouter::agents::extension::{Envs, ExtensionConfig};
use biorouter::catalog::{CatalogChangeReason, CatalogEntryChange, CatalogEvents};
use biorouter::config::extensions::{
    get_all_extensions, remove_extension, set_extension, set_extension_enabled, ExtensionEntry,
};
use serial_test::serial;

fn sandbox() {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::env::set_var("BIOROUTER_PATH_ROOT", dir.path());
        std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
        for sub in ["config", "data", "state"] {
            std::fs::create_dir_all(dir.path().join(sub)).expect("a sandbox tree");
        }
        std::fs::write(dir.path().join("config/config.yaml"), "{}\n").expect("a config file");
        std::fs::write(dir.path().join("config/secrets.yaml"), "{}\n").expect("a secrets file");
        dir
    });
}

fn entry(name: &str, enabled: bool, arg: &str) -> ExtensionEntry {
    ExtensionEntry {
        enabled,
        config: ExtensionConfig::Stdio {
            name: name.to_string(),
            description: String::new(),
            cmd: "uv".to_string(),
            args: vec!["run".to_string(), arg.to_string()],
            envs: Envs::default(),
            env_keys: Vec::new(),
            timeout: Some(300),
            bundled: None,
            available_tools: Vec::new(),
        },
    }
}

/// The single most load-bearing assertion in the issue: installing an extension
/// moves the revision every inventory in the app is following.
#[test]
#[serial]
fn installing_an_extension_publishes_a_catalogue_change() {
    sandbox();
    let events = CatalogEvents::global();
    let before = events.revision();

    set_extension(entry("cat-install", true, "a"));

    let delta = events.since(before);
    assert!(
        delta.revision > before,
        "a new extension did not move the catalogue revision"
    );
    let row = delta
        .changes
        .iter()
        .flat_map(|c| c.extensions.iter())
        .find(|e| e.key == "cat-install")
        .expect("the installed extension is named in the delta");
    assert_eq!(row.change, CatalogEntryChange::Added);
    assert!(row.enabled);
    assert!(
        row.config.is_some(),
        "the row carries the config, so a consumer can repair a row without a refetch"
    );
    assert!(delta
        .changes
        .iter()
        .any(|c| c.reason == CatalogChangeReason::Install));

    remove_extension("cat-install");
}

/// A toggle and a reconfiguration are different events. A surface that could not
/// tell them apart would rebuild a whole row for a checkbox.
#[test]
#[serial]
fn a_toggle_and_a_reconfiguration_publish_different_events() {
    sandbox();
    let events = CatalogEvents::global();
    set_extension(entry("cat-toggle", true, "a"));

    let before = events.revision();
    set_extension_enabled("cat-toggle", false);
    let row = events
        .since(before)
        .changes
        .iter()
        .flat_map(|c| c.extensions.iter())
        .find(|e| e.key == "cat-toggle")
        .expect("the toggle is published")
        .clone();
    assert_eq!(row.change, CatalogEntryChange::Disabled);
    assert!(!row.enabled);

    let before = events.revision();
    set_extension(entry("cat-toggle", false, "b"));
    let row = events
        .since(before)
        .changes
        .iter()
        .flat_map(|c| c.extensions.iter())
        .find(|e| e.key == "cat-toggle")
        .expect("the reconfiguration is published")
        .clone();
    assert_eq!(row.change, CatalogEntryChange::Updated);

    remove_extension("cat-toggle");
}

/// A write that changed nothing is not an event. Publishing one would wake every
/// client in the app to refetch an identical inventory — and the file watcher
/// rewrites its snapshot on every touch, so this happens more often than it
/// looks.
#[test]
#[serial]
fn a_write_that_changes_nothing_publishes_nothing() {
    sandbox();
    let events = CatalogEvents::global();
    set_extension(entry("cat-noop", true, "a"));

    let before = events.revision();
    // Same state, twice more.
    set_extension_enabled("cat-noop", true);
    remove_extension("cat-noop-that-was-never-there");
    assert_eq!(events.revision(), before);

    remove_extension("cat-noop");
}

/// Removing an extension announces it and drops the config, because there is no
/// longer one to carry.
#[test]
#[serial]
fn removing_an_extension_publishes_a_removal_without_a_config() {
    sandbox();
    let events = CatalogEvents::global();
    set_extension(entry("cat-remove", true, "a"));

    let before = events.revision();
    remove_extension("cat-remove");

    let row = events
        .since(before)
        .changes
        .iter()
        .flat_map(|c| c.extensions.iter())
        .find(|e| e.key == "cat-remove")
        .expect("the removal is published")
        .clone();
    assert_eq!(row.change, CatalogEntryChange::Removed);
    assert!(row.config.is_none());
    assert!(!row.enabled);
}

/// ⚠ **This is the CLI case, and it is the one the bug report was about.**
///
/// `biorouter extension install` writes `config.yaml` from a different process,
/// so it reaches none of the choke points above. The daemon's watcher notices by
/// comparing what it last knew against what is on disk — and must report only
/// what *it* did not do, or every write would be announced twice.
#[test]
#[serial]
fn a_change_made_outside_this_process_is_detected_exactly_once() {
    sandbox();
    let events = CatalogEvents::global();

    // The daemon's baseline, as `spawn_config_watcher` establishes it.
    events.sync_snapshot(&get_all_extensions());

    // A write this process DID make announces itself and leaves the watcher
    // nothing to find.
    set_extension(entry("cat-external", true, "a"));
    assert!(
        events
            .detect_external_change(&get_all_extensions())
            .is_empty(),
        "the watcher re-announced a write the choke point had already published"
    );

    // Now simulate the other process: change the map behind the snapshot by
    // rewinding what the watcher believes.
    events.sync_snapshot(&[]);
    let changes = events.detect_external_change(&get_all_extensions());
    assert!(
        changes
            .iter()
            .any(|c| c.key == "cat-external" && c.change == CatalogEntryChange::Added),
        "a change made outside this process was not detected: {changes:?}"
    );

    // And a second look reports nothing: the watcher adopted it.
    assert!(events
        .detect_external_change(&get_all_extensions())
        .is_empty());

    remove_extension("cat-external");
}

/// ⚠ An identical rewrite is not a change.
///
/// `syncBundledExtensions` and the capability migrations both re-save entries at
/// every startup. Announcing those would have every client in the app refetch
/// its whole inventory on launch, every launch — a refetch storm dressed up as
/// freshness.
#[test]
#[serial]
fn rewriting_an_entry_unchanged_publishes_nothing() {
    sandbox();
    let events = CatalogEvents::global();
    set_extension(entry("cat-rewrite", true, "a"));

    let before = events.revision();
    set_extension(entry("cat-rewrite", true, "a"));
    set_extension(entry("cat-rewrite", true, "a"));
    assert_eq!(
        events.revision(),
        before,
        "an identical rewrite moved the revision"
    );

    // ...but a real edit still does.
    set_extension(entry("cat-rewrite", true, "b"));
    assert!(events.revision() > before);

    remove_extension("cat-rewrite");
}
