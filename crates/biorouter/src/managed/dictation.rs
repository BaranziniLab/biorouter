//! The managed `dictation:` block (DD-90).
//!
//! An administrator narrows local dictation through a `dictation:` block in the
//! managed policy file. [`super::settings::ManagedPolicyFile`] keeps the block as
//! raw YAML and this module parses it on its own, for two reasons:
//!
//! * A bad block must not fail the file. A parse error anywhere in the file makes
//!   the whole policy inert, which lifts every managed deny, so a dictation key an
//!   older binary does not know must not be able to do that.
//! * The block fails closed for itself (O-27). An unknown key, a value of the
//!   wrong type, and a file that could not be trusted, read or parsed all turn
//!   dictation off, with a warning, while the rest of the file still applies.
//!
//! Every key only narrows, and `true` equals absent. Nothing here caches:
//! [`super::ManagedPolicy::load`] reads the file on every call and computes the
//! [`DictationPolicy`] it carries then, so an administrator's change applies at
//! the next request.
//!
//! The switch prevents accidents; it is not a boundary. `BIOROUTER_PATH_ROOT`
//! relocates the policy file, and on Windows the file's location is trusted
//! rather than verified ([`super::trust`]).

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use tracing::warn;
use utoipa::ToSchema;

use crate::permission::managed_inspector::MANAGED_DENY_REASON;

/// MSG-POLICY: dictation is off.
pub const MANAGED_DICTATION_OFF: &str =
    "Dictation is turned off on this computer by your organization's managed policy.";

/// MSG-POLICY-CAPTURE: live microphones are off, transcribing a recording is not.
pub const MANAGED_DICTATION_CAPTURE_OFF: &str =
    "Your organization's managed policy turns off live microphone dictation on this computer. \
     Transcribing a recording still works.";

/// The prefix every tool of the `dictionary` capability carries.
pub const DICTIONARY_TOOL_PREFIX: &str = "dictionary__";

/// Every key a `dictation:` block may carry. Any other key turns dictation off
/// (O-27), so a newer key on an older binary narrows rather than being ignored.
pub const DICTATION_POLICY_KEYS: [&str; 8] = [
    "enabled",
    "capture",
    "model_downloads",
    "allowed_models",
    "models_dir",
    "external_engine",
    "dictionary",
    "dictionary_agent_writes",
];

/// MSG-POLICY-DOWNLOADS, naming the folder models must come from.
fn downloads_sentence(models_dir: Option<&Path>) -> String {
    let folder = models_dir.map_or_else(
        || "a folder your administrator chooses".to_string(),
        |dir| dir.display().to_string(),
    );
    format!(
        "Your organization's managed policy turns off model downloads on this computer. \
         Speech models come only from {folder}; ask your administrator to place one there."
    )
}

/// MSG-POLICY-MODELS, naming the ids the policy allows.
fn models_sentence(allowed: &[String]) -> String {
    let models = if allowed.is_empty() {
        "none".to_string()
    } else {
        allowed.join(", ")
    };
    format!("Your organization's managed policy allows only these speech models here: {models}.")
}

/// Whether one door is open, and if not, the sentence to show. This is the shape
/// every `policy` entry takes in `GET /dictation/status` and every refusal's
/// reason (P-15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Allowance {
    pub allowed: bool,
    /// The sentence to show when `allowed` is false; `None` when it is true.
    pub reason: Option<String>,
}

impl Allowance {
    /// An open door.
    pub fn granted() -> Self {
        Self {
            allowed: true,
            reason: None,
        }
    }

    /// A closed door, with the sentence that says why.
    pub fn refused(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: Some(reason.into()),
        }
    }
}

/// One key's resolved verdict.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Rule {
    #[default]
    Allowed,
    /// Forbidden by the administrator, with the sentence to show.
    Forbidden(Cow<'static, str>),
}

