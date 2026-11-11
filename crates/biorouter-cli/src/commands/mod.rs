pub mod apps;
pub mod bench;
pub mod configure;
pub mod crew;
pub(crate) mod crew_step;
pub mod decide;
pub mod decision_pull;
pub mod doctor;
pub mod exe_path;
pub mod extension;
pub mod info;
pub mod knowledge;
pub mod models;
pub mod needs_terminal;
pub mod project;
pub mod schedule;
pub mod serve;
pub mod session;
pub mod session_decisions;
pub mod session_gates;
pub mod session_grouping;
pub mod session_monitors;
pub mod session_subagents;
pub mod session_watch;
pub mod shared_conversation;
pub mod skill;
pub mod supervision_stop;
pub mod term;
pub mod usage;
pub mod web;
pub mod workflow;
pub mod workspace_parity;

/// `--format text|json`, beside `--json`: every session read-out command takes
/// both spellings, so a script never needs to know which one a command chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum TextOrJson {
    #[default]
    Text,
    Json,
}

/// Print one line of a command's output. A reader that went away (the output
/// piped into `head`, say) ends the command quietly, as other Unix tools do,
/// where `println!` panics with "failed printing to stdout: Broken pipe".
/// Only the read-out commands use it; nothing is left half done when they stop.
pub fn print_line(line: impl std::fmt::Display) {
    if let Err(error) = write_line(&mut std::io::stdout().lock(), &line) {
        if error.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        eprintln!("could not write the output: {error}");
        std::process::exit(1);
    }
}

/// [`print_line`] for a command that has work left after its output (a
/// `decide` run waits for late answers to store them): a reader that went
/// away stops the output, never the command, and never panics.
pub fn print_line_keep_going(line: impl std::fmt::Display) {
    let _ = write_line(&mut std::io::stdout().lock(), &line);
}

/// [`print_line`]'s write, apart so a test can hand it a closed pipe.
pub fn write_line(
    out: &mut impl std::io::Write,
    line: &impl std::fmt::Display,
) -> std::io::Result<()> {
    writeln!(out, "{line}")?;
    out.flush()
}

#[cfg(test)]
mod print_line_tests {
    use super::write_line;

    struct Closed;
    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A closed reader is an error the caller can act on, never a panic.
    #[test]
    fn a_closed_reader_is_an_error_not_a_panic() {
        let error = write_line(&mut Closed, &"a line").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        let mut buffer = Vec::new();
        write_line(&mut buffer, &"a line").unwrap();
        assert_eq!(buffer, b"a line\n");
    }
}
