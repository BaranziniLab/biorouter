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

static COMPUTER_USE_FAILED_ATTEMPTS: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
const COMPUTER_USE_ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
const COMPUTER_USE_ATTEMPT_BUDGET: usize = 10;

pub fn computer_use_action_locked_out() -> bool {
    let mut attempts = COMPUTER_USE_FAILED_ATTEMPTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    prune_computer_use_attempts(&mut attempts, Instant::now());
    attempts.len() >= COMPUTER_USE_ATTEMPT_BUDGET
}

fn prune_computer_use_attempts(attempts: &mut Vec<Instant>, now: Instant) {
    attempts.retain(|attempt| now.duration_since(*attempt) < COMPUTER_USE_ATTEMPT_WINDOW);
}

static COMPUTER_USE_ACTION_DIGEST: OnceLock<Option<[u8; 32]>> = OnceLock::new();

pub fn install_computer_use_action_digest(digest: Option<[u8; 32]>) {
    let _ = COMPUTER_USE_ACTION_DIGEST.set(digest);
}

/// Browser approval authority for Biorouter Copilot is deliberately separate from
/// general user authority.
/// The launcher passes only a digest on stdin; neither the browser cookie nor the
/// daemon API secret can authorize desktop access.
pub fn computer_use_action_proof(headers: &axum::http::HeaderMap) -> UserActionProof {
    let existing = user_action_proof(headers);
    if existing == UserActionProof::Proven {
        return existing;
    }
    let digest = COMPUTER_USE_ACTION_DIGEST.get().and_then(Option::as_ref);
    if digest.is_none() {
        return existing;
    }
    let mut attempts = COMPUTER_USE_FAILED_ATTEMPTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let now = Instant::now();
    prune_computer_use_attempts(&mut attempts, now);
    if attempts.len() >= COMPUTER_USE_ATTEMPT_BUDGET {
        return UserActionProof::Unproven;
    }
    let proof = computer_use_proof_with_digest(headers, existing, digest);
    if proof == UserActionProof::Proven {
        attempts.clear();
    } else {
        attempts.push(now);
    }
    proof
}

fn computer_use_proof_with_digest(
    headers: &axum::http::HeaderMap,
    existing: UserActionProof,
    expected: Option<&[u8; 32]>,
) -> UserActionProof {
    if existing == UserActionProof::Proven {
        return existing;
    }
    let Some(expected) = expected else {
        return existing;
    };
    if user_action_matches(
        headers
            .get("X-Computer-Use-Key")
            .and_then(|value| value.to_str().ok()),
        Some(expected),
    ) {
        UserActionProof::Proven
    } else {
        UserActionProof::Unproven
    }
}

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
        Some(operator) if from_served_document(headers, operator) => operator.capability,
        _ => biorouter::privacy::ProviderTier::Public,
    }
}

