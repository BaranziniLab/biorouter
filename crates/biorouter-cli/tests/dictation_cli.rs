//! `biorouter dictation`'s model commands, through the real binary (PR-11,
//! `testing-and-release.md`, "Models and installer"). Offline: nothing here downloads.
//!
//! * `install` refuses in an agent's shell (`BIOROUTER_TERMINAL=1`) even with `--yes`,
//!   needs `--yes` without a terminal, and `--system` needs root or Administrator; all
//!   three exit 2 with their sentence alone;
//! * `install --from` identifies a renamed file by its SHA-256, installs it with its
//!   license and `installed.json`, and lists the file it did not recognize;
//! * with managed `model_downloads: false`, `install` refuses with the policy's own
//!   sentence (the same text as `status.policy.install.reason`), and a verified file the
//!   administrator placed in `models_dir` is found, with `location: managed`;
//! * `urls` prints each file's URL, size and SHA-256;
//! * `verify` hashes again and deletes a failing copy in this account's folder;
//! * `remove` is never refused by policy; the voice detector stays while another speech
//!   model in the folder needs it or is being installed, and goes with the last one;
//!   `remove` given the detector's id refuses while a speech model needs it;
//! * with downloads off, `install --system --from` into the managed folder is the one
//!   install admitted (as root; elsewhere it stops at the root check);
//! * `use` refuses an unknown id, an uninstalled model, and a `.en` model while the
//!   language is not English (MSG-LANGUAGE-EN);
//! * `list --format json` rates every catalog model.
//!
//! The vendored detector fixture is byte identical to the production detector, so it
//! stands in for a real catalog file. A speech model is 264 MB or more, so the cases
//! that need one plant a file whose bytes are wrong: `remove` deletes by path, and
//! everything that reads a model verifies it first.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before
// `main`, as every test binary here does.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const VAD_ID: &str = "silero-vad-v6.2.0";
const VAD_FILE: &str = "ggml-silero-v6.2.0.bin";
const SMALL_EN: &str = "whisper-small.en-q8_0";
const TURBO: &str = "whisper-large-v3-turbo-q5_0";

fn biorouter() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_biorouter"))
}

fn detector_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source/models/for-tests-silero-v6.2.0-ggml.bin")
}

/// A fresh Biorouter root and HOME.
fn sandbox() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    root
}

fn models_dir(root: &Path) -> PathBuf {
    root.join("data").join("dictation").join("models")
}

