//! BR-18: per-action risk grading — the signal that makes `SmartApprove`
//! actually *smart* instead of a second name for `Approve`.
//!
//! Before this module, `PermissionInspector` carried two `HashSet<String>`s
//! (`readonly_tools` / `regular_tools`) that were constructed empty at
//! `agent.rs` with a "will be populated from extension manager" comment and had
//! no setter and no second construction site. The read-only short-circuit could
//! therefore never fire, the `read_only_hint` annotations every extension
//! carefully sets were ignored by the permission layer, and both gating modes
//! prompted on *every* call — including a plain file read.
//!
//! The replacement is a free, no-LLM signal: grade each tool from the MCP
//! `ToolAnnotations` the server already publishes, keep the grades in a
//! registry refreshed from the exact tool list handed to the model each turn,
//! and let the smart tier auto-approve only what grades `Low` (read-only).
//!
//! Grading is **fail-closed** in every ambiguous direction:
//! * a tool that claims *both* `read_only_hint: true` and
//!   `destructive_hint: true` is self-contradictory → [`ToolRisk::High`];
//! * a tool that says it writes but says nothing about destructiveness inherits
//!   MCP's own default for a non-read-only tool (destructive) → `High`;
//! * a tool with no annotations at all, or one the registry has never seen, is
//!   [`ToolRisk::Unknown`] → confirmed by default.
//!
//! Modelled on OpenHands' per-action `security_risk` + `ConfirmRisky(threshold,
//! confirm_unknown)` confirmation policy.

use std::collections::HashMap;
use std::sync::RwLock;

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::config::Config;

/// Per-action risk grade for a tool call.
///
/// The `Ord` derive is load-bearing: `Low < Medium < High < Unknown`, so an
/// un-gradeable tool sorts above every graded one and a naive `>= threshold`
/// comparison already fails closed. [`SmartApproveConfig::requires_confirmation`]
/// still treats `Unknown` explicitly so it can be opted out of.
///
/// BR-63 also ships the grade to the confirmation card, so it is a wire type:
/// it serialises as `"low" | "medium" | "high" | "unknown"`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ToolRisk {
    /// Read-only: retrieves information without mutating data or state.
    Low,
    /// Mutating but non-destructive (the tool says so itself).
    Medium,
    /// Destructive, or a writer that never disclaimed destructiveness.
    High,
    /// No usable annotation — the tool never told us. Fail closed.
    #[default]
    Unknown,
}

impl ToolRisk {
    /// Parse a configured threshold. Unrecognised values fall back to `Medium`
    /// (the safe default) rather than silently widening the auto-approve set.
    fn parse_threshold(raw: &str) -> ToolRisk {
        match raw.trim().to_ascii_lowercase().as_str() {
            "low" => ToolRisk::Low,
            "high" => ToolRisk::High,
            _ => ToolRisk::Medium,
        }
    }

    /// Short human-readable label used in approval reasons.
    pub fn label(self) -> &'static str {
        match self {
            ToolRisk::Low => "low",
            ToolRisk::Medium => "medium",
            ToolRisk::High => "high",
            ToolRisk::Unknown => "unknown",
        }
    }
}

/// Grade a tool from its MCP annotations.
///
/// See the module docs for why every ambiguous case grades upward.
pub fn grade_annotations(annotations: Option<&ToolAnnotations>) -> ToolRisk {
    let Some(a) = annotations else {
        return ToolRisk::Unknown;
    };

    // A destructive claim always wins, even over a (contradictory) read-only
    // claim — a mis-annotated "read-only" tool that also admits to destruction
    // must not slip into the auto-approve set.
    if a.destructive_hint == Some(true) {
        return ToolRisk::High;
    }

    if a.read_only_hint == Some(true) {
        return ToolRisk::Low;
    }

    match (a.read_only_hint, a.destructive_hint) {
        // Explicitly a non-destructive writer.
        (_, Some(false)) => ToolRisk::Medium,
        // Says it writes, says nothing about destructiveness. MCP's own default
        // for a non-read-only tool is `destructive: true` — honour it.
        (Some(false), None) => ToolRisk::High,
        // Annotations present but neither hint set (e.g. only a title).
        _ => ToolRisk::Unknown,
    }
}

/// Grade a whole [`Tool`].
pub fn grade_tool(tool: &Tool) -> ToolRisk {
    grade_annotations(tool.annotations.as_ref())
}

