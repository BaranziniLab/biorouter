//! Declarative policy rule types + matching logic.
//!
//! Rules are plain data (deserialized from YAML) so they are auditable and
//! self-testable, replacing the inline `THREAT_PATTERNS` `const`. Each rule
//! carries embedded `tests` (Codex `execpolicy` style) that the engine runs at
//! load time and `cargo test` enforces.

use regex::Regex;
use serde::Deserialize;

use super::command::ParsedCommand;
use super::target::{Blast, Dialect, Platform};

/// The three first-class outcomes a rule can assert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

/// Rule authority tier (ascending). Higher tiers win ties; Slice 1 ships only
/// `Baseline` (embedded), but the ordering is defined now so the external-file
/// loader in a later slice slots in without a data-model change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Baseline,
    User,
    Project,
    Admin,
}

impl Tier {
    /// Base of the effective-priority space; a rule's own `priority` is added on
    /// top, so an Admin rule always outranks any Baseline/User/Project rule.
    pub fn base_priority(&self) -> u32 {
        match self {
            Tier::Baseline => 0,
            Tier::User => 2000,
            Tier::Project => 3000,
            Tier::Admin => 4000,
        }
    }
}

/// The criteria that decide whether a rule applies to a tool call. A field left
/// unset (`None`) is a wildcard for that dimension. `binary` and `path_glob`
/// must be satisfied by the *same* pipeline segment, so an unrelated later
/// command cannot supply a missing token (`rm -rf ./x && cd /etc`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RuleMatch {
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub binary: Option<Vec<String>>,
    #[serde(default)]
    pub command_prefix: Option<Vec<String>>,
    #[serde(default)]
    pub arg_regex: Option<String>,
    #[serde(default)]
    pub path_glob: Option<Vec<String>>,
    #[serde(default)]
    pub pipes_to_shell: Option<bool>,
    /// BR-68: match a path argument by its *blast radius* rather than by
    /// enumerating `path_glob`s per platform. A rule with
    /// `target_blast: [root, system_dir, home_bare, wildcard_at_root]` fires on
    /// any target whose normalized blast is one of those — the discriminator
    /// that keeps `Remove-Item -Recurse -Force .\dist` allowed. Like `path_glob`,
    /// it must co-occur with `binary` in the *same* segment.
    #[serde(default)]
    pub target_blast: Option<Vec<Blast>>,
    /// BR-68: the segment reached an obfuscated PowerShell execution sink (the
    /// residue signal). Used by the single `ask`-tier obfuscation rule.
    #[serde(default)]
    pub obfuscated_exec: Option<bool>,
}

impl RuleMatch {
    /// A rule with no criteria at all would match every command — a footgun, so
    /// the loader rejects it.
    pub fn is_empty(&self) -> bool {
        self.tools.is_none()
            && self.binary.is_none()
            && self.command_prefix.is_none()
            && self.arg_regex.is_none()
            && self.path_glob.is_none()
            && self.pipes_to_shell.is_none()
            && self.target_blast.is_none()
            && self.obfuscated_exec.is_none()
    }
}

/// Embedded positive/negative examples, checked by `PolicyEngine::run_self_tests`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RuleTests {
    #[serde(default)]
    pub matches: Vec<String>,
    #[serde(default)]
    pub not_matches: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// A single declarative policy rule.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub justification: String,
    #[serde(rename = "match")]
    pub r#match: RuleMatch,
    pub decision: Decision,
    #[serde(default)]
    pub priority: u16,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tier: Tier,
    /// BR-68: platforms this rule is in force on. Empty/absent = all platforms,
    /// so every pre-BR-68 baseline rule keeps firing everywhere (`rm -rf /etc`
    /// typed into `pwsh` on Linux is still `rm -rf /etc`).
    #[serde(default)]
    pub platforms: Vec<Platform>,
    /// BR-68: shell dialects this rule understands. Empty/absent = all dialects.
    #[serde(default)]
    pub shells: Vec<Dialect>,
    #[serde(default)]
    pub tests: RuleTests,
}

