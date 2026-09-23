use super::base::Config;
use crate::agents::extension::PLATFORM_EXTENSIONS;
use crate::agents::ExtensionConfig;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use tracing::warn;
use utoipa::ToSchema;

pub const DEFAULT_EXTENSION: &str = "developer";
pub const DEFAULT_EXTENSION_TIMEOUT: u64 = 300;
pub const DEFAULT_EXTENSION_DESCRIPTION: &str = "";
pub const DEFAULT_DISPLAY_NAME: &str = "Developer";
const EXTENSIONS_CONFIG_KEY: &str = "extensions";
/// ⚠ The VALUE is already written into every existing user's config.yaml.
/// Rename the constant freely; renaming the string resets everyone to version 0
/// and re-runs the capability split against a tree that has already been split.
const COMPUTER_USE_MIGRATION_KEY: &str = "computer_use_capabilities_version";
/// v1 split the legacy Computer Controller capability in two. v2 rewrites the
/// stored display name to "Biorouter Copilot": `inject_builtin_extensions`
/// inserts only when the key is ABSENT, so for an upgrading user the old label
/// already sits in config.yaml and no default can reach it. The migration only
/// ever assigns, so re-running it is safe.
const COMPUTER_USE_MIGRATION_VERSION: u32 = 2;

/// The capability's display name is written by the version-gated migration, so a
/// rename that does not also move this version reaches fresh installs only: every
/// existing user keeps the old label in their `config.yaml`, and
/// `inject_builtin_extensions` will not overwrite it because the key is present.
///
/// ⚠ This sits at module scope ON PURPOSE. As a runtime `assert!` inside the test
/// module it enforced nothing (the comparison folds at compile time, and clippy
/// rejects it as a constant assertion), and even as a `const` block it would then
/// compile only under `--tests`. Here it fails an ordinary build.
const _: () = assert!(
    COMPUTER_USE_MIGRATION_VERSION >= 2,
    "the Biorouter Copilot label is written by the version-gated migration, so \
     bumping the label without bumping the version reaches fresh installs only"
);
const WEB_DOCUMENT_TOOLS: &[&str] = &["web_scrape", "xlsx_tool", "docx_tool", "pdf_tool", "cache"];
const RETIRED_BUILTIN_EXTENSIONS: &[&str] = &["tutorial"];

/// ⚠ `PartialEq` is not derive-everything hygiene: `remove_extension_if_matches`
/// already compares the two fields by hand, and `remove_extension`'s
/// post-approval re-validation compares whole entries — an approval is only
/// atomic against a tree that moved under it if "the same entry" has one
/// definition.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, ToSchema)]
pub struct ExtensionEntry {
    pub enabled: bool,
    #[serde(flatten)]
    pub config: ExtensionConfig,
}

pub fn name_to_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn get_extensions_map() -> IndexMap<String, ExtensionEntry> {
    let raw: Mapping = Config::global()
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_else(|err| {
            warn!(
                "Failed to load {}: {err}. Falling back to empty object.",
                EXTENSIONS_CONFIG_KEY
            );
            Default::default()
        });

    let mut extensions_map = IndexMap::with_capacity(raw.len());
    for (k, v) in raw {
        match (k, serde_yaml::from_value::<ExtensionEntry>(v)) {
            (serde_yaml::Value::String(_), Ok(entry)) if is_retired_builtin_extension(&entry) => {}
            (serde_yaml::Value::String(key), Ok(entry)) => {
                extensions_map.insert(key, entry);
            }
            (k, v) => {
                warn!(
                    key = ?k,
                    value = ?v,
                    "Skipping malformed extension config entry"
                );
            }
        }
    }

    let migrated = Config::global()
        .get_param::<u32>(COMPUTER_USE_MIGRATION_KEY)
        .unwrap_or_default();
    if migrated < COMPUTER_USE_MIGRATION_VERSION {
        let migration = Config::global().update_param::<IndexMap<String, ExtensionEntry>, _, _>(
            EXTENSIONS_CONFIG_KEY,
            |current| {
                migrate_computer_use_capabilities(current);
                current.clone()
            },
        );
        match migration {
            Ok(current) => {
                extensions_map = current;
                extensions_map.retain(|_, entry| !is_retired_builtin_extension(entry));
                if let Err(error) = Config::global()
                    .set_param(COMPUTER_USE_MIGRATION_KEY, COMPUTER_USE_MIGRATION_VERSION)
                {
                    warn!(%error, "Could not record Biorouter Copilot capability migration");
                }
            }
            Err(error) => {
                warn!(%error, "Could not persist Biorouter Copilot capability migration");
                migrate_computer_use_capabilities(&mut extensions_map);
            }
        }
    }
    // Also project the idempotent split on reads, including a concurrent pre-migration snapshot.
    migrate_computer_use_capabilities(&mut extensions_map);
    inject_builtin_extensions(&mut extensions_map);
    inject_platform_extensions(&mut extensions_map);
    extensions_map
}

fn builtin_entry(name: &str, display_name: &str, description: &str) -> ExtensionEntry {
    ExtensionEntry {
        enabled: true,
        config: ExtensionConfig::Builtin {
            name: name.to_owned(),
            display_name: Some(display_name.to_owned()),
            description: description.to_owned(),
            timeout: Some(DEFAULT_EXTENSION_TIMEOUT),
            bundled: Some(true),
            available_tools: Vec::new(),
        },
    }
}

