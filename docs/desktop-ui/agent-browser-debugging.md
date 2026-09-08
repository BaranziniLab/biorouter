# Debugging the dev GUI with agent-browser

> **What this is.** A guide to driving the BioRouter Electron dev GUI from an ordinary
> terminal using the `agent-browser` CLI over the Chrome DevTools Protocol, and an
> explanation of why this repo exposes that protocol on port 9333 rather than the
> Playwright default.
> **Status:** Current.
> **Audience:** developers working on the desktop GUI, and agents driving it.

Reproducing a desktop UI bug means clicking through the real app. This repo ships a
bundled `playwright-electron` MCP (Model Context Protocol) server for that, but its
endpoint is resolved once at session start and cannot be re-pointed afterwards, which
pins it to port 9222. `agent-browser` avoids that constraint: it targets a Chrome
DevTools Protocol (CDP) port **per command**, so you can point it wherever the dev app
actually landed.

[agent-browser](https://github.com/vercel-labs/agent-browser) is a fast native (Rust)
CLI that drives any Chromium browser — including Electron apps — over CDP. Because
BioRouter's desktop app is Electron, the dev GUI already exposes a CDP port, so
agent-browser can snapshot, click, type, read the console, eval JS, and screenshot it.

## One-time install

```bash
npm install -g agent-browser     # ships a native arm64/x64 binary via postinstall
agent-browser --version          # confirm the binary installed
```

This guide was written against agent-browser 0.30.x.

You do not need `agent-browser install` (the Chrome download step) — agent-browser
attaches to the Electron app's own Chromium, not to a standalone browser.

## Why a dedicated port (9333, not 9222)

The Playwright default port is **9222**, but a regular Google Chrome is often already
listening there. If the Electron app loses the race to bind 9222, agent-browser
silently connects to Chrome instead, and you will see Google or YouTube tabs rather
than BioRouter. The `agent-browser-ui` workflow therefore exposes CDP on **9333** via
`PLAYWRIGHT_CDP_PORT`, which `ui/desktop/src/main.ts` honors.

## Launch and drive the app

Terminal 1 — build the debug backend and launch the dev GUI with CDP on 9333, with
config sandboxed under an isolated `BIOROUTER_PATH_ROOT` so the dev app cannot clobber
`~/.config/biorouter`:

```bash
just agent-browser-ui          # or: just agent-browser-ui 9444  to override the port
```

> **Warning.** Set `BIOROUTER_NO_HMR=1` for this workflow. It freezes the renderer — no
> Vite watching, no hot reload. Without it, any save anywhere under `ui/desktop/src/`
> full-reloads the page and destroys the chat session under test, which makes
> agent-browser runs fail in ways that look like app bugs.

Terminal 2 — connect once, then interact:

```bash
agent-browser connect 9333         # binds this session to the dev app's CDP
agent-browser snapshot -i          # accessibility snapshot with refs (@e1, @e2, ...)
agent-browser click @e5            # interact by ref
agent-browser fill @e3 "hello"
agent-browser screenshot ui.png    # visual state
agent-browser console --json       # renderer console + errors
agent-browser errors               # just the errors
agent-browser eval "window.location.href"
agent-browser close                # detach (does NOT quit the app)
```

Re-run `agent-browser snapshot -i` after any navigation or state change to get fresh
refs. `agent-browser skills get electron` and `agent-browser skills get core --full`
print the canonical workflows and the full command reference.

If the app was already running on the wrong port, quit it and relaunch — the
`--remote-debugging-port` switch is only read at startup.

## Reset the viewport you set

> **Rule.** Any driver that sets the viewport — `agent-browser set_viewport`,
> Playwright's `setViewportSize`, DevTools device mode — **must reset it before it
> detaches.** Leaving it set is the single most expensive mistake available at
> this endpoint.

All three apply `Emulation.setDeviceMetricsOverride`, which pins the renderer's
viewport to a fixed size regardless of the real window. **The override outlives
your `agent-browser close`** — measured: a viewport pinned at 1440×900 was still
pinned after the session that set it had closed. Detaching does not undo the
emulation, it only removes the one session that could have. What the next person
sees is an app that no longer scales with its window, with a blank band below and
to the right of the page. It reads as a CSS regression and is not one. This repo
has burned hours on it at least twice; the full story, and the measurements, are
in [When the app "stops scaling with the window"](window-scaling-regressions.md),
under *Viewport emulation pins `innerWidth`*.

Worse, an **orphaned** override freezes `outerWidth` as well as `innerWidth`, so
the page reports a window size that the window manager disagrees with — the OS
resize really happened, and the page denies it. That is why the reset belongs in
the session that set it, while it still can.

How to reset, in order of preference:

```bash
# 1. Best: in the SAME session that set it, before detaching.
agent-browser set_viewport 0 0        # or the tool's own clear/reset
# 2. Check any instance at any time — exit 0 clean, 1 pinned, 2 harness problem.
cd ui/desktop && npm run cdp:viewport-check -- 9333
# 3. Stopgap only, from another session. Read the caveat below first.
npm run cdp:viewport-check -- 9333 --clear
```

⚠ **A clear from a *different* CDP session is not a cure — it is a coin flip.**
Chromium keeps emulation state per DevTools session, so a foreign session has to
apply its own override before it can drop one, and what it hands back is not the
state the page started in. Measured 2026-09-08 on the affected instance: the
clear restored `inner == outer` immediately and left the renderer
**half-frozen**, following the next OS resize once and then stopping with
`outerWidth` stale. Measured on a fresh instance pinned the same way: it
recovered completely. Nothing visible tells the two apart, and the clean-looking
outcome is the one that costs you the next hour. **Restart the instance** unless
a restart would cost you the state you are debugging.

In a development build the app now warns about this itself — a `Viewport pinned
at …` line in the renderer console — so a driver that forgets the reset at least
leaves evidence for whoever finds the app next.

## Optional: the agent-browser MCP server

`.mcp.json` also registers an `agent-browser` MCP server (`agent-browser mcp --tools
all`) so a harness can call the tools directly. As with any MCP server, it is loaded at
session start; use the `connect` or `cdp` tools at runtime to point it at port 9333.

## Related documentation

- [When the app "stops scaling with the window"](window-scaling-regressions.md) — the impostor a forgotten `set_viewport` creates, and how to tell it from a real layout bug.
- [Diverge behavior checklist](diverge-behavior-checklist.md) — the desktop QA script whose `[UI]` items you drive with exactly this setup.
- [Environment variables](../configuration/environment-variables.md) — reference for `PLAYWRIGHT_CDP_PORT`, `BIOROUTER_PATH_ROOT`, and the other knobs used above.
- [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md) — what to collect once you have reproduced a GUI failure.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md) — check here before assuming a dev-GUI symptom is a new bug.
