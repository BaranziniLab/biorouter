//! Trusting a server's host key the person verified (the owner's D2 ruling on AO-12), and the
//! known hosts files Crew's ssh reads.
//!
//! Crew forces `StrictHostKeyChecking yes` on every hop, so a server whose key this computer does
//! not know is refused. The person may now trust such a key from Crew or a chat, with their proof
//! (`POST /crew/host-keys/trust`), and only on these terms:
//!
//! * **Only an unknown key.** [`trust`] refuses whenever the known hosts files already hold any
//!   key for the host: a changed key, a revoked one, a certificate authority
//!   (`crew_ssh_host_key_changed`, the code a connect answers for a changed key). Crew never
//!   replaces or removes a key. A file it can't read, or an `ssh-keygen` that can't say what a
//!   file holds, could hide such a key, so that is refused too (`crew_request_refused`).
//! * **Only the key the person saw.** The daemon reads the key itself again with `ssh-keyscan`
//!   and writes only a key whose fingerprint equals the one the person was shown
//!   (`crew_ssh_host_key_changed` otherwise: the server's key is not the one they verified). The
//!   fingerprint is the request's only say in what is written; the key comes from the server.
//! * **Only a server this computer reaches straight.** The name and port are resolved with
//!   `ssh -G` as Crew's ssh resolves them (the development profile's configuration included); a
//!   jump host, a ProxyCommand or a HostKeyAlias means a scan from here may reach another
//!   machine than the one ssh reaches, so nothing is read or written (`crew_request_refused`).
//!   So does a jump host saved on a Crew connection to the server, which `ssh -G` can't see.
//!   A server that gives no key is `crew_ssh_unreachable`.
//!
//! The line goes to the first `UserKnownHostsFile` `ssh -G` names, hashed when `HashKnownHosts`
//! is set, under the name OpenSSH looks the key up by (`host`, or `[host]:port` off port 22).
//! [`known_hosts_files`] is the list a waiting chat card reads for the key.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use base64::Engine as _;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

/// The code of a trust refused because the known hosts files already hold a key for the host, or
/// because the key the server gives now is not the one the person was shown: either way, not
/// the unknown key the person verified. The same code a connect answers for a changed key.
pub const CHANGED: &str = "crew_ssh_host_key_changed";
/// The code of a server this computer read no host key from.
pub const UNREACHABLE: &str = "crew_ssh_unreachable";
/// The code of a key Crew won't read or write from here: the person's SSH settings reach the
/// server another way, can't be read, or name no known hosts file Crew can write.
pub const REFUSED: &str = "crew_request_refused";
/// The code of a request whose host, port, login or fingerprint is not plain.
pub const INVALID: &str = "crew_request_invalid";

/// How long one `ssh -G`, `ssh-keygen` or `ssh-keyscan` may take.
const TOOL_LIMIT: Duration = Duration::from_secs(15);
/// How much of their output is read.
const TOOL_OUTPUT: u64 = 256 * 1024;
/// The key types a scan asks for, in the order ssh prefers them.
const SCAN_TYPES: &str = "ed25519,ecdsa,rsa";

/// Trusts are made one at a time, so two clicks can never both find the host unknown.
static TRUSTING: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(Default::default);

/// A refusal to read or trust a host key: a code and one sentence for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyRefused {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for HostKeyRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HostKeyRefused {}

fn refused(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(HostKeyRefused {
        code,
        message: message.into(),
    })
}

/// A key this computer read from a server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct HostKeyRead {
    pub host: String,
    pub port: u16,
    /// `SHA256:…`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    /// The key's type, as OpenSSH names it (`ssh-ed25519`).
    pub key_type: String,
}

/// A trusted key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct HostKeyTrusted {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    /// The known hosts file already held exactly this key, so nothing was written.
    pub already_trusted: bool,
    /// The file the key is in.
    pub known_hosts_file: String,
}

/// What the person asked to read or trust, as the request named it.
#[derive(Clone, Debug)]
pub struct HostKeyTarget {
    /// The host as OpenSSH looked the key up by, without brackets or port.
    pub host: String,
    pub port: u16,
    /// The SSH login the refused step used (`you@lab-server`), so its own configuration is
    /// resolved; the host when none is given.
    pub login: Option<String>,
}

impl HostKeyTarget {
    /// Refuse a target whose parts are not plain names: nothing here reaches a command line as
    /// anything but an argument, and nothing that looks like an option does at all.
    fn check(&self) -> Result<()> {
        if !plain_host(&self.host) || self.port == 0 {
            return Err(refused(
                INVALID,
                "Name the server by its host name and port.",
            ));
        }
        if let Some(login) = &self.login {
            if !plain_login(login) {
                return Err(refused(
                    INVALID,
                    "Name the SSH login like you@hpc.example.edu.",
                ));
            }
        }
        Ok(())
    }