/// Whether `headers` carry the cookie of the document a serve daemon served.
fn from_served_document(headers: &axum::http::HeaderMap, operator: &ServedOperator) -> bool {
    served_document_matches(
        crate::routes::web_ui::session_cookie(headers),
        &operator.browser_token,
    )
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

// --- Agent Drafter apps: the browser surface (W2-HRD-1) ---------------------
//
// A browser tab cannot send `X-Secret-Key`, so an app's page, bundle and agent
// socket take a credential a browser can carry: a per-app access cookie, set by
// redeeming a launch link that only a caller holding the secret can mint
// (`POST /apps/{id}/launch`). These routes used to be exempt outright, on the
// premise that the daemon is loopback-only; loopback is not one account. On a
// shared login node any local account could GET an app's page, read the socket
// token in it and drive that app's agent under the owner's account and key, and
// the 404 for a guessed title slug said which apps existed.
//
// Two values, deliberately. The LAUNCH token rides a URL, into the browser's
// history and whatever hands the URL to the browser. It is single-use and
// expires within minutes, so a copy found afterwards opens nothing. The COOKIE
// value never appears in a URL or on a command line; it lives for the daemon's
// run, so a reload, the agent socket and every asset keep working after the one
// redemption.
//
// ⚠ Single use does nothing against a reader who is FIRST. Whoever redeems the
// link gets the cookie, and the victim's browser sees only the refusal below. So
// a launch link must never go on a command line, where every account on the
// machine can read it (`ps` on macOS, `/proc/<pid>/cmdline` on Linux) while
// `open`, `xdg-open` or a starting browser run, and a co-tenant polling for
// `?t=` outruns the browser. Every opener Biorouter ships writes the link into a
// file only this account can read and opens the file (`biorouter apps open`,
// the exported `run.sh` and `run.ps1`, the desktop's Applications view), and
// the exchange answers with a page rather than a redirect so that works under
// `SameSite=Strict` (`routes::apps::launch_bounce`). The desktop preview loads
// the link in-process. `biorouter serve --open` hands over the served
// document's browser token the same way, and `routes::web_ui::exchange_bounce`
// answers it the same way: that token opens every app below on a serve daemon
// (`app_access_granted_by`), and it is not single use.
//
// ⚠ The stores live HERE, in the lib-only `auth`, and not in `routes::apps`:
// `src/routes/` is compiled twice (`lib.rs`), so a static there exists once for
// the daemon binary's routes and once for this module, and a link the launch
// route minted would never match what is checked below. `routes::apps` names
// these functions through `biorouter_server::auth`.

/// How long a launch link may wait to be opened.
const APP_LAUNCH_TTL: Duration = Duration::from_secs(5 * 60);

/// How many unredeemed launch links are kept at once; the oldest goes first. A
/// caller that holds the secret mints them, so this is a bound, not a defense.
const APP_LAUNCH_LIMIT: usize = 256;

#[derive(Default)]
struct AppAccess {
    /// The access cookie's value for each app a link was minted for, for the
    /// daemon's run. Never written to disk, so a restarted daemon hands out new
    /// ones and an old cookie stops working.
    cookies: HashMap<String, String>,
    /// Unredeemed launch tokens: which app each opens, and until when.
    launches: HashMap<String, (String, Instant)>,
}

impl AppAccess {
    fn mint(&mut self, app_id: &str, now: Instant) -> String {
        self.cookies
            .entry(app_id.to_string())
            .or_insert_with(random_token);
        self.launches.retain(|_, (_, until)| *until > now);
        while self.launches.len() >= APP_LAUNCH_LIMIT {
            let oldest = self
                .launches
                .iter()
                .min_by_key(|(_, (_, until))| *until)
                .map(|(token, _)| token.clone());
            match oldest {
                Some(token) => self.launches.remove(&token),
                None => break,
            };
        }
        let token = random_token();
        self.launches
            .insert(token.clone(), (app_id.to_string(), now + APP_LAUNCH_TTL));
        token
    }

    fn pending(&self, app_id: &str, presented: &str, now: Instant) -> bool {
        self.launches
            .get(presented)
            .is_some_and(|(app, until)| app == app_id && *until > now)
    }

    /// The cookie value `presented` redeems for, once. A token for another app
    /// is left alone, so it cannot be spent by a page it does not open.
    fn redeem(&mut self, app_id: &str, presented: &str, now: Instant) -> Option<String> {
        let (app, until) = self.launches.get(presented)?.clone();
        if app != app_id {
            return None;
        }
        self.launches.remove(presented);
        if until <= now {
            return None;
        }
        self.cookies.get(app_id).cloned()
    }

    /// Compares against a stand-in when `app_id` has no cookie, so the work done
    /// does not say whether a link was ever minted for that app.
    fn cookie_matches(&self, app_id: &str, presented: &str) -> bool {
        const STAND_IN: &str = "0000000000000000000000000000000000000000000000000000000000000000";
        let expected = self.cookies.get(app_id);
        let matched = secret_matches(presented, expected.map_or(STAND_IN, String::as_str));
        matched && expected.is_some()
    }
}

static APP_ACCESS: OnceLock<Mutex<AppAccess>> = OnceLock::new();

fn app_access() -> std::sync::MutexGuard<'static, AppAccess> {
    APP_ACCESS
        .get_or_init(|| Mutex::new(AppAccess::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

/// The access cookie's name, unique to this daemon run.
///
/// A cookie is scoped by host and path but not by port, so two daemons on
/// 127.0.0.1 (the desktop's and one `biorouter apps open` started, both serving
/// the same app store) would otherwise overwrite each other's cookie for the
/// same app id, and the first tab would stop working.
fn app_access_cookie_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let run: [u8; 8] = rand::random();
        format!("biorouter_app_{}", hex::encode(run))
    })
}

