//! The personal dictionary's store, where it lives and what keeps tools out of it
//! (`docs/dictation/dictionary.md`, "Store protection and boundaries", DD-42).
//!
//! The store is `<config>/dictionary/`: `dictionary.json`, `usage.json` and `.lock`
//! ([`dictionary_dir`]). Its code is the leaf crate's ([`store`]), re-exported here so
//! the CLI, and from PR-16 the routes, open it through the one folder this module
//! resolves.
//!
//! # Why a tool may not read it
//!
//! The dictionary holds the person's terms in two tiers, and every door that lists or
//! changes them filters by the caller's standing: a public chat never sees a private
//! term. `cat ~/.config/biorouter/dictionary/dictionary.json` reaches every term with
//! none of that, and `text_editor` writing the file would land rows with no stamp at
//! all. So [`DictionaryStoreInspector`] refuses a tool call that names the folder as a
//! path, for callers of both tiers: the refusal is about the channel, as for the session
//! store (`security::session_store`), whose argument reading this module repeats, so a
//! doc or a commit message that merely names the path is not refused.
//!
//! # The two writing tools at the uninspected doors
//!
//! `POST /agent/call_tool` dispatches with `CallCapability::public_enforced()` on the
//! secret alone, and a script's inner calls inherit the capability its
//! `execute_code` call was admitted with, which for a script a person ran through that
//! route is the same public one. A private chat's term added there would land public.
//! So [`uninspected_boundary_refusal`] refuses `dictionary__add_terms` and
//! `dictionary__remove_added_terms` at both doors, from either tier, and the store's
//! path there too, read from the already evaluated arguments.
//! `dictionary__list_terms` stays callable from scripts.
//!
//! # What this is not
//!
//! A shell can still read the file (DR-17 left the filesystem open), and a command that
//! computes the path walks past the inspector, which sees only the call a model wrote.
//! These are accident prevention for cooperative actors, as everywhere in Biorouter's
//! privacy tiers, not a boundary against a determined one.
//!
//! # The dictation history
//!
//! The same inspector, and the same refusal at the uninspected doors, cover the dictation
//! history's folder, `<local data>/dictation/history/` (P-18,
//! `crate::dictation::history::history_dir`). It holds the text of everything the person
//! dictated, private dictations included, and the only door a chat has to it is the
//! capability's search, which applies the tier and institution rules; `cat` of the file
//! would apply none. The refusal names the dictation history.
//!
//! # How a path is spelled
//!
//! A folder is looked for in the forms a tool argument might spell it in, built from the
//! folder `Paths` resolves. On Windows a form and the text are compared with `\` and `/`
//! alike and letter case ignored, and the forms include `%LOCALAPPDATA%`, `%APPDATA%`,
//! the profile folder in cmd, PowerShell and POSIX syntax, and the verbatim `\\?\`
//! prefix. (The history's folder resolves there as `...\data\dictation/history`, so an
//! exact comparison missed every path written with `\`.) Off Windows the four forms and
//! the exact comparison are unchanged. A short (8.3) name or a `..` in the path is not
//! recognized.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::config::paths::Paths;
use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::security::policy::command::{redirect_targets, ParsedCommand};
use crate::security::policy::{EnvFacts, Platform};
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};

/// The leaf crate's dictionary store.
pub use biorouter_dictation::store;

/// The store's folder under the configuration folder.
pub const DICTIONARY_FOLDER: &str = "dictionary";

/// This inspector's name, as it appears on an [`InspectionResult`].
pub const DICTIONARY_STORE_INSPECTOR_NAME: &str = "dictionary_store";

/// The capability's tool that writes terms (PR-23).
pub const ADD_TERMS_TOOL: &str = "dictionary__add_terms";
/// The capability's tool that removes the agent's own unreviewed terms (PR-23).
pub const REMOVE_ADDED_TERMS_TOOL: &str = "dictionary__remove_added_terms";

/// `<config>/dictionary`: `~/.config/biorouter/dictionary` on macOS and Linux,
/// `%APPDATA%\UCSF\biorouter\config\dictionary` on Windows, and
/// `$BIOROUTER_PATH_ROOT/config/dictionary` under a sandbox root.
pub fn dictionary_dir() -> PathBuf {
    Paths::in_config_dir(DICTIONARY_FOLDER)
}

/// This account's dictionary.
pub fn open_store() -> store::DictionaryStore {
    store::DictionaryStore::open(dictionary_dir())
}

/// Whether `path` is the store folder or inside it, compared as written and, where the
/// path exists, after resolving links.
pub fn is_in_dictionary_store(path: &Path) -> bool {
    is_inside(path, &dictionary_dir())
}

/// Whether `path` is the dictation history's folder or inside it, compared the same way.
pub fn is_in_dictation_history(path: &Path) -> bool {
    is_inside(path, &crate::dictation::history::history_dir())
}

fn is_inside(path: &Path, dir: &Path) -> bool {
    if path.starts_with(dir) {
        return true;
    }
    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => false,
    }
}

/// The folders this module keeps tools out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtectedFolder {
    Dictionary,
    History,
}

impl ProtectedFolder {
    const ALL: [ProtectedFolder; 2] = [ProtectedFolder::Dictionary, ProtectedFolder::History];

    fn dir(self) -> PathBuf {
        match self {
            ProtectedFolder::Dictionary => dictionary_dir(),
            ProtectedFolder::History => crate::dictation::history::history_dir(),
        }
    }
}

