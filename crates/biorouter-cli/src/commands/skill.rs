//! `biorouter skill` subcommands — install a skill or skill package from a
//! `.zip` or a repository URL, list installed skills with their enabled state,
//! enable/disable a skill without removing it, and remove one.
//!
//! ⚠ **Install and remove go through
//! `biorouter::agents::skill_package`**, the one import pipeline the daemon,
//! the interface and the marketplace also use. This module used to carry its
//! own archive parser — a near-copy of the daemon's, recognising three shapes
//! by counting slashes — and the two drifted, which is how a repository archive
//! came to install as one flattened skill per `SKILL.md` in the terminal and in
//! the app alike (#115). It was deleted rather than kept in step.
//!
//! Discovery and frontmatter parsing are the backend's own
//! (`biorouter::agents::skills_extension::SkillsClient`) — same roots
//! (`~/.claude/skills`, `~/.config/agents/skills`, the Biorouter config
//! skills dir, extension `skills/` subdirs, project-local dirs), same YAML
//! semantics — so `skill list` shows exactly what the agent can load.
//!
//! Enable/disable state lives in `~/.config/biorouter/skills-config.json`
//! (`{"disabled": [...]}`), shared with the desktop GUI's per-skill toggles.
//! The backend (`skills_extension.rs`) matches entries against the skill's
//! frontmatter `name` and its bundle directory name — never the on-disk slug —
//! so `enable`/`disable` here resolve whatever the user typed to the
//! identifier the backend actually filters on.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use biorouter::agents::skill_redirects::{Redirect, Redirects, Replacement, ReplacementKind};
use biorouter::agents::skills_extension::{
    skills_root as biorouter_skills_root, Skill, SkillsClient,
};
use biorouter::config::paths::Paths;
use console::{style, Color};

const ACCENT: Color = Color::Color256(137);

/// The skills root this CLI installs into and removes from.
///
/// A thin wrapper over the library's `skills_root` rather than a second
/// spelling of the same join: a CLI that removed from a directory the daemon
/// does not discover would report success and change nothing.
fn skills_root() -> PathBuf {
    biorouter_skills_root(&Paths::config_dir())
}

// ──────────────────────────────────────────────────────────────────────────────
// install
// ──────────────────────────────────────────────────────────────────────────────

/// `biorouter skill install <path-or-url>`.
///
/// ⚠ **The same pipeline the daemon, the interface and the marketplace use**
/// (`biorouter::agents::skill_package`). This function used to carry its own
/// depth-counting archive parser — a near-copy of the daemon's, and it drifted
/// from it — which is why a repository archive installed as one flattened skill
/// per `SKILL.md` here and in the app alike (#115).
pub async fn handle_install(source: String, force: bool, choice: Option<String>) -> Result<()> {
    use biorouter::agents::skill_package::{self, ImportSource};

    let import_source = if source.starts_with("http://") || source.starts_with("https://") {
        ImportSource::Url {
            url: source.clone(),
            reference: None,
        }
    } else {
        let path = PathBuf::from(&source);
        if !path.exists() {
            bail!("File not found: {}", path.display());
        }
        ImportSource::Archive { path }
    };

    let fetched = skill_package::fetch(&import_source).await?;
    let plan =
        skill_package::plan_from_entries(fetched.entries, &fetched.id_hints, fetched.source)?;

    // A terminal has no dialog, so an unanswered question is printed and the
    // command stops. It is NOT resolved by picking one — that is the flattening
    // this replaced.
    let plans = match (choice.as_deref(), plan.ambiguity.clone()) {
        (Some("bundle"), _) => vec![plan.as_bundle()],
        (Some("individual"), _) => {
            let keep: Vec<String> = plan.components.iter().map(|c| c.name.clone()).collect();
            plan.into_individual(&keep)
        }
        (Some(other), _) => bail!("--as must be 'bundle' or 'individual', not '{other}'"),
        (None, Some(ambiguity)) => {
            println!("  {} {}", style("?").yellow().bold(), ambiguity.reason);
            for name in &ambiguity.components {
                println!("    {} {}", style("·").dim(), style(name).dim());
            }
            bail!(
                "Re-run with `--as bundle` to install them as one package, or `--as individual` \
                 to install them separately."
            );
        }
        (None, None) => vec![plan],
    };

    let root = skills_root();
    refuse_before_installing(&plans, &root, force)?;

    for plan in &plans {
        let installed = skill_package::install(plan, &root)?;
        match installed.kind {
            skill_package::ImportKind::Single => {
                let component = &plan.components[0];
                report_single(
                    &component.name,
                    &component.description,
                    &installed.id,
                    &installed.directory,
                    plan.files.len(),
                );
            }
            skill_package::ImportKind::Bundle => {
                println!(
                    "  {} installed skill package {} {}",
                    style(if installed.replaced { "↻" } else { "✓" }).green(),
                    style(&installed.display_name).fg(ACCENT).bold(),
                    style(format!(
                        "({} skills, {} files)",
                        installed.skills.len(),
                        plan.files.len()
                    ))
                    .dim()
                );
                for component in &plan.components {
                    let marker = if component.entry_point { "→" } else { "·" };
                    let group = component
                        .group
                        .as_deref()
                        .map(|g| format!(" [{g}]"))
                        .unwrap_or_default();
                    println!(
                        "    {} {}{}",
                        style(marker).dim(),
                        style(&component.name).dim(),
                        style(group).dim()
                    );
                }
                println!(
                    "    {} {}",
                    style("path:").dim(),
                    style(installed.directory.display()).dim()
                );
            }
        }
    }
    Ok(())
}

/// Every refusal an install can know about before it writes anything, for
/// every plan, so a multi-plan install stops before its first write rather
/// than halfway through.
///
/// ⚠ **The shipped-name refusal comes FIRST, from the installer's own guard.**
/// This used to check only "already exists", and answered a shipped name with
/// "Re-run with --force to replace it" — advertising an escape hatch that
/// `skill_package::install` then refuses (`refuse_shipped`), so following the
/// advice earned a second refusal (QA-D F10). A shipped name is refused
/// whatever `--force` says, so the advice must never be offered for one. The
/// verdict comes from `shipped_refusal`, the same predicate `install` applies,
/// rather than a copy of the rule here that could drift from it.
fn refuse_before_installing(
    plans: &[biorouter::agents::skill_package::ImportPlan],
    root: &Path,
    force: bool,
) -> Result<()> {
    for plan in plans {
        if let Some(refusal) =
            biorouter::agents::skill_package::install::shipped_refusal(plan, root)
        {
            bail!(refusal);
        }
        if !force && plan.destination(root).exists() {
            bail!(
                "Already installed at {}. Re-run with --force to replace it.",
                plan.destination(root).display()
            );
        }
    }
    Ok(())
}