fn migrate_computer_use_capabilities(extensions: &mut IndexMap<String, ExtensionEntry>) {
    let existing_web = extensions.contains_key("webdocuments")
        || extensions
            .values()
            .any(|entry| entry.config.key() == "webdocuments");
    let Some(computer) = extensions.values_mut().find(|entry| {
        matches!(&entry.config, ExtensionConfig::Builtin { name, .. } if name_to_key(name) == "computercontroller")
    }) else {
        return;
    };
    let ExtensionConfig::Builtin {
        display_name,
        available_tools,
        ..
    } = &mut computer.config
    else {
        return;
    };
    *display_name = Some("Biorouter Copilot".to_owned());
    if existing_web {
        return;
    }
    let mut web = builtin_entry(
        "webdocuments",
        "Web & Documents",
        "Read web pages and work with spreadsheets, documents, and PDFs.",
    );
    web.enabled = computer.enabled;
    if !available_tools.is_empty() {
        let moved: Vec<String> = available_tools
            .iter()
            .filter_map(|tool| {
                let bare = tool.strip_prefix("computercontroller__").unwrap_or(tool);
                WEB_DOCUMENT_TOOLS.contains(&bare).then(|| bare.to_owned())
            })
            .collect();
        if moved.is_empty() {
            // An empty allowlist means unrestricted; never turn an empty intersection into that.
            web.enabled = false;
        } else if let ExtensionConfig::Builtin {
            available_tools, ..
        } = &mut web.config
        {
            *available_tools = moved;
        }
        // Keep the original Biorouter Copilot allowlist. Retired names match no native tool,
        // so a legacy utility-only configuration cannot silently gain desktop control.
    }
    extensions.insert("webdocuments".to_owned(), web);
}

const BUILTIN_EXTENSION_METADATA: &[(&str, &str, &str)] = &[
    (
        "developer",
        "Developer",
        "Read, write and run code, and run shell commands.",
    ),
    (
        "computercontroller",
        "Biorouter Copilot",
        "View and control desktop apps for an approved task.",
    ),
    (
        "webdocuments",
        "Web & Documents",
        "Read web pages and work with spreadsheets, documents, and PDFs.",
    ),
    (
        "autovisualiser",
        "Auto Visualiser",
        "Interactive charts, diagrams, networks, maps and scientific plots, rendered inline.",
    ),
    (
        "memory",
        "Memory",
        "Teach Biorouter your preferences so it remembers them as you go.",
    ),
    (
        "knowledge",
        "Knowledge",
        "Personal knowledge bases stored as markdown with full history.",
    ),
    (
        "agent_drafter",
        "Agent Drafter",
        "Build interactive artifacts, static pages or apps with an embedded Biorouter agent.",
    ),
];

pub fn bundled_extension_display_name(name: &str) -> Option<&'static str> {
    match name {
        "Workspace" | "workspace" => return Some("Workspace Control"),
        "code_execution" => return Some("Code Execution"),
        "skills" => return Some("Skills"),
        "todo" => return Some("Todo"),
        _ => {}
    }
    BUILTIN_EXTENSION_METADATA
        .iter()
        .find_map(|(id, label, _)| (*id == name).then_some(*label))
}

fn inject_builtin_extensions(extensions: &mut IndexMap<String, ExtensionEntry>) {
    for &(name, label, description) in BUILTIN_EXTENSION_METADATA {
        if !extensions.contains_key(name)
            && !extensions.values().any(|entry| entry.config.key() == name)
        {
            extensions.insert(name.to_owned(), builtin_entry(name, label, description));
        }
    }
}

fn is_retired_builtin_extension(entry: &ExtensionEntry) -> bool {
    matches!(
        &entry.config,
        ExtensionConfig::Builtin { name, .. }
            if RETIRED_BUILTIN_EXTENSIONS.contains(&name_to_key(name).as_str())
    )
}

fn inject_platform_extensions(extensions: &mut IndexMap<String, ExtensionEntry>) {
    for (key, def) in PLATFORM_EXTENSIONS.iter() {
        let configured_platform = extensions
            .get(*key)
            .filter(|entry| platform_entry_matches_key(entry, key))
            .cloned()
            .or_else(|| {
                extensions
                    .values()
                    .find(|entry| platform_entry_matches_key(entry, key))
                    .cloned()
            });
        let colliding_keys = extensions
            .iter()
            .filter(|(stored_key, entry)| *stored_key == key || entry.config.key() == *key)
            .map(|(stored_key, _)| stored_key.clone())
            .collect::<Vec<_>>();
        let has_non_platform_collision = colliding_keys.iter().any(|stored_key| {
            extensions
                .get(stored_key)
                .is_some_and(|entry| !platform_entry_matches_key(entry, key))
        });
        for stored_key in colliding_keys {
            extensions.shift_remove(&stored_key);
        }

        if has_non_platform_collision {
            warn!(
                key,
                "Ignoring extension that occupies a reserved platform extension key"
            );
        }
        extensions.insert(
            key.to_string(),
            configured_platform.unwrap_or_else(|| ExtensionEntry {
                config: ExtensionConfig::Platform {
                    name: def.name.to_string(),
                    description: def.description.to_string(),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                },
                enabled: def.default_enabled,
            }),
        );
    }
}