/// How a platform writes a path, as data, so that the Windows spellings are checked on
/// every platform and not only on a Windows runner.
///
/// On Windows `\` and `/` both separate a path's components, a run of them reads as one,
/// and letter case never matters, so a form and the text it is looked for in are both
/// compared folded ([`Self::fold`]). A protected folder is also reached there through
/// `%LOCALAPPDATA%`, `%APPDATA%` and the profile folder, and through a verbatim
/// `\\?\` prefix. Elsewhere a path is compared exactly as written, and the forms are the
/// four they always were.
#[derive(Debug, Clone)]
struct PathSpelling {
    /// The platform whose shell reads a command, and whose rules fold a spelling.
    platform: Platform,
    /// Folders a path may start from by name, each with the folder it stands for.
    anchors: Vec<(&'static str, String)>,
}

/// The variables a Windows path may start from, read from the environment.
const WINDOWS_ANCHORS: [&str; 4] = ["USERPROFILE", "HOME", "LOCALAPPDATA", "APPDATA"];

/// The prefixes that put a Windows drive path in its verbatim or device form.
const WINDOWS_PATH_PREFIXES: [&str; 2] = [r"\\?\", r"\\.\"];

/// Does `path` start with a drive letter (`C:`)?
fn has_drive_letter(path: &str) -> bool {
    let mut chars = path.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next() == Some(':')
}

impl PathSpelling {
    /// The running platform, with the folders its environment names.
    fn host() -> Self {
        let platform = Platform::host();
        let anchors = if platform.is_windows() {
            WINDOWS_ANCHORS
                .into_iter()
                .filter_map(|name| {
                    std::env::var_os(name)
                        .map(|value| (name, value.to_string_lossy().into_owned()))
                        .filter(|(_, value)| !value.is_empty())
                })
                .collect()
        } else {
            // The home folder as these forms always read it: `HOME`, else `USERPROFILE`.
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| ("HOME", home.to_string_lossy().into_owned()))
                .into_iter()
                .collect()
        };
        PathSpelling { platform, anchors }
    }

    /// `text` as this platform compares it. On Windows every run of `\` and `/` becomes
    /// one `/` and ASCII letters become lower case; elsewhere `text` is unchanged. A match
    /// in the text as written is still a match folded, so folding only ever refuses more.
    fn fold<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if !self.platform.is_windows() {
            return Cow::Borrowed(text);
        }
        let mut folded = String::with_capacity(text.len());
        let mut after_separator = false;
        for c in text.chars() {
            if c == '/' || c == '\\' {
                if !after_separator {
                    folded.push('/');
                }
                after_separator = true;
            } else {
                folded.push(c.to_ascii_lowercase());
                after_separator = false;
            }
        }
        Cow::Owned(folded)
    }

    /// What follows `anchor` in `dir` when `dir` is inside it, as a form writes it.
    fn tail_under(&self, dir: &str, anchor: &str) -> Option<String> {
        if !self.platform.is_windows() {
            // Exactly as these forms were always computed.
            return Path::new(dir)
                .strip_prefix(Path::new(anchor))
                .ok()
                .map(|tail| tail.to_string_lossy().into_owned());
        }
        let dir = self.fold(dir);
        let anchor = self.fold(anchor);
        let anchor = anchor.trim_end_matches('/');
        if anchor.is_empty() {
            return None;
        }
        let tail = dir.strip_prefix(anchor)?.strip_prefix('/')?;
        (!tail.is_empty()).then(|| tail.to_string())
    }

    /// How a path names the folder the variable `name` stands for: `~`, `$HOME` and
    /// `${HOME}` for the home folder, and on Windows also cmd's `%NAME%`, PowerShell's
    /// `$env:NAME` and `${env:NAME}`, and a POSIX shell's `$NAME` and `${NAME}`.
    fn anchor_prefixes(&self, name: &str) -> Vec<String> {
        let windows = self.platform.is_windows();
        let mut prefixes = Vec::new();
        if name == "HOME" || (windows && name == "USERPROFILE") {
            prefixes.extend(["~".to_string(), "$HOME".to_string(), "${HOME}".to_string()]);
        }
        if windows {
            for prefix in [
                format!("%{name}%"),
                format!("$env:{name}"),
                format!("${{env:{name}}}"),
                format!("${name}"),
                format!("${{{name}}}"),
            ] {
                if !prefixes.contains(&prefix) {
                    prefixes.push(prefix);
                }
            }
        }
        prefixes
    }

    /// Every spelling of the folder `dir` a tool argument might carry: the absolute path
    /// (on Windows also in its verbatim and device forms), and the path from each folder
    /// a variable names that holds it, the home folder's `~`, `$HOME` and `${HOME}` forms
    /// among them.
    fn forms(&self, dir: &str) -> Vec<String> {
        let mut forms = vec![dir.to_string()];
        let push = |form: String, forms: &mut Vec<String>| {
            if !forms.contains(&form) {
                forms.push(form);
            }
        };
        let mut plain = dir;
        if self.platform.is_windows() {
            plain = WINDOWS_PATH_PREFIXES
                .iter()
                .find_map(|prefix| dir.strip_prefix(prefix))
                .filter(|rest| has_drive_letter(rest))
                .unwrap_or(dir);
            if has_drive_letter(plain) {
                push(plain.to_string(), &mut forms);
                for prefix in WINDOWS_PATH_PREFIXES {
                    push(format!("{prefix}{plain}"), &mut forms);
                }
            }
        }
        for (name, value) in &self.anchors {
            let Some(tail) = self.tail_under(plain, value) else {
                continue;
            };
            for prefix in self.anchor_prefixes(name) {
                push(format!("{prefix}/{tail}"), &mut forms);
            }
        }
        forms
    }

