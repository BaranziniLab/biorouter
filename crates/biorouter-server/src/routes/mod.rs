use std::sync::LazyLock;

use axum::http::HeaderMap;

/// A web origin reduced to what same-origin compares (scheme, host, port) and
/// normalised once, here: scheme and host lowercased, as RFC 6454 compares
/// them. Every origin decision in the daemon parses through this, so no two of
/// them can disagree about case again — QA-D F7 found `is_local_origin`
/// comparing the host case-sensitively fifteen lines above a check that did not.
///
/// The port is kept exactly as written, present or absent, and is not filled in
/// with the scheme's default. `http://host` and `http://host:80` are one origin
/// in the RFC, but a gate that started admitting the second spelling where it
/// had been refused would be a gate that got wider, and these only get
/// narrower. Browsers never write the default port in either header, so the
/// two spellings never actually meet.
///
/// Parsed rather than prefix-matched: `http://127.0.0.1:` is a prefix of
/// `http://127.0.0.1:8080.evil.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebOrigin {
    scheme: &'static str,
    host: String,
    port: Option<u16>,
}

impl WebOrigin {
    /// An `Origin` header's value: exactly `scheme://host[:port]` under `http`
    /// or `https`. `None` for anything else — `null`, `file://`, a trailing
    /// path, userinfo — never a best guess.
    pub fn parse(origin: &str) -> Option<Self> {
        let (scheme, authority) = origin.split_once("://")?;
        Self::with_authority(scheme, authority)
    }

    /// The origin a request was addressed to: its `Host`, under the scheme the
    /// client reached the daemon with ([`request_scheme`]).
    fn of_request(host: &str, scheme: &str) -> Option<Self> {
        Self::with_authority(scheme, host)
    }

    fn with_authority(scheme: &str, authority: &str) -> Option<Self> {
        let scheme = if scheme.eq_ignore_ascii_case("http") {
            "http"
        } else if scheme.eq_ignore_ascii_case("https") {
            "https"
        } else {
            return None;
        };
        let (host, port) = split_authority(authority)?;
        Some(Self { scheme, host, port })
    }

    /// Plain `http` to `localhost` or `127.0.0.1`, on any port: the only
    /// origins the daemon's cross-origin policy has ever admitted.
    fn is_loopback_http(&self) -> bool {
        self.scheme == "http" && matches!(self.host.as_str(), "localhost" | "127.0.0.1")
    }
}

/// The origin as a browser serializes it, normalised: `http://localhost:5173`.
impl std::fmt::Display for WebOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        match self.port {
            Some(port) => write!(f, ":{port}"),
            None => Ok(()),
        }
    }
}

/// `host[:port]`, strictly: a bracketed IPv6 literal or a name of letters,
/// digits, `-`, `.` and `_`, then optionally a port of one to five digits with
/// no leading zero. The host comes back lowercased. Anything else (an empty
/// host or port, a second colon, userinfo, a path) is `None`.
///
/// No leading zero because `:080` and `:80` are the same number and different
/// strings: before this parser, the socket gates compared authorities as
/// strings, and a zero-padded port that was refused then must stay refused.
fn split_authority(authority: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = match authority.strip_prefix('[') {
        Some(bracketed) => {
            let (address, rest) = bracketed.split_once(']')?;
            let is_v6 = |b: u8| b.is_ascii_hexdigit() || b == b':' || b == b'.';
            if address.is_empty() || !address.bytes().all(is_v6) {
                return None;
            }
            let port = match rest {
                "" => None,
                rest => Some(rest.strip_prefix(':')?),
            };
            (format!("[{address}]"), port)
        }
        None => {
            let (host, port) = match authority.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            };
            let is_name = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_');
            if host.is_empty() || !host.bytes().all(is_name) {
                return None;
            }
            (host.to_string(), port)
        }
    };
    let port = match port {
        None => None,
        Some(port) => {
            if port.is_empty()
                || port.len() > 5
                || port.starts_with('0')
                || !port.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            Some(port.parse::<u16>().ok()?)
        }
    };
    Some((host.to_ascii_lowercase(), port))
}

