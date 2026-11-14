//! On-disk schema (serde) for the managed/enterprise policy file (BR-65).
//!
//! The file lives at an admin-owned, ownership-verified path (see
//! [`crate::config::paths::Paths::managed_policy_path`] and [`super::trust`]).
//! Its schema is additive and forward-compatible: it reuses [`HooksConfig`]
//! (which skips unknown events) and carries an inert `command_rules` field so
//! BR-20/BR-21 can extend it later without breaking older binaries.

use serde::Deserialize;

use crate::hooks::HooksConfig;

/// A managed verdict for a tool, resolved with **deny > ask > allow** precedence
/// (deny/ask always win, mirroring Claude Code managed settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedVerdict {
    /// Force auto-approve (applied as the permission baseline, ahead of the
    /// user's own permission table — the one decision escalation cannot make).
    Allow,
    /// Force a human-approval prompt (rides the escalation-only merge).
    Ask,
    /// Force a hard denial (rides the escalation-only merge; non-bypassable).
    Deny,
}

/// Root schema of `managed-policy.yaml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ManagedPolicyFile {
    /// Managed hooks: same schema as the user `hooks:` map. Always run, resolved
    /// before user/project groups, and cannot be disabled by the user.
    #[serde(default)]
    pub hooks: HooksConfig,

    /// If set, overrides the user's `allow_project_hooks` opt-in
    /// (`Some(false)` forbids project hooks org-wide; `Some(true)` forces them
    /// on). `None` leaves the user/env value untouched.
    #[serde(default)]
    pub allow_project_hooks: Option<bool>,

    /// Managed permission rules over tool names.
    #[serde(default)]
    pub permissions: ManagedPermissions,

    /// The administrator's `dictation:` block (DD-90), kept as raw YAML and
    /// parsed apart by [`super::dictation::DictationPolicy`]. Any YAML value
    /// deserializes here, so a block an older binary does not understand cannot
    /// fail the file and lift every `permissions.deny`; the block fails closed
    /// for dictation alone.
    ///
    /// `None` only when the key is absent. A key with an empty value
    /// (`dictation:`, `dictation: ~`, `dictation: null`) is
    /// `Some(Value::Null)`, which turns dictation off: the likeliest way to
    /// write one is a block whose keys lost their indentation, and those keys
    /// then land at the top level, where unknown keys are ignored.
    #[serde(default, deserialize_with = "present")]
    pub dictation: Option<serde_yaml::Value>,
}

/// Keep a present key as `Some`, even when its value is null. A plain
/// `Option<Value>` reads a null value as `None`, the same as an absent key;
/// `#[serde(default)]` still gives `None` when the key is missing.
fn present<'de, D>(deserializer: D) -> Result<Option<serde_yaml::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_yaml::Value::deserialize(deserializer).map(Some)
}

impl ManagedPolicyFile {
    /// The managed verdict for a tool name, or `None` if no managed rule
    /// applies. Precedence is deny > ask > allow.
    pub fn permission_for(&self, tool_name: &str) -> Option<ManagedVerdict> {
        if Self::any_match(&self.permissions.deny, tool_name) {
            return Some(ManagedVerdict::Deny);
        }
        if Self::any_match(&self.permissions.ask, tool_name) {
            return Some(ManagedVerdict::Ask);
        }
        if Self::any_match(&self.permissions.allow, tool_name) {
            return Some(ManagedVerdict::Allow);
        }
        None
    }

    /// Match against exact tool names or the anchored-regex matcher the hooks
    /// engine already uses (so `a|b` alternation and `foo__.*` both work).
    fn any_match(patterns: &[String], tool_name: &str) -> bool {
        patterns
            .iter()
            .any(|pattern| crate::hooks::matcher::matcher_matches(Some(pattern), tool_name))
    }
}

/// Managed permission rules. Each list is a set of exact tool names or anchored
/// regex matchers (`matcher.rs` semantics).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ManagedPermissions {
    /// Force auto-approve (applied as the permission baseline).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Force a human-approval prompt (escalation).
    #[serde(default)]
    pub ask: Vec<String>,
    /// Force a hard denial (escalation).
    #[serde(default)]
    pub deny: Vec<String>,
    /// Reserved for BR-20 (catastrophic list) / BR-21 (argv/prefix command
    /// policy). Parsed but **inert** in phase 1 — never evaluated — so a managed
    /// file can carry these forward without breaking older binaries.
    #[serde(default)]
    pub command_rules: Vec<ManagedCommandRule>,
}

/// A forward-compatible, currently-inert command rule (BR-20/BR-21 will define
/// the semantics). Unknown fields are preserved verbatim rather than rejected.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ManagedCommandRule {
    /// The tool this rule scopes to (e.g. `developer__shell`). Optional so an
    /// unscoped/global rule is also representable.
    #[serde(default)]
    pub tool: Option<String>,
    /// Any additional predicate fields, preserved but not interpreted yet.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookEvent;

    const FULL: &str = r#"
allow_project_hooks: false
hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "./managed-guard.sh"
  Stop:
    - hooks:
        - type: command
          command: "echo '{\"decision\":\"block\",\"reason\":\"finish the audit\"}'"
permissions:
  allow:
    - "developer__text_editor"
  ask:
    - "memory__.*"
  deny:
    - "developer__shell"
  command_rules:
    - tool: "developer__shell"
      deny_prefixes: ["rm -rf"]
