//! Local dictation in the host process: the one [`service::DictationService`] per
//! `biorouterd` or interactive `biorouter`, which owns the supervisor of the engine's
//! worker (`docs/dictation/engine-and-models.md`, "Supervisor and service API"),
//! [`paths`], the order in which the folders that may hold speech models are read,
//! [`file`], which transcribes a whole recording (`biorouter transcribe`), [`bias`], the
//! one producer of the dictionary entries a dictation may use (DD-36), and [`target`],
//! which decides whether a dictation is aimed at a private chat (DD-30, DD-77).
//!
//! The engine itself never runs here (DD-4): it runs in a worker process that the
//! service starts from this binary with a hidden first argument. A build without the
//! `local-dictation` feature still has this module; its status answers `no_engine` and
//! every request answers [`MSG_NO_ENGINE`].

pub mod bias;
pub mod file;
pub mod paths;
pub mod service;
pub mod target;

/// The catalog, the installer and the suitability rules, from the leaf crate, so a
/// caller built without the engine reaches them too; the PCM helpers and the recognizer
/// for `biorouter transcribe`.
pub use biorouter_dictation::{audio, catalog, hardware, models, recognize};

pub use file::{RecordingTimings, RecordingTranscript, TimedText};

pub use bias::{bias_set, BiasChannels, BiasTarget, DictionaryRequest, Standing};
pub use biorouter_dictation::audio::{PcmSlice, Span};
pub use biorouter_dictation::protocol::{Hello, Loaded, TranscribeResult};
pub use biorouter_dictation::supervisor::LoadSpec;
pub use service::{
    service, DictatedRecording, DictationError, DictationService, EngineBuild, EngineState,
    EngineStatus, TranscribeRequest, WarmUp, WorkerProbe,
};
pub use target::{chat_reach, cli_bias, ChatReach, CliTerminal};

/// MSG-NO-ENGINE.
pub const MSG_NO_ENGINE: &str = "This build of Biorouter has no local dictation engine.";

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn repo_file(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            // A Windows checkout may carry CRLF line ends.
            .replace("\r\n", "\n")
    }

    /// The `Version:` line of `NOTICE`'s entry headed `name`.
    fn notice_version(notice: &str, name: &str) -> String {
        let heading = format!("\n{name}\n");
        let (_, entry) = notice
            .split_once(&heading)
            .unwrap_or_else(|| panic!("NOTICE has no {name} entry"));
        let entry = entry.split("\n\n\n").next().unwrap_or(entry);
        let line = entry
            .lines()
            .find(|l| l.trim_start().starts_with("Version:"))
            .unwrap_or_else(|| panic!("NOTICE's {name} entry has no Version line"));
        let mut version = line
            .trim_start()
            .trim_start_matches("Version:")
            .trim()
            .to_string();
        // A version may continue on the next line, indented.
        let rest: Vec<&str> = entry.lines().skip_while(|l| *l != line).skip(1).collect();
        for next in rest {
            if next.starts_with("             ") && !next.trim().is_empty() {
                version.push(' ');
                version.push_str(next.trim());
            } else {
                break;
            }
        }
        version
    }

    /// DD-18: `NOTICE` names the versions the code builds. The whisper.cpp commit and
    /// ggml version come from `vendor/whisper.cpp/pin.json` through the leaf crate's
    /// build facts; miniaudio's from the vendored header; llama.cpp's from
    /// `LLAMA_SERVER_BUILD`, which a stale "build b9611" contradicted until PR-11.
    #[test]
    fn notice_versions_match_the_code() {
        let notice = repo_file("NOTICE");
        let pin: serde_json::Value =
            serde_json::from_str(&repo_file("vendor/whisper.cpp/pin.json")).unwrap();
        let commit = pin["commit"].as_str().unwrap();
        assert!(commit.starts_with(biorouter_dictation::build_info::WHISPER_COMMIT));
        let whisper = notice_version(&notice, "whisper.cpp");
        assert!(
            whisper.contains(&format!("pinned at commit {commit}")),
            "NOTICE says whisper.cpp {whisper}, the pin is {commit}"
        );
        let ggml = biorouter_dictation::build_info::GGML_VERSION;
        assert!(
            whisper.contains(&format!("ggml {ggml}")),
            "NOTICE says {whisper}, ggml is {ggml}"
        );

        let header = repo_file("vendor/whisper.cpp/source/examples/miniaudio.h");
        let define = |name: &str| -> String {
            header
                .lines()
                .find_map(|l| l.strip_prefix(&format!("#define {name} ")))
                .unwrap_or_else(|| panic!("miniaudio.h has no {name}"))
                .trim()
                .to_string()
        };
        let miniaudio = format!(
            "{}.{}.{}",
            define("MA_VERSION_MAJOR"),
            define("MA_VERSION_MINOR"),
            define("MA_VERSION_REVISION")
        );
        assert_eq!(notice_version(&notice, "miniaudio"), miniaudio);

        let llama = crate::providers::llamacpp_sidecar::LLAMA_SERVER_BUILD;
        assert_eq!(
            notice_version(&notice, "llama.cpp"),
            format!("build {llama}")
        );

        // The speech models entry names where they come from, and that they are not
        // shipped.
        let (_, entry) = notice
            .split_once("\nSpeech models downloaded on request\n")
            .expect("NOTICE has no speech models entry");
        let entry = entry.split("\n\n\n").next().unwrap_or(entry);
        assert!(entry.contains(biorouter_dictation::catalog::MODEL_REPOSITORY));
        for license in [
            biorouter_dictation::catalog::LICENSE_OPENAI_WHISPER,
            biorouter_dictation::catalog::LICENSE_WHISPER_CPP,
            biorouter_dictation::catalog::LICENSE_SILERO_VAD,
        ] {
            assert!(entry.contains(license.file_name), "{}", license.file_name);
        }
    }
}