/// Origins the daemon's cross-origin policy admits: plain `http` to
/// `localhost` or `127.0.0.1`, on any port. This is the CORS rule
/// (`commands::agent`), for the dev renderer's `fetch` calls from its own vite
/// port, and nothing else.
///
/// It is NOT a WebSocket rule, and stopped being one in QA-D F7. CORS does not
/// govern a WebSocket handshake at all, and "any loopback port" admitted every
/// other local page's socket as though it were this daemon's own. The socket
/// gates ask [`UpgradeOrigin::is_this_daemons`] instead.
pub fn is_local_origin(origin: &str) -> bool {
    WebOrigin::parse(origin).is_some_and(|origin| origin.is_loopback_http())
}

/// Whether `origin` names the very origin this request was addressed to:
/// same scheme, same host, same port.
///
/// A browser may reach the daemon at a LAN address or a hostname that the
/// daemon cannot enumerate: it may have bound `0.0.0.0`, and the address the
/// user typed is not knowable from the bind. What *is* knowable is the `Host`
/// the request carries. A same-origin page always presents an `Origin` whose
/// host and port equal that `Host`, and a page on any other origin cannot: the
/// browser sets both, and neither is reachable from script. So comparing the
/// two is a precise same-origin test that needs no configuration and no
/// wildcard, and it holds for every address the interface is ever reached at.
///
/// The scheme is compared too, against `scheme` — see [`request_scheme`].
/// Without it this was an *authority* test (QA-D F7): behind the TLS proxy the
/// deployment guide recommends, a plain-`http` page at the same host and port
/// passed it.
///
/// Both are compared whole. `http://evil.com` is not admitted by a `Host` of
/// `evil.com.attacker.net`, because this is an equality test rather than a
/// prefix one.
pub fn origin_matches_host(origin: &str, host: Option<&str>, scheme: &str) -> bool {
    let Some(host) = host else {
        // No `Host` to compare against. Refuse rather than guess.
        return false;
    };
    match (
        WebOrigin::parse(origin),
        WebOrigin::of_request(host, scheme),
    ) {
        (Some(origin), Some(request)) => origin == request,
        // `null`, `file://` and anything else opaque, or a `Host` that is not
        // one. Callers that admit `file://` do so by name; this is not the
        // place for it.
        _ => false,
    }
}

/// The scheme the client used to reach this daemon.
///
/// The daemon itself speaks only plain HTTP, so that is the answer unless a
/// reverse proxy in front of it says otherwise with `X-Forwarded-Proto` — the
/// documented way to put TLS in front of `biorouter serve`. Such a proxy has to
/// forward the original `Host` as well; the socket gates needed that already.
///
/// ⚠ **The LAST value, not the first.** Nearly every proxy *replaces* this
/// header (nginx's `proxy_set_header X-Forwarded-Proto $scheme`), and where one
/// does the two readings are the same value. They differ only for a proxy that
/// *appends*, and there the last entry is the one that proxy wrote while the
/// first is whatever the client sent — so reading the first is worse twice over.
/// It is less trustworthy: a client that writes `https` keeps that value even
/// behind a proxy that appends its own `http`. And it is less **available**,
/// which is how this was found: a client that writes `http` turns a legitimate
/// https page's handshake into `"http, https"` → `http`, which then fails the
/// same-origin comparison and refuses **every** WebSocket upgrade from that
/// deployment. Fail-closed, but a LAN attacker could trigger it at will. What
/// reading the last costs is a chain whose outer hop is https and whose inner
/// hops are not, and such a deployment fixes that at the inner proxy by
/// preserving the value it was handed.
///
/// ⚠ **It is trusted unconditionally, and the reason is a property of the
/// CLIENT rather than of this daemon.** Worth writing down, because `auth.rs`'s
/// rate-limit key and `commands::agent`'s CORS both explicitly REFUSE to trust
/// `X-Forwarded-For` a module away, and the asymmetry reads as an oversight. It
/// is not the same question. `X-Forwarded-For` is the only evidence of who a
/// caller is, so forging it buys an attacker someone else's identity. This
/// header only decides how an `Origin` is compared to a `Host`, and that
/// comparison exists solely to constrain a **browser** page on another origin —
/// which cannot set this header on a WebSocket handshake at all. A client that
/// can set it is not a browser and gains nothing by it: it may simply send no
/// `Origin`, which both socket gates admit by design, their token being the
/// authority there. A trusted-proxy allowlist would add a configuration surface
/// and close nothing. If the `Origin` test ever becomes load-bearing for callers
/// that are not browsers, this is the line that has to change with it.
pub(crate) fn request_scheme(headers: &HeaderMap) -> &'static str {
    let forwarded = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        // The value the proxy NEAREST this daemon wrote; see above.
        .and_then(|value| value.rsplit(',').next())
        .map(str::trim);
    match forwarded {
        Some(proto) if proto.eq_ignore_ascii_case("https") => "https",
        _ => "http",
    }
}

