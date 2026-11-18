//! Reading an archive, and stripping the directory a source archive wraps
//! itself in.
//!
//! GitHub's source archives are wrapped: `hyperframes-main/skills/…`. So are
//! most hand-made ones. Stripping that wrapper before anything looks at the
//! structure is what lets the rest of this module reason about a repository's
//! real layout — the previous parser counted slashes on the *wrapped* names and
//! therefore recognised nothing.
//!
//! [`read_package`] is what the importer calls. It reads every name from the
//! central directory first, decides the wrapper on the whole list, and then
//! inflates only the files a plan can use. A repository's docs, registry and
//! build output are never inflated, so they never count against the limit.

use anyhow::{bail, Result};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

/// Ceiling on the total **uncompressed** size of what is read from an archive,
/// and on the size of a downloaded one. Enforced while reading rather than
/// from the archive's own declared sizes, which the person who built the
/// archive chose.
pub const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// The files at the (unwrapped) archive root that the detection ladder reads.
/// Kept in step with `manifest::detect` and `plan::MANIFEST_FILES`: a file a
/// plan reads and this list leaves out would be silently missing from it.
const MANIFEST_INPUTS: &[&str] = &[
    crate::agents::skill_catalog::PACKAGE_RECORD_FILE,
    ".codex-plugin/plugin.json",
    ".claude-plugin/plugin.json",
    "skills-manifest.json",
];

/// The shared package folders a bundle keeps beside its components, as
/// `plan::keep_bundle_files` names them.
const SHARED_ROOTS: &[&str] = &["assets", "references", "scripts"];

/// The one sentence a person reads when a source is over `limit`, whichever
/// limit it hit: the downloaded archive, or the skill files read from it.
///
/// It names the limit in a unit people use and what to import instead. It
/// used to be "the archive expands past the 268435456 byte limit", which gave
/// a raw count and no way on.
///
/// ⚠ **The remedies are the ones that work.** A release tag is usually a
/// smaller archive, and a `.zip` of only the skills folder holds nothing else.
/// A `…/tree/<branch>/skills` URL is not offered: GitHub serves a whole
/// repository as one archive, so `source.rs` reads `main/skills` as a ref
/// name and the lookup answers 404.
pub fn too_large_message(limit: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    let size = if limit >= MIB {
        format!("{} MB", limit / MIB)
    } else {
        format!("{} KB", (limit / 1024).max(1))
    };
    format!(
        "This source is over Biorouter's {size} import limit, so import a release tag \
         (…/tree/<tag>) or a .zip of only its skills folder instead."
    )
}

/// One file from an archive. Directories are dropped on read — every shape
/// question here is answered by where the *files* are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `/`-separated, relative, with `..` already refused.
    pub name: String,
    pub data: Vec<u8>,
}

impl Entry {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).to_string()
    }
}

/// Refuse a zip entry that would escape the directory it is unpacked into.
///
/// Absolute paths, `..` anywhere, NUL bytes, and Windows separators (folded to
/// `/` first, so `..\..\etc` is caught too).
pub fn safe_entry_name(raw: &str) -> Result<String> {
    if raw.is_empty() || raw.contains('\0') {
        bail!("unsafe archive entry path: {raw}");
    }
    let folded = raw.replace('\\', "/");
    if folded.starts_with('/') || Path::new(&folded).is_absolute() {
        bail!("unsafe archive entry path: {raw}");
    }
    let parts: Vec<&str> = folded
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.contains(&"..") {
        bail!("unsafe archive entry path: {raw}");
    }
    if parts.is_empty() {
        bail!("unsafe archive entry path: {raw}");
    }
    Ok(parts.join("/"))
}

/// One file named in an archive's central directory, before anything is
/// inflated.
struct Listed {
    index: usize,
    /// Safe, `/`-separated, and stripped of the wrapper once that is known.
    name: String,
    /// What the archive says the file expands to. A hint for failing early,
    /// never the bound itself: the archive's author chose it.
    declared: u64,
}

fn open(bytes: &[u8]) -> Result<zip::ZipArchive<std::io::Cursor<&[u8]>>> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| anyhow::anyhow!("this does not look like a .zip archive: {e}"))
}

/// Every file's name and declared size, from the central directory. Nothing
/// is inflated. An unsafe name anywhere refuses the whole archive, whether or
/// not a plan would read that file.
fn list(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Result<Vec<Listed>> {
    let mut listed = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index_raw(index)
            .map_err(|e| anyhow::anyhow!("could not read the archive: {e}"))?;
        if file.is_dir() {
            continue;
        }
        listed.push(Listed {
            index,
            name: safe_entry_name(file.name())?,
            declared: file.size(),
        });
    }
    if listed.is_empty() {
        bail!("the archive is empty");
    }
    Ok(listed)
}

