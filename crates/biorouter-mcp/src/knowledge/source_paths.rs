use anyhow::{Context, Result};
use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{copy, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use tar::Archive;
use tempfile::Builder;
use xz2::read::XzDecoder;
use zip::ZipArchive;

const MAX_EXPANDED_FILES: usize = 200;
const MAX_EXPANDED_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INGEST_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_INGEST_CSV_BYTES: u64 = 8 * 1024 * 1024;
const SNIFF_BYTES: usize = 4096;
const STAGING_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const STAGING_DIR_NAME: &str = "biorouter-knowledge-expansion";

const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".tbz", ".tbz2", ".txz", ".zip", ".tar", ".gz",
    ".bz2", ".xz", ".7z", ".rar",
];

const UNSUITABLE_EXTENSIONS: &[&str] = &[
    ".app", ".bin", ".dll", ".dmg", ".dylib", ".exe", ".iso", ".msi", ".pkg", ".so",
];

const LIKELY_BINARY_EXTENSIONS: &[&str] = &[
    ".gif", ".heic", ".icns", ".ico", ".jar", ".jpeg", ".jpg", ".mov", ".mp3", ".mp4", ".otf",
    ".png", ".ttf", ".wav", ".webm", ".webp",
];

const TEXT_EXTENSIONS: &[&str] = &[
    ".c",
    ".cc",
    ".cpp",
    ".css",
    ".csv",
    ".go",
    ".graphql",
    ".h",
    ".hpp",
    ".htm",
    ".html",
    ".java",
    ".js",
    ".json",
    ".jsonl",
    ".md",
    ".markdown",
    ".mjs",
    ".py",
    ".rb",
    ".rs",
    ".sh",
    ".sql",
    ".svg",
    ".toml",
    ".ts",
    ".tsx",
    ".txt",
    ".xml",
    ".yaml",
    ".yml",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedPathFile {
    pub path: PathBuf,
    pub name: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedPathWarning {
    pub level: WarningLevel,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpandedPathSet {
    pub files: Vec<ExpandedPathFile>,
    pub warnings: Vec<ExpandedPathWarning>,
}

#[derive(Default)]
struct ExpansionState {
    file_count: usize,
    total_bytes: u64,
}

pub fn expand_ingest_path(path: &Path) -> Result<ExpandedPathSet> {
    cleanup_stale_staging_dirs()?;

    let root_label = path.file_name().and_then(OsStr::to_str).unwrap_or("staged");
    let mut out = ExpandedPathSet::default();
    let mut state = ExpansionState::default();
    collect_path(path, root_label, &mut out, &mut state)?;
    Ok(out)
}

fn collect_path(
    path: &Path,
    relative_path: &str,
    out: &mut ExpandedPathSet,
    state: &mut ExpansionState,
) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;

    if meta.is_dir() {
        return collect_directory(path, relative_path, out, state);
    }

    collect_file(path, relative_path, meta.len(), out, state)
}

fn collect_directory(
    path: &Path,
    relative_path: &str,
    out: &mut ExpandedPathSet,
    state: &mut ExpansionState,
) -> Result<()> {
    for entry in fs::read_dir(path).with_context(|| format!("read_dir {}", path.display()))? {
        let entry = entry?;
        let child_name = entry.file_name().to_string_lossy().into_owned();
        if should_ignore_relative_name(&child_name) {
            continue;
        }
        let child_relative = if relative_path.is_empty() {
            child_name
        } else {
            format!("{relative_path}/{child_name}")
        };
        collect_path(&entry.path(), &child_relative, out, state)?;
        if reached_expansion_limit(state) {
            break;
        }
    }

    Ok(())
}

fn collect_file(
    path: &Path,
    relative_path: &str,
    size: u64,
    out: &mut ExpandedPathSet,
    state: &mut ExpansionState,
) -> Result<()> {
    let ext = file_extension(path);
    let relative_lower = relative_path.to_ascii_lowercase();

    if let Some(warning) = brkb_warning(relative_path, &relative_lower) {
        out.warnings.push(warning);
        return Ok(());
    }

    if is_archive_extension(&ext) {
        return collect_archive(path, relative_path, &ext, out, state);
    }

    if is_unsuitable_extension(&ext) {
        out.warnings.push(warn(
            WarningLevel::Warning,
            "Ignored binary file",
            format!(
                "{relative_path} looks like an executable or compiled binary, so it was left out of the staged ingest set."
            ),
        ));
        return Ok(());
    }

    if let Some(limit_warning) = per_file_limit_warning(relative_path, &ext, size) {
        out.warnings.push(limit_warning);
        return Ok(());
    }

    state.total_bytes += size;
    if state.total_bytes > MAX_EXPANDED_TOTAL_BYTES {
        out.warnings.push(warn(
            WarningLevel::Error,
            "Expanded content exceeded the staging limit",
            format!(
                "Only the first {} MB of readable content was staged from this drop. Split very large folders or archives into smaller curation batches.",
                MAX_EXPANDED_TOTAL_BYTES / 1024 / 1024
            ),
        ));
        return Ok(());
    }

    if state.file_count >= MAX_EXPANDED_FILES {
        out.warnings.push(warn(
            WarningLevel::Error,
            "Expanded content exceeded the file-count limit",
            format!(
                "Only the first {MAX_EXPANDED_FILES} readable files were staged from this drop. Omit clearly unsuitable assets before retrying."
            ),
        ));
        return Ok(());
    }

    if !is_supported_readable_file(path, &ext)? {
        out.warnings.push(warn(
            WarningLevel::Warning,
            "Ignored likely binary asset",
            format!(
                "{relative_path} did not look like readable source material, so it was skipped. During curation, omit files like installers, media blobs, or generated assets that are unlikely to improve the knowledge graph."
            ),
        ));
        return Ok(());
    }

    state.file_count += 1;
    out.files.push(ExpandedPathFile {
        path: path.to_path_buf(),
        name: display_name(relative_path),
        relative_path: relative_path.to_string(),
    });

    if let Some(curate_warning) = curation_warning(relative_path, &ext, size) {
        out.warnings.push(curate_warning);
    }

    Ok(())
}

fn collect_archive(
    path: &Path,
    relative_path: &str,
    ext: &str,
    out: &mut ExpandedPathSet,
    state: &mut ExpansionState,
) -> Result<()> {
    let extracted_root = prepare_archive_staging_dir()?;
    if let Err(err) = extract_archive(path, &extracted_root) {
        out.warnings.push(warn(
            WarningLevel::Error,
            "Archive could not be expanded",
            format!(
                "{relative_path} uses a compressed format that could not be unpacked here: {err}"
            ),
        ));
        return Ok(());
    }
    let archive_root = collapse_archive_root(&extracted_root)?;
    collect_path(
        &archive_root,
        path_stem_label(relative_path, ext),
        out,
        state,
    )
}

fn brkb_warning(relative_path: &str, relative_lower: &str) -> Option<ExpandedPathWarning> {
    relative_lower.ends_with(".brkb").then(|| {
        warn(
            WarningLevel::Error,
            "Use the knowledge base importer for .brkb archives",
            format!(
                "{relative_path} looks like a full knowledge-base archive. Import it from the knowledge-base selector instead of staging it for digestion."
            ),
        )
    })
}

fn is_unsuitable_extension(ext: &str) -> bool {
    UNSUITABLE_EXTENSIONS.contains(&ext)
}

fn reached_expansion_limit(state: &ExpansionState) -> bool {
    state.file_count >= MAX_EXPANDED_FILES || state.total_bytes >= MAX_EXPANDED_TOTAL_BYTES
}

fn per_file_limit_warning(
    relative_path: &str,
    ext: &str,
    size: u64,
) -> Option<ExpandedPathWarning> {
    let limit = if ext == ".csv" {
        MAX_INGEST_CSV_BYTES
    } else {
        MAX_INGEST_FILE_BYTES
    };

    (size > limit).then(|| {
        warn(
            WarningLevel::Error,
            "File is too large to digest safely",
            format!(
                "{relative_path} is {}. The current limit is {} for this kind of staged file.",
                human_bytes(size),
                human_bytes(limit)
            ),
        )
    })
}

fn curation_warning(relative_path: &str, ext: &str, size: u64) -> Option<ExpandedPathWarning> {
    if ext == ".csv" && size > (2 * 1024 * 1024) {
        return Some(warn(
            WarningLevel::Warning,
            "Large CSV staged for curation",
            format!(
                "{relative_path} is a large table ({}). If it contains bulk numeric exports or many irrelevant columns, trim it down before digestion so the graph focuses on the readable signal.",
                human_bytes(size)
            ),
        ));
    }

    if !TEXT_EXTENSIONS.contains(&ext) && ext != ".pdf" && ext != ".docx" {
        return Some(warn(
            WarningLevel::Warning,
            "Staging as plain readable text",
            format!(
                "{relative_path} is not one of the common first-class ingest formats. Biorouter will try to read it as text, but during curation you should gloss over or omit files that are clearly not meant to encode readable knowledge."
            ),
        ));
    }

    None
}

fn is_supported_readable_file(path: &Path, ext: &str) -> Result<bool> {
    if ext == ".pdf" || ext == ".docx" {
        return Ok(true);
    }

    if TEXT_EXTENSIONS.contains(&ext) {
        return Ok(true);
    }

    if LIKELY_BINARY_EXTENSIONS.contains(&ext) {
        return Ok(false);
    }

    looks_like_text(path)
}

fn extract_archive(path: &Path, out_dir: &Path) -> Result<()> {
    let ext = file_extension(path);
    match ext.as_str() {
        ".zip" => extract_zip(path, out_dir),
        ".tar" => extract_tar(File::open(path)?, out_dir),
        ".tgz" | ".tar.gz" => extract_tar(GzDecoder::new(File::open(path)?), out_dir),
        ".tbz" | ".tbz2" | ".tar.bz2" => extract_tar(BzDecoder::new(File::open(path)?), out_dir),
        ".txz" | ".tar.xz" => extract_tar(XzDecoder::new(File::open(path)?), out_dir),
        ".gz" => inflate_single(path, out_dir, GzDecoder::new(File::open(path)?), &ext),
        ".bz2" => inflate_single(path, out_dir, BzDecoder::new(File::open(path)?), &ext),
        ".xz" => inflate_single(path, out_dir, XzDecoder::new(File::open(path)?), &ext),
        ".7z" => extract_archive_with_command("7z", &["x"], path, out_dir),
        ".rar" => extract_rar(path, out_dir),
        other => anyhow::bail!("unsupported archive extension: {other}"),
    }
}

fn extract_rar(path: &Path, out_dir: &Path) -> Result<()> {
    extract_archive_with_command("unar", &["-quiet", "-output-directory"], path, out_dir)
        .or_else(|_| extract_archive_with_command("unrar", &["x", "-o+"], path, out_dir))
}

fn extract_archive_with_command(
    command: &str,
    prefix_args: &[&str],
    path: &Path,
    out_dir: &Path,
) -> Result<()> {
    let mut cmd = Command::new(command);
    cmd.args(prefix_args);

    if command == "7z" {
        cmd.arg(path);
        cmd.arg(format!("-o{}", out_dir.display()));
        cmd.arg("-y");
    } else if command == "unar" {
        cmd.arg(out_dir);
        cmd.arg(path);
    } else {
        cmd.arg(path);
        cmd.arg(out_dir);
    }

    crate::developer::shell::strip_daemon_private_env_std(&mut cmd);
    crate::developer::shell::no_console_window_std(&mut cmd);
    let output = cmd.output().with_context(|| format!("spawn {command}"))?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{}",
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .to_string()
                .if_empty_then(|| { format!("{command} exited with status {}", output.status) })
        )
    }
}

