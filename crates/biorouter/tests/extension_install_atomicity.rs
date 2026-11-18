#![cfg(unix)]

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use biorouter::config::extensions::get_extension_entry_by_name;
use biorouter::config::Config;
use biorouter::extension_install::brxt::extensions_root;
use biorouter::extension_install::claim::claims_dir;
use biorouter::extension_install::{
    BrxtEnvVar, BrxtManifest, ClaimPhase, ClaimSource, CredentialPolicy,
    ExtensionInstallTransaction, InstallClaim, InstallReport, InstallSource, InstallState,
};
use biorouter::privacy::provenance::{extension_provenance_for_key, record, ExtensionProvenance};

fn sandbox() -> &'static Path {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
        std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
        std::fs::create_dir_all(root.path().join("config")).unwrap();
        std::fs::write(root.path().join("config/config.yaml"), "{}\n").unwrap();
        std::fs::write(root.path().join("config/secrets.yaml"), "{}\n").unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let uv = bin.join("uv");
        // Model uv's absolute interpreter paths and a sync that fails after
        // changing its environment, without resolving any real dependencies.
        std::fs::write(
            &uv,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'uv 0.9.0\n'
  exit 0
fi
if [ "$1" != "sync" ]; then exit 64; fi
mkdir -p .venv/bin
version=$(cat fixture-version)
ln -sf /bin/sh .venv/bin/python
printf '#!%s/.venv/bin/python\nprintf "runtime-%s\\n"\n' "$PWD" "$version" > .venv/bin/fixture
chmod +x .venv/bin/fixture
if [ "$version" = "2.0.0" ] && [ -f sync-channel ]; then
  channel=$(cat sync-channel)
  printf 'ready\n' > "$channel/ready"
  IFS= read -r outcome < "$channel/release"
  printf 'synthetic controlled sync failure\n' >&2
  exit 19
fi
if [ -f fail-sync ]; then
  printf 'synthetic dependency sync failure\n' >&2
  exit 19
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&uv, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        root
    })
    .path()
}

