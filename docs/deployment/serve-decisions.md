# Decisions behind `biorouter serve`

> **What this is.** The decision records governing browser-served Biorouter — why the daemon
> serves the interface itself, why a browser session cannot change its model yet starts every
> chat on the one the operator chose, why the standalone `biorouter-headless` binary was
> retired, how long the launch token stays good for, and which chats the deprecated
> `biorouter web` may still open. Each record states the ruling, the alternatives it
> displaced, and the consequence a future change would have to accept.
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
must say so before the user reaches for it (SD-8), the reason the controls that stop and settle
a turn answer to the reach gate there instead (SD-11), and the reason the one model the
operator chose must not need that proof at all (SD-12). Read [the architecture](serve-architecture.md)
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

**A delegated subagent's tab is the same case** (2026-09-11; SD-11 recorded it as open). A
subagent's chat is where the proof decides everything — a message there is recorded as a person
intervening, and the parent is told so — and the daemon refuses these writes to it from a caller
that cannot prove a person acted: `POST /reply`, `POST /agent/cancel` and the two continuation
routes SD-11 admits elsewhere, `POST /interrupt` (which asks for the proof on every daemon, so it
refuses here for its own reason rather than SD-11's), `POST /agent/stop`, the extension routes,
`POST /agent/update_working_dir`, and `POST /agent/resume` itself. On a `serve` daemon that is every caller, so from the tab's own controls
there is nothing to do but read.

⚠ **That is an enumeration, and it must not be read as "every write".** It said "every write" for
one day and was wrong on its own terms: `POST /agent/update_working_dir` — which repoints a chat at
a directory of the caller's choosing and restarts its agent there — consulted only
[`session_reach`](../security/privacy-tiers.md), the privacy gate, which is *deliberately inert for
a public session*, and a delegated child's chat is normally public. It is on the list above because
it was gated (2026-09-12); the claim is stated as a list because the sweep that found it found
eight more session-addressing writes that reach a child's row and ask nothing about it — `DELETE
/sessions/{id}` (which cancels the child's in-flight turn before deleting, so it is a Stop by
another name), `PUT /sessions/{id}/name`, `PUT /sessions/{id}/user_workflow_values`, `POST
/sessions/{id}/edit_message`, `POST /sessions/{id}/diverge`, `POST /agent/call_tool` and `POST
/agent/read_resource`. All predate this record. What SD-8 is about is the tab's own controls; a
sentence claiming the API surface as a whole is closed would be [#47](https://github.com/BaranziniLab/biorouter/issues/47)'s
claim to make, and #47 is open.

**`GET /agent/callable_tool_count` was gated in the same pass**, and for a reason worth separating
from the tab: the renderer stopped *calling* it for a subagent's chat (below), and a client that
avoids an ungated route leaves it ungated. It answers through `get_or_create_agent`, so an unproven
caller naming any chat could mint an agent for it, and the route's own 424 would then report what
it had found. It now consults `session_reach` before the agent is fetched, like `GET
/sessions/{id}` and `POST /agent/resume`.

⚠ **The two 403 bodies differ, and that is recorded rather than smoothed over.** A public
subagent's chat is told it is a subagent; an id that does not exist is told only that it is out of
reach. In isolation the pair is an existence oracle for subagent ids. It is dominated, and the
measurement is in
`routes::agent::resume_update_security_tests::a_private_subagent_and_an_unknown_id_are_refused_in_the_same_words`:
for a **private** subagent the two refusals are byte-identical, because `session_reach` fires first
and its one sentence answers "private" and "no such chat" alike; and for a **public** one the same
unproven caller is answered **200** by `/agent/resume` on an ordinary public chat and **200** by
`GET /sessions/{id}` on the subagent's, `session_type` included — so the body discloses nothing the
route next door does not hand over outright. ⚠ With the privacy master switch **OFF**
`session_reach` returns `Ok` before its store read, so the private row joins the public one and the
pair separates for every chat on the machine. That is the switch's pre-existing blast radius
(DR-17), not this route's, and it is not closed here.

It could not even be read. The renderer loaded every chat through `/agent/resume`, so a subagent's
tab rendered *"Could not load this chat"* over the daemon's refusal — including the tab the daemon
itself opens to show a subagent it has just spawned. Measured against a real `biorouter serve`:
`POST /agent/resume` answered 403 for the child while `GET /sessions/{id}` and
`GET /sessions/{id}/events` answered 200 for the same chat. In a browser the tab now loads through
those two reads, and never asks for the agent — not `/agent/resume` again, not the rejoin (which
re-POSTs `/reply`), and nothing that reads AGENT state, because `/agent/callable_tool_count`
answers through `get_or_create_agent` and would mint a bare placeholder agent under the child's
session id. A note takes the composer's place, one line takes the header Stop's, and the
transcript's "still working" nudge stops pointing at a composer that is not there
(`ui/desktop/src/components/subagent/subagentReadOnly.ts`).

⚠ **The tab decides this at mount, and the badge the daemon's own workspace frame put on it is
not enough on its own.** In a browser the two reads queue behind the page's open event streams —
six connections per origin, one stream per observed tab — and with a subagent running the refused
resume alone took 4.8 s, with the session read still pending five seconds later. All of that is
the running window, which is exactly when the ordinary composer was offering a Stop that could
only be refused. The badge is the one source known at mount, so it was added first; but
`tabAnnotations` is ordinary renderer state written only from live daemon frames, while the tab
LAYOUT is persisted per window — so a page **reload** restores a subagent's tab with no badge, and
a tab reached from History never had one. Every source then read "no", and "no" mounted the
composer: the decision **failed open on reload**, which is the opposite of SD-8's promise.

So the decision has **three** states, not two (`subagentComposerKind`): a subagent's chat, a chat
that is definitely not one, and *not yet known*. A boolean reported the third as the second. In a
browser the third **withholds** the composer rather than mounting one that may have to be taken
back; on the desktop, which holds the key, it changes nothing. The cost is close to invisible,
because the transcript of a browser chat does not paint until that same read lands either — and
the two states that could turn withholding into a lockout are resolved deliberately: a tab with no
session id yet (the empty tab before a first message) and a chat the store could not load at all
both count as "not a subagent".

**Two writes the missing composer never reached**, both inside the transcript rather than under it,
and so both still live on a read-only tab until they were withheld by the same flag: an
**elicitation card**, which posts its answer through `/reply` exactly as the composer does, and
**artifact auto-repair**, which needs no click at all — a figure that fails to render is the
trigger, and `shouldAutoRepairArtifact` is satisfied by a subagent's chat because that chat really
is live. Both are withheld by passing no callback, which is how the read-only transcript surfaces
already withhold them: `BioRouterMessage` renders the elicitation form only when handed a submit
callback, and `ArtifactViewer` installs its `postMessage` listener only when handed `onRenderError`.

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
stopping — or, for a token the operator chose (`--token`, or `BIOROUTER_BROWSER_TOKEN` in the
environment `serve` runs in), when a different one is chosen.

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
  closes; and a bookmark of an address fixed with `--token` or `BIOROUTER_BROWSER_TOKEN`, which
  [browser access](browser-access.md) offers precisely so that the address survives restarts.
  A service unit is the case that needs the variable rather than the flag: nobody is watching its
  terminal for a new token, and a flag on `ExecStart` is visible in `ps` to every user on the
  host. ⚠ `serve` ignored that variable and minted a token over it until 2026-09-12, which made
  the systemd recipe in [headless Linux](headless-linux.md) unusable as written; honouring it
  needed no new ruling, because a fixed operator-chosen token is the shape this record already
  provides for.
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
outlive it — and, for an address fixed by the operator, choosing a new token and restarting. Treat it like the
password it is. `the_token_is_not_consumed_by_the_exchange` in `routes::web_ui` pins the
behaviour, so changing it means revisiting this record, not making a quiet fix.

---

## SD-10 — The served interface keeps its operator's reach on listings and knowledge bases, and gains nothing else

**Ruling (2026-09-11).** Since the privacy fix for QA findings H2 and M1 (2026-09-10), every
daemon route that lists chats, or names, lists or reads a knowledge base, answers a caller that
holds only the daemon secret as a **public model**: private chats are left out of lists, and a
private knowledge base is refused. The desktop application is told apart by the proof-of-user
header it sends. A `serve` daemon holds no such proof (SD-7), so it recognises its **own
interface** another way. A request that carries the served document's session cookie, on a daemon
started with a browser token, is given the tier implied by the provider the operator configured
(SD-1). That tier is read once at launch: the declared tier of the configured provider, reduced with
`least` over a configured lead provider, which is the reduction a bound lead/worker pair gets.

- On a **private** provider (institution-hosted, or local), the History list and the Knowledge
  view show private chats and knowledge bases, as they did before the fix.
- On a **public** provider they show public ones only. That is also what any caller holding just
  the secret sees.

**Why.** SD-1 already makes every session in a `serve` daemon run on the operator's provider, so
that provider's tier is the only capability the interface can be said to have. The cookie is what
separates the interface from anything else holding the secret. Without it the fix would have had
to go one of two ways, and both are wrong. One strips an operator on a private provider of their
own history and knowledge, which is a hard regression. The other hands every holder of the secret
the operator's reach, which reopens H2 on every `serve` daemon.

**What it does not do.**

- **It reaches no private transcript.** The transcript gate, and every route that names one chat
  (open, export, the live event stream, delete, rename, and the rest), never read this standing.
  They judge a `serve` browser exactly as they judged it before this ruling: on the proof it
  carries, which is none (SD-7), and on the capability it states with `X-Caller-Provider`, which
  they judge as they judge any caller's. An interface that states no capability — the case this
  ruling was written against — sees private chats in its History list that it cannot open, delete
  or rename. That is SD-7's limitation, left where this ruling found it, and it keeps deleting a
  chat from ever being easier than reading it. Letting the transcript gate honour the cookie
  itself would be the first time a gate widened. It is an **open decision**, recorded here and not
  taken.
- **It is not authentication, and not a proof of a person.** `biorouter serve` passes both the
  secret and the browser token in the daemon's environment. A caller that can read one can read
  the other, which is the residual the `X-Caller-Provider` header already carries
  ([issue #47](https://github.com/BaranziniLab/biorouter/issues/47)). It never satisfies a
  proof-of-user check, so SD-1 and SD-8 stand exactly as they were.
- **It follows the address, not a person.** The token is not single-use (SD-9), so every browser
  that opens the address `serve` printed gets this standing: a second browser, a colleague on a
  shared host, a bookmark of an address fixed with `--token`. That is no more than the address
  gave before this ruling, when these listings and knowledge-base routes were open to any caller
  holding the secret the served document carries. Revoking the standing means what revoking the
  address means: stopping `serve`.
- **A `--no-token` daemon gives it to nobody.** Without a token there is no cookie, and the
  interface cannot be told apart from any other local caller. Such a daemon shows public chats and
  knowledge bases only.
- **It creates no cross-site request forgery surface.** The cookie is `SameSite=Strict`, so no
  cross-site request carries it, and every API request still needs `X-Secret-Key` to reach this
  standing at all. It can only narrow a caller that already holds the secret, never admit one that
  does not.

**Consequence to accept.** Two `serve` daemons on one machine, configured with providers of
different tiers, show different subsets of one shared history and knowledge store. That follows
from SD-1, which already made the provider a property of the daemon rather than of the tab.

Implemented in `crates/biorouter-server/src/auth.rs` (`install_served_operator`,
`served_operator_capability`) and `routes::session_reach::HttpCaller`. Pinned in two places,
each asserting both halves — the interface keeps its listing and knowledge-base reach, and the
cookie gains it no transcript: `a_served_interface_keeps_its_listing_reach_and_gains_no_transcript`
in `routes::session_reach`'s lib tests, which is the copy CI runs, and
`crates/biorouter-server/tests/serve_operator_reach.rs`, which adds the keyless arm — a daemon with
no user-action key, as `serve` really starts it.

---

## SD-11 — Stop works on a daemon with no key; steering does not, and a subagent's tab stays the person's

**Ruling.** On a daemon that holds no user-action key — the one `biorouter serve` starts (SD-7), or
a `biorouterd` started by hand — **three** of the four routes that control a running turn admit
exactly the callers `POST /agent/stop` already admits there:

| Route | What it does |
|---|---|
| `POST /agent/cancel` | Stop, and the first half of Stop-and-Send. |
| `POST /agent/continuation/abandon` | Gives up a Stop-and-Send replacement. |
| `POST /agent/continuation/recover` | Takes a Stop-and-Send replacement back after a reload, or gives it up. |

That gate is `authorize_agent_control` in `routes/agent.rs`, called by name rather than written
again: the reach rule — every public chat, and a private one only for a caller whose stated
capability covers it — and then no subagent's chat.

**`POST /interrupt` — mid-turn steering — is excluded, and refuses in words.** It asks for the
user-action proof on **both** kinds of daemon, so on a keyless one it is unavailable to everybody,
the person at the browser included. Its refusal is a `403` carrying
`reply.rs::STEER_NO_KEY`, a sentence naming this daemon as what cannot check a proof rather than
telling a person to go and prove they are one (SD-8) — and never an empty body, because an empty
turn-control `403` is how `biorouter session attach` recognises a daemon that *does* hold a key and
decides to ask the person for it. *Why* is the section below.

Two things hold beside all four:

- A daemon that holds a key — the desktop application's — is unchanged. These routes take the proof
  there and nothing else, and a steer admitted there is stamped `UserDirect`, exactly as before.
- A subagent's chat is refused on a keyless daemon, as `/reply` and `/agent/stop` already refuse it
  there, and the refusal now names the daemon, not the caller, as what cannot prove a person acted
  (SD-8).

**Why.** The four routes asked for proof of a person before anything else, and on a daemon that
holds no key that proof can only refuse everyone, the person at the browser included. Measured on
2026-09-11 against a real `biorouter serve`, from the page, with the daemon secret and the host's
provider stated: `POST /agent/cancel` and `POST /interrupt` both answered `403` with an empty body.
The Stop button could stop nothing on any `serve` host, including one configured with a public
model. `crates/biorouter-server/tests/turn_control_no_user_key.rs` reproduces it on the code this
record changed: on a keyless daemon and an ordinary public chat, `/agent/stop` answered `200` and
cancelled the turn while `/agent/cancel` answered `403`. Stop is the control that ends a runaway
turn, and a chat that cannot be stopped from the only interface it has is less safe, not more.

And the refusal protected nothing. The proof is on these routes so that a model holding the daemon
secret, which AR-11 found recoverable, cannot stop another chat's turn or put words in a person's
mouth. On a keyless daemon that caller already does each of those next door:

| What the refusal withheld | Where the same caller already does it on a keyless daemon | Guarded by |
|---|---|---|
| Cancel the running turn of a chat it can reach | `POST /agent/stop`, which trips the same turn and evicts its agent besides | `authorize_agent_control` — the gate this record adopts |
| The same, as a model holding no secret at all | `workspace_close { scope: "turn" }` | `refuse_unless_writable`: the tier, and nothing else |
| Put text in front of the chat's model as the user, **before or after a turn** | `POST /reply`, which a non-subagent chat accepts, unstamped, from any caller that reaches it | `session_reach` |
| Give up a chat's pending Stop-and-Send | `workspace_close { scope: "turn" }`, which abandons every pending continuation of the chat it names | `refuse_unless_writable` |

So admitting a caller to those three gives nothing that holds the secret a capability it lacked.
The person gains the Stop button and Stop-and-Send, which the desktop application has always had.

⚠ **The third row of that table is the whole argument for the steer, and it does not hold.**
`POST /reply` takes the BR-33 single-turn lock and answers `409 CONFLICT` whenever a *different*
turn is already running in that chat; `POST /interrupt` is meaningful only while one is, and
answers `409` when none is. The two preconditions are **disjoint**: in the exact state where a
steer lands, the route said to dominate it is refused. So admitting the steer would hand a caller
holding only the daemon secret something genuinely new — attacker-chosen text injected into a turn
already in flight, *without cancelling it*, which the person watching sees as their own turn
changing direction. The nearest thing that caller already has is cancel-then-reply, and that is
**visible**: the turn dies first. It is not a tier crossing — `session_reach` still refuses a
private chat to a public caller, and the subagent rule still refuses every child's chat — but it is
a capability asymmetry, and the dominance argument is the only thing this record had to offer for
it. Hence the exclusion. (Found in the security review of this change, before it merged;
`reply.rs::steer_refusal` carries the same reasoning at the code.)

**Why the other three routes move together.** Stop-and-Send cancels with a continuation, and the
continuation mints a lease that holds the chat for its replacement: until the lease is used or
given up, every other turn in that chat is refused. A daemon that let the cancel through and
refused the abandon or the recover would wedge the chat the first time a person removed a queued
message or reloaded the page. The test binary above measures both.

**What a browser user loses, and what happens instead.** On a keyless daemon the browser's steer
is refused, and the renderer's `steer()` already treats any refusal as "fall back to an ordinary
send" (`chatStreamStore.tsx`): the text is queued and delivered when the turn ends, rather than
injected into it or lost. That is the cost of the exclusion, and it is a delay rather than a
capability the person no longer has.

**Why not stamp the keyless steer `UserDirect` and admit it?** This was the shape the record took
before the review. `UserDirect` is a claim that a person typed the text, and the subagent machinery
acts on it, so the steer would have had to arrive unstamped — which is what `/reply` gives the same
caller's message. But the stamp was never the problem: the *injection into a running turn* is, and
an unstamped steer still redirects the model mid-flight and still appears in the transcript beside
the person's own messages.

**Why a subagent's tab is not included.** That tab is where `UserDirect` means something, and
`/reply` and `/agent/stop` already refuse an unproven caller there, on every daemon. Admitting turn
control alone would give that tab a Stop that works beside a composer that cannot send, and the
rule for the tab is a decision about the whole tab rather than one of its buttons.

**Why not on every daemon.** On a daemon that holds a key the proof costs the person nothing — the
renderer attaches it to every request — and it still refuses a caller that cannot present it.
Relaxing it there buys the person nothing, and would admit an unstamped steer where the desktop has
always stamped one.

**Who can do this, and what else reaches the same place.** The two questions every privacy control
answers in writing ([privacy tiers §3.1](../security/privacy-tiers.md#31-the-review-checklist--two-questions-every-control-answers-in-writing)):

- *Who can initiate it.* On a keyless daemon, anything holding the daemon secret: the person in the
  browser and, indistinguishably, a model running in a chat on that daemon that has recovered the
  secret. Both reach the chats the reach gate admits them to, less every subagent's. On a daemon
  that holds a key, only a request carrying the proof.
- *What else reaches the same place:*

  | Other entry point to the same capability | Reachable by | Guarded by | Where that guard is called |
  |---|---|---|---|
  | `POST /agent/stop` — cancels the running turn | user; any holder of the daemon secret | `authorize_agent_control`: reach, then the subagent rule | `routes/agent.rs::stop_agent` |
  | `workspace_close { scope: "turn" \| "agent" }` — cancels the turn, abandons its pending continuations | model | `refuse_unless_writable` (the tier) | `agents/workspace_extension.rs::handle_close` |
  | `POST /active_work/{id}/cancel` — cancels a running subagent or background job by registry id | user; any holder of the daemon secret | **nothing**: the id is not a session id, so the reach gate cannot be applied | `routes/active_work.rs::cancel_active_work`; an open residual in `session_reach.rs` |
  | `POST /reply` — puts the caller's text in front of the chat's model | user; any holder of the daemon secret | `session_reach`, then the subagent rule | `routes/reply.rs::reply` |
  | `workspace_send_prompt { mode: "steer" \| "turn" }` — the same, from another chat | model | `refuse_unless_writable`; the text arrives framed as `AgentInjection`, and a private-to-public write raises a first-crossing approval | `agents/workspace_extension.rs::handle_send_prompt` |
  | `biorouter session cancel`, `attach` and `send` → these routes, and `/reply` | user at a terminal | the routes' own gates: the CLI sends the proof only when the person supplied it or a daemon that holds a key refused without it (see *The terminal, since* below) | `commands/session_watch.rs::with_key_if_wanted` |

  The third row is a finding rather than a guard: a subagent's work can be cancelled on any daemon
  by a caller that holds the secret and reads the id from `GET /active_work`. It is the residual
  `session_reach.rs` already records, and this record does not widen it.

**Displaced alternatives.**

- *Keep the refusal, and explain it before the click (SD-8).* Rejected. SD-8 is for a control whose
  absence is safe to explain; this one is how a person ends a turn they did not want. A Stop button
  that says "unavailable here" is honest and still leaves the turn running.
- *Point the browser's Stop at `/agent/stop`.* Rejected. It cancels whatever turn is running rather
  than the generation the person saw, so a late click can kill a successor; it does not wait for the
  turn to settle and has no Stop-and-Send; and it evicts the agent. It would give the browser a
  blunter Stop than the desktop's, to route around a refusal that guards nothing.
- *Relax the routes on every daemon.* Rejected; see *Why not on every daemon*.
- *Admit `/interrupt` too, with the steer arriving unstamped.* Rejected in review; see the ⚠
  paragraph under *Why* and *Why not stamp the keyless steer `UserDirect` and admit it?*.
- *Admit `/interrupt` and refuse it with an empty `403` like the others.* Rejected. `biorouter
  session attach` reads an empty turn-control `403` as "this daemon holds a key" and prompts the
  person for one; on a keyless daemon that prompt asks for a credential that does not exist and
  then reports it as the wrong key. The refusal carries `STEER_NO_KEY` instead.
- *Admit plain Stop and refuse Stop-and-Send.* Rejected. The queue's "Stop and send" and a typed
  "stop" in a busy composer both cancel with a continuation, so this would leave half of the Stop
  controls refused, and a lease a person can mint must be a lease they can give up.

**Consequence to accept.** On a keyless daemon, a model running in a chat on that daemon that has
recovered the daemon secret can stop or settle a turn in any chat the reach gate admits it to — a
public chat always, and a private one by stating a private provider, which the header does not
authenticate (`session_reach.rs` records as much). It could already stop those turns through
`/agent/stop` or `workspace_close`, and put text in front of those chats through `/reply`; what it
gains over that is the exact generation semantics and the Stop-and-Send lease, not a new reach. It
is recorded rather than closed: on a daemon that cannot tell a person from a model, closing it
means a Stop button nobody can press. **Steering a running turn is the line this stops at**, for
the reason above.

**A daemon that is keyless by accident says so at startup.** Before this record, a desktop daemon
that came up without its key announced itself at the first click: Stop answered `403` and the user
complained. Now Stop works there, so the same misconfiguration is silent — a *weaker* daemon rather
than a visibly broken one. `read_user_action_digest` therefore reports **why** it holds no key
rather than returning one undifferentiated "none": stdin was a terminal, stdin closed with nothing
on it (`serve`'s `Stdio::null()`), a writer held the pipe open and wrote nothing inside the 2 s
bound, or the line was not a 32-byte hex digest. The last two are launcher faults and nothing
Biorouter ships does either on purpose, so their warning says so and says to restart. Every arm
names both consequences — what this daemon refuses, and what it now admits instead. The bound is
unchanged: nothing measured says the desktop launcher misses it, and what was missing was the
report, not the time.

**Not decided here.** An ordinary browser chat's steer control is still offered on a keyless daemon
and still refuses — its text falls back to the send queue, so nothing is lost, but SD-8's rule would
have it say so first. The terminal's half of this was closed since, below.

**Decided since, under SD-8.** This record left a subagent's tab in a browser offering a composer,
a steer and a Stop that all refuse, and said SD-8 required them to say so before the click.
Measuring it found the tab worse off than that — it did not open at all — and the whole case,
with what the interface now does instead, is written up in
[SD-8](#sd-8--a-control-that-can-never-work-here-says-so-rather-than-failing-on-click). Nothing
about the refusals above changed.

**The terminal, since.** `biorouter session cancel`, `attach` and `send` used to demand the
user-action key from the terminal before they sent anything, so against a keyless daemon they
refused locally the requests this record admits: the browser's defect, one client over. A terminal
cannot ask a daemon whether it holds a key, so each command now lets the daemon answer. It sends its
request without the proof, unless the person supplied the key on stdin (`--user-action-key-stdin`),
and asks the person for the key only when the answer is the empty 403 of a daemon that holds one;
then it sends once more, with the key. A refusal that carries a sentence, which is every refusal a
keyless daemon gives on these routes, is printed instead, because no key would change it.

- `attach` asks as it joins, with an empty steer: the gate answers before `/interrupt` reads the
  text, and empty text is refused before anything is touched. It cannot wait for the first real
  steer, because by then stdin carries the person's messages, and a hidden prompt would have to share
  it with them.
- `send` asks the same question after a refused `/reply`, because `/reply`'s own refusal for a
  subagent's session is an empty 403 on either kind of daemon and cannot say which kind this is.

Nothing is relaxed. The daemon stays the boundary, and every refusal the terminal reads is given
before the route touches the turn, so sending the request a second time cannot deliver anything
twice. A subagent's session still needs the proof. The raw key still comes only from the
terminal or from stdin, never from argv, the environment, config or logs, and it is now sent only
when the person supplied it or a daemon asked for it. Keeping the local refusal and rewording it to
say what to do was rejected, because it would ask the person whether the daemon holds a key, and
the daemon answers that itself. The shapes the terminal reads are pinned from the daemon's side, in
`routes::reply`'s keyed tests and in `tests/turn_control_no_user_key.rs`; the reader is
`key_verdict` in `commands/session_watch.rs`.

## SD-12 — A new chat starts on the operator's model without a proof, and nothing else does

**Ruling.** On a daemon that holds no user-action key — the one `biorouter serve` starts (SD-7),
or a `biorouterd` started by hand — `POST /agent/start` binds the operator's configured provider
to the new chat without asking for proof of a person, whether that provider is public or private,
**for as long as that configuration is still the one the daemon was launched with**. Five things
hold beside it:

- **The exemption is pinned to the launch configuration.** A daemon samples the
  capability-deciding configuration once, before any route is mounted, and the exemption applies
  only while the live values still match it. If any of them has moved, the bind is refused with a
  409 that names the key and says to restart the daemon. `biorouter_server::launch`.
- **A daemon that expected a key and did not get one keeps the refusal.** A launcher that hands
  over a digest declares so in the environment (`BIOROUTER_USER_ACTION_EXPECTED`), so
  "no proof can ever exist here" is distinguishable from "the proof went missing". The second is
  a fault to repair, and it keeps the pre-SD-12 behaviour — every private new chat refused, now in
  a sentence that says why — plus a startup `ERROR` naming the consequence.
- On that daemon, `POST /agent/update_provider` refuses every move onto a private model, whatever
  the chat runs on now. The configured model is the only private model a chat there can reach.
- A daemon that holds a key — the desktop application's — is unchanged. Its renderer sends the
  proof on every start, and a start that lacks it is refused as before.
- The browser interface states the host's configured model on the requests that reach into a chat
  (`X-Caller-Provider`), the way `biorouter session` already does from a terminal, so a chat its
  first reply made private stays reachable from the tab that started it.

**Why.** The configured model is the person's decision, made out of band. SD-1 already says the
tier that choice implies *holds for every session in that daemon*, and open question 24 of the
privacy plan already put the raise at the moment the choice is written — *a raise of every future
session*. A new chat taking that tier is the choice being honored, not a switch. DR-16 governs
raising a chat that exists, and a chat that did not exist a moment ago has nothing to raise.

> ⚠ **The first version of this record justified the exemption with a claim the tree contradicts,
> and the correction is the load-bearing part.** It said: *"`/agent/start` names no provider: it
> binds `BIOROUTER_PROVIDER`, which only a proven person may write over HTTP."* The HTTP doors to
> that key really are closed — `/config/set_provider` 409s on a keyless daemon, and `/config/upsert`
> and `/config/remove` both gate `privacy::is_capability_key` behind the proof — but **`config.yaml`
> is not an HTTP resource.** §9.5's general filesystem read/write deny, DR-14, is recorded
> **DEFERRED** in [privacy tiers, *"Did not ship"*](../security/privacy-tiers.md#what-shipped-and-what-did-not);
> the agent holds `developer__shell`; and `Config`'s value cache is keyed on a `FileStamp` it
> re-`stat`s on every read, so the file is reloaded live and the bind reads whatever it says **at
> request time**. A model with a shell on a keyless daemon whose operator had configured a *public*
> default could therefore write a private provider into that file and `POST /agent/start` to get a
> 200 and Private capability — unlocking `chatrecall` over private chats, private knowledge bases
> and an unfiltered Gate E roster with an extension set of its own choosing — where `main` answers
> 409. That is not the operator's choice being honored; it is the door's own text being rewritten by
> the thing the door constrains.
>
> So the exemption is **pinned** rather than re-derived per request. The bind still reads the
> configuration — an operator who edits the file and restarts is served, which is the whole point of
> restarting — but the *exemption* survives only while the capability-deciding values still match
> what this daemon started with. That makes SD-1's sentence true of the door rather than merely
> asserted near it, and it makes a model's ability to write `config.yaml` irrelevant here.

**What is pinned, and why that list.** `privacy::CAPABILITY_CONFIG_KEYS` verbatim —
`BIOROUTER_PROVIDER`, `BIOROUTER_LEAD_MODEL`, `BIOROUTER_LEAD_PROVIDER`, `OLLAMA_HOST`,
`LLAMACPP_EXTERNAL_HOST` — plus `BIOROUTER_MODEL`. Reusing the list `/config/upsert` already guards
is deliberate: `privacy::config_keys`'s scan of the tier-input files is what keeps it honest, so a
key that starts deciding capability is pinned here without anyone remembering to, and a second
hand-written list would be a third answer to a question that already has two agreeing ones.
⚠ **The provider's name alone would not have been enough:** `self_hosted_tier` reads `ollama` as
Private exactly while its host is loopback, so flipping `OLLAMA_HOST` moves the tier with
`BIOROUTER_PROVIDER` untouched — the same escalation through a different key.

`BIOROUTER_MODEL` is pinned for a different reason and is **not** a capability key. No `tier()`
implementation reads the model name — all five were checked: both Versa modules resolve
`ucsf_gateway_tier(endpoint)`, `ollama` and `llamacpp` resolve `self_hosted_tier(base_url)`, and the
lead/worker composite takes the `least` of its two halves. Writing it cannot move a tier; it decides
which model runs, and `/agent/start` binds *both* halves of the operator's declaration
(`configured_new_session_provider` requires the provider **and** the model), so the pin covers what
the operator actually declared. Its classification, with that reasoning, is a row in
`privacy::config_keys::NOT_CAPABILITY_CONFIG_KEYS`; what it permits without a proof is an integrity
and availability question — silently downgrading every new chat to a different model, or making new
chats fail outright — not a tier one.

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
- *Leave the justification as written and record the hole in the consequences.* Rejected. It would
  have meant writing down that this door grants Private capability to anything with a shell — a
  statement that is true and that nobody reading the ruling would expect from it. The cost of
  closing it is one configuration comparison per new chat.
- *Bind the launch snapshot itself and never read the file again.* Rejected, though it is the
  narrower rule. It also makes a **deliberate** operator edit silently ineffective: the daemon would
  keep serving the old provider with nothing to say about it. Pinning the exemption rather than the
  binding fails loudly instead, and names the key that moved.
- *Treat a desktop daemon whose key never arrived as a keyless deployment.* Rejected — Finding 3 of
  the 2026-09-12 review. `UserActionProof::NoKeyInstalled` means only *"this process read no valid
  32-byte digest off stdin"*, which a desktop spawn satisfies when `userActionKey` is undefined
  (`stdin` is `end()`ed empty, so `hex::decode("")` yields an empty vector that is not 32 bytes) or
  when the daemon's bounded 2s stdin read times out. On `main` that degradation was loud and safe.
  Letting it inherit the exemption would have turned a repairable fault into a silent relaxation
  announced by one `WARN` in a log nobody reads. The launcher declares its intent instead, which is
  the only signal that can tell the two apart, and the declaration can make this daemon *stricter*
  only — so reading it from the environment is safe even though the model can see it.
- *Let a keyless daemon treat its configured model as the capability of any request that states
  none.* Rejected. An absent header resolving to Public is the fail-safe the reach gate is built on,
  and a default that raised it would speak for every caller rather than for the client that says
  what it runs.

**Consequence to accept.** On a keyless daemon whose configured model *is* private, a model running
in a chat on that daemon — a public-model chat resumed from the shared session store — that has
recovered the daemon secret can start a private-capability chat through `/agent/start` with
extensions it chose, and can reach private chats by stating the host's provider. It could already
do the first through `workspace_open { new }`, and the second by spelling a provider name (the
header is not authentication, as `session_reach.rs` records); and the filesystem read-deny that
would stop it carrying anything back out did not ship. It is recorded rather than closed: on a
daemon that cannot tell a person from a model, closing it means refusing the person. What the pin
changes is that this is now confined to the tier the operator *launched* the daemon on — a public
deployment cannot be turned into a private-capability one from inside a chat.

⚠ **And the pin is not DR-14.** It closes one door's stated guarantee, not the general property a
reader might take from it. A shell is still a shell: a model that can write `config.yaml` can read
the session store (`~/.config/biorouter/sessions/`) and the knowledge bases
(`~/.config/biorouter/knowledge/`) as ordinary files, and can start a second `biorouterd` of its
own with any configuration it likes. The privacy barrier is safety before it is security — it stops
mistakes reaching the wrong model — and on a machine where DR-14 is deferred, the part of this door
worth guarding is the Gate C / Gate E *roster*: private connectors (UCSF OMOP, CDW, SPOKE) whose
credentials live in the operating system's credential store rather than on disk. That is what the
pin protects, and it is the honest scope of the claim.

And one visible change: a browser tab on a host configured with a private model now opens private
chats started in the desktop application on the same machine, which it was refused before. That is
the reach rule — *the caller's capability must be at least the chat's classification* — admitting
it, exactly as it admits `biorouter session` configured with the same model. On a host configured
with a public model nothing changes, and private chats stay out of the browser's reach.

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

**Also closed: no other origin may read the page the token is in.** `build_cors_layer`
allow-listed `http://localhost:3000`, `http://127.0.0.1:3000` and this server's own origin
whenever no `--auth-token` was passed. Without a token the auth middleware lets every request
through, so a cross-origin `fetch` of `/session/…` that the browser permits *reads the page* — and
`data-ws-token` is in it. From there: open `/ws` with the token, which CORS does not govern, and
send a message to an agent holding `developer__shell`. **That is the same capability the
reflection above gave, by a different route**, so escaping one and leaving the other would have
closed the sink and left the outcome. No origin but the server's own is granted a read now.

The grant's shape is worth recording, because it looks harmless until the port moves. `--port`
**defaults to 3000**, so on a default run all three entries are this server and the allowance
means nothing. On any other port it hands `http://…:3000` — a frontend dev server, or a page the
operator was talked into opening — read access to a chat page on, say, `:8080`. `--port 8080` is
one of this command's documented invocations.

**Displaced alternative: keep the allowance behind an opt-in flag.** Rejected, and the reason is
this record's own first half. The two routes a cross-origin browser client could have wanted,
`/api/sessions` and `/api/sessions/{id}`, are the ones deleted above; what is left is the page,
`/static/*`, a static `/api/health` and the WebSocket, which CORS does not govern. Nothing in the
repository reads any of it from another origin — `scripts/test_web.sh` uses `curl`, which ignores
CORS entirely. An opt-in would therefore be an opt-in to the token leak and to nothing else.

**What this leaves.** The token is still in the page, because the page needs it; what changed is
that no other origin is told it may read that page. The remaining ways to it are same-origin
(where the question does not arise) and local process inspection, which is issue #47 and unchanged.

### The socket itself gets an origin check, and its token is no longer optional

**Ruling (2026-09-11).** `websocket_handler` refuses a handshake whose `Origin` is not this very
server, and checks the socket token on **every** path rather than only when `--auth-token` is
absent. `handle_web` always generates that token, and an empty expected token is refused outright.
An empty `--auth-token` is rejected at argument-parse time.

**Why the origin check.** `/ws` is the chat: a message on it runs a turn and streams the reply.
CORS does not govern a WebSocket handshake, so a page on *any* origin that held the token could
drive an agent carrying `developer__shell` — classic cross-site WebSocket hijacking. The tree's
other two upgrade sites, `routes/workspace.rs` and `routes/apps.rs`, have had such a check all
along; this one had none on any path. Closing the reflected XSS above on the grounds that the
injected script could read the token and drive the socket, while leaving the socket reachable from
any origin, would have closed the sink and left the capability.

The rule is the **strict core** of the daemon's `routes::origin_matches_host` — the `Origin` must
match this request's own `Host` — with neither of that helper's exceptions:

- **No `is_local_origin` widening.** PR #233 is removing exactly that from the daemon's socket
  gates ("`is_local_origin` is the CORS rule now and nothing else; do not hand it back to a
  socket"), and here it would re-open the allowance closed immediately above, by admitting a page
  on `localhost:3000`.
- **No `file://` and no declared-renderer origin.** Those exist for the Electron renderer, which
  reaches the daemon from another local origin. This server serves its own page from its own
  origin and has no such client, so an opaque origin is refused like any other.

⚠ **It is a duplicate of that rule, not a call to it, and that is a crate boundary rather than a
preference.** `biorouter-cli` does not depend on `biorouter-server` — SD-7 is why `serve` *spawns*
`biorouterd` instead of linking it — so the rule is mirrored, exactly as `token_matches` already
mirrors the daemon's `secret_matches` in this same file. If the two ever need to be one symbol, the
move is into the `biorouter` core library that both already depend on; adding a
command-line-interface-to-server dependency to share a six-line comparison would undo SD-7.

A client that sends no `Origin` is still let past this gate, as the daemon's gates let one past: it
is a non-browser client, and the token guards it.

⚠ **Why "make the token check unconditional" is a trap on its own.** The check was skipped whenever
`--auth-token` was set, and `handle_web` made `ws_token` the empty string in exactly that mode —
so the skip was load-bearing. `token_matches("", "")` is `true`, which means deleting the `if`
without also changing the generation would have admitted **every** socket while reading like a
tightening. The generation is unconditional now, the check is unconditional, and an empty expected
token is refused, so the pair cannot be half-fixed.

**And an empty `--auth-token` is not a token.** `Some("")` satisfied the network-exposure guard, so
`--host 0.0.0.0 --auth-token ""` bound to every interface; `auth_middleware` would then admit
anyone who sent `Authorization: Bearer ` with nothing after it. The one check whose entire job is
to insist on protection was satisfied by its absence. `cli.rs` refuses an empty or whitespace-only
value at parse time, and `validate_network_auth` treats one as absent as well, because
`handle_web` is a public function and the guard must not rely on its caller having been careful.

**Sequencing.** This record's reach gate keys on a session **id**. PR #264 establishes that ids
were being reissued after a delete and adds a high-water allocator; until it lands, a reissued id
defeats the gate. #264 merges first.

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