    /// The name the known hosts files hold the key under.
    pub fn lookup(&self) -> String {
        lookup_name(&self.host, self.port)
    }

    fn resolved_name(&self) -> &str {
        self.login.as_deref().unwrap_or(&self.host)
    }
}

/// `host` on port 22, else `[host]:port`, as OpenSSH files a key: in lower case, because ssh
/// lowercases the name before it looks a key up, and a hashed entry matches only that spelling.
pub fn lookup_name(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

/// A host name or address, and nothing else: letters, digits and `.-_:`, at most 253 bytes, not
/// starting with `-` or `.`.
fn plain_host(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && !text.starts_with(['-', '.'])
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(&b))
}

/// An SSH login (`user@host`, or a host alias): letters, digits and `._-@%+:`, not starting with
/// `-`, at most 320 bytes.
fn plain_login(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 320
        && !text.starts_with('-')
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@%+:".contains(&b))
}

/// Whether `text` is a SHA-256 host-key fingerprint and nothing else.
pub fn is_fingerprint(text: &str) -> bool {
    text.strip_prefix("SHA256:").is_some_and(|digest| {
        (32..=64).contains(&digest.len())
            && digest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    })
}

/// `SHA256:…` for an OpenSSH public key blob (the base64 word of a known hosts or
/// `ssh-keyscan` line), as `ssh-keygen -l` prints it.
pub fn fingerprint_of(blob: &str) -> Option<String> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .ok()?;
    let digest = Sha256::digest(&key);
    Some(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    ))
}

// ---------------------------------------------------------------------------------------------
// What ssh resolves
// ---------------------------------------------------------------------------------------------

/// What `ssh -G` resolved for a login, as far as host keys go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub hostname: Option<String>,
    pub port: Option<u16>,
    /// The account ssh signs in as (`User`, or this computer's own user name when unset).
    pub user: Option<String>,
    /// No jump host, no ProxyCommand and no HostKeyAlias: ssh reaches `hostname` straight from
    /// here and files its key under that name.
    pub direct: bool,
    pub user_files: Vec<PathBuf>,
    pub global_files: Vec<PathBuf>,
    pub hash: bool,
}

impl Resolved {
    /// Read `ssh -G`'s output. `home` expands a leading `~/`.
    pub fn parse(text: &str, home: Option<&Path>) -> Self {
        let setting = |key: &str| {
            text.lines().find_map(|line| {
                let (name, value) = line.split_once(' ')?;
                name.eq_ignore_ascii_case(key).then(|| value.trim())
            })
        };
        let unset = |key: &str| setting(key).is_none_or(|value| value.eq_ignore_ascii_case("none"));
        let files = |key: &str| -> Vec<PathBuf> {
            setting(key)
                .unwrap_or_default()
                .split_whitespace()
                .filter(|path| !path.eq_ignore_ascii_case("none"))
                .filter_map(|path| expand(path, home))
                .collect()
        };
        Self {
            hostname: setting("hostname").map(str::to_owned),
            port: setting("port").and_then(|port| port.parse().ok()),
            user: setting("user")
                .filter(|user| !user.is_empty())
                .map(str::to_owned),
            direct: unset("proxyjump") && unset("proxycommand") && unset("hostkeyalias"),
            user_files: files("userknownhostsfile"),
            global_files: files("globalknownhostsfile"),
            hash: setting("hashknownhosts").is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        }
    }

    /// Whether ssh reaches exactly `target`'s host and port straight from here.
    fn reaches_directly(&self, target: &HostKeyTarget) -> bool {
        self.direct
            && self
                .hostname
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(&target.host))
            && self.port == Some(target.port)
    }
}

/// `~/x` under `home`; any other relative path is not one ssh would read reliably.
fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        return home.map(|home| home.join(rest));
    }
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// The options Crew's own ssh carries that decide where host keys are read: the development
/// profile's configuration and known hosts file (`transport::login_args`), nothing otherwise.
fn profile_args() -> Vec<OsString> {
    let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") else {
        return Vec::new();
    };
    let ssh = PathBuf::from(profile).join("home/.ssh");
    let mut known = OsString::from("UserKnownHostsFile=");
    known.push(ssh.join("known_hosts"));
    vec![
        "-F".into(),
        ssh.join("config").into_os_string(),
        "-o".into(),
        known,
    ]
}

/// What `ssh -G` resolves for `login` (and `port`, when given), as Crew's ssh would. `None` when
/// it could not be read.
pub async fn resolve(login: &str, port: Option<u16>) -> Option<Resolved> {
    if !plain_login(login) {
        return None;
    }
    let mut command = key_tool("ssh");
    command.arg("-G").args(profile_args());
    if let Some(port) = port {
        command.arg("-p").arg(port.to_string());
    }
    command.arg(login);
    let (ran, text) = run_tool(command).await?;
    let home = crate::config::paths::Paths::home_dir();
    ran.then(|| Resolved::parse(&text, home.as_deref()))
}

