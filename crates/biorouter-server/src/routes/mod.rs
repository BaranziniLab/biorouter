/// Origins the daemon is willing to be driven by. It binds loopback, so this is
/// every origin it legitimately serves.
///
/// CORS does not govern WebSocket handshakes and browsers freely open
/// cross-origin WebSockets, so any endpoint that upgrades must check this itself.
///
/// Parsed rather than prefix-matched: `http://127.0.0.1:` is a prefix of
/// `http://127.0.0.1:8080.evil.com`.
pub fn is_local_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    if host != "localhost" && host != "127.0.0.1" {
        return false;
    }
    match port {
        None => true,
        Some(port) => !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
    }
}

/// Whether `origin` names the very origin this request was addressed to.
///
/// [`is_local_origin`] answers "is this loopback", which was the whole question
/// while the daemon only ever served a browser on the same machine. Since it can
/// serve its own interface, a browser may legitimately reach it at a LAN address
/// or a hostname — and the daemon cannot enumerate those. It may have bound
/// `0.0.0.0`, and the address the user typed is not knowable from the bind.
///
/// What *is* knowable is the `Host` the request carries. A same-origin page
/// always presents an `Origin` whose authority equals that `Host`, and a page on
/// any other origin cannot — the browser sets both, and neither is reachable
/// from script. So comparing the two is a precise same-origin test that needs no
/// configuration and no wildcard, and it holds for every address the interface
/// is ever reached at.
///
/// Both are compared whole. `http://evil.com` is not admitted by a `Host` of
/// `evil.com.attacker.net`, because this is an equality test rather than a
/// prefix one — the same trap [`is_local_origin`] documents.
pub fn origin_matches_host(origin: &str, host: Option<&str>) -> bool {
    let Some(host) = host else {
        // No `Host` to compare against. Refuse rather than guess: the caller
        // still has the loopback rule and the socket's token.
        return false;
    };
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(authority) = authority else {
        // `null`, `file://`, and anything else opaque. Callers that admit
        // `file://` do so by name; this is not the place for it.
        return false;
    };
    !authority.is_empty() && !host.is_empty() && authority.eq_ignore_ascii_case(host)
}

/// Compare secrets without an early return, so a caller cannot recover the key
/// one byte at a time by timing the response.
///
/// It lives here, beside `is_local_origin`, rather than in `auth.rs` with the
/// middleware that is its main caller, for the reason that already put
/// `is_local_origin` here: `auth` is a **lib-only** module (`main.rs`
/// re-declares the module tree and pulls `check_token` from the lib —
/// `commands::agent`'s `use biorouter_server::auth::check_token`), while
/// `src/routes/` is compiled into the `biorouterd` binary as well, so nothing
/// under `src/routes/` can name `crate::auth`. `routes::workspace`'s socket gate
/// checks this very secret and has to use the middleware's own comparator rather
/// than a second copy of it. `auth` re-exports this, so `check_token` and
/// `auth::tests::secret_compare_is_exact` are unchanged.
pub(crate) fn secret_matches(candidate: &str, expected: &str) -> bool {
    let (a, b) = (candidate.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The body of `signature`'s function in `src`: everything after the signature
/// up to the next line that is a bare `}` at column 0 — which is
/// `awk '/sig/,/^}/'` in Rust.
///
/// It is the ONE span a structural assertion about a handler is allowed to read.
/// Several route facts in this crate cannot be asserted behaviourally —
/// `AppState::new()` opens the developer's REAL session database — and the
/// failure mode of a whole-file `contains` is that it finds the fact in the
/// handler *next door*. A second copy of this extractor is how two scans start
/// disagreeing about where a function ends, so there is one, and every test
/// using it carries a negative control proving it did not over-read.
///
/// It lives here rather than in `auth.rs` beside the first scan that needed it,
/// for the reason recorded on [`secret_matches`]: `auth` is a **lib-only**
/// module and `src/routes/` is compiled into the `biorouterd` binary as well, so
/// nothing under `src/routes/` can name `crate::auth`. A scan in a route module
/// that reached for it there compiles for the lib and breaks the binary.
///
/// `split_once` rather than byte-index slicing so the whole thing is panic-free
/// by construction (`clippy::string_slice`).
///
/// `#[cfg(test)]`, so it is absent from every shipped binary.
#[cfg(test)]
pub(crate) fn body_of<'a>(src: &'a str, signature: &str) -> &'a str {
    let (_, from_signature) = src
        .split_once(signature)
        .unwrap_or_else(|| panic!("`{signature}` is not in this file"));
    from_signature
        .split_once("\n}\n")
        .map_or(from_signature, |(body, _)| body)
}