impl Rule {
    fn gate(allowed: bool, reason: impl Into<Cow<'static, str>>) -> Self {
        if allowed {
            Self::Allowed
        } else {
            Self::Forbidden(reason.into())
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// The sentence to show, when forbidden.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allowed => None,
            Self::Forbidden(reason) => Some(reason.as_ref()),
        }
    }

    pub fn allowance(&self) -> Allowance {
        match self {
            Self::Allowed => Allowance::granted(),
            Self::Forbidden(reason) => Allowance::refused(reason.clone().into_owned()),
        }
    }
}

/// Why a `dictation:` block turned dictation off. The text names the key, so the
/// warning tells the administrator what to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    /// `dictation:` with no value. Usually its keys lost their indentation and
    /// became top-level keys, which the file ignores.
    Empty,
    NotAMapping,
    UnknownKey(String),
    WrongType {
        key: &'static str,
        expected: &'static str,
    },
    RelativeModelsDir(String),
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "`dictation` has no value (indent its keys under it; keys written at the top \
                 level of the file are ignored)"
            ),
            Self::NotAMapping => write!(f, "`dictation` is not a mapping of keys to values"),
            Self::UnknownKey(key) => write!(
                f,
                "`dictation.{key}` is not a key this version knows (it knows {})",
                DICTATION_POLICY_KEYS.join(", ")
            ),
            Self::WrongType { key, expected } => {
                write!(f, "`dictation.{key}` must be {expected}")
            }
            Self::RelativeModelsDir(dir) => write!(
                f,
                "`dictation.models_dir` must be an absolute folder path, not {dir:?}"
            ),
        }
    }
}

impl std::error::Error for BlockError {}

/// The administrator's dictation rules, resolved once when a
/// [`super::ManagedPolicy`] value is built.
///
/// The rules are stored already composed: `enabled: false` forbids every other
/// rule with MSG-POLICY, and `dictionary: false` forbids
/// `dictionary_agent_writes` too, so a door that reads one rule gets the right
/// answer without knowing the others.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DictationPolicy {
    /// Whether an administrator's policy speaks about dictation at all: a
    /// `dictation:` block (valid or not), or a managed file that could not be
    /// trusted, read or parsed. False with no file, and with a trusted file
    /// that has no `dictation:` key.
    pub managed: bool,
    /// Capture and transcription anywhere, downloads, and the dictionary.
    pub enabled: Rule,
    /// Live microphones. Transcribing a recording does not read this.
    pub capture: Rule,
    /// Downloading speech models. When forbidden, `models_dir` is the only
    /// source of models.
    pub model_downloads: Rule,
    /// The only model ids that may be installed, selected or used, when set.
    pub allowed_models: Option<Vec<String>>,
    /// A read only folder of speech models, first in the lookup order. Its files
    /// are still hash checked.
    pub models_dir: Option<PathBuf>,
    /// Parsed in v1 and without effect: there is no off machine engine yet
    /// (DD-87). Parsing it keeps a later policy file from turning v1 off.
    pub external_engine: Rule,
    /// Dictionary edits, the sidebar row, the capability, the extractor and bias.
    pub dictionary: Rule,
    /// The agent's dictionary tools and the chat extractor; when forbidden,
    /// `agent_writes` is pinned to `suggest`.
    pub dictionary_agent_writes: Rule,
}

impl DictationPolicy {
    /// No administrator rule: everything allowed, `managed: false`.
    pub fn unmanaged() -> Self {
        Self::default()
    }

    /// Dictation off everywhere, as for a file that could not be trusted, read
    /// or parsed, or a block this version cannot understand.
    pub fn off() -> Self {
        Self::off_keeping(None, None)
    }

    fn off_keeping(allowed_models: Option<Vec<String>>, models_dir: Option<PathBuf>) -> Self {
        let off = || Rule::Forbidden(Cow::Borrowed(MANAGED_DICTATION_OFF));
        Self {
            managed: true,
            enabled: off(),
            capture: off(),
            model_downloads: off(),
            allowed_models,
            models_dir,
            external_engine: off(),
            dictionary: off(),
            dictionary_agent_writes: off(),
        }
    }