/// The environment variable a launcher sets to name the renderer that drives
/// this daemon from an origin other than the daemon's own.
///
/// There are two such renderers and both are the desktop app. In development
/// vite serves it from its own port (5173, or the next free one), so its
/// sockets present `http://localhost:517x` while the daemon sits on an
/// ephemeral port; packaged, Electron loads it from a `file:` URL and its
/// sockets present [`ELECTRON_FILE_ORIGIN`]. The Electron main process knows
/// which of the two it loaded and declares it when it spawns the daemon, and
/// `just debug-server` declares vite's default. `biorouter serve` declares
/// nothing and strips an inherited value, so a `serve` daemon admits only its
/// own origin.
pub const RENDERER_ORIGIN_ENV: &str = "BIOROUTER_RENDERER_ORIGIN";

/// The `Origin` a page loaded from a `file:` URL presents on a WebSocket
/// handshake, as Chromium serializes it.
///
/// Not `null`: that is the opaque origin of a *sandboxed* frame — including the
/// agent-authored figures this app renders in its artifact panel — and
/// `routes::workspace`'s gate refuses it by name.
pub(crate) const ELECTRON_FILE_ORIGIN: &str = "file://";

/// What a launcher declared with [`RENDERER_ORIGIN_ENV`]: the one page this
/// daemon admits a socket from that its own origin does not account for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeclaredRenderer {
    /// The dev renderer: vite's page on its own loopback port.
    LoopbackHttp(WebOrigin),
    /// The packaged desktop renderer, loaded from a `file:` URL.
    ///
    /// [`WebOrigin`] cannot hold it, and should not: `file://` has no host and
    /// no port, so there is nothing for a same-origin test to compare. It can
    /// only ever be matched by name — and, since the security review of QA-D
    /// F7, only on a daemon whose launcher said it has such a renderer.
    ElectronFile,
}

impl std::fmt::Display for DeclaredRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoopbackHttp(origin) => write!(f, "{origin}"),
            Self::ElectronFile => f.write_str(ELECTRON_FILE_ORIGIN),
        }
    }
}

/// The renderer a launcher declared, if the value names one this daemon admits.
///
/// Either the exact literal [`ELECTRON_FILE_ORIGIN`], or plain `http` to
/// `localhost` or `127.0.0.1` — exactly the origins the socket gates admitted on
/// every port before QA-D F7 — so a declaration can only ever narrow them back
/// to one port, never admit anything they refused. An unset or empty value
/// declares nothing; anything else is refused with a warning rather than guessed
/// at.
pub(crate) fn declared_renderer(value: Option<&str>) -> Option<DeclaredRenderer> {
    let value = value.map(str::trim).filter(|value| !value.is_empty())?;
    if value == ELECTRON_FILE_ORIGIN {
        return Some(DeclaredRenderer::ElectronFile);
    }
    match WebOrigin::parse(value).filter(WebOrigin::is_loopback_http) {
        Some(origin) => Some(DeclaredRenderer::LoopbackHttp(origin)),
        None => {
            tracing::warn!(
                "{RENDERER_ORIGIN_ENV}={value:?} is neither {ELECTRON_FILE_ORIGIN:?} nor an http \
                 origin on localhost or 127.0.0.1; no renderer origin is admitted"
            );
            None
        }
    }
}