/// The known hosts files Crew's ssh reads for `login` on `port`: the user files `ssh -G` names,
/// then the global ones. Where `ssh -G` can't be read, the development profile's file, else
/// OpenSSH's defaults.
pub async fn known_hosts_files(login: &str, port: Option<u16>) -> Vec<PathBuf> {
    match resolve(login, port).await {
        Some(resolved) if !resolved.user_files.is_empty() || !resolved.global_files.is_empty() => {
            let mut files = resolved.user_files;
            files.extend(resolved.global_files);
            files
        }
        _ => default_files(),
    }
}

fn default_files() -> Vec<PathBuf> {
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        return vec![PathBuf::from(profile).join("home/.ssh/known_hosts")];
    }
    let mut files: Vec<PathBuf> = crate::config::paths::Paths::home_dir()
        .map(|home| {
            vec![
                home.join(".ssh/known_hosts"),
                home.join(".ssh/known_hosts2"),
            ]
        })
        .unwrap_or_default();
    files.extend([
        PathBuf::from("/etc/ssh/ssh_known_hosts"),
        PathBuf::from("/etc/ssh/ssh_known_hosts2"),
    ]);
    files
}

// ---------------------------------------------------------------------------------------------
// What the known hosts files hold
// ---------------------------------------------------------------------------------------------

/// How a known hosts entry is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Plain,
    Revoked,
    CertAuthority,
}

/// One entry `ssh-keygen -l -F` listed: its marker and fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub marker: Marker,
    pub fingerprint: String,
}

/// The entries `ssh-keygen -l -F` printed. Each follows a `# Host NAME found: line N` header,
/// which ends in `REVOKED` or `CA` for a marked one.
pub fn listed_entries(output: &str) -> Vec<Entry> {
    let mut marker = None;
    let mut found = Vec::new();
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("# Host ") {
            marker = header.rsplit_once(" found: line ").map(|(_, rest)| {
                match rest.split_whitespace().nth(1) {
                    None => Marker::Plain,
                    Some("REVOKED") => Marker::Revoked,
                    Some(_) => Marker::CertAuthority,
                }
            });
            continue;
        }
        if let Some(marker) = marker.take() {
            found.extend(sha256_values(line).into_iter().map(|fingerprint| Entry {
                marker,
                fingerprint,
            }));
        }
    }
    found
}

/// The fingerprints of the plain entries `ssh-keygen -l -F` printed.
pub fn listed_fingerprints(output: &str) -> Vec<String> {
    listed_entries(output)
        .into_iter()
        .filter(|entry| entry.marker == Marker::Plain)
        .map(|entry| entry.fingerprint)
        .collect()
}

/// Every SHA-256 fingerprint in `line`.
fn sha256_values(line: &str) -> Vec<String> {
    const MARKER: &str = "SHA256:";
    line.match_indices(MARKER)
        .filter_map(|(at, _)| {
            let digest: String = line
                .get(at + MARKER.len()..)?
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
                .collect();
            let value = format!("{MARKER}{digest}");
            is_fingerprint(&value).then_some(value)
        })
        .collect()
}

/// Every entry `files` hold for `lookup`, as `ssh-keygen -l -F` reads them (hashed names and
/// patterns included; the name in lower case, as ssh looks it up). A file that does not exist
/// holds nothing. `Err` names the first file whose answer can't be read (it can't be opened,
/// `ssh-keygen` failed, timed out or printed what this can't parse), so a key on file is never
/// mistaken for none.
pub async fn listed(lookup: &str, files: &[PathBuf]) -> std::result::Result<Vec<Entry>, PathBuf> {
    let lookup = lookup.to_ascii_lowercase();
    let mut found = Vec::new();
    for file in files {
        match std::fs::metadata(file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(metadata) if metadata.is_file() => {}
            _ => return Err(file.clone()),
        }
        let mut command = key_tool("ssh-keygen");
        command.args(["-l", "-F", lookup.as_str(), "-f"]).arg(file);
        match lookup_answer(run_tool_code(command).await) {
            Some(entries) => found.extend(entries),
            None => return Err(file.clone()),
        }
    }
    Ok(found)
}

/// What one `ssh-keygen -l -F` run says its file holds: the entries it found (exit 0, each one
/// read), or none (exit 1 with nothing printed). `None` for anything else: it could not run or
/// timed out, read the file with an error (exit 255), or printed an entry this can't read.
fn lookup_answer(run: Option<(Option<i32>, String)>) -> Option<Vec<Entry>> {
    match run? {
        (Some(0), output) => {
            let entries = listed_entries(&output);
            (!entries.is_empty()).then_some(entries)
        }
        (Some(1), output) if output.trim().is_empty() => Some(Vec::new()),
        _ => None,
    }
}

