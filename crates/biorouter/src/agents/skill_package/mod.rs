//! One import pipeline for skill packages (#115).
//!
//! # The failure this replaces
//!
//! Given a repository URL, BioRouter had no repository-aware installer, so an
//! agent improvised with shell and file copies and produced one unrelated
//! top-level skill per `SKILL.md`. For a coordinated package such as
//! [HyperFrames](https://github.com/heygen-com/hyperframes) — a declared plugin
//! with 20 component skills and a mandatory `hyperframes` router — flattening
//! discards **behaviour**, not merely grouping: the entry-point relationship,
//! the core-versus-on-demand distinction, the package's identity, coordinated
//! updates, and install/remove as one unit.
//!
//! The ZIP path was no better. Both the daemon and the CLI recognised exactly
//! three shapes — `SKILL.md`, `<skill>/SKILL.md`, `<bundle>/<skill>/SKILL.md` —
//! by counting slashes, and read no manifest at all. A normal GitHub source
//! archive is `<repo>-<ref>/skills/<name>/SKILL.md`, which matches none of them.
//!
//! # The rules this module is built on
//!
//! **Metadata beats shape.** The detection ladder is
//! [`manifest::detect`]: a BioRouter package manifest, then
//! `.codex-plugin/plugin.json`'s skills path, then a compatible
//! `.claude-plugin/plugin.json`, then `skills-manifest.json`, and only then
//! structural inference. A rung that fires records *which* rung it was, so the
//! preview can say why it thinks what it thinks.
//!
//! **A shared name prefix is never the signal.** HyperFrames' members include
//! `media-use`, `slideshow`, `product-launch-video` and `faceless-explainer`,
//! none of which begin with `hyperframes-`. Detection uses structure and
//! manifests, and every component keeps its declared name exactly — no prefix
//! is added as a grouping mechanism, because the frontmatter `name` is the
//! identifier every enablement surface keys on.
//!
//! **Ambiguity is a question, not a default.** When no manifest speaks and the
//! structure alone could mean either one package or several unrelated skills,
//! [`ImportPlan::ambiguity`] is set and the caller must choose. Flattening by
//! default is the behaviour this issue exists to remove.
//!
//! **A partial write never lands.** [`install::install`] stages the whole
//! package in a sibling directory and swaps it in with renames, so a failure
//! anywhere leaves either the previous version or nothing — never half a
//! package.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub mod archive;
pub mod install;
pub mod manifest;
pub mod pending;
pub mod plan;
pub mod source;

#[cfg(test)]
mod tests;

pub use install::{install, remove, remove_keeping_choices, InstalledPackage};
pub use plan::plan_from_entries;
pub use source::{fetch, ImportSource};

/// Which shape the importer decided on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ImportKind {
    /// One skill, installed at `<root>/<slug>/SKILL.md`.
    Single,
    /// A package, installed at `<root>/<id>/<component>/SKILL.md` with a
    /// `biorouter-package.json` record at its root.
    Bundle,
}

/// Which rung of the detection ladder decided the shape. Reported so a preview
/// can explain itself, and so a test can assert that the *manifest* decided
/// rather than the structure happening to agree with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum Evidence {
    /// `biorouter-package.json` — a package this importer wrote before.
    BiorouterManifest,
    /// `.codex-plugin/plugin.json`, with its `skills` path.
    CodexPlugin,
    /// `.claude-plugin/plugin.json`.
    ClaudePlugin,
    /// `skills-manifest.json`.
    SkillsManifest,
    /// A `skills/` directory holding one folder per skill.
    SkillsDirectory,
    /// Several sibling `<name>/SKILL.md` folders and nothing else to go on.
    StructuralInference,
    /// Exactly one `SKILL.md`.
    SingleSkill,
}

/// Why the caller has to choose, and what the choices are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ambiguity {
    /// One sentence for the dialog.
    pub reason: String,
    /// The component names the caller is choosing among.
    pub components: Vec<String>,
}

/// What a resolved import came from, kept so an update knows where to look and
/// a user can see what they installed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceProvenance {
    pub url: Option<String>,
    /// The branch, tag or commit that was asked for.
    pub reference: Option<String>,
    /// The immutable commit the archive actually came from, when the host tells
    /// us (GitHub returns it in `ETag`).
    pub resolved_commit: Option<String>,
    /// `repository` | `archive` | `marketplace` | `cli` | `agent`.
    pub installer: Option<String>,
}

/// One component of a planned import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlannedSkill {
    /// The frontmatter `name`, preserved exactly.
    pub name: String,
    pub description: String,
    /// Directory name under the package root. Taken from the source layout so
    /// a manifest referring to `skills/media-use` still resolves after install.
    pub directory: String,
    /// The manifest's group for this component, e.g. `core` / `on-demand`.
    pub group: Option<String>,
    /// The router the package declares as its way in.
    pub entry_point: bool,
}

