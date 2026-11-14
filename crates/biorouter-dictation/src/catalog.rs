//! The speech model catalog compiled into every binary (DD-14, DD-17,
//! `docs/dictation/engine-and-models.md`, "Catalog").
//!
//! [`STT_MODEL_CATALOG`] is the only source of model pins. Every file is checked against
//! its SHA-256 before anything parses it, and every route and command names a model by
//! its id, never by a URL. The catalog changes only with an app release; there is no
//! remote catalog. The URLs point at immutable releases of `BaranziniLab/biorouter-models`
//! (DD-16), never at `releases/latest`, Hugging Face or the GitHub API.

use crate::cpu::HardwareClass;

/// Speech or voice detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttKind {
    Speech,
    Vad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttEngine {
    WhisperCpp,
}

/// The legacy ggml file format (magic `0x67676d6c`, the bytes `lmgg` on disk), not GGUF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttFormat {
    GgmlBin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Languages {
    EnglishOnly,
    Multilingual,
    NotApplicable,
}

impl Languages {
    /// The `languages` field of `list --format json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Languages::EnglishOnly => "en",
            Languages::Multilingual => "multilingual",
            Languages::NotApplicable => "n/a",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tokenizer {
    Gpt2,
    Multilingual,
    NotApplicable,
}

/// A license text compiled into the binary and written beside every installed file
/// (DD-18), so a model copied in with `--from` carries its notice too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LicenseText {
    pub file_name: &'static str,
    pub text: &'static str,
}

pub const LICENSE_OPENAI_WHISPER: LicenseText = LicenseText {
    file_name: "LICENSE-openai-whisper.txt",
    text: include_str!("../licenses/LICENSE-openai-whisper.txt"),
};
pub const LICENSE_WHISPER_CPP: LicenseText = LicenseText {
    file_name: "LICENSE-whisper.cpp.txt",
    text: include_str!("../licenses/LICENSE-whisper.cpp.txt"),
};
pub const LICENSE_SILERO_VAD: LicenseText = LicenseText {
    file_name: "LICENSE-silero-vad.txt",
    text: include_str!("../licenses/LICENSE-silero-vad.txt"),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SttCatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: SttKind,
    pub engine: SttEngine,
    pub format: SttFormat,
    pub languages: Languages,
    pub tokenizer: Tokenizer,
    /// The upstream file name, kept by the release and by the installer.
    pub file_name: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    /// One GitHub release URL each in v1.
    pub urls: &'static [&'static str],
    pub release_tag: &'static str,
    pub licenses: &'static [LicenseText],
    /// Speech models require the voice detector.
    pub requires: &'static [&'static str],
    /// The hardware classes the model is offered on.
    pub classes: &'static [HardwareClass],
    /// The classes whose `biorouter dictation install` with no argument picks it.
    pub default_for: &'static [HardwareClass],
    /// Total RAM below this rates the model `below_minimum` (DD-22).
    pub min_total_ram_gib: u32,
    /// Earlier revisions that stay valid; always contains `sha256` (DD-23).
    pub accepted_sha256: &'static [&'static str],
}

impl SttCatalogEntry {
    pub fn is_speech(&self) -> bool {
        self.kind == SttKind::Speech
    }

    /// Whether `digest` is this entry's current pin or an earlier accepted revision.
    pub fn accepts(&self, digest: &str) -> bool {
        self.sha256 == digest || self.accepted_sha256.contains(&digest)
    }

    /// The GitHub URL every attempt requests again (DD-21).
    pub fn primary_url(&self) -> &'static str {
        self.urls[0]
    }

    pub fn english_only(&self) -> bool {
        self.languages == Languages::EnglishOnly
    }
}

/// An id that is no longer in the catalog, and the id that replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetiredSttModel {
    pub id: &'static str,
    pub replaced_by: &'static str,
}

/// The repository every pin comes from.
pub const MODEL_REPOSITORY: &str = "BaranziniLab/biorouter-models";
/// How a person reads the source in `install`'s summary.
pub const MODEL_SOURCE: &str = "github.com/BaranziniLab/biorouter-models";
/// Every URL in the catalog starts with this.
pub const URL_PREFIX: &str =
    "https://github.com/BaranziniLab/biorouter-models/releases/download/stt-";
