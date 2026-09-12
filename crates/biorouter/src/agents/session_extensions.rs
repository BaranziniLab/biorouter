//! The one home for "this chat's extension roster is now X" — the write into
//! `enabled_extensions.v0` on the session row, and the classifier that says
//! which model-facing catalog tools require it.
//!
//! Both used to be private items in [`crate::agents::agent`], which is a
//! `pub(crate) mod`: reachable from the reply loop and from nowhere else. The
//! extension handler needs the write (so an attach is durable *before* it
//! reports `"attached"`), and `biorouter-server`'s `/agent/call_tool` needs the
//! classifier (so a `manage_extensions` that arrives over HTTP is persisted at
//! all). Mirrors [`crate::agents::session_skills`], which is the same shape for
//! the other half of the catalog.

use anyhow::{anyhow, Result};

use crate::agents::extension_manager::ExtensionManager;
use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};
use crate::session::session_manager::{SessionManager, SessionType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolCatalogMutation {
    pub persist_extension_state: bool,
}

/// Model-facing catalog tools that can change the callable surface during the
/// current turn. Read-only browse/search calls are deliberately absent.
pub fn tool_catalog_mutation(tool_name: &str) -> Option<ToolCatalogMutation> {
    let persist_extension_state = match tool_name {
        "extensionmanager__manage_extensions"
        | "extensionmanager__install_extension"
        | "extensionmanager__delete_extension_package"
        | "extensionmanager__remove_extension" => true,
        "skills__installMarketplaceSkill"
        | "skills__importSkillPackage"
        | "skills__removeSkillPackage"
        | "skills__setSkillEnabled"
        // The retired pair still dispatches, so it still mutates the catalog.
        | "skills__hotLoadSkill"
        | "skills__hotUnloadSkill" => false,
        _ => return None,
    };
    Some(ToolCatalogMutation {
        persist_extension_state,
    })
}

/// Record the live manager's roster as this session's `enabled_extensions.v0`.
///
/// ⚠ The write closure is `move |_| Ok(value)` — a whole-key REPLACE, not a
/// merge. That is deliberate and load-bearing: a removal is expressed by the
/// key's *absence* from the live snapshot, so a `union(stored, live)` would
/// make disabling an extension unpersistable. `update_extension_state` still
/// does the read and the write inside one transaction, so a concurrent writer
/// of a *different* key of `extension_data` is not clobbered.
///
/// The snapshot goes through [`ExtensionManager::get_extension_configs`]
/// because the `!inprocess && origin != AutoInjected` filter lives inside it —
/// an auto-injected extension must never reach the session row.
pub async fn record(
    session_manager: &SessionManager,
    extension_manager: &ExtensionManager,
    session_id: &str,
) -> Result<()> {
    let session = session_manager.get_session(session_id, false).await?;
    if session.session_type == SessionType::SubAgent {
        return Err(anyhow!(
            "subagent extension grants are immutable runtime-profile authority"
        ));
    }
    let extensions_state =
        EnabledExtensionsState::new(extension_manager.get_extension_configs().await);
    let value = extensions_state
        .to_value()
        .map_err(|e| anyhow!("Extension state serialization failed: {}", e))?;

    let written = session_manager
        .update_extension_state(
            session_id,
            EnabledExtensionsState::EXTENSION_NAME,
            EnabledExtensionsState::VERSION,
            move |_| Ok(value),
        )
        .await?;
    if written.is_none() {
        return Err(anyhow!(
            "cannot record extension state: no session {session_id}"
        ));
    }
    Ok(())
}

/// This conversation's SAVED extension roster, telling "nothing saved" apart
/// from "saved and unreadable".
///
/// [`EnabledExtensionsState::from_extension_data`] collapses both into `None`
/// — it ends in `.ok()` — and for a caller about to REPLACE the key those two
/// answers could not be further apart. An absent key has nothing to lose. An
/// unreadable one is a roster this build cannot see, and overwriting it is the
/// data loss, not the repair.
pub fn saved_roster_of(
    extension_data: &crate::session::extension_data::ExtensionData,
    session_id: &str,
) -> Result<Vec<crate::agents::ExtensionConfig>> {
    let Some(value) = extension_data.get_extension_state(
        EnabledExtensionsState::EXTENSION_NAME,
        EnabledExtensionsState::VERSION,
    ) else {
        return Ok(Vec::new());
    };
    Ok(EnabledExtensionsState::from_value(value)
        .map_err(|e| {
            anyhow!(
                "conversation {session_id} has a saved extension roster this build cannot \
                 read ({e}); refusing to replace it with anything"
            )
        })?
        .extensions)
}

/// [`saved_roster_of`] for a session id.
pub async fn saved_roster(
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<Vec<crate::agents::ExtensionConfig>> {
    let session = session_manager.get_session(session_id, false).await?;
    saved_roster_of(&session.extension_data, session_id)
}

/// Apply ONE change to the conversation's saved roster: the stored set, minus
/// `remove`, plus `add`.
///
/// ⚠ **Not [`record`], and the difference is the whole point.** `record`
/// snapshots the LIVE manager, which is right for the reply loop — the chat is
/// open, its manager is its roster, and a removal is expressed by an absence.
/// `workspace_set_tools` writes into conversations that are **not open**, where
/// the live manager is an empty agent `get_or_create_agent` has just minted:
/// snapshotting that wrote the one change as the conversation's entire roster
/// and reported success. Measured on a cold chat holding three extensions — one
/// `add_extensions` left one, one `remove_extensions` left none.
///
/// So this writes a DELTA on the durable state instead of a snapshot of a
/// volatile one, which is also the right answer for a chat that *is* open: the
/// caller knows exactly what it changed, and everything else in the row is
/// state it was never asked to touch. An unreadable roster fails loudly here
/// rather than being replaced (see [`saved_roster_of`]).
pub async fn apply_saved_roster_delta(
    session_manager: &SessionManager,
    session_id: &str,
    add: &[crate::agents::ExtensionConfig],
    remove: &[String],
) -> Result<Vec<crate::agents::ExtensionConfig>> {
    use crate::agents::extension_manager::normalize;

    let session = session_manager.get_session(session_id, false).await?;
    // The same refusal `record` makes, for the same reason: a subagent's grant
    // is runtime-profile authority, not a preference.
    if session.session_type == SessionType::SubAgent {
        return Err(anyhow!(
            "subagent extension grants are immutable runtime-profile authority"
        ));
    }

    let mut roster = saved_roster_of(&session.extension_data, session_id)?;
    let dropped: Vec<String> = remove.iter().map(|name| normalize(name)).collect();
    roster.retain(|config| !dropped.contains(&normalize(&config.name())));
    for config in add {
        let name = normalize(&config.name());
        // Re-adding replaces rather than duplicates: two entries under one name
        // is a roster whose meaning depends on iteration order.
        roster.retain(|existing| normalize(&existing.name()) != name);
        roster.push(config.clone());
    }

    let value = EnabledExtensionsState::new(roster.clone())
        .to_value()
        .map_err(|e| anyhow!("Extension state serialization failed: {}", e))?;
    let written = session_manager
        .update_extension_state(
            session_id,
            EnabledExtensionsState::EXTENSION_NAME,
            EnabledExtensionsState::VERSION,
            move |_| Ok(value),
        )
        .await?;
    if written.is_none() {
        return Err(anyhow!(
            "cannot record extension state: no session {session_id}"
        ));
    }
    Ok(roster)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::extension_data::ExtensionData;

    fn stdio(name: &str) -> crate::agents::ExtensionConfig {
        crate::agents::ExtensionConfig::Stdio {
            name: name.to_string(),
            description: String::new(),
            cmd: "true".to_string(),
            args: Vec::new(),
            envs: Default::default(),
            env_keys: Vec::new(),
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    /// The three answers a reader has to tell apart, and the one
    /// `EnabledExtensionsState::from_extension_data` collapses.
    ///
    /// It ends in `.ok()`, so "no roster saved" and "a roster this build cannot
    /// parse" are both `None` there. A caller about to REPLACE the key needs
    /// them apart: the first has nothing to lose, the second is the data loss.
    #[test]
    fn an_unreadable_saved_roster_is_not_an_absent_one() {
        let mut absent = ExtensionData::new();
        absent.set_extension_state("todo", "v0", serde_json::json!({ "content": "" }));
        assert!(
            saved_roster_of(&absent, "s1").unwrap().is_empty(),
            "no roster saved is an empty roster, not an error"
        );

        let mut readable = ExtensionData::new();
        readable.set_extension_state(
            EnabledExtensionsState::EXTENSION_NAME,
            EnabledExtensionsState::VERSION,
            EnabledExtensionsState::new(vec![stdio("a"), stdio("b")])
                .to_value()
                .unwrap(),
        );
        assert_eq!(
            saved_roster_of(&readable, "s1")
                .unwrap()
                .iter()
                .map(|c| c.name())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()]
        );

        let mut unreadable = ExtensionData::new();
        unreadable.set_extension_state(
            EnabledExtensionsState::EXTENSION_NAME,
            EnabledExtensionsState::VERSION,
            serde_json::json!({ "extensions": "written by a newer build" }),
        );
        let err = saved_roster_of(&unreadable, "s1")
            .expect_err("an unreadable roster must not read as an empty one")
            .to_string();
        assert!(err.contains("cannot read"), "{err}");
        assert!(err.contains("refusing to replace it"), "{err}");
    }
}
