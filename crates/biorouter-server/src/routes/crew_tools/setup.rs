//! The person tools' setup and connection actions (AO-12): `describe`, `status`, the saved
//! connections, setting Crew up on a server, hosting, opening and joining a workspace, adding
//! this computer again, connecting, and the model Crew tasks start on.
//!
//! Each calls what the matching route or `biorouter crew` command calls: [`CrewManager`]'s
//! methods, the setup service (`start_setup`, `setup_status`, `cancel_setup`) and the saved
//! configuration. A flow the CLI takes in several steps (`host`, `open`, `join`) takes the same
//! steps here, after one card that names everything it will do.

use anyhow::Result;
use biorouter::config::Config;
use biorouter::crew::authentication::{
    InvitationAdvanced, InvitationOutcome, InvitationOverrides, JoinState,
};
use biorouter::crew::{
    ClusterMode, Connection, CrewManager, SaveConnection, SetupArtifact, SetupInstalled,
    SetupRequest, SetupState, SetupStatus, SetupWorkspace, SetupWorkspaceRole, SetupWorkspaceState,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::{catalog, job_answer, job_target, remember_job_target, said, Card, Ctx, Progress};
use crate::routes::crew::connection_view;
use crate::routes::crew_authentication::InvitationSummary;

/// The configuration key holding the model Crew tasks start on, as `biorouter crew defaults`
/// and Settings write it.
pub(crate) const CREW_TASK_MODEL: &str = "CREW_TASK_MODEL";
/// Said for a setup job this computer no longer has.
const NO_JOB: &str = "That setup job is not on this computer any more. Check the server again with crew__read action setup_check.";

// ------------------------------------------------------------------------------------- reads

pub(super) fn describe(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = ctx.required("action")?;
        let action = catalog::find_any(&name).ok_or_else(|| {
            said(
                None,
                format!(
                    "There is no action {name}. crew__read: {}. crew__change: {}.",
                    biorouter::crew::tool_host::READ_ACTIONS.join(", "),
                    biorouter::crew::tool_host::CHANGE_ACTIONS.join(", ")
                ),
            )
        })?;
        Ok(catalog::describe(action))
    })
}

/// `biorouter crew status`: every saved connection as `GET /crew/connections` lists it. A
/// connection list carries no workspace content, so any chat gets it.
pub(super) fn status(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connections = ctx.crew()?.list().await;
        let mut views = Vec::with_capacity(connections.len());
        for connection in &connections {
            views.push(connection_view(connection).await?);
        }
        Ok(json!({ "connections": views }))
    })
}

pub(super) fn connections_show(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        Ok(serde_json::to_value(connection_view(&connection).await?)?)
    })
}

fn overrides(ctx: &Ctx) -> Result<InvitationOverrides> {
    let mode = ctx
        .text("mode")
        .map(|mode| serde_json::from_value::<ClusterMode>(json!(mode)))
        .transpose()?;
    Ok(InvitationOverrides {
        username: ctx
            .text("username")
            .map(|name| name.trim_start_matches('@').to_owned()),
        mode,
        institution_id: ctx.text("institution_id"),
        advanced: InvitationAdvanced {
            name: ctx.text("local_name"),
            ..InvitationAdvanced::default()
        },
        replace: None,
    })
}

/// `POST /crew/connections/from-invitation` with `preview: true`: what saving the invitation
/// would do, with nothing saved.
pub(super) fn connections_preview(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let text = ctx.required("invitation")?;
        match ctx
            .crew()?
            .connection_from_invitation(&text, true, overrides(ctx)?)
            .await?
        {
            InvitationOutcome::Preview(preview) => Ok(serde_json::to_value(
                InvitationSummary::new(*preview, &text),
            )?),
            InvitationOutcome::Saved(_) => Err(said(None, super::FAILED)),
        }
    })
}

/// Which server a setup job is for, from `server` (a login nothing is saved for) or a saved
/// `connection`, and whether a saved connection names it.
struct SetupTarget {
    target: Value,
    login: String,
    saved: Option<Connection>,
}

async fn setup_target(ctx: &Ctx) -> Result<SetupTarget> {
    let Some(server) = ctx.text("server") else {
        let connection = ctx.connection().await?;
        return Ok(SetupTarget {
            target: json!({"kind": "connection", "connection_id": connection.id}),
            login: connection.ssh_target.clone(),
            saved: Some(connection),
        });
    };
    if ctx.text("connection").is_some() {
        return Err(said(
            None,
            "Name the server or a saved connection, not both.",
        ));
    }
    if server.contains(char::is_whitespace) {
        return Err(said(
            None,
            "Name the server's SSH login, like you@hpc.example.edu.",
        ));
    }
    let mut target = json!({"kind": "server", "ssh_target": server});
    if let Some(port) = ctx.number("port") {
        target["port"] = json!(port);
    }
    if let Some(file) = ctx.text("identity_file") {
        if !file.starts_with('/') {
            return Err(said(
                None,
                "Give the SSH key file's full path on this computer.",
            ));
        }
        target["identity_file"] = json!(file);
    }
    if let Some(jump) = ctx.text("proxy_jump") {
        target["proxy_jump"] = json!(jump);
    }
    // A saved connection names this server only when the job takes its whole route: the same
    // login, port, key file and jump host. Any other route signs in somewhere the person never
    // saved, so it asks first (AO-12).
    let saved = ctx.crew()?.list().await.into_iter().find(|connection| {
        connection.ssh_target == server
            && connection.port.map(u64::from) == ctx.number("port")
            && connection.identity_file.as_deref() == target["identity_file"].as_str()
            && connection.proxy_jump.as_deref() == target["proxy_jump"].as_str()
    });
    Ok(SetupTarget {
        target,
        login: server,
        saved,
    })
}

/// Start the setup job `request` describes, remember its target for `open`, and wait for it
/// ([`Ctx::wait_for_job`]).
async fn start_job(ctx: &Ctx, request: Value) -> Result<Value> {
    let request: SetupRequest = serde_json::from_value(request).map_err(|_| {
        said(
            Some("crew_request_invalid"),
            "Those setup choices don't fit together. Get them with crew__read action describe.",
        )
    })?;
    let target = serde_json::to_value(&request.target)?;
    let started = ctx.crew()?.start_setup(request).await?;
    remember_job_target(&started.job_id, target);
    Ok(ctx.wait_for_job(&started.job_id).await)
}

