//! Every action of the two person tools (account ownership AO-12, "Names"): its parameters,
//! an example, and the function that runs it. `crew__read describe` serves the first two, so a
//! chat's own tool schemas stay small, and [`check_params`] holds every call to the same
//! parameters, so what `describe` says is what a call is checked against.
//!
//! Every action AO-12 lists runs. An action whose `run` is `None` (one added to the plumbing's
//! lists before it has a runner here) answers [`NOT_YET`].

use biorouter::crew::tool_host::PersonTool;
use serde_json::{json, Map, Value};

use super::{channels, messages, setup, tasks, workspace, Run};

/// What a parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A string.
    Text,
    /// `true` or `false`.
    Flag,
    /// A whole number, zero or more.
    Number,
    /// A list of strings.
    Texts,
    /// One of these words.
    Choice(&'static [&'static str]),
}

/// One parameter of an action.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Param {
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    pub(crate) required: bool,
    pub(crate) about: &'static str,
}

/// One action of `crew__read` or `crew__change`.
pub(crate) struct Action {
    pub(crate) tool: PersonTool,
    pub(crate) name: &'static str,
    /// One sentence: what it does.
    pub(crate) summary: &'static str,
    pub(crate) params: &'static [Param],
    /// An example `params` object, as JSON text.
    pub(crate) example: &'static str,
    /// The action, or `None` while it is not available in a chat yet.
    pub(crate) run: Option<Run>,
}

/// Said for an action listed in AO-12 that a chat cannot run yet.
pub(crate) const NOT_YET: &str =
    "That action isn't available in a chat yet. Use the Crew view or biorouter crew for it.";

const fn p(name: &'static str, kind: Kind, about: &'static str) -> Param {
    Param {
        name,
        kind,
        required: false,
        about,
    }
}

const fn req(name: &'static str, kind: Kind, about: &'static str) -> Param {
    Param {
        name,
        kind,
        required: true,
        about,
    }
}

use Kind::{Choice, Flag, Number, Text, Texts};

const CONNECTION: Param = p(
    "connection",
    Text,
    "The saved connection: its name, SSH login or ID. Omit it when only one is saved.",
);
const SERVER: Param = p(
    "server",
    Text,
    "The server's SSH login, like you@hpc.example.edu.",
);
const SERVER_REQUIRED: Param = req(
    "server",
    Text,
    "The server's SSH login, like you@hpc.example.edu.",
);
const PORT: Param = p("port", Number, "The SSH port, when it is not 22.");
const IDENTITY_FILE: Param = p(
    "identity_file",
    Text,
    "The SSH key file on this computer to sign in with.",
);
const PROXY_JUMP: Param = p(
    "proxy_jump",
    Text,
    "A jump host to reach the server through.",
);
const AGENT_ACCESS: Param = p(
    "agent_access",
    Choice(&["off", "folder", "full"]),
    "What agents may do on the server: off, folder (a confined work folder) or full (run as the person).",
);
const JOB_ID: Param = req(
    "job_id",
    Text,
    "The setup job's ID, from the action that started it.",
);
const MODE: Param = req(
    "mode",
    Choice(&["private", "public"]),
    "The privacy: private or public.",
);
const INSTITUTION: Param = p(
    "institution_id",
    Text,
    "The institution, like ucsf. A private workspace needs one.",
);
const CHANNEL: Param = req("channel", Text, "The channel: #name, team/name or its ID.");
const TEAM_OPTIONAL: Param = p("team", Text, "The team: its name or ID.");
const TEAM: Param = req("team", Text, "The team: its name or ID.");
const MEMBER: Param = req("member", Text, "The person: @username or their ID.");
const USERNAME: Param = req(
    "username",
    Text,
    "The person's account on the server, like @bob.",
);
const NAME: Param = req("name", Text, "The new name.");

