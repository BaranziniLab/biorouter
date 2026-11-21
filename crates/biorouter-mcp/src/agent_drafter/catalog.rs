//! What this Biorouter install actually has.
//!
//! Application Drafter's 12 tools were all create/configure/build/launch/read — **none
//! of them could ask what exists**. The authoring instructions nonetheless told
//! the model to give the app "the extensions/skills/knowledge it needs", and
//! `AgentConfig` had no field in which to say *"this app wants a ClinVar KB and
//! there isn't one"*. So the only way for the model to express the need was to
//! invent an id.
//!
//! It did, at scale. Across the 100-app test drive: 13 invented knowledge-base
//! ids, 7 nonexistent skill lists, and — memorably — the literal string `br.kb`,
//! which is the *client API namespace*, not an identifier. The runtime then made
//! it worse: the server armed the `skills` extension whenever the skill list was
//! non-empty (real or not) and wrote a system prompt commanding the model to use
//! skills that could not load, so turn 1 failed by construction.
//!
//! The model's error was epistemic — it could not see its environment — so no
//! amount of prompt text could fix it. This module is the missing eye. It backs:
//!
//!   * the `list_platform_catalog` tool (discovery),
//!   * write-boundary rejection in `create_app`/`configure_app` (an unknown id
//!     cannot be saved),
//!   * the `requires` manifest block (a typed slot for an unmet need), and
//!   * the runtime capability report (what was requested vs what is real).

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::knowledge::caller::KbCaller;

use super::BUILTIN_EXTENSION_NAMES;

/// An installed knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KbEntry {
    pub id: String,
    pub name: String,
}

/// An installed skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// An extension the app may name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtEntry {
    pub name: String,
    /// Built-in extensions ship with the daemon and are always available.
    pub builtin: bool,
}

/// The environment an app is being authored against.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Catalog {
    pub knowledge_bases: Vec<KbEntry>,
    pub skills: Vec<SkillEntry>,
    pub extensions: Vec<ExtEntry>,
}

impl Catalog {
    /// Scan this install, from the point of view of a caller with this
    /// capability (issue #56, **CP5**).
    ///
    /// Honours `BIOROUTER_PATH_ROOT` through [`crate::paths`], so a sandboxed
    /// run sees the sandbox's catalog — not the user's global one. (Before the
    /// path fix, an "isolated" test drive was reading the user's real knowledge
    /// bases.)
    ///
    /// A caller the barrier would refuse gets the base **omitted**, exactly as
    /// `kb_list_bases` does: a KB id and name are user-authored and routinely
    /// name a cohort or a study, so they are content and not an existence side
    /// channel. (DR-7 covers the latter and this does not chase it — a caller
    /// that guesses an id and asks about that one id can still be answered
    /// truthfully; volunteering the whole list is the crossing.)
    ///
    /// **The filter lives here and not in the validators.** There are three
    /// knowledge-base renderings in `validate`, plus `has_kb`, plus the runtime
    /// report's `missing_knowledge_base` — a base that is simply *absent from the
    /// catalog* fixes every one of them with no second rule to keep in sync, and
    /// produces the right words for free: a public session that names a private
    /// base by hand is told it "is not installed on this Biorouter", which is
    /// omission rather than a refusal that would itself confirm the base exists.
    ///
    /// ⚠ **Audit finding 17, second spelling.** This took a bare
    /// `caller_is_private: bool` and asked the tier axis alone, so a chat bound
    /// to a model covered by another institution's agreements was handed
    /// `{id, name}` for a base that `br.kb` (CP3) and `export_app` (CP4) — the
    /// two barriers this catalogue feeds — would then refuse. The drafting model
    /// learns KB ids from here and nowhere else; a listing that names what the
    /// barrier refuses is the leak, and the refusal is merely the tell. It now
    /// carries [`KbCaller`], which cannot express half a caller, and asks
    /// [`KbCaller::can_reach`] — the barrier, negated.
    ///
    /// A `biorouter-mcp` type and not `ProviderTier` + `ModelAffiliation`
    /// because this crate cannot depend on `biorouter`; `routes/apps.rs` does
    /// the crossing at its call sites.
    pub fn discover(caller: &KbCaller) -> Self {
        Self {
            knowledge_bases: discover_kbs(caller),
            skills: discover_skills(),
            extensions: BUILTIN_EXTENSION_NAMES
                .iter()
                .map(|name| ExtEntry {
                    name: (*name).to_string(),
                    builtin: true,
                })
                .chain(discover_external_extensions())
                .collect(),
        }
    }