fn platform_entry_matches_key(entry: &ExtensionEntry, key: &str) -> bool {
    matches!(
        &entry.config,
        ExtensionConfig::Platform { name, .. } if name_to_key(name) == key
    )
}

fn save_extensions_map(extensions: IndexMap<String, ExtensionEntry>) {
    let config = Config::global();
    if let Err(e) = config.set_param(EXTENSIONS_CONFIG_KEY, &extensions) {
        // TODO(jack) why is this just a debug statement?
        tracing::debug!("Failed to save extensions config: {}", e);
    }
}

fn retain_bundled_extensions(extensions: &mut IndexMap<String, ExtensionEntry>) -> usize {
    let before = extensions.len();
    extensions.retain(|_, entry| entry.config.is_bundled());
    before - extensions.len()
}

pub fn reset_to_bundled_extensions() -> anyhow::Result<usize> {
    let mut extensions = get_extensions_map();
    let removed = retain_bundled_extensions(&mut extensions);
    Config::global()
        .set_param(EXTENSIONS_CONFIG_KEY, &extensions)
        .map_err(anyhow::Error::from)?;
    Ok(removed)
}

pub fn get_extension_by_name(name: &str) -> Option<ExtensionConfig> {
    get_extension_entry_by_name(name).map(|entry| entry.config)
}

/// Names (`config.name()`) of every extension entry actually present in the
/// persisted config file — i.e. operator-authored entries, before
/// [`get_extensions_map`] injects absent platform extensions with their
/// defaults. Malformed entries are skipped, mirroring `get_extensions_map`.
///
/// This is the provenance signal for the `manage_extensions` enable gate and
/// the `search_available_extensions` labeling (#42): only an entry the
/// operator wrote with `enabled: false` counts as operator-disabled. An
/// injected default-off platform extension (e.g. `chatrecall`) is absent
/// here and stays agent-enableable.
pub fn persisted_extension_names() -> std::collections::HashSet<String> {
    let raw: Mapping = Config::global()
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_default();
    persisted_names_from_raw(&raw)
}

fn persisted_names_from_raw(raw: &Mapping) -> std::collections::HashSet<String> {
    raw.iter()
        .filter_map(
            |(k, v)| match (k, serde_yaml::from_value::<ExtensionEntry>(v.clone())) {
                (serde_yaml::Value::String(_), Ok(entry)) => Some(entry.config.name().to_string()),
                _ => None,
            },
        )
        .collect()
}

/// True when `name` has an entry the operator actually wrote into the config
/// file, as opposed to an injected platform default. See
/// [`persisted_extension_names`].
pub fn extension_entry_is_persisted(name: &str) -> bool {
    persisted_extension_names().contains(name)
}

/// Like [`get_extension_by_name`], but returns the whole config entry so
/// callers can see the operator's `enabled` flag, not just the config.
pub fn get_extension_entry_by_name(name: &str) -> Option<ExtensionEntry> {
    find_entry_by_name(&get_extensions_map(), name).cloned()
}

fn find_entry_by_name<'a>(
    extensions: &'a IndexMap<String, ExtensionEntry>,
    name: &str,
) -> Option<&'a ExtensionEntry> {
    extensions
        .values()
        .find(|entry| entry.config.name() == name)
}

/// Issue #112. The three functions below are the ONLY places this process
/// writes the extension map, so they are where a catalogue change is announced.
/// Every inventory in the app — Settings, the composer picker, the running
/// agent — invalidates from that announcement rather than from a reload key
/// somebody remembered to bump.
///
/// ⚠ A write made by *another* process (`biorouter extension install` in a
/// terminal, a hand-edited `config.yaml`) reaches none of these. That case is
/// covered by [`crate::catalog::spawn_config_watcher`], and it is the one the
/// bug report was actually about.
fn announce(
    reason: crate::catalog::CatalogChangeReason,
    change: crate::catalog::CatalogExtensionChange,
) {
    let events = crate::catalog::CatalogEvents::global();
    events.publish(reason, vec![change], Vec::new(), None);
    // Refresh the watcher's baseline in the same breath, or it would see this
    // write two seconds later and announce it a second time.
    events.sync_snapshot(&get_all_extensions());
}

/// A config entry reduced to a comparable string, so an identical rewrite can
/// be told from a real edit.
fn fingerprint(entry: &ExtensionEntry) -> String {
    serde_json::to_string(&entry.config).unwrap_or_else(|_| entry.config.name())
}

fn change_row(
    key: &str,
    entry: Option<&ExtensionEntry>,
    change: crate::catalog::CatalogEntryChange,
) -> crate::catalog::CatalogExtensionChange {
    crate::catalog::CatalogExtensionChange {
        key: key.to_string(),
        name: entry
            .map(|e| e.config.name())
            .unwrap_or_else(|| key.to_string()),
        display_name: None,
        change,
        config: entry.map(|e| e.config.clone()),
        enabled: entry.map(|e| e.enabled).unwrap_or(false),
        bundled_skill_ids: Vec::new(),
    }
}