/// What the known hosts files say about trusting `fingerprint` for a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// No key at all: the one case a key may be added.
    Unknown,
    /// Exactly this key, plain, and nothing else.
    AlreadyTrusted,
    /// Any other key, a revoked one or a certificate authority: never touched.
    Known,
}

fn standing(entries: &[Entry], fingerprint: &str) -> Standing {
    if entries.is_empty() {
        return Standing::Unknown;
    }
    let only_this = entries
        .iter()
        .all(|entry| entry.marker == Marker::Plain && entry.fingerprint == fingerprint);
    if only_this {
        Standing::AlreadyTrusted
    } else {
        Standing::Known
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a key, and trusting it
// ---------------------------------------------------------------------------------------------

/// One key an `ssh-keyscan` line gave: its type, its blob and its fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Scanned {
    key_type: String,
    blob: String,
    fingerprint: String,
}

/// The keys `ssh-keyscan` printed, in its order. Comment lines and words that are not a key
/// type and a base64 blob are skipped.
fn scanned_keys(output: &str) -> Vec<Scanned> {
    output
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (_name, key_type, blob) = (words.next()?, words.next()?, words.next()?);
            let typed = !key_type.is_empty()
                && key_type
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b));
            let fingerprint = fingerprint_of(blob)?;
            typed.then(|| Scanned {
                key_type: key_type.to_owned(),
                blob: blob.to_owned(),
                fingerprint,
            })
        })
        .collect()
}

/// The sentence for a server reached through another computer or under another name.
fn reached_another_way(host: &str) -> anyhow::Error {
    refused(
        REFUSED,
        format!("Your SSH settings reach {host} through another computer or under another name, so Crew can't read its key from here. Add the key in a terminal of your own."),
    )
}

/// Whether a saved Crew connection that names the target (its login, or a login on its host)
/// reaches it through a jump host. Crew hands a saved jump host to ssh itself, so `ssh -G`
/// can't see it. `saved` is each connection's login and jump hosts.
fn saved_route_jumps<'a>(
    target: &HostKeyTarget,
    saved: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> bool {
    let login = target.resolved_name();
    saved.into_iter().any(|(saved_login, jump)| {
        let saved_host = saved_login.rsplit_once('@').map_or(saved_login, |(_, h)| h);
        let names = saved_login.eq_ignore_ascii_case(login)
            || saved_host.eq_ignore_ascii_case(&target.host);
        let jumps = jump
            .map(str::trim)
            .is_some_and(|jump| !jump.is_empty() && !jump.eq_ignore_ascii_case("none"));
        names && jumps
    })
}

/// Refuse a target a saved connection reaches through a jump host ([`saved_route_jumps`]): a
/// key read from here could be another machine's. Refused too when the saved connections can't
/// be read, since one of them could.
async fn refuse_saved_jump(target: &HostKeyTarget) -> Result<()> {
    let host = &target.host;
    let crew = super::manager().map_err(|_| {
        refused(
            REFUSED,
            format!("Crew couldn't read its saved connections, so it read no key for {host}. Add the key in a terminal of your own."),
        )
    })?;
    let saved = crew.list().await;
    let routes = saved.iter().map(|connection| {
        (
            connection.ssh_target.as_str(),
            connection.proxy_jump.as_deref(),
        )
    });
    if saved_route_jumps(target, routes) {
        return Err(reached_another_way(host));
    }
    Ok(())
}

/// What `ssh -G` resolves for the target, as Crew's ssh would; refused when it can't be read.
async fn resolve_target(target: &HostKeyTarget) -> Result<Resolved> {
    resolve(target.resolved_name(), Some(target.port))
        .await
        .ok_or_else(|| {
            let host = &target.host;
            refused(
                REFUSED,
                format!("Crew couldn't read your SSH settings for {host}, so it read no key. Add the key in a terminal of your own."),
            )
        })
}

/// The host's keys, read straight from this computer, once `resolved` (what `ssh -G` said for
/// the target) confirms ssh reaches the host straight from here.
async fn scan(target: &HostKeyTarget, resolved: &Resolved) -> Result<Vec<Scanned>> {
    let host = &target.host;
    if !resolved.reaches_directly(target) {
        return Err(reached_another_way(host));
    }
    let port = target.port.to_string();
    let mut command = key_tool("ssh-keyscan");
    command
        .args(["-T", "5", "-t", SCAN_TYPES, "-p", port.as_str()])
        .arg(host);
    let keys = match run_tool(command).await {
        Some((_, output)) => scanned_keys(&output),
        None => Vec::new(),
    };
    if keys.is_empty() {
        return Err(refused(
            UNREACHABLE,
            format!("This computer couldn't read a host key from {host}. Check that it is reachable, or add the key in a terminal of your own."),
        ));
    }
    Ok(keys)
}