struct Fixture {
    _bundle_root: tempfile::TempDir,
    name: String,
    path: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        sandbox();
        let bundle_root = tempfile::tempdir().unwrap();
        Self {
            name: format!("l7{label}{}", uuid::Uuid::new_v4().simple()),
            path: bundle_root.path().join("fixture.brxt"),
            _bundle_root: bundle_root,
        }
    }

    fn install_dir(&self) -> PathBuf {
        extensions_root().join(&self.name)
    }

    fn secret_key(&self) -> String {
        format!("{}_PASSCODE", self.name.to_ascii_uppercase())
    }

    fn write_bundle(&self, version: &str, keys: &[String], extra: &[(&str, &[u8])]) {
        let manifest = BrxtManifest {
            name: self.name.clone(),
            display_name: self.name.clone(),
            description: format!("version {version}"),
            version: version.to_owned(),
            entry_point: "src/main.py".to_owned(),
            repository: "https://example.invalid/l7-fixture".to_owned(),
            tools_count: None,
            env_vars: keys
                .iter()
                .map(|key| BrxtEnvVar {
                    key: key.clone(),
                    required: true,
                    auto_propagate: false,
                    default: None,
                    description: "Synthetic fixture credential".to_owned(),
                    secret: true,
                })
                .collect(),
        };
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&self.path).unwrap());
        let options = zip::write::FileOptions::default();
        for (entry, bytes) in [
            ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
            ("README.md", b"# Synthetic fixture\n".to_vec()),
            (
                "pyproject.toml",
                b"[project]\nname = 'l7-fixture'\nversion = '0.0.1'\ndependencies = []\n".to_vec(),
            ),
            ("src/main.py", format!("# version {version}\n").into_bytes()),
            ("fixture-version", version.as_bytes().to_vec()),
        ] {
            zip.start_file(entry, options).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        for (entry, bytes) in extra {
            zip.start_file(*entry, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn transaction(&self) -> ExtensionInstallTransaction {
        ExtensionInstallTransaction::new(InstallSource::LocalFile {
            path: self.path.clone(),
        })
    }

    async fn seed_working_install(&self, enabled: bool) {
        let key = self.secret_key();
        self.write_bundle("1.0.0", std::slice::from_ref(&key), &[]);
        let report = self
            .transaction()
            .enabled(enabled)
            .with_values(HashMap::from([(
                key,
                "synthetic-existing-secret".to_owned(),
            )]))
            .run(CredentialPolicy::Refuse, None)
            .await;
        assert_eq!(report.state, InstallState::Installed, "{report:?}");
        record(
            &self.name,
            ExtensionProvenance {
                install_id: Some(report.install_id),
                registry_id: "synthetic-l7-registry".to_owned(),
                install_dir: Some(self.install_dir().display().to_string()),
                source_url: Some("https://example.invalid/fixture.brxt".to_owned()),
                bundle_sha256: Some("synthetic-v1-digest".to_owned()),
                recorded_at: Some("2026-11-18T00:00:00Z".to_owned()),
            },
        )
        .unwrap();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TreeEntry {
    File(Vec<u8>),
    Symlink(PathBuf),
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, TreeEntry>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_symlink() {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    TreeEntry::Symlink(std::fs::read_link(&path).unwrap()),
                );
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    TreeEntry::File(std::fs::read(path).unwrap()),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn claims_on_disk() -> Vec<InstallClaim> {
    let Ok(entries) = std::fs::read_dir(claims_dir()) else {
        return Vec::new();
    };
    entries
        .map(|entry| {
            serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap()
        })
        .collect()
}

fn install_paths() -> BTreeSet<PathBuf> {
    std::fs::read_dir(extensions_root())
        .into_iter()
        .flatten()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn assert_failed(report: &InstallReport, reason: &str) {
    match &report.state {
        InstallState::Failed { reason: actual } => assert!(actual.contains(reason), "{report:?}"),
        _ => panic!("expected a failed install: {report:?}"),
    }
    assert!(claims_on_disk()
        .iter()
        .all(|claim| claim.install_id != report.install_id));
}

fn assert_runtime_at_stable_path(fixture: &Fixture, version: &str) {
    let output = std::process::Command::new(fixture.install_dir().join(".venv/bin/fixture"))
        .output()
        .expect("the committed runtime's absolute interpreter must exist");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("runtime-{version}\n")
    );
}

#[tokio::test]
#[serial_test::serial]
async fn failed_upgrade_restores_working_tree_metadata_switch_and_credentials() {
    for enabled in [false, true] {
        let fixture = Fixture::new("syncfailure");
        fixture.seed_working_install(enabled).await;
        let before = tree_bytes(&fixture.install_dir());
        let paths_before = install_paths();
        let entry = get_extension_entry_by_name(&fixture.name).unwrap();
        let provenance =
            serde_json::to_value(extension_provenance_for_key(&entry.config.key())).unwrap();
        fixture.write_bundle("2.0.0", &[fixture.secret_key()], &[("fail-sync", b"fail")]);

        let report = fixture
            .transaction()
            .enabled(!enabled)
            .run(CredentialPolicy::Refuse, None)
            .await;

        assert_failed(&report, "synthetic dependency sync failure");
        assert_eq!(
            tree_bytes(&fixture.install_dir()),
            before,
            "failed upgrade changed the working package"
        );
        let current = get_extension_entry_by_name(&fixture.name).unwrap();
        assert_eq!(current.enabled, enabled);
        assert_eq!(current.config, entry.config);
        assert_eq!(
            serde_json::to_value(extension_provenance_for_key(&current.config.key())).unwrap(),
            provenance
        );
        assert_eq!(
            Config::global()
                .get_secret::<String>(&fixture.secret_key())
                .unwrap(),
            "synthetic-existing-secret"
        );
        assert_runtime_at_stable_path(&fixture, "1.0.0");
        assert_eq!(
            install_paths(),
            paths_before,
            "failed upgrade left an unowned tree"
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn first_install_partial_extraction_removes_owned_tree_and_claim() {
    let fixture = Fixture::new("partialfirst");
    let paths_before = install_paths();
    fixture.write_bundle(
        "1.0.0",
        &[],
        &[
            ("conflict", b"a file"),
            ("conflict/child", b"cannot extract"),
        ],
    );

    let report = fixture
        .transaction()
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert!(
        matches!(report.state, InstallState::Failed { .. }),
        "{report:?}"
    );
    assert!(
        !fixture.install_dir().exists(),
        "a partial first extraction must be owned before it can fail"
    );
    assert!(get_extension_entry_by_name(&fixture.name).is_none());
    assert!(claims_on_disk()
        .iter()
        .all(|claim| claim.install_id != report.install_id));
    assert_eq!(install_paths(), paths_before);
}

#[tokio::test]
#[serial_test::serial]
async fn partial_upgrade_extraction_restores_the_working_tree() {
    let fixture = Fixture::new("partialupgrade");
    fixture.seed_working_install(false).await;
    let before = tree_bytes(&fixture.install_dir());
    let paths_before = install_paths();
    fixture.write_bundle(
        "2.0.0",
        &[fixture.secret_key()],
        &[
            ("conflict", b"a file"),
            ("conflict/child", b"cannot extract"),
        ],
    );

    let report = fixture
        .transaction()
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert!(
        matches!(report.state, InstallState::Failed { .. }),
        "{report:?}"
    );
    assert_eq!(tree_bytes(&fixture.install_dir()), before);
    assert!(claims_on_disk()
        .iter()
        .all(|claim| claim.install_id != report.install_id));
    assert_runtime_at_stable_path(&fixture, "1.0.0");
    assert_eq!(install_paths(), paths_before);
}

#[tokio::test]
#[serial_test::serial]
async fn first_install_sync_failure_removes_owned_tree_and_claim() {
    let fixture = Fixture::new("failedfirst");
    let paths_before = install_paths();
    fixture.write_bundle("1.0.0", &[], &[("fail-sync", b"fail")]);

    let report = fixture
        .transaction()
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_failed(&report, "synthetic dependency sync failure");
    assert!(!fixture.install_dir().exists());
    assert!(get_extension_entry_by_name(&fixture.name).is_none());
    assert_eq!(install_paths(), paths_before);
}

#[tokio::test]
#[serial_test::serial]
async fn parked_upgrade_keeps_working_package_until_a_fresh_resume_commits() {
    let fixture = Fixture::new("parkresume");
    fixture.seed_working_install(false).await;
    let before = tree_bytes(&fixture.install_dir());
    let paths_before = install_paths();
    let entry = get_extension_entry_by_name(&fixture.name).unwrap();
    let new_key = format!("{}_NEW", fixture.secret_key());
    fixture.write_bundle("2.0.0", &[fixture.secret_key(), new_key.clone()], &[]);

    let parked = fixture
        .transaction()
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_eq!(
        parked.state,
        InstallState::NeedsCredentials {
            keys: vec![new_key.clone()]
        }
    );
    let claim = claims_on_disk()
        .into_iter()
        .find(|claim| claim.install_id == parked.install_id)
        .expect("parked upgrade has a durable resume record");
    assert_eq!(claim.phase, ClaimPhase::Parked);
    assert!(claim.existed_before);
    assert_eq!(claim.pending_keys, vec![new_key.clone()]);
    assert_eq!(
        tree_bytes(&fixture.install_dir()),
        before,
        "a credential prompt must leave the registered working version usable"
    );
    assert_eq!(
        get_extension_entry_by_name(&fixture.name).unwrap().config,
        entry.config
    );
    assert_runtime_at_stable_path(&fixture, "1.0.0");

    let source = match claim.source {
        ClaimSource::LocalFile { path } => InstallSource::LocalFile { path },
        ClaimSource::Marketplace { registry_id, url } => {
            InstallSource::Marketplace { registry_id, url }
        }
    };
    let resumed = ExtensionInstallTransaction::new(source)
        .with_install_id(claim.install_id)
        .with_values(HashMap::from([(
            new_key.clone(),
            "synthetic-new-secret".to_owned(),
        )]))
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_eq!(resumed.state, InstallState::Installed, "{resumed:?}");
    assert_eq!(resumed.install_id, parked.install_id);
    assert!(
        !resumed.enabled,
        "resume retains the operator's disabled switch"
    );
    assert!(claims_on_disk()
        .iter()
        .all(|claim| claim.install_id != resumed.install_id));
    assert_eq!(
        Config::global()
            .get_secret::<String>(&fixture.secret_key())
            .unwrap(),
        "synthetic-existing-secret"
    );
    assert_eq!(
        Config::global().get_secret::<String>(&new_key).unwrap(),
        "synthetic-new-secret"
    );
    assert_runtime_at_stable_path(&fixture, "2.0.0");
    assert_eq!(
        install_paths(),
        paths_before,
        "completed resume left a parked upgrade tree"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn successful_upgrade_keeps_absolute_runtime_paths_valid_at_the_final_directory() {
    let fixture = Fixture::new("stablepath");
    fixture.seed_working_install(true).await;
    let paths_before = install_paths();
    fixture.write_bundle("2.0.0", &[fixture.secret_key()], &[]);

    let report = fixture
        .transaction()
        .enabled(false)
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_eq!(report.state, InstallState::Installed, "{report:?}");
    assert!(report.enabled);
    assert_runtime_at_stable_path(&fixture, "2.0.0");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture.install_dir().join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["version"], "2.0.0");
    assert!(claims_on_disk()
        .iter()
        .all(|claim| claim.install_id != report.install_id));
    assert_eq!(
        install_paths(),
        paths_before,
        "successful upgrade left an unowned tree"
    );
}

async fn candidate_link_boundary(directory_link: bool) {
    let fixture = Fixture::new("linkcandidate");
    fixture.seed_working_install(false).await;
    let outside = fixture._bundle_root.path().join("outside-installed-tree");
    std::fs::create_dir(&outside).unwrap();
    let referent = outside.join("sentinel.txt");
    std::fs::write(&referent, b"synthetic outside original\n").unwrap();
    let link = fixture.install_dir().join("linked");
    std::os::unix::fs::symlink(if directory_link { &outside } else { &referent }, &link).unwrap();
    let before = tree_bytes(&fixture.install_dir());
    let member = if directory_link {
        "linked/sentinel.txt"
    } else {
        "linked"
    };
    fixture.write_bundle(
        "2.0.0",
        &[fixture.secret_key()],
        &[
            (member, b"synthetic archive replacement\n"),
            ("fail-sync", b"fail"),
        ],
    );

    let report = fixture
        .transaction()
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert!(
        matches!(report.state, InstallState::Failed { .. }),
        "{report:?}"
    );
    assert_eq!(
        std::fs::read(&referent).unwrap(),
        b"synthetic outside original\n"
    );
    assert_eq!(tree_bytes(&fixture.install_dir()), before);
    assert!(std::fs::symlink_metadata(link)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[tokio::test]
#[serial_test::serial]
async fn candidate_rust_file_link_cannot_change_an_external_referent() {
    candidate_link_boundary(false).await;
}

#[tokio::test]
#[serial_test::serial]
async fn candidate_rust_ancestor_link_cannot_change_an_external_referent() {
    candidate_link_boundary(true).await;
}

#[tokio::test]
#[serial_test::serial]
async fn candidate_rust_regular_nested_file_keeps_the_external_control_unchanged() {
    let fixture = Fixture::new("regularcandidate");
    fixture.seed_working_install(false).await;
    let outside = fixture._bundle_root.path().join("external-control.txt");
    std::fs::write(&outside, b"synthetic outside original\n").unwrap();
    fixture.write_bundle(
        "2.0.0",
        &[fixture.secret_key()],
        &[("src/nested/entry.py", b"# synthetic nested file\n")],
    );

    let report = fixture
        .transaction()
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_eq!(report.state, InstallState::Installed, "{report:?}");
    assert_eq!(
        std::fs::read(fixture.install_dir().join("src/nested/entry.py")).unwrap(),
        b"# synthetic nested file\n"
    );
    assert_eq!(
        std::fs::read(outside).unwrap(),
        b"synthetic outside original\n"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn candidate_uv_failure_before_values_preserves_the_existing_credential_control() {
    let fixture = Fixture::new("credentialuvcontrol");
    fixture.seed_working_install(false).await;
    fixture.write_bundle("2.0.0", &[fixture.secret_key()], &[("fail-sync", b"fail")]);

    let report = fixture
        .transaction()
        .with_values(HashMap::from([(
            fixture.secret_key(),
            "synthetic-replacement-secret".to_owned(),
        )]))
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_failed(&report, "synthetic dependency sync failure");
    assert!(
        Config::global()
            .get_secret::<String>(&fixture.secret_key())
            .is_ok_and(|value| value == "synthetic-existing-secret"),
        "a pre-existing credential changed before supplied values were applied"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn candidate_parked_upgrade_restores_a_replaced_existing_credential() {
    let fixture = Fixture::new("credentialparkcandidate");
    fixture.seed_working_install(false).await;
    let pending_key = format!("{}_NEW", fixture.secret_key());
    fixture.write_bundle("2.0.0", &[fixture.secret_key(), pending_key.clone()], &[]);

    let report = fixture
        .transaction()
        .with_values(HashMap::from([(
            fixture.secret_key(),
            "synthetic-replacement-secret".to_owned(),
        )]))
        .run(CredentialPolicy::Refuse, None)
        .await;

    assert_eq!(
        report.state,
        InstallState::NeedsCredentials {
            keys: vec![pending_key]
        }
    );
    assert!(
        Config::global()
            .get_secret::<String>(&fixture.secret_key())
            .is_ok_and(|value| value == "synthetic-existing-secret"),
        "parking an upgrade revoked or changed a pre-existing credential"
    );
}

struct CandidateSyncChannel {
    root: PathBuf,
    ready: std::fs::File,
    release: std::fs::File,
}

impl CandidateSyncChannel {
    fn new(root: PathBuf) -> Self {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;

        std::fs::create_dir(&root).unwrap();
        let mut files = Vec::new();
        for name in ["ready", "release"] {
            let path = root.join(name);
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            files.push(
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)
                    .unwrap(),
            );
        }
        Self {
            root,
            ready: files.remove(0),
            release: files.remove(0),
        }
    }

    fn wait_ready(&mut self) {
        use std::io::Read;
        use std::os::fd::AsRawFd;

        let mut descriptor = libc::pollfd {
            fd: self.ready.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(
            unsafe { libc::poll(&mut descriptor, 1, 5_000) },
            1,
            "fake uv did not reach its controlled gate"
        );
        let mut bytes = [0; 32];
        let read = self.ready.read(&mut bytes).unwrap();
        assert_eq!(&bytes[..read], b"ready\n");
    }

    fn release_failure(&mut self) {
        self.release.write_all(b"fail\n").unwrap();
    }
}

impl Drop for CandidateSyncChannel {
    fn drop(&mut self) {
        let _ = self.release.write_all(b"fail\n");
    }
}

#[test]
#[serial_test::serial]
fn candidate_overlapping_failed_upgrade_preserves_another_committed_upgrade() {
    let first = Fixture::new("overlapcandidate");
    let runtime = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    };
    runtime().block_on(first.seed_working_install(false));
    let mut channel = CandidateSyncChannel::new(first._bundle_root.path().join("sync-channel"));
    let channel_path = channel.root.to_string_lossy().into_owned();
    first.write_bundle(
        "2.0.0",
        &[first.secret_key()],
        &[("sync-channel", channel_path.as_bytes())],
    );
    let first_transaction = first.transaction().enabled(false);
    let first_running = std::thread::spawn(move || {
        runtime().block_on(first_transaction.run(CredentialPolicy::Refuse, None))
    });
    channel.wait_ready();

    let mut second = Fixture::new("secondoverlapcandidate");
    second.name = first.name.clone();
    second.write_bundle("3.0.0", &[first.secret_key()], &[]);
    let second_transaction = second.transaction().enabled(false);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let second_running = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let report = runtime().block_on(second_transaction.run(CredentialPolicy::Refuse, None));
        let _ = finished_tx.send(report.state.is_success());
        report
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    // A queued implementation needs the first gate released. An unguarded
    // completed second transaction is observed before that release.
    let committed_before_release = finished_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap_or(false);
    channel.release_failure();
    let first_report = first_running.join().unwrap();
    assert_failed(&first_report, "synthetic controlled sync failure");
    let second_report = second_running.join().unwrap();
    if !second_report.state.is_success() {
        let retry = runtime().block_on(
            second
                .transaction()
                .enabled(false)
                .run(CredentialPolicy::Refuse, None),
        );
        assert!(
            retry.state.is_success(),
            "a fresh retry must commit after a busy attempt: {retry:?}"
        );
    }

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(first.install_dir().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest["version"], "3.0.0",
        "second transaction finished before first release: {committed_before_release}"
    );
    assert_runtime_at_stable_path(&first, "3.0.0");
    let entry = get_extension_entry_by_name(&first.name).unwrap();
    assert!(!entry.enabled);
    match entry.config {
        biorouter::agents::ExtensionConfig::Stdio { description, .. } => {
            assert_eq!(description, "version 3.0.0")
        }
        _ => panic!("committed package must retain its stdio configuration"),
    }
}

mod collected_door_residual {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct EmptySyntheticOverride {
        key: String,
    }

    impl EmptySyntheticOverride {
        fn new(key: &str) -> Self {
            assert!(key.starts_with("L7COLLECTEDPROMPT"));
            assert!(
                std::env::var_os(key).is_none(),
                "the unique synthetic key must not inherit an environment value"
            );
            std::env::set_var(key, "");
            Self { key: key.into() }
        }
    }

    impl Drop for EmptySyntheticOverride {
        fn drop(&mut self) {
            std::env::remove_var(&self.key);
        }
    }

    fn saved_original_is_present(key: &str) -> bool {
        let values = Config::global()
            .all_secrets()
            .expect("read isolated synthetic credential storage");
        values.get(key).and_then(serde_json::Value::as_str) == Some("synthetic-existing-secret")
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn collected_prompt_restores_a_saved_value_hidden_by_empty_env() {
        let fixture = Fixture::new("collectedprompt");
        fixture.seed_working_install(false).await;
        let key = fixture.secret_key();
        let pending_key = format!("{key}_NEW");
        let original_tree = tree_bytes(&fixture.install_dir());
        let original_entry = get_extension_entry_by_name(&fixture.name).unwrap();
        assert!(saved_original_is_present(&key));

        // Environment lookup can hide a usable saved value that collection overwrites.
        let _empty_override = EmptySyntheticOverride::new(&key);
        assert!(!biorouter::extension_install::brxt::secret_already_stored(
            &key
        ));
        assert!(saved_original_is_present(&key));
        fixture.write_bundle("2.0.0", &[key.clone(), pending_key.clone()], &[]);

        let observations = Arc::new(Mutex::new(Vec::new()));
        let captured_observations = Arc::clone(&observations);
        let expected_unmet = BTreeSet::from([key.clone(), pending_key.clone()]);
        let replacement_key = key.clone();
        let prompt = CredentialPolicy::Prompt(Box::new(move |unmet| {
            let requested: BTreeSet<_> = unmet.iter().map(|var| var.key.clone()).collect();
            let stored = Config::global().all_secrets()?;
            let original_present = stored
                .get(&replacement_key)
                .and_then(serde_json::Value::as_str)
                == Some("synthetic-existing-secret");
            captured_observations
                .lock()
                .map_err(|_| anyhow::anyhow!("the synthetic prompt observation lock was poisoned"))?
                .push((requested, original_present));
            Ok(HashMap::from([(
                replacement_key.clone(),
                "synthetic-collected-replacement".to_owned(),
            )]))
        }));

        let report = fixture.transaction().run(prompt, None).await;

        let observations = observations.lock().unwrap();
        assert_eq!(observations.len(), 1);
        assert!(observations[0].0 == expected_unmet);
        assert!(observations[0].1);
        assert_eq!(
            report.state,
            InstallState::NeedsCredentials {
                keys: vec![pending_key]
            }
        );
        assert!(claims_on_disk().iter().any(
            |claim| claim.install_id == report.install_id && claim.phase == ClaimPhase::Parked
        ));
        assert_eq!(tree_bytes(&fixture.install_dir()), original_tree);
        let current_entry = get_extension_entry_by_name(&fixture.name).unwrap();
        assert_eq!(current_entry.config, original_entry.config);
        assert_eq!(current_entry.enabled, original_entry.enabled);
        assert!(
            saved_original_is_present(&key),
            "parking must retain the original saved credential despite an empty environment override"
        );
    }
}