/// Write an entry **without announcing it**, for a caller that will publish a
/// richer catalogue event itself.
///
/// The one such caller is the install transaction, which knows the bundle's
/// skills and so can fill in `bundled_skill_ids` — a fact this function cannot
/// see. Two events for one install would be harmless (consumers refetch) but
/// noisy, and the second would be the only complete one.
///
/// ⚠ **A caller that uses this and then fails to publish leaves every inventory
/// stale**, which is the exact bug #112 exists to fix. Use `set_extension`
/// unless you are the one publishing.
pub fn set_extension_silent(entry: ExtensionEntry) {
    let mut extensions = get_extensions_map();
    extensions.insert(entry.config.key(), entry);
    save_extensions_map(extensions);
    crate::catalog::CatalogEvents::global().sync_snapshot(&get_all_extensions());
}

pub fn set_extension(entry: ExtensionEntry) {
    use crate::catalog::{CatalogChangeReason, CatalogEntryChange};
    let mut extensions = get_extensions_map();
    let key = entry.config.key();
    let previous = extensions.get(&key).cloned();
    // ⚠ An identical rewrite is not a change. `syncBundledExtensions` and the
    // capability migrations both re-save entries at every startup, and
    // announcing those would have every client in the app refetch its whole
    // inventory on launch for nothing.
    if let Some(before) = &previous {
        if before.enabled == entry.enabled && fingerprint(before) == fingerprint(&entry) {
            extensions.insert(key, entry);
            save_extensions_map(extensions);
            return;
        }
    }
    let (reason, change) = match &previous {
        None => (CatalogChangeReason::Install, CatalogEntryChange::Added),
        Some(before) if before.enabled != entry.enabled => (
            if entry.enabled {
                CatalogChangeReason::Enable
            } else {
                CatalogChangeReason::Disable
            },
            if entry.enabled {
                CatalogEntryChange::Enabled
            } else {
                CatalogEntryChange::Disabled
            },
        ),
        Some(_) => (CatalogChangeReason::Update, CatalogEntryChange::Updated),
    };
    let row = change_row(&key, Some(&entry), change);
    extensions.insert(key, entry);
    save_extensions_map(extensions);
    announce(reason, row);
}

pub fn remove_extension(key: &str) {
    use crate::catalog::{CatalogChangeReason, CatalogEntryChange};
    let mut extensions = get_extensions_map();
    let Some(removed) = extensions.shift_remove(key) else {
        // Nothing was there. Announcing a removal would wake every client to
        // refetch an unchanged inventory.
        return;
    };
    let row = change_row(key, Some(&removed), CatalogEntryChange::Removed);
    save_extensions_map(extensions);
    announce(
        CatalogChangeReason::Uninstall,
        crate::catalog::CatalogExtensionChange {
            config: None,
            enabled: false,
            ..row
        },
    );
}

/// Remove an extension only when the persisted entry is still the exact entry
/// that a destructive operation validated. A concurrent replacement is left
/// untouched.
/// Returns the map key the entry was actually stored under, when one was
/// removed.
///
/// ⚠ **The returned key is the ROLLBACK's only way to put the entry back where
/// it was.** `stored_key_of` deliberately resolves a hand-written `config.yaml`
/// key that is not `config.key()` — that is the whole reason `remove_extension`
/// works on an operator's own mapping — so the key removed is frequently NOT
/// the derived one. Returning a bare `bool` here threw that away, and
/// [`restore_extension_if_absent`] then re-derived `config.key()` and silently
/// re-keyed a failed uninstall's entry to the normalized spelling: the
/// extension survived, as intended, under a name the operator never typed.
pub fn remove_extension_if_matches(
    key: &str,
    expected: &ExtensionEntry,
) -> Result<Option<String>, super::base::ConfigError> {
    use crate::catalog::{CatalogChangeReason, CatalogEntryChange};
    let expected = expected.clone();
    let removed = Config::global().update_param::<IndexMap<String, ExtensionEntry>, _, _>(
        EXTENSIONS_CONFIG_KEY,
        |extensions| remove_matching_extension(extensions, key, &expected),
    )?;
    let Some((stored_key, removed)) = removed else {
        return Ok(None);
    };
    let row = change_row(key, Some(&removed), CatalogEntryChange::Removed);
    announce(
        CatalogChangeReason::Uninstall,
        crate::catalog::CatalogExtensionChange {
            config: None,
            enabled: false,
            ..row
        },
    );
    Ok(Some(stored_key))
}

fn remove_matching_extension(
    extensions: &mut IndexMap<String, ExtensionEntry>,
    key: &str,
    expected: &ExtensionEntry,
) -> Option<(String, ExtensionEntry)> {
    let stored_key = stored_key_of(extensions, key, expected)?;
    let removed = extensions.shift_remove(&stored_key)?;
    Some((stored_key, removed))
}

