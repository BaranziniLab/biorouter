use crate::action_required_manager::ActionRequiredManager;
use crate::agents::types::SharedProvider;
use crate::privacy::CallCapability;
use crate::session_context::SESSION_ID_HEADER;
use rmcp::model::{
    Content, CreateElicitationRequestParams, CreateElicitationResult, ElicitationAction, ErrorCode,
    Extensions, JsonObject, Meta, NumberOrString, ProgressToken,
};
/// MCP client implementation for Biorouter
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotification,
        CancelledNotificationMethod, CancelledNotificationParam, ClientCapabilities, ClientInfo,
        ClientRequest, CreateMessageRequestParams, CreateMessageResult, GetPromptRequest,
        GetPromptRequestParams, GetPromptResult, Implementation, InitializeResult,
        ListPromptsRequest, ListPromptsResult, ListResourcesRequest, ListResourcesResult,
        ListToolsRequest, ListToolsResult, LoggingMessageNotification,
        LoggingMessageNotificationMethod, PaginatedRequestParams, ProgressNotification,
        ProgressNotificationMethod, ProtocolVersion, ReadResourceRequest,
        ReadResourceRequestParams, ReadResourceResult, RequestId, Role, SamplingMessage,
        ServerNotification, ServerResult,
    },
    service::{
        ClientInitializeError, PeerRequestOptions, RequestContext, RequestHandle, RunningService,
        ServiceRole,
    },
    transport::IntoTransport,
    ClientHandler, ErrorData, Peer, RoleClient, ServiceError, ServiceExt,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::{sync::Arc, time::Duration};
use tokio::sync::{
    mpsc::{self, Sender},
    Mutex,
};
use tokio_util::sync::CancellationToken;

type SamplingBinding = (
    std::sync::Weak<Mutex<Option<Arc<dyn crate::providers::base::Provider>>>>,
    Vec<String>,
);
static SAMPLING_BINDINGS: std::sync::LazyLock<std::sync::Mutex<Vec<SamplingBinding>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

pub(crate) fn bind_sampling_session(
    provider: &SharedProvider,
    session_id: &str,
) -> anyhow::Result<()> {
    let mut bindings = SAMPLING_BINDINGS
        .lock()
        .map_err(|_| anyhow::anyhow!("MCP sampling attribution is unavailable"))?;
    bindings.retain(|(provider, _)| provider.strong_count() > 0);
    if let Some((_, sessions)) = bindings.iter_mut().find(|(entry, _)| {
        entry
            .upgrade()
            .is_some_and(|bound| Arc::ptr_eq(&bound, provider))
    }) {
        if !sessions.iter().any(|id| id == session_id) {
            sessions.push(session_id.to_string());
        }
    } else {
        bindings.push((Arc::downgrade(provider), vec![session_id.to_string()]));
    }
    Ok(())
}

pub(crate) async fn crew_sampling_allowed(provider: &SharedProvider) -> anyhow::Result<()> {
    let sessions = {
        let mut bindings = SAMPLING_BINDINGS
            .lock()
            .map_err(|_| anyhow::anyhow!("MCP sampling attribution is unavailable"))?;
        bindings.retain(|(provider, _)| provider.strong_count() > 0);
        bindings
            .iter()
            .find(|(entry, _)| {
                entry
                    .upgrade()
                    .is_some_and(|bound| Arc::ptr_eq(&bound, provider))
            })
            .map(|(_, sessions)| sessions.clone())
            .unwrap_or_default()
    };
    let crew = crate::crew::manager()?;
    for session in sessions {
        anyhow::ensure!(
            !crew.is_scoped_session(&session).await,
            "Unattributed auxiliary sampling is unavailable for Crew-scoped sessions"
        );
    }
    Ok(())
}

pub type BoxError = Box<dyn std::error::Error + Sync + Send>;

pub type Error = rmcp::ServiceError;

/// The shared handle type for an MCP client. A `SharedMcpPool` hands out clones
/// of one of these so N agents address one process (BR-54); the unpooled path
/// still owns a unique one per extension.
///
/// H6: this is deliberately NOT wrapped in a `Mutex`. `McpClientTrait::call_tool`
/// takes `&self` and every implementation is internally synchronized (the rmcp
/// transport guards its own framing; in-process servers guard their own state),
/// so an outer mutex bought nothing and cost everything: it was held across the
/// entire `call_tool` await, converting `max(tool durations)` into
/// `sum(tool durations)` for concurrent calls on one extension.
/// See `h6_parallel_same_extension` in `extension_manager.rs`.
pub type McpClientBox = Arc<dyn McpClientTrait>;

/// Per-dispatch notification routes, keyed by the MCP progress token assigned to
/// a single tool call. Shared between the [`McpClient`] and its [`BioRouterClient`]
/// handler so server progress notifications can be routed back to exactly the
/// session that made the call, instead of broadcast to every session sharing the
/// process (the isolation core of the SharedMcpPool).
type ProgressRoutes = Arc<Mutex<HashMap<String, Sender<ServerNotification>>>>;

/// The session ids with an in-flight `call_tool` on this client connection —
/// a multiset, one entry per in-flight call. Shared between the [`McpClient`]
/// (whose `call_tool` records each dispatch for its duration) and its
/// [`BioRouterClient`] handler, so a server-initiated elicitation raised
/// mid-call can be attributed to the session actually running a tool on this
/// connection (#40): the `ActionRequiredManager` then delivers the request
/// ONLY to that session's agent loop, instead of letting whichever concurrent
/// session wins the wake-up race persist the prompt under its own session id.
/// A `std::sync::Mutex` so the RAII guard can clean up in `Drop` (a cancelled
/// dispatch drops the `call_tool` future without reaching any `.await`).
type ActiveCallSessions = Arc<std::sync::Mutex<Vec<String>>>;

