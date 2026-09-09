use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper, Result};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::CompletionCache;
use biorouter::agents::resource_refs::{reference_marker, RefKind};
use biorouter::config::{get_enabled_extensions, paths::Paths};

/// Available in-session slash commands, in the order they should be offered.
/// Keep in sync with `input::handle_slash_command`.
pub(crate) const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/clear",
    "/compact",
    "/diverge",
    "/rename",
    "/goal",
    "/loop",
    "/schedule",
    "/mode",
    "/plan",
    "/endplan",
    "/workflow",
    "/extension",
    "/builtin",
    "/t",
    "/r",
    "/exit",
    "/quit",
    "/?",
];

fn strip_yaml_quotes(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return inner.trim().to_string();
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return inner.trim().to_string();
    }
    value.to_string()
}

fn frontmatter_name(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let name = strip_yaml_quotes(value);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn skill_name_from_file(skill_md: &Path, fallback: &str) -> String {
    std::fs::read_to_string(skill_md)
        .ok()
        .and_then(|contents| frontmatter_name(&contents))
        .unwrap_or_else(|| fallback.to_string())
}

fn configured_skill_dirs() -> Vec<PathBuf> {
    let mut dirs = BTreeSet::new();

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.insert(home.join(".config/biorouter/skills"));
        dirs.insert(home.join(".claude/skills"));
        dirs.insert(home.join(".config/agents/skills"));
    }
    dirs.insert(biorouter::agents::skills_extension::skills_root(
        &Paths::config_dir(),
    ));

    dirs.into_iter().collect()
}

fn skill_reference_names_from_dirs(dirs: &[PathBuf]) -> Vec<String> {
    let mut names = BTreeSet::new();

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let fallback = entry.file_name().to_string_lossy().to_string();
            let skill_md = folder.join("SKILL.md");
            if skill_md.is_file() {
                names.insert(skill_name_from_file(&skill_md, &fallback));
                continue;
            }

            let Ok(sub_entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            if sub_entries
                .flatten()
                .any(|sub| sub.path().join("SKILL.md").is_file())
            {
                names.insert(fallback);
            }
        }
    }

    names.into_iter().collect()
}

pub(crate) fn list_skill_reference_names() -> Vec<String> {
    skill_reference_names_from_dirs(&configured_skill_dirs())
}

fn enabled_extension_reference_names() -> Vec<String> {
    const COMPACT_EXTENSION_CANONICALS: &[&str] =
        &["agent_drafter", "autovisualiser", "Extension Manager"];

    let mut names: Vec<String> = get_enabled_extensions()
        .into_iter()
        .map(|extension| extension.name())
        .filter(|name| {
            !COMPACT_EXTENSION_CANONICALS
                .iter()
                .any(|canonical| name.eq_ignore_ascii_case(canonical))
        })
        .collect();
    names.extend(
        biorouter_mcp::BUILTIN_EXTENSIONS
            .keys()
            .filter(|name| !COMPACT_EXTENSION_CANONICALS.contains(name))
            .map(|name| (*name).to_string()),
    );
    names.extend([
        "agentdrafter".to_string(),
        "autovisualizer".to_string(),
        "extensionmanager".to_string(),
    ]);
    names.sort();
    names.dedup();
    names
}

/// The reference to insert for an extension named `name` (issue #65, CLI half —
/// the desktop composer shares the rule via `resource_refs::reference_marker`).
///
/// The names offered here are `ExtensionConfig::name()`, which for a Platform
/// extension is its DISPLAY string, and two of those contain a space
/// (`Extension Manager`, `Chat Recall`); a user extension may have one too, and
/// a user *skill* may contain anything a YAML scalar can hold.
///
/// #60 handled the spaced case by deleting the whitespace, which worked only
/// because every `/ext:` consumer re-normalises whitespace away — a coincidence
/// that does not hold for skills, where `loadSkill` matches the frontmatter name
/// exactly. `reference_marker` replaces that with one rule for all three kinds:
/// keep the compact marker when the real extractor proves it round-trips, and
/// emit the canonical `<biorouter-ref …>` tag when it does not.
pub(super) fn extension_marker(name: &str) -> String {
    reference_marker(RefKind::Extension, name)
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let mut chars = needle.chars();
    let Some(mut current) = chars.next() else {
        return true;
    };
    for hay in haystack.chars() {
        if hay == current {
            if let Some(next) = chars.next() {
                current = next;
            } else {
                return true;
            }
        }
    }
    false
}