/// Read the host's key straight from this computer, for the person to compare with the
/// fingerprint their IT team gave them: the first of the types ssh prefers.
pub async fn read(target: HostKeyTarget) -> Result<HostKeyRead> {
    target.check()?;
    refuse_saved_jump(&target).await?;
    let resolved = resolve_target(&target).await?;
    let key = scan(&target, &resolved).await?.remove(0);
    Ok(HostKeyRead {
        host: target.host,
        port: target.port,
        fingerprint: key.fingerprint,
        key_type: key.key_type,
    })
}

/// Trust the host's key whose fingerprint is `fingerprint`, which the person was shown and
/// verified. See the module documentation for every refusal.
pub async fn trust(target: HostKeyTarget, fingerprint: &str) -> Result<HostKeyTrusted> {
    target.check()?;
    if !is_fingerprint(fingerprint) {
        return Err(refused(
            INVALID,
            "Send the fingerprint as SHA256: and its digest.",
        ));
    }
    let _one_at_a_time = TRUSTING.lock().await;
    let host = target.host.clone();
    refuse_saved_jump(&target).await?;
    let resolved = resolve_target(&target).await?;
    let files: Vec<PathBuf> = resolved
        .user_files
        .iter()
        .chain(&resolved.global_files)
        .cloned()
        .collect();
    let first = resolved.user_files.first().cloned();
    let entries = listed(&target.lookup(), &files).await.map_err(|file| {
        refused(
            REFUSED,
            format!("Crew couldn't read {}, so it can't tell whether this computer already knows a key for {host}. Nothing was added.", file.display()),
        )
    })?;
    match standing(&entries, fingerprint) {
        Standing::AlreadyTrusted => {
            return Ok(trusted(&target, fingerprint, true, first.as_deref()));
        }
        Standing::Known => {
            return Err(refused(
                CHANGED,
                format!("This computer already knows a host key for {host}, so Crew won't add another. If the server's key changed, don't connect until your IT team confirms it."),
            ));
        }
        Standing::Unknown => {}
    }
    let Some(file) = first else {
        return Err(refused(
            REFUSED,
            format!("Crew couldn't find your known hosts file, so nothing was added for {host}."),
        ));
    };
    let key = scan(&target, &resolved)
        .await?
        .into_iter()
        .find(|key| key.fingerprint == fingerprint)
        .ok_or_else(|| {
            refused(
                CHANGED,
                format!("The key {host} gives now doesn't match the fingerprint you were shown, so nothing was added. Check the fingerprint with your IT team again."),
            )
        })?;
    write_key(&target, &key, resolved.hash, &file).await?;
    Ok(trusted(&target, fingerprint, false, Some(&file)))
}

/// Append `key` for the target to the known hosts file `file`, under its lookup name, hashed
/// when `hash` (`HashKnownHosts yes`).
async fn write_key(target: &HostKeyTarget, key: &Scanned, hash: bool, file: &Path) -> Result<()> {
    let lookup = target.lookup();
    let name = if hash {
        hashed_name(&lookup).await.ok_or_else(|| {
            refused(
                REFUSED,
                "Crew couldn't hash the server's name for your known hosts file, so nothing was added.",
            )
        })?
    } else {
        lookup
    };
    append_line(file, &format!("{name} {} {}", key.key_type, key.blob)).map_err(|_| {
        refused(
            REFUSED,
            format!(
                "Crew couldn't write to {}, so nothing was added.",
                file.display()
            ),
        )
    })
}

fn trusted(
    target: &HostKeyTarget,
    fingerprint: &str,
    already_trusted: bool,
    file: Option<&Path>,
) -> HostKeyTrusted {
    HostKeyTrusted {
        host: target.host.clone(),
        port: target.port,
        fingerprint: fingerprint.to_owned(),
        already_trusted,
        known_hosts_file: file
            .map(|file| file.display().to_string())
            .unwrap_or_default(),
    }
}

/// `lookup` hashed as `HashKnownHosts` files it (`|1|salt|hash`), by `ssh-keygen -H` on a copy
/// in a private temporary folder. `None` when it could not be hashed.
async fn hashed_name(lookup: &str) -> Option<String> {
    let dir = tempfile::tempdir().ok()?;
    let file = dir.path().join("known_hosts");
    // Any well-formed key does: only the name is kept.
    std::fs::write(
        &file,
        format!("{lookup} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDVMyBQ3psSwstgc8+Mce5VHIZN5GFuu3xE6WV8pfFkD\n"),
    )
    .ok()?;
    let mut command = key_tool("ssh-keygen");
    command.arg("-H").arg("-f").arg(&file);
    let (ran, _) = run_tool(command).await?;
    if !ran {
        return None;
    }
    let text = std::fs::read_to_string(&file).ok()?;
    let name = text.lines().next()?.split_whitespace().next()?;
    (name.starts_with("|1|") && name.len() < 200).then(|| name.to_owned())
}

