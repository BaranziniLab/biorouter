# Launching the dev GUI from a shell without a TTY

> **What this is.** The procedure for starting the Electron dev GUI from an agent
> shell, a CI step, or any other context without a TTY — and the six failure modes
> that make a working app look broken.
> **Status:** Current.
> **Audience:** agents driving the desktop app, and developers automating a GUI launch.

`just run-dev` is the right command **at a human terminal**. It does not survive being
launched without a TTY, and the ways it fails all look like application bugs rather
than launcher bugs. This page is the procedure that works and, more usefully, the six
wrong turns that each cost a debugging cycle, so the next session recognises them
instead of rediscovering them.

## The procedure

```bash
cd ui/desktop
source ../../bin/activate-hermit

# 0. ASK THE BUNDLE which port it will connect to. Do NOT assume 5173 — see
#    failure 6. Whatever this prints is the port step 1 must serve on.
grep -oE "http://localhost:[0-9]+" .vite/build/main.js | sort | uniq -c
PORT=5174   # <- set from the line above, NOT from this example

# 1. Renderer dev server — MUST name the renderer config (see failure 3).
#    The port and the bundle's baked-in URL must AGREE, which is what
#    `--strictPort` protects: without it vite silently takes the next free port,
#    Electron still asks for the baked one, and you get ERR_CONNECTION_REFUSED
#    and a blank window — which reads as a broken app rather than a mis-launched
#    one.
BIOROUTER_NO_HMR=1 npx vite --config vite.renderer.config.mts \
  --port "$PORT" --strictPort > /tmp/vite.log 2>&1 &

# 2. Electron against the built main process, detached from this shell.
#    `env -u ELECTRON_RUN_AS_NODE` is load-bearing (see failure 1).
#    $RUN is a throwaway folder: HOME, BIOROUTER_PATH_ROOT and --user-data-dir
#    keep the app and its daemon off your real profile (the standing cautions).
#    ⚠ Do NOT pass MAIN_WINDOW_VITE_DEV_SERVER_URL here — it is a build-time
#    constant, not an env var, and setting it does nothing (failure 6).
RUN=$(mktemp -d) && mkdir -p "$RUN/home"
env -u ELECTRON_RUN_AS_NODE -u ELECTRON_NO_ATTACH_CONSOLE \
  BIOROUTER_NO_HMR=1 HOME="$RUN/home" BIOROUTER_PATH_ROOT="$RUN" \
  node_modules/electron/dist/Electron.app/Contents/MacOS/Electron \
  .vite/build/main.js --remote-debugging-port=9333 --user-data-dir="$RUN/electron" \
  > /tmp/electron.log 2>&1 &
```

`.vite/build/main.js` must exist. If it does not, run `just run-dev` once at a
real terminal. `npx electron-forge start` also leaves the bundles behind, but it
may start the whole app rather than exit: forge removes `ELECTRON_RUN_AS_NODE`
(failure 1), and only a closed stdin stops it (failure 2). Set an isolated `HOME`
and `BIOROUTER_PATH_ROOT` before you run it, as in step 2.

Then drive and inspect it over the Chrome DevTools Protocol (CDP) rather than by
screenshotting the desktop:

```text
agent_browser_connect  target: 9333
agent_browser_screenshot
agent_browser_console
agent_browser_eval
```

Verify it actually came up healthy:

```bash
pgrep -f "MacOS/Electron .vite"        # renderer process
pgrep -f "target/debug/biorouterd"     # the daemon the app spawned
curl -s http://localhost:9333/json/list | grep '"title"'   # -> "Biorouter"
```

## The six failures, and how to recognise each

**1. `ELECTRON_RUN_AS_NODE=1` in the environment — Electron exits instantly.**
Agent shells commonly export this. Electron then runs `main.js` as plain Node
and quits with no window and *no error*. Always
`env -u ELECTRON_RUN_AS_NODE`. `ELECTRON_NO_ATTACH_CONSOLE` is worth clearing
alongside it. This stops a direct Electron launch only. Forge 7.11.1 removes the
variable from the app's environment (`@electron-forge/core`, `dist/api/start.js`),
so a forge start that exits at once is failure 2, and one whose stdin stays open
starts the whole app, under your real `HOME` unless you set another.

**2. `electron-forge start` exits the moment stdin closes.** Forge reads stdin
for its interactive `rs` (restart) command, so `nohup … < /dev/null &` gives it
EOF and it tears the app down with it. Identical symptom to failure 1 —
`✔ Launched Electron app`, then nothing — which is what makes the two so easy to
confuse. Wrapping it in `script -q /dev/null` to fake a pty does **not** fix it.
Launch the Electron binary directly instead, as above.

**3. A bare `npx vite` serves the app with no CSS.** The renderer's Tailwind
plugin is configured in `vite.renderer.config.mts`; plain `npx vite` picks up
`vite.config.*` (absent here) and silently serves an unstyled page. Symptom: the
app *works* — sidebar entries, chat history, everything present and clickable —
but renders as unstyled HTML in serif type with no layout. It reads as "the app
is broken"; it is actually the launcher. Always pass
`--config vite.renderer.config.mts`.

