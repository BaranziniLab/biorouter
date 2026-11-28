use crate::config::paths::Paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use utoipa::ToSchema;

const PERMISSION_FILE: &str = "permission.yaml";

static PERMISSION_MANAGER: LazyLock<Arc<PermissionManager>> =
    LazyLock::new(|| Arc::new(PermissionManager::new(Paths::config_dir())));

/// Tools a chat must always put to a person before they run, in every
/// permission mode. An application names them in its manifest's
/// `guardrails.needs_approval`, and the app's socket registers them for the
/// app's chat each time a page connects ([`set_session_always_confirm`]).
///
/// Kept in memory and keyed by the chat's id AND its creation time: a chat id
/// is reissued after the day's newest chat is deleted, and a new chat under a
/// reused id must not inherit an app's approvals. A daemon restart forgets the
/// set, and the next page that connects registers it again before any turn
/// runs.
static SESSION_ALWAYS_CONFIRM: LazyLock<RwLock<HashMap<String, SessionAlwaysConfirm>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Clone)]
struct SessionAlwaysConfirm {
    created_at: chrono::DateTime<chrono::Utc>,
    tools: Vec<String>,
}

/// Register the tools `session` must always confirm with a person, replacing
/// any earlier registration. Names are full tool names (`developer__shell`);
/// blank entries are dropped, and an empty list removes the registration.
pub fn set_session_always_confirm(session: &crate::session::Session, tools: &[String]) {
    let mut tools: Vec<String> = tools
        .iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect();
    tools.sort();
    tools.dedup();
    let mut map = SESSION_ALWAYS_CONFIRM
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if tools.is_empty() {
        map.remove(&session.id);
    } else {
        map.insert(
            session.id.clone(),
            SessionAlwaysConfirm {
                created_at: session.created_at,
                tools,
            },
        );
    }
}

/// Whether `tool_name` must be put to a person before it runs in `session`,
/// whatever the permission mode ([`set_session_always_confirm`]).
///
/// Names are compared by their current spelling
/// ([`crate::config::extension_renames::current_tool_name`]), so an
/// application whose manifest still names a tool by its extension's old id
/// keeps asking before the renamed tool runs.
pub fn session_always_confirms(session: &crate::session::Session, tool_name: &str) -> bool {
    use crate::config::extension_renames::current_tool_name;
    let map = SESSION_ALWAYS_CONFIRM
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let wanted = current_tool_name(tool_name);
    map.get(&session.id).is_some_and(|entry| {
        entry.created_at == session.created_at
            && entry.tools.iter().any(|t| current_tool_name(t) == wanted)
    })
}

/// How restrictive a level is, for choosing between two spellings of one tool.
fn restrictiveness(level: &PermissionLevel) -> u8 {
    match level {
        PermissionLevel::AlwaysAllow => 0,
        PermissionLevel::AskBefore => 1,
        PermissionLevel::NeverAllow => 2,
    }
}

/// Enum representing the possible permission levels for a tool.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    AlwaysAllow, // Tool can always be used without prompt
    AskBefore,   // Tool requires permission to be granted before use
    NeverAllow,  // Tool is never allowed to be used
}

/// Struct representing the configuration of permissions, categorized by level.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct PermissionConfig {
    pub always_allow: Vec<String>, // List of tools that are always allowed
    pub ask_before: Vec<String>,   // List of tools that require user consent
    pub never_allow: Vec<String>,  // List of tools that are never allowed
}

/// PermissionManager manages permission configurations for various tools.
#[derive(Debug)]
pub struct PermissionManager {
    config_path: PathBuf,
    permission_map: RwLock<HashMap<String, PermissionConfig>>,
}

// Constants representing specific permission categories
const USER_PERMISSION: &str = "user";
const SMART_APPROVE_PERMISSION: &str = "smart_approve";