/// Inflate `wanted`, in archive order, bounded by `limit` while reading.
fn inflate(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    wanted: Vec<Listed>,
    limit: u64,
) -> Result<Vec<Entry>> {
    // Fail before inflating anything when the archive itself says it is too
    // large. An archive that understates is caught by the budget below.
    let declared = wanted
        .iter()
        .fold(0u64, |sum, file| sum.saturating_add(file.declared));
    if declared > limit {
        bail!(too_large_message(limit));
    }
    let mut entries = Vec::with_capacity(wanted.len());
    let mut budget = limit;
    for file in wanted {
        let mut reader = archive
            .by_index(file.index)
            .map_err(|e| anyhow::anyhow!("could not read the archive: {e}"))?;
        let mut data = Vec::new();
        // One byte past the remaining budget, so "exactly fills it" and
        // "overruns it" are distinguishable.
        reader
            .by_ref()
            .take(budget.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|e| anyhow::anyhow!("could not read the archive: {e}"))?;
        if data.len() as u64 > budget {
            bail!(too_large_message(limit));
        }
        budget -= data.len() as u64;
        entries.push(Entry {
            name: file.name,
            data,
        });
    }
    Ok(entries)
}

/// Read a `.zip` into entries, every file inflated, bounded by
/// [`MAX_ARCHIVE_BYTES`].
///
/// The importer calls [`read_package`] instead, which inflates only what a
/// plan reads. The fixture matrix runs both and checks they plan alike.
pub fn read_zip(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut archive = open(bytes)?;
    let listed = list(&mut archive)?;
    inflate(&mut archive, listed, MAX_ARCHIVE_BYTES)
}

/// Read a package archive the way the importer needs it: the wrapper stripped,
/// and only the files a plan can use inflated, within [`MAX_ARCHIVE_BYTES`].
///
/// Returns the entries and the wrapper directory, exactly as
/// `strip_wrapper(read_zip(bytes)?, hint)` would, minus the files no plan
/// reads (see [`plan_reads`]).
///
/// ⚠ **The wrapper is decided on every name, before anything is dropped.**
/// `common_root` and the reveal test both look at the whole archive: a
/// `README.md` beside a folder, or any file under `skills/`, changes the
/// answer. Deciding it on the kept files alone would change plans.
pub fn read_package(bytes: &[u8], hint: WrapperHint<'_>) -> Result<(Vec<Entry>, Option<String>)> {
    read_package_limited(bytes, hint, MAX_ARCHIVE_BYTES)
}

/// [`read_package`] with the limit as an argument, so a test can exceed it with
/// a few kilobytes.
pub(crate) fn read_package_limited(
    bytes: &[u8],
    hint: WrapperHint<'_>,
    limit: u64,
) -> Result<(Vec<Entry>, Option<String>)> {
    let mut archive = open(bytes)?;
    let mut listed = list(&mut archive)?;

    let wrapper = {
        let names: Vec<&str> = listed.iter().map(|file| file.name.as_str()).collect();
        wrapper_for(&names, hint)
    };
    if let Some(wrapper) = &wrapper {
        let prefix = format!("{wrapper}/");
        // A wrapper is the common root, so every name carries the prefix.
        listed = listed
            .into_iter()
            .filter_map(|mut file| {
                let name = file.name.strip_prefix(&prefix)?.to_string();
                file.name = name;
                Some(file)
            })
            .collect();
    }

    let keep = {
        let names: Vec<&str> = listed.iter().map(|file| file.name.as_str()).collect();
        plan_reads(&names)
    };
    let wanted = listed
        .into_iter()
        .zip(keep)
        .filter_map(|(file, keep)| keep.then_some(file))
        .collect();
    Ok((inflate(&mut archive, wanted, limit)?, wrapper))
}

/// Which of `names` (already unwrapped) a plan can read.
///
/// A superset of what `plan::plan_from_entries` and `manifest::detect` read,
/// so dropping the rest changes no plan:
/// - the manifests at the root ([`MANIFEST_INPUTS`]);
/// - every file under a directory holding a `SKILL.md`, and every file at all
///   when the root holds one (a root skill owns the whole archive);
/// - the shared folders ([`SHARED_ROOTS`]) at the root or beside any skill's
///   parent, where a bundle keeps its common assets.
///
/// ⚠ Keeping too much costs only budget. Keeping too little drops a file from
/// an install without a word, so when `plan.rs` or `manifest.rs` learns to
/// read another file, it goes here too.
fn plan_reads(names: &[&str]) -> Vec<bool> {
    let skill_dirs: BTreeSet<&str> = names
        .iter()
        .filter_map(|name| {
            if *name == "SKILL.md" {
                Some("")
            } else {
                name.strip_suffix("/SKILL.md")
            }
        })
        .collect();
    if skill_dirs.contains("") {
        return vec![true; names.len()];
    }

    let mut parents: BTreeSet<&str> = BTreeSet::from([""]);
    for dir in &skill_dirs {
        let mut current = *dir;
        while let Some((parent, _)) = current.rsplit_once('/') {
            parents.insert(parent);
            current = parent;
        }
    }

    let mut prefixes: Vec<String> = skill_dirs.iter().map(|dir| format!("{dir}/")).collect();
    for parent in &parents {
        for shared in SHARED_ROOTS {
            prefixes.push(if parent.is_empty() {
                format!("{shared}/")
            } else {
                format!("{parent}/{shared}/")
            });
        }
    }

    names
        .iter()
        .map(|name| {
            MANIFEST_INPUTS.contains(name)
                || prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix.as_str()))
        })
        .collect()
}

