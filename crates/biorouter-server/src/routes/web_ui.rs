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
//! 2. `GET /?t=<token>` validates it, sets a session cookie, and answers a
//!    short page of this origin that moves the browser on to the interface's
//!    mount, so the token leaves the address bar ([`exchange_bounce`]).
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
//! **The cookie gates the document, and authenticates nothing else** but the
//! app documents described below. It is not accepted as authentication on any
//! API route. Accepting it there would make
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
//! One admission, and it is documents again rather than the API: the browser
//! surface of this daemon's Agent Drafter apps (`auth::is_app_browser_get`: an
//! app's page, bundle, assets and agent socket) accepts this cookie in place of
//! the per-app access cookie `POST /apps/{id}/launch` hands out (W2-HRD-1). A
//! browser holding it was handed the daemon secret with the shell, so this grants
//! nothing it could not already do, and it lets an app opened from the web
//! interface load without a launch link. Every management route under `/apps`
//! still takes the secret.
//!
//! # The shell shares its origin with code an agent wrote
//!
//! Those apps are served from this same origin, and the cookie's `Path` is the
//! interface's own (`/`, or the prefix `--public-url` declared), so it rides
//! along on any request an app's script makes here. An app could
//! `fetch('/')` or frame `/` and read the secret out of the shell. So the shell
//! is served only to a document navigation (`auth::is_document_navigation`),
//! cannot be framed, and its opener policy (`SHELL_CSP`) leaves an app that
//! opens it in a popup with a severed handle. These are defence in depth on one
//! origin; serving apps on an origin of their own is the follow-up that makes
//! the question go away (SD-14 in `docs/deployment/serve-decisions.md`).
//!
//! ⚠ Both need a secure context. At a plain-HTTP network address (`serve
//! --host 0.0.0.0` opened at its LAN address) a browser sends no Fetch Metadata
//! and ignores the opener policy, so an app's `fetch('/')` would be served the
//! shell. Both shell and app documents are therefore refused at such an
//! address (`auth::reached_over_a_secure_context`), including the token exchange.
//! An old app tab can remain open during an update, so refusing new app pages
//! alone would not protect the shell from code already running there.
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
    http::{header, HeaderMap, Response, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use tower_http::services::ServeDir;

/// The name of the session cookie the token exchange sets.
const SESSION_COOKIE: &str = "biorouter_session";

/// The name of the cookie that says a browser redeemed a valid token at the
/// address it is on, set beside the session cookie when that one is scoped to a
/// declared path ([`WebUi::wrong_address`]).
const EXCHANGED_COOKIE: &str = "biorouter_exchanged";

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
    /// The `Path` of the session cookie: `/`, or the path prefix a reverse proxy
    /// serves the interface under, from `biorouter serve --public-url`
    /// (`routes::PublicUrl`). Behind JupyterHub every user's server shares one
    /// hostname, and a cookie at `/` would carry this token to all of them.
    cookie_path: String,
    /// The address `--public-url` declared, the declaration `cookie_path` came
    /// from. Its scheme decides `Secure` behind a proxy that rewrites `Host`
    /// ([`secure_attribute`]), and a browser that redeemed its token somewhere
    /// else is pointed at it ([`WebUi::wrong_address`]).
    public: Option<super::PublicUrl>,
    /// The value of [`EXCHANGED_COOKIE`]: random per daemon run, so the cookie a
    /// previous daemon set does not count, and it carries nothing of the token.
    exchange_marker: String,
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
    ///
    /// `public_path` is the path of the address `biorouter serve --public-url`
    /// declared, `/` or a prefix with no trailing slash, and scopes the cookie.
    /// The rest of that declaration is read from where `commands::agent` read
    /// the path, [`super::declared_public_url`], because the path alone does not
    /// carry the scheme or the address to name.
    pub fn new(
        web_dir: &Path,
        secret_key: &str,
        browser_token: Option<String>,
        public_path: Option<&str>,
    ) -> std::io::Result<Self> {
        let raw = std::fs::read_to_string(web_dir.join("index.html"))?;
        Ok(Self {
            index_html: inject_runtime_config(&raw, secret_key),
            browser_token,
            cookie_path: public_path.unwrap_or("/").to_string(),
            public: super::declared_public_url()
                .filter(|declared| public_path == Some(declared.path()))
                .cloned(),
            exchange_marker: hex::encode(rand::random::<[u8; 16]>()),
        })
    }

    /// Does the request carry a session cookie holding the token? Any of them
    /// counts ([`session_cookies`]); each is compared in constant time.
    fn cookie_matches(&self, headers: &HeaderMap) -> bool {
        self.browser_token.is_none()
            || session_cookies(headers).any(|presented| self.token_matches(Some(presented)))
    }

    /// The `Set-Cookie` values a redeemed token answers with: the session
    /// cookie, scoped to the declared path, and, when that path is not `/`, the
    /// [`EXCHANGED_COOKIE`] marker with no `Path`, which the browser scopes to
    /// the address it is really on. The marker comes back on the next request
    /// whether or not that address is under the declared path; the session
    /// cookie comes back only when it is. That difference is the whole of
    /// [`WebUi::wrong_address`]'s evidence.
    fn exchange_cookies(&self, token: &str, secure: &str) -> Vec<String> {
        let mut cookies = vec![format!(
            "{SESSION_COOKIE}={token}; Path={}; HttpOnly; SameSite=Strict{secure}",
            self.cookie_path
        )];
        if self.cookie_path != "/" {
            cookies.push(format!(
                "{EXCHANGED_COOKIE}={}; Max-Age=300; HttpOnly; SameSite=Strict{secure}",
                self.exchange_marker
            ));
        }
        cookies
    }

    /// The declared address, when a request without a usable session cookie
    /// comes from a browser that redeemed a valid token at another address:
    /// the marker [`WebUi::exchange_cookies`] set came back and the session
    /// cookie did not, so the browser is outside the path the cookie is scoped
    /// to. The proxy strips that path before the daemon sees it, so the cookies
    /// are the only witness the daemon has.
    fn wrong_address(&self, headers: &HeaderMap) -> Option<&super::PublicUrl> {
        let public = self.public.as_ref()?;
        cookie_values(headers, EXCHANGED_COOKIE)
            .any(|presented| super::secret_matches(presented, &self.exchange_marker))
            .then_some(public)
    }

    fn token_matches(&self, presented: Option<&str>) -> bool {
        match &self.browser_token {
            // No token configured: the launcher bound loopback and chose not to
            // require one. `biorouter serve` refuses this combination for any
            // non-loopback bind, so reaching here means every caller is on this
            // machine: its other accounts, and a reverse proxy on it forwarding
            // whoever reaches the proxy. That is the operator's call to make,
            // and why `docs/deployment/browser-access.md` keeps the token on a
            // JupyterHub whose users share one machine.
            None => true,
            Some(expected) => presented.is_some_and(|p| super::secret_matches(p, expected)),
        }
    }
}