/// RAII entry in [`ActiveCallSessions`]: registers the dispatching session on
/// creation, removes ONE occurrence on drop — including when the `call_tool`
/// future is dropped mid-await by a cancellation.
struct ActiveCallGuard {
    sessions: ActiveCallSessions,
    session_id: String,
}

impl ActiveCallGuard {
    fn register(sessions: &ActiveCallSessions, session_id: &str) -> Self {
        sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(session_id.to_string());
        Self {
            sessions: sessions.clone(),
            session_id: session_id.to_string(),
        }
    }
}

impl Drop for ActiveCallGuard {
    fn drop(&mut self) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pos) = sessions.iter().position(|s| s == &self.session_id) {
            sessions.swap_remove(pos);
        }
    }
}

/// The session an incoming elicitation request belongs to (#40).
///
/// MCP gives an elicitation no linkage to the tool call that raised it, so
/// attribution is inferred:
/// 1. a server that echoes the `biorouter-session-id` meta we attach to every
///    call is believed exactly;
/// 2. otherwise, if every in-flight tool call on this connection belongs to
///    ONE session, it must be that session's;
/// 3. otherwise (`None`) — no in-flight call, or a shared pooled client
///    running calls for several sessions at once — the request goes out
///    unscoped, deliverable by any session's loop (the pre-scoping behavior,
///    kept so it still surfaces instead of timing out silently).
fn elicitation_session_scope(meta: &Meta, active: &ActiveCallSessions) -> Option<String> {
    if let Some(Value::String(session_id)) = meta
        .0
        .iter()
        .find_map(|(k, v)| k.eq_ignore_ascii_case(SESSION_ID_HEADER).then_some(v))
    {
        return Some(session_id.clone());
    }
    let sessions = active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut ids = sessions.iter();
    let first = ids.next()?;
    ids.all(|s| s == first).then(|| first.clone())
}

#[derive(Clone, Debug)]
pub struct McpMeta {
    pub session_id: String,
    pub computer_use_generation: Option<String>,
    /// Per-dispatch MCP progress token. When set, it is written into the call's
    /// `_meta.progressToken` so the server echoes it on progress notifications,
    /// letting a pooled (shared) client route those notifications to exactly this
    /// dispatch's session (BR-54). `None` on the unpooled path (legacy broadcast).
    pub progress_token: Option<String>,
    /// Issue #56. The capability this call was ADMITTED on. Set from
    /// `dispatch_tool_call`'s parameter, never re-derived: an in-process
    /// extension that re-reads the provider mutex from inside the driven future
    /// reads it minutes later, past the dispatch semaphore.
    pub capability: CallCapability,
    /// Whether the model bound to this session is private (issue #56).
    ///
    /// `None` for every extension that is not a Biorouter built-in: the session
    /// id already goes to third-party MCP servers, and this deliberately does
    /// not follow that precedent — "this user is on an institutional model" is a
    /// fact about their configuration, not something a third-party server needs.
    /// A built-in receiving `None` reads it as PUBLIC, which is the safe
    /// direction for every gate that consumes it.
    ///
    /// Distinct from `capability` above, which never leaves the process: this is
    /// the ON-THE-WIRE disclosure, and it is opt-in per extension.
    pub capability_private: Option<bool>,
    /// Whose agreements cover the model bound to this session (issue #56, DR-26
    /// / Task 50 Step 0), already in the wire spelling
    /// [`biorouter_mcp::knowledge::affiliation::capability_meta_value`] produces.
    ///
    /// A **second** key beside [`Self::capability_private`] rather than a richer
    /// value on that one: widening the tier key's grammar would make
    /// `tier::caller_is_private` — which compares against the exact word
    /// `private` — read every new-grammar value as PUBLIC in any binary that has
    /// not been updated, and this tree ships a separately-installed PATH CLI
    /// that routinely lags the app. The whole argument is in that module's
    /// header.
    ///
    /// `None` on the same terms as `capability_private`: built-ins only, and a
    /// built-in that receives nothing reads
    /// [`CallerAffiliation::Unstated`](biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated),
    /// which is the restrictive answer for every gate that consumes it.
    pub capability_affiliation: Option<String>,
    /// The caller received Workspace Control only as the derived delegation
    /// surface. Read/watch/close must then stay within its direct subagent
    /// children even when privacy tiers are disabled.
    pub workspace_child_scope_only: bool,
}

impl McpMeta {
    pub fn new(session_id: impl Into<String>, capability: CallCapability) -> Self {
        Self {
            session_id: session_id.into(),
            computer_use_generation: None,
            progress_token: None,
            capability,
            capability_private: None,
            capability_affiliation: None,
            workspace_child_scope_only: false,
        }
    }

    /// Attach a progress token so this call's notifications can be routed back to
    /// this session on a shared client.
    pub fn with_progress_token(mut self, token: impl Into<String>) -> Self {
        self.progress_token = Some(token.into());
        self
    }

    /// Disclose the caller's capability tier to this call's server (issue #56).
    /// Built-ins only — see [`McpMeta::capability_private`].
    pub fn with_capability_private(mut self, private: bool) -> Self {
        self.capability_private = Some(private);
        self
    }

    pub fn with_workspace_child_scope_only(mut self, restricted: bool) -> Self {
        self.workspace_child_scope_only = restricted;
        self
    }

