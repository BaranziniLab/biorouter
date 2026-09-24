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
    mpsc::{self, error::TrySendError, Sender},
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

/// How many notifications one dispatch's channel holds before a new one is
/// dropped (D14).
///
/// Delivery is `try_send`, never a blocking send, and that stays: rmcp hands
/// every notification to a task of its own, so a blocking send would park one
/// task per line behind a consumer that has stopped reading (the tool stream
/// stops polling the moment the call answers) and hold the route map while it
/// waited. The bound is what a burst has to fit in. It was 16, and a shell
/// command printing a screenful at once lost most of it from the live view.
const DISPATCH_CHANNEL_CAPACITY: usize = 256;

/// One dispatch's notification route: the sending half of its channel, and how
/// many notifications it lost to a full channel. The count is reported once,
/// when the route goes away, so a lossy burst is visible in the log without a
/// line per dropped notification.
struct DispatchRoute {
    sender: Sender<ServerNotification>,
    dropped: u64,
}

impl DispatchRoute {
    fn new(sender: Sender<ServerNotification>) -> Self {
        Self { sender, dropped: 0 }
    }

    /// Offer one notification without waiting. A full channel counts a drop;
    /// a closed one (the consumer is gone) is not a loss anybody will miss.
    fn offer(&mut self, notification: ServerNotification) {
        if let Err(TrySendError::Full(_)) = self.sender.try_send(notification) {
            self.dropped += 1;
        }
    }

    fn report_drops(&self, token: &str) {
        if self.dropped > 0 {
            tracing::warn!(
                progress_token = token,
                dropped = self.dropped,
                capacity = DISPATCH_CHANNEL_CAPACITY,
                "a tool call's live notifications overflowed its channel; the dropped ones \
                 are missing from the live view only, never from the tool result"
            );
        }
    }
}

/// Per-dispatch notification routes, keyed by the MCP progress token assigned to
/// a single tool call. Shared between the [`McpClient`] and its [`BioRouterClient`]
/// handler so server notifications can be routed back to exactly the call that
/// asked for them, instead of broadcast to every call on the connection — which
/// on a pooled client means every session sharing the process (the isolation
/// core of the SharedMcpPool), and on an unpooled one every call in a parallel
/// batch (D14).
///
/// A `std::sync::Mutex`: it is never held across an `.await`, and that is what
/// lets [`DispatchRouteGuard`] remove a route in `Drop` when a cancellation
/// drops the `call_tool` future mid-await.
type ProgressRoutes = Arc<std::sync::Mutex<HashMap<String, DispatchRoute>>>;

fn lock_routes(
    routes: &ProgressRoutes,
) -> std::sync::MutexGuard<'_, HashMap<String, DispatchRoute>> {
    routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Install a route under `token` and return the receiver that gets ONLY what
/// is routed to it. Routes whose receiver is gone (a dispatch dropped before
/// its call ran) are pruned on the way, so the map cannot grow without bound.
fn register_route(routes: &ProgressRoutes, token: &str) -> mpsc::Receiver<ServerNotification> {
    let (tx, rx) = mpsc::channel(DISPATCH_CHANNEL_CAPACITY);
    let mut routes = lock_routes(routes);
    routes.retain(|token, route| {
        let open = !route.sender.is_closed();
        if !open {
            route.report_drops(token);
        }
        open
    });
    routes.insert(token.to_string(), DispatchRoute::new(tx));
    rx
}

/// Remove `token`'s route, reporting any drops it counted. Removing the route
/// drops its sender, so the dispatch's receiver ends once drained.
fn deregister_route(routes: &ProgressRoutes, token: &str) {
    if let Some(route) = lock_routes(routes).remove(token) {
        route.report_drops(token);
    }
}

/// RAII deregistration of one call's route: removes it when `call_tool`
/// returns AND when a cancellation drops the `call_tool` future mid-await.
struct DispatchRouteGuard {
    routes: ProgressRoutes,
    token: Option<String>,
}

impl Drop for DispatchRouteGuard {
    fn drop(&mut self) {
        if let Some(token) = &self.token {
            deregister_route(&self.routes, token);
        }
    }
}

