use std::path::{Path, PathBuf};

use crate::AppError;
use crate::sandbox::provider_files::materialize;
use crate::sandbox::provider_spec::ProviderSpec;

pub(crate) const PROVIDERS_DIR: &str = "providers";
pub(crate) const SANDBOXES_DIR: &str = "sandboxes";

/// The materialized provider directory and the root the sandboxes live under.
pub(crate) fn resolve(root: &Path, spec: &ProviderSpec) -> Result<(PathBuf, PathBuf), AppError> {
    let provider_dir = materialize(&root.join(PROVIDERS_DIR), spec)?;
    Ok((provider_dir, root.join(SANDBOXES_DIR)))
}
