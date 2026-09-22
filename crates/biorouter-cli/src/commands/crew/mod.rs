mod args;
mod files;
mod output;

use crate::daemon_client::CrewClient;
use anyhow::{anyhow, ensure, Context, Result};
pub use args::CrewOptions;
use args::*;
use biorouter::crew::observation::{Initial, ObserveEvent, ObserveRequest};
use output::{component, emit, read_input};
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};
use zeroize::Zeroizing;

pub async fn handle(mut options: CrewOptions) -> Result<()> {
    let format = options.output_format;
    let request_id = options
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    options.request_id = Some(request_id.clone());
    execute(options).await.map_err(|error| {
        let message = output::safe_text(&format!("{error:#}"));
        if matches!(format, OutputFormat::Json | OutputFormat::StreamJson) {
            let _ = emit(&json!({"error": message, "request_id": request_id}), format);
        }
        anyhow!(
            "{message} [Crew request ID: {}]",
            output::safe_text(&request_id)
        )
    })
}

async fn execute(options: CrewOptions) -> Result<()> {
    let CrewOptions {
        connection,
        no_start,
        approval_key_stdin,
        output_format,
        request_id,
        command,
    } = options;
    if let CrewCommand::Daemon(command) = command {
        let action = match command {
            DaemonCommand::Start => "start",
            DaemonCommand::Status => "status",
            DaemonCommand::Stop => "stop",
        };
        return emit(
            &crate::daemon_client::daemon_control(action, approval_key_stdin).await?,
            output_format,
        );
    }
    if let CrewCommand::Credentials(command) = command {
        let action = match command {
            CredentialCommand::Status => "status",
            CredentialCommand::Init => "init",
            CredentialCommand::Unlock => "unlock",
            CredentialCommand::Lock => "lock",
        };
        return emit(
            &crate::daemon_client::credentials_control(action, approval_key_stdin).await?,
            output_format,
        );
    }
    let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    component(&request_id).context("Invalid Crew request ID")?;
    let client = CrewClient::connect_with_input(no_start, approval_key_stdin).await?;
    let api = Api {
        client,
        selected: connection,
        request_id,
        format: output_format,
    };
    execute_connected(api, command).await
}

async fn execute_connected(api: Api, command: CrewCommand) -> Result<()> {
    let result = match command {
        CrewCommand::Daemon(_) | CrewCommand::Credentials(_) => unreachable!(),
        CrewCommand::Status => api.connections().await?,
        CrewCommand::Connections(command) => connections(&api, command).await?,
        CrewCommand::Auth => {
            let id = api.connection_id().await?;
            api.client.authenticate(&id).await?
        }
        CrewCommand::Connect => api.connection_action("connect", json!({})).await?,
        CrewCommand::Disconnect => api.connection_action("disconnect", json!({})).await?,
        CrewCommand::Workspace(WorkspaceCommand::Show) => api.snapshot().await?,
        CrewCommand::Workspace(WorkspaceCommand::Bootstrap) => {
            let connection = api.connection().await?;
            api.broker(
                "auth.bootstrap",
                json!({"public_key": connection["public_key"]}),
                true,
            )
            .await?
        }
        CrewCommand::Enroll(command) => enrollment(&api, command).await?,
        CrewCommand::Members => api.snapshot_field("principals").await?,
        CrewCommand::Teams(TeamCommand::List) => api.snapshot_field("teams").await?,
        CrewCommand::Teams(TeamCommand::Create { name }) => {
            api.broker("team.create", json!({"name":name}), true)
                .await?
        }
        CrewCommand::Channels(command) => channels(&api, command).await?,
        CrewCommand::Invites(command) => invitations(&api, command).await?,
        CrewCommand::Profile(ProfileCommand::Show) => api.snapshot_field("actor").await?,
        CrewCommand::Profile(ProfileCommand::Set { nickname, avatar }) => {
            api.broker(
                "profile.update",
                json!({"nickname":nickname,"avatar":avatar}),
                true,
            )
            .await?
        }
        CrewCommand::Ownership(OwnershipCommand::Offer { channel, successor }) => {
            api.broker(
                "channel.transfer",
                json!({"channel_id":channel,"successor_id":successor}),
                true,
            )
            .await?
        }
        CrewCommand::Ownership(OwnershipCommand::Accept { channel }) => {
            api.broker("transfer.accept", json!({"channel_id":channel}), true)
                .await?
        }
        CrewCommand::RemoveMember { channel, principal } => {
            api.broker(
                "membership.revoke",
                json!({"channel_id":channel,"principal_id":principal}),
                true,
            )
            .await?
        }
        CrewCommand::History(args) => {
            api.broker("messages.history", history_params(args), false)
                .await?
        }
        CrewCommand::Search {
            channel,
            query,
            limit,
            after,
        } => {
            let mut params = json!({"channel_id":channel,"query":query,"limit":limit});
            if let Some(after) = after {
                params["after"] = json!(after);
            }
            api.broker("messages.search", params, false).await?
        }
        CrewCommand::Watch(args) => return watch(&api, args).await,
        CrewCommand::Send(args) => send_message(&api, args).await?,
        CrewCommand::Context { session } => api.session_get(&session, "context").await?,
        CrewCommand::Files(command) => files::handle(&api, command).await?,
        CrewCommand::Tasks(command) => return tasks(&api, command).await,
        CrewCommand::Grants(command) => grants(&api, command).await?,
        CrewCommand::Privacy(command) => privacy(&api, command).await?,
    };
    emit(&result, api.format)
}