fn reference_matches_query(query: &str, key: &str, name: &str, kind: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let searchable = format!("{key} {kind} {name}").to_lowercase();
    let compact_searchable = searchable
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '_' | '/' | ' '))
        .collect::<String>();
    let compact_query = query
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '_' | '/' | ' '))
        .collect::<String>();

    searchable.contains(query) || is_subsequence(&compact_query, &compact_searchable)
}

fn reference_pairs_from_names<I, J>(line: &str, skills: I, extensions: J) -> Vec<Pair>
where
    I: IntoIterator<Item = String>,
    J: IntoIterator<Item = String>,
{
    let query = line.trim_start_matches('/').to_lowercase();
    let mut pairs = Vec::new();

    for name in skills {
        let key = format!("skill:{name}");
        if reference_matches_query(&query, &key.to_lowercase(), &name.to_lowercase(), "skill") {
            pairs.push(Pair {
                display: format!("/{key}"),
                replacement: format!("{} ", reference_marker(RefKind::Skill, &name)),
            });
        }
    }

    for name in extensions {
        let key = format!("ext:{name}");
        if reference_matches_query(
            &query,
            &key.to_lowercase(),
            &name.to_lowercase(),
            "extension",
        ) {
            pairs.push(Pair {
                // The display keeps the full name — that is what the user is
                // picking and filtering on. Only the INSERTED text may differ;
                // see `extension_marker`.
                display: format!("/{key}"),
                replacement: format!("{} ", extension_marker(&name)),
            });
        }
    }

    pairs
}

/// Completion pairs for `/kb:<id>` knowledge-base references, matching how the
/// TUI sources them: list bases, drop hidden ones, and fuzzy-match on id + name.
/// Degrades to no completions if the knowledge store is missing or unreadable.
fn kb_reference_pairs(line: &str) -> Vec<Pair> {
    use biorouter::knowledge::service::KnowledgeService;

    let query = line.trim_start_matches('/').to_lowercase();
    let mut pairs = Vec::new();

    let Ok(service) = KnowledgeService::new_default() else {
        return pairs;
    };
    let Ok(bases) = service.list_bases() else {
        return pairs;
    };
    let hidden = service.get_hidden_persisted().unwrap_or_default();

    for base in bases.into_iter().filter(|b| !hidden.contains(&b.id)) {
        let key = format!("kb:{}", base.id);
        let searchable_name = format!("{} {}", base.id, base.name);
        if reference_matches_query(
            &query,
            &key.to_lowercase(),
            &searchable_name.to_lowercase(),
            "knowledge",
        ) {
            pairs.push(Pair {
                display: format!("/{key}"),
                replacement: format!("{} ", reference_marker(RefKind::KnowledgeBase, &base.id)),
            });
        }
    }

    pairs
}

/// Completer for biorouter CLI commands
pub struct BioRouterCompleter {
    _completion_cache: Arc<std::sync::RwLock<CompletionCache>>,
    filename_completer: FilenameCompleter,
}

impl BioRouterCompleter {
    /// Create a new BioRouterCompleter with a reference to the Session's completion cache
    pub fn new(completion_cache: Arc<std::sync::RwLock<CompletionCache>>) -> Self {
        Self {
            _completion_cache: completion_cache,
            filename_completer: FilenameCompleter::new(),
        }
    }

    /// Complete flags for the /mode command
    fn complete_mode_flags(&self, line: &str) -> Result<(usize, Vec<Pair>)> {
        let modes = ["auto", "approve", "smart_approve", "chat"];

        let parts: Vec<&str> = line.split_whitespace().collect();

        // If we're just after "/mode" with a space, show all options
        if line == "/mode " {
            return Ok((
                line.len(),
                modes
                    .iter()
                    .map(|mode| Pair {
                        display: mode.to_string(),
                        replacement: format!("{} ", mode),
                    })
                    .collect(),
            ));
        }

        // If we're typing a mode name, show the flags for that mode
        if parts.len() == 2 {
            let partial = parts[1].to_lowercase();
            return Ok((
                line.len() - partial.len(),
                modes
                    .iter()
                    .filter(|mode| mode.to_lowercase().starts_with(&partial.to_lowercase()))
                    .map(|mode| Pair {
                        display: mode.to_string(),
                        replacement: format!("{} ", mode),
                    })
                    .collect(),
            ));
        }

        // No completions available
        Ok((line.len(), vec![]))
    }

