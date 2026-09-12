//! Serving the web interface to a browser.
//!
//! The daemon serves the built single-page application itself, on its own
//! origin, rather than sitting behind a separate binary that proxied it. That
//! one property is what makes browser mode work at all: `/ui/workspace` and
//! `/apps/{id}/agent` are daemon routes, so reached same-origin they need no
//! proxy support, and the reverse proxy that used to sit here could not carry a
//! WebSocket upgrade at all. See `docs/deployment/serve-architecture.md`.
//!
//! # Where this sits relative to the secret-key middleware
//!
//! Deliberately outside it. [`attach`] is called on a router that has *already*
//! been wrapped in `check_token`, and `Router::layer` wraps only what was added
//! before it — so the routes added here are structurally beyond that middleware
//! rather than exempted from it by name. That distinction matters: the
//! alternative was to add path patterns to [`crate::auth::is_unauthenticated_path`],
//! and every hashed asset filename would have needed a prefix match there. That
//! module's own comments explain why a `starts_with` exemption is dangerous —
//! it exempts every future route under the prefix. Nothing in this file can
//! widen the API's authentication, because it never touches it.
//!
//! # What authenticates a browser
//!
//! A browser's first request cannot carry a header, so the `X-Secret-Key`
//! scheme every other client uses cannot gate the initial document. The
//! exchange instead is:
//!
//! 1. `biorouter serve` mints a browser token for the launch and prints it in
//!    the URL it shows the user.
//! 2. `GET /?t=<token>` validates it, sets a session cookie, and redirects to
//!    `/` so the token leaves the address bar.
//! 3. `GET /` with that cookie returns the shell, with the daemon's secret
//!    injected into it.
//! 4. From then on the application presents `X-Secret-Key` exactly as the
//!    desktop renderer does, and every API route is guarded exactly as before.
//!
//! Step 2 does not consume the token. It is honoured as often as it is
//! presented until the daemon stops, and the cookie's value is the token
//! itself, so single use would need a daemon-minted session and would break a
//! second browser, a bookmark, and a browser that has dropped its cookie.
//! Decision SD-9 in `docs/deployment/serve-decisions.md` has the reasoning.
//!
//! **The cookie gates the document, and authenticates nothing else.** It is not
//! accepted as authentication on any API route. Accepting it there would make
//! every API route reachable by a credential the browser attaches automatically,
//! which is a cross-site request forgery surface the header scheme does not
//! have. Keeping the cookie's authority to one request is why `check_token`
//! needed no change.
//!
//! It has one other reader, and it is a narrowing rather than an admission: an
//! API request that already passed `check_token` and ALSO carries this cookie
//! came from the document this daemon served, so `auth::served_operator_capability`
//! gives it the operator's configured tier on the listing and knowledge-base
//! gates (`routes::session_reach`). A request holding only the secret is a
//! public caller there. `SameSite=Strict` keeps the cookie off every cross-site
//! request, and a forged request still needs the secret, so no CSRF surface
//! appears. See `docs/deployment/serve-decisions.md` SD-10.
//!
//! # Why there is no brute-force throttle here
//!
//! The token is 32 bytes from the system generator. Guessing it is not a
//! reachable attack, and a throttle would have to be keyed on the peer address —
//! which is exactly what makes `check_token`'s throttle a hazard on this path:
//! it refuses after twenty failures in a minute, so a mistyped URL would lock
//! the user out of their own machine, and behind network address translation it
//! would lock out their colleagues too. Being outside that middleware is what
//! avoids it; adding an equivalent here would reintroduce it.

use std::path::{Path, PathBuf};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use tower_http::services::ServeDir;

/// The name of the session cookie the token exchange sets.
const SESSION_COOKIE: &str = "biorouter_session";

/// Everything the serving path needs, resolved once at startup.
#[derive(Clone)]
pub struct WebUi {
    /// The application shell, with the runtime configuration already spliced
    /// in. Precomputed because it never varies between requests; the per-request
    /// work is only deciding whether the caller may have it.
    index_html: String,
    /// The browser token, hex-encoded. `None` disables the gate entirely, which
    /// is only correct for a loopback bind the launcher chose not to token.
    browser_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    /// The browser token, as printed in the URL.
    t: Option<String>,
}