/// Which map key actually holds `expected`, when any does.
///
/// ⚠ **A map key is not derived from the name, and the uninstall doors hold a
/// derived one.** Every writer in this process keys an entry by
/// `config.key()` — `name_to_key(config.name())` — but a `config.yaml` an
/// operator wrote by hand keeps whatever key they typed; [`find_entry_by_name`]
/// exists precisely because "map keys are not names". A caller that resolved an
/// entry BY NAME therefore holds a key the stored mapping may not use, and
/// looking the entry up by that key alone silently finds nothing: the uninstall
/// then reports "the extension configuration changed before deletion" for a
/// hand-added MCP server that changed not at all, and the entry survives — the
/// one case `remove_extension` exists for.
///
/// Three arms, narrowest first. The exact key still wins, so a mapping this
/// process wrote resolves without a scan and cannot be re-pointed by the arms
/// below it. Then a stored key that differs only in case or whitespace, in
/// EITHER direction (`name_to_key` is idempotent, so the requested key is
/// reduced too and a raw key requested against a normalized stored one resolves
/// as well). Then the entry itself, which is the identity the caller validated
/// and the only thing that can find an arbitrary hand-written key.
///
/// ⚠ **Every arm stays guarded by whole-entry equality**, so this widens which
/// KEY is looked at and never which ENTRY may be deleted: a concurrent
/// replacement is left alone exactly as before.
fn stored_key_of(
    extensions: &IndexMap<String, ExtensionEntry>,
    key: &str,
    expected: &ExtensionEntry,
) -> Option<String> {
    let matches = |current: &ExtensionEntry| {
        current.enabled == expected.enabled && current.config == expected.config
    };
    if extensions.get(key).is_some_and(matches) {
        return Some(key.to_owned());
    }
    let normalized = name_to_key(key);
    extensions
        .iter()
        .find(|(stored, current)| name_to_key(stored.as_str()) == normalized && matches(current))
        .or_else(|| extensions.iter().find(|(_, current)| matches(current)))
        .map(|(stored, _)| stored.clone())
}

/// Restore a removed entry only when no concurrent writer has already filled
/// its key. This is the rollback counterpart of
/// [`remove_extension_if_matches`].
/// `key` MUST be the key the entry was removed from — the value
/// [`remove_extension_if_matches`] returned — not `entry.config.key()`. A
/// rollback that re-derives the key restores the entry under the NORMALIZED
/// spelling, so an operator's hand-written `config.yaml` mapping silently
/// changes name on a failed uninstall.
pub fn restore_extension_if_absent(
    key: String,
    entry: ExtensionEntry,
) -> Result<bool, super::base::ConfigError> {
    use crate::catalog::{CatalogChangeReason, CatalogEntryChange};
    let restored = Config::global().update_param::<IndexMap<String, ExtensionEntry>, _, _>(
        EXTENSIONS_CONFIG_KEY,
        |extensions| insert_extension_if_absent(extensions, key.clone(), entry.clone()),
    )?;
    if restored {
        announce(
            CatalogChangeReason::Install,
            change_row(&key, Some(&entry), CatalogEntryChange::Added),
        );
    }
    Ok(restored)
}

fn insert_extension_if_absent(
    extensions: &mut IndexMap<String, ExtensionEntry>,
    key: String,
    entry: ExtensionEntry,
) -> bool {
    if extensions.contains_key(&key) {
        false
    } else {
        extensions.insert(key, entry);
        true
    }
}

pub fn set_extension_enabled(key: &str, enabled: bool) {
    use crate::catalog::{CatalogChangeReason, CatalogEntryChange};
    let mut extensions = get_extensions_map();
    let Some(entry) = extensions.get_mut(key) else {
        return;
    };
    if entry.enabled == enabled {
        return;
    }
    entry.enabled = enabled;
    let row = change_row(
        key,
        Some(&entry.clone()),
        if enabled {
            CatalogEntryChange::Enabled
        } else {
            CatalogEntryChange::Disabled
        },
    );
    save_extensions_map(extensions);
    announce(
        if enabled {
            CatalogChangeReason::Enable
        } else {
            CatalogChangeReason::Disable
        },
        row,
    );
}

pub fn get_all_extensions() -> Vec<ExtensionEntry> {
    let extensions = get_extensions_map();
    extensions.into_values().collect()
}

pub fn get_all_extension_names() -> Vec<String> {
    let extensions = get_extensions_map();
    extensions.keys().cloned().collect()
}

pub fn is_extension_enabled(key: &str) -> bool {
    let extensions = get_extensions_map();
    extensions.get(key).map(|e| e.enabled).unwrap_or(false)
}

pub fn get_enabled_extensions() -> Vec<ExtensionConfig> {
    get_all_extensions()
        .into_iter()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.config)
        .collect()
}

pub fn get_warnings() -> Vec<String> {
    let raw: Mapping = Config::global()
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_default();

    let mut warnings = Vec::new();
    for (k, v) in raw {
        if let (serde_yaml::Value::String(key), Ok(entry)) =
            (k, serde_yaml::from_value::<ExtensionEntry>(v))
        {
            if let ExtensionConfig::Builtin {
                name,
                available_tools,
                ..
            } = &entry.config
            {
                if name_to_key(name) == "computercontroller"
                    && available_tools.iter().any(|tool| {
                        let bare = tool.strip_prefix("computercontroller__").unwrap_or(tool);
                        WEB_DOCUMENT_TOOLS.contains(&bare)
                            || matches!(bare, "automation_script" | "computer_control")
                    })
                {
                    warnings.push(format!("'{key}': legacy Computer Controller tool restrictions remain restricted. Web/document tools moved to Web & Documents; select the new native Biorouter Copilot tools explicitly to allow desktop control. Retired scripts are no longer callable."));
                }
            }
            if matches!(entry.config, ExtensionConfig::Sse { .. }) {
                warnings.push(format!(
                    "'{}': SSE is unsupported, migrate to streamable_http",
                    key
                ));
            }
        }
    }
    warnings
}

