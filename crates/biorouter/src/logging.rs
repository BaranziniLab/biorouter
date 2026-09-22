use crate::config::paths::Paths;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Returns the directory where log files should be stored for a specific component.
/// Creates the directory structure if it doesn't exist.
///
/// # Arguments
///
/// * `component` - The component name (e.g., "cli", "server", "debug", "llm")
/// * `use_date_subdir` - Whether to create a date-based subdirectory
pub fn prepare_log_directory(component: &str, use_date_subdir: bool) -> Result<PathBuf> {
    prepare_log_directory_in(&Paths::in_state_dir("logs"), component, use_date_subdir)
}

/// [`prepare_log_directory`] under `base_log_dir` instead of the state dir's
/// `logs/`. Only the tests below call it: they pass a directory of their own
/// rather than moving `BIOROUTER_PATH_ROOT` (see `test_sandbox::in_a_process_of_its_own`).
fn prepare_log_directory_in(
    base_log_dir: &Path,
    component: &str,
    use_date_subdir: bool,
) -> Result<PathBuf> {
    if let Err(e) = cleanup_old_logs_in(base_log_dir, component) {
        tracing::warn!("Log cleanup failed: {}", e);
    }

    let component_dir = base_log_dir.join(component);

    let log_dir = if use_date_subdir {
        component_dir.join(chrono::Local::now().format("%Y-%m-%d").to_string())
    } else {
        component_dir
    };

    fs::create_dir_all(&log_dir)
        .with_context(|| format!("Failed to create log directory: {:?}", log_dir))?;

    Ok(log_dir)
}

pub fn cleanup_old_logs(component: &str) -> Result<()> {
    cleanup_old_logs_in(&Paths::in_state_dir("logs"), component)
}

fn cleanup_old_logs_in(base_log_dir: &Path, component: &str) -> Result<()> {
    let component_dir = base_log_dir.join(component);

    if !component_dir.exists() {
        return Ok(());
    }

    let two_weeks = SystemTime::now() - Duration::from_secs(14 * 24 * 60 * 60);
    let entries = fs::read_dir(&component_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();

        if let Ok(metadata) = entry.metadata() {
            if let Ok(modified) = metadata.modified() {
                if modified < two_weeks && path.is_dir() {
                    if let Err(e) = fs::remove_dir_all(&path) {
                        tracing::warn!("Failed to clean up old log directory {:?}: {}", path, e);
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// A `logs/` directory of the test's own, handed to
    /// `prepare_log_directory_in`.
    ///
    /// These tests used to point `BIOROUTER_PATH_ROOT` at a `TempDir` under the
    /// env lock and call `prepare_log_directory`. The lock ordered them against
    /// the other tests that took it and against nothing else, and the (at least)
    /// 466 tests here that resolve `Paths` without it could take that `TempDir`
    /// for their state, config and data dirs while it was held, and find it
    /// deleted afterwards (see `test_sandbox::in_a_process_of_its_own`). A directory passed
    /// in is never anyone's ambient root. The production resolution is pinned
    /// once, by `component_logs_land_where_the_diagnostics_bundle_reads`.
    fn scratch_logs() -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let logs = temp.path().join("state").join("logs");
        (temp, logs)
    }

    /// `prepare_log_directory` puts a component's logs where the diagnostics
    /// bundle sweeps them (`logs/cli/`, `logs/server/`). Paths only, under the
    /// sandbox pin; the directory it creates is inside the sandbox.
    #[test]
    fn component_logs_land_where_the_diagnostics_bundle_reads() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        let log_dir = prepare_log_directory("cli", true).unwrap();
        let swept = crate::session::DiagnosticsSources::resolve()
            .logs_dir()
            .join("cli");
        assert_eq!(
            log_dir.parent(),
            Some(swept.as_path()),
            "the CLI logs into {} but the diagnostics bundle sweeps {}",
            log_dir.display(),
            swept.display()
        );
    }

    #[test]
    fn test_get_log_directory_basic_functionality() {
        let (_temp, logs) = scratch_logs();

        // Test basic directory creation without date subdirectory
        let result = prepare_log_directory_in(&logs, "cli", false);
        assert!(result.is_ok());

        let log_dir = result.unwrap();

        // Verify the directory was created and has correct structure
        assert!(log_dir.exists());
        assert!(log_dir.is_dir());

        let path_str = log_dir.to_string_lossy();
        assert!(path_str.contains("cli"));
        assert!(path_str.contains("logs"));

        // Verify we can write to the directory
        let test_file = log_dir.join("test.log");
        assert!(fs::write(&test_file, "test log content").is_ok());
        let _ = fs::remove_file(&test_file);
    }

    #[test]
    fn test_get_log_directory_with_date_subdir() {
        let (_temp, logs) = scratch_logs();

        // Test date-based subdirectory creation
        let result = prepare_log_directory_in(&logs, "server", true);
        assert!(result.is_ok());

        let log_dir = result.unwrap();

        // Verify the directory was created
        assert!(log_dir.exists());
        assert!(log_dir.is_dir());

        let path_str = log_dir.to_string_lossy();
        assert!(path_str.contains("server"));
        assert!(path_str.contains("logs"));

        // Verify date format (YYYY-MM-DD) is present
        let now = chrono::Local::now();
        let date_str = now.format("%Y-%m-%d").to_string();
        assert!(path_str.contains(&date_str));

        // Verify path structure: logs -> component -> date
        let logs_pos = path_str.find("logs").unwrap();
        let component_pos = path_str.find("server").unwrap();
        let date_pos = path_str.find(&date_str).unwrap();
        assert!(logs_pos < component_pos);
        assert!(component_pos < date_pos);
    }

    #[test]
    fn test_get_log_directory_idempotent() {
        let (_temp, logs) = scratch_logs();

        // Test that multiple calls return the same result and don't fail
        let component = "debug";

        let result1 = prepare_log_directory_in(&logs, component, false);
        assert!(result1.is_ok());
        let log_dir1 = result1.unwrap();

        let result2 = prepare_log_directory_in(&logs, component, false);
        assert!(result2.is_ok());
        let log_dir2 = result2.unwrap();

        // Both calls should return the same path and directory should exist
        assert_eq!(log_dir1, log_dir2);
        assert!(log_dir1.exists());
        assert!(log_dir2.exists());

        // Test same behavior with date subdirectories
        let result3 = prepare_log_directory_in(&logs, component, true);
        assert!(result3.is_ok());
        let log_dir3 = result3.unwrap();

        let result4 = prepare_log_directory_in(&logs, component, true);
        assert!(result4.is_ok());
        let log_dir4 = result4.unwrap();

        assert_eq!(log_dir3, log_dir4);
        assert!(log_dir3.exists());
    }

    #[test]
    fn test_get_log_directory_different_components() {
        let (_temp, logs) = scratch_logs();

        // Test that different components create different directories
        let components = ["cli", "server", "debug"];
        let mut created_dirs = Vec::new();

        for component in &components {
            let result = prepare_log_directory_in(&logs, component, false);
            assert!(result.is_ok(), "Failed for component: {}", component);

            let log_dir = result.unwrap();
            assert!(log_dir.exists());
            assert!(log_dir.to_string_lossy().contains(component));

            created_dirs.push(log_dir);
        }

        // Verify all directories are different
        for i in 0..created_dirs.len() {
            for j in i + 1..created_dirs.len() {
                assert_ne!(created_dirs[i], created_dirs[j]);
            }
        }
    }
}
