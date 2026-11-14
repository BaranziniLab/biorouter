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

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::config::paths::Paths;
use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::security::policy::command::{redirect_targets, ParsedCommand};
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
    let dir = dictionary_dir();
    if path.starts_with(&dir) {
        return true;
    }
    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => false,
    }
}

/// Every spelling of the store folder a tool argument might carry: the absolute path,
/// and when the folder is under the home folder the `~`, `$HOME` and `${HOME}` forms.
fn store_path_forms() -> Vec<String> {
    let dir = dictionary_dir();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let mut forms = vec![dir.to_string_lossy().into_owned()];
    if let Some(home) = home.as_ref() {
        if let Ok(tail) = dir.strip_prefix(Path::new(home)) {
            let tail = tail.to_string_lossy();
            forms.push(format!("~/{tail}"));
            forms.push(format!("$HOME/{tail}"));
            forms.push(format!("${{HOME}}/{tail}"));
        }
    }
    forms
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
fn shell_command_names_store(command: &str, forms: &[String]) -> bool {
    let token_is_store = |token: &str| {
        forms.iter().any(|form| {
            token.strip_prefix(form.as_str()).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
            })
        })
    };
    let parsed = ParsedCommand::parse(command, Path::new("."));
    parsed
        .segments
        .iter()
        .flat_map(|segment| segment.argv.iter())
        .any(|token| token_is_store(token))
        || redirect_targets(command).iter().any(|t| token_is_store(t))
}

/// Does `args` name the store in an argument that would open it?
fn references_dictionary_store(args: &Map<String, Value>) -> bool {
    fn in_map(map: &Map<String, Value>, forms: &[String]) -> bool {
        map.iter().any(|(key, value)| match argument_role(key) {
            Some(role) => in_role(value, forms, role),
            None => descend(value, forms),
        })
    }

    fn descend(value: &Value, forms: &[String]) -> bool {
        match value {
            Value::Object(map) => in_map(map, forms),
            Value::Array(items) => items.iter().any(|item| descend(item, forms)),
            _ => false,
        }
    }

    fn in_role(value: &Value, forms: &[String], role: ArgumentRole) -> bool {
        match value {
            Value::String(text) => match role {
                ArgumentRole::Path | ArgumentRole::Script => {
                    forms.iter().any(|form| names_path(text, form))
                }
                ArgumentRole::ShellCommand => shell_command_names_store(text, forms),
            },
            Value::Array(items) => items.iter().any(|item| in_role(item, forms, role)),
            Value::Object(map) => map.values().any(|v| in_role(v, forms, role)),
            _ => false,
        }
    }

    in_map(args, &store_path_forms())
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

/// Classify a tool call against the store's path. `None` for every call that does not
/// name it.
pub fn dictionary_store_gate(args: &Map<String, Value>) -> Option<String> {
    references_dictionary_store(args).then(path_refusal)
}

/// The refusal a dispatch door that no [`ToolInspector`] reaches owes (DD-42):
/// `dictionary__add_terms` and `dictionary__remove_added_terms` whatever their
/// arguments, and any call whose already evaluated arguments name the store's folder.
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

/// Refuses tool reads and writes of the store's folder, in every mode that runs tools,
/// for both tiers.
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
