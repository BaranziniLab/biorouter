# Decisions behind `biorouter serve`

> **What this is.** The decision records governing browser-served Biorouter — why the daemon
> serves the interface itself, why a browser session cannot change its model, why the
> standalone `biorouter-headless` binary was retired, how long the launch token stays good for,
> and which chats the deprecated `biorouter web` may still open. Each record states the ruling,
> the alternatives it displaced, and the consequence a future change would have to accept.
> **Status:** Current.
> **Audience:** developers working on the daemon, the CLI, or release packaging; agents making
> changes anywhere near the serving path.

Biorouter can be reached from an ordinary web browser: `biorouter serve` starts the daemon,
hands it the built interface, and prints a URL. Before this existed, the same job was done by a
separate `biorouter-headless` binary that spawned the daemon and reverse-proxied it — shipped
only as a Linux tarball, and only to a provisioned server.

This page records the decisions that replaced that arrangement. They were taken together, and
several of them only make sense as a set: the reason a browser session cannot switch models
(SD-1) is also the reason it needs no proof-of-user mechanism, which is the reason the daemon
can be spawned with a closed stdin (SD-7) — and the reason every control that needs that proof
must say so before the user reaches for it (SD-8). Read [the architecture](serve-architecture.md)
for how the result is built, and [browser access](browser-access.md) for how to use it.

Records are identified `SD-n` — *serve decision*. The numbering is stable; a superseded record
keeps its number and says what replaced it.

---

## SD-1 — A browser session cannot change its model or provider, and that is the point

**Ruling.** `POST /config/set_provider` (`set_config_provider` in
`crates/biorouter-server/src/routes/config_management.rs`) continues to refuse a request that
carries no proof a human made it. Browser-served Biorouter installs no such proof. A browser
session therefore runs whatever provider and model the machine was already configured with, and
the model picker is inert.

> **Note.** Until 2026-09 this record named the route `POST /config/provider`. No such route
> exists, so an audit of SD-1 that followed the old text measured a 404 and could read it as
> "no gate". The gate is on `/config/set_provider`, which answers a browser session with 409.

