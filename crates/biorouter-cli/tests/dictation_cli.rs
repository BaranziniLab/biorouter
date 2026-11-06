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
//! * `list --format json` rates every catalog model;
//! * `biorouter transcribe` (PR-12): `jfk.wav` with the `for-tests` model exits 3 with
//!   `{schema: 1, error_code: "no_speech", message}`, from a file and from standard input;
//!   generated silence exits 3 before any worker is needed; both hold in an agent's shell
//!   (`BIOROUTER_TERMINAL=1`); a recording under half a second exits 3; Opus and WebM
//!   exit 1 with the `ffmpeg` line; a missing file exits 1; no model, an unknown or
//!   uninstalled model, a language the model cannot take and a managed `enabled: false`
//!   exit 2 with their sentences.
//!
//! Ignored, for the accuracy workflow and by hand: an MP3 encoded by `ffmpeg` decodes to
//! the expected length and an M4A and an Ogg Vorbis file are refused with the `ffmpeg`
//! line, and `transcribe` with the real catalog models in
//! `BIOROUTER_DICTATION_MODELS_DIR` writes the JFK sentence and reports its timings; and
//! (PR-14, DD-77) `transcribe --session <a private chat>` at a pseudo terminal with a
//! private model configured reaches the private terms, while the same run in an agent's
//! shell (`BIOROUTER_TERMINAL=1`) is public and writes none of them.
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

// ---- biorouter transcribe (PR-12) -------------------------------------------------

const JFK_TEXT: &str = "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country.";

/// Words, case folded, without punctuation: the catalog models agree on the JFK words
/// and not on its commas (`whisper-small.en-q8_0` writes "And so, my fellow Americans,
/// ... for you. Ask what ..." [M]).
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn vendored(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

fn jfk_wav() -> PathBuf {
    vendored("samples/jfk.wav")
}

/// Runs `biorouter transcribe <args>` under `root`, with `stdin` piped in when given.
fn transcribe(root: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Output {
    let mut command = Command::new(biorouter());
    command
        .arg("transcribe")
        .args(args)
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env_remove("BIOROUTER_TERMINAL")
        .env_remove("BIOROUTER_DICTATION_MODELS_DIR")
        .env_remove("BIOROUTER_DICTATION_TEST_MODEL")
        .env_remove("BIOROUTER_DICTATION_WORKER")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("run biorouter");
    if let Some(bytes) = stdin {
        use std::io::Write;
        let mut pipe = child.stdin.take().expect("stdin");
        pipe.write_all(bytes).expect("write stdin");
    }
    child.wait_with_output().expect("biorouter ends")
}

/// A 16 bit mono WAV at 16 kHz: `ms` of a 440 Hz tone, or of exact zeros.
fn wav_16k(ms: u64, tone: bool) -> Vec<u8> {
    let n = (ms * 16) as usize;
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        let x = if tone {
            (8_000.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 16_000.0).sin()) as i16
        } else {
            0
        };
        data.extend_from_slice(&x.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// The schema 1 error `--format json` prints, and its exit code.
fn assert_json_error(output: &Output, code: i32, error_code: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}{}",
        stdout(output),
        stderr(output)
    );
    let error = json(output);
    assert_eq!(error["schema"], 1, "{error}");
    assert_eq!(error["error_code"], error_code, "{error}");
    assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()));
    error
}

