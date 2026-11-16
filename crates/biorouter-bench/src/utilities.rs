use anyhow::Result;
use std::env;
use std::process::{Child, Command};
use std::thread::JoinHandle;
use tracing;

/// Waits for every child process and every worker thread, then fails when any
/// of them did: a child that exited non-zero or could not be waited for, a
/// thread whose run returned an error, or one that panicked.
///
/// Everything is waited for before the answer, so a failure never leaves a
/// child running behind the run. A failure used to be logged and forgotten,
/// so `bench run` exited 0 with no results and a script read it as a success.
pub fn await_process_exits(
    child_processes: &mut [Child],
    handles: Vec<JoinHandle<Result<()>>>,
) -> Result<()> {
    let mut failures: Vec<String> = Vec::new();
    for child in child_processes.iter_mut() {
        match child.wait() {
            Ok(status) if status.success() => {
                tracing::info!("Child exited with status: {}", status)
            }
            Ok(status) => {
                tracing::error!("Child exited with status: {}", status);
                failures.push(format!("a bench process exited with {status}"));
            }
            Err(e) => {
                tracing::error!("Error waiting for child: {}", e);
                failures.push(format!("a bench process could not be waited for: {e}"));
            }
        }
    }

    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => (),
            Ok(Err(e)) => {
                tracing::error!("Bench run failed: {:#}", e);
                failures.push(format!("{e:#}"));
            }
            Err(e) => {
                tracing::error!("Thread panicked: {:?}", e);
                failures.push("a bench run panicked".to_string());
            }
        }
    }

    match failures.as_slice() {
        [] => Ok(()),
        [one] => anyhow::bail!("{one}"),
        many => anyhow::bail!("{} parts failed: {}", many.len(), many.join("; ")),
    }
}

/// The first of several outcomes that failed, after all of them ran. A step of
/// a bench run keeps going after one of its parts fails, so the other parts'
/// results are still written, and fails at the end.
pub fn first_failure(outcomes: Vec<Result<()>>) -> Result<()> {
    let mut failures = outcomes.into_iter().filter_map(Result::err);
    match failures.next() {
        None => Ok(()),
        Some(first) => match failures.count() {
            0 => Err(first),
            rest => anyhow::bail!("{first:#} ({rest} more part(s) of the bench run failed too)"),
        },
    }
}

pub fn parallel_bench_cmd(bench_cmd: String, config: String, envs: Vec<(String, String)>) -> Child {
    let current_exe = env::current_exe().expect("Failed to get current executable path");

    let mut cmd = Command::new(current_exe);
    cmd.arg("bench").arg(bench_cmd).arg("--config").arg(config);

    for (key, value) in envs.into_iter() {
        cmd.env(key, value);
    }

    cmd.spawn().expect("Failed to spawn child process")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W1-06/W1-10: a bench child that crashed (the eval's session panicked) used
    /// to be logged and forgotten, so `bench run` exited 0 with no results and a
    /// script or CI read the run as a success.
    #[test]
    fn a_failed_child_fails_the_run() {
        let failed = Command::new("false").spawn().expect("spawn `false`");
        let ok = Command::new("true").spawn().expect("spawn `true`");
        let mut children = [ok, failed];
        let outcome = await_process_exits(&mut children, Vec::new());
        assert!(
            outcome.is_err(),
            "a child that exited non-zero must fail the run"
        );
        // Every child is waited for, the failed one included, before the run fails.
        for child in children.iter_mut() {
            assert!(
                child.try_wait().expect("try_wait").is_some(),
                "every child is reaped"
            );
        }
    }

    /// A worker thread whose run returned an error, or panicked, fails the run too.
    #[test]
    fn a_failed_or_panicked_thread_fails_the_run() {
        let failed = std::thread::spawn(|| -> Result<()> { anyhow::bail!("the eval failed") });
        assert!(await_process_exits(&mut [], vec![failed]).is_err());

        let panicked = std::thread::spawn(|| -> Result<()> { panic!("the eval panicked") });
        assert!(await_process_exits(&mut [], vec![panicked]).is_err());
    }

    /// Children and threads that all succeed leave the run successful.
    #[test]
    fn a_run_whose_children_all_succeed_is_ok() {
        let mut children = [Command::new("true").spawn().expect("spawn `true`")];
        let fine = std::thread::spawn(|| -> Result<()> { Ok(()) });
        assert!(await_process_exits(&mut children, vec![fine]).is_ok());
    }

    /// A step that keeps going past a failed part still fails at the end, with
    /// the first failure named and the others counted.
    #[test]
    fn the_first_failure_is_kept_after_every_part_ran() {
        assert!(first_failure(vec![Ok(()), Ok(())]).is_ok());
        let one = first_failure(vec![Ok(()), Err(anyhow::anyhow!("model a failed"))]);
        assert_eq!(one.unwrap_err().to_string(), "model a failed");
        let two = first_failure(vec![
            Err(anyhow::anyhow!("model a failed")),
            Ok(()),
            Err(anyhow::anyhow!("model b failed")),
        ])
        .unwrap_err()
        .to_string();
        assert_eq!(
            two,
            "model a failed (1 more part(s) of the bench run failed too)"
        );
    }
}