impl WebUi {
    /// Read the shell from `web_dir` and splice in the runtime configuration
    /// the renderer reads.
    ///
    /// `secret_key` is the daemon's own secret. It is placed in the document,
    /// which is why the document is gated: anyone who can read the shell can
    /// drive the API, and that is the intended equivalence — a browser that has
    /// authenticated is as capable as the desktop application.
    pub fn new(
        web_dir: &Path,
        secret_key: &str,
        browser_token: Option<String>,
    ) -> std::io::Result<Self> {
        let raw = std::fs::read_to_string(web_dir.join("index.html"))?;
        Ok(Self {
            index_html: inject_runtime_config(&raw, secret_key),
            browser_token,
        })
    }

    fn token_matches(&self, presented: Option<&str>) -> bool {
        match &self.browser_token {
            // No token configured: the launcher bound loopback and chose not to
            // require one. `biorouter serve` refuses this combination for any
            // non-loopback bind, so reaching here means the only callers are on
            // this machine.
            None => true,
            Some(expected) => presented.is_some_and(|p| super::secret_matches(p, expected)),
        }
    }
}

/// Splice the runtime configuration the renderer reads into the shell.
///
/// `ui/desktop/src/renderer.tsx` looks for `window.__BIOROUTER_HEADLESS_CONFIG__`
/// and derives its API base, its interface-endpoint base and its secret from it.
/// Everything downstream of that global — the shim that stands in for
/// `window.electron`, the API client, the secret handling — already exists and
/// is untouched by this change.
///
/// `apiBaseUrl` is the empty-meaning origin rather than an absolute URL on
/// purpose. The renderer falls back to `http://127.0.0.1:3000` when it sees a
/// loopback hostname and no configured base, which is the wrong port for a
/// daemon on an ephemeral one, and plainly wrong for a browser on another
/// machine. Naming the origin explicitly removes that guess.
fn inject_runtime_config(raw: &str, secret_key: &str) -> String {
    // `apiBaseUrl` is deliberately ABSENT rather than empty. The renderer treats
    // the mere presence of this global as "the daemon served me", and then uses
    // its own origin -- which is the only correct answer, and the only one the
    // daemon could not supply: it does not know the address the browser used.
    // An empty string here would be falsy and fall through to the renderer's
    // `http://127.0.0.1:3000` guess, which names a port the daemon need not be
    // on and is meaningless from another machine.
    let config = serde_json::json!({
        "headlessBaseUrl": "/headless",
        "secretKey": secret_key,
    });
    // `</script>` inside a JSON string would close the tag early. The daemon
    // secret is hex and the rest is fixed, so this cannot currently trigger --
    // it is here so that it stays true if a future field carries user text.
    let json = serde_json::to_string(&config)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</", "<\\/");
    let snippet = format!(
        "<script>window.__BIOROUTER_HEADLESS_CONFIG__={json};\
         window.addEventListener('vite:preloadError',function(e){{e.preventDefault();}});</script>"
    );
    match raw.find("</head>") {
        Some(at) => {
            // `split_at` rather than two range indexes: this workspace's clippy
            // configuration denies `string_slice`, and `find` already
            // guarantees the offset is a character boundary.
            let (before, after) = raw.split_at(at);
            let mut out = String::with_capacity(raw.len() + snippet.len());
            out.push_str(before);
            out.push_str(&snippet);
            out.push_str(after);
            out
        }
        // A shell with no </head> is not something Vite produces; serving it
        // unmodified would strand the renderer with no configuration, so put
        // the snippet first where it will still run before the bundle.
        None => format!("{snippet}{raw}"),
    }
}

/// Read one cookie out of a `Cookie` header.
///
/// Hand-rolled rather than pulling in a cookie crate: one name is read and one
/// is written, and the parsing rule -- split on `;`, then on the first `=` --
/// is the whole of what is needed.
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim())
}

