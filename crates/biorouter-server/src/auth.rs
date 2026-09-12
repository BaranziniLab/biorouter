use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The constant-time secret comparison this middleware gates on.
///
/// The implementation moved to `routes` (with its rationale) so that
/// `routes::workspace`'s WebSocket gate — which checks the same server secret,
/// on a path this module exempts from the header check *and therefore from the
/// rate limiter* — can share it instead of re-implementing it. `src/routes/` is
/// compiled into the `biorouterd` binary as well as the lib and cannot name
/// `crate::auth`, so the shared direction is this one. Re-exported here so
/// `check_token` and `mod tests` are unaffected.
use crate::routes::secret_matches;

static FAILED_ATTEMPTS: OnceLock<Mutex<HashMap<String, Vec<Instant>>>> = OnceLock::new();

/// The SHA-256 digest of the user-action key, handed to this process on **stdin**
/// by whoever launched it (issue #56, DR-16).
///
/// `Option` inside the `OnceLock` so "installed, and there is no key" is
/// representable and distinct from "never installed". Both fail closed; keeping
/// them apart is what lets `commands::agent` log the warning exactly once.
///
/// ⚠ The **digest**, never the key. AR-11 measured this daemon's own API secret
/// to be recoverable from inside the daemon, so a credential the daemon holds in
/// full is a credential the model can present. A tool that reads this heap
/// recovers a value it cannot use: the guard hashes what the caller presented
/// and compares, so the stored bytes authenticate nothing. That asymmetry is the
/// only part of AR-11's residual this closes — the raw key lives in the Electron
/// main process, and a caller who can read *that* is unaffected (Open
/// question 20).
static USER_ACTION_DIGEST: OnceLock<Option<[u8; 32]>> = OnceLock::new();

/// Publish the digest read off stdin at startup. Called once, before the router
/// is built, from `commands::agent::run`.
pub fn install_user_action_digest(digest: Option<[u8; 32]>) {
    let _ = USER_ACTION_DIGEST.set(digest);
}