    /// An empty catalog — for tests, and for the "nothing is installed" case that
    /// the test drive ran against without knowing it.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn has_kb(&self, id: &str) -> bool {
        self.knowledge_bases.iter().any(|k| k.id == id)
    }

    pub fn has_skill(&self, id: &str) -> bool {
        self.skills.iter().any(|s| s.id == id)
    }

    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|e| e.name == name)
    }

    pub fn kb_ids(&self) -> Vec<&str> {
        self.knowledge_bases.iter().map(|k| k.id.as_str()).collect()
    }

    pub fn skill_ids(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.id.as_str()).collect()
    }

    pub fn extension_names(&self) -> Vec<&str> {
        self.extensions.iter().map(|e| e.name.as_str()).collect()
    }

    /// Render a list for an error message. An empty catalog says so in words —
    /// "(none installed)" is actionable; an empty list looks like a bug.
    pub fn render_list(items: &[&str]) -> String {
        if items.is_empty() {
            "(none installed)".to_string()
        } else {
            items.join(", ")
        }
    }
}

fn discover_kbs(caller: &KbCaller) -> Vec<KbEntry> {
    let Ok(service) = crate::knowledge::service::KnowledgeService::new_default() else {
        return Vec::new();
    };
    let root = service.root().to_path_buf();
    service
        .list_bases()
        .map(|bases| {
            bases
                .into_iter()
                // Issue #56. Per base, and BEFORE the map: a filter applied after
                // the `KbEntry` is built is the same code with one more chance to
                // be reordered into a post-filter on a serialised string — which
                // would leave `has_kb` true, so a public session could still
                // configure an app against a private base and have its KB tools
                // armed.
                //
                // ⚠ Finding 17. This asked `tier::is_private` with its own
                // `privacy_tiers_enabled()` conjunct — a SECOND spelling of a
                // question the barrier already answers, and one axis short of
                // it. It now asks the barrier. DR-15's master opt-out comes with
                // it, read once inside `tier::assert_reachable`: a caller that
                // read the toggle here *and* called the barrier would be the
                // two-reads race in miniature.
                .filter(|m| caller.can_reach(&root, &m.id))
                .map(|m| KbEntry {
                    id: m.id,
                    name: m.name,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The directories `biorouter`'s `SkillsExtension` scans. Duplicated here because
/// `biorouter-mcp` cannot depend on `biorouter` (circular) — pinned against drift
/// by a cross-crate agreement test in `biorouter`
/// (`tests/skill_catalog_agreement.rs`).
fn skill_directories() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Ok(home) = etcetera::home_dir() {
        dirs.push(home.join(".claude/skills"));
        dirs.push(home.join(".config/agents/skills"));
    }

    dirs.push(crate::paths::in_config_dir("skills"));

    let extensions_dir = crate::paths::in_config_dir("extensions");
    if let Ok(entries) = std::fs::read_dir(&extensions_dir) {
        for entry in entries.flatten() {
            let skills_subdir = entry.path().join("skills");
            if skills_subdir.is_dir() {
                dirs.push(skills_subdir);
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join(".claude/skills"));
        dirs.push(cwd.join(".biorouter/skills"));
        dirs.push(cwd.join(".agents/skills"));
    }

    dirs
}

/// A skill is a directory holding a `SKILL.md`, whose id is the frontmatter
/// `name`. A directory of such directories is a *bundle*, scanned one level deep.
fn discover_skills() -> Vec<SkillEntry> {
    let mut found: Vec<SkillEntry> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for dir in skill_directories() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.join("SKILL.md").exists() {
                if let Some(skill) = read_skill(&path.join("SKILL.md")) {
                    if seen.insert(skill.id.clone()) {
                        found.push(skill);
                    }
                }
            } else if let Ok(subs) = std::fs::read_dir(&path) {
                // A bundle: <dir>/<bundle>/<skill>/SKILL.md
                for sub in subs.flatten() {
                    let sub_skill = sub.path().join("SKILL.md");
                    if sub_skill.exists() {
                        if let Some(skill) = read_skill(&sub_skill) {
                            if seen.insert(skill.id.clone()) {
                                found.push(skill);
                            }
                        }
                    }
                }
            }
        }
    }

    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// Pull `name` and `description` out of a `SKILL.md`'s YAML frontmatter. Kept
/// deliberately small — we need the *id*, not the skill.
fn read_skill(skill_md: &std::path::Path) -> Option<SkillEntry> {
    let text = std::fs::read_to_string(skill_md).ok()?;
    let body = text.strip_prefix("---")?;
    let (frontmatter, _) = body.split_once("\n---")?;

    let mut id = None;
    let mut description = String::new();
    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'').trim();
        match key.trim() {
            "name" if !value.is_empty() => id = Some(value.to_string()),
            "description" if !value.is_empty() => description = value.to_string(),
            _ => {}
        }
    }

    Some(SkillEntry {
        id: id?,
        description,
    })
}

/// Installed `.brxt` extensions — each is a directory under `<config>/extensions`.
fn discover_external_extensions() -> Vec<ExtEntry> {
    let dir = crate::paths::in_config_dir("extensions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<ExtEntry> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !BUILTIN_EXTENSION_NAMES.contains(&name.as_str()))
        .map(|name| ExtEntry {
            name,
            builtin: false,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A sandboxed knowledge root that [`Catalog::discover`] will really resolve to,
/// with `ids` created through the **real** `create_base` (issue #56).
///
/// Returned as a triple because all three must outlive the assertions: dropping
/// the `TempDir` unlinks the tree, and dropping the `EnvGuard` un-points
/// `BIOROUTER_PATH_ROOT` — after which `discover` reads the developer's own
/// knowledge bases and the test says nothing.
///
/// `create_base` rather than hand-made directories: a directory with no tier
/// entry reads private by inference (`tier`'s decision 3), which would make
/// "the public catalog omits it" pass for every base and prove nothing.
///
/// Lives at module level, not inside `mod tests`, so `validate`'s tests and
/// `agent_drafter`'s can share the one fixture.
#[cfg(test)]
pub(crate) fn drafter_catalog_root_with_kbs(
    ids: &[&str],
) -> (tempfile::TempDir, PathBuf, env_lock::EnvGuard<'static>) {
    let d = tempfile::tempdir().unwrap();
    // `crate::paths::config_dir` reads `BIOROUTER_PATH_ROOT` and
    // `knowledge::paths::knowledge_root` is `<config>/knowledge`, so this is
    // exactly where `discover_kbs`'s `new_default()` will look. `env_lock` and
    // not `set_var`: the variable is read on every config-dir lookup anywhere in
    // the process, and the guard restores it even from a panicking test.
    let guard = env_lock::lock_env([
        (
            "BIOROUTER_PATH_ROOT",
            Some(d.path().to_string_lossy().into_owned()),
        ),
        // Strict mode ON, so the validators really render their catalog lists.
        // `relax_catalog_strictness` in `agent_drafter`'s tests turns this off
        // process-wide; it now takes the same lock, so the two cannot interleave.
        ("BIOROUTER_APPS_CATALOG_STRICT", None),
    ]);
    let root = d.path().join("config").join("knowledge");
    let svc = crate::knowledge::service::KnowledgeService::new(root.clone());
    for id in ids {
        // The NAME embeds the id, so one `contains(id)` assertion catches a leak
        // of either field — a KB name is user-authored and is the half a
        // redaction-not-omission implementation would keep.
        svc.create_base(id, &format!("Cohort {id}"), None).unwrap();
    }
    (d, root, guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #56, CP5. `discover_kbs` had **no filter of any kind**, and the tool
    /// that serialises its output (`list_platform_catalog`) tells the model to
    /// call it before `configure_app` — so this ran on every app-building turn.
    #[test]
    fn the_catalog_omits_a_private_knowledge_base_from_a_public_caller() {
        let (_d, root, _env) = drafter_catalog_root_with_kbs(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();

        let public = Catalog::discover(&KbCaller::restricted());
        assert_eq!(public.kb_ids(), vec!["default"]);
        assert!(
            !serde_json::to_string(&public).unwrap().contains("omop"),
            "the id or the NAME survived serialisation"
        );
        // …and `has_kb` reads the same filtered vector, so a public session
        // cannot configure an app against it either (the "filter the serialised
        // JSON" wrong implementation passes the assertion above and fails this).
        assert!(!public.has_kb("omop"));

        let private = Catalog::discover(&private_local());
        assert_eq!(private.kb_ids(), vec!["default", "omop"]);
        assert!(private.has_kb("omop"));
    }

    /// A private caller whose model is covered by one institution's agreements.
    fn private_at(institution: &str) -> KbCaller {
        KbCaller::new(
            true,
            crate::knowledge::affiliation::CallerAffiliation::Institution(institution.to_string()),
        )
    }

    /// A private, LOCAL model — the caller DR-26 clears everywhere, because it
    /// transfers nothing.
    fn private_local() -> KbCaller {
        KbCaller::new(
            true,
            crate::knowledge::affiliation::CallerAffiliation::Local,
        )
    }

    /// **Audit finding 17, second spelling.** This filter asked
    /// `tier::is_private` — the tier axis alone — so both callers below are
    /// PRIVATE and it gave them the same answer. The drafting model learns
    /// knowledge-base ids from this catalogue and nowhere else, and `br.kb`
    /// (CP3) then refuses every read of a base it may not reach: the listing was
    /// the leak and the refusal merely the tell.
    ///
    /// The discrimination is the point — `default` must survive for BOTH, or the
    /// filter is "refuse the second caller" and this test would pass against a
    /// fix that broke the feature.
    #[test]
    fn the_catalog_asks_the_affiliation_axis_not_only_the_tier() {
        let (_d, root, _env) = drafter_catalog_root_with_kbs(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        crate::knowledge::tier::raise_affiliation_unlocked(
            &root,
            "omop",
            &crate::knowledge::affiliation::CallerAffiliation::Institution("ucsf".to_string()),
        )
        .unwrap();

        let ucsf = Catalog::discover(&private_at("ucsf"));
        assert_eq!(ucsf.kb_ids(), vec!["default", "omop"]);

        let stanford = Catalog::discover(&private_at("stanford"));
        assert_eq!(
            stanford.kb_ids(),
            vec!["default"],
            "the catalogue named a base the barrier refuses across an \
             institutional boundary"
        );
        assert!(
            !serde_json::to_string(&stanford).unwrap().contains("omop"),
            "the id or the NAME survived serialisation"
        );
        // `has_kb` reads the same filtered vector, so the cross-institution
        // session cannot configure an app against it and have its `kb_*` tools
        // armed either.
        assert!(!stanford.has_kb("omop"));

        // A private model that states nothing is DR-26's restrictive caller.
        let unstated = Catalog::discover(&KbCaller::new(
            true,
            crate::knowledge::affiliation::CallerAffiliation::Unstated,
        ));
        assert_eq!(unstated.kb_ids(), vec!["default"]);
    }

    #[test]
    fn builtins_are_always_present() {
        let c = Catalog::discover(&KbCaller::restricted());
        for name in BUILTIN_EXTENSION_NAMES {
            assert!(c.has_extension(name), "builtin '{name}' missing");
        }
    }

    #[test]
    fn an_empty_catalog_says_so_in_words() {
        let c = Catalog::empty();
        assert_eq!(Catalog::render_list(&c.kb_ids()), "(none installed)");
        assert_eq!(Catalog::render_list(&c.skill_ids()), "(none installed)");
        assert!(!c.has_kb("clinvar"));
        assert!(!c.has_skill("pathway-analysis"));
    }

    #[test]
    fn render_list_joins_real_ids() {
        assert_eq!(Catalog::render_list(&["a", "b"]), "a, b");
    }

    #[test]
    fn a_skill_is_read_from_its_frontmatter_name_not_its_directory_name() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("some-folder-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: pathway-analysis\ndescription: Reason over pathways\n---\n\nBody.\n",
        )
        .unwrap();

        let skill = read_skill(&skill_dir.join("SKILL.md")).expect("parsed");
        assert_eq!(skill.id, "pathway-analysis");
        assert_eq!(skill.description, "Reason over pathways");
    }

    #[test]
    fn a_skill_md_without_a_name_is_not_a_skill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SKILL.md"),
            "---\ndescription: no name here\n---\n",
        )
        .unwrap();
        assert!(read_skill(&dir.path().join("SKILL.md")).is_none());
    }
}
