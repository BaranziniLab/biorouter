# Deployment

This folder covers reaching Biorouter through a browser — on your own machine or on a shared
host. `biorouter serve` starts the `biorouterd` daemon, hands it the built interface, and prints a
URL: the same interface the desktop application shows, in a tab. That works on a laptop with no
desktop application installed just as well as it works on a server, so this folder is not only
about servers; it is about the browser as a way in, and about what changes when the machine doing
the compute is not the machine you are typing on.

Come here when you want Biorouter in a browser, or when the compute belongs on a machine other than
the user's laptop. If you are installing Biorouter for yourself and want the desktop application,
start with [installation](../getting-started/installation.md) instead. If you are cutting a signed,
notarized, multi-platform release, that is [releases](../releases/README.md). Settings that apply to
any deployment live in [configuration](../configuration/environment-variables.md).

> **Note.** The standalone `biorouter-headless` binary and its
> `biorouter-headless-linux-x64.tar.gz` release artifact have been retired. The daemon serves the
> interface itself now, and `biorouter headless` survives only as an alias for `biorouter serve`.
> The reasoning is [decision SD-6](serve-decisions.md).

## Documents in this folder

| Document | What it covers |
|---|---|
| [Reaching Biorouter from a browser](browser-access.md) | The user-facing guide to `biorouter serve`: quickstart, the access token, reaching it from another machine and what that exposes, why a browser session cannot change its model, and troubleshooting. Start here. |
| [Headless Linux deployment](headless-linux.md) | Running `biorouter serve` as a long-lived service on a Linux host with no graphical desktop: the CLI-only packages, the systemd unit, migrating secrets onto the host, and network exposure. |
| [Reaching a private chat from a script](programmatic-session-access.md) | The `X-Caller-Provider` header: how a monitoring dashboard, a CI job or a shell script reads and follows a **private** conversation over the HTTP API, what the header is not (it is not authentication), and which routes honour it. |
| [How browser-served Biorouter is built](serve-architecture.md) | Developer-facing architecture: what the daemon does with a web directory, how a browser is authenticated, and what the retired front door was replaced by. |
| [Decisions behind `biorouter serve`](serve-decisions.md) | The ten decision records governing the serving path — why a browser session cannot change its model, why the bind defaults to loopback, why the standalone binary was retired, why the launch token is reusable until the daemon stops, and which chats the deprecated `biorouter web` may still open. |

## Related documentation

- [Secret storage](../security/secret-storage.md) — how secrets are stored per platform, and what `BIOROUTER_DISABLE_KEYRING=true` switches to on a host with no desktop keyring.
- [Environment variables](../configuration/environment-variables.md) — the full set of variables the daemon and `biorouter serve` read.
- [Config file reference](../configuration/config-file-reference.md) — the server-side config directory a deployment points Biorouter at.
- [Privacy tiers](../security/privacy-tiers.md) — the classification system behind the fixed-model rule a browser session runs under.
- [biorouter CLI command reference](../cli/command-reference.md) — `serve` and every other subcommand.