pub fn resolve_extensions_for_new_session(
    workflow_extensions: Option<&[ExtensionConfig]>,
    override_extensions: Option<Vec<ExtensionConfig>>,
) -> Vec<ExtensionConfig> {
    if let Some(exts) = workflow_extensions {
        return exts.to_vec();
    }

    if let Some(exts) = override_extensions {
        return exts;
    }

    get_enabled_extensions()
}

#[cfg(test)]
mod persisted_provenance_tests {
    use super::*;

    fn raw_mapping(yaml: &str) -> Mapping {
        serde_yaml::from_str(yaml).expect("valid yaml mapping")
    }

    fn stdio_entry(name: &str, command: &str) -> ExtensionEntry {
        ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::stdio(name, command, "fixture", 30_u64),
        }
    }

    #[test]
    fn conditional_removal_never_deletes_a_replacement_and_rollback_never_overwrites_one() {
        let approved = stdio_entry("package", "approved-command");
        let replacement = stdio_entry("package", "replacement-command");
        let mut extensions = IndexMap::from([("package".to_owned(), replacement.clone())]);

        assert!(remove_matching_extension(&mut extensions, "package", &approved).is_none());
        assert_eq!(
            extensions.get("package").unwrap().config,
            replacement.config
        );
        assert!(!insert_extension_if_absent(
            &mut extensions,
            "package".to_owned(),
            approved.clone(),
        ));
        assert_eq!(
            extensions.get("package").unwrap().config,
            replacement.config
        );

        assert_eq!(
            remove_matching_extension(&mut extensions, "package", &replacement)
                .unwrap()
                .1
                .config,
            replacement.config
        );
        assert!(insert_extension_if_absent(
            &mut extensions,
            "package".to_owned(),
            approved.clone(),
        ));
        assert_eq!(extensions.get("package").unwrap().config, approved.config);
    }

    /// A FAILED uninstall must put the entry back under the key it was removed
    /// from, not under the derived one.
    ///
    /// ⚠ The rollback is the half nobody watches: the uninstall itself is
    /// correct, the operator sees an error, the extension still works — and its
    /// `config.yaml` mapping has quietly been renamed from `custom-key` to the
    /// normalized spelling. The next hand edit then targets a key that is no
    /// longer there. This pins the round trip, which is the only place the bug
    /// is visible: remove resolves the stored key, and restore must consume
    /// THAT key rather than re-deriving one.
    #[test]
    fn a_rolled_back_uninstall_restores_the_hand_written_key() {
        let entry = stdio_entry("custom name", "run-me");
        let derived_key = name_to_key(&entry.config.name());
        assert_ne!(derived_key, "custom-key", "the fixture must diverge");

        let mut extensions = IndexMap::from([("custom-key".to_owned(), entry.clone())]);

        // The uninstall resolves the hand-written key even though the caller
        // asked with the derived one.
        let (stored_key, removed) =
            remove_matching_extension(&mut extensions, &derived_key, &entry)
                .expect("the hand-written key resolves");
        assert_eq!(stored_key, "custom-key", "the removal reports where it was");
        assert!(extensions.is_empty());

        // …and the rollback puts it back THERE.
        assert!(insert_extension_if_absent(
            &mut extensions,
            stored_key.clone(),
            removed,
        ));
        assert!(
            extensions.contains_key("custom-key"),
            "a failed uninstall must not rename an operator's mapping; got {:?}",
            extensions.keys().collect::<Vec<_>>()
        );
        assert!(
            !extensions.contains_key(&derived_key),
            "restoring under the DERIVED key is the defect: {:?}",
            extensions.keys().collect::<Vec<_>>()
        );
    }

    /// ⚠ **The uninstall doors hold `name_to_key(<installed name>)`; the stored
    /// mapping holds whatever the operator typed.** A hand-written
    /// `config.yaml` key need not be derived from the entry's name at all —
    /// `persisted_names_reflect_only_entries_written_to_the_config_file` pins
    /// that "map keys are not names" — so a lookup by the derived key alone
    /// finds nothing and the removal reports a configuration that "changed"
    /// while it sat untouched. That is exactly the extension `remove_extension`
    /// was written for.
    #[test]
    fn conditional_removal_finds_a_hand_written_key_that_is_not_the_normalized_name() {
        let entry = stdio_entry("custom name", "run-me");
        let derived_key = name_to_key(&entry.config.name());
        assert_ne!(derived_key, "custom-key", "the fixture must diverge");

        // The headline case: an arbitrary hand-written key.
        let mut extensions = IndexMap::from([("custom-key".to_owned(), entry.clone())]);
        assert_eq!(
            remove_matching_extension(&mut extensions, &derived_key, &entry),
            Some(("custom-key".to_owned(), entry.clone())),
        );
        assert!(extensions.is_empty());

        // A key that differs only in case and whitespace, and the reverse —
        // a raw spelling requested against a normalized stored key.
        let mut extensions = IndexMap::from([("Custom Name".to_owned(), entry.clone())]);
        assert!(remove_matching_extension(&mut extensions, &derived_key, &entry).is_some());
        let mut extensions = IndexMap::from([(derived_key.clone(), entry.clone())]);
        assert!(remove_matching_extension(&mut extensions, "Custom Name", &entry).is_some());

        // The exact key still wins over both fallbacks, so a mapping this
        // process wrote cannot be re-pointed at a neighbour by them.
        let other = stdio_entry("custom name", "different-command");
        let mut extensions = IndexMap::from([
            ("aardvark".to_owned(), other.clone()),
            (derived_key.clone(), entry.clone()),
        ]);
        assert_eq!(
            remove_matching_extension(&mut extensions, &derived_key, &entry),
            Some((derived_key.clone(), entry.clone())),
        );
        assert_eq!(extensions.get("aardvark"), Some(&other));

        // …and widening the KEY search must not widen which ENTRY may go: a
        // replacement under a hand-written key is still left alone.
        let mut extensions = IndexMap::from([("custom-key".to_owned(), other.clone())]);
        assert!(remove_matching_extension(&mut extensions, &derived_key, &entry).is_none());
        assert_eq!(extensions.get("custom-key"), Some(&other));
    }

    // #42: provenance must come from the raw persisted mapping, not the
    // post-injection map — an injected default-off platform extension
    // (chatrecall) must NOT read as operator-authored.
    #[test]
    fn persisted_names_reflect_only_entries_written_to_the_config_file() {
        let raw = raw_mapping(
            r#"
developer:
  enabled: false
  type: builtin
  name: developer
custom-key:
  enabled: true
  type: stdio
  name: custom name
  cmd: run-me
  args: []
  description: A custom server
  timeout: 30
"#,
        );
        let names = persisted_names_from_raw(&raw);
        assert!(names.contains("developer"));
        assert!(
            names.contains("custom name"),
            "must record config.name(), matching get_extension_entry_by_name: {names:?}"
        );
        assert!(
            !names.contains("chatrecall"),
            "absent platform extensions are injected, never persisted"
        );
        assert!(!names.contains("custom-key"), "map keys are not names");
    }

    #[test]
    fn persisted_names_skip_malformed_entries_like_get_extensions_map() {
        let raw = raw_mapping(
            r#"
broken:
  type: no-such-type
developer:
  enabled: true
  type: builtin
  name: developer
"#,
        );
        let names = persisted_names_from_raw(&raw);
        assert_eq!(
            names,
            std::collections::HashSet::from(["developer".to_string()])
        );
        assert!(persisted_names_from_raw(&Mapping::new()).is_empty());
    }
}