/// The legacy ggml file magic as it appears on disk: `0x67676d6c` little endian.
pub const GGML_MAGIC: [u8; 4] = *b"lmgg";

pub const TURBO_ID: &str = "whisper-large-v3-turbo-q5_0";
pub const SMALL_EN_ID: &str = "whisper-small.en-q8_0";
pub const VAD_ID: &str = "silero-vad-v6.2.0";

const TURBO_SHA256: &str = "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2";
const SMALL_EN_SHA256: &str = "67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067";
const VAD_SHA256: &str = "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987";

/// The three pinned files of v1. Their digests equal what GitHub reports for the release
/// assets (checked on 2026-11-06 and every night by `dictation-model-pins.yml`).
pub const STT_MODEL_CATALOG: &[SttCatalogEntry] = &[
    SttCatalogEntry {
        id: TURBO_ID,
        display_name: "Whisper large-v3-turbo (q5_0)",
        kind: SttKind::Speech,
        engine: SttEngine::WhisperCpp,
        format: SttFormat::GgmlBin,
        languages: Languages::Multilingual,
        tokenizer: Tokenizer::Multilingual,
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: TURBO_SHA256,
        urls: &["https://github.com/BaranziniLab/biorouter-models/releases/download/stt-whisper-large-v3-turbo-q5_0-r1/ggml-large-v3-turbo-q5_0.bin"],
        release_tag: "stt-whisper-large-v3-turbo-q5_0-r1",
        licenses: &[LICENSE_OPENAI_WHISPER, LICENSE_WHISPER_CPP],
        requires: &[VAD_ID],
        classes: &[HardwareClass::As, HardwareClass::Cpu],
        default_for: &[HardwareClass::As],
        min_total_ram_gib: 8,
        accepted_sha256: &[TURBO_SHA256],
    },
    SttCatalogEntry {
        id: SMALL_EN_ID,
        display_name: "Whisper small.en (q8_0)",
        kind: SttKind::Speech,
        engine: SttEngine::WhisperCpp,
        format: SttFormat::GgmlBin,
        languages: Languages::EnglishOnly,
        tokenizer: Tokenizer::Gpt2,
        file_name: "ggml-small.en-q8_0.bin",
        size_bytes: 264_477_561,
        sha256: SMALL_EN_SHA256,
        urls: &["https://github.com/BaranziniLab/biorouter-models/releases/download/stt-whisper-small.en-q8_0-r1/ggml-small.en-q8_0.bin"],
        release_tag: "stt-whisper-small.en-q8_0-r1",
        licenses: &[LICENSE_OPENAI_WHISPER, LICENSE_WHISPER_CPP],
        requires: &[VAD_ID],
        classes: &[HardwareClass::As, HardwareClass::Cpu],
        default_for: &[HardwareClass::Cpu],
        min_total_ram_gib: 4,
        accepted_sha256: &[SMALL_EN_SHA256],
    },
    SttCatalogEntry {
        id: VAD_ID,
        display_name: "Silero voice detector v6.2.0",
        kind: SttKind::Vad,
        engine: SttEngine::WhisperCpp,
        format: SttFormat::GgmlBin,
        languages: Languages::NotApplicable,
        tokenizer: Tokenizer::NotApplicable,
        file_name: "ggml-silero-v6.2.0.bin",
        size_bytes: 885_098,
        sha256: VAD_SHA256,
        urls: &["https://github.com/BaranziniLab/biorouter-models/releases/download/stt-silero-vad-v6.2.0-r1/ggml-silero-v6.2.0.bin"],
        release_tag: "stt-silero-vad-v6.2.0-r1",
        licenses: &[LICENSE_SILERO_VAD],
        requires: &[],
        classes: &[HardwareClass::As, HardwareClass::Cpu],
        default_for: &[],
        min_total_ram_gib: 0,
        accepted_sha256: &[VAD_SHA256],
    },
];

/// Retired ids, the `RETIRED_MODELS` shape of the Llama Server catalog. None yet.
pub const RETIRED_STT_MODELS: &[RetiredSttModel] = &[];

