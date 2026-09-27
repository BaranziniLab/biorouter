# Crew desktop integration handoff

Implementation worktree: `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter`, branch `codex/biorouter-crew`. Product UI changes were authored by Astra. Luna owns tests, builds, computer use, and runtime acceptance. The instructions below describe the implemented launch contract; they are not evidence that a launch passed.

## Build and launch contract

Use one coordinated build of the current daemon and desktop assets. The existing `ui/desktop/scripts/build-e2e.mjs` emits the main/preload/renderer assets under `.vite`; it removes that directory first, so do not run it concurrently with another builder or active asset generation. Do not generate OpenAPI concurrently with the parent lane. The parent and Luna select and record the actual build commands and executable hashes.

After the current assets exist, launch each client as its own Electron process from `ui/desktop`. For example:

```sh
env BIOROUTER_DEV_PROFILE_ROOT=/private/tmp/biorouter-crew-profiles/alice BIOROUTER_DEV_PROFILE_NAME=alice ENABLE_PLAYWRIGHT=1 PLAYWRIGHT_CDP_PORT=9411 ./node_modules/.bin/electron .
```

Use distinct roots, names, and CDP ports for Bob and Carol (for example `bob`/`9412` and `carol`/`9413`). Start them in separate process sessions. Do not run three Forge builders. Do not set an external-backend environment variable or configure an external backend; isolated profiles refuse it. Installed/packaged builds reject this development-profile switch.

`developmentProfile.ts` executes before settings and logger imports. Each profile creates:

- `electron/`: Electron userData, settings, and persistent renderer storage. The logger writes under `electron/logs/`.
- `session/`, `temp/`, `logs/`, `home/`: dedicated Electron paths.
- `biorouter/config/`, `biorouter/data/`, `biorouter/state/`: daemon storage through `BIOROUTER_PATH_ROOT`.
- `profile.json`: profile name, root, main PID, and resolved profile paths, without secrets.

The daemon receives the dedicated HOME/USERPROFILE, XDG and temporary paths. Inherited credential environment variables are removed, keyring is disabled, and the transport's development-only key store is profile scoped. Native SSH authentication also removes inherited credential variables and the personal SSH agent. The transport explicitly selects profile SSH configuration, identities, and known_hosts. Provision verified fixture keys under each profile independently; never copy a participant's provider credentials into another profile.

Crew's header displays the profile name; the connected channel header displays the verified remote SSH username. These are presentation aids, not a substitute for Luna's process/path/hash verification.

## UI and API integration

`/crew` is a native sidebar route and is available from provider onboarding without choosing a model. All Crew HTTP requests, including metadata reads, use the existing daemon secret and per-request user-action proof.

The ordinary-chat `/crew` command carries an existing session ID into `/crew?sessionId=...`. The channel page offers an explicit destination/context grant dialog. Its submit calls the parent's typed session-grant route. The server's idle-turn guard returns its actionable refusal directly into the still-open dialog; the UI does not navigate away or announce success after a refusal. On success it returns to that exact session. Provider identity and authorization remain backend resolved.

Owned execution cards use the parent's typed `/runs` API, not broker grant records as invented execution status. They open the actual session for approvals and detailed activity. Cancellation is enabled only for active local-owned run records.

SSH authentication is a narrow main-process IPC accepting only a saved connection ID. Main fetches the authenticated daemon plan itself and directly starts native `ssh` under node-pty. Exact owner/connection/plan identity reuses an existing master. Route navigation detaches its terminal display while preserving the master for ordinary chat; explicit Close disposes the exact PTY. Renderer reload/crash/closure and app teardown retain the existing owner-scoped registry cleanup. The transport separately closes its exact control socket on disconnect/removal/update. No prompt transcript or input is persisted. Native host-key checking remains strict; the UI gives independent-verification and known-hosts import instructions, with no trust-on-first-use shortcut.

## File behavior

Uploads are bounded at 64 MiB to bound browser hashing/download memory. Larger data can be shared through the broker's real restricted `reference.create`/`reference.get` API. References are clearly marked as metadata only: not uploaded, existence/access not verified, and never executed or fetched by preview. `message.post.references` carries the opaque reference IDs.

Pending upload recovery persists only a bounded list of connection/channel/blob IDs, byte size, SHA-256, request idempotency key, and timestamp in profile-scoped renderer storage. It stores no filename, local path, bytes, credentials, or conversation history. The begin key is saved before the first remote admission. After restarting, the user chooses Resume/restore: an incomplete transfer requires reselecting matching size/hash bytes, and the broker's current offset is authoritative. A completed unsent upload can be restored to the composer without retransferring bytes. Successful message publication removes corresponding recovery metadata. Invalid metadata produces an explicit refusal and a local-record reset control; forgetting a record does not delete remote objects.

Downloads check SHA-256 before exposing a result. Raster previews are limited to PNG/JPEG/GIF/WebP; PDF and arbitrary files remain downloads. Downloads resume in memory while the view stays mounted, and are discarded on leaving the view. Offline channel history caching remains off.

## Validation status

No test, build, fixture launch, or computer-use action was executed by the UI implementation lane. Earlier Luna checks identified TypeScript issues and a filename-sanitizer lint issue, which were corrected. Restart recovery, remote-reference integration, auth-master route survival, accessibility, and the final source revision require Luna's current checks. This handoff does not mark the three-person acceptance gate complete.
