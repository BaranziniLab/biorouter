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
//!
//! **The saved login is a separate question** ([`own_alias_for_login`], G5). OpenSSH applies a
//! `Host` block only when the name on the command line matches it, so a joiner whose identity
//! file, port or jump host for the server sit under an alias would not get them from
//! `crew_bob@52.33.141.141`. When the person chose no login, port or jump host, an invitation
//! is saved under the first literal `Host` entry that `ssh -G` resolves to the same user, the
//! same host name and the same port as that login: the same account on the same server, with
//! the person's own settings for it. Matching on all three, never on the label, is what keeps
//! it from signing in as anyone else. An entry Crew's own SSH command could not use is passed
//! over ([`crew_can_use`]): one its connection preflight refuses (a custom `ProxyCommand`,
//! delegated GSSAPI credentials and the rest of `ssh_policy`), or one that replaces or starves
//! the bridge command (`RemoteCommand`, `SessionType`, `StdinNull`), where `user@host` works.

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

/// `ssh -G [-p port] <name>`'s settings, under the profile's `-F` like the bridge.
async fn settings_of(name: &str, port: Option<u16>) -> Option<HashMap<String, String>> {
    if !super::safe_atom(name) {
        return None;
    }
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
    if let Some(port) = port {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    args.push(name.to_owned());
    resolve_ssh(&args).await.ok()
}

/// `ssh -G <name>`'s host name and port, under the profile's `-F` like the bridge.
async fn endpoint_of(name: &str) -> Option<(String, u16)> {
    resolved_endpoint(&settings_of(name, None).await?)
        .ok()
        .map(|(host, port)| (host.to_ascii_lowercase(), port))
}

/// The account, lowercase host name and port `ssh -G` settings sign in to.
fn login_of(settings: &HashMap<String, String>) -> Option<(String, String, u16)> {
    let user = settings
        .get("user")
        .filter(|user| biorouter_crew::valid_username(user))?;
    let (host, port) = resolved_endpoint(settings).ok()?;
    Some((user.clone(), host.to_ascii_lowercase(), port))
}

/// Whether `ssh -G` settings route the connection themselves, through a jump host or a proxy
/// command.
fn sets_route(settings: &HashMap<String, String>) -> bool {
    ["proxyjump", "proxycommand"].iter().any(|key| {
        settings
            .get(*key)
            .is_some_and(|value| !value.eq_ignore_ascii_case("none"))
    })
}

/// Whether Crew's own SSH command can use the entry `name`, whose `ssh -G` settings are
/// `settings`: no remote command, session type or closed stdin of its own in place of the
/// bridge command, and the connection preflight every connect runs ([`super::ssh_policy`])
/// admits it under the options Crew's command line sets.
async fn crew_can_use(name: &str, settings: &HashMap<String, String>) -> bool {
    let replaces_command = settings
        .get("remotecommand")
        .is_some_and(|command| !command.eq_ignore_ascii_case("none"))
        || settings
            .get("sessiontype")
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("default"))
        || settings
            .get("stdinnull")
            .is_some_and(|closed| matches!(closed.as_str(), "yes" | "true"));
    if replaces_command {
        return false;
    }
    let mut args = super::transport::login_args(None, None, None, None);
    args.extend(["-o".to_owned(), "BatchMode=yes".to_owned(), name.to_owned()]);
    super::ssh_policy::preflight(&args, name).await.is_ok()
}

/// A `Host` entry of the person's SSH configuration that signs in to the same login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnAlias {
    pub(super) name: String,
    /// The entry sets its own jump host or proxy command, which then takes the place of the
    /// invitation's jump host hint.
    pub(super) sets_route: bool,
}