"#;

    #[test]
    fn parses_full_managed_file() {
        let file: ManagedPolicyFile = serde_yaml::from_str(FULL).unwrap();
        assert_eq!(file.allow_project_hooks, Some(false));
        assert!(file.hooks.events.contains_key(&HookEvent::PreToolUse));
        assert!(file.hooks.events.contains_key(&HookEvent::Stop));
        assert_eq!(file.permissions.allow, vec!["developer__text_editor"]);
        assert_eq!(file.permissions.deny, vec!["developer__shell"]);
        assert_eq!(file.permissions.command_rules.len(), 1);
        assert_eq!(
            file.permissions.command_rules[0].tool.as_deref(),
            Some("developer__shell")
        );
        // The inert extra predicate is preserved but uninterpreted.
        assert!(file.permissions.command_rules[0]
            .extra
            .contains_key("deny_prefixes"));
    }

    #[test]
    fn empty_file_is_default() {
        let file: ManagedPolicyFile = serde_yaml::from_str("{}").unwrap();
        assert_eq!(file, ManagedPolicyFile::default());
        assert_eq!(file.allow_project_hooks, None);
        assert!(file.permission_for("developer__shell").is_none());
    }

    #[test]
    fn deny_beats_ask_beats_allow() {
        // A tool listed in all three resolves to the most restrictive (deny).
        let file: ManagedPolicyFile = serde_yaml::from_str(
            "permissions:\n  allow: [\"t\"]\n  ask: [\"t\"]\n  deny: [\"t\"]\n",
        )
        .unwrap();
        assert_eq!(file.permission_for("t"), Some(ManagedVerdict::Deny));

        let file: ManagedPolicyFile =
            serde_yaml::from_str("permissions:\n  allow: [\"t\"]\n  ask: [\"t\"]\n").unwrap();
        assert_eq!(file.permission_for("t"), Some(ManagedVerdict::Ask));

        let file: ManagedPolicyFile =
            serde_yaml::from_str("permissions:\n  allow: [\"t\"]\n").unwrap();
        assert_eq!(file.permission_for("t"), Some(ManagedVerdict::Allow));
    }

    #[test]
    fn matcher_regex_and_exact_both_apply() {
        let file: ManagedPolicyFile =
            serde_yaml::from_str("permissions:\n  deny: [\"developer__shell\", \"memory__.*\"]\n")
                .unwrap();
        assert_eq!(
            file.permission_for("developer__shell"),
            Some(ManagedVerdict::Deny)
        );
        assert_eq!(
            file.permission_for("memory__store"),
            Some(ManagedVerdict::Deny)
        );
        assert!(file.permission_for("developer__text_editor").is_none());
    }

    /// A `dictation:` block whose contents no binary understands still parses,
    /// so the rest of the file keeps its force. The block itself is judged by
    /// `managed::dictation`, which turns dictation off for it.
    #[test]
    fn a_malformed_dictation_block_does_not_fail_the_file() {
        for block in [
            "dictation: [not, a, map]\n",
            "dictation: 42\n",
            "dictation:\n  enabled: maybe\n  future_key: { nested: true }\n",
        ] {
            let yaml = format!("{block}permissions:\n  deny: [\"developer__shell\"]\n");
            let file: ManagedPolicyFile = serde_yaml::from_str(&yaml)
                .unwrap_or_else(|e| panic!("{block:?} failed the whole file: {e}"));
            assert!(file.dictation.is_some(), "{block:?}");
            assert_eq!(
                file.permission_for("developer__shell"),
                Some(ManagedVerdict::Deny),
                "{block:?} lifted permissions.deny"
            );
        }
    }

    #[test]
    fn a_file_without_a_dictation_key_has_no_block() {
        let file: ManagedPolicyFile =
            serde_yaml::from_str("permissions:\n  deny: [\"x\"]\n").unwrap();
        assert!(file.dictation.is_none());
    }

    /// A `dictation:` key with an empty value is present, not absent, so it
    /// reaches `managed::dictation` (which turns dictation off for it) instead
    /// of reading as a file that says nothing about dictation.
    #[test]
    fn an_empty_dictation_key_is_kept_as_null() {
        for yaml in [
            "dictation:\n",
            "dictation: ~\n",
            "dictation: null\n",
            // A block whose keys lost their indentation: `enabled` lands at the
            // top level, where unknown keys are ignored.
            "dictation:\nenabled: false\n",
        ] {
            let yaml = format!("{yaml}permissions:\n  deny: [developer__shell]\n");
            let file: ManagedPolicyFile = serde_yaml::from_str(&yaml)
                .unwrap_or_else(|e| panic!("{yaml:?} failed the whole file: {e}"));
            assert_eq!(file.dictation, Some(serde_yaml::Value::Null), "{yaml:?}");
            assert_eq!(
                file.permission_for("developer__shell"),
                Some(ManagedVerdict::Deny),
                "{yaml:?} lifted permissions.deny"
            );
        }
    }

    #[test]
    fn unknown_top_level_keys_are_ignored() {
        // Forward-compat: a future key must not fail the whole parse.
        let file: ManagedPolicyFile =
            serde_yaml::from_str("future_setting: 42\npermissions:\n  deny: [\"x\"]\n").unwrap();
        assert_eq!(file.permission_for("x"), Some(ManagedVerdict::Deny));
    }
}
