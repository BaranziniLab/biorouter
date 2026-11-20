use super::{RECORD_FILE, create, list, load};
use crate::sandbox::sandbox_record::SandboxRecord;
use crate::storage::private_file::write_atomic;
use crate::storage::private_file_parent::ensure_private;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-desktop-sandbox-store-{label}-{}",
            crate::refs::new_snapshot_id()
        ));
        Self(path)
    }

    fn root(&self) -> PathBuf {
        self.0.join("sandboxes")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn record(name: &str, provider: &str, created_at: u64) -> SandboxRecord {
    SandboxRecord {
        name: name.to_string(),
        provider: provider.to_string(),
        created_at,
    }
}

fn write_record(root: &Path, name: &str, bytes: &[u8]) {
    ensure_private(&root.join(name)).expect("private sandbox directory");
    write_atomic(&root.join(name).join(RECORD_FILE), bytes).expect("private record");
}

#[test]
fn duplicate_create_fails_and_keeps_the_first_record() {
    let scratch = Scratch::new("duplicate");
    let root = scratch.root();
    create(&root, &record("dev-1", "macos", 1)).unwrap();

    let error = create(&root, &record("dev-1", "macos", 2)).unwrap_err();

    assert_eq!(error.code(), "INVALID_ARGS");
    let kept = load(&root, "dev-1").unwrap();
    assert_eq!(kept.provider, "macos");
    assert_eq!(kept.created_at, 1);
}

#[test]
fn concurrent_creates_of_one_name_admit_exactly_one_and_refuse_the_rest_as_duplicates() {
    let scratch = Scratch::new("concurrent");
    let root: Arc<Path> = Arc::from(scratch.root().as_path());
    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();
    for index in 0..threads {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            create(&root, &record("race", "macos", index as u64))
                .map(|_| ())
                .map_err(|error| error.code().to_string())
        }));
    }
    let results: Vec<Result<(), String>> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for result in &results {
        if let Err(code) = result {
            assert_eq!(
                code.as_str(),
                "INVALID_ARGS",
                "a losing create reports a duplicate"
            );
        }
    }
    assert_eq!(load(&root, "race").unwrap().name, "race");
}

#[test]
fn malformed_record_is_kept_beside_a_healthy_one_and_non_sandboxes_are_skipped() {
    let scratch = Scratch::new("list-malformed");
    let root = scratch.root();
    create(&root, &record("healthy", "macos", 1)).expect("healthy record");
    write_record(&root, "broken", b"{not json");
    std::fs::create_dir_all(root.join("Not-A-Sandbox")).expect("invalid name directory");

    let entries = list(&root).expect("list");

    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(
        names,
        ["broken", "healthy"],
        "invalid names and files are skipped"
    );
    let broken = entries[0].record.as_ref().expect_err("malformed record");
    assert_eq!(broken.code(), "INVALID_ARGS");
    assert!(
        broken.suggestion().is_some_and(
            |suggestion| suggestion.contains(&root.join("broken").display().to_string())
        ),
        "the error names the directory to delete"
    );
    assert_eq!(
        entries[1].record.as_ref().expect("healthy record").provider,
        "macos"
    );
}

#[test]
fn empty_and_missing_records_are_kept_as_broken_directories() {
    let scratch = Scratch::new("list-empty-missing");
    let root = scratch.root();
    write_record(&root, "empty", b"");
    ensure_private(&root.join("missing")).expect("empty sandbox directory");

    let entries = list(&root).expect("list");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "empty");
    assert_eq!(
        entries[0].record.as_ref().expect_err("empty").code(),
        "INVALID_ARGS"
    );
    assert_eq!(entries[1].name, "missing");
    assert_eq!(
        entries[1].record.as_ref().expect_err("missing").code(),
        "INVALID_ARGS"
    );
}

#[test]
fn a_root_that_is_not_a_directory_is_an_error_not_an_empty_list() {
    let scratch = Scratch::new("root-file");
    std::fs::create_dir_all(&scratch.0).expect("scratch directory");
    std::fs::write(scratch.root(), b"not a directory").expect("root file");

    assert!(list(&scratch.root()).is_err());
}

#[test]
fn subprocess_crash_before_record_publish() {
    let Some(root) = std::env::var_os("AGENT_DESKTOP_CRASH_SANDBOX_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let _ = create(&root, &record("crash", "macos", 1));
    panic!("crash failpoint did not abort");
}

#[test]
fn a_crash_before_publish_leaves_no_record_and_the_next_create_publishes() {
    let scratch = Scratch::new("crash");
    let root = scratch.root();
    let destination = root.join("crash").join(RECORD_FILE);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("sandbox::sandbox_store::tests::subprocess_crash_before_record_publish")
        .arg("--nocapture")
        .env("AGENT_DESKTOP_CRASH_SANDBOX_ROOT", &root)
        .env("AGENT_DESKTOP_TEST_CRASH_BEFORE_RENAME", &destination)
        .status()
        .unwrap();
    assert!(!status.success(), "the failpoint aborts the writer");
    assert_eq!(
        load(&root, "crash")
            .expect_err("nothing was published")
            .code(),
        "INVALID_ARGS"
    );

    create(&root, &record("crash", "macos", 1)).expect("a crashed publish is retryable");

    assert_eq!(load(&root, "crash").unwrap().created_at, 1);
}

#[test]
fn names_that_escape_the_root_are_refused() {
    let scratch = Scratch::new("traversal");
    let error = load(&scratch.root(), "../outside").unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGS");
}

#[test]
fn a_record_that_names_another_sandbox_is_refused_as_malformed() {
    let scratch = Scratch::new("renamed");
    let root = scratch.root();
    create(&root, &record("victim", "macos", 1)).unwrap();
    let foreign = serde_json::to_vec(&record("victim", "macos", 2)).unwrap();
    write_record(&root, "dev-1", &foreign);

    assert_eq!(load(&root, "dev-1").unwrap_err().code(), "INVALID_ARGS");
    let listed = list(&root).unwrap();
    let entry = listed.iter().find(|entry| entry.name == "dev-1").unwrap();
    assert!(entry.record.is_err(), "list never reports the other name");
}

#[test]
fn removing_a_directory_the_provider_already_removed_succeeds() {
    let scratch = Scratch::new("removed");
    let root = scratch.root();
    create(&root, &record("dev-1", "macos", 1)).unwrap();
    std::fs::remove_dir_all(root.join("dev-1")).unwrap();

    super::remove(&root, "dev-1").expect("nothing is left to remove");
}
