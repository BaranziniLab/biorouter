//! `biorouter dictionary` through the real binary (PR-13, `testing-and-release.md`,
//! "Dictionary store and bias set"). Offline. Every run here has no terminal, the way a
//! pipe, cron or an agent's shell runs it; the interactive paths (typed confirmations,
//! twins, review, settings) are driven with a scripted prompter by the unit tests in
//! `src/commands/dictionary.rs`.
//!
//! * every write except `import` is refused without a terminal, exit 2, with its
//!   sentence alone; every write, `import` included, is refused in an agent's shell
//!   (`BIOROUTER_TERMINAL=1`) with MSG-DICTIONARY-AGENT;
//! * a piped `import -` merges, and a second process lists the term at once;
//! * a piped `list` withholds private rows and prints no count of them, and a piped
//!   `export` carries public rows only, while `export --include-private` is refused;
//! * processes importing at once lose nothing, under the store's lock;
//! * a store this version cannot read refuses `import` with MSG-DICTIONARY-FILE and
//!   stays byte identical, and `list` prints nothing;
//! * managed `dictionary: false` refuses `import` with the policy's sentence and still
//!   exports.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const MSG_DICTIONARY_TTY: &str = "Changing the dictionary needs you at an interactive terminal. To add terms from a file or a pipe, use `biorouter dictionary import`.";
const MSG_DICTIONARY_AGENT: &str =
    "Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms.";
const MSG_EXPORT_PRIVATE_REFUSED: &str = "Exporting private terms needs you at your own terminal and a private model configured on this computer. Run the command without --include-private to export public terms.";

fn biorouter() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_biorouter"))
}

/// A fresh Biorouter root and HOME.
fn sandbox() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    root
}

fn store_file(root: &Path) -> PathBuf {
    root.join("config")
        .join("dictionary")
        .join("dictionary.json")
}

fn command(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(biorouter());
    command
        .arg("dictionary")
        .args(args)
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env_remove("BIOROUTER_TERMINAL")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    command
}