/// The wire form of a progress token Biorouter minted or was handed.
///
/// [`McpClient`] mints decimal numbers, so they go out as JSON numbers — the
/// form rmcp's own provider uses and the recorded MCP cassettes hold — and
/// anything else goes out as the string it is. Only a canonical decimal is
/// read as a number, so the route key ([`progress_token_key`] of the echo)
/// always equals the string the route was registered under.
fn wire_progress_token(token: &str) -> ProgressToken {
    match token.parse::<i64>() {
        Ok(number) if number.to_string() == token => ProgressToken(NumberOrString::Number(number)),
        _ => ProgressToken(NumberOrString::String(token.to_string().into())),
    }
}

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
    /// Per-dispatch MCP progress token. When set, it is sent as the call's
    /// `_meta.progressToken` so the server echoes it — on progress
    /// notifications, and in `data.progress_token` on the developer shell's
    /// live lines — letting the client route those notifications to exactly
    /// this dispatch (BR-54, D14). `McpClient::register_dispatch` mints one on
    /// every client, pooled or not; `None` only where a caller built the meta
    /// without a registered route.
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
            //
            // ⚠ This alone does NOT put the token on the wire: rmcp's
            // `send_request_with_option` overwrites `progressToken` in exactly
            // this object with a number of its own. `McpClient::send_request`
            // re-applies the token through `PeerRequestOptions::meta`, which
            // rmcp merges AFTER its own (D14). Kept here so the extensions a
            // caller inspects say the same thing the wire does.
            let mut meta = extensions.get::<Meta>().cloned().unwrap_or_default();
            meta.set_progress_token(wire_progress_token(token));
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
    /// Direct broadcast subscribers ([`McpClientTrait::subscribe`]). Dispatches
    /// no longer subscribe here — every dispatch has a route — so on
    /// `McpClient` this is empty unless something subscribes explicitly.
    notification_handlers: Arc<Mutex<Vec<Sender<ServerNotification>>>>,
    /// Per-dispatch routes keyed by progress token, on every client (D14).
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
            Arc::new(std::sync::Mutex::new(HashMap::new())),
            false,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            provider,
        )
    }

    fn with_routing(
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

    /// Deliver one notification. Factored out so the routing decision is
    /// unit-testable without a live MCP server.
    ///
    /// - A notification that names a progress token belongs to exactly one
    ///   dispatch: it goes to that dispatch's route and nowhere else. A token
    ///   no route holds (a call that already answered, a request that was not
    ///   a dispatch, a token the server made up) is DROPPED, never broadcast —
    ///   broadcasting it is how one call's lines reached another (D14).
    /// - A notification with no token cannot be attributed. A shared
    ///   (`routed_only`) client drops it rather than bleed it across sessions;
    ///   an unpooled one keeps the legacy broadcast to every call in flight,
    ///   which is all a server that echoes nothing can be given.
    async fn deliver(&self, token: Option<&str>, notification: ServerNotification) {
        if let Some(token) = token {
            if let Some(route) = lock_routes(&self.progress_routes).get_mut(token) {
                route.offer(notification);
            }
            return;
        }
        if self.routed_only {
            // Shared client, no owning session -> drop rather than bleed across sessions.
            return;
        }
        {
            let mut routes = lock_routes(&self.progress_routes);
            for route in routes.values_mut() {
                route.offer(notification.clone());
            }
        }
        for handler in self.notification_handlers.lock().await.iter() {
            let _ = handler.try_send(notification.clone());
        }
    }

    /// Route one logging notification by the progress token the server echoed
    /// in its `data` (see [`logging_attribution`]).
    async fn deliver_logging(
        &self,
        params: rmcp::model::LoggingMessageNotificationParam,
        extensions: Extensions,
    ) {
        let token = match logging_attribution(&params.data) {
            LoggingAttribution::Token(token) => Some(token),
            LoggingAttribution::Unattributed => None,
            LoggingAttribution::Malformed => {
                tracing::debug!(
                    "dropping a logging notification whose progress_token is malformed"
                );
                return;
            }
        };
        let notification =
            ServerNotification::LoggingMessageNotification(LoggingMessageNotification {
                params,
                method: LoggingMessageNotificationMethod,
                extensions,
            });
        self.deliver(token.as_deref(), notification).await;
    }
}

