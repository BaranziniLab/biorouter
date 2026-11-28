//! The start migration of a renamed extension id (`gptragent` -> `gptr`), end
//! to end through the public functions, on a fake config root: the config
//! entry, the install folder and the provenance record move once, every reader
//! loads the current id before, during and after the move, and the private
//! classification never drops. On Unix, the Rust installer installs an old id
//! under its current name.
//!
//! No network: the fake `uv` below answers `--version` and `sync` locally.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use biorouter::agents::ExtensionConfig;
use biorouter::config::extensions::{
    extension_entry_is_persisted, get_all_extension_names, get_extension_entry_by_name,
    ExtensionEntry,
};
use biorouter::config::paths::Paths;
use biorouter::extension_install::brxt::extensions_root;
use biorouter::extension_rename::{
    migrate_on_start, migrate_stopping_after, MigrationEnv, Phase, RenameOutcome, JOURNAL_DIR,
};
use biorouter::privacy::provenance::{
    extension_provenance_for_key, marketplace_installs_for_registry_id, provenance_path, record,
    ExtensionProvenance,
};
use biorouter::privacy::{classify_extension_entry, ProviderTier};

/// The renames are compiled in, so every test here works on the same two ids
/// and must not overlap another.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn sandbox() -> &'static Path {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
        std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::write(root.path().join("config/config.yaml"), "{}\n").unwrap();
        std::fs::write(root.path().join("config/secrets.yaml"), "{}\n").unwrap();
        #[cfg(unix)]
        install_fake_uv(root.path());
        root
    })
    .path()
}