    /// Disclose the caller's **affiliation** to this call's server (issue #56,
    /// DR-26). Built-ins only — see [`McpMeta::capability_affiliation`].
    ///
    /// The wire spelling is composed by the reader's own crate
    /// ([`biorouter_mcp::knowledge::affiliation::capability_meta_value`]), not
    /// here: one spelling, one function, both sides. `None` — an unstated
    /// affiliation — leaves the key off entirely, which is exactly how an older
    /// daemon looks and is read the same restrictive way.
    pub fn with_capability_affiliation(mut self, affiliation: Option<String>) -> Self {
        self.capability_affiliation = affiliation;
        self
    }

    pub(crate) fn inject_into_extensions(&self, extensions: Extensions) -> Extensions {
        let mut extensions = inject_session_id_into_extensions(extensions, &self.session_id);
        if let Some(generation) = &self.computer_use_generation {
            let mut meta = extensions.get::<Meta>().cloned().unwrap_or_default();
            meta.0.insert(
                "computer_use_generation".into(),
                serde_json::Value::String(generation.clone()),
            );
            extensions.insert(meta);
        }
        if let Some(private) = self.capability_private {
            // Issue #56. The SAME `_meta` object the session id rides in, for
            // the same wire-collision reason the progress token below gives.
            // BOTH halves come from the shared module — the key from its const
            // and the value from `capability_meta_value` — because the reader
            // (`tier::caller_is_private`) compares against that module's own
            // spelling, and a hand-typed literal here would drift silently.
            let mut meta = extensions.get::<Meta>().cloned().unwrap_or_default();
            meta.0.insert(
                biorouter_mcp::knowledge::tier::CAPABILITY_TIER_META_KEY.to_string(),
                serde_json::Value::String(
                    biorouter_mcp::knowledge::tier::capability_meta_value(private).to_string(),
                ),
            );
            extensions.insert(meta);
        }
        if let Some(affiliation) = &self.capability_affiliation {
            // Issue #56 DR-26 / Task 50 Step 0. The SAME `_meta` object again,
            // under its own key: `tier::caller_is_private` compares the tier
            // key's value against the exact word `private`, so a richer value
            // there would read PUBLIC on any binary that has not been updated.
            let mut meta = extensions.get::<Meta>().cloned().unwrap_or_default();
            meta.0.insert(
                biorouter_mcp::knowledge::affiliation::CAPABILITY_AFFILIATION_META_KEY.to_string(),
                serde_json::Value::String(affiliation.clone()),
            );
            extensions.insert(meta);
        }
        if let Some(token) = &self.progress_token {
            // Add the progressToken to the SAME `_meta` object the session id
            // rides in (rmcp serializes `extensions.get::<Meta>()` as params._meta),
            // so we never set the params.meta field and can't collide on the wire.
            let mut meta = extensions.get::<Meta>().cloned().unwrap_or_default();
            meta.set_progress_token(ProgressToken(NumberOrString::String(token.clone().into())));
            extensions.insert(meta);
        }
        extensions
    }
}

#[async_trait::async_trait]
pub trait McpClientTrait: Send + Sync {
    async fn list_tools(
        &self,
        next_cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error>;

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        meta: McpMeta,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error>;

    fn get_info(&self) -> Option<&InitializeResult>;

    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: Value,
        _cancel_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        mpsc::channel(1).1
    }

    /// Register a per-dispatch notification route and return the progress token to
    /// attach to the call plus the receiver that will get ONLY this dispatch's
    /// notifications. The default implementation returns no token and falls back
    /// to the broadcast [`subscribe`](Self::subscribe) — i.e. legacy behavior for
    /// clients that are not shared across sessions.
    async fn register_dispatch(&self) -> (Option<String>, mpsc::Receiver<ServerNotification>) {
        (None, self.subscribe().await)
    }

    /// Whether the client's transport is still believed to be alive. Used by the
    /// SharedMcpPool to evict and respawn a dead shared process instead of handing
    /// it to a new session. Default: assume alive.
    fn is_running(&self) -> bool {
        true
    }

    async fn get_moim(&self, _session_id: &str) -> Option<String> {
        None
    }
}

pub struct BioRouterClient {
    /// Legacy broadcast subscribers (the unpooled path). Empty on pooled clients.
    notification_handlers: Arc<Mutex<Vec<Sender<ServerNotification>>>>,
    /// Per-dispatch routes keyed by progress token (the pooled/isolated path).
    progress_routes: ProgressRoutes,
    /// When true (a pooled client shared across sessions), a notification that
    /// cannot be attributed to a registered progress token is DROPPED rather than
    /// broadcast, so one session never sees another's progress/log stream.
    routed_only: bool,
    /// Sessions with an in-flight `call_tool` on this connection, for
    /// attributing server-initiated elicitations to their session (#40).
    active_call_sessions: ActiveCallSessions,
    provider: SharedProvider,
}

impl BioRouterClient {
    pub fn new(
        handlers: Arc<Mutex<Vec<Sender<ServerNotification>>>>,
        provider: SharedProvider,
    ) -> Self {
        Self::with_routing(
            handlers,
            Arc::new(Mutex::new(HashMap::new())),
            false,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            provider,
        )
    }

    pub fn with_routing(
        handlers: Arc<Mutex<Vec<Sender<ServerNotification>>>>,
        progress_routes: ProgressRoutes,
        routed_only: bool,
        active_call_sessions: ActiveCallSessions,
        provider: SharedProvider,
    ) -> Self {
        BioRouterClient {
            notification_handlers: handlers,
            progress_routes,
            routed_only,
            active_call_sessions,
            provider,
        }
    }

