# How browser-served Biorouter is built

> **What this is.** The architecture of the serving path: what `biorouter serve` starts, how the
> daemon serves the interface, how a browser is authenticated, and which pieces of the old
> reverse-proxying front door were deleted rather than moved.
> **Status:** Current. Describes shipped code; every mechanism below is in the tree and
> exercised by the `serve` job in `.github/workflows/rust.yml`.
> **Audience:** developers working on `biorouter-server`, `biorouter-cli`, or the renderer.

Biorouter's interface is a single-page application. The desktop application loads it inside
Electron; browser-served Biorouter loads the same bundle over HTTP. This page describes the
second path — what serves the files, what authenticates the request, and why the arrangement
looks the way it does.

The reasoning behind each choice is recorded separately in [the decision
records](serve-decisions.md); this page assumes them and describes the result. Read
[browser access](browser-access.md) if you only want to use the command.

---

## The shape, in one picture

```text
  biorouter serve
        │
        │  spawns, with a closed stdin (SD-1, SD-7)
        ▼
   biorouterd  ── serves ──▶  the interface bundle at  /
        │                     the interface's own endpoints at  /headless/*
        │                     the agent API at its existing paths
        │
        └── one process, one origin, one secret
```

Everything the browser talks to is the daemon, on one origin. There is no second process and no
proxy hop.

## Why one origin does so much work

The previous arrangement put a separate binary in front of the daemon, serving the bundle itself
and reverse-proxying everything else. Three properties follow from removing that hop, and they
are the main reason the change is worth making.