    /// Complete slash commands
    fn complete_slash_commands(&self, line: &str) -> Result<(usize, Vec<Pair>)> {
        // Find commands that match the prefix
        let mut matching_commands: Vec<Pair> = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(line))
            .map(|cmd| Pair {
                display: cmd.to_string(),
                replacement: format!("{} ", cmd), // Add a space after the command
            })
            .collect();

        if matching_commands
            .iter()
            .any(|candidate| candidate.display == line)
        {
            return Ok((0, matching_commands));
        }

        matching_commands.extend(reference_pairs_from_names(
            line,
            list_skill_reference_names(),
            enabled_extension_reference_names(),
        ));
        matching_commands.extend(kb_reference_pairs(line));

        if !matching_commands.is_empty() {
            return Ok((0, matching_commands));
        }

        // No command completions available
        Ok((line.len(), vec![]))
    }

    /// Complete file paths
    fn complete_file_path(&self, line: &str, ctx: &Context) -> Result<(usize, Vec<Pair>)> {
        let parts: Vec<&str> = line.split_whitespace().collect();

        if let Some(last_part) = parts.last() {
            // Skip filename completion for words starting with special characters
            if last_part.starts_with('/') && last_part.len() == 1 {
                // Just a slash - no completion
                return Ok((line.len(), vec![]));
            }

            if last_part.starts_with('-') || last_part.contains('=') {
                // Skip flag or key-value pairs
                return Ok((line.len(), vec![]));
            }

            // Complete the partial path
            let pos = line.len() - last_part.len();
            let (start, candidates) =
                self.filename_completer
                    .complete(last_part, last_part.len(), ctx)?;

            // Return the completion results, with adjusted position
            return Ok((pos + start, candidates));
        }

        Ok((line.len(), vec![]))
    }
}

impl Completer for BioRouterCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>)> {
        // If the cursor is not at the end of the line, don't try to complete
        if pos < line.len() {
            return Ok((pos, vec![]));
        }

        if line.starts_with("/mode") {
            return self.complete_mode_flags(line);
        }

        let token_start = line
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(index, ch)| index + ch.len_utf8())
            .unwrap_or(0);
        let token = line.get(token_start..).unwrap_or("");
        if let Some(token_body) = token.strip_prefix('/') {
            if token_body.contains('/') {
                return self.complete_file_path(line, ctx);
            }
            let (_, candidates) = self.complete_slash_commands(token)?;
            return Ok((token_start, candidates));
        }

        // For normal text (not slash commands), try file path completion
        self.complete_file_path(line, ctx)
    }
}

// Implement the Helper trait which is required by rustyline
impl Helper for BioRouterCompleter {}

// Implement required traits with default implementations
impl Hinter for BioRouterCompleter {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        // Inline "ghost" autofill for slash commands: as the user types `/co`,
        // show the dim remainder (`mpact`) of the first matching command. Tab
        // still opens the full candidate list.
        if pos != line.len() || !line.starts_with('/') || line.contains(' ') || line.len() < 2 {
            return None;
        }
        SLASH_COMMANDS
            .iter()
            .find(|cmd| cmd.starts_with(line) && **cmd != line)
            .and_then(|cmd| cmd.get(line.len()..).map(str::to_string))
    }
}

impl Highlighter for BioRouterCompleter {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // Style the hint text with a dim color
        let styled = console::Style::new().dim().apply_to(hint).to_string();
        Cow::Owned(styled)
    }

    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _cmd_kind: CmdKind) -> bool {
        false
    }
}