#[cfg(test)]
mod entry_lookup_tests {
    use super::*;

    #[test]
    fn find_entry_by_name_matches_config_name_and_keeps_enabled_flag() {
        let mut extensions = IndexMap::new();
        extensions.insert(
            "developer".into(),
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::default(),
            },
        );
        extensions.insert(
            "custom-key".into(),
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio("custom name", "cmd", "Custom", 30_u64),
            },
        );

        // Matches on config.name(), not the map key.
        let entry = find_entry_by_name(&extensions, "custom name").expect("found by name");
        assert!(entry.enabled);
        assert!(find_entry_by_name(&extensions, "custom-key").is_none());

        // The operator's enabled:false flag survives the lookup — this is
        // what the manage_extensions enable gate relies on (#42).
        let entry = find_entry_by_name(&extensions, "developer").expect("found");
        assert!(!entry.enabled);

        assert!(find_entry_by_name(&extensions, "missing").is_none());
    }
}

#[cfg(test)]
mod platform_extension_tests {
    use super::*;

    #[test]
    fn custom_extension_cannot_shadow_reserved_platform_key() {
        let mut extensions = IndexMap::from([(
            "custom-key".to_string(),
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio("skills", "untrusted", "custom", 30_u64),
            },
        )]);

        inject_platform_extensions(&mut extensions);

        assert!(matches!(
            extensions["skills"].config,
            ExtensionConfig::Platform { ref name, .. } if name == "skills"
        ));
        assert!(!extensions.contains_key("custom-key"));
    }

    #[test]
    fn persisted_platform_enabled_state_is_preserved() {
        let mut extensions = IndexMap::from([(
            "chatrecall".to_string(),
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::Platform {
                    name: "Chat Recall".to_string(),
                    description: "configured".to_string(),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                },
            },
        )]);

        inject_platform_extensions(&mut extensions);

        assert!(extensions["chatrecall"].enabled);
        assert_eq!(extensions["chatrecall"].config.name(), "Chat Recall");
    }
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    #[test]
    fn reset_filter_keeps_only_bundled_extensions() {
        let mut extensions = IndexMap::new();
        extensions.insert(
            "developer".into(),
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::default(),
            },
        );
        extensions.insert(
            "custom".into(),
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio("custom", "custom", "Custom", 30_u64),
            },
        );

        assert_eq!(retain_bundled_extensions(&mut extensions), 1);
        assert_eq!(extensions.len(), 1);
        assert!(extensions["developer"].config.is_bundled());
    }
}

