//! OW-9: setting Crew up on a server, as one daemon service the desktop and the CLI share.
//!
//! A setup job checks the server, signs in when the server asks for a password or a code,
//! installs Crew's server program in the person's own account at `~/.local/bin/biorouter-crew`,
//! and then starts the workspace (a host) or connects to it (a member). Nothing a renderer, a
//! terminal or an agent writes becomes command text:
//!
//! - **Every remote command is a fixed script.** Each step runs `exec /bin/sh -c '<script>'`,
//!   where the script is a constant of this module with no single quote, no `!`, no newline
//!   and no backslash pair (so every common login shell, csh and fish included, hands it to
//!   `/bin/sh` unchanged). Its only substitutions are values this daemon made or checked: a
//!   job number (32 hex digits), the artifact's size, SHA-256 and version, the SHA-256 the
//!   probe found, and a workspace name under the broker's own rule (`[a-z0-9-]`). `$HOME` is
//!   expanded on the server only, inside double quotes, so a home folder with spaces is safe.
//!   Free text coming back (paths, version lines) is hex-encoded by the server and decoded
//!   here.
//! - **The program is this app's own copy.** It comes from the desktop build itself
//!   (`crew-broker/linux-x86_64/` beside `biorouterd`, with a manifest), is checked against
//!   that manifest and this daemon's version before it is sent, travels over the `ssh`
//!   session's standard input, and is checked again on the server (size, SHA-256, its own
//!   `--version`) before an atomic rename puts it in place. The copy it replaces stays as
//!   `biorouter-crew.previous`. A newer copy is never replaced by an older one, a link is never
//!   followed, and a file another account owns is never touched.
//! - **The person's own account only, signed in once.** Every step multiplexes over the SSH
//!   control path the connection itself uses (for a host, the preparation's: a prepared
//!   connection's ID *is* its preparation ID), with `BatchMode=yes`. A server that asks for a
//!   password parks the job at `needs_sign_in` until the person signs in through the native
//!   terminal (`POST /crew/setup/{job_id}/authentication`), whose master every later step, the
//!   start, Create and the connect ride: one password covers them all. That master serves only
//!   the login and route it signed in to: a job for another route, or a connection saved with
//!   another, ends it rather than riding it.
//! - **A member's agents get only the access the person chose** (`agent_access`, design
//!   OW-6). A job that was not told a level leaves the connection's access exactly as it is.

use super::{
    authentication, host_start, ssh_policy, transport, AuthenticationPlan, Connection, CrewManager,
    RemoteAccess, SaveConnection, SshFailureKind, StartOutput,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// How long the server probe, the work folder and the context probe may each take.
const PROBE_LIMIT: Duration = Duration::from_secs(60);
/// How long the transfer and install, and the start, may each take.
const INSTALL_LIMIT: Duration = Duration::from_secs(180);
/// How long a job waits for the person to sign in.
const SIGN_IN_WAIT: Duration = Duration::from_secs(10 * 60);
/// How long a finished job stays readable.
const KEEP_FINISHED: Duration = Duration::from_secs(15 * 60);
/// At most this many jobs at once, across every server on this daemon.
const MAX_RUNNING: usize = 2;
/// The largest server program this daemon will read and send.
const MAX_ARTIFACT: u64 = 64 * 1024 * 1024;
/// The platform Crew's server program is built for.
const BROKER_TARGET: &str = "x86_64-unknown-linux-gnu";
/// Where a build keeps it, relative to `biorouterd`'s own folder.
const BUNDLE_DIR: &str = "crew-broker/linux-x86_64";
/// A development copy: a folder holding `biorouter-crew` and, usually, its `manifest.json`.
const BROKER_DIR_ENV: &str = "BIOROUTER_CREW_BROKER_DIR";
/// The C library the release broker is linked against at most (`GLIBC_MAX` in
/// `scripts/release.sh`), for a copy whose manifest does not say.
const GLIBC_FLOOR: &str = "2.31";
/// What a person reads for the installed program's place on the server.
const INSTALLED_PATH: &str = "~/.local/bin/biorouter-crew";

/// The folders a host's workspace would be kept under that this account owns and that other
/// accounts may write to: `w` walks up from `~/.local/share/biorouter-crew` (or the nearest
/// folder of it that exists), as its real path, to `/`, and calls `$A` with each folder whose
/// mode has group or other write and no sticky bit and whose owner is this account. The broker
/// refuses to keep a workspace under any such folder (`unsafe_storage: writable non-sticky
/// ancestor`), so setup asks first.
macro_rules! storage_walk {
    () => {
        concat!(
            "U=$(id -u); ",
            "w() { D=\"$HOME/.local/share/biorouter-crew\"; ",
            "until [ -d \"$D\" ] || [ \"$D\" = / ] || [ -z \"$D\" ]; do D=$(dirname \"$D\"); done; ",
            "D=$(cd \"$D\" 2>/dev/null && pwd -P); ",
            "while [ -n \"$D\" ]; do set -- $(ls -ldn \"$D\" 2>/dev/null); ",
            "case \"$1\" in ?????w*|????????w*) case \"$1\" in ?????????[tT]*) ;; *) [ \"$3\" = \"$U\" ] && \"$A\" \"$D\";; esac;; esac; ",
            "[ \"$D\" = / ] && break; D=$(dirname \"$D\"); done; }; ",
        )
    };
}

/// What the server says about itself. Reads only the account's own facts and files.
const PROBE_SCRIPT: &str = concat!(
    "set -f; P=BRCREW_PROBE; ",
    "h() { od -An -v -tx1 | tr -d \" \\n\"; }; ",
    "echo \"$P begin 1\"; ",
    "echo \"$P uid $(id -u)\"; ",
    "echo \"$P user_hex $(id -un | head -c 200 | h)\"; ",
    "echo \"$P home_hex $(printf %s \"$HOME\" | head -c 4096 | h)\"; ",
    "echo \"$P os $(uname -s)\"; echo \"$P arch $(uname -m)\"; ",
    "echo \"$P kernel_hex $(uname -r | head -c 200 | h)\"; ",
    "echo \"$P glibc $(getconf GNU_LIBC_VERSION 2>/dev/null | cut -d \" \" -f 2)\"; ",
    "for t in sha256sum shasum openssl timeout; do command -v \"$t\" >/dev/null 2>&1 && echo \"$P tool $t\"; done; ",
    "k() { if [ -L \"$1\" ]; then echo symlink; elif [ -f \"$1\" ]; then echo file; elif [ -d \"$1\" ]; then echo dir; elif [ -e \"$1\" ]; then echo other; else echo missing; fi; }; ",
    "B=\"$HOME/.local/bin\"; F=\"$B/biorouter-crew\"; ",
    "set -- $(df -Pk \"$HOME\" 2>/dev/null | tail -n 1); echo \"$P free_kib $4\"; ",
    "echo \"$P bin_dir $(k \"$B\")\"; echo \"$P binary $(k \"$F\")\"; ",
    "if [ -L \"$F\" ]; then :; elif [ -f \"$F\" ]; then ",
    "set -- $(ls -ln \"$F\"); echo \"$P binary_owner $3\"; ",
    "S=$( { sha256sum \"$F\" 2>/dev/null || shasum -a 256 \"$F\" 2>/dev/null || openssl dgst -sha256 -r \"$F\" 2>/dev/null; } | cut -c 1-64); ",
    "echo \"$P binary_sha256 $S\"; ",
    "if command -v timeout >/dev/null 2>&1; then V=$(timeout 10 \"$F\" --version 2>&1); else V=$(\"$F\" --version 2>&1); fi; C=$?; ",
    "echo \"$P binary_exit $C\"; echo \"$P binary_version_hex $(printf %s \"$V\" | head -c 400 | h)\"; ",
    "fi; ",
    storage_walk!(),
    "r() { echo \"$P storage_unsafe_hex $(printf %s \"$1\" | head -c 4096 | h)\"; }; A=r; w; ",
    "echo \"$P end\""
);

/// Take write access for other accounts away (`chmod go-w`) from exactly the folders the walk
/// finds, each checked again here: this account's, a folder, not a link, and inside the
/// account's own home (its real path). Then say what changed and what the walk still finds.
/// Nothing is substituted: the server finds the folders itself.
const STORAGE_FIX_SCRIPT: &str = concat!(
    "set -f; P=BRCREW_STORAGE; ",
    "h() { od -An -v -tx1 | tr -d \" \\n\"; }; ",
    storage_walk!(),
    "H=$(cd \"$HOME\" 2>/dev/null && pwd -P); [ \"$H\" = / ] && H=; ",
    "r() { echo \"$P $1 $(printf %s \"$2\" | head -c 4096 | h)\"; }; ",
    "x() { if [ -n \"$H\" ] && [ -d \"$1\" ]; then case \"$1\" in \"$H\"|\"$H\"/*) ",
    "if [ -L \"$1\" ]; then :; elif chmod go-w \"$1\"; then r fixed \"$1\"; fi;; esac; fi; }; ",
    "u() { r unsafe \"$1\"; }; ",
    "A=x; w; A=u; w; echo \"$P done\""
);

/// Receive the program on standard input and install it. `@JOB@`, `@SIZE@`, `@SHA@`, `@VER@`
/// and `@PREV@` (the SHA-256 the probe found, or `none`) are the only substitutions.
const INSTALL_SCRIPT: &str = concat!(
    "set -f; P=BRCREW_INSTALL; ",
    "h() { od -An -v -tx1 | tr -d \" \\n\"; }; ",
    "f() { echo \"$P fail $*\"; exit 3; }; ",
    "B=\"$HOME/.local/bin\"; F=\"$B/biorouter-crew\"; R=\"$B/biorouter-crew.previous\"; ",
    "T=\"$B/.biorouter-crew.@JOB@.partial\"; L=\"$B/.biorouter-crew.install-lock\"; ",
    // A folder this makes is writable by the account alone, whatever the login's umask: a
    // group-writable ~/.local is a folder Crew then refuses to keep a workspace under.
    "umask 022; mkdir -p \"$B\" || f permission_denied bin_dir; ",
    "umask 077; ",
    "if [ -L \"$B\" ]; then f unexpected_target bin_dir; fi; [ -d \"$B\" ] || f unexpected_target bin_dir; ",
    "set -- $(ls -ldn \"$B\"); [ \"$3\" = \"$(id -u)\" ] || f unexpected_target bin_dir_owner; ",
    "if [ -d \"$L\" ] && [ -n \"$(find \"$L\" -maxdepth 0 -mmin +10 2>/dev/null)\" ]; then rmdir \"$L\" 2>/dev/null; fi; ",
    "mkdir \"$L\" 2>/dev/null || f busy; ",
    "c() { rm -f \"$T\"; rmdir \"$L\" 2>/dev/null; }; trap c EXIT; trap \"c; exit 4\" HUP INT TERM; ",
    "s() { { sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\" 2>/dev/null || openssl dgst -sha256 -r \"$1\" 2>/dev/null; } | cut -c 1-64; }; ",
    "if [ -L \"$F\" ]; then f unexpected_target binary_symlink; fi; ",
    "if [ -e \"$F\" ]; then [ -f \"$F\" ] || f unexpected_target binary_other; fi; ",
    "if [ -f \"$F\" ]; then set -- $(ls -ln \"$F\"); [ \"$3\" = \"$(id -u)\" ] || f unexpected_target binary_owner; [ \"$(s \"$F\")\" = \"@PREV@\" ] || f changed; fi; ",
    "cat > \"$T\" || f no_space write; ",
    "set -- $(wc -c < \"$T\"); [ \"$1\" = \"@SIZE@\" ] || f transfer_incomplete \"$1\"; ",
    "S=$(s \"$T\"); [ -n \"$S\" ] || f checksum_tool_missing; [ \"$S\" = \"@SHA@\" ] || f checksum_mismatch \"$S\"; ",
    "chmod 0755 \"$T\" || f permission_denied chmod; ",
    "V=$(\"$T\" --version 2>&1); C=$?; ",
    "[ \"$C\" = 0 ] && [ \"$V\" = \"biorouter-crew @VER@\" ] || f staged_version \"$C\" \"$(printf %s \"$V\" | head -c 400 | h)\"; ",
    "sync \"$T\" 2>/dev/null; ",
    "K=0; if [ -f \"$F\" ]; then rm -f \"$R\"; ln \"$F\" \"$R\" 2>/dev/null || cp -p \"$F\" \"$R\" || f permission_denied previous; K=1; fi; ",
    "mv -f \"$T\" \"$F\" || f permission_denied rename; ",
    "sync \"$B\" 2>/dev/null; ",
    "V=$(\"$F\" --version 2>&1); C=$?; ",
    "echo \"$P installed $C $(printf %s \"$V\" | head -c 400 | h)\"; echo \"$P previous $K\"; echo \"$P done\""
);

/// Make the workspace's work folder, `~/crew-work/@SLUG@`, private when it is new, and say
/// where it really is.
const FOLDER_SCRIPT: &str = concat!(
    "set -f; P=BRCREW_FOLDER; ",
    "h() { od -An -v -tx1 | tr -d \" \\n\"; }; ",
    "f() { echo \"$P fail $*\"; exit 3; }; ",
    "umask 077; D=\"$HOME/crew-work/@SLUG@\"; ",
    "mkdir -p \"$D\" || f permission_denied; cd \"$D\" || f permission_denied; ",
    "set -- $(ls -ldn .); [ \"$3\" = \"$(id -u)\" ] || f foreign_owner; ",
    "echo \"$P path_hex $(printf %s \"$(pwd -P)\" | h)\"; echo \"$P done\""
);

/// The server facts the context document is drafted from (OW-8).
const CONTEXT_SCRIPT: &str = "\"$HOME/.local/bin/biorouter-crew\" probe --output-format json";

/// The one remote argument every step sends: `script`, handed to `/bin/sh` whatever the
/// account's login shell is.
fn remote(script: &str) -> String {
    debug_assert!(
        !script.contains(['\'', '!', '\n', '@']) && !script.contains("\\\\"),
        "a setup script must survive every login shell's single quotes"
    );
    format!("exec /bin/sh -c '{script}'")
}

// ---------------------------------------------------------------------------------------------
// The wire
// ---------------------------------------------------------------------------------------------

/// What to set up (`POST /crew/setup`).
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    pub target: SetupTarget,
    pub intent: SetupIntent,
    /// The workspace's name (`lab`, `chen-lab`), as the broker's rule allows it. A host's job
    /// needs it; a member's uses it for the work folder when given.
    #[serde(default)]
    pub workspace_name: Option<String>,
    /// Replace a Crew program in the account that is older than this app's, or that does not
    /// run. The one replaced stays as `biorouter-crew.previous`. A newer one is never replaced;
    /// one of this app's version that is another build of it (another SHA-256) is replaced
    /// whether or not this is set.
    #[serde(default)]
    pub replace_existing: bool,
    /// What a member's agents may do on the server once the job connects (design OW-6), as the
    /// person chose it: `off`, `folder` or `full`. Absent leaves the connection's agent access
    /// exactly as it is. Only a member's job reads it.
    #[serde(default)]
    #[schema(inline)]
    pub agent_access: Option<SetupAgentAccess>,
    /// Take write access for other accounts away (`chmod go-w`) from the folders the job finds
    /// above where the workspace is kept, which the account owns, when Crew would otherwise
    /// refuse to start there (`crew_host_storage_unsafe`). Only a host's job reads it.
    #[serde(default)]
    pub fix_storage_permissions: bool,
}