trait StringFallback {
    fn if_empty_then(self, fallback: impl FnOnce() -> String) -> String;
}

impl StringFallback for String {
    fn if_empty_then(self, fallback: impl FnOnce() -> String) -> String {
        if self.is_empty() {
            fallback()
        } else {
            self
        }
    }
}

fn extract_zip(path: &Path, out_dir: &Path) -> Result<()> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(safe_name) = entry.enclosed_name().map(|value| value.to_path_buf()) else {
            continue;
        };
        let out_path = out_dir.join(safe_name);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out_file = File::create(out_path)?;
        copy(&mut entry, &mut out_file)?;
    }
    Ok(())
}

fn extract_tar<R: Read>(reader: R, out_dir: &Path) -> Result<()> {
    let mut archive = Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        entry.unpack_in(out_dir)?;
    }
    Ok(())
}

fn inflate_single<R: Read>(path: &Path, out_dir: &Path, mut reader: R, ext: &str) -> Result<()> {
    let out_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("expanded-file")
        .strip_suffix(ext)
        .unwrap_or("expanded-file");
    let out_path = out_dir.join(out_name);
    let mut writer = File::create(out_path)?;
    copy(&mut reader, &mut writer)?;
    Ok(())
}

fn looks_like_text(path: &Path) -> Result<bool> {
    let mut file = File::open(path)?;
    let mut buf = [0_u8; SNIFF_BYTES];
    let read = file.read(&mut buf)?;
    let slice = &buf[..read];
    if slice.is_empty() {
        return Ok(true);
    }
    if slice.contains(&0) {
        return Ok(false);
    }
    let suspicious = slice
        .iter()
        .filter(|byte| {
            let value = **byte;
            !(value == b'\t'
                || value == b'\n'
                || value == b'\r'
                || (32..=126).contains(&value)
                || value >= 160)
        })
        .count();
    Ok((suspicious as f32) / (slice.len() as f32) < 0.2)
}

