//! A short fingerprint of what a request sends BEFORE the conversation: the
//! system prompt and the tools array.
//!
//! A provider caches a request's prefix, and the system text and the tool
//! definitions come first, so any change there makes the next request re-read
//! the whole conversation at the fresh-input price. Measured on Versa
//! (gpt-5.6, 2026-11): changing only the prompt's clock line took cached
//! tokens from 26,694 to 0. The motivating chat read 11.7% of its input from
//! the cache, and nothing in the logs said which requests had started over.
//!
//! [`observe`] logs both hashes at debug level for every request, and at info
//! level when either changed since the chat's previous request, naming which.
//! That line is the one to look for when a chat's cache rate is low.

use rmcp::model::Tool;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

/// The two hashes of one request's fixed part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptFingerprint {
    pub system: u64,
    pub tools: u64,
}

/// What changed between a chat's previous request and this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintChange {
    /// The chat's first request this process has seen.
    First,
    Unchanged,
    System,
    Tools,
    Both,
}

impl FingerprintChange {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Unchanged => "unchanged",
            Self::System => "system prompt",
            Self::Tools => "tools",
            Self::Both => "system prompt and tools",
        }
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Hash the system prompt and the tools as they are serialized (name,
/// description, schema and annotations, in the order they are sent).
pub fn fingerprint(system_prompt: &str, tools: &[Tool]) -> PromptFingerprint {
    let tools_json = serde_json::to_vec(tools).unwrap_or_default();
    PromptFingerprint {
        system: hash_bytes(system_prompt.as_bytes()),
        tools: hash_bytes(&tools_json),
    }
}

/// The last fingerprint per chat, process-wide. Bounded: past
/// [`MAX_TRACKED_CHATS`] it is cleared, which only costs one "first" line per
/// chat.
static LAST: LazyLock<Mutex<HashMap<String, PromptFingerprint>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const MAX_TRACKED_CHATS: usize = 512;

/// Compare `current` with the chat's previous request and remember it.
pub fn record(session_id: &str, current: PromptFingerprint) -> FingerprintChange {
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if last.len() >= MAX_TRACKED_CHATS && !last.contains_key(session_id) {
        last.clear();
    }
    let previous = last.insert(session_id.to_string(), current);
    match previous {
        None => FingerprintChange::First,
        Some(previous) => match (
            previous.system != current.system,
            previous.tools != current.tools,
        ) {
            (false, false) => FingerprintChange::Unchanged,
            (true, false) => FingerprintChange::System,
            (false, true) => FingerprintChange::Tools,
            (true, true) => FingerprintChange::Both,
        },
    }
}

/// Fingerprint one request and log it (see the module docs). Cheap: one
/// serialization of the tool list and two hashes.
pub fn observe(session_id: &str, system_prompt: &str, tools: &[Tool]) -> FingerprintChange {
    let current = fingerprint(system_prompt, tools);
    let change = record(session_id, current);
    let system_hash = format!("{:016x}", current.system);
    let tools_hash = format!("{:016x}", current.tools);
    match change {
        FingerprintChange::System | FingerprintChange::Tools | FingerprintChange::Both => {
            tracing::info!(
                session_id,
                system_hash = %system_hash,
                tools_hash = %tools_hash,
                tool_count = tools.len(),
                changed = change.as_str(),
                "the request's fixed prefix changed since this chat's last request; \
                 the provider's prompt cache restarts here"
            );
        }
        FingerprintChange::First | FingerprintChange::Unchanged => {
            tracing::debug!(
                session_id,
                system_hash = %system_hash,
                tools_hash = %tools_hash,
                tool_count = tools.len(),
                changed = change.as_str(),
                "prompt prefix fingerprint"
            );
        }
    }
    change
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn tool(name: &str, description: &str) -> Tool {
        Tool::new(
            name.to_string(),
            description.to_string(),
            Arc::new(serde_json::Map::new()),
        )
    }

    #[test]
    fn a_change_is_named_and_an_unchanged_prefix_is_not() {
        let id = format!("fp-{}", uuid::Uuid::new_v4());
        let tools = vec![tool("a", "first"), tool("b", "second")];
        assert_eq!(observe(&id, "system", &tools), FingerprintChange::First);
        assert_eq!(observe(&id, "system", &tools), FingerprintChange::Unchanged);
        assert_eq!(observe(&id, "system 2", &tools), FingerprintChange::System);
        let reordered = vec![tool("b", "second"), tool("a", "first")];
        assert_eq!(
            observe(&id, "system 2", &reordered),
            FingerprintChange::Tools,
            "order is part of the prefix"
        );
        assert_eq!(
            observe(&id, "system 3", &[tool("a", "first")]),
            FingerprintChange::Both
        );
    }

    #[test]
    fn the_same_inputs_hash_the_same() {
        let tools = vec![tool("a", "first")];
        assert_eq!(fingerprint("s", &tools), fingerprint("s", &tools));
        assert_ne!(fingerprint("s", &tools), fingerprint("t", &tools));
    }
}
