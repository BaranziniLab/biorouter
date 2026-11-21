use crate::AppError;
use crate::sandbox::sandbox_record::SandboxRecord;

/// One named sandbox directory and whether its record could be read.
pub(crate) struct SandboxEntry {
    pub(crate) name: String,
    pub(crate) record: Result<SandboxRecord, AppError>,
}
