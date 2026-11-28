//! Matcher semantics.
//!
//! **Name matcher** (`matcher:`): `None`/`""`/`"*"` match everything;
//! otherwise exact string equality, falling back to an anchored regex (which
//! provides `a|b` alternation and full regex patterns in one rule, mirroring
//! Claude Code).
//!
//! **Input matcher** (`input_matcher:`, BR-27): an optional *narrowing* filter
//! on the event's `tool_input`, so a rule can say "only guard `rm -rf`" or
//! "only writes under `/etc`" instead of shelling out to a guard script on
//! every tool call. It is either a single regex searched against the whole
//! `tool_input` JSON, or a map of dotted field path -> regex where every entry
//! must match. Unlike the name matcher these are *searched*, not anchored, so
//! `rm\s+-rf` hits anywhere in the value; anchor explicitly with `^…$`.
//! An event with no `tool_input` (Stop, UserPromptSubmit, …) never matches a
//! group that declares an `input_matcher`.
//!
//! Compiled regexes are cached process-wide, keyed by pattern source, so a
//! matcher is compiled once rather than on every tool call. Invalid patterns
//! are logged once and cached as non-matching.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

/// Process-wide compiled-regex cache. `None` records a pattern that failed to
/// compile, so a bad rule is neither recompiled nor re-warned on every call.
static REGEX_CACHE: LazyLock<RwLock<HashMap<String, Option<Arc<Regex>>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Compile (or fetch from cache) `pattern`. `source` is the user-written
/// pattern, used only for the warning.
fn compiled(pattern: &str, source: &str) -> Option<Arc<Regex>> {
    if let Some(cached) = REGEX_CACHE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(pattern)
    {
        return cached.clone();
    }
    let result = match Regex::new(pattern) {
        Ok(re) => Some(Arc::new(re)),
        Err(e) => {
            warn!("hooks: invalid matcher regex '{}': {}", source, e);
            None
        }
    };
    REGEX_CACHE
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(pattern.to_string(), result.clone());
    result
}

/// Returns true when `matcher` matches `key`.
///
/// A tool name is matched under every spelling a rule may use for it
/// ([`crate::config::extension_renames::tool_name_aliases`]): a hook or a
/// managed rule written for `gptragent__r` before the extension was renamed
/// still matches `gptr__r`, so a managed deny cannot be lost by the rename. A
/// key with no renamed prefix (any other tool, or a non-tool key) has one
/// spelling and is matched exactly as before.
///
/// Invalid regex patterns are logged and treated as non-matching.
pub fn matcher_matches(matcher: Option<&str>, key: &str) -> bool {
    let Some(pattern) = matcher else {
        return true;
    };
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    crate::config::extension_renames::tool_name_aliases(key)
        .iter()
        .any(|spelling| matches_one(pattern, spelling))
}

/// [`matcher_matches`] for one spelling of the key: exact string equality,
/// then the anchored regex.
fn matches_one(pattern: &str, key: &str) -> bool {
    if pattern == key {
        return true;
    }
    compiled(&format!("^(?:{pattern})$"), pattern).is_some_and(|re| re.is_match(key))
}

/// An optional filter on the event's `tool_input` (BR-27).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum InputMatcher {
    /// A single regex, searched against the whole `tool_input` JSON text.
    Any(String),
    /// Dotted field path -> regex, searched against that field's value.
    /// Every entry must match (AND); a missing field never matches.
    Fields(BTreeMap<String, String>),
}

impl InputMatcher {
    /// Whether this matcher accepts `tool_input`. `None` (an event that
    /// carries no tool input) never matches.
    pub fn matches(&self, tool_input: Option<&Value>) -> bool {
        let Some(input) = tool_input else {
            return false;
        };
        match self {
            InputMatcher::Any(pattern) => search(pattern, &value_text(input)),
            InputMatcher::Fields(fields) => fields.iter().all(|(path, pattern)| {
                lookup(input, path).is_some_and(|value| search(pattern, &value_text(value)))
            }),
        }
    }
}