    /// Deliver one notification, either to the single subscriber that owns
    /// `token` (isolated routing) or, when there is no token match, to the legacy
    /// broadcast set — unless this is a shared (`routed_only`) client, in which
    /// case an unattributable notification is dropped. Factored out so the
    /// routing decision is unit-testable without a live MCP server.
    async fn deliver(&self, token: Option<&str>, notification: ServerNotification) {
        if let Some(token) = token {
            if let Some(sender) = self.progress_routes.lock().await.get(token) {
                let _ = sender.try_send(notification);
                return;
            }
        }
        if self.routed_only {
            // Shared client, no owning session -> drop rather than bleed across sessions.
            return;
        }
        for handler in self.notification_handlers.lock().await.iter() {
            let _ = handler.try_send(notification.clone());
        }
    }
}

/// Extract the string form of a progress token for use as a route key.
fn progress_token_key(token: &ProgressToken) -> String {
    match &token.0 {
        NumberOrString::String(s) => s.to_string(),
        NumberOrString::Number(n) => n.to_string(),
    }
}

impl ClientHandler for BioRouterClient {
    async fn on_progress(
        &self,
        params: rmcp::model::ProgressNotificationParam,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        let token = progress_token_key(&params.progress_token);
        let notification = ServerNotification::ProgressNotification(ProgressNotification {
            params,
            method: ProgressNotificationMethod,
            extensions: context.extensions.clone(),
        });
        self.deliver(Some(&token), notification).await;
    }

    async fn on_logging_message(
        &self,
        params: rmcp::model::LoggingMessageNotificationParam,
        context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        // Logging notifications carry no progress token, so a shared client cannot
        // attribute them to a session -> `deliver` drops them when `routed_only`.
        let notification =
            ServerNotification::LoggingMessageNotification(LoggingMessageNotification {
                params,
                method: LoggingMessageNotificationMethod,
                extensions: context.extensions.clone(),
            });
        self.deliver(None, notification).await;
    }

    async fn create_message(
        &self,
        params: CreateMessageRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateMessageResult, ErrorData> {
        // Attribute from local agent bindings and dispatch leases, never from
        // session labels supplied by the external MCP server.
        let admission = async {
            crew_sampling_allowed(&self.provider).await?;
            let sessions = self
                .active_call_sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("MCP sampling attribution is unavailable"))?
                .clone();
            let crew = crate::crew::manager()?;
            for session in sessions {
                anyhow::ensure!(
                    !crew.is_scoped_session(&session).await,
                    "MCP sampling is unavailable for Crew-scoped sessions"
                );
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if admission.is_err() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                "MCP sampling is unavailable under the current Crew scope",
                None,
            ));
        }
        let provider = self
            .provider
            .lock()
            .await
            .as_ref()
            .ok_or(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Could not use provider",
                None,
            ))?
            .clone();

        let provider_ready_messages: Vec<crate::conversation::message::Message> = params
            .messages
            .iter()
            .map(|msg| {
                let base = match msg.role {
                    Role::User => crate::conversation::message::Message::user(),
                    Role::Assistant => crate::conversation::message::Message::assistant(),
                };

                match msg.content.as_text() {
                    Some(text) => base.with_text(&text.text),
                    None => base.with_content(msg.content.clone().into()),
                }
            })
            .collect();

        let system_prompt = params
            .system_prompt
            .as_deref()
            .unwrap_or("You are a general-purpose AI agent called biorouter");

        let (response, usage) = provider
            .complete(system_prompt, &provider_ready_messages, &[])
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Unexpected error while completing the prompt",
                    Some(Value::from(e.to_string())),
                )
            })?;

        Ok(CreateMessageResult {
            model: usage.model,
            stop_reason: Some(CreateMessageResult::STOP_REASON_END_TURN.to_string()),
            message: SamplingMessage {
                role: Role::Assistant,
                // TODO(alexhancock): MCP sampling currently only supports one content on each SamplingMessage
                // https://modelcontextprotocol.io/specification/draft/client/sampling#messages
                // This doesn't mesh well with biorouter's approach which has Vec<MessageContent>
                // There is a proposal to MCP which is agreed to go in the next version to have SamplingMessages support multiple content parts
                // https://github.com/modelcontextprotocol/modelcontextprotocol/pull/198
                // Until that is formalized, we can take the first message content from the provider and use it
                content: if let Some(content) = response.content.first() {
                    match content {
                        crate::conversation::message::MessageContent::Text(text) => {
                            Content::text(&text.text)
                        }
                        crate::conversation::message::MessageContent::Image(img) => {
                            Content::image(&img.data, &img.mime_type)
                        }
                        // TODO(alexhancock) - Content::Audio? biorouter's messages don't currently have it
                        _ => Content::text(""),
                    }
                } else {
                    Content::text("")
                },
            },
        })
    }

    async fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, ErrorData> {
        let schema_value = serde_json::to_value(&request.requested_schema).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize elicitation schema: {}", e),
                None,
            )
        })?;

        // #40: attribute the request to the session whose tool call raised
        // it, so the ActionRequiredManager delivers the prompt to THAT
        // session's loop only — never to a concurrent session's UI.
        let session_scope = elicitation_session_scope(&context.meta, &self.active_call_sessions);

        ActionRequiredManager::global()
            .request_and_wait(
                request.message.clone(),
                schema_value,
                Duration::from_secs(300),
                session_scope.as_deref(),
            )
            .await
            .map(|outcome| match outcome {
                Some(user_data) => CreateElicitationResult {
                    action: ElicitationAction::Accept,
                    content: Some(user_data),
                },
                // The user (or an unattended run that can never collect
                // input, #40) cancelled: a first-class, model-visible MCP
                // outcome — the server sees `action: cancel` instead of a
                // 300s timeout error.
                None => CreateElicitationResult {
                    action: ElicitationAction::Cancel,
                    content: None,
                },
            })
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Elicitation request timed out or failed: {}", e),
                    None,
                )
            })
    }

    fn get_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ClientCapabilities::builder()
                .enable_sampling()
                .enable_elicitation()
                .build(),
            client_info: Implementation {
                name: "biorouter".to_string(),
                version: std::env::var("BIOROUTER_MCP_CLIENT_VERSION")
                    .unwrap_or(env!("CARGO_PKG_VERSION").to_owned()),
                icons: None,
                title: None,
                website_url: None,
            },
            meta: None,
        }
    }
}