/// Splice the runtime configuration the renderer reads into the shell.
///
/// `ui/desktop/src/renderer.tsx` looks for `window.__BIOROUTER_HEADLESS_CONFIG__`,
/// takes its secret from it, and reads its presence as "the daemon served me",
/// which sends it to the shell's `<base href>` ([`with_base`]) for its API base
/// and its interface-endpoint base.
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
    // the address it is mounted at -- which is the only correct answer, and the
    // only one the daemon could not supply: it does not know the address the
    // browser used. An empty string here would be falsy and fall through to the
    // renderer's `http://127.0.0.1:3000` guess, which names a port the daemon
    // need not be on and is meaningless from another machine.
    //
    // `headlessBaseUrl` is absent for the same reason. It used to be the literal
    // `/headless`, which is right only at the root of a hostname; behind a
    // proxy that serves the interface under a path (issue #402) it sent every
    // interface request to the proxy's root. The renderer derives it from the
    // mount, the `<base href>` [`index`] writes into the shell.
    let config = serde_json::json!({
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

/// Every value a request carries for the cookie `name`, in the order sent.
///
/// Hand-rolled rather than pulling in a cookie crate: the parsing rule -- split
/// on `;`, then on the first `=` -- is the whole of what is needed.
///
/// ⚠ All of them, not the first. A browser sends every cookie whose `Path`
/// covers the request, the longest path first, and two cookies of one name
/// with different paths are both kept. After `biorouter serve --public-url
/// https://hub/user/a/proxy/9999/` set its cookie at that path, a later
/// `serve` without the flag sets a fresh one at `/` under the same name, and
/// the browser then sends the stale, longer-path value first. Reading only the
/// first refused every new link with "This link needs its access token".
fn cookie_values<'a>(headers: &'a HeaderMap, name: &'a str) -> impl Iterator<Item = &'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .filter(move |(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim())
}

/// The session cookies the request carries: every value of the cookie the
/// token exchange sets. A caller asks whether ANY of them is the token, never
/// reads one (see [`cookie_values`]).
///
/// The one reader of [`SESSION_COOKIE`]: the shell below asks it whether to
/// serve the document, and `auth::served_operator_capability` asks it whether a
/// request came from that document — which earns a serve daemon's own interface
/// the operator's tier on the listing and knowledge-base gates; and
/// `auth::check_token` asks it whether to admit a GET of an app's browser
/// surface (W2-HRD-1), a document like this one. It is never accepted as
/// authentication on an API route: `check_token` still demands `X-Secret-Key`
/// there, so the cookie can only narrow a caller that already holds the secret,
/// never admit one that does not.
pub(crate) fn session_cookies(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    cookie_values(headers, SESSION_COOKIE)
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
    uri: Uri,
) -> Response<Body> {
    // The shell carries the daemon secret and is the same origin as every
    // Agent Drafter app this daemon serves, whose code an agent wrote. Its
    // cookie (`Path=/`) rides along on any same-origin request, so before this
    // an app's script could `fetch('/')`, or frame `/`, and read the secret out
    // of the document. Fetch Metadata tells that apart from opening the shell,
    // and script cannot forge it; a client that sends none is not a browser
    // running anyone's script and is answered as before. See `SHELL_CSP` for
    // the other half, and `docs/deployment/serve-decisions.md` SD-14.
    // An old app tab can remain open across an update. Refusing new app pages
    // alone therefore cannot keep the secret out of an HTTP LAN fetch.
    if !biorouter_server::auth::reached_over_a_secure_context(&headers, &uri) {
        return insecure_address();
    }
    if !biorouter_server::auth::is_document_navigation(&headers) {
        return not_a_navigation();
    }

    // Where the interface is mounted, relative to where this request landed.
    // See [`mount_href`].
    let mount = mount_href(uri.path());

    // The token in the URL is exchanged for a cookie and the browser is moved
    // on, so it does not linger in the address bar, in the tab's history, or in
    // the `Referer` of anything the page later loads.
    if let Some(token) = query.t.as_deref() {
        if ui.token_matches(Some(token)) {
            // The cookie holds the CONFIGURED token, never the text the URL
            // presented. With none configured every `?t=` matches, and copying
            // it would let a `;` in it add cookie attributes, and a CR or LF
            // fail the response; such a daemon checks no cookie, so it sets none
            // and only moves the browser on.
            let cookies = match ui.browser_token.as_deref() {
                Some(expected) => {
                    let secure = secure_attribute(&headers, &uri, ui.public.as_ref());
                    ui.exchange_cookies(expected, secure)
                }
                None => Vec::new(),
            };
            return exchange_bounce(&cookies, &mount);
        }
        return unauthorized();
    }

    if !ui.cookie_matches(&headers) {
        return match ui.wrong_address(&headers) {
            Some(public) => wrong_address(public),
            None => unauthorized(),
        };
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        // The shell carries the daemon secret, so it must never be cached by
        // anything between here and the browser.
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_SECURITY_POLICY, SHELL_CSP)
        .header(header::X_FRAME_OPTIONS, "DENY")
        .header("cross-origin-opener-policy", "same-origin")
        .body(Body::from(with_base(&ui.index_html, &mount)))
        .expect("valid html response")
}

/// The interface's mount, as a URL relative to the path a request was sent to:
/// `./` for `/` (or any path one segment deep), `../` one directory further
/// down, and so on.
///
/// The bundle is built with a relative base (`npm run build:web` passes `--base
/// ./`), so every URL it holds -- the entry script and stylesheet in the shell,
/// the code-split chunks, the fonts its stylesheets name -- resolves against
/// the document, and the renderer reads its API base off the same document
/// (`renderer.tsx`). Naming the mount in a `<base href>` is what lets one
/// bundle work wherever it is opened:
///
/// - at the root of a hostname, which is what it always did;
/// - under a path prefix a reverse proxy strips before forwarding (issue #402:
///   JupyterHub's `/user/<name>/proxy/<port>/`, nginx's `location /p/ {
///   proxy_pass http://127.0.0.1:8765/; }`). The daemon never sees the prefix,
///   and does not need to: the part of the path below the mount is the same on
///   both sides of the proxy, so the distance back up to the mount is too;
/// - on a path that is not the mount at all, which falls through to the shell.
///   A relative bundle there would ask for `/x/assets/...` and be answered
///   with this page as a script, which is why the bundle used to have to be
///   built with an absolute base, and why this cannot be left to the document's
///   own URL.
///
/// It names only `.` and `..` segments, so nothing from the request reaches the
/// page, and it always resolves to this origin, which the bundle's own content
/// policy (`base-uri 'self'`, in the `<meta>` of `ui/desktop/index.html`)
/// accepts.
fn mount_href(path: &str) -> String {
    match path.matches('/').count().saturating_sub(1) {
        0 => "./".to_string(),
        depth => "../".repeat(depth),
    }
}

/// The shell with `<base href="{mount}">` as the first element of its head, so
/// it precedes every URL the head holds.
fn with_base(shell: &str, mount: &str) -> String {
    let base = format!("<base href=\"{mount}\">");
    match shell.find("<head>") {
        Some(at) => {
            // `split_at`, as in `inject_runtime_config`, because this workspace
            // denies `string_slice`; `find` gives a character boundary.
            let (before, after) = shell.split_at(at + "<head>".len());
            let mut out = String::with_capacity(shell.len() + base.len());
            out.push_str(before);
            out.push_str(&base);
            out.push_str(after);
            out
        }
        // Not something Vite produces; `inject_runtime_config` already put its
        // own snippet first in that case, and this goes before it.
        None => format!("{base}{shell}"),
    }
}

/// The policy the shell is served under. Only `frame-ancestors`: the shell's
/// own scripts and styles are whatever the build emitted, and this header is
/// not the place to restate them.
///
/// `frame-ancestors 'none'` (with `X-Frame-Options: DENY` for older engines)
/// keeps any page, an app of this daemon included, from framing the shell and
/// reading the secret through the frame's DOM. `Cross-Origin-Opener-Policy:
/// same-origin`, set beside it, puts the shell in its own browsing-context
/// group whenever it is opened by, or opens, a page without the same policy,
/// which every app page is, so neither side keeps a scriptable handle on the
/// other. The interface still opens an app in a new tab: the tab is opened
/// blank, sent to the app, and cut off from the shell (`openAppInThisBrowser`).
const SHELL_CSP: &str = "frame-ancestors 'none'";

fn insecure_address() -> Response<Body> {
    (
        StatusCode::FORBIDDEN,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        "Open Biorouter through localhost or HTTPS. Use an SSH tunnel, or an HTTPS reverse proxy that passes the browser's Sec-Fetch-* headers through, and a current browser that sends them.",
    )
        .into_response()
}

/// The answer to a request for the shell that is not a document navigation: a
/// `fetch`, an `<iframe>`, an `<object>`, a `<script src>`. Carries nothing.
fn not_a_navigation() -> Response<Body> {
    (
        StatusCode::FORBIDDEN,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        "Open Biorouter by visiting this address in the browser.",
    )
        .into_response()
}

/// The answer to a valid token: a page of this origin that sets the session
/// cookie and moves the browser on to `/`, replacing the tokenised address.
///
/// ⚠ A `200` page that navigates, not a `303`. The cookie is `SameSite=Strict`,
/// and a navigation another site started stays cross-site through its
/// redirects, so a `303` lands on the shell without the cookie it just set: the
/// person sees "This link needs its access token" with the right link in hand.
/// `biorouter serve --open` starts exactly such a navigation. The address
/// carries a token that works until the daemon stops, so it never goes on an
/// opener's command line, where every account on the machine can read it; the
/// opener is handed a `file:` page only this account can read, and a `file:`
/// page is another site. The navigation this page starts is same-site, so the
/// cookie goes with it. The same rule, for the same reason, as an app's launch
/// link (`routes::apps::launch_bounce`). A meta refresh rather than a script,
/// because the page's policy allows no script, and no token in the page: the
/// cookie carries it, and the address the page moves to is the mount
/// ([`mount_href`]), relative, so the browser stays under a proxy's path prefix.
fn exchange_bounce(cookies: &[String], mount: &str) -> Response<Body> {
    let mut bounce = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; \
             form-action 'none'; frame-ancestors 'none'",
        );
    for cookie in cookies {
        bounce = bounce.header(header::SET_COOKIE, cookie);
    }
    bounce
        .body(Body::from(exchange_bounce_html(mount)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `"; Secure"` for a cookie set in answer to a request the browser sent to an
/// `https` address, as `auth::browser_used_https` decides it from the request
/// and the address `--public-url` declared; nothing otherwise. Never on a plain
/// `http` address, loopback included: the browser would then keep the cookie
/// off every request there, and `biorouter serve` on `http://127.0.0.1` would
/// refuse its own link. Shared with an app's launch link (`routes::apps`).
pub(crate) fn secure_attribute(
    headers: &HeaderMap,
    uri: &Uri,
    public: Option<&super::PublicUrl>,
) -> &'static str {
    let declared = public.map(|public| public.origin().to_string());
    if biorouter_server::auth::browser_used_https(headers, uri, declared.as_deref()) {
        "; Secure"
    } else {
        ""
    }
}

/// `mount` is a [`mount_href`], only `.` and `..` segments, so it needs no
/// escaping in an attribute.
fn exchange_bounce_html(mount: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8>\
         <meta http-equiv=\"refresh\" content=\"0;url={mount}\">\
         <title>Opening Biorouter</title>\
         <body style=\"font:15px system-ui;margin:3rem auto;max-width:32rem\">\
         <p>Opening Biorouter. If nothing happens, <a href=\"{mount}\">continue here</a>.</p>"
    )
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

/// The refusal for a browser that redeemed a valid token at an address other
/// than the one `--public-url` declared ([`WebUi::wrong_address`]): the
/// session cookie is scoped to the declared path, so the token worked and the
/// cookie never came back. Names the declared address, never the token. The
/// address is the operator's, and `ps` already shows it to every account on
/// the machine.
fn wrong_address(public: &super::PublicUrl) -> Response<Body> {
    let address = match public.path() {
        "/" => format!("{}/", public.origin()),
        path => format!("{}{path}/", public.origin()),
    };
    let address = html_text(&address);
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        format!(
            "<!doctype html><meta charset=utf-8><title>Biorouter</title>\
             <body style=\"font:15px system-ui;margin:3rem auto;max-width:32rem\">\
             <h1 style=\"font-size:1.2rem\">This is not the address Biorouter was started with</h1>\
             <p>The access token was accepted, but Biorouter was started for \
             <code>{address}</code>, and its access cookie works only there.</p>\
             <p>Open that address with the <code>?t=</code> part \
             <code>biorouter serve</code> printed. To use this address instead, \
             restart <code>biorouter serve</code> with <code>--public-url</code> set to it.</p>"
        ),
    )
        .into_response()
}

/// `text` safe to place in HTML text or a quoted attribute.
fn html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
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
            cookie_path: "/".to_string(),
            public: None,
            exchange_marker: "marker".to_string(),
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
        // So must the interface endpoints' base: the literal `/headless` it
        // used to be is wrong under a proxy's path prefix (issue #402).
        assert!(!out.contains("headlessBaseUrl"), "{out}");
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
        assert_eq!(
            cookie_values(&headers, SESSION_COOKIE).collect::<Vec<_>>(),
            ["tok"]
        );
        assert_eq!(cookie_values(&headers, "absent").next(), None);
    }

    #[test]
    fn a_cookie_name_that_merely_ends_with_the_real_one_is_not_it() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "evil_biorouter_session=tok".parse().unwrap(),
        );
        assert_eq!(cookie_values(&headers, SESSION_COOKIE).next(), None);
    }

    /// The daemon secret rides in the document. A cache anywhere between the
    /// daemon and the browser holding it would hand it to the next caller.
    #[tokio::test]
    async fn the_shell_is_never_cached() {
        let ui = ui_with(None);
        let res = index(
            State(ui),
            HeaderMap::new(),
            Query(IndexQuery { t: None }),
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }

    /// The exchange sets the cookie and moves the browser on to `/` with a page
    /// of this origin, not a redirect: under `SameSite=Strict` a redirect keeps
    /// the cookie off a navigation that `biorouter serve --open`'s `file:` page
    /// started (see [`exchange_bounce`]).
    #[tokio::test]
    async fn the_token_is_exchanged_for_a_cookie_and_moved_out_of_the_address_bar() {
        let ui = ui_with(Some("tok"));
        let res = index(
            State(ui),
            HeaderMap::new(),
            Query(IndexQuery {
                t: Some("tok".into()),
            }),
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().get(header::LOCATION).is_none());
        let header_value = |name: header::HeaderName| {
            res.headers()
                .get(&name)
                .unwrap_or_else(|| panic!("no {name} header"))
                .to_str()
                .unwrap()
                .to_string()
        };
        let cookie = header_value(header::SET_COOKIE);
        assert!(cookie.contains("biorouter_session=tok"));
        assert!(cookie.contains("Path=/;"), "{cookie}");
        // Both flags are load-bearing: HttpOnly keeps page script from reading
        // it, and SameSite=Strict is what makes the cookie useless to a
        // cross-site request -- the reason it is safe for the cookie to exist
        // at all.
        assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly");
        assert!(
            cookie.contains("SameSite=Strict"),
            "cookie must be SameSite=Strict"
        );
        assert_eq!(header_value(header::CACHE_CONTROL), "no-store");
        assert_eq!(header_value(header::REFERRER_POLICY), "no-referrer");
        let policy = header_value(header::CONTENT_SECURITY_POLICY);
        assert!(policy.starts_with("default-src 'none';"), "{policy}");
        assert!(!policy.contains("script-src"), "{policy}");
        assert_eq!(
            header_value(header::CONTENT_TYPE),
            "text/html; charset=utf-8"
        );

        let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        // Relative, so a browser under a proxy's path prefix stays under it.
        assert!(
            body.contains("<meta http-equiv=\"refresh\" content=\"0;url=./\">"),
            "{body}"
        );
        assert!(
            !body.contains("tok"),
            "the page must not carry the token: {body}"
        );
        assert!(!body.contains("<script"), "{body}");
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
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // And with no credential at all.
        let res = index(
            State(ui),
            HeaderMap::new(),
            Query(IndexQuery { t: None }),
            Uri::default(),
        )
        .await;
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
                Uri::default(),
            )
            .await;
            assert_eq!(
                res.status(),
                StatusCode::OK,
                "redemption {attempt} must succeed like the first"
            );
            assert!(
                res.headers()
                    .get(header::SET_COOKIE)
                    .is_some_and(|cookie| cookie
                        .to_str()
                        .unwrap()
                        .contains("biorouter_session=tok")),
                "redemption {attempt} must set the cookie like the first"
            );
        }
    }

    #[tokio::test]
    async fn a_valid_cookie_is_enough_on_a_later_request() {
        let ui = ui_with(Some("tok"));
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "biorouter_session=tok".parse().unwrap());
        let res = index(
            State(ui),
            headers,
            Query(IndexQuery { t: None }),
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// A request carrying the session cookie, as the shell's own origin sends it,
    /// with the Fetch Metadata a browser attaches for `mode` and `dest`.
    fn cookie_request(mode: Option<&str>, dest: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "biorouter_session=tok".parse().unwrap());
        if let Some(mode) = mode {
            headers.insert("sec-fetch-mode", mode.parse().unwrap());
        }
        if let Some(dest) = dest {
            headers.insert("sec-fetch-dest", dest.parse().unwrap());
        }
        headers
    }

    #[tokio::test]
    async fn lan_requests_cannot_read_the_shell_or_spoof_a_tls_proxy() {
        let ui = WebUi {
            index_html: inject_runtime_config(
                "<html><head></head><body></body></html>",
                "lan-secret",
            ),
            browser_token: Some("tok".into()),
            cookie_path: "/".to_string(),
            public: None,
            exchange_marker: "marker".to_string(),
        };
        for forwarded in [None, Some("https")] {
            for token in [None, Some("tok")] {
                let mut headers = cookie_request(None, None);
                headers.insert(header::HOST, "192.168.1.5:8765".parse().unwrap());
                if let Some(proto) = forwarded {
                    headers.insert("x-forwarded-proto", proto.parse().unwrap());
                }
                let res = index(
                    State(ui.clone()),
                    headers,
                    Query(IndexQuery {
                        t: token.map(str::to_owned),
                    }),
                    Uri::default(),
                )
                .await;
                assert_eq!(res.status(), StatusCode::FORBIDDEN);
                assert!(res.headers().get(header::SET_COOKIE).is_none());
                assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
                let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
                    .await
                    .unwrap();
                let body = String::from_utf8_lossy(&body);
                assert!(body.contains("SSH tunnel"));
                assert!(!body.contains("lan-secret"));
            }
        }
    }

    /// Issue #402. A browser sends Fetch Metadata only to an https or loopback
    /// address, so a navigation carrying it through a proxy is the browser
    /// saying it applies the shell's protections there. What the proxy chain
    /// says about the scheme is not asked: JupyterHub's configurable-http-proxy
    /// appends `http` after the TLS terminator's `https`, and its documented
    /// nginx block sends `X-Scheme` and no `X-Forwarded-Proto` at all. Each of
    /// those was refused with "Open Biorouter through localhost or HTTPS".
    #[tokio::test]
    async fn modern_tls_proxy_navigations_can_open_the_shell_and_exchange_tokens() {
        let ui = ui_with(Some("tok"));
        for proto in [Some("https"), Some("https,http"), Some("http"), None] {
            for token in [None, Some("tok")] {
                let mut headers = cookie_request(Some("navigate"), Some("document"));
                headers.insert(header::HOST, "biorouter.example.org".parse().unwrap());
                if let Some(proto) = proto {
                    headers.insert("x-forwarded-proto", proto.parse().unwrap());
                }
                let res = index(
                    State(ui.clone()),
                    headers,
                    Query(IndexQuery {
                        t: token.map(str::to_owned),
                    }),
                    Uri::default(),
                )
                .await;
                assert_eq!(res.status(), StatusCode::OK, "{proto:?} {token:?}");
                assert_eq!(
                    res.headers().get(header::SET_COOKIE).is_some(),
                    token.is_some()
                );
            }
        }
    }

    #[test]
    fn the_mount_is_named_relative_to_where_the_request_landed() {
        for (path, mount) in [
            ("/", "./"),
            ("/sessions", "./"),
            ("/sessions/", "../"),
            ("/a/b", "../"),
            ("/a/b/c", "../../"),
            ("/a//b", "../../"),
            ("", "./"),
        ] {
            assert_eq!(mount_href(path), mount, "{path:?}");
        }
    }

    /// The bundle is relative, so the shell names its mount before anything in
    /// its head asks for a URL. At `/` that is `./`, which is what the document's
    /// own address already said; on a path that fell through to the shell it is
    /// the way back up, so `./assets/...` does not become `/x/assets/...`.
    #[tokio::test]
    async fn the_shell_names_its_mount_first_in_its_head() {
        let ui = WebUi {
            index_html: inject_runtime_config(
                "<!doctype html><html><head><meta charset=utf-8>\
                 <script type=module src=\"./assets/index.js\"></script></head><body></body></html>",
                "s3cr3t",
            ),
            browser_token: None,
            cookie_path: "/".to_string(),
            public: None,
            exchange_marker: "marker".to_string(),
        };
        for (uri, mount) in [("/", "./"), ("/x/y/z", "../../")] {
            let res = index(
                State(ui.clone()),
                cookie_request(Some("navigate"), Some("document")),
                Query(IndexQuery { t: None }),
                uri.parse().unwrap(),
            )
            .await;
            assert_eq!(res.status(), StatusCode::OK);
            let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(
                body.starts_with(&format!(
                    "<!doctype html><html><head><base href=\"{mount}\"><meta charset=utf-8>"
                )),
                "{uri}: {body}"
            );
            assert_eq!(body.matches("<base").count(), 1, "{body}");
        }
    }

    /// A daemon with no token configured accepts any `?t=`, so the exchange
    /// must not copy it into a cookie: `x; Domain=evil.test` would have added an
    /// attribute, and a CR or LF would have failed the response with a 500. It
    /// sets no cookie at all and still moves the browser on.
    #[tokio::test]
    async fn a_daemon_without_a_token_writes_no_cookie_from_the_url() {
        let ui = ui_with(None);
        for presented in ["x; Domain=evil.test", "x\r\nX-Injected: 1", "anything"] {
            let res = index(
                State(ui.clone()),
                cookie_request(Some("navigate"), Some("document")),
                Query(IndexQuery {
                    t: Some(presented.into()),
                }),
                Uri::default(),
            )
            .await;
            assert_eq!(res.status(), StatusCode::OK, "{presented:?}");
            assert!(
                res.headers().get(header::SET_COOKIE).is_none(),
                "{presented:?}: {:?}",
                res.headers().get_all(header::SET_COOKIE)
            );
            assert!(res.headers().get("x-injected").is_none(), "{presented:?}");
        }
    }

    /// Behind a proxy's path prefix (`biorouter serve --public-url
    /// https://hub/user/a/proxy/8765/`) the session cookie is scoped to that
    /// prefix: every JupyterHub user's server shares the hostname, and a cookie
    /// at `/` would carry this token to all of them.
    #[tokio::test]
    async fn the_session_cookie_is_scoped_to_the_declared_public_path() {
        let ui = WebUi {
            cookie_path: "/user/a/proxy/8765".to_string(),
            ..ui_with(Some("tok"))
        };
        let res = index(
            State(ui),
            cookie_request(Some("navigate"), Some("document")),
            Query(IndexQuery {
                t: Some("tok".into()),
            }),
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let cookie = res.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains("; Path=/user/a/proxy/8765; "), "{cookie}");
    }

    /// Every `Set-Cookie` a response carries, in order.
    fn set_cookies(res: &Response<Body>) -> Vec<String> {
        res.headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect()
    }

    async fn body_text(res: Response<Body>) -> String {
        let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    /// A navigation through an https proxy to the shell, carrying `cookie`.
    fn proxied_navigation(host: &str, cookie: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        headers.insert("sec-fetch-mode", "navigate".parse().unwrap());
        headers.insert("sec-fetch-dest", "document".parse().unwrap());
        if let Some(cookie) = cookie {
            headers.insert(header::COOKIE, cookie.parse().unwrap());
        }
        headers
    }

    /// The live test of PR #404 (2026-10-17): `serve --public-url
    /// https://hub.test:18443/user/minrk/proxy/9999/` set its cookie at that
    /// path; restarted without the flag, `serve` set a fresh one at `/` under the
    /// same name. Chrome keeps both and sends the longer path first, so a reader
    /// of the first value answered every new link "This link needs its access
    /// token".
    #[tokio::test]
    async fn a_stale_cookie_at_a_longer_path_does_not_shadow_the_fresh_one() {
        let ui = ui_with(Some("fresh"));
        for headers in [
            proxied_navigation(
                "hub.test:18443",
                Some("biorouter_session=stale; biorouter_session=fresh"),
            ),
            // Split across two header lines, as HTTP/2 allows.
            {
                let mut headers =
                    proxied_navigation("hub.test:18443", Some("other=1; biorouter_session=stale"));
                headers.append(header::COOKIE, "biorouter_session=fresh".parse().unwrap());
                headers
            },
        ] {
            let res = index(
                State(ui.clone()),
                headers,
                Query(IndexQuery { t: None }),
                "/".parse().unwrap(),
            )
            .await;
            assert_eq!(res.status(), StatusCode::OK);
        }
        // Neither stale value alone, nor a pair of wrong ones, opens it.
        let res = index(
            State(ui),
            proxied_navigation(
                "hub.test:18443",
                Some("biorouter_session=stale; biorouter_session=older"),
            ),
            Query(IndexQuery { t: None }),
            "/".parse().unwrap(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// The cookie names `Secure` exactly when the browser is at an `https`
    /// address, and never on plain `http` to loopback, where the browser would
    /// then never send it back and `serve` on `http://127.0.0.1` would refuse
    /// its own link.
    #[tokio::test]
    async fn the_session_cookie_is_secure_only_behind_https() {
        let declared = crate::routes::PublicUrl::parse("https://hub.test:18443/u/a/").unwrap();
        let exchange = |public: Option<crate::routes::PublicUrl>, headers: HeaderMap| {
            let ui = WebUi {
                cookie_path: public
                    .as_ref()
                    .map_or("/", crate::routes::PublicUrl::path)
                    .to_string(),
                public,
                ..ui_with(Some("tok"))
            };
            async move {
                let res = index(
                    State(ui),
                    headers,
                    Query(IndexQuery {
                        t: Some("tok".into()),
                    }),
                    "/".parse().unwrap(),
                )
                .await;
                assert_eq!(res.status(), StatusCode::OK);
                set_cookies(&res)
            }
        };
        let rewritten_by_a_proxy = {
            let mut headers = proxied_navigation("127.0.0.1:8765", None);
            headers.insert("x-forwarded-for", "10.0.0.7".parse().unwrap());
            headers
        };
        for (case, public, headers, secure) in [
            (
                "loopback http",
                None,
                proxied_navigation("127.0.0.1:8765", None),
                false,
            ),
            (
                "loopback http with an https address declared",
                Some(declared.clone()),
                proxied_navigation("127.0.0.1:8765", None),
                false,
            ),
            (
                "an https proxy that keeps Host",
                None,
                proxied_navigation("biorouter.example.org", None),
                true,
            ),
            (
                "the declared https address",
                Some(declared.clone()),
                proxied_navigation("hub.test:18443", None),
                true,
            ),
            (
                "a proxy that rewrites Host, https declared",
                Some(declared.clone()),
                rewritten_by_a_proxy.clone(),
                true,
            ),
            (
                "a proxy that rewrites Host, nothing declared",
                None,
                rewritten_by_a_proxy,
                false,
            ),
        ] {
            let cookies = exchange(public, headers).await;
            assert!(!cookies.is_empty(), "{case}");
            for cookie in cookies {
                assert_eq!(cookie.ends_with("; Secure"), secure, "{case}: {cookie}");
            }
        }
    }

    /// `serve --public-url https://hub.test:18443/user/minrk/biorouter/`
    /// opened at `/user/minrk/proxy/9999/` instead: the token is valid and the
    /// cookie, scoped to the declared path, never comes back. The refusal says
    /// which address to open rather than that the link lacks its token.
    #[tokio::test]
    async fn a_valid_token_redeemed_at_another_address_names_the_declared_one() {
        let public =
            crate::routes::PublicUrl::parse("https://hub.test:18443/user/minrk/biorouter/")
                .unwrap();
        let ui = WebUi {
            cookie_path: public.path().to_string(),
            public: Some(public),
            ..ui_with(Some("c0ffee5ecre7"))
        };
        let exchanged = index(
            State(ui.clone()),
            proxied_navigation("hub.test:18443", None),
            Query(IndexQuery {
                t: Some("c0ffee5ecre7".into()),
            }),
            "/".parse().unwrap(),
        )
        .await;
        assert_eq!(exchanged.status(), StatusCode::OK);
        let cookies = set_cookies(&exchanged);
        assert_eq!(cookies.len(), 2, "{cookies:?}");
        assert!(
            cookies[0].starts_with("biorouter_session=c0ffee5ecre7; Path=/user/minrk/biorouter;"),
            "{cookies:?}"
        );
        // The marker carries no `Path`, so the browser scopes it to where it
        // really is, and none of the token.
        assert!(
            cookies[1].starts_with("biorouter_exchanged=marker;"),
            "{cookies:?}"
        );
        assert!(!cookies[1].contains("Path="), "{cookies:?}");
        assert!(!cookies[1].contains("c0ffee5ecre7"), "{cookies:?}");

        // Back with the marker and no session cookie: outside the declared path.
        let res = index(
            State(ui.clone()),
            proxied_navigation("hub.test:18443", Some("biorouter_exchanged=marker")),
            Query(IndexQuery { t: None }),
            "/".parse().unwrap(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
        let body = body_text(res).await;
        assert!(
            body.contains("not the address Biorouter was started with"),
            "{body}"
        );
        assert!(
            body.contains("<code>https://hub.test:18443/user/minrk/biorouter/</code>"),
            "{body}"
        );
        assert!(!body.contains("c0ffee5ecre7"), "{body}");

        // A marker of another daemon run, or none, gets the usual answer.
        for cookie in [Some("biorouter_exchanged=another"), None] {
            let res = index(
                State(ui.clone()),
                proxied_navigation("hub.test:18443", cookie),
                Query(IndexQuery { t: None }),
                "/".parse().unwrap(),
            )
            .await;
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
            assert!(
                body_text(res).await.contains("needs its access token"),
                "{cookie:?}"
            );
        }

        // At the root a cookie always comes back, so no marker is set.
        let res = index(
            State(ui_with(Some("tok"))),
            proxied_navigation("hub.test:18443", None),
            Query(IndexQuery {
                t: Some("tok".into()),
            }),
            "/".parse().unwrap(),
        )
        .await;
        assert_eq!(set_cookies(&res).len(), 1);
    }

    #[test]
    fn html_text_escapes_what_a_url_path_may_hold() {
        assert_eq!(
            html_text("https://h/a&b'c\"<d>"),
            "https://h/a&amp;b&#39;c&quot;&lt;d&gt;"
        );
    }

    #[tokio::test]
    async fn authority_without_a_host_header_still_checks_the_shell_address() {
        let res = index(
            State(ui_with(Some("tok"))),
            cookie_request(None, None),
            Query(IndexQuery { t: None }),
            "http://192.168.1.5:8765/".parse().unwrap(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// The shell carries the daemon secret and shares its origin with every
    /// app this daemon serves, so an app's script could `fetch('/')` or frame
    /// `/` with the cookie attached and read the secret. Only opening it works.
    #[tokio::test]
    async fn the_shell_is_served_only_to_a_navigation() {
        let ui = WebUi {
            index_html: inject_runtime_config("<html><head></head><body></body></html>", "s3cr3t"),
            browser_token: Some("tok".into()),
            cookie_path: "/".to_string(),
            public: None,
            exchange_marker: "marker".to_string(),
        };
        for (mode, dest) in [
            (Some("cors"), Some("empty")),
            (Some("no-cors"), Some("empty")),
            (Some("navigate"), Some("iframe")),
            (Some("navigate"), Some("object")),
            (Some("no-cors"), Some("script")),
            (None, Some("iframe")),
        ] {
            let res = index(
                State(ui.clone()),
                cookie_request(mode, dest),
                Query(IndexQuery { t: None }),
                Uri::default(),
            )
            .await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{mode:?} {dest:?}");
            let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
                .await
                .unwrap();
            assert!(
                !String::from_utf8_lossy(&body).contains("s3cr3t"),
                "{mode:?} {dest:?}"
            );
        }
        // Nor does a fetch of the exchange set the cookie.
        let res = index(
            State(ui.clone()),
            cookie_request(Some("cors"), Some("empty")),
            Query(IndexQuery {
                t: Some("tok".into()),
            }),
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert!(res.headers().get(header::SET_COOKIE).is_none());

        // Opening it, or asking without Fetch Metadata (not a browser), works.
        for (mode, dest) in [(Some("navigate"), Some("document")), (None, None)] {
            let res = index(
                State(ui.clone()),
                cookie_request(mode, dest),
                Query(IndexQuery { t: None }),
                Uri::default(),
            )
            .await;
            assert_eq!(res.status(), StatusCode::OK, "{mode:?} {dest:?}");
            let body = axum::body::to_bytes(res.into_body(), 64 * 1024)
                .await
                .unwrap();
            assert!(String::from_utf8_lossy(&body).contains("s3cr3t"));
        }
    }

    /// No page may frame the shell, and the shell keeps no scriptable handle
    /// across to a page without its opener policy (every app page), nor that
    /// page to it.
    #[tokio::test]
    async fn the_shell_cannot_be_framed_and_isolates_its_browsing_context() {
        let res = index(
            State(ui_with(Some("tok"))),
            cookie_request(Some("navigate"), Some("document")),
            Query(IndexQuery { t: None }),
            Uri::default(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let value = |name: &str| {
            res.headers()
                .get(name)
                .unwrap_or_else(|| panic!("no {name} header"))
                .to_str()
                .unwrap()
                .to_string()
        };
        assert!(value("content-security-policy").contains("frame-ancestors 'none'"));
        assert_eq!(value("x-frame-options"), "DENY");
        assert_eq!(value("cross-origin-opener-policy"), "same-origin");
    }
}
