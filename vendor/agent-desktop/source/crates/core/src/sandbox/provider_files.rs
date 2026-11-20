use crate::AppError;
use crate::sandbox::provider_spec::ProviderSpec;
use crate::storage::private_file::write_atomic;
use crate::storage::private_file_parent::ensure_private;
use rustc_hash::FxHasher;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

pub(crate) fn materialize(root: &Path, spec: &ProviderSpec) -> Result<PathBuf, AppError> {
    let dir = root.join(spec.name).join(content_hash(spec));
    ensure_private(&dir)?;
    for (file, contents) in spec.files {
        let path = dir.join(file);
        if !path.is_file() {
            write_atomic(&path, contents.as_bytes())?;
        }
    }
    Ok(dir)
}

fn content_hash(spec: &ProviderSpec) -> String {
    let mut hasher = FxHasher::default();
    for (file, contents) in spec.files {
        hasher.write_usize(file.len());
        hasher.write(file.as_bytes());
        hasher.write_usize(contents.len());
        hasher.write(contents.as_bytes());
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
#[path = "provider_files_tests.rs"]
mod tests;