/// The MCP client is the interface for MCP operations.
pub struct McpClient {
    client: Mutex<RunningService<RoleClient, BioRouterClient>>,
    notification_subscribers: Arc<Mutex<Vec<mpsc::Sender<ServerNotification>>>>,
    /// Per-dispatch progress-token routes (shared with the `BioRouterClient`).
    progress_routes: ProgressRoutes,
    /// In-flight `call_tool` session ids (shared with the `BioRouterClient`),
    /// for attributing server-initiated elicitations to their session (#40).
    active_call_sessions: ActiveCallSessions,
    /// When true, this client is shared across sessions (via a SharedMcpPool) and
    /// notifications are routed per-dispatch instead of broadcast.
    routed_only: bool,
    /// Monotonic counter feeding unique per-dispatch progress tokens.
    next_token: AtomicU64,
    /// Cleared when the transport is observed closed, so a pooled client can be
    /// evicted and respawned rather than handed to a new session (BR-54 recovery).
    healthy: Arc<AtomicBool>,
    server_info: Option<InitializeResult>,
    timeout: std::time::Duration,
}

impl McpClient {
    /// Connect an unpooled client: notifications broadcast to every subscriber
    /// (legacy behavior). Kept as the entry point for per-session/per-app clients.
    pub async fn connect<T, E, A>(
        transport: T,
        timeout: std::time::Duration,
        provider: SharedProvider,
    ) -> Result<Self, ClientInitializeError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + From<std::io::Error> + Send + Sync + 'static,
    {
        Self::connect_routed(transport, timeout, provider, false).await
    }

    /// Connect a client, choosing whether its notifications are isolated per
    /// dispatch (`routed_only = true`, for SharedMcpPool clients shared across
    /// sessions) or broadcast to all subscribers (`false`, legacy per-session).
    pub async fn connect_routed<T, E, A>(
        transport: T,
        timeout: std::time::Duration,
        provider: SharedProvider,
        routed_only: bool,
    ) -> Result<Self, ClientInitializeError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + From<std::io::Error> + Send + Sync + 'static,
    {
        let notification_subscribers =
            Arc::new(Mutex::new(Vec::<mpsc::Sender<ServerNotification>>::new()));
        let progress_routes: ProgressRoutes = Arc::new(Mutex::new(HashMap::new()));
        let active_call_sessions: ActiveCallSessions = Arc::new(std::sync::Mutex::new(Vec::new()));

        let client = BioRouterClient::with_routing(
            notification_subscribers.clone(),
            progress_routes.clone(),
            routed_only,
            active_call_sessions.clone(),
            provider,
        );
        let client: rmcp::service::RunningService<rmcp::RoleClient, BioRouterClient> =
            client.serve(transport).await?;
        let server_info = client.peer_info().cloned();

        Ok(Self {
            client: Mutex::new(client),
            notification_subscribers,
            progress_routes,
            active_call_sessions,
            routed_only,
            next_token: AtomicU64::new(0),
            healthy: Arc::new(AtomicBool::new(true)),
            server_info,
            timeout,
        })
    }

    /// A shared handle to this client's health flag (for the SharedMcpPool).
    pub fn health_flag(&self) -> Arc<AtomicBool> {
        self.healthy.clone()
    }

    async fn send_request(
        &self,
        request: ClientRequest,
        cancel_token: CancellationToken,
    ) -> Result<ServerResult, Error> {
        let handle = self
            .client
            .lock()
            .await
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .inspect_err(|_| self.healthy.store(false, Ordering::Relaxed))?;

        let result = await_response(handle, self.timeout, &cancel_token).await;
        if matches!(result, Err(ServiceError::TransportClosed)) {
            self.healthy.store(false, Ordering::Relaxed);
        }
        result
    }

    /// Mint a unique progress token, register a bounded channel under it, and
    /// return the token plus the receiver. Only used on shared (`routed_only`)
    /// clients — the returned token is attached to the call so the server echoes
    /// it, and `deregister_progress` removes the route when the call finishes.
    async fn register_progress(&self) -> (String, mpsc::Receiver<ServerNotification>) {
        let token = format!(
            "bp-{:x}-{}",
            self as *const _ as usize,
            self.next_token.fetch_add(1, Ordering::Relaxed)
        );
        let (tx, rx) = mpsc::channel(16);
        let mut routes = self.progress_routes.lock().await;
        // Opportunistically drop routes whose receiver is gone (e.g. a dispatch
        // cancelled before its call ran) so the map can't grow without bound.
        routes.retain(|_, sender| !sender.is_closed());
        routes.insert(token.clone(), tx);
        (token, rx)
    }

    async fn deregister_progress(&self, token: &str) {
        self.progress_routes.lock().await.remove(token);
    }
}