/// A member's agent access, as setup is asked to set it (design OW-6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupAgentAccess {
    /// No work folder: the job changes nothing and makes no folder.
    Off,
    /// The work folder, confined, with commands where the server can confine them.
    Folder,
    /// Run as the person on the server, starting in the work folder. Where the server or the
    /// bridge does not take full access, the job sets the work folder instead, and says so.
    Full,
}

/// Which server, and which account on it.
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SetupTarget {
    /// A saved connection: its login, route and control path.
    Connection { connection_id: String },
    /// This computer's pending host setup (`POST /crew/devices/prepare`), with the login and
    /// route the person typed. Its sign-in is kept for the connection the setup becomes.
    Pending {
        preparation_id: String,
        ssh_target: String,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        identity_file: Option<String>,
        #[serde(default)]
        proxy_jump: Option<String>,
    },
}

/// What the job is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupIntent {
    /// Check the server and the account's Crew program; change nothing.
    Check,
    /// Make sure this app's Crew program (or a newer one) is installed in the account.
    Install,
    /// Install, then start the workspace (a `pending` target).
    Host,
    /// Install, then connect (a `connection` target). Never starts a workspace.
    Member,
}

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    Running,
    /// The server asks for a password or a code: open the sign-in with
    /// `POST /crew/setup/{job_id}/authentication`.
    NeedsSignIn,
    Finished,
    Failed,
    Cancelled,
}

/// A step of a job, in the order a job takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupStageName {
    /// Checking the server and the account's Crew program.
    Check,
    /// Waiting for the person to sign in.
    SignIn,
    /// Installing Crew in the account.
    Install,
    /// Starting the workspace (a host).
    Start,
    /// Connecting to the workspace (a member).
    Connect,
    /// Making the workspace's work folder, `~/crew-work/<workspace>`.
    WorkFolder,
    /// Reading the server's facts for the workspace's context document.
    Context,
    /// Ready.
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupStageState {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupStage {
    pub name: SetupStageName,
    pub state: SetupStageState,
    /// A few words on what the step found or why it was skipped, for a person.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What the server said about itself. Each field is `null` until the check has read it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupServer {
    /// The SSH login the job uses.
    pub login: String,
    pub uid: Option<u32>,
    pub user: Option<String>,
    pub home: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub kernel: Option<String>,
    pub glibc: Option<String>,
    /// Free space in the account's home folder, in KiB.
    pub free_kib: Option<u64>,
    /// The tool the server checks a SHA-256 with: `sha256sum`, `shasum` or `openssl`.
    pub checksum_tool: Option<String>,
    /// Folders above where a host's workspace is kept (`~/.local/share/biorouter-crew`), as
    /// their real paths, that this account owns and other accounts may write to (group or other
    /// write, no sticky bit). Crew won't start a workspace under them
    /// (`crew_host_storage_unsafe`) until `chmod go-w` takes that away.
    pub storage_unsafe: Vec<String>,
}

/// The account's Crew program, as the check found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupInstalled {
    /// Always `~/.local/bin/biorouter-crew`.
    pub path: String,
    /// `missing`, `file`, `symlink`, `dir` or `other`.
    pub state: String,
    pub version: Option<String>,
    pub sha256: Option<String>,
    /// Whether `--version` ran and named a version; `null` when there is no file.
    pub runnable: Option<bool>,
    /// Why it cannot be used or replaced, for a person.
    pub problem: Option<String>,
}

/// The program this daemon installs (`GET /crew/setup/artifact`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupArtifact {
    /// What its `--version` prints after `biorouter-crew `.
    pub version: String,
    /// The platform it is built for: `x86_64-unknown-linux-gnu`.
    pub target: String,
    /// Its SHA-256, as 64 lowercase hex digits.
    pub sha256: String,
    pub size: u64,
    /// `bundled` (part of this app) or `dev` (a development copy from
    /// `BIOROUTER_CREW_BROKER_DIR`).
    pub source: String,
}

/// What a job did, once it is done.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupOutcome {
    /// The Crew program in the account now.
    pub installed_version: Option<String>,
    /// The program already there was kept: the same version as this app's, a newer one, or an
    /// older one that was not to be replaced.
    pub reused: bool,
    /// The program replaced stays as `~/.local/bin/biorouter-crew.previous`.
    pub previous_kept: bool,
    /// The program this job replaced, and the one it put in its place: an older version, or
    /// another build of this app's version (another SHA-256). `null` when nothing was replaced.
    #[schema(inline)]
    pub upgraded_from: Option<SetupUpgrade>,
    /// A host's start, read exactly as `POST /crew/host/start` reads it.
    pub host: Option<StartOutput>,
    /// The start command that ran, character for character what the Host dialog shows.
    pub host_command: Option<String>,
    /// A member's connection, connected and verified.
    pub connection_id: Option<String>,
    /// The workspace's work folder on the server, as its canonical path.
    pub work_folder: Option<String>,
    /// What this job set a member's connection's agent access to, as the request's
    /// `agent_access` asked: `full`, or `folder` (asked for, or where full access is not
    /// available). `null` when the job changed nothing.
    pub remote_access: Option<RemoteAccess>,
    /// The first context document, drafted from the server's facts (Markdown).
    pub context_draft: Option<String>,
}

/// A Crew program a job replaced ([`SetupOutcome::upgraded_from`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupUpgrade {
    /// The version the replaced program named, when it named one.
    pub version: Option<String>,
    /// The replaced program's SHA-256, as 64 lowercase hex digits.
    pub sha256: Option<String>,
    /// The version now installed.
    pub installed_version: String,
    /// The SHA-256 of the program now installed.
    pub installed_sha256: String,
}

/// Why a job stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct SetupError {
    /// The SSH codes (`crew_ssh_auth_required`, `crew_ssh_key_refused`,
    /// `crew_ssh_host_key_unknown`, `crew_ssh_host_key_changed`, `crew_ssh_unreachable`,
    /// `crew_bridge_missing`, `crew_broker_not_running`, `crew_ssh_failed`), a connect's
    /// (`crew_workspace_identity_mismatch`, `crew_request_refused`), or one of setup's own:
    /// `crew_setup_cancelled`, `crew_setup_timed_out`, `crew_setup_sign_in_expired`,
    /// `crew_setup_server_unreadable` (the server's answer could not be read),
    /// `crew_setup_start_failed` (Crew did not start the workspace; see `outcome.host`, and
    /// `detail` for what Crew printed), `crew_host_storage_unsafe` (other accounts can write to
    /// a folder above where the workspace is kept; `detail` lists them one per line, and
    /// `fix_storage_permissions` lets setup fix the ones the account owns),
    /// `crew_setup_changed` (the connection's login or route changed while the job ran),
    /// `crew_install_artifact_unavailable`, `crew_install_unsupported_platform`,
    /// `crew_install_dependency_missing`, `crew_install_incompatible` (the program there does
    /// not run or name its version), `crew_install_unexpected_target` (a link, a folder or
    /// another account's file where the program goes), `crew_install_checksum_tool_missing`,
    /// `crew_install_no_space`, `crew_install_permission_denied`, `crew_install_busy` (another
    /// install in the account), `crew_install_transfer_incomplete`,
    /// `crew_install_checksum_mismatch` (nothing was replaced) or
    /// `crew_install_outcome_unknown` (the connection dropped after the transfer; set up again
    /// to check).
    pub code: String,
    /// One plain sentence for a person.
    pub message: String,
    /// The server's or OpenSSH's own bounded words, for Copy details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The SSH hop a connect failure concerns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// A job, as `GET /crew/setup/{job_id}` answers it.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SetupStatus {
    pub job_id: String,
    pub intent: SetupIntent,
    pub state: SetupState,
    /// The step under way, or the one the job stopped at.
    pub stage: SetupStageName,
    pub stages: Vec<SetupStage>,
    pub server: SetupServer,
    pub installed: Option<SetupInstalled>,
    pub artifact: Option<SetupArtifact>,
    pub outcome: Option<SetupOutcome>,
    pub error: Option<SetupError>,
}

/// A refusal to start, read or sign in to a job, with the HTTP shape the route answers.
#[derive(Debug)]
pub struct SetupRefused {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for SetupRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SetupRefused {}

fn refused(status: u16, code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(SetupRefused {
        status,
        code,
        message: message.into(),
    })
}

/// A host-start refusal (the route and host-setup checks setup shares) as a setup refusal.
fn as_setup(error: anyhow::Error) -> anyhow::Error {
    match error.downcast::<host_start::HostStartRefused>() {
        Ok(refusal) => refused(refusal.status, refusal.code, refusal.message),
        Err(error) => error,
    }
}

/// A connection's server facts and the context document drafted from them
/// (`POST /crew/connections/{id}/context/draft`).
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ContextDraft {
    /// The draft, in Markdown ([`super::context::render_workspace_context`]).
    pub draft: String,
    /// The facts, exactly as `biorouter-crew probe --output-format json` answered them.
    #[schema(value_type = Object)]
    pub facts: Value,
}

// ---------------------------------------------------------------------------------------------
// The artifact
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    name: String,
    version: String,
    target: String,
    #[serde(default)]
    glibc_max: Option<String>,
    sha256: String,
    size: u64,
    /// `dev` lets a development copy carry another version than this daemon's.
    #[serde(default)]
    source: Option<String>,
}