/// The session cookie the token exchange set, if the request carries one.
///
/// The one reader of [`SESSION_COOKIE`]: the shell below asks it whether to
/// serve the document, and `auth::served_operator_capability` asks it whether a
/// request came from that document — which earns a serve daemon's own interface
/// the operator's tier on the listing and knowledge-base gates, and nothing
/// else. It is never accepted as authentication on an API route: `check_token`
/// still demands `X-Secret-Key`, so the cookie can only narrow a caller that
/// already holds the secret, never admit one that does not.
pub(crate) fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    cookie_value(headers, SESSION_COOKIE)
}

/// The application shell, and the token-for-cookie exchange that gates it.
///
/// This handler also serves every unmatched path, so a deep link into the
/// application (`/sessions/<id>`) returns the shell and the client-side router
/// takes it from there.
async fn index(
    State(ui): State<WebUi>,
    headers: HeaderMap,
    Query(query): Query<IndexQuery>,
) -> Response<Body> {
    // The token in the URL is exchanged for a cookie and then redirected away,
    // so it does not linger in the address bar, in browser history, or in the
    // `Referer` of anything the page later loads.
    if let Some(token) = query.t.as_deref() {
        if ui.token_matches(Some(token)) {
            return Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(header::LOCATION, "/")
                .header(
                    header::SET_COOKIE,
                    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict"),
                )
                .body(Body::empty())
                .expect("valid redirect");
        }
        return unauthorized();
    }

    if !ui.token_matches(session_cookie(&headers)) {
        return unauthorized();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        // The shell carries the daemon secret, so it must never be cached by
        // anything between here and the browser.
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(ui.index_html.clone()))
        .expect("valid html response")
}

/// Refuse without saying anything a caller does not already know.
fn unauthorized() -> Response<Body> {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><meta charset=utf-8><title>Biorouter</title>\
         <body style=\"font:15px system-ui;margin:3rem auto;max-width:32rem\">\
         <h1 style=\"font-size:1.2rem\">This link needs its access token</h1>\
         <p>Open the full address <code>biorouter serve</code> printed, including \
         the <code>?t=</code> part. It is shown once per launch.</p>",
    )
        .into_response()
}