/// A new single-use launch token for `app_id`, good for [`APP_LAUNCH_TTL`].
/// Called by the launch route, which requires the secret, after checking the
/// app exists; nothing an unauthenticated caller can reach mints, so such a
/// caller can neither grow these stores nor learn from them which apps exist.
pub fn mint_app_launch(app_id: &str) -> String {
    app_access().mint(app_id, Instant::now())
}

/// Redeem `presented` for `app_id`: once, the `Set-Cookie` value that grants the
/// app's browser surface. A token that is unknown, expired, already redeemed or
/// for another app gets `None`, and a redeemed one is gone.
///
/// `Path=/apps/{id}` (no trailing slash) covers the redirect route, the page and
/// everything below it, and no other app. `HttpOnly` keeps the page's own script
/// from reading it, and `SameSite=Strict` keeps it off every request another site
/// starts: the two flags `routes::web_ui` sets on the served document's cookie.
pub fn redeem_app_launch(app_id: &str, presented: &str) -> Option<String> {
    let value = app_access().redeem(app_id, presented, Instant::now())?;
    Some(format!(
        "{}={value}; Path=/apps/{app_id}; HttpOnly; SameSite=Strict",
        app_access_cookie_name()
    ))
}

/// The access cookie a request carries, if any.
fn app_access_cookie(headers: &axum::http::HeaderMap) -> Option<&str> {
    let name = app_access_cookie_name();
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim())
}

/// The launch token in a page request's query string (`?t=…`), if any.
fn launch_token(query: Option<&str>) -> Option<&str> {
    query?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "t")
        .map(|(_, value)| value)
}

/// One of an app's browser-facing GETs, as [`is_app_browser_get`] recognised it.
#[derive(Debug, PartialEq, Eq)]
struct AppBrowserGet<'a> {
    id: &'a str,
    /// `/apps/{id}/` itself: the one path that may carry the launch token.
    page: bool,
}

/// The routes a browser loads for a live app: its page, bundle, assets, model
/// catalog, run state and agent socket. Recognised by shape, before any handler
/// runs, so the answer never depends on whether the app exists.
fn is_app_browser_get<'a>(method: &axum::http::Method, path: &'a str) -> Option<AppBrowserGet<'a>> {
    if method != axum::http::Method::GET {
        return None;
    }
    let rest = path.strip_prefix("/apps/")?;
    let mut segments = rest.split('/');
    let id = segments.next()?;
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    // Keep this route list explicit so a future management GET does not become
    // reachable on a browser's cookie merely because it lives below `/apps/{id}`.
    let tail = segments.collect::<Vec<_>>();
    let browser = matches!(
        tail.as_slice(),
        [] | [""] | ["agent"] | ["models"] | ["runstate"]
    ) || matches!(tail.as_slice(), ["dist" | "assets", _, ..]);
    browser.then_some(AppBrowserGet {
        id,
        page: tail.as_slice() == [""],
    })
}

/// Whether a request to an app's browser surface carries that app's access: its
/// cookie, the launch token on the page itself, or, on a `biorouter serve`
/// daemon, the cookie of the document it served (whose holder was handed the
/// daemon secret with it, so this grants nothing new). The answer depends on
/// nothing but these, so it is the same for an app that exists and one that
/// does not.
fn app_access_granted(
    app: &AppBrowserGet<'_>,
    query: Option<&str>,
    headers: &axum::http::HeaderMap,
) -> bool {
    app_access_granted_by(app, query, headers, SERVED_OPERATOR.get())
}