/// `biorouter crew doctor`: check a server and the Crew program in the account there, and
/// list its workspaces. A check changes nothing on the server, but it signs in there as the
/// person, so a server no saved connection names needs their approval first (AO-12). A server
/// login is checked with this computer's prepared device, as `biorouter crew open` checks it,
/// so opening a workspace it finds needs no second sign-in.
pub(super) fn setup_check(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let SetupTarget {
            mut target,
            login,
            saved,
        } = setup_target(ctx).await?;
        if saved.is_none() {
            ctx.approve(
                Card::new(format!(
                    "Check {login} as you: sign in, look at the Crew program in your account and list the workspaces there"
                ))
                .line("It changes nothing on the server.")
                .show("server", login.clone()),
            )
            .await?;
        }
        if target["kind"] == "server" {
            let prepared = ctx.crew()?.prepare_device().await?;
            target["preparation_id"] = json!(prepared.preparation_id);
        }
        start_job(ctx, json!({"target": target, "intent": "check"})).await
    })
}

pub(super) fn setup_status(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let job = ctx.required("job_id")?;
        let status = biorouter::crew::setup_status(&job).ok_or_else(|| said(None, NO_JOB))?;
        Ok(job_answer(&status, false))
    })
}

/// `biorouter crew defaults show`: the model for Crew tasks and the default for new chats, as
/// stored.
pub(super) fn defaults_show(_ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let config = Config::global();
        let crew_tasks = config
            .get_param::<Value>(CREW_TASK_MODEL)
            .ok()
            .filter(|value| value["provider"].is_string() && value["model"].is_string());
        let provider = config.get_param::<String>("BIOROUTER_PROVIDER").ok();
        let model = config.get_param::<String>("BIOROUTER_MODEL").ok();
        let new_chats = match (provider, model) {
            (Some(provider), Some(model)) => json!({"provider": provider, "model": model}),
            _ => Value::Null,
        };
        let source = if crew_tasks.is_some() {
            "crew_tasks"
        } else {
            "new_chats"
        };
        Ok(json!({
            "crew_tasks": crew_tasks,
            "new_chats": new_chats,
            "source": source,
        }))
    })
}

// ----------------------------------------------------------------------------------- changes

/// The most recent listing of `login`'s workspaces this computer has, from any setup job.
fn last_listing(login: &str) -> Option<SetupStatus> {
    biorouter::crew::setup_jobs()
        .into_iter()
        .find(|job| job.server.login == login && job.workspaces_scan.is_some())
}

fn workspace_name(workspace: &SetupWorkspace) -> String {
    workspace
        .name
        .clone()
        .unwrap_or_else(|| workspace.workspace_id.clone())
}

/// What a setup will restart and start, from the last listing of the server (AO-12: "for
/// setup the workspaces it will restart and who is connected"). That listing is usually a
/// check's, taken with the program installed then, before this setup puts this app's in place;
/// so when the setup will install, every workspace the account hosts and runs is counted as
/// one it may restart, unless it runs a newer Crew than this app's.
fn restart_lines(
    login: &str,
    keep_older: bool,
    restart: Option<bool>,
    start: &[String],
) -> Vec<String> {
    let listing = last_listing(login);
    let ours = biorouter::crew::setup_artifact()
        .ok()
        .or_else(|| listing.as_ref().and_then(|job| job.artifact.clone()));
    let installs = listing
        .as_ref()
        .is_some_and(|job| installs_ours(job.installed.as_ref(), ours.as_ref(), keep_older));
    planned_restarts(
        listing.as_ref().map(|job| job.workspaces.as_slice()),
        installs
            .then(|| ours.map(|ours| ours.version))
            .flatten()
            .as_deref(),
        restart,
        start,
    )
}

