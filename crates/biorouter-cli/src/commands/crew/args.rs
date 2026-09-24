//! Arguments for `biorouter crew`.
//!
//! Every argument that names a person, team or channel takes a *selector*
//! ("Selectors and the resolver" in `docs/research/biorouter-crew/naming-design.md`):
//!
//! - a person is `@bob` (the username on the server; a display name never selects anyone);
//! - a team is its name or handle, `analysis-lab` or `"Analysis Lab"`;
//! - a channel is `methods`, `#methods` or `analysis-lab/methods`. The `#` is optional
//!   everywhere, because it starts a comment in bash and zsh unless quoted;
//! - a saved connection (`--connection`) is its name, its SSH target or its ID.
//!
//! The shared daemon resolves names against the person's own workspace snapshot. UUID-shaped
//! text is always an ID and is sent as it is, so scripts that pass IDs keep working.

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
    /// Saved connection: its name, its SSH target or its ID. Required when more than one
    /// connection is saved.
    #[arg(long, global = true, value_name = "CONNECTION")]
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
    /// Show the machine IDs of people, teams, channels, tasks and files in text output.
    /// JSON output always carries them.
    #[arg(long, global = true)]
    pub show_ids: bool,
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
    /// Join the workspace a saved connection points to: show this computer's code for the
    /// host, then wait until they let you in.
    Join(JoinArgs),
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    #[command(subcommand)]
    Enroll(EnrollmentCommand),
    /// List the people in the selected workspace.
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
    /// Remove a member from a channel you own.
    RemoveMember {
        /// The channel: methods, '#methods' or analysis-lab/methods.
        channel: String,
        /// The member, as @username.
        member: String,
        /// Remove someone who has already left the workspace.
        #[arg(long)]
        former: bool,
    },
    /// Read a page of authorized channel messages.
    History(HistoryArgs),
    /// Search authorized channel messages.
    Search {
        /// The channel: methods, '#methods' or analysis-lab/methods.
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
    /// Save a connection from the invitation your host sent. Use - to paste it on stdin.
    JoinInvitation(JoinInvitationArgs),
    /// Print the invitation message to send someone you invited (host).
    Invitation {
        /// The invited person, as @username; the message then names them.
        #[arg(long = "for", value_name = "PERSON")]
        invitee: Option<String>,
    },
}

/// `connections join-invitation`: the invitation and the choices the Join screen offers.
#[derive(Args)]
pub struct JoinInvitationArgs {
    /// A file holding the host's whole message, the bare `brcrew1:` line, or the JSON
    /// `biorouter-crew status` prints. Use - for stdin.
    pub input: PathBuf,
    /// Show what the invitation says and save nothing.
    #[arg(long, conflicts_with = "yes")]
    pub preview: bool,
    /// Save without asking. Needed when there is no terminal to ask in, including when the
    /// invitation is read from stdin.
    #[arg(long)]
    pub yes: bool,
    /// Your username on the server. Default: the username the host invited.
    #[arg(long)]
    pub username: Option<String>,
    /// How this computer treats the workspace. Default: the workspace's own mode.
    #[arg(long, value_enum)]
    pub mode: Option<PrivacyMode>,
    /// Institution for a Private connection. Default: the workspace's.
    #[arg(long = "institution")]
    pub institution_id: Option<String>,
    /// This computer's name for the connection. Default: the workspace's name.
    #[arg(long)]
    pub name: Option<String>,
    /// A login from your own SSH settings (an alias or user@host), used instead of
    /// username@server. The invitation's port and jump host are then not applied.
    #[arg(long)]
    pub ssh_target: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub identity_file: Option<String>,
    /// A jump route. An empty value means none, even when the invitation suggests one.
    #[arg(long)]
    pub proxy_jump: Option<String>,
    /// The remote work folder agents may use.
    #[arg(long)]
    pub remote_root: Option<String>,
    /// Allow agents to run commands in the remote work folder.
    #[arg(long, requires = "remote_root")]
    pub remote_execution: bool,
    /// A prepared hosting identity, when a host saves their own workspace from what
    /// `biorouter-crew start` printed.
    #[arg(long)]
    pub preparation_id: Option<String>,
}