async fn send_message(api: &Api, args: SendArgs) -> Result<Value> {
    let body = if args.text.is_none() && args.input.is_none() {
        ensure!(
            !args.attachments.is_empty() || !args.references.is_empty(),
            "Choose --text, --input, --attachment, or --reference"
        );
        String::new()
    } else {
        text_input(TextInput {
            text: args.text,
            input: args.input,
        })?
    };
    api.broker("message.post", json!({"channel_id":args.channel,"body":body,"attachments":args.attachments,"references":args.references}), true).await
}

struct Api {
    client: CrewClient,
    selected: Option<String>,
    request_id: String,
    format: OutputFormat,
}

impl Api {
    async fn connections(&self) -> Result<Value> {
        self.client.request("GET", "/crew/connections", None).await
    }
    async fn connection(&self) -> Result<Value> {
        let response = self.connections().await?;
        let connections = response["connections"]
            .as_array()
            .context("Daemon returned an invalid connection list")?;
        if let Some(id) = &self.selected {
            component(id)?;
            return connections
                .iter()
                .find(|item| item["id"].as_str() == Some(id.as_str()))
                .cloned()
                .context("Selected Crew connection was not found; run crew connections list");
        }
        ensure!(
            connections.len() == 1,
            "Select a saved connection with --connection ID; run crew connections list"
        );
        Ok(connections[0].clone())
    }
    async fn connection_id(&self) -> Result<String> {
        let value = self.connection().await?;
        Ok(component(
            value["id"]
                .as_str()
                .context("Daemon returned a connection without its ID")?,
        )?
        .to_string())
    }
    async fn path(&self, suffix: &str) -> Result<String> {
        Ok(format!(
            "/crew/connections/{}{suffix}",
            self.connection_id().await?
        ))
    }
    async fn connection_action(&self, action: &str, body: Value) -> Result<Value> {
        self.client
            .request("POST", &self.path(&format!("/{action}")).await?, Some(body))
            .await
    }
    async fn broker(&self, method: &str, mut params: Value, mutation: bool) -> Result<Value> {
        if mutation {
            params["idempotency_key"] = json!(self.request_id);
        }
        let body = json!({"method":method,"params":params,"request_id":if mutation {Some(&self.request_id)} else {None}});
        self.connection_action("request", body).await
    }
    async fn snapshot(&self) -> Result<Value> {
        self.broker("workspace.snapshot", json!({}), false).await
    }
    async fn snapshot_field(&self, field: &str) -> Result<Value> {
        self.snapshot()
            .await?
            .get(field)
            .cloned()
            .context("Daemon returned an incomplete workspace snapshot")
    }
    async fn session_get(&self, session: &str, action: &str) -> Result<Value> {
        let path = self
            .path(&format!("/sessions/{}/{action}", component(session)?))
            .await?;
        self.client.request("GET", &path, None).await
    }
}