fn file_extension(path: &Path) -> String {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    ARCHIVE_EXTENSIONS
        .iter()
        .find(|candidate| lower.ends_with(**candidate))
        .map(|value| (*value).to_string())
        .unwrap_or_else(|| {
            path.extension()
                .map(|value| format!(".{}", value.to_string_lossy().to_ascii_lowercase()))
                .unwrap_or_default()
        })
}

fn is_archive_extension(ext: &str) -> bool {
    ARCHIVE_EXTENSIONS.contains(&ext)
}

fn display_name(relative_path: &str) -> String {
    let normalized = relative_path.replace('\\', "/");
    let parts: Vec<_> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() <= 1 {
        return parts
            .first()
            .copied()
            .unwrap_or("staged-file.txt")
            .to_string();
    }
    format!("{}__{}", parts[parts.len() - 2], parts[parts.len() - 1])
}

fn path_stem_label<'a>(relative_path: &'a str, ext: &str) -> &'a str {
    relative_path.strip_suffix(ext).unwrap_or(relative_path)
}

fn should_ignore_relative_name(name: &str) -> bool {
    name == "__MACOSX" || name == ".DS_Store" || name.starts_with("._")
}

fn collapse_archive_root(extracted_root: &Path) -> Result<PathBuf> {
    let mut visible_children = fs::read_dir(extracted_root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !should_ignore_relative_name(&name)
        })
        .collect::<Vec<_>>();

    if visible_children.len() == 1 {
        let only_child = visible_children.pop().expect("len checked");
        if only_child.file_type()?.is_dir() {
            return Ok(only_child.path());
        }
    }

    Ok(extracted_root.to_path_buf())
}

