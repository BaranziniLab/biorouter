//! D-ALIAS: what to call a connection's server on screen.
//!
//! An invitation deliberately carries the host's `ssh -G`-resolved address, because the joiner
//! may have no alias for the server, so a joiner's saved login reads `crew_bob@52.33.141.141`
//! even when their own `~/.ssh/config` calls that machine `lab-server` (six critics). The label
//! prefers the person's own name for the server:
//!
//! 1. the alias they typed, when the login's host is one (`ssh -G` resolves it to another
//!    host name), which covers a host's own login and a joiner who overrode the server;
//! 2. else a literal `Host` entry of their SSH configuration (and the files it `Include`s) that
//!    `ssh -G` resolves to the same host name and port, in the order the file lists them;
//! 3. else the login's host as it is.
//!
//! **Display only.** The label is never an SSH target, never saved, never signed, never put in
//! an invitation, and never compared for identity: the saved login, the host-key check and the
//! pinned workspace key are unchanged, so an alias can never redirect a connection. It is
//! resolved with the same `ssh -G` and the same profile `-F` the bridge uses.

use super::authentication::{resolve_ssh, resolved_endpoint, ssh_endpoint};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// At most this many configuration files are read (the file and what it includes).
const MAX_CONFIG_FILES: usize = 16;
/// At most this many bytes of each file are read.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// At most this many `Host` entries are asked about per server.
const MAX_CANDIDATES: usize = 32;
/// How long a label is reused while the configuration is unchanged.
const CACHE_FOR: Duration = Duration::from_secs(60);
/// The longest alias shown.
const MAX_ALIAS: usize = 64;

/// The person's SSH configuration file, as the bridge reads it: the development profile's
/// `-F` file, else `~/.ssh/config`.
fn config_path() -> Option<PathBuf> {
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        return Some(PathBuf::from(profile).join("home/.ssh/config"));
    }
    dirs::home_dir().map(|home| home.join(".ssh/config"))
}

/// A name `ssh` may be handed and a screen may show: letters, digits, `.`, `_` and `-`, not
/// starting with `-`.
fn plain_alias(name: &str) -> bool {
    (1..=MAX_ALIAS).contains(&name.len())
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// One configuration line as `keyword` and its arguments: `Host a b`, `Host=a`, comments and
/// quotes as OpenSSH reads them (quoted arguments are kept whole; a pattern in quotes is still
/// a pattern).
fn directive(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (keyword, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
        Some(at) => line.split_at(at),
        None => (line, ""),
    };
    let rest = rest.trim_start().strip_prefix('=').unwrap_or(rest).trim();
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted && current.is_empty() => break,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    arguments.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    Some((keyword.to_ascii_lowercase(), arguments))
}

/// The literal host names `Host` lines declare, in file order: never a pattern (`*`, `?`), a
/// negation (`!`) or anything [`plain_alias`] refuses. `Match` blocks name no aliases.
/// `Include` arguments are returned separately, to be read in their place.
pub(super) fn host_entries(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let Some((keyword, arguments)) = directive(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => entries.extend(
                arguments
                    .into_iter()
                    .filter(|name| !name.contains(['*', '?', '!']) && plain_alias(name))
                    .map(Entry::Host),
            ),
            "include" => entries.extend(arguments.into_iter().map(Entry::Include)),
            _ => {}
        }
    }
    entries
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Host(String),
    Include(String),
}

/// The files an `Include` argument names, as OpenSSH resolves it: `~/` from the home
/// directory, a relative path from `~/.ssh`, and a `*` or `?` in the file name matched against
/// that directory's entries (sorted, as `glob` returns them).
fn included(argument: &str, ssh_dir: &Path) -> Vec<PathBuf> {
    let path = if let Some(rest) = argument.strip_prefix("~/") {
        match ssh_dir.parent() {
            Some(home) => home.join(rest),
            None => return Vec::new(),
        }
    } else if Path::new(argument).is_absolute() {
        PathBuf::from(argument)
    } else {
        ssh_dir.join(argument)
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(directory) = path.parent() else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut matches: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|file| wildcard(name.as_bytes(), file.as_bytes()))
        })
        .map(|entry| entry.path())
        .collect();
    matches.sort();
    matches
}

/// `*` and `?` matching, as a file-name glob.
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            wildcard(&pattern[1..], text) || (!text.is_empty() && wildcard(pattern, &text[1..]))
        }
        (Some(b'?'), Some(_)) => wildcard(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => wildcard(&pattern[1..], &text[1..]),
        _ => false,
    }
}