**WebSockets work at all.** `/ui/workspace` (workspace control) and `/apps/{id}/agent` (an
Agent Drafter app's live agent socket) are daemon routes. Reached on the daemon's own origin
they need nothing added. Through the old proxy they could not work, and the client's
retry-with-backoff meant they failed silently rather than reporting it.

**There is one secret, checked once.** The proxy held the daemon's credential and attached it to
whatever arrived. Now the browser presents the same credential every other client presents, and
`check_token` is the only thing that inspects it.

**The interface's own endpoints are authenticated.** The sixteen `/headless/*` paths — the
filesystem browser, settings, extension installation, skill extraction — were previously served
by a router carrying exactly one layer, `TraceLayer`. Moved into the daemon they sit behind the
same middleware as everything else.

## What the daemon does with a web directory

`Settings` gains a third field beside `host` and `port`. It is a flat structure read from the
environment with a `BIOROUTER_` prefix, so the new setting is `BIOROUTER_SERVE_UI`, pointing at
a directory containing the built bundle. When it is unset the daemon behaves exactly as it does
today and serves no interface — which is what the desktop application wants, since Electron
loads the bundle itself.

When it is set, the daemon mounts two things:

| What | Path | Authentication |
|---|---|---|
| The application shell | `GET /` | Browser token, exchanged for a session cookie |
| Hashed static assets | `/assets/*` and other built files | None — they carry no secrets |
| The interface's own endpoints | `/headless/*` | The ordinary secret-key header |

The shell is served at the root and nowhere else (SD-4), so the bundle's baked-in
root-absolute asset URLs are correct as built and nothing rewrites them.

### The interface's routes never reach the daemon

The application uses a **hash router** (`ui/desktop/src/App.tsx`, `ImmediateHashRouter`).
Every one of its routes — `settings`, `sessions`, `knowledge`, `apps` and the rest — lives in
the URL fragment, and a fragment is never sent to a server. Whatever the user navigates to, the
daemon sees a request for `/`.

Two consequences worth knowing before changing anything here:

- **The application's route space and the API's URL space cannot collide.** They would
  otherwise: `/sessions/{session_id}` is a real API route, and a browser-history router asking
  for that path would be answered by the API rather than by the shell — a `401` on what looks
  like an ordinary page load.
- **The catch-all fallback is defensive, not load-bearing.** It exists so an unexpected path
  returns the application instead of a bare `404`, and it is gated exactly as `/` is so it
  cannot become a way to read the shell without the token.

> **Warning.** Switching the interface to a browser-history router would make the collision
> above real, and it would surface as a handful of pages that 401 while the rest work. If that
> change is ever made, the API needs a path prefix of its own first.

### The one thing injected into the shell

The renderer already knows how to run outside Electron. `ui/desktop/src/renderer.tsx` reads a
global the server places in the document:

```ts
const globalConfig = window.__BIOROUTER_HEADLESS_CONFIG__ ?? {};
```

and derives its API base, its interface-endpoint base and its secret from it, falling back to
query parameters and `sessionStorage`. The daemon's only job is to populate that global before
handing over the document. Everything downstream — the ninety-six-method shim that stands in for
`window.electron`, the API client, the secret handling — already exists and is unchanged.

> **Why this matters for scope.** There is no second frontend and there never was. One Vite
> bundle serves both Electron and the browser, and the shipped Electron bundle already contains
> the browser shim. Interface features reach the browser automatically unless they depend on an
> Electron main-process capability.

## Authenticating a browser

A browser's first request cannot carry a header, so the secret-key scheme every other client uses
cannot gate the initial document. The exchange is therefore:

1. `biorouter serve` mints a random browser token for the launch and prints it in the URL.
2. `GET /?t=<token>` validates it, sets an `httpOnly`, `SameSite=Strict` session cookie, and
   redirects to `/`.
3. `GET /` with that cookie returns the shell, with the daemon's secret injected into it.
4. From then on the application presents `X-Secret-Key` exactly as the desktop renderer does.

The cookie gates **the document only**. It is deliberately not accepted as authentication on the
API routes: doing so would make every API route reachable by a cookie the browser attaches
automatically, which is a cross-site request forgery surface that the header scheme does not
have. Keeping the cookie's job to one request means `check_token` is unchanged.

The cookie has one other reader, and it narrows rather than admits. An API request that has
already passed `check_token` and also carries the cookie came from the document this daemon
served, so the listing and knowledge-base gates give it the tier of the provider the operator
configured. A request holding only the secret is a public caller there. The transcript gate never
reads the cookie. See
[decision SD-10](serve-decisions.md#sd-10--the-served-interface-keeps-its-operators-reach-on-listings-and-knowledge-bases-and-gains-nothing-else).

> **Warning.** `check_token` records a failed attempt for every request without the secret and
> refuses after twenty inside sixty seconds, keyed on the peer address. The browser-token check
> must not feed that same counter — a mistyped URL would otherwise lock the user out of their own
> machine for a minute, and behind network address translation it would lock out their
> colleagues too.

## Reaching it from another machine

The default bind is loopback (SD-2). A non-loopback bind is requested explicitly and requires a
token, and in that configuration the command prints a URL built from a reachable address rather
than `127.0.0.1`, so it can be pasted into a browser on another machine.

Two existing checks constrain what that address may be, and both must be widened deliberately
rather than relaxed:

- `is_local_origin` (`crates/biorouter-server/src/routes/mod.rs`) accepts only
  `http://localhost` and `http://127.0.0.1` on any port. It backs the daemon's cross-origin
  policy, and nothing else.
- The WebSocket routes carry their own origin checks, for cross-site WebSocket hijacking.

Same-origin requests are unaffected by the first — a browser does not apply cross-origin rules
to a page talking to its own origin — but the WebSocket origin gates are explicit checks in
handler code, so both were taught the daemon's own serving origin.

The rule is `origin_matches_host`: the request's `Origin` must match its own `Host` in scheme,
host and port. That is a same-origin test rather than a widening — the browser sets both headers
and neither is reachable from script, so a page on any other origin cannot make them agree. It
needs no configuration and no wildcard, and it holds for every address the interface is reached
at, including ones the daemon could not have enumerated because it bound `0.0.0.0`. Both are
compared whole, so a `Host` of `evil.com.attacker.net` does not admit an `Origin` of
`http://evil.com`, and case is normalised once (`WebOrigin`), as RFC 6454 compares origins.

The scheme is the one the client used to reach the daemon. The daemon speaks plain HTTP, so it is
`http` unless a reverse proxy in front says `X-Forwarded-Proto: https` — the documented way to
put TLS in front of `serve`.

That header is taken on trust, and the reason is a property of the **client** rather than of the
daemon, which is worth stating because the rate-limit key and the CORS predicate both refuse to
trust `X-Forwarded-For`. They are not the same question: `X-Forwarded-For` is the only evidence of
who a caller is, so forging it buys an attacker someone else's identity, while this header only
decides how an `Origin` is compared to a `Host` — and that comparison exists solely to constrain a
**browser** page on another origin, which cannot set this header on a WebSocket handshake at all. A
client that can set it is not a browser and gains nothing by it: it may send no `Origin`, which both
gates admit because their token is the authority there. A trusted-proxy allowlist would add
configuration and close nothing. If the origin test ever becomes load-bearing for callers that are
not browsers, that reasoning has to be revisited with it.

When the header arrives with **several** comma-separated values — a proxy that appends rather than
replaces — the daemon reads the **last**, the one written by the proxy nearest to it. Reading the
first would take whatever the client sent: less trustworthy, since a client-written `https` would
then survive a proxy that appends its own `http`, and less *available*, since a client-written
`http` in front of a legitimate `https` page resolves to `http` and refuses every WebSocket upgrade
from that deployment. Nearly every proxy replaces the header, where the two readings are the same
value, so this matters only for a chain — and a chain whose outer hop is https should have its inner
proxies preserve the value they are handed.

Until QA-D F7 (2026-09-11) the gates also admitted `is_local_origin` — any loopback port, any
scheme — so every other local page's socket passed as the daemon's own, and behind a TLS proxy a
plain-`http` page at the same host passed the authority-only comparison. What remains beside the
same-origin test is one **declared** renderer, and a `serve` daemon declares none. The desktop
app's dev renderer is vite's page on its own port, so the Electron main process names that origin
in `BIOROUTER_RENDERER_ORIGIN` when it spawns the daemon; packaged, it loads from a `file:` URL,
whose WebSocket `Origin` is the literal `file://`, and it declares that instead. Anything else in
that variable is refused with a warning.

`file://` is matched by name, because it has no host and no port for a same-origin test to
compare — which is why it has to be declared to be admitted at all. It was not: the workspace gate
took that literal on every daemon, so a local `.html` opened in Chromium presents exactly that
origin and cleared the gate on a `serve` host, leaving the `?secret=` query token as the whole
authority on a path that is also exempt from `check_token`. `serve` now strips the variable from
the daemon it spawns, so the allowance cannot be inherited from whoever's shell it ran in. The
per-app agent socket has no such allowance and needs none — an app's page is served by this
daemon over http, so it is same-origin with its own socket.

## How `serve` starts and stops the daemon

`biorouter serve` spawns `biorouterd agent` rather than running the server itself (SD-7), so it
is a supervisor, and the half of supervision that matters is stopping. The daemon, not `serve`,
holds the port, answers the browser token and serves the shell that carries its secret — so for
as long as it runs, the URL `serve` printed works. The daemon has to stop whenever `serve` does,
and two layers see to that:

| How `serve` ends | What stops the daemon |
|---|---|
| `Ctrl-C`, or `SIGTERM` (`kill <pid>`, `systemctl stop`) | `serve` sends the daemon `SIGTERM`, waits up to ten seconds, then kills it, and reaps it before exiting. A second request skips the wait. |
| The daemon exits, or never becomes ready | The same path, with nothing or less to stop. |
| `serve` is killed outright (`SIGKILL`), or crashes | On Unix the daemon was started with `--exit-with-parent <pid of serve>`. It sees within half a second that its parent has changed and shuts itself down, exiting regardless ten seconds later. |

The listeners for the first row are installed **before** the daemon is spawned. Installing one
replaces the default action, which for `SIGTERM` was to end `serve` on the spot — so a signal
that arrives during the readiness wait is held until it is read, not lost with the daemon still
running.

Three details are deliberate:

- **The ten seconds exist because a graceful shutdown waits for open connections to finish**,
  and a browser tab left open holds some that never do. The daemon applies the same figure to
  itself when it is orphaned, because then nobody is left to escalate. Measured with one
  request in flight — the renderer's catalog long poll, which an open tab always has parked, for
  up to 25 s — `serve` exited at 10.05 s by killing the daemon; with none, in under 0.1 s. A
  daemon killed that way skips its own cleanup, so a llama-server sidecar it started is left for
  the next launch's pidfile reaper (`llamacpp_sidecar::reap_orphans`).
- **The parent check compares `getppid()` with the pid `serve` named, not with 1.** An orphan is
  re-parented to the nearest *subreaper* — `systemd --user` on most Linux desktops, a
  container's init shim — and to pid 1 only when there is none, so `getppid() == 1` would
  never fire there. And the pid is passed in rather than read by the daemon at startup, because
  a `serve` that died before that read would be recorded as the subreaper that inherited it.
- **The flag is opt-in.** The desktop application starts `biorouterd agent` without it, and so
  does anyone running the daemon by hand.

Windows has neither `SIGTERM` nor the parent check. A console `Ctrl-C` reaches every process
attached to the console, so both stop; `biorouter.exe` ended any other way leaves the daemon
running.

Until 2026-09 neither layer existed, although a comment in `serve` said the first did. The
daemon's `Child` had been moved into the task waiting on it, so the `Ctrl-C` handler held no
handle to kill it with; only a terminal's `Ctrl-C`, which signals the whole foreground process
group, ever reached the daemon. `kill <pid of serve>` from anywhere else left it running with the
port, the token and the secret. `crates/biorouter-cli/tests/serve_lifecycle.rs` stops `serve` by
pid with each signal and asserts the daemon is gone and the port is closed.

## What is deleted

The `biorouter-headless` crate goes entirely (SD-6). Of its two thousand lines, the parts with no
successor are:

- the reverse proxy and its two header allowlists;
- `spawn_biorouterd`, the readiness poll, and the child supervision around them;
- the path-prefix rewriting machinery — the HTML shell rewrite, the per-asset JavaScript and
  stylesheet rewrites, and the routes registered to serve the rewritten copies (SD-4);
- the cloud-metadata probes performed on every start.

What moves rather than dies is the `/headless/*` surface — sixteen paths, seventeen handlers,
since `/headless/settings` answers both `GET` and `POST` — which becomes a route module in the
daemon, and the resolution of where the web directory lives.

## Where the bundle comes from

The bundle the daemon serves is built by Vite with the default root base. That build already
exists — `scripts/build-headless-linux.sh` runs it inside a container — and becomes an ordinary
package script so every platform's packaging can call it.

> **Warning.** Electron Forge forces a *relative* base for the bundle it packages. A relative-base
> bundle served at the root resolves its assets against the current path, so a deep link breaks
> while the landing page appears to work. The bundle the daemon serves must be built with the
> root base, from `vite.renderer.config.mts` directly. Reusing the packaged Electron bundle is not
> a shortcut; it is a different artifact.

A directory named explicitly — `--web-dir`, or else `BIOROUTER_SERVE_UI` — is used as given or
refused with the same error, never skipped. The variable used to be only the first candidate of
the search, so one naming an empty directory was passed over and `serve` served whatever the
search found next, while the same path given as `--web-dir` was fatal. With neither set, the
resolver looks in a fixed order — locations relative to the executable, then a system-wide path
for the Linux packages — and when it finds none, the error names every path it tried.

## Related documentation

- [Reaching a private chat from a script](programmatic-session-access.md) — what `X-Secret-Key` does *not* prove, and the capability header a private session additionally requires.
- [Decisions behind `biorouter serve`](serve-decisions.md) — why each of the above was chosen.
- [Browser access](browser-access.md) — using the command.
- [Privacy tiers](../security/privacy-tiers.md) — the classification the serving path must not weaken.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_SERVE_UI` and neighbours.