/// The smart tier's confirmation policy (OpenHands `ConfirmRisky`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartApproveConfig {
    /// Master switch for the whole BR-18 risk tier. `false` restores the old
    /// behaviour (SmartApprove == Approve: confirm everything).
    /// `SMART_APPROVE_RISK_GRADING`, default `true`.
    pub enabled: bool,
    /// Lowest grade that still requires a human. Grades `>= threshold` confirm.
    /// `SMART_APPROVE_RISK_THRESHOLD` = `low` | `medium` | `high`, default
    /// `medium` — i.e. only read-only (`Low`) calls auto-approve.
    pub threshold: ToolRisk,
    /// Whether an un-gradeable tool must be confirmed.
    /// `SMART_APPROVE_CONFIRM_UNKNOWN`, default `true` (fail closed).
    pub confirm_unknown: bool,
    /// Consult the LLM permission judge for tools the annotations cannot grade,
    /// applying its verdict only to the inspected invocation.
    /// `SMART_APPROVE_JUDGE`, default `false` — it costs a provider round-trip
    /// on the critical path, so it is opt-in.
    pub judge_unknown: bool,
}

impl Default for SmartApproveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: ToolRisk::Medium,
            confirm_unknown: true,
            judge_unknown: false,
        }
    }
}

impl SmartApproveConfig {
    /// Read the policy from config/env, falling back to the safe defaults.
    pub fn from_config() -> Self {
        let config = Config::global();
        let defaults = Self::default();
        Self {
            enabled: config
                .get_param::<bool>("SMART_APPROVE_RISK_GRADING")
                .unwrap_or(defaults.enabled),
            threshold: config
                .get_param::<String>("SMART_APPROVE_RISK_THRESHOLD")
                .map(|raw| ToolRisk::parse_threshold(&raw))
                .unwrap_or(defaults.threshold),
            confirm_unknown: config
                .get_param::<bool>("SMART_APPROVE_CONFIRM_UNKNOWN")
                .unwrap_or(defaults.confirm_unknown),
            judge_unknown: config
                .get_param::<bool>("SMART_APPROVE_JUDGE")
                .unwrap_or(defaults.judge_unknown),
        }
    }

    /// Whether a call of this grade needs a human.
    pub fn requires_confirmation(&self, risk: ToolRisk) -> bool {
        match risk {
            ToolRisk::Unknown => self.confirm_unknown,
            graded => graded >= self.threshold,
        }
    }
}

/// Tool-name → risk grade, refreshed from the exact tool list handed to the
/// model each turn (`prepare_tools_and_prompt`), so platform/frontend tools and
/// freshly-enabled extensions are graded too — not just whatever the extension
/// manager happened to expose at agent construction.
///
/// A tool the registry has never seen grades [`ToolRisk::Unknown`], so a tool
/// call arriving before any refresh (or naming a tool that no longer exists)
/// fails closed.
#[derive(Debug, Default)]
pub struct ToolRiskRegistry {
    risks: RwLock<HashMap<String, ToolRisk>>,
}

