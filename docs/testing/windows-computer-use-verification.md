> **What this is.** The procedure for verifying Computer Use on Windows on a real machine, and the record of why continuous integration cannot do it.
> **Status:** Current — the verification is OUTSTANDING and blocks the 1.91.0 release.
> **Audience:** Whoever runs the Windows verification, and anyone deciding whether a red Windows acceptance job is a defect or a missing capability.

# Verifying Computer Use on Windows

The `Install packages (win32-x64)` job in
[`computer-use-package-acceptance.yml`](../../.github/workflows/computer-use-package-acceptance.yml)
is red, and **no code change can turn it green.** It asserts `--expect-status ready`, and
a GitHub-hosted Windows runner cannot reach that state. This document says why, what to run
on a machine that can, and what a pass looks like.

## Why CI cannot do this

The Windows runtime needs a signed-in interactive desktop session.
[`vendor/computer-use/README.md`](../../vendor/computer-use/README.md)
states it outright — *"Windows requires the signed-in user's interactive session and Windows
PowerShell with UI Automation support"* — and
[`scripts/test-computer-use-windows-fixture.py`](../../scripts/test-computer-use-windows-fixture.py)
opens with *"Exercise UI Automation on an isolated WinForms fixture in an interactive
session."* A GitHub-hosted runner has no such session.

**Linux passes only because the harness manufactures one.**
`computer-use-package-acceptance.py` installs `xvfb`, `openbox` and `dbus-x11` into the
container and runs the check under `Xvfb :99` inside `dbus-run-session`. That is a real
virtual desktop. Windows has no equivalent, which is the entire asymmetry: darwin's
expected-status list admits `desktop_unavailable`, Linux manufactures a desktop and demands
`ready`, and Windows demands `ready` with nothing to provide it.

So the job is not reporting a product regression. It is reporting that a capability
requiring a desktop was asked to prove itself where there is no desktop.

## What is already known — do not re-derive this

Measured on run [`35474721330`](https://github.com/BaranziniLab/biorouter/actions/runs/35474721330),
after per-phase stderr breadcrumbs were added to `doctor`:

    cold: 34.91s, exit 0, processes_left_behind: []
    warm: exceeded its 20s budget
    Phase trace (stderr): [doctor] start (+0.00s)

Three conclusions, each of which retires an earlier guess:

1. **Not a budget-summing problem any more.** `handle_doctor` used to run the dependency
   check and the Computer Use probe in series, so their worst cases added. They are
   concurrent now and cold went from *exceeding* 40 s to completing in 34.91 s.
2. **Not first-execution antivirus scanning.** Warm is not faster than cold. The ~46 s
   figure in earlier notes invited that hypothesis; the cold/warm pair refutes it.
3. **The helper never answers.** 34.91 s is `Runtime::doctor`'s bound plus its 5 s
   shutdown. The probe spends its entire bound waiting for a line that never arrives.

A related defect was found and fixed while measuring this, and it changes what you will
see: the Windows helper bounds its own PowerShell call at 30 s and answers
`missing_dependency` / *"Windows runtime timed out after 30s"* on expiry — but
`Runtime::doctor` also allowed 30 s and starts strictly earlier, so that answer could never
be delivered. The outer bound is now 45 s. **On a machine where PowerShell genuinely cannot
be reached you should now see the helper's own sentence rather than a bare "could not
check".** If you still see the bare message, the fix did not take and that is itself a
finding.

## Prerequisites on the Windows machine

- A **signed-in, interactive desktop session**. Not RDP-disconnected, not a service
  context, not Session 0. If the screen is locked the UI Automation calls will not behave
  as a user's would.
- Windows PowerShell (`powershell.exe`, the 5.x one — the runtime invokes it by that name
  with `-MTA -NoProfile -NonInteractive -ExecutionPolicy Bypass`).
- Python 3.12, Node 24, Go 1.26.8, Rust 1.92, `protoc`.
- A checkout at the commit you intend to ship, clean.

⚠ **The backends are not built by the acceptance script on Windows.** For non-darwin
targets it expects them to exist already at `target/x86_64-pc-windows-gnu/release/`, with a
`package-source-commit.txt` whose contents equal `git rev-parse HEAD`; it refuses otherwise
with *"Cross-build artifact came from a different source revision"*. Note the triple is
**`-gnu`, not `-msvc`** — the shipped Windows backend is cross-compiled with the GNU
toolchain. Get them either by downloading the `package-backends-x86_64-pc-windows-gnu`
artifact from the `cross` job of a run at your commit, or by cross-building them the way
`scripts/release.sh` does.

## The procedure

```powershell
python scripts\test-installed-computer-use.py
python scripts\computer-use-package-acceptance.py build win32-x64
python scripts\computer-use-package-acceptance.py verify win32-x64
```

The first is a pure unit suite and needs no desktop; run it first so a harness fault is
never mistaken for a runtime one. The second builds a genuine candidate package. The third
extracts it, verifies the packaged dependencies, runs the installed `biorouter doctor`
twice (cold and warm), and then exercises `test-computer-use-windows-fixture.py` against an
isolated WinForms fixture.

Evidence lands in `target\package-acceptance\`. Keep the whole directory.

## What a pass looks like

    doctor --format json → computer_use.status == "ready"
                           permissions.accessibility == true
                           permissions.screen_recording == true
                           integrity == "verified"
                           development_override == false
    cold and warm attempts both well inside their budgets (40s / 20s)
    the WinForms fixture reports a real UI Automation interaction

## What to record if it does not pass

The point of running this on a real machine is to separate three things the CI failure
cannot. Please record which one you see:

1. **The helper answers, and says something specific.** Best case — the diagnosis is now in
   hand. Capture the `message` verbatim.
2. **The helper answers `ready` and a later step fails.** Then the readiness path is fine
   and the defect is in the fixture or the tools; capture the failing step.
3. **The helper still never answers, on a machine with a real desktop session.** Then the
   hang is not about the session at all, and the next place to look is `runPowerShell` in
   `apps/OpenComputerUseWindows/main.go` — specifically whether `cmd.CombinedOutput()` can
   block past its own context expiry when `powershell.exe` leaves a grandchild holding the
   inherited pipe. That is the same pipe-EOF mechanism that caused two earlier hangs in this
   subsystem, and it has not been ruled out here.

## Afterwards: the CI expectation

Once the real behaviour is known, decide what the job should assert. The honest options are
to give Windows a real desktop session on a self-hosted runner, or to admit the achievable
non-ready statuses the way darwin already does. **Do not relax `--expect-status` before the
real behaviour is known** — that converts an unanswered question into a permanently green
job, which is how this class of defect stays hidden. The Linux `/opt` prefix, the Intel
`node-pty` architecture and this hang were all found by a check running somewhere it had
never run; none of them crashed anything.

## Related documentation

- [Computer Use implementation status](../design/computer-use-implementation-status.md) — the acceptance ledger and every gate's evidence
- [Computer Use integration plan](../design/computer-use-integration-plan.md) — the design this verifies
- [Testing](README.md) — what tests in this repository may safely touch