/// Every action, reads first, in AO-12's order.
pub(crate) static ACTIONS: &[Action] = &[
    // ----------------------------------------------------------------------------- crew__read
    Action {
        tool: PersonTool::Read,
        name: "describe",
        summary: "Return an action's parameters and an example.",
        params: &[req("action", Text, "The action to describe, like setup or members.")],
        example: r#"{"action":"setup_check"}"#,
        run: Some(setup::describe),
    },
    Action {
        tool: PersonTool::Read,
        name: "status",
        summary: "List this computer's saved Crew connections and how each stands.",
        params: &[],
        example: "{}",
        run: Some(setup::status),
    },
    Action {
        tool: PersonTool::Read,
        name: "connections_show",
        summary: "Show one saved connection: its server, privacy, agent access and new devices.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(setup::connections_show),
    },
    Action {
        tool: PersonTool::Read,
        name: "connections_preview",
        summary: "Read an invitation the person pasted, before anything is saved.",
        params: &[
            req("invitation", Text, "The invitation text the host sent, with its brcrew1: line."),
            p("username", Text, "The person's account on the server, when it is not the one invited."),
            p("mode", Choice(&["private", "public"]), "How this computer would treat the workspace."),
            INSTITUTION,
        ],
        example: r#"{"invitation":"brcrew1:..."}"#,
        run: Some(setup::connections_preview),
    },
    Action {
        tool: PersonTool::Read,
        name: "setup_check",
        summary: "Check a server and the Crew program in the person's account there, and list the workspaces it hosts or belongs to. Changes nothing on the server; a server no saved connection names needs the person's approval, because it signs in as them.",
        params: &[SERVER, CONNECTION, PORT, IDENTITY_FILE, PROXY_JUMP],
        example: r#"{"server":"you@hpc.example.edu"}"#,
        run: Some(setup::setup_check),
    },
    Action {
        tool: PersonTool::Read,
        name: "setup_status",
        summary: "Where a setup job stands: its stage, what it found and what it did.",
        params: &[JOB_ID],
        example: r#"{"job_id":"5b1f..."}"#,
        run: Some(setup::setup_status),
    },
    Action {
        tool: PersonTool::Read,
        name: "workspace_show",
        summary: "The workspace as the person sees it: its teams, channels, people and settings.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(workspace::workspace_show),
    },
    Action {
        tool: PersonTool::Read,
        name: "members",
        summary: "The workspace's members, with the host first.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(workspace::members),
    },
    Action {
        tool: PersonTool::Read,
        name: "enroll_pending",
        summary: "Who is waiting to join the workspace (the host sees this).",
        params: &[CONNECTION],
        example: "{}",
        run: Some(workspace::enroll_pending),
    },
    Action {
        tool: PersonTool::Read,
        name: "invites_list",
        summary: "The person's pending invitations to teams and channels.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(workspace::invites_list),
    },
    Action {
        tool: PersonTool::Read,
        name: "teams_list",
        summary: "The teams the person is in.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(channels::teams_list),
    },
    Action {
        tool: PersonTool::Read,
        name: "channels_list",
        summary: "The person's channels, and in an open workspace the ones they have not joined.",
        params: &[CONNECTION, TEAM_OPTIONAL],
        example: r#"{"team":"analysis-lab"}"#,
        run: Some(channels::channels_list),
    },
    Action {
        tool: PersonTool::Read,
        name: "history",
        summary: "A channel's messages, newest last; with after, only newer ones (watch).",
        params: &[
            CONNECTION,
            CHANNEL,
            p("after", Text, "A message's sequence token: only messages after it."),
            p("before", Text, "A message's sequence token: only messages before it."),
            p("limit", Number, "At most this many messages."),
        ],
        example: r##"{"channel":"#methods","limit":20}"##,
        run: Some(messages::history),
    },
    Action {
        tool: PersonTool::Read,
        name: "search",
        summary: "Search one channel's messages.",
        params: &[
            CONNECTION,
            CHANNEL,
            req("query", Text, "The words to look for."),
            p("limit", Number, "At most this many messages."),
            p("after", Text, "A message's sequence token: only messages after it."),
        ],
        example: r##"{"channel":"#methods","query":"batch effect"}"##,
        run: Some(messages::search),
    },
    Action {
        tool: PersonTool::Read,
        name: "files_list",
        summary: "The files shared in a channel, newest first.",
        params: &[
            CONNECTION,
            CHANNEL,
            p("limit", Number, "At most this many files."),
        ],
        example: r##"{"channel":"#methods"}"##,
        run: Some(messages::files_list),
    },
    Action {
        tool: PersonTool::Read,
        name: "files_status",
        summary: "Where a file transfer stands.",
        params: &[req("transfer_id", Text, "The transfer's ID, from files_share or files_download.")],
        example: r#"{"transfer_id":"..."}"#,
        run: Some(messages::files_status),
    },
    Action {
        tool: PersonTool::Read,
        name: "context_show",
        summary: "The workspace's context document, which members wrote about the server.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(messages::context_show),
    },
    Action {
        tool: PersonTool::Read,
        name: "privacy_show",
        summary: "The privacy in force: the connection's own setting, the workspace's, and each channel's.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(workspace::privacy_show),
    },
    Action {
        tool: PersonTool::Read,
        name: "profile_show",
        summary: "The person's own profile in the workspace.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(workspace::profile_show),
    },
    Action {
        tool: PersonTool::Read,
        name: "profile_devices",
        summary: "The computers enrolled as the person, which one is this computer, and which are new.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(workspace::profile_devices),
    },
    Action {
        tool: PersonTool::Read,
        name: "defaults_show",
        summary: "The model Crew tasks start on, and the default model for new chats.",
        params: &[],
        example: "{}",
        run: Some(setup::defaults_show),
    },
    Action {
        tool: PersonTool::Read,
        name: "tasks_list",
        summary: "The person's agent tasks on this connection and how each stands.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(tasks::tasks_list),
    },
    Action {
        tool: PersonTool::Read,
        name: "tasks_watch",
        summary: "A task's progress since a point, waiting up to 20 seconds for news.",
        params: &[
            CONNECTION,
            req("task", Text, "The task's ID."),
            p("after", Text, "Only progress after this token."),
            p("wait_seconds", Number, "How long to wait for news, at most 20."),
        ],
        example: r#"{"task":"run-...","wait_seconds":10}"#,
        run: Some(tasks::tasks_watch),
    },
    Action {
        tool: PersonTool::Read,
        name: "grants_list",
        summary: "The chats and tasks with Crew access through a connection, and the chats the person let read it as them.",
        params: &[CONNECTION],
        example: "{}",
        run: Some(workspace::grants_list),
    },
    // --------------------------------------------------------------------------- crew__change
    Action {
        tool: PersonTool::Change,
        name: "setup",
        summary: "Install or update Crew in the person's account on a server, restart the workspaces they host there on it, and for a saved connection, connect it.",
        params: &[
            SERVER,
            CONNECTION,
            PORT,
            IDENTITY_FILE,
            PROXY_JUMP,
            AGENT_ACCESS,
            p("keep_older", Flag, "Keep an older Crew program instead of replacing it."),
            p("replace_existing", Flag, "Replace a Crew program that does not run or name its version."),
            p("restart_workspaces", Flag, "true restarts the hosted workspaces even with others connected; false only reports them."),
            p("restart_workspace_ids", Texts, "IDs of hosted workspaces to restart even with others connected, from setup_check, whatever restart_workspaces says."),
            p("start_workspaces", Texts, "IDs of stopped workspaces the person hosts there to start, from setup_check."),
        ],
        example: r#"{"server":"you@hpc.example.edu"}"#,
        run: Some(setup::setup),
    },
    Action {
        tool: PersonTool::Change,
        name: "setup_cancel",
        summary: "Stop a setup job.",
        params: &[JOB_ID],
        example: r#"{"job_id":"5b1f..."}"#,
        run: Some(setup::setup_cancel),
    },
    Action {
        tool: PersonTool::Change,
        name: "host",
        summary: "Host a new workspace on a server as the person, and save and join it here.",
        params: &[
            SERVER_REQUIRED,
            req("name", Text, "The workspace's name, like lab."),
            MODE,
            INSTITUTION,
            AGENT_ACCESS,
            PORT,
            IDENTITY_FILE,
            PROXY_JUMP,
            p("fix_storage_permissions", Flag, "Take other accounts' write access off the person's folders above where the workspace is kept, when Crew refuses to start there."),
        ],
        example: r#"{"server":"you@hpc.example.edu","name":"lab","mode":"private","institution_id":"ucsf"}"#,
        run: Some(setup::host),
    },
    Action {
        tool: PersonTool::Change,
        name: "open",
        summary: "Open a workspace a setup_check job found on the server, as the person: save it here and join it.",
        params: &[
            JOB_ID,
            req("workspace", Text, "The workspace's name or ID, as setup_check listed it."),
            p("host", Text, "The host's username, when two hosts have a workspace of that name."),
            AGENT_ACCESS,
            p("local_name", Text, "This computer's name for the connection."),
        ],
        example: r#"{"job_id":"5b1f...","workspace":"baranzini-lab"}"#,
        run: Some(setup::open),
    },
    Action {
        tool: PersonTool::Change,
        name: "join",
        summary: "Join a workspace: from an invitation the person pasted, or a saved connection that is ready to join.",
        params: &[
            CONNECTION,
            p("invitation", Text, "The invitation text the host sent, with its brcrew1: line."),
            p("username", Text, "The person's account on the server, when it is not the one invited."),
            p("mode", Choice(&["private", "public"]), "How this computer treats the workspace."),
            INSTITUTION,
            p("local_name", Text, "This computer's name for the connection."),
        ],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(setup::join),
    },
    Action {
        tool: PersonTool::Change,
        name: "rejoin",
        summary: "Add this computer to a workspace again with a new key, as the account on the server.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(setup::rejoin),
    },
    Action {
        tool: PersonTool::Change,
        name: "connect",
        summary: "Connect a saved connection to its workspace.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(setup::connect),
    },
    Action {
        tool: PersonTool::Change,
        name: "disconnect",
        summary: "Disconnect a saved connection.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(setup::disconnect),
    },
    Action {
        tool: PersonTool::Change,
        name: "connections_remove",
        summary: "Remove a saved connection and its key from this computer.",
        params: &[CONNECTION],
        example: r#"{"connection":"baranzini-lab"}"#,
        run: Some(setup::connections_remove),
    },
    Action {
        tool: PersonTool::Change,
        name: "connections_update",
        summary: "Change a saved connection's name or SSH route.",
        params: &[
            CONNECTION,
            p("name", Text, "This computer's new name for the connection."),
            p("ssh_target", Text, "A new SSH login for the same server."),
            PORT,
            IDENTITY_FILE,
            PROXY_JUMP,
        ],
        example: r#"{"connection":"baranzini-lab","name":"lab"}"#,
        run: Some(setup::connections_update),
    },
    Action {
        tool: PersonTool::Change,
        name: "connections_set_access",
        summary: "What agents may do on the server through a connection: off, folder or full.",
        params: &[
            CONNECTION,
            req("access", Choice(&["off", "folder", "full"]), "off, folder (a confined work folder) or full (run as the person)."),
            p("folder", Text, "The work folder's full path on the server, for folder or full."),
        ],
        example: r#"{"access":"folder","folder":"/home/you/crew-work/lab"}"#,
        run: Some(setup::connections_set_access),
    },
    Action {
        tool: PersonTool::Change,
        name: "workspace_rename",
        summary: "Rename the workspace for everyone.",
        params: &[CONNECTION, NAME],
        example: r#"{"name":"lab-2"}"#,
        run: Some(workspace::workspace_rename),
    },
    Action {
        tool: PersonTool::Change,
        name: "workspace_delete",
        summary: "Delete the workspace on its server for everyone (its host only), and remove it here.",
        params: &[CONNECTION],
        example: r#"{"connection":"old-lab"}"#,
        run: Some(workspace::workspace_delete),
    },
    Action {
        tool: PersonTool::Change,
        name: "workspace_governance",
        summary: "Choose who manages teams, channels and people: open (every member) or managed.",
        params: &[
            CONNECTION,
            req("governance", Choice(&["open", "managed"]), "open or managed."),
        ],
        example: r#"{"governance":"open"}"#,
        run: Some(workspace::workspace_governance),
    },
    Action {
        tool: PersonTool::Change,
        name: "context_set",
        summary: "Replace the workspace's context document, which every member's agent reads.",
        params: &[CONNECTION, req("text", Text, "The whole new document, in Markdown, up to 64 KB.")],
        example: r##"{"text":"# Server\nRun jobs with sbatch."}"##,
        run: Some(workspace::context_set),
    },
    Action {
        tool: PersonTool::Change,
        name: "enroll_invite",
        summary: "Invite a person on the server to join the workspace by their account.",
        params: &[
            CONNECTION,
            USERNAME,
            p("add_device", Flag, "Invite an existing member to add another computer."),
        ],
        example: r#"{"username":"@bob"}"#,
        run: Some(workspace::enroll_invite),
    },
    Action {
        tool: PersonTool::Change,
        name: "enroll_cancel",
        summary: "Cancel a person's invitation to join.",
        params: &[CONNECTION, USERNAME],
        example: r#"{"username":"@bob"}"#,
        run: Some(workspace::enroll_cancel),
    },
    Action {
        tool: PersonTool::Change,
        name: "enroll_approve",
        summary: "Let in a person who joins from an older Biorouter, with the code they sent.",
        params: &[
            CONNECTION,
            USERNAME,
            req("code", Text, "The device code the person sent, like ABCD-EFGH-JKMN-PQRS."),
            p("replace", Flag, "Replace a code saved before."),
        ],
        example: r#"{"username":"@bob","code":"ABCD-EFGH-JKMN-PQRS"}"#,
        run: Some(workspace::enroll_approve),
    },
    Action {
        tool: PersonTool::Change,
        name: "enroll_revoke",
        summary: "Remove a person from the workspace for everyone: their membership, devices and agent grants.",
        params: &[CONNECTION, MEMBER],
        example: r#"{"member":"@bob"}"#,
        run: Some(workspace::enroll_revoke),
    },
    Action {
        tool: PersonTool::Change,
        name: "members_add",
        summary: "Add a member to a team or to channels.",
        params: &[
            CONNECTION,
            MEMBER,
            TEAM_OPTIONAL,
            p("channels", Texts, "Channels to add them to."),
        ],
        example: r##"{"member":"@bob","team":"analysis-lab","channels":["#methods"]}"##,
        run: Some(channels::members_add),
    },
    Action {
        tool: PersonTool::Change,
        name: "teams_create",
        summary: "Create a team.",
        params: &[CONNECTION, req("name", Text, "The team's name.")],
        example: r#"{"name":"analysis-lab"}"#,
        run: Some(channels::teams_create),
    },
    Action {
        tool: PersonTool::Change,
        name: "teams_rename",
        summary: "Rename a team.",
        params: &[CONNECTION, TEAM, NAME],
        example: r#"{"team":"analysis-lab","name":"analysis"}"#,
        run: Some(channels::teams_rename),
    },
    Action {
        tool: PersonTool::Change,
        name: "teams_join",
        summary: "Join a team in an open workspace.",
        params: &[CONNECTION, TEAM],
        example: r#"{"team":"analysis-lab"}"#,
        run: Some(channels::teams_join),
    },
    Action {
        tool: PersonTool::Change,
        name: "teams_delete",
        summary: "Delete a team for everyone.",
        params: &[CONNECTION, TEAM],
        example: r#"{"team":"analysis-lab"}"#,
        run: Some(channels::teams_delete),
    },
    Action {
        tool: PersonTool::Change,
        name: "channels_create",
        summary: "Create a channel in a team. It is restricted unless classification says public_safe.",
        params: &[
            CONNECTION,
            req("name", Text, "The channel's name."),
            p("team", Text, "The team: its name or ID. Omit it when the person is in one team."),
            p(
                "classification",
                Choice(&["restricted", "public_safe"]),
                "restricted (the default: public models never read it) or public_safe.",
            ),
        ],
        example: r#"{"name":"methods","team":"analysis-lab"}"#,
        run: Some(channels::channels_create),
    },
    Action {
        tool: PersonTool::Change,
        name: "channels_rename",
        summary: "Rename a channel.",
        params: &[CONNECTION, CHANNEL, NAME],
        example: r##"{"channel":"#methods","name":"methods-2"}"##,
        run: Some(channels::channels_rename),
    },
    Action {
        tool: PersonTool::Change,
        name: "channels_archive",
        summary: "Archive a channel for everyone.",
        params: &[CONNECTION, CHANNEL],
        example: r##"{"channel":"#methods"}"##,
        run: Some(channels::channels_archive),
    },
    Action {
        tool: PersonTool::Change,
        name: "channels_join",
        summary: "Join a channel in an open workspace.",
        params: &[CONNECTION, CHANNEL],
        example: r##"{"channel":"#methods"}"##,
        run: Some(channels::channels_join),
    },
    Action {
        tool: PersonTool::Change,
        name: "channels_delete",
        summary: "Delete a channel and its messages for everyone.",
        params: &[CONNECTION, CHANNEL],
        example: r##"{"channel":"#methods"}"##,
        run: Some(channels::channels_delete),
    },
    Action {
        tool: PersonTool::Change,
        name: "channels_read",
        summary: "Mark a channel read.",
        params: &[CONNECTION, CHANNEL],
        example: r##"{"channel":"#methods"}"##,
        run: Some(channels::channels_read),
    },
    Action {
        tool: PersonTool::Change,
        name: "invites_send",
        summary: "Invite a member to a team or a channel.",
        params: &[
            CONNECTION,
            MEMBER,
            TEAM_OPTIONAL,
            p("channel", Text, "The channel: #name, team/name or its ID."),
        ],
        example: r##"{"member":"@bob","channel":"#methods"}"##,
        run: Some(channels::invites_send),
    },
    Action {
        tool: PersonTool::Change,
        name: "invites_accept",
        summary: "Accept one of the person's invitations to a team or channel.",
        params: &[CONNECTION, p("invitation", Text, "The team or channel it is for, or its ID.")],
        example: r##"{"invitation":"#methods"}"##,
        run: Some(channels::invites_accept),
    },
    Action {
        tool: PersonTool::Change,
        name: "ownership_offer",
        summary: "Offer a channel's ownership to another member.",
        params: &[CONNECTION, CHANNEL, req("successor", Text, "The member: @username.")],
        example: r##"{"channel":"#methods","successor":"@bob"}"##,
        run: Some(channels::ownership_offer),
    },
    Action {
        tool: PersonTool::Change,
        name: "ownership_accept",
        summary: "Accept a channel's ownership offered to the person.",
        params: &[CONNECTION, CHANNEL],
        example: r##"{"channel":"#methods"}"##,
        run: Some(channels::ownership_accept),
    },
    Action {
        tool: PersonTool::Change,
        name: "remove_member",
        summary: "Remove a member from a channel.",
        params: &[
            CONNECTION,
            CHANNEL,
            MEMBER,
            p("former", Flag, "The member left the workspace."),
        ],
        example: r##"{"channel":"#methods","member":"@bob"}"##,
        run: Some(channels::remove_member),
    },
    Action {
        tool: PersonTool::Change,
        name: "profile_set",
        summary: "Change the person's own profile: their nickname or avatar.",
        params: &[
            CONNECTION,
            p("nickname", Text, "The name people see."),
            p("avatar", Text, "The avatar: an emoji or a short text."),
        ],
        example: r#"{"nickname":"Sam"}"#,
        run: Some(workspace::profile_set),
    },
    Action {
        tool: PersonTool::Change,
        name: "profile_revoke_device",
        summary: "Revoke one of the person's own computers in the workspace, by its fingerprint, or this computer.",
        params: &[
            CONNECTION,
            p("fingerprint", Text, "The device's fingerprint, as profile_devices lists it."),
            p("this_device", Flag, "Revoke this computer's own device."),
        ],
        example: r#"{"fingerprint":"3F2A 9C1E 77B0 D4E1"}"#,
        run: Some(workspace::profile_revoke_device),
    },
    Action {
        tool: PersonTool::Change,
        name: "send",
        summary: "Post a message in a channel as the person.",
        params: &[CONNECTION, CHANNEL, req("text", Text, "The message.")],
        example: r##"{"channel":"#methods","text":"The run finished."}"##,
        run: Some(messages::send_message),
    },
    Action {
        tool: PersonTool::Change,
        name: "files_share",
        summary: "Share a file from this computer in a channel.",
        params: &[
            CONNECTION,
            CHANNEL,
            req("path", Text, "The file's path on this computer."),
            p("text", Text, "A message to post with it."),
        ],
        example: r##"{"channel":"#methods","path":"~/results/plot.png"}"##,
        run: Some(messages::files_share),
    },
    Action {
        tool: PersonTool::Change,
        name: "files_download",
        summary: "Download a file shared in a channel to this computer.",
        params: &[
            CONNECTION,
            req("file", Text, "The shared file's ID, or its name with channel."),
            p("channel", Text, "The channel it was shared in, when file is a name."),
            p("destination", Text, "Where to save it on this computer: a file, or a folder. Your Downloads folder when omitted."),
        ],
        example: r##"{"file":"plot.png","channel":"#methods","destination":"~/Downloads"}"##,
        run: Some(messages::files_download),
    },
    Action {
        tool: PersonTool::Change,
        name: "files_reference",
        summary: "Post a reference to a file on the server in a channel.",
        params: &[
            CONNECTION,
            CHANNEL,
            req("path", Text, "The file's path on the server."),
            p("text", Text, "A message to post with it."),
        ],
        example: r##"{"channel":"#methods","path":"/data/lab/results.csv"}"##,
        run: Some(messages::files_reference),
    },
    Action {
        tool: PersonTool::Change,
        name: "tasks_ask",
        summary: "Start an agent task in a channel, with its own Crew access.",
        params: &[
            CONNECTION,
            CHANNEL,
            req("prompt", Text, "What to ask the agent."),
            p("context_channels", Texts, "Other channels it may read."),
            p("provider", Text, "The model's provider."),
            p("model", Text, "The model."),
        ],
        example: r##"{"channel":"#methods","prompt":"Summarize this week's runs."}"##,
        run: Some(tasks::tasks_ask),
    },
    Action {
        tool: PersonTool::Change,
        name: "tasks_cancel",
        summary: "Stop an agent task.",
        params: &[CONNECTION, req("task", Text, "The task's ID.")],
        example: r#"{"task":"run-..."}"#,
        run: Some(tasks::tasks_cancel),
    },
    Action {
        tool: PersonTool::Change,
        name: "grants_revoke",
        summary: "Take back a chat's Crew access through a connection, or with reads, its permission to read the workspace as the person.",
        params: &[
            CONNECTION,
            req("session", Text, "The chat's ID, as grants_list lists it."),
            p("reads", Flag, "Take back the chat's permission to read the workspace as the person instead."),
        ],
        example: r#"{"session":"20260924_2","reads":true}"#,
        run: Some(workspace::grants_revoke),
    },
    Action {
        tool: PersonTool::Change,
        name: "privacy_set_personal",
        summary: "How this computer treats the workspace: private or public.",
        params: &[CONNECTION, MODE, INSTITUTION],
        example: r#"{"mode":"private","institution_id":"ucsf"}"#,
        run: Some(workspace::privacy_set_personal),
    },
    Action {
        tool: PersonTool::Change,
        name: "privacy_set_workspace",
        summary: "The workspace's privacy for everyone (its host only): private, or public allowed.",
        params: &[CONNECTION, MODE, INSTITUTION],
        example: r#"{"mode":"private","institution_id":"ucsf"}"#,
        run: Some(workspace::privacy_set_workspace),
    },
    Action {
        tool: PersonTool::Change,
        name: "defaults_set",
        summary: "The model Crew tasks start on from now on.",
        params: &[
            req("provider", Text, "The model's provider, like versa_azure."),
            req("model", Text, "The model, like gpt-5.5."),
        ],
        example: r#"{"provider":"versa_azure","model":"gpt-5.5"}"#,
        run: Some(setup::defaults_set),
    },
    Action {
        tool: PersonTool::Change,
        name: "defaults_clear",
        summary: "Crew tasks go back to the default model for new chats.",
        params: &[],
        example: "{}",
        run: Some(setup::defaults_clear),
    },
];

