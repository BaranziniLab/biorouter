use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::Mutex;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::{
    filter::LevelFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
    Registry,
};

use biorouter_bench::bench_session::BenchAgentError;
use biorouter_bench::error_capture::ErrorCaptureLayer;

// Used to ensure we only set up tracing once
static INIT: Once = Once::new();

/// Sets up the logging infrastructure for the application.
/// This includes:
/// - File-based logging with JSON formatting (DEBUG level)
/// - No console output (all logs go to files only)
/// - Optional error capture layer for benchmarking
pub fn setup_logging(
    name: Option<&str>,
    error_capture: Option<Arc<Mutex<Vec<BenchAgentError>>>>,
) -> Result<()> {
    setup_logging_internal(name, error_capture, false)
}

/// Internal function that allows bypassing the Once check for testing
fn setup_logging_internal(
    name: Option<&str>,
    error_capture: Option<Arc<Mutex<Vec<BenchAgentError>>>>,
    force: bool,
) -> Result<()> {
    let mut result = Ok(());

    // Register the error vector if provided
    if let Some(errors) = error_capture {
        ErrorCaptureLayer::register_error_vector(errors);
    }

    let mut setup = || {
        result = (|| {
            let log_dir = biorouter::logging::prepare_log_directory("cli", true)?;
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
            let log_filename = if let Some(n) = name {
                format!("{}-{}.log", timestamp, n)
            } else {
                format!("{}.log", timestamp)
            };
            let file_appender = tracing_appender::rolling::RollingFileAppender::new(
                Rotation::NEVER, // we do manual rotation via file naming and cleanup_old_logs
                log_dir,
                log_filename,
            );

            // Create JSON file logging layer with all logs (DEBUG and above)
            let file_layer = fmt::layer()
                .with_target(true)
                .with_level(true)
                .with_writer(file_appender)
                .with_ansi(false)
                .json();

            // Base filter
            let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Set default levels for different modules
                EnvFilter::new("")
                    // Set mcp-client to DEBUG
                    .add_directive("mcp_client=debug".parse().unwrap())
                    // Set biorouter module to DEBUG
                    .add_directive("biorouter=debug".parse().unwrap())
                    // Set biorouter-cli to INFO
                    .add_directive("biorouter_cli=info".parse().unwrap())
                    // Set everything else to WARN
                    .add_directive(LevelFilter::WARN.into())
            });

            // Start building the subscriber
            let mut layers = vec![
                file_layer.with_filter(env_filter).boxed(),
                // Console logging disabled for CLI - all logs go to files only
            ];

            // Only add ErrorCaptureLayer if not in test mode
            if !force {
                layers.push(ErrorCaptureLayer::new().boxed());
            }

            // Build the subscriber
            let subscriber = Registry::default().with(layers);

            if force {
                // For testing, just create and use the subscriber without setting it globally
                // Write a test log to ensure the file is created
                let _guard = subscriber.set_default();
                tracing::warn!("Test log entry from setup");
                tracing::info!("Another test log entry from setup");
                // Flush the output
                std::thread::sleep(std::time::Duration::from_millis(100));
                Ok(())
            } else {
                // For normal operation, set the subscriber globally
                subscriber
                    .try_init()
                    .context("Failed to set global subscriber")?;
                Ok(())
            }
        })();
    };

    if force {
        setup();
    } else {
        INIT.call_once(setup);
    }

    result
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    /// Pin the home directory to a scratch tree **and clear
    /// `BIOROUTER_PATH_ROOT`**, under the workspace's process-wide environment
    /// lock, for the rest of the enclosing test.
    ///
    /// `prepare_log_directory` resolves `BIOROUTER_PATH_ROOT` on every call, and
    /// that variable is process-global. Since issue #56 this binary contains
    /// tests that point it at a `TempDir` of their own
    /// (`commands::knowledge::tests::privacy_tier`), so this test — which asserts
    /// the DEFAULT state directory, the one whose path carries a `biorouter`
    /// component — would intermittently resolve into their scratch root and fail
    /// on exactly that assertion. Clearing the override is what makes the
    /// assertion mean what it says; taking the same lock those tests take is what
    /// stops the two interleaving. Setting the home variable under the lock too
    /// closes the symmetric hazard, which this test previously wrote unguarded.
    ///
    /// ⚠ Deliberately NOT `scoped_state_dir!` from `biorouter`'s own logging
    /// tests, which pins `BIOROUTER_PATH_ROOT` **to** a temp root. That is right
    /// there — its assertions only look for `logs` and `cli` — and wrong here: a
    /// temp root has no `biorouter` component, so pinning would turn this test
    /// red rather than fix it.
    ///
    /// ⚠ Clearing the override moves it for every test in the process, so
    /// only a test in a process of its own may do it
    /// (`crate::test_sandbox::in_a_process_of_its_own`): held in the shared
    /// process, every sibling resolving `Paths` without the lock resolved this
    /// test's temporary `HOME` instead of the sandbox.
    ///
    /// A macro rather than a function so the `TempDir` and the guard live in
    /// the calling test's scope, dropped in reverse order at its end.
    macro_rules! scoped_default_home {
        ($temp:ident) => {
            let $temp = TempDir::new().unwrap();
            let _guard = crate::test_sandbox::relocate_home_off_the_path_root($temp.path());
        };
    }

    #[test]
    fn test_log_directory_creation() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        scoped_default_home!(_temp_dir);
        let log_dir = biorouter::logging::prepare_log_directory("cli", true).unwrap();
        assert!(log_dir.exists());
        assert!(log_dir.is_dir());

        // Verify directory structure
        let path_components: Vec<_> = log_dir.components().collect();
        assert!(path_components.iter().any(|c| c.as_os_str() == "biorouter"));
        assert!(path_components.iter().any(|c| c.as_os_str() == "logs"));
        assert!(path_components.iter().any(|c| c.as_os_str() == "cli"));
    }
}