/// `X.Y.Z`, numbers only, as setup compares versions.
fn version_numbers(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether a setup puts this app's program (`ours`) in place of `installed`, as setup decides:
/// not when it is the same program or a newer one, nor an older one the person keeps; yes when
/// what is installed can't be told.
fn installs_ours(
    installed: Option<&SetupInstalled>,
    ours: Option<&SetupArtifact>,
    keep_older: bool,
) -> bool {
    let Some(ours) = ours else {
        return false;
    };
    let Some(installed) = installed else {
        return true;
    };
    if installed.sha256.as_deref() == Some(ours.sha256.as_str()) {
        return false;
    }
    let theirs = installed.version.as_deref().and_then(version_numbers);
    match (theirs, version_numbers(&ours.version)) {
        (Some(theirs), Some(ours)) if theirs > ours => false,
        (Some(theirs), Some(ours)) if theirs < ours => !keep_older,
        _ => true,
    }
}

/// The card's lines for what a setup restarts and starts, from a listing's `workspaces`
/// (`None`: the server was never listed here). `installing` is this app's version when the
/// setup installs it, which makes every hosted running workspace on an older Crew, or one that
/// names none, one it restarts; otherwise only those the listing found on an older program.
/// Who is connected decides each line, as it decides the restart; a count the server could not
/// give is said, never read as nobody.
pub(super) fn planned_restarts(
    workspaces: Option<&[SetupWorkspace]>,
    installing: Option<&str>,
    restart: Option<bool>,
    start: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();
    let newer = |version: Option<&str>| match (
        version.and_then(version_numbers),
        installing.and_then(version_numbers),
    ) {
        (Some(running), Some(ours)) => running > ours,
        _ => false,
    };
    let restarted = |workspace: &&SetupWorkspace| {
        workspace.role == SetupWorkspaceRole::Host
            && workspace.state == SetupWorkspaceState::Running
            && (workspace.binary_current == Some(false)
                || (installing.is_some() && !newer(workspace.version.as_deref())))
    };
    match (restart, workspaces) {
        (Some(false), _) => {
            lines.push("It restarts no workspace; any running an older Crew are only reported.".into())
        }
        (_, None) => lines.push(
            "It restarts the workspaces you host there that run an older Crew and that nobody else is connected to. Check the server first to see which.".into(),
        ),
        (_, Some(workspaces)) => {
            for workspace in workspaces.iter().filter(restarted) {
                lines.push(restart_line(workspace, restart == Some(true)));
            }
        }
    }
    for id in start {
        let name = workspaces
            .into_iter()
            .flatten()
            .find(|workspace| &workspace.workspace_id == id)
            .map_or_else(|| id.clone(), workspace_name);
        lines.push(format!("It starts {name}."));
    }
    lines
}

/// What a setup does to one hosted workspace it finds on an older Crew: restarts it when nobody
/// else is connected, or when `restart` says to anyway; otherwise leaves it running.
fn restart_line(workspace: &SetupWorkspace, restart: bool) -> String {
    let name = workspace_name(workspace);
    match (workspace.others_connected, restart) {
        (Some(0), _) => format!("It restarts {name}, which nobody else is connected to."),
        (Some(1), true) => format!(
            "It restarts {name}: 1 other person is connected, and their running agent commands will stop."
        ),
        (Some(others), true) => format!(
            "It restarts {name}: {others} other people are connected, and their running agent commands will stop."
        ),
        (Some(1), false) => {
            format!("It leaves {name} running the older Crew: 1 other person is connected.")
        }
        (Some(others), false) => {
            format!("It leaves {name} running the older Crew: {others} other people are connected.")
        }
        (None, true) => format!(
            "It restarts {name}. Its server can't say who is connected, so anyone's running agent commands there will stop."
        ),
        (None, false) => format!(
            "It leaves {name} running the older Crew: its server can't say who is connected."
        ),
    }
}

fn access_line(access: Option<&str>, server: &str) -> Option<String> {
    match access? {
        "full" => Some(format!(
            "Agents get full access: they can run any command as you on {server}, read and change any file your account can, and use the network."
        )),
        "folder" => Some(format!(
            "Agents get a work folder on {server}, confined to that folder."
        )),
        _ => Some(format!("Agents can't work on {server}.")),
    }
}

/// `biorouter crew setup`: install or update Crew in the person's account on a server, and
/// for a saved connection, connect it (AO-8).
pub(super) fn setup(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let SetupTarget {
            target,
            login,
            saved,
        } = setup_target(ctx).await?;
        let member = target["kind"] == "connection";
        let access = ctx.text("agent_access");
        if access.is_some() && !member {
            return Err(said(
                None,
                "agent_access is set for a saved connection: name it with connection.",
            ));
        }
        let start = ctx.texts("start_workspaces");
        if let Some(bad) = start.iter().find(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err(said(
                None,
                format!("{bad} isn't a workspace ID. Use the IDs setup_check lists."),
            ));
        }
        let restart = ctx.flag("restart_workspaces");
        let mut request =
            json!({"target": target, "intent": if member {"member"} else {"install"}});
        for key in ["keep_older", "replace_existing"] {
            if ctx.flag(key) == Some(true) {
                request[key] = json!(true);
            }
        }
        if let Some(restart) = restart {
            request["restart_workspaces"] = json!(restart);
        }
        if !start.is_empty() {
            request["start_workspaces"] = json!(start);
        }
        if let Some(access) = &access {
            request["agent_access"] = json!(access);
        }
        let summary = match &saved {
            Some(connection) if member => format!(
                "Set up Crew for {} on {login}: install or update it in your account there, then connect",
                connection.name
            ),
            _ => format!("Install or update Crew in your account on {login}"),
        };
        let mut card = Card::new(summary).show("server", login.clone());
        let keep_older = ctx.flag("keep_older") == Some(true);
        for line in restart_lines(&login, keep_older, restart, &start) {
            card = card.line(line);
        }
        if let Some(line) = access_line(access.as_deref(), &login) {
            card = card.line(line);
        }
        for key in [
            "keep_older",
            "replace_existing",
            "restart_workspaces",
            "start_workspaces",
            "agent_access",
        ] {
            if !request[key].is_null() {
                card = card.show(key, request[key].clone());
            }
        }
        ctx.approve(card).await?;
        start_job(ctx, request).await
    })
}

pub(super) fn setup_cancel(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let job = ctx.required("job_id")?;
        let status = biorouter::crew::setup_status(&job).ok_or_else(|| said(None, NO_JOB))?;
        ctx.approve(
            Card::new(format!("Stop setting up {}", status.server.login))
                .line("Whatever the job already changed on the server stays changed.")
                .show("job_id", job.clone()),
        )
        .await?;
        if biorouter::crew::cancel_setup(&job) {
            Ok(json!({"cancelled": true, "job_id": job}))
        } else {
            Err(said(None, NO_JOB))
        }
    })
}

/// `biorouter crew host`: host a new workspace on a server as the person, in one setup job that
/// starts it, saves its connection here, joins it and sets its privacy (AO-9, "Hosting in one
/// job").
pub(super) fn host(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let server = ctx.required("server")?;
        let name = ctx.required("name")?;
        if !biorouter_crew::workspace_name_valid(&name) {
            return Err(said(
                None,
                "A workspace name is 1 to 64 lowercase letters, digits and hyphens, like lab or chen-lab.",
            ));
        }
        let mode = ctx.required("mode")?;
        let institution = ctx.text("institution_id");
        if mode == "private" && institution.is_none() {
            return Err(said(
                None,
                "A Private workspace needs an institution, like ucsf.",
            ));
        }
        let access = ctx.text("agent_access").unwrap_or_else(|| "full".into());
        let mut card = Card::new(format!(
            "Host a new workspace {name} on {server} as you, and save and join it on this computer"
        ))
        .line(match &institution {
            Some(institution) => format!("Privacy: {mode}, institution {institution}."),
            None => format!("Privacy: {mode}."),
        })
        .line(format!(
            "Your account on {server} becomes its host. Anyone who signs in as that account can act as its host."
        ))
        .show("server", server.clone())
        .show("name", name.clone())
        .show("mode", mode.clone())
        .show("agent_access", access.clone());
        if let Some(line) = access_line(Some(&access), &server) {
            card = card.line(line);
        }
        if let Some(institution) = &institution {
            card = card.show("institution_id", institution.clone());
        }
        if ctx.flag("fix_storage_permissions") == Some(true) {
            card = card
                .line("It takes other accounts' write access off your folders above where the workspace is kept.")
                .show("fix_storage_permissions", true);
        }
        ctx.approve(card).await?;
        let prepared = ctx.crew()?.prepare_device().await?;
        let mut target = json!({
            "kind": "pending",
            "preparation_id": prepared.preparation_id,
            "ssh_target": server,
        });
        if let Some(port) = ctx.number("port") {
            target["port"] = json!(port);
        }
        if let Some(file) = ctx.text("identity_file") {
            target["identity_file"] = json!(file);
        }
        if let Some(jump) = ctx.text("proxy_jump") {
            target["proxy_jump"] = json!(jump);
        }
        let mut request = json!({
            "target": target,
            "intent": "host",
            "workspace_name": name,
            "finish": true,
            "mode": mode,
            "agent_access": access,
        });
        if let Some(institution) = institution {
            request["institution_id"] = json!(institution);
        }
        if ctx.flag("fix_storage_permissions") == Some(true) {
            request["fix_storage_permissions"] = json!(true);
        }
        start_job(ctx, request).await
    })
}