/// Who a logging notification belongs to, read from its `data`.
#[derive(Debug, PartialEq, Eq)]
enum LoggingAttribution {
    /// `data.progress_token` names this route key.
    Token(String),
    /// No `progress_token` at all: a server that does not echo one.
    Unattributed,
    /// A `progress_token` that is neither a string nor an integer. It claims
    /// an owner it cannot name, so it is dropped rather than broadcast.
    Malformed,
}

/// MCP logging notifications carry no request linkage, so a server that
/// streams per-call output (the developer shell) echoes the call's progress
/// token in `data.progress_token`. The key is spelled exactly as
/// [`progress_token_key`] spells the token on a progress notification, so one
/// route serves both.
fn logging_attribution(data: &Value) -> LoggingAttribution {
    match data.get("progress_token") {
        None => LoggingAttribution::Unattributed,
        Some(Value::String(token)) => LoggingAttribution::Token(token.clone()),
        Some(Value::Number(number)) => match number.as_i64() {
            Some(n) => LoggingAttribution::Token(n.to_string()),
            None => LoggingAttribution::Malformed,
        },
        Some(_) => LoggingAttribution::Malformed,
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
        // D14: a logging notification is routed by the progress token the
        // server echoed in its `data`. Without one it cannot be attributed:
        // `deliver` drops it on a shared client and broadcasts it on an
        // unpooled one.
        self.deliver_logging(params, context.extensions.clone())
            .await;
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
    /// Monotonic counter feeding every progress token this connection sends —
    /// per-dispatch route keys and every other request's alike, so a token a
    /// route is keyed on can never also name some other request (D14).
    next_token: AtomicU64,
    /// Cleared when the transport is observed closed, so a pooled client can be
    /// evicted and respawned rather than handed to a new session (BR-54 recovery).
    healthy: Arc<AtomicBool>,
    server_info: Option<InitializeResult>,
    timeout: std::time::Duration,
}

impl McpClient {
    /// Connect an unpooled client: a notification that names no progress token
    /// is broadcast to every call in flight (legacy behavior); one that names a
    /// token goes to that call alone. Kept as the entry point for
    /// per-session/per-app clients.
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

    /// Connect a client, choosing what happens to a notification that names no
    /// progress token: dropped (`routed_only = true`, for SharedMcpPool clients
    /// shared across sessions) or broadcast to every call in flight (`false`,
    /// legacy per-session). A tokened notification is routed to its own call
    /// either way.
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
        let progress_routes: ProgressRoutes = Arc::new(std::sync::Mutex::new(HashMap::new()));
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

    /// Send one request carrying `progress_token`, or a freshly minted one.
    ///
    /// ⚠ The token has to go through `PeerRequestOptions::meta`. rmcp's
    /// `send_request_with_option` writes a `progressToken` of its own into the
    /// request's `_meta` and only THEN merges `options.meta`, so a token set on
    /// the request itself never reaches the server. That is how the pooled
    /// path's per-dispatch token was silently replaced by rmcp's counter, and
    /// every notification the server echoed it on was dropped as unknown
    /// (D14). Minting a token for every request, not only dispatches, keeps
    /// all of this connection's tokens from one counter, so a route key can
    /// never collide with a token rmcp chose for a listing.
    async fn send_request(
        &self,
        request: ClientRequest,
        progress_token: Option<&str>,
        cancel_token: CancellationToken,
    ) -> Result<ServerResult, Error> {
        let progress_token = match progress_token {
            Some(token) => wire_progress_token(token),
            None => wire_progress_token(&self.mint_progress_token()),
        };
        let mut meta = Meta::new();
        meta.set_progress_token(progress_token);
        let options = PeerRequestOptions {
            meta: Some(meta),
            ..PeerRequestOptions::no_options()
        };
        let handle = self
            .client
            .lock()
            .await
            .send_cancellable_request(request, options)
            .await
            .inspect_err(|_| self.healthy.store(false, Ordering::Relaxed))?;

        let result = await_response(handle, self.timeout, &cancel_token).await;
        if matches!(result, Err(ServiceError::TransportClosed)) {
            self.healthy.store(false, Ordering::Relaxed);
        }
        result
    }

    /// The next progress token for this connection: a decimal number, unique
    /// per client. Route maps are per client, so it needs no other qualifier.
    fn mint_progress_token(&self) -> String {
        self.next_token.fetch_add(1, Ordering::Relaxed).to_string()
    }

    /// Mint a unique progress token, register a bounded channel under it, and
    /// return the token plus the receiver. The token is attached to the call so
    /// the server echoes it, and the call's [`DispatchRouteGuard`] removes the
    /// route when the call finishes.
    fn register_progress(&self) -> (String, mpsc::Receiver<ServerNotification>) {
        let token = self.mint_progress_token();
        let rx = register_route(&self.progress_routes, &token);
        (token, rx)
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
                None,
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
                None,
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
                None,
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
        // The per-dispatch progress route is removed when the call completes
        // (or errors) — and, RAII, when a cancellation drops this future
        // mid-await — regardless of outcome.
        let _route = DispatchRouteGuard {
            routes: self.progress_routes.clone(),
            token: meta.progress_token.clone(),
        };
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
                meta.progress_token.as_deref(),
                cancel_token,
            )
            .await;

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
                None,
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
                None,
                cancel_token,
            )
            .await?;

        match res {
            ServerResult::GetPromptResult(result) => Ok(result),
            _ => Err(ServiceError::UnexpectedResponse),
        }
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (tx, rx) = mpsc::channel(DISPATCH_CHANNEL_CAPACITY);
        let mut subscribers = self.notification_subscribers.lock().await;
        subscribers.retain(|sender| !sender.is_closed());
        subscribers.push(tx);
        rx
    }

    async fn register_dispatch(&self) -> (Option<String>, mpsc::Receiver<ServerNotification>) {
        // Every client, pooled or not, mints a token and routes ONLY this
        // dispatch's notifications to this receiver (D14). On a shared client
        // that keeps a concurrent call from another session out of it; on an
        // unpooled one it keeps a sibling call in the same batch out — both
        // run on one server process, and a broadcast subscription handed each
        // shell call the other's lines. A notification the server cannot
        // attribute still reaches every in-flight dispatch here, through the
        // routes, on an unpooled client (`BioRouterClient::deliver`).
        let (token, rx) = self.register_progress();
        (Some(token), rx)
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

    fn new_routes() -> ProgressRoutes {
        Arc::new(std::sync::Mutex::new(HashMap::new()))
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
        let routes = new_routes();
        let client = BioRouterClient::with_routing(
            Arc::new(Mutex::new(Vec::new())),
            routes.clone(),
            true,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            empty_provider(),
        );

        let mut rx_a = register_route(&routes, "tok-A");
        let mut rx_b = register_route(&routes, "tok-B");

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
        let routes = new_routes();
        let mut rx_a = register_route(&routes, "tok-A");

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

    fn scoped_registry_fixture(root: &std::path::Path, session: &str) {
        let crew = root.join("config/crew");
        std::fs::create_dir_all(&crew).unwrap();
        std::fs::write(
            crew.join("connections.json"),
            serde_json::json!({
                "connections": [],
                "scopes": {
                    session: {
                        "connection_id": "fixture-connection",
                        "run_id": "fixture-run",
                        "channel_id": "fixture-channel",
                        "source_channels": ["fixture-channel"],
                        "epoch": 0,
                        "provider_binding": "fixture-provider",
                        "public_provider": false,
                        "origin_restricted": false,
                        "expired": false
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn shared_provider_sampling_rejects_idle_crew_scope() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        scoped_registry_fixture(root.path(), "crew-session");
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let provider = empty_provider();
        bind_sampling_session(&provider, "crew-session").unwrap();
        let error = crew_sampling_allowed(&provider)
            .await
            .expect_err("auxiliary sampling must refuse a Crew-scoped provider");
        assert!(error.to_string().contains("Crew-scoped sessions"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn unscoped_sampling_remains_allowed_and_dropped_provider_binding_does_not_transfer() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        scoped_registry_fixture(root.path(), "crew-session");
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let old_provider = empty_provider();
        bind_sampling_session(&old_provider, "crew-session").unwrap();
        drop(old_provider);

        let fresh_provider = empty_provider();
        bind_sampling_session(&fresh_provider, "ordinary-session").unwrap();
        crew_sampling_allowed(&fresh_provider)
            .await
            .expect("an ordinary unscoped session remains allowed");
    }

    /// Session metadata supplied by an MCP server is not an authority for
    /// auxiliary routing. A forged Crew-looking value must not authorize a
    /// delivery to another session.
    #[tokio::test]
    async fn shared_client_does_not_trust_forged_server_session_metadata() {
        let routes = new_routes();
        let mut rx_a = register_route(&routes, "tok-A");
        let client = BioRouterClient::with_routing(
            Arc::new(Mutex::new(Vec::new())),
            routes,
            true,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            empty_provider(),
        );

        let mut notification = progress_notif("forged-server-token");
        if let ServerNotification::ProgressNotification(progress) = &mut notification {
            let mut meta = Meta::new();
            meta.0.insert(
                "biorouter-session-id".into(),
                Value::String("crew-scoped-session".into()),
            );
            progress.extensions.insert(meta);
        }
        client.deliver(None, notification).await;
        assert!(
            rx_a.try_recv().is_err(),
            "server-supplied session metadata must not authorize shared-client delivery"
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
            new_routes(),
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

    // ---- D14: live shell output routing ----------------------------------

    fn log_line(
        output: &str,
        token: Option<Value>,
    ) -> rmcp::model::LoggingMessageNotificationParam {
        let mut data = serde_json::json!({
            "type": "shell_output",
            "stream": "stdout",
            "output": output,
        });
        if let Some(token) = token {
            data["progress_token"] = token;
        }
        rmcp::model::LoggingMessageNotificationParam {
            level: rmcp::model::LoggingLevel::Info,
            logger: Some("shell_tool".to_string()),
            data,
        }
    }

    /// Every `output` waiting in `rx` right now, in arrival order.
    fn drain_outputs(rx: &mut mpsc::Receiver<ServerNotification>) -> Vec<String> {
        let mut outputs = Vec::new();
        while let Ok(notification) = rx.try_recv() {
            if let ServerNotification::LoggingMessageNotification(log) = notification {
                outputs.push(log.params.data["output"].as_str().unwrap_or("").to_string());
            }
        }
        outputs
    }

    fn client_with(routes: &ProgressRoutes, routed_only: bool) -> BioRouterClient {
        BioRouterClient::with_routing(
            Arc::new(Mutex::new(Vec::new())),
            routes.clone(),
            routed_only,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            empty_provider(),
        )
    }

    /// D14 (cross-talk): two calls in flight on ONE unpooled client — a
    /// parallel batch, or two chats on one developer process — each receive
    /// only the lines that echo their own token. The old unpooled path
    /// subscribed every dispatch to a broadcast, so each got both.
    #[tokio::test]
    async fn unpooled_client_delivers_tokened_logging_to_its_own_dispatch_only() {
        let routes = new_routes();
        let client = client_with(&routes, false);
        let mut rx_a = register_route(&routes, "4");
        let mut rx_b = register_route(&routes, "5");

        for (output, token) in [
            ("a0", serde_json::json!(4)),
            ("b0", serde_json::json!(5)),
            ("a1", serde_json::json!("4")),
            ("b1", serde_json::json!(5)),
        ] {
            client
                .deliver_logging(log_line(output, Some(token)), Extensions::default())
                .await;
        }

        assert_eq!(drain_outputs(&mut rx_a), ["a0", "a1"]);
        assert_eq!(drain_outputs(&mut rx_b), ["b0", "b1"]);

        // A server that echoes nothing still reaches every call in flight on
        // an unpooled client: the legacy fallback, and only for that case.
        client
            .deliver_logging(log_line("legacy", None), Extensions::default())
            .await;
        assert_eq!(drain_outputs(&mut rx_a), ["legacy"]);
        assert_eq!(drain_outputs(&mut rx_b), ["legacy"]);
    }

    /// D14 (pooled drop): a shared client used to drop EVERY logging
    /// notification, because it had no token to route by, so a pooled
    /// developer extension showed no live output at all. A tokened line now
    /// reaches its own dispatch; an untokened one is still dropped.
    #[tokio::test]
    async fn routed_only_client_delivers_tokened_logging_to_the_right_route() {
        let routes = new_routes();
        let client = client_with(&routes, true);
        let mut rx_a = register_route(&routes, "10");
        let mut rx_b = register_route(&routes, "11");

        client
            .deliver_logging(
                log_line("mine", Some(serde_json::json!(11))),
                Extensions::default(),
            )
            .await;
        client
            .deliver_logging(log_line("nobody's", None), Extensions::default())
            .await;

        assert_eq!(drain_outputs(&mut rx_b), ["mine"]);
        assert!(
            drain_outputs(&mut rx_a).is_empty(),
            "neither the other session's line nor an unattributable one may land here"
        );
    }

    /// D14: a token no route holds names SOME call — one that already
    /// answered, or one on another connection — so it is dropped, never
    /// broadcast, on either kind of client. A malformed token is dropped too.
    #[tokio::test]
    async fn an_unknown_or_malformed_token_is_dropped_not_broadcast() {
        for routed_only in [false, true] {
            let routes = new_routes();
            let subscribers = Arc::new(Mutex::new(Vec::new()));
            let (tx, mut direct) = mpsc::channel(8);
            subscribers.lock().await.push(tx);
            let client = BioRouterClient::with_routing(
                subscribers,
                routes.clone(),
                routed_only,
                Arc::new(std::sync::Mutex::new(Vec::new())),
                empty_provider(),
            );
            let mut rx = register_route(&routes, "1");

            for token in [
                serde_json::json!(999),
                serde_json::json!("999"),
                serde_json::json!(true),
                serde_json::json!(1.5),
            ] {
                client
                    .deliver_logging(log_line("stray", Some(token)), Extensions::default())
                    .await;
            }
            // The same rule for a progress notification whose token is
            // unknown: on an unpooled client this used to broadcast.
            client.deliver(Some("999"), progress_notif("999")).await;

            assert!(
                drain_outputs(&mut rx).is_empty() && rx.try_recv().is_err(),
                "routed_only={routed_only}: a stray token reached an unrelated dispatch"
            );
            assert!(
                direct.try_recv().is_err(),
                "routed_only={routed_only}: a stray token was broadcast"
            );
        }
    }

    /// D14 (bounded channel): a burst of 200 lines — a screenful printed at
    /// once — reaches its route complete and in order even when nothing reads
    /// until the burst is over. At the old capacity of 16 it lost 184.
    #[tokio::test]
    async fn a_burst_of_200_lines_to_one_route_arrives_complete() {
        let routes = new_routes();
        let client = client_with(&routes, false);
        let mut rx = register_route(&routes, "3");

        let expected: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        for output in &expected {
            client
                .deliver_logging(
                    log_line(output, Some(serde_json::json!(3))),
                    Extensions::default(),
                )
                .await;
        }

        assert_eq!(drain_outputs(&mut rx), expected);
        assert_eq!(lock_routes(&routes)["3"].dropped, 0);
    }

    /// Past the bound, `try_send` still never waits — the MCP reader must not
    /// stall behind a slow consumer — but every loss is counted on its route,
    /// so it can be reported once when the dispatch ends.
    #[tokio::test]
    async fn an_overflowing_route_counts_what_it_dropped() {
        let routes = new_routes();
        let client = client_with(&routes, false);
        let mut rx = register_route(&routes, "8");

        let sent = DISPATCH_CHANNEL_CAPACITY + 10;
        for i in 0..sent {
            client
                .deliver_logging(
                    log_line(&format!("line {i}"), Some(serde_json::json!(8))),
                    Extensions::default(),
                )
                .await;
        }

        assert_eq!(lock_routes(&routes)["8"].dropped, 10);
        assert_eq!(drain_outputs(&mut rx).len(), DISPATCH_CHANNEL_CAPACITY);
        deregister_route(&routes, "8");
        assert!(lock_routes(&routes).is_empty());
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)),
            "deregistering drops the sender, so the dispatch's stream ends"
        );
    }

    #[test]
    fn logging_attribution_reads_the_token_as_progress_token_key_spells_it() {
        assert_eq!(
            logging_attribution(&serde_json::json!({ "progress_token": 12 })),
            LoggingAttribution::Token(progress_token_key(&ProgressToken(NumberOrString::Number(
                12
            ))))
        );
        assert_eq!(
            logging_attribution(&serde_json::json!({ "progress_token": "tok" })),
            LoggingAttribution::Token("tok".to_string())
        );
        assert_eq!(
            logging_attribution(&serde_json::json!({ "type": "shell_output" })),
            LoggingAttribution::Unattributed
        );
        assert_eq!(
            logging_attribution(&serde_json::json!("a plain string log")),
            LoggingAttribution::Unattributed
        );
        for malformed in [
            serde_json::json!({ "progress_token": null }),
            serde_json::json!({ "progress_token": 2.5 }),
            serde_json::json!({ "progress_token": [1] }),
        ] {
            assert_eq!(
                logging_attribution(&malformed),
                LoggingAttribution::Malformed
            );
        }
    }

    /// A minted token goes out as a JSON number — rmcp's own form, and the
    /// form the recorded MCP cassettes hold — and its echo keys back to the
    /// same string. Anything else keeps its string form.
    #[test]
    fn wire_progress_token_round_trips_to_the_route_key() {
        for token in ["0", "41", "-3", "tok-xyz", "007", "1e3"] {
            assert_eq!(progress_token_key(&wire_progress_token(token)), token);
        }
        assert_eq!(
            wire_progress_token("41"),
            ProgressToken(NumberOrString::Number(41))
        );
        assert_eq!(
            wire_progress_token("007"),
            ProgressToken(NumberOrString::String("007".into()))
        );
    }

    /// An MCP server whose one tool streams `count` lines labelled `label` as
    /// logging notifications echoing the call's progress token — the shape the
    /// developer shell emits — then waits for the test to release it.
    #[derive(Clone)]
    struct StreamingServer {
        /// Every call is in flight before any of them streams a line.
        start: Arc<tokio::sync::Barrier>,
        /// Holds the calls open until the test has read what it needs, so
        /// the routes are alive for the whole emission.
        release: Arc<tokio::sync::Semaphore>,
    }

    impl rmcp::ServerHandler for StreamingServer {
        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            context: RequestContext<rmcp::RoleServer>,
        ) -> Result<CallToolResult, ErrorData> {
            let args = request.arguments.unwrap_or_default();
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let count = args.get("count").and_then(Value::as_u64).unwrap_or(0);
            let token = context.meta.get_progress_token();
            self.start.wait().await;
            for seq in 0..count {
                let mut data = serde_json::json!({
                    "type": "shell_output",
                    "stream": "stdout",
                    "output": format!("{label}-{seq}"),
                    "seq": seq,
                });
                if let Some(token) = &token {
                    data["progress_token"] = serde_json::to_value(token).unwrap();
                }
                context
                    .peer
                    .notify_logging_message(rmcp::model::LoggingMessageNotificationParam {
                        level: rmcp::model::LoggingLevel::Info,
                        logger: Some("shell_tool".to_string()),
                        data,
                    })
                    .await
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            }
            self.release
                .acquire()
                .await
                .expect("the release semaphore stays open")
                .forget();
            // Report the token the server actually received.
            Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string(&token).unwrap(),
            )]))
        }
    }

    async fn connect_streaming(server: StreamingServer, routed_only: bool) -> McpClient {
        let (server_read, client_write) = tokio::io::duplex(1 << 16);
        let (client_read, server_write) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            if let Ok(running) = server.serve((server_read, server_write)).await {
                let _ = running.waiting().await;
            }
        });
        McpClient::connect_routed(
            (client_read, client_write),
            Duration::from_secs(30),
            empty_provider(),
            routed_only,
        )
        .await
        .expect("the in-process test server connects")
    }

    /// Wait for `n` logging lines on one dispatch's receiver.
    async fn take_outputs(rx: &mut mpsc::Receiver<ServerNotification>, n: usize) -> Vec<String> {
        let mut outputs = Vec::new();
        while outputs.len() < n {
            let notification = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("the dispatch's lines should arrive")
                .expect("the route closed before its lines arrived");
            if let ServerNotification::LoggingMessageNotification(log) = notification {
                outputs.push(log.params.data["output"].as_str().unwrap_or("").to_string());
            }
        }
        outputs
    }

    /// D14 end to end, over a real MCP connection: two concurrent dispatches
    /// on one client each receive exactly their own lines, the token each
    /// was routed under is the token the server received, and each stream
    /// ends when its call answers. Run for both kinds of client.
    async fn concurrent_dispatches_receive_only_their_own_lines(routed_only: bool) {
        const LINES: usize = 40;
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let client = connect_streaming(
            StreamingServer {
                start: Arc::new(tokio::sync::Barrier::new(2)),
                release: release.clone(),
            },
            routed_only,
        )
        .await;

        // Burn one of OUR tokens, so this client's counter and rmcp's own
        // disagree: if the token did not survive to the wire, the server
        // would report rmcp's number and the assertion below would see it.
        drop(client.register_dispatch().await);
        let (token_a, mut rx_a) = client.register_dispatch().await;
        let (token_b, mut rx_b) = client.register_dispatch().await;
        let token_a = token_a.expect("every dispatch is given a token");
        let token_b = token_b.expect("every dispatch is given a token");

        let call = |label: &'static str, token: String| {
            let arguments = serde_json::json!({ "label": label, "count": LINES });
            client.call_tool(
                "stream",
                arguments.as_object().cloned(),
                McpMeta::new(
                    format!("sess-{label}"),
                    crate::privacy::CallCapability::for_test_restricted(),
                )
                .with_progress_token(token),
                CancellationToken::new(),
            )
        };
        let collect = async {
            let a = take_outputs(&mut rx_a, LINES).await;
            let b = take_outputs(&mut rx_b, LINES).await;
            release.add_permits(2);
            (a, b)
        };
        let (result_a, result_b, (mut lines_a, mut lines_b)) = tokio::join!(
            call("A", token_a.clone()),
            call("B", token_b.clone()),
            collect
        );

        let expected = |label: &str| {
            let mut lines: Vec<String> = (0..LINES).map(|i| format!("{label}-{i}")).collect();
            lines.sort();
            lines
        };
        // rmcp hands each notification to its own task, so arrival order is
        // not the send order; the SET is what isolation is about.
        lines_a.sort();
        lines_b.sort();
        assert_eq!(lines_a, expected("A"), "routed_only={routed_only}");
        assert_eq!(lines_b, expected("B"), "routed_only={routed_only}");

        for (result, token) in [(result_a, &token_a), (result_b, &token_b)] {
            let result = result.expect("the call succeeds");
            let echoed = result.content[0].as_text().expect("text").text.clone();
            assert_eq!(
                echoed,
                serde_json::to_string(&wire_progress_token(token)).unwrap(),
                "routed_only={routed_only}: the server must receive the token the route is keyed on"
            );
        }

        // The call answered, so its route is gone and its stream ends — with
        // nothing further in it, the other call's lines included.
        for rx in [&mut rx_a, &mut rx_b] {
            let end = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            assert!(
                matches!(end, Ok(None)),
                "routed_only={routed_only}: expected the stream to end, got {end:?}"
            );
        }
        assert!(lock_routes(&client.progress_routes).is_empty());
    }

    #[tokio::test]
    async fn concurrent_dispatches_on_one_unpooled_client_each_receive_only_their_own_lines() {
        concurrent_dispatches_receive_only_their_own_lines(false).await;
    }

    #[tokio::test]
    async fn concurrent_dispatches_on_one_pooled_client_each_receive_only_their_own_lines() {
        concurrent_dispatches_receive_only_their_own_lines(true).await;
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