async fn connections(api: &Api, command: ConnectionCommand) -> Result<Value> {
    match command {
        ConnectionCommand::List => api.connections().await,
        ConnectionCommand::Show => api.connection().await,
        ConnectionCommand::Prepare => prepare(api).await,
        ConnectionCommand::Save { input } => {
            let body: Value = serde_json::from_str(&read_input(&input)?)
                .context("Connection input must be a JSON descriptor")?;
            api.client
                .request("POST", "/crew/connections", Some(body))
                .await
        }
        ConnectionCommand::Update { input } => {
            let body: Value = serde_json::from_str(&read_input(&input)?)
                .context("Connection input must be a JSON descriptor")?;
            api.client
                .request("PATCH", &api.path("").await?, Some(body))
                .await
        }
        ConnectionCommand::Remove => {
            api.client
                .request("DELETE", &api.path("").await?, None)
                .await
        }
    }
}

async fn prepare(api: &Api) -> Result<Value> {
    api.client
        .request("POST", "/crew/devices/prepare", Some(json!({})))
        .await
}

async fn enrollment(api: &Api, command: EnrollmentCommand) -> Result<Value> {
    match command {
        EnrollmentCommand::Prepare => prepare(api).await,
        EnrollmentCommand::Invite {
            uid,
            public_key,
            existing_principal,
        } => {
            let mut params = json!({"uid":uid,"public_key":public_key});
            if let Some(principal) = existing_principal {
                params["existing_principal_id"] = json!(principal);
            }
            api.broker("enrollment.invite", params, true).await
        }
        EnrollmentCommand::Accept(input) => {
            let connection = api.connection().await?;
            let secret = read_secret(input).await?;
            api.broker(
                "auth.enroll",
                json!({"invitation":secret.as_str(),"public_key":connection["public_key"]}),
                true,
            )
            .await
        }
        EnrollmentCommand::Revoke { principal } => {
            api.broker("enrollment.revoke", json!({"principal_id":principal}), true)
                .await
        }
    }
}

async fn read_secret(input: SecretInput) -> Result<Zeroizing<String>> {
    tokio::task::spawn_blocking(move || {
        let mut secret = if input.token_stdin {
            read_secret_line(std::io::stdin().lock())?
        } else if let Some(fd) = input.token_fd {
            ensure!(fd >= 0, "Secret file descriptor must be nonnegative");
            #[cfg(unix)]
            {
                read_secret_line(
                    std::fs::File::open(format!("/dev/fd/{fd}"))
                        .context("Could not read the supplied secret descriptor")?,
                )?
            }
            #[cfg(not(unix))]
            {
                return Err(anyhow!(
                    "Secret descriptors are unavailable on this platform; use --token-stdin"
                ));
            }
        } else {
            ensure!(
                std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
                "Enrollment requires a hidden terminal prompt, --token-stdin, or --token-fd"
            );
            eprintln!("Enrollment token (input hidden):");
            Zeroizing::new(console::Term::stderr().read_secure_line()?)
        };
        while secret.ends_with('\n') || secret.ends_with('\r') {
            secret.pop();
        }
        ensure!(
            !secret.is_empty() && secret.len() <= 8192,
            "Enrollment token must contain 1–8192 bytes"
        );
        Ok(secret)
    })
    .await
    .context("Enrollment token input failed")?
}