/// The plan's offline acceptance (DD-48): the `for-tests` model decodes `jfk.wav` to
/// nothing, so the command exits 3 with `no_speech`, from a file and from standard input.
/// Debug builds only, since a release build ignores `BIOROUTER_DICTATION_TEST_MODEL`.
#[cfg(debug_assertions)]
#[test]
fn transcribe_jfk_with_the_test_model_hears_nothing() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let model = vendored("models/for-tests-ggml-base.en.bin");
    let env = [("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap())];
    let jfk = jfk_wav();
    let output = transcribe(
        root.path(),
        &[jfk.to_str().unwrap(), "--format", "json"],
        &env,
        None,
    );
    assert_json_error(&output, 3, "no_speech");

    let bytes = std::fs::read(&jfk).unwrap();
    let output = transcribe(root.path(), &["-", "--format", "json"], &env, Some(&bytes));
    assert_json_error(&output, 3, "no_speech");

    // Text format: the sentence alone on stderr, nothing on stdout.
    let output = transcribe(root.path(), &[jfk.to_str().unwrap()], &env, None);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(stdout(&output).is_empty());
    assert_eq!(
        stderr(&output).trim(),
        "No speech was heard in that recording, so nothing was transcribed."
    );

    // The fixture is English only.
    let output = transcribe(
        root.path(),
        &[
            jfk.to_str().unwrap(),
            "--language",
            "de",
            "--format",
            "json",
        ],
        &env,
        None,
    );
    let error = assert_json_error(&output, 2, "language");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("understands only English"));
}

/// Exact zeros are no speech, decided before the engine is needed: no model is installed
/// and the worker named cannot even start, yet the answer is exit 3.
#[test]
fn transcribe_hears_nothing_in_digital_silence_without_a_worker() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let silence = root.path().join("silence.wav");
    std::fs::write(&silence, wav_16k(2_000, false)).unwrap();
    let nowhere = root.path().join("no-such-worker");
    let output = transcribe(
        root.path(),
        &[silence.to_str().unwrap(), "--format", "json"],
        &[("BIOROUTER_DICTATION_WORKER", nowhere.to_str().unwrap())],
        None,
    );
    assert_json_error(&output, 3, "no_speech");
}

/// `transcribe` works in an agent's shell (S11): `BIOROUTER_TERMINAL=1` with no terminal
/// changes nothing, where `dictate` and `install` refuse. In a debug build the `for-tests`
/// model takes the whole path through the worker there too.
#[test]
fn transcribe_works_in_an_agent_shell() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let agent = ("BIOROUTER_TERMINAL", "1");
    let silence = root.path().join("silence.wav");
    std::fs::write(&silence, wav_16k(2_000, false)).unwrap();
    let output = transcribe(
        root.path(),
        &[silence.to_str().unwrap(), "--format", "json"],
        &[agent],
        None,
    );
    assert_json_error(&output, 3, "no_speech");
    if cfg!(debug_assertions) {
        let model = vendored("models/for-tests-ggml-base.en.bin");
        let jfk = jfk_wav();
        let output = transcribe(
            root.path(),
            &[jfk.to_str().unwrap(), "--format", "json"],
            &[
                agent,
                ("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap()),
            ],
            None,
        );
        assert_json_error(&output, 3, "no_speech");
    }
}

#[test]
fn transcribe_refuses_what_it_cannot_use() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let dir = root.path();

    // Under half a second: exit 3, before any model is needed.
    let short = dir.join("short.wav");
    std::fs::write(&short, wav_16k(300, true)).unwrap();
    let output = transcribe(dir, &[short.to_str().unwrap()], &[], None);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        stderr(&output).trim(),
        "That recording was under half a second, so nothing was transcribed."
    );

    // Opus and WebM: exit 1 with the ffmpeg line.
    let opus = dir.join("memo.opus");
    std::fs::write(&opus, wav_16k(1_000, true)).unwrap();
    let output = transcribe(dir, &[opus.to_str().unwrap()], &[], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("Biorouter cannot read Opus recordings")
            && stderr(&output).contains("ffmpeg -i"),
        "{}",
        stderr(&output)
    );
    let webm = dir.join("talk.webm");
    std::fs::write(&webm, wav_16k(1_000, true)).unwrap();
    let output = transcribe(
        dir,
        &[webm.to_str().unwrap(), "--format", "json"],
        &[],
        None,
    );
    assert_json_error(&output, 1, "unsupported_format");

    // A file that is not there.
    let missing = dir.join("missing.wav");
    let output = transcribe(
        dir,
        &[missing.to_str().unwrap(), "--format", "json"],
        &[],
        None,
    );
    assert_json_error(&output, 1, "not_found");

    // Speech to transcribe, but no model on this computer: exit 2 with MSG-NO-MODEL-CLI.
    let tone = dir.join("tone.wav");
    std::fs::write(&tone, wav_16k(2_000, true)).unwrap();
    let output = transcribe(dir, &[tone.to_str().unwrap()], &[], None);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with("Dictation needs a speech model on this computer"),
        "{}",
        stderr(&output)
    );
    let output = transcribe(
        dir,
        &[
            tone.to_str().unwrap(),
            "--model",
            SMALL_EN,
            "--format",
            "json",
        ],
        &[],
        None,
    );
    let error = assert_json_error(&output, 2, "not_installed");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("biorouter dictation install whisper-small.en-q8_0"));
    let output = transcribe(
        dir,
        &[
            tone.to_str().unwrap(),
            "--model",
            "whisper-tiny",
            "--format",
            "json",
        ],
        &[],
        None,
    );
    assert_json_error(&output, 2, "unknown_model");
    let output = transcribe(
        dir,
        &[
            tone.to_str().unwrap(),
            "--language",
            "xx",
            "--format",
            "json",
        ],
        &[],
        None,
    );
    assert_json_error(&output, 2, "language");
}