/// Runs `biorouter dictionary <args>` under `root` with `input` piped to stdin.
fn run(root: &Path, args: &[&str], input: &str, env: &[(&str, &str)]) -> Output {
    let mut child = command(root, args, env)
        .stdin(Stdio::piped())
        .spawn()
        .expect("run biorouter");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().expect("wait for biorouter")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn listed(root: &Path) -> Vec<Value> {
    let output = run(root, &["list", "--format", "json"], "", &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_slice::<Value>(&output.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", stdout(&output)))
        .as_array()
        .unwrap()
        .clone()
}

fn assert_refused(output: &Output, sentence: &str) {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}{}",
        stdout(output),
        stderr(output)
    );
    assert_eq!(
        stderr(output).trim(),
        sentence,
        "stdout: {}",
        stdout(output)
    );
}

const WRITES: &[&[&str]] = &[
    &["add", "EPAS1"],
    &["add", "--private", "PHD2"],
    &["edit", "EPAS1", "--term", "EPAS2"],
    &["remove", "EPAS1"],
    &["review"],
    &["accept", "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60"],
    &["dismiss", "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60"],
    &["publicize", "PHD2"],
    &["privatize", "EPAS1"],
    &["settings", "--agent-writes", "add"],
    &["settings", "--learn-from-chats", "on"],
    &["clear"],
];

#[test]
fn every_write_but_import_needs_a_terminal() {
    let root = sandbox();
    for args in WRITES {
        assert_refused(&run(root.path(), args, "", &[]), MSG_DICTIONARY_TTY);
    }
    assert!(!store_file(root.path()).exists(), "nothing was written");
}

#[test]
fn every_write_is_refused_in_an_agent_shell() {
    let root = sandbox();
    const IMPORT: &[&str] = &["import", "-", "--format", "txt"];
    let agent = [("BIOROUTER_TERMINAL", "1")];
    for args in WRITES.iter().copied().chain([IMPORT]) {
        assert_refused(
            &run(root.path(), args, "EPAS1\n", &agent),
            MSG_DICTIONARY_AGENT,
        );
    }
    assert!(!store_file(root.path()).exists(), "nothing was written");
    // Reads still work there, public rows only.
    let output = run(root.path(), &["list"], "", &agent);
    assert!(output.status.success(), "{}", stderr(&output));
}

/// DD-43's sync path: `import -` from a pipe merges, and a second process lists the
/// terms at once, because readers cache by the file's time, length and identity.
#[test]
fn a_piped_import_merges_and_another_process_lists_it_at_once() {
    let root = sandbox();
    let first = run(
        root.path(),
        &["import", "-", "--format", "txt"],
        "# terms\nEPAS1 = epas one\n",
        &[],
    );
    assert!(first.status.success(), "{}", stderr(&first));
    assert_eq!(
        stdout(&first),
        "Imported: 1 new, 0 merged, 0 duplicate, 0 invalid.\n"
    );
    assert!(
        stderr(&first).contains(
            &root
                .path()
                .join("config")
                .join("dictionary")
                .display()
                .to_string()
        ),
        "the folder it acts on: {}",
        stderr(&first)
    );
    let rows = listed(root.path());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["written"], "EPAS1");
    assert_eq!(rows[0]["source"], "import");

    let second = run(
        root.path(),
        &["import", "-", "--format", "txt"],
        "EPAS1 = e pass one\nUCSF\nhttps://example.org\n",
        &[],
    );
    assert_eq!(
        stdout(&second),
        "Imported: 1 new, 1 merged, 0 duplicate, 1 invalid.\nline 3: url: A dictionary term cannot be a web address, an email address or a file path.\n"
    );
    let rows = listed(root.path());
    assert_eq!(rows.len(), 2, "an import never deletes");
    let epas = rows.iter().find(|r| r["written"] == "EPAS1").unwrap();
    assert_eq!(
        epas["spoken_forms"],
        serde_json::json!(["epas one", "e pass one"])
    );

    // A file names its format by its extension.
    let file = root.path().join("terms.csv");
    std::fs::write(
        &file,
        "written,spoken_forms,note\nHIF1A,hif one a,\"a gene, human\"\n",
    )
    .unwrap();
    let csv = run(root.path(), &["import", file.to_str().unwrap()], "", &[]);
    assert_eq!(
        stdout(&csv),
        "Imported: 1 new, 0 merged, 0 duplicate, 0 invalid.\n"
    );
    let dry = run(
        root.path(),
        &["import", "-", "--format", "txt", "--dry-run"],
        "VHL\n",
        &[],
    );
    assert_eq!(
        stdout(&dry),
        "Dry run: 1 new, 0 merged, 0 duplicate, 0 invalid.\n"
    );
    assert_eq!(listed(root.path()).len(), 3, "a dry run changes nothing");
}

