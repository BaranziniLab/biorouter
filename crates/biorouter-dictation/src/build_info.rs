//! What this binary's engine was built from, as `build.rs` read it from
//! `vendor/whisper.cpp/pin.json` and the target, and the offline fixtures' pins.

use serde::Serialize;

/// The first eight hex digits of the vendored whisper.cpp commit (`4da27c41`).
pub const WHISPER_COMMIT: &str = env!("BR_WHISPER_COMMIT");
/// The ggml version at that commit (`0.28.0`).
pub const GGML_VERSION: &str = env!("BR_GGML_VERSION");
/// The instruction set the engine was compiled for (build rule 2): `x86-64-v3`,
/// `armv8.2-a+dotprod+fp16` or `armv8.2-a+dotprod`; empty when no engine was built.
pub const ISA: &str = env!("BR_DICTATION_ISA");

/// The `build` object of `biorouter dictation status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BuildInfo {
    pub whisper: &'static str,
    pub ggml: &'static str,
    pub isa: &'static str,
}

/// `None` when this binary carries no engine.
pub fn build_info() -> Option<BuildInfo> {
    crate::ENGINE_COMPILED.then_some(BuildInfo {
        whisper: WHISPER_COMMIT,
        ggml: GGML_VERSION,
        isa: ISA,
    })
}

/// The four offline fixtures PR-08 vendored under `vendor/whisper.cpp/source`, with the
/// SHA-256 pins of `docs/dictation/engine-and-models.md` ("`pin.json` and
/// `source-manifest.json`"). The self test accepts only these files (P-11); PR-11 adds
/// the catalog's pins.
pub mod fixtures {
    /// `models/for-tests-ggml-base.en.bin`: real hyperparameters and vocabulary, no tensors.
    pub const BASE_EN: &str = "models/for-tests-ggml-base.en.bin";
    pub const BASE_EN_SHA256: &str =
        "1bc042ca584ff1895897e95bffb34ccf357be46c1fca97cf7fbe32f2060aa9e8";
    /// `models/for-tests-ggml-tiny.bin`: the multilingual counterpart.
    pub const TINY: &str = "models/for-tests-ggml-tiny.bin";
    pub const TINY_SHA256: &str =
        "c486fb9f14a28b1c1dc252741a431646cc573450c900b9d9c406e10294aa01e6";
    /// `models/for-tests-silero-v6.2.0-ggml.bin`: identical to the production detector.
    pub const SILERO: &str = "models/for-tests-silero-v6.2.0-ggml.bin";
    pub const SILERO_SHA256: &str =
        "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987";
    /// `samples/jfk.wav`: 11.0 s, 16 kHz, mono, 16 bit.
    pub const JFK: &str = "samples/jfk.wav";
    pub const JFK_SHA256: &str = "59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e";

    /// Models the self test accepts as `--model`.
    pub const MODEL_PINS: [&str; 2] = [BASE_EN_SHA256, TINY_SHA256];
    /// Detectors the self test accepts as `--vad`.
    pub const DETECTOR_PINS: [&str; 1] = [SILERO_SHA256];

    /// A fixture's path in this checkout, for tests.
    #[cfg(test)]
    pub(crate) fn path(relative: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("vendor")
            .join("whisper.cpp")
            .join("source")
            .join(relative)
    }
}

/// Lower case hex SHA-256 of a file, read in 1 MiB blocks.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        hasher.update(block.get(..n).unwrap_or_default());
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::fixtures::{self, path};
    use super::sha256_file;

    /// A CRLF conversion or a Git LFS pointer in place of a fixture fails here and names
    /// the file, before any engine test runs on it.
    #[test]
    fn fixtures_match_their_pins() {
        for (relative, pin) in [
            (fixtures::BASE_EN, fixtures::BASE_EN_SHA256),
            (fixtures::TINY, fixtures::TINY_SHA256),
            (fixtures::SILERO, fixtures::SILERO_SHA256),
            (fixtures::JFK, fixtures::JFK_SHA256),
        ] {
            let digest = sha256_file(&path(relative))
                .unwrap_or_else(|e| panic!("{relative} is not readable: {e}"));
            assert_eq!(
                digest, pin,
                "{relative} does not hash to its pin; a CRLF conversion or an LFS pointer?"
            );
        }
    }

    #[test]
    fn the_pin_strings_come_from_pin_json() {
        assert_eq!(super::WHISPER_COMMIT, "4da27c41");
        assert_eq!(super::GGML_VERSION, "0.28.0");
        assert_eq!(super::build_info().is_some(), crate::ENGINE_COMPILED);
        if crate::ENGINE_COMPILED {
            assert!(!super::ISA.is_empty());
        }
    }
}