/// The program to send, checked.
struct Artifact {
    info: SetupArtifact,
    glibc_max: String,
    bytes: Vec<u8>,
}

/// Where a copy may be, in the order they are looked for.
#[derive(Debug, PartialEq, Eq)]
enum Place {
    /// `BIOROUTER_CREW_BROKER_DIR`: a manifest is optional, and may name another version.
    Dev(PathBuf),
    /// Beside `biorouterd`: a manifest is required, naming this daemon's version.
    Bundled(PathBuf),
    /// `biorouter-crew` itself beside `biorouterd`, on a Linux x86-64 daemon (the CLI
    /// packages install both in one folder).
    Beside(PathBuf),
}

fn places(dev: Option<PathBuf>, exe_dir: Option<&Path>) -> Vec<Place> {
    let mut places: Vec<Place> = dev.into_iter().map(Place::Dev).collect();
    if let Some(dir) = exe_dir {
        places.push(Place::Bundled(dir.join(BUNDLE_DIR)));
        places.push(Place::Bundled(
            dir.join("../libexec/biorouter").join(BUNDLE_DIR),
        ));
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            places.push(Place::Beside(dir.join("biorouter-crew")));
        }
    }
    places
}

fn unavailable(why: impl std::fmt::Display) -> anyhow::Error {
    refused(
        409,
        "crew_install_artifact_unavailable",
        format!(
            "This copy of Biorouter has no Crew server program to install ({why}). Install Crew in your account yourself, or use a full Biorouter download."
        ),
    )
}

/// Whether `bytes` start as a 64-bit little-endian x86-64 ELF file.
fn x86_64_elf(bytes: &[u8]) -> bool {
    bytes.len() >= 20
        && bytes[..4] == [0x7f, b'E', b'L', b'F']
        && bytes[4] == 2
        && bytes[5] == 1
        && u16::from_le_bytes([bytes[18], bytes[19]]) == 0x3e
}

fn lower_hex_digest(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The copy at `place`, checked; `Ok(None)` when there is none there.
fn artifact_at(place: &Place) -> Result<Option<Artifact>> {
    let (binary, manifest, source) = match place {
        Place::Dev(dir) => (
            dir.join("biorouter-crew"),
            Some(dir.join("manifest.json")),
            "dev",
        ),
        Place::Bundled(dir) => (
            dir.join("biorouter-crew"),
            Some(dir.join("manifest.json")),
            "bundled",
        ),
        Place::Beside(binary) => (binary.clone(), None, "bundled"),
    };
    let metadata = match std::fs::metadata(&binary) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unavailable(format!("{}: {error}", binary.display()))),
    };
    if !metadata.is_file() || metadata.len() > MAX_ARTIFACT {
        return Err(unavailable(format!(
            "{} is not a program file of at most 64 MiB",
            binary.display()
        )));
    }
    let bytes = std::fs::read(&binary)
        .map_err(|error| unavailable(format!("{}: {error}", binary.display())))?;
    if !x86_64_elf(&bytes) {
        return Err(unavailable(format!(
            "{} is not an x86-64 Linux program",
            binary.display()
        )));
    }
    let sha256 = super::hex(&Sha256::digest(&bytes));
    let size = bytes.len() as u64;
    let manifest = match manifest {
        Some(path) => match std::fs::read(&path) {
            Ok(text) => Some(
                serde_json::from_slice::<Manifest>(&text)
                    .map_err(|error| unavailable(format!("{}: {error}", path.display())))?,
            ),
            // A development folder may hold the program alone.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && matches!(place, Place::Dev(_)) =>
            {
                None
            }
            Err(error) => return Err(unavailable(format!("{}: {error}", path.display()))),
        },
        None => None,
    };
    let daemon_version = env!("CARGO_PKG_VERSION");
    let (version, glibc_max) = match manifest {
        Some(manifest) => {
            let named = |what: &str| unavailable(format!("its manifest {what}"));
            if manifest.schema != 1 || manifest.name != "biorouter-crew" {
                return Err(named("is not a Crew server program's manifest"));
            }
            if manifest.target != BROKER_TARGET {
                return Err(named(&format!(
                    "names {}, not {BROKER_TARGET}",
                    manifest.target
                )));
            }
            if !lower_hex_digest(&manifest.sha256, 64) || manifest.sha256 != sha256 {
                return Err(named("names another SHA-256 than the program's"));
            }
            if manifest.size != size {
                return Err(named("names another size than the program's"));
            }
            if version_of(&manifest.version).is_none() {
                return Err(named("names no version"));
            }
            let may_differ =
                matches!(place, Place::Dev(_)) && manifest.source.as_deref() == Some("dev");
            if manifest.version != daemon_version && !may_differ {
                return Err(named(&format!(
                    "names version {}, and this Biorouter is {daemon_version}",
                    manifest.version
                )));
            }
            let glibc_max = manifest
                .glibc_max
                .filter(|glibc| glibc_version(glibc).is_some())
                .unwrap_or_else(|| GLIBC_FLOOR.to_owned());
            (manifest.version, glibc_max)
        }
        None => (daemon_version.to_owned(), GLIBC_FLOOR.to_owned()),
    };
    Ok(Some(Artifact {
        info: SetupArtifact {
            version,
            target: BROKER_TARGET.into(),
            sha256,
            size,
            source: source.into(),
        },
        glibc_max,
        bytes,
    }))
}

/// The first copy found, in the order of [`places`]. A copy that is there but does not check
/// out is the answer: a broken development copy never falls through to another.
fn find_artifact(places: &[Place]) -> Result<Artifact> {
    for place in places {
        if let Some(artifact) = artifact_at(place)? {
            return Ok(artifact);
        }
    }
    Err(unavailable("none was found beside biorouterd"))
}

#[cfg(test)]
static TEST_ARTIFACT: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// Send `bytes` as this daemon's program, at its version, in tests that run it on this
/// computer (a script, which no ELF check would admit).
#[cfg(test)]
pub(super) fn use_test_artifact(bytes: Vec<u8>) {
    *TEST_ARTIFACT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(bytes);
}

fn load_artifact() -> Result<Artifact> {
    #[cfg(test)]
    if let Some(bytes) = TEST_ARTIFACT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    {
        return Ok(Artifact {
            info: SetupArtifact {
                version: env!("CARGO_PKG_VERSION").into(),
                target: BROKER_TARGET.into(),
                sha256: super::hex(&Sha256::digest(&bytes)),
                size: bytes.len() as u64,
                source: "dev".into(),
            },
            glibc_max: GLIBC_FLOOR.into(),
            bytes,
        });
    }
    let dev = std::env::var_os(BROKER_DIR_ENV)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let exe = std::env::current_exe().ok();
    let exe = exe.map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe));
    find_artifact(&places(dev, exe.as_deref().and_then(Path::parent)))
}

/// The program this daemon would install, checked (`GET /crew/setup/artifact`).
pub fn setup_artifact() -> Result<SetupArtifact> {
    load_artifact().map(|artifact| artifact.info)
}

// ---------------------------------------------------------------------------------------------
// Reading what the server printed
// ---------------------------------------------------------------------------------------------

type Version = (u64, u64, u64);

/// `X.Y.Z`, numbers only.
fn version_of(text: &str) -> Option<Version> {
    let mut parts = text.split('.');
    let number = |part: Option<&str>| {
        part.filter(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))?
            .parse()
            .ok()
    };
    let version = (
        number(parts.next())?,
        number(parts.next())?,
        number(parts.next())?,
    );
    parts.next().is_none().then_some(version)
}

/// `X.Y` (or more parts), numbers only, as a glibc version reads.
fn glibc_version(text: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = text
        .split('.')
        .map(|p| {
            (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse().ok())
                .flatten()
        })
        .collect();
    parts.filter(|parts| parts.len() >= 2 && parts.len() <= 4)
}

/// The version `biorouter-crew --version` names: `biorouter-crew X.Y.Z` on its first line.
fn broker_version(output: &str) -> Option<String> {
    let version = output
        .lines()
        .next()?
        .trim()
        .strip_prefix("biorouter-crew ")?;
    version_of(version).map(|_| version.to_owned())
}

/// Text the server hex-encoded, decoded and made safe to show: at most `limit` bytes.
fn unhex_text(value: &str, limit: usize) -> Option<String> {
    if !value.len().is_multiple_of(2) || value.len() > limit * 2 {
        return None;
    }
    let bytes = super::unhex(value).ok()?;
    let text = host_start::shown(&bytes);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// A word the server printed: letters, digits and `._-`, at most 64 of them.
fn word(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
    .then(|| value.to_owned())
}

/// What the probe found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Probe {
    uid: Option<u32>,
    user: Option<String>,
    home: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    kernel: Option<String>,
    glibc: Option<String>,
    tools: Vec<String>,
    free_kib: Option<u64>,
    bin_dir: Option<String>,
    binary: Option<String>,
    binary_owner: Option<u32>,
    binary_sha256: Option<String>,
    binary_exit: Option<i32>,
    binary_output: Option<String>,
    storage_unsafe: Vec<String>,
}

const KINDS: [&str; 5] = ["missing", "file", "symlink", "dir", "other"];
/// At most this many folders are read from one storage walk (there are as many as the path
/// has parts).
const MAX_STORAGE_FOLDERS: usize = 32;

/// A folder the storage walk named: an absolute path, hex-encoded on the server.
fn walked_folder(value: &str) -> Option<String> {
    unhex_text(value, 4096).filter(|path| path.starts_with('/'))
}

/// The probe's `BRCREW_PROBE` lines, read strictly; every other line (a login banner, a
/// `.bashrc` that prints) is ignored. `None` unless the probe both began and ended.
fn read_probe(stdout: &[u8]) -> Option<Probe> {
    let text = String::from_utf8_lossy(stdout);
    let mut probe = Probe::default();
    let (mut begun, mut ended) = (false, false);
    for line in text.lines() {
        let Some(rest) = line.trim_end().strip_prefix("BRCREW_PROBE ") else {
            continue;
        };
        let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
        let value = value.trim();
        match key {
            "begin" => begun = value == "1",
            "end" => ended = begun,
            _ if !begun || ended => {}
            "uid" => probe.uid = value.parse().ok(),
            "user_hex" => probe.user = unhex_text(value, 200),
            "home_hex" => probe.home = unhex_text(value, 4096).filter(|home| home.starts_with('/')),
            "os" => probe.os = word(value),
            "arch" => probe.arch = word(value),
            "kernel_hex" => probe.kernel = unhex_text(value, 200),
            "glibc" => probe.glibc = glibc_version(value).map(|_| value.to_owned()),
            "tool" => {
                if ["sha256sum", "shasum", "openssl", "timeout"].contains(&value) {
                    probe.tools.push(value.to_owned());
                }
            }
            "free_kib" => probe.free_kib = value.parse().ok(),
            "bin_dir" => probe.bin_dir = KINDS.contains(&value).then(|| value.to_owned()),
            "binary" => probe.binary = KINDS.contains(&value).then(|| value.to_owned()),
            "binary_owner" => probe.binary_owner = value.parse().ok(),
            "binary_sha256" => {
                probe.binary_sha256 = lower_hex_digest(value, 64).then(|| value.to_owned())
            }
            "binary_exit" => probe.binary_exit = value.parse().ok(),
            "binary_version_hex" => probe.binary_output = unhex_text(value, 400),
            "storage_unsafe_hex" => {
                if let Some(folder) = walked_folder(value) {
                    if probe.storage_unsafe.len() < MAX_STORAGE_FOLDERS
                        && !probe.storage_unsafe.contains(&folder)
                    {
                        probe.storage_unsafe.push(folder);
                    }
                }
            }
            _ => {}
        }
    }
    (begun && ended).then_some(probe)
}