/// The workspace `typed` names in a check job's listing, as `biorouter crew open` picks it:
/// by name (letter case aside) or ID, and by host when two hosts share the name. One that is
/// removed, deleted, stopped or unlisted is refused with what to do instead.
fn workspace_to_open(
    status: &SetupStatus,
    typed: &str,
    host: Option<&str>,
) -> Result<SetupWorkspace> {
    let server = status.server.login.as_str();
    let host = host
        .map(|host| host.trim().trim_start_matches('@'))
        .filter(|host| !host.is_empty());
    let found: Vec<&SetupWorkspace> = status
        .workspaces
        .iter()
        .filter(|workspace| {
            workspace.workspace_id.eq_ignore_ascii_case(typed)
                || workspace
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(typed))
        })
        .filter(|workspace| {
            host.is_none_or(|host| workspace.host_username.as_deref() == Some(host))
        })
        .collect();
    let openable: Vec<&SetupWorkspace> = found
        .iter()
        .copied()
        .filter(|workspace| {
            workspace.state != SetupWorkspaceState::Deleted
                && workspace.role != SetupWorkspaceRole::Removed
        })
        .collect();
    let one = match openable.as_slice() {
        [one] => *one,
        [] => {
            return Err(said(
                None,
                match found.first() {
                    Some(workspace) if workspace.role == SetupWorkspaceRole::Removed => {
                        format!("You were removed from {typed}. Ask a member to invite you again.")
                    }
                    Some(_) => format!("{typed} on {server} was deleted by its host."),
                    None => {
                        format!("No workspace named {typed} on {server} is yours or invites you.")
                    }
                },
            ))
        }
        many => {
            let mut hosts: Vec<String> = many
                .iter()
                .filter_map(|workspace| workspace.host_username.clone())
                .collect();
            hosts.sort();
            hosts.dedup();
            return Err(said(
                None,
                if hosts.len() > 1 {
                    format!(
                        "More than one host on {server} has a workspace named {typed}: {}. Name the host with host.",
                        hosts
                            .iter()
                            .map(|host| format!("@{host}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    format!("There is more than one workspace named {typed} on {server}, so Biorouter can't tell which one to open. Use its ID.")
                },
            ));
        }
    };
    if one.state == SetupWorkspaceState::Stopped {
        return Err(said(
            None,
            if one.role == SetupWorkspaceRole::Host {
                format!("{typed} is stopped. Start it with crew__change action setup, start_workspaces [\"{}\"], then open it.", one.workspace_id)
            } else {
                let host = one
                    .host_username
                    .as_deref()
                    .map_or_else(|| "its host".to_owned(), |host| format!("@{host}"));
                format!("{typed} isn't running on {server}. Ask {host} to start it.")
            },
        ));
    }
    if one.invitation.is_none() {
        return Err(said(
            None,
            format!("The server didn't say how to open {typed}."),
        ));
    }
    Ok(one.clone())
}

fn mode_word(mode: Option<ClusterMode>) -> &'static str {
    match mode {
        Some(ClusterMode::Public) => "Public",
        _ => "Private",
    }
}

/// Save `invitation` with `overrides`, answering the saved connection's ID.
async fn save_invitation(
    crew: &CrewManager,
    invitation: &str,
    overrides: InvitationOverrides,
) -> Result<String> {
    match crew
        .connection_from_invitation(invitation, false, overrides)
        .await?
    {
        InvitationOutcome::Saved(connection) => Ok(connection.id),
        InvitationOutcome::Preview(_) => Err(said(None, super::FAILED)),
    }
}

/// Connect `id` when it is not connected, then join its workspace, as `biorouter crew join`
/// does. Answers the join and the connection.
async fn connect_and_join(crew: &CrewManager, id: &str, progress: &Progress) -> Result<Value> {
    if crew.connection(id).await?.status != "connected" {
        progress.set("stage", "connect");
        crew.connect(id).await?;
    }
    progress.set("stage", "join");
    let joined = crew.join(id).await?;
    let connection = crew.connection(id).await?;
    Ok(json!({
        "joined": true,
        "connection_id": id,
        "connection": connection_view(&connection).await?,
        "status": joined,
    }))
}

/// Run a member's setup of `id` with `access`, and wait for it to settle.
async fn member_setup(
    crew: &CrewManager,
    id: &str,
    access: &str,
    progress: &Progress,
) -> Result<()> {
    let request: SetupRequest = serde_json::from_value(json!({
        "target": {"kind": "connection", "connection_id": id},
        "intent": "member",
        "agent_access": access,
    }))?;
    let started = crew.start_setup(request).await?;
    progress.set("stage", "setup");
    progress.set("job_id", started.job_id.clone());
    loop {
        let Some(status) = biorouter::crew::setup_status(&started.job_id) else {
            return Err(said(None, NO_JOB));
        };
        match status.state {
            SetupState::Finished => return Ok(()),
            SetupState::Failed | SetupState::Cancelled => {
                let (code, message) = status
                    .error
                    .map(|error| (error.code, error.message))
                    .unwrap_or_else(|| {
                        ("crew_setup_failed".into(), "Setting up Crew failed.".into())
                    });
                return Err(said(
                    None,
                    format!("{message} (code: {code}) The connection is saved; join it with crew__change action join once that is fixed."),
                ));
            }
            SetupState::NeedsSignIn | SetupState::Running => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

/// The card for opening `entry` (AO-12: "for join and open the server, host and workspace-key
/// fingerprint and the privacy chosen"). `reused` is the connection this computer already
/// saved for the workspace, which the open joins through instead of saving one: when it signs
/// in as another login than the one checked, the card says so, names it, and says nothing of
/// the role the checked login has, since the join and any agent access are that connection's.
pub(super) fn open_card(
    entry: &SetupWorkspace,
    login: &str,
    access: Option<&str>,
    reused: Option<&Connection>,
) -> Card {
    let host = entry
        .host_username
        .as_deref()
        .map_or_else(|| "its host".to_owned(), |host| format!("@{host}"));
    let shown = workspace_name(entry);
    let mut card = Card::new(format!(
        "Open {shown}, hosted by {host}, on {login}, and join it as you"
    ))
    .line(format!(
        "Workspace key fingerprint: {}.",
        entry
            .workspace_key_fingerprint
            .as_deref()
            .unwrap_or("not given")
    ))
    .line(privacy_line(
        entry.mode.unwrap_or(ClusterMode::Private),
        entry.institution_id.as_deref(),
    ));
    card = match reused {
        Some(saved) if saved.ssh_target != login => card
            .line(format!(
                "It uses the connection already saved on this computer as {}, which signs in as {}, not {login}.",
                saved.name, saved.ssh_target
            ))
            .show("connection", saved.name.clone()),
        Some(_) => card
            .line("It uses the connection already saved for it on this computer.")
            .line(format!("You join it as its {}.", role_of(entry))),
        None => card.line(format!("You join it as its {}.", role_of(entry))),
    };
    card = card
        .show("server", login.to_owned())
        .show("workspace", shown)
        .show("workspace_id", entry.workspace_id.clone());
    // Agent access is set on the connection the open joins through, as its login.
    let access_login = reused.map_or(login, |saved| saved.ssh_target.as_str());
    if let Some(line) = access_line(access, access_login) {
        card = card.line(line).show("agent_access", access);
    }
    card
}

fn role_of(entry: &SetupWorkspace) -> &'static str {
    if entry.role == SetupWorkspaceRole::Host {
        "host"
    } else {
        "member"
    }
}

/// Save `entry` from its invitation line with the check's login, route and this computer's
/// prepared device, as `biorouter crew open` saves it. Answers the saved connection's ID.
async fn save_opened(
    crew: &CrewManager,
    entry: &SetupWorkspace,
    target: &Value,
    local_name: Option<String>,
) -> Result<String> {
    let prepared = crew.prepare_device().await?;
    let advanced = InvitationAdvanced {
        ssh_target: target["ssh_target"].as_str().map(str::to_owned),
        port: target["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok()),
        identity_file: target["identity_file"].as_str().map(str::to_owned),
        proxy_jump: target["proxy_jump"].as_str().map(str::to_owned),
        name: local_name,
        preparation_id: Some(prepared.preparation_id),
        ..InvitationAdvanced::default()
    };
    let invitation = entry.invitation.clone().unwrap_or_default();
    let overrides = InvitationOverrides {
        advanced,
        ..InvitationOverrides::default()
    };
    save_invitation(crew, &invitation, overrides).await
}

/// `biorouter crew open NAME --server LOGIN`: open a workspace a check job found (AO-9,
/// "Open"). The card names the server, the host, the workspace key's fingerprint and the
/// privacy the connection is saved with; then the workspace is saved from its invitation line
/// with the check's login, route and prepared device, set up when agent access was asked for,
/// and joined.
pub(super) fn open(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let job = ctx.required("job_id")?;
        let typed = ctx.required("workspace")?;
        let status = biorouter::crew::setup_status(&job).ok_or_else(|| said(None, NO_JOB))?;
        if status.state != SetupState::Finished {
            return Err(said(
                None,
                "That check hasn't finished. Follow it with crew__read action setup_status, then open.",
            ));
        }
        let target = job_target(&job)
            .filter(|target| target["kind"] == "server")
            .ok_or_else(|| {
                said(
                    None,
                    "Open a workspace from a server checked in this chat: run crew__read action setup_check with the server first.",
                )
            })?;
        let entry = workspace_to_open(&status, &typed, ctx.text("host").as_deref())?;
        let access = ctx.text("agent_access");
        let crew = ctx.crew()?;
        // The connection the open joins through, as the card names it: one this computer saved
        // for the workspace may sign in as another login than the one checked.
        let reused = match &entry.saved_connection_id {
            Some(id) => crew.connection(id).await.ok(),
            None => None,
        };
        let login = target["ssh_target"]
            .as_str()
            .unwrap_or(&status.server.login)
            .to_owned();
        ctx.approve(open_card(
            &entry,
            &login,
            access.as_deref(),
            reused.as_ref(),
        ))
        .await?;
        let local_name = ctx.text("local_name");
        let role = role_of(&entry);
        let progress = Progress::default();
        progress.set("stage", "save");
        let watched = progress.clone();
        ctx.follow(progress, async move {
            let id = match reused {
                Some(saved) => saved.id,
                None => save_opened(&crew, &entry, &target, local_name).await?,
            };
            watched.set("connection_id", id.clone());
            if let Some(access) = access.filter(|access| access != "off") {
                member_setup(&crew, &id, &access, &watched).await?;
            }
            let mut answer = connect_and_join(&crew, &id, &watched).await?;
            answer["role"] = json!(role);
            Ok(answer)
        })
        .await
    })
}

fn fingerprint_of(connection: &Connection) -> String {
    biorouter_crew::invitation::workspace_key_fingerprint(&connection.workspace_public_key)
        .map_or_else(
            || "not readable".to_owned(),
            |fingerprint| biorouter_crew::invitation::grouped_fingerprint(&fingerprint),
        )
}

fn privacy_line(mode: ClusterMode, institution: Option<&str>) -> String {
    match institution {
        Some(institution) => format!(
            "Privacy: {}, institution {institution}.",
            mode_word(Some(mode))
        ),
        None => format!("Privacy: {}.", mode_word(Some(mode))),
    }
}

/// What saving `preview` still needs, as the words the preview uses for them.
fn missing_words(preview: &biorouter::crew::authentication::InvitationPreview) -> Vec<String> {
    preview
        .missing
        .iter()
        .map(|missing| {
            serde_json::to_value(missing)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default()
        })
        .collect()
}

/// `join` from an invitation the person pasted: previewed, so the card can name the server,
/// the host, the fingerprint and the privacy; then saved (unless this computer already has
/// it), connected and joined.
async fn join_invitation(ctx: &Ctx, invitation: String) -> Result<Value> {
    let crew = ctx.crew()?;
    let overrides = overrides(ctx)?;
    let InvitationOutcome::Preview(preview) = crew
        .connection_from_invitation(&invitation, true, overrides.clone())
        .await?
    else {
        return Err(said(None, super::FAILED));
    };
    if !preview.missing.is_empty() {
        return Err(said(
            Some("crew_invitation_invalid"),
            format!(
                "Saving this invitation still needs: {}. Ask the person, then join again with them.",
                missing_words(&preview).join(", ")
            ),
        ));
    }
    let host = preview
        .host_username
        .as_deref()
        .map_or_else(|| "its host".to_owned(), |host| format!("@{host}"));
    let server = preview
        .server_label
        .clone()
        .or_else(|| preview.server.clone())
        .unwrap_or_else(|| "its server".to_owned());
    ctx.approve(
        Card::new(format!(
            "Join {}, hosted by {host}, on {server}, as you",
            preview.workspace_label
        ))
        .line(format!(
            "Workspace key fingerprint: {}.",
            preview.fingerprint
        ))
        .line(privacy_line(
            preview.mode,
            preview.institution_id.as_deref(),
        ))
        .line(match &preview.existing_connection_id {
            Some(_) => "It uses the connection already saved for it on this computer.".to_owned(),
            None => format!("It saves it on this computer as {}.", preview.name),
        })
        .show("invitation", invitation.clone()),
    )
    .await?;
    let existing = preview.existing_connection_id.clone();
    let progress = Progress::default();
    progress.set("stage", "save");
    let watched = progress.clone();
    ctx.follow(progress, async move {
        let id = match existing {
            Some(id) => id,
            None => save_invitation(&crew, &invitation, overrides).await?,
        };
        watched.set("connection_id", id.clone());
        connect_and_join(&crew, &id, &watched).await
    })
    .await
}

/// What a connected saved connection in join state `state` answers before any card: `None`
/// when it is `ready` (or `approved`, an older joiner's) and can be joined, an answer when it
/// already is, and a refusal saying why otherwise.
fn join_state_answer(state: JoinState, connection: &Connection) -> Option<Result<Value>> {
    let name = &connection.name;
    Some(match state {
        JoinState::Ready | JoinState::Approved => return None,
        JoinState::Joined => Ok(json!({
            "joined": true,
            "already": true,
            "connection_id": connection.id,
        })),
        JoinState::Removed => Err(said(
            Some("crew_join_removed"),
            format!("You were removed from {name}. Ask a member to invite you again."),
        )),
        JoinState::NotInvited => Err(said(
            None,
            format!("This account is not a member of {name}. Ask a member to invite it."),
        )),
        JoinState::Expired => Err(said(
            None,
            format!("The invitation to {name} expired. Ask a member to invite you again."),
        )),
        other => {
            let state = serde_json::to_value(other).unwrap_or_default();
            Err(said(
                None,
                format!(
                    "{name} isn't ready to join yet (its join status is {}). Check again later, or finish it in Crew.",
                    state.as_str().unwrap_or("unknown")
                ),
            ))
        }
    })
}

/// `biorouter crew join`: join a workspace, from an invitation the person pasted (saved first)
/// or a saved connection that is ready to join (AO-9, "Join states").
pub(super) fn join(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        if let Some(invitation) = ctx.text("invitation") {
            if ctx.text("connection").is_some() {
                return Err(said(
                    None,
                    "Join from an invitation or a saved connection, not both.",
                ));
            }
            return join_invitation(ctx, invitation).await;
        }
        let crew = ctx.crew()?;
        let connection = ctx.connection().await?;
        let server = Ctx::server_label(&connection).await;
        let connected = connection.status == "connected";
        let mut host = None;
        if connected {
            let status = crew.join_status(&connection.id).await?;
            if let Some(answer) = join_state_answer(status.status, &connection) {
                return answer;
            }
            // The workspace says who hosts it to an account it knows; an older one can't.
            host = crew
                .account_status(&connection.id)
                .await
                .ok()
                .and_then(|status| status.host_username);
        }
        // Named as the other two join cards name a host, with the account the invitation
        // pinned when its username is not known yet.
        let host = host.map_or_else(
            || format!("its host (UID {})", connection.owner_uid),
            |host| format!("@{host}"),
        );
        ctx.approve(
            Card::new(if connected {
                format!(
                    "Join {}, hosted by {host}, on {server}, as you",
                    connection.name
                )
            } else {
                format!(
                    "Connect {} to {server} and join it, hosted by {host}, as you",
                    connection.name
                )
            })
            .line(format!(
                "Workspace key fingerprint: {}.",
                fingerprint_of(&connection)
            ))
            .line(privacy_line(
                connection.mode,
                connection.institution_id.as_deref(),
            ))
            .show("connection", connection.name.clone()),
        )
        .await?;
        let id = connection.id.clone();
        let progress = Progress::default();
        progress.set("connection_id", id.clone());
        let watched = progress.clone();
        ctx.follow(progress, async move {
            connect_and_join(&crew, &id, &watched).await
        })
        .await
    })
}