    /// The policy for a trusted file's `dictation:` value, warning and turning
    /// dictation off when the block cannot be understood. `source` names the
    /// file in the warning.
    pub(crate) fn from_block(block: Option<&Value>, source: &Path) -> Self {
        let Some(block) = block else {
            return Self::unmanaged();
        };
        Self::parse_block(block).unwrap_or_else(|error| {
            warn!(
                "managed policy: dictation is off because {error} in {}",
                source.display()
            );
            Self::off()
        })
    }

    /// Parse a `dictation:` block. Every key only narrows; `true` equals absent;
    /// an unknown key or a value of the wrong type is an error (O-27).
    pub fn parse_block(block: &Value) -> Result<Self, BlockError> {
        let map = match block {
            Value::Mapping(map) => map,
            Value::Null => return Err(BlockError::Empty),
            _ => return Err(BlockError::NotAMapping),
        };
        let mut raw = RawBlock::default();
        for (key, value) in map {
            let Some(name) = key.as_str() else {
                return Err(BlockError::UnknownKey(render_key(key)));
            };
            match name {
                "enabled" => raw.enabled = flag("enabled", value)?,
                "capture" => raw.capture = flag("capture", value)?,
                "model_downloads" => raw.model_downloads = flag("model_downloads", value)?,
                "allowed_models" => raw.allowed_models = Some(model_ids(value)?),
                "models_dir" => raw.models_dir = Some(folder(value)?),
                "external_engine" => raw.external_engine = flag("external_engine", value)?,
                "dictionary" => raw.dictionary = flag("dictionary", value)?,
                "dictionary_agent_writes" => {
                    raw.dictionary_agent_writes = flag("dictionary_agent_writes", value)?
                }
                other => return Err(BlockError::UnknownKey(other.to_string())),
            }
        }
        Ok(raw.resolve())
    }

    /// Whether `id` may be installed, selected or used: MSG-POLICY when
    /// dictation is off, MSG-POLICY-MODELS when `allowed_models` leaves it out.
    pub fn model(&self, id: &str) -> Allowance {
        if !self.enabled.is_allowed() {
            return self.enabled.allowance();
        }
        match &self.allowed_models {
            Some(allowed) if !allowed.iter().any(|candidate| candidate == id) => {
                Allowance::refused(models_sentence(allowed))
            }
            _ => Allowance::granted(),
        }
    }

    /// Whether `id` may be downloaded, in the precedence `install.reason`
    /// documents: MSG-POLICY, then MSG-POLICY-DOWNLOADS, then MSG-POLICY-MODELS.
    /// This is the managed half only; the operator lock and the caller's
    /// authority are the route's to add.
    pub fn install(&self, id: &str) -> Allowance {
        if !self.model_downloads.is_allowed() {
            return self.model_downloads.allowance();
        }
        self.model(id)
    }

    /// Whether this policy denies a tool by name: every `dictionary__*` tool
    /// while the agent may not write the dictionary, which `enabled: false` and
    /// `dictionary: false` both imply.
    pub fn denies_tool(&self, tool_name: &str) -> bool {
        tool_name.starts_with(DICTIONARY_TOOL_PREFIX) && !self.dictionary_agent_writes.is_allowed()
    }
}

/// A block's keys as written, before the rules are composed.
struct RawBlock {
    enabled: bool,
    capture: bool,
    model_downloads: bool,
    allowed_models: Option<Vec<String>>,
    models_dir: Option<PathBuf>,
    external_engine: bool,
    dictionary: bool,
    dictionary_agent_writes: bool,
}

impl Default for RawBlock {
    /// Absent equals `true`: an empty block allows everything.
    fn default() -> Self {
        Self {
            enabled: true,
            capture: true,
            model_downloads: true,
            allowed_models: None,
            models_dir: None,
            external_engine: true,
            dictionary: true,
            dictionary_agent_writes: true,
        }
    }
}