/// A catalog: the compiled one in every build, a synthetic one in tests.
#[derive(Clone, Copy, Debug)]
pub struct Catalog {
    pub entries: &'static [SttCatalogEntry],
    pub retired: &'static [RetiredSttModel],
}

/// The compiled catalog.
pub const CATALOG: Catalog = Catalog {
    entries: STT_MODEL_CATALOG,
    retired: RETIRED_STT_MODELS,
};

impl Catalog {
    pub fn entry(&self, id: &str) -> Option<&'static SttCatalogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// `id`, or the replacement of a retired `id`; the flag says it was retired.
    pub fn resolve(&self, id: &str) -> Option<(&'static SttCatalogEntry, bool)> {
        if let Some(entry) = self.entry(id) {
            return Some((entry, false));
        }
        let retired = self.retired.iter().find(|r| r.id == id)?;
        self.entry(retired.replaced_by).map(|entry| (entry, true))
    }

    /// The entry a file with `digest` belongs to, by its pin or an accepted revision.
    pub fn by_sha256(&self, digest: &str) -> Option<&'static SttCatalogEntry> {
        self.entries.iter().find(|e| e.accepts(digest))
    }

    /// The entries whose pinned size is `size`: the only files worth hashing.
    pub fn sized(&self, size: u64) -> impl Iterator<Item = &'static SttCatalogEntry> {
        self.entries.iter().filter(move |e| e.size_bytes == size)
    }

    pub fn speech_models(&self) -> impl Iterator<Item = &'static SttCatalogEntry> {
        self.entries.iter().filter(|e| e.is_speech())
    }

    /// The model `install` with no argument picks for `class`; none for `No`.
    pub fn default_for(&self, class: HardwareClass) -> Option<&'static SttCatalogEntry> {
        self.speech_models()
            .find(|entry| entry.default_for.contains(&class))
    }

    /// `entry` and every file it requires, required files first.
    pub fn with_requirements(
        &self,
        entry: &'static SttCatalogEntry,
    ) -> Vec<&'static SttCatalogEntry> {
        let mut files: Vec<&'static SttCatalogEntry> = entry
            .requires
            .iter()
            .filter_map(|id| self.entry(id))
            .collect();
        files.push(entry);
        files
    }

    /// Speech models that require `vad_id`.
    pub fn requiring(&self, vad_id: &str) -> impl Iterator<Item = &'static SttCatalogEntry> + '_ {
        let vad_id = vad_id.to_string();
        self.speech_models()
            .filter(move |entry| entry.requires.contains(&vad_id.as_str()))
    }
}