impl Probe {
    fn checksum_tool(&self) -> Option<&str> {
        ["sha256sum", "shasum", "openssl"]
            .into_iter()
            .find(|tool| self.tools.iter().any(|found| found == tool))
    }
    /// The installed program's version, when it ran and named one.
    fn installed_version(&self) -> Option<String> {
        (self.binary_exit == Some(0))
            .then(|| self.binary_output.as_deref().and_then(broker_version))
            .flatten()
    }
}

/// What the install script reported.
#[derive(Debug, PartialEq, Eq)]
enum Installed {
    /// It finished: `--version`'s exit code and output from the program now in place, and
    /// whether the one it replaced was kept.
    Done {
        exit: Option<i32>,
        output: Option<String>,
        previous: bool,
    },
    /// It stopped, before anything was replaced (or, for `rename`, without replacing).
    Failed { reason: String, rest: Vec<String> },
    /// It said neither: the connection may have dropped.
    Unknown,
}

fn read_install(stdout: &[u8]) -> Installed {
    let text = String::from_utf8_lossy(stdout);
    let (mut exit, mut output, mut previous, mut done) = (None, None, false, false);
    for line in text.lines() {
        let Some(rest) = line.trim_end().strip_prefix("BRCREW_INSTALL ") else {
            continue;
        };
        let mut words = rest.split(' ').filter(|w| !w.is_empty());
        match words.next() {
            Some("fail") => {
                let reason = words.next().and_then(word).unwrap_or_default();
                let rest = words.take(4).map(str::to_owned).collect();
                return Installed::Failed { reason, rest };
            }
            Some("installed") => {
                exit = words.next().and_then(|code| code.parse().ok());
                output = words.next().and_then(|hex| unhex_text(hex, 400));
            }
            Some("previous") => previous = words.next() == Some("1"),
            Some("done") => done = true,
            _ => {}
        }
    }
    if done {
        Installed::Done {
            exit,
            output,
            previous,
        }
    } else {
        Installed::Unknown
    }
}

/// What the storage fix changed, and what the walk still finds after it.
#[derive(Debug, Default, PartialEq, Eq)]
struct StorageFix {
    fixed: Vec<String>,
    left: Vec<String>,
}

/// The storage fix's `BRCREW_STORAGE` lines; `None` unless it said it was done.
fn read_storage_fix(stdout: &[u8]) -> Option<StorageFix> {
    let text = String::from_utf8_lossy(stdout);
    let mut fix = StorageFix::default();
    for line in text.lines() {
        let Some(rest) = line.trim_end().strip_prefix("BRCREW_STORAGE ") else {
            continue;
        };
        let mut words = rest.split(' ');
        let list = match words.next() {
            Some("fixed") => &mut fix.fixed,
            Some("unsafe") => &mut fix.left,
            Some("done") => return Some(fix),
            _ => continue,
        };
        if let Some(folder) = words.next().and_then(walked_folder) {
            if list.len() < MAX_STORAGE_FOLDERS && !list.contains(&folder) {
                list.push(folder);
            }
        }
    }
    None
}

/// A path as a POSIX shell reads it back as one word: as it is when it needs no quoting, else
/// in single quotes.
fn shell_word(path: &str) -> String {
    if !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+,:%=".contains(&b))
    {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// What a start printed, for Copy details: its standard output and error, line by line, each
/// without control characters, at most 40 lines and 2000 characters, blank lines dropped.
fn printed(stdout: &[u8], stderr: &str) -> Option<String> {
    let stdout = host_start::shown(stdout);
    let lines: Vec<String> = stdout
        .lines()
        .chain(stderr.lines())
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .take(40)
        .collect();
    detail(&lines.join("\n"))
}

/// The work folder's canonical path, or why it could not be made.
fn read_folder(stdout: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(stdout);
    let mut path = None;
    for line in text.lines() {
        let Some(rest) = line.trim_end().strip_prefix("BRCREW_FOLDER ") else {
            continue;
        };
        let mut words = rest.split(' ');
        match words.next() {
            Some("fail") => return Err(words.next().and_then(word).unwrap_or_default()),
            Some("path_hex") => {
                path = words
                    .next()
                    .and_then(|hex| unhex_text(hex, 4096))
                    .filter(|path| path.starts_with('/') && path != "/");
            }
            Some("done") => return path.ok_or_else(|| "unreadable".to_owned()),
            _ => {}
        }
    }
    Err("unreadable".into())
}

/// The context probe's JSON: one object, the first line that is one.
fn read_facts(stdout: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim();
    if let Ok(value @ Value::Object(_)) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(Value::is_object)
}

/// The server as a person calls it in a sentence: the host of `user@host`, or the alias.
fn server_name(ssh_target: &str) -> &str {
    match ssh_target.rsplit_once('@') {
        Some((_, host)) if !host.is_empty() => host,
        _ => ssh_target,
    }
}

/// OpenSSH's or the server's own words, bounded, for Copy details.
fn detail(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(2000).collect())
}

// ---------------------------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------------------------

/// The login and route a job reaches its server by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Route {
    ssh_target: String,
    port: Option<u16>,
    identity_file: Option<String>,
    proxy_jump: Option<String>,
}

impl Route {
    fn of(connection: &Connection) -> Self {
        Self {
            ssh_target: connection.ssh_target.clone(),
            port: connection.port,
            identity_file: connection.identity_file.clone(),
            proxy_jump: connection.proxy_jump.clone(),
        }
    }
    /// What a setup sign-in is bound to: the job's route, so a sign-in never carries another.
    fn digest(&self) -> Vec<u8> {
        Sha256::digest(serde_json::to_vec(self).unwrap_or_default()).to_vec()
    }
}

/// The route digest a setup sign-in for `connection`'s login and route is bound to, so a kept
/// setup master is let go when the connection that would ride it names another route (C12).
pub(super) fn route_digest(connection: &Connection) -> Vec<u8> {
    Route::of(connection).digest()
}

/// Why a job stopped.
struct Stop {
    state: SetupState,
    error: SetupError,
}

#[derive(Clone)]
struct Progress {
    state: SetupState,
    stage: SetupStageName,
    stages: Vec<SetupStage>,
    server: SetupServer,
    installed: Option<SetupInstalled>,
    artifact: Option<SetupArtifact>,
    outcome: Option<SetupOutcome>,
    error: Option<SetupError>,
}

struct Job {
    /// The SSH control key: the connection's ID, or the pending preparation's (the ID the
    /// connection it becomes will have).
    key: String,
    intent: SetupIntent,
    route: Route,
    control: PathBuf,
    manager: Arc<CrewManager>,
    /// A host's hosting key, from this daemon's own pending setup.
    bootstrap_key: Option<String>,
    workspace_name: Option<String>,
    replace_existing: bool,
    /// The agent access a member's job sets, as the person chose it; `None` leaves it as is.
    agent_access: Option<SetupAgentAccess>,
    /// A host's job may `chmod go-w` the account's own folders above the workspace.
    fix_storage_permissions: bool,
    connection_id: Option<String>,
    progress: Mutex<Progress>,
    finished_at: Mutex<Option<Instant>>,
    cancel: CancellationToken,
    signed_in: tokio::sync::Notify,
}

static JOBS: LazyLock<Mutex<HashMap<String, Arc<Job>>>> = LazyLock::new(Default::default);
/// Starts jobs one at a time, so two clicks can never both start one for one server.
static STARTING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Job>>> {
    let mut jobs = JOBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    jobs.retain(|_, job| {
        job.finished_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none_or(|at| at.elapsed() < KEEP_FINISHED)
    });
    jobs
}

fn job(job_id: &str) -> Option<Arc<Job>> {
    jobs().get(job_id).cloned()
}

/// A job's status, or `None` for an unknown (or long finished) job.
pub fn setup_status(job_id: &str) -> Option<SetupStatus> {
    job(job_id).map(|job| job.status(job_id))
}

/// Stop a job. `false` for an unknown job; stopping a finished job changes nothing.
pub fn cancel_setup(job_id: &str) -> bool {
    match job(job_id) {
        Some(job) => {
            job.cancel.cancel();
            true
        }
        None => false,
    }
}

/// Open the sign-in a job waits for (`POST /crew/setup/{job_id}/authentication`). Its
/// terminal is the ordinary `GET /crew/authentication/{id}/terminal`.
pub async fn prepare_setup_sign_in(
    job_id: &str,
    request_id: &str,
    controller_id: &str,
    cols: u16,
    rows: u16,
) -> Result<authentication::AuthenticationSession> {
    let job = job(job_id).ok_or_else(not_found)?;
    if job.progress().state != SetupState::NeedsSignIn {
        return Err(refused(
            409,
            "crew_setup_not_waiting_for_sign_in",
            "This setup isn't waiting for you to sign in.",
        ));
    }
    let plan = job.sign_in_plan().await?;
    authentication::prepare_setup(
        &job.manager,
        &job.key,
        job_id,
        job.route.digest(),
        &job.route.ssh_target,
        plan,
        request_id,
        controller_id,
        cols,
        rows,
    )
    .await
}

fn not_found() -> anyhow::Error {
    refused(
        404,
        "crew_setup_not_found",
        "That setup is not on this computer any more. Start again.",
    )
}

/// The route a setup sign-in for `job_id` must still be bound to, while the job is known.
pub(super) fn sign_in_route(job_id: &str) -> Option<Vec<u8>> {
    job(job_id).map(|job| job.route.digest())
}

/// Whether `job_id` is known and waiting for its person to sign in. A sign-in left open for a
/// job that is not (it ended, or asked again for another sign-in) holds its server's key for
/// nothing, so a new one replaces it.
pub(super) fn waiting_for_sign_in(job_id: &str) -> bool {
    job(job_id).is_some_and(|job| job.progress().state == SetupState::NeedsSignIn)
}

/// The manager whose control paths and lifecycle a setup sign-in for `job_id` uses.
pub(super) fn sign_in_manager(job_id: &str) -> Option<Arc<CrewManager>> {
    job(job_id).map(|job| Arc::clone(&job.manager))
}

/// The sign-in a setup job's server is signed in to with, as it would be prepared now.
pub(super) async fn sign_in_plan(job_id: &str) -> Result<AuthenticationPlan> {
    job(job_id).ok_or_else(not_found)?.sign_in_plan().await
}

/// The person signed in for `job_id`: its master is up, adopted, at the job's control path.
pub(super) fn signed_in(job_id: &str) {
    if let Some(job) = job(job_id) {
        job.signed_in.notify_one();
    }
}

/// The steps a job of `intent` takes.
fn stages_for(intent: SetupIntent) -> Vec<SetupStage> {
    use SetupStageName as Name;
    let names: &[Name] = match intent {
        SetupIntent::Check => &[Name::Check, Name::SignIn, Name::Done],
        SetupIntent::Install => &[Name::Check, Name::SignIn, Name::Install, Name::Done],
        SetupIntent::Host => &[
            Name::Check,
            Name::SignIn,
            Name::Install,
            Name::Start,
            Name::WorkFolder,
            Name::Context,
            Name::Done,
        ],
        SetupIntent::Member => &[
            Name::Check,
            Name::SignIn,
            Name::Install,
            Name::Connect,
            Name::WorkFolder,
            Name::Context,
            Name::Done,
        ],
    };
    names
        .iter()
        .map(|name| SetupStage {
            name: *name,
            state: SetupStageState::Pending,
            detail: None,
        })
        .collect()
}

/// What the check decided to do about the account's program.
enum Plan {
    /// Keep the program there, at this version.
    Reuse(String),
    /// Send this app's copy; `previous` is the SHA-256 of the one it replaces, if any, and
    /// `from` the version that one named.
    Install {
        previous: Option<String>,
        from: Option<String>,
    },
}