impl RawBlock {
    fn resolve(self) -> DictationPolicy {
        if !self.enabled {
            return DictationPolicy::off_keeping(self.allowed_models, self.models_dir);
        }
        let dictionary = Rule::gate(self.dictionary, MANAGED_DICTATION_OFF);
        let dictionary_agent_writes = if dictionary.is_allowed() {
            Rule::gate(self.dictionary_agent_writes, MANAGED_DENY_REASON)
        } else {
            dictionary.clone()
        };
        DictationPolicy {
            managed: true,
            enabled: Rule::Allowed,
            capture: Rule::gate(self.capture, MANAGED_DICTATION_CAPTURE_OFF),
            model_downloads: Rule::gate(
                self.model_downloads,
                downloads_sentence(self.models_dir.as_deref()),
            ),
            allowed_models: self.allowed_models,
            models_dir: self.models_dir,
            external_engine: Rule::gate(self.external_engine, MANAGED_DENY_REASON),
            dictionary,
            dictionary_agent_writes,
        }
    }
}

fn render_key(key: &Value) -> String {
    serde_yaml::to_string(key)
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|_| "a key that is not text".to_string())
}

fn flag(key: &'static str, value: &Value) -> Result<bool, BlockError> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        _ => Err(BlockError::WrongType {
            key,
            expected: "true or false",
        }),
    }
}

fn model_ids(value: &Value) -> Result<Vec<String>, BlockError> {
    let wrong = || BlockError::WrongType {
        key: "allowed_models",
        expected: "a list of speech model ids",
    };
    let Value::Sequence(items) = value else {
        return Err(wrong());
    };
    items
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(wrong))
        .collect()
}