/// Every literal `Host` name in the configuration, in the order OpenSSH reads the files (an
/// `Include` is read where it stands), and a fingerprint of what was read: each file's path,
/// length and modification time.
fn candidates() -> (Vec<String>, Fingerprint) {
    let mut names = Vec::new();
    let mut read = Vec::new();
    if let Some(config) = config_path() {
        let ssh_dir = config.parent().map(Path::to_path_buf).unwrap_or_default();
        collect(&config, &ssh_dir, 0, &mut names, &mut read);
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|name| seen.insert(name.to_ascii_lowercase()));
    (names, read)
}

/// [`candidates`]' reading of one file, and of the files it includes, depth-first.
fn collect(
    path: &Path,
    ssh_dir: &Path,
    depth: usize,
    names: &mut Vec<String>,
    read: &mut Fingerprint,
) {
    // OpenSSH stops at 16 levels of Include; a loop or a long chain ends here too.
    if depth > 16 || read.len() >= MAX_CONFIG_FILES || read.iter().any(|(seen, ..)| seen == path) {
        return;
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    read.push((path.to_path_buf(), metadata.len(), metadata.modified().ok()));
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    let mut text = String::new();
    use std::io::Read;
    if file
        .take(MAX_CONFIG_BYTES)
        .read_to_string(&mut text)
        .is_err()
    {
        return;
    }
    for entry in host_entries(&text) {
        match entry {
            Entry::Host(name) => names.push(name),
            Entry::Include(argument) => {
                for file in included(&argument, ssh_dir) {
                    collect(&file, ssh_dir, depth + 1, names, read);
                }
            }
        }
    }
}

type Fingerprint = Vec<(PathBuf, u64, Option<SystemTime>)>;

/// A label, what it was resolved from, and when, per saved login and port.
type LabelCache = HashMap<(String, Option<u16>), (String, Fingerprint, Instant)>;

static CACHE: LazyLock<Mutex<LabelCache>> = LazyLock::new(Default::default);

/// `ssh -G <name>`'s host name and port, under the profile's `-F` like the bridge.
async fn endpoint_of(name: &str) -> Option<(String, u16)> {
    let mut args = Vec::new();
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        args.extend([
            "-F".to_owned(),
            PathBuf::from(profile)
                .join("home/.ssh/config")
                .to_string_lossy()
                .into_owned(),
        ]);
    }
    args.push(name.to_owned());
    let settings = resolve_ssh(&args).await.ok()?;
    resolved_endpoint(&settings)
        .ok()
        .map(|(host, port)| (host.to_ascii_lowercase(), port))
}

/// What to call the server a saved login (`user@host` or an alias) reaches. See the module
/// documentation; display only.
pub async fn server_label(ssh_target: &str, port: Option<u16>) -> String {
    let host = ssh_target
        .rsplit_once('@')
        .map_or(ssh_target, |(_, host)| host)
        .to_owned();
    let (names, fingerprint) = candidates();
    let key = (ssh_target.to_owned(), port);
    if let Some((label, seen, at)) = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        if *seen == fingerprint && at.elapsed() < CACHE_FOR {
            return label.clone();
        }
    }
    let label = tokio::time::timeout(
        Duration::from_secs(5),
        resolve_label(&host, ssh_target, port, &names),
    )
    .await
    .unwrap_or_else(|_| host.clone());
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (label.clone(), fingerprint, Instant::now()));
    label
}