fn read_secret_line(mut source: impl Read) -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    let mut byte = [0];
    while source.read(&mut byte)? != 0 {
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
        ensure!(bytes.len() <= 8192, "Enrollment token exceeds 8192 bytes");
    }
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes)
            .context("Enrollment token must be UTF-8")?
            .to_string(),
    ))
}

async fn channels(api: &Api, command: ChannelCommand) -> Result<Value> {
    match command {
        ChannelCommand::List { team } => {
            let channels = api.snapshot_field("channels").await?;
            if let Some(team) = team {
                Ok(json!(channels
                    .as_array()
                    .context("Invalid channel list")?
                    .iter()
                    .filter(|item| item["team_id"].as_str() == Some(team.as_str()))
                    .collect::<Vec<_>>()))
            } else {
                Ok(channels)
            }
        }
        ChannelCommand::Create {
            name,
            team,
            classification,
        } => {
            api.broker(
                "channel.create",
                json!({"name":name,"team_id":team,"classification":classification.as_str()}),
                true,
            )
            .await
        }
        ChannelCommand::Archive { channel } => {
            api.broker("channel.archive", json!({"channel_id":channel}), true)
                .await
        }
        ChannelCommand::MarkRead { channel, cursor } => {
            api.broker(
                "channel.read",
                json!({"channel_id":channel,"sequence":cursor}),
                true,
            )
            .await
        }
    }
}

async fn invitations(api: &Api, command: InvitationCommand) -> Result<Value> {
    match command {
        InvitationCommand::List => api.snapshot_field("invitations").await,
        InvitationCommand::Create {
            principal,
            team,
            channel,
        } => {
            let (kind, target) = match (team, channel) {
                (Some(team), None) => ("team", team),
                (None, Some(channel)) => ("channel", channel),
                _ => return Err(anyhow!("Choose exactly one --team or --channel")),
            };
            api.broker(
                "invitation.create",
                json!({"kind":kind,"target_id":target,"principal_id":principal}),
                true,
            )
            .await
        }
        InvitationCommand::Accept { invitation } => {
            api.broker(
                "invitation.accept",
                json!({"invitation_id":invitation}),
                true,
            )
            .await
        }
    }
}

fn history_params(args: HistoryArgs) -> Value {
    let mut params = json!({"channel_id":args.channel,"limit":args.limit,"latest":args.latest});
    if let Some(before) = args.before {
        params["before"] = json!(before);
    }
    if let Some(after) = args.after {
        params["after"] = json!(after);
    }
    params
}

fn text_input(input: TextInput) -> Result<String> {
    match (input.text, input.input) {
        (Some(text), None) => Ok(text),
        (None, Some(path)) => read_input(&path),
        _ => Err(anyhow!("Choose exactly one --text or --input")),
    }
}

async fn watch(api: &Api, args: WatchArgs) -> Result<()> {
    let path = api.path("/observe").await?;
    let mut cursor = args.after;
    loop {
        let request = ObserveRequest {
            channel_id: Some(args.channel.clone()),
            after: cursor.clone(),
            initial: Initial::All,
        };
        cursor = tokio::select! {
            result = api.client.observe(&path, &request, |event| watch_event(api, event)) => result?,
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(()); }
        };
    }
}

fn watch_event(api: &Api, event: ObserveEvent) -> Result<std::ops::ControlFlow<Option<String>>> {
    match event {
        ObserveEvent::State { .. } => {}
        ObserveEvent::Messages { messages, .. } => {
            for message in messages {
                emit(&message, output::stream_format(api.format))?;
            }
        }
        ObserveEvent::Reconnect { cursor } => return Ok(std::ops::ControlFlow::Break(cursor)),
        ObserveEvent::Error { code, error, clear } => {
            if !matches!(api.format, OutputFormat::Text) {
                emit(
                    &json!({"type":"error","code":code,"error":error,"clear":clear}),
                    output::stream_format(api.format),
                )?;
            }
            return Err(anyhow!("Crew observation stopped [{}]: {}. Review the connection and cursor before watching again", output::safe_text(&code), output::safe_text(&error)));
        }
    }
    Ok(std::ops::ControlFlow::Continue(()))
}

