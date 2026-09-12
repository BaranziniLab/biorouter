//! BR-71 §4.3: each Electron window connects once at startup with a stable
//! window_id. Outbound: workspace command frames. Inbound: workspace_echo
//! (debounced layout report) and workspace_result (resolves parked round
//! trips). Auth: the server secret as a query token (the browser WebSocket API
//! cannot set headers) + the origin gate — same two-gate shape as the app
//! agent socket (apps.rs:538-556), with the Electron file origin allowed.

use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::Value;

use crate::state::AppState;
use crate::workspace::bridge;

/// State for this module's routes: the app state PLUS the server secret.
///
/// The secret is not global — the daemon threads it into
/// `routes::configure(state, secret_key)`, which hands it by value to this
/// route, the only one that needs it.
#[derive(Clone)]
struct WorkspaceRouteState {
    /// Held for the socket handler's future use (session lookups when the
    /// renderer starts echoing per-tab state); the auth gate needs only
    /// `secret`.
    state: Arc<AppState>,
    secret: String,
}

fn check_workspace_ws_auth(
    upgrade: &super::UpgradeOrigin<'_>,
    token: Option<&str>,
    expected: &str,
) -> Result<(), &'static str> {
    if upgrade.origin.is_some() {
        // The packaged renderer is loaded from a `file:` URL
        // (`ui/desktop/src/main.ts`, `pathToFileURL`), so it presents
        // `file://`. The dev renderer presents vite's `http://localhost:517x`.
        // **Both are admitted only as the renderer this daemon's launcher
        // declared** (`routes::RENDERER_ORIGIN_ENV`), and only as that; before
        // QA-D F7 the second was admitted as "any loopback port", which admitted
        // every other local page's socket too.
        //
        // ⚠ **`file://` used to be admitted by NAME, on every daemon**, and the
        // security review of QA-D F7 found what that meant. Nothing here asks
        // whether Electron launched this daemon, `routes::configure` mounts these
        // routes on all of them, and `/ui/workspace` is one of only two paths
        // exempt from `check_token` — so the origin test plus the `?secret=`
        // query token are the whole authority on it. A local `.html` opened in
        // Chromium serializes its origin as exactly `file://` and sends it on a
        // WebSocket handshake, so such a page cleared this gate on a `biorouter
        // serve` host, where the secret it is then left alone with is the literal
        // `test` under `just debug-server`. It is DECLARED now, so a `serve`
        // daemon never carries the allowance: `commands/serve.rs` strips the
        // variable from the daemon it spawns, and `ui/desktop/src/main.ts` sets
        // it to `file://` exactly when the renderer it loaded is a `file:` page.
        //
        // It was NOT removed outright — the packaged app's workspace channel *is*
        // that `file:` page's socket. `apps::check_ws_auth` living without such
        // an allowance is not evidence that this one can: an app's page is
        // **served by this daemon over http**, so it is same-origin with its own
        // socket and never needed one.
        //
        // **"null" is NOT admitted.** It is the opaque origin of any sandboxed
        // frame — including the agent-authored figures this very app renders in
        // its artifact side panel, which are put into a `srcDoc` iframe with
        // `sandbox="allow-scripts allow-downloads"` and no `allow-same-origin`
        // (`ui/desktop/src/components/artifacts/ArtifactViewer.tsx`, and
        // `wrapArtifactForBrowser` in `ui/desktop/src/utils/artifactSecurity.ts`
        // for the opened/expanded view). `routes/mod.rs`'s own `origin_tests`
        // rejects it by name (`assert!(!is_local_origin("null"))`).
        // This gate must stay at least as strict as `apps::check_ws_auth`,
        // which is the route the design claims parity with; `file://` is the
        // only thing it admits that that one does not.
        //
        // `is_this_daemons` is a same-origin test against the request's own
        // `Host` — scheme, host and port — which is what admits a browser that
        // reached this daemon at a LAN address or a hostname, as it may now
        // that the daemon serves its own interface (`routes::web_ui`). A page
        // on any other origin cannot match, a page on another loopback port
        // included, and `null` is refused because it is not an origin at all.
        if !upgrade.is_declared_electron_renderer() && !upgrade.is_this_daemons() {
            return Err("cross-origin connect rejected");
        }
    }
    // Constant time, not `!=`. This is the SAME server secret `check_token`
    // guards, and `/ui/workspace` is the one path exempt from `check_token`
    // (`auth::is_unauthenticated_path`) — which means it is also exempt from
    // that middleware's rate limiter, so an attacker here gets unlimited,
    // unthrottled timing samples against the daemon's master key. `str` equality
    // is a length check plus an early-returning memcmp. `secret_matches` is
    // `check_token`'s own comparator, shared rather than re-implemented so the
    // two can never drift; its doc comment carries the invariant.
    if !token.is_some_and(|token| super::secret_matches(token, expected)) {
        return Err("missing or invalid workspace socket secret");
    }
    Ok(())
}

