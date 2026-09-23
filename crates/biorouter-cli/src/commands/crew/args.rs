use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    StreamJson,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PrivacyMode {
    Private,
    Public,
}

impl PrivacyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Classification {
    Restricted,
    PublicSafe,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Restricted => "restricted",
            Self::PublicSafe => "public_safe",
        }
    }
}

#[derive(Args)]
pub struct CrewOptions {
    /// Saved connection ID. Required when more than one connection is saved.
    #[arg(long, global = true)]
    pub connection: Option<String>,
    /// Require this privacy mode for send, task start, grants, and file upload/download/resume.
    #[arg(long, global = true, value_enum)]
    pub expected_mode: Option<PrivacyMode>,
    /// Require this verified connection policy epoch when starting tasks or granting access.
    #[arg(long, global = true)]
    pub expected_policy_epoch: Option<u64>,
    /// Require this verified workspace policy epoch when starting tasks or granting access.
    #[arg(long, global = true)]
    pub expected_workspace_policy_epoch: Option<u64>,
    /// Require an already-running shared daemon.
    #[arg(long, global = true)]
    pub no_start: bool,
    /// Read the human approval key from stdin's first line instead of a hidden prompt.
    #[arg(long, global = true)]
    pub approval_key_stdin: bool,
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub output_format: OutputFormat,
    /// Reuse this identifier when retrying the same mutation after an uncertain result.
    #[arg(long, global = true)]
    pub request_id: Option<String>,
    #[command(subcommand)]
    pub command: CrewCommand,
}

#[derive(Subcommand)]
pub enum CrewCommand {
    #[command(subcommand)]
    Daemon(DaemonCommand),
    #[command(subcommand)]
    Credentials(CredentialCommand),
    /// Show saved connections and their daemon-reported state.
    Status,
    #[command(subcommand)]
    Connections(ConnectionCommand),
    /// Authenticate SSH through the shared daemon's owned authentication session.
    Auth,
    /// Open the verified SSH bridge for the selected connection.
    Connect,
    /// Close the selected SSH connection.
    Disconnect,
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    #[command(subcommand)]
    Enroll(EnrollmentCommand),
    /// List the principals visible in the selected workspace.
    Members,
    #[command(subcommand)]
    Teams(TeamCommand),
    #[command(subcommand)]
    Channels(ChannelCommand),
    #[command(subcommand)]
    Invites(InvitationCommand),
    #[command(subcommand)]
    Profile(ProfileCommand),
    #[command(subcommand)]
    Ownership(OwnershipCommand),
    /// Remove a member from a channel owned by you.
    RemoveMember { channel: String, principal: String },
    /// Read a page of authorized channel messages.
    History(HistoryArgs),
    /// Search authorized channel messages.
    Search {
        channel: String,
        query: String,
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=200))]
        limit: u16,
        #[arg(long)]
        after: Option<String>,
    },
    /// Follow channel messages. Ctrl-C detaches without cancelling tasks.
    Watch(WatchArgs),
    /// Post as your own authenticated workspace identity.
    Send(SendArgs),
    /// Show the channel/context scope of an existing grant.
    Context { session: String },
    #[command(subcommand)]
    Files(FileCommand),
    #[command(subcommand)]
    Tasks(TaskCommand),
    #[command(subcommand)]
    Grants(GrantCommand),
    #[command(subcommand)]
    Privacy(PrivacyCommand),
}

#[cfg(test)]
mod tests {
    use super::{CrewCommand, CrewOptions, PrivacyMode};
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        crew: CrewOptions,
    }

    #[test]
    fn expected_mode_accepts_private_and_public() {
        let private = TestCli::try_parse_from(["crew", "--expected-mode", "private", "status"])
            .expect("private expected mode should parse");
        assert!(matches!(
            private.crew.expected_mode,
            Some(PrivacyMode::Private)
        ));

        let public = TestCli::try_parse_from(["crew", "--expected-mode", "public", "status"])
            .expect("public expected mode should parse");
        assert!(matches!(
            public.crew.expected_mode,
            Some(PrivacyMode::Public)
        ));
        assert!(matches!(public.crew.command, CrewCommand::Status));
    }

    #[test]
    fn expected_mode_is_omitted_by_default_and_rejects_unknown_values() {
        let omitted =
            TestCli::try_parse_from(["crew", "status"]).expect("expected mode is optional");
        assert!(omitted.crew.expected_mode.is_none());

        let invalid = TestCli::try_parse_from(["crew", "--expected-mode", "internal", "status"]);
        assert!(invalid.is_err());
    }
}

#[derive(Subcommand)]
pub enum DaemonCommand {
    Start,
    Status,
    Stop,
}

#[derive(Subcommand)]
pub enum CredentialCommand {
    Status,
    Init,
    Unlock,
    Lock,
}

#[derive(Subcommand)]
pub enum ConnectionCommand {
    List,
    Show,
    /// Prepare this device's enrollment public key in the daemon.
    Prepare,
    /// Save a verified connection descriptor from JSON. Use - for stdin.
    Save {
        input: PathBuf,
    },
    /// Replace the selected connection descriptor from JSON. Use - for stdin.
    Update {
        input: PathBuf,
    },
    Remove,
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    Show,
    /// Initialize the workspace using this device's prepared public identity.
    Bootstrap,
}