/// Without a terminal private rows are withheld from `list` and `export`, with no count
/// of them, and private terms cannot be exported.
#[test]
fn a_piped_list_withholds_private_rows() {
    let root = sandbox();
    let export = r#"{"format": "biorouter.dictionary", "version": 1, "exported_at": "2026-11-04T18:00:00Z",
      "entries": [
        {"written": "EPAS1", "spoken_forms": [], "misrecognitions": [], "kind": "gene_symbol", "match": "exact", "note": null},
        {"written": "PHD2", "spoken_forms": [], "misrecognitions": [], "kind": null, "match": "exact", "note": null, "tier": "private", "affiliation": {"institutions": ["ucsf"]}}
      ]}"#;
    let imported = run(root.path(), &["import", "-"], export, &[]);
    assert_eq!(
        stdout(&imported),
        "Imported: 1 new, 0 merged, 0 duplicate, 0 invalid.\nPrivate terms received: 1.\n"
    );
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(store_file(root.path())).unwrap()).unwrap();
    let phd2 = on_disk["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["written"] == "PHD2")
        .unwrap();
    assert_eq!(phd2["tier"], "private");
    assert_eq!(
        phd2["affiliation"],
        serde_json::json!({"institutions": ["ucsf"]})
    );

    let rows = listed(root.path());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["written"], "EPAS1");
    assert!(
        rows[0].get("uses").is_none(),
        "usage needs private standing"
    );
    for format in ["text", "csv"] {
        let output = run(root.path(), &["list", "--format", format], "", &[]);
        let text = stdout(&output) + &stderr(&output);
        assert!(!text.contains("PHD2"), "{text}");
        assert_eq!(
            stdout(&output).lines().count(),
            2,
            "{format}: {}",
            stdout(&output)
        );
    }
    let exported = run(root.path(), &["export"], "", &[]);
    assert!(exported.status.success());
    assert!(stdout(&exported).contains("EPAS1") && !stdout(&exported).contains("PHD2"));
    assert_refused(
        &run(root.path(), &["export", "--include-private"], "", &[]),
        MSG_EXPORT_PRIVATE_REFUSED,
    );
}

/// Writers in separate processes take the store's lock and read it fresh, so none
/// overwrites another's terms.
#[test]
fn processes_importing_at_once_lose_nothing() {
    let root = sandbox();
    let children: Vec<_> = (0..8)
        .map(|p| {
            let mut child = command(root.path(), &["import", "-", "--format", "txt"], &[])
                .stdin(Stdio::piped())
                .spawn()
                .expect("run biorouter");
            let terms: String = (0..5).map(|t| format!("TERM{p}X{t}\n")).collect();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(terms.as_bytes())
                .unwrap();
            child
        })
        .collect();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
    }
    assert_eq!(listed(root.path()).len(), 40);
}

#[test]
fn a_corrupt_store_refuses_import_and_is_left_byte_identical() {
    let root = sandbox();
    let file = store_file(root.path());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        b"{\"format\": \"biorouter.dictionary.store\", \"version\": 7}",
    )
    .unwrap();
    let before = std::fs::read(&file).unwrap();
    let message = format!(
        "The dictionary file at {} could not be read, so nothing was changed. Fix or remove it, then try again.",
        file.display()
    );
    let output = run(
        root.path(),
        &["import", "-", "--format", "txt"],
        "UCSF\n",
        &[],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains(&message), "{}", stderr(&output));
    let list = run(root.path(), &["list"], "", &[]);
    assert_eq!(list.status.code(), Some(1));
    assert!(stdout(&list).is_empty());
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

/// A managed policy in the sandbox: owned by this account and not group writable, as
/// `managed::trust` requires.
fn write_managed(root: &Path, block: &str) {
    let dir = root.join("managed");
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let file = dir.join("managed-policy.yaml");
    std::fs::write(&file, format!("dictation:\n{block}")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}

#[test]
fn managed_dictionary_off_refuses_import_and_still_exports() {
    let root = sandbox();
    let seeded = run(
        root.path(),
        &["import", "-", "--format", "txt"],
        "EPAS1\n",
        &[],
    );
    assert!(seeded.status.success());
    write_managed(root.path(), "  dictionary: false\n");
    assert_refused(
        &run(
            root.path(),
            &["import", "-", "--format", "txt"],
            "UCSF\n",
            &[],
        ),
        "Dictation is turned off on this computer by your organization's managed policy.",
    );
    let exported = run(root.path(), &["export"], "", &[]);
    assert!(exported.status.success(), "{}", stderr(&exported));
    assert!(stdout(&exported).contains("EPAS1"));
}

#[test]
fn settings_print_without_a_terminal() {
    let root = sandbox();
    let output = run(root.path(), &["settings"], "", &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "agent_writes: on_request\nlearn_from_chats: off\n"
    );
}