/// [`app_access_granted`] with the serve daemon's standing passed in, so a test
/// can drive it without installing the process global.
fn app_access_granted_by(
    app: &AppBrowserGet<'_>,
    query: Option<&str>,
    headers: &axum::http::HeaderMap,
    operator: Option<&ServedOperator>,
) -> bool {
    if operator.is_some_and(|operator| from_served_document(headers, operator)) {
        return true;
    }
    let access = app_access();
    if app_access_cookie(headers).is_some_and(|value| access.cookie_matches(app.id, value)) {
        return true;
    }
    app.page
        && launch_token(query).is_some_and(|token| access.pending(app.id, token, Instant::now()))
}

/// The refusal an app's browser surface gets without access: the same status and
/// body for every app id, existing or not, so it answers nothing about which
/// apps exist. Written for the person who followed an old or bare link.
fn app_access_refused(status: StatusCode) -> Response {
    let mut response = Response::new(axum::body::Body::from(APP_ACCESS_REFUSED_HTML));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

const APP_ACCESS_REFUSED_HTML: &str =
    "<!doctype html><meta charset=utf-8><title>Biorouter app</title>\
     <body style=\"font:15px system-ui;margin:3rem auto;max-width:32rem\">\
     <h1 style=\"font-size:1.2rem\">Open this app from Biorouter</h1>\
     <p>An app's address works only in the browser Biorouter opened it in, and only until \
     Biorouter restarts. Open the app again from its preview in the chat, from Applications, \
     or with <code>biorouter apps open</code> and the app's name.</p>";

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
    // so they can't send the secret-key header. A browser-facing GET of a
    // *specific* app is admitted on that app's access cookie (W2-HRD-1, see
    // `app_access_granted`); management operations and source/content export
    // still require the secret, and so does `GET /apps`, the list.
    //
    // Nothing here is exempt any more. The agent socket runs agent turns and
    // carries its own tool-approval frames, and its Origin check and socket token
    // (`apps::agent_ws`) were the only gates while the page holding that token
    // was served to every local account.
    let app = is_app_browser_get(request.method(), path);
    if app
        .as_ref()
        .is_some_and(|app| app_access_granted(app, request.uri().query(), request.headers()))
    {
        return Ok(next.run(request).await);
    }
    let app_browser = app.is_some();

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
        if !crate::daemon_service::instance_matches(request.headers()) {
            return Err(StatusCode::CONFLICT);
        }
        return Ok(next.run(request).await);
    }

    // Key the throttle on the real peer. `x-forwarded-for` is client-supplied,
    // so an attacker could rotate it and defeat the limit entirely.
    let client_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let refused = refuse_unauthenticated(&client_ip);
    if app_browser {
        // One answer for every app id, existing or not (W2-HRD-1).
        return Ok(app_access_refused(refused));
    }
    Err(refused)
}

