# Decisions behind `biorouter serve`

> **What this is.** The decision records governing browser-served Biorouter — why the daemon
> serves the interface itself, why a browser session cannot change its model yet starts every
> chat on the one the operator chose, why the standalone `biorouter-headless` binary was
> retired, and how long the launch token stays good for. Each record states the ruling, the
> alternatives it displaced, and the consequence a future change would have to accept.
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
must say so before the user reaches for it (SD-8), and the reason the one model the operator
chose must not need that proof at all (SD-12). Read [the architecture](serve-architecture.md)
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

> ⚠ **The first half of that sentence was unreachable until SD-12.** A `serve` daemon holds no
> user-action key, and the new-chat bind asked for that key's proof before binding a private
> model — so with an institutional model configured, no chat could be started at all, and the
> interface showed nothing (the 2026-09-10 QA run, finding F1). See
> [SD-12](#sd-12--a-new-chat-starts-on-the-operators-model-without-a-proof-and-nothing-else-does).

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

## SD-12 — A new chat starts on the operator's model without a proof, and nothing else does

**Ruling.** On a daemon that holds no user-action key — the one `biorouter serve` starts (SD-7),
or a `biorouterd` started by hand — `POST /agent/start` binds the operator's configured provider
to the new chat without asking for proof of a person, whether that provider is public or private.
Three things hold beside it:

- On that daemon, `POST /agent/update_provider` refuses every move onto a private model, whatever
  the chat runs on now. The configured model is the only private model a chat there can reach.
- A daemon that holds a key — the desktop application's — is unchanged. Its renderer sends the
  proof on every start, and a start that lacks it is refused as before.
- The browser interface states the host's configured model on the requests that reach into a chat
  (`X-Caller-Provider`), the way `biorouter session` already does from a terminal, so a chat its
  first reply made private stays reachable from the tab that started it.

**Why.** The configured model is the person's decision, made out of band. `/agent/start` names no
provider: it binds `BIOROUTER_PROVIDER`, which only a proven person may write over HTTP, or which
the operator wrote at the terminal with `biorouter configure`. Open question 24 of the privacy plan
already put the raise at that write — *a raise of every future session* — and SD-1 already says the
tier that choice implies *holds for every session in that daemon*. A new chat taking that tier is
the choice being honored, not a switch. DR-16 governs raising a chat that exists, and a chat that
did not exist a moment ago has nothing to raise.

On a daemon with no key, asking for the proof can only refuse everyone. The 2026-09-10 QA run
measured it: with an institutional model configured, every new chat on a `serve` daemon was
refused 409, in a sentence written for a model that pointed at a model picker SD-1 disables, and
the interface showed nothing at all. A control nobody can pass is not a boundary; it is the
product not working.

The other two halves close what the exemption would otherwise open. Without the rule on
`/agent/update_provider`, a chat bound to the private default could be moved to a different private
model — `Private → Private`, which DR-16's raise predicate calls sideways and allows — that nobody
configured. Without the capability statement, the tab that started a chat lost it after one reply:
the reply makes the chat private (the classification ratchets on the turn, never on the bind), and a
keyless daemon reaches a private chat only for a caller whose stated capability covers it. Measured:
the chat's next request answered 403 with nothing stated, and 200 with the host's provider stated.

**Why not on every daemon.** On a daemon that holds a key the proof costs the person nothing — the
renderer attaches it to every start — and it still refuses a caller that cannot present it. That
includes a model holding the daemon secret, which AR-11 found recoverable and which could otherwise
mint a private-capability chat through `/agent/start` with an extension set of its own choosing.
Relaxing the gate there buys the person nothing and gives that model something.

**Who can do this, and what else reaches the same place.** The two questions every privacy
control answers in writing ([privacy tiers §3.1](../security/privacy-tiers.md)):

- *Who can initiate it.* On a keyless daemon, anything holding the daemon secret: the person in
  the browser and, indistinguishably, a model running in a chat on that daemon that has recovered
  the secret. Both get the configured model and nothing else.
- *What else reaches a chat running on the configured private model:*

  | Door | Proof asked | Changed here |
  |---|---|---|
  | `workspace_open { new: … }` — binds the machine default through `restore_provider_from_session` | None, on every daemon (privacy tiers, "Did not ship") | No |
  | `POST /agent/restart` on a row that names no provider — `restore_provider_from_session` falls back to the configured default | None | No |
  | An app session's creation bind (DR-21) | None, deliberately | No |
  | `POST /agent/update_provider` onto a private model | The proof; on a keyless daemon, refused outright | Yes |
  | `POST /config/set_provider`, and `/config/upsert` or `/config/remove` on a capability key | The proof (SD-1, open question 24) | No — the configured model stays the operator's to choose |

**Displaced alternatives.**

- *Keep the refusal, and explain it in the interface.* Rejected. SD-8's explanation is for a
  control that can never work; this one is the product's core. A `serve` deployment whose only
  model is institutional would be a chat application that cannot chat.
- *Exempt every new chat, whatever provider it asks for.* Rejected. The operator's choice is what
  makes the bind legitimate, so a provider the request picked would be a switch. `/agent/start`
  names none today, and a field that ever let it name one must not inherit this exemption.
- *Exempt the configured model on every daemon.* Rejected; see *Why not on every daemon*.
- *Let a keyless daemon treat its configured model as the capability of any request that states
  none.* Rejected. An absent header resolving to Public is the fail-safe the reach gate is built on,
  and a default that raised it would speak for every caller rather than for the client that says
  what it runs.

**Consequence to accept.** On a keyless daemon whose configured model is private, a model running
in a chat on that daemon — a public-model chat resumed from the shared session store — that has
recovered the daemon secret can start a private-capability chat through `/agent/start` with
extensions it chose, and can reach private chats by stating the host's provider. It could already
do the first through `workspace_open { new }`, and the second by spelling a provider name (the
header is not authentication, as `session_reach.rs` records); and the filesystem read-deny that
would stop it carrying anything back out did not ship. It is recorded rather than closed: on a
daemon that cannot tell a person from a model, closing it means refusing the person.

And one visible change: a browser tab on a host configured with a private model now opens private
chats started in the desktop application on the same machine, which it was refused before. That is
the reach rule — *the caller's capability must be at least the chat's classification* — admitting
it, exactly as it admits `biorouter session` configured with the same model. On a host configured
with a public model nothing changes, and private chats stay out of the browser's reach.

---

## Related documentation

- [Architecture of the serving path](serve-architecture.md) — how the decisions above are built.
- [Browser access](browser-access.md) — the user-facing guide to `biorouter serve`.
- [Privacy tiers](../security/privacy-tiers.md) — the classification system SD-1 protects.
- [Environment variables](../configuration/environment-variables.md) — the settings the daemon
  and the command read.