/// Add the web interface to an already-layered API router.
///
/// Call this *after* `check_token` has been applied, so the shell and the
/// static bundle sit outside it. See the module documentation.
pub fn attach(app: Router, web_dir: PathBuf, ui: WebUi) -> Router {
    let shell = Router::new().route("/", get(index)).with_state(ui.clone());

    // Real files -- the hashed `/assets/*` and the icons -- come off disk.
    // Directory auto-indexing is off so that `/` falls through to the gated
    // shell above rather than to the raw, un-injected `index.html` on disk.
    //
    // Static assets are not gated. They are the application bundle: they carry
    // no secret, and the document that does carry one is the thing behind the
    // token. Gating them would buy nothing and would put the 8 MB of JavaScript
    // behind a cookie check on every request.
    let assets = ServeDir::new(web_dir)
        .append_index_html_on_directories(false)
        .fallback(get(index).with_state(ui));

    app.merge(shell).fallback_service(assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui_with(token: Option<&str>) -> WebUi {
        WebUi {
            index_html: "<html><head></head><body>shell</body></html>".to_string(),
            browser_token: token.map(str::to_string),
        }
    }

    #[test]
    fn the_runtime_config_names_the_origin_rather_than_leaving_the_renderer_to_guess() {
        let out = inject_runtime_config("<html><head></head><body></body></html>", "deadbeef");
        assert!(out.contains("__BIOROUTER_HEADLESS_CONFIG__"));
        // Absent, not empty: an empty string is falsy in the renderer and would
        // fall through to its hardcoded loopback guess.
        assert!(
            !out.contains("apiBaseUrl"),
            "apiBaseUrl must be absent so the renderer uses its own origin"
        );
        assert!(out.contains("\"secretKey\":\"deadbeef\""));
        // Injected inside <head>, so it runs before the module bundle that
        // reads it.
        let cfg_at = out.find("__BIOROUTER_HEADLESS_CONFIG__").unwrap();
        let head_close = out.find("</head>").unwrap();
        assert!(cfg_at < head_close, "config must precede </head>");
    }

    /// A shell Vite would not produce, but which must not silently lose its
    /// configuration if it ever appears.
    #[test]
    fn a_shell_without_a_head_still_gets_its_configuration() {
        let out = inject_runtime_config("<body>no head</body>", "abc");
        assert!(out.contains("__BIOROUTER_HEADLESS_CONFIG__"));
        assert!(out.starts_with("<script>"));
    }

    #[test]
    fn a_configured_token_is_required_and_compared_whole() {
        let ui = ui_with(Some("0123456789abcdef"));
        assert!(ui.token_matches(Some("0123456789abcdef")));
        assert!(
            !ui.token_matches(Some("0123456789abcde")),
            "prefix must not pass"
        );
        assert!(!ui.token_matches(Some("")), "empty must not pass");
        assert!(!ui.token_matches(None), "absent must not pass");
    }

    /// The launcher refuses a non-loopback bind without a token, so "no token
    /// configured" means "loopback, and the user declined one".
    #[test]
    fn no_configured_token_admits_everyone() {
        let ui = ui_with(None);
        assert!(ui.token_matches(None));
        assert!(ui.token_matches(Some("anything")));
    }

    #[test]
    fn a_cookie_is_read_out_of_a_header_carrying_several() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; biorouter_session=tok; third=3".parse().unwrap(),
        );
        assert_eq!(cookie_value(&headers, SESSION_COOKIE), Some("tok"));
        assert_eq!(cookie_value(&headers, "absent"), None);
    }

    #[test]
    fn a_cookie_name_that_merely_ends_with_the_real_one_is_not_it() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "evil_biorouter_session=tok".parse().unwrap(),
        );
        assert_eq!(cookie_value(&headers, SESSION_COOKIE), None);
    }

    /// The daemon secret rides in the document. A cache anywhere between the
    /// daemon and the browser holding it would hand it to the next caller.
    #[tokio::test]
    async fn the_shell_is_never_cached() {
        let ui = ui_with(None);
        let res = index(State(ui), HeaderMap::new(), Query(IndexQuery { t: None })).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }

    #[tokio::test]
    async fn the_token_is_exchanged_for_a_cookie_and_redirected_out_of_the_address_bar() {
        let ui = ui_with(Some("tok"));
        let res = index(
            State(ui),
            HeaderMap::new(),
            Query(IndexQuery {
                t: Some("tok".into()),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get(header::LOCATION).unwrap(), "/");
        let cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("biorouter_session=tok"));
        // Both flags are load-bearing: HttpOnly keeps page script from reading
        // it, and SameSite=Strict is what makes the cookie useless to a
        // cross-site request -- the reason it is safe for the cookie to exist
        // at all.
        assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly");
        assert!(
            cookie.contains("SameSite=Strict"),
            "cookie must be SameSite=Strict"
        );
    }

    /// Would pass trivially against an implementation with no gate at all, so
    /// it is paired with the positive case above.
    #[tokio::test]
    async fn a_wrong_token_gets_the_shell_from_nobody() {
        let ui = ui_with(Some("tok"));
        let res = index(
            State(ui.clone()),
            HeaderMap::new(),
            Query(IndexQuery {
                t: Some("wrong".into()),
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // And with no credential at all.
        let res = index(State(ui), HeaderMap::new(), Query(IndexQuery { t: None })).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// The exchange does not spend the token (SD-9 in
    /// `docs/deployment/serve-decisions.md`): a second browser, a bookmark of a
    /// `--token` address, and a browser that has dropped its cookie all present
    /// it again. Making it single-use is a decision to revisit there, not a fix
    /// to make here.
    #[tokio::test]
    async fn the_token_is_not_consumed_by_the_exchange() {
        let ui = ui_with(Some("tok"));
        for attempt in 1..=3 {
            let res = index(
                State(ui.clone()),
                HeaderMap::new(),
                Query(IndexQuery {
                    t: Some("tok".into()),
                }),
            )
            .await;
            assert_eq!(
                res.status(),
                StatusCode::SEE_OTHER,
                "redemption {attempt} must succeed like the first"
            );
        }
    }

    #[tokio::test]
    async fn a_valid_cookie_is_enough_on_a_later_request() {
        let ui = ui_with(Some("tok"));
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "biorouter_session=tok".parse().unwrap());
        let res = index(State(ui), headers, Query(IndexQuery { t: None })).await;
        assert_eq!(res.status(), StatusCode::OK);
    }
}