    /// `command` read by this platform's shell, through the policy engine's tokenizer.
    fn parse(&self, command: &str) -> ParsedCommand {
        // The host's own facts on the host, exactly `ParsedCommand::parse`; another
        // platform's facts in a test that reads its spelling. The words a command splits
        // into do not depend on them.
        let env = if self.platform == Platform::host() {
            EnvFacts::host(".")
        } else {
            EnvFacts::for_platform(self.platform, ".", "")
        };
        ParsedCommand::parse_for(command, self.platform, &env)
    }
}

/// Every spelling of a protected folder a tool argument might carry on this platform.
fn store_path_forms(folder: ProtectedFolder, spelling: &PathSpelling) -> Vec<String> {
    spelling.forms(&folder.dir().to_string_lossy())
}

/// Does `text` name `store` as a path, ending on a path boundary, rather than merely
/// start with its characters (`<config>/dictionary-notes/` is not the store)?
fn names_path(text: &str, store: &str) -> bool {
    text.match_indices(store).any(|(at, matched)| {
        text.get(at + matched.len()..)
            .and_then(|after| after.chars().next())
            .is_none_or(|c| {
                c == '/' || c == '\\' || c.is_whitespace() || "\"'`)];,;:&|".contains(c)
            })
    })
}

/// What an argument's value is, which decides how a mention of the store's path in it
/// is read: see `security::session_store`, whose distinction this repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgumentRole {
    Path,
    ShellCommand,
    Script,
}

/// Argument keys whose value is a directory, on top of the shared path keys: the store
/// is a folder, so a tool run inside it reaches it.
const DIRECTORY_ARG_KEYS: &[&str] = &["dir", "directory", "working_directory", "local_dir"];

/// Argument keys whose value is executable source text.
const SCRIPT_ARG_KEYS: &[&str] = &["code", "script"];

fn argument_role(key: &str) -> Option<ArgumentRole> {
    let lowered = key.to_ascii_lowercase();
    if crate::security::is_path_argument_key(&lowered)
        || DIRECTORY_ARG_KEYS.contains(&lowered.as_str())
    {
        return Some(ArgumentRole::Path);
    }
    if crate::security::is_command_argument_key(&lowered) {
        return Some(ArgumentRole::ShellCommand);
    }
    if SCRIPT_ARG_KEYS.contains(&lowered.as_str()) {
        return Some(ArgumentRole::Script);
    }
    None
}

/// Does this shell command line name the store as a path operand or a redirect
/// target? Read through the policy engine's tokenizer, so a quoted commit message that
/// mentions the path is one word and not a path.
fn shell_command_names_store(command: &str, forms: &[String], spelling: &PathSpelling) -> bool {
    let forms: Vec<Cow<'_, str>> = forms.iter().map(|form| spelling.fold(form)).collect();
    let token_is_store = |token: &str| {
        let token = spelling.fold(token);
        forms.iter().any(|form| {
            token.strip_prefix(form.as_ref()).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
            })
        })
    };
    let parsed = spelling.parse(command);
    parsed
        .segments
        .iter()
        .flat_map(|segment| segment.argv.iter())
        .any(|token| token_is_store(token))
        || redirect_targets(command).iter().any(|t| token_is_store(t))
}

/// Which protected folder `args` names in an argument that would open it, if any.
fn referenced_folder(args: &Map<String, Value>) -> Option<ProtectedFolder> {
    let spelling = PathSpelling::host();
    ProtectedFolder::ALL
        .into_iter()
        .find(|&folder| references_folder(args, &store_path_forms(folder, &spelling), &spelling))
}

/// Does `args` name the folder these forms spell in an argument that would open it?
fn references_folder(args: &Map<String, Value>, forms: &[String], spelling: &PathSpelling) -> bool {
    fn in_map(map: &Map<String, Value>, forms: &[String], spelling: &PathSpelling) -> bool {
        map.iter().any(|(key, value)| match argument_role(key) {
            Some(role) => in_role(value, forms, spelling, role),
            None => descend(value, forms, spelling),
        })
    }

    fn descend(value: &Value, forms: &[String], spelling: &PathSpelling) -> bool {
        match value {
            Value::Object(map) => in_map(map, forms, spelling),
            Value::Array(items) => items.iter().any(|item| descend(item, forms, spelling)),
            _ => false,
        }
    }

    fn in_role(
        value: &Value,
        forms: &[String],
        spelling: &PathSpelling,
        role: ArgumentRole,
    ) -> bool {
        match value {
            Value::String(text) => match role {
                ArgumentRole::Path | ArgumentRole::Script => {
                    let text = spelling.fold(text);
                    forms
                        .iter()
                        .any(|form| names_path(&text, &spelling.fold(form)))
                }
                ArgumentRole::ShellCommand => shell_command_names_store(text, forms, spelling),
            },
            Value::Array(items) => items
                .iter()
                .any(|item| in_role(item, forms, spelling, role)),
            Value::Object(map) => map.values().any(|v| in_role(v, forms, spelling, role)),
            _ => false,
        }
    }

    in_map(args, forms, spelling)
}

/// What the model is told when a call names the store. It names the doors that work.
fn path_refusal() -> String {
    "Refused: this call names Biorouter's dictionary store by path. That folder holds the \
     person's dictionary terms, including terms kept only for private chats, and reading \
     or writing it as a file bypasses every check that decides which terms a chat may \
     see or change. The refusal is about the channel, not about your capability: it \
     applies to a private-capability session exactly as it does to a public one. The \
     person views and changes their dictionary on the Dictionary page in Biorouter, or \
     with `biorouter dictionary` in their own terminal. This project's own files are not \
     affected."
        .to_string()
}

