//! Which account a running `biorouter serve` belongs to (issue #526, gap 4).
//!
//! `biorouter configure` saves settings and keys for the account that runs it.
//! A `biorouter serve` installed as a service usually runs as a service account,
//! and a person who opens a shell on the host as their own login user (an SSM
//! or SSH session, `ssm-user`) and runs `configure` there saves the key where
//! the service never looks. Nothing said so.
//!
//! This reads only what every account on the host may read about another's
//! processes: the owner and the command line. It never reads a process's
//! environment or any file of the other account, so no secret is touched. A
//! host that hides other accounts' processes (`hidepid` on Linux) shows none,
//! and then nothing is said.

use std::path::{Path, PathBuf};

/// A `biorouter serve` running as an account other than this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeElsewhere {
    pub pid: u32,
    pub uid: u32,
    /// The account's name and home folder, when the host's user database has them.
    pub account: Option<(String, PathBuf)>,
}

/// Whether a command line is a running `biorouter serve` (or its `headless`
/// alias): the program is `biorouter`, its first word that is not an option is
/// `serve` or `headless`, and no later word asks for something other than
/// serving (`serve url`, `--help`).
pub(crate) fn is_serve_command<S: AsRef<str>>(args: &[S]) -> bool {
    let Some(program) = args.first() else {
        return false;
    };
    let program = Path::new(program.as_ref())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if program != "biorouter" {
        return false;
    }
    let mut rest = args[1..].iter().map(AsRef::as_ref);
    if !matches!(
        rest.find(|arg| !arg.starts_with('-')),
        Some("serve" | "headless")
    ) {
        return false;
    }
    !rest.any(|arg| matches!(arg, "url" | "help" | "--help" | "-h" | "--version" | "-V"))
}

/// Every `biorouter serve` this account can see that runs as another account.
pub fn serving_elsewhere() -> Vec<ServeElsewhere> {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        let found = if cfg!(target_os = "linux") {
            scan_proc(Path::new("/proc"), me)
        } else {
            scan_ps(me)
        };
        found
            .into_iter()
            .map(|(pid, uid)| ServeElsewhere {
                pid,
                uid,
                account: account_of(uid),
            })
            .collect()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// `(pid, uid)` of each serve process under a Linux `/proc` (or a folder laid
/// out like one) whose owner is not `me`. A process that ends while it is read
/// is skipped.
#[cfg(unix)]
pub(crate) fn scan_proc(root: &Path, me: u32) -> Vec<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(owner) = entry.metadata().map(|m| m.uid()) else {
            continue;
        };
        if owner == me {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let args: Vec<String> = cmdline
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect();
        if is_serve_command(&args) {
            found.push((pid, owner));
        }
    }
    found.sort_unstable();
    found
}

/// The same answer from `ps`, for a host with no `/proc` (macOS).
#[cfg(unix)]
fn scan_ps(me: u32) -> Vec<(u32, u32)> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axww", "-o", "uid=,pid=,args="])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    parse_ps(&String::from_utf8_lossy(&output.stdout), me)
}

/// `(pid, uid)` of each serve process in `ps -o uid=,pid=,args=` output whose
/// owner is not `me`.
pub(crate) fn parse_ps(text: &str, me: u32) -> Vec<(u32, u32)> {
    let mut found = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(uid), Some(pid)) = (
            words.next().and_then(|w| w.parse::<u32>().ok()),
            words.next().and_then(|w| w.parse::<u32>().ok()),
        ) else {
            continue;
        };
        if uid == me {
            continue;
        }
        let args: Vec<&str> = words.collect();
        if is_serve_command(&args) {
            found.push((pid, uid));
        }
    }
    found.sort_unstable();
    found
}

/// The name and home folder of `uid` from the host's user database.
#[cfg(unix)]
fn account_of(uid: u32) -> Option<(String, PathBuf)> {
    use std::ffi::CStr;
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: an all-zero passwd is a valid value for getpwuid_r to fill in.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call, and the buffer's length is
    // the one passed.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || entry.pw_name.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so both fields point at NUL-terminated
    // strings inside `buffer`, which outlives these reads.
    let (name, home) = unsafe {
        (
            CStr::from_ptr(entry.pw_name).to_string_lossy().into_owned(),
            CStr::from_ptr(entry.pw_dir).to_string_lossy().into_owned(),
        )
    };
    Some((name, PathBuf::from(home)))
}

fn who(serve: &ServeElsewhere) -> String {
    match &serve.account {
        Some((name, _)) => format!("{name} (process {})", serve.pid),
        None => format!("user id {} (process {})", serve.uid, serve.pid),
    }
}

fn where_it_reads(serve: &ServeElsewhere) -> String {
    match &serve.account {
        Some((name, home)) => format!(
            "{name}'s settings, by default {}",
            home.join(".config").join("biorouter").display()
        ),
        None => format!("the settings of user id {}", serve.uid),
    }
}

/// Said before anything is entered: which `serve` will not see what this
/// command saves, where it reads its settings, where they go instead, and how
/// to save them for it.
pub fn warning_before_saving(
    elsewhere: &[ServeElsewhere],
    saved_in: &Path,
    command: &str,
) -> Option<String> {
    let first = elsewhere.first()?;
    let servers = elsewhere.iter().map(who).collect::<Vec<_>>().join(", ");
    let run_as = match &first.account {
        Some((name, _)) => format!("`sudo -H -u {name} {command}`"),
        None => format!("`{command}` as that account"),
    };
    Some(format!(
        "Warning: `biorouter serve` is running as {servers}, not as this account. It reads \
         {}, and a key saved here goes to {} instead, where that server never looks. To set the \
         key it uses, run {run_as}, with the environment the service starts with (for example \
         the same BIOROUTER_DISABLE_KEYRING).",
        where_it_reads(first),
        saved_in.display(),
    ))
}