fn folder(value: &Value) -> Result<PathBuf, BlockError> {
    let Value::String(text) = value else {
        return Err(BlockError::WrongType {
            key: "models_dir",
            expected: "an absolute folder path",
        });
    };
    let path = PathBuf::from(text);
    if text.trim().is_empty() || !path.is_absolute() {
        return Err(BlockError::RelativeModelsDir(text.clone()));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed::{ManagedPolicy, ManagedPolicyFile, ManagedVerdict};

    fn parse(yaml: &str) -> Result<DictationPolicy, BlockError> {
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        DictationPolicy::parse_block(&value)
    }

    fn policy_from(yaml: &str) -> ManagedPolicy {
        let file: ManagedPolicyFile = serde_yaml::from_str(yaml).unwrap();
        ManagedPolicy::from_file(file)
    }

    /// An absolute folder on whichever system runs the test.
    fn absolute_dir() -> PathBuf {
        std::env::temp_dir().join("biorouter-managed-models")
    }

    fn assert_off(policy: &DictationPolicy, case: &str) {
        assert!(policy.managed, "{case}: managed");
        for (name, rule) in [
            ("enabled", &policy.enabled),
            ("capture", &policy.capture),
            ("model_downloads", &policy.model_downloads),
            ("external_engine", &policy.external_engine),
            ("dictionary", &policy.dictionary),
            ("dictionary_agent_writes", &policy.dictionary_agent_writes),
        ] {
            assert_eq!(
                rule.reason(),
                Some(MANAGED_DICTATION_OFF),
                "{case}: {name} must be off with MSG-POLICY"
            );
        }
        assert_eq!(
            policy.install("whisper-small.en-q8_0"),
            Allowance::refused(MANAGED_DICTATION_OFF),
            "{case}"
        );
        assert!(policy.denies_tool("dictionary__add_terms"), "{case}");
    }

    fn assert_all_allowed(policy: &DictationPolicy, case: &str) {
        for (name, rule) in [
            ("enabled", &policy.enabled),
            ("capture", &policy.capture),
            ("model_downloads", &policy.model_downloads),
            ("external_engine", &policy.external_engine),
            ("dictionary", &policy.dictionary),
            ("dictionary_agent_writes", &policy.dictionary_agent_writes),
        ] {
            assert!(rule.is_allowed(), "{case}: {name}");
            assert_eq!(rule.allowance(), Allowance::granted(), "{case}: {name}");
        }
        assert_eq!(policy.allowed_models, None, "{case}");
        assert_eq!(policy.models_dir, None, "{case}");
        assert_eq!(policy.install("any-model"), Allowance::granted(), "{case}");
        assert!(!policy.denies_tool("dictionary__add_terms"), "{case}");
    }

    #[test]
    fn no_file_is_unmanaged() {
        let policy = ManagedPolicy::empty();
        assert!(!policy.dictation().managed);
        assert_all_allowed(policy.dictation(), "no file");
        assert_eq!(policy.permission_for("dictionary__add_terms"), None);
    }

    #[test]
    fn a_trusted_file_without_a_block_is_unmanaged_for_dictation() {
        let policy = policy_from("permissions:\n  deny: [\"developer__shell\"]\n");
        assert!(!policy.dictation().managed);
        assert_all_allowed(policy.dictation(), "no block");
        assert_eq!(policy.permission_for("dictionary__list_terms"), None);
    }

    #[test]
    fn an_empty_block_and_true_values_equal_absent() {
        let empty = parse("{}").unwrap();
        assert!(empty.managed);
        assert_all_allowed(&empty, "empty block");

        let all_true = parse(
            "enabled: true\ncapture: true\nmodel_downloads: true\nexternal_engine: true\n\
             dictionary: true\ndictionary_agent_writes: true\n",
        )
        .unwrap();
        assert_eq!(all_true, empty, "true must equal absent for every key");
    }

    #[test]
    fn enabled_false_turns_every_door_off() {
        let policy = parse("enabled: false\n").unwrap();
        assert_off(&policy, "enabled: false");
        assert_eq!(
            policy.model("whisper-small.en-q8_0"),
            Allowance::refused(MANAGED_DICTATION_OFF)
        );
        for tool in [
            "dictionary__add_terms",
            "dictionary__list_terms",
            "dictionary__remove_added_terms",
        ] {
            assert!(policy.denies_tool(tool), "{tool}");
        }
        assert!(!policy.denies_tool("developer__shell"));
        assert!(!policy.denies_tool("dictionaryish__tool"));
    }

    /// `enabled: false` wins even over keys that try to open a door.
    #[test]
    fn enabled_false_is_not_reopened_by_other_keys() {
        let policy = parse("capture: true\nenabled: false\ndictionary: true\n").unwrap();
        assert_off(&policy, "enabled: false with true keys");
    }

    #[test]
    fn capture_false_stops_only_live_microphones() {
        let policy = parse("capture: false\n").unwrap();
        assert!(policy.managed);
        assert!(policy.enabled.is_allowed());
        assert_eq!(
            policy.capture.allowance(),
            Allowance::refused(MANAGED_DICTATION_CAPTURE_OFF)
        );
        assert!(policy.model_downloads.is_allowed());
        assert!(policy.dictionary.is_allowed());
        assert!(policy.dictionary_agent_writes.is_allowed());
        assert_eq!(
            policy.install("whisper-small.en-q8_0"),
            Allowance::granted()
        );
        assert!(!policy.denies_tool("dictionary__add_terms"));
    }

    #[test]
    fn model_downloads_false_names_the_managed_folder() {
        let dir = absolute_dir();
        let yaml = format!(
            "model_downloads: false\nmodels_dir: {}\n",
            serde_yaml::to_string(&dir.display().to_string())
                .unwrap()
                .trim()
        );
        let policy = parse(&yaml).unwrap();
        assert_eq!(policy.models_dir.as_deref(), Some(dir.as_path()));
        let refusal = policy.install("whisper-small.en-q8_0");
        assert!(!refusal.allowed);
        let reason = refusal.reason.unwrap();
        assert!(
            reason.starts_with(
                "Your organization's managed policy turns off model downloads on this computer."
            ),
            "{reason}"
        );
        assert!(reason.contains(&dir.display().to_string()), "{reason}");
        // Downloads are the only thing narrowed: the model itself is allowed.
        assert_eq!(policy.model("whisper-small.en-q8_0"), Allowance::granted());
        assert!(policy.capture.is_allowed());
    }

    #[test]
    fn model_downloads_false_without_a_folder_still_refuses() {
        let policy = parse("model_downloads: false\n").unwrap();
        let reason = policy.install("x").reason.unwrap();
        assert!(
            reason.contains("a folder your administrator chooses"),
            "{reason}"
        );
    }

    #[test]
    fn allowed_models_refuses_other_ids_and_names_the_allowed_ones() {
        let policy =
            parse("allowed_models: [whisper-small.en-q8_0, whisper-base.en-q8_0]\n").unwrap();
        assert_eq!(policy.model("whisper-small.en-q8_0"), Allowance::granted());
        assert_eq!(policy.install("whisper-base.en-q8_0"), Allowance::granted());
        assert_eq!(
            policy.model("whisper-large-v3-turbo-q5_0"),
            Allowance::refused(
                "Your organization's managed policy allows only these speech models here: \
                 whisper-small.en-q8_0, whisper-base.en-q8_0."
            )
        );
        assert!(!policy.install("whisper-large-v3-turbo-q5_0").allowed);
        assert!(policy.capture.is_allowed());
    }

    #[test]
    fn an_empty_allowed_models_list_allows_no_model() {
        let policy = parse("allowed_models: []\n").unwrap();
        assert_eq!(
            policy.model("whisper-small.en-q8_0").reason.as_deref(),
            Some("Your organization's managed policy allows only these speech models here: none.")
        );
    }

    /// The precedence `install.reason` documents: downloads before the model list.
    #[test]
    fn install_reports_downloads_before_the_model_list() {
        let policy = parse("model_downloads: false\nallowed_models: [a]\n").unwrap();
        let reason = policy.install("b").reason.unwrap();
        assert!(reason.contains("turns off model downloads"), "{reason}");
        assert!(policy.model("b").reason.unwrap().contains("allows only"));
    }

    #[test]
    fn dictionary_false_denies_the_tools_and_the_agent_writes() {
        let policy = parse("dictionary: false\n").unwrap();
        assert_eq!(policy.dictionary.reason(), Some(MANAGED_DICTATION_OFF));
        assert_eq!(
            policy.dictionary_agent_writes.reason(),
            Some(MANAGED_DICTATION_OFF),
            "no dictionary means no agent writes"
        );
        assert!(policy.denies_tool("dictionary__list_terms"));
        // Transcription, capture and installs are untouched.
        assert!(policy.enabled.is_allowed());
        assert!(policy.capture.is_allowed());
        assert_eq!(policy.install("x"), Allowance::granted());
    }

    #[test]
    fn dictionary_agent_writes_false_denies_the_tools_only() {
        let policy = parse("dictionary_agent_writes: false\n").unwrap();
        assert!(policy.dictionary.is_allowed());
        assert_eq!(
            policy.dictionary_agent_writes.reason(),
            Some(MANAGED_DENY_REASON)
        );
        for tool in [
            "dictionary__add_terms",
            "dictionary__list_terms",
            "dictionary__remove_added_terms",
        ] {
            assert!(policy.denies_tool(tool), "{tool}");
        }
    }

    #[test]
    fn external_engine_false_is_parsed_and_narrows_only_itself() {
        let policy = parse("external_engine: false\n").unwrap();
        assert_eq!(policy.external_engine.reason(), Some(MANAGED_DENY_REASON));
        assert!(policy.enabled.is_allowed());
        assert!(policy.capture.is_allowed());
        assert!(!policy.denies_tool("dictionary__add_terms"));
    }

    #[test]
    fn the_documented_example_block_parses() {
        let dir = absolute_dir();
        let yaml = format!(
            "enabled: true\ncapture: false\nmodel_downloads: false\n\
             allowed_models: [whisper-small.en-q8_0]\nmodels_dir: {}\n\
             external_engine: false\ndictionary: true\ndictionary_agent_writes: false\n",
            serde_yaml::to_string(&dir.display().to_string())
                .unwrap()
                .trim()
        );
        let policy = parse(&yaml).unwrap();
        assert!(policy.managed);
        assert!(policy.enabled.is_allowed());
        assert!(!policy.capture.is_allowed());
        assert!(!policy.model_downloads.is_allowed());
        assert_eq!(
            policy.allowed_models.as_deref(),
            Some(&["whisper-small.en-q8_0".to_string()][..])
        );
        assert!(!policy.external_engine.is_allowed());
        assert!(policy.dictionary.is_allowed());
        assert!(!policy.dictionary_agent_writes.is_allowed());
    }

    /// O-27: a key this version does not know turns dictation off, and the
    /// error names the key so the warning says what to fix.
    #[test]
    fn an_unknown_key_turns_dictation_off_and_is_named() {
        let error = parse("enabled: true\nengine_url: https://example.org\n").unwrap_err();
        assert_eq!(error, BlockError::UnknownKey("engine_url".to_string()));
        assert!(
            error.to_string().contains("dictation.engine_url"),
            "{error}"
        );

        let error = parse("1: true\n").unwrap_err();
        assert!(matches!(error, BlockError::UnknownKey(_)), "{error:?}");

        let policy = policy_from(
            "dictation:\n  capture: true\n  engine_url: x\npermissions:\n  deny: [\"developer__shell\"]\n",
        );
        assert_off(policy.dictation(), "unknown key");
        assert_eq!(
            policy.permission_for("developer__shell"),
            Some(ManagedVerdict::Deny),
            "the rest of the file still applies"
        );
    }

    #[test]
    fn a_wrong_type_turns_dictation_off_and_is_named() {
        let cases: [(&str, &str); 12] = [
            ("enabled: \"false\"\n", "enabled"),
            ("enabled: 0\n", "enabled"),
            // YAML 1.2: these are text, not switches (the policy doc says so).
            ("enabled: yes\n", "enabled"),
            ("capture: no\n", "capture"),
            ("dictionary: off\n", "dictionary"),
            ("capture: no-thanks\n", "capture"),
            ("capture:\n", "capture"),
            ("model_downloads: [false]\n", "model_downloads"),
            ("allowed_models: whisper-small.en-q8_0\n", "allowed_models"),
            (
                "allowed_models: [whisper-small.en-q8_0, 3]\n",
                "allowed_models",
            ),
            ("models_dir: [a, b]\n", "models_dir"),
            ("dictionary_agent_writes: {}\n", "dictionary_agent_writes"),
        ];
        for (yaml, key) in cases {
            let error = parse(yaml).unwrap_err();
            assert!(
                matches!(&error, BlockError::WrongType { key: named, .. } if *named == key),
                "{yaml:?}: {error:?}"
            );
            assert!(error.to_string().contains(key), "{yaml:?}: {error}");
        }
    }

    #[test]
    fn a_relative_or_blank_models_dir_turns_dictation_off() {
        for yaml in [
            "models_dir: models\n",
            "models_dir: ./models\n",
            "models_dir: \"\"\n",
        ] {
            assert!(
                matches!(parse(yaml), Err(BlockError::RelativeModelsDir(_))),
                "{yaml:?}"
            );
        }
    }

    #[test]
    fn a_block_that_is_not_a_mapping_turns_dictation_off() {
        for yaml in ["false\n", "[enabled]\n", "42\n", "text\n"] {
            assert_eq!(parse(yaml), Err(BlockError::NotAMapping), "{yaml:?}");
        }
        let policy = policy_from("dictation: false\n");
        assert_off(policy.dictation(), "dictation: false");
    }

    /// An empty `dictation:` is a block, not an absent key, so it turns
    /// dictation off (O-27) and the warning says what probably went wrong.
    #[test]
    fn an_empty_block_turns_dictation_off_and_says_why() {
        for yaml in ["~\n", "null\n", "\n"] {
            assert_eq!(parse(yaml), Err(BlockError::Empty), "{yaml:?}");
        }
        assert!(
            BlockError::Empty.to_string().contains("indent its keys"),
            "{}",
            BlockError::Empty
        );
    }

    /// The administrator's likeliest slip: the block's keys lost their
    /// indentation, so YAML reads `dictation:` as empty and `enabled: false` as
    /// a top-level key the file ignores. Dictation is off rather than on, and
    /// the rest of the file still applies.
    #[test]
    fn an_empty_or_unindented_block_turns_dictation_off_and_keeps_permissions() {
        for block in [
            "dictation:\n",
            "dictation: ~\n",
            "dictation:\nenabled: false\n",
        ] {
            let policy = policy_from(&format!(
                "{block}permissions:\n  deny: [developer__shell]\n"
            ));
            assert_off(policy.dictation(), block);
            assert_eq!(
                policy.permission_for("developer__shell"),
                Some(ManagedVerdict::Deny),
                "{block:?}: the rest of the file still applies"
            );
        }
    }

    /// A malformed block leaves `permissions.deny` in force, and the dictionary
    /// tools join the denied set.
    #[test]
    fn a_malformed_block_leaves_permissions_deny_in_force() {
        let policy = policy_from(
            "dictation:\n  enabled: perhaps\npermissions:\n  deny: [\"developer__shell\"]\n  \
             allow: [\"dictionary__add_terms\"]\n",
        );
        assert!(policy.is_active());
        assert_off(policy.dictation(), "malformed block");
        assert_eq!(
            policy.permission_for("developer__shell"),
            Some(ManagedVerdict::Deny)
        );
        assert_eq!(
            policy.permission_for("dictionary__add_terms"),
            Some(ManagedVerdict::Deny),
            "a managed allow cannot reopen a tool dictation policy denies"
        );
        assert_eq!(policy.permission_for("developer__text_editor"), None);
    }

    #[test]
    fn allowance_serializes_as_allowed_and_reason() {
        assert_eq!(
            serde_json::to_value(Allowance::granted()).unwrap(),
            serde_json::json!({"allowed": true, "reason": null})
        );
        assert_eq!(
            serde_json::to_value(Allowance::refused(MANAGED_DICTATION_OFF)).unwrap(),
            serde_json::json!({"allowed": false, "reason": MANAGED_DICTATION_OFF})
        );
    }

    /// The sentences are the message catalog's, word for word.
    #[test]
    fn the_policy_sentences_match_the_message_catalog() {
        assert_eq!(
            MANAGED_DICTATION_OFF,
            "Dictation is turned off on this computer by your organization's managed policy."
        );
        assert_eq!(
            MANAGED_DICTATION_CAPTURE_OFF,
            "Your organization's managed policy turns off live microphone dictation on this \
             computer. Transcribing a recording still works."
        );
        assert_eq!(
            downloads_sentence(Some(Path::new("/srv/models"))),
            "Your organization's managed policy turns off model downloads on this computer. \
             Speech models come only from /srv/models; ask your administrator to place one there."
        );
    }

    /// Every key the parser accepts is in the published list, and nothing else.
    #[test]
    fn every_listed_key_parses_and_no_other() {
        for key in DICTATION_POLICY_KEYS {
            let value = match key {
                "allowed_models" => "[]".to_string(),
                "models_dir" => serde_yaml::to_string(&absolute_dir().display().to_string())
                    .unwrap()
                    .trim()
                    .to_string(),
                _ => "true".to_string(),
            };
            parse(&format!("{key}: {value}\n"))
                .unwrap_or_else(|e| panic!("{key} is listed but refused: {e}"));
        }
    }
}