/// What the model is told when a call names the dictation history's folder.
fn history_path_refusal() -> String {
    "Refused: this call names Biorouter's dictation history by path. That folder holds the \
     text of everything the person dictated, including dictations kept only for private \
     chats, and reading or writing it as a file bypasses every check that decides which \
     dictations a chat may see. The refusal is about the channel, not about your \
     capability: it applies to a private-capability session exactly as it does to a public \
     one. The person views their dictation history on the Dictionary page in Biorouter, or \
     with `biorouter dictation history` in their own terminal. This project's own files are \
     not affected."
        .to_string()
}

/// The refusal for a writing tool at a door that cannot tell which chat calls it.
fn writing_tool_refusal(tool_name: &str) -> String {
    format!(
        "Refused: `{tool_name}` records terms with the privacy tier of the chat that calls \
         it, and this call did not come from a chat's own turn (it came from a script inside \
         execute_code, or from a direct tool call), so that tier cannot be known. Call it \
         from the chat itself. A person adds terms on the Dictionary page in Biorouter or \
         with `biorouter dictionary add` in their own terminal."
    )
}

fn is_writing_tool(tool_name: &str) -> bool {
    tool_name == ADD_TERMS_TOOL || tool_name == REMOVE_ADDED_TERMS_TOOL
}

/// Classify a tool call against the store's path and the dictation history's. `None` for
/// every call that names neither.
pub fn dictionary_store_gate(args: &Map<String, Value>) -> Option<String> {
    referenced_folder(args).map(|folder| match folder {
        ProtectedFolder::Dictionary => path_refusal(),
        ProtectedFolder::History => history_path_refusal(),
    })
}

/// The refusal a dispatch door that no [`ToolInspector`] reaches owes (DD-42):
/// `dictionary__add_terms` and `dictionary__remove_added_terms` whatever their
/// arguments, and any call whose already evaluated arguments name the store's folder or
/// the dictation history's.
/// `boundary` is logged, never read by the decision.
pub fn uninspected_boundary_refusal(
    tool_name: &str,
    args: Option<&Map<String, Value>>,
    boundary: crate::security::UninspectedBoundary,
) -> Option<String> {
    let reason = if is_writing_tool(tool_name) {
        writing_tool_refusal(tool_name)
    } else {
        dictionary_store_gate(args?)?
    };
    tracing::warn!(
        counter.biorouter.dictionary_store_uninspected_refused = 1,
        tool_name = %tool_name,
        boundary = ?boundary,
        "Refused a dictionary write or a store path at a boundary no inspector sees"
    );
    Some(reason)
}

/// Refuses tool reads and writes of the store's folder and the dictation history's, in
/// every mode that runs tools, for both tiers.
pub struct DictionaryStoreInspector;

