// DR-20 requirement (2), and DR-24 Step 3. `privacy-test-auth` compiles a seam
// (`privacy/system_auth_seam.rs`) that answers the declassification system
// prompt without an operating system prompt. It exists for tests and for driving
// the dev GUI. It may never be compiled with debug assertions off — which is
// every profile this workspace ships: `release`, and `release-dist` and `quick`,
// which both inherit it.
//
// This is a COMPILER error, not a script a release path could skip, which is
// what makes it stronger than the `scripts/check-*.sh` convention the rest of
// this repo uses for its invariants.
#[cfg(all(feature = "privacy-test-auth", not(debug_assertions)))]
compile_error!(
    "privacy-test-auth is a TEST SEAM that bypasses the DR-20 declassification \
     prompt and must never be compiled into a release profile. Drop the feature \
     from this build; do not relax this guard."
);

pub mod action_required_manager;
pub mod agents;
pub mod catalog;
pub mod catalog_search;
pub mod checkpoint;
pub mod config;
pub mod context_budget;
pub mod context_mgmt;
pub mod conversation;
pub mod execution;
pub mod extension_install;
pub mod guardrails;
pub mod hints;
pub mod hooks;
pub mod knowledge;
pub mod logging;
pub mod managed;
pub mod marketplace;
pub mod mcp_utils;
pub mod model;
pub mod oauth;
pub mod observability;
pub mod pending_user_action;
pub mod permission;
pub mod privacy;
pub mod prompt_template;
pub mod providers;
pub mod scheduler;
pub mod scheduler_trait;
pub mod security;
pub mod session;
pub mod session_context;
pub mod session_events;
pub mod session_meta;
pub mod slash_commands;
pub mod subprocess;
pub mod system;
/// Test-binary-only: pin `BIOROUTER_PATH_ROOT` under a temp dir before any test
/// runs, so a default `cargo test -p biorouter --lib` cannot write into the
/// developer's real `~/.config/biorouter`.
#[cfg(test)]
mod test_sandbox;
pub mod token_counter;
pub mod tool_inspection;
pub mod tool_monitor;
pub mod tracing;
pub mod user_surface;
pub mod utils;
pub mod workflow;
pub mod workflow_deeplink;
pub mod workspace_services;