/// The action `name` of `tool`.
pub(crate) fn find(tool: PersonTool, name: &str) -> Option<&'static Action> {
    ACTIONS
        .iter()
        .find(|action| action.tool == tool && action.name == name)
}

/// The action named `name` in either tool, for `describe`.
pub(crate) fn find_any(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.name == name)
}

/// Whether the table and the plumbing's two lists name exactly the same actions, in order.
#[cfg(test)]
pub(crate) fn matches_the_tools() -> bool {
    let reads: Vec<&str> = ACTIONS
        .iter()
        .filter(|action| action.tool == PersonTool::Read)
        .map(|action| action.name)
        .collect();
    let changes: Vec<&str> = ACTIONS
        .iter()
        .filter(|action| action.tool == PersonTool::Change)
        .map(|action| action.name)
        .collect();
    reads == biorouter::crew::tool_host::READ_ACTIONS
        && changes == biorouter::crew::tool_host::CHANGE_ACTIONS
}

fn kind_schema(kind: Kind) -> Value {
    match kind {
        Text => json!({"type": "string"}),
        Flag => json!({"type": "boolean"}),
        Number => json!({"type": "integer", "minimum": 0}),
        Texts => json!({"type": "array", "items": {"type": "string"}}),
        Choice(words) => json!({"type": "string", "enum": words}),
    }
}