#[derive(Args)]
pub struct SecretInput {
    /// Read the enrollment token from the first stdin line instead of a hidden prompt.
    #[arg(long, conflicts_with = "token_fd")]
    pub token_stdin: bool,
    /// Read the enrollment token from an explicitly supplied file descriptor.
    #[arg(long, conflicts_with = "token_stdin")]
    pub token_fd: Option<i32>,
}

#[derive(Subcommand)]
pub enum EnrollmentCommand {
    Prepare,
    /// Invite a colleague's remote Unix UID and enrollment public key.
    Invite {
        #[arg(long)]
        uid: u32,
        #[arg(long)]
        public_key: String,
        /// Required when adding a device to an existing principal.
        #[arg(long)]
        existing_principal: Option<String>,
    },
    Accept(SecretInput),
    /// Revoke the principal's enrollment, devices and grants.
    Revoke {
        principal: String,
    },
}

#[derive(Subcommand)]
pub enum TeamCommand {
    List,
    Create { name: String },
}

#[derive(Subcommand)]
pub enum ChannelCommand {
    List {
        #[arg(long)]
        team: Option<String>,
    },
    Create {
        name: String,
        #[arg(long)]
        team: String,
        #[arg(long, value_enum, default_value = "restricted")]
        classification: Classification,
    },
    Archive {
        channel: String,
    },
    /// Advance your read marker to an opaque message cursor.
    MarkRead {
        channel: String,
        cursor: String,
    },
}

#[derive(Subcommand)]
pub enum InvitationCommand {
    List,
    Create {
        principal: String,
        #[arg(long, conflicts_with = "channel", required_unless_present = "channel")]
        team: Option<String>,
        #[arg(long, conflicts_with = "team", required_unless_present = "team")]
        channel: Option<String>,
    },
    Accept {
        invitation: String,
    },
}

#[derive(Subcommand)]
pub enum ProfileCommand {
    Show,
    Set {
        nickname: String,
        /// Emoji or initials, up to the workspace's supported length.
        #[arg(long)]
        avatar: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum OwnershipCommand {
    /// Offer channel ownership to another member.
    Offer { channel: String, successor: String },
    /// Accept an offer. The previous owner leaves the channel.
    Accept { channel: String },
}

#[derive(Args)]
pub struct HistoryArgs {
    pub channel: String,
    #[arg(long, conflicts_with = "after")]
    pub before: Option<String>,
    #[arg(long, conflicts_with = "before")]
    pub after: Option<String>,
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=200))]
    pub limit: u16,
    /// Read the newest matching page.
    #[arg(long)]
    pub latest: bool,
}

#[derive(Args)]
pub struct WatchArgs {
    pub channel: String,
    #[arg(long)]
    pub after: Option<String>,
}

#[derive(Args)]
pub struct TextInput {
    #[arg(long, conflicts_with = "input", required_unless_present = "input")]
    pub text: Option<String>,
    /// Read UTF-8 content from a file, or - for stdin.
    #[arg(long, conflicts_with = "text", required_unless_present = "text")]
    pub input: Option<PathBuf>,
}

#[derive(Args)]
pub struct SendArgs {
    pub channel: String,
    #[arg(long, conflicts_with = "input")]
    pub text: Option<String>,
    /// Read UTF-8 content from a file, or - for stdin.
    #[arg(long, conflicts_with = "text")]
    pub input: Option<PathBuf>,
    #[arg(long = "attachment")]
    pub attachments: Vec<String>,
    #[arg(long = "reference")]
    pub references: Vec<String>,
}

#[derive(Subcommand)]
pub enum FileCommand {
    Upload {
        channel: String,
        file: PathBuf,
    },
    Resume {
        transfer: String,
        file: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    Status {
        transfer: String,
    },
    /// Follow daemon transfer progress; Ctrl-C leaves the transfer running.
    Watch {
        transfer: String,
    },
    Pending,
    Pause {
        transfer: String,
    },
    /// Remove a transfer receipt, cleaning up its owned partial download when needed.
    Forget {
        transfer: String,
        /// Original destination for an incomplete download. Published files are retained.
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Download {
        blob: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Share a remote path reference; this does not upload or validate its contents.
    Reference {
        channel: String,
        path: String,
        #[arg(long)]
        label: String,
    },
    ShowReference {
        reference: String,
    },
}

#[derive(Subcommand)]
pub enum TaskCommand {
    Start {
        channel: String,
        #[command(flatten)]
        prompt: TextInput,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long = "context-channel")]
        context_channels: Vec<String>,
        /// Allow this owned task to publish results to its destination channel.
        #[arg(long)]
        allow_posting: bool,
    },
    List,
    Show {
        run: String,
    },
    Watch {
        run: String,
    },
    Cancel {
        run: String,
    },
}

#[derive(Subcommand)]
pub enum GrantCommand {
    List,
    Grant {
        session: String,
        channel: String,
        #[arg(long = "context-channel")]
        context_channels: Vec<String>,
    },
    Revoke {
        session: String,
    },
}

#[derive(Subcommand)]
pub enum PrivacyCommand {
    Show,
    SetPersonal {
        #[arg(value_enum)]
        mode: PrivacyMode,
        /// Canonical institution ID for this SSH cluster, required for a new Private label.
        #[arg(long = "institution")]
        institution_id: Option<String>,
    },
    SetWorkspace {
        #[arg(value_enum)]
        mode: PrivacyMode,
        /// Confirm the immutable workspace institution as its authorized host.
        #[arg(long = "institution")]
        institution_id: Option<String>,
    },
}