/// Whether `bytes` starts with the legacy ggml magic.
pub fn has_ggml_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == GGML_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DD-14, DD-17: the invariants `engine-and-models.md` lists under "Catalog".
    #[test]
    fn catalog_invariants() {
        let ids: Vec<&str> = STT_MODEL_CATALOG.iter().map(|e| e.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "duplicate ids: {ids:?}");
        for entry in STT_MODEL_CATALOG {
            let id = entry.id;
            assert_eq!(entry.sha256.len(), 64, "{id}: sha256 is not 64 digits");
            assert!(
                entry
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{id}: sha256 is not lower case hex"
            );
            assert!(
                entry.size_bytes > 0 && entry.size_bytes < 2_147_483_648,
                "{id}: size must be under GitHub's 2 GiB asset limit"
            );
            assert!(!entry.urls.is_empty(), "{id}: no URL");
            for url in entry.urls {
                assert!(url.starts_with(URL_PREFIX), "{id}: {url}");
                assert!(
                    url.ends_with(&format!("/{}", entry.file_name)),
                    "{id}: {url}"
                );
                assert!(
                    url.contains(&format!("/{}/", entry.release_tag)),
                    "{id}: {url} does not name {}",
                    entry.release_tag
                );
                for banned in ["huggingface.co", "api.github.com", "/latest/"] {
                    assert!(!url.contains(banned), "{id}: {url} contains {banned}");
                }
            }
            assert!(
                entry.release_tag.starts_with(&format!("stt-{id}-r")),
                "{id}: tag {} is not stt-<id>-r<n>",
                entry.release_tag
            );
            for required in entry.requires {
                let target = CATALOG.entry(required).unwrap_or_else(|| {
                    panic!("{id} requires {required}, which is not in the catalog")
                });
                assert_eq!(target.kind, SttKind::Vad, "{id} requires a speech model");
            }
            assert!(entry.accepted_sha256.contains(&entry.sha256), "{id}");
            assert!(!entry.licenses.is_empty(), "{id}: no license text");
            for license in entry.licenses {
                assert!(license.text.starts_with("MIT License"), "{id}");
                assert!(license.file_name.starts_with("LICENSE-"), "{id}");
            }
            assert!(
                entry.file_name.is_ascii() && entry.file_name.ends_with(".bin"),
                "{id}: {}",
                entry.file_name
            );
            if entry.is_speech() {
                assert_eq!(entry.requires, &[VAD_ID], "{id}");
            }
        }
        for class in [HardwareClass::As, HardwareClass::Cpu] {
            let defaults: Vec<&str> = CATALOG
                .speech_models()
                .filter(|e| e.default_for.contains(&class))
                .map(|e| e.id)
                .collect();
            assert_eq!(defaults.len(), 1, "{class:?} has defaults {defaults:?}");
        }
        assert!(CATALOG.default_for(HardwareClass::No).is_none());
        for retired in RETIRED_STT_MODELS {
            assert!(
                CATALOG.entry(retired.id).is_none(),
                "{} is retired and listed",
                retired.id
            );
            assert!(
                CATALOG.entry(retired.replaced_by).is_some(),
                "{} points at {}",
                retired.id,
                retired.replaced_by
            );
        }
    }

    /// The three pins of `engine-and-models.md`, which equal the digests GitHub reports
    /// for A-1's immutable releases.
    #[test]
    fn the_pins_are_the_published_ones() {
        let turbo = CATALOG.entry(TURBO_ID).unwrap();
        assert_eq!(turbo.size_bytes, 574_041_195);
        assert_eq!(turbo.sha256, TURBO_SHA256);
        let small = CATALOG.entry(SMALL_EN_ID).unwrap();
        assert_eq!(small.size_bytes, 264_477_561);
        let vad = CATALOG.entry(VAD_ID).unwrap();
        assert_eq!(vad.size_bytes, 885_098);
        assert_eq!(CATALOG.default_for(HardwareClass::As).unwrap().id, TURBO_ID);
        assert_eq!(
            CATALOG.default_for(HardwareClass::Cpu).unwrap().id,
            SMALL_EN_ID
        );
        // The vendored detector fixture is byte identical to the production detector.
        assert_eq!(VAD_SHA256, crate::build_info::fixtures::SILERO_SHA256);
    }

    #[test]
    fn lookups() {
        assert_eq!(
            CATALOG.resolve(SMALL_EN_ID).map(|(e, r)| (e.id, r)),
            Some((SMALL_EN_ID, false))
        );
        assert!(CATALOG.resolve("whisper-medium").is_none());
        assert_eq!(CATALOG.by_sha256(VAD_SHA256).unwrap().id, VAD_ID);
        assert!(CATALOG.by_sha256(&"0".repeat(64)).is_none());
        assert_eq!(CATALOG.sized(885_098).count(), 1);
        let files: Vec<&str> = CATALOG
            .with_requirements(CATALOG.entry(TURBO_ID).unwrap())
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(files, [VAD_ID, TURBO_ID]);
        let users: Vec<&str> = CATALOG.requiring(VAD_ID).map(|e| e.id).collect();
        assert_eq!(users, [TURBO_ID, SMALL_EN_ID]);
        assert!(has_ggml_magic(b"lmggxyz"));
        assert!(!has_ggml_magic(b"GGUF"));
        assert!(!has_ggml_magic(b"lm"));
    }

    #[test]
    fn retired_ids_resolve_to_their_replacement() {
        static RETIRED: &[RetiredSttModel] = &[RetiredSttModel {
            id: "whisper-small.en-q5_1",
            replaced_by: SMALL_EN_ID,
        }];
        let catalog = Catalog {
            entries: STT_MODEL_CATALOG,
            retired: RETIRED,
        };
        let (entry, retired) = catalog.resolve("whisper-small.en-q5_1").unwrap();
        assert_eq!(entry.id, SMALL_EN_ID);
        assert!(retired);
    }
}