/// Runs `biorouter dictation <args>` under `root`, with stdin closed (no terminal) and
/// no dictation variables from the caller's environment.
fn run(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(biorouter());
    command
        .arg("dictation")
        .args(args)
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env_remove("BIOROUTER_TERMINAL")
        .env_remove("BIOROUTER_DICTATION_MODELS_DIR")
        .env_remove("BIOROUTER_DICTATION_MODEL_MIRROR")
        .env_remove("BIOROUTER_DICTATION_WORKER")
        .stdin(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("run biorouter")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", stdout(output)))
}

/// Whether this build and processor can install at all; the refusal tests below hold
/// either way, the install tests need it.
fn can_install() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

/// The refusal's sentence alone on stderr, exit 2.
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

const MSG_INSTALL_SHELL: &str = "Install the speech model yourself: run this in a terminal, or use Settings > Models > Speech models.";

#[test]
fn install_refuses_in_an_agent_shell_even_with_yes() {
    let root = sandbox();
    let agent = [("BIOROUTER_TERMINAL", "1")];
    for args in [
        vec!["install", "--yes", SMALL_EN],
        vec!["install", "--yes"],
        vec![
            "install",
            "--yes",
            "--from",
            detector_fixture().to_str().unwrap(),
        ],
        vec!["pull", "--yes", SMALL_EN],
    ] {
        assert_refused(&run(root.path(), &args, &agent), MSG_INSTALL_SHELL);
    }
    let change = "Change speech models yourself: run this in a terminal, or use Settings > Models > Speech models.";
    assert_refused(
        &run(root.path(), &["remove", "--yes", SMALL_EN], &agent),
        change,
    );
    assert_refused(&run(root.path(), &["use", SMALL_EN], &agent), change);
    assert!(!models_dir(root.path()).exists(), "nothing may be written");
}

#[test]
fn install_needs_yes_without_a_terminal() {
    let root = sandbox();
    let output = run(root.path(), &["install", SMALL_EN], &[]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    if can_install() {
        assert_eq!(
            stderr(&output).trim(),
            format!("Installing a speech model needs your confirmation at a terminal. To install without one, run: biorouter dictation install --yes {SMALL_EN}")
        );
        // It printed what it would install, and where, before refusing.
        let out = stdout(&output);
        assert!(out.contains("264,477,561 B  sha256 67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067"), "{out}");
        assert!(
            out.contains("github.com/BaranziniLab/biorouter-models"),
            "{out}"
        );
        assert!(
            out.contains(&models_dir(root.path()).display().to_string()),
            "{out}"
        );
    }
    let output = run(
        root.path(),
        &["install", "--from", detector_fixture().to_str().unwrap()],
        &[],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(!models_dir(root.path()).join(VAD_ID).exists());
}

#[cfg(unix)]
#[test]
fn system_installs_need_root() {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 || !can_install() {
        return;
    }
    let root = sandbox();
    let output = run(
        root.path(),
        &["install", "--system", "--yes", SMALL_EN],
        &[],
    );
    assert_refused(&output, "Installing into the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation install --system`, or leave out --system to install for your account.");
    let output = run(root.path(), &["remove", "--system", "--yes", SMALL_EN], &[]);
    assert_eq!(output.status.code(), Some(2));
}

/// Installs the detector fixture under `root` with `--from`, from a copy named `name`.
fn install_detector_from(root: &Path, name: &str) -> Output {
    let carried = root.join("carried");
    std::fs::create_dir_all(&carried).unwrap();
    std::fs::copy(detector_fixture(), carried.join(name)).unwrap();
    std::fs::write(carried.join("notes.txt"), b"not a model").unwrap();
    run(
        root,
        &["install", "--yes", "--from", carried.to_str().unwrap()],
        &[],
    )
}

#[test]
fn from_installs_a_renamed_file_by_hash_and_reports_the_rest() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let output = install_detector_from(root.path(), "downloaded-detector.data");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let out = stdout(&output);
    let installed = models_dir(root.path()).join(VAD_ID).join(VAD_FILE);
    assert!(
        out.contains(&format!("Installed {VAD_ID} ({})", installed.display())),
        "{out}"
    );
    assert!(
        out.contains("notes.txt is not a Biorouter model file"),
        "{out}"
    );
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        std::fs::read(detector_fixture()).unwrap()
    );
    assert!(models_dir(root.path())
        .join(VAD_ID)
        .join("LICENSE-silero-vad.txt")
        .is_file());
    let record: Value = serde_json::from_slice(
        &std::fs::read(models_dir(root.path()).join(VAD_ID).join("installed.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["source"], "import");
    assert_eq!(record["release_tag"], "stt-silero-vad-v6.2.0-r1");

    let status = json(&run(root.path(), &["status", "--format", "json"], &[]));
    let models = status["models"].as_array().unwrap();
    assert_eq!(models.len(), 1, "{status}");
    assert_eq!(models[0]["id"], VAD_ID);
    assert_eq!(models[0]["location"], "user");
    assert_eq!(models[0]["verified"], true);
    // A detector alone is not a dictation model.
    assert_eq!(status["active_model"], Value::Null);

    // The same file again is already there; a folder with nothing known fails.
    let again = install_detector_from(root.path(), "again.bin");
    assert_eq!(again.status.code(), Some(0));
    assert!(stdout(&again).contains(&format!("{VAD_ID} is already installed")));
    let empty = root.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("x.bin"), b"lmgg").unwrap();
    let none = run(
        root.path(),
        &["install", "--yes", "--from", empty.to_str().unwrap()],
        &[],
    );
    assert_eq!(none.status.code(), Some(1));
    assert!(
        stderr(&none).contains("matches a Biorouter model"),
        "{}",
        stderr(&none)
    );
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
fn downloads_off_refuses_and_the_managed_folder_is_used() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let admin = root.path().join("admin-models");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::copy(detector_fixture(), admin.join(VAD_FILE)).unwrap();
    write_managed(
        root.path(),
        &format!(
            "  model_downloads: false\n  models_dir: {}\n",
            admin.display()
        ),
    );
    let status = json(&run(root.path(), &["status", "--format", "json"], &[]));
    let reason = status["policy"]["install"]["reason"]
        .as_str()
        .expect("downloads off has a reason")
        .to_string();
    assert!(reason.contains(&admin.display().to_string()), "{reason}");
    assert_eq!(status["install"]["reason"], reason.as_str());
    assert_eq!(status["models"][0]["id"], VAD_ID);
    assert_eq!(status["models"][0]["location"], "managed");
    assert_eq!(
        status["models"][0]["path"],
        admin.join(VAD_FILE).display().to_string()
    );

    let output = run(root.path(), &["install", "--yes", SMALL_EN], &[]);
    assert_refused(&output, &reason);
    // `--from` into this account's folder would never be read while the managed folder
    // is the only source, so it is refused the same way.
    let output = install_detector_from(root.path(), "d.bin");
    assert_refused(&output, &reason);
}

/// The one install downloads off admits: `--system --from` when the computer's shared
/// folder is the managed `models_dir`. Under `BIOROUTER_PATH_ROOT` the shared folder is
/// `<root>/system/dictation/models`. As root it installs and `status` finds the detector
/// at `location: managed`; as anyone else it stops at the root check, which comes first.
#[test]
fn downloads_off_admits_system_from_into_the_managed_folder() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let shared = root.path().join("system").join("dictation").join("models");
    write_managed(
        root.path(),
        &format!(
            "  model_downloads: false\n  models_dir: {}\n",
            shared.display()
        ),
    );
    let carried = root.path().join("carried");
    std::fs::create_dir_all(&carried).unwrap();
    std::fs::copy(detector_fixture(), carried.join("d.bin")).unwrap();
    let output = run(
        root.path(),
        &[
            "install",
            "--system",
            "--yes",
            "--from",
            carried.join("d.bin").to_str().unwrap(),
        ],
        &[],
    );
    if !biorouter::dictation::models::running_as_administrator() {
        assert_refused(&output, "Installing into the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation install --system`, or leave out --system to install for your account.");
        return;
    }
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let status = json(&run(root.path(), &["status", "--format", "json"], &[]));
    assert_eq!(status["models"][0]["id"], VAD_ID);
    assert_eq!(status["models"][0]["location"], "managed");
}

#[test]
fn urls_prints_address_size_and_sha256() {
    let root = sandbox();
    let output = run(root.path(), &["urls", SMALL_EN], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "whisper-small.en-q8_0  264,477,561 B  sha256 67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067\n  \
         https://github.com/BaranziniLab/biorouter-models/releases/download/stt-whisper-small.en-q8_0-r1/ggml-small.en-q8_0.bin\n\
         silero-vad-v6.2.0  885,098 B  sha256 2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987\n  \
         https://github.com/BaranziniLab/biorouter-models/releases/download/stt-silero-vad-v6.2.0-r1/ggml-silero-v6.2.0.bin\n"
    );
    let unknown = run(root.path(), &["urls", "whisper-tiny"], &[]);
    assert_refused(
        &unknown,
        "There is no speech model named whisper-tiny. Run `biorouter dictation list` to see the models.",
    );
}

#[test]
fn verify_hashes_again_and_deletes_a_failing_copy() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    assert_eq!(
        install_detector_from(root.path(), "d.bin").status.code(),
        Some(0)
    );
    let ok = run(root.path(), &["verify"], &[]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));
    assert!(
        stdout(&ok).starts_with(&format!("ok      {VAD_ID} (user, ")),
        "{}",
        stdout(&ok)
    );

    // Same size, same modification time, other bytes: only a full hash sees it.
    let installed = models_dir(root.path()).join(VAD_ID).join(VAD_FILE);
    let mtime = std::fs::metadata(&installed).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(&installed).unwrap();
    bytes[1000] ^= 1;
    std::fs::write(&installed, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&installed)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let status = json(&run(root.path(), &["status", "--format", "json"], &[]));
    assert_eq!(
        status["models"][0]["id"], VAD_ID,
        "the record is trusted until verify"
    );
    let failed = run(root.path(), &["verify"], &[]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        stdout(&failed).contains("does not match its pin, so it was deleted"),
        "{}",
        stdout(&failed)
    );
    assert!(!installed.exists());
}