#[cfg(test)]
mod tests {
    use super::{is_app_browser_get, is_unauthenticated_path, secret_matches, user_action_matches};
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
    ///
    /// ⚠ **The needle is [`user_action_proof`], not `is_user_action`** (SD-8,
    /// 2026-09-12), the same spelling `the_kb_tier_route_consults_the_user_action_guard`
    /// below already asserts. The boolean form collapses `Unproven` and
    /// `NoKeyInstalled`, and a `biorouter serve` daemon — where every caller,
    /// the person at the keyboard included, is `NoKeyInstalled` — answered a
    /// human with the sentence written for a model. The guard is unchanged in
    /// what it admits; only the verdict it reads is finer.
    ///
    /// ⚠ **A consulted verdict is not a guard**, which is the way this scan
    /// could have gone soft: `user_action_proof(` would still be present in a
    /// handler that read it and then ignored it. So the refusal's early return
    /// is asserted too, and the two together say the verdict is read AND acted
    /// on before anything else happens.
    #[test]
    fn the_declassify_route_consults_the_user_action_guard() {
        let session_rs = include_str!("routes/session.rs");
        let handler = body_of(session_rs, "async fn declassify_session");
        assert!(
            handler.contains("user_action_proof("),
            "the declassify route does not consult the user-action guard"
        );
        assert!(
            handler.contains("declassify_refusal(user_action_proof(&headers))"),
            "the declassify route reads the user-action verdict but no longer hands it to the \
             function that turns it into a refusal"
        );
        assert!(
            handler.contains("return Err((StatusCode::FORBIDDEN, refusal).into_response());"),
            "the declassify route no longer REFUSES on the user-action verdict it read"
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
            !unguarded.contains("user_action_proof("),
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
        let browser = |method: &Method, path: &str| is_app_browser_get(method, path).is_some();
        assert!(browser(&Method::GET, "/apps/example/"));
        assert!(browser(&Method::GET, "/apps/example/dist/app.js"));
        assert!(browser(&Method::GET, "/apps/example/agent"));
        assert!(!browser(&Method::GET, "/apps/example/export"));
        assert!(!browser(&Method::GET, "/apps/example/export/"));
        assert!(!browser(&Method::GET, "/apps/example/future-admin"));
        assert!(!browser(&Method::GET, "/apps/bad%2Fid/"));
        assert!(!browser(&Method::POST, "/apps/example/build"));
        assert!(!browser(&Method::POST, "/apps/example/launch"));
        assert!(!browser(&Method::GET, "/apps"));
        // Only the page itself may carry the launch token.
        assert!(is_app_browser_get(&Method::GET, "/apps/example/").is_some_and(|app| app.page));
        for not_the_page in [
            "/apps/example",
            "/apps/example/agent",
            "/apps/example/dist/app.js",
        ] {
            assert!(
                is_app_browser_get(&Method::GET, not_the_page).is_some_and(|app| !app.page),
                "{not_the_page}"
            );
        }
    }

    // --- W2-HRD-1: an app's browser surface ---------------------------------

    use super::{
        app_access_granted_by, check_token as the_middleware, mint_app_launch, redeem_app_launch,
        AppAccess, ServedOperator, APP_LAUNCH_LIMIT, APP_LAUNCH_TTL,
    };

    /// Stand-ins for the app routes: `present-app` exists and every other id
    /// answers the handler's own 404, which a caller without access must never see.
    async fn stand_in(
        axum::extract::Path(params): axum::extract::Path<std::collections::HashMap<String, String>>,
    ) -> (StatusCode, &'static str) {
        if params.get("id").map(String::as_str) == Some("present-app") {
            (StatusCode::OK, "reached the app route")
        } else {
            (StatusCode::NOT_FOUND, "no such app")
        }
    }

    /// The real middleware in front of the app routes' shapes, as
    /// `commands::agent::run` layers it in front of the real router.
    fn guarded_apps() -> axum::Router {
        axum::Router::new()
            .route("/status", axum::routing::get(|| async { "ok" }))
            .route("/apps/{id}", axum::routing::get(stand_in))
            .route("/apps/{id}/", axum::routing::get(stand_in))
            .route("/apps/{id}/agent", axum::routing::get(stand_in))
            .route("/apps/{id}/models", axum::routing::get(stand_in))
            .route("/apps/{id}/dist/{*path}", axum::routing::get(stand_in))
            .route("/apps/{id}/launch", axum::routing::post(stand_in))
            .layer(axum::middleware::from_fn_with_state(
                TEST_SECRET.to_string(),
                the_middleware,
            ))
    }

    /// One request from `peer` (addresses from TEST-NET-2, which no other test
    /// here uses: the throttle's map is one process-wide static).
    async fn app_request(
        app: &axum::Router,
        peer: [u8; 4],
        method: Method,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap();
        for (name, value) in headers {
            request.headers_mut().append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((peer, 51_000))));
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// The `Cookie` header a browser sends after opening a launch link for
    /// `app_id`.
    fn cookie_for(app_id: &str) -> String {
        let launch = mint_app_launch(app_id);
        redeem_app_launch(app_id, &launch)
            .expect("a fresh launch link redeems")
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    /// W2-HRD-1, as the QA probe found it: from another account on the login
    /// node, with no secret, `/apps/<guess>/` answered 404 `no such app` for a
    /// miss, and the page (with the socket token in it) for a hit. Every
    /// browser-facing app route now answers one 401, the same for both.
    #[tokio::test]
    async fn an_app_route_answers_the_same_401_whether_or_not_the_app_exists() {
        let app = guarded_apps();
        let peer = [198, 51, 100, 41];
        let mut answers = Vec::new();
        for uri in [
            "/apps/present-app/",
            "/apps/missing-app/",
            "/apps/present-app",
            "/apps/missing-app",
            "/apps/present-app/agent?token=0123",
            "/apps/missing-app/agent",
            "/apps/present-app/models",
            "/apps/present-app/dist/app.js",
        ] {
            let (status, body) = app_request(&app, peer, Method::GET, uri, &[]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
            assert!(!body.contains("no such app"), "{uri}: {body}");
            answers.push(body);
        }
        answers.dedup();
        assert_eq!(answers.len(), 1, "every app route answers the same body");
        // Liveness stays open.
        assert_eq!(
            app_request(&app, peer, Method::GET, "/status", &[]).await.0,
            StatusCode::OK
        );
    }

    /// The socket's own gates, an `Origin` check and a token read off the page,
    /// admitted any local client that sent no `Origin`. An `Origin` is proof of
    /// nothing outside a browser, so the upgrade needs the cookie (or the secret)
    /// whatever `Origin` it sends.
    #[tokio::test]
    async fn the_agent_socket_needs_the_access_cookie_whatever_origin_it_sends() {
        let app = guarded_apps();
        let peer = [198, 51, 100, 42];
        let launch = mint_app_launch("present-app");
        let upgrade = [
            ("connection", "upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-version", "13"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ];
        let uri = "/apps/present-app/agent?token=from-the-page";
        let bare = app_request(&app, peer, Method::GET, uri, &upgrade).await;
        assert_eq!(bare.0, StatusCode::UNAUTHORIZED, "no Origin");
        let mut same_origin = upgrade.to_vec();
        same_origin.push(("host", "127.0.0.1:41841"));
        same_origin.push(("origin", "http://127.0.0.1:41841"));
        assert_eq!(
            app_request(&app, peer, Method::GET, uri, &same_origin)
                .await
                .0,
            StatusCode::UNAUTHORIZED,
            "a matching Origin"
        );
        // The launch token is for the page, not the socket.
        let with_launch_token = format!("/apps/present-app/agent?t={launch}");
        assert_eq!(
            app_request(&app, peer, Method::GET, &with_launch_token, &upgrade)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let cookie = cookie_for("present-app");
        let mut with_cookie = upgrade.to_vec();
        with_cookie.push(("cookie", &cookie));
        assert_eq!(
            app_request(&app, peer, Method::GET, uri, &with_cookie)
                .await
                .0,
            StatusCode::OK,
            "the cookie reaches the socket's own gates"
        );
    }

    /// One app's cookie opens that app's routes and no other app's; a cookie
    /// with the right value under another name, or a wrong value, opens nothing;
    /// and the launch token opens the page it is exchanged on and nothing else.
    #[tokio::test]
    async fn an_apps_access_opens_that_app_and_nothing_else() {
        let app = guarded_apps();
        let peer = [198, 51, 100, 43];
        let cookie = cookie_for("present-app");
        let (name, value) = cookie.split_once('=').unwrap();
        for uri in [
            "/apps/present-app/",
            "/apps/present-app",
            "/apps/present-app/agent",
            "/apps/present-app/models",
            "/apps/present-app/dist/app.js",
        ] {
            let (status, _) =
                app_request(&app, peer, Method::GET, uri, &[("cookie", &cookie)]).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
        }
        // Another app, on this app's cookie: nothing minted for it, so nothing matches.
        assert_eq!(
            app_request(
                &app,
                peer,
                Method::GET,
                "/apps/other-app/",
                &[("cookie", &cookie)]
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        let unredeemed = mint_app_launch("present-app");
        for wrong in [
            format!("{name}={}", "0".repeat(64)),
            format!("{name}="),
            format!("biorouter_app={value}"),
            format!("x{name}={value}"),
            // A launch token is not a cookie value.
            format!("{name}={unredeemed}"),
        ] {
            assert_eq!(
                app_request(
                    &app,
                    peer,
                    Method::GET,
                    "/apps/present-app/",
                    &[("cookie", &wrong)]
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED,
                "{wrong}"
            );
        }
        // An unredeemed launch token, on the page only.
        let page = format!("/apps/present-app/?t={unredeemed}");
        assert_eq!(
            app_request(&app, peer, Method::GET, &page, &[]).await.0,
            StatusCode::OK
        );
        let bundle = format!("/apps/present-app/dist/app.js?t={unredeemed}");
        assert_eq!(
            app_request(&app, peer, Method::GET, &bundle, &[]).await.0,
            StatusCode::UNAUTHORIZED
        );
        let other = format!("/apps/other-app/?t={unredeemed}");
        assert_eq!(
            app_request(&app, peer, Method::GET, &other, &[]).await.0,
            StatusCode::UNAUTHORIZED,
            "a launch link opens the app it was minted for"
        );
        // Once redeemed, the same link opens nothing.
        assert!(redeem_app_launch("present-app", &unredeemed).is_some());
        assert_eq!(
            app_request(&app, peer, Method::GET, &page, &[]).await.0,
            StatusCode::UNAUTHORIZED
        );
        // Neither the cookie nor the launch token mints a link: that takes the secret.
        assert_eq!(
            app_request(
                &app,
                peer,
                Method::POST,
                "/apps/present-app/launch",
                &[("cookie", &cookie)]
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app_request(
                &app,
                peer,
                Method::POST,
                "/apps/present-app/launch",
                &[("x-secret-key", TEST_SECRET)]
            )
            .await
            .0,
            StatusCode::OK
        );
    }

    /// The secret still opens every app route, and a caller holding it may learn
    /// whether an app exists: it could list them anyway.
    #[tokio::test]
    async fn the_secret_still_opens_every_app_route() {
        let app = guarded_apps();
        let peer = [198, 51, 100, 44];
        let secret = [("x-secret-key", TEST_SECRET)];
        assert_eq!(
            app_request(&app, peer, Method::GET, "/apps/present-app/", &secret)
                .await
                .0,
            StatusCode::OK
        );
        let (status, body) =
            app_request(&app, peer, Method::GET, "/apps/missing-app/", &secret).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_FOUND, "no such app")
        );
    }

    /// A launch link rides a URL, and a URL outlives its use in browser history;
    /// so it opens once, for a few minutes, and only its own app. (Keeping it off
    /// command lines is the openers' job: single use cannot stop a reader who
    /// redeems it first.) The cookie value it redeems for is not in it, and is the
    /// same however many links are redeemed.
    #[test]
    fn a_launch_link_opens_once_and_only_until_it_expires() {
        let mut access = AppAccess::default();
        let t0 = Instant::now();
        let first = access.mint("ttl-app", t0);
        assert!(access.pending("ttl-app", &first, t0));
        assert!(!access.pending("other-app", &first, t0));
        assert_eq!(access.redeem("other-app", &first, t0), None);
        assert!(
            access.pending("ttl-app", &first, t0),
            "a token for another app is not spent by a page it does not open"
        );
        let cookie = access.redeem("ttl-app", &first, t0).expect("redeems once");
        assert_eq!(access.redeem("ttl-app", &first, t0), None, "and only once");
        assert!(!access.pending("ttl-app", &first, t0));
        assert!(access.cookie_matches("ttl-app", &cookie));
        assert!(!access.cookie_matches("ttl-app", &first));
        assert!(!access.cookie_matches("other-app", &cookie));

        let late = access.mint("ttl-app", t0);
        let deadline = t0 + APP_LAUNCH_TTL;
        assert!(access.pending("ttl-app", &late, deadline - Duration::from_millis(1)));
        assert!(!access.pending("ttl-app", &late, deadline));
        assert_eq!(access.redeem("ttl-app", &late, deadline), None);

        let again = access.mint("ttl-app", t0);
        assert_eq!(access.redeem("ttl-app", &again, t0), Some(cookie));
    }

    /// Unredeemed links are bounded, oldest out first, and expired ones are
    /// dropped as new ones are minted.
    #[test]
    fn unredeemed_launch_links_are_bounded() {
        let mut access = AppAccess::default();
        let t0 = Instant::now();
        let oldest = access.mint("bound-app", t0);
        for n in 1..=APP_LAUNCH_LIMIT {
            access.mint("bound-app", t0 + Duration::from_millis(n as u64));
        }
        assert!(access.launches.len() <= APP_LAUNCH_LIMIT);
        assert!(!access.pending("bound-app", &oldest, t0));
        access.mint("bound-app", t0 + APP_LAUNCH_TTL + Duration::from_secs(1));
        assert_eq!(access.launches.len(), 1, "expired links are dropped");
    }

    /// On a `biorouter serve` daemon the browser holding the served document's
    /// cookie was handed the daemon secret with it, so that cookie opens apps
    /// too; on any other daemon, or with another value, it opens nothing.
    #[test]
    fn a_serve_documents_cookie_opens_its_daemons_apps() {
        let page = is_app_browser_get(&Method::GET, "/apps/some-app/").unwrap();
        let operator = ServedOperator {
            browser_token: "served-document-token".into(),
            capability: biorouter::privacy::ProviderTier::Public,
        };
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "biorouter_session=served-document-token".parse().unwrap(),
        );
        assert!(app_access_granted_by(
            &page,
            None,
            &headers,
            Some(&operator)
        ));
        assert!(!app_access_granted_by(&page, None, &headers, None));
        headers.insert(
            axum::http::header::COOKIE,
            "biorouter_session=another-token".parse().unwrap(),
        );
        assert!(!app_access_granted_by(
            &page,
            None,
            &headers,
            Some(&operator)
        ));
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

#[cfg(test)]
mod computer_use_proof_tests {
    use super::*;

    #[test]
    fn scoped_proof_does_not_accept_browser_cookie_api_secret_or_general_header() {
        let digest: [u8; 32] =
            <sha2::Sha256 as sha2::Digest>::digest(b"human-held-computer-key").into();
        for header in ["Cookie", "X-Secret-Key", "X-User-Action"] {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(header, "human-held-computer-key".parse().unwrap());
            assert_eq!(
                computer_use_proof_with_digest(
                    &headers,
                    UserActionProof::NoKeyInstalled,
                    Some(&digest)
                ),
                UserActionProof::Unproven
            );
        }
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "X-Computer-Use-Key",
            "human-held-computer-key".parse().unwrap(),
        );
        assert_eq!(
            computer_use_proof_with_digest(
                &headers,
                UserActionProof::NoKeyInstalled,
                Some(&digest)
            ),
            UserActionProof::Proven
        );
        assert_eq!(
            computer_use_proof_with_digest(&headers, UserActionProof::NoKeyInstalled, None),
            UserActionProof::NoKeyInstalled
        );
        headers.insert("X-Computer-Use-Key", hex::encode(digest).parse().unwrap());
        assert_eq!(
            computer_use_proof_with_digest(
                &headers,
                UserActionProof::NoKeyInstalled,
                Some(&digest)
            ),
            UserActionProof::Unproven
        );
    }

    #[test]
    fn scoped_approval_attempt_window_expires_without_accepting_fresh_guesses() {
        let now = Instant::now();
        let mut attempts = vec![now; COMPUTER_USE_ATTEMPT_BUDGET];
        prune_computer_use_attempts(&mut attempts, now + Duration::from_secs(59));
        assert_eq!(attempts.len(), COMPUTER_USE_ATTEMPT_BUDGET);
        prune_computer_use_attempts(&mut attempts, now + COMPUTER_USE_ATTEMPT_WINDOW);
        assert!(attempts.is_empty());
    }

    #[test]
    fn existing_desktop_human_proof_still_authorizes_computer_use() {
        assert_eq!(
            computer_use_proof_with_digest(
                &axum::http::HeaderMap::new(),
                UserActionProof::Proven,
                None
            ),
            UserActionProof::Proven
        );
    }
}
