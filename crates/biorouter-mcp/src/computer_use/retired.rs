//! The retired engine's leftover on macOS (AD-19 in docs/design/copilot-agent-desktop.md).
//!
//! Before 1.94.6 Biorouter Copilot ran on the retired Open Computer Use engine, whose macOS
//! helper started a background app agent through LaunchServices that outlived the daemon. An update replaces the
//! app's files but not that running process, so after an update the daemon ends it once at start.
//! The process is matched by its exact executable path inside THIS installation, never by name, so
//! another installation's processes (a second copy of Biorouter, a development build) are never
//! touched. Agent-desktop starts no agent of its own, so there is nothing else to end.
use std::path::{Path, PathBuf};

/// The retired engine's bundle and executable, relative to an app's `Contents/Resources`.
const RETIRED_AGENT: &str = "computer-use/BioRouter Computer Use.app/Contents/MacOS/ocu";

/// Where the retired engine's agent ran from for the installation that holds `executable`: the
/// outermost `.app` the daemon runs inside. `None` outside an app bundle (a CLI, a development
/// build), where no such agent was ever installed.
pub fn retired_agent_path(executable: &Path) -> Option<PathBuf> {
    let app = executable
        .ancestors()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .last()?;
    Some(app.join("Contents/Resources").join(RETIRED_AGENT))
}

/// End the retired engine's agent left running by an earlier version of this installation.
/// Returns how many processes were asked to end. Best effort: a process that cannot be ended
/// (another user's) is left alone, and nothing here can fail the daemon's start.
pub fn end_retired_engine_agent() -> usize {
    #[cfg(target_os = "macos")]
    {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};
        let Some(path) = std::env::current_exe()
            .ok()
            .and_then(|exe| dunce::canonicalize(exe).ok())
            .and_then(|exe| retired_agent_path(&exe))
        else {
            return 0;
        };
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
        );
        let mut ended = 0;
        for process in system.processes().values() {
            if process.exe() == Some(path.as_path())
                && process.kill_with(Signal::Term).unwrap_or(false)
            {
                tracing::info!(
                    pid = process.pid().as_u32(),
                    "ended the retired Open Computer Use agent left running by an earlier version"
                );
                ended += 1;
            }
        }
        ended
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_retired_agent_is_found_only_inside_this_installation() {
        assert_eq!(
            retired_agent_path(Path::new(
                "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd"
            )),
            Some(PathBuf::from(
                "/Applications/Biorouter.app/Contents/Resources/computer-use/BioRouter Computer Use.app/Contents/MacOS/ocu"
            ))
        );
        // The daemon may also run from inside the engine's own bundle path layout: the outermost
        // app is still the installation.
        assert_eq!(
            retired_agent_path(Path::new(
                "/Users/x/Applications/Biorouter 2.app/Contents/Resources/computer-use/Biorouter Copilot.app/Contents/MacOS/agent-desktop"
            )),
            Some(PathBuf::from(
                "/Users/x/Applications/Biorouter 2.app/Contents/Resources/computer-use/BioRouter Computer Use.app/Contents/MacOS/ocu"
            ))
        );
        for outside in [
            "/usr/local/bin/biorouterd",
            "/Users/x/src/biorouter/target/debug/biorouterd",
            "/usr/libexec/biorouter/biorouterd",
        ] {
            assert_eq!(retired_agent_path(Path::new(outside)), None, "{outside}");
        }
    }

    #[test]
    fn nothing_outside_macos_is_ended() {
        if !cfg!(target_os = "macos") {
            assert_eq!(end_retired_engine_agent(), 0);
        }
    }
}