/// This process's declared renderer, read once.
pub(crate) fn declared_renderer_origin() -> Option<&'static DeclaredRenderer> {
    static DECLARED: LazyLock<Option<DeclaredRenderer>> =
        LazyLock::new(|| declared_renderer(std::env::var(RENDERER_ORIGIN_ENV).ok().as_deref()));
    DECLARED.as_ref()
}

/// What a WebSocket upgrade says about where it came from and where it was
/// sent, read off the request once so that both socket gates — the workspace
/// socket and the per-app agent socket — ask the same question of it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UpgradeOrigin<'a> {
    /// The browser-set `Origin`. A non-browser client sends none.
    pub origin: Option<&'a str>,
    /// An `Origin` header WAS sent and could not be read as a string.
    ///
    /// Kept apart from `origin: None`, which means nothing sent one at all.
    /// Both socket gates admit an upgrade with no `Origin`, because their token
    /// is the authority for a client that is not a browser — so folding a
    /// present-but-unreadable header into that case skips the gate entirely,
    /// which is how it behaved until the security review of QA-D F7. `Host`
    /// fails CLOSED in the same situation ([`origin_matches_host`] refuses a
    /// `None` host rather than guessing), and the two must not disagree about
    /// what an unreadable header means. A browser cannot produce one — they
    /// punycode hosts — which is exactly why refusing costs nothing.
    pub origin_unreadable: bool,
    /// The `Host` the upgrade was addressed to.
    pub host: Option<&'a str>,
    /// The scheme the client used; see [`request_scheme`].
    pub scheme: &'static str,
    /// The renderer this daemon's launcher declared, if any; see
    /// [`RENDERER_ORIGIN_ENV`].
    pub renderer: Option<&'a DeclaredRenderer>,
}

impl<'a> UpgradeOrigin<'a> {
    pub(crate) fn from_headers(headers: &'a HeaderMap) -> Self {
        let origin = headers.get(axum::http::header::ORIGIN);
        Self {
            origin: origin.and_then(|value| value.to_str().ok()),
            origin_unreadable: origin.is_some_and(|value| value.to_str().is_err()),
            host: headers
                .get(axum::http::header::HOST)
                .and_then(|value| value.to_str().ok()),
            scheme: request_scheme(headers),
            renderer: declared_renderer_origin(),
        }
    }

    /// Is the page behind this upgrade one of this daemon's own: served from
    /// its origin, or the renderer its launcher declared?
    ///
    /// `false` when the upgrade carries no `Origin`. Whether that is admitted
    /// is each gate's own decision, and both admit it, because their token is
    /// the authority there.
    pub(crate) fn is_this_daemons(&self) -> bool {
        let Some(origin) = self.origin else {
            return false;
        };
        origin_matches_host(origin, self.host, self.scheme)
            || matches!(
                self.renderer,
                Some(DeclaredRenderer::LoopbackHttp(renderer))
                    if WebOrigin::parse(origin).as_ref() == Some(renderer)
            )
    }