/// G5: the person's own name for the login an invitation would save (`user@host` on `port`):
/// the first literal `Host` entry of their SSH configuration, in file order, that `ssh -G`
/// resolves to the same user, the same host name and the same port, and that Crew's own SSH
/// command can use ([`crew_can_use`]). See the module documentation. `None` when the login's
/// host is itself one of their entries (their settings for it already apply), when no entry
/// matches, or when `ssh` can't say.
pub(super) async fn own_alias_for_login(ssh_target: &str, port: Option<u16>) -> Option<OwnAlias> {
    let (_, host) = ssh_target.rsplit_once('@')?;
    let (names, _) = candidates();
    if names.is_empty() || names.iter().any(|name| name.eq_ignore_ascii_case(host)) {
        return None;
    }
    let search = async {
        let login = login_of(&settings_of(ssh_target, port).await?)?;
        for name in names.iter().take(MAX_CANDIDATES) {
            let Some(settings) = settings_of(name, None).await else {
                continue;
            };
            if login_of(&settings).as_ref() == Some(&login) && crew_can_use(name, &settings).await {
                return Some(OwnAlias {
                    name: name.clone(),
                    sets_route: sets_route(&settings),
                });
            }
        }
        None
    };
    tokio::time::timeout(Duration::from_secs(5), search)
        .await
        .ok()
        .flatten()
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
        // An absolute path is taken as it stands. Built from the temporary directory rather than
        // written as `/etc/ssh/extra`: on Windows a path with a root but no drive is not
        // absolute, so that literal took the relative arm and came back as `C:/etc/ssh/extra`.
        let absolute = home.path().join("etc").join("extra");
        assert_eq!(included(absolute.to_str().unwrap(), &ssh), vec![absolute]);
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
        let logins: Vec<_> = table
            .iter()
            .map(|(name, host, port)| (*name, "localme", *host, *port, ""))
            .collect();
        fake_ssh_logins(root, &logins, config)
    }

    /// What the fake `ssh -G` prints after a name's own lines: settings Crew's connection
    /// preflight admits. A name's own line for a key comes first, and the first one is read.
    #[cfg(unix)]
    const POLICY_DEFAULTS: &str = "printf '%s\\n' 'stricthostkeychecking yes' 'forwardagent no' \
        'forwardx11 no' 'permitlocalcommand no' 'clearallforwardings yes' \
        'nohostauthenticationforlocalhost no' 'tunnel no' 'forkafterauthentication no' \
        'gssapidelegatecredentials no' 'proxycommand none' 'controlmaster no' \
        'controlpersist no' 'controlpath none'\n";

    /// [`fake_ssh`] with a `user` and any further `ssh -G` lines per name. Any other login
    /// resolves to its own user (else `localme`, the local account), its host and the port
    /// given with `-p` (else 22).
    #[cfg(unix)]
    fn fake_ssh_logins(
        root: &Path,
        table: &[(&str, &str, &str, u16, &str)],
        config: &str,
    ) -> env_lock::EnvGuard<'static> {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mut cases = String::new();
        for (name, user, host, port, extra) in table {
            cases.push_str(&format!(
                "    {name}) printf 'user {user}\\nhostname {host}\\nport {port}\\n{extra}' ;;\n"
            ));
        }
        std::fs::write(
            bin.join("ssh"),
            format!(
                "#!/bin/sh\n[ \"$1\" = \"-G\" ] || exit 1\nport=22\nprev=\nfor last; do [ \"$prev\" = \"-p\" ] && port=$last; prev=$last; done\nhost=${{last##*@}}\ncase \"$last\" in *@*) user=${{last%@*}} ;; *) user=localme ;; esac\ncase \"$host\" in\n{cases}    *) printf 'user %s\\nhostname %s\\nport %s\\n' \"$user\" \"$host\" \"$port\" ;;\nesac\n{POLICY_DEFAULTS}"
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

    /// G5: a login is saved under the person's own `Host` entry only when that entry signs in
    /// as the same user, to the same host name, on the same port.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_persons_own_alias_for_a_login_is_the_same_user_host_and_port() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _env = fake_ssh_logins(
            root.path(),
            &[
                ("leo-local", "localme", "leo.ucsf.edu", 22, ""),
                ("leo-port", "wanjun", "leo.ucsf.edu", 2222, ""),
                ("leo-tunnel", "wanjun", "leo.ucsf.edu", 22, "proxycommand nc gw 22\\n"),
                ("leo-kerberos", "wanjun", "leo.ucsf.edu", 22, "gssapidelegatecredentials yes\\n"),
                ("leo-tmux", "wanjun", "leo.ucsf.edu", 22, "remotecommand tmux attach\\n"),
                ("leo-wj-crew", "wanjun", "leo.ucsf.edu", 22, "proxycommand none\\n"),
                ("leo-second", "wanjun", "leo.ucsf.edu", 22, ""),
                ("gpu-box", "wanjun", "gpu.ucsf.edu", 22, "proxyjump gw.ucsf.edu\\n"),
                ("hpc", "wanjun", "hpc.ucsf.edu", 22, ""),
            ],
            "Host leo-local\nHost leo-port\nHost leo-tunnel\nHost leo-kerberos\nHost leo-tmux\nHost leo-wj-crew\nHost leo-second\nHost gpu-box\nHost hpc.ucsf.edu\n  IdentityFile /keys/hpc\nHost hpc\n",
        );
        let alias = |name: &str, sets_route: bool| {
            Some(OwnAlias {
                name: name.into(),
                sets_route,
            })
        };
        // The first entry, in file order, with the same user, host name and port. An entry
        // that signs in as the local account, or on another port, is not the same login, and
        // `proxycommand none` is no route of its own. One Crew's SSH command could not use is
        // passed over: a custom proxy command or delegated credentials, which the connection
        // preflight refuses, and a remote command of its own, which `ssh` refuses beside the
        // bridge's.
        assert_eq!(
            own_alias_for_login("wanjun@leo.ucsf.edu", None).await,
            alias("leo-wj-crew", false)
        );
        assert_eq!(
            own_alias_for_login("wanjun@leo.ucsf.edu", Some(2222)).await,
            alias("leo-port", false)
        );
        assert_eq!(
            own_alias_for_login("localme@leo.ucsf.edu", None).await,
            alias("leo-local", false)
        );
        // Another account, or a server no entry names: the login stays as it is.
        assert_eq!(
            own_alias_for_login("crew_bob@leo.ucsf.edu", None).await,
            None
        );
        assert_eq!(own_alias_for_login("wanjun@192.0.2.7", None).await, None);
        // An entry with its own jump host says so, for the invitation's hint to give way.
        assert_eq!(
            own_alias_for_login("wanjun@gpu.ucsf.edu", None).await,
            alias("gpu-box", true)
        );
        // The login's host is itself one of the person's entries: their settings already apply.
        assert_eq!(own_alias_for_login("wanjun@hpc.ucsf.edu", None).await, None);
        // An alias is not a login this looks up.
        assert_eq!(own_alias_for_login("leo-wj-crew", None).await, None);
        // The label is unchanged: the first entry for the same host name and port, whatever
        // account it signs in as.
        assert_eq!(server_label("wanjun@leo.ucsf.edu", None).await, "leo-local");
    }
}