#[derive(Args)]
pub struct JoinArgs {
    /// Show where joining stands and return instead of waiting.
    #[arg(long)]
    pub no_wait: bool,
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    Show,
    /// Initialize the workspace using this device's prepared public identity.
    Bootstrap,
    /// Rename the workspace (host only): lowercase letters, numbers and dashes.
    Rename {
        name: String,
    },
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
    /// Invite someone to join the workspace by their username on the server (host).
    Invite(EnrollInviteArgs),
    /// Show who is waiting to join and whether you let them in yet (host).
    Pending,
    /// Let someone in: type the code their Biorouter shows them (host).
    Approve {
        /// The person, as @username.
        person: String,
        /// The 16-character code they sent you, like 7QK2-M9XA-3JTP-WZ4D.
        code: String,
        /// Replace a code you already approved for them.
        #[arg(long)]
        replace: bool,
    },
    /// Withdraw an invitation to join (host).
    Cancel {
        /// The person, as @username.
        person: String,
    },
    /// Deprecated: accept an enrollment token from a host using an older version of Biorouter.
    /// Join with an invitation instead.
    #[command(hide = true)]
    Accept(SecretInput),
    /// Revoke a member's enrollment, devices and grants (host).
    Revoke {
        /// The member, as @username or their ID. An ID never prompts, for scripts.
        member: String,
        /// Confirm by typing the member's @username again. Required for a name when there is
        /// no terminal to ask in.
        #[arg(long, value_name = "PERSON")]
        confirm: Option<String>,
    },
}

/// `enroll invite @bob [--add-device]`, and the deprecated `--uid --public-key` form an older
/// Biorouter on the joiner's computer still needs.
#[derive(Args)]
pub struct EnrollInviteArgs {
    /// The person's username on the server, as @username.
    #[arg(conflicts_with = "uid", required_unless_present = "uid")]
    pub username: Option<String>,
    /// Invite an existing member to add another computer.
    #[arg(long, conflicts_with = "uid")]
    pub add_device: bool,
    /// Deprecated: the colleague's numeric user ID on the server (legacy enrollment token).
    #[arg(long, hide = true, required_unless_present = "username")]
    pub uid: Option<u32>,
    /// Deprecated: the colleague's prepared device public key (legacy enrollment token).
    #[arg(
        long,
        hide = true,
        required_unless_present = "username",
        conflicts_with = "username"
    )]
    pub public_key: Option<String>,
    /// Deprecated: the member a legacy token adds a device to. With a username, use
    /// --add-device instead.
    #[arg(long, hide = true, requires = "uid", conflicts_with = "username")]
    pub existing_principal: Option<String>,
}