**4. Do not verify with `screencapture` of the whole screen.** It captures
whatever the user has open — mail, browser history, private documents — and the
app window is usually *behind* the editor anyway, so the capture does not even
show what you need. `AXRaise`/`frontmost` via System Events is unreliable here.
Use the CDP screenshot: it captures the renderer only, needs no focus, and
cannot pick up the user's other windows.

**5. The standalone dev bundle hangs on an *invisible* modal dialog.**
`scripts/build-main-dev.mjs` emits `electron-devtools-installer` as
`const installExtension = require("electron-devtools-installer")` and then calls
it, so `installExtension` is the module namespace rather than the function.
`createChat` throws `TypeError: installExtension is not a function`, the
`app.whenReady()` handler catches it, and `dialog.showErrorBox` opens an
`NSAlert` — which nothing is there to dismiss. The main thread then blocks
forever inside `-[NSAlert runModal]`.

The symptom is the worst kind: a CDP page target *exists* but its `url` is `""`,
nothing is written to stdout or stderr, and the process ignores `SIGTERM`
(a modal run loop does not service it), so `pkill` appears to do nothing. It
reads as a hung app or a wedged renderer; it is neither.

Recognise it with `sample <pid>` — an `-[NSAlert runModal]` frame on the main
thread is conclusive. `kill -9` is required to clear it. `electron-forge start`
does **not** hit this, because it does not use the dev bundle; only the
direct-Electron path above does. The fix is to patch the (gitignored)
`.vite/build/main.dev.js` so `installExtension` is callable before launching.

**6. The dev-server port is baked into the bundle, and it is not always 5173.**
`MAIN_WINDOW_VITE_DEV_SERVER_URL` is a **build-time constant**: forge substitutes
it into `.vite/build/main.js` at build time, so the identifier does not survive
into the bundle at all (`grep -c MAIN_WINDOW_VITE_DEV_SERVER_URL .vite/build/main.js`
returns **0**). Passing it as an environment variable therefore does nothing —
the app connects to whatever URL was compiled in, and ignores you.

A bundle built by `npx electron-forge start` while forge's own vite held 5173 has
**5174** compiled in. Start your vite on 5173, and Electron asks for 5174, finds
nothing (forge's server died with forge), and logs
`Failed to load URL: http://localhost:5174/#/? with error: ERR_CONNECTION_REFUSED`
behind a blank window. The window exists, the process is healthy, and the CDP
target is present with `"title": "localhost:5174/#/?"` instead of `"Biorouter"` —
that title is the tell, and it is worth checking first.

Read the port out of the bundle rather than assuming it:

```bash
grep -oE "http://localhost:[0-9]+" .vite/build/main.js | sort | uniq -c
```

Serve vite on whichever port that prints. A healthy launch reports
`"title": "Biorouter"` on `curl -s http://localhost:9333/json/list`.

## Two things that are NOT the problem

Both were suspected and ruled out with evidence, so don't re-litigate them:

- **Electron can open windows from an agent shell.** A minimal Electron app
  launched the same way fires `ready`, creates a `BrowserWindow` and stays
  alive. The window-server attachment is fine; the failures above are all
  environmental or configuration.
- **The staged binaries are usually fine.** `ui/desktop/src/bin/{biorouter,biorouterd}`
  being Linux ELF from a prior cross-build *is* a real failure mode (the app
  quits when the backend cannot exec), but check it with
  `file ui/desktop/src/bin/biorouterd` before assuming it — a healthy tree shows
  `Mach-O 64-bit executable arm64` and `--version` prints the workspace version.
  The **`x86_64`** variant is the same trap wearing a friendlier face: a release
  run that ended on `bundle:intel` leaves an Intel binary staged, which does not
  announce itself the way an ELF does. `just copy-binary` restores the arm64 pair
  and re-signs them, so Keychain grants survive. A dev launch reads
  `target/debug/` rather than `src/bin/`, so this bites the *packaged* app first —
  restore it anyway before you trust a dev session's backend behaviour.

## Four standing cautions

- `BIOROUTER_NO_HMR=1` freezes the renderer. Without it any save under
  `ui/desktop/src/` full-reloads the page and destroys the chat session under
  test, which makes agent-driven runs fail in ways that look like app bugs.
- Launching the dev GUI can overwrite `~/.config/biorouter`. Back it up, or
  sandbox with `XDG_CONFIG_HOME`, before a session that will touch settings.
- Pick a CDP port that is not 9222 — that is usually the developer's own Chrome,
  and a debugger client will silently attach to it instead of the app.
- Set an isolated `HOME` and `BIOROUTER_PATH_ROOT` (and pass `--user-data-dir`)
  before any `electron-forge start` or Electron launch. Forge starts the app even
  with `ELECTRON_RUN_AS_NODE=1`, and under your real `HOME` the daemon reads
  `~/.claude/skills` and the project list, which can raise a macOS privacy prompt
  for the Desktop folder.

## Related documentation

- [Debugging the dev GUI with agent-browser](agent-browser-debugging.md) — driving the app over CDP once it is running, and why this repo uses port 9333
- [Diverge behavior checklist](diverge-behavior-checklist.md) — what to exercise once the app is in front of you
- [System overview](../architecture/system-overview.md) — how the Electron main process, the daemon and the renderer fit together
