# Reaching Biorouter from a browser

> **What this is.** The user-facing guide to `biorouter serve` — starting Biorouter so it can be
> used from a web browser, on your own machine or on a shared host, and what a browser session
> can and cannot do.
> **Status:** Current.
> **Audience:** end users and operators running Biorouter without the desktop application.

Biorouter normally runs as a desktop application. `biorouter serve` runs the same interface in a
browser instead: it starts the `biorouterd` daemon, points it at the built interface, and prints a
URL to open. The daemon serves the interface and the API on one origin, so nothing is proxied and
everything the desktop application does over a WebSocket — workspace control, live app agents —
works unchanged.

Use it when you have no desktop application (a Linux server, a container, a machine you only reach
over SSH), when you want Biorouter's compute on a different machine from the one you are typing on,
or when you simply prefer a tab. If you are deploying it as a long-running service, read this page
first and then [Headless Linux deployment](headless-linux.md).

---

## Quickstart

Choose the provider and model **before** you start serving — a browser session cannot change them
(see [The model is fixed before you start](#the-model-is-fixed-before-you-start)). Either kind
works: a commercial model, or a private one your institution hosts or that runs on the machine,
which is the choice to make for patient data.

```bash
biorouter configure
biorouter serve --open
```

`serve` prints the address to open and stays in the foreground:

```text
  Biorouter is serving at

      http://127.0.0.1:8765/?t=1f4c9e02…

  The token above is shown once, and is new on every launch.

  The model is whichever `biorouter configure` chose; a browser cannot change it.
  Press Ctrl-C to stop.
```

`--open` opens your browser at that address. It hands the browser a page only your account can
read, which carries the address, and never the address itself: a browser's command line can be
read by other accounts on the machine, and the address works for anyone until `serve` stops.
Without `--open`, copy the URL, including the `?t=` part, which is what authenticates you. `Ctrl-C` stops the daemon and frees the port; so does
stopping `serve` any other way (see [Stopping it](#stopping-it)).

`biorouter headless` is an accepted alias for `biorouter serve` and behaves identically. It is the
name the retired standalone binary was known by, kept so older instructions still land in the right
place.

## The command and its options

```bash
biorouter serve [--host <addr>] [--port <n>] [--token <t>] [--no-token] [--web-dir <dir>] [--public-url <url>] [--open]
```

| Option | What it does | Default |
|---|---|---|
| `--host <addr>` | Address to bind. Anything reachable from another machine requires a token. | `127.0.0.1` |
| `-p, --port <n>` | Port to listen on. | `8765` |
| `--token <t>` | Use this access token instead of generating a fresh one. The flag stays on the command line, where other accounts can read it, and `serve` prints a note saying so; `BIOROUTER_BROWSER_TOKEN` does not. | A new random token each launch |
| `--no-token` | Serve with no access token. Refused for a non-loopback bind, and cannot be combined with `--token`. | Off |
| `--web-dir <dir>` | Directory holding the built interface. It must contain an `index.html`, or `serve` refuses to start. Takes precedence over `BIOROUTER_SERVE_UI`. | `BIOROUTER_SERVE_UI` if set, otherwise found automatically — see [When the interface cannot be found](#when-the-interface-cannot-be-found) |
| `--public-url <url>` | The `https://` address a reverse proxy serves the interface at, path prefix included. `serve` prints it with the token, and the live views accept connections from it. Takes precedence over `BIOROUTER_PUBLIC_URL`. See [Behind a reverse proxy](#behind-a-reverse-proxy). | None |
| `--open` | Open a browser once the server is ready, through a page only your account can read. | Off |

The default port is `8765` rather than `3000` deliberately: `3000` is `biorouterd`'s own default, so
a `serve` default of `3000` would collide with the daemon the command starts.

> **Note.** `serve` starts its **own** daemon, with its own secret, separate from the one the
> desktop application runs. The two share the on-disk session store, so past conversations are
> visible from both, but a turn running in one is not visible to the other. Having the desktop
> application open does not mean `serve` is talking to it.

## Stopping it

Stop `serve` with `Ctrl-C` in its terminal, or by sending it `SIGTERM` — `kill <pid>`, which is
also what `systemctl stop` and most process managers send. Either way `serve` stops the daemon it
started and frees the port: it asks the daemon to shut down, gives it ten seconds to finish, then
kills it. A second `Ctrl-C` skips the wait.

With no browser tab open the daemon is gone in a fraction of a second. With one open, expect the
full ten seconds and the line `biorouterd did not finish within 10s … killing it.` — the interface
always keeps a request waiting on the daemon, and a graceful shutdown waits for it. That is
expected, not a fault. The one thing a daemon stopped that way skips is shutting down a
llama-server it started for a local model; the next Biorouter launch cleans that up.

This is more than tidiness. The daemon honours the access token for as long as it runs, so
stopping `serve` is how you revoke the address it printed.

If `serve` itself is killed outright (`kill -9`) it cannot stop anything, so on macOS and Linux the
daemon watches for that: it notices within a second that its parent has gone and shuts itself
down, taking at most ten seconds more. On Windows there is no such watch — `Ctrl-C` reaches both
processes, but if `biorouter.exe` is ended some other way, from Task Manager for example, end
`biorouterd.exe` as well.

## The access token

A browser cannot send an authentication header on its first request, so the address `serve` prints
carries a credential instead.

- **It is minted per launch** — 32 random bytes, printed as 64 hexadecimal characters in the URL's
  `?t=` parameter, and different every time. It is shown once, in the terminal; there is nowhere
  else to read it back from.
- **Opening the address exchanges it for a cookie.** The daemon validates the token, sets an
  `HttpOnly`, `SameSite=Strict` session cookie named `biorouter_session`, and answers a short page
  that moves the browser on to `/`. The token then disappears from the address bar and from the
  tab's back and forward list, and it is not in the `Referer` of anything the page later loads. A
  page rather than a redirect, because a redirect would lose the cookie when the address was opened
  from a file, which is how `--open` opens it.
- **It is not used up.** The exchange works every time the token is presented, from any browser,
  until the daemon stops — so a second browser, a colleague, or the same browser after it has
  discarded the cookie can all open the same address. Anyone holding the address can do the same,
  and stopping `serve` is how you revoke it ([decision SD-9](serve-decisions.md#sd-9--the-launch-token-works-until-the-daemon-stops-it-is-not-single-use)
  records why it is not single-use).
- **The cookie gates the document, and authenticates nothing else.** It is not accepted as
  authentication on any API route. From the moment the page loads, the interface presents the
  daemon's secret key as a header, exactly as the desktop application does. Its one other effect
  narrows rather than admits: it tells the daemon a request came from the page it served, which
  decides whether private chats and knowledge bases are listed there
  ([decision SD-10](serve-decisions.md#sd-10--the-served-interface-keeps-its-operators-reach-on-listings-and-knowledge-bases-and-gains-nothing-else)).
- **Opening the address without the token** returns a short page saying the link needs its access
  token. Open the full address the command printed, `?t=` included.

To keep one address working across restarts — a bookmark, or a service that restarts — pass the
same token each time with `--token <t>`, or set `BIOROUTER_BROWSER_TOKEN` in the environment `serve`
runs in. `serve` uses it as given (trimmed), hands it to the daemon, and says on the banner that the
token came from there rather than claiming it is new; `--token` wins when both are set. A service
unit should use the variable, from a file only the service user can read, rather than the flag,
which `ps` shows to every user on the host. Treat that value like a password: anyone who has it has
the same access you do.

`--no-token` turns the gate off entirely. It is accepted **only** for a loopback bind, where the
only callers are processes on the same machine, and `serve` prints a line saying so:

```text
  No access token: anything that can reach this port can use it.
```

## Reaching it from another machine

The default bind is `127.0.0.1`: reachable from a browser on the same machine and from nowhere
else. Exposing the port is an explicit act.

```bash
biorouter serve --host 0.0.0.0
```

With a wildcard bind, `serve` prints a second address built from a routable address of the machine.
Browser access from another machine requires a TLS proxy or an SSH tunnel: the plain-HTTP network
address refuses the interface and apps, even with a valid access token. On a machine with no default
route `serve` says so rather than printing a loopback address that could not work.

A token is **mandatory** on any non-loopback bind. Passing `--no-token` alongside one does not
warn, it refuses:

```text
--no-token cannot be combined with --host 0.0.0.0, which is reachable from other machines.
Either bind a loopback address (the default), or drop --no-token and open the URL this command prints.
```

### What exposing the port means

Understand the posture before you take it. A browser session that has authenticated is **as capable
as the desktop application**: it drives an agent that runs shell commands, reads and writes files,
and reaches whatever extensions are configured — all on the serving machine, as the user who
started `serve`.

- **There is no transport encryption.** `serve` speaks plain HTTP. Over an untrusted network the
  token and everything else are readable in transit. Browser access from another machine requires
  a TLS-terminating reverse proxy or an SSH tunnel. The proxy must pass the browser's
  `Sec-Fetch-*` request headers through; [Behind a reverse proxy](#behind-a-reverse-proxy) and the
  [headless Linux guide](headless-linux.md#decide-who-can-reach-the-port) have the configuration.
- **There is one credential and no user accounts.** Everyone who opens the address is the same
  user, with the same files and the same history. Biorouter has no notion of separate accounts
  here.
- **Restrict who can reach the port.** A host firewall rule or a cloud security group scoped to the
  addresses that need it is worth more than the token alone.
- **The interface and apps require localhost or HTTPS.** Opened as `http://192.168.x.y:8765`,
  both are refused with TLS or SSH guidance, even with a valid browser token or cookie. A browser
  applies the protections that keep one app's script away from another app and from the interface's
  secret only at a secure address. Refusing the interface also protects it from an old app tab left
  open during an update (see [Apps share the interface's origin](#apps-share-the-interfaces-origin)).
  Use the SSH tunnel below, which opens Biorouter as `localhost`, or the TLS proxy above.

For one person on one remote machine, the safest option is not to expose the port at all — leave
the bind on loopback and forward it over SSH:

```bash
ssh -N -L 8765:127.0.0.1:8765 user@remote-host
```

Then open the URL `serve` printed on the remote host, unchanged, in a local browser. For a shared
host meant to stay up, see [Headless Linux deployment](headless-linux.md).

### Behind a reverse proxy

Put an HTTPS reverse proxy in front of a loopback bind, and tell `serve` the address people open:

```bash
biorouter serve --port 8765 --public-url https://biorouter.example.org/
```

The interface also works under a path on a shared hostname. The proxy must remove the path prefix
before it forwards the request, and pass WebSocket upgrades through:

```bash
biorouter serve --port 8765 --public-url https://hub.example.org/biorouter/
```

`--public-url` does three things:

1. `serve` prints that address, with the access token, as the one to open.
2. The live views accept a connection from a page at that address. Without it, they accept only a
   page whose address matches the `Host` and `X-Forwarded-Proto` headers the daemon receives,
   which a chain of proxies may not preserve.
3. The access cookie is limited to that path, so it is not sent to other servers on the same
   hostname.

The proxy must pass the browser's `Sec-Fetch-*` request headers through, which proxies do unless
configured to remove them. The browser sends them only to an `https` or loopback address, and the
daemon serves the interface to another address only when they arrive. Open the address with its
trailing slash: the interface loads its files relative to it.

#### JupyterHub

With [jupyter-server-proxy](https://jupyter-server-proxy.readthedocs.io/), each person's server can
start Biorouter and serve it at `https://<hub>/user/<name>/biorouter/`. The hub's own login protects
the address, so the access token can be turned off:

```python
c.ServerProxy.servers = {
    "biorouter": {
        "command": [
            "biorouter", "serve", "--port", "{port}", "--no-token",
            "--public-url", "https://hub.example.org{base_url}biorouter/",
        ],
        "timeout": 60,
    },
}
```

`{base_url}` is the server's own prefix, for example `/user/alice/`. Replace `hub.example.org` with
the hub's hostname. `--no-token` is accepted because `serve` binds the loopback address, which only
the proxy on the same machine can reach.

## The model is fixed before you start

**A browser session cannot change its model or provider.** The picker is inert, and this is
deliberate — it is a privacy property, not a defect.

Biorouter classifies a conversation by the sensitivity of what it has touched, and a conversation
that has reached a private, institution-hosted model may never later reach a public one. That
guarantee depends on knowing which model a conversation ran against. The desktop application can
change the provider mid-session because it holds proof that a human at the keyboard asked for the
change; a browser tab holds no such proof, and manufacturing one would mean putting the key
somewhere page JavaScript could reach — which is precisely what the mechanism exists to prevent.

So the capability is withdrawn rather than approximated. The consequence is a good one: the
provider is chosen once, at the terminal, and the tier that choice implies holds for **every**
session in that daemon. A run started against an institutional model is private for its whole life;
one started against a commercial model is public for its whole life. Neither can drift.

What that means for a chat you start in the browser:

- **It starts on the configured model**, private or not, with nothing asked of you — choosing it
  at the terminal was the decision.
- **A private model makes it private with its first reply.** The tab you are in keeps working with
  it: the browser tells the daemon which model it runs under, and a chat is open to anything
  running under a model at least as private as the chat.
- **Nothing in the browser can move it onto a different private model.** The configured one is the
  only private model a chat started here ever reaches.
- **A private chat started in the desktop application opens here only when the host's model is
  private too.** On a host configured with a commercial model it stays out of reach, with a card
  saying so; open it in the desktop application instead.

The reasoning is recorded as [decision SD-12](serve-decisions.md#sd-12--a-new-chat-starts-on-the-operators-model-without-a-proof-and-nothing-else-does).

**The fix is to choose the provider before you start serving:**

```bash
biorouter configure     # pick the provider and model
biorouter serve
```

To change it, stop `serve` with `Ctrl-C`, run `biorouter configure` again, and start it back up.
The reasoning is recorded in full as [decision SD-1](serve-decisions.md#sd-1--a-browser-session-cannot-change-its-model-or-provider-and-that-is-the-point);
the classification system it protects is [privacy tiers](../security/privacy-tiers.md).

## What a browser can and cannot do

One interface bundle serves both the desktop application and the browser, so features arrive in the
browser automatically unless they need something only the Electron desktop shell can provide. What
differs:

| Area | In a browser |
|---|---|
| Chat, sessions, history, extensions, skills, knowledge bases, workflows | Work as they do in the desktop application, for everything public, with two differences that follow from the fixed model. A new chat starts on the host's configured model, and a private chat opens only on a host whose configured model is private — see [The model is fixed before you start](#the-model-is-fixed-before-you-start) and [decision SD-12](serve-decisions.md#sd-12--a-new-chat-starts-on-the-operators-model-without-a-proof-and-nothing-else-does). **Private** chats and knowledge bases appear in History and the Knowledge view only when the provider you configured is private, and being listed does not by itself make a private chat openable — see [decision SD-10](serve-decisions.md#sd-10--the-served-interface-keeps-its-operators-reach-on-listings-and-knowledge-bases-and-gains-nothing-else). |
| Workspace control, several conversations at once, live app agents | Work — these are WebSocket-backed daemon routes, reached on the same origin. |
| Stopping a response, Stop and send | Work in an ordinary chat ([SD-11](serve-decisions.md#sd-11--stop-works-on-a-daemon-with-no-key-steering-does-not-and-a-subagents-tab-stays-the-persons)). |
| Steering a response while it runs | **Not available.** Injecting text into a turn that is already running needs proof that a person acted, which only the desktop application holds. What you type is queued instead and sent when the turn ends, so nothing is lost — it simply does not redirect the answer in flight. |
| A delegated subagent's own tab | **Read-only, and it says so before you try.** The tab shows the subagent's conversation and follows it while it works, but has no composer and no Stop button: sending to a subagent, steering it and stopping it need proof that a person acted, which only the desktop application holds. A note takes the composer's place, and one line takes the Stop button's ([SD-8](serve-decisions.md#sd-8--a-control-that-can-never-work-here-says-so-rather-than-failing-on-click)). |
| Resetting app data (Settings → App → Danger zone → Reset data) | **Not available, and it says so before you try.** A reset deletes chats, knowledge bases and the rest for good, private ones included, so the daemon performs one only for a request that proves a person asked, which only the desktop application can send. The Reset data button is disabled, with a note under it saying why. On the machine running `biorouter serve`, reset from Settings in the Biorouter app there, or remove items from a terminal there with `biorouter session remove`, `schedule remove`, `skill remove` and `extension remove` ([SD-8](serve-decisions.md#sd-8--a-control-that-can-never-work-here-says-so-rather-than-failing-on-click)). |
| Model and provider selection | **Not available.** See [The model is fixed before you start](#the-model-is-fixed-before-you-start). |
| Crew | **Not available, and it says so before you try.** Opening **Crew** in the browser shows "Crew needs the Biorouter desktop app", and nothing on that page asks the daemon for Crew data. A chat shows no Crew bar. Every Crew action, even listing saved workspaces, needs proof that a person is at this computer, which the desktop application and `biorouter crew` supply themselves; the daemon `biorouter serve` starts never holds it. A Crew request that reaches it anyway is refused with `crew_human_authority_unavailable` and the sentence "Crew isn't available in a browser opened with biorouter serve…", and no setting in the browser changes that. Use Crew in the desktop application, or with `biorouter crew` in a terminal, on your own computer. See the [Crew user manual](../crew/README.md) and [SD-8](serve-decisions.md#sd-8--a-control-that-can-never-work-here-says-so-rather-than-failing-on-click). |
| File and folder pickers | No native dialog. You type a path, and it is a path **on the machine running the daemon**, not on the machine holding the browser. |
| Artifacts and diagnostics bundles | The artifact side panel works as usual. Opening an artifact outside the panel opens a new tab; a diagnostics bundle downloads as a file. |
| Sending a region of the preview to the chat, and the agent's screenshot of the panel | **Not available, and it says so before you try.** Both are a picture taken by the desktop application's window, the only thing that can see into the sandboxed frames most previews draw in, and a browser page has no such window. The camera button in the panel's header is disabled and its tooltip says why; an agent that asks for a screenshot is told to read the panel's text instead, which works ([SD-8](serve-decisions.md#sd-8--a-control-that-can-never-work-here-says-so-rather-than-failing-on-click)). |
| The in-app terminal | Not available — it is an Electron capability. Use a shell on the serving machine. |
| Application updates, installing the CLI, one-click dependency installs | Not available. Update by upgrading the installed packages on the serving machine. |
| Desktop niceties — system notifications, spellcheck, dock and menu-bar icons, keeping the machine awake | Not available. |
| Opening a chat in its own window | There is no second window. A new chat opens in the current tab; branching a session into a separate window does nothing. |

### Apps share the interface's origin

Every app you build is served from the same address as the interface, and the interface's page
holds the daemon's secret. An app's code is written by the agent, so the daemon keeps that page
away from it: in supported modern browsers at a secure address, the interface is served only when
the browser opens it as a page. Fetches and frames are refused, it cannot be framed, and it is isolated from any page it opens or that opens
it ([SD-14](serve-decisions.md#sd-14--the-shell-is-served-only-to-a-navigation-while-apps-share-its-origin)).
An app's own page is served the same way and cannot be framed, its run-state and model routes want
a token only that page holds, and a page that opens it in a popup is left without a handle on it,
so one app's script cannot read another app's page.

All of this depends on the browser treating the address as secure: `localhost` or another loopback
address, or `https`. At a plain-HTTP network address a browser sends none of the request headers
these checks read and ignores the opener policy, so an app's script there could read the interface
and every other app. Neither the interface nor apps are served on browser credentials at such an
address; open them through an
SSH tunnel or an HTTPS proxy ([What exposing the port means](#what-exposing-the-port-means)).
The proxy must preserve Fetch Metadata. What a proxy says about the scheme is not consulted: an
old HTTP app tab can forge `X-Forwarded-Proto: https` on a fetch, but cannot forge the browser's
`Sec-Fetch-*` headers, and the browser sends those only to an `https` or loopback address.

Gaps remain even at a secure address: a browser that does not support the `noopener-allow-popups`
opener policy (Chromium-based browsers do) lets an app read another app's page through a popup, a
browser that sends no Fetch Metadata at all (Safari before 16.4) lets an app fetch another app's
page or the interface, and every app shares one `localStorage`. Serving each app on its own origin
is the planned fix. Until then, open apps you did not build yourself in a separate browser profile.

Two properties of browser cookies apply here and no setting changes them. A cookie's path does not
separate pages of one origin, which is why the rules above exist. And a cookie is not scoped by
port: when any page in the browser sends a request to another port of `127.0.0.1`, the browser
attaches the interface's cookie, and an app's access cookie if the path is under that app, for
whatever listens on that port to read. On a shared machine, another account listening on a
loopback port could collect them that way and use them until the daemon stops. App cookies change
each time the daemon starts (and deleting an app retires its cookie), and so does the interface's
unless you fixed its token with `--token` or `BIOROUTER_BROWSER_TOKEN`; stopping `biorouter serve`
revokes them all.

## Troubleshooting

**`port 8765 on 127.0.0.1 is already in use.`** Something else holds the port. Choose another with
`--port <n>`, or stop the other process — on macOS and Linux, `lsof -nP -iTCP:8765 -sTCP:LISTEN`
names it. A `biorouterd agent` holding it is usually left over from a `serve` of version 1.90.3 or
earlier, which left its daemon running when it was stopped by anything other than `Ctrl-C` in its
own terminal.

**`biorouterd exited during startup`, or it never starts listening.** The daemon is started as a
child process and watched while it comes up; if it dies, `serve` reports that rather than pretending
the port is healthy. Run `biorouterd agent` by hand to see its own output, and check that the
configuration it reads is valid.

**`could not start biorouterd`, on Windows.** `serve` starts the daemon that belongs with the CLI
you ran, and finds it either next to that CLI or next to the application the CLI was installed from
(see [When the interface cannot be found](#when-the-interface-cannot-be-found) below for how that is
recorded). If the application has moved or been reinstalled since, run `biorouter setup-path` again
from inside it — `<application folder>\resources\bin\biorouter.exe setup-path`.

**A message does not start a chat.** The composer keeps what you typed, and a notice in the corner
says why. The commonest cause is on the host rather than in the browser — for example *Failed to
configure the selected provider for the new chat: Configuration value not found:
OPENAI_API_KEY* means the model chosen with `biorouter configure` has no credential on the serving
machine. Fix it there, restart `serve`, and open the new address it prints. **Copy error** on the
notice copies the daemon's own words, for a bug report.

**The tab says the link needs its access token.** The `?t=` part was dropped — from a copy-paste, a
chat client shortening the link, or a bookmark saved after the address changed to `/`. Use the full address as
printed. If the launch has since restarted, the token has changed; read the new one from the
terminal.

**The page loads but nothing connects.** Check that you are on the address `serve` printed. The
live views are WebSockets, and the daemon admits one only from the scheme, host and port the
browser used. Behind a reverse proxy, pass that address with `--public-url` (see
[Behind a reverse proxy](#behind-a-reverse-proxy)). Without it, the daemon works the address out
from the `Host` and `X-Forwarded-Proto` headers, so the proxy must forward the original `Host` and
set `X-Forwarded-Proto: https`, and a chain of proxies that appends `http` to that header refuses
every live view. The daemon logs each refusal as
`rejected workspace WS: cross-origin connect rejected`, with the origin it was given.

**Behind a proxy, the page says to open Biorouter through localhost or HTTPS.** The browser's
`Sec-Fetch-*` request headers did not arrive. Check that the address in the browser is `https`,
and that no proxy in the chain removes those headers.

**Under a path prefix, the page stays on "Loading Biorouter".** The interface's files did not
load. Check that the proxy removes the prefix before forwarding, and that the address ends in `/`.
A bundle built by an earlier release asks for its files at the hostname's root; rebuild it with
`npm run build:web`.

**A file path the agent uses does not exist.** Paths are resolved on the serving machine. When the
browser is on a different computer, its local files are not visible to the agent; copy them to the
serving machine first.

**A private chat or knowledge base you can see in the desktop app is missing from the browser.**
Private chats and knowledge bases are shown in the browser only when the provider `serve` was
started with is itself private, meaning institution-hosted or running on this machine, and only
when `serve` was started with its access token (the default). The desktop app proves a person is at
the keyboard; a browser cannot, so it is given the reach of the model its daemon runs on and no
more. On a private provider the chat is listed, but being listed does not by itself let the
browser open, rename or delete it; when it cannot, open the chat in the desktop app. The reasoning is
[decision SD-10](serve-decisions.md#sd-10--the-served-interface-keeps-its-operators-reach-on-listings-and-knowledge-bases-and-gains-nothing-else).

### When the interface cannot be found

A directory you name is used as named, or not at all. `--web-dir <dir>` wins when it is given;
otherwise `BIOROUTER_SERVE_UI` does, if it is set to anything but blank. Whichever it is must
contain an `index.html`. If it does not, `serve` stops with the same error for both, naming where
the path came from:

```text
no web interface at /srv/biorouter/wbe (expected an index.html there; the path came from BIOROUTER_SERVE_UI)
```

It does not move on to a bundle it found somewhere else. Through version 1.90.3 a
`BIOROUTER_SERVE_UI` with no `index.html` was skipped without a word, and `serve` served whichever
bundle the search below turned up next — one you had not chosen.

With neither set, `serve` searches in a fixed order, and names every location it tried when it
finds none:

1. `web/` beside the installed binaries (a packaged application).
2. `ui/desktop/src/web/` in a development tree.
3. `web/` beside the application a Windows install was made from.
4. `/usr/share/biorouter/web` (where the Linux packages put it).

"Beside" means beside the **real** binary. On macOS and Linux the CLI is installed on `PATH` as a
symlink (`~/.local/bin/biorouter` → the application bundle), and steps 1 and 2 follow that link
before deriving anything from it — otherwise they would name directories in your home folder, which
is what they did in v1.89.5 through v1.90.2.

Windows has no symlink to follow. `biorouter setup-path` — and the in-app "Biorouter CLI Update"
card, which runs it — *copies* `biorouter.exe` into `%LOCALAPPDATA%\Biorouter\bin`, leaving
`biorouterd.exe` and the interface behind inside the application. So the copy also records the
folder it came from, in a small file named `.biorouter-origin` beside itself, and step 3 reads that
back. The same record is how `serve` and `biorouter apps` find `biorouterd.exe`. It is rewritten
every time you install, so updating the application and running `biorouter setup-path` again is what
points the CLI at the new one.

If you move, rename or uninstall the Biorouter application, the recorded folder is stale — it will
be named in the list of places `serve` looked. Run `biorouter setup-path` from the application's own
copy of the CLI to record the new location.

In a development tree, build it first:

```bash
cd ui/desktop && npm run build:web
```

## Related documentation

- [Reaching a private chat from a script](programmatic-session-access.md) — the `X-Caller-Provider` header, for automation that must read or follow a private conversation over the API.
- [Headless Linux deployment](headless-linux.md) — running this as a long-lived service on a Linux server.
- [Decisions behind `biorouter serve`](serve-decisions.md) — why the browser session is shaped this way.
- [How browser-served Biorouter is built](serve-architecture.md) — the architecture, for developers.
- [Privacy tiers](../security/privacy-tiers.md) — the classification system the fixed-model rule protects.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — what to pick before you start serving.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_SERVE_UI`, `BIOROUTER_BROWSER_TOKEN` and the rest.
- [biorouter CLI command reference](../cli/command-reference.md) — every other subcommand.