#[test]
fn remove_is_never_refused_by_policy_and_takes_the_last_detector_with_it() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    assert_eq!(
        install_detector_from(root.path(), "d.bin").status.code(),
        Some(0)
    );
    // Two planted speech models: `remove` deletes by path; nothing would ever load them.
    let plant = |id: &str, file: &str| {
        let dir = models_dir(root.path()).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), b"lmgg not really").unwrap();
        dir
    };
    let speech = plant(SMALL_EN, "ggml-small.en-q8_0.bin");
    let turbo = plant(TURBO, "ggml-large-v3-turbo-q5_0.bin");
    write_managed(root.path(), "  enabled: false\n");
    // DD-23: the detector stays while another speech model in the folder needs it.
    let output = run(root.path(), &["rm", "--yes", SMALL_EN], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains(&format!("Removed {SMALL_EN}")), "{out}");
    assert!(!out.contains(&format!("Removed {VAD_ID}")), "{out}");
    assert!(!speech.exists());
    assert!(models_dir(root.path())
        .join(VAD_ID)
        .join(VAD_FILE)
        .is_file());
    // ... and goes with the last one.
    let output = run(root.path(), &["rm", "--yes", TURBO], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains(&format!("Removed {TURBO}")), "{out}");
    assert!(out.contains(&format!("Removed {VAD_ID}")), "{out}");
    assert!(!turbo.exists());
    assert!(!models_dir(root.path()).join(VAD_ID).exists());
    let again = run(root.path(), &["remove", "--yes", SMALL_EN], &[]);
    assert_eq!(again.status.code(), Some(1));
    assert!(
        stderr(&again).contains("is not installed"),
        "{}",
        stderr(&again)
    );
}

