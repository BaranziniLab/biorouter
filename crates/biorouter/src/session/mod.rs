pub(crate) mod chat_fts;
pub mod chat_history_search;
mod diagnostics;
pub mod extension_data;
mod legacy;
pub mod message_blobs;
pub mod session_manager;

pub use chat_history_search::SearchReach;
/// For `providers::utils`'s test that a request log lands where a bundle reads.
#[cfg(test)]
pub(crate) use diagnostics::DiagnosticsSources;
pub use diagnostics::{generate_diagnostics, get_system_info, SystemInfo};
pub use extension_data::{
    EnabledExtensionsState, ExtensionData, ExtensionState, TodoItem, TodoState, TodoStatus,
};
pub use session_manager::{
    ActivityWindow, DailyActivity, ModelUsageRow, Session, SessionInsights, SessionManager,
    SessionSummary, SessionType, SessionUpdateBuilder, UsageGroup, UsageReportRow, UsageSummary,
    UsageTotals, WorkingDirUpdate, DEFAULT_SESSION_NAME,
};