async fn await_response(
    handle: RequestHandle<RoleClient>,
    timeout: Duration,
    cancel_token: &CancellationToken,
) -> Result<<RoleClient as ServiceRole>::PeerResp, ServiceError> {
    let receiver = handle.rx;
    let peer = handle.peer;
    let request_id = handle.id;
    tokio::select! {
        result = receiver => {
            result.map_err(|_e| ServiceError::TransportClosed)?
        }
        _ = tokio::time::sleep(timeout) => {
            send_cancel_message(&peer, request_id, Some("timed out".to_owned())).await?;
            Err(ServiceError::Timeout{timeout})
        }
        _ = cancel_token.cancelled() => {
            send_cancel_message(&peer, request_id, Some("operation cancelled".to_owned())).await?;
            Err(ServiceError::Cancelled { reason: None })
        }
    }
}

async fn send_cancel_message(
    peer: &Peer<RoleClient>,
    request_id: RequestId,
    reason: Option<String>,
) -> Result<(), ServiceError> {
    peer.send_notification(
        CancelledNotification {
            params: CancelledNotificationParam { request_id, reason },
            method: CancelledNotificationMethod,
            extensions: Default::default(),
        }
        .into(),
    )
    .await
}

#[async_trait::async_trait]
impl McpClientTrait for McpClient {
    fn get_info(&self) -> Option<&InitializeResult> {
        self.server_info.as_ref()
    }