async fn tasks(api: &Api, command: TaskCommand) -> Result<()> {
    let result = match command {
        TaskCommand::Start {
            channel,
            prompt,
            provider,
            model,
            context_channels,
            allow_posting,
        } => {
            ensure!(
                allow_posting,
                "Starting a Crew task requires --allow-posting for its destination channel"
            );
            api.connection_action("runs", json!({"request_id":api.request_id,"channel_id":channel,"prompt":text_input(prompt)?,"provider":provider,"model":model,"context_channels":context_channels,"posting_grant":allow_posting})).await?
        }
        TaskCommand::List => {
            api.client
                .request("GET", &api.path("/runs").await?, None)
                .await?
        }
        TaskCommand::Show { run } => task(api, &run).await?,
        TaskCommand::Watch { run } => return watch_task(api, &run).await,
        TaskCommand::Cancel { run } => {
            api.connection_action(
                &format!("runs/{}/cancel", component(&run)?),
                json!({"request_id":api.request_id}),
            )
            .await?
        }
    };
    emit(&result, api.format)
}

async fn task(api: &Api, id: &str) -> Result<Value> {
    component(id)?;
    let result = api
        .client
        .request("GET", &api.path("/runs").await?, None)
        .await?;
    result["runs"]
        .as_array()
        .context("Daemon returned an invalid task list")?
        .iter()
        .find(|run| run["run_id"].as_str() == Some(id))
        .cloned()
        .context("Task was not found on this device and connection")
}

async fn watch_task(api: &Api, id: &str) -> Result<()> {
    let mut previous = Value::Null;
    loop {
        let current = tokio::select! {
            result = task(api, id) => result?,
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(()); }
        };
        if current != previous {
            emit(&current, output::stream_format(api.format))?;
        }
        if matches!(
            current["status"].as_str(),
            Some(
                "completed"
                    | "failed"
                    | "cancelled"
                    | "interrupted"
                    | "outcome_not_durable"
                    | "cancellation_unconfirmed"
            )
        ) {
            return Ok(());
        }
        previous = current;
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(()); }
        }
    }
}

async fn grants(api: &Api, command: GrantCommand) -> Result<Value> {
    match command {
        GrantCommand::List => {
            api.client
                .request("GET", &api.path("/grants").await?, None)
                .await
        }
        GrantCommand::Grant {
            session,
            channel,
            context_channels,
        } => {
            api.connection_action(
                &format!("sessions/{}/grant", component(&session)?),
                json!({"channel_id":channel,"context_channels":context_channels}),
            )
            .await
        }
        GrantCommand::Revoke { session } => {
            api.connection_action(
                &format!("sessions/{}/revoke", component(&session)?),
                json!({"request_id":api.request_id}),
            )
            .await
        }
    }
}

async fn privacy(api: &Api, command: PrivacyCommand) -> Result<Value> {
    match command {
        PrivacyCommand::Show => {
            let connection = api.connection().await?;
            let snapshot = api.snapshot().await?;
            Ok(
                json!({"connection_id":connection["id"],"personal_mode":connection["mode"],"connection_policy_epoch":connection["policy_epoch"],"workspace":snapshot["workspace"],"channels":snapshot["channels"]}),
            )
        }
        PrivacyCommand::SetPersonal { mode } => {
            let connection = api.connection().await?;
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
                "cluster_connection_id",
            ] {
                if let Some(value) = connection.get(key) {
                    input.insert(key.into(), value.clone());
                }
            }
            input.insert("mode".into(), json!(mode.as_str()));
            api.client
                .request("PATCH", &api.path("").await?, Some(Value::Object(input)))
                .await
        }
        PrivacyCommand::SetWorkspace { mode } => {
            api.broker("policy.set", json!({"mode":mode.as_str()}), true)
                .await
        }
    }
}
