# Headless Linux deployment

> **What this is.** How to run Biorouter as a long-lived service on a Linux host that has no
> graphical desktop: which package to install, how credentials get onto the host, the systemd unit
> that keeps `biorouter serve` running, and how much of the network should be able to reach it.
> **Status:** Current.
> **Audience:** operators deploying Biorouter to a shared Linux host, and agents asked to set one
> up.

Run Biorouter this way when the compute belongs on a server rather than on a laptop: `biorouterd`
executes the agent loop on the host, and users reach the ordinary Biorouter interface through a
browser. Two Biorouter binaries do the work, `biorouter` and `biorouterd`, and both come from one
package.

> **Note.** *Headless* here means **a host with no graphical desktop**. It is not a separate
> product or a separate binary — the retired `biorouter-headless` executable and its
> `biorouter-headless-linux-x64.tar.gz` tarball are gone ([decision SD-6](serve-decisions.md)), and
> the browser interface is served by the same `biorouterd` the desktop application runs. The word
> also means non-interactive prompt mode elsewhere in Biorouter; neither sense is meant here.

Read [Reaching Biorouter from a browser](browser-access.md) first. It covers the command, the
access token and what a browser session can do; this page covers only what is different about a
server.

---

## Install the command-line packages

The **CLI-only** Linux packages install `biorouter` and `biorouterd` with no desktop application,
and they ship the browser interface bundle the daemon serves. Download the current version from the
[releases page](https://github.com/BaranziniLab/biorouter/releases):

```bash
# Debian / Ubuntu
sudo apt install ./biorouter-cli_<version>_amd64.deb

# Fedora / RHEL / Rocky
sudo dnf install ./biorouter-cli-<version>-1.x86_64.rpm
```

They lay down three things:

| Path | What it is |
|---|---|
| `/usr/bin/biorouter` | The command-line interface, including `biorouter serve`. |
| `/usr/bin/biorouterd` | The daemon that runs the agent loop and serves the interface. |
| `/usr/share/biorouter/web` | The built browser interface. `biorouter serve` finds it here with no configuration. |

Check the install before going further:

```bash
biorouter --version
biorouter doctor
```

`doctor` reports optional prerequisites — `git`, `uv`, `node` — that some extensions need. The
packages themselves depend only on `libxcb` and `zlib`.

> **Note.** The Linux binaries are built against a glibc 2.31 baseline, which covers Debian 11,
> Ubuntu 22.04 and Rocky 9 and anything newer. Do not install the **desktop** `biorouter_*.deb` /
> `Biorouter-*.rpm` on a server: it carries an Electron application the host cannot display.

## Configure the provider before anything else

**A browser session cannot change its model or provider** — the picker is inert, deliberately, and
the reasoning is [decision SD-1](serve-decisions.md) and
[Reaching Biorouter from a browser](browser-access.md#the-model-is-fixed-before-you-start). The
provider is chosen once, on the host, and holds for every session that daemon runs.

```bash
biorouter configure
```

Run it as the user Biorouter will run as, because it writes into that user's configuration
directory. If you are setting this up as a service, create the service account first — see
[Run it as a service](#run-it-as-a-service) — and configure as that account.

### Credentials on a host with no desktop keyring

Biorouter stores secrets in the operating system's credential store. A Linux host with no desktop
session has no Secret Service to unlock, so it falls back to a file-backed store at
`~/.config/biorouter/secrets.yaml`. Set it explicitly for a service, so the fallback is a decision
rather than an accident:

```bash
BIOROUTER_DISABLE_KEYRING=true
```

That file holds **plaintext** credentials. Keep it readable only by the service user
(`chmod 600`), keep it off shared storage and out of backups that others can read, and treat any
host that has it as holding the keys it contains. [Secret storage](../security/secret-storage.md)
covers the mechanism in full.

To move credentials from a machine you already use, the supported path is to run
`biorouter configure` on the host and enter them there. If you copy `secrets.yaml` from another
machine instead, copy only that file — not `config.yaml`, and not the session store — and fix its
permissions afterwards.

> **Warning.** Never bake credentials into an image, a package or a release artifact. Credential
> setup is a runtime step on the host.

## Start it by hand first

Prove the deployment works interactively before wrapping it in a service:

```bash
biorouter serve --host 0.0.0.0
```

`serve` prints the address to open, including the `?t=` access token, plus a second address built
from a routable address of the host. A token is mandatory for any non-loopback bind — `--no-token`
is refused there rather than warned about. `Ctrl-C` stops it.

If it cannot find the interface, pass `--web-dir /usr/share/biorouter/web` explicitly; the error
names every location it tried.

## Run it as a service

Give the service its own unprivileged account, so a browser session's shell commands run as that
account and not as root or as a person:

```bash
sudo useradd --create-home --shell /usr/sbin/nologin biorouter
```

Then configure as that account — `sudo -u biorouter -H biorouter configure` — so the provider and
credentials land in the home directory the service reads.

For a service, the token must be stable across restarts — nobody is watching the terminal for a new
one. Put it in an environment file rather than on the command line, where `ps` would show it to
every user on the host:

```bash
sudo install -d -m 750 /etc/biorouter
sudo tee /etc/biorouter/env >/dev/null <<'EOF'
BIOROUTER_DISABLE_KEYRING=true
BIOROUTER_BROWSER_TOKEN=<a long random string>
EOF
sudo chmod 600 /etc/biorouter/env
```

Generate the token with something that is actually random, for example `openssl rand -hex 32`.

`serve` reads `BIOROUTER_BROWSER_TOKEN` from the environment the unit gives it and uses that token
as the address's `?t=`, so the URL below is the one it prints. (Until 2026-09 it did not: it minted
a random token over the operator's own and answered the published address with 401. Pass `--token`
only from an interactive shell — on a service's `ExecStart` it would be visible in `ps` to every
user on the host.)

```ini
# /etc/systemd/system/biorouter.service
[Unit]
Description=Biorouter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=biorouter
WorkingDirectory=/home/biorouter
EnvironmentFile=/etc/biorouter/env
ExecStart=/usr/bin/biorouter serve --host 0.0.0.0 --port 8765
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now biorouter.service
systemctl status biorouter.service
```

The URL to hand to users is `http://<host>:8765/?t=<the token from the environment file>`. It stays
valid until you change the token.

> **Note.** `biorouter serve` starts `biorouterd` as a child process and stops it whenever it stops
> itself — `systemctl stop` sends `serve` SIGTERM, and `serve` gives the daemon ten seconds to
> finish before killing it — so systemd supervises one unit and not two. There is no separate
> daemon unit to enable. See [Stopping it](browser-access.md#stopping-it).

## Decide who can reach the port

A browser session that has authenticated is as capable as the desktop application: it drives an
agent that runs shell commands and reads and writes files on this host, as the service user. Treat
reaching the port as equivalent to a shell account.

- **There is no transport encryption.** `biorouter serve` speaks plain HTTP. On anything but a
  trusted local network, terminate TLS in front of it — bind loopback (`--host 127.0.0.1`) and put
  nginx, Caddy or another reverse proxy on the public port. Serve it at the **root of a hostname**,
  not under a path prefix; the interface is served at `/` and a subpath is not supported
  ([decision SD-4](serve-decisions.md)).

  The proxy must also pass WebSocket upgrades through, forward the browser's original `Host`,
  and set `X-Forwarded-Proto`. The daemon's socket gates admit a page only from the origin the
  browser reached it at, in scheme, host and port, and the scheme is `http` unless that header
  says `https` — so without it, the interface loads over `https` and its live views never
  connect. Caddy does all three by default. For nginx:

  ```nginx
  location / {
      proxy_pass http://127.0.0.1:8765;
      proxy_http_version 1.1;
      proxy_set_header Upgrade $http_upgrade;
      proxy_set_header Connection "upgrade";
      proxy_set_header Host $http_host;
      proxy_set_header X-Forwarded-Proto $scheme;
  }
  ```

  `$http_host` rather than `$host`, which drops a non-default port and so no longer matches the
  page's origin. `proxy_set_header` **replaces** the header, which is what you want: where a proxy
  chain appends instead, the daemon reads the last value — the one the proxy nearest it wrote — so
  an inner proxy that overwrites a correct `https` with its own `http` will refuse every WebSocket
  upgrade. Have inner proxies pass the value through rather than re-derive it.
- **Restrict the source addresses.** Scope a cloud security group or a host firewall rule to the
  addresses that need it, rather than relying on the token alone:

  ```bash
  sudo ufw allow from 10.0.0.0/8 to any port 8765 proto tcp
  ```

- **For a single user, expose nothing.** Leave the bind on loopback and forward the port over SSH:

  ```bash
  ssh -N -L 8765:127.0.0.1:8765 user@host
  ```

- **There are no user accounts.** Everyone who opens the address is the same user, with the same
  files and the same conversation history.

## Optional: a virtual display for GUI automation

Most of Biorouter needs no display. The [Computer Controller](../extensions/built-in/computer-controller.md)
extension is the exception: on Linux it drives X11 tools (`xdotool`, `wmctrl`, `xclip`,
`xwininfo`), which need a display to talk to. On a host with no desktop, give it a virtual one:

```bash
sudo apt install xvfb xdotool wmctrl xclip x11-utils
```

Run `Xvfb` as its own unit, and set `DISPLAY` in `/etc/biorouter/env` so the service inherits it:

```ini
# /etc/systemd/system/biorouter-xvfb.service
[Unit]
Description=Virtual X display for Biorouter GUI automation
After=network-online.target

[Service]
Type=simple
User=biorouter
ExecStart=/usr/bin/Xvfb :99 -screen 0 1920x1080x24 -nolisten tcp
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
```

Then add `DISPLAY=:99` to the environment file and `After=biorouter-xvfb.service` to the Biorouter
unit. Skip all of this if you are not using that extension.

## Upgrading

Install the newer package over the old one and restart the service:

```bash
sudo apt install ./biorouter-cli_<version>_amd64.deb   # or dnf install
sudo systemctl restart biorouter.service
```

The package replaces the binaries **and** the interface bundle at `/usr/share/biorouter/web`
together, so the two cannot drift apart. Configuration, secrets and session history live in the
service user's home directory and are untouched.

## Watching for interface drift

The browser interface and the Electron desktop application are built from one shared renderer, so a
change to a shared component reaches both. Two things follow for anyone maintaining this
deployment:

- Changes made for browser readability belong in the shared renderer components, not in anything
  specific to this deployment. There is no second frontend to edit.
- Wide-browser layouts are constrained by the `ReadableContent` wrapper and the
  `.biorouter-readable-content` class, whose browser-only rules are scoped to
  `body.biorouter-headless-browser`. When checking whether the browser and the desktop application
  have diverged, read the shared source first, then confirm against the deployed browser rather
  than a local development server, which can fall into onboarding without the daemon and
  configuration state the data-backed routes need.

## Related documentation

- [Reaching Biorouter from a browser](browser-access.md) — the command, the access token, and what a browser session can and cannot do.
- [Decisions behind `biorouter serve`](serve-decisions.md) — why the model is fixed, why a token is mandatory off loopback, and why the standalone binary was retired.
- [How browser-served Biorouter is built](serve-architecture.md) — the architecture, for developers.
- [Secret storage](../security/secret-storage.md) — what `BIOROUTER_DISABLE_KEYRING=true` switches to, and why a host with no desktop keyring needs it.
- [Environment variables](../configuration/environment-variables.md) — every variable the daemon and `biorouter serve` read.
- [Config file reference](../configuration/config-file-reference.md) — the configuration directory a deployment points Biorouter at.
- [Agent browser debugging](../desktop-ui/agent-browser-debugging.md) — driving the renderer in a real browser when the deployed interface misbehaves.