/// A managed `enabled: false` declines every dictation door, `transcribe` included;
/// `capture: false` stops only live microphones.
#[test]
fn transcribe_follows_the_managed_switch() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let tone = root.path().join("tone.wav");
    std::fs::write(&tone, wav_16k(2_000, true)).unwrap();
    write_managed(root.path(), "  enabled: false\n");
    let output = transcribe(root.path(), &[tone.to_str().unwrap()], &[], None);
    assert_refused(&output, biorouter::managed::MANAGED_DICTATION_OFF);

    write_managed(root.path(), "  capture: false\n");
    let output = transcribe(root.path(), &[tone.to_str().unwrap()], &[], None);
    // Past the switch, to the missing model.
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).starts_with("Dictation needs a speech model"));
}

/// MP3 and M4A, which no test process can encode, through `ffmpeg` (the accuracy workflow
/// installs it). A 44.1 kHz stereo MP3 tone decodes to 16 kHz mono within 1 % of its
/// length: gapless decoding trims the encoder's delay and padding, which on a 3 s tone
/// were 1 % of it otherwise [M]. M4A (AAC) and Ogg Vorbis are refused with the `ffmpeg`
/// line, from a file and from standard input: DD-78's size rule left their decoders out.
#[cfg(feature = "dictation")]
#[test]
#[ignore = "needs ffmpeg"]
fn mp3_decodes_to_the_expected_length_and_m4a_is_refused() {
    use biorouter_cli::dictation::decode::{decode_path, decode_reader, DecodeError};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tone.wav");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=3",
            "-ac",
            "2",
        ])
        .arg(&source)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    let encode = |name: &str, codec: &str| {
        let encoded = dir.path().join(name);
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&source)
            .args(["-c:a", codec])
            .arg(&encoded)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success(), "{name}");
        encoded
    };
    let mp3 = decode_path(&encode("tone.mp3", "libmp3lame")).expect("MP3");
    let expected = 3.0 * 16_000.0;
    let got = mp3.samples.len() as f64;
    assert!(
        (got - expected).abs() <= expected * 0.01,
        "{got} samples, expected {expected}"
    );
    assert_eq!(mp3.channels, 2);

    // FLAC in an Ogg page stands for any Ogg stream: Vorbis needs libvorbis, which
    // Homebrew's ffmpeg does not carry, and every Ogg stream is refused alike.
    for (name, codec, kind) in [
        ("tone.m4a", "aac", "M4A and AAC"),
        ("tone.ogg", "flac", "Ogg"),
    ] {
        let encoded = encode(name, codec);
        let refused = DecodeError::UnsupportedFormat {
            kind: Some(kind.into()),
        };
        assert_eq!(decode_path(&encoded).unwrap_err(), refused, "{name}");
        let bytes = std::fs::read(&encoded).unwrap();
        assert_eq!(
            decode_reader(bytes.as_slice()).unwrap_err(),
            refused,
            "{name} on stdin"
        );
        assert!(refused.message(name).contains("ffmpeg -i"));
    }
}