/// `biorouter crew rejoin`: give the saved connection a new device key in place, as the account
/// on the server (AO-9, "Adding this computer again").
pub(super) fn rejoin(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let crew = ctx.crew()?;
        let connection = ctx.connection().await?;
        let server = Ctx::server_label(&connection).await;
        ctx.approve(
            Card::new(format!(
                "Add this computer to {} on {server} again, with a new key, as your account there",
                connection.name
            ))
            .line("Its old key on this computer stops working.")
            .line("Chats with Crew access through this connection keep the connection, but each needs access granted again.")
            .show("connection", connection.name.clone()),
        )
        .await?;
        let id = connection.id.clone();
        let progress = Progress::default();
        progress.set("connection_id", id.clone());
        let watched = progress.clone();
        ctx.follow(progress, async move {
            if crew.connection(&id).await?.status != "connected" {
                watched.set("stage", "connect");
                crew.connect(&id).await?;
            }
            watched.set("stage", "rejoin");
            Ok(serde_json::to_value(crew.rejoin(&id).await?)?)
        })
        .await
    })
}

pub(super) fn connect(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let crew = ctx.crew()?;
        let connection = ctx.connection().await?;
        let server = Ctx::server_label(&connection).await;
        ctx.approve(
            Card::new(format!("Connect {} to {server} as you", connection.name))
                .show("connection", connection.name.clone()),
        )
        .await?;
        let id = connection.id.clone();
        let progress = Progress::default();
        progress.set("connection_id", id.clone());
        progress.set("stage", "connect");
        ctx.follow(progress, async move {
            let connected = crew.connect(&id).await?;
            Ok(serde_json::to_value(connection_view(&connected).await?)?)
        })
        .await
    })
}