/// Cap on one renderer→daemon frame.
///
/// A `workspace_echo` is a single window's tab/pane layout; 128 KiB is orders of
/// magnitude more than that and still bounded. It matters because `store_echo`'s
/// value is handed to the model verbatim as `workspace_list`'s `gui` — so an
/// uncapped frame is unbounded daemon memory *and* an unbounded injection into
/// the agent's context.
///
/// The oversized frame is dropped, not the connection: the echo is a periodic
/// report, so a renderer bug that emits one bad frame must not cost the user
/// their window channel, and the bridge keeps the last good echo.
const MAX_INBOUND_FRAME_BYTES: usize = 128 * 1024;

/// The registry key a connection claims, validated.
///
/// `bridge::bridge_for` inserts on first sight into `BRIDGES`, a process-lifetime
/// map that never evicts, and retains the key together with that window's last
/// echo. Unbounded, it is a memory sink one query parameter wide. The
/// charset+length rule is `auth::is_public_app_get`'s, which bounds the other
/// client-supplied identifier this daemon keys retained state on.
///
/// **Absent is not invalid.** A single-window client may omit it and share the
/// `"default"` window; only a *present but malformed* id is refused, so this
/// cannot turn into a handshake failure for a client that never sends one.
fn window_id_from(raw: Option<&str>) -> Result<String, &'static str> {
    let Some(raw) = raw.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok("default".to_string());
    };
    if raw.len() > 128
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("invalid window_id");
    }
    Ok(raw.to_string())
}

async fn workspace_ws(
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(rs): State<WorkspaceRouteState>,
) -> Response {
    let upgrade = super::UpgradeOrigin::from_headers(&headers);
    let origin = upgrade.origin;
    let state = rs.state.clone();
    if let Err(reason) = check_workspace_ws_auth(
        &upgrade,
        params.get("secret").map(String::as_str),
        &rs.secret,
    ) {
        tracing::warn!(
            origin = origin.unwrap_or("<none>"),
            "rejected workspace WS: {reason}"
        );
        return (axum::http::StatusCode::FORBIDDEN, reason).into_response();
    }
    // After the auth gate, deliberately: an unauthenticated caller learns
    // nothing about this route's input rules.
    let window_id = match window_id_from(params.get("window_id").map(String::as_str)) {
        Ok(window_id) => window_id,
        Err(reason) => {
            tracing::warn!(
                origin = origin.unwrap_or("<none>"),
                "rejected workspace WS: {reason}"
            );
            return (axum::http::StatusCode::BAD_REQUEST, reason).into_response();
        }
    };
    // Task 31's live gate has to record what the packaged renderer actually
    // sends here: whether Chromium presents `file://` or the opaque `null` on a
    // handshake from a `file:` page is version-dependent. If it is `null`, the
    // fix is on the client (connect through the loopback dev-server origin, or
    // open the socket from the main process) — NOT to widen the gate above,
    // which would admit every sandboxed agent-authored frame in the app.
    tracing::info!(
        origin = origin.unwrap_or("<none>"),
        window_id = %window_id,
        "workspace WS handshake accepted"
    );
    ws.on_upgrade(move |socket| handle_workspace_socket(socket, state, window_id))
}