#[cfg(test)]
mod retired_builtin_tests {
    use super::*;

    fn entry(config: ExtensionConfig) -> ExtensionEntry {
        ExtensionEntry {
            enabled: true,
            config,
        }
    }

    #[test]
    fn only_the_retired_tutorial_builtin_is_filtered() {
        let tutorial = entry(ExtensionConfig::Builtin {
            name: "Tutorial".into(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: Vec::new(),
        });
        let external_tutorial = entry(ExtensionConfig::stdio(
            "tutorial",
            "external-tutorial",
            "External tutorial",
            30_u64,
        ));

        assert!(is_retired_builtin_extension(&tutorial));
        assert!(!is_retired_builtin_extension(&external_tutorial));
        assert!(!is_retired_builtin_extension(&entry(
            ExtensionConfig::default()
        )));
    }
}

#[cfg(test)]
mod computer_use_migration_tests {
    use super::*;

    fn legacy(enabled: bool, tools: &[&str]) -> ExtensionEntry {
        let mut entry = builtin_entry("computercontroller", "Computer Controller", "legacy");
        entry.enabled = enabled;
        if let ExtensionConfig::Builtin {
            available_tools, ..
        } = &mut entry.config
        {
            *available_tools = tools.iter().map(|tool| (*tool).to_owned()).collect();
        }
        entry
    }

    #[test]
    fn fresh_defaults_are_available_without_a_renderer() {
        let mut entries = IndexMap::new();
        inject_builtin_extensions(&mut entries);
        assert!(entries["computercontroller"].enabled);
        assert!(entries["webdocuments"].enabled);
    }

    /// ⚠ The label is the whole reason `COMPUTER_USE_MIGRATION_VERSION` is 2.
    /// `inject_builtin_extensions` inserts only when the key is absent, so an
    /// upgrading user's stored legacy Computer Controller label is reachable only
    /// here.
    #[test]
    fn the_migration_rewrites_a_stored_legacy_label() {
        let mut entries = IndexMap::from([("computercontroller".to_owned(), legacy(true, &[]))]);
        migrate_computer_use_capabilities(&mut entries);
        let ExtensionConfig::Builtin { display_name, .. } = &entries["computercontroller"].config
        else {
            panic!()
        };
        assert_eq!(display_name.as_deref(), Some("Biorouter Copilot"));
    }

    #[test]
    fn disabled_computer_use_keeps_both_capabilities_disabled() {
        let mut entries = IndexMap::from([("custom-key".to_owned(), legacy(false, &[]))]);
        migrate_computer_use_capabilities(&mut entries);
        inject_builtin_extensions(&mut entries);
        assert!(!entries["custom-key"].enabled);
        assert!(!entries["webdocuments"].enabled);
        assert!(!entries.contains_key("computercontroller"));
    }

    #[test]
    fn utility_restrictions_split_without_authorizing_native_actions() {
        let original = legacy(true, &["computercontroller__pdf_tool"]);
        let mut entries = IndexMap::from([("computercontroller".to_owned(), original.clone())]);
        migrate_computer_use_capabilities(&mut entries);
        let ExtensionConfig::Builtin {
            available_tools, ..
        } = &entries["webdocuments"].config
        else {
            panic!()
        };
        assert_eq!(available_tools, &["pdf_tool"]);
        let ExtensionConfig::Builtin {
            available_tools, ..
        } = &entries["computercontroller"].config
        else {
            panic!()
        };
        assert_eq!(available_tools, &["computercontroller__pdf_tool"]);
        let first = entries.clone();
        migrate_computer_use_capabilities(&mut entries);
        assert_eq!(entries, first);
    }

    #[test]
    fn no_matching_utility_never_becomes_an_unrestricted_enabled_capability() {
        let mut entries = IndexMap::from([(
            "computercontroller".to_owned(),
            legacy(true, &["computer_control"]),
        )]);
        migrate_computer_use_capabilities(&mut entries);
        assert!(!entries["webdocuments"].enabled);
    }

    #[test]
    fn destination_key_collision_never_overwrites_a_custom_extension() {
        let custom = ExtensionEntry {
            enabled: false,
            config: ExtensionConfig::stdio("custom", "custom-command", "configured", 30_u64),
        };
        let mut entries = IndexMap::from([
            ("computercontroller".to_owned(), legacy(true, &[])),
            ("webdocuments".to_owned(), custom.clone()),
        ]);
        migrate_computer_use_capabilities(&mut entries);
        inject_builtin_extensions(&mut entries);
        assert_eq!(entries["webdocuments"], custom);
    }

    #[test]
    fn existing_destination_disable_and_allowlist_are_preserved() {
        let mut web = builtin_entry("webdocuments", "Web & Documents", "configured");
        web.enabled = false;
        if let ExtensionConfig::Builtin {
            available_tools, ..
        } = &mut web.config
        {
            *available_tools = vec!["docx_tool".to_owned()];
        }
        let mut entries = IndexMap::from([
            ("computercontroller".to_owned(), legacy(true, &[])),
            ("webdocuments".to_owned(), web.clone()),
        ]);
        migrate_computer_use_capabilities(&mut entries);
        inject_builtin_extensions(&mut entries);
        assert_eq!(entries["webdocuments"], web);
    }
}