pub(super) fn disconnect(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let crew = ctx.crew()?;
        let connection = ctx.connection().await?;
        ctx.approve(
            Card::new(format!("Disconnect {}", connection.name))
                .line("Chats and tasks working through it lose it until it connects again.")
                .show("connection", connection.name.clone()),
        )
        .await?;
        crew.disconnect(&connection.id).await?;
        Ok(json!({"disconnected": true, "connection_id": connection.id}))
    })
}

/// Whether `snapshot`'s caller hosts the workspace: `None` when it cannot tell.
pub(crate) fn hosts(snapshot: &Value) -> Option<bool> {
    let actor = &snapshot["actor"];
    let actor_id = actor["id"].as_str()?;
    let workspace = &snapshot["workspace"];
    match workspace["host_principal_id"].as_str() {
        Some(host) => Some(host == actor_id),
        None => Some(workspace["host_uid"].as_u64()? == actor["uid"].as_u64()?),
    }
}

/// `biorouter crew connections remove`. On a server whose Crew knows the account, the account
/// keeps the workspace and its host controls (AO-9, "The removal guard"); on an older one,
/// removing the host's only computer would end them, so the chat refuses and says how to update
/// Crew first.
pub(super) fn connections_remove(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let crew = ctx.crew()?;
        let connection = ctx.connection().await?;
        let deleted = crew.workspace_deleted(&connection.id);
        let mut card = Card::new(format!("Remove {} from this computer", connection.name))
            .show("connection", connection.name.clone());
        if deleted {
            card = card.line("Its host deleted it on the server, so this removes what is left of it here, including this computer's key for it.");
        } else {
            card = card
                .line("It disconnects, ends every chat's access through it and deletes this computer's key for the workspace. Messages stay on the server.")
                .line("Chats given Crew access through it can't get it through a new connection. If this computer only lost its key, use rejoin instead: it keeps the connection.");
            if crew.account_identity(&connection.id) == Some(true) {
                card = card.line("The workspace can be opened again from any computer that signs in to the server as your account.");
            } else {
                let only_host_computer = match ctx.snapshot(&connection).await {
                    Ok(snapshot) => {
                        hosts(&snapshot) == Some(true)
                            && snapshot["actor"]["devices"]
                                .as_array()
                                .is_none_or(|devices| devices.len() <= 1)
                    }
                    Err(_) => false,
                };
                if only_host_computer {
                    return Err(said(
                        Some("crew_host_controls_would_end"),
                        format!(
                            "This is the only computer that can act as host of {}, and its server runs an older Crew, so removing it would end the host controls for good. Update Crew on the server first with crew__change action setup, or remove it in the Crew view.",
                            connection.name
                        ),
                    ));
                }
                card = card.line("If you host this workspace and this is your only computer enrolled as host, its host controls end.");
            }
        }
        ctx.approve(card).await?;
        crew.remove(&connection.id).await?;
        Ok(json!({"removed": true, "connection_id": connection.id}))
    })
}

