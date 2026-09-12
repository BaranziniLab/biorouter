pub mod cli;
pub mod commands;
pub mod logging;
pub mod project_tracker;
pub mod scenario_tests;
pub mod session;
pub mod signal;
// Redirects this test binary's config/data root at a throwaway directory
// before `main`. `#[cfg(test)]`, so it is compiled out of the shipped
// `biorouter` binary and out of every integration binary — each `tests/*.rs`
// declares its own copy, and `tests/cli_test_binaries_are_sandboxed.rs` is what
// notices when one does not.
#[cfg(test)]
mod test_sandbox;
pub mod workflows;

// Re-export commonly used types
pub use session::CliSession;