impl Validator for BioRouterCompleter {
    fn validate(
        &self,
        _ctx: &mut rustyline::validate::ValidationContext,
    ) -> Result<rustyline::validate::ValidationResult> {
        Ok(rustyline::validate::ValidationResult::Valid(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::agents::resource_refs::extracted_refs;
    use std::sync::{Arc, RwLock};

    // Helper function to create a test completion cache
    fn create_test_cache() -> Arc<RwLock<CompletionCache>> {
        Arc::new(RwLock::new(CompletionCache::new()))
    }

    #[test]
    fn test_complete_slash_commands() {
        let cache = create_test_cache();
        let completer = BioRouterCompleter::new(cache);

        // Test complete match
        let (pos, candidates) = completer.complete_slash_commands("/exit").unwrap();
        assert_eq!(pos, 0);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].display, "/exit");
        assert_eq!(candidates[0].replacement, "/exit ");

        // Test partial match
        let (pos, candidates) = completer.complete_slash_commands("/e").unwrap();
        assert_eq!(pos, 0);
        // There might be multiple commands starting with "e" like "/exit" and "/extension"
        assert!(!candidates.is_empty());

        // Test multiple matches
        let (pos, candidates) = completer.complete_slash_commands("/").unwrap();
        assert_eq!(pos, 0);
        assert!(candidates.len() > 1);

        // Test no match
        let (_pos, candidates) = completer.complete_slash_commands("/nonexistent").unwrap();
        assert_eq!(candidates.len(), 0);
    }

    #[test]
    fn test_reference_pairs_complete_to_routed_markers() {
        let pairs = reference_pairs_from_names(
            "/skill:lit",
            vec!["literature-review".to_string(), "debugging".to_string()],
            vec!["pubmed".to_string()],
        );

        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].display, "/skill:literature-review");
        assert_eq!(pairs[0].replacement, "/skill:literature-review ");