    /// Is this upgrade the **packaged desktop renderer's own page**, on a daemon
    /// whose launcher said it has one?
    ///
    /// Asked only by `routes::workspace`, which is the only socket an Electron
    /// `file:` page opens. `routes::apps` neither asks nor should: an app's page
    /// is served by this daemon over http, so it is same-origin with its own
    /// socket and has never needed an allowance by name. That asymmetry is why
    /// this is a separate question rather than another arm inside
    /// [`Self::is_this_daemons`] — folding it in would silently widen the apps
    /// gate to admit a `file:` page too.
    pub(crate) fn is_declared_electron_renderer(&self) -> bool {
        self.origin == Some(ELECTRON_FILE_ORIGIN)
            && matches!(self.renderer, Some(DeclaredRenderer::ElectronFile))
    }
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
    use super::{
        declared_renderer, is_local_origin, origin_matches_host, request_scheme, DeclaredRenderer,
        UpgradeOrigin, WebOrigin,
    };
    use axum::http::HeaderMap;

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

    /// QA-D F7: this compared the host case-sensitively while
    /// `origin_matches_host` beside it did not. Both parse through
    /// `WebOrigin` now, which lowercases once. What the rule admits is
    /// otherwise exactly what it admitted: plain `http`, `localhost` or
    /// `127.0.0.1`, any port. `[::1]` was never in it and is not now.
    #[test]
    fn the_cors_rule_ignores_case_and_admits_nothing_else_new() {
        assert!(is_local_origin("http://LOCALHOST:5173"));
        assert!(is_local_origin("HTTP://127.0.0.1:5173"));
        assert!(!is_local_origin("http://[::1]:5173"));
        assert!(!is_local_origin("http://localhost:0"));
        assert!(!is_local_origin("http://localhost:99999"));
        assert!(!is_local_origin("http://localhost:05173"));
        assert!(!is_local_origin("http://user@localhost:5173"));
        assert!(!is_local_origin("http://localhost:5173/"));
    }

    /// Same origin means scheme, host AND port, against the request's own
    /// `Host`. Each refused case here was admitted by the gates before QA-D F7,
    /// either as "any loopback port" or by an authority-only comparison.
    #[test]
    fn same_origin_compares_scheme_host_and_port() {
        let daemon = Some("127.0.0.1:9380");
        // Accepted: the daemon's own page, however its host is cased.
        assert!(origin_matches_host("http://127.0.0.1:9380", daemon, "http"));
        assert!(origin_matches_host(
            "http://LOCALHOST:9380",
            Some("localhost:9380"),
            "http"
        ));
        assert!(origin_matches_host(
            "http://[::1]:9380",
            Some("[::1]:9380"),
            "http"
        ));
        // Accepted: a LAN address or a hostname, which nothing enumerated.
        assert!(origin_matches_host(
            "http://192.168.1.42:8765",
            Some("192.168.1.42:8765"),
            "http"
        ));
        // Accepted: behind a TLS proxy that says so, with no port in either.
        assert!(origin_matches_host(
            "https://lab.example.org",
            Some("lab.example.org"),
            "https"
        ));

        // Refused: another loopback port.
        assert!(!origin_matches_host("http://127.0.0.1:1", daemon, "http"));
        assert!(!origin_matches_host(
            "http://localhost:3000",
            daemon,
            "http"
        ));
        // Refused: another scheme, both ways round.
        assert!(!origin_matches_host(
            "https://127.0.0.1:9380",
            daemon,
            "http"
        ));
        assert!(!origin_matches_host(
            "http://lab.example.org",
            Some("lab.example.org"),
            "https"
        ));
        // Refused: another host, however it is cased. `localhost` and
        // `127.0.0.1` are two origins, not one.
        assert!(!origin_matches_host(
            "http://LOCALHOST:9380",
            daemon,
            "http"
        ));
        assert!(!origin_matches_host(
            "http://localhost:9380",
            daemon,
            "http"
        ));
        // Refused: the default port written in one header and not the other.
        // One origin in the RFC, but a spelling these gates refused before,
        // and they only narrow. No browser writes it in either.
        assert!(!origin_matches_host(
            "http://example.org",
            Some("example.org:80"),
            "http"
        ));
        assert!(!origin_matches_host(
            "http://127.0.0.1:09380",
            daemon,
            "http"
        ));
        // Refused: anything that is not an origin, and no Host at all.
        for origin in [
            "null",
            "file://",
            "",
            "http://",
            "http://127.0.0.1:9380/",
            "http://user@127.0.0.1:9380",
        ] {
            assert!(!origin_matches_host(origin, daemon, "http"), "{origin:?}");
        }
        assert!(!origin_matches_host("http://127.0.0.1:9380", None, "http"));
    }