/// The voice detector stays while a speech model in its folder is there, or is being
/// installed by another process (whose lock is held): such an install planned its files
/// while the detector was there and does not fetch it again. Found by PR-11's live check,
/// where removing the last installed model during another model's download left that
/// model with "voice detector missing".
#[test]
fn the_detector_stays_while_a_speech_model_needs_it_or_is_being_installed() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    assert_eq!(
        install_detector_from(root.path(), "d.bin").status.code(),
        Some(0)
    );
    let vad = models_dir(root.path()).join(VAD_ID).join(VAD_FILE);
    let turbo = models_dir(root.path()).join(TURBO);
    std::fs::create_dir_all(&turbo).unwrap();
    std::fs::write(
        turbo.join("ggml-large-v3-turbo-q5_0.bin"),
        b"lmgg not really",
    )
    .unwrap();
    // The detector's own id is refused while a speech model needs it (MSG-DETECTOR-NEEDED).
    assert_refused(
        &run(root.path(), &["remove", "--yes", VAD_ID], &[]),
        "whisper-large-v3-turbo-q5_0 needs silero-vad-v6.2.0; it is removed with the last speech model that needs it.",
    );
    assert!(vad.is_file());

    // Another process is installing small.en and holds its lock.
    let small = models_dir(root.path()).join(SMALL_EN);
    std::fs::create_dir_all(&small).unwrap();
    let held = std::fs::File::create(small.join(".lock")).unwrap();
    held.lock().unwrap();
    let output = run(root.path(), &["remove", "--yes", TURBO], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains(&format!("Removed {TURBO}")), "{out}");
    assert!(!out.contains(&format!("Removed {VAD_ID}")), "{out}");
    assert!(vad.is_file());
    assert_refused(
        &run(root.path(), &["remove", "--yes", VAD_ID], &[]),
        "whisper-small.en-q8_0 needs silero-vad-v6.2.0; it is removed with the last speech model that needs it.",
    );
    held.unlock().unwrap();
    drop(held);

    // Nothing needs it now, so it may go on its own.
    let output = run(root.path(), &["remove", "--yes", VAD_ID], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stdout(&output).contains(&format!("Removed {VAD_ID}")),
        "{}",
        stdout(&output)
    );
    assert!(!vad.exists());
}

#[test]
fn use_checks_the_id_the_language_and_the_files() {
    let root = sandbox();
    assert_refused(
        &run(root.path(), &["use", "whisper-nope"], &[]),
        "There is no speech model named whisper-nope. Run `biorouter dictation list` to see the models.",
    );
    assert_refused(
        &run(root.path(), &["use", VAD_ID], &[]),
        "There is no speech model named silero-vad-v6.2.0. Run `biorouter dictation list` to see the models.",
    );
    assert_refused(
        &run(root.path(), &["use", "whisper-large-v3-turbo-q5_0"], &[]),
        "whisper-large-v3-turbo-q5_0 is not installed on this computer. Run `biorouter dictation install whisper-large-v3-turbo-q5_0`.",
    );
    let config = root.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.yaml"),
        "BIOROUTER_DICTATION_LANGUAGE: fr\n",
    )
    .unwrap();
    assert_refused(
        &run(root.path(), &["use", SMALL_EN], &[]),
        "whisper-small.en-q8_0 understands only English. Choose a multilingual model for other languages, or set the language to English.",
    );
}

#[test]
fn list_rates_every_catalog_model() {
    let root = sandbox();
    let output = run(root.path(), &["list", "--format", "json"], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let list = json(&output);
    let models = list["models"].as_array().unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["whisper-large-v3-turbo-q5_0", SMALL_EN, VAD_ID]);
    let suitabilities = [
        "recommended",
        "suitable",
        "too_slow",
        "below_minimum",
        "unsupported_cpu",
        "unknown",
    ];
    for model in models {
        assert!(
            suitabilities.contains(&model["suitability"].as_str().unwrap()),
            "{model}"
        );
        assert_eq!(model["installed"], false);
    }
    let class = biorouter::dictation::hardware::HostFacts::current().class;
    let has_default = biorouter::dictation::catalog::CATALOG
        .default_for(class)
        .is_some();
    assert_eq!(
        models.iter().filter(|m| m["recommended"] == true).count(),
        usize::from(has_default),
        "one recommended model on a supported host"
    );
    let text = run(root.path(), &["ls"], &[]);
    assert_eq!(text.status.code(), Some(0));
    assert!(stdout(&text).contains("not installed"));
}