/// PR-14's flags: `--session`, `--context-file` and `--no-dictionary` are accepted, a
/// context file that is not there is `not_found` (exit 1) before the model is needed, and a
/// `--session` naming no chat leaves the run as it was.
#[test]
fn transcribe_takes_the_dictionary_flags() {
    if !can_install() {
        return;
    }
    let root = sandbox();
    let model = vendored("models/for-tests-ggml-base.en.bin");
    let env = [("BIOROUTER_DICTATION_TEST_MODEL", model.to_str().unwrap())];
    let jfk = jfk_wav();
    let jfk = jfk.to_str().unwrap();
    let missing = root.path().join("no-such-context.txt");
    let output = transcribe(
        root.path(),
        &[
            jfk,
            "--format",
            "json",
            "--context-file",
            missing.to_str().unwrap(),
        ],
        &env,
        None,
    );
    assert_json_error(&output, 1, "not_found");
    let context = root.path().join("context.txt");
    std::fs::write(&context, "EPAS1 and HIF-2α in the Tibetan cohort").unwrap();
    for args in [
        vec![jfk, "--format", "json", "--no-dictionary"],
        vec![jfk, "--format", "json", "--session", "20261106_1"],
        vec![
            jfk,
            "--format",
            "json",
            "--context-file",
            context.to_str().unwrap(),
        ],
    ] {
        let output = transcribe(root.path(), &args, &env, None);
        assert_json_error(&output, 3, "no_speech");
    }
}