#[cfg(test)]
mod origin_tests {
    use super::is_local_origin;

    #[test]
    fn accepts_loopback_origins() {
        assert!(is_local_origin("http://localhost"));
        assert!(is_local_origin("http://localhost:3000"));
        assert!(is_local_origin("http://127.0.0.1"));
        assert!(is_local_origin("http://127.0.0.1:8080"));
    }

    #[test]
    fn rejects_everything_else() {
        assert!(!is_local_origin("https://evil.com"));
        assert!(!is_local_origin("null"));
        assert!(!is_local_origin(""));
        // A suffix must not ride in on the prefix match.
        assert!(!is_local_origin("http://localhost.evil.com"));
        assert!(!is_local_origin("http://127.0.0.1.evil.com"));
        assert!(!is_local_origin("http://127.0.0.1:8080.evil.com"));
        assert!(!is_local_origin("http://127.0.0.1:"));
        // https to loopback is not an origin this server serves.
        assert!(!is_local_origin("https://127.0.0.1:8080"));
    }
}

pub mod action_required;
pub mod active_work;
pub mod agent;
pub mod apps;
pub mod audio;
pub mod catalog;
pub mod coding_agents;
pub mod config_management;
pub mod errors;
pub mod knowledge;
pub mod llamacpp;
pub mod memory;
pub mod reply;
pub mod reset;
pub mod schedule;
pub mod session;
pub mod session_events;
pub mod session_reach;
pub mod setup;
pub mod shell;
pub mod skills;
pub mod status;
pub mod tool_bridge;
pub mod tunnel;
pub mod usage;
pub mod utils;
pub mod web_ui;
pub mod workflow;
pub mod workflow_utils;
pub mod workspace;

use std::sync::Arc;

use axum::Router;

// Function to configure all routes
pub fn configure(state: Arc<crate::state::AppState>, secret_key: String) -> Router {
    Router::new()
        .merge(status::routes(state.clone()))
        .merge(active_work::routes(state.clone()))
        .merge(reply::routes(state.clone()))
        .merge(reset::routes(state.clone()))
        .merge(action_required::routes(state.clone()))
        .merge(catalog::routes(state.clone()))
        .merge(agent::routes(state.clone()))
        .merge(apps::routes(state.clone()))
        .merge(audio::routes(state.clone()))
        .merge(config_management::routes(state.clone()))
        .merge(workflow::routes(state.clone()))
        .merge(session::routes(state.clone()))
        .merge(usage::routes(state.clone()))
        .merge(schedule::routes(state.clone()))
        .merge(setup::routes(state.clone()))
        .merge(coding_agents::routes(state.clone()))
        .merge(llamacpp::routes(state.clone()))
        .merge(memory::routes(state.clone()))
        // The interface's own endpoints -- the filesystem browser, settings,
        // extension installation. They stood in front of the daemon with no
        // authentication at all; inside `configure` they take `check_token`
        // like everything else. See `routes::shell`.
        .merge(shell::routes(state.clone()))
        .merge(skills::routes(state.clone()))
        // No secret-key gate: the path carries a single-turn capability nonce, and
        // Codex sends no Authorization header at all, so a header scheme would
        // authenticate one client and not the other. See the module header.
        .merge(tool_bridge::routes())
        .merge(tunnel::routes(state.clone()))
        .merge(workspace::routes(state.clone(), secret_key))
        .merge(session_events::routes(state.clone()))
        .nest(
            "/knowledge",
            // Issue #56 Task 58 / #47. Both methods on `/knowledge/active`
            // address a named chat's knowledge-base selection, so reads and
            // writes take the same session-reach gate. It is layered rather
            // than called from the handlers because
            // this router is state-typed on `Arc<KnowledgeService>` so that it
            // can be tested without an `AppState` — see
            // `session_reach::gate_knowledge_active`, which explains the choice
            // and buffers the body only for the one route it gates.
            knowledge::router(state.knowledge_service.clone()).layer(
                axum::middleware::from_fn_with_state(
                    state.clone(),
                    session_reach::gate_knowledge_active,
                ),
            ),
        )
}