    #[test]
    fn the_scheme_is_http_unless_a_proxy_says_https() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-proto", value.parse().unwrap());
            request_scheme(&headers)
        };
        assert_eq!(request_scheme(&HeaderMap::new()), "http");
        assert_eq!(with("https"), "https");
        assert_eq!(with("HTTPS"), "https");
        assert_eq!(with("http"), "http");
        assert_eq!(with("gopher"), "http");

        // A proxy that APPENDS rather than replaces: the value it wrote is the
        // last one, and whatever the client sent sits in front of it. Reading
        // the first refused every upgrade from a proxied https deployment whose
        // client had written `http` — fail-closed, and triggerable at will.
        assert_eq!(with("http, https"), "https");
        // …and symmetrically, a client-written `https` does not survive a proxy
        // that appends the truth after it.
        assert_eq!(with("https, http"), "http");
        assert_eq!(with("https , http"), "http");
        assert_eq!(with("http,https"), "https");
        // One value, which is what a replacing proxy sends, reads the same
        // either way — that is why this choice is free for every ordinary
        // deployment.
        assert_eq!(with(" https "), "https");
    }

    /// The declaration names either the packaged renderer's `file:` page or a
    /// loopback-http origin the socket gates admitted on every port before QA-D
    /// F7 — so it narrows them back to one port and can never admit anything
    /// they refused.
    #[test]
    fn a_renderer_is_declared_as_loopback_http_or_the_electron_file_page() {
        assert_eq!(
            declared_renderer(Some("http://localhost:5173")),
            WebOrigin::parse("http://localhost:5173").map(DeclaredRenderer::LoopbackHttp)
        );
        assert!(declared_renderer(Some(" http://127.0.0.1:5174 ")).is_some());
        // The packaged app's own page. Admitted by NAME, which is why it has to
        // be declared to be admitted at all.
        assert_eq!(
            declared_renderer(Some("file://")),
            Some(DeclaredRenderer::ElectronFile)
        );
        assert_eq!(
            declared_renderer(Some(" file:// ")),
            Some(DeclaredRenderer::ElectronFile)
        );
        for refused in [
            "https://localhost:5173",
            "http://example.org:5173",
            "http://[::1]:5173",
            "http://localhost:5173/",
            // Neither the opaque origin of a sandboxed frame nor a file URL
            // with a path is the packaged renderer's origin.
            "null",
            "file:///",
            "file:///Users/me/evil.html",
            "localhost:5173",
        ] {
            assert_eq!(declared_renderer(Some(refused)), None, "{refused:?}");
        }
        assert_eq!(declared_renderer(Some("")), None);
        assert_eq!(declared_renderer(None), None);
    }

    /// The whole question both socket gates ask, with and without a declared
    /// renderer.
    /// A header that was SENT and cannot be read is not the same as no header,
    /// and `from_headers` is where the difference has to be preserved: both gates
    /// admit `origin: None`, so anything folded into it skips them.
    #[test]
    fn a_present_but_unreadable_origin_is_not_an_absent_one() {
        let mut headers = HeaderMap::new();
        // Valid as a header value (obs-text permits 0x80..=0xFF) and not UTF-8,
        // so `HeaderValue::to_str` fails on it.
        headers.insert(
            axum::http::header::ORIGIN,
            axum::http::HeaderValue::from_bytes(b"http://\xff.example").unwrap(),
        );
        let upgrade = UpgradeOrigin::from_headers(&headers);
        assert_eq!(upgrade.origin, None);
        assert!(
            upgrade.origin_unreadable,
            "an unreadable Origin must be distinguishable from an absent one, or the gates \
             that admit `None` admit it too"
        );

        // Nothing sent one: the case both gates deliberately admit.
        let no_headers = HeaderMap::new();
        let absent = UpgradeOrigin::from_headers(&no_headers);
        assert_eq!(absent.origin, None);
        assert!(!absent.origin_unreadable);

        // And a readable one is unaffected.
        let mut ok = HeaderMap::new();
        ok.insert(
            axum::http::header::ORIGIN,
            "http://127.0.0.1:9380".parse().unwrap(),
        );
        let readable = UpgradeOrigin::from_headers(&ok);
        assert_eq!(readable.origin, Some("http://127.0.0.1:9380"));
        assert!(!readable.origin_unreadable);
    }

    #[test]
    fn an_upgrade_is_this_daemons_when_same_origin_or_the_declared_renderer() {
        let vite =
            DeclaredRenderer::LoopbackHttp(WebOrigin::parse("http://localhost:5173").unwrap());
        let upgrade = |origin, renderer| UpgradeOrigin {
            origin,
            origin_unreadable: false,
            host: Some("127.0.0.1:9380"),
            scheme: "http",
            renderer,
        };
        assert!(upgrade(Some("http://127.0.0.1:9380"), None).is_this_daemons());
        assert!(upgrade(Some("http://localhost:5173"), Some(&vite)).is_this_daemons());
        // Only the declared port, not its neighbour...
        assert!(!upgrade(Some("http://localhost:5174"), Some(&vite)).is_this_daemons());
        // ...and nothing on loopback when nothing is declared.
        assert!(!upgrade(Some("http://localhost:5173"), None).is_this_daemons());
        // No Origin is not "this daemon's"; each gate decides that case itself.
        assert!(!upgrade(None, Some(&vite)).is_this_daemons());
    }

    /// The packaged renderer's `file:` page is a DIFFERENT question from
    /// `is_this_daemons`, asked only by the workspace gate, and answered only on
    /// a daemon whose launcher declared such a renderer.
    ///
    /// Before the security review of QA-D F7, `routes::workspace` compared the
    /// origin to the literal `"file://"` with nothing else asked — so a local
    /// `.html` opened in Chromium cleared that gate on every daemon, `biorouter
    /// serve` included.
    #[test]
    fn the_electron_file_page_is_admitted_only_where_it_was_declared() {
        let electron = DeclaredRenderer::ElectronFile;
        let vite =
            DeclaredRenderer::LoopbackHttp(WebOrigin::parse("http://localhost:5173").unwrap());
        let upgrade = |origin, renderer| UpgradeOrigin {
            origin,
            origin_unreadable: false,
            host: Some("127.0.0.1:9380"),
            scheme: "http",
            renderer,
        };
        assert!(upgrade(Some("file://"), Some(&electron)).is_declared_electron_renderer());
        // Nothing declared — `biorouter serve`, or a hand-run `biorouterd`.
        assert!(!upgrade(Some("file://"), None).is_declared_electron_renderer());
        // A dev daemon declares vite's page, not a file page.
        assert!(!upgrade(Some("file://"), Some(&vite)).is_declared_electron_renderer());
        // And the declaration admits that one literal and nothing near it.
        for origin in ["null", "file:///", "file:///Users/me/evil.html", "FILE://"] {
            assert!(
                !upgrade(Some(origin), Some(&electron)).is_declared_electron_renderer(),
                "{origin:?}"
            );
        }
        assert!(!upgrade(None, Some(&electron)).is_declared_electron_renderer());
        // It is not `is_this_daemons`, which is what `routes::apps` asks — so
        // declaring an Electron renderer must not widen the apps socket's gate.
        assert!(!upgrade(Some("file://"), Some(&electron)).is_this_daemons());
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
pub mod session_meta;
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
        .merge(session_meta::routes(state.clone()))
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