/// A `uv` that builds nothing and records where each `sync` ran.
#[cfg(unix)]
fn install_fake_uv(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let uv = bin.join("uv");
    std::fs::write(
        &uv,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'uv 0.9.0\n'
  exit 0
fi
if [ "$1" != "sync" ]; then exit 64; fi
mkdir -p .venv
pwd -P >> "$BIOROUTER_TEST_UV_SYNC_LOG"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&uv, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("BIOROUTER_TEST_UV_SYNC_LOG", root.join("uv-sync.log"));
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
}

fn config_file() -> PathBuf {
    sandbox().join("config/config.yaml")
}

fn old_dir() -> PathBuf {
    extensions_root().join("gptragent")
}

fn new_dir() -> PathBuf {
    extensions_root().join("gptr")
}

fn journal_dir() -> PathBuf {
    Paths::in_config_dir(JOURNAL_DIR)
}

/// Back to a machine that never had either id.
fn reset() {
    sandbox();
    std::fs::write(config_file(), "{}\n").unwrap();
    for dir in [old_dir(), new_dir(), extensions_root().join("CDWAgent")] {
        let _ = std::fs::remove_dir_all(dir);
    }
    let provenance = provenance_path();
    let _ = std::fs::remove_file(&provenance);
    let mut journal = provenance.into_os_string();
    journal.push(".d");
    let _ = std::fs::remove_dir_all(PathBuf::from(journal));
    let _ = std::fs::remove_dir_all(journal_dir());
    let _ = std::fs::remove_file(sandbox().join("uv-sync.log"));
}

fn old_config() -> ExtensionConfig {
    ExtensionConfig::Stdio {
        name: "gptragent".to_owned(),
        description: "Work in a live R session.".to_owned(),
        cmd: "uv".to_owned(),
        args: vec![
            "run".to_owned(),
            "--directory".to_owned(),
            old_dir().display().to_string(),
            "gptragent".to_owned(),
        ],
        envs: Default::default(),
        env_keys: Vec::new(),
        timeout: Some(300),
        bundled: None,
        available_tools: Vec::new(),
    }
}

/// A marketplace install of `gptragent` as release 1.94.6 left it. No
/// `pyproject.toml`: where the fake `uv` is not on the path (Windows), the
/// move's background `uv sync` then fails at once instead of resolving a
/// project.
fn install_old() {
    std::fs::create_dir_all(old_dir().join(".venv")).unwrap();
    std::fs::write(old_dir().join(".venv/marker"), "venv").unwrap();
    let entries = serde_yaml::to_string(&serde_yaml::Mapping::from_iter([(
        serde_yaml::Value::from("extensions"),
        serde_yaml::to_value(indexmap::IndexMap::from([(
            "gptragent".to_owned(),
            ExtensionEntry {
                enabled: true,
                config: old_config(),
            },
        )]))
        .unwrap(),
    )]))
    .unwrap();
    std::fs::write(config_file(), entries).unwrap();
    record(
        "gptragent",
        ExtensionProvenance {
            install_id: Some("11111111-2222-3333-4444-555555555555".to_owned()),
            registry_id: "gptragent".to_owned(),
            install_dir: Some(old_dir().display().to_string()),
            source_url: Some(
                "https://github.com/Broccolito/GptrAgent/releases/download/v0.1.2/gptragent.brxt"
                    .to_owned(),
            ),
            bundle_sha256: Some("6de59047".to_owned()),
            recorded_at: Some("2026-11-01T00:00:00Z".to_owned()),
        },
    )
    .unwrap();
}

/// `config.yaml`'s `extensions` keys, read off the file itself.
fn stored_keys() -> Vec<String> {
    let text = std::fs::read_to_string(config_file()).unwrap();
    let values: serde_yaml::Mapping = serde_yaml::from_str(&text).unwrap();
    match values.get("extensions") {
        Some(serde_yaml::Value::Mapping(extensions)) => extensions
            .keys()
            .filter_map(|key| key.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn folder_arg(config: &ExtensionConfig) -> PathBuf {
    match config {
        ExtensionConfig::Stdio { args, .. } => PathBuf::from(&args[2]),
        other => panic!("a stdio entry: {other:?}"),
    }
}

/// What every reader in this process loads for the extension, whatever state
/// the move is in: the current id, a folder that exists, and Private.
fn assert_loads_as_gptr(when: &str) {
    for name in ["gptragent", "gptr"] {
        let entry = get_extension_entry_by_name(name)
            .unwrap_or_else(|| panic!("{when}: {name} finds the extension"));
        assert_eq!(entry.config.name(), "gptr", "{when}");
        assert!(
            folder_arg(&entry.config).join(".venv/marker").is_file(),
            "{when}: it runs from {}",
            folder_arg(&entry.config).display()
        );
        assert_eq!(
            classify_extension_entry(&entry.config.key(), Some(&entry.config)),
            ProviderTier::Private,
            "{when}: the move never makes it public, not even for one call"
        );
    }
    let names = get_all_extension_names();
    assert!(names.iter().any(|name| name == "gptr"), "{when}: {names:?}");
    assert!(
        !names.iter().any(|name| name == "gptragent"),
        "{when}: {names:?}"
    );
    assert!(extension_entry_is_persisted("gptr"), "{when}");
}

fn assert_moved() {
    assert_eq!(stored_keys(), ["gptr"]);
    let entry = get_extension_entry_by_name("gptr").unwrap();
    assert_eq!(folder_arg(&entry.config), new_dir());
    assert!(!old_dir().exists());
    assert!(new_dir().join(".venv/marker").is_file());

    let record = extension_provenance_for_key("gptr").expect("the record moved");
    assert_eq!(record.registry_id, "gptragent");
    assert_eq!(
        record.install_dir.as_deref(),
        Some(new_dir().display().to_string().as_str())
    );
    assert!(extension_provenance_for_key("gptragent").is_none());
    assert_eq!(
        marketplace_installs_for_registry_id("gptragent").len(),
        1,
        "a marketplace uninstall finds exactly one package"
    );
    let journals = std::fs::read_dir(journal_dir())
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(journals, 0, "a finished move leaves no journal");
}

#[cfg(unix)]
fn wait_for_resync_in(dir: &Path) {
    let wanted = dir.canonicalize().unwrap();
    let log = sandbox().join("uv-sync.log");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let synced = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .any(|line| Path::new(line) == wanted);
        if synced {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the moved environment was never rebuilt in {}",
            wanted.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test]
async fn the_daemon_start_moves_an_old_install_once_and_every_reader_follows() {
    let _serial = SERIAL.lock().await;
    reset();
    install_old();
    assert_loads_as_gptr("before the move");

    migrate_on_start();
    assert_moved();
    assert_loads_as_gptr("after the move");
    #[cfg(unix)]
    wait_for_resync_in(&new_dir());

    let before = std::fs::read(config_file()).unwrap();
    migrate_on_start();
    assert_eq!(
        std::fs::read(config_file()).unwrap(),
        before,
        "every start after the first writes nothing"
    );
    assert_moved();
}

#[tokio::test]
async fn a_start_interrupted_after_any_step_keeps_running_and_finishes_next_time() {
    let _serial = SERIAL.lock().await;
    for phase in Phase::ALL {
        reset();
        install_old();
        let outcomes = migrate_stopping_after(&MigrationEnv::global(), phase);
        let outcome = outcomes
            .iter()
            .find(|(rename, _)| rename.from == "gptragent")
            .map(|(_, outcome)| outcome.clone());
        assert_eq!(outcome, Some(RenameOutcome::Stopped(phase)));
        assert_loads_as_gptr(&format!("stopped after {phase:?}"));

        migrate_on_start();
        assert_moved();
        assert_loads_as_gptr(&format!("finished after {phase:?}"));
    }
}

/// A writer that still says `gptragent` (an older desktop's install form, a
/// hand-made call) writes the current entry, never a second one beside it:
/// two entries would load two copies of the same tools.
#[tokio::test]
async fn an_entry_written_under_the_old_id_is_the_current_entry() {
    let _serial = SERIAL.lock().await;
    reset();
    install_old();
    migrate_on_start();
    let mut rewritten = old_config();
    if let ExtensionConfig::Stdio { timeout, .. } = &mut rewritten {
        *timeout = Some(42);
    }
    biorouter::config::extensions::set_extension(ExtensionEntry {
        enabled: false,
        config: rewritten,
    });
    let keys = stored_keys();
    assert!(keys.iter().any(|key| key == "gptr"), "{keys:?}");
    assert!(!keys.iter().any(|key| key == "gptragent"), "{keys:?}");
    let entry = get_extension_entry_by_name("gptr").unwrap();
    assert!(!entry.enabled);
    let ExtensionConfig::Stdio { name, timeout, .. } = &entry.config else {
        panic!("a stdio entry: {:?}", entry.config)
    };
    assert_eq!((name.as_str(), *timeout), ("gptr", Some(42)));
    assert_eq!(
        folder_arg(&entry.config),
        new_dir(),
        "an old folder argument follows the moved folder"
    );
}

/// A local install records no marketplace provenance, and a build older than
/// the rename knows `gptragent` but not `gptr`. Once an entry says `gptr`, a
/// record keyed `gptr` naming the old registry id is what keeps it private in
/// such a build (`extension_rename::identity_record`).
fn assert_carries_a_record_an_older_build_reads() {
    let record = extension_provenance_for_key("gptr").expect("a record keyed by the current id");
    assert_eq!(record.registry_id, "gptragent");
    assert_eq!(
        record.install_dir.as_deref(),
        Some(new_dir().display().to_string().as_str())
    );
    assert_eq!(record.source_url, None);
    assert!(
        marketplace_installs_for_registry_id("gptragent").is_empty(),
        "a local install is never taken for a marketplace package"
    );
}

/// The desktop's file install registers its entry through the daemon's config
/// write, under the current id, with no provenance at all.
#[tokio::test]
async fn an_entry_written_under_the_current_id_without_a_record_is_given_one() {
    let _serial = SERIAL.lock().await;
    reset();
    std::fs::create_dir_all(new_dir()).unwrap();
    let mut config = old_config();
    if let ExtensionConfig::Stdio { name, args, .. } = &mut config {
        *name = "gptr".to_owned();
        args[2] = new_dir().display().to_string();
    }
    biorouter::config::extensions::set_extension(ExtensionEntry {
        enabled: true,
        config,
    });
    assert_carries_a_record_an_older_build_reads();

    // Written once: a second write leaves the record as it is.
    let first = extension_provenance_for_key("gptr").unwrap();
    biorouter::config::extensions::set_extension_enabled("gptr", false);
    assert_eq!(extension_provenance_for_key("gptr"), Some(first));
}

#[cfg(unix)]
mod installers {
    use super::*;
    use biorouter::extension_install::{
        BrxtManifest, CredentialPolicy, ExtensionInstallTransaction, InstallSource, InstallState,
    };
    use std::io::Write;

    fn bundle(dir: &Path, name: &str, entry_point: &str) -> PathBuf {
        let manifest = BrxtManifest {
            name: name.to_owned(),
            display_name: "gptr Agent".to_owned(),
            description: "fixture".to_owned(),
            version: "0.1.2".to_owned(),
            entry_point: entry_point.to_owned(),
            repository: "https://github.com/Broccolito/gptr".to_owned(),
            tools_count: None,
            env_vars: Vec::new(),
        };
        let path = dir.join(format!("{name}.brxt"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::FileOptions::default();
        for (entry, bytes) in [
            ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
            ("README.md", b"# fixture\n".to_vec()),
            (
                "pyproject.toml",
                format!("[project]\nname = '{entry_point}'\nversion = '0.1.2'\n").into_bytes(),
            ),
            ("src/server.py", b"# fixture\n".to_vec()),
        ] {
            zip.start_file(entry, options).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    /// An install saves the whole extension map, built-in entries included.
    fn assert_gptr_stored_and_gptragent_not() {
        let keys = stored_keys();
        assert!(keys.iter().any(|key| key == "gptr"), "{keys:?}");
        assert!(!keys.iter().any(|key| key == "gptragent"), "{keys:?}");
    }

    fn args(config: &ExtensionConfig) -> Vec<String> {
        match config {
            ExtensionConfig::Stdio { args, .. } => args.clone(),
            other => panic!("a stdio entry: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_bundle_under_the_old_id_installs_as_the_current_one() {
        let _serial = SERIAL.lock().await;
        reset();
        let bundles = tempfile::tempdir().unwrap();
        let report = ExtensionInstallTransaction::new(InstallSource::LocalFile {
            path: bundle(bundles.path(), "gptragent", "gptragent"),
        })
        .run(CredentialPolicy::Refuse, None)
        .await;
        assert_eq!(report.state, InstallState::Installed, "{report:?}");
        assert_eq!(report.extension_name.as_deref(), Some("gptr"));
        assert_gptr_stored_and_gptragent_not();
        let entry = get_extension_entry_by_name("gptr").unwrap();
        assert_eq!(entry.config.name(), "gptr");
        assert_eq!(
            args(&entry.config),
            [
                "run".to_owned(),
                "--directory".to_owned(),
                new_dir().display().to_string(),
                "gptragent".to_owned(),
            ],
            "the folder is the current id; the entry point is the bundle's own script"
        );
        assert!(new_dir().join("manifest.json").is_file());
        assert!(!old_dir().exists());
        assert_carries_a_record_an_older_build_reads();

        // Nothing for the daemon to move afterwards.
        migrate_on_start();
        assert_gptr_stored_and_gptragent_not();
        assert!(new_dir().join("manifest.json").is_file());
    }

    #[tokio::test]
    async fn every_other_bundle_keeps_its_exact_name() {
        let _serial = SERIAL.lock().await;
        reset();
        let bundles = tempfile::tempdir().unwrap();
        let report = ExtensionInstallTransaction::new(InstallSource::LocalFile {
            path: bundle(bundles.path(), "CDWAgent", "cdwagent"),
        })
        .run(CredentialPolicy::Refuse, None)
        .await;
        assert_eq!(report.state, InstallState::Installed, "{report:?}");
        assert_eq!(report.extension_name.as_deref(), Some("CDWAgent"));
        let entry = get_extension_entry_by_name("CDWAgent").unwrap();
        assert_eq!(entry.config.name(), "CDWAgent");
        assert_eq!(
            folder_arg(&entry.config),
            extensions_root().join("CDWAgent")
        );
        reset();
    }

    #[tokio::test]
    async fn a_marketplace_row_refuses_a_bundle_of_another_extension() {
        let _serial = SERIAL.lock().await;
        reset();
        let bundles = tempfile::tempdir().unwrap();
        let refused = ExtensionInstallTransaction::new(InstallSource::LocalFile {
            path: bundle(bundles.path(), "spokeagent", "spokeagent"),
        })
        .expect_extension("gptragent")
        .run(CredentialPolicy::Refuse, None)
        .await;
        let InstallState::Failed { reason } = &refused.state else {
            panic!("another extension's bundle is refused: {refused:?}")
        };
        assert!(reason.contains("spokeagent"), "{reason}");
        assert!(!extensions_root().join("spokeagent").exists());
        assert!(get_extension_entry_by_name("spokeagent").is_none());

        // The renamed bundle is the same extension as the row's old id.
        let accepted = ExtensionInstallTransaction::new(InstallSource::LocalFile {
            path: bundle(bundles.path(), "gptr", "gptr"),
        })
        .expect_extension("gptragent")
        .run(CredentialPolicy::Refuse, None)
        .await;
        assert_eq!(accepted.state, InstallState::Installed, "{accepted:?}");
        assert_eq!(accepted.extension_name.as_deref(), Some("gptr"));
        reset();
    }
}