/// Imports `rows` (the export JSON's entries) into the sandbox's dictionary through a pipe,
/// the one write a run without a terminal may make.
fn import_rows(root: &Path, rows: Value) {
    let export = serde_json::json!({
        "format": "biorouter.dictionary", "version": 1,
        "exported_at": "2026-11-06T00:00:00Z", "entries": rows,
    });
    let mut child = Command::new(biorouter())
        .args(["dictionary", "import", "-"])
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env_remove("BIOROUTER_TERMINAL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run biorouter");
    {
        use std::io::Write;
        let mut pipe = child.stdin.take().expect("stdin");
        pipe.write_all(export.to_string().as_bytes())
            .expect("write");
    }
    let output = child.wait_with_output().expect("biorouter ends");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

/// A 16 kHz mono WAV of `pcm`.
fn wav_of(pcm: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    let data = (pcm.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Creates a chat in the sandbox's session store and raises it to Private the way a real
/// chat gets there, by a turn on a private provider, and returns its id.
fn private_chat(root: &Path) -> String {
    use biorouter::model::ModelConfig;
    use biorouter::privacy::SessionClassification;
    use biorouter::session::session_manager::SessionType;
    use biorouter::session::SessionManager;

    let data = root.join("data");
    tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(async {
            let manager = SessionManager::new(data.clone());
            let chat = manager
                .create_session(data.clone(), "Cohort notes".into(), SessionType::User)
                .await
                .expect("a chat");
            manager
                .update(&chat.id)
                .provider_name("versa_azure")
                .model_config(ModelConfig::new("gpt-5.5").expect("a model"))
                .raise_privacy(SessionClassification::Private, "turn:versa_azure")
                .apply()
                .await
                .expect("raised");
            let stored = manager
                .get_session(&chat.id, false)
                .await
                .expect("the chat");
            assert_eq!(stored.privacy_tier, SessionClassification::Private);
            manager.close().await;
            chat.id
        })
}

/// [`transcribe`] with standard input and standard error on a pseudo terminal, so the
/// command's `prompt_can_run()` is true, as for a person at a terminal. Standard output
/// stays a pipe for the JSON. What the command writes to the terminal is drained while it
/// runs and returned as `stderr`.
#[cfg(unix)]
fn transcribe_at_a_terminal(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    use std::io::Read;
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::sync::{Arc, Mutex};

    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty writes two new descriptors into `master` and `slave`; the name, the
    // terminal settings and the window size are not asked for.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    // SAFETY: both descriptors are open and owned by nothing else.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let mut command = Command::new(biorouter());
    command
        .arg("transcribe")
        .args(args)
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env_remove("BIOROUTER_TERMINAL")
        .env_remove("BIOROUTER_DICTATION_MODELS_DIR")
        .env_remove("BIOROUTER_DICTATION_TEST_MODEL")
        .env_remove("BIOROUTER_DICTATION_WORKER")
        .stdin(Stdio::from(slave.try_clone().expect("a second descriptor")))
        .stderr(Stdio::from(slave))
        .stdout(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let child = command.spawn().expect("run biorouter");
    // The command holds this process's copies of the terminal's other end; dropping it
    // closes them, so the terminal ends when the child and its worker do.
    drop(command);
    let written = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let written = Arc::clone(&written);
        std::thread::spawn(move || {
            let mut terminal = std::fs::File::from(master);
            let mut buf = [0u8; 4096];
            while let Ok(n) = terminal.read(&mut buf) {
                if n == 0 {
                    break;
                }
                written.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        })
    };
    let mut output = child.wait_with_output().expect("biorouter ends");
    // A worker that outlives the command could keep the terminal open; wait a little for
    // the drain, never for ever.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !reader.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    output.stderr = written.lock().unwrap().clone();
    output
}

/// PR-14, DD-77: in an agent's shell (`BIOROUTER_TERMINAL=1`), `transcribe --session` is
/// public only, whatever else holds. The run is set up so that everything else admits
/// private terms: the chat is a real private chat in the sandbox's store, the command runs
/// at a (pseudo) terminal, and the configured model is a private one. A control run of the
/// same command without `BIOROUTER_TERMINAL` must report `bias_scope: "private"`, so the
/// variable alone separates the two outcomes. The private terms are ones the token prompt
/// and the decoder bonus carry (none collides with an English word); on JFK's speech
/// played backwards, and forwards, none may come out of the agent's shell, and nothing
/// may be corrected there.
#[cfg(unix)]
#[test]
#[ignore = "needs the real speech models"]
fn an_agent_shell_never_gets_private_terms() {
    let models = std::env::var("BIOROUTER_DICTATION_MODELS_DIR")
        .expect("set BIOROUTER_DICTATION_MODELS_DIR to a folder holding the catalog models");
    let root = sandbox();
    std::fs::create_dir_all(root.path().join("config")).unwrap();
    // A private provider's name: only its declared tier is read, nothing is sent to it.
    std::fs::write(
        root.path().join("config").join("config.yaml"),
        "BIOROUTER_PROVIDER: versa_azure\nBIOROUTER_MODEL: gpt-5.5\n",
    )
    .unwrap();
    let chat = private_chat(root.path());
    let private: Vec<&str> = vec!["EPAS1", "EGLN1", "natalizumab", "ocrelizumab", "Biorouter"];
    for term in &private {
        assert!(
            !biorouter::dictation::recognize::collides(term),
            "{term} collides, so the prompt and the bonus would leave it out anyway"
        );
    }
    import_rows(
        root.path(),
        Value::Array(
            private
                .iter()
                .map(|w| serde_json::json!({"written": w, "tier": "private", "affiliation": null}))
                .collect(),
        ),
    );
    let bytes = std::fs::read(jfk_wav()).unwrap();
    let mut pcm = biorouter::dictation::audio::parse_wav_s16(&bytes)
        .expect("16 bit PCM")
        .to_16k_mono();
    pcm.reverse();
    let reversed = root.path().join("reversed.wav");
    std::fs::write(&reversed, wav_of(&pcm)).unwrap();
    let forward = jfk_wav();
    let selected: Vec<String> = std::env::var("BIOROUTER_DICTATION_ACCURACY_MODELS")
        .ok()
        .filter(|list| !list.trim().is_empty())
        .map(|list| list.split(',').map(|id| id.trim().to_string()).collect())
        .unwrap_or_else(|| vec![SMALL_EN.to_string(), TURBO.to_string()]);
    for model in selected.iter().map(String::as_str) {
        for audio in [reversed.to_str().unwrap(), forward.to_str().unwrap()] {
            let is_forward = audio == forward.to_str().unwrap();
            let args = [
                audio,
                "--model",
                model,
                "--format",
                "json",
                "--session",
                chat.as_str(),
            ];
            // The control: a person at a terminal, a private chat, a private model.
            let control = transcribe_at_a_terminal(
                root.path(),
                &args,
                &[("BIOROUTER_DICTATION_MODELS_DIR", models.as_str())],
            );
            let code = control.status.code();
            assert!(
                matches!(code, Some(0) | Some(3)) && (code == Some(0) || !is_forward),
                "{model} control: {}",
                stderr(&control)
            );
            let transcript = json(&control);
            if code == Some(0) {
                assert_eq!(
                    transcript["bias_scope"], "private",
                    "{model}: the control run must reach the private terms"
                );
            }
            println!(
                "{model} {audio} control: {:?}",
                transcript["text"].as_str().unwrap_or_default()
            );

            // The same run, in an agent's shell.
            let output = transcribe_at_a_terminal(
                root.path(),
                &args,
                &[
                    ("BIOROUTER_DICTATION_MODELS_DIR", models.as_str()),
                    ("BIOROUTER_TERMINAL", "1"),
                ],
            );
            let code = output.status.code();
            assert!(
                matches!(code, Some(0) | Some(3)) && (code == Some(0) || !is_forward),
                "{model}: {}",
                stderr(&output)
            );
            let transcript = json(&output);
            if code == Some(0) {
                assert_eq!(transcript["bias_scope"], "public", "{model}");
                let changes = transcript["changes"].as_array().unwrap();
                assert!(changes.is_empty(), "{model}: {changes:?}");
            }
            let text = transcript["text"].as_str().unwrap_or_default();
            for term in &private {
                assert!(!text.contains(term), "{model} wrote {term}: {text}");
            }
            println!("{model} {audio} agent shell: {text:?}");
        }
    }
}

/// `transcribe` with each catalog model found in `BIOROUTER_DICTATION_MODELS_DIR` writes
/// the JFK sentence and reports its timings (printed for the record). The models are
/// those `BIOROUTER_DICTATION_ACCURACY_MODELS` names, comma separated, or both.
#[test]
#[ignore = "needs the real speech models"]
fn transcribe_writes_the_jfk_sentence_with_each_catalog_model() {
    let models = std::env::var("BIOROUTER_DICTATION_MODELS_DIR")
        .expect("set BIOROUTER_DICTATION_MODELS_DIR to a folder holding the catalog models");
    let selected: Vec<String> = std::env::var("BIOROUTER_DICTATION_ACCURACY_MODELS")
        .ok()
        .filter(|list| !list.trim().is_empty())
        .map(|list| list.split(',').map(|id| id.trim().to_string()).collect())
        .unwrap_or_else(|| vec![SMALL_EN.to_string(), TURBO.to_string()]);
    let root = sandbox();
    let jfk = jfk_wav();
    for model in selected.iter().map(String::as_str) {
        let output = transcribe(
            root.path(),
            &[jfk.to_str().unwrap(), "--model", model, "--format", "json"],
            &[("BIOROUTER_DICTATION_MODELS_DIR", models.as_str())],
            None,
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{model}: {}",
            stderr(&output)
        );
        let transcript = json(&output);
        let text = transcript["text"].as_str().unwrap();
        assert_eq!(words(text), words(JFK_TEXT), "{model}: {text}");
        assert_eq!(transcript["model"], model);
        assert_eq!(transcript["no_speech"], false);
        println!("{model}: {text}\n{model}: {}", transcript["timings"]);
    }
}