impl PermissionManager {
    pub fn new(config_dir: PathBuf) -> Self {
        let permission_path = config_dir.join(PERMISSION_FILE);
        let permission_map = if permission_path.exists() {
            let file_contents = fs::read_to_string(&permission_path).unwrap_or_else(|e| {
                tracing::warn!("Failed to read permission.yaml: {}", e);
                String::new()
            });
            serde_yaml::from_str(&file_contents).unwrap_or_else(|_| HashMap::new())
        } else {
            // Consolidate directory creation for re-use in global singleton or ACP.
            if let Err(e) = fs::create_dir_all(&config_dir) {
                tracing::warn!("Failed to create config directory: {}", e);
            }
            HashMap::new()
        };
        PermissionManager {
            config_path: permission_path,
            permission_map: RwLock::new(permission_map),
        }
    }

    pub fn instance() -> Arc<PermissionManager> {
        Arc::clone(&PERMISSION_MANAGER)
    }

    /// Returns a list of all the names (keys) in the permission map.
    pub fn get_permission_names(&self) -> Vec<String> {
        self.permission_map
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// Retrieves the user permission level for a specific tool.
    pub fn get_user_permission(&self, principal_name: &str) -> Option<PermissionLevel> {
        self.get_permission(USER_PERMISSION, principal_name)
    }

    /// Retrieves restrictive legacy smart-approve preferences. Automatic
    /// tool-wide approvals cannot authorize a new invocation.
    pub fn get_smart_approve_permission(&self, principal_name: &str) -> Option<PermissionLevel> {
        self.get_permission(SMART_APPROVE_PERMISSION, principal_name)
            .filter(|level| *level != PermissionLevel::AlwaysAllow)
    }

    /// Retrieves the config file path.
    pub fn get_config_path(&self) -> &Path {
        self.config_path.as_path()
    }

    /// Helper function to retrieve the permission level for a specific permission category and tool.
    ///
    /// A tool whose extension was renamed is judged under every spelling a
    /// stored choice may use ([`crate::config::extension_renames::tool_name_aliases`],
    /// the exact name first), so a choice written before the rename keeps
    /// applying after it. When two spellings disagree, the more restrictive one
    /// wins: that can only happen when an older build, which knows only the old
    /// name, wrote one of them, and a deny must never be lost to the rename
    /// (I4). A tool of an extension that was never renamed has one spelling and
    /// is judged exactly as before.
    fn get_permission(&self, name: &str, principal_name: &str) -> Option<PermissionLevel> {
        let map = self.permission_map.read().unwrap();
        // Check if the permission category exists in the map
        let permission_config = map.get(name)?;
        let level_of = |spelling: &str| {
            let listed = |list: &Vec<String>| list.iter().any(|p| p == spelling);
            // Check the permission levels for the given tool
            if listed(&permission_config.always_allow) {
                Some(PermissionLevel::AlwaysAllow)
            } else if listed(&permission_config.ask_before) {
                Some(PermissionLevel::AskBefore)
            } else if listed(&permission_config.never_allow) {
                Some(PermissionLevel::NeverAllow)
            } else {
                None
            }
        };
        let mut found: Option<PermissionLevel> = level_of(principal_name);
        for alias in crate::config::extension_renames::tool_name_aliases(principal_name) {
            if alias.as_ref() == principal_name {
                continue;
            }
            if let Some(level) = level_of(&alias) {
                if found
                    .as_ref()
                    .is_none_or(|held| restrictiveness(&level) > restrictiveness(held))
                {
                    found = Some(level);
                }
            }
        }
        found // None if no matching permission level is found
    }

    /// Updates the user permission level for a specific tool.
    pub fn update_user_permission(&self, principal_name: &str, level: PermissionLevel) {
        self.update_permission(USER_PERMISSION, principal_name, level)
    }

    /// Updates the smart approve permission level for a specific tool.
    pub fn update_smart_approve_permission(&self, principal_name: &str, level: PermissionLevel) {
        self.update_permission(SMART_APPROVE_PERMISSION, principal_name, level)
    }

    /// Helper function to update a permission level for a specific tool in a given permission category.
    ///
    /// The choice is stored under the tool's current name and replaces every
    /// spelling of the tool in every list, the old ones a rename left behind
    /// included, so the newest choice is the one that holds.
    fn update_permission(&self, name: &str, principal_name: &str, level: PermissionLevel) {
        let spellings: Vec<String> =
            crate::config::extension_renames::tool_name_aliases(principal_name)
                .into_iter()
                .map(|alias| alias.into_owned())
                .collect();
        let principal_name =
            crate::config::extension_renames::current_tool_name(principal_name).into_owned();
        let principal_name = principal_name.as_str();
        let mut map = self.permission_map.write().unwrap();
        // Get or create a new PermissionConfig for the specified category
        let permission_config = map.entry(name.to_string()).or_default();

        // Remove the principal, under every spelling, from all existing lists
        // to avoid duplicates
        let other_spelling = |p: &String| !spellings.iter().any(|spelling| spelling == p);
        permission_config.always_allow.retain(other_spelling);
        permission_config.ask_before.retain(other_spelling);
        permission_config.never_allow.retain(other_spelling);

        // Add the principal to the appropriate list
        match level {
            PermissionLevel::AlwaysAllow => permission_config
                .always_allow
                .push(principal_name.to_string()),
            PermissionLevel::AskBefore => permission_config
                .ask_before
                .push(principal_name.to_string()),
            PermissionLevel::NeverAllow => permission_config
                .never_allow
                .push(principal_name.to_string()),
        }

        // Serialize the updated permission map and write it back to the config file
        match serde_yaml::to_string(&*map) {
            Ok(yaml_content) => {
                if let Err(e) = fs::write(&self.config_path, yaml_content) {
                    tracing::warn!("Failed to write to permission.yaml: {}", e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize permission config: {}", e),
        }
    }

    /// Removes the choices for the given extension's tools (`<name>__…`, and an
    /// entry naming the extension itself), and those of every other id the
    /// extension has had ([`crate::config::extension_renames::all_keys`]) that
    /// is not configured now.
    pub fn remove_extension(&self, extension_name: &str) {
        let configured: Vec<String> = crate::config::extensions::get_all_extensions()
            .into_iter()
            .map(|entry| entry.config.key())
            .collect();
        self.remove_extension_choices(extension_name, &configured);
    }

    /// [`Self::remove_extension`], with the configured keys given.
    ///
    /// ⚠ **The full `<id>__` prefix, never the bare name.** A bare
    /// `starts_with("gptr")` also matched `gptrtools__x`, so removing one
    /// extension dropped another's choices, its denies included. And an id
    /// still configured keeps its choices: both ids installed is a conflict the
    /// rename move leaves alone, with both entries loaded.
    pub(crate) fn remove_extension_choices(
        &self,
        extension_name: &str,
        still_configured: &[String],
    ) {
        let prefixes: Vec<String> = crate::config::extension_renames::all_keys(extension_name)
            .into_iter()
            .chain(std::iter::once(extension_name))
            .filter(|key| *key == extension_name || !still_configured.iter().any(|c| c == key))
            .map(|key| format!("{key}__"))
            .collect();
        let kept = |p: &String| {
            p != extension_name && !prefixes.iter().any(|prefix| p.starts_with(prefix.as_str()))
        };
        let mut map = self.permission_map.write().unwrap();
        for permission_config in map.values_mut() {
            permission_config.always_allow.retain(kept);
            permission_config.ask_before.retain(kept);
            permission_config.never_allow.retain(kept);
        }

        match serde_yaml::to_string(&*map) {
            Ok(yaml_content) => {
                if let Err(e) = fs::write(&self.config_path, yaml_content) {
                    tracing::warn!("Failed to write to permission.yaml: {}", e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize permission config: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper function to create a test instance of PermissionManager with a temp dir
    fn create_test_permission_manager() -> (PermissionManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let manager = PermissionManager::new(temp_dir.path().to_path_buf());
        (manager, temp_dir)
    }

    #[test]
    fn test_get_permission_names_empty() {
        let (manager, _temp_dir) = create_test_permission_manager();

        assert!(manager.get_permission_names().is_empty());
    }

    #[test]
    fn test_update_user_permission() {
        let (manager, _temp_dir) = create_test_permission_manager();
        manager.update_user_permission("tool1", PermissionLevel::AlwaysAllow);

        let permission = manager.get_user_permission("tool1");
        assert_eq!(permission, Some(PermissionLevel::AlwaysAllow));
    }

    #[test]
    fn test_update_smart_approve_permission() {
        let (manager, _temp_dir) = create_test_permission_manager();
        manager.update_smart_approve_permission("tool2", PermissionLevel::AskBefore);

        let permission = manager.get_smart_approve_permission("tool2");
        assert_eq!(permission, Some(PermissionLevel::AskBefore));
    }

    #[test]
    fn legacy_smart_allow_is_ignored_after_loading_permissions() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(PERMISSION_FILE),
            r#"
smart_approve:
  always_allow: [legacy_tool]
  ask_before: [ask_tool]
  never_allow: [deny_tool]
user:
  always_allow: [explicit_tool]
  ask_before: []
  never_allow: []
"#,
        )
        .unwrap();
        let manager = PermissionManager::new(temp.path().to_owned());

        assert_eq!(manager.get_smart_approve_permission("legacy_tool"), None);
        assert_eq!(
            manager.get_smart_approve_permission("ask_tool"),
            Some(PermissionLevel::AskBefore)
        );
        assert_eq!(
            manager.get_smart_approve_permission("deny_tool"),
            Some(PermissionLevel::NeverAllow)
        );
        assert_eq!(
            manager.get_user_permission("explicit_tool"),
            Some(PermissionLevel::AlwaysAllow)
        );
    }

    #[test]
    fn test_get_permission_not_found() {
        let (manager, _temp_dir) = create_test_permission_manager();

        let permission = manager.get_user_permission("non_existent_tool");
        assert_eq!(permission, None);
    }

    #[test]
    fn test_permission_levels() {
        let (manager, _temp_dir) = create_test_permission_manager();

        manager.update_user_permission("tool4", PermissionLevel::AlwaysAllow);
        manager.update_user_permission("tool5", PermissionLevel::AskBefore);
        manager.update_user_permission("tool6", PermissionLevel::NeverAllow);

        // Check the permission levels
        assert_eq!(
            manager.get_user_permission("tool4"),
            Some(PermissionLevel::AlwaysAllow)
        );
        assert_eq!(
            manager.get_user_permission("tool5"),
            Some(PermissionLevel::AskBefore)
        );
        assert_eq!(
            manager.get_user_permission("tool6"),
            Some(PermissionLevel::NeverAllow)
        );
    }

    #[test]
    fn test_permission_update_replaces_existing_level() {
        let (manager, _temp_dir) = create_test_permission_manager();

        // Initially AlwaysAllow
        manager.update_user_permission("tool7", PermissionLevel::AlwaysAllow);
        assert_eq!(
            manager.get_user_permission("tool7"),
            Some(PermissionLevel::AlwaysAllow)
        );

        // Now change to NeverAllow
        manager.update_user_permission("tool7", PermissionLevel::NeverAllow);
        assert_eq!(
            manager.get_user_permission("tool7"),
            Some(PermissionLevel::NeverAllow)
        );

        // Ensure it's removed from other levels
        let map = manager.permission_map.read().unwrap();
        let config = map.get(USER_PERMISSION).unwrap();
        assert!(!config.always_allow.contains(&"tool7".to_string()));
        assert!(!config.ask_before.contains(&"tool7".to_string()));
        assert!(config.never_allow.contains(&"tool7".to_string()));
    }

    fn chat(id: &str) -> crate::session::Session {
        crate::session::Session {
            id: id.to_string(),
            ..Default::default()
        }
    }

    /// An app's `guardrails.needs_approval` is registered for the app's own
    /// chat and nowhere else; an empty list removes it.
    #[test]
    fn a_session_always_confirm_set_is_kept_per_chat() {
        let app_chat = chat("perm-always-confirm-app");
        let other = chat("perm-always-confirm-other");
        set_session_always_confirm(
            &app_chat,
            &[" developer__shell ".to_string(), String::new()],
        );
        assert!(session_always_confirms(&app_chat, "developer__shell"));
        assert!(!session_always_confirms(
            &app_chat,
            "developer__text_editor"
        ));
        assert!(!session_always_confirms(&other, "developer__shell"));

        set_session_always_confirm(&app_chat, &[]);
        assert!(!session_always_confirms(&app_chat, "developer__shell"));
    }

    /// A chat that reuses a deleted chat's id is a different chat: it was
    /// created at another time, so it inherits none of the old app's asks.
    #[test]
    fn a_reissued_chat_id_inherits_no_always_confirm_set() {
        let first = chat("perm-always-confirm-reissued");
        set_session_always_confirm(&first, &["developer__shell".to_string()]);
        let reissued = crate::session::Session {
            created_at: first.created_at + chrono::Duration::seconds(5),
            ..first.clone()
        };
        assert!(session_always_confirms(&first, "developer__shell"));
        assert!(!session_always_confirms(&reissued, "developer__shell"));
        set_session_always_confirm(&first, &[]);
    }

    /// I4 of the gptr rename: a choice a person wrote for a tool before its
    /// extension was renamed keeps judging the tool under its new name, in both
    /// categories, and a deny is among them.
    #[test]
    fn a_choice_written_for_an_old_tool_name_judges_the_new_name() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(PERMISSION_FILE),
            r#"
user:
  always_allow: [gptragent__r]
  ask_before: [gptragent__inspect]
  never_allow: [gptragent__sessions]
smart_approve:
  always_allow: []
  ask_before: []
  never_allow: [gptragent__inspect]
"#,
        )
        .unwrap();
        let manager = PermissionManager::new(temp.path().to_owned());

        assert_eq!(
            manager.get_user_permission("gptr__r"),
            Some(PermissionLevel::AlwaysAllow)
        );
        assert_eq!(
            manager.get_user_permission("gptr__inspect"),
            Some(PermissionLevel::AskBefore)
        );
        assert_eq!(
            manager.get_user_permission("gptr__sessions"),
            Some(PermissionLevel::NeverAllow)
        );
        assert_eq!(
            manager.get_smart_approve_permission("gptr__inspect"),
            Some(PermissionLevel::NeverAllow)
        );
        // A tool of an extension that was never renamed is judged exactly as before.
        assert_eq!(manager.get_user_permission("developer__shell"), None);
    }

    /// A deny under either spelling is never shadowed by an allow under the
    /// other. Two spellings of one tool can only both be present when an older
    /// build (which knows only the old name) wrote one of them, and the
    /// restrictive answer is the one that cannot be wrong.
    #[test]
    fn a_deny_under_either_spelling_of_a_tool_is_never_shadowed() {
        for (allowed, denied) in [("gptr__r", "gptragent__r"), ("gptragent__r", "gptr__r")] {
            let temp = TempDir::new().unwrap();
            fs::write(
                temp.path().join(PERMISSION_FILE),
                format!(
                    "user:\n  always_allow: [{allowed}]\n  ask_before: []\n  never_allow: [{denied}]\n"
                ),
            )
            .unwrap();
            let manager = PermissionManager::new(temp.path().to_owned());
            for name in ["gptr__r", "gptragent__r"] {
                assert_eq!(
                    manager.get_user_permission(name),
                    Some(PermissionLevel::NeverAllow),
                    "{name} with {allowed} allowed and {denied} denied"
                );
            }
        }
    }

    /// Writing a choice for a tool clears every other spelling of it, so the
    /// newest choice is the one that holds.
    #[test]
    fn writing_a_choice_clears_the_old_spellings_of_the_same_tool() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(PERMISSION_FILE),
            "user:\n  always_allow: []\n  ask_before: []\n  never_allow: [gptragent__r]\n",
        )
        .unwrap();
        let manager = PermissionManager::new(temp.path().to_owned());
        manager.update_user_permission("gptr__r", PermissionLevel::AlwaysAllow);
        assert_eq!(
            manager.get_user_permission("gptr__r"),
            Some(PermissionLevel::AlwaysAllow)
        );
        let map = manager.permission_map.read().unwrap();
        let config = map.get(USER_PERMISSION).unwrap();
        for list in [
            &config.always_allow,
            &config.ask_before,
            &config.never_allow,
        ] {
            assert!(!list.iter().any(|name| name == "gptragent__r"), "{list:?}");
        }
        assert_eq!(config.always_allow, vec!["gptr__r".to_string()]);
    }

    /// An application's `needs_approval` list written with an old tool name
    /// still makes the chat ask before the renamed tool runs, and the other way
    /// round.
    #[test]
    fn a_session_always_confirm_set_matches_either_spelling_of_a_tool() {
        let app_chat = chat("perm-always-confirm-renamed");
        set_session_always_confirm(&app_chat, &["gptragent__r".to_string()]);
        assert!(session_always_confirms(&app_chat, "gptr__r"));
        assert!(session_always_confirms(&app_chat, "gptragent__r"));
        assert!(!session_always_confirms(&app_chat, "gptr__inspect"));

        set_session_always_confirm(&app_chat, &["gptr__r".to_string()]);
        assert!(session_always_confirms(&app_chat, "gptragent__r"));
        set_session_always_confirm(&app_chat, &[]);
    }

    /// Removing an extension's choices removes those written under any of its
    /// ids.
    #[test]
    fn removing_an_extension_removes_the_choices_of_each_of_its_ids() {
        let (manager, _temp_dir) = create_test_permission_manager();
        manager.update_user_permission("gptr__r", PermissionLevel::AlwaysAllow);
        manager.update_user_permission("developer__shell", PermissionLevel::AlwaysAllow);
        manager.remove_extension_choices("gptragent", &[]);
        assert_eq!(manager.get_user_permission("gptr__r"), None);
        assert_eq!(
            manager.get_user_permission("developer__shell"),
            Some(PermissionLevel::AlwaysAllow)
        );
    }

    /// A tool whose name merely begins with the removed extension's name is
    /// another extension's, and keeps its choices, a deny included.
    #[test]
    fn removing_an_extension_keeps_the_choices_of_one_whose_name_it_begins() {
        let (manager, _temp_dir) = create_test_permission_manager();
        manager.update_user_permission("gptr__r", PermissionLevel::AlwaysAllow);
        manager.update_user_permission("gptrtools__x", PermissionLevel::NeverAllow);
        manager.remove_extension_choices("gptr", &[]);
        assert_eq!(manager.get_user_permission("gptr__r"), None);
        assert_eq!(
            manager.get_user_permission("gptrtools__x"),
            Some(PermissionLevel::NeverAllow)
        );
    }

    /// Both ids configured (the conflict the rename move leaves alone):
    /// removing one keeps the other's choices.
    #[test]
    fn removing_one_of_two_configured_ids_keeps_the_other_s_choices() {
        let (manager, _temp_dir) = create_test_permission_manager();
        manager.update_user_permission("gptr__r", PermissionLevel::NeverAllow);
        // A choice a build before the rename wrote, under the old spelling.
        manager
            .permission_map
            .write()
            .unwrap()
            .entry(USER_PERMISSION.to_string())
            .or_default()
            .always_allow
            .push("gptragent__inspect".to_string());
        manager.remove_extension_choices("gptragent", &["gptr".to_owned()]);
        assert_eq!(
            manager.get_user_permission("gptr__r"),
            Some(PermissionLevel::NeverAllow)
        );
        let map = manager.permission_map.read().unwrap();
        let config = map.get(USER_PERMISSION).unwrap();
        assert!(!config
            .always_allow
            .contains(&"gptragent__inspect".to_string()));
    }

    #[test]
    fn test_remove_extension() {
        let (manager, _temp_dir) = create_test_permission_manager();
        manager.update_user_permission("prefix__tool1", PermissionLevel::AlwaysAllow);
        manager.update_user_permission("nonprefix__tool2", PermissionLevel::AlwaysAllow);
        manager.update_user_permission("prefix__tool3", PermissionLevel::AskBefore);

        // Remove entries starting with "prefix"
        manager.remove_extension_choices("prefix", &[]);

        let map = manager.permission_map.read().unwrap();
        let config = map.get(USER_PERMISSION).unwrap();

        // Verify entries with "prefix" are removed
        assert!(!config.always_allow.contains(&"prefix__tool1".to_string()));
        assert!(!config.ask_before.contains(&"prefix__tool3".to_string()));

        // Verify other entries remain
        assert!(config
            .always_allow
            .contains(&"nonprefix__tool2".to_string()));
    }
}