**Why.** This looks like a missing feature and is actually the privacy boundary holding. The
privacy tier system (issue #56) classifies a conversation by the sensitivity of what it has
touched, and a session that has reached a private model may never later reach a public one. The
guarantee rests on knowing which model a conversation ran against. If a browser tab could
rebind the provider mid-session, that knowledge would depend on a control living in page
JavaScript — reachable by anything running in the page, which is precisely what the desktop
app's proof-of-user mechanism (DR-16) exists to prevent.

Fixing the refusal would have meant minting a user-action key somewhere a browser could reach
it. Leaving it in place means the operator chooses the provider once, at the terminal, before
anyone opens a tab — and the tier that choice implies holds for every session in that daemon.
A run started against an institutional Bedrock model is private for its whole life; one started
against a commercial model is public for its whole life. Neither can drift.

**Displaced alternatives.**

- *Mint a digest scoped to a loopback bind.* Rejected: it makes the guarantee depend on the
  bind address, so the same code path is safe or unsafe according to a flag, and the failure is
  silent.
- *A per-tab key.* Rejected: it puts the key in the page, which is the thing being avoided.
- *Pre-seed providers out of band and accept the refusal.* Adopted — this is the ruling.

**Consequence to accept.** The interface must explain the refusal rather than appear broken. A
disabled picker with a reason is the requirement; a 409 toast is not.

> **Why this closes an open question.** `docs/security/privacy-tiers-execution-plan.md` Open
> Question 23 left the headless case explicitly unruled, noting that such a deployment "has no
> GUI, so there is no process that can mint a key on the user's behalf." SD-1 answers it: none
> is minted, and the capability is withdrawn rather than approximated.

---

## SD-2 — Loopback by default; reaching it from another machine is an explicit act

**Ruling.** `biorouter serve` binds `127.0.0.1` unless told otherwise. A non-loopback bind must
be requested explicitly, and when it is, a browser token is mandatory rather than optional — the
command refuses to start without one.

**Why.** The default is the case that needs no threat model: a browser and a daemon on one
machine, reachable by nobody else. The remote case is real and supported, but it is a different
posture — the port is exposed to whatever network the interface is on — and the person taking it
should have said so.

Making the token mandatory on that path rather than merely available is the load-bearing half.
An optional credential on an exposed port is not a credential; it is a setting nobody changed.

**Consequence to accept.** There is no configuration that exposes the port without a token. An
operator who wants an unauthenticated LAN service has to put something else in front of it, and
that is the correct amount of friction.

---

## SD-3 — The daemon serves the interface; the reverse proxy is deleted

**Ruling.** `biorouterd` gains the ability to serve the built interface directly. The
reverse-proxying front door is removed, not fixed.

**Why.** Same-origin is not a tidiness argument, it is what makes three separate problems stop
existing:

- **WebSockets work.** `/ui/workspace` and `/apps/{id}/agent` are daemon routes. Reached
  same-origin they need no proxy support at all. Through the old proxy they could not work —
  the `Connection` header was stripped by the request allowlist, `upgrade()` was never called,
  and the inbound upgrade was never extracted. Any one of those was fatal, and the client
  retried forever rather than reporting it.
- **The credential stops being handled twice.** The proxy injected the daemon secret on behalf
  of whoever connected. With one process there is one secret and one place that checks it.
- **The interface's own endpoints land behind authentication.** They were previously served by
  a router with no auth layer at all.

The change removes more code than it adds: the proxy, the child-process supervisor, the
readiness poll and both header allowlists all go.

**Displaced alternatives.**

- *Keep the proxy, split the crate into a library and a thin binary.* Rejected: it preserves
  every problem above and adds a WebSocket-proxy rewrite plus a deliberate origin relaxation.
  Its one advantage — surviving an unreachable daemon — is worth nothing when the daemon is the
  product.
- *Ship the standalone binary on every platform.* Rejected; see SD-6.

---

## SD-4 — One serving shape: the interface is served at the root

**Ruling.** The interface is served at `/`. Serving it under a path prefix is not supported.

**Why.** The prefix mode existed to sit behind a reverse proxy at a subpath, and it cost a
disproportionate amount: the built bundle bakes root-absolute asset URLs into its JavaScript and
its stylesheets as well as its HTML, so supporting a prefix meant rewriting each of those and
registering per-asset routes for the rewritten copies. That machinery, and the bulk of the tests
around it, exists only to serve the prefix case.

Serving at the root makes the bundle's own URLs correct as built. An operator who needs Biorouter
at a subpath can give it a hostname instead — which is what a browser application with absolute
asset URLs wants anyway.

**Consequence to accept.** This removes a capability. It was reachable only by passing an
explicit flag, and the deployment it was built for disabled the proxy that would have used it.

---

## SD-5 — The verb is `serve`; `headless` remains as an alias

**Ruling.** `biorouter serve` is the command. `biorouter headless` is kept as an alias and
continues to work. Documentation leads with `serve` and mentions the alias once.

**Why.** `serve` says what the command does to someone who has never read this page. `headless`
describes the deployment it came from, and the word already means non-interactive prompt mode
elsewhere in the interface — a second meaning on the same word in the same command surface is a
cost paid by every reader.

Keeping the alias costs one line and protects anyone following existing instructions.

---

## SD-6 — The standalone binary and its Linux tarball are retired

**Ruling.** The `biorouter-headless` crate is removed. The
`biorouter-headless-linux-x64.tar.gz` release asset is removed with it. The release ships ten
assets rather than eleven.

**Why.** Once the daemon serves the interface (SD-3), the standalone binary has no remaining
job: it existed to spawn and proxy a daemon that now needs neither. Keeping it would mean
shipping a third executable in every artifact to duplicate a capability the daemon already has.

The distribution story is simpler as a result, and matches how the product is actually
installed: the desktop application for people who want a desktop application, and the
command-line packages for people who want a server — including the Linux command-line-only
packages, which carry no interface bundle overhead they cannot use.

**Consequence to accept.** The asset count assertion in the release script, the verification
phase, and the download page's filename list all move together. A drop from eleven to ten must
be deliberate in each place, because each is a tripwire designed to catch exactly this.

> **Note.** "Headless" is overloaded in the release scripts and means two unrelated things. The
> `cli-linux` packages are described as headless because they carry no graphical application;
> they are unaffected by this decision and continue to ship. What is retired is the separate
> browser-serving binary.

---

## SD-7 — `serve` spawns the daemon; the command-line interface does not link the server

**Ruling.** `biorouter serve` starts `biorouterd` as a child process. It does not run the
server in-process.

**Why.** `biorouter-cli` deliberately carries no dependency on `biorouter-server`, and the
boundary is documented at the one place that came closest to crossing it
(`crates/biorouter-cli/src/commands/session_watch.rs`, which duplicates a header constant rather
than import it). Running the server in-process would erase that boundary and merge two large
binaries into one.

Spawning also matches what the desktop application already does, so there is one supervision
model rather than two.

**Consequence to accept.** The child's standard input is closed rather than carrying a
proof-of-user digest. Under SD-1 that is the intended configuration, not a limitation — but it
means the daemon a `serve` session talks to is deliberately less capable than the one the
desktop application starts, and anything that assumes otherwise is wrong.

**And the child must never outlive the parent.** The daemon, not `serve`, holds the port,
answers the browser token and serves the shell carrying its secret, so a `serve` that exits
without stopping it has revoked nothing. `serve` therefore stops the daemon on every path it can
run code on, and on Unix starts it with `--exit-with-parent` so that it stops itself on the paths
`serve` cannot — see [how `serve` starts and stops the daemon](serve-architecture.md#how-serve-starts-and-stops-the-daemon).
A comment in `serve` claimed the first half from the start; until 2026-09 neither half was true,
and only a terminal's `Ctrl-C`, which signals the whole process group, ever reached the daemon.

---

## SD-8 — A control that can never work here says so, rather than failing on click

**Ruling.** Everything that depends on the proof-of-user digest SD-7 declines to install must
declare itself unavailable **before** the user acts on it, and the daemon's refusal must be
distinguishable from an ordinary "you did not prove this" refusal.

Concretely: `POST /action-required/tool-confirmation` answers a refused approval with a body
carrying `reason: "unproven"` or `reason: "noKeyInstalled"` and a sentence addressed to a person;
the approval card reads `isBrowserSurface()` up front and renders that sentence in place of its
Allow/Deny buttons; and the agent is not offered tools whose only path runs through such an
approval — the three skill mutations and the extension manager's install and delete are withheld
from the advertised roster when no person is reachable.

**Why.** SD-1 already required that *"the interface must explain the refusal rather than appear
broken"*, and stated it about the model picker. The same argument covers every proof-backed
control, and an approval card is the worst case: three buttons that look live, a bare 403 on
click, and a refusal written for an AI agent. Withholding the tools is the other half — a model
that proposes an install it can never finish has wasted a turn and taught the user that the
feature is broken rather than absent.

**What this is NOT.** It is not a security change. Nothing that was refused becomes permitted;
the gate in `confirm_tool_action` is unchanged in what it allows. This record is about a surface
telling the truth in advance, which is exactly the asymmetry the privacy work rests on: a user
who insists may proceed past a warning, but nothing proceeds automatically.

**Consequence to accept.** The advertised tool roster is now a function of how the daemon was
started, so two daemons on the same machine can offer different tools. The availability flag is
sampled once per roster and threaded, rather than re-read inside each declaration, so a roster
can never half-believe a person is reachable.

---

## SD-9 — The launch token works until the daemon stops; it is not single-use

**Ruling.** `GET /?t=<token>` exchanges the token for the session cookie every time it is
presented, not only the first time. The exchange takes the token out of the address bar; it does
not consume it. The token stops working when the daemon stops — which SD-7 ties to `serve`
stopping — or, for one passed with `--token`, when a different one is passed.

**Why.** The token was first described as "spent on the first request", and that was never true:
the 2026-09-10 QA run redeemed one token four more times after the first and got a 303 each time.
The choice was then whether to make the description true or correct it, and single use cannot be
had without breaking what the product promises:

- **It would be a different mechanism, not an added check.** The session cookie's value *is* the
  token — the daemon compares both against one string — so a "spent" token would still open the
  shell for anyone who set the cookie by hand. Real single use needs a cookie the daemon mints
  and remembers: a session table, emptied by every restart.
- **The supported uses need a second redemption.** A second browser, or a colleague on a shared
  host, where everyone who opens the address is the same user; the same browser after it has
  dropped its session cookie, which carries no expiry and may be discarded when the browser
  closes; and a bookmark of an address fixed with `--token`, which
  [browser access](browser-access.md) offers precisely so that the address survives restarts.
- **Things other than people fetch links.** A browser prefetching a pasted address, or a chat
  client unfurling it, would spend a single-use link before anyone clicked it.

What the exchange is for is keeping the token out of browser history and out of the `Referer` of
everything the page loads afterwards, and the redirect does that whether or not the token is
consumed.

**Displaced alternatives.**

- *Single use, with a session cookie minted by the daemon.* Rejected for the reasons above.
- *Keep the word "spent".* Rejected. In a section about security it reads as single use, and an
  operator who believes a leaked address stopped working after its first use has the wrong
  picture of their exposure.

**Consequence to accept.** The address `serve` prints is a bearer credential for as long as the
daemon runs. Revoking it means stopping `serve` — which is why SD-7 requires that the daemon never
outlive it — and, for an address fixed with `--token`, choosing a new token. Treat it like the
password it is. `the_token_is_not_consumed_by_the_exchange` in `routes::web_ui` pins the
behaviour, so changing it means revisiting this record, not making a quiet fix.

---

## SD-13 — `biorouter web` serves no transcripts, and opens no private chat it did not start

**Ruling (2026-09-11).** The deprecated `biorouter web` command no longer serves
`GET /api/sessions` or `GET /api/sessions/{id}`. The one way into a chat it keeps — a WebSocket
message, which runs a turn in whichever chat it names — is judged before anything touches that
chat. The page is a **public** caller, except in a chat this server started itself through
`GET /`, where it holds the tier of the provider the server was started on. A chat it may not
reach is refused with one sentence, identical for a private chat and for an id that names
nothing.

**Why.** Both routes predate the privacy tiers (issue #56) and never learned them. The list
returned every user and scheduled chat on the machine with its title and working directory; the
transcript route returned any chat's full conversation, private ones included. The only
credential in front of them was the page's own, and it held nothing back:

- **Without `--auth-token`** — the default, and all a loopback bind requires — the auth
  middleware lets every request through, so anything that can reach the port reads every chat.
  A model with a shell does it with `curl`.
- **With `--auth-token`**, the token is a command-line argument. Any process running as the same
  user reads it with `ps -axww -o args` (measured on macOS), and on Linux `/proc/<pid>/cmdline`
  is readable by every user unless `/proc` is mounted with `hidepid`. That is
  [AR-11](../security/privacy-tiers-execution-plan.md#ar-11--amended-by-dr-17--the-daemons-own-api-secret-is-recoverable)'s
  recovery of the daemon's secret, through a channel that is more open than the environment.

The page read the transcript route for a message count and a tab title, and never read the list.
Gating them would have kept two routes nobody needed, so both were deleted.

The WebSocket could not be deleted, because it is the chat; it is gated instead. It was the
larger way in, and it was open as well. A message naming a private chat started anywhere else
ran a turn there — Gate B rebinds the one shared agent to the private model that chat's row
names — and streamed the reply, which can quote the whole conversation, back to whoever held
the socket. That is the daemon's `POST /reply` under another name, and `/reply` heads the
daemon's gated list because it dominates every read route. Deleting the transcript route alone
would have closed the smaller way in and left this one.

**How the page's capability is decided.** Nothing on the socket names the model or the person on
the other end, so the page is a public caller, which is also how the daemon treats a caller that
states no capability. A chat this server started is the exception, reached at the tier of the
provider the server was started on. Without it, a server on a private model would give one reply
per chat: the first reply ratchets the chat to private, and the next message would be refused.
On a public model the exception changes nothing, so a chat this server started that was taken
private somewhere else is refused like any other.

**Displaced alternatives.**

- *Gate the two routes: list public chats only, and refuse a private transcript.* Rejected. It
  keeps a list nothing reads and a transcript the page never showed, and every route kept is one
  more place the reach rule has to be right.
- *Give the page the server's tier for every chat.* Rejected. On a private model, any process
  that can reach the port — a public-model chat's shell included — would reach every private
  chat on the machine without stating anything. The daemon's residual at least requires the
  caller to name a private provider.
- *Refuse every private chat.* Rejected. It breaks the command on the second message of every
  chat for exactly the operator who chose a private model.

**What this is NOT.** It is not authentication. The page's credential is still within any local
process's reach — served to whoever can reach the port without `--auth-token`, read from argv
with it — so a local process can still drive public chats and the chats this server started, as
it could before; issue #47 is unchanged. Nothing that was refused before is permitted now: the
change removes two routes and refuses turns, and grants nothing.

**Consequence to accept.** A chat is known as started here only for the life of the process.
After a restart it counts as started elsewhere, and a private one must be continued in the
desktop app. The page also stops showing "Session resumed: N messages loaded", because that
count came from the transcript route. Implemented in `crates/biorouter-cli/src/commands/web.rs`
(`turn_reach`, `page_capability` and `refuse_turn_unless_reachable`) and pinned by that module's
tests, three of which drive the real router and WebSocket handler over a socket, against a real
session store.

### The same page reflected the URL into script context

**Ruling (2026-09-11).** `GET /session/{name}` no longer writes anything into a `<script>` body.
The two values the page needs to boot — the chat's id and the WebSocket token — are written as
HTML attributes on a `<div id="biorouter-boot">` and read back through `dataset`. The response
carries a `Content-Security-Policy` whose `script-src` is `'self'`, and the template it is built
from carries no inline event handler for that policy to refuse.

**Why.** The handler built the page like this:

```rust
"<script>window.BIOROUTER_SESSION_NAME = '{}'; …</script>", session_name
```

`session_name` is a path segment, so it is whatever the sender typed — behind no credential at
all on the loopback bind that requires none. A `'` ended the string literal and a `</script>`
ended the element. `GET /session/</script><img src=x onerror=…>` was served back as:

```html
<script>window.BIOROUTER_SESSION_NAME = '</script><img src=x onerror=alert(1)>…
```

On this page that is not defacement. The injected script runs on the server's own origin, reads
`data-ws-token` out of the very document it was injected into, opens `/ws` with it, and sends a
message to an agent that holds `developer__shell`. WebSockets are not subject to the same-origin
policy, so that token is the only thing standing between a drive-by page and the socket — and the
injection is handed it. One link the operator clicks is remote code execution as the operator.

**Displaced alternatives.**

- *HTML-escape the value inside the `<script>`.* Rejected, and it is the trap: the HTML parser
  does not decode entities inside `<script>`, so `&lt;/script&gt;` reaches the JavaScript parser
  verbatim and nothing has been neutralised. A fix that looks right and is not.
- *Serialize it as JSON into a `<script type="application/json">` block.* Rejected. `serde_json`
  escapes for JSON, which says nothing about HTML: it leaves `<` and `/` alone, so a value
  holding `</script` still ends the element. It would need a second, HTML-specific escape on top
  — which is the attribute answer with an extra step.
- *Validate the id's shape and 404 anything else.* Rejected as the primary fix. It is a guess
  about a format that has changed before, it would refuse ids this route currently serves, and a
  correct escape does not need it. Nothing stops it being added later as depth.

**What this is NOT.** The `Content-Security-Policy` is depth behind the escape, not the fix. It is
what makes the *next* missed sink on this page inert, and it is why `index.html`'s suggestion
pills bind their handlers in `script.js` — an inline `onclick` is exactly what `script-src 'self'`
refuses, so the two move together or neither does.

**Also fixed, same page, same class.** Four holes in `static/script.js` put model-controlled text
into `innerHTML` unescaped: a tool's name (twice) and a tool call's arguments through
`JSON.stringify` (twice). A prompt injection in a file the agent reads reaches all four. They are
escaped now; `escapeHtml` is adequate for them and only because every one sits in element content
rather than in an attribute value, which it does not escape for.

**Not closed, and a maintainer's call.** `build_cors_layer` allow-lists `http://localhost:3000`
and `http://127.0.0.1:3000` whenever no `--auth-token` is passed, so a page served from port 3000
of the same machine can read `/session/…` cross-origin and lift the WebSocket token out of it.
That is a far higher bar than clicking a link, and removing it may break a frontend-dev workflow
this command was once used for, so it is recorded rather than changed here.

**The standing recommendation.** This command is deprecated in favour of `biorouter serve`, which
serves the real interface. Every hole above lives in a page nothing else uses, and deleting the
command would close all of them permanently and retire this record's whole surface with it.

---

## Related documentation

- [Architecture of the serving path](serve-architecture.md) — how the decisions above are built.
- [Browser access](browser-access.md) — the user-facing guide to `biorouter serve`.
- [Privacy tiers](../security/privacy-tiers.md) — the classification system SD-1 protects.
- [Environment variables](../configuration/environment-variables.md) — the settings the daemon
  and the command read.
