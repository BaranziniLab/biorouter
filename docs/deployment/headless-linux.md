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
packages themselves depend on `libxcb` and `zlib` plus the Biorouter Copilot helper's runtime:
`python3`, the GObject and AT-SPI introspection bindings, and GTK 3. The full list is declared in
[`packaging/biorouter-cli.yaml`](../../packaging/biorouter-cli.yaml). They install on a host with no
display; Biorouter Copilot itself still reports `desktop_unavailable` there (see
[below](#biorouter-copilot-requires-a-desktop-session)).

> **Note.** The Linux binaries are built against a glibc 2.31 baseline, which covers Debian 11,
> Ubuntu 22.04 and Rocky 9 and anything newer. Do not install the **desktop** `biorouter_*.deb` /
> `Biorouter-*.rpm` on a server: it carries an Electron application the host cannot display.

## Configure the provider

Choose the provider in the browser, or at the terminal on the host. With nothing configured, the
first page the browser shows is the provider list, and the page continues into the app once a
provider works. That needs the page to be reached over `https` (a reverse proxy) or on the host
itself, and a browser token, which `serve` always uses unless told `--no-token`; see
[Choosing the provider and model](browser-access.md#choosing-the-provider-and-model) and
[decision SD-18](serve-decisions.md#sd-18-the-person-in-the-served-page-sets-up-providers-and-an-agent-still-cannot).
At the terminal:

```bash
biorouter configure
```

Run it as the user Biorouter will run as, because it writes into that user's configuration
directory. If you are setting this up as a service, create the service account first — see
[Run it as a service](#run-it-as-a-service) — and configure as that account.

### Entering a key without echo

To enter only a provider's API key on the host, name the provider and let Biorouter ask for the
key:

```bash
biorouter configure --provider <provider> --secret-stdin
```

At a terminal it asks for each required key at a hidden prompt, so the key is never typed at the
shell's own prompt, never shown, and never kept in shell history or on a command line. Without a
terminal it reads `NAME=value` lines from stdin, for example from a file only you can read:

```bash
biorouter configure --provider openai --secret-stdin < ~/openai.env   # OPENAI_API_KEY=...
```

It takes only the provider's secret settings (an endpoint or a region is set with
`biorouter configure`), checks the key with the provider, and writes it only when the check
passes; a rejected key, an unreachable provider or a 30-second timeout leaves the saved settings as
they were. It does not change which provider or model is the default. It prints where the key went
(the system keychain, or the `secrets.yaml` file) and which settings folder that is, so a key saved
for the wrong account is easy to see. Run it as the service account, then restart the service so
the running `serve` reads the new key. It refuses in an agent's shell: a key typed for an agent
would pass through its model.

When a `biorouter serve` on the host runs as another account, both this command and the
interactive `biorouter configure` say so before anything is typed: the account and process, where
that server reads its settings (that account's `~/.config/biorouter` by default), that a key saved
now goes to this account's settings folder instead, and the `sudo -H -u <account>` line to run.
`--secret-stdin` repeats the point after saving. They find the server in the process list, from
each process's owner and command line only, never another account's environment or files, so a
host that hides other accounts' processes (`/proc` mounted with `hidepid`) shows nothing.

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

For a non-loopback bind, `serve` prints an SSH tunnel command and the local browser address to
open, `http://127.0.0.1:<port>/`. Its `?t=` access token is printed only to a terminal, for a token
made for this launch; otherwise `biorouter serve url` prints the whole address (see
[Where the token is printed](browser-access.md#where-the-token-is-printed)). It does not offer a
plain HTTP network address to open. A token is
mandatory for this bind; `--no-token` is refused. `Ctrl-C` stops it.

If it cannot find the interface, pass `--web-dir /usr/share/biorouter/web` explicitly; the error
names every location it tried.

## Run it as a service

Give the service its own unprivileged account, so a browser session's shell commands run as that
account and not as root or as a person:

```bash
sudo useradd --create-home --shell /usr/sbin/nologin biorouter
```

Then configure as that account — `sudo -u biorouter -H biorouter configure` — so the provider and
credentials land in the home directory the service reads. To enter only a key, use
`sudo -u biorouter -H env BIOROUTER_DISABLE_KEYRING=true biorouter configure --provider <provider> --secret-stdin`
([Entering a key without echo](#entering-a-key-without-echo)); the variable matches the one the
service's environment file sets, so the key lands in the store the service reads.

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
as the address's `?t=`, so the URL below is the one to open. It does not print the token: a
service's output is the journal, which other accounts on the host can read (on Ubuntu, the `adm`
group), so the banner there shows the address without it. (Until 2026-09 it did not: it minted
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

Give users the HTTPS address from the reverse proxy, with `?t=<the token from the environment
file>`, or have them use an SSH tunnel and open `http://127.0.0.1:8765/?t=<token>`. Do not hand
out a plain HTTP network address. The token stays valid until you change it.

To show the whole address on the host, token included, run `biorouter serve url` as the service
account:

```bash
sudo -u biorouter -H biorouter serve url --port 8765
```

It reads a file only that account can read, which `serve` writes when it starts and removes when it
stops. Never search the journal for the address: reading the service log should never be a way to
hold the token, and nothing there carries it. Whatever prints the address puts it on your screen
and in any recording of the session, so run it where you would type a password.

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
  nginx, Caddy or another reverse proxy on the public port, and pass the address people open to
  `serve` with `--public-url`. The interface works at the root of a hostname or under a path
  prefix that the proxy removes before forwarding
  ([decision SD-15](serve-decisions.md#sd-15-behind-a-reverse-proxy-at-the-root-or-under-a-path)).

  The proxy must also pass WebSocket upgrades through and leave the browser's `Sec-Fetch-*`
  request headers in place. The daemon's socket gates admit a page only from the origin the
  browser reached it at, in scheme, host and port. `--public-url` names that origin. Without it,
  the daemon reads it from the original `Host` and `X-Forwarded-Proto`, so the proxy must forward
  both, or the interface loads over `https` and its live views never connect. Caddy does all of
  this by default. For nginx:

  ```nginx
  location / {
      client_body_buffer_size 512k;
      proxy_pass http://127.0.0.1:8765;
      proxy_http_version 1.1;
      proxy_set_header Upgrade $http_upgrade;
      proxy_set_header Connection "upgrade";
      proxy_set_header Host $http_host;
      proxy_set_header X-Forwarded-Proto $scheme;
  }
  ```

  The `client_body_buffer_size 512k;` line keeps Biorouter's audio chunks in nginx memory.
  `proxy_request_buffering off;` on this location is another option. Stock nginx writes chunks
  larger than its 16 KiB buffer to a temporary file. Keep the request body limit at least 512 KiB
  plus headers; nginx's 1 MiB default suffices. Prefer HTTP/2 from the browser to the proxy when
  several tabs may be open. The browser shows a notice before its first microphone recording:
  its permission covers every page at that origin. On a shared hub, use a subdomain per user when
  possible, and recommend a grant for one visit.

  For a path prefix, end both the `location` and the `proxy_pass` address with a slash, which
  makes nginx remove the prefix: `location /biorouter/ { proxy_pass http://127.0.0.1:8765/; ... }`,
  with `--public-url https://example.org/biorouter/`.

  `$http_host` rather than `$host`, which drops a non-default port and so no longer matches the
  page's origin. `proxy_set_header` **replaces** the header, which is what you want: where a proxy
  chain appends instead, the daemon reads the last value — the one the proxy nearest it wrote — so
  an inner proxy that overwrites a correct `https` with its own `http` will refuse every WebSocket
  upgrade. Have inner proxies pass the value through rather than re-derive it, or pass
  `--public-url`, which the socket gates accept whatever the chain reports.
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

## Biorouter Copilot requires a desktop session

Most of BioRouter needs no display. [Biorouter Copilot](../extensions/built-in/computer-controller.md)
uses its bundled native helper on the backend host. A headless service must report
`desktop_unavailable` without breaking chat or Web & Documents. Opening the browser client does
not expose the browser user's desktop.

Linux desktop operation requires a real user session, the declared AT-SPI/Python GI dependencies,
and supported capture/input methods. Xvfb alone does not establish accessibility or Wayland
support. Do not install the retired shell-automation stack as a fallback. See the
[implementation status](../design/computer-use-implementation-status.md) for platform evidence.
Task approval and OS permissions are required before observing or controlling the desktop.

## Upgrading

Install the newer package over the old one and restart the service:

```bash
sudo apt install ./biorouter-cli_<version>_amd64.deb   # or dnf install
sudo systemctl restart biorouter.service
```

The package replaces the binaries **and** the interface bundle at `/usr/share/biorouter/web`
together, so the two cannot drift apart. Configuration, secrets and session history live in the
service user's home directory and are untouched.

### Updating from the page

A running `serve` also offers an **Update to X** row in the page when a new release is out.
Biorouter has already downloaded the update and checked it against the SHA-256 the release
publishes.

- If `serve` runs as root, or as an account where `sudo -n` works without a password, the update
  takes one click and the package manager installs the package.
- If `serve` runs as an ordinary service user (the unprivileged `biorouter` account above), the
  page shows **Updating needs administrator rights on this server** and a one-line command behind a
  Copy button. An administrator runs it in a terminal on the server; it checks the file's SHA-256
  as root before installing. Then click **Check again** and **Restart**.
- On restart, `serve` re-executes itself in place, with the same pid, command line and address, so
  systemd does not see the unit stop. The unit above works as written.
- Chats that are still working stop when it restarts. If the new version does not start, the
  previous one is put back.

The details are in [Updating from the page](browser-access.md#updating-from-the-page) and
[decision SD-19](serve-decisions.md#sd-19-the-person-on-the-page-updates-serve-and-it-comes-back-on-the-same-address).

### Speech models and containers

Install speech models for dictation on the host as the service user, from your own terminal:
`sudo -u biorouter -H biorouter dictation install`. `biorouter serve --no-dictation-install` only stops browsers from installing them. To run
`biorouter serve` in a container instead of a systemd unit, see
[Running Biorouter in Docker](../../BUILDING_DOCKER.md#running-biorouter-in-docker).

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
- [Decisions behind `biorouter serve`](serve-decisions.md): why an agent cannot change the provider, why a token is mandatory off loopback, why the standalone binary was retired, and how `serve` updates itself.
- [How browser-served Biorouter is built](serve-architecture.md) — the architecture, for developers.
- [Building and running Biorouter with Docker](../../BUILDING_DOCKER.md): the container image, including `serve` and browser dictation.
- [Voice dictation](../dictation/README.md): speech models, `biorouter dictation install` and the dictionary.
- [Secret storage](../security/secret-storage.md) — what `BIOROUTER_DISABLE_KEYRING=true` switches to, and why a host with no desktop keyring needs it.
- [Environment variables](../configuration/environment-variables.md) — every variable the daemon and `biorouter serve` read.
- [Config file reference](../configuration/config-file-reference.md) — the configuration directory a deployment points Biorouter at.
- [Agent browser debugging](../desktop-ui/agent-browser-debugging.md) — driving the renderer in a real browser when the deployed interface misbehaves.
