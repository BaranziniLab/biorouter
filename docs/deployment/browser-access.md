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
(see [The model is fixed before you start](#the-model-is-fixed-before-you-start)):

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

`--open` launches your browser at that address. Without it, copy the URL — including the `?t=`
part, which is what authenticates you. `Ctrl-C` stops the daemon and frees the port; so does
stopping `serve` any other way (see [Stopping it](#stopping-it)).

`biorouter headless` is an accepted alias for `biorouter serve` and behaves identically. It is the
name the retired standalone binary was known by, kept so older instructions still land in the right
place.

## The command and its options

```bash
biorouter serve [--host <addr>] [--port <n>] [--token <t>] [--no-token] [--web-dir <dir>] [--open]
```

| Option | What it does | Default |
|---|---|---|
| `--host <addr>` | Address to bind. Anything reachable from another machine requires a token. | `127.0.0.1` |
| `-p, --port <n>` | Port to listen on. | `8765` |
| `--token <t>` | Use this access token instead of generating a fresh one. | A new random token each launch |
| `--no-token` | Serve with no access token. Refused for a non-loopback bind, and cannot be combined with `--token`. | Off |
| `--web-dir <dir>` | Directory holding the built interface. It must contain an `index.html`, or `serve` refuses to start. Takes precedence over `BIOROUTER_SERVE_UI`. | `BIOROUTER_SERVE_UI` if set, otherwise found automatically — see [When the interface cannot be found](#when-the-interface-cannot-be-found) |
| `--open` | Open a browser once the server is ready. | Off |

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
  `HttpOnly`, `SameSite=Strict` session cookie named `biorouter_session`, and redirects to `/`. The
  token then disappears from the address bar, so it is not left in browser history or in the
  `Referer` of anything the page later loads.
- **It is not used up.** The exchange works every time the token is presented, from any browser,
  until the daemon stops — so a second browser, a colleague, or the same browser after it has
  discarded the cookie can all open the same address. Anyone holding the address can do the same,
  and stopping `serve` is how you revoke it ([decision SD-9](serve-decisions.md#sd-9--the-launch-token-works-until-the-daemon-stops-it-is-not-single-use)
  records why it is not single-use).
- **The cookie gates the document and nothing else.** It is not accepted as authentication on any
  API route. From the moment the page loads, the interface presents the daemon's secret key as a
  header, exactly as the desktop application does.
- **Opening the address without the token** returns a short page saying the link needs its access
  token. Open the full address the command printed, `?t=` included.

To keep one address working across restarts — a bookmark, or a service that restarts — pass the
same token each time with `--token <t>`, or set `BIOROUTER_BROWSER_TOKEN` for the daemon. Treat that
value like a password: anyone who has it has the same access you do.

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

With a wildcard bind, `serve` prints a second address built from a routable address of the machine,
which is the one to paste into a browser elsewhere. On a machine with no default route it says so
rather than printing a loopback address that could not work.

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
  token and everything else are readable in transit. For anything beyond a trusted local network,
  put a TLS-terminating reverse proxy in front of it, or do not expose the port at all.
- **There is one credential and no user accounts.** Everyone who opens the address is the same
  user, with the same files and the same history. Biorouter has no notion of separate accounts
  here.
- **Restrict who can reach the port.** A host firewall rule or a cloud security group scoped to the
  addresses that need it is worth more than the token alone.

For one person on one remote machine, the safest option is not to expose the port at all — leave
the bind on loopback and forward it over SSH:

```bash
ssh -N -L 8765:127.0.0.1:8765 user@remote-host
```

Then open the URL `serve` printed on the remote host, unchanged, in a local browser. For a shared
host meant to stay up, see [Headless Linux deployment](headless-linux.md).

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
| Chat, sessions, history, extensions, skills, knowledge bases, workflows | Work as they do in the desktop application. |
| Workspace control, several conversations at once, live app agents | Work — these are WebSocket-backed daemon routes, reached on the same origin. |
| Stopping a response, steering it while it runs, Stop and send | Work in an ordinary chat ([SD-11](serve-decisions.md#sd-11--stop-and-steering-work-on-a-daemon-with-no-key-a-subagents-tab-stays-the-persons)). A message you steer with is recorded as an ordinary message rather than as one the desktop application marks as typed by you. |
| A delegated subagent's own tab | **Read-only.** Sending to a subagent, steering it and stopping it from its tab need proof that a person acted, which only the desktop application holds. The tab does not yet say so before you try. |
| Model and provider selection | **Not available.** See [The model is fixed before you start](#the-model-is-fixed-before-you-start). |
| File and folder pickers | No native dialog. You type a path, and it is a path **on the machine running the daemon**, not on the machine holding the browser. |
| Artifacts and diagnostics bundles | The artifact side panel works as usual. Opening an artifact outside the panel opens a new tab; a diagnostics bundle downloads as a file. |
| The in-app terminal | Not available — it is an Electron capability. Use a shell on the serving machine. |
| Application updates, installing the CLI, one-click dependency installs | Not available. Update by upgrading the installed packages on the serving machine. |
| Desktop niceties — system notifications, spellcheck, dock and menu-bar icons, keeping the machine awake | Not available. |
| Opening a chat in its own window | There is no second window. A new chat opens in the current tab; branching a session into a separate window does nothing. |

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

**The tab says the link needs its access token.** The `?t=` part was dropped — from a copy-paste, a
chat client shortening the link, or a bookmark saved after the redirect. Use the full address as
printed. If the launch has since restarted, the token has changed; read the new one from the
terminal.

**The page loads but nothing connects.** Check that you are on the address `serve` printed. A
browser reaching the daemon on a different origin is not the supported configuration — the
interface is served at the root of the daemon's own origin and nowhere else.

**A file path the agent uses does not exist.** Paths are resolved on the serving machine. When the
browser is on a different computer, its local files are not visible to the agent; copy them to the
serving machine first.

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