/// Append `line` to the known hosts file `file`, on a line of its own: the folder is made
/// (owner only) and the file created (owner read and write) when missing.
fn append_line(file: &Path, line: &str) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    if let Some(parent) = file.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut open = options.open(file)?;
    let length = open.metadata()?.len();
    let mut text = String::new();
    if length > 0 {
        let mut last = [0u8; 1];
        open.seek(SeekFrom::Start(length - 1))?;
        open.read_exact(&mut last)?;
        if last[0] != b'\n' {
            text.push('\n');
        }
    }
    text.push_str(line);
    text.push('\n');
    open.write_all(text.as_bytes())?;
    open.sync_all()
}

/// A key tool (`ssh`, `ssh-keygen`, `ssh-keyscan`) as the daemon's own child: no console
/// window on Windows, and none of the daemon's own secrets in its environment.
pub fn key_tool(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    crate::subprocess::prepare_agent_child_command(&mut command);
    command
}

/// Run a key tool (`ssh -G`, `ssh-keygen`, `ssh-keyscan`) as the daemon's own child: no input,
/// its output bounded, killed past [`TOOL_LIMIT`]. Whether it exited cleanly, and what it
/// printed; `None` when it could not run or printed nothing readable.
pub async fn run_tool(command: tokio::process::Command) -> Option<(bool, String)> {
    let (code, output) = run_tool_code(command).await?;
    Some((code == Some(0), output))
}