/// A complete, reviewable description of what an install would do.
///
/// It carries the file bytes, so previewing and installing cannot disagree
/// about what was in the archive — an install re-fetching the source could
/// legitimately get a different one.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub kind: ImportKind,
    /// The install directory name under the skills root.
    pub id: String,
    pub display_name: String,
    pub version: Option<String>,
    pub entry_point: Option<String>,
    pub groups: BTreeMap<String, Vec<String>>,
    pub components: Vec<PlannedSkill>,
    pub evidence: Evidence,
    pub ambiguity: Option<Ambiguity>,
    pub source: SourceProvenance,
    /// The approval card's rendering of where this came from — exactly the JSON
    /// `fresh_import_source` builds, carried through a parked plan.
    ///
    /// ⚠ It exists because `SourceProvenance` cannot answer for a local
    /// archive: that variant carries no path, so the card for a file import
    /// rendered `{"url":null,"reference":null,"resolvedCommit":null,
    /// "installer":"archive"}` — four fields, three null, naming nothing the
    /// user could recognise. On the `dry_run` → `needsChoice` → `plan_id` path
    /// that is the ONLY approval anyone sees.
    ///
    /// Deliberately NOT a `SourceProvenance` change: that type is
    /// `utoipa::ToSchema` and rides in `ImportPreview`, so widening it would
    /// force an OpenAPI and TypeScript-client regeneration for a card's label.
    pub origin: Option<serde_json::Value>,
    /// Component names this package would shadow, or be shadowed by, elsewhere
    /// on the machine. A warning, not a refusal: later roots already shadow
    /// earlier ones by design.
    pub shadows: Vec<String>,
    /// Package-relative path → bytes, already normalised.
    pub files: Vec<(String, Vec<u8>)>,
}

/// The serialisable half of a plan — everything except the file bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub kind: ImportKind,
    pub id: String,
    pub display_name: String,
    pub version: Option<String>,
    pub entry_point: Option<String>,
    #[schema(value_type = Object, additional_properties)]
    pub groups: BTreeMap<String, Vec<String>>,
    pub components: Vec<PlannedSkill>,
    pub evidence: Evidence,
    pub ambiguity: Option<Ambiguity>,
    pub source: SourceProvenance,
    pub shadows: Vec<String>,
    /// How many files would be written.
    pub file_count: usize,
}

impl ImportPlan {
    pub fn preview(&self) -> ImportPreview {
        ImportPreview {
            kind: self.kind,
            id: self.id.clone(),
            display_name: self.display_name.clone(),
            version: self.version.clone(),
            entry_point: self.entry_point.clone(),
            groups: self.groups.clone(),
            components: self.components.clone(),
            evidence: self.evidence.clone(),
            ambiguity: self.ambiguity.clone(),
            source: self.source.clone(),
            shadows: self.shadows.clone(),
            file_count: self.files.len(),
        }
    }

    /// Where this package would be installed.
    pub fn destination(&self, root: &std::path::Path) -> PathBuf {
        root.join(&self.id)
    }

    /// Answer an ambiguity by taking the whole thing as one package.
    ///
    /// It only clears the question — the plan was already a bundle, because
    /// that is the shape [`plan_from_entries`] builds before asking.
    pub fn as_bundle(mut self) -> Self {
        self.ambiguity = None;
        self
    }

    /// Answer an ambiguity by keeping only the named components, **each as its
    /// own top-level skill**.
    ///
    /// ⚠ This is the *flattening* path, and it exists only because a person
    /// asked for it in the dialog. It is never a default, and it returns one
    /// plan per component rather than a one-component bundle: a bundle of one
    /// installs a directory level deeper than every other single-skill install,
    /// which is not what "install these separately" means.
    pub fn into_individual(self, keep: &[String]) -> Vec<ImportPlan> {
        self.components
            .iter()
            .filter(|component| keep.iter().any(|k| k == &component.name))
            .filter_map(|component| {
                let prefix = format!("{}/", component.directory);
                let files: Vec<(String, Vec<u8>)> = self
                    .files
                    .iter()
                    .filter_map(|(path, data)| {
                        path.strip_prefix(&prefix)
                            .map(|rest| (rest.to_string(), data.clone()))
                    })
                    .collect();
                Some(ImportPlan {
                    // ⚠ Copied, not defaulted. A flattening answer is still
                    // the same import, and the second approval card must name
                    // the same source the first one did.
                    origin: self.origin.clone(),
                    kind: ImportKind::Single,
                    id: sanitize_package_id(&component.name)?,
                    display_name: component.name.clone(),
                    version: self.version.clone(),
                    entry_point: None,
                    groups: BTreeMap::new(),
                    components: vec![PlannedSkill {
                        directory: String::new(),
                        entry_point: false,
                        group: None,
                        ..component.clone()
                    }],
                    evidence: Evidence::SingleSkill,
                    ambiguity: None,
                    source: self.source.clone(),
                    shadows: Vec::new(),
                    files,
                })
            })
            .collect()
    }
}

/// How a package's install directory is named.
///
/// ⚠ **Never `skills`.** Zipping only a repository's `skills/` directory
/// produces an archive whose one top-level folder is literally called that, and
/// the old depth-counting parser would have installed HyperFrames as a bundle
/// named "skills" — losing the declared package identity outright. Every
/// candidate is filtered through here.
pub fn sanitize_package_id(raw: &str) -> Option<String> {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() || slug == "skills" || slug == "." || slug == ".." {
        return None;
    }
    Some(slug)
}
