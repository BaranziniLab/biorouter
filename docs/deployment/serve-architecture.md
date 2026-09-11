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

**The interface's own endpoints are authenticated.** The sixteen `/headless/*` endpoints — the
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

- `is_local_origin` (`crates/biorouter-server/src/routes/mod.rs:9`) accepts only
  `http://localhost` and `http://127.0.0.1` on any port. It backs the daemon's cross-origin
  policy.
- The WebSocket routes carry their own origin checks, for cross-site WebSocket hijacking.

Same-origin requests are unaffected by the first — a browser does not apply cross-origin rules
to a page talking to its own origin — but the WebSocket origin gates are explicit checks in
handler code, so both were taught the daemon's own serving origin.

The rule is `origin_matches_host`: the request's `Origin` must equal its own `Host`. That is a
same-origin test rather than a widening — the browser sets both headers and neither is reachable
from script, so a page on any other origin cannot make them agree. It needs no configuration and
no wildcard, and it holds for every address the interface is reached at, including ones the
daemon could not have enumerated because it bound `0.0.0.0`. Both are compared whole, so a `Host`
of `evil.com.attacker.net` does not admit an `Origin` of `http://evil.com`.

## What is deleted

The `biorouter-headless` crate goes entirely (SD-6). Of its two thousand lines, the parts with no
successor are:

- the reverse proxy and its two header allowlists;
- `spawn_biorouterd`, the readiness poll, and the child supervision around them;
- the path-prefix rewriting machinery — the HTML shell rewrite, the per-asset JavaScript and
  stylesheet rewrites, and the routes registered to serve the rewritten copies (SD-4);
- the cloud-metadata probes performed on every start.

What moves rather than dies is the sixteen `/headless/*` handlers, which become a route module in
the daemon, and the resolution of where the web directory lives.

## Where the bundle comes from

The bundle the daemon serves is built by Vite with the default root base. That build already
exists — `scripts/build-headless-linux.sh` runs it inside a container — and becomes an ordinary
package script so every platform's packaging can call it.

> **Warning.** Electron Forge forces a *relative* base for the bundle it packages. A relative-base
> bundle served at the root resolves its assets against the current path, so a deep link breaks
> while the landing page appears to work. The bundle the daemon serves must be built with the
> root base, from `vite.renderer.config.mts` directly. Reusing the packaged Electron bundle is not
> a shortcut; it is a different artifact.

The resolver looks for the directory in a fixed order — an explicit flag or environment variable,
then a location relative to the executable, then a system-wide path for the Linux packages. When
it finds none, the error names every path it tried.

## Related documentation

- [Reaching a private chat from a script](programmatic-session-access.md) — what `X-Secret-Key` does *not* prove, and the capability header a private session additionally requires.
- [Decisions behind `biorouter serve`](serve-decisions.md) — why each of the above was chosen.
- [Browser access](browser-access.md) — using the command.
- [Privacy tiers](../security/privacy-tiers.md) — the classification the serving path must not weaken.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_SERVE_UI` and neighbours.