fn report_single(name: &str, description: &str, slug: &str, dest: &Path, files: usize) {
    println!(
        "  {} installed skill {} {}",
        style("✓").green(),
        style(slug).fg(ACCENT).bold(),
        style(format!("({} files)", files)).dim()
    );
    println!("    {} {}", style("name:").dim(), name);
    if !description.is_empty() {
        println!("    {} {}", style("desc:").dim(), style(description).dim());
    }
    println!(
        "    {} {}",
        style("path:").dim(),
        style(dest.display()).dim()
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// discovery (shared by list / enable / disable)
// ──────────────────────────────────────────────────────────────────────────────

/// An installed skill exactly as the backend's skills extension discovered
/// it. The backend's disabled set matches the frontmatter `name` and the
/// bundle directory name — never the on-disk slug. Files the backend rejects
/// (unparseable frontmatter) never appear here, because the backend will
/// never load them and toggling them would have no effect.
#[derive(Debug, Clone)]
struct InstalledSkill {
    /// Directory path relative to its skills root
    /// (e.g. `my-skill` or `superpowers/brainstorming`).
    slug: String,
    /// Frontmatter `name` — the identifier the backend filters on.
    name: String,
    description: String,
    /// Top-level bundle directory name when the skill is part of a bundle.
    bundle: Option<String>,
    /// The skills root directory this skill was discovered under.
    source_root: PathBuf,
    /// When this skill is a redirect stub whose replacement is installed: the
    /// installed skill or bundle it resolves to (J-P06). Its row then shows the
    /// replacement's state, and a switch naming it is saved under the
    /// replacement.
    replacement: Option<Replacement>,
    /// The library's one sentence about this stub, when it is one.
    moved_notice: Option<String>,
    /// The stub's redirect as the library resolved it, so the machine-wide
    /// rule is the library's own (`Redirects::expand_disabled`).
    redirect: Option<Redirect>,
}

/// Discover skills with the backend's own routine over the backend's own
/// roots (`SkillsClient::get_default_skill_directories`), so the CLI sees the
/// same set — including shared, extension-provided, and project-local skills
/// — with identical YAML frontmatter semantics and identical later-root-wins
/// shadowing.
fn collect_installed_skills() -> Vec<InstalledSkill> {
    let directories: Vec<PathBuf> = SkillsClient::get_default_skill_directories()
        .into_iter()
        .filter(|d| d.exists())
        .collect();
    installed_from_backend(SkillsClient::discover_skills_in_directories(&directories))
}

/// A slug is a LOGICAL identifier, not a path, so its separator is `/` on
/// every platform: `superpowers/brainstorming` on macOS, Linux **and**
/// Windows. That is what `skill list` prints, what the user types back at
/// `skill enable`, and what [`collect_candidates`] splits on to get the last
/// component. `Path::to_string_lossy` hands back `superpowers\brainstorming`
/// on Windows, and every one of those three then quietly stops working: the
/// printed slug is untypeable, an exact-slug query never matches, and
/// `rsplit('/')` returns the whole slug so bundle sub-skills stop colliding
/// on their shared last component — turning a real ambiguity into a silent
/// "no match", which `enable` reads as a stale entry and deletes.
///
/// ⚠ Joining `components()` rather than `replace('\\', "/")` is deliberate:
/// a backslash is a legal character in a Unix directory name, so the blunt
/// replacement would invent a separator inside a real one-component name.
fn slug_from_relative_path(relative: &Path) -> String {
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Map the backend's discovery result into rows for listing/resolution,
/// sorted by (source root, slug) for stable grouped output.
fn installed_from_backend(skills: HashMap<String, Skill>) -> Vec<InstalledSkill> {
    let redirects = Redirects::from_skills(&skills);
    let mut out: Vec<InstalledSkill> = skills
        .into_values()
        .map(|skill| {
            let redirect = redirects.get(&skill.metadata.name);
            let slug = slug_from_relative_path(
                skill
                    .directory
                    .strip_prefix(&skill.source_root)
                    .unwrap_or(&skill.directory),
            );
            InstalledSkill {
                slug,
                name: skill.metadata.name,
                description: skill.metadata.description,
                bundle: skill.bundle_name,
                source_root: skill.source_root,
                replacement: redirect.and_then(|redirect| redirect.replacement.clone()),
                moved_notice: redirect.map(|redirect| redirect.notice()),
                redirect: redirect.cloned(),
            }
        })
        .collect();
    out.sort_by(|a, b| (&a.source_root, &a.slug).cmp(&(&b.source_root, &b.slug)));
    out
}

// ──────────────────────────────────────────────────────────────────────────────
// skills-config.json (shared with the GUI's per-skill toggles)
// ──────────────────────────────────────────────────────────────────────────────

fn skills_config_path() -> PathBuf {
    Paths::config_dir().join("skills-config.json")
}

/// Read `skills-config.json` as a raw JSON value so unknown fields written by
/// other surfaces (the GUI, future versions) survive a read-modify-write.
fn load_skills_config(path: &Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("{} is not valid JSON. Fix or remove it", path.display()))?;
    if !value.is_object() {
        bail!("{} must contain a JSON object", path.display());
    }
    Ok(value)
}

/// Add or remove `identifier` in the config's `disabled` array, touching
/// nothing else. Idempotent; returns `true` when the config changed.
fn set_disabled_state(
    config: &mut serde_json::Value,
    identifier: &str,
    disable: bool,
) -> Result<bool> {
    let obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("skills-config.json must contain a JSON object"))?;
    let entry = obj
        .entry("disabled")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let arr = entry.as_array_mut().ok_or_else(|| {
        anyhow!("the 'disabled' field in skills-config.json must be an array of skill names")
    })?;
    let present = arr.iter().any(|v| v.as_str() == Some(identifier));
    if disable {
        if present {
            return Ok(false);
        }
        arr.push(serde_json::Value::String(identifier.to_string()));
    } else {
        if !present {
            return Ok(false);
        }
        arr.retain(|v| v.as_str() != Some(identifier));
    }
    Ok(true)
}

/// Write the config atomically — the GUI writes the same file, so never
/// leave a half-written JSON behind. The temp file is uniquely named
/// (`tempfile`) so two concurrent writers can never scribble into the same
/// staging file, and `persist` renames it over the target in one step.
fn write_skills_config(path: &Path, config: &serde_json::Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)?;
    let body = serde_json::to_string_pretty(config)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating a temp file in {}", parent.display()))?;
    std::io::Write::write_all(&mut tmp, body.as_bytes())
        .with_context(|| format!("writing {}", tmp.path().display()))?;
    tmp.persist(path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Advisory cross-process lock serializing every read-modify-write of
/// `skills-config.json`. Held on a sibling `.lock` file — never on the
/// config itself, whose inode is replaced by each atomic rename, which would
/// make the lock worthless. Released when the returned handle drops.
fn lock_skills_config(path: &Path) -> Result<fs::File> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("json.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening lock file {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("locking {}", lock_path.display()))?;
    Ok(file)
}

/// The one way to change skills-config.json: take the advisory lock,
/// re-read the file fresh under it (another process — GUI or a second CLI —
/// may have written since any earlier read), apply `mutate`, and atomically
/// replace the file only when something changed. Returns whether the config
/// changed plus the final config.
fn mutate_skills_config<F>(path: &Path, mutate: F) -> Result<(bool, serde_json::Value)>
where
    F: FnOnce(&mut serde_json::Value) -> Result<bool>,
{
    let _lock = lock_skills_config(path)?;
    let mut config = load_skills_config(path)?;
    let changed = mutate(&mut config)?;
    if changed {
        write_skills_config(path, &config)?;
    }
    Ok((changed, config))
}

fn disabled_set(config: &serde_json::Value) -> HashSet<String> {
    config
        .get("disabled")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The exact machine-wide test the backend applies (`skill_catalog::Composition`):
/// a skill is disabled when its frontmatter name or its bundle name is listed,
/// after an old name listed there is carried to its replacement
/// ([`carried_disabled`]), and a redirect stub answers for its replacement.
fn is_enabled(skill: &InstalledSkill, disabled: &HashSet<String>) -> bool {
    let (name, bundle) = match &skill.replacement {
        Some(replacement) => (replacement.name.as_str(), replacement.bundle.as_deref()),
        None => (skill.name.as_str(), skill.bundle.as_deref()),
    };
    let name_disabled = disabled.contains(name);
    let bundle_disabled = bundle.is_some_and(|b| disabled.contains(b));
    !(name_disabled || bundle_disabled)
}

/// `disabled` with every old name it lists, and every other bundle an old name
/// sat in, also switching the installed replacement off: the library's
/// `Redirects::expand_disabled`, over the rows this command holds (J-P06).
fn carried_disabled(skills: &[InstalledSkill], disabled: &HashSet<String>) -> HashSet<String> {
    redirects_of(skills).expand_disabled(disabled)
}

fn redirects_of(skills: &[InstalledSkill]) -> Redirects {
    Redirects::from_redirects(skills.iter().filter_map(|skill| skill.redirect.clone()))
}

/// The line `enable` prints when an old bundle still keeps `name` off.
fn kept_off_by_old_bundle(name: &str, bundle: &str, mark: &str) -> String {
    format!(
        "  {mark} {name} stays off while the bundle {bundle} is disabled: a skill that moved \
         into {name} was in that bundle, and the choice to switch it off carries. Run \
         `biorouter skill enable {bundle}`, or switch {name} on in a chat"
    )
}

/// What `skill list` prints after a moved row's name: where its content went.
fn moved_note(skill: &InstalledSkill) -> Option<String> {
    match (&skill.replacement, &skill.moved_notice) {
        (Some(replacement), _) => Some(format!(
            "moved into {}, whose state it shows",
            replacement.name
        )),
        (None, Some(notice)) => Some(notice.clone()),
        (None, None) => None,
    }
}

/// The target a switch is saved under: a redirect stub's installed
/// replacement, with the notice to print, or the target itself.
fn switch_target(
    skills: &[InstalledSkill],
    target: ResolvedTarget,
) -> (ResolvedTarget, Option<String>) {
    let ResolvedTarget::Skill { name, .. } = &target else {
        return (target, None);
    };
    let Some(stub) = skills.iter().find(|skill| &skill.name == name) else {
        return (target, None);
    };
    let notice = stub.moved_notice.clone();
    let Some(replacement) = &stub.replacement else {
        return (target, notice);
    };
    let redirected = match replacement.kind {
        ReplacementKind::Bundle => ResolvedTarget::Bundle {
            name: replacement.name.clone(),
            skills: skills
                .iter()
                .filter(|skill| skill.bundle.as_deref() == Some(replacement.name.as_str()))
                .count(),
        },
        ReplacementKind::Skill => ResolvedTarget::Skill {
            name: replacement.name.clone(),
            slug: skills
                .iter()
                .find(|skill| skill.name == replacement.name)
                .map_or_else(|| replacement.name.clone(), |skill| skill.slug.clone()),
            bundle: replacement.bundle.clone(),
            via_slug: false,
        },
    };
    (redirected, notice)
}

// ──────────────────────────────────────────────────────────────────────────────
// identifier resolution (name / bundle / slug → what the backend matches on)
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum ResolvedTarget {
    /// A single skill, identified by its frontmatter name.
    Skill {
        name: String,
        slug: String,
        bundle: Option<String>,
        /// True when the query matched the on-disk slug rather than the
        /// frontmatter name (worth surfacing the mapping to the user).
        via_slug: bool,
    },
    /// A whole bundle, identified by its top-level directory name.
    Bundle { name: String, skills: usize },
}

/// Typed outcome of [`resolve_identifier`], so callers can tell a genuine
/// zero-match (the ONLY state where `handle_enable`'s stale-entry cleanup may
/// mutate the config) from every other failure — ambiguity, empty query —
/// which must surface to the user, never be swallowed by cleanup (Codex B2
/// finding 3).
#[derive(Debug)]
enum ResolveError {
    /// No installed skill or bundle matched the query in any namespace.
    NoMatch(anyhow::Error),
    /// A real resolution failure (ambiguous or empty query): report it,
    /// change nothing.
    Fatal(anyhow::Error),
}

impl ResolveError {
    fn into_inner(self) -> anyhow::Error {
        match self {
            ResolveError::NoMatch(err) | ResolveError::Fatal(err) => err,
        }
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NoMatch(err) | ResolveError::Fatal(err) => err.fmt(f),
        }
    }
}

/// Resolve what the user typed — frontmatter name, bundle name, or directory
/// slug — to the identifier the backend's disabled set matches on.
///
/// Candidates are collected across ALL identifier namespaces at once
/// (frontmatter name, bundle directory, full slug, slug last component) and
/// deduplicated when they refer to the same skill. Distinct targets are an
/// error listing every candidate — never a silent precedence pick, which
/// would mutate skill A when the user meant skill B whose slug happens to
/// equal A's name (Codex B2 finding 2). Exact matches (in any namespace) win
/// over case-insensitive ones.
fn resolve_identifier(
    skills: &[InstalledSkill],
    query: &str,
) -> Result<ResolvedTarget, ResolveError> {
    let q = query.trim();
    if q.is_empty() {
        return Err(ResolveError::Fatal(anyhow!("Empty skill name.")));
    }
    let ql = q.to_lowercase();

    let exact = collect_candidates(skills, &|id: &str| id == q);
    let candidates = if exact.is_empty() {
        collect_candidates(skills, &|id: &str| id.to_lowercase() == ql)
    } else {
        exact
    };

    match candidates.len() {
        1 => Ok(candidates.into_iter().next().expect("len checked")),
        0 => Err(ResolveError::NoMatch(no_match_error(skills, q, &ql))),
        _ => {
            let listed: Vec<String> = candidates.iter().map(candidate_label).collect();
            Err(ResolveError::Fatal(anyhow!(
                "'{q}' is ambiguous. It matches {} distinct targets: {}. \
                 Use the full slug for a skill, or the exact directory name for a bundle.",
                candidates.len(),
                listed.join("; ")
            )))
        }
    }
}

/// Every target the predicate selects, across all identifier namespaces: one
/// candidate per matched skill (a skill matched via both its name and its
/// slug is a single target, reported via the name) plus one per matched
/// bundle directory.
fn collect_candidates(
    skills: &[InstalledSkill],
    matches: &dyn Fn(&str) -> bool,
) -> Vec<ResolvedTarget> {
    let mut out = Vec::new();
    for skill in skills {
        let by_name = matches(&skill.name);
        let last = skill.slug.rsplit('/').next().unwrap_or(&skill.slug);
        let by_slug = matches(&skill.slug) || matches(last);
        if by_name || by_slug {
            out.push(ResolvedTarget::Skill {
                name: skill.name.clone(),
                slug: skill.slug.clone(),
                bundle: skill.bundle.clone(),
                via_slug: !by_name,
            });
        }
    }
    let mut bundles: Vec<&str> = skills.iter().filter_map(|s| s.bundle.as_deref()).collect();
    bundles.sort_unstable();
    bundles.dedup();
    for bundle in bundles {
        if matches(bundle) {
            let count = skills
                .iter()
                .filter(|s| s.bundle.as_deref() == Some(bundle))
                .count();
            out.push(ResolvedTarget::Bundle {
                name: bundle.to_string(),
                skills: count,
            });
        }
    }
    out
}

/// How a candidate is presented in the ambiguity error.
fn candidate_label(target: &ResolvedTarget) -> String {
    match target {
        ResolvedTarget::Skill { name, slug, .. } => format!("skill '{name}' ({slug})"),
        ResolvedTarget::Bundle { name, skills } => format!("bundle '{name}' ({skills} skills)"),
    }
}

/// Nothing matched in any namespace — the error text, with close-identifier
/// suggestions when any exist.
fn no_match_error(skills: &[InstalledSkill], q: &str, ql: &str) -> anyhow::Error {
    let mut bundles: Vec<&str> = skills.iter().filter_map(|s| s.bundle.as_deref()).collect();
    bundles.sort_unstable();
    bundles.dedup();
    let mut suggestions: Vec<String> = skills
        .iter()
        .flat_map(|s| {
            let mut ids = vec![s.slug.clone()];
            if s.name != s.slug {
                ids.push(s.name.clone());
            }
            ids
        })
        .chain(bundles.iter().map(|b| b.to_string()))
        .filter(|c| {
            let cl = c.to_lowercase();
            cl.contains(ql) || ql.contains(&cl)
        })
        .collect();
    suggestions.sort();
    suggestions.dedup();
    if suggestions.is_empty() {
        anyhow!(
            "No installed skill or bundle matches '{q}'. Run `biorouter skill list` to see installed skills (slug, name, and enabled state)."
        )
    } else {
        anyhow!(
            "No installed skill or bundle matches '{q}'. Did you mean: {}?",
            suggestions.join(", ")
        )
    }
}

fn target_identifier(target: &ResolvedTarget) -> &str {
    match target {
        ResolvedTarget::Skill { name, .. } => name,
        ResolvedTarget::Bundle { name, .. } => name,
    }
}

fn target_label(target: &ResolvedTarget) -> String {
    match target {
        ResolvedTarget::Skill { name, .. } => {
            format!("skill {}", style(name).fg(ACCENT).bold())
        }
        ResolvedTarget::Bundle { name, skills } => {
            let count = if *skills == 1 {
                "(1 skill)".to_string()
            } else {
                format!("({skills} skills)")
            };
            format!(
                "bundle {} {}",
                style(name).fg(ACCENT).bold(),
                style(count).dim()
            )
        }
    }
}

/// When the user passed a slug, show which backend identifier it mapped to.
fn slug_mapping_note(target: &ResolvedTarget) -> Option<String> {
    match target {
        ResolvedTarget::Skill {
            name,
            slug,
            via_slug: true,
            ..
        } if name != slug => Some(format!(
            "  {} slug {} → skill name {}",
            style("·").dim(),
            style(slug).bold(),
            style(name).fg(ACCENT).bold()
        )),
        _ => None,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// list
// ──────────────────────────────────────────────────────────────────────────────

/// One rendered row of `skill list`: (slug, display name, enabled).
fn list_rows(skills: &[InstalledSkill], disabled: &HashSet<String>) -> Vec<(String, String, bool)> {
    let disabled = carried_disabled(skills, disabled);
    skills
        .iter()
        .map(|skill| {
            (
                skill.slug.clone(),
                skill.name.clone(),
                is_enabled(skill, &disabled),
            )
        })
        .collect()
}

/// `~`-shortened path for display.
fn display_root(root: &Path) -> String {
    if let Ok(home) = etcetera::home_dir() {
        if let Ok(rel) = root.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    root.display().to_string()
}

pub async fn handle_list() -> Result<()> {
    println!("  {} {}", style("▌").fg(ACCENT), style("Skills").bold());
    let skills = collect_installed_skills();
    if skills.is_empty() {
        println!("    {}", style("none installed").dim());
        return Ok(());
    }

    let disabled = match load_skills_config(&skills_config_path()) {
        Ok(config) => disabled_set(&config),
        Err(err) => {
            println!("    {}", style(format!("warning: {err:#}")).yellow());
            HashSet::new()
        }
    };

    let rows = list_rows(&skills, &disabled);
    let slug_width = rows.iter().map(|(s, _, _)| s.len()).max().unwrap_or(0);
    let name_width = rows.iter().map(|(_, n, _)| n.len()).max().unwrap_or(0);
    // Rows are sorted by (source root, slug); print a dim root header per
    // group so the user can tell where each skill is loaded from.
    let mut current_root: Option<&Path> = None;
    for ((slug, name, enabled), skill) in rows.iter().zip(&skills) {
        if current_root != Some(skill.source_root.as_path()) {
            current_root = Some(skill.source_root.as_path());
            println!(
                "    {}",
                style(display_root(&skill.source_root)).dim().underlined()
            );
        }
        let dot = if *enabled {
            style("●").green().to_string()
        } else {
            style("○").dim().to_string()
        };
        // Pad the raw strings first: styling adds invisible ANSI bytes that
        // would break `{:<width$}` alignment when styles differ per row.
        let slug_cell = style(format!("{slug:<slug_width$}")).bold();
        let name_cell = if *enabled {
            style(format!("{name:<name_width$}")).to_string()
        } else {
            style(format!("{name:<name_width$}")).dim().to_string()
        };
        let description = match moved_note(skill) {
            Some(note) => format!(
                "{} {}",
                style(format!("→ {note}")).yellow(),
                style(&skill.description).dim()
            ),
            None => style(&skill.description).dim().to_string(),
        };
        println!("    {} {}  {}  {}", dot, slug_cell, name_cell, description);
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// enable / disable
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_disable(query: String) -> Result<()> {
    let skills = collect_installed_skills();
    disable_with(&skills, &skills_config_path(), &query)
}

/// The whole disable flow over already-discovered skills and an explicit
/// config path, split from `handle_disable` for the same reason as
/// [`enable_with`].
fn disable_with(skills: &[InstalledSkill], path: &Path, query: &str) -> Result<()> {
    let target = resolve_identifier(skills, query).map_err(ResolveError::into_inner)?;
    if let Some(note) = slug_mapping_note(&target) {
        println!("{note}");
    }
    let (target, moved) = switch_target(skills, target);
    if let Some(moved) = moved {
        println!("  {} {moved}", style("·").dim());
    }

    let (changed, _) = mutate_skills_config(path, |config| {
        set_disabled_state(config, target_identifier(&target), true)
    })?;
    if changed {
        println!(
            "  {} disabled {}",
            style("✓").green(),
            target_label(&target)
        );
    } else {
        println!(
            "  {} {} is already disabled",
            style("·").dim(),
            target_label(&target)
        );
    }
    Ok(())
}

pub async fn handle_enable(query: String) -> Result<()> {
    let skills = collect_installed_skills();
    enable_with(&skills, &skills_config_path(), &query)
}

/// The whole enable flow over already-discovered skills and an explicit
/// config path — split from `handle_enable` so tests can drive it against a
/// temp tree and temp config file.
fn enable_with(skills: &[InstalledSkill], path: &Path, query: &str) -> Result<()> {
    let target = match resolve_identifier(skills, query) {
        Ok(target) => target,
        Err(ResolveError::NoMatch(err)) => {
            // Nothing installed matches, but a stale entry (e.g. from a skill
            // removed after being disabled) can still be cleaned up when the
            // raw query matches it exactly. Genuine zero-match ONLY: an
            // ambiguity (or any other resolution failure) must surface, not
            // silently mutate the config and claim nothing matched.
            let (cleaned, _) = mutate_skills_config(path, |config| {
                set_disabled_state(config, query.trim(), false)
            })?;
            if cleaned {
                println!(
                    "  {} removed stale disabled entry {} (no installed skill matches it)",
                    style("✓").green(),
                    style(query.trim()).bold()
                );
                return Ok(());
            }
            return Err(err);
        }
        Err(ResolveError::Fatal(err)) => return Err(err),
    };
    if let Some(note) = slug_mapping_note(&target) {
        println!("{note}");
    }
    let (target, moved) = switch_target(skills, target);
    if let Some(moved) = moved {
        println!("  {} {moved}", style("·").dim());
    }

    // J-P06: an old name still listed would keep its replacement off, so
    // enabling the replacement clears every old name that resolves to it.
    let identifier = target_identifier(&target).to_string();
    let old_names: Vec<String> = skills
        .iter()
        .filter(|skill| {
            skill
                .replacement
                .as_ref()
                .is_some_and(|replacement| replacement.name == identifier)
        })
        .map(|skill| skill.name.clone())
        .collect();
    let (changed, config) = mutate_skills_config(path, |config| {
        let mut changed = set_disabled_state(config, &identifier, false)?;
        for old in &old_names {
            changed |= set_disabled_state(config, old, false)?;
        }
        Ok(changed)
    })?;
    if changed {
        println!("  {} enabled {}", style("✓").green(), target_label(&target));
    } else {
        println!(
            "  {} {} is already enabled",
            style("·").dim(),
            target_label(&target)
        );
    }

    // J-P06: a component that moved out of a bundle stays off while that
    // bundle is, and clearing the bundle here would switch the rest of it on.
    for bundle in redirects_of(skills).bundles_keeping_off(&identifier, &disabled_set(&config)) {
        println!(
            "{}",
            kept_off_by_old_bundle(&identifier, &bundle, &style("!").yellow().to_string())
        );
    }

    // Enabling a sub-skill is moot while its whole bundle stays disabled.
    if let ResolvedTarget::Skill {
        bundle: Some(bundle),
        ..
    } = &target
    {
        if disabled_set(&config).contains(bundle) {
            println!(
                "  {} its bundle {} is still disabled. Run `biorouter skill enable {}` to re-enable the whole bundle",
                style("!").yellow(),
                style(bundle).bold(),
                bundle
            );
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// remove
// ──────────────────────────────────────────────────────────────────────────────

/// Remove an installed skill or package, and every component with it.
///
/// Goes through the shared remover, which renames the directory aside before
/// deleting it — so it leaves the catalog's view in one step rather than
/// emptying out underneath a scan in flight.
///
/// A redirect stub's saved choices are pinned onto its replacement first, in
/// every chat and machine-wide (`skill_package::remove_keeping_choices`), so
/// removing a stub switches nothing back on.
pub async fn handle_remove(slug: String) -> Result<()> {
    let before = biorouter::agents::skill_catalog::current().skills();
    let sessions = biorouter::session::SessionManager::instance();
    let removed = biorouter::agents::skill_package::remove_keeping_choices(
        &slug,
        &skills_root(),
        &before,
        &sessions,
    )
    .await
    .map_err(|e| {
        // ⚠ Only add the separator when the message does not already end a
        // sentence. Some of these are one clause ("no package named `x` is
        // installed") and some are several sentences of advice; appending
        // unconditionally gave the latter a visible `..`.
        let message = format!("{e:#}");
        let separator = if message.trim_end().ends_with(['.', '!', '?']) {
            ""
        } else {
            "."
        };
        anyhow!("{message}{separator} Run `biorouter skill list`.")
    })?;
    let (removed, kept) = removed;
    println!(
        "  {} removed {}",
        style("✓").green(),
        style(&removed.display_name).bold()
    );
    if let Some(line) = kept_choices_line(&kept) {
        println!("  {} {line}", style("·").dim());
    }
    Ok(())
}

/// What `remove` says when it pinned choices a removed stub was carrying.
fn kept_choices_line(kept: &biorouter::agents::skill_redirects::KeptChoices) -> Option<String> {
    let mut parts = Vec::new();
    if kept.sessions > 0 {
        parts.push(format!(
            "kept the skill choices of {} chat{} that named a removed old name",
            kept.sessions,
            if kept.sessions == 1 { "" } else { "s" }
        ));
    }
    if !kept.machine_disabled.is_empty() {
        parts.push(format!(
            "kept {} switched off machine-wide",
            kept.machine_disabled.join(", ")
        ));
    }
    if !kept.problems.is_empty() {
        parts.push(format!(
            "could not keep every saved choice: {}",
            kept.problems.join("; ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write_skill(root: &Path, rel: &str, name: &str, desc: &str) {
        let dir = root.join(rel);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\nBody\n"),
        )
        .unwrap();
    }

    /// Run the shared backend discovery over explicit roots and map into CLI
    /// rows — what `collect_installed_skills` does, minus the global default
    /// directories (which unit tests must not touch).
    fn installed_at(roots: &[PathBuf]) -> Vec<InstalledSkill> {
        installed_from_backend(SkillsClient::discover_skills_in_directories(roots))
    }

    /// A skills tree with every shape the backend discovers: single skill,
    /// bundle sub-skills, an ambiguous last-component slug, broken
    /// frontmatter, and one dir nested too deep for the backend to see.
    fn sample_tree() -> (TempDir, Vec<InstalledSkill>) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_skill(root, "my-skill", "My Skill", "A single skill");
        write_skill(root, "superpowers/brainstorming", "brainstorming", "Ideas");
        write_skill(root, "superpowers/debugging", "debugging", "Bugs");
        write_skill(root, "pack-a/tool", "alpha-tool", "Ambiguous a");
        write_skill(root, "pack-b/tool", "beta-tool", "Ambiguous b");
        write_skill(
            root,
            "too/deep/skill",
            "invisible",
            "Backend never sees this",
        );
        let broken = root.join("broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("SKILL.md"), "no frontmatter here").unwrap();
        let skills = installed_at(&[root.to_path_buf()]);
        (tmp, skills)
    }

    // ── discovery ────────────────────────────────────────────────────────────

    #[test]
    fn discovery_is_the_backends_own() {
        let (tmp, skills) = sample_tree();
        let slugs: Vec<&str> = skills.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec![
                "my-skill",
                "pack-a/tool",
                "pack-b/tool",
                "superpowers/brainstorming",
                "superpowers/debugging",
            ],
            "three-deep SKILL.md and backend-rejected frontmatter must be \
             invisible, exactly like in the backend"
        );

        let single = skills.iter().find(|s| s.slug == "my-skill").unwrap();
        assert_eq!(single.name, "My Skill");
        assert_eq!(single.bundle, None);
        assert_eq!(
            single.source_root,
            tmp.path(),
            "each skill must carry the root it was discovered under"
        );

        let sub = skills
            .iter()
            .find(|s| s.slug == "superpowers/brainstorming")
            .unwrap();
        assert_eq!(sub.name, "brainstorming");
        assert_eq!(sub.bundle.as_deref(), Some("superpowers"));
    }

    /// A slug is a logical identifier with a `/` separator on every platform,
    /// not a rendered path.
    ///
    /// ⚠ The first assertion is the one that was failing, and it can only
    /// FAIL on Windows: `Path::join` is what makes the relative path
    /// multi-component, and only there does the native separator differ from
    /// `/`. On Unix it passes against the broken `to_string_lossy`
    /// implementation too, so treat a green run here as no evidence — the
    /// mutation check for it lives on Windows CI.
    ///
    /// The second assertion is the one with teeth everywhere: it rejects the
    /// tempting `replace('\\', "/")` shortcut. A backslash is a legal
    /// character in a Unix directory name, so that version would split one
    /// real component into a fake two and hand the user a slug that resolves
    /// to nothing.
    #[test]
    fn a_slug_separator_is_a_slash_on_every_platform() {
        assert_eq!(
            slug_from_relative_path(&Path::new("superpowers").join("brainstorming")),
            "superpowers/brainstorming",
            "a nested slug joins its components with '/', never the platform separator"
        );

        #[cfg(unix)]
        assert_eq!(
            slug_from_relative_path(Path::new("odd\\name")),
            "odd\\name",
            "a backslash inside a single Unix directory name is part of the name, \
             not a separator to rewrite"
        );

        assert_eq!(
            slug_from_relative_path(Path::new("my-skill")),
            "my-skill",
            "a one-component slug is unchanged"
        );
    }

    // Finding 6 parity: the backend's YAML parser accepts frontmatter the old
    // line-based CLI parser could not (e.g. a folded multi-line description),
    // and the CLI must see the identical identifier + description.
    #[test]
    fn discovery_applies_backend_yaml_frontmatter_semantics() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("folded");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: folded-skill\ndescription: >-\n  Line one\n  and line two\n---\nBody\n",
        )
        .unwrap();

        let skills = installed_at(&[tmp.path().to_path_buf()]);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "folded-skill");
        assert_eq!(
            skills[0].description, "Line one and line two",
            "folded YAML scalar must parse exactly as the backend parses it"
        );
    }

    // Finding 5 parity: multiple roots, later root wins by frontmatter name —
    // the CLI must report the same winner the backend loads.
    #[test]
    fn discovery_scans_multiple_roots_with_later_root_winning() {
        let tmp = TempDir::new().unwrap();
        let early = tmp.path().join("early");
        let late = tmp.path().join("late");
        write_skill(&early, "shared", "shared-skill", "From early root");
        write_skill(&late, "shared", "shared-skill", "From late root");
        write_skill(&early, "only-early", "only-early", "Unshadowed");

        let skills = installed_at(&[early.clone(), late.clone()]);
        assert_eq!(skills.len(), 2);
        let shared = skills.iter().find(|s| s.name == "shared-skill").unwrap();
        assert_eq!(shared.description, "From late root");
        assert_eq!(shared.source_root, late, "later root must shadow earlier");
        let unshadowed = skills.iter().find(|s| s.name == "only-early").unwrap();
        assert_eq!(unshadowed.source_root, early);
    }

    // ── identifier resolution ────────────────────────────────────────────────

    #[test]
    fn resolves_frontmatter_name_directly() {
        let (_tmp, skills) = sample_tree();
        let target = resolve_identifier(&skills, "My Skill").unwrap();
        assert_eq!(
            target,
            ResolvedTarget::Skill {
                name: "My Skill".into(),
                slug: "my-skill".into(),
                bundle: None,
                via_slug: false,
            }
        );
        assert_eq!(target_identifier(&target), "My Skill");
        assert!(slug_mapping_note(&target).is_none());
    }

    #[test]
    fn resolves_name_case_insensitively_but_writes_exact_name() {
        let (_tmp, skills) = sample_tree();
        let target = resolve_identifier(&skills, "MY SKILL").unwrap();
        assert_eq!(target_identifier(&target), "My Skill");
    }

    #[test]
    fn resolves_slug_to_frontmatter_name_with_mapping_note() {
        let (_tmp, skills) = sample_tree();
        let target = resolve_identifier(&skills, "my-skill").unwrap();
        match &target {
            ResolvedTarget::Skill { name, via_slug, .. } => {
                assert_eq!(name, "My Skill");
                assert!(via_slug);
            }
            other => panic!("expected skill, got {other:?}"),
        }
        let note = slug_mapping_note(&target).expect("slug → name mapping must be surfaced");
        assert!(note.contains("my-skill") && note.contains("My Skill"));
    }

    #[test]
    fn resolves_bundle_name_to_bundle_target() {
        let (_tmp, skills) = sample_tree();
        let target = resolve_identifier(&skills, "superpowers").unwrap();
        assert_eq!(
            target,
            ResolvedTarget::Bundle {
                name: "superpowers".into(),
                skills: 2,
            }
        );
        assert_eq!(target_identifier(&target), "superpowers");
    }

    #[test]
    fn resolves_bundle_sub_skill_by_name() {
        let (_tmp, skills) = sample_tree();
        let target = resolve_identifier(&skills, "brainstorming").unwrap();
        match target {
            ResolvedTarget::Skill {
                name,
                bundle,
                via_slug,
                ..
            } => {
                assert_eq!(name, "brainstorming");
                assert_eq!(bundle.as_deref(), Some("superpowers"));
                assert!(!via_slug, "name match wins over slug match");
            }
            other => panic!("expected skill, got {other:?}"),
        }
    }

    #[test]
    fn ambiguous_slug_component_is_rejected_with_candidates() {
        let (_tmp, skills) = sample_tree();
        let err = resolve_identifier(&skills, "tool").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(
            err.contains("pack-a/tool") && err.contains("pack-b/tool"),
            "{err}"
        );
    }

    // Finding 2: a frontmatter-name match must not silently shadow a
    // different skill whose slug equals that name — collect across
    // namespaces and report distinct targets.
    #[test]
    fn name_matching_another_skills_slug_is_ambiguous_across_namespaces() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Skill at slug `alpha` is NAMED "shadow"; a different skill LIVES at
        // slug `shadow`. The old tiered resolution silently picked the first.
        write_skill(root, "alpha", "shadow", "Name is shadow");
        write_skill(root, "shadow", "something-else", "Slug is shadow");
        let skills = installed_at(&[root.to_path_buf()]);

        let err = resolve_identifier(&skills, "shadow")
            .unwrap_err()
            .to_string();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(
            err.contains("skill 'shadow' (alpha)")
                && err.contains("skill 'something-else' (shadow)"),
            "both distinct targets must be listed as candidates: {err}"
        );
    }

    #[test]
    fn name_matching_a_bundle_directory_is_ambiguous_across_namespaces() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_skill(root, "sp", "superpowers", "A skill named like the bundle");
        write_skill(root, "superpowers/brainstorming", "brainstorming", "Ideas");
        let skills = installed_at(&[root.to_path_buf()]);

        let err = resolve_identifier(&skills, "superpowers")
            .unwrap_err()
            .to_string();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(
            err.contains("skill 'superpowers' (sp)") && err.contains("bundle 'superpowers'"),
            "skill and bundle candidates must both be listed: {err}"
        );
    }

    #[test]
    fn same_skill_matched_via_name_and_slug_is_a_single_target() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // name == slug for the same skill: two namespace hits, one target.
        write_skill(root, "solo", "solo", "Name equals slug");
        let skills = installed_at(&[root.to_path_buf()]);

        let target = resolve_identifier(&skills, "solo").unwrap();
        match &target {
            ResolvedTarget::Skill { name, via_slug, .. } => {
                assert_eq!(name, "solo");
                assert!(!via_slug, "name match must be the reported route");
            }
            other => panic!("expected skill, got {other:?}"),
        }
    }

    #[test]
    fn full_slug_disambiguates() {
        let (_tmp, skills) = sample_tree();
        let target = resolve_identifier(&skills, "pack-a/tool").unwrap();
        assert_eq!(target_identifier(&target), "alpha-tool");
    }

    #[test]
    fn unknown_identifier_gets_suggestions() {
        let (_tmp, skills) = sample_tree();
        let err = resolve_identifier(&skills, "brainstorm")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Did you mean"), "{err}");
        assert!(err.contains("brainstorming"), "{err}");

        let err = resolve_identifier(&skills, "zzz-nothing")
            .unwrap_err()
            .to_string();
        assert!(err.contains("biorouter skill list"), "{err}");
    }

    #[test]
    fn backend_rejected_skill_is_invisible_and_not_resolvable() {
        // The backend never loads a SKILL.md whose frontmatter fails to
        // parse, so toggling it would be a silent no-op — the CLI reports it
        // as not installed instead of listing a phantom entry.
        let (_tmp, skills) = sample_tree();
        assert!(
            !skills.iter().any(|s| s.slug == "broken"),
            "backend-rejected file must not be listed"
        );
        let err = resolve_identifier(&skills, "broken")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("No installed skill or bundle matches"),
            "{err}"
        );
    }

    // ── typed resolution errors + stale-entry cleanup gating ─────────────────

    // Finding 3: only a genuine zero-match may trigger handle_enable's
    // stale-entry cleanup; ambiguity and other failures must surface.
    #[test]
    fn resolution_errors_are_typed_no_match_vs_fatal() {
        let (_tmp, skills) = sample_tree();
        assert!(
            matches!(
                resolve_identifier(&skills, "zzz-nothing"),
                Err(ResolveError::NoMatch(_))
            ),
            "unknown identifier is a genuine zero-match"
        );
        assert!(
            matches!(
                resolve_identifier(&skills, "tool"),
                Err(ResolveError::Fatal(_))
            ),
            "ambiguity is fatal, never a stale-cleanup trigger"
        );
        assert!(
            matches!(
                resolve_identifier(&skills, "   "),
                Err(ResolveError::Fatal(_))
            ),
            "empty query is fatal"
        );
    }

    #[test]
    fn enable_does_not_treat_ambiguity_as_a_stale_entry() {
        let (_tmp, skills) = sample_tree();
        let cfg = TempDir::new().unwrap();
        let path = cfg.path().join("skills-config.json");
        // 'tool' is ambiguous (pack-a/tool vs pack-b/tool) AND present in the
        // disabled list — the old code deleted it and reported "no installed
        // skill matches", suppressing the ambiguity error.
        fs::write(&path, r#"{"disabled":["tool"],"future":{"keep":1}}"#).unwrap();

        let err = enable_with(&skills, &path, "tool").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "ambiguity must surface: {err}");

        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            reread["disabled"],
            json!(["tool"]),
            "config must be untouched on a fatal resolution error"
        );
        assert_eq!(reread["future"], json!({"keep": 1}));
    }

    #[test]
    fn enable_cleans_up_stale_entry_only_on_genuine_zero_match() {
        let (_tmp, skills) = sample_tree();
        let cfg = TempDir::new().unwrap();
        let path = cfg.path().join("skills-config.json");
        fs::write(&path, r#"{"disabled":["ghost-skill"]}"#).unwrap();

        enable_with(&skills, &path, "ghost-skill").unwrap();
        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reread["disabled"], json!([]), "stale entry cleaned up");
    }

    // ── skills-config.json mutation ──────────────────────────────────────────

    #[test]
    fn disable_and_enable_are_idempotent() {
        let mut config = json!({});
        assert!(set_disabled_state(&mut config, "My Skill", true).unwrap());
        assert!(
            !set_disabled_state(&mut config, "My Skill", true).unwrap(),
            "second disable must be a no-op"
        );
        assert_eq!(config["disabled"], json!(["My Skill"]));

        assert!(set_disabled_state(&mut config, "My Skill", false).unwrap());
        assert!(
            !set_disabled_state(&mut config, "My Skill", false).unwrap(),
            "second enable must be a no-op"
        );
        assert_eq!(config["disabled"], json!([]));
    }

    #[test]
    fn mutation_preserves_unknown_fields_and_other_entries() {
        let mut config = json!({
            "disabled": ["keep-me"],
            "future": {"nested": true},
            "note": "GUI forward-compat"
        });
        set_disabled_state(&mut config, "My Skill", true).unwrap();
        assert_eq!(config["disabled"], json!(["keep-me", "My Skill"]));
        assert_eq!(config["future"], json!({"nested": true}));
        assert_eq!(config["note"], json!("GUI forward-compat"));

        set_disabled_state(&mut config, "keep-me", false).unwrap();
        assert_eq!(config["disabled"], json!(["My Skill"]));
        assert_eq!(config["future"], json!({"nested": true}));
    }

    #[test]
    fn malformed_disabled_field_is_an_error_not_a_clobber() {
        let mut config = json!({"disabled": "not-an-array"});
        let err = set_disabled_state(&mut config, "x", true).unwrap_err();
        assert!(err.to_string().contains("array"), "{err}");
        assert_eq!(
            config["disabled"], "not-an-array",
            "the malformed value must be left untouched"
        );
    }

    #[test]
    fn load_write_round_trip_preserves_unknown_fields_on_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills-config.json");
        fs::write(
            &path,
            r#"{"disabled":["a"],"future":{"keep":1},"note":"hi"}"#,
        )
        .unwrap();

        let mut config = load_skills_config(&path).unwrap();
        set_disabled_state(&mut config, "b", true).unwrap();
        write_skills_config(&path, &config).unwrap();

        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reread["disabled"], json!(["a", "b"]));
        assert_eq!(reread["future"], json!({"keep": 1}));
        assert_eq!(reread["note"], json!("hi"));
    }

    #[test]
    fn load_handles_absent_empty_and_invalid_files() {
        let tmp = TempDir::new().unwrap();

        let absent = tmp.path().join("absent.json");
        assert_eq!(load_skills_config(&absent).unwrap(), json!({}));

        let empty = tmp.path().join("empty.json");
        fs::write(&empty, "  \n").unwrap();
        assert_eq!(load_skills_config(&empty).unwrap(), json!({}));

        let invalid = tmp.path().join("invalid.json");
        fs::write(&invalid, "{ not json").unwrap();
        assert!(load_skills_config(&invalid).is_err(), "must not clobber");

        let non_object = tmp.path().join("array.json");
        fs::write(&non_object, "[1,2]").unwrap();
        assert!(load_skills_config(&non_object).is_err());
    }

    #[test]
    fn write_creates_parent_dirs_and_valid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested/dir/skills-config.json");
        write_skills_config(&path, &json!({"disabled": ["x"]})).unwrap();
        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reread, json!({"disabled": ["x"]}));
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file must be renamed away"
        );
    }

    // ── locked read-modify-write (finding 1) ─────────────────────────────────

    // Concurrent writers must serialize on the advisory lock: every update
    // lands, unknown fields survive. With the old fixed `.json.tmp` staging
    // name and no lock, concurrent writers could clobber each other's temp
    // file and drop updates.
    #[test]
    fn concurrent_locked_mutations_lose_no_updates() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills-config.json");
        fs::write(&path, r#"{"disabled":[],"note":"keep"}"#).unwrap();

        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    mutate_skills_config(&path, |config| {
                        set_disabled_state(config, &format!("skill-{i}"), true)
                    })
                    .unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let disabled = disabled_set(&reread);
        for i in 0..8 {
            assert!(
                disabled.contains(&format!("skill-{i}")),
                "update for skill-{i} was lost: {reread}"
            );
        }
        assert_eq!(reread["note"], json!("keep"), "unknown fields must survive");
    }

    // The mutation must re-read the file fresh under the lock — a write by
    // another surface (GUI, second CLI) after any earlier read still lands in
    // the final file.
    #[test]
    fn mutation_rereads_the_file_fresh_under_the_lock() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills-config.json");
        fs::write(&path, r#"{"disabled":["a","external"],"gui":true}"#).unwrap();

        let (changed, _) =
            mutate_skills_config(&path, |config| set_disabled_state(config, "b", true)).unwrap();
        assert!(changed);

        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reread["disabled"], json!(["a", "external", "b"]));
        assert_eq!(reread["gui"], json!(true));
    }

    // ── list output ──────────────────────────────────────────────────────────

    #[test]
    fn list_rows_reflect_backend_enabled_semantics() {
        let (_tmp, skills) = sample_tree();

        // Disable one skill by frontmatter name and one whole bundle.
        let disabled: HashSet<String> = ["My Skill".to_string(), "superpowers".to_string()]
            .into_iter()
            .collect();
        let rows = list_rows(&skills, &disabled);
        let by_slug = |slug: &str| rows.iter().find(|(s, _, _)| s == slug).unwrap();

        let (_, name, enabled) = by_slug("my-skill");
        assert_eq!(name, "My Skill");
        assert!(!enabled, "name-disabled skill must show as disabled");

        assert!(
            !by_slug("superpowers/brainstorming").2 && !by_slug("superpowers/debugging").2,
            "bundle disable must cover every sub-skill"
        );

        assert!(by_slug("pack-a/tool").2, "untouched skill stays enabled");
    }

    // ── J-P06: superseded names ──────────────────────────────────────────────

    /// `viz-old-tracks` merged into `viz-new-tracks`, its folder holding the
    /// redirect stub biorouter-skills generates.
    fn merged_tree() -> (TempDir, Vec<InstalledSkill>) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_skill(root, "viz/new-tracks", "viz-new-tracks", "Tracks");
        let stub = root.join("viz/old-tracks");
        fs::create_dir_all(&stub).unwrap();
        fs::write(
            stub.join("SKILL.md"),
            "---\nname: viz-old-tracks\ndescription: \"Moved: now part of viz-new-tracks.\"\n\
             metadata:\n  status: \"superseded_by:viz/viz-new-tracks\"\n---\n\npointer\n",
        )
        .unwrap();
        let skills = installed_at(&[root.to_path_buf()]);
        (tmp, skills)
    }

    fn disabled_on_disk(path: &Path) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap()
            ["disabled"]
            .clone()
    }

    #[test]
    fn a_machine_wide_entry_under_a_superseded_name_lists_its_replacement_as_off() {
        let (_tmp, skills) = merged_tree();
        let disabled: HashSet<String> = ["viz-old-tracks".to_string()].into_iter().collect();
        let rows = list_rows(&skills, &disabled);
        let replacement = rows.iter().find(|(_, n, _)| n == "viz-new-tracks").unwrap();
        assert!(
            !replacement.2,
            "skill list must show what the agent will do: the old name's entry switches its replacement off"
        );
    }

    #[test]
    fn disabling_a_superseded_name_writes_its_replacement() {
        let (_tmp, skills) = merged_tree();
        let cfg = TempDir::new().unwrap();
        let path = cfg.path().join("skills-config.json");

        disable_with(&skills, &path, "viz-old-tracks").unwrap();
        assert_eq!(disabled_on_disk(&path), json!(["viz-new-tracks"]));
    }

    #[test]
    fn enabling_a_replacement_clears_the_old_names_that_keep_it_off() {
        let (_tmp, skills) = merged_tree();
        let cfg = TempDir::new().unwrap();
        let path = cfg.path().join("skills-config.json");
        fs::write(&path, r#"{"disabled":["viz-old-tracks","other"]}"#).unwrap();

        enable_with(&skills, &path, "viz-new-tracks").unwrap();
        assert_eq!(disabled_on_disk(&path), json!(["other"]));
    }

    /// `skill list` marks a moved row, so a switch on it that changes another
    /// row is not a surprise.
    #[test]
    fn skill_list_marks_a_moved_row() {
        let (_tmp, skills) = merged_tree();
        let stub = skills.iter().find(|s| s.name == "viz-old-tracks").unwrap();
        assert_eq!(
            moved_note(stub).as_deref(),
            Some("moved into viz-new-tracks, whose state it shows")
        );
        let live = skills.iter().find(|s| s.name == "viz-new-tracks").unwrap();
        assert_eq!(moved_note(live), None);
    }

    /// A component that moved into another bundle stays off while the bundle
    /// it sat in is off, and `enable` says so instead of claiming it is on.
    #[test]
    fn a_disabled_old_bundle_keeps_a_component_that_moved_out_off() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_skill(root, "single/cells", "single-cells", "Cells");
        write_skill(root, "crispr/perturb", "crispr-perturb", "Perturb");
        let stub = root.join("single/perturb");
        fs::create_dir_all(&stub).unwrap();
        fs::write(
            stub.join("SKILL.md"),
            "---\nname: single-perturb\ndescription: \"Moved\"\nmetadata:\n  status: \
             \"superseded_by:crispr/crispr-perturb\"\n---\n\npointer\n",
        )
        .unwrap();
        let skills = installed_at(&[root.to_path_buf()]);
        let disabled: HashSet<String> = ["single".to_string()].into_iter().collect();

        let rows = list_rows(&skills, &disabled);
        let perturb = rows.iter().find(|(_, n, _)| n == "crispr-perturb").unwrap();
        assert!(!perturb.2, "skill list shows what the agent will do");
        assert_eq!(
            redirects_of(&skills).bundles_keeping_off("crispr-perturb", &disabled),
            ["single"]
        );
        assert!(kept_off_by_old_bundle("crispr-perturb", "single", "!")
            .contains("Run `biorouter skill enable single`"));
    }

    #[test]
    fn disabled_set_reads_the_exact_gui_written_shape() {
        // The GUI writes {"disabled": [...]} via JSON.stringify(..., 2).
        let config: serde_json::Value =
            serde_json::from_str("{\n  \"disabled\": [\n    \"a\",\n    \"b\"\n  ]\n}").unwrap();
        let set = disabled_set(&config);
        assert!(set.contains("a") && set.contains("b"));
        assert_eq!(disabled_set(&json!({})), HashSet::new());
    }

    fn plan_named(id: &str) -> biorouter::agents::skill_package::ImportPlan {
        use biorouter::agents::skill_package::{Evidence, ImportKind, ImportPlan, PlannedSkill};
        ImportPlan {
            kind: ImportKind::Single,
            id: id.to_string(),
            display_name: id.to_string(),
            version: None,
            entry_point: None,
            groups: Default::default(),
            components: vec![PlannedSkill {
                name: id.to_string(),
                description: "a package that shadows a name".to_string(),
                directory: id.to_string(),
                group: None,
                entry_point: false,
            }],
            evidence: Evidence::SingleSkill,
            ambiguity: None,
            source: Default::default(),
            origin: None,
            shadows: Vec::new(),
            files: vec![("SKILL.md".to_string(), b"---\nname: x\n---\n".to_vec())],
        }
    }

    /// QA-D F10: over a SHIPPED name the pre-install check must never
    /// recommend `--force` — the installer refuses a shipped name whatever
    /// `--force` says — and must answer with the installer's own refusal,
    /// with or without the flag, before anything is written.
    ///
    /// Under the process-wide environment lock with the root pinned to a temp
    /// tree, like every test here that resolves `Paths`: the installer's guard
    /// reads the seeded root from `Paths`, and an unpinned read could land on
    /// the developer's real skills directory or race a test that moved it.
    #[test]
    fn a_shipped_name_is_never_answered_with_force_advice() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_sandbox::relocate_path_root(tmp.path());
        let root = skills_root();
        assert!(
            root.starts_with(tmp.path()),
            "the fixture resolved to {} — refusing to write there",
            root.display()
        );
        let shipped = "about-biorouter";
        assert!(
            biorouter::agents::skills_extension::is_shipped_entry_name(shipped),
            "the fixture must use a name Biorouter really ships, or this proves nothing"
        );

        // The state the QA run met: the seeded copy is on disk, so the old
        // "exists → re-run with --force" branch was reachable.
        fs::create_dir_all(root.join(shipped)).unwrap();
        for force in [false, true] {
            let err = refuse_before_installing(&[plan_named(shipped)], &root, force)
                .unwrap_err()
                .to_string();
            assert!(!err.contains("--force"), "force={force}: {err}");
            assert!(err.contains("ships with Biorouter"), "force={force}: {err}");
        }

        // The positive controls: an ordinary name that exists still gets the
        // advice, and `--force` still lets it through.
        fs::create_dir_all(root.join("my-own-skill")).unwrap();
        let err = refuse_before_installing(&[plan_named("my-own-skill")], &root, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Re-run with --force"), "{err}");
        refuse_before_installing(&[plan_named("my-own-skill")], &root, true).unwrap();

        // …and the shipped refusal belongs to Biorouter's own root only: the
        // same name under another root is somebody else's to install over.
        let elsewhere = tmp.path().join("elsewhere");
        refuse_before_installing(&[plan_named(shipped)], &elsewhere, false).unwrap();
    }
}