#[derive(Subcommand)]
pub enum TeamCommand {
    List,
    Create {
        name: String,
    },
    /// Rename a team you created.
    Rename {
        /// The team: its name or handle.
        team: String,
        /// The new name.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum ChannelCommand {
    List {
        /// Only this team's channels.
        #[arg(long)]
        team: Option<String>,
    },
    Create {
        name: String,
        /// The team: its name or handle.
        #[arg(long)]
        team: String,
        #[arg(long, value_enum, default_value = "restricted")]
        classification: Classification,
    },
    Archive {
        /// The channel: methods, '#methods' or analysis-lab/methods.
        channel: String,
    },
    /// Mark a channel read, up to its newest message or to an opaque message cursor.
    MarkRead {
        /// The channel: methods, '#methods' or analysis-lab/methods.
        channel: String,
        /// Mark read up to this message cursor instead of the newest message.
        cursor: Option<String>,
    },
    /// Rename a channel you own.
    Rename {
        /// The channel: methods, '#methods' or analysis-lab/methods.
        channel: String,
        /// The new name.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum InvitationCommand {
    List,
    /// Invite a member to a team you created or a channel you own.
    Create {
        /// The person, as @username.
        person: String,
        /// The team: its name or handle.
        #[arg(long, conflicts_with = "channel", required_unless_present = "channel")]
        team: Option<String>,
        /// The channel: methods, '#methods' or analysis-lab/methods.
        #[arg(long, conflicts_with = "team", required_unless_present = "team")]
        channel: Option<String>,
    },
    /// Accept an invitation. With one pending, it needs no argument.
    Accept {
        /// The team or channel you were invited to (analysis-lab, analysis-lab/methods), or
        /// the invitation's ID.
        invitation: Option<String>,
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
    Offer {
        /// The channel: methods, '#methods' or analysis-lab/methods.
        channel: String,
        /// The member, as @username.
        successor: String,
    },
    /// Accept an offer. The previous owner leaves the channel.
    Accept {
        /// The channel: methods, '#methods' or analysis-lab/methods.
        channel: String,
    },
}

#[derive(Args)]
pub struct HistoryArgs {
    /// The channel: methods, '#methods' or analysis-lab/methods.
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
    /// The channel: methods, '#methods' or analysis-lab/methods.
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
    /// The channel: methods, '#methods' or analysis-lab/methods.
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
        /// The channel: methods, '#methods' or analysis-lab/methods.
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
        /// The channel: methods, '#methods' or analysis-lab/methods.
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
        /// The channel the task posts to: methods, '#methods' or analysis-lab/methods.
        channel: String,
        #[command(flatten)]
        prompt: TextInput,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        /// Another channel the task may read. Repeat for more.
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
        /// The channel the chat may read and post in: methods, '#methods' or
        /// analysis-lab/methods.
        channel: String,
        /// Another channel the chat may read. Repeat for more.
        #[arg(long = "context-channel")]
        context_channels: Vec<String>,
    },
    /// Stop a chat or task using Crew. Exits non-zero unless the workspace confirmed it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        crew: CrewOptions,
    }

    fn parse(args: &[&str]) -> CrewOptions {
        let mut argv = vec!["crew"];
        argv.extend_from_slice(args);
        match TestCli::try_parse_from(argv) {
            Ok(cli) => cli.crew,
            Err(error) => panic!("{args:?} should parse: {error}"),
        }
    }

    fn refused(args: &[&str]) -> bool {
        let mut argv = vec!["crew"];
        argv.extend_from_slice(args);
        TestCli::try_parse_from(argv).is_err()
    }

    const ID: &str = "0f9c2d4e-8a1b-4c3d-9e5f-6a7b8c9d0e1f";

    #[test]
    fn expected_mode_accepts_private_and_public() {
        let private = parse(&["--expected-mode", "private", "status"]);
        assert!(matches!(private.expected_mode, Some(PrivacyMode::Private)));

        let public = parse(&["--expected-mode", "public", "status"]);
        assert!(matches!(public.expected_mode, Some(PrivacyMode::Public)));
        assert!(matches!(public.command, CrewCommand::Status));
    }

    #[test]
    fn expected_mode_is_omitted_by_default_and_rejects_unknown_values() {
        let omitted = parse(&["status"]);
        assert!(omitted.expected_mode.is_none());
        assert!(refused(&["--expected-mode", "internal", "status"]));
    }

    #[test]
    fn show_ids_and_a_connection_name_are_global() {
        let options = parse(&["members", "--show-ids", "--connection", "UCSF HPC"]);
        assert!(options.show_ids);
        assert_eq!(options.connection.as_deref(), Some("UCSF HPC"));
        assert!(!parse(&["members"]).show_ids);
    }

    /// Every channel argument takes `methods`, `#methods`, `team/methods` or an ID, verbatim:
    /// the daemon strips the `#`, so the CLI must not.
    #[test]
    fn every_channel_argument_takes_a_selector_with_or_without_the_hash() {
        for channel in ["methods", "#methods", "analysis-lab/methods", ID] {
            let history = parse(&["history", channel, "--latest"]);
            let CrewCommand::History(args) = history.command else {
                panic!("history")
            };
            assert_eq!(args.channel, channel);

            let CrewCommand::Search { channel: got, .. } =
                parse(&["search", channel, "counts"]).command
            else {
                panic!("search")
            };
            assert_eq!(got, channel);

            let CrewCommand::Watch(args) = parse(&["watch", channel]).command else {
                panic!("watch")
            };
            assert_eq!(args.channel, channel);

            let CrewCommand::Send(args) = parse(&["send", channel, "--text", "hi"]).command else {
                panic!("send")
            };
            assert_eq!(args.channel, channel);

            let CrewCommand::Channels(ChannelCommand::Archive { channel: got }) =
                parse(&["channels", "archive", channel]).command
            else {
                panic!("archive")
            };
            assert_eq!(got, channel);

            let CrewCommand::Channels(ChannelCommand::Rename { channel: got, name }) =
                parse(&["channels", "rename", channel, "data"]).command
            else {
                panic!("channel rename")
            };
            assert_eq!((got.as_str(), name.as_str()), (channel, "data"));

            let CrewCommand::Ownership(OwnershipCommand::Accept { channel: got }) =
                parse(&["ownership", "accept", channel]).command
            else {
                panic!("ownership accept")
            };
            assert_eq!(got, channel);

            let CrewCommand::Files(FileCommand::Upload { channel: got, .. }) =
                parse(&["files", "upload", channel, "./counts.csv"]).command
            else {
                panic!("files upload")
            };
            assert_eq!(got, channel);

            let CrewCommand::Files(FileCommand::Reference { channel: got, .. }) =
                parse(&["files", "reference", channel, "/data", "--label", "Data"]).command
            else {
                panic!("files reference")
            };
            assert_eq!(got, channel);

            let CrewCommand::Tasks(TaskCommand::Start {
                channel: got,
                context_channels,
                ..
            }) = parse(&[
                "tasks",
                "start",
                channel,
                "--text",
                "Sum the counts",
                "--provider",
                "p",
                "--model",
                "m",
                "--context-channel",
                channel,
                "--context-channel",
                "raw",
                "--allow-posting",
            ])
            .command
            else {
                panic!("tasks start")
            };
            assert_eq!(got, channel);
            assert_eq!(context_channels, [channel, "raw"]);

            let CrewCommand::Grants(GrantCommand::Grant {
                channel: got,
                context_channels,
                ..
            }) = parse(&[
                "grants",
                "grant",
                "20260924_2",
                channel,
                "--context-channel",
                channel,
            ])
            .command
            else {
                panic!("grants grant")
            };
            assert_eq!(got, channel);
            assert_eq!(context_channels, [channel]);

            let CrewCommand::Invites(InvitationCommand::Create {
                channel: Some(got),
                team: None,
                ..
            }) = parse(&["invites", "create", "@bob", "--channel", channel]).command
            else {
                panic!("invites create --channel")
            };
            assert_eq!(got, channel);
        }
    }

    #[test]
    fn mark_read_takes_an_optional_cursor() {
        let CrewCommand::Channels(ChannelCommand::MarkRead { channel, cursor }) =
            parse(&["channels", "mark-read", "#methods"]).command
        else {
            panic!("mark-read")
        };
        assert_eq!((channel.as_str(), cursor), ("#methods", None));

        let CrewCommand::Channels(ChannelCommand::MarkRead { cursor, .. }) =
            parse(&["channels", "mark-read", "methods", "m-42"]).command
        else {
            panic!("mark-read with cursor")
        };
        assert_eq!(cursor.as_deref(), Some("m-42"));
    }

    #[test]
    fn every_person_argument_takes_a_username_or_an_id() {
        for person in ["@bob", "bob", ID] {
            let CrewCommand::Invites(InvitationCommand::Create {
                person: got,
                team: Some(team),
                ..
            }) = parse(&["invites", "create", person, "--team", "analysis-lab"]).command
            else {
                panic!("invites create --team")
            };
            assert_eq!((got.as_str(), team.as_str()), (person, "analysis-lab"));

            let CrewCommand::RemoveMember {
                channel,
                member,
                former,
            } = parse(&["remove-member", "#methods", person]).command
            else {
                panic!("remove-member")
            };
            assert_eq!(
                (channel.as_str(), member.as_str(), former),
                ("#methods", person, false)
            );

            let CrewCommand::Ownership(OwnershipCommand::Offer { channel, successor }) =
                parse(&["ownership", "offer", "methods", person]).command
            else {
                panic!("ownership offer")
            };
            assert_eq!((channel.as_str(), successor.as_str()), ("methods", person));
        }
        let CrewCommand::RemoveMember { former, .. } =
            parse(&["remove-member", "methods", "@bob", "--former"]).command
        else {
            panic!("remove-member --former")
        };
        assert!(former);
    }

    #[test]
    fn team_arguments_take_a_name_or_handle() {
        for team in ["analysis-lab", "Analysis Lab", ID] {
            let CrewCommand::Channels(ChannelCommand::List { team: got }) =
                parse(&["channels", "list", "--team", team]).command
            else {
                panic!("channels list")
            };
            assert_eq!(got.as_deref(), Some(team));

            let CrewCommand::Channels(ChannelCommand::Create { team: got, .. }) =
                parse(&["channels", "create", "Data Analysis", "--team", team]).command
            else {
                panic!("channels create")
            };
            assert_eq!(got, team);

            let CrewCommand::Teams(TeamCommand::Rename { team: got, name }) =
                parse(&["teams", "rename", team, "Analysis Lab 2"]).command
            else {
                panic!("teams rename")
            };
            assert_eq!((got.as_str(), name.as_str()), (team, "Analysis Lab 2"));
        }
        let CrewCommand::Workspace(WorkspaceCommand::Rename { name }) =
            parse(&["workspace", "rename", "lab"]).command
        else {
            panic!("workspace rename")
        };
        assert_eq!(name, "lab");
    }

    #[test]
    fn invites_create_needs_exactly_one_target_and_accept_needs_no_argument() {
        assert!(refused(&["invites", "create", "@bob"]));
        assert!(refused(&[
            "invites",
            "create",
            "@bob",
            "--team",
            "lab",
            "--channel",
            "lab/x"
        ]));
        let CrewCommand::Invites(InvitationCommand::Accept { invitation }) =
            parse(&["invites", "accept"]).command
        else {
            panic!("invites accept")
        };
        assert!(invitation.is_none());
        let CrewCommand::Invites(InvitationCommand::Accept { invitation }) =
            parse(&["invites", "accept", "analysis-lab/methods"]).command
        else {
            panic!("invites accept NAME")
        };
        assert_eq!(invitation.as_deref(), Some("analysis-lab/methods"));
    }

    #[test]
    fn revoke_takes_a_username_with_confirm_or_a_bare_id() {
        let CrewCommand::Enroll(EnrollmentCommand::Revoke { member, confirm }) =
            parse(&["enroll", "revoke", "@bob", "--confirm", "@bob"]).command
        else {
            panic!("revoke --confirm")
        };
        assert_eq!(
            (member.as_str(), confirm.as_deref()),
            ("@bob", Some("@bob"))
        );

        // Parsing never demands --confirm: whether a terminal can ask instead is decided at
        // run time, and an ID never needs it.
        let CrewCommand::Enroll(EnrollmentCommand::Revoke { member, confirm }) =
            parse(&["enroll", "revoke", ID]).command
        else {
            panic!("revoke ID")
        };
        assert_eq!((member.as_str(), confirm), (ID, None));
    }

    #[test]
    fn join_by_invitation_commands_parse() {
        let CrewCommand::Connections(ConnectionCommand::JoinInvitation(args)) = parse(&[
            "connections",
            "join-invitation",
            "-",
            "--username",
            "bob",
            "--mode",
            "private",
            "--institution",
            "ucsf",
            "--yes",
        ])
        .command
        else {
            panic!("join-invitation")
        };
        assert_eq!(args.input, PathBuf::from("-"));
        assert_eq!(args.username.as_deref(), Some("bob"));
        assert!(matches!(args.mode, Some(PrivacyMode::Private)));
        assert_eq!(args.institution_id.as_deref(), Some("ucsf"));
        assert!(args.yes && !args.preview);
        assert!(refused(&[
            "connections",
            "join-invitation",
            "-",
            "--preview",
            "--yes"
        ]));
        assert!(refused(&[
            "connections",
            "join-invitation",
            "-",
            "--remote-execution"
        ]));

        let CrewCommand::Connections(ConnectionCommand::Invitation { invitee }) =
            parse(&["connections", "invitation", "--for", "@bob"]).command
        else {
            panic!("connections invitation")
        };
        assert_eq!(invitee.as_deref(), Some("@bob"));

        let CrewCommand::Join(args) = parse(&["join"]).command else {
            panic!("join")
        };
        assert!(!args.no_wait);
        let CrewCommand::Join(args) = parse(&["join", "--no-wait"]).command else {
            panic!("join --no-wait")
        };
        assert!(args.no_wait);
    }

    #[test]
    fn host_admission_commands_parse() {
        let CrewCommand::Enroll(EnrollmentCommand::Invite(args)) =
            parse(&["enroll", "invite", "@bob", "--add-device"]).command
        else {
            panic!("enroll invite")
        };
        assert_eq!(args.username.as_deref(), Some("@bob"));
        assert!(args.add_device && args.uid.is_none() && args.public_key.is_none());

        assert!(matches!(
            parse(&["enroll", "pending"]).command,
            CrewCommand::Enroll(EnrollmentCommand::Pending)
        ));

        let CrewCommand::Enroll(EnrollmentCommand::Approve {
            person,
            code,
            replace,
        }) = parse(&["enroll", "approve", "@bob", "7QK2-M9XA-3JTP-WZ4D"]).command
        else {
            panic!("enroll approve")
        };
        assert_eq!(
            (person.as_str(), code.as_str(), replace),
            ("@bob", "7QK2-M9XA-3JTP-WZ4D", false)
        );

        let CrewCommand::Enroll(EnrollmentCommand::Cancel { person }) =
            parse(&["enroll", "cancel", "@bob"]).command
        else {
            panic!("enroll cancel")
        };
        assert_eq!(person, "@bob");
    }

    #[test]
    fn the_deprecated_token_forms_still_parse() {
        let CrewCommand::Enroll(EnrollmentCommand::Invite(args)) = parse(&[
            "enroll",
            "invite",
            "--uid",
            "12345",
            "--public-key",
            "abcd",
            "--existing-principal",
            ID,
        ])
        .command
        else {
            panic!("legacy invite")
        };
        assert_eq!(args.uid, Some(12345));
        assert_eq!(args.public_key.as_deref(), Some("abcd"));
        assert_eq!(args.existing_principal.as_deref(), Some(ID));
        assert!(args.username.is_none());

        let CrewCommand::Enroll(EnrollmentCommand::Accept(input)) =
            parse(&["enroll", "accept", "--token-stdin"]).command
        else {
            panic!("legacy accept")
        };
        assert!(input.token_stdin);
    }

    #[test]
    fn a_username_and_a_uid_are_one_or_the_other() {
        // username conflicts with uid; uid and public_key are required without a username.
        assert!(refused(&["enroll", "invite"]));
        assert!(refused(&["enroll", "invite", "@bob", "--uid", "12345"]));
        assert!(refused(&["enroll", "invite", "--uid", "12345"]));
        assert!(refused(&["enroll", "invite", "--public-key", "abcd"]));
        assert!(refused(&[
            "enroll",
            "invite",
            "--uid",
            "1",
            "--public-key",
            "k",
            "--add-device"
        ]));
        assert!(refused(&[
            "enroll",
            "invite",
            "@bob",
            "--existing-principal",
            ID
        ]));
        assert!(refused(&["enroll", "invite", "@bob", "--public-key", "k"]));
    }

    #[test]
    fn the_deprecated_forms_are_hidden_from_help() {
        let mut command = TestCli::command();
        let enroll = command
            .find_subcommand_mut("enroll")
            .expect("enroll subcommand");
        let help = enroll.render_long_help().to_string();
        assert!(help.contains("approve") && help.contains("pending"));
        assert!(!help.contains("accept"), "enroll accept is hidden: {help}");
        let invite = enroll
            .find_subcommand_mut("invite")
            .expect("invite subcommand");
        let help = invite.render_long_help().to_string();
        assert!(help.contains("--add-device"));
        for hidden in ["--uid", "--public-key", "--existing-principal"] {
            assert!(!help.contains(hidden), "{hidden} is hidden: {help}");
        }
    }
}