/// Said after a key was saved, beside where it went.
pub fn note_after_saving(elsewhere: &[ServeElsewhere]) -> Option<String> {
    let first = elsewhere.first()?;
    Some(format!(
        "The `biorouter serve` running as {} does not read this key: it reads {}.",
        who(first),
        where_it_reads(first),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_serve_command_line_is_recognised_and_nothing_else_is() {
        for args in [
            vec!["biorouter", "serve"],
            vec![
                "/usr/bin/biorouter",
                "serve",
                "--host",
                "0.0.0.0",
                "--port",
                "8765",
            ],
            vec!["/opt/br/biorouter", "headless"],
            vec!["biorouter", "--debug", "serve"],
        ] {
            assert!(is_serve_command(&args), "{args:?}");
        }
        for args in [
            vec![],
            vec!["biorouter"],
            vec!["biorouter", "configure"],
            vec!["biorouter", "apps", "serve", "my-app"],
            vec!["biorouter", "serve", "url"],
            vec!["biorouter", "serve", "--port", "8765", "url"],
            vec!["biorouter", "serve", "--help"],
            vec!["biorouterd", "agent"],
            vec!["/usr/bin/python3", "biorouter", "serve"],
            vec!["vim", "serve"],
        ] {
            assert!(!is_serve_command(&args), "{args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_proc_scan_reports_serve_processes_of_other_owners_only() {
        let root = tempfile::tempdir().unwrap();
        let process = |pid: &str, args: &[&str]| {
            let dir = root.path().join(pid);
            std::fs::create_dir(&dir).unwrap();
            let mut cmdline = args.join("\0").into_bytes();
            cmdline.push(0);
            std::fs::write(dir.join("cmdline"), cmdline).unwrap();
        };
        process("812", &["/usr/bin/biorouter", "serve", "--port", "8765"]);
        process(
            "90",
            &["/usr/bin/biorouterd", "agent", "--exit-with-parent", "812"],
        );
        process("17", &["bash"]);
        process("self", &["/usr/bin/biorouter", "serve"]);
        std::fs::create_dir(root.path().join("23")).unwrap();

        // SAFETY: geteuid has no preconditions and cannot fail.
        let owner = unsafe { libc::geteuid() };
        assert_eq!(scan_proc(root.path(), owner + 1), vec![(812, owner)]);
        assert_eq!(scan_proc(root.path(), owner), Vec::<(u32, u32)>::new());
        assert!(scan_proc(&root.path().join("missing"), owner + 1).is_empty());
    }

    #[test]
    fn ps_output_is_read_the_same_way() {
        let text = "  998   812 /usr/bin/biorouter serve --host 0.0.0.0 --port 8765\n\
                    \x20 998   813 /usr/bin/biorouterd agent --exit-with-parent 812\n\
                    \x20 501  4410 biorouter serve\n\
                    \x20 501  4411 -zsh\n\
                    garbage line\n";
        assert_eq!(parse_ps(text, 501), vec![(812, 998)]);
        assert_eq!(parse_ps(text, 998), vec![(4410, 501)]);
    }

    #[cfg(unix)]
    #[test]
    fn this_account_is_found_in_the_user_database() {
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        let (name, home) = account_of(me).expect("the running account has an entry");
        assert!(!name.is_empty());
        assert!(home.is_absolute(), "{}", home.display());
    }

    #[test]
    fn the_warning_names_the_account_where_it_reads_and_where_the_key_went() {
        let elsewhere = [ServeElsewhere {
            pid: 812,
            uid: 998,
            account: Some(("biorouter".to_string(), PathBuf::from("/var/lib/biorouter"))),
        }];
        let saved_in = Path::new("/home/ssm-user/.config/biorouter");
        let command = "biorouter configure --provider openai --secret-stdin";
        let warning = warning_before_saving(&elsewhere, saved_in, command).unwrap();
        assert!(
            warning.contains("running as biorouter (process 812)"),
            "{warning}"
        );
        assert!(
            warning.contains("/var/lib/biorouter/.config/biorouter"),
            "{warning}"
        );
        assert!(
            warning.contains("goes to /home/ssm-user/.config/biorouter instead"),
            "{warning}"
        );
        assert!(
            warning.contains(&format!("`sudo -H -u biorouter {command}`")),
            "{warning}"
        );
        let note = note_after_saving(&elsewhere).unwrap();
        assert!(note.contains("does not read this key"), "{note}");
        assert!(
            note.contains("/var/lib/biorouter/.config/biorouter"),
            "{note}"
        );

        let unnamed = [ServeElsewhere {
            pid: 7,
            uid: 4242,
            account: None,
        }];
        let warning = warning_before_saving(&unnamed, saved_in, "biorouter configure").unwrap();
        assert!(warning.contains("user id 4242 (process 7)"), "{warning}");
        assert!(
            warning.contains("`biorouter configure` as that account"),
            "{warning}"
        );

        assert_eq!(warning_before_saving(&[], saved_in, command), None);
        assert_eq!(note_after_saving(&[]), None);
    }
}