impl Rule {
    /// Effective priority used to resolve competing rules across tiers.
    pub fn effective_priority(&self) -> u32 {
        self.tier.base_priority() + self.priority as u32
    }
}

/// The engine's decision for one tool call.
#[derive(Debug, Clone)]
pub struct PolicyVerdict {
    pub decision: Decision,
    /// Which rule fired (`None` = no rule matched -> default allow).
    pub rule_id: Option<String>,
    pub justification: String,
    /// `POL-<uuid>` (replaces the old `SEC-<uuid>` for command governance).
    pub finding_id: String,
}

/// A tiny glob matcher (`*`, `**`, `?`) compiled to a regex. `*` stops at `/`,
/// `**` (or a `/**` suffix) spans path separators, so `/etc/**` matches both
/// `/etc` and everything beneath it.
#[derive(Debug, Clone)]
pub struct Glob {
    re: Regex,
}

impl Glob {
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            re: Regex::new(&glob_to_regex(pattern))?,
        })
    }

    pub fn matches(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    pub fn matches_path(&self, path: &std::path::Path) -> bool {
        let s = path.to_string_lossy().replace('\\', "/");
        self.re.is_match(&s)
    }
}

fn glob_to_regex(glob: &str) -> String {
    // Sentinels keep the multi-char `/**` / `**` cases out of the char loop.
    let prepared = glob.replace("/**", "\u{1}").replace("**", "\u{2}");
    let mut re = String::from("^");
    for c in prepared.chars() {
        match c {
            '\u{1}' => re.push_str("(/.*)?"),
            '\u{2}' => re.push_str(".*"),
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            other => re.push(other),
        }
    }
    re.push('$');
    re
}

/// A rule with its regex/globs pre-compiled, ready for hot-path evaluation.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub rule: Rule,
    arg_re: Option<Regex>,
    tool_globs: Option<Vec<Glob>>,
    path_globs: Option<Vec<Glob>>,
}

impl CompiledRule {
    /// Compile a rule, returning an error string (for logging) on a bad
    /// regex/glob or an empty match — a broken rule must not silently pass.
    pub fn compile(rule: Rule) -> Result<Self, String> {
        if rule.r#match.is_empty() {
            return Err(format!("rule '{}' has no match criteria", rule.id));
        }