/// The saved connection's own fields, as `PATCH /crew/connections/{id}` takes them back, so an
/// edit changes only what it means to (the CLI's `saved_fields`).
pub(crate) fn saved_fields(connection: &Connection) -> Result<serde_json::Map<String, Value>> {
    let whole = serde_json::to_value(connection)?;
    let mut input = serde_json::Map::new();
    for key in [
        "name",
        "ssh_target",
        "port",
        "identity_file",
        "proxy_jump",
        "socket_path",
        "owner_uid",
        "workspace_id",
        "workspace_public_key",
        "remote_root",
        "remote_execution",
        "remote_access",
        "cluster_connection_id",
        "mode",
        "institution_id",
    ] {
        if let Some(value) = whole.get(key) {
            input.insert(key.into(), value.clone());
        }
    }
    Ok(input)
}

/// Save `input` over `connection`, as `PATCH /crew/connections/{id}` does, and answer the saved
/// connection as the routes show it, with whether the save ended the chats' access through it.
///
/// `connection` is the one read before the card, and `input` is built from it: a card waits up
/// to ten minutes, and saving its fields over a connection changed meanwhile (agent access
/// turned off, made Private) would silently put back what the card never showed. So the
/// connection is read again, and when its saved fields or its policy epoch differ, nothing is
/// saved and the refusal says why.
pub(crate) async fn save_over(
    connection: &Connection,
    input: serde_json::Map<String, Value>,
) -> Result<Value> {
    let crew = biorouter::crew::manager()?;
    let now = super::saved_connection(&crew, &connection.id).await?;
    if now.policy_epoch != connection.policy_epoch
        || saved_fields(&now)? != saved_fields(connection)?
    {
        return Err(said(
            None,
            format!(
                "{} changed while this waited for your approval, so nothing was changed. Look at it again with crew__read action connections_show, then ask again.",
                connection.name
            ),
        ));
    }
    let request: SaveConnection = serde_json::from_value(Value::Object(input))?;
    let saved = crew.update(&connection.id, request).await?;
    let ended = saved.policy_epoch != connection.policy_epoch;
    let mut answer = json!({"connection": connection_view(&saved).await?});
    if ended {
        answer["access_ended"] = json!(true);
    }
    Ok(answer)
}