    async fn list_resources(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ListResourcesRequest(ListResourcesRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_current_session_id_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListResourcesResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn read_resource(
        &self,
        uri: &str,
        cancel_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ReadResourceRequest(ReadResourceRequest {
                    params: ReadResourceRequestParams {
                        uri: uri.to_string(),
                        meta: None,
                    },
                    method: Default::default(),
                    extensions: inject_current_session_id_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ReadResourceResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn list_tools(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ListToolsRequest(ListToolsRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_current_session_id_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListToolsResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        meta: McpMeta,
        cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        // Remember the per-dispatch progress route so we can clean it up after the
        // call completes (or errors), regardless of outcome.
        let progress_token = meta.progress_token.clone();
        // #40: record which session this call belongs to for the duration of
        // the dispatch, so an elicitation the server raises mid-call can be
        // attributed to it (see `elicitation_session_scope`). RAII: the guard
        // also unregisters when a cancellation drops this future mid-await.
        let _active_call = ActiveCallGuard::register(&self.active_call_sessions, &meta.session_id);
        let res = self
            .send_request(
                ClientRequest::CallToolRequest(CallToolRequest {
                    params: CallToolRequestParams {
                        task: None,
                        name: name.to_string().into(),
                        arguments,
                        meta: None,
                    },
                    method: Default::default(),
                    extensions: meta.inject_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await;

        if let Some(token) = progress_token {
            self.deregister_progress(&token).await;
        }

        match res? {
            ServerResult::CallToolResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn list_prompts(
        &self,
        cursor: Option<String>,
        cancel_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        let res = self
            .send_request(
                ClientRequest::ListPromptsRequest(ListPromptsRequest {
                    params: Some(PaginatedRequestParams { cursor, meta: None }),
                    method: Default::default(),
                    extensions: inject_current_session_id_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::ListPromptsResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn get_prompt(
        &self,
        name: &str,
        arguments: Value,
        cancel_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        let arguments = match arguments {
            Value::Object(map) => Some(map),
            _ => None,
        };
        let res = self
            .send_request(
                ClientRequest::GetPromptRequest(GetPromptRequest {
                    params: GetPromptRequestParams {
                        name: name.to_string(),
                        arguments,
                        meta: None,
                    },
                    method: Default::default(),
                    extensions: inject_current_session_id_into_extensions(Default::default()),
                }),
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::GetPromptResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (tx, rx) = mpsc::channel(16);
        self.notification_subscribers.lock().await.push(tx);
        rx
    }

    async fn register_dispatch(&self) -> (Option<String>, mpsc::Receiver<ServerNotification>) {
        if self.routed_only {
            // Shared client: mint a token and route ONLY this dispatch's
            // notifications, so a concurrent call from another session cannot
            // land in this receiver.
            let (token, rx) = self.register_progress().await;
            (Some(token), rx)
        } else {
            // Unpooled client: legacy broadcast subscription, no token.
            (None, self.subscribe().await)
        }
    }

    fn is_running(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
}

/// Injects the given session_id into Extensions._meta.
fn inject_session_id_into_extensions(mut extensions: Extensions, session_id: &str) -> Extensions {
    let mut meta_map = extensions
        .get::<Meta>()
        .map(|meta| meta.0.clone())
        .unwrap_or_default();

    // JsonObject is case-sensitive, so we use retain for case-insensitive removal
    meta_map.retain(|k, _| !k.eq_ignore_ascii_case(SESSION_ID_HEADER));

    meta_map.insert(
        SESSION_ID_HEADER.to_string(),
        Value::String(session_id.to_string()),
    );

    extensions.insert(Meta(meta_map));
    extensions
}

/// Injects session ID from task-local context into Extensions._meta.
fn inject_current_session_id_into_extensions(extensions: Extensions) -> Extensions {
    if let Some(session_id) = crate::session_context::current_session_id() {
        inject_session_id_into_extensions(extensions, &session_id)
    } else {
        extensions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Meta;

    fn empty_provider() -> SharedProvider {
        Arc::new(Mutex::new(None))
    }

    fn progress_notif(token: &str) -> ServerNotification {
        ServerNotification::ProgressNotification(ProgressNotification {
            params: rmcp::model::ProgressNotificationParam {
                progress_token: ProgressToken(NumberOrString::String(token.into())),
                progress: 1.0,
                total: None,
                message: None,
            },
            method: ProgressNotificationMethod,
            extensions: Default::default(),
        })
    }

    /// The isolation core: on a shared (`routed_only`) client, a progress
    /// notification for session B's token must reach ONLY session B's receiver,
    /// never session A's. A naive broadcast would fail this.
    #[tokio::test]
    async fn test_shared_client_routes_progress_to_owning_session_only() {
        let routes: ProgressRoutes = Arc::new(Mutex::new(HashMap::new()));
        let client = BioRouterClient::with_routing(
            Arc::new(Mutex::new(Vec::new())),
            routes.clone(),
            true,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            empty_provider(),
        );

        let (tx_a, mut rx_a) = mpsc::channel(4);
        let (tx_b, mut rx_b) = mpsc::channel(4);
        {
            let mut r = routes.lock().await;
            r.insert("tok-A".to_string(), tx_a);
            r.insert("tok-B".to_string(), tx_b);
        }

        client.deliver(Some("tok-B"), progress_notif("tok-B")).await;

        assert!(
            rx_b.try_recv().is_ok(),
            "session B must receive its own progress"
        );
        assert!(
            rx_a.try_recv().is_err(),
            "session A must NOT receive session B's progress (no cross-session bleed)"
        );
    }

    /// A shared client drops any notification it cannot attribute to a token
    /// (unknown token, or an untokened logging message) rather than bleeding it.
    #[tokio::test]
    async fn test_shared_client_drops_unattributable_notifications() {
        let routes: ProgressRoutes = Arc::new(Mutex::new(HashMap::new()));
        let (tx_a, mut rx_a) = mpsc::channel(4);
        routes.lock().await.insert("tok-A".to_string(), tx_a);

        let client = BioRouterClient::with_routing(
            Arc::new(Mutex::new(Vec::new())),
            routes,
            true,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            empty_provider(),
        );

        // Unknown token -> dropped.
        client
            .deliver(Some("tok-unknown"), progress_notif("tok-unknown"))
            .await;
        // Untokened (e.g. a logging message) -> dropped.
        client.deliver(None, progress_notif("ignored")).await;

        assert!(
            rx_a.try_recv().is_err(),
            "an unattributable notification must not leak to another session"
        );
    }

    /// An unpooled client keeps the legacy broadcast behavior: an untokened
    /// notification reaches every subscriber.
    #[tokio::test]
    async fn test_unpooled_client_broadcasts_untokened_notifications() {
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (tx, mut rx) = mpsc::channel(4);
        subscribers.lock().await.push(tx);

        let client = BioRouterClient::with_routing(
            subscribers,
            Arc::new(Mutex::new(HashMap::new())),
            false,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            empty_provider(),
        );

        client.deliver(None, progress_notif("whatever")).await;

        assert!(
            rx.try_recv().is_ok(),
            "legacy (unpooled) client must still broadcast untokened notifications"
        );
    }

    #[test]
    fn test_progress_token_key_forms() {
        assert_eq!(
            progress_token_key(&ProgressToken(NumberOrString::String("abc".into()))),
            "abc"
        );
        assert_eq!(
            progress_token_key(&ProgressToken(NumberOrString::Number(42))),
            "42"
        );
    }

    /// The progress token attaches to the SAME `_meta` that carries the session
    /// id, so both ride the wire together and neither clobbers the other.
    #[tokio::test]
    async fn test_meta_carries_session_and_progress_token_together() {
        let meta = McpMeta::new(
            "sess-1",
            crate::privacy::CallCapability::for_test_restricted(),
        )
        .with_progress_token("tok-xyz");
        let ext = meta.inject_into_extensions(Default::default());
        let m = ext.get::<Meta>().expect("meta present");
        assert_eq!(
            m.0.get(SESSION_ID_HEADER).and_then(|v| v.as_str()),
            Some("sess-1")
        );
        assert_eq!(
            m.get_progress_token(),
            Some(ProgressToken(NumberOrString::String("tok-xyz".into())))
        );
    }

    /// Issue #56. The capability tier rides the SAME `_meta` object as the
    /// session id. The wire key is spelled literally exactly here — this is the
    /// one place pinning the format is the point; every other reader takes the
    /// const from `biorouter_mcp::knowledge::tier`.
    #[test]
    fn the_capability_tier_rides_the_same_meta_object_as_the_session_id() {
        let meta = McpMeta::new(
            "sess-1",
            crate::privacy::CallCapability::for_test_restricted(),
        )
        .with_capability_private(true);
        let ext = meta.inject_into_extensions(Extensions::default());
        let m = ext.get::<Meta>().unwrap();
        assert_eq!(
            m.0.get("biorouter-session-id").and_then(|v| v.as_str()),
            Some("sess-1")
        );
        assert_eq!(
            m.0.get("biorouter-capability-tier")
                .and_then(|v| v.as_str()),
            Some("private")
        );
    }

    /// Issue #56, the OTHER half: what the daemon writes is what the barrier
    /// reads. The test above pins the wire format against a literal on this
    /// side, and `tier.rs`'s tests compare against consts on that side — so a
    /// drift in the VALUE (the key is already a shared const) would have been
    /// caught by neither. This composes the writer with the real reader and is
    /// the only test that crosses the seam.
    #[test]
    fn the_injected_capability_tier_is_read_back_as_the_same_decision() {
        for private in [true, false] {
            let ext = McpMeta::new(
                "sess-1",
                crate::privacy::CallCapability::for_test_restricted(),
            )
            .with_capability_private(private)
            .inject_into_extensions(Extensions::default());
            let m = ext.get::<Meta>().unwrap();
            assert_eq!(
                biorouter_mcp::knowledge::tier::caller_is_private(m),
                private,
                "the daemon wrote {:?} and the barrier read it back as {}",
                m.0.get(biorouter_mcp::knowledge::tier::CAPABILITY_TIER_META_KEY),
                !private
            );
        }
        // And an extension that is never told — decision (4)'s third parties —
        // reads PUBLIC, which is the safe direction for every gate that
        // consumes it.
        let ext = McpMeta::new(
            "sess-1",
            crate::privacy::CallCapability::for_test_restricted(),
        )
        .inject_into_extensions(Extensions::default());
        assert!(!biorouter_mcp::knowledge::tier::caller_is_private(
            ext.get::<Meta>().unwrap()
        ));
    }

    #[tokio::test]
    async fn test_session_id_in_mcp_meta() {
        use serde_json::json;

        let session_id = "test-session-789";
        crate::session_context::with_session_id(Some(session_id.to_string()), async {
            let extensions = inject_current_session_id_into_extensions(Default::default());
            let meta = extensions.get::<Meta>().unwrap();

            assert_eq!(
                &meta.0,
                json!({
                    SESSION_ID_HEADER: session_id
                })
                .as_object()
                .unwrap()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn test_no_session_id_in_mcp_when_absent() {
        let extensions = inject_current_session_id_into_extensions(Default::default());
        let meta = extensions.get::<Meta>();

        assert!(meta.is_none());
    }

    #[tokio::test]
    async fn test_all_mcp_operations_include_session() {
        use serde_json::json;

        let session_id = "consistent-session-id";
        crate::session_context::with_session_id(Some(session_id.to_string()), async {
            let ext1 = inject_current_session_id_into_extensions(Default::default());
            let ext2 = inject_current_session_id_into_extensions(Default::default());
            let ext3 = inject_current_session_id_into_extensions(Default::default());

            for ext in [&ext1, &ext2, &ext3] {
                assert_eq!(
                    &ext.get::<Meta>().unwrap().0,
                    json!({
                        SESSION_ID_HEADER: session_id
                    })
                    .as_object()
                    .unwrap()
                );
            }
        })
        .await;
    }

    #[tokio::test]
    async fn test_session_id_case_insensitive_replacement() {
        use rmcp::model::{Extensions, Meta};
        use serde_json::{from_value, json};

        let session_id = "new-session-id";
        crate::session_context::with_session_id(Some(session_id.to_string()), async {
            let mut extensions = Extensions::new();
            extensions.insert(
                from_value::<Meta>(json!({
                    "BIOROUTER-SESSION-ID": "old-session-1",
                    "Biorouter-Session-Id": "old-session-2",
                    "other-key": "preserve-me"
                }))
                .unwrap(),
            );

            let extensions = inject_current_session_id_into_extensions(extensions);
            let meta = extensions.get::<Meta>().unwrap();

            assert_eq!(
                &meta.0,
                json!({
                    SESSION_ID_HEADER: session_id,
                    "other-key": "preserve-me"
                })
                .as_object()
                .unwrap()
            );
        })
        .await;
    }

    fn active(sessions: &[&str]) -> ActiveCallSessions {
        Arc::new(std::sync::Mutex::new(
            sessions.iter().map(|s| s.to_string()).collect(),
        ))
    }

    /// #40: how an incoming elicitation is attributed to a session. The
    /// scope decides which agent loop the ActionRequiredManager may deliver
    /// the prompt to, so a wrong `Some` here would leak the prompt into
    /// another session's UI — ambiguity must yield `None` (unscoped), never
    /// a guess.
    #[test]
    fn elicitation_scope_attribution_matrix() {
        let meta = Meta::default();

        // No in-flight call: nothing to attribute.
        assert_eq!(elicitation_session_scope(&meta, &active(&[])), None);
        // Exactly one in-flight call: unambiguous.
        assert_eq!(
            elicitation_session_scope(&meta, &active(&["sess-a"])),
            Some("sess-a".to_string())
        );
        // Several in-flight calls, all one session (parallel tools in one
        // batch): still unambiguous.
        assert_eq!(
            elicitation_session_scope(&meta, &active(&["sess-a", "sess-a"])),
            Some("sess-a".to_string())
        );
        // A shared pooled client running calls for TWO sessions at once:
        // ambiguous, must fall back to unscoped rather than guess.
        assert_eq!(
            elicitation_session_scope(&meta, &active(&["sess-a", "sess-b"])),
            None
        );
    }

    /// A server that echoes the `biorouter-session-id` meta we attach to
    /// every call is believed exactly — even over ambiguous in-flight state.
    #[test]
    fn elicitation_scope_prefers_an_echoed_session_meta() {
        let mut meta_map = rmcp::model::JsonObject::new();
        meta_map.insert(
            SESSION_ID_HEADER.to_string(),
            Value::String("sess-echoed".to_string()),
        );
        let meta = Meta(meta_map);
        assert_eq!(
            elicitation_session_scope(&meta, &active(&["sess-a", "sess-b"])),
            Some("sess-echoed".to_string())
        );
    }

    /// The RAII guard must unregister its session even when the `call_tool`
    /// future is dropped mid-await by a cancellation — a leaked entry would
    /// mis-attribute every later elicitation on this connection.
    #[test]
    fn active_call_guard_unregisters_on_drop() {
        let sessions = active(&[]);
        {
            let _a = ActiveCallGuard::register(&sessions, "sess-a");
            let _b = ActiveCallGuard::register(&sessions, "sess-a");
            assert_eq!(sessions.lock().unwrap().len(), 2);
        }
        assert!(
            sessions.lock().unwrap().is_empty(),
            "dropping the guards must remove exactly their entries"
        );
    }
}