/// Unanchored regex search. `""`/`"*"` impose no constraint.
fn search(pattern: &str, haystack: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    compiled(pattern, pattern).is_some_and(|re| re.is_match(haystack))
}

/// A JSON value as text: strings as-is, anything else as compact JSON, so a
/// regex can be written against numbers, arrays and nested objects too.
fn value_text(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(s) => Cow::Borrowed(s),
        other => Cow::Owned(other.to_string()),
    }
}

/// Resolve a dotted path (`command`, `params.path`, `args.0`) inside a JSON
/// value. Numeric segments index arrays.
fn lookup<'a>(input: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = input;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_star_and_none_match_all() {
        assert!(matcher_matches(None, "anything"));
        assert!(matcher_matches(Some(""), "anything"));
        assert!(matcher_matches(Some("*"), "anything"));
    }

    #[test]
    fn exact_match() {
        assert!(matcher_matches(
            Some("developer__shell"),
            "developer__shell"
        ));
        assert!(!matcher_matches(
            Some("developer__shell"),
            "developer__text_editor"
        ));
    }

    #[test]
    fn alternation() {
        let m = Some("developer__shell|developer__text_editor");
        assert!(matcher_matches(m, "developer__shell"));
        assert!(matcher_matches(m, "developer__text_editor"));
        assert!(!matcher_matches(m, "memory__store"));
    }

    #[test]
    fn regex_is_anchored() {
        assert!(matcher_matches(Some("developer__.*"), "developer__shell"));
        assert!(!matcher_matches(Some("developer"), "developer__shell"));
        assert!(!matcher_matches(Some("__shell"), "developer__shell"));
    }

    #[test]
    fn invalid_regex_is_non_match() {
        assert!(!matcher_matches(Some("foo[("), "foo"));
    }

    // ---- BR-27: compiled-regex cache ----

    #[test]
    fn compiled_regex_is_cached() {
        let pattern = "^br27_cache_probe_[0-9]+$";
        let first = compiled(pattern, pattern).expect("compiles");
        let second = compiled(pattern, pattern).expect("cached");
        assert!(
            Arc::ptr_eq(&first, &second),
            "the same pattern must be compiled once and reused"
        );
    }

    #[test]
    fn invalid_regex_is_cached_as_non_matching() {
        let pattern = "br27_bad_probe[(";
        assert!(compiled(pattern, pattern).is_none());
        // Second lookup is served from the cache (no recompile, no re-warn).
        assert!(compiled(pattern, pattern).is_none());
        assert!(REGEX_CACHE
            .read()
            .unwrap()
            .get(pattern)
            .is_some_and(|entry| entry.is_none()));
    }

    // ---- BR-27: tool_input matchers ----

    #[test]
    fn any_form_searches_whole_tool_input() {
        let m = InputMatcher::Any(r"rm\s+-rf".to_string());
        assert!(m.matches(Some(&json!({"command": "rm -rf /tmp/x"}))));
        assert!(!m.matches(Some(&json!({"command": "ls -la"}))));
    }

    #[test]
    fn fields_form_matches_named_field() {
        let m = InputMatcher::Fields(BTreeMap::from([("path".to_string(), "^/etc/".to_string())]));
        assert!(m.matches(Some(&json!({"command": "write", "path": "/etc/hosts"}))));
        assert!(!m.matches(Some(&json!({"command": "write", "path": "/home/me/notes"}))));
    }

    #[test]
    fn fields_form_ands_every_entry() {
        let m = InputMatcher::Fields(BTreeMap::from([
            ("command".to_string(), "^write$".to_string()),
            ("path".to_string(), r"\.rs$".to_string()),
        ]));
        assert!(m.matches(Some(&json!({"command": "write", "path": "src/main.rs"}))));
        // Right file, wrong command.
        assert!(!m.matches(Some(&json!({"command": "view", "path": "src/main.rs"}))));
        // Right command, wrong file.
        assert!(!m.matches(Some(&json!({"command": "write", "path": "README.md"}))));
    }

    #[test]
    fn missing_field_never_matches() {
        let m = InputMatcher::Fields(BTreeMap::from([("path".to_string(), ".*".to_string())]));
        assert!(!m.matches(Some(&json!({"command": "ls"}))));
    }

    #[test]
    fn nested_and_indexed_paths_resolve() {
        let m = InputMatcher::Fields(BTreeMap::from([
            ("params.path".to_string(), "^/etc/".to_string()),
            ("argv.0".to_string(), "^rm$".to_string()),
        ]));
        assert!(m.matches(Some(&json!({
            "params": {"path": "/etc/passwd"},
            "argv": ["rm", "-rf"],
        }))));
        assert!(!m.matches(Some(&json!({
            "params": {"path": "/tmp/passwd"},
            "argv": ["rm", "-rf"],
        }))));
    }

    #[test]
    fn non_string_values_match_against_their_json_text() {
        let m = InputMatcher::Fields(BTreeMap::from([
            ("recursive".to_string(), "^true$".to_string()),
            ("depth".to_string(), "^[0-9]+$".to_string()),
        ]));
        assert!(m.matches(Some(&json!({"recursive": true, "depth": 3}))));
        assert!(!m.matches(Some(&json!({"recursive": false, "depth": 3}))));
    }

    #[test]
    fn input_matcher_without_tool_input_never_matches() {
        let m = InputMatcher::Any(".*".to_string());
        assert!(!m.matches(None));
    }

    #[test]
    fn invalid_input_regex_is_non_matching() {
        let m = InputMatcher::Any("foo[(".to_string());
        assert!(!m.matches(Some(&json!({"command": "foo"}))));
    }

    #[test]
    fn wildcard_input_pattern_imposes_no_constraint() {
        let m = InputMatcher::Fields(BTreeMap::from([("command".to_string(), "*".to_string())]));
        assert!(m.matches(Some(&json!({"command": "anything at all"}))));
        // Still requires the field to exist.
        assert!(!m.matches(Some(&json!({"other": "x"}))));
    }

    /// I4 of the gptr rename: a hook or managed matcher written for an old
    /// tool name still matches the renamed tool, exactly or as a pattern, and a
    /// managed deny written for the old name denies the new one.
    #[test]
    fn a_matcher_written_for_an_old_tool_name_matches_the_renamed_tool() {
        assert!(matcher_matches(Some("gptragent__r"), "gptr__r"));
        assert!(matcher_matches(Some("gptragent__.*"), "gptr__inspect"));
        assert!(matcher_matches(
            Some("developer__shell|gptragent__r"),
            "gptr__r"
        ));
        assert!(matcher_matches(Some("gptr__r"), "gptragent__r"));
        assert!(!matcher_matches(Some("gptragent__r"), "gptr__inspect"));
        assert!(!matcher_matches(Some("gptragent__r"), "developer__shell"));
        assert!(!matcher_matches(Some("gptragent"), "gptr"));

        let policy: crate::managed::settings::ManagedPolicyFile =
            serde_yaml::from_str("permissions:\n  deny: [gptragent__r]\n  allow: [gptr__r]\n")
                .unwrap();
        assert_eq!(
            policy.permission_for("gptr__r"),
            Some(crate::managed::settings::ManagedVerdict::Deny)
        );
    }

    #[test]
    fn deserializes_both_forms() {
        let any: InputMatcher = serde_yaml::from_str(r#""rm -rf""#).unwrap();
        assert_eq!(any, InputMatcher::Any("rm -rf".to_string()));

        let fields: InputMatcher = serde_yaml::from_str("path: \"^/etc/\"\n").unwrap();
        assert_eq!(
            fields,
            InputMatcher::Fields(BTreeMap::from([("path".to_string(), "^/etc/".to_string())]))
        );
    }
}