/// ⚠ Two known gaps, recorded against Task 31's live pass in
/// `docs/agent-loop/designs/br71-execution-plan.md` (which carries the reasoning
/// and the live checks) rather than fixed here:
///
/// - **No keepalive.** This loop exits only on a close frame, a read error or a
///   failed write, so a half-open connection (sleeping laptop, dropped Wi-Fi)
///   leaves `is_attached()` — and therefore `gui_attached()` — true for a window
///   nobody can see, which defeats `workspace_send_prompt`'s Decision 4 refusal
///   and keeps `focused_or_recent()` routing into a dead socket.
/// - **The writer blocks the reader.** `socket_tx.send(...).await` runs inside a
///   `select!` branch, so a backpressured sink stops inbound frames — including
///   the `workspace_result` that would unpark a round trip. Bounded (every
///   `emit_and_wait` has a timeout), never deadlocking, but it turns one slow
///   writer into a stalled turn.
async fn handle_workspace_socket(socket: WebSocket, _state: Arc<AppState>, window_id: String) {
    use futures::{SinkExt, StreamExt};
    let bridge = bridge::bridge_for(&window_id);
    let (mut outbound_rx, token) = bridge.attach();
    let (mut socket_tx, mut socket_rx) = socket.split();

    loop {
        tokio::select! {
            frame = outbound_rx.recv() => match frame {
                Some(frame) => {
                    let text = frame.to_string();
                    if socket_tx.send(WsMessage::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                None => break, // a newer connection replaced us
            },
            inbound = socket_rx.next() => match inbound {
                Some(Ok(WsMessage::Text(text))) => {
                    if text.len() > MAX_INBOUND_FRAME_BYTES {
                        tracing::warn!(
                            window_id = %window_id,
                            bytes = text.len(),
                            "dropping an oversized workspace frame"
                        );
                        continue;
                    }
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                    apply_inbound_frame(&bridge, &window_id, value);
                }
                Some(Ok(WsMessage::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
    bridge.detach(token);
}

/// The renderer→daemon frame vocabulary, lifted out of the socket loop so it has
/// a test (`inbound_frames_reach_the_bridge_by_type`). Everything above it is
/// transport; this is the behaviour.
///
/// `window_id` is the **connection's**, taken from the handshake query — never
/// the payload's. See the echo arm.
fn apply_inbound_frame(bridge: &bridge::WorkspaceBridge, window_id: &str, mut value: Value) {
    match value.get("type").and_then(Value::as_str) {
        Some("workspace_echo") => {
            // Stamp the identity from the connection, overwriting whatever the
            // client claimed. `merged_layout()` hands these echoes to the model
            // as `workspace_list`'s `gui`, and the model then targets commands
            // by that id — so a client-asserted `window_id` lets one
            // authenticated window impersonate another in the agent's view of
            // the workspace. It would also disagree with the `BRIDGES` key the
            // echo is stored under (`bridge_for(&window_id)`), which is the
            // connection's id and nothing else.
            //
            // `get` on a non-object `Value` returns `None`, so reaching this arm
            // already proves `value` is an object; `as_object_mut` keeps that a
            // fact rather than an assumption `IndexMut` would panic on.
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "window_id".to_string(),
                    Value::String(window_id.to_string()),
                );
            }
            bridge.store_echo(value);
        }
        Some("workspace_result") => {
            if let Some(id) = value.get("request_id").and_then(Value::as_str) {
                bridge.resolve(id, value.clone());
            }
        }
        _ => {}
    }
}

pub fn routes(state: Arc<AppState>, secret_key: String) -> Router {
    Router::new()
        .route("/ui/workspace", get(workspace_ws))
        .with_state(WorkspaceRouteState {
            state,
            secret: secret_key,
        })
}

/// ⚠ **These are the two pure halves only.** Nothing here reaches
/// `workspace_ws` or `handle_workspace_socket`, so a build that calls NEITHER of
/// them — an unauthenticated socket, or one that drops every renderer frame —
/// keeps this module green, and so does one where `routes::configure` never
/// merges this router or `auth::is_unauthenticated_path` drops
/// `"/ui/workspace"`. The mounted socket is tested end to end over a real
/// loopback connection in **`tests/workspace_socket.rs`**, which fails on all
/// four; it cannot live here because it must name `auth::check_token`, and
/// `src/routes/` is compiled into the `biorouterd` binary as well as the lib.
/// **Run it too:** `cargo test -p biorouter-server --test workspace_socket`.
#[cfg(test)]
mod tests {
    use super::super::{DeclaredRenderer, UpgradeOrigin, WebOrigin};
    use super::*;

    /// An upgrade as the gate sees it: plain HTTP, no declared renderer.
    fn upgrade<'a>(origin: Option<&'a str>, host: Option<&'a str>) -> UpgradeOrigin<'a> {
        UpgradeOrigin {
            origin,
            host,
            scheme: "http",
            renderer: None,
        }
    }

    #[test]
    fn ws_auth_requires_secret_and_local_or_app_origin() {
        let secret = "test-secret";
        let daemon = Some("127.0.0.1:9380");
        // A browser-set web origin must be this daemon's own (CSWSH).
        assert!(check_workspace_ws_auth(
            &upgrade(Some("https://evil.com"), daemon),
            Some(secret),
            secret
        )
        .is_err());
        assert!(check_workspace_ws_auth(
            &upgrade(Some("http://127.0.0.1:9380"), daemon),
            Some(secret),
            secret
        )
        .is_ok());
        // The dev renderer is vite's page on another loopback port. It is
        // admitted as the renderer its launcher declared, and only as that.
        let dev =
            DeclaredRenderer::LoopbackHttp(WebOrigin::parse("http://localhost:5173").unwrap());
        let declared = UpgradeOrigin {
            renderer: Some(&dev),
            ..upgrade(Some("http://localhost:5173"), daemon)
        };
        assert!(check_workspace_ws_auth(&declared, Some(secret), secret).is_ok());
        // Undeclared, it is just another loopback port (QA-D F7), which this
        // gate used to admit as though it were the daemon's own.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("http://localhost:5173"), daemon),
            Some(secret),
            secret
        )
        .is_err());
        // Decision 3's Electron allowance, kept to ONE measured literal — the
        // packaged renderer loads from a file: URL (main.ts `pathToFileURL`) —
        // and, since the security review of QA-D F7, admitted only on a daemon
        // whose launcher DECLARED such a renderer.
        let electron = DeclaredRenderer::ElectronFile;
        let packaged = UpgradeOrigin {
            renderer: Some(&electron),
            ..upgrade(Some("file://"), daemon)
        };
        assert!(check_workspace_ws_auth(&packaged, Some(secret), secret).is_ok());
        // ⚠ And refused where nothing declared one. This is the finding: these
        // routes are mounted on EVERY daemon, `/ui/workspace` is one of only two
        // paths exempt from `check_token`, and a local `.html` opened in Chromium
        // presents exactly this origin — so on a `biorouter serve` host such a
        // page used to clear this gate and be left alone with the secret.
        assert!(
            check_workspace_ws_auth(&upgrade(Some("file://"), daemon), Some(secret), secret)
                .is_err(),
            "`file://` must be admitted only where a launcher declared an Electron renderer"
        );
        // A daemon that declared the DEV renderer has not declared a file page.
        let dev_only = UpgradeOrigin {
            renderer: Some(&dev),
            ..upgrade(Some("file://"), daemon)
        };
        assert!(check_workspace_ws_auth(&dev_only, Some(secret), secret).is_err());
        // "null" is REFUSED. It is the opaque origin of every sandboxed frame,
        // including the agent-authored figures this app renders in its artifact
        // side panel (a srcDoc iframe carrying `sandbox="allow-scripts
        // allow-downloads"`, no allow-same-origin — set in
        // ui/desktop/src/components/artifacts/ArtifactViewer.tsx and in
        // `wrapArtifactForBrowser`, ui/desktop/src/utils/artifactSecurity.ts) —
        // and routes/mod.rs's own `origin_tests` rejects it by name. Admitting
        // it would make this gate strictly weaker than `apps::check_ws_auth`
        // (`apps.rs`), the route the design claims parity with, leaving the
        // socket secret-only.
        assert!(
            check_workspace_ws_auth(&upgrade(Some("null"), daemon), Some(secret), secret).is_err()
        );
        assert!(check_workspace_ws_auth(&upgrade(None, daemon), Some(secret), secret).is_ok());
        // Wrong/missing secret always refuses.
        assert!(check_workspace_ws_auth(&upgrade(None, None), Some("wrong"), secret).is_err());
        assert!(check_workspace_ws_auth(&upgrade(None, None), None, secret).is_err());
        // Same length, differing in one byte, and a prefix: the comparison is
        // `secret_matches`, which returns early on LENGTH only. A call that got
        // its arguments confused, or compared lengths alone, passes the two
        // cases above (`"wrong"` is 5 bytes against 11) and fails these.
        assert!(
            check_workspace_ws_auth(&upgrade(None, None), Some("test-secreT"), secret).is_err()
        );
        assert!(check_workspace_ws_auth(&upgrade(None, None), Some("test-secre"), secret).is_err());
    }

    /// An upgrade as the real handler sees one: parsed off headers, so the
    /// reading of a malformed `Origin` is the reading under test rather than one
    /// a test helper chose.
    fn upgrade_from(origin: Option<&[u8]>, host: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        if let Some(origin) = origin {
            headers.insert(
                axum::http::header::ORIGIN,
                axum::http::HeaderValue::from_bytes(origin).unwrap(),
            );
        }
        headers.insert(axum::http::header::HOST, host.parse().unwrap());
        headers
    }

    /// **Finding 1.** `file://` is matched by NAME, so it must be admitted only
    /// where a launcher declared a renderer that presents it.
    ///
    /// Nothing here asks whether Electron launched this daemon, `routes::configure`
    /// mounts these routes on all of them, and `/ui/workspace` is one of only two
    /// paths exempt from `check_token` — so on a `biorouter serve` host the origin
    /// test plus the `?secret=` query token were the whole authority, and a local
    /// `.html` opened in Chromium presents exactly this origin.
    #[test]
    fn the_electron_file_origin_is_refused_where_no_renderer_was_declared() {
        let secret = "test-secret";
        let daemon = Some("127.0.0.1:9380");
        assert!(
            check_workspace_ws_auth(&upgrade(Some("file://"), daemon), Some(secret), secret)
                .is_err(),
            "`file://` must be admitted only where a launcher declared an Electron renderer"
        );
    }

    /// **Finding 3.** An `Origin` that was SENT and cannot be read must refuse,
    /// not degrade into the no-`Origin` case this gate deliberately admits.
    ///
    /// Driven through `UpgradeOrigin::from_headers`, because the degradation is in
    /// that reading: `HeaderValue::to_str` fails and the `Option` it produces is
    /// indistinguishable from "no browser sent one". `Host` fails closed in the
    /// same situation, and the two must not disagree. Unreachable from a browser,
    /// which punycodes hosts — which is the argument for refusing it.
    #[test]
    fn an_unreadable_origin_refuses_where_an_absent_one_is_admitted() {
        let secret = "test-secret";
        // Valid as a header value (obs-text permits 0x80..=0xFF) and not UTF-8.
        let unreadable = upgrade_from(Some(b"http://\xff.example"), "127.0.0.1:9380");
        let unreadable = super::super::UpgradeOrigin::from_headers(&unreadable);
        assert!(
            check_workspace_ws_auth(&unreadable, Some(secret), secret).is_err(),
            "a present-but-unreadable Origin must refuse rather than skip the gate"
        );
        // …while nothing sent one stays admitted: that is a client which is not a
        // browser, and its token is the authority.
        let absent = upgrade_from(None, "127.0.0.1:9380");
        let absent = super::super::UpgradeOrigin::from_headers(&absent);
        assert!(check_workspace_ws_auth(&absent, Some(secret), secret).is_ok());
    }

    /// The daemon serves its own interface now (`routes::web_ui`), so a browser
    /// can legitimately reach it at a LAN address or a hostname the daemon
    /// never enumerated. The same-origin rule is what admits those, and it must
    /// admit ONLY those.
    #[test]
    fn a_browser_that_reached_this_daemon_at_a_lan_address_is_same_origin() {
        let secret = "test-secret";
        // Served at a LAN address: Origin and Host agree, so it is the very
        // page this daemon handed out.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("http://192.168.1.42:8765"), Some("192.168.1.42:8765")),
            Some(secret),
            secret,
        )
        .is_ok());
        // A hostname works identically -- nothing is enumerated.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("http://lab-server:8765"), Some("lab-server:8765")),
            Some(secret),
            secret,
        )
        .is_ok());
        // Behind the TLS proxy the deployment guide recommends, the page is
        // `https` and the proxy says so. The same page reached over plain
        // `http` at the same host is a different origin, and is refused.
        let behind_tls = |origin| UpgradeOrigin {
            scheme: "https",
            ..upgrade(Some(origin), Some("lab.example.org"))
        };
        assert!(check_workspace_ws_auth(
            &behind_tls("https://lab.example.org"),
            Some(secret),
            secret
        )
        .is_ok());
        assert!(check_workspace_ws_auth(
            &behind_tls("http://lab.example.org"),
            Some(secret),
            secret
        )
        .is_err());
    }

    /// The half that makes the rule a gate rather than a hole. Each of these
    /// passes an implementation that merely checks "a Host header is present",
    /// or that prefix-matches instead of comparing whole, or that compares the
    /// authority and not the scheme.
    #[test]
    fn a_cross_origin_page_still_cannot_reach_the_socket_however_it_was_addressed() {
        let secret = "test-secret";
        // The attack the gate exists for: a page on another origin, connecting
        // to the daemon. The browser sets Origin to the page, Host to the
        // target -- they differ, so it is refused.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("https://evil.com"), Some("192.168.1.42:8765")),
            Some(secret),
            secret,
        )
        .is_err());
        // Prefix confusion in both directions.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("http://evil.com"), Some("evil.com.attacker.net")),
            Some(secret),
            secret,
        )
        .is_err());
        assert!(check_workspace_ws_auth(
            &upgrade(
                Some("http://192.168.1.42:8765.evil.com"),
                Some("192.168.1.42:8765")
            ),
            Some(secret),
            secret,
        )
        .is_err());
        // QA-D F7's three shapes, each against the daemon's own Host: another
        // loopback port, another scheme, and another loopback host, spelled
        // in upper case.
        for origin in [
            "http://127.0.0.1:1",
            "http://localhost:3000",
            "https://127.0.0.1:9380",
            "http://LOCALHOST:9380",
        ] {
            assert!(
                check_workspace_ws_auth(
                    &upgrade(Some(origin), Some("127.0.0.1:9380")),
                    Some(secret),
                    secret
                )
                .is_err(),
                "{origin} is not this daemon's origin"
            );
        }
        // A matching Host does not rescue an opaque origin.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("null"), Some("null")),
            Some(secret),
            secret
        )
        .is_err());
        // And the secret is still required on the same-origin path, so the
        // widening cannot be mistaken for an exemption.
        assert!(check_workspace_ws_auth(
            &upgrade(Some("http://192.168.1.42:8765"), Some("192.168.1.42:8765")),
            Some("wrong"),
            secret,
        )
        .is_err());
    }

    #[test]
    fn the_window_id_is_bounded_and_absence_is_not_an_error() {
        // A single-window client may simply omit it.
        assert_eq!(window_id_from(None).unwrap(), "default");
        assert_eq!(window_id_from(Some("  ")).unwrap(), "default");
        assert_eq!(window_id_from(Some(" win-2 ")).unwrap(), "win-2");
        assert_eq!(window_id_from(Some("W_9")).unwrap(), "W_9");
        // `bridge_for` inserts on first sight into a process-lifetime map that
        // never evicts, and the key is retained with the window's last echo. An
        // unbounded id is therefore a memory sink one query parameter wide.
        assert!(window_id_from(Some(&"a".repeat(128))).is_ok());
        assert!(window_id_from(Some(&"a".repeat(129))).is_err());
        // Same charset as `auth::is_public_app_get`'s app id — the other
        // client-supplied identifier this daemon keys retained state on. No
        // separators, so a window id can never be read as a path or carry
        // structure into a log line.
        assert!(window_id_from(Some("win/../other")).is_err());
        assert!(window_id_from(Some("win 2")).is_err());
        assert!(window_id_from(Some("win\n2")).is_err());
    }

    /// The socket loop's INBOUND vocabulary. Without this, `handle_workspace_socket`
    /// — the half that turns renderer frames into bridge state — has no coverage at
    /// all: a loop that parsed `workspace_echo` into nothing would leave
    /// `workspace_list`'s `gui` block permanently empty, and a loop that never
    /// called `resolve` would leave every `emit_and_wait` to time out after 10 s,
    /// with `check_workspace_ws_auth`'s tests still green.
    ///
    /// The dispatch is extracted (`apply_inbound_frame`) so it is testable without
    /// a live WebSocket; the loop below is a two-line `match` over it.
    #[test]
    fn inbound_frames_reach_the_bridge_by_type() {
        use crate::workspace::bridge::WorkspaceBridge;
        let bridge = WorkspaceBridge::new();
        let (_rx, _token) = bridge.attach();

        apply_inbound_frame(
            &bridge,
            "w1",
            serde_json::json!({
                "type": "workspace_echo", "window_id": "w-impersonated",
                "focused_session": "s1", "layout": []
            }),
        );
        assert_eq!(
            bridge.last_echo().unwrap()["focused_session"],
            "s1",
            "workspace_echo must land in the bridge's last_echo: it IS workspace_list's `gui`"
        );
        assert_eq!(
            bridge.last_echo().unwrap()["window_id"],
            "w1",
            "the window's identity is the CONNECTION's, not the payload's claim: the model \
             targets commands by this id, and the echo is stored under the connection's key"
        );

        // A result frame resolves the parked request it names, and only that one.
        let (tx, mut rx_result) = tokio::sync::oneshot::channel::<serde_json::Value>();
        bridge.insert_pending_for_test("wsreq-1", tx);
        apply_inbound_frame(
            &bridge,
            "w1",
            serde_json::json!({"type": "workspace_result", "request_id": "wsreq-9", "ok": false}),
        );
        assert!(
            rx_result.try_recv().is_err(),
            "a mismatched request_id resolves nothing"
        );
        apply_inbound_frame(
            &bridge,
            "w1",
            serde_json::json!({"type": "workspace_result", "request_id": "wsreq-1", "ok": true}),
        );
        assert_eq!(rx_result.try_recv().unwrap()["ok"], true);

        // Anything else is ignored, not treated as an echo.
        apply_inbound_frame(
            &bridge,
            "w1",
            serde_json::json!({"type": "hello", "focused_session": "s9"}),
        );
        assert_eq!(bridge.last_echo().unwrap()["focused_session"], "s1");
    }
}