/// The single directory every entry sits inside, if there is one.
pub fn common_root(entries: &[Entry]) -> Option<String> {
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    common_root_of(&names)
}

fn common_root_of(names: &[&str]) -> Option<String> {
    let roots: BTreeSet<&str> = names
        .iter()
        .filter_map(|name| name.split('/').next().filter(|c| !c.is_empty()))
        .collect();
    if roots.len() != 1 {
        return None;
    }
    let root = *roots.iter().next()?;
    // A root that is itself a file (`SKILL.md` at the top) is not a wrapper.
    names
        .iter()
        .all(|name| name.len() > root.len())
        .then(|| root.to_string())
}

/// What the caller knows about the archive's wrapper directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapperHint<'a> {
    /// A source archive downloaded from a code host. GitHub's always wraps its
    /// contents in exactly one directory, so the single common root **is** the
    /// wrapper whatever it is named.
    ///
    /// ⚠ Knowing this rather than predicting the name matters: codeload's
    /// directory is `<repo>-<branch>` with `/` folded to `-`, and for a tag it
    /// also drops a leading `v` — so `refs/tags/v1.0` unpacks into `repo-1.0`.
    /// A predicted name would silently fail to match on exactly the refs a
    /// user is most likely to pin to.
    SourceArchive,
    /// This exact directory, if the archive has it as its single root.
    Named(&'a str),
    /// Nothing known.
    Infer,
}

/// Strip the archive's wrapper directory, if it has one.
///
/// ⚠ **The `Infer` rule is the load-bearing one.** With no hint, the wrapper is
/// stripped only when stripping **reveals** package structure: a manifest, a
/// `skills/` directory, or a root `SKILL.md`. That is what keeps a genuine
/// bundle archive — `pack/alpha/SKILL.md`, `pack/beta/SKILL.md` — from being
/// unwrapped into two unrelated skills, which is exactly the flattening #115 is
/// about. A repository whose root holds one folder per skill has the same shape
/// as that bundle, so nothing but the caller's own knowledge can separate them.
pub fn strip_wrapper(entries: Vec<Entry>, hint: WrapperHint<'_>) -> (Vec<Entry>, Option<String>) {
    let wrapper = {
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        wrapper_for(&names, hint)
    };
    let Some(root) = wrapper else {
        return (entries, None);
    };
    let prefix = format!("{root}/");

    let stripped = entries
        .into_iter()
        .filter_map(|entry| {
            entry.name.strip_prefix(&prefix).map(|name| Entry {
                name: name.to_string(),
                data: entry.data,
            })
        })
        .collect();
    (stripped, Some(root))
}

/// The wrapper directory `strip_wrapper` would remove, decided on names alone.
fn wrapper_for(names: &[&str], hint: WrapperHint<'_>) -> Option<String> {
    let root = common_root_of(names)?;
    let known = match hint {
        WrapperHint::SourceArchive => true,
        WrapperHint::Named(expected) => expected == root,
        WrapperHint::Infer => false,
    };
    (known || reveals_structure(names, &format!("{root}/"))).then_some(root)
}

fn reveals_structure(names: &[&str], prefix: &str) -> bool {
    names.iter().any(|name| {
        let Some(rest) = name.strip_prefix(prefix) else {
            return false;
        };
        rest == "SKILL.md"
            || rest == "skills-manifest.json"
            || rest == crate::agents::skill_catalog::PACKAGE_RECORD_FILE
            || rest.starts_with("skills/")
            || rest.starts_with(".codex-plugin/")
            || rest.starts_with(".claude-plugin/")
    })
}

/// Every `SKILL.md` in the entry set, as `(directory, entry)` where the
/// directory is `""` for one at the root.
pub fn skill_files(entries: &[Entry]) -> Vec<(String, &Entry)> {
    entries
        .iter()
        .filter(|entry| entry.name == "SKILL.md" || entry.name.ends_with("/SKILL.md"))
        .map(|entry| {
            let directory = entry
                .name
                .strip_suffix("SKILL.md")
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string();
            (directory, entry)
        })
        .collect()
}

/// Every entry under `directory` (or all of them when it is empty), renamed
/// relative to it.
pub fn entries_under(entries: &[Entry], directory: &str) -> Vec<Entry> {
    if directory.is_empty() {
        return entries.to_vec();
    }
    let prefix = format!("{directory}/");
    entries
        .iter()
        .filter_map(|entry| {
            entry.name.strip_prefix(&prefix).map(|name| Entry {
                name: name.to_string(),
                data: entry.data.clone(),
            })
        })
        .collect()
}