/// Does `presented` hash to `expected`?
///
/// Pure, so the whole rule is testable without a process global or a server —
/// which matters here because the alternative home for these assertions is an
/// `AppState`-backed HTTP test, and `AppState::new()` opens the ONE session DB this
/// binary shares (see `routes::agent::working_dir_lock_tests`).
///
/// `expected: None` is "this daemon was handed no key" and fails closed: `just
/// run-server`, a hand-run `biorouterd agent` and every headless deployment land
/// there, and they refuse every raise — including one made by the person at the
/// keyboard (open question 23).
///
/// Compared without an early return, the same way [`secret_matches`] is, so a
/// caller cannot recover the digest one byte at a time by timing the response.
pub fn user_action_matches(presented: Option<&str>, expected: Option<&[u8; 32]>) -> bool {
    let (Some(presented), Some(expected)) = (presented, expected) else {
        return false;
    };
    let got = <sha2::Sha256 as sha2::Digest>::digest(presented.as_bytes());
    let mut diff = 0u8;
    for (x, y) in got.iter().zip(expected.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// What [`user_action_proof`] found. Three answers, not two, because two of them
/// need to be *said* differently: a caller who presented no proof is being told
/// "the user decides this", while a caller on a daemon that holds no key is
/// being told "this control is unavailable on this daemon" — and reporting the
/// second as the first sends the person at the keyboard hunting for a permission
/// they can never obtain (open question 23).
///
/// Both non-`Proven` answers refuse. The distinction is in the message only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserActionProof {
    /// The header carried a key that hashes to the installed digest.
    Proven,
    /// A digest is installed and the request did not match it.
    Unproven,
    /// This daemon was handed no user-action key on stdin, so there is nothing
    /// to verify against and every caller is refused — including the human.
    /// `just run-server`, a hand-run `biorouterd agent` and any headless
    /// deployment land here. (`just debug-server` does not: it pipes the
    /// published dev key.)
    NoKeyInstalled,
}

/// Did this request come from the user rather than from the model?
///
/// A **per-route** requirement, not a second gate on every request:
/// [`check_token`] is untouched, because refusing every identity-free request
/// would take the user's own model picker away along with the model's — the
/// posture DR-16 rejected. CORS already passes the header through; the daemon's
/// layer is `.allow_headers(Any)`.
///
/// ONE header, ONE key, ONE comparison. Every route that needs a proof of user
/// reads this; a second header name anywhere is the defect the tier route's own
/// gate looks for.
pub fn user_action_proof(headers: &axum::http::HeaderMap) -> UserActionProof {
    let Some(expected) = USER_ACTION_DIGEST.get().and_then(|d| d.as_ref()) else {
        return UserActionProof::NoKeyInstalled;
    };
    if user_action_matches(
        headers.get("X-User-Action").and_then(|v| v.to_str().ok()),
        Some(expected),
    ) {
        UserActionProof::Proven
    } else {
        UserActionProof::Unproven
    }
}

/// The boolean form, for the five raise channels that have one refusal to give.
///
/// Defined in terms of [`user_action_proof`] so there is exactly one place where
/// "does this request carry the proof" is decided — a second implementation that
/// happened to read the same header would still be a second answer.
pub fn is_user_action(headers: &axum::http::HeaderMap) -> bool {
    matches!(user_action_proof(headers), UserActionProof::Proven)
}

/// The standing a `biorouter serve` daemon gives its OWN web interface (issue
/// #56, the QA follow-up of 2026-09-10 that closed H2 and M1).
///
/// A serve daemon holds no user-action digest (SD-7) and pins the provider for
/// every session it runs (SD-1), so the tier the operator's configured provider
/// implies is the only capability its interface can be said to have. This keeps
/// that tier beside the browser token whose cookie marks a request as coming
/// from the document this daemon served — which is how a request from the
/// operator's browser is told from one that merely holds the secret.
///
/// ⚠ **It widens nothing that was refused.** It is read only by the listing and
/// knowledge-base gates in `routes::session_reach`, which were open to this
/// interface before they existed; the transcript gate, `session_reach` itself,
/// never reads it. A serve daemon's browser therefore keeps exactly the reach it
/// had, and a caller holding only the secret loses it.
///
/// ⚠ **Not authentication, and not a proof of a person.** `biorouter serve`
/// hands this daemon the token in its environment, beside the secret, so a
/// caller that can read one can read the other — the residual `X-Caller-Provider`
/// already carries (#47). It never satisfies a proof-of-user check: SD-1 and
/// SD-8 stand exactly as they were.
struct ServedOperator {
    browser_token: String,
    capability: biorouter::privacy::ProviderTier,
}

static SERVED_OPERATOR: OnceLock<ServedOperator> = OnceLock::new();

/// Record a serve daemon's operator standing. Called once, from
/// `commands::agent::run`, and only when the web interface is served behind a
/// browser token: a `--no-token` daemon cannot tell its own interface from any
/// other local caller, so it gives none.
pub fn install_served_operator(
    browser_token: String,
    capability: biorouter::privacy::ProviderTier,
) {
    let _ = SERVED_OPERATOR.set(ServedOperator {
        browser_token,
        capability,
    });
}

/// The capability a request earns by presenting the served document's cookie:
/// the operator's tier on a serve daemon, `Public` for every other request on
/// every other daemon.
pub fn served_operator_capability(
    headers: &axum::http::HeaderMap,
) -> biorouter::privacy::ProviderTier {
    match SERVED_OPERATOR.get() {
        Some(operator)
            if served_document_matches(
                crate::routes::web_ui::session_cookie(headers),
                &operator.browser_token,
            ) =>
        {
            operator.capability
        }
        _ => biorouter::privacy::ProviderTier::Public,
    }
}

/// Does the presented cookie carry the served document's token?
///
/// Pure, so the rule is testable without the process global; compared without
/// an early return, the same way the secret is. An empty token matches nothing.
pub fn served_document_matches(presented: Option<&str>, browser_token: &str) -> bool {
    match presented {
        Some(presented) if !browser_token.is_empty() => secret_matches(presented, browser_token),
        _ => false,
    }
}

/// How many failed authentications one address may have answered `401` inside
/// [`FAILED_AUTH_WINDOW`]. Past it, the rest of that address's failures in the
/// window are answered `429`.
const FAILED_AUTH_BUDGET: usize = 20;

/// The sliding window [`FAILED_AUTH_BUDGET`] is counted over.
const FAILED_AUTH_WINDOW: Duration = Duration::from_secs(60);

fn get_failed_attempts() -> &'static Mutex<HashMap<String, Vec<Instant>>> {
    FAILED_ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The status for a request that FAILED authentication from `client_ip`,
/// recorded against that address. Nothing that presented the correct secret
/// reaches this; see the comment at its one call site in [`check_token`].
fn refuse_unauthenticated(client_ip: &str) -> StatusCode {
    let mut map = get_failed_attempts()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let attempts = map.entry(client_ip.to_string()).or_default();
    failed_attempt_verdict(attempts, Instant::now())
}

/// The window itself, over one address's recent failures, with the clock
/// passed in so a test can step past the window instead of sleeping through it.
///
/// Over budget, the failure is answered and NOT recorded, so an address's
/// vector never holds more than [`FAILED_AUTH_BUDGET`] entries however hard it
/// is hammered. That bound predates this function and is kept on purpose.
fn failed_attempt_verdict(attempts: &mut Vec<Instant>, now: Instant) -> StatusCode {
    attempts.retain(|t| now.duration_since(*t) < FAILED_AUTH_WINDOW);
    if attempts.len() >= FAILED_AUTH_BUDGET {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    attempts.push(now);
    StatusCode::UNAUTHORIZED
}

fn is_public_app_get(method: &axum::http::Method, path: &str) -> bool {
    if method != axum::http::Method::GET {
        return false;
    }
    let Some(rest) = path.strip_prefix("/apps/") else {
        return false;
    };
    let mut segments = rest.split('/');
    let Some(id) = segments.next() else {
        return false;
    };
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return false;
    }
    // Keep this route list explicit so a future management GET does not become
    // unauthenticated merely because it lives below `/apps/{id}`.
    let tail = segments.collect::<Vec<_>>();
    matches!(
        tail.as_slice(),
        [] | [""] | ["agent"] | ["models"] | ["runstate"]
    ) || matches!(tail.as_slice(), ["dist" | "assets", _, ..])
}

/// Paths served without the `X-Secret-Key` header. Each one carries its own
/// gate; the list is a predicate rather than a chain of `||` inside
/// `check_token` so it is unit-testable — a security allowlist that no test
/// can reach is one refactor away from admitting `/ui/workspaceX`.
fn is_unauthenticated_path(path: &str) -> bool {
    // The coding-agent tool bridge. A `claude` or `codex` child calls this to use
    // Biorouter's own tools, and it cannot send the secret key: the child's
    // environment is scrubbed of it (issue #57), and Codex sends no Authorization
    // header on MCP requests at all. The capability is the 32-hex single-turn
    // nonce in the path, which `routes::tool_bridge` resolves against a grant that
    // its lease revokes when the turn ends.
    //
    // ⚠ This has to be an exemption rather than "the handler answers 401 itself":
    // the child never has the secret, so without it every bridge call is a 401
    // and the bridge never works. It used to cost more than that. Until QA-D F3
    // (2026-09-10) `check_token` consulted its failed-attempt throttle BEFORE
    // comparing the secret, keyed on the client IP, and the child's IP is
    // 127.0.0.1 — the desktop app's — so a couple of coding-agent turns 429'd the
    // user out of their own app for a minute. A correct secret is no longer
    // throttled at all, but the bridge's failures would still fill that
    // address's window for no reason, which is one more reason to keep this.
    // Exactly one segment after the prefix, for the same reason the workspace
    // socket below is an exact match: a bare `starts_with` would exempt every
    // future route under this prefix, and the daemon has no other authentication.
    if let Some(rest) = path.strip_prefix("/tool_bridge/") {
        return !rest.is_empty() && !rest.contains('/');
    }

    matches!(
        path,
        "/status"
            // BR-71: the desktop renderer opens this WebSocket, and a browser
            // WebSocket cannot send headers. The route carries its own two
            // gates — the same secret as a query token, plus the Origin check
            // (CSWSH) — in `routes::workspace::check_workspace_ws_auth`,
            // exactly as the app agent socket does (`apps::agent_ws`).
            | "/ui/workspace"
    )
}

pub async fn check_token(
    State(state): State<String>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path();
    if is_unauthenticated_path(path) {
        return Ok(next.run(request).await);
    }
    // Biorouter apps are opened directly in the browser (and connect a WebSocket),
    // so they can't send the secret-key header. Allow browser-facing GET reads
    // of a *specific* app (serving the bundle + the per-app agent socket);
    // management operations and source/content export still require the secret.
    //
    // `GET /apps` -- the list -- is deliberately NOT exempt: it enumerates app
    // ids, and an id is all `/apps/{id}/agent` needs. That socket runs agent
    // turns and carries its own tool-approval frames, so it additionally
    // validates `Origin` (see `apps::agent_ws`).
    if is_public_app_get(request.method(), path) {
        return Ok(next.run(request).await);
    }

    let secret_key = request
        .headers()
        .get("X-Secret-Key")
        .and_then(|value| value.to_str().ok());

    // The secret FIRST, and the failed-attempt throttle only for a request that
    // failed it. A caller presenting the correct secret is never throttled.
    //
    // The throttle is keyed on the peer address, and where it matters that
    // address is shared. Every browser on a `serve` daemon arrives from one
    // address: 127.0.0.1 on the default loopback bind, over an SSH tunnel, or
    // from the TLS proxy the deployment guide puts in front; on a LAN bind,
    // everyone behind one NAT. The desktop app shares 127.0.0.1 with every
    // other local process. With the throttle checked before the secret,
    // anyone's twenty failures refused everyone on that address for a minute,
    // including the user holding the correct key. A stale tab left over from a
    // previous `serve` launch was enough (QA-D F3, 2026-09-10).
    //
    // What the window still does: past twenty failures a minute, an address's
    // failures are answered `429` and no longer recorded. What it never did, and
    // so does not stop doing here, is bound guesses at the secret.
    // `/ui/workspace` compares this same secret with no throttle at all (it is
    // exempt from this middleware), and a guess that is right is not a failure.
    // Resistance to guessing is the secret's entropy: 256 random bits when the
    // desktop app or `biorouter serve` launches the daemon, 128 when a bare
    // `biorouterd` mints its own, and whatever an operator chose when they set
    // `BIOROUTER_SERVER__SECRET_KEY` by hand (`just debug-server` uses `test`).
    if secret_key.is_some_and(|key| secret_matches(key, &state)) {
        return Ok(next.run(request).await);
    }

    // Key the throttle on the real peer. `x-forwarded-for` is client-supplied,
    // so an attacker could rotate it and defeat the limit entirely.
    let client_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Err(refuse_unauthenticated(&client_ip))
}

#[cfg(test)]
mod tests {
    use super::{is_public_app_get, is_unauthenticated_path, secret_matches, user_action_matches};
    /// The shared handler-body extractor. It lives under `src/routes/` rather
    /// than here for the reason [`secret_matches`] does — see its doc.
    use crate::routes::body_of;
    use axum::http::Method;

    /// SHA-256 of `key` — what the launcher hands the daemon on stdin, and the
    /// only form of the user-action key the daemon ever holds.
    fn digest_of(key: &str) -> [u8; 32] {
        <sha2::Sha256 as sha2::Digest>::digest(key.as_bytes()).into()
    }

    #[test]
    fn a_daemon_with_no_user_action_key_refuses_every_raise() {
        // Open question 23, as an assertion. `just run-server`, a hand-run
        // `biorouterd agent`, and every headless deployment land here.
        assert!(!user_action_matches(Some("anything"), None));
        assert!(!user_action_matches(None, None));
        assert!(!user_action_matches(None, Some(&digest_of("k"))));
    }

    #[test]
    fn the_daemon_stores_a_digest_and_never_the_key() {
        let expected = digest_of("the-real-key");
        assert!(user_action_matches(Some("the-real-key"), Some(&expected)));
        assert!(!user_action_matches(Some("the-real-ke"), Some(&expected)));
        // The stored value is not itself presentable: handing the daemon back
        // what it holds must NOT authenticate. This is the assertion that fails
        // an implementation which stores the raw key "for simplicity".
        assert!(!user_action_matches(
            Some(&hex::encode(expected)),
            Some(&expected)
        ));
    }

    #[test]
    fn all_five_raise_channels_call_the_guard() {
        // A source scan, because the alternative -- five HTTP tests -- has to
        // build `AppState`, which opens the user's real session DB
        // (routes/agent.rs's `working_dir_lock_tests` doc comment). This is the
        // test that fails a PARTIAL implementation: covering `update_provider`
        // and leaving `add_extension` or the config routes open.
        //
        // `add_extension`'s guard is not `is_user_action`, and that is DR-16
        // (c) rather than an omission: attaching a private extension to a
        // public session is not a raise the USER can authorize either, so the
        // route refuses it outright and its guard is the refusal itself.
        //
        // `/config/remove` is the fifth, and the task's own enumeration said
        // four: deleting a capability key restores its DEFAULT, and for
        // `OLLAMA_HOST` that default is loopback, i.e. Private. See the comment
        // on `remove_config`.
        //
        // ⚠ What this scan CANNOT see, stated so nobody reads it as more than
        // it is: it proves each handler mentions its guard, not that the guard
        // runs before the mutation, and not that its condition is right. A
        // guard placed after `config.set`, or one reduced to `if
        // is_user_action(..) {}`, passes here. Step 5's gate group 1 is what
        // holds the shape of the condition; nothing in CI exercises these
        // routes at the HTTP layer, because `AppState::new()` may not be built
        // in a test.
        let agent_rs = include_str!("routes/agent.rs");
        let config_rs = include_str!("routes/config_management.rs");
        for (src, func, guard) in [
            (
                agent_rs,
                "async fn update_agent_provider",
                "is_user_action(",
            ),
            (
                agent_rs,
                "async fn agent_add_extension",
                "PrivateExtensionOverHttp",
            ),
            (config_rs, "pub async fn upsert_config", "is_user_action("),
            (config_rs, "pub async fn remove_config", "is_user_action("),
            (
                config_rs,
                "pub async fn set_config_provider",
                "is_user_action(",
            ),
        ] {
            assert!(
                body_of(src, func).contains(guard),
                "{func} does not consult the user-action guard (`{guard}`)"
            );
        }

        // A negative control, so the scan is provably not vacuous. This handler
        // sits in the same file, is not a raise channel, and must come back
        // WITHOUT the guard — if it does not, `body_of` is over-reading past a
        // function end and every assertion above is passing on someone else's
        // body.
        assert!(
            !body_of(agent_rs, "async fn agent_remove_extension").contains("is_user_action("),
            "the body scan is over-reading: a handler with no guard reported one"
        );
    }

    /// Issue #56 §12.4. Declassification is not a *raise*, so it is not one of
    /// the five channels above — it is the one channel that goes the other way,
    /// and it needs the same proof for the same reason: `check_token` compares
    /// one machine-wide bearer that AR-11 measured to be recoverable from inside
    /// the daemon, so an authenticated request is not evidence of a human.
    ///
    /// Its own module is where the behaviour is pinned
    /// (`routes::session::declassify_tests::the_route_needs_more_than_the_secret_key`
    /// drives all three credential sets through the real `check_token` layer).
    /// This scan is the cheap tripwire that survives a refactor which moves that
    /// test: `is_public_app_get` needs no change for this route — it only ever
    /// matches GETs under `/apps/{id}`, so a POST under `/sessions` can never
    /// reach the exemption — and with nothing in `check_token` to change either,
    /// the ONLY thing standing between the model and this route is the line this
    /// asserts is present.
    ///
    /// It also pins WHERE the proof-of-user is minted, which is the half its
    /// sibling in `privacy::declassify` cannot see.
    /// `the_proof_of_user_is_constructed_in_exactly_two_places` counts the
    /// constructor across the tree and requires exactly one call in this file
    /// (the other permitted site being the CLI's `declassify` subcommand, which
    /// has no route and no guard to assert about); this requires that call to be
    /// inside the body of the handler asserted above to consult the guard.
    /// Neither alone is enough — a count of one says
    /// nothing about which function holds it, and a guarded handler says nothing
    /// about a second, unguarded one next to it — and together they say the
    /// proof is minted once, behind the guard.
    #[test]
    fn the_declassify_route_consults_the_user_action_guard() {
        let session_rs = include_str!("routes/session.rs");
        let handler = body_of(session_rs, "async fn declassify_session");
        assert!(
            handler.contains("is_user_action("),
            "the declassify route does not consult the user-action guard"
        );
        // Split across two literals so this file does not itself become a place
        // that names the proof-of-user: its sibling audit asserts the set of
        // files containing that name is exactly {routes/session.rs}, and a
        // spelled-out needle here would make this test break that one.
        let mint = concat!("User", "Confirmation::from_typed_confirmation(");
        assert!(
            handler.contains(mint),
            "the proof-of-user is no longer minted inside the handler that checks the guard"
        );
        // Same negative control as above, in the same file: a handler that has
        // neither must come back with neither, or `body_of` is over-reading past
        // a function end and both assertions above are passing on someone else's
        // body. The mint is the one that needs this most — it sits at the very
        // bottom of `declassify_session`, so a scan that ran long would find it
        // no matter which function it was asked about.
        let unguarded = body_of(session_rs, "async fn get_session_extensions");
        assert!(
            !unguarded.contains("is_user_action("),
            "the body scan is over-reading: a handler with no guard reported one"
        );
        assert!(
            !unguarded.contains(mint),
            "the body scan is over-reading: a handler that mints nothing reported the proof"
        );
    }

    /// Issue #56 DR-18. The knowledge-base tier route is the second channel that
    /// goes the *other* way, and it needs the same proof for the same reason:
    /// `check_token` compares one machine-wide bearer that AR-11 measured to be
    /// recoverable from inside the daemon, so an authenticated request is not
    /// evidence of a human.
    ///
    /// The behaviour is pinned over HTTP by
    /// `knowledge_routes::tier_route::the_tier_route_needs_more_than_the_secret_key`
    /// and its keyless sibling; this is the cheap tripwire that survives a
    /// refactor which moves those. `is_public_app_get` needs no change for this
    /// route — it only ever matches GETs under `/apps/{id}`, so a POST under
    /// `/knowledge` can never reach the exemption — and with nothing in
    /// `check_token` to change either, the ONLY thing standing between the model
    /// and this route is the line this asserts is present.
    ///
    /// It also pins WHERE the proof-of-user is minted, the half its sibling in
    /// `knowledge::tier_user` cannot see: that test counts the constructor across
    /// the tree and requires exactly one call in `routes/knowledge.rs`; this
    /// requires that call to be inside the body of the handler that consults the
    /// guard. Neither alone is enough.
    #[test]
    fn the_kb_tier_route_consults_the_user_action_guard() {
        let knowledge_rs = include_str!("routes/knowledge.rs");
        let handler = body_of(knowledge_rs, "pub async fn set_kb_tier");
        assert!(
            handler.contains("user_action_proof("),
            "the knowledge-base tier route does not consult the user-action guard"
        );
        // Split across two literals so this file does not itself become a place
        // that names the proof-of-user: `tier_user`'s audit asserts the set of
        // files containing that name is exactly {service.rs, routes/knowledge.rs},
        // and a spelled-out needle here would break it.
        let mint = concat!("User", "KbTierChange::from_user_action(");
        assert!(
            handler.contains(mint),
            "the proof-of-user is no longer minted inside the handler that checks the guard"
        );
        // Both refusal arms, so a handler that admits the keyless daemon — the
        // easy mistake, because `Proven` is the only arm the happy path needs —
        // fails here rather than in production.
        assert!(
            handler.contains("NoKeyInstalled"),
            "the tier route does not distinguish a daemon with no user-action key"
        );

        // The negative control, so the scan is provably not vacuous: a handler in
        // the same file that has neither must come back with neither, or
        // `body_of` is over-reading past a function end.
        let unguarded = body_of(knowledge_rs, "pub async fn get_kb_tier");
        assert!(
            !unguarded.contains("user_action_proof("),
            "the body scan is over-reading: a handler with no guard reported one"
        );
        assert!(
            !unguarded.contains(mint),
            "the body scan is over-reading: a handler that mints nothing reported the proof"
        );
    }

    /// Issue #56 DR-26 / Task 49. The cross-affiliation grant route is the third
    /// channel needing a proof of a human, and it needs it for the reason the
    /// other two do: `check_token` compares one machine-wide bearer that AR-11
    /// measured to be recoverable from inside the daemon, so an authenticated
    /// request is not evidence of a person.
    ///
    /// It also pins WHERE the proof is minted, the half `privacy::grant`'s own
    /// audit cannot see: that test counts the constructor across the tree and
    /// requires exactly one call in `routes/agent.rs`; this requires that call to
    /// be inside the body of the handler that consults the guard. Neither alone is
    /// enough — a count of one says nothing about which function holds it, and a
    /// guarded handler says nothing about a second, unguarded one beside it.
    #[test]
    fn the_cross_affiliation_grant_route_consults_the_user_action_guard() {
        let agent_rs = include_str!("routes/agent.rs");
        let handler = body_of(agent_rs, "async fn agent_cross_affiliation_grant");
        assert!(
            handler.contains("user_action_proof("),
            "the cross-affiliation grant route does not consult the user-action guard"
        );
        // Split across two literals so this file does not itself become a place
        // that names the proof-of-user: `grant`'s audit asserts the set of files
        // naming it is exactly {routes/agent.rs}, and a spelled-out needle here
        // would break it.
        let mint = concat!("User", "CrossAffiliationGrant::from_user_action(");
        assert!(
            handler.contains(mint),
            "the proof-of-user is no longer minted inside the handler that checks the guard"
        );
        // Both refusal arms, so an implementation that admits the keyless daemon
        // — the easy mistake, because `Proven` is the only arm the happy path
        // needs — fails here rather than in production. They live one function
        // down, in the pure mapping
        // `routes::agent::refuse_grant_unless_user`, precisely so a TEST can
        // drive them (`only_a_proven_user_action_gets_past_the_grant_guard`);
        // this asserts the shape that test cannot see, which is that all three
        // verdicts are still distinguished somewhere.
        let mapping = body_of(agent_rs, "fn refuse_grant_unless_user");
        for arm in ["Proven", "Unproven", "NoKeyInstalled"] {
            assert!(
                mapping.contains(arm),
                "the grant guard no longer distinguishes {arm}; a daemon with no user-action \
                 key must not be told it is the model"
            );
        }

        // The negative control, so the scan is provably not vacuous: a handler in
        // the same file that has neither must come back with neither, or
        // `body_of` is over-reading past a function end.
        //
        // BOTH directions. `agent_remove_extension` sits AFTER the grant handler
        // in the file and `agent_add_extension` BEFORE it, and a control on one
        // side only passes against an extractor that over-reads towards the
        // other.
        for control in [
            "async fn agent_remove_extension",
            "async fn agent_add_extension",
        ] {
            let unguarded = body_of(agent_rs, control);
            assert!(
                !unguarded.contains("user_action_proof("),
                "the body scan is over-reading: {control} has no guard and reported one"
            );
            assert!(
                !unguarded.contains(mint),
                "the body scan is over-reading: {control} mints nothing and reported the proof"
            );
        }
    }

    /// DR-26 / Task 49 again, from the other side: adding the grant route must
    /// NOT have weakened the tier refusal beside it.
    ///
    /// `/agent/add_extension` refuses a private extension on a public session
    /// **outright**, with no user-proof branch, because that is not a raise the
    /// user can authorize either. The grant route's whole premise is the opposite
    /// case — a Private↔Private flow the user explicitly may accept — and the
    /// plausible wrong implementation is to conclude from it that the tier
    /// refusal should also become user-authorizable.
    ///
    /// ⚠ **Scoped to the span the claim is about, since Task 58.** This handler
    /// now *does* have a proof-of-user branch — `session_reach(` on its first
    /// line, which asks whether the caller may address the named chat at all.
    /// That is a different question with a different answer: it is a no-op for
    /// every public chat, which is precisely the case this refusal governs. So
    /// the assertion can no longer be "the word does not appear in this handler".
    /// That form went on passing through Task 58 by SPELLING — the new branch
    /// reaches the proof through a helper — and it would go on passing if someone
    /// bolted `&& !is_user_action(&headers)` onto the tier condition through
    /// another one. What is asserted instead is the property, in two parts:
    ///
    /// 1. the refusal's own guard names **exactly three** things — tiers being
    ///    enforced, the extension's classification, the bound provider's tier —
    ///    so a fourth input cannot be added to it, by a helper or otherwise;
    /// 2. and nothing downstream of the reach gate mentions the proof at all.
    ///
    /// The first is what the old form could not do: a conjunct spelled through
    /// a helper reads as ordinary code to a denylist, and the old assertion
    /// passed against one.
    ///
    /// ⚠ **The three inputs are NAMED, not counted, since `2662619a`.** That
    /// commit ("one enable gate, one clause order, called from every door")
    /// replaced this route's hand-written `classification.tier.is_private() &&
    /// capability == ProviderTier::Public` with the shared
    /// `refusal::tier_refuses(..)` predicate, so that the four doors to this
    /// capability could not drift apart — and in doing so folded two of the
    /// three conjuncts into a call. Counting `&&` therefore read 1 where it had
    /// read 2, against a guard whose inputs had not changed at all. The claim is
    /// unchanged and the scan is stricter for it: the old form pinned only *how
    /// many* things the guard named, this one pins **which** three, so swapping
    /// one of them for the caller is caught where a bare count would have
    /// stayed green.
    #[test]
    fn the_add_extension_route_still_refuses_a_public_session_outright() {
        let handler = body_of(
            include_str!("routes/agent.rs"),
            "async fn agent_add_extension",
        );
        assert!(
            handler.contains("PrivateExtensionOverHttp"),
            "the outright tier refusal has left /agent/add_extension"
        );
        // The reach gate is the ONE proof-of-user call this handler may make, and
        // `routes::session_reach`'s ordering scan pins it ahead of everything
        // else. If it is ever absent, the whole body is the span — which is the
        // stricter reading, and the right one.
        let tier_decision = handler
            .split_once("session_reach(")
            .map_or(handler, |(_, after_the_gate)| after_the_gate);
        // The REFUSAL, not the prose about it. `PrivateExtensionOverHttp` is
        // also named in the comment above the guard, which explains why the
        // predicate is asked rather than re-typed — and splitting on the bare
        // name landed there instead, putting the guard outside this span and
        // making the scan report a refusal that had gone nowhere. The
        // path-qualified spelling is the constructor and nothing else.
        let (before_the_refusal, _) = tier_decision
            .split_once("PrivacyRefusal::PrivateExtensionOverHttp")
            .expect(
                "the tier refusal now runs BEFORE the reach gate, so this scan no longer \
                     covers the span its claim is about",
            );
        // Back to the `if` that guards it — the last one before the refusal,
        // whatever a reformat does to the line breaks inside the condition.
        let (_, guard) = before_the_refusal
            .rsplit_once("    if ")
            .expect("the outright tier refusal is no longer guarded by an `if`");
        let (condition, _) = guard
            .split_once('{')
            .expect("the guarded block is no longer a block");
        // The three inputs, by name. Two of them now reach the guard as the
        // arguments of `tier_refuses`, so they are asserted where they are
        // rather than as a conjunct count.
        for input in ["enforced", "classification.tier", "capability"] {
            assert!(
                condition.contains(input),
                "the outright tier refusal no longer reads `{input}`. Its three inputs are \
                 tiers being enforced, the extension's classification, and the bound \
                 provider's tier; dropping one is a change to what it refuses. \
                 Guard reads: {condition}"
            );
        }
        assert_eq!(
            condition.matches("&&").count(),
            1,
            "the outright tier refusal takes a FOURTH input beyond the extension's \
             classification, the bound provider's tier, and DR-15's master opt-out. If that \
             input is the caller, attaching a private extension to a public chat has become a \
             raise the user can authorize, which DR-16 says it is not, and a helper would \
             hide it from the scan below. Guard reads: {condition}"
        );
        for proof in ["user_action_proof(", "is_user_action(", "X-User-Action"] {
            assert!(
                !tier_decision.contains(proof),
                "attaching a private extension to a public chat is not a raise the user can \
                 authorize, so nothing downstream of the reach gate may branch on `{proof}`"
            );
        }
    }

    #[test]
    fn the_workspace_socket_is_exempt_and_nothing_that_merely_starts_with_it_is() {
        assert!(is_unauthenticated_path("/ui/workspace"));
        // Exact match only. A `starts_with` would exempt every future route
        // under this prefix, and the daemon has no other authentication.
        assert!(!is_unauthenticated_path("/ui/workspaceX"));
        assert!(!is_unauthenticated_path("/ui/workspace/admin"));
        assert!(!is_unauthenticated_path("/ui/workspace?secret=x"));
        // The other exempt path still is.
        assert!(is_unauthenticated_path("/status"));
        // …and nothing else is.
        assert!(!is_unauthenticated_path("/reply"));
        assert!(!is_unauthenticated_path("/sessions"));
    }

    /// The coding-agent tool bridge is exempt, and only its one nonce segment is.
    ///
    /// Why it must be exempt at all, rather than letting the handler answer for
    /// itself: the child never has the secret, so `check_token` would answer
    /// every bridge call `401` and record it against 127.0.0.1 — the desktop
    /// app's own address. Before QA-D F3 that also 429'd the user out of their
    /// own app for a minute, because the throttle ran before the secret was
    /// compared (`a_correct_secret_is_never_throttled_by_someone_elses_failures`
    /// pins the fix). This was found by running a real turn against a real
    /// daemon; the route's own tests served it on a bare Router with no
    /// middleware and so could not see it.
    #[test]
    fn the_tool_bridge_is_exempt_and_only_its_nonce_segment_is() {
        assert!(is_unauthenticated_path(
            "/tool_bridge/21be84521e4745fbb1db5ff4e5d789a3"
        ));
        // One segment only. Anything deeper is a different route and gets the
        // daemon's ordinary authentication.
        assert!(!is_unauthenticated_path("/tool_bridge/abc/admin"));
        assert!(!is_unauthenticated_path("/tool_bridge/"));
        assert!(!is_unauthenticated_path("/tool_bridge"));
        // A prefix that merely looks similar is not exempt.
        assert!(!is_unauthenticated_path("/tool_bridgeX/abc"));
    }

    /// The serve daemon's operator standing is earned by the served document's
    /// cookie and by nothing else: the whole token, not a prefix; not an empty
    /// one; not an absent one.
    #[test]
    fn only_the_served_documents_cookie_earns_the_operator_standing() {
        use super::served_document_matches;
        assert!(served_document_matches(Some("0123abcd"), "0123abcd"));
        assert!(!served_document_matches(Some("0123abc"), "0123abcd"));
        assert!(!served_document_matches(Some(""), "0123abcd"));
        assert!(!served_document_matches(None, "0123abcd"));
        // An empty token is "no token", and "no token" earns nothing — never
        // the equality of two empty strings.
        assert!(!served_document_matches(Some(""), ""));
    }

    #[test]
    fn secret_compare_is_exact() {
        assert!(secret_matches("abc", "abc"));
        assert!(!secret_matches("abc", "abd"));
        assert!(!secret_matches("ab", "abc"));
        assert!(!secret_matches("", "abc"));
    }

    #[test]
    fn app_exports_still_require_the_server_secret() {
        assert!(is_public_app_get(&Method::GET, "/apps/example/"));
        assert!(is_public_app_get(&Method::GET, "/apps/example/dist/app.js"));
        assert!(is_public_app_get(&Method::GET, "/apps/example/agent"));
        assert!(!is_public_app_get(&Method::GET, "/apps/example/export"));
        assert!(!is_public_app_get(&Method::GET, "/apps/example/export/"));
        assert!(!is_public_app_get(
            &Method::GET,
            "/apps/example/future-admin"
        ));
        assert!(!is_public_app_get(&Method::GET, "/apps/bad%2Fid/"));
        assert!(!is_public_app_get(&Method::POST, "/apps/example/build"));
    }

    // --- The failed-attempt throttle (QA-D F3) ------------------------------

    use super::{check_token, failed_attempt_verdict, FAILED_AUTH_BUDGET, FAILED_AUTH_WINDOW};
    use axum::extract::ConnectInfo;
    use axum::http::StatusCode;
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};
    use tower::ServiceExt;

    const TEST_SECRET: &str = "qa-d-f3-throttle-secret";

    /// The real middleware in front of one trivial route, layered the way
    /// `commands::agent::run` layers it in front of the real router.
    fn guarded() -> axum::Router {
        axum::Router::new()
            .route("/sessions", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                TEST_SECRET.to_string(),
                check_token,
            ))
    }

    /// One request from `peer`. The throttle keys on the peer address, which
    /// the real server puts on every request through
    /// `into_make_service_with_connect_info`; `oneshot` has no connection, so
    /// the test puts it there itself. Each test uses addresses from the
    /// documentation range that no other test uses: the throttle's map is one
    /// process-wide static.
    async fn status_from(app: &axum::Router, peer: [u8; 4], secret: Option<&str>) -> StatusCode {
        let mut request = axum::http::Request::builder()
            .uri("/sessions")
            .body(axum::body::Body::empty())
            .unwrap();
        if let Some(secret) = secret {
            request
                .headers_mut()
                .insert("X-Secret-Key", secret.parse().unwrap());
        }
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((peer, 51_000))));
        app.clone().oneshot(request).await.unwrap().status()
    }

    /// QA-D F3, as reported: twenty-one requests without a secret, then the
    /// user's own correctly authenticated request from the same address. That
    /// request was answered `429` for a minute, because the throttle ran before
    /// the secret was compared. On a `serve` daemon every browser shares one
    /// address, so "the same address" is the normal case, not a contrived one.
    #[tokio::test]
    async fn a_correct_secret_is_never_throttled_by_someone_elses_failures() {
        let app = guarded();
        let shared = [203, 0, 113, 31];

        // The attacker's half first, so the other assertions cannot pass
        // against a throttle that simply stopped working.
        for n in 1..=FAILED_AUTH_BUDGET {
            assert_eq!(
                status_from(&app, shared, None).await,
                StatusCode::UNAUTHORIZED,
                "failure {n} is inside the budget"
            );
        }
        assert_eq!(
            status_from(&app, shared, None).await,
            StatusCode::TOO_MANY_REQUESTS,
            "past the budget, an address's failures are refused 429"
        );
        // A WRONG secret is a failure too, and is locked out the same way: the
        // report's other shape, a stale tab still holding the previous launch's
        // secret.
        assert_eq!(
            status_from(&app, shared, Some("the-previous-launchs-secret")).await,
            StatusCode::TOO_MANY_REQUESTS
        );

        // The user, from the same address, inside the same window.
        assert_eq!(
            status_from(&app, shared, Some(TEST_SECRET)).await,
            StatusCode::OK,
            "a correct secret must never be throttled by someone else's failures"
        );

        // And the user's success lifts nothing for the attacker: this address's
        // next failure is still refused.
        assert_eq!(
            status_from(&app, shared, None).await,
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    /// The window is per address. A bystander's first failure is answered as
    /// its own first, not as the attacker's twenty-first.
    #[tokio::test]
    async fn one_addresss_failures_do_not_throttle_another_address() {
        let app = guarded();
        let attacker = [203, 0, 113, 32];
        let bystander = [203, 0, 113, 33];

        for _ in 0..FAILED_AUTH_BUDGET {
            status_from(&app, attacker, Some("guess")).await;
        }
        assert_eq!(
            status_from(&app, attacker, Some("guess")).await,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            status_from(&app, bystander, Some("typo")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    /// The window over synthetic time, so its edges are asserted rather than
    /// slept through. Also pins the bound `refuse_unauthenticated` relies on:
    /// an address over its budget is answered without being recorded, so a
    /// flood from it cannot grow its entry.
    #[test]
    fn the_failure_window_slides_and_never_holds_more_than_the_budget() {
        let t0 = Instant::now();
        let mut attempts = Vec::new();
        for _ in 0..FAILED_AUTH_BUDGET {
            assert_eq!(
                failed_attempt_verdict(&mut attempts, t0),
                StatusCode::UNAUTHORIZED
            );
        }
        for _ in 0..1_000 {
            assert_eq!(
                failed_attempt_verdict(&mut attempts, t0),
                StatusCode::TOO_MANY_REQUESTS
            );
        }
        assert_eq!(attempts.len(), FAILED_AUTH_BUDGET);

        // One millisecond short of the window, every failure still counts.
        assert_eq!(
            failed_attempt_verdict(
                &mut attempts,
                t0 + FAILED_AUTH_WINDOW - Duration::from_millis(1)
            ),
            StatusCode::TOO_MANY_REQUESTS
        );
        // At the window, they have all aged out and the address starts over.
        assert_eq!(
            failed_attempt_verdict(&mut attempts, t0 + FAILED_AUTH_WINDOW),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(attempts.len(), 1);
    }
}