fn warn(
    level: WarningLevel,
    title: impl Into<String>,
    message: impl Into<String>,
) -> ExpandedPathWarning {
    ExpandedPathWarning {
        level,
        title: title.into(),
        message: message.into(),
    }
}

fn prepare_archive_staging_dir() -> Result<PathBuf> {
    let root = staging_root();
    fs::create_dir_all(&root).with_context(|| format!("create_dir_all {}", root.display()))?;
    let dir = Builder::new()
        .prefix("expanded-")
        .tempdir_in(&root)
        .with_context(|| format!("tempdir_in {}", root.display()))?;
    Ok(dir.keep())
}

fn staging_root() -> PathBuf {
    std::env::temp_dir().join(STAGING_DIR_NAME)
}

fn cleanup_stale_staging_dirs() -> Result<()> {
    let root = staging_root();
    if !root.exists() {
        return Ok(());
    }

    let cutoff = SystemTime::now()
        .checked_sub(STAGING_TTL)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    for entry in fs::read_dir(&root).with_context(|| format!("read_dir {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if modified <= cutoff {
            let _ = fs::remove_dir_all(&path);
        }
    }

    Ok(())
}

fn human_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        let whole = bytes / MIB;
        if bytes >= 10 * MIB {
            format!("{whole} MB")
        } else {
            format!("{:.1} MB", bytes as f64 / MIB as f64)
        }
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn expands_directory_and_ignores_binary() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("note.md"), "# Hello").unwrap();
        fs::write(dir.path().join("run.exe"), [0, 1, 2, 3]).unwrap();

        let result = expand_ingest_path(dir.path()).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(
            result.files[0].relative_path,
            format!(
                "{}/note.md",
                dir.path().file_name().unwrap().to_string_lossy()
            )
        );
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn expands_zip_archive() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("bundle.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::FileOptions::default();
        zip.start_file("bundle/docs/readme.md", options).unwrap();
        zip.write_all(b"# Archive file").unwrap();
        zip.start_file("__MACOSX/bundle/docs/._readme.md", options)
            .unwrap();
        zip.write_all(b"metadata").unwrap();
        zip.finish().unwrap();

        let result = expand_ingest_path(&zip_path).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, "bundle/docs/readme.md");
        assert!(result.files[0].path.exists());
    }

    #[test]
    fn warns_for_large_csvs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.csv");
        fs::write(&path, vec![b'a'; (MAX_INGEST_CSV_BYTES + 1) as usize]).unwrap();

        let result = expand_ingest_path(&path).unwrap();
        assert!(result.files.is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.title == "File is too large to digest safely"));
    }
}