/// The action's parameters as a JSON schema.
pub(crate) fn schema(action: &Action) -> Value {
    let mut properties = Map::new();
    for param in action.params {
        let mut schema = kind_schema(param.kind);
        schema["description"] = json!(param.about);
        properties.insert(param.name.to_owned(), schema);
    }
    let required: Vec<&str> = action
        .params
        .iter()
        .filter(|param| param.required)
        .map(|param| param.name)
        .collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// What `crew__read describe` answers for `action`.
pub(crate) fn describe(action: &Action) -> Value {
    let example: Value = serde_json::from_str(action.example).unwrap_or_else(|_| json!({}));
    json!({
        "action": action.name,
        "tool": action.tool.name(),
        "summary": action.summary,
        "params": schema(action),
        "example": {"action": action.name, "params": example},
        "available": action.run.is_some(),
        "approval": match action.tool {
            PersonTool::Read => "none, except the first read of a workspace's content in this chat, which asks the person once",
            PersonTool::Change => "every call waits for the person to allow it once",
        },
    })
}

fn kind_fits(kind: Kind, value: &Value) -> bool {
    match kind {
        Text => value.is_string(),
        Flag => value.is_boolean(),
        Number => value.is_u64(),
        Texts => value
            .as_array()
            .is_some_and(|items| items.len() <= 64 && items.iter().all(Value::is_string)),
        Choice(words) => value.as_str().is_some_and(|word| words.contains(&word)),
    }
}

fn kind_words(kind: Kind) -> String {
    match kind {
        Text => "text".into(),
        Flag => "true or false".into(),
        Number => "a whole number".into(),
        Texts => "a list of texts".into(),
        Choice(words) => format!("one of {}", words.join(", ")),
    }
}

/// Check `params` against `action`'s parameters: no unknown names, every required one there,
/// each of its kind. `null` counts as absent. The sentence names the parameter and how to see
/// them all.
pub(crate) fn check_params(action: &Action, params: &Value) -> Result<(), String> {
    let Some(given) = params.as_object() else {
        return Err("params must be an object of the action's parameters.".into());
    };
    let help = format!(
        "Get its parameters with crew__read action describe, params {{\"action\": \"{}\"}}.",
        action.name
    );
    for (name, value) in given {
        let Some(param) = action.params.iter().find(|param| param.name == name) else {
            return Err(format!("{} takes no parameter {name}. {help}", action.name));
        };
        if !value.is_null() && !kind_fits(param.kind, value) {
            return Err(format!(
                "{} needs {name} as {}. {help}",
                action.name,
                kind_words(param.kind)
            ));
        }
        if let Some(text) = value.as_str() {
            if text.len() > 256 * 1024 {
                return Err(format!("{name} is too long."));
            }
        }
    }
    for param in action.params.iter().filter(|param| param.required) {
        if given.get(param.name).is_none_or(Value::is_null) {
            return Err(format!("{} needs {}. {help}", action.name, param.name));
        }
    }
    Ok(())
}