#[async_trait]
impl ToolInspector for DictionaryStoreInspector {
    fn name(&self) -> &'static str {
        DICTIONARY_STORE_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        biorouter_mode: BioRouterMode,
        // Deliberately unread: the store is refused by channel, not by tier.
        _session: &crate::session::Session,
    ) -> Result<Vec<InspectionResult>> {
        if biorouter_mode == BioRouterMode::Chat {
            return Ok(vec![]);
        }
        let mut results = Vec::new();
        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            let Some(args) = tool_call.arguments.as_ref() else {
                continue;
            };
            let Some(reason) = dictionary_store_gate(args) else {
                continue;
            };
            tracing::warn!(
                counter.biorouter.dictionary_store_read_refused = 1,
                tool_name = %tool_call.name,
                tool_request_id = %request.id,
                "Refused a tool that named the dictionary store by path"
            );
            results.push(InspectionResult {
                tool_request_id: request.id.clone(),
                action: InspectionAction::Deny,
                reason,
                confidence: 1.0,
                inspector_name: self.name().to_string(),
                finding_id: Some(format!("DCT-{}", Uuid::new_v4().simple())),
            });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::SessionClassification;
    use crate::security::UninspectedBoundary;
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    /// The store's files, built apart from the production helper.
    fn store_files() -> Vec<String> {
        let dir = Paths::config_dir().join("dictionary");
        ["dictionary.json", "usage.json", ".lock"]
            .iter()
            .map(|f| dir.join(f).to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_folder_is_the_config_folders_dictionary() {
        assert_eq!(dictionary_dir(), Paths::config_dir().join("dictionary"));
        assert_eq!(open_store().dir(), dictionary_dir());
        assert!(is_in_dictionary_store(
            &dictionary_dir().join("dictionary.json")
        ));
        assert!(is_in_dictionary_store(&dictionary_dir()));
        assert!(!is_in_dictionary_store(
            &Paths::config_dir().join("dictionary-notes.md")
        ));
        assert!(!is_in_dictionary_store(
            &Paths::config_dir().join("config.yaml")
        ));
    }

    /// PR-13's acceptance: `cat` and `text_editor view` of the store are refused, and so
    /// are the other ways a model reaches or writes a file.
    #[test]
    fn every_read_and_write_of_the_store_is_refused() {
        let dir = dictionary_dir().to_string_lossy().into_owned();
        for file in store_files() {
            for (label, arguments) in [
                ("cat", json!({ "command": format!("cat {file}") })),
                ("view", json!({ "command": "view", "path": file.clone() })),
                (
                    "write",
                    json!({ "command": "write", "path": file.clone(), "file_text": "{}" }),
                ),
                ("quoted", json!({ "command": format!("cat \"{file}\"") })),
                (
                    "copy out",
                    json!({ "command": format!("cp {file} /tmp/x") }),
                ),
                ("redirect", json!({ "command": format!("echo x > {file}") })),
                ("nested", json!({ "opts": { "paths": [file.clone()] } })),
                (
                    "script",
                    json!({ "code": format!("shell({{command: 'cat {file}'}})") }),
                ),
            ] {
                let refusal = dictionary_store_gate(&args(arguments.clone()));
                assert!(refusal.is_some(), "{label} reached the store: {arguments}");
                assert!(refusal.unwrap().contains("Dictionary page"));
            }
        }
        for arguments in [
            json!({ "command": format!("ls -la {dir}") }),
            json!({ "command": format!("cd {dir} && cat dictionary.json") }),
            json!({ "working_directory": dir.clone() }),
        ] {
            assert!(
                dictionary_store_gate(&args(arguments.clone())).is_some(),
                "the folder is the store: {arguments}"
            );
        }
    }

    #[test]
    fn mentions_and_lookalikes_are_not_refused() {
        let config = Paths::config_dir().to_string_lossy().into_owned();
        let file = store_files().remove(0);
        for arguments in [
            json!({ "command": format!("cat {config}/dictionary-notes/terms.txt") }),
            json!({ "command": format!("cat {config}/config.yaml") }),
            json!({ "command": "cat ./dictionary/dictionary.json" }),
            json!({ "command": "view", "path": "docs/dictation/dictionary.md" }),
            json!({
                "command": "write",
                "path": "docs/notes.md",
                "file_text": format!("The dictionary lives in {file}.\n"),
            }),
            json!({ "command": format!("git commit -m \"docs: the store is {file}\"") }),
            json!({ "command": "biorouter dictionary list" }),
        ] {
            assert!(
                dictionary_store_gate(&args(arguments.clone())).is_none(),
                "{arguments} is not the store and must not be refused"
            );
        }
    }

    /// DD-42: the writing tools are refused at both doors whatever their arguments; the
    /// listing tool is not; the path is refused there too, and nothing else is.
    #[test]
    fn both_uninspected_doors_refuse_the_writing_tools_and_the_path() {
        for boundary in [
            UninspectedBoundary::ExecuteCodeScript,
            UninspectedBoundary::AgentCallToolRoute,
        ] {
            for tool in [ADD_TERMS_TOOL, REMOVE_ADDED_TERMS_TOOL] {
                for arguments in [
                    None,
                    Some(args(json!({ "terms": [{ "written": "EPAS1" }] }))),
                ] {
                    let refusal = uninspected_boundary_refusal(tool, arguments.as_ref(), boundary)
                        .unwrap_or_else(|| panic!("{tool} ran at {boundary:?}"));
                    assert!(refusal.contains(tool) && refusal.contains("biorouter dictionary add"));
                }
            }
            assert_eq!(
                uninspected_boundary_refusal(
                    "dictionary__list_terms",
                    Some(&args(json!({ "query": "EPAS" }))),
                    boundary
                ),
                None,
                "listing stays callable from scripts"
            );
            let file = store_files().remove(0);
            assert_eq!(
                uninspected_boundary_refusal(
                    "developer__shell",
                    Some(&args(json!({ "command": format!("cat {file}") }))),
                    boundary
                ),
                Some(path_refusal())
            );
            assert_eq!(
                uninspected_boundary_refusal(
                    "developer__shell",
                    Some(&args(json!({ "command": "ls -la /tmp" }))),
                    boundary
                ),
                None
            );
            assert_eq!(
                uninspected_boundary_refusal("developer__shell", None, boundary),
                None
            );
        }
    }

    /// Every door consults this module: the agent loop registers the inspector, and the
    /// route's chain and the script door's chain each call
    /// [`uninspected_boundary_refusal`]. A unit test of the decision cannot say that the
    /// doors ask it, so the three places are read here.
    #[test]
    fn every_door_asks_this_module() {
        let agent = include_str!("../agents/agent.rs");
        let registry = agent
            .split("fn create_tool_inspection_manager(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n").next())
            .expect("the inspector registry is in agent.rs");
        assert!(
            registry.contains("dictionary_store::DictionaryStoreInspector"),
            "the agent loop no longer registers the dictionary store inspector"
        );
        let script = include_str!("../agents/code_execution_extension.rs");
        let route = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../biorouter-server/src/routes/agent.rs"),
        )
        .expect("the server's agent routes are readable");
        // A body runs from its signature to the closing brace at its own indent.
        let body_of = |source: &str, signature: &str, closing: &str| -> String {
            let start = source
                .find(signature)
                .unwrap_or_else(|| panic!("{signature} is gone"));
            let rest = source.get(start..).unwrap_or_default();
            let end = rest.find(closing).unwrap_or(rest.len());
            rest.get(..end).unwrap_or_default().to_string()
        };
        let needle = "dictionary_store::uninspected_boundary_refusal(";
        assert!(
            body_of(
                script,
                "async fn uninspected_boundary_refusal(",
                "\n    }\n"
            )
            .contains(needle),
            "the execute_code door no longer asks the dictionary store"
        );
        assert!(
            body_of(&route, "fn call_tool_boundary_refusal_under(", "\n}\n").contains(needle),
            "POST /agent/call_tool no longer asks the dictionary store"
        );
    }

    fn request(id: &str, name: &str, arguments: Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: name.to_string().into(),
                arguments: Some(args(arguments)),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    async fn inspect(
        mode: BioRouterMode,
        tier: SessionClassification,
        name: &str,
        arguments: Value,
    ) -> Vec<InspectionResult> {
        let session = crate::session::Session {
            privacy_tier: tier,
            ..Default::default()
        };
        DictionaryStoreInspector
            .inspect(&[request("r1", name, arguments)], &[], mode, &session)
            .await
            .unwrap()
    }

    /// PR-13's acceptance, through the inspector: `cat` and `text_editor view` of the
    /// store are refused for both tiers, in every mode that runs tools.
    #[tokio::test]
    async fn cat_and_view_are_refused_for_both_tiers() {
        let file = store_files().remove(0);
        for tier in [
            SessionClassification::Public,
            SessionClassification::Private,
        ] {
            for mode in [
                BioRouterMode::Auto,
                BioRouterMode::Approve,
                BioRouterMode::SmartApprove,
            ] {
                for (name, arguments) in [
                    (
                        "developer__shell",
                        json!({ "command": format!("cat {file}") }),
                    ),
                    (
                        "developer__text_editor",
                        json!({ "command": "view", "path": file.clone() }),
                    ),
                ] {
                    let results = inspect(mode, tier, name, arguments).await;
                    let refusal = results
                        .first()
                        .unwrap_or_else(|| panic!("{name} in {mode:?} for {tier:?} ran"));
                    assert_eq!(refusal.action, InspectionAction::Deny);
                    assert_eq!(refusal.inspector_name, DICTIONARY_STORE_INSPECTOR_NAME);
                    assert!(refusal
                        .finding_id
                        .as_deref()
                        .unwrap_or("")
                        .starts_with("DCT-"));
                }
            }
        }
        let chat = inspect(
            BioRouterMode::Chat,
            SessionClassification::Public,
            "developer__shell",
            json!({ "command": format!("cat {file}") }),
        )
        .await;
        assert!(chat.is_empty(), "Chat dispatches no tools");
        let ordinary = inspect(
            BioRouterMode::Auto,
            SessionClassification::Public,
            "developer__shell",
            json!({ "command": "ls -la /tmp" }),
        )
        .await;
        assert!(ordinary.is_empty());
    }

    /// The dictation history's files, built apart from the production helper.
    fn history_files() -> Vec<String> {
        let dir = Paths::local_data_dir().join("dictation").join("history");
        ["history.jsonl", "settings.json", ".lock"]
            .iter()
            .map(|f| dir.join(f).to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_history_folder_is_the_local_data_folders() {
        let dir = Paths::local_data_dir().join("dictation").join("history");
        assert!(is_in_dictation_history(&dir.join("history.jsonl")));
        assert!(is_in_dictation_history(&dir));
        assert!(!is_in_dictation_history(
            &Paths::local_data_dir().join("dictation").join("models")
        ));
        assert!(!is_in_dictation_history(&dictionary_dir()));
        assert!(!is_in_dictionary_store(&dir));
    }

    /// P-18: every read and write of the history's files is refused, with a refusal that
    /// names the dictation history, and the speech models beside it are not.
    #[test]
    fn every_read_and_write_of_the_history_is_refused() {
        let dir = Paths::local_data_dir()
            .join("dictation")
            .join("history")
            .to_string_lossy()
            .into_owned();
        for file in history_files() {
            for (label, arguments) in [
                ("cat", json!({ "command": format!("cat {file}") })),
                ("view", json!({ "command": "view", "path": file.clone() })),
                (
                    "write",
                    json!({ "command": "write", "path": file.clone(), "file_text": "{}" }),
                ),
                ("quoted", json!({ "command": format!("cat \"{file}\"") })),
                ("grep", json!({ "command": format!("grep EPAS1 {file}") })),
                (
                    "redirect",
                    json!({ "command": format!("echo x >> {file}") }),
                ),
                ("nested", json!({ "opts": { "paths": [file.clone()] } })),
                (
                    "script",
                    json!({ "code": format!("shell({{command: 'cat {file}'}})") }),
                ),
            ] {
                let refusal = dictionary_store_gate(&args(arguments.clone()))
                    .unwrap_or_else(|| panic!("{label} reached the history: {arguments}"));
                assert!(refusal.contains("dictation history"), "{refusal}");
                assert!(refusal.contains("Dictionary page"), "{refusal}");
            }
        }
        for arguments in [
            json!({ "command": format!("ls -la {dir}") }),
            json!({ "working_directory": dir.clone() }),
        ] {
            assert!(
                dictionary_store_gate(&args(arguments.clone())).is_some(),
                "the folder is the history: {arguments}"
            );
        }
        let data = Paths::local_data_dir().to_string_lossy().into_owned();
        for arguments in [
            json!({ "command": format!("ls {data}/dictation/models") }),
            json!({ "command": format!("cat {data}/dictation/history-notes/a.txt") }),
            json!({ "command": "biorouter dictation history --limit 5" }),
            json!({
                "command": format!("git commit -m \"docs: the history is {dir}\"")
            }),
        ] {
            assert!(
                dictionary_store_gate(&args(arguments.clone())).is_none(),
                "{arguments} is not the history and must not be refused"
            );
        }
        // The dictionary's own refusal still names the dictionary.
        let refusal = dictionary_store_gate(&args(
            json!({ "command": format!("cat {}", store_files().remove(0)) }),
        ))
        .unwrap();
        assert!(refusal.contains("dictionary store"), "{refusal}");
    }

    /// The doors no inspector reaches refuse the history's path too.
    #[test]
    fn both_uninspected_doors_refuse_the_history_path() {
        let file = history_files().remove(0);
        for boundary in [
            UninspectedBoundary::ExecuteCodeScript,
            UninspectedBoundary::AgentCallToolRoute,
        ] {
            let refusal = uninspected_boundary_refusal(
                "developer__shell",
                Some(&args(json!({ "command": format!("cat {file}") }))),
                boundary,
            )
            .unwrap_or_else(|| panic!("the history was read at {boundary:?}"));
            assert_eq!(refusal, history_path_refusal());
        }
    }

    /// P-18, through the inspector: `cat` and `text_editor view` of the history are
    /// refused for both tiers, in every mode that runs tools.
    #[tokio::test]
    async fn cat_and_view_of_the_history_are_refused_for_both_tiers() {
        let file = history_files().remove(0);
        for tier in [
            SessionClassification::Public,
            SessionClassification::Private,
        ] {
            for mode in [
                BioRouterMode::Auto,
                BioRouterMode::Approve,
                BioRouterMode::SmartApprove,
            ] {
                for (name, arguments) in [
                    (
                        "developer__shell",
                        json!({ "command": format!("cat {file}") }),
                    ),
                    (
                        "developer__text_editor",
                        json!({ "command": "view", "path": file.clone() }),
                    ),
                ] {
                    let results = inspect(mode, tier, name, arguments).await;
                    let refusal = results
                        .first()
                        .unwrap_or_else(|| panic!("{name} in {mode:?} for {tier:?} ran"));
                    assert_eq!(refusal.action, InspectionAction::Deny);
                    assert_eq!(refusal.inspector_name, DICTIONARY_STORE_INSPECTOR_NAME);
                    assert!(refusal.reason.contains("dictation history"));
                }
            }
        }
    }

    /// One Windows account's folders, spelled as `Paths` and the environment spell them
    /// there, so the Windows spellings are checked on every platform and not only on a
    /// Windows runner. The history's folder keeps the `/` that
    /// `Paths::in_local_data_dir("dictation/history")` leaves before its last component.
    const WINDOWS_HOME: &str = r"C:\Users\Ada";
    const WINDOWS_HISTORY: &str =
        r"C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation/history";
    const WINDOWS_DICTIONARY: &str =
        r"C:\Users\Ada\AppData\Roaming\UCSF\biorouter\config\dictionary";

    fn windows_spelling() -> PathSpelling {
        PathSpelling {
            platform: Platform::Windows,
            anchors: vec![
                ("USERPROFILE", WINDOWS_HOME.to_string()),
                ("LOCALAPPDATA", format!(r"{WINDOWS_HOME}\AppData\Local")),
                ("APPDATA", format!(r"{WINDOWS_HOME}\AppData\Roaming")),
            ],
        }
    }

    /// The ways a model reaches a file at `path`, read as a Windows shell reads them.
    fn windows_reads_of(path: &str) -> Vec<(&'static str, Value)> {
        vec![
            ("cat", json!({ "command": format!("cat {path}") })),
            (
                "double quoted",
                json!({ "command": format!("Get-Content -Raw \"{path}\"") }),
            ),
            (
                "single quoted",
                json!({ "command": format!("type '{path}'") }),
            ),
            ("view", json!({ "command": "view", "path": path })),
            (
                "write",
                json!({ "command": "write", "path": path, "file_text": "{}" }),
            ),
            (
                "redirect",
                json!({ "command": format!("echo x >> {path}") }),
            ),
            ("nested", json!({ "opts": { "paths": [path] } })),
            (
                "script",
                json!({ "code": format!("shell({{command: 'cat {path}'}})") }),
            ),
        ]
    }

    /// P-18 on Windows. `test (windows-latest)` in run 45902896689 failed the three history
    /// tests above: the folder was spelled only as `...\data\dictation/history`, which no
    /// path written with `\` contains. Every spelling of the folder that reaches it there
    /// is refused: either separator, any letter case (the drive letter's included), a
    /// verbatim or device prefix, and the folder named through `%LOCALAPPDATA%`,
    /// `%USERPROFILE%` or the home folder in cmd's, PowerShell's and a POSIX shell's
    /// syntax.
    #[test]
    fn the_windows_spellings_of_the_history_are_refused() {
        let spelling = windows_spelling();
        // As `Paths` resolves it, and as `canonicalize` would print it.
        for dir in [
            WINDOWS_HISTORY.to_string(),
            format!(r"\\?\{}", WINDOWS_HISTORY.replace('/', r"\")),
        ] {
            the_windows_spellings_of_this_history_are_refused(&spelling, &dir);
        }
    }

    fn the_windows_spellings_of_this_history_are_refused(spelling: &PathSpelling, dir: &str) {
        let forms = spelling.forms(dir);
        for path in [
            r"C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation/history\settings.json",
            "C:/Users/Ada/AppData/Local/UCSF/biorouter/data/dictation/history/history.jsonl",
            r"c:\users\ada\appdata\local\ucsf\biorouter\data\dictation\history\history.jsonl",
            r"C:\USERS\ADA\APPDATA\LOCAL\UCSF\BIOROUTER\DATA\DICTATION\HISTORY\.LOCK",
            r"C:\\Users\\Ada\\AppData\\Local\\UCSF\\biorouter\\data\\dictation\\history\\history.jsonl",
            r"\\?\C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"\\.\C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"%LOCALAPPDATA%\UCSF\biorouter\data\dictation\history\history.jsonl",
            "%LocalAppData%/UCSF/biorouter/data/dictation/history/history.jsonl",
            r"$env:LOCALAPPDATA\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"${env:LOCALAPPDATA}\UCSF\biorouter\data\dictation\history\history.jsonl",
            "$LOCALAPPDATA/UCSF/biorouter/data/dictation/history/history.jsonl",
            "${LOCALAPPDATA}/UCSF/biorouter/data/dictation/history/history.jsonl",
            r"%USERPROFILE%\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"$env:USERPROFILE\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"~\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            "~/AppData/Local/UCSF/biorouter/data/dictation/history/history.jsonl",
            r"$HOME\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
            r"${HOME}\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl",
        ] {
            for (label, arguments) in windows_reads_of(path) {
                assert!(
                    references_folder(&args(arguments.clone()), &forms, spelling),
                    "{label} reached the history: {arguments}"
                );
            }
        }
        for arguments in [
            json!({
                "command": r"Get-ChildItem C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation\history"
            }),
            json!({
                "command": r"cd $env:LOCALAPPDATA\UCSF\biorouter\data\dictation\history && type history.jsonl"
            }),
            json!({
                "working_directory": "c:/users/ada/appdata/local/ucsf/biorouter/data/dictation/history"
            }),
        ] {
            assert!(
                references_folder(&args(arguments.clone()), &forms, spelling),
                "the folder is the history: {arguments}"
            );
        }
        for arguments in [
            json!({
                "command": r"Get-ChildItem C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation\models"
            }),
            json!({
                "command": r"type C:\Users\Ada\AppData\Local\UCSF\biorouter\data\dictation\history-notes\a.txt"
            }),
            json!({
                "command": r"type C:\Users\Bob\AppData\Local\UCSF\biorouter\data\dictation\history\history.jsonl"
            }),
            // The roaming data folder is another folder; the history never roams.
            json!({
                "command": r"type %APPDATA%\UCSF\biorouter\data\dictation\history\history.jsonl"
            }),
            json!({ "command": r"type .\dictation\history\history.jsonl" }),
            json!({ "command": format!("git commit -m \"docs: the history is {WINDOWS_HISTORY}\"") }),
            json!({ "command": "biorouter dictation history --limit 5" }),
            json!({ "command": "view", "path": r"docs\dictation\history.md" }),
        ] {
            assert!(
                !references_folder(&args(arguments.clone()), &forms, spelling),
                "{arguments} is not the history and must not be refused"
            );
        }
    }

    /// The dictionary store on Windows, under the roaming configuration folder, is refused
    /// in the same spellings, `%APPDATA%` among them.
    #[test]
    fn the_windows_spellings_of_the_dictionary_are_refused() {
        let spelling = windows_spelling();
        let forms = spelling.forms(WINDOWS_DICTIONARY);
        for path in [
            r"C:\Users\Ada\AppData\Roaming\UCSF\biorouter\config\dictionary\dictionary.json",
            "c:/users/ada/appdata/roaming/ucsf/biorouter/config/dictionary/usage.json",
            r"%APPDATA%\UCSF\biorouter\config\dictionary\dictionary.json",
            r"$env:APPDATA\UCSF\biorouter\config\dictionary\.lock",
            r"~\AppData\Roaming\UCSF\biorouter\config\dictionary\dictionary.json",
            r"\\?\C:\Users\Ada\AppData\Roaming\UCSF\biorouter\config\dictionary\dictionary.json",
        ] {
            for (label, arguments) in windows_reads_of(path) {
                assert!(
                    references_folder(&args(arguments.clone()), &forms, &spelling),
                    "{label} reached the store: {arguments}"
                );
            }
        }
        for arguments in [
            json!({ "command": r"type %APPDATA%\UCSF\biorouter\config\dictionary-notes\terms.txt" }),
            json!({ "command": r"type %APPDATA%\UCSF\biorouter\config\config.yaml" }),
            json!({ "command": r"type %LOCALAPPDATA%\UCSF\biorouter\config\dictionary\dictionary.json" }),
        ] {
            assert!(
                !references_folder(&args(arguments.clone()), &forms, &spelling),
                "{arguments} is not the store and must not be refused"
            );
        }
    }

    /// Off Windows a path is compared exactly as written, so the forms there are the four
    /// they always were, a trailing separator on the home folder changing nothing.
    #[test]
    fn the_unix_spellings_are_the_four_they_always_were() {
        for home in ["/home/ada", "/home/ada/"] {
            for platform in [Platform::Linux, Platform::Macos] {
                let spelling = PathSpelling {
                    platform,
                    anchors: vec![("HOME", home.to_string())],
                };
                assert_eq!(
                    spelling.forms("/home/ada/.local/share/biorouter/dictation/history"),
                    vec![
                        "/home/ada/.local/share/biorouter/dictation/history",
                        "~/.local/share/biorouter/dictation/history",
                        "$HOME/.local/share/biorouter/dictation/history",
                        "${HOME}/.local/share/biorouter/dictation/history",
                    ]
                );
                assert_eq!(
                    spelling.forms("/srv/biorouter/config/dictionary"),
                    vec!["/srv/biorouter/config/dictionary"]
                );
                assert_eq!(spelling.fold(r"/Home/Ada\x"), r"/Home/Ada\x");
            }
        }
    }

    /// The store keeps institution ids as the core normalizes them (`name_to_key`), so a
    /// row's ids and an `InstitutionId` built from the same text agree.
    #[test]
    fn stored_institution_ids_normalize_like_institution_ids() {
        for raw in ["ucsf", " UCSF ", "U C S F", "Stanford", "MIT\t"] {
            assert_eq!(
                store::schema::normalize_institution_id(raw).as_deref(),
                Some(crate::privacy::affiliation::InstitutionId::new(raw).as_str()),
                "{raw:?}"
            );
        }
    }
}

#[cfg(test)]
mod issue543_redirect_consumer_tests {
    use super::*;

    #[test]
    fn issue543_store_redirects_cannot_hide_behind_operators_or_quotes() {
        let forms = vec!["/issue543/private-store".to_string()];
        for command in [
            "echo x >/issue543/private-store",
            "echo x >|/issue543/private-store",
            "echo x >/issue543/private-store|head",
            "echo x >/issue543/private-store&& echo ok",
            r#"echo x >"/issue543/private-store""#,
            r#"echo x >'/issue543/private-store'"#,
            "echo x >>/issue543/private-store/child",
        ] {
            assert!(
                shell_command_names_store(command, &forms, &PathSpelling::host()),
                "protected store redirect was lost: {command}"
            );
        }
        for command in [
            "ls 2>/dev/null)",
            "echo x >/tmp/issue543-ordinary|head",
            "printf 'documentation mentions /issue543/private-store'",
        ] {
            assert!(
                !shell_command_names_store(command, &forms, &PathSpelling::host()),
                "unrelated command was misclassified: {command}"
            );
        }
    }
}
