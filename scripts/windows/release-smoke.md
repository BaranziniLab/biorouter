# Windows release startup acceptance

`release-artifact-smoke.yml` verifies authenticated GitHub ZIP and Setup.exe
asset digests and sizes. The Setup check installs the real installer into an
owned disposable directory and compares its runtime payload against the ZIP.
Only the two top-level metadata files excluded by the observed Squirrel NuGet
package may be absent.

The installer first runs a distinct renderer/backend check with
`ENABLE_PLAYWRIGHT=true`. It then stops that app and invokes
`normal-release-smoke.ps1` against the installed executable. The ZIP path also
invokes the normal check independently. The normal check clears the Playwright,
development profile, external backend, shared-daemon override, server-secret and
port environment settings. Its only isolation settings are a private explicit
`--user-data-dir`, `BIOROUTER_PATH_ROOT`, account directories and keyring-off.
Chromium's loopback remote debugging port observes the production renderer; it
does not enable Biorouter's test-driver mode.

Normal acceptance requires a visible native app window, the packaged `file:`
renderer, exactly one descendant daemon with the expected executable and
listening TCP port, matching release executable hashes and live backend version,
and repeated successful status/authentication checks. A second normal window
must use the same backend and automatically receive its generated proof. The
inspector checks separate generated server/user-action keys without returning
those keys or their digests. An authenticated `/system_info` request succeeds,
an unauthenticated request is refused, `/crew/connections` without a person's
proof is refused, and that read succeeds with the automatically generated proof.
No provider or remote Crew connection is required. Native owned windows and the
renderer are checked for a temporary-password or daemon-approval conversation.
The check requires v1.92.0 or later because older apps do not honor the explicit
private Electron profile contract. The report, screenshot and application logs are retained; profile data and
exactly identified owned process descendants are cleaned up.

Windows intentionally uses an app-owned TCP backend shared across its windows.
It stops with the app and holds automatic user-action proof in memory.
`biorouterd.ts` excludes Windows from `sharedRuntime`, and `daemonRuntime.ts`
rejects persistent Windows profile attachment until an owner-protected named
pipe exists. Therefore Windows acceptance must not claim the macOS/Linux
persistent `runtime.json` / `user-action-key.json` behavior. The normal check
asserts those unsupported persistent files were not created. The per-window
fallback is selected only by `BIOROUTER_SHARED_DAEMON=0`
(or `false`, `off`, `no`); this check clears that override and proves two windows
share one backend. `ENABLE_PLAYWRIGHT` enables the app's inspector switch and
contributes to guarded development-only automation; it does not itself select
the per-window fallback. The existing Playwright installer check and this
normal startup check are separate evidence.

This supplemental test branch starts at application source
`c0aed55e9e54f646c52d3dd073a7c5315669aad6`. It changes release acceptance only;
no application code, version or packaged bytes change. Its workflow source SHA
is recorded separately from the authenticated release assets. It is held
unmerged until the v1.92.0 app release is published. The pinned source's fresh
draft smoke remains a separate publisher requirement.