        let arg_re = match &rule.r#match.arg_regex {
            Some(src) => Some(
                Regex::new(src).map_err(|e| format!("rule '{}' bad arg_regex: {e}", rule.id))?,
            ),
            None => None,
        };
        let tool_globs = compile_globs(&rule.id, "tools", &rule.r#match.tools)?;
        let path_globs = compile_globs(&rule.id, "path_glob", &rule.r#match.path_glob)?;

        Ok(Self {
            rule,
            arg_re,
            tool_globs,
            path_globs,
        })
    }

    /// Whether this rule applies to `(tool, cmd)`. All specified dimensions must
    /// hold (AND); `binary`, `path_glob`, `target_blast` and the dialect gate
    /// must co-occur in one segment (BR-68 keeps this co-occurrence — it is why
    /// `rm -rf ./x && cd /etc` is not fooled).
    pub fn matches(&self, tool: &str, cmd: &ParsedCommand) -> bool {
        // BR-68 platform gate: cheapest early-out. A rule with no `platforms`
        // applies everywhere.
        if !self.rule.platforms.is_empty() && !self.rule.platforms.contains(&cmd.platform) {
            return false;
        }
        if let Some(globs) = &self.tool_globs {
            // Every spelling of the tool: a rule written for a tool before its
            // extension was renamed still governs it, so a deny cannot be lost
            // by the rename (`config::extension_renames`).
            let spellings = crate::config::extension_renames::tool_name_aliases(tool);
            if !spellings
                .iter()
                .any(|spelling| globs.iter().any(|g| g.matches(spelling)))
            {
                return false;
            }
        }
        if let Some(re) = &self.arg_re {
            if !re.is_match(&cmd.raw) {
                return false;
            }
        }
        if let Some(expected) = self.rule.r#match.pipes_to_shell {
            if cmd.reads_shell != expected {
                return false;
            }
        }
        if let Some(prefixes) = &self.rule.r#match.command_prefix {
            let hit = cmd.segments.iter().any(|seg| {
                self.dialect_ok(seg) && {
                    let line = seg.command_line();
                    prefixes.iter().any(|p| line.starts_with(p.trim()))
                }
            });
            if !hit {
                return false;
            }
        }

        let binaries = &self.rule.r#match.binary;
        let blasts = &self.rule.r#match.target_blast;
        let obf = self.rule.r#match.obfuscated_exec;
        if binaries.is_some() || self.path_globs.is_some() || blasts.is_some() || obf.is_some() {
            let ok = cmd.segments.iter().any(|seg| {
                if !self.dialect_ok(seg) {
                    return false;
                }
                let bin_ok = binaries
                    .as_ref()
                    .is_none_or(|bins| bins.iter().any(|b| b.eq_ignore_ascii_case(&seg.binary)));
                let path_ok = self.path_globs.as_ref().is_none_or(|globs| {
                    seg.paths
                        .iter()
                        .any(|p| globs.iter().any(|g| g.matches_path(p)))
                });
                let blast_ok = blasts
                    .as_ref()
                    .is_none_or(|wanted| seg.targets.iter().any(|t| wanted.contains(&t.blast)));
                let obf_ok = obf.is_none_or(|want| seg.obfuscated_exec == want);
                bin_ok && path_ok && blast_ok && obf_ok
            });
            if !ok {
                return false;
            }
        }

        true
    }

    /// A rule's `shells` gate, evaluated per segment (a segment carries the
    /// dialect that will interpret it). Empty `shells` = all dialects.
    fn dialect_ok(&self, seg: &super::command::Segment) -> bool {
        self.rule.shells.is_empty() || self.rule.shells.contains(&seg.dialect)
    }
}

fn compile_globs(
    rule_id: &str,
    field: &str,
    patterns: &Option<Vec<String>>,
) -> Result<Option<Vec<Glob>>, String> {
    match patterns {
        None => Ok(None),
        Some(pats) => {
            let mut globs = Vec::with_capacity(pats.len());
            for p in pats {
                globs.push(
                    Glob::new(p).map_err(|e| format!("rule '{rule_id}' bad {field} '{p}': {e}"))?,
                );
            }
            Ok(Some(globs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_for_tools(tools: &[&str]) -> CompiledRule {
        let tools: Vec<String> = tools.iter().map(|tool| format!("\"{tool}\"")).collect();
        let yaml = format!(
            "id: renamed-tool\nmatch:\n  tools: [{}]\ndecision: deny\n",
            tools.join(", ")
        );
        CompiledRule::compile(serde_yaml::from_str(&yaml).unwrap()).unwrap()
    }

    /// I4 of the gptr rename: a policy rule whose tool globs name an old tool
    /// still applies to the renamed tool, so a deny cannot be lost by the
    /// rename, and it reaches no other tool.
    #[test]
    fn a_tool_glob_written_for_an_old_tool_name_matches_the_renamed_tool() {
        let cmd = ParsedCommand::parse("echo hi", std::path::Path::new("/tmp"));
        assert!(rule_for_tools(&["gptragent__r"]).matches("gptr__r", &cmd));
        assert!(rule_for_tools(&["gptragent__*"]).matches("gptr__inspect", &cmd));
        assert!(rule_for_tools(&["gptr__r"]).matches("gptragent__r", &cmd));
        assert!(!rule_for_tools(&["gptragent__r"]).matches("gptr__inspect", &cmd));
        assert!(!rule_for_tools(&["gptragent__*"]).matches("developer__shell", &cmd));
        assert!(rule_for_tools(&["developer__shell"]).matches("developer__shell", &cmd));
    }
}