/// [`run_tool`], with the exit code (`None` when a signal ended it).
async fn run_tool_code(mut command: tokio::process::Command) -> Option<(Option<i32>, String)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::subprocess::prepare_agent_child_command(&mut command);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let ran = async move {
        let mut bytes = Vec::new();
        stdout
            .take(TOOL_OUTPUT)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        let status = child.wait().await.ok()?;
        Some((status.code(), bytes))
    };
    let (code, bytes) = tokio::time::timeout(TOOL_LIMIT, ran).await.ok()??;
    Some((code, String::from_utf8(bytes).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIDVMyBQ3psSwstgc8+Mce5VHIZN5GFuu3xE6WV8pfFkD";
    const FINGERPRINT: &str = "SHA256:XA01Ou0eChdPDyZiadGxCZ/g+7RnODImHcZEtuRmV8g";
    const OTHER: &str = "SHA256:TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmU";

    fn target(host: &str, port: u16, login: Option<&str>) -> HostKeyTarget {
        HostKeyTarget {
            host: host.into(),
            port,
            login: login.map(str::to_owned),
        }
    }

    #[test]
    fn a_key_blob_s_fingerprint_is_the_one_ssh_keygen_prints() {
        assert_eq!(fingerprint_of(BLOB).as_deref(), Some(FINGERPRINT));
        assert_eq!(fingerprint_of("not base64!"), None);
        assert!(is_fingerprint(FINGERPRINT));
        assert!(!is_fingerprint("SHA256:short"));
        assert!(!is_fingerprint(&format!("{FINGERPRINT} ")));
    }

    #[test]
    fn a_key_is_filed_under_the_name_openssh_looks_it_up_by() {
        assert_eq!(lookup_name("hpc.example.edu", 22), "hpc.example.edu");
        assert_eq!(lookup_name("127.0.0.1", 2222), "[127.0.0.1]:2222");
    }

    #[test]
    fn only_plain_names_reach_a_command_line() {
        assert!(target("hpc.example.edu", 22, Some("you@hpc"))
            .check()
            .is_ok());
        for bad in [
            target("-oProxyCommand=x", 22, None),
            target("hpc example", 22, None),
            target("[hpc]:22", 22, None),
            target("hpc.example.edu", 0, None),
            target("hpc.example.edu", 22, Some("-F/tmp/x")),
            target("hpc.example.edu", 22, Some("you@hpc;rm")),
        ] {
            let error = bad.check().unwrap_err();
            let refused = error.downcast_ref::<HostKeyRefused>().unwrap();
            assert_eq!(refused.code, INVALID, "{bad:?}");
        }
    }

    /// `ssh -G`'s list of known hosts files, its hashing and whether the host is reached
    /// straight from here.
    #[test]
    fn what_ssh_resolves_is_read_from_its_own_words() {
        let home = PathBuf::from("/home/you");
        let text = "host lab\nhostname hpc.example.edu\nuser alice\nport 2222\nproxycommand none\nhashknownhosts yes\nuserknownhostsfile ~/.ssh/known_hosts /home/you/.ssh/known_hosts2\nglobalknownhostsfile /etc/ssh/ssh_known_hosts\n";
        let resolved = Resolved::parse(text, Some(&home));
        assert_eq!(resolved.hostname.as_deref(), Some("hpc.example.edu"));
        assert_eq!(resolved.port, Some(2222));
        assert_eq!(resolved.user.as_deref(), Some("alice"));
        assert!(resolved.direct);
        assert!(resolved.hash);
        assert_eq!(
            resolved.user_files,
            vec![
                home.join(".ssh/known_hosts"),
                home.join(".ssh/known_hosts2")
            ]
        );
        assert_eq!(
            resolved.global_files,
            vec![PathBuf::from("/etc/ssh/ssh_known_hosts")]
        );
        assert!(resolved.reaches_directly(&target("hpc.example.edu", 2222, None)));
        assert!(resolved.reaches_directly(&target("HPC.example.edu", 2222, None)));
        assert!(!resolved.reaches_directly(&target("hpc.example.edu", 22, None)));
        assert!(!resolved.reaches_directly(&target("10.0.0.7", 2222, None)));
        for indirect in [
            format!("{text}proxyjump gate.example.edu\n"),
            text.replace("proxycommand none", "proxycommand nc %h %p"),
            format!("{text}hostkeyalias lab\n"),
        ] {
            assert!(
                !Resolved::parse(&indirect, Some(&home)).direct,
                "{indirect}"
            );
        }
        let plain = Resolved::parse(
            "hostname h\nport 22\nhashknownhosts no\nuserknownhostsfile relative/file\n",
            None,
        );
        assert!(!plain.hash);
        assert!(plain.user_files.is_empty(), "a relative path is never read");
    }

    #[test]
    fn every_listed_entry_keeps_its_marker() {
        let output = format!(
            "# Host h found: line 1 REVOKED\nh ED25519 {OTHER}\n# Host h found: line 2 CA\nh ED25519 {OTHER}\n# Host h found: line 3 \nh ED25519 {FINGERPRINT}\n"
        );
        assert_eq!(
            listed_entries(&output),
            vec![
                Entry {
                    marker: Marker::Revoked,
                    fingerprint: OTHER.into()
                },
                Entry {
                    marker: Marker::CertAuthority,
                    fingerprint: OTHER.into()
                },
                Entry {
                    marker: Marker::Plain,
                    fingerprint: FINGERPRINT.into()
                },
            ]
        );
        assert_eq!(listed_fingerprints(&output), vec![FINGERPRINT.to_owned()]);
    }

    /// Only a host with no key at all may get one; exactly this key already there is no change;
    /// anything else (another key, a revoked one, a certificate authority) is never touched.
    #[test]
    fn a_key_is_added_only_where_none_is_known() {
        let plain = |fingerprint: &str| Entry {
            marker: Marker::Plain,
            fingerprint: fingerprint.into(),
        };
        assert_eq!(standing(&[], FINGERPRINT), Standing::Unknown);
        assert_eq!(
            standing(&[plain(FINGERPRINT)], FINGERPRINT),
            Standing::AlreadyTrusted
        );
        assert_eq!(standing(&[plain(OTHER)], FINGERPRINT), Standing::Known);
        assert_eq!(
            standing(&[plain(FINGERPRINT), plain(OTHER)], FINGERPRINT),
            Standing::Known
        );
        for marker in [Marker::Revoked, Marker::CertAuthority] {
            let entry = Entry {
                marker,
                fingerprint: FINGERPRINT.into(),
            };
            assert_eq!(standing(&[entry], FINGERPRINT), Standing::Known);
        }
    }

    #[test]
    fn a_scan_yields_only_well_formed_keys() {
        let output = format!(
            "# hpc.example.edu:22 SSH-2.0-OpenSSH_9.6\nhpc.example.edu ssh-ed25519 {BLOB}\nhpc.example.edu ssh-rsa not-base64!\nhpc.example.edu bad;type {BLOB}\n"
        );
        assert_eq!(
            scanned_keys(&output),
            vec![Scanned {
                key_type: "ssh-ed25519".into(),
                blob: BLOB.into(),
                fingerprint: FINGERPRINT.into(),
            }]
        );
    }

    #[test]
    fn a_line_is_appended_on_a_line_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new/.ssh/known_hosts");
        append_line(&file, "a ssh-ed25519 X").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a ssh-ed25519 X\n");
        std::fs::write(&file, "a ssh-ed25519 X").unwrap();
        append_line(&file, "b ssh-ed25519 Y").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "a ssh-ed25519 X\nb ssh-ed25519 Y\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(file.parent().unwrap()), 0o700);
            let fresh = dir.path().join("fresh");
            append_line(&fresh, "c ssh-ed25519 Z").unwrap();
            assert_eq!(mode(&fresh), 0o600);
        }
    }

    /// The name is hashed the way ssh reads it back.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_hashed_name_is_one_ssh_keygen_finds() {
        let Some(name) = hashed_name("[hpc.example.edu]:2222").await else {
            // No ssh-keygen on this machine: nothing to check against.
            return;
        };
        assert!(name.starts_with("|1|"), "{name}");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        std::fs::write(&file, format!("{name} ssh-ed25519 {BLOB}\n")).unwrap();
        let entries = listed("[hpc.example.edu]:2222", std::slice::from_ref(&file)).await;
        assert_eq!(
            entries,
            Ok(vec![Entry {
                marker: Marker::Plain,
                fingerprint: FINGERPRINT.into()
            }])
        );
        // Another spelling of the same name finds the hashed entry too, as ssh does: it
        // lowercases the name before it looks the key up.
        let upper = target("HPC.Example.EDU", 2222, None).lookup();
        assert_eq!(upper, "[hpc.example.edu]:2222");
        assert_eq!(
            listed("[HPC.EXAMPLE.EDU]:2222", std::slice::from_ref(&file)).await,
            entries
        );
        assert_eq!(listed("hpc.example.edu", &[file]).await, Ok(Vec::new()));
    }

    /// A jump host saved on a Crew connection, which `ssh -G` can't see, keeps a server from
    /// being read from here: by the connection's own login, or by any login on its host.
    #[test]
    fn a_saved_jump_host_keeps_a_server_from_being_read_from_here() {
        let jumped = [("bob@hpc.example.edu", Some("gate.example.edu"))];
        for asked in [
            target("hpc.example.edu", 22, Some("bob@hpc.example.edu")),
            target("hpc.example.edu", 22, None),
            target("HPC.example.edu", 22, Some("alice@HPC.example.edu")),
        ] {
            assert!(saved_route_jumps(&asked, jumped), "{asked:?}");
        }
        let direct = target("hpc.example.edu", 22, Some("bob@hpc.example.edu"));
        for saved in [
            [("bob@hpc.example.edu", None)],
            [("bob@hpc.example.edu", Some(" "))],
            [("bob@hpc.example.edu", Some("none"))],
            [("bob@other.example.edu", Some("gate.example.edu"))],
        ] {
            assert!(!saved_route_jumps(&direct, saved), "{saved:?}");
        }
    }

    /// A host's key is filed and looked up under the lower case name ssh uses.
    #[test]
    fn a_name_is_looked_up_in_lower_case() {
        assert_eq!(lookup_name("HPC.Example.EDU", 22), "hpc.example.edu");
        assert_eq!(lookup_name("Lab-Server", 2222), "[lab-server]:2222");
        assert_eq!(
            target("HPC.EXAMPLE.EDU", 22, None).lookup(),
            "hpc.example.edu"
        );
    }

    /// Only a clean answer counts as "no key on file": exit 1 with nothing printed. A tool that
    /// could not run or timed out, an error reading the file, or an entry this can't read could
    /// all hide a key, so none of them reads as unknown.
    #[test]
    fn only_a_clean_not_found_reads_as_no_key() {
        let found = format!("# Host h found: line 1 \nh ED25519 {FINGERPRINT}\n");
        assert_eq!(
            lookup_answer(Some((Some(0), found))),
            Some(vec![Entry {
                marker: Marker::Plain,
                fingerprint: FINGERPRINT.into()
            }])
        );
        assert_eq!(lookup_answer(Some((Some(1), String::new()))), Some(vec![]));
        for unreadable in [
            None,
            Some((Some(255), String::new())),
            Some((Some(2), String::new())),
            Some((None, String::new())),
            Some((Some(1), "do_known_hosts: something".to_owned())),
            Some((
                Some(0),
                "# Host h found: line 1 \nh ED25519 MD5:aa\n".to_owned(),
            )),
        ] {
            assert_eq!(lookup_answer(unreadable.clone()), None, "{unreadable:?}");
        }
    }

    /// A known hosts file this can't read names itself, and so does a path that is not a file;
    /// a file that does not exist holds nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_that_cannot_be_read_is_never_read_as_empty() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(
            listed("h", std::slice::from_ref(&missing)).await,
            Ok(vec![])
        );
        assert_eq!(
            listed("h", &[dir.path().to_path_buf()]).await,
            Err(dir.path().to_path_buf())
        );
        let locked = dir.path().join("known_hosts");
        std::fs::write(&locked, format!("h ssh-ed25519 {BLOB}\n")).unwrap();
        if listed("h", std::slice::from_ref(&locked)).await.is_err() {
            // No ssh-keygen on this machine: nothing to check against.
            return;
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked).is_ok() {
            // Running as root, which reads any file: nothing to check against.
            return;
        }
        assert_eq!(
            listed("h", &[missing, locked.clone()]).await,
            Err(locked.clone())
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// Every sentence this module says, with a host in it, uses no dash and says computer.
    #[test]
    fn refusals_are_plain_sentences() {
        let source = include_str!("host_keys.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        assert!(!body.contains(['\u{2014}', '\u{2013}']));
        assert!(!body.to_lowercase().contains("device"));
    }
}