impl Job {
    fn progress(&self) -> Progress {
        self.progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn update(&self, change: impl FnOnce(&mut Progress)) {
        change(
            &mut self
                .progress
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }
    fn status(&self, job_id: &str) -> SetupStatus {
        let progress = self.progress();
        SetupStatus {
            job_id: job_id.to_owned(),
            intent: self.intent,
            state: progress.state,
            stage: progress.stage,
            stages: progress.stages,
            server: progress.server,
            installed: progress.installed,
            artifact: progress.artifact,
            outcome: progress.outcome,
            error: progress.error,
        }
    }
    fn running(&self) -> bool {
        matches!(
            self.progress().state,
            SetupState::Running | SetupState::NeedsSignIn
        )
    }
    fn mark(&self, name: SetupStageName, state: SetupStageState, detail: Option<String>) {
        self.update(|progress| {
            if matches!(state, SetupStageState::Running) {
                progress.stage = name;
            }
            if let Some(stage) = progress.stages.iter_mut().find(|stage| stage.name == name) {
                stage.state = state;
                if detail.is_some() {
                    stage.detail = detail;
                }
            }
        });
    }
    fn begin(&self, name: SetupStageName) {
        self.mark(name, SetupStageState::Running, None);
    }
    fn done(&self, name: SetupStageName, detail: Option<String>) {
        self.mark(name, SetupStageState::Done, detail);
    }
    fn skip(&self, name: SetupStageName, detail: impl Into<String>) {
        self.mark(name, SetupStageState::Skipped, Some(detail.into()));
    }
    fn outcome(&self, change: impl FnOnce(&mut SetupOutcome)) {
        self.update(|progress| change(progress.outcome.get_or_insert_with(Default::default)));
    }
    fn server(&self) -> &str {
        server_name(&self.route.ssh_target)
    }
    fn stop(&self, code: &str, message: impl Into<String>, detail: Option<String>) -> Stop {
        Stop {
            state: SetupState::Failed,
            error: SetupError {
                code: code.into(),
                message: message.into(),
                detail,
                host: None,
            },
        }
    }
    fn cancelled(&self, sent: bool) -> Stop {
        let message = if sent {
            format!(
                "Stopped. Crew may have been installed on {}; set up again to check.",
                self.server()
            )
        } else {
            format!("Stopped. Nothing more will be done on {}.", self.server())
        };
        Stop {
            state: SetupState::Cancelled,
            ..self.stop("crew_setup_cancelled", message, None)
        }
    }
    /// The `ssh` arguments that run `remote` over the job's control path, never prompting.
    fn args(&self, remote: String) -> Vec<String> {
        let mut args = transport::login_args(
            self.route.port,
            self.route.identity_file.as_deref(),
            self.route.proxy_jump.as_deref(),
            Some(&self.control),
        );
        args.extend([
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ControlMaster=no".into(),
            "-o".into(),
            "ControlPersist=no".into(),
            self.route.ssh_target.clone(),
            remote,
        ]);
        args
    }
    /// The sign-in for this job's server: a master at its control path, as a connection's own
    /// sign-in is (`sign_in_args`), checked by the same preflight.
    async fn sign_in_plan(&self) -> Result<AuthenticationPlan> {
        let mut args = transport::login_args(
            self.route.port,
            self.route.identity_file.as_deref(),
            self.route.proxy_jump.as_deref(),
            Some(&self.control),
        );
        args.extend([
            "-M".into(),
            "-N".into(),
            "-o".into(),
            "ControlPersist=600".into(),
            "-o".into(),
            "BatchMode=no".into(),
            self.route.ssh_target.clone(),
        ]);
        ssh_policy::preflight(&args, &self.route.ssh_target).await?;
        Ok(AuthenticationPlan {
            program: "ssh".into(),
            args,
            connection_id: self.key.clone(),
            authentication_id: uuid::Uuid::new_v4().to_string(),
        })
    }
    async fn run(
        &self,
        script: String,
        input: Option<Vec<u8>>,
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Result<host_start::Fixed, Stop> {
        host_start::run_fixed(&self.args(remote(&script)), input, limit, cancel)
            .await
            .map_err(|error| {
                self.stop(
                    "crew_ssh_failed",
                    "Biorouter couldn't run ssh on this computer.",
                    detail(&format!("{error:#}")),
                )
            })
    }
    /// Why a finished run is a stop, if it is one: cancelled, too slow, or `ssh` itself
    /// (exit 255) could not run it.
    fn interrupted(&self, ran: &host_start::Fixed) -> Option<Stop> {
        if ran.cancelled {
            return Some(self.cancelled(ran.input_written));
        }
        if ran.timed_out {
            return Some(self.stop(
                "crew_setup_timed_out",
                format!(
                    "{} took too long to answer, so setup stopped. Try again.",
                    self.server()
                ),
                None,
            ));
        }
        (ran.exit_code == Some(255)).then(|| self.ssh_stop(ran))
    }
    fn ssh_stop(&self, ran: &host_start::Fixed) -> Stop {
        let kind = transport::classify_exit(ran.exit_code, &ran.stderr);
        self.stop(
            kind.api_code(),
            host_start::ssh_sentence(kind, &ran.stderr, &self.route.ssh_target),
            detail(&ran.stderr),
        )
    }

    async fn drive(&self) -> Result<(), Stop> {
        use SetupStageName as Name;
        self.begin(Name::Check);
        let probe = self.probe().await?;
        self.note(&probe);
        let artifact = load_artifact();
        if let Ok(artifact) = &artifact {
            self.update(|progress| progress.artifact = Some(artifact.info.clone()));
        }
        let plan = self.plan(&probe, artifact.as_ref().map_err(|error| error.to_string()));
        let plan = match plan {
            Ok(plan) => plan,
            Err(stop) => {
                self.update(|progress| {
                    if let Some(installed) = &mut progress.installed {
                        installed.problem = Some(stop.error.message.clone());
                    }
                });
                return Err(stop);
            }
        };
        let ours = artifact.as_ref().ok().map(|a| a.info.version.clone());
        let decided = match &plan {
            Plan::Reuse(version) => format!("Crew {version} is installed in your account."),
            Plan::Install { previous: None, .. } => {
                "Crew isn't installed in your account yet.".into()
            }
            Plan::Install {
                previous: Some(_),
                from: Some(from),
            } if Some(from) == ours.as_ref() => format!(
                "Crew {from} in your account is another build than this app's, so it will be replaced."
            ),
            Plan::Install {
                previous: Some(_), ..
            } => "Crew in your account will be replaced with this app's.".into(),
        };
        self.done(Name::Check, Some(decided));
        if self.intent == SetupIntent::Check {
            self.mark_unreached();
            return Ok(());
        }
        let installed_version = match plan {
            Plan::Reuse(version) => {
                let detail = match artifact.as_ref().ok().map(|a| &a.info.version) {
                    Some(ours) if version_of(&version) < version_of(ours) => {
                        format!("Kept Crew {version}; this app has {ours}.")
                    }
                    _ => format!("Kept Crew {version}."),
                };
                self.skip(Name::Install, detail);
                self.outcome(|outcome| {
                    outcome.reused = true;
                    outcome.installed_version = Some(version.clone());
                });
                version
            }
            Plan::Install { previous, from } => {
                self.begin(Name::Install);
                // `plan` only installs with a copy in hand.
                let artifact = artifact.map_err(|error| {
                    self.stop("crew_install_artifact_unavailable", error.to_string(), None)
                })?;
                let replaced = previous.is_some();
                let version = self.install(&artifact, previous, from.clone()).await?;
                let said = match from {
                    Some(from) if replaced && from == version => {
                        format!("Replaced Crew {version} with this app's build of it.")
                    }
                    Some(from) if replaced => format!("Updated Crew from {from} to {version}."),
                    _ if replaced => format!("Replaced Crew in your account with {version}."),
                    _ => format!("Installed Crew {version}."),
                };
                self.done(Name::Install, Some(said));
                version
            }
        };
        if self.cancel.is_cancelled() {
            return Err(self.cancelled(false));
        }
        match self.intent {
            SetupIntent::Host => self.start().await?,
            SetupIntent::Member => self.connect().await?,
            SetupIntent::Check | SetupIntent::Install => {}
        }
        if matches!(self.intent, SetupIntent::Host | SetupIntent::Member) {
            self.work_folder(&installed_version).await?;
            self.context().await?;
        }
        Ok(())
    }

    /// Probe the server, waiting once for the person to sign in when it asks for a password.
    async fn probe(&self) -> Result<Probe, Stop> {
        let mut signed_in = false;
        loop {
            let ran = self
                .run(PROBE_SCRIPT.into(), None, PROBE_LIMIT, &self.cancel)
                .await?;
            if ran.exit_code == Some(255)
                && !ran.cancelled
                && !ran.timed_out
                && !signed_in
                && transport::classify_exit(ran.exit_code, &ran.stderr)
                    == SshFailureKind::AuthRequired
            {
                self.wait_for_sign_in().await?;
                signed_in = true;
                continue;
            }
            if let Some(stop) = self.interrupted(&ran) {
                return Err(stop);
            }
            return read_probe(&ran.stdout).ok_or_else(|| self.unreadable(&ran));
        }
    }

    fn unreadable(&self, ran: &host_start::Fixed) -> Stop {
        self.stop(
            "crew_setup_server_unreadable",
            format!(
                "Biorouter couldn't read what {} said about itself. Check that you can run commands there over SSH, then try again.",
                self.server()
            ),
            detail(&format!(
                "{}\n{}",
                host_start::shown(&ran.stdout),
                ran.stderr
            )),
        )
    }

    async fn wait_for_sign_in(&self) -> Result<(), Stop> {
        self.begin(SetupStageName::SignIn);
        self.update(|progress| progress.state = SetupState::NeedsSignIn);
        let outcome = tokio::select! {
            () = self.signed_in.notified() => Ok(()),
            () = self.cancel.cancelled() => Err(self.cancelled(false)),
            () = tokio::time::sleep(SIGN_IN_WAIT) => Err(self.stop(
                "crew_setup_sign_in_expired",
                format!("No one signed in to {} within 10 minutes, so setup stopped. Start it again.", self.server()),
                None,
            )),
        };
        self.update(|progress| progress.state = SetupState::Running);
        outcome?;
        self.done(SetupStageName::SignIn, None);
        self.begin(SetupStageName::Check);
        Ok(())
    }

    /// Show what the probe found.
    fn note(&self, probe: &Probe) {
        let installed_version = probe.installed_version();
        self.update(|progress| {
            progress.server = SetupServer {
                login: self.route.ssh_target.clone(),
                uid: probe.uid,
                user: probe.user.clone(),
                home: probe.home.clone(),
                os: probe.os.clone(),
                arch: probe.arch.clone(),
                kernel: probe.kernel.clone(),
                glibc: probe.glibc.clone(),
                free_kib: probe.free_kib,
                checksum_tool: probe.checksum_tool().map(str::to_owned),
                storage_unsafe: probe.storage_unsafe.clone(),
            };
            progress.installed = Some(SetupInstalled {
                path: INSTALLED_PATH.into(),
                state: probe.binary.clone().unwrap_or_else(|| "missing".into()),
                version: installed_version.clone(),
                sha256: probe.binary_sha256.clone(),
                runnable: (probe.binary.as_deref() == Some("file"))
                    .then_some(installed_version.is_some()),
                problem: None,
            });
        });
    }

    /// What to do about the account's program, or why nothing can be done. With no copy of its
    /// own to send (`artifact` says why), a job still uses a program there that runs.
    fn plan(&self, probe: &Probe, artifact: Result<&Artifact, String>) -> Result<Plan, Stop> {
        let server = self.server();
        let unexpected =
            |message: String| self.stop("crew_install_unexpected_target", message, None);
        let previous = match probe.binary.as_deref() {
            Some("missing") => None,
            Some("symlink") => {
                return Err(unexpected(format!(
                    "{INSTALLED_PATH} on {server} is a link, not a program, so Biorouter won't replace it. Remove the link, then set up Crew again."
                )))
            }
            Some("file") => {
                if probe.uid.is_none() || probe.binary_owner != probe.uid {
                    return Err(unexpected(format!(
                        "{INSTALLED_PATH} on {server} belongs to another account, so Biorouter won't replace it. Remove it, or ask whoever installed it, then set up Crew again."
                    )));
                }
                match (probe.installed_version(), &artifact) {
                    (Some(version), Err(_)) => return Ok(Plan::Reuse(version)),
                    (Some(version), Ok(artifact)) => {
                        let ours = &artifact.info.version;
                        let (older, same) = match (version_of(&version), version_of(ours)) {
                            (Some(theirs), Some(ours)) => (theirs < ours, theirs == ours),
                            _ => (version != *ours, version == *ours),
                        };
                        // This app's version, built from other bytes (a development build,
                        // or one a person copied there): replaced by the copy this app checked,
                        // the one it replaces kept as `.previous`. A newer one stays.
                        let rebuilt = same
                            && probe
                                .binary_sha256
                                .as_deref()
                                .is_some_and(|sha| sha != artifact.info.sha256);
                        if !rebuilt && (!older || !self.replace_existing) {
                            return Ok(Plan::Reuse(version));
                        }
                    }
                    (None, _) if self.replace_existing => {}
                    (None, _) => return Err(self.not_runnable(probe)),
                }
                probe.binary_sha256.clone()
            }
            Some("dir" | "other") => {
                return Err(unexpected(format!(
                    "{INSTALLED_PATH} on {server} isn't a file, so Biorouter won't replace it. Move it aside, then set up Crew again."
                )))
            }
            _ => {
                return Err(self.stop(
                    "crew_setup_server_unreadable",
                    format!("Biorouter couldn't read what {server} said about your account's Crew program."),
                    None,
                ))
            }
        };
        let artifact =
            artifact.map_err(|why| self.stop("crew_install_artifact_unavailable", why, None))?;
        self.check_install_target(probe, artifact)?;
        let from = previous.as_ref().and_then(|_| probe.installed_version());
        Ok(Plan::Install { previous, from })
    }

    /// Why `artifact` cannot be installed in the account `probe` describes, if it cannot: the
    /// platform it is built for, the glibc it needs, a folder to put it in, a tool to check the
    /// copy with, and room for it.
    fn check_install_target(&self, probe: &Probe, artifact: &Artifact) -> Result<(), Stop> {
        let server = self.server();
        let (os, arch) = (probe.os.as_deref(), probe.arch.as_deref());
        if os != Some("Linux") || !matches!(arch, Some("x86_64" | "amd64")) {
            return Err(self.stop(
                "crew_install_unsupported_platform",
                format!(
                    "Crew's server program runs on x86-64 Linux, and {server} is {} on {}.",
                    os.unwrap_or("an unknown system"),
                    arch.unwrap_or("an unknown processor")
                ),
                None,
            ));
        }
        if let (Some(theirs), Some(needed)) = (
            probe.glibc.as_deref().and_then(glibc_version),
            glibc_version(&artifact.glibc_max),
        ) {
            if theirs < needed {
                return Err(self.stop(
                    "crew_install_dependency_missing",
                    format!(
                        "Crew's server program needs glibc {} or newer, and {server} has {}.",
                        artifact.glibc_max,
                        probe.glibc.as_deref().unwrap_or_default()
                    ),
                    None,
                ));
            }
        }
        if !matches!(probe.bin_dir.as_deref(), Some("missing" | "dir")) {
            return Err(self.stop(
                "crew_install_unexpected_target",
                format!(
                    "~/.local/bin on {server} isn't a folder, so Biorouter can't install Crew there."
                ),
                None,
            ));
        }
        if probe.checksum_tool().is_none() {
            return Err(self.stop(
                "crew_install_checksum_tool_missing",
                format!(
                    "{server} has no sha256sum, shasum or openssl, so Biorouter can't check the copy it sends. Ask the server's administrator to install one of them."
                ),
                None,
            ));
        }
        if let Some(free) = probe.free_kib {
            if free.saturating_mul(1024) < artifact.info.size.saturating_mul(3) {
                return Err(self.stop(
                    "crew_install_no_space",
                    format!(
                        "Your account on {server} doesn't have room for Crew's server program ({} MB). Free some space, then try again.",
                        artifact.info.size.div_ceil(1_000_000) * 3
                    ),
                    None,
                ));
            }
        }
        Ok(())
    }

    /// Why the program there cannot be used, from what its `--version` printed.
    fn not_runnable(&self, probe: &Probe) -> Stop {
        let server = self.server();
        let output = probe.binary_output.clone().unwrap_or_default();
        let lower = output.to_ascii_lowercase();
        let (code, message) = if lower.contains("glibc_")
            || lower.contains("cannot open shared object")
        {
            (
                "crew_install_dependency_missing",
                format!("The Crew program in your account on {server} can't run: the server lacks a library it needs. Set up again and choose to replace it with this app's."),
            )
        } else if probe.binary_exit == Some(126)
            || lower.contains("exec format error")
            || lower.contains("cannot execute")
        {
            (
                "crew_install_unsupported_platform",
                format!("The Crew program in your account on {server} isn't built for that server. Set up again and choose to replace it with this app's."),
            )
        } else {
            (
                "crew_install_incompatible",
                format!("The Crew program in your account on {server} didn't say which version it is, so Biorouter won't use it. Set up again and choose to replace it with this app's."),
            )
        };
        self.stop(code, message, detail(&output))
    }

    /// Send and install `artifact` in place of the program whose SHA-256 is `previous` and
    /// whose version is `from` (if any), answering the version now in place.
    async fn install(
        &self,
        artifact: &Artifact,
        previous: Option<String>,
        from: Option<String>,
    ) -> Result<String, Stop> {
        let server = self.server().to_owned();
        let version = artifact.info.version.clone();
        let upgrade = previous.as_ref().map(|sha256| SetupUpgrade {
            version: from,
            sha256: Some(sha256.clone()),
            installed_version: version.clone(),
            installed_sha256: artifact.info.sha256.clone(),
        });
        let script = INSTALL_SCRIPT
            .replace("@JOB@", &super::hex(&rand::random::<[u8; 16]>()))
            .replace("@SIZE@", &artifact.info.size.to_string())
            .replace("@SHA@", &artifact.info.sha256)
            .replace("@VER@", &version)
            .replace("@PREV@", previous.as_deref().unwrap_or("none"));
        let ran = self
            .run(
                script,
                Some(artifact.bytes.clone()),
                INSTALL_LIMIT,
                &self.cancel,
            )
            .await?;
        if ran.cancelled {
            return Err(self.cancelled(ran.input_written));
        }
        match read_install(&ran.stdout) {
            Installed::Done {
                exit,
                output,
                previous,
            } => {
                if exit != Some(0)
                    || output.as_deref().and_then(broker_version) != Some(version.clone())
                {
                    return Err(self.stop(
                        "crew_install_incompatible",
                        format!(
                            "Crew was installed on {server}, but it didn't run there afterwards."
                        ),
                        output.as_deref().and_then(detail),
                    ));
                }
                self.outcome(|outcome| {
                    outcome.installed_version = Some(version.clone());
                    outcome.previous_kept = previous;
                    outcome.upgraded_from = upgrade;
                });
                Ok(version)
            }
            Installed::Failed { reason, rest } => Err(self.install_failure(&reason, &rest)),
            // The connection dropped, the run timed out, or the output was lost. Once every
            // byte was sent, the rename may have happened: only a fresh look can say.
            Installed::Unknown if ran.input_written => {
                let version = self.settle_unknown(artifact).await?;
                self.outcome(|outcome| outcome.upgraded_from = upgrade);
                Ok(version)
            }
            Installed::Unknown => Err(self
                .interrupted(&ran)
                .unwrap_or_else(|| self.unreadable(&ran))),
        }
    }

    /// After a transfer whose end was not seen: installed exactly when the program in place now
    /// is the one sent.
    async fn settle_unknown(&self, artifact: &Artifact) -> Result<String, Stop> {
        let unknown = || {
            self.stop(
                "crew_install_outcome_unknown",
                format!(
                    "The connection to {} dropped while Crew was being installed, and Biorouter couldn't tell whether it finished. Set up Crew again to check.",
                    self.server()
                ),
                None,
            )
        };
        let ran = self
            .run(
                PROBE_SCRIPT.into(),
                None,
                PROBE_LIMIT,
                &CancellationToken::new(),
            )
            .await
            .map_err(|_| unknown())?;
        let probe = read_probe(&ran.stdout).ok_or_else(unknown)?;
        self.note(&probe);
        if probe.binary.as_deref() != Some("file")
            || probe.binary_sha256.as_deref() != Some(artifact.info.sha256.as_str())
        {
            return Err(unknown());
        }
        let version = artifact.info.version.clone();
        self.outcome(|outcome| outcome.installed_version = Some(version.clone()));
        Ok(version)
    }

    fn install_failure(&self, reason: &str, rest: &[String]) -> Stop {
        let server = self.server();
        let (code, message) = match reason {
            "permission_denied" => (
                "crew_install_permission_denied",
                format!("Biorouter couldn't write to ~/.local/bin on {server}. Check that folder's permissions, then try again."),
            ),
            "unexpected_target" => (
                "crew_install_unexpected_target",
                format!("Something other than Crew's program is at {INSTALLED_PATH} or ~/.local/bin on {server}, so nothing was replaced. Check it, then set up Crew again."),
            ),
            "busy" => (
                "crew_install_busy",
                format!("Crew is being installed in your account on {server} from somewhere else right now. Wait a minute, then try again."),
            ),
            "changed" => (
                "crew_install_busy",
                format!("Crew changed in your account on {server} while this ran, so nothing was replaced. Try again."),
            ),
            "no_space" => (
                "crew_install_no_space",
                format!("Your account on {server} ran out of room for Crew's server program. Free some space, then try again."),
            ),
            "transfer_incomplete" => (
                "crew_install_transfer_incomplete",
                format!("Only part of Crew's server program reached {server}, so it wasn't installed. Try again."),
            ),
            "checksum_tool_missing" => (
                "crew_install_checksum_tool_missing",
                format!("{server} has no sha256sum, shasum or openssl, so Biorouter can't check the copy it sends. Ask the server's administrator to install one of them."),
            ),
            "checksum_mismatch" => (
                "crew_install_checksum_mismatch",
                format!("The copy of Crew that reached {server} didn't match the one this app sent, so it wasn't installed and nothing was replaced. Try again."),
            ),
            "staged_version" => {
                let output = rest.get(1).and_then(|hex| unhex_text(hex, 400)).unwrap_or_default();
                let probe = Probe {
                    binary_exit: rest.first().and_then(|code| code.parse().ok()),
                    binary_output: Some(output),
                    ..Probe::default()
                };
                let stop = self.not_runnable(&probe);
                let (code, detail) = (stop.error.code, stop.error.detail);
                let message = match code.as_str() {
                    "crew_install_dependency_missing" => format!("Crew's server program can't run on {server}: the server lacks a library it needs. Nothing was replaced."),
                    "crew_install_unsupported_platform" => format!("Crew's server program can't run on {server}: it isn't built for that server. Nothing was replaced."),
                    _ => format!("Crew's server program didn't run on {server} as expected, so nothing was replaced."),
                };
                return self.stop(&code, message, detail);
            }
            _ => (
                "crew_install_incompatible",
                format!("Installing Crew on {server} stopped, and nothing was replaced."),
            ),
        };
        self.stop(code, message, None)
    }

    /// Start the workspace with the host setup's own hosting key.
    async fn start(&self) -> Result<(), Stop> {
        self.begin(SetupStageName::Start);
        let (Some(slug), Some(key)) = (
            self.workspace_name.as_deref(),
            self.bootstrap_key.as_deref(),
        ) else {
            return Err(self.stop(
                "crew_request_invalid",
                "A host's setup needs the workspace name.",
                None,
            ));
        };
        let command = host_start::host_start_command(slug, key)
            .map_err(|error| self.stop("crew_request_invalid", error.to_string(), None))?;
        let fixed = self.safe_storage().await?;
        let ran = self
            .run(
                command.replace('\n', "; "),
                None,
                INSTALL_LIMIT,
                &self.cancel,
            )
            .await?;
        if let Some(stop) = self.interrupted(&ran) {
            return Err(stop);
        }
        let output = format!("{}\n{}", host_start::shown(&ran.stdout), ran.stderr);
        let result = host_start::read_start_output(&output, ran.exit_code);
        self.outcome(|outcome| {
            outcome.host = Some(result.clone());
            outcome.host_command = Some(command.clone());
        });
        let server = self.server();
        let message = match &result {
            StartOutput::Found { .. } => {
                let said = (!fixed.is_empty()).then(|| {
                    format!(
                        "Other accounts can no longer write to {}.",
                        fixed.join(", ")
                    )
                });
                self.done(SetupStageName::Start, said);
                return Ok(());
            }
            // A folder above the workspace that the walk could not see as this account's (one
            // another account owns): the broker's own check is the one that says so.
            StartOutput::Problem { .. }
                if output.contains("unsafe_storage: writable non-sticky") =>
            {
                return Err(self.stop(
                    "crew_host_storage_unsafe",
                    format!(
                        "Crew on {server} won't keep the workspace in your account, because another account can write to a folder above it. Ask the server's administrator; the details say what Crew found."
                    ),
                    printed(&ran.stdout, &ran.stderr),
                ));
            }
            StartOutput::Problem { problem, .. } => match problem.as_str() {
                "starting" => {
                    format!("Crew is still starting on {server}. Wait a moment, then check again.")
                }
                "not_installed" => format!(
                    "Crew was installed, but {server} couldn't run it to start the workspace."
                ),
                "server_error" => format!(
                    "Crew on {server} didn't start the workspace. The details say what it printed."
                ),
                _ => format!("Crew ran on {server}, but Biorouter couldn't read what it printed."),
            },
        };
        // Everything Crew printed, so a person (or Copy details) sees the server's own words.
        Err(self.stop(
            "crew_setup_start_failed",
            message,
            printed(&ran.stdout, &ran.stderr),
        ))
    }

    /// Before a host's start: the folders above where the workspace is kept that other accounts
    /// may write to stop it, unless the person asked setup to fix them
    /// (`fix_storage_permissions`), which takes that write access away from exactly the folders
    /// the server finds itself. Answers the folders it fixed.
    async fn safe_storage(&self) -> Result<Vec<String>, Stop> {
        let found = self.progress().server.storage_unsafe;
        if found.is_empty() {
            return Ok(Vec::new());
        }
        if !self.fix_storage_permissions {
            return Err(self.storage_unsafe(&found, false));
        }
        let ran = self
            .run(STORAGE_FIX_SCRIPT.into(), None, PROBE_LIMIT, &self.cancel)
            .await?;
        if let Some(stop) = self.interrupted(&ran) {
            return Err(stop);
        }
        let fix = read_storage_fix(&ran.stdout).ok_or_else(|| self.unreadable(&ran))?;
        self.update(|progress| progress.server.storage_unsafe = fix.left.clone());
        if !fix.left.is_empty() {
            return Err(self.storage_unsafe(&fix.left, true));
        }
        Ok(fix.fixed)
    }

    /// `crew_host_storage_unsafe`, naming `folders` and the command that fixes them.
    fn storage_unsafe(&self, folders: &[String], tried: bool) -> Stop {
        let server = self.server();
        let names = folders.join(", ");
        let command = format!(
            "chmod go-w {}",
            folders
                .iter()
                .map(|folder| shell_word(folder))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let message = if tried {
            format!(
                "Biorouter couldn't stop other accounts writing to {names} on {server}, so Crew won't keep a workspace there. Run {command} on {server}, or ask its administrator, then start again."
            )
        } else {
            format!(
                "Crew won't keep a workspace on {server} while other accounts can write to {names}. Run {command} on {server}, or let Biorouter fix it for you, then start again."
            )
        };
        self.stop(
            "crew_host_storage_unsafe",
            message,
            Some(folders.join("\n")),
        )
    }

    /// Connect, verified as a person's Connect is, over the master the job signed in with.
    async fn connect(&self) -> Result<(), Stop> {
        self.begin(SetupStageName::Connect);
        let Some(id) = self.connection_id.as_deref() else {
            return Err(self.stop(
                "crew_request_invalid",
                "A member's setup needs a saved connection.",
                None,
            ));
        };
        // The master this job signed in with is for the route it started with; a connection
        // edited to another route since must not ride it (C12).
        let current = self.manager.connection(id).await.ok();
        if current.as_ref().map(Route::of).as_ref() != Some(&self.route) {
            authentication::retire_setup_masters(id, None);
            return Err(self.stop(
                "crew_setup_changed",
                format!(
                    "The connection to {} changed while Crew was being set up, so setup stopped. Set Crew up again.",
                    self.server()
                ),
                None,
            ));
        }
        match self.manager.connect(id).await {
            Ok(_) => {
                self.outcome(|outcome| outcome.connection_id = Some(id.to_owned()));
                self.done(SetupStageName::Connect, None);
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                let ssh = error
                    .chain()
                    .find_map(|cause| cause.downcast_ref::<super::SshFailure>());
                let code = if let Some(refusal) = super::CrewRefusal::find(&error) {
                    refusal.code().to_owned()
                } else if let Some(failure) = ssh {
                    failure.api_code().to_owned()
                } else if error
                    .chain()
                    .any(|cause| cause.is::<super::WorkspaceIdentityError>())
                {
                    "crew_workspace_identity_mismatch".to_owned()
                } else {
                    "crew_request_refused".to_owned()
                };
                let mut stop = self.stop(&code, message, ssh.and_then(|f| f.detail.clone()));
                stop.error.host = ssh.and_then(|failure| failure.host.clone());
                Err(stop)
            }
        }
    }

    /// The workspace's name for its work folder: the one asked with, else the one the
    /// workspace's signed `hello` names.
    fn workspace_slug(&self) -> Option<String> {
        let valid = |name: &str| {
            biorouter_crew::workspace_name_valid(name)
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        self.workspace_name
            .clone()
            .or_else(|| {
                self.connection_id
                    .as_deref()
                    .and_then(|id| self.manager.broker_hello(id))
                    .and_then(|hello| hello.workspace_name)
            })
            .filter(|name| valid(name))
    }

    /// Make `~/crew-work/<workspace>`. A member's connection gets the agent access the request
    /// asked for (design OW-6), in the folder it already has or this one; a member's job that
    /// was asked for none, or for `off`, changes nothing and makes no folder.
    async fn work_folder(&self, installed_version: &str) -> Result<(), Stop> {
        use SetupStageName as Name;
        self.begin(Name::WorkFolder);
        let member = self
            .connection_id
            .as_deref()
            .filter(|_| self.intent == SetupIntent::Member);
        let mut chosen = None;
        if let Some(id) = member {
            match self.agent_access {
                None => {
                    self.skip(
                        Name::WorkFolder,
                        format!(
                            "What your agents may do on {} is as you set it.",
                            self.server()
                        ),
                    );
                    return Ok(());
                }
                Some(SetupAgentAccess::Off) => {
                    self.skip(
                        Name::WorkFolder,
                        format!(
                            "Your agents have no folder on {}, as you chose.",
                            self.server()
                        ),
                    );
                    return Ok(());
                }
                Some(level) => {
                    let root = self
                        .manager
                        .connection(id)
                        .await
                        .ok()
                        .and_then(|connection| connection.remote_root);
                    chosen = Some((id, level, root));
                }
            }
        }
        let folder = match chosen.as_ref().and_then(|(_, _, root)| root.clone()) {
            // The folder the person already gave the connection stays its folder.
            Some(root) => root,
            None => match self.make_work_folder().await? {
                Some(folder) => folder,
                None => return Ok(()),
            },
        };
        self.outcome(|outcome| outcome.work_folder = Some(folder.clone()));
        let mut said = format!("Your work folder is {folder}.");
        if let Some((id, level, _)) = chosen {
            match self
                .manager
                .apply_agent_access(id, level, &folder, installed_version)
                .await
            {
                Ok(applied) => {
                    self.outcome(|outcome| outcome.remote_access = Some(applied.access));
                    said.push(' ');
                    said.push_str(&applied.sentence(self.server()));
                }
                Err(error) => {
                    tracing::info!(connection = id, %error, "Crew setup couldn't set the connection's agent access");
                    said.push_str(
                        " It isn't set on your connection; choose it in Connection settings.",
                    );
                }
            }
        }
        self.done(Name::WorkFolder, Some(said));
        Ok(())
    }

    /// Make `~/crew-work/<workspace>`, answering its canonical path; `None`, with the stage
    /// skipped and why, when it could not be made.
    async fn make_work_folder(&self) -> Result<Option<String>, Stop> {
        use SetupStageName as Name;
        let Some(slug) = self.workspace_slug() else {
            self.skip(Name::WorkFolder, "The workspace's name isn't known yet.");
            return Ok(None);
        };
        let ran = self
            .run(
                FOLDER_SCRIPT.replace("@SLUG@", &slug),
                None,
                PROBE_LIMIT,
                &self.cancel,
            )
            .await?;
        if ran.cancelled {
            return Err(self.cancelled(false));
        }
        match read_folder(&ran.stdout) {
            Ok(folder) => Ok(Some(folder)),
            Err(reason) => {
                let why = match reason.as_str() {
                    "foreign_owner" => format!("~/crew-work/{slug} belongs to another account."),
                    "permission_denied" => format!("Biorouter couldn't make ~/crew-work/{slug}."),
                    _ => format!("Biorouter couldn't read where ~/crew-work/{slug} is."),
                };
                self.skip(Name::WorkFolder, why);
                Ok(None)
            }
        }
    }

    /// Draft the context document from the installed program's own probe. Nothing here stops
    /// a job: an older program, or an answer that cannot be read, leaves no draft.
    async fn context(&self) -> Result<(), Stop> {
        use SetupStageName as Name;
        self.begin(Name::Context);
        let ran = self
            .run(CONTEXT_SCRIPT.into(), None, PROBE_LIMIT, &self.cancel)
            .await?;
        if ran.cancelled {
            return Err(self.cancelled(false));
        }
        match (ran.exit_code, read_facts(&ran.stdout)) {
            (Some(0), Some(facts)) => {
                let draft = super::context::render_workspace_context(&facts);
                self.outcome(|outcome| outcome.context_draft = Some(draft));
                self.done(Name::Context, None);
            }
            _ => self.skip(
                Name::Context,
                "Crew on the server couldn't describe it, so there is no draft guide.",
            ),
        }
        Ok(())
    }

    /// Steps a job that ended early never reached.
    fn mark_unreached(&self) {
        self.update(|progress| {
            for stage in &mut progress.stages {
                if stage.state == SetupStageState::Pending && stage.name != SetupStageName::Done {
                    stage.state = SetupStageState::Skipped;
                }
            }
        });
    }

    fn end(&self, result: Result<(), Stop>) {
        self.mark_unreached();
        self.update(|progress| match result {
            Ok(()) => {
                progress.state = SetupState::Finished;
                progress.stage = SetupStageName::Done;
                if let Some(done) = progress
                    .stages
                    .iter_mut()
                    .find(|stage| stage.name == SetupStageName::Done)
                {
                    done.state = SetupStageState::Done;
                }
                progress.outcome.get_or_insert_with(Default::default);
            }
            Err(stop) => {
                let at = progress.stage;
                if let Some(stage) = progress.stages.iter_mut().find(|stage| stage.name == at) {
                    stage.state = SetupStageState::Failed;
                }
                progress.state = stop.state;
                progress.error = Some(stop.error);
            }
        });
        *self
            .finished_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
    }
}

/// Refuse a setup request whose workspace name the start command would not take, or whose
/// intent does not fit its target.
fn check_request(request: &SetupRequest) -> Result<()> {
    if let Some(name) = &request.workspace_name {
        // The start command's own grammar.
        host_start::host_start_command(name, &"0".repeat(64))
            .map_err(|error| refused(400, "crew_request_invalid", error.to_string()))?;
    }
    match (request.intent, &request.target) {
        (SetupIntent::Host, SetupTarget::Pending { .. }) if request.workspace_name.is_none() => {
            Err(refused(
                400,
                "crew_request_invalid",
                "Name the workspace to host.",
            ))
        }
        (SetupIntent::Host, SetupTarget::Connection { .. }) => Err(refused(
            400,
            "crew_request_invalid",
            "Hosting starts from this computer's host setup, not from a saved connection.",
        )),
        (SetupIntent::Member, SetupTarget::Pending { .. }) => Err(refused(
            400,
            "crew_request_invalid",
            "A member's setup is for a saved connection.",
        )),
        _ => Ok(()),
    }
}

impl CrewManager {
    /// Start a setup job (see the module documentation). Answers the job already under way for
    /// the same server and intent rather than starting a second.
    pub async fn start_setup(&self, request: SetupRequest) -> Result<SetupStatus> {
        let manager = self
            .this
            .get()
            .and_then(std::sync::Weak::upgrade)
            .context("Crew setup needs the daemon's own Crew manager")?;
        check_request(&request)?;
        let (key, route, bootstrap_key, connection_id) = self.setup_target(&request.target).await?;
        let control = self.control_path(&key)?;
        let _starting = STARTING.lock().await;
        {
            let jobs = jobs();
            if let Some((id, job)) = jobs.iter().find(|(_, job)| job.running() && job.key == key) {
                // The same job only for the same login and route: one for another would ride
                // the sign-in this one is using (C12).
                if job.intent == request.intent && job.route == route {
                    return Ok(job.status(id));
                }
                return Err(refused(
                    409,
                    "crew_setup_busy",
                    "Crew is already being set up on this server from this computer. Wait for it to finish.",
                ));
            }
            if jobs.values().filter(|job| job.running()).count() >= MAX_RUNNING {
                return Err(refused(
                    409,
                    "crew_setup_busy",
                    "Crew is already being set up on other servers from this computer. Wait for one to finish.",
                ));
            }
        }
        // A master an earlier job's sign-in left at this key serves only the login and route
        // it signed in to. One for another route ends here, so this job's check asks for its
        // own sign-in rather than running every step as the other account (C12).
        authentication::retire_setup_masters(&key, Some(&route.digest()));
        let server = SetupServer {
            login: route.ssh_target.clone(),
            ..SetupServer::default()
        };
        let job = Arc::new(Job {
            key,
            intent: request.intent,
            route,
            control,
            manager,
            bootstrap_key,
            workspace_name: request.workspace_name,
            replace_existing: request.replace_existing,
            agent_access: request.agent_access,
            fix_storage_permissions: request.fix_storage_permissions,
            connection_id,
            progress: Mutex::new(Progress {
                state: SetupState::Running,
                stage: SetupStageName::Check,
                stages: stages_for(request.intent),
                server,
                installed: None,
                artifact: None,
                outcome: None,
                error: None,
            }),
            finished_at: Mutex::new(None),
            cancel: CancellationToken::new(),
            signed_in: tokio::sync::Notify::new(),
        });
        // The preflight reads the invocation every step runs, login included.
        ssh_policy::preflight(&job.args(remote(PROBE_SCRIPT)), &job.route.ssh_target).await?;
        let job_id = uuid::Uuid::new_v4().to_string();
        jobs().insert(job_id.clone(), Arc::clone(&job));
        let running = Arc::clone(&job);
        let ended = job_id.clone();
        tokio::spawn(async move {
            let result = running.drive().await;
            running.end(result);
            // A sign-in the job opened and nobody finished holds the server's key for nothing
            // now; a master it adopted stays, for the start, Create and the connect.
            authentication::end_setup_sign_ins(&ended);
        });
        Ok(job.status(&job_id))
    }

    /// Where a setup job for `target` runs: its SSH control key, its route, a host's hosting
    /// key, and the saved connection it is for, if it is for one.
    async fn setup_target(
        &self,
        target: &SetupTarget,
    ) -> Result<(String, Route, Option<String>, Option<String>)> {
        Ok(match target {
            SetupTarget::Pending {
                preparation_id,
                ssh_target,
                port,
                identity_file,
                proxy_jump,
            } => {
                host_start::check_route(
                    ssh_target,
                    proxy_jump.as_deref(),
                    identity_file.as_deref(),
                )
                .map_err(as_setup)?;
                let bootstrap_key = self
                    .host_setup_key(preparation_id)
                    .await
                    .map_err(as_setup)?;
                let route = Route {
                    ssh_target: ssh_target.clone(),
                    port: *port,
                    identity_file: identity_file.clone(),
                    proxy_jump: proxy_jump.clone(),
                };
                (preparation_id.clone(), route, Some(bootstrap_key), None)
            }
            SetupTarget::Connection { connection_id } => {
                let connection = self.connection(connection_id).await.map_err(|_| {
                    refused(
                        404,
                        "crew_connection_not_found",
                        "No saved connection has that ID on this computer.",
                    )
                })?;
                let route = Route::of(&connection);
                host_start::check_route(
                    &route.ssh_target,
                    route.proxy_jump.as_deref(),
                    route.identity_file.as_deref(),
                )
                .map_err(as_setup)?;
                (
                    connection_id.clone(),
                    route,
                    None,
                    Some(connection_id.clone()),
                )
            }
        })
    }

    /// Give `id` the agent access `level` (never [`SetupAgentAccess::Off`], which changes
    /// nothing), working in `folder`: full access where its server and bridge take it (as its
    /// last verified connect found) and the installed program names a version, else the folder,
    /// with commands where the server can confine them.
    async fn apply_agent_access(
        &self,
        id: &str,
        level: SetupAgentAccess,
        folder: &str,
        installed_version: &str,
    ) -> Result<AppliedAccess> {
        let connection = self.connection(id).await?;
        let access = self.server_access(id);
        let full = access.as_ref().is_some_and(|a| a.full_access_available)
            && version_of(installed_version).is_some();
        let confined = access.as_ref().and_then(|a| a.confined_execution_available) == Some(true);
        let applied = match level {
            SetupAgentAccess::Full if full => AppliedAccess {
                access: RemoteAccess::Full,
                execution: true,
                full_unavailable: false,
            },
            SetupAgentAccess::Full | SetupAgentAccess::Folder => AppliedAccess {
                access: RemoteAccess::Folder,
                execution: confined,
                full_unavailable: level == SetupAgentAccess::Full,
            },
            SetupAgentAccess::Off => anyhow::bail!("Off changes nothing"),
        };
        let root = connection
            .remote_root
            .clone()
            .unwrap_or_else(|| folder.to_owned());
        self.save_keeping_master(
            id,
            access_save(&connection, &root, applied.execution, applied.access),
        )
        .await?;
        Ok(applied)
    }

    /// Save `input` over `id` as [`Self::update`] does, for an edit that keeps the route: the
    /// policy epoch moves (every grant on it ends), the bridge and what it announced are
    /// dropped, and a connection that was connected is dialled again from scratch. Unlike
    /// [`Self::update`], it leaves the SSH master and the sign-in it was adopted from alone
    /// (C11): the route is the same, so they are still this connection's, and the dial rides
    /// them. A password-only account would otherwise end setup disconnected, asked for its
    /// password again.
    async fn save_keeping_master(&self, id: &str, input: SaveConnection) -> Result<Connection> {
        let _lifecycle = self.connection_guard(id).await?;
        Self::validate_connection(&input)?;
        let saved = self.connection(id).await?;
        ensure!(
            super::same_route(&saved, &input),
            "The connection changed while Crew was being set up. Set Crew up again."
        );
        if let Some(unchanged) = self.unchanged_by(id, &input).await? {
            return Ok(unchanged);
        }
        // P-1: a save that would be refused is refused while the bridge is still up.
        self.refusal_before_saving(id, &input).await?;
        self.disarm_idle_redial(id);
        let removed = self.transports.lock().await.remove(id);
        let reconnect = removed.is_some();
        if let Some(transport) = removed {
            transport.lock().await.close().await;
        }
        self.forget_broker(id);
        {
            let mut registry = self.registry.lock().await;
            if let Some(c) = registry.connections.iter_mut().find(|c| c.id == id) {
                c.status = "disconnected".into();
            }
        }
        let saved = self.save_inner(Some(id), input).await?;
        if !reconnect || authentication::ensure_connect_available(id).is_err() {
            return Ok(saved);
        }
        match self.connect_locked(id).await {
            Ok(connected) => Ok(connected),
            Err(error) => {
                let message = error.to_string();
                tracing::info!(connection = id, %error, "Crew couldn't reconnect after setup saved the connection");
                {
                    let mut registry = self.registry.lock().await;
                    if let Some(c) = registry.connections.iter_mut().find(|c| c.id == id) {
                        c.status = "disconnected".into();
                        c.last_error = Some(message.clone());
                    }
                }
                self.note_error_code(id, &error, &message);
                self.connection(id).await
            }
        }
    }

    /// The server's facts over `id`'s own control path, and the context document drafted from
    /// them (`POST /crew/connections/{id}/context/draft`). Never prompts.
    pub async fn context_draft(&self, id: &str) -> Result<ContextDraft> {
        let connection = self.connection(id).await.map_err(|_| {
            refused(
                404,
                "crew_connection_not_found",
                "No saved connection has that ID on this computer.",
            )
        })?;
        let mut args = transport::ssh_args(&connection, &self.control_path(id)?);
        args.extend([
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ControlMaster=no".into(),
            "-o".into(),
            "ControlPersist=no".into(),
            connection.ssh_target.clone(),
            remote(CONTEXT_SCRIPT),
        ]);
        ssh_policy::preflight(&args, &connection.ssh_target).await?;
        let ran =
            host_start::run_fixed(&args, None, PROBE_LIMIT, &CancellationToken::new()).await?;
        let server = server_name(&connection.ssh_target);
        if ran.timed_out {
            return Err(refused(
                409,
                "crew_setup_timed_out",
                format!("{server} took too long to describe itself. Try again."),
            ));
        }
        if ran.exit_code == Some(255) {
            let kind = transport::classify_exit(ran.exit_code, &ran.stderr);
            return Err(refused(
                400,
                kind.api_code(),
                host_start::ssh_sentence(kind, &ran.stderr, &connection.ssh_target),
            ));
        }
        if ran.exit_code != Some(0) {
            return Err(refused(
                409,
                "crew_server_probe_unsupported",
                format!(
                    "The Crew program in your account on {server} is too old to describe the server. Set up Crew again to update it."
                ),
            ));
        }
        let facts = read_facts(&ran.stdout).ok_or_else(|| {
            refused(
                409,
                "crew_setup_server_unreadable",
                format!("Biorouter couldn't read what {server} said about itself."),
            )
        })?;
        Ok(ContextDraft {
            draft: super::context::render_workspace_context(&facts),
            facts,
        })
    }
}

/// The agent access [`CrewManager::apply_agent_access`] set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AppliedAccess {
    access: RemoteAccess,
    /// Whether the connection's agents may run commands.
    execution: bool,
    /// Full access was asked for, and the server or the bridge does not take it.
    full_unavailable: bool,
}

impl AppliedAccess {
    /// What it means for the person, after "Your work folder is …".
    fn sentence(&self, server: &str) -> String {
        match (self.access, self.full_unavailable, self.execution) {
            (RemoteAccess::Full, _, _) => "Your agents can now work there as you.".into(),
            (_, true, true) => format!(
                "Full access isn't available on {server} yet, so your agents work only in that folder."
            ),
            (_, true, false) => format!(
                "Full access isn't available on {server} yet, so your agents can read and write files in that folder but not run commands."
            ),
            (_, false, true) => "Your agents work only in that folder.".into(),
            (_, false, false) => format!(
                "Your agents can read and write files in that folder; {server} can't keep commands inside it, so they can't run commands."
            ),
        }
    }
}

/// `connection` as a save that gives it `root` at agent access `access`, commands on or off as
/// `execution` says, all else as it is.
fn access_save(
    connection: &Connection,
    root: &str,
    execution: bool,
    access: RemoteAccess,
) -> SaveConnection {
    SaveConnection {
        preparation_id: None,
        name: connection.name.clone(),
        ssh_target: connection.ssh_target.clone(),
        port: connection.port,
        identity_file: connection.identity_file.clone(),
        proxy_jump: connection.proxy_jump.clone(),
        socket_path: connection.socket_path.clone(),
        owner_uid: connection.owner_uid,
        workspace_id: connection.workspace_id.clone(),
        workspace_public_key: connection.workspace_public_key.clone(),
        remote_root: Some(root.to_owned()),
        remote_execution: execution,
        remote_access: access,
        cluster_connection_id: Some(connection.cluster_connection_id.clone()),
        mode: connection.mode,
        institution_id: connection.institution_id.clone(),
    }
}

#[cfg(all(test, unix))]
#[path = "setup_tests.rs"]
mod tests;
