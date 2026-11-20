> **What this is.** How Biorouter Copilot is driven against real fixture apps through its own
> tools on Linux, Windows and macOS, what each drive checks, how to reproduce the Linux drive in
> Docker on any machine, and what has been measured.
> **Status:** Current. Written 2026-11-20 for 1.94.6 (AD-21 and AD-22 in
> [Biorouter Copilot on agent-desktop](../design/copilot-agent-desktop.md)). The Linux drive passes;
> the Windows and macOS drives have not run yet.
> **Audience:** anyone changing Biorouter Copilot's wrapper, its engine pin or its packaging, and
> whoever runs the live macOS gate before a release.

# Biorouter Copilot fixture acceptance

Unit tests prove that the wrapper builds the right command lines and refuses the right things.
They cannot prove that a drag really reorders a list in a real app, or that Stop really lifts a
pressed button. The fixture acceptance does: it starts a small app built for the purpose, drives it
through Copilot's own tools, and reads every outcome from a state file the app writes itself,
never from what a tool reported.

## The pieces

| Piece | What it is |
|---|---|
| `crates/biorouter-mcp/tests/computer_use_fixture.rs` | The driver. Ignored by default. It serves `ComputerControllerServer`, the same MCP server the desktop app runs, and calls its tools with the dispatch meta the host's consent gate attaches (`biorouter-session-id`, `computer_use_generation`, `computer_use_grant`). |
| `scripts/copilot-fixture-gtk.py` | The Linux fixture app (GTK 3). |
| `scripts/copilot-fixture-winforms.cs` | The Windows fixture app (WinForms, C# 5), compiled to `BiorouterCopilotFixture.exe`. It is never hosted in PowerShell: Copilot treats PowerShell as a terminal and refuses to type into it. |
| `scripts/copilot-fixture-macos.swift` | The macOS fixture app (AppKit), built into `Biorouter Copilot Fixture.app` by `scripts/build-copilot-fixture-macos.sh`. |
| `scripts/test-copilot-fixture-linux.sh` | Runs the Linux drive in a throwaway X11 session: Xvfb, a session bus with the accessibility bridge, and openbox, which headed actions need to focus a window. |
| `scripts/test-copilot-fixture-windows.ps1` | Compiles the WinForms fixture with Windows PowerShell 5.1 and runs the Windows drive. |

The three apps share one set of names, window titles and state keys, so one driver serves all of
them. The main window, "Biorouter Copilot Fixture", holds a text field `FixtureInput`, a button
"Apply fixture" that copies it, a checkbox "Fixture option", a password field `FixtureSecret`, a
list `FixtureList` (Alpha, Beta, Gamma) reordered by dragging one item onto another, a button
"Drag me" whose drag carries the text "Dropped by Copilot", and a long scrolling text. A second
window, "Biorouter Copilot Fixture Drop", holds a text field `FixtureDrop` that accepts the dragged
text. The state file records the field, what was applied, the checkbox, the password field's
length (never its text), the list order, the dropped text, the scroll position and whether the
pointer's left button is down; the Windows fixture also records any console window that appeared.

## What a drive checks

In order, each against the app's own state:

1. `list_apps` shows the fixture and `get_app_state` names every target with a reference.
2. A `set_value` under a look only grant is refused, and the field stays empty.
3. `set_value`, then `click` on "Apply fixture", then `perform_secondary_action` `check` all land.
4. `type_text` into the password field is refused, and nothing is typed into it.
5. A reference from another chat is refused before anything runs.
6. Two drags reorder the list exactly: Gamma onto Alpha, then Beta onto Gamma.
7. A drag from "Drag me" to the other window's field drops the text there.
8. `scroll` moves the long text.
9. Stop (`computer_use::stop_session`, the function the host's Stop calls) in the middle of a drag:
   the call is refused as cancelled, no button stays pressed, and a reference observed before Stop
   is refused afterwards.
10. A new task after Stop starts clean, and no console window appeared on Windows.

The host's consent itself (the card, the grant, revocation) is covered by the unit tests in
`security::computer_use`, not here.

## Running it

**In CI.** `computer-use-native.yml` runs the Linux drive (`linux-fixture`, Ubuntu 24.04) and the
Windows drive (`windows-fixture`, `windows-latest`) and its release gate needs both. macOS hosted
runners cannot grant Accessibility to the Biorouter Copilot bundle, so the macOS job only builds the
fixture.

**On Linux, or anywhere with Docker.** The drive needs no Linux desktop of your own. Build the
engine payload and the driver for the container's architecture, then run the script in an Ubuntu
24.04 image that has the session tools:

```bash
bash scripts/build-computer-use-helper-cross.sh linux-arm64        # or linux-x64
docker run --name copilot-fixture-base ubuntu:24.04 bash -c 'apt-get update && \
  DEBIAN_FRONTEND=noninteractive apt-get install -y python3 python3-gi gir1.2-gtk-3.0 \
  at-spi2-core xvfb xauth dbus-x11 openbox'
docker commit copilot-fixture-base biorouter-copilot-fixture:24.04
# Build the driver inside a rust:1.92-bookworm container with
#   cargo test --locked -p biorouter-mcp --test computer_use_fixture --no-run
# and copy the test binary out (here to /tmp/out/computer_use_fixture).
docker run --rm --init -v "$PWD:/src:ro" \
  -v "$PWD/target/computer-use/linux-arm64:/payload:ro" -v /tmp/out:/out:ro \
  biorouter-copilot-fixture:24.04 bash -c 'mkdir -p /tmp/home; HOME=/tmp/home \
  BIOROUTER_COPILOT_FIXTURE_DRIVER=/out/computer_use_fixture \
  /src/scripts/test-copilot-fixture-linux.sh /payload'
```

`--init` matters: without it `xvfb-run` becomes the container's first process and waits forever
for a signal it never receives.

**On Windows**, on a desktop session nobody is using:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-copilot-fixture-windows.ps1 target\computer-use\win32-x64
```

**On macOS** (the live gate, AD-22), only after the owner has allowed Biorouter Copilot in
Accessibility and Screen Recording. The drive moves the real pointer, so nobody may use the Mac
while it runs:

```bash
scripts/build-copilot-fixture-macos.sh
BIOROUTER_COMPUTER_USE_DIR=target/computer-use/darwin-arm64 \
BIOROUTER_COPILOT_FIXTURE='["target/copilot-fixture/Biorouter Copilot Fixture.app/Contents/MacOS/BiorouterCopilotFixture"]' \
cargo test -p biorouter-mcp --test computer_use_fixture -- --ignored --nocapture
```

## What has been measured

| When | Where | Result |
|---|---|---|
| 2026-11-20 | Ubuntu 24.04 arm64 in Docker on an Apple silicon Mac: Xvfb, openbox, at-spi2-core 2.52.0, the `linux-arm64` payload of agent-desktop 1.3.1 | Passed, four runs in four, about 9 s each. Both list drags and the drop into the second window land: drag and drop works within a GTK app and between two of its windows on X11. No button stayed pressed after Stop. |
| 2026-11-20 | The engine CLI alone against the GTK fixture, same container | `set-value`, `click` and `check` are delivered through the accessibility API (`semantic_api`); `drag --headed` is a physical XTEST gesture (`physical_synthetic`, 22 events) whose GTK drop handlers run. |
| 2026-11-20 | `mcs -langversion:5` (mono 6.12) | The WinForms fixture compiles as C# 5. Not yet run on Windows. |
| 2026-11-20 | `swiftc` on macOS 26 | The AppKit fixture builds. Not yet driven: that needs the owner's grants. |

Not measured yet: the Windows drive (its UI Automation path with the windows-gnu engine, AD-05),
the macOS drive, Wayland (where Copilot offers no pointer input, so drag is refused), drags between
two different apps, and file drops from Finder or a file manager.

## Related documentation

- [Biorouter Copilot on agent-desktop](../design/copilot-agent-desktop.md): the design whose gates these drives serve (AD-21, AD-22)
- [Biorouter Copilot capability](../extensions/built-in/computer-controller.md): the user facing page
- [Continuous integration](continuous-integration.md): where the fixture jobs sit among the workflows
- [Biorouter Copilot on Open Computer Use](../history/copilot-open-computer-use/README.md): the retired engine's records, including the Windows verification this replaces