async fn resolve_label(
    host: &str,
    ssh_target: &str,
    port: Option<u16>,
    names: &[String],
) -> String {
    let Some(endpoint) = ssh_endpoint(ssh_target, port).await else {
        return host.to_owned();
    };
    // The login names an alias: the person's own name for the server already.
    if !host.eq_ignore_ascii_case(&endpoint.0) {
        return host.to_owned();
    }
    for name in names
        .iter()
        .filter(|name| !name.eq_ignore_ascii_case(host))
        .take(MAX_CANDIDATES)
    {
        if endpoint_of(name).await.as_ref() == Some(&endpoint) {
            return name.clone();
        }
    }
    host.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_lines_give_literal_names_only_in_file_order() {
        let text = "\
# a comment
Host lab-server gpu.lab
  HostName 52.33.141.141
Host *.internal !bastion web?
Host=\"quoted-host\" -dash
Match host 52.33.141.141
  User crew
Include config.d/*.conf ~/.ssh/extra
host second # trailing comment
";
        assert_eq!(
            host_entries(text),
            vec![
                Entry::Host("lab-server".into()),
                Entry::Host("gpu.lab".into()),
                Entry::Host("quoted-host".into()),
                Entry::Include("config.d/*.conf".into()),
                Entry::Include("~/.ssh/extra".into()),
                Entry::Host("second".into()),
            ]
        );
    }

    #[test]
    fn an_include_resolves_like_openssh() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(ssh.join("config.d")).unwrap();
        for name in ["b.conf", "a.conf", "notes.txt"] {
            std::fs::write(ssh.join("config.d").join(name), "").unwrap();
        }
        assert_eq!(
            included("config.d/*.conf", &ssh),
            vec![ssh.join("config.d/a.conf"), ssh.join("config.d/b.conf")]
        );
        assert_eq!(
            included("~/elsewhere", &ssh),
            vec![home.path().join("elsewhere")]
        );
        assert_eq!(
            included("/etc/ssh/extra", &ssh),
            vec![PathBuf::from("/etc/ssh/extra")]
        );
        assert!(wildcard(b"*.conf", b"a.conf"));
        assert!(!wildcard(b"*.conf", b"a.txt"));
        assert!(wildcard(b"h?st", b"host"));
    }

    #[test]
    fn only_a_plain_name_is_ever_an_alias() {
        for good in ["lab-server", "gpu.lab", "hpc_2"] {
            assert!(plain_alias(good), "{good}");
        }
        for bad in [
            "-oProxyCommand=x",
            ".hidden",
            "a b",
            "lab/server",
            "",
            "x;y",
        ] {
            assert!(!plain_alias(bad), "{bad}");
        }
        assert!(!plain_alias(&"a".repeat(MAX_ALIAS + 1)));
    }

    /// A fake `ssh -G` that resolves each name to `hostname`/`port` from a table, and a
    /// configuration naming them, in the development profile the bridge reads.
    #[cfg(unix)]
    fn fake_ssh(
        root: &Path,
        table: &[(&str, &str, u16)],
        config: &str,
    ) -> env_lock::EnvGuard<'static> {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mut cases = String::new();
        for (name, host, port) in table {
            cases.push_str(&format!(
                "    {name}) printf 'hostname {host}\\nport {port}\\n' ;;\n"
            ));
        }
        std::fs::write(
            bin.join("ssh"),
            format!(
                "#!/bin/sh\n[ \"$1\" = \"-G\" ] || exit 1\nport=22\nprev=\nfor last; do [ \"$prev\" = \"-p\" ] && port=$last; prev=$last; done\nhost=${{last##*@}}\ncase \"$host\" in\n{cases}    *) printf 'hostname %s\\nport %s\\n' \"$host\" \"$port\" ;;\nesac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(bin.join("ssh"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let profile = root.join("profile");
        std::fs::create_dir_all(profile.join("home/.ssh")).unwrap();
        std::fs::write(profile.join("home/.ssh/config"), config).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let profile = profile.to_string_lossy().into_owned();
        crate::test_sandbox::relocate_path_root_and(
            profile.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
                ("PATH", Some(path.as_str())),
            ],
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_persons_own_alias_names_the_server() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _env = fake_ssh(
            root.path(),
            &[
                ("lab-server", "52.33.141.141", 22),
                ("lab-alt", "52.33.141.141", 2222),
                ("other", "10.0.0.9", 22),
            ],
            "Host other\n  HostName 10.0.0.9\nHost lab-alt\n  Port 2222\nHost lab-server\n  HostName 52.33.141.141\n",
        );
        // A joiner's login carries the address; their own Host entry names it. An entry for
        // the same address on another port is not the same server.
        assert_eq!(
            server_label("crew_bob@52.33.141.141", None).await,
            "lab-server"
        );
        // Typed as an alias: that alias, as typed.
        assert_eq!(
            server_label("crew_alice@lab-server", None).await,
            "lab-server"
        );
        // No alias maps to it: the host as it is.
        assert_eq!(server_label("crew_bob@192.0.2.7", None).await, "192.0.2.7");
        // The port the login uses decides which entry is the same server.
        assert_eq!(
            server_label("crew_bob@52.33.141.141", Some(2222)).await,
            "lab-alt"
        );
    }
}