pub(super) fn connections_update(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        let mut input = saved_fields(&connection)?;
        let mut card = Card::new(format!("Change the saved connection {}", connection.name));
        let mut changed = false;
        for key in ["name", "ssh_target", "identity_file", "proxy_jump"] {
            if let Some(value) = ctx.text(key) {
                card = card.show(key, value.clone());
                input.insert(key.into(), json!(value));
                changed = true;
            }
        }
        if let Some(port) = ctx.number("port") {
            card = card.show("port", port);
            input.insert("port".into(), json!(port));
            changed = true;
        }
        if !changed {
            return Err(said(
                None,
                "Say what to change: name, ssh_target, port, identity_file or proxy_jump.",
            ));
        }
        if ["ssh_target", "port", "identity_file", "proxy_jump"]
            .iter()
            .any(|key| ctx.param_given(key))
        {
            card = card.line("A new login or route reconnects it, and ends every chat's and task's access through it.");
        }
        ctx.approve(card).await?;
        save_over(&connection, input).await
    })
}

/// A work folder on the server as a connection saves it, as `biorouter crew connections
/// set-access` reads it: a full path, or `~/…` once the account's home there is known. The
/// bridge checks the folder itself; this only refuses what cannot be one.
fn work_folder(
    typed: Option<String>,
    view: &crate::routes::crew::wire::CrewConnectionView,
) -> Result<Option<String>> {
    let Some(text) = typed else {
        return Ok(None);
    };
    let home = view
        .server_access
        .as_ref()
        .and_then(|access| access.home.clone())
        .filter(|home| home.starts_with('/'));
    let folder = match (text.strip_prefix("~/"), &home) {
        (Some(rest), Some(home)) => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => text,
    };
    if !folder.starts_with('/') || folder == "/" || folder.contains(['\n', '\r', '\0']) {
        return Err(said(
            Some("crew_folder_required"),
            format!(
                "Give the work folder's full path on {}, like /home/you/crew-work/lab.",
                view.server_label
            ),
        ));
    }
    Ok(Some(folder))
}

/// `biorouter crew connections set-access off|folder|full` (OW-6).
pub(super) fn connections_set_access(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        let access = ctx.required("access")?;
        let view = connection_view(&connection).await?;
        let server = view.server_label.clone();
        let folder = work_folder(ctx.text("folder"), &view)?;
        let existing = connection
            .remote_root
            .clone()
            .filter(|root| !root.is_empty());
        let folder_or_existing = || {
            folder.clone().or_else(|| existing.clone()).ok_or_else(|| {
                said(
                    Some("crew_folder_required"),
                    format!("Choose the folder agents work in on {server}: add folder with its full path."),
                )
            })
        };
        let mut input = saved_fields(&connection)?;
        let summary = match access.as_str() {
            "off" => {
                if folder.is_some() {
                    return Err(said(
                        None,
                        "folder is for folder or full access; off uses no folder.",
                    ));
                }
                input.insert("remote_root".into(), Value::Null);
                input.insert("remote_execution".into(), json!(false));
                input.insert("remote_access".into(), json!("folder"));
                format!(
                    "Stop agents working on {server} through {}",
                    connection.name
                )
            }
            "folder" => {
                let root = folder_or_existing()?;
                let confined = view
                    .server_access
                    .as_ref()
                    .and_then(|access| access.confined_execution_available)
                    != Some(false);
                input.insert("remote_root".into(), json!(root));
                input.insert("remote_execution".into(), json!(confined));
                input.insert("remote_access".into(), json!("folder"));
                if confined {
                    format!("Let agents work in {root} on {server}, confined to that folder")
                } else {
                    format!("Let agents read and write files in {root} on {server}; its kernel can't confine commands, so they can't run any there")
                }
            }
            _ => {
                if view
                    .server_access
                    .as_ref()
                    .is_some_and(|access| !access.full_access_available)
                {
                    return Err(said(
                        None,
                        format!("The Crew in your account on {server} can't give full access yet. Update it with crew__change action setup, then try again."),
                    ));
                }
                let root = folder_or_existing()?;
                input.insert("remote_root".into(), json!(root));
                input.insert("remote_execution".into(), json!(true));
                input.insert("remote_access".into(), json!("full"));
                format!(
                    "Let agents work as you on {server}, starting in {root}: through {} an agent can run any command your account can, read and change any file it can reach, use the network, and start jobs that keep running after its task ends. Public models never get this access",
                    connection.name
                )
            }
        };
        ctx.approve(
            Card::new(summary)
                .line(format!(
                    "This ends every chat's and task's access through {}; grant it again where you need it.",
                    connection.name
                ))
                .show("connection", connection.name.clone())
                .show("access", access.clone())
                .show("folder", input.get("remote_root").cloned().unwrap_or(Value::Null)),
        )
        .await?;
        save_over(&connection, input).await
    })
}

pub(super) fn defaults_set(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let provider = ctx.required("provider")?;
        let model = ctx.required("model")?;
        if [&provider, &model]
            .iter()
            .any(|word| word.contains(char::is_whitespace))
        {
            return Err(said(
                None,
                "Name the provider and the model as single words, like versa_azure and gpt-5.5.",
            ));
        }
        ctx.approve(
            Card::new(format!(
                "Start Crew tasks on {model} ({provider}) from now on, unless a task names another model"
            ))
            .show("provider", provider.clone())
            .show("model", model.clone()),
        )
        .await?;
        let value = json!({"provider": provider, "model": model});
        Config::global().set(CREW_TASK_MODEL, &value, false)?;
        Ok(json!({"crew_tasks": value}))
    })
}

pub(super) fn defaults_clear(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let config = Config::global();
        let stored = config.get_param::<Value>(CREW_TASK_MODEL).ok();
        ctx.approve(Card::new(
            "Start Crew tasks on your default model for new chats from now on",
        ))
        .await?;
        if stored.is_some() {
            config.delete(CREW_TASK_MODEL)?;
        }
        Ok(json!({"cleared": stored.is_some()}))
    })
}