impl ToolRiskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the table with grades derived from `tools`.
    pub fn refresh_from_tools(&self, tools: &[Tool]) {
        let graded: HashMap<String, ToolRisk> = tools
            .iter()
            .map(|tool| (tool.name.to_string(), grade_tool(tool)))
            .collect();
        if let Ok(mut risks) = self.risks.write() {
            *risks = graded;
        }
    }

    /// The grade for `tool_name`; [`ToolRisk::Unknown`] when unseen.
    pub fn risk_for(&self, tool_name: &str) -> ToolRisk {
        self.risks
            .read()
            .ok()
            .and_then(|risks| risks.get(tool_name).copied())
            .unwrap_or(ToolRisk::Unknown)
    }

    /// Test/diagnostic helper: number of graded tools.
    pub fn len(&self) -> usize {
        self.risks.read().map(|r| r.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::object;
    use std::sync::Arc;

    fn tool(name: &str, annotations: Option<ToolAnnotations>) -> Tool {
        let t = Tool::new(
            name.to_string(),
            "desc".to_string(),
            Arc::new(object!({"type": "object", "properties": {}})),
        );
        match annotations {
            Some(a) => t.annotate(a),
            None => t,
        }
    }

    fn annot(read_only: Option<bool>, destructive: Option<bool>) -> ToolAnnotations {
        ToolAnnotations {
            title: None,
            read_only_hint: read_only,
            destructive_hint: destructive,
            idempotent_hint: None,
            open_world_hint: None,
        }
    }

    #[test]
    fn read_only_annotation_grades_low() {
        assert_eq!(
            grade_annotations(Some(&annot(Some(true), Some(false)))),
            ToolRisk::Low
        );
        // read-only with destructiveness unstated is still read-only
        assert_eq!(
            grade_annotations(Some(&annot(Some(true), None))),
            ToolRisk::Low
        );
    }

    #[test]
    fn destructive_annotation_grades_high_even_when_also_read_only() {
        // Self-contradictory annotation: fail closed on the destructive claim.
        assert_eq!(
            grade_annotations(Some(&annot(Some(true), Some(true)))),
            ToolRisk::High
        );
        assert_eq!(
            grade_annotations(Some(&annot(Some(false), Some(true)))),
            ToolRisk::High
        );
    }

    #[test]
    fn non_destructive_writer_grades_medium() {
        assert_eq!(
            grade_annotations(Some(&annot(Some(false), Some(false)))),
            ToolRisk::Medium
        );
    }

    #[test]
    fn writer_without_destructive_hint_grades_high() {
        // MCP's default for a non-read-only tool is destructive: true.
        assert_eq!(
            grade_annotations(Some(&annot(Some(false), None))),
            ToolRisk::High
        );
    }

    #[test]
    fn missing_or_empty_annotations_grade_unknown() {
        assert_eq!(grade_annotations(None), ToolRisk::Unknown);
        assert_eq!(
            grade_annotations(Some(&annot(None, None))),
            ToolRisk::Unknown
        );
    }

    #[test]
    fn registry_grades_from_the_tool_list_and_fails_closed_on_unseen() {
        let registry = ToolRiskRegistry::new();
        registry.refresh_from_tools(&[
            tool("dev__read_file", Some(annot(Some(true), Some(false)))),
            tool("dev__shell", Some(annot(Some(false), Some(true)))),
            tool("mystery__thing", None),
        ]);

        assert_eq!(registry.risk_for("dev__read_file"), ToolRisk::Low);
        assert_eq!(registry.risk_for("dev__shell"), ToolRisk::High);
        assert_eq!(registry.risk_for("mystery__thing"), ToolRisk::Unknown);
        // never seen at all
        assert_eq!(registry.risk_for("dev__nope"), ToolRisk::Unknown);
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn refresh_replaces_rather_than_merges() {
        let registry = ToolRiskRegistry::new();
        registry.refresh_from_tools(&[tool("a__x", Some(annot(Some(true), Some(false))))]);
        assert_eq!(registry.risk_for("a__x"), ToolRisk::Low);

        // Extension disabled; its tools must stop being auto-approvable.
        registry.refresh_from_tools(&[tool("b__y", Some(annot(Some(true), Some(false))))]);
        assert_eq!(registry.risk_for("a__x"), ToolRisk::Unknown);
        assert_eq!(registry.risk_for("b__y"), ToolRisk::Low);
    }

    #[test]
    fn default_policy_auto_approves_only_read_only() {
        let cfg = SmartApproveConfig::default();
        assert!(!cfg.requires_confirmation(ToolRisk::Low));
        assert!(cfg.requires_confirmation(ToolRisk::Medium));
        assert!(cfg.requires_confirmation(ToolRisk::High));
        assert!(cfg.requires_confirmation(ToolRisk::Unknown));
    }

    #[test]
    fn threshold_high_also_auto_approves_medium() {
        let cfg = SmartApproveConfig {
            threshold: ToolRisk::High,
            ..SmartApproveConfig::default()
        };
        assert!(!cfg.requires_confirmation(ToolRisk::Low));
        assert!(!cfg.requires_confirmation(ToolRisk::Medium));
        assert!(cfg.requires_confirmation(ToolRisk::High));
        // Unknown is governed by confirm_unknown, not the threshold.
        assert!(cfg.requires_confirmation(ToolRisk::Unknown));
    }

    #[test]
    fn threshold_low_confirms_everything() {
        let cfg = SmartApproveConfig {
            threshold: ToolRisk::Low,
            ..SmartApproveConfig::default()
        };
        assert!(cfg.requires_confirmation(ToolRisk::Low));
        assert!(cfg.requires_confirmation(ToolRisk::Medium));
    }

    #[test]
    fn confirm_unknown_can_be_opted_out() {
        let cfg = SmartApproveConfig {
            confirm_unknown: false,
            ..SmartApproveConfig::default()
        };
        assert!(!cfg.requires_confirmation(ToolRisk::Unknown));
        assert!(cfg.requires_confirmation(ToolRisk::High));
    }

    #[test]
    fn unparseable_threshold_falls_back_to_medium() {
        assert_eq!(ToolRisk::parse_threshold("banana"), ToolRisk::Medium);
        assert_eq!(ToolRisk::parse_threshold(""), ToolRisk::Medium);
        assert_eq!(ToolRisk::parse_threshold("  HIGH "), ToolRisk::High);
        assert_eq!(ToolRisk::parse_threshold("Low"), ToolRisk::Low);
    }

    #[test]
    fn unknown_sorts_above_every_graded_risk() {
        assert!(ToolRisk::Low < ToolRisk::Medium);
        assert!(ToolRisk::Medium < ToolRisk::High);
        assert!(ToolRisk::High < ToolRisk::Unknown);
        assert_eq!(ToolRisk::default(), ToolRisk::Unknown);
    }
}