        let pairs = reference_pairs_from_names(
            "/pm",
            vec!["literature-review".to_string()],
            vec!["pubmed".to_string()],
        );
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].replacement, "/ext:pubmed ");
    }

    /// Issue #60's case, now held to issue #65's standard.
    ///
    /// The names come from `enabled_extension_reference_names`, which is
    /// `get_enabled_extensions().map(|e| e.name())`, and a Platform extension's
    /// `name` is its DISPLAY string: `chatrecall` is registered as
    /// `"Chat Recall"` (`agents::chatrecall_extension::EXTENSION_NAME`). It is
    /// not one of the three names `COMPACT_EXTENSION_CANONICALS` swaps for a
    /// compact alias, so nothing rescues it — and any user extension may have a
    /// space too.
    ///
    /// #60 inserted `/ext:ChatRecall`, deleting the whitespace. That resolved,
    /// but only because every `/ext:` consumer re-normalises whitespace away;
    /// it is not the name the user picked, and the same trick applied to a
    /// skill resolves to nothing. The insert is now the canonical tag, which
    /// carries `Chat Recall` exactly — a strictly stronger guarantee than the
    /// assertion this test used to make.
    ///
    /// The display string keeps its space either way: that is what the user is
    /// picking and filtering on.
    #[test]
    fn extension_completion_carries_a_spaced_name_intact() {
        let pairs = reference_pairs_from_names(
            "/chat",
            Vec::<String>::new(),
            vec!["Chat Recall".to_string()],
        );

        assert_eq!(
            pairs.len(),
            1,
            "expected the Chat Recall extension; got {:?}",
            pairs
                .iter()
                .map(|p| p.replacement.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(pairs[0].display, "/ext:Chat Recall");
        assert_eq!(
            extracted_refs(RefKind::Extension, &pairs[0].replacement),
            vec!["Chat Recall".to_string()]
        );

        // ...and nothing but whitespace is collapsed.
        let pairs = reference_pairs_from_names(
            "/my_tool",
            Vec::<String>::new(),
            vec!["my_tool".to_string()],
        );
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].replacement, "/ext:my_tool ");
    }

    /// Issue #65, the CLI half — and the guard that supersedes the two tests
    /// above.
    ///
    /// #60 asked a weaker question: does the inserted marker survive
    /// `extract_inline_refs`' whitespace split *as a token*. `/ext:ChatRecall`
    /// passes that and still resolves, because every `/ext:` consumer
    /// re-normalises whitespace away anyway. Skills have no such escape hatch —
    /// `loadSkill` looks the frontmatter name up exactly — so the question this
    /// completer actually has to answer is the strict one: does what it inserts
    /// come back out of the agent's extractor as the EXACT name the user
    /// picked. Anything less is how `/skill:my skill` became `my`.
    ///
    /// Asserted against `resource_refs::extracted_refs`, i.e. the real
    /// extractor, so this cannot drift from the parser the way a hand-copied
    /// rule would.
    #[test]
    fn every_offered_completion_round_trips_to_the_exact_name() {
        let skills = vec![
            "literature-review".to_string(),
            "my skill".to_string(),
            r#"say "hi""#.to_string(),
            "tom & jerry".to_string(),
            "trailing dot.".to_string(),
        ];
        let extensions = vec![
            "developer".to_string(),
            "my_tool".to_string(),
            "my-tool".to_string(),
            "Chat Recall".to_string(),
            "Extension Manager".to_string(),
            "my spaced tool".to_string(),
        ];

        let pairs = reference_pairs_from_names("/", skills.clone(), extensions.clone());
        assert_eq!(pairs.len(), skills.len() + extensions.len());

        let expected = skills
            .iter()
            .map(|name| (RefKind::Skill, name))
            .chain(extensions.iter().map(|name| (RefKind::Extension, name)));

        for (pair, (kind, name)) in pairs.iter().zip(expected) {
            assert_eq!(
                extracted_refs(kind, &pair.replacement),
                vec![name.clone()],
                "inserting `{}` for `{name}` does not reach the agent as `{name}`",
                pair.replacement
            );
        }
    }

    /// The other half of the rule, and the one a well-meaning simplification
    /// would break: the tag is used only where it is *needed*.
    ///
    /// Exactness is guarded above. This guards readability — a terminal has no
    /// chip to render a tag into, so collapsing `reference_marker` to "always
    /// emit the tag" would leave every CLI completion pasting 45 characters of
    /// markup into the user's prompt line for a name that never needed it.
    #[test]
    fn a_completion_uses_the_tag_only_when_the_compact_marker_cannot_carry_it() {
        let compact = vec![
            "developer".to_string(),
            "my_tool".to_string(),
            "my-tool".to_string(),
            "agentdrafter".to_string(),
        ];
        for pair in reference_pairs_from_names("/", Vec::<String>::new(), compact.clone()) {
            assert!(
                pair.replacement.starts_with("/ext:"),
                "`{}` did not need a tag",
                pair.replacement
            );
        }

        let needs_a_tag = vec![
            "Chat Recall".to_string(),
            "Extension Manager".to_string(),
            "my spaced tool".to_string(),
        ];
        for pair in reference_pairs_from_names("/", Vec::<String>::new(), needs_a_tag) {
            assert!(
                pair.replacement.starts_with("<biorouter-ref "),
                "`{}` would reach the resolver truncated",
                pair.replacement
            );
        }
    }

    #[test]
    fn test_slash_references_complete_mid_sentence() {
        let cache = create_test_cache();
        let completer = BioRouterCompleter::new(cache);
        let history = rustyline::history::DefaultHistory::new();
        let ctx = Context::new(&history);

        let (pos, candidates) = completer
            .complete("please use /agentdra", 20, &ctx)
            .unwrap();
        assert_eq!(pos, "please use ".len());
        assert!(candidates
            .iter()
            .any(|candidate| candidate.replacement == "/ext:agentdrafter "));
    }

    #[test]
    fn test_skill_reference_names_match_desktop_skill_layout() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("skills");
        std::fs::create_dir(&root).unwrap();

        let single = root.join("lit-review");
        std::fs::create_dir(&single).unwrap();
        std::fs::write(
            single.join("SKILL.md"),
            "---\nname: literature-review\ndescription: Search papers\n---\n",
        )
        .unwrap();

        let bundle = root.join("analysis-bundle");
        let bundled_skill = bundle.join("summarize");
        std::fs::create_dir_all(&bundled_skill).unwrap();
        std::fs::write(
            bundled_skill.join("SKILL.md"),
            "---\nname: summarizer\ndescription: Summarize\n---\n",
        )
        .unwrap();

        let names = skill_reference_names_from_dirs(&[root]);
        assert_eq!(
            names,
            vec![
                "analysis-bundle".to_string(),
                "literature-review".to_string()
            ]
        );
    }
}
