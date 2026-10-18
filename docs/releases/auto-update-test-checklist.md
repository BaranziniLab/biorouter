# Auto-update test checklist

> **What this is.** The verification plan for the one-click "Restart & Update" flow on macOS and the assisted-download fallback on Windows and Linux. It has two parts: a frozen record of the tests already executed (Section A), and an unrun checklist to work through for each release (Sections B–H).
> **Status:** Current. Sections B–H are the live pre-release QA script for auto-update; Section A is a completed evidence log frozen at the 1.86.0 release cycle, with the most recent entry executed 2026-07-14.
> **Audience:** maintainers cutting a release.
>
> **Identifier key.** Section letters A–H are referenced by other sections of this file ("see Section A", "Sections B/C below") — keep them when editing. In Sections B–H, **`X`** is the throwaway test release you cut, and **`X-1`** is the older build already installed on the test machine.

BioRouter's updater replaces the old "open the download page, grab the DMG, drag it into Applications, quit the app" experience with a one-click **Restart & Update** button driven by `electron-updater`. On macOS the app swaps itself in place and relaunches; on Windows and Linux the app auto-downloads the installer and guides the user through it. This checklist exists because the final in-place swap can only be proven on a signed and notarized build running on real hardware, so a portion of the verification cannot be automated in CI.

## What changed

| Area | Before | After |
|------|--------|-------|
| Startup "update available" modal | Opened `biorouter.ucsf.edu/download` in a browser | Listens to `electron-updater` events; background-downloads; shows a **Restart & Update** button that quits + installs + relaunches in one click |
| Settings → Check for Updates | Opened the GitHub release page | Drives the same `electron-updater` pipeline with progress + one-click install |
| Release artifacts | 7 (5 GUI + 2 CLI) | 11 — 5 GUI + 2 CLI + the headless Linux tarball + `Biorouter-darwin-arm64-{ver}.zip`, `Biorouter-darwin-x64-{ver}.zip`, and `latest-mac.yml` for in-place macOS updates |
| Dependency / CLI checks | unchanged | **unchanged** (DependencySetupModal "Biorouter CLI Update" card behaves exactly as before) |
| User config / sessions / extensions | preserved | **preserved** (live in `~/.config/biorouter`, never touched by app replacement) |

Files touched:
`ui/desktop/src/utils/updaterState.ts` (new pure reducer),
`ui/desktop/src/utils/autoUpdater.ts` (richer persisted state),
`ui/desktop/src/preload.ts` (disposer + wider state type),
`ui/desktop/src/components/UpdateAvailableModal.tsx`,
`ui/desktop/src/components/settings/app/UpdateSection.tsx`,
`ui/desktop/scripts/generate-update-manifests.js` (new),
[`scripts/release.sh`](../../scripts/release.sh) (`mac-manifest` phase + publish/verify wiring).

## Platform support matrix

| Platform | One-click in-place auto-update? | Mechanism |
|----------|-------------------------------|-----------|
| **macOS arm64** | ✅ Yes | Squirrel.Mac via signed zip + `latest-mac.yml`; symlinked terminal `biorouter` auto-updates because it points into the in-place-replaced `.app` bundle |
| **macOS x64 (Intel)** | ✅ Yes | same, x64 zip |
| **Windows** | ⚠️ Assisted | plain `.zip` has no electron-updater installer; auto-downloads to `~/Downloads`, guided install. Terminal `biorouter` re-link via the existing CLI card |
| **Linux (.deb/.rpm)** | ⚠️ Assisted | no electron-updater in-place installer; auto-downloads installer, guided install |

> **Note.** The user's primary platform is macOS, where the full one-click experience is
> delivered. Windows and Linux keep the improved assisted-download flow (still
> better than "open a web page").

## Section A — executed evidence

Everything in this section has been run and recorded. The ticked boxes are a
historical record, not a to-do list — re-run the commands rather than trusting
the ticks if you need current numbers.

### Automated tests

All of the following were run and are green (48 automated tests + 3 production
bundle builds). They cover the state machine, the actual React UI wiring, the
**real electron-updater parser/arch-selection**, the manifest generator, and a
full Electron bundle build — i.e. everything except the irreducible "click the
button on a physically notarized app" step (Section B/C, which needs Apple
notarization creds + real hardware).

- [x] `npx vitest run src/utils/updaterState.test.ts` — **23** unit tests: reducer, version compare, snapshot recovery, monotonic progress, sticky-downloaded, error handling.
- [x] `npx vitest run src/components/UpdateAvailableModal.test.tsx` — **7** component-integration tests: renders nothing until an event; progress bar → one-click **Restart & Update** button → clicking it calls `installUpdate()`; pre-mount recovery via `getUpdateState`; "Later" records dismissal; error state shows no fake install button; a post-download error doesn't hide the install button; dismissed-version is not re-prompted.
- [x] `npx vitest run src/components/settings/app/UpdateSection.test.tsx` — **5** component-integration tests: current version shown; **Check for Updates** drives `electron-updater` → progress → one-click **Restart & Update to X** calls `installUpdate()`; up-to-date; error surfaced; ready-to-install recovered on a freshly opened panel.
- [x] `node scripts/generate-update-manifests.test.js` — **7** generator tests: base64 SHA-512, both-arch listing, arch-token distinguishability, YAML shape, single-arch, error cases.
- [x] `node scripts/electron-updater-compat.test.js` — **6** integration tests against the **real** `electron-updater` code (`parseUpdateInfo` / `resolveFiles` / `findFile` from `node_modules`): our `latest-mac.yml` parses into a valid `UpdateInfo`; GitHub asset URLs resolve correctly; an **arm64 client selects the arm64 zip** and an **Intel client the x64 zip** via MacUpdater's exact arch filter; every selected file carries the sha512 electron-updater verifies; a tampered checksum is detectable.
- [x] `node scripts/run-live-update-e2e.js` — **LIVE end-to-end inside a real Electron 39.8.10 process** driving the real `electron-updater` engine against a locally-served release built from our generator. Proven for real: (a) **valid** path — `checking-for-update` → `update-available` (version parsed from our `latest-mac.yml`) → real `download-progress` to 100% → **`update-downloaded`**, which is the exact event that flips the one-click **Restart & Update** button on; (b) **tampered** path — a corrupted served zip is **rejected with `sha512 checksum mismatch`** and never reaches the installable state. This executes the entire update pipeline up to — but not including — the final `quitAndInstall` OS swap (the only step that needs a notarized signature; see below).
- [x] `npm run typecheck` — clean.
- [x] `npx eslint <all changed + test files> --max-warnings 0` — clean.
- [x] `npx vite build -c vite.renderer.config.mts` **and** `-c vite.main.config.mts` **and** `-c vite.preload.config.mts` — all three Electron bundles build with the new code (renderer + main-process `autoUpdater.ts` + `preload.ts`).
- [x] `bash -n scripts/release.sh` — syntax OK.
- [x] Manifest sanity against the real built zips in `out/make/zip/darwin/*`: each `sha512` equals `openssl dgst -sha512 -binary <zip> | base64`, and each `maker-zip` contains `Biorouter.app/` + `Contents/_CodeSignature` at the archive root (Squirrel.Mac requirement).

> **Note.** Pre-existing baseline: the repo has **32 Vitest failures unrelated to this
> work** (e.g. `LeadWorkerSettings`, `DashboardProvider`, `App.test.tsx`),
> identical on the clean tree (`git stash -u` → run → compare). This branch adds
> 35 passing Vitest tests and **0 new failures** (359→394 passed, 32→32 failed).
>
> _Snapshot taken at the time of that release; the names above are what was
> actually observed then. `DashboardProvider` and `App.test.tsx` no longer exist
> (dashboard mode was removed on 2026-07-18 — see the
> [dashboard mode removal record](../history/dashboard-mode/README.md)) — left
> unedited because rewriting a recorded measurement would falsify it. Re-measure
> rather than read these as current._

### Live notarized swap — the full one-click update, end-to-end

The complete one-click update — including the macOS Squirrel.Mac in-place swap
of a **signed + notarized** build, the step previously called out as
hardware/credential-gated — has now been **executed and passes**.

Procedure (scripted by [`ui/desktop/scripts/notarized-swap-test.sh`](../../ui/desktop/scripts/notarized-swap-test.sh)):

1. Built two real apps under hermit Node 24 (reusing the arm64 Rust backend):
   **N = 1.86.0 signed + notarized + stapled** (the update payload) and
   **O = 1.85.5 signed** (UCSF identity, the app under test). Both verified:
   `codesign --verify --deep --strict` ok, UCSF Developer ID → Apple Root CA
   chain; N's `xcrun stapler validate` = "The validate action worked!".
2. Generated `latest-mac.yml` from N's zip and served N + manifest over local
   HTTP (electron-updater's generic-feed layout).
3. Launched O pointed at that feed (sandboxed `XDG_CONFIG_HOME`, isolated
   `--user-data-dir`, so the user's real install/config is untouched).
4. Observed, from O's own log: `Update feed override active` → `Checking for
   update` → update found → handed to **native Squirrel.Mac**
   (`…requested by Squirrel.Mac, pipe …Biorouter-darwin-arm64-1.86.0.zip`;
   `Download completed to …com.electron.biorouter.ShipIt/update…`) → app quit
   and the on-disk bundle **swapped 1.85.5 → 1.86.0**.
5. Verified the swapped bundle: version **1.86.0**, UCSF-signed, **stapler
   validate passes** (notarized), arm64 backend.

Result line: `PASS: notarized in-place auto-update swap succeeded (1.85.5 -> 1.86.0)`.

This exercises exactly the user-facing flow: an installed older build detects a
newer release, downloads the notarized app, and — on the one-click install —
Squirrel.Mac validates the notarized signature and swaps the app in place, then
relaunches on the new version. Settings/config are preserved (the swap replaces
only the bundle; `~/.config/biorouter` is untouched).

Notes and gotchas captured while executing this:

- **Build under hermit Node 24**, not Homebrew Node 26 — the packager silently
  no-ops (event loop empties, exit 0, no app) under Node 26.
- **`src/bin` must hold the macOS arm64 backend**, not the Linux ELF binaries a
  prior Linux release leaves behind (`just copy-binary` / restage from
  `target/release/`) — otherwise the packaged backend can't exec and the app
  quits before the updater runs.
- New env knobs (all guarded, off by default): `BIOROUTER_UPDATE_FEED_URL`
  (generic feed for self-hosted/enterprise + testing), `BIOROUTER_UPDATE_AUTO_INSTALL=1`
  (test-only deterministic install, gated behind the feed override),
  `BIOROUTER_SKIP_NOTARIZE=1` (fast signed-but-not-notarized builds in `forge.config.ts`).

### Live Electron-runtime end-to-end run — one caveat to reproduce

`scripts/run-live-update-e2e.js` drives the **real** electron-updater engine in
a **real Electron 39.8.10 process** through the entire update pipeline and
passes (see the automated tests above). It proves the full happy path reaches the
`update-downloaded` state the one-click button depends on, and that a tampered
download is rejected by the real sha512 integrity check.

Reproduce caveat: this environment did not ship the Electron binary in
`node_modules/electron/dist` (only `LICENSES.chromium.html`; `path.txt` absent),
and the @electron/get re-download is network-blocked. It was made runnable by
populating the runtime from the on-disk @electron/get **cache**:

```bash
CACHE=~/Library/Caches/electron/<hash>/electron-v39.8.10-darwin-arm64.zip
rm -rf node_modules/electron/dist && mkdir -p node_modules/electron/dist
unzip -q "$CACHE" -d node_modules/electron/dist
printf 'Electron.app/Contents/MacOS/Electron' > node_modules/electron/path.txt
xattr -dr com.apple.quarantine node_modules/electron/dist/Electron.app   # clear Gatekeeper quarantine
# run Electron with ELECTRON_RUN_AS_NODE unset (this env sets it to 1):
env -u ELECTRON_RUN_AS_NODE node scripts/run-live-update-e2e.js
```

On a normal dev machine `npm install` provides the Electron binary and the
runner works directly (it SKIPs gracefully if no Electron runtime is present).

The single step the live runner does **not** execute is the final
`quitAndInstall` OS swap: on macOS, Squirrel.Mac validates the downloaded app's
**code signature** before replacing the bundle, so it only completes for a
signed + notarized build — which is exactly Sections B/C below.

### Conditional sidebar UPDATE button — executed 2026-07-14

The updater checks shortly after every launch and then every three hours
while BioRouter stays open. Detection is non-interruptive: a small **UPDATE**
button appears above the BioRouter identity in the lower-left sidebar only when
a newer version is known. Clicking it opens the existing progress/restart
prompt; after BioRouter restarts on the current version, the button is absent.

- [x] `src/utils/updateCheckSchedule.test.ts` — **2** tests prove the startup
  delay, exact three-hour cadence, repeated checks, and cancellation.
- [x] `src/components/BioRouterSidebar/SidebarUpdateButton.test.tsx` — **5**
  tests cover hidden/current, visible/newer, persisted-state recovery,
  recheck/error stability, and button-to-modal integration.
- [x] Focused updater suite — **46/46** tests passed across cadence, reducer,
  sidebar, modal, and Settings update flows.
- [x] Full desktop suite — **104/104 files, 843/843 tests** passed.
- [x] Focused ESLint/Prettier, renderer/main/preload production builds,
  manifest compatibility, and the real Electron updater end-to-end run all passed.
- [x] Real Electron dev-app QA confirmed the button's lower-left placement,
  verified no modal appears on detection, clicked the button through the
  accessibility surface, and observed the normal update prompt open.

### Dropped downloads and the restart that did nothing, executed 2026-10-18

Two reports against 1.93.0: "Update download failed" with
`net::ERR_CONNECTION_RESET` on the way to 1.93.1, and, from earlier releases,
"the app doesn't restart after the update". Both were real, and both are fixed
in 1.93.2.

- **The download.** electron-updater fetched the ~250 MB zip in one request with
  no retry, so one reset ended the update and the next attempt was the
  three-hourly check, from byte 0. Downloads now go through
  `utils/resumableDownload.ts`: a dropped connection is followed by a
  `Range: bytes=<n>-` request for the rest, the checksum is taken over the
  finished file, and the dialog has **Try again** instead of only "Download from
  Biorouter".
- **The restart.** Three ways **Restart & update** did nothing, each now either
  works or says why (`utils/macUpdateInstall.ts`):
  - electron-updater's `update-downloaded` fires before Squirrel.Mac has staged
    the update, and the button appeared then. A click while Squirrel was still
    working did nothing visible, and quitting by hand then installed the update
    WITHOUT a relaunch. The dialog now says **Preparing the update…** until
    Squirrel.Mac's own `update-downloaded`.
  - A Squirrel.Mac failure after the download (a read-only or translocated app,
    a bad signature) was hidden by the rule that a background error never hides
    a ready update. It is now `install-failed`, shown with its reason.
  - `isUsingGitHubFallback` stayed set for the life of the app after one failed
    check, and from then on a macOS install went to the assisted path, threw
    "Update file path not found" and was ignored by the renderer. The install now
    follows whoever downloaded the update, and the renderer awaits the result.
- **No re-staging.** Every three-hourly check downloaded the same update again
  and had Squirrel.Mac stage it again (ShipIt's log showed an "install request"
  every three hours). A version already downloaded is no longer fetched again.
- **Windows.** The assisted install classified the Downloads FOLDER rather than
  the downloaded file, so a downloaded `Biorouter-Setup-<ver>.exe` got the zip
  instructions and an "Open Folder Only" button that the installer must never be
  offered.

Evidence:

- [x] `node scripts/run-live-update-e2e.js`: **4 passed** in a real Electron
  39.8.10 with the real electron-updater. New `reset` mode: the server resets
  the connection 40% into the zip; with the app's download installed the second
  request asks for `bytes=2516582-` and the update reaches `update-downloaded`,
  and with stock electron-updater the same reset fails with
  `net::ERR_CONNECTION_RESET`, the error in the report.
- [x] `src/utils/resumableDownload.test.ts` (28), `macUpdateInstall.test.ts`
  (13), `resumableUpdaterDownloads.test.ts` (9, three of which read the
  installed electron-updater so an upgrade that moves its internals fails
  there), and the reducer, dialog and Settings tests for the new states.
- [x] `scripts/notarized-swap-test.sh` now passes only on ShipIt's
  "Successfully launched application" line for the staged copy. It used to pass
  on the swap alone, which a ShipIt that installed and never relaunched also
  produces. `FEED_VER` lets one signed build serve as its own update.
- [x] Run against the signed and notarized 1.93.2 arm64 build, serving itself
  as 99.0.0: **PASS**. Downloaded at 11:23:20.2, staged by Squirrel.Mac at
  11:23:21.9 (the app quit on that event, since the install had been requested
  while it staged), installed by ShipIt at 11:23:23.0 and relaunched at
  11:23:24.0. Three things the script needed, each of which made a working
  update look broken:
  - It launches the app through `open`, as Finder does. Started from an agent
    shell, Squirrel.Mac wrote its install request but ShipIt, a per-user launchd
    job, did not run, or ran minutes later.
  - `--user-data-dir` must be mode 0700 (`applicationDataProfile.ts`), or the app
    stops at a modal error before it logs anything.
  - `BIOROUTER_PATH_ROOT` keeps the test app off the installed app's shared
    background service. ShipIt's state and electron-updater's download cache are
    still shared with an installed Biorouter, so a run replaces any update that
    app had staged, and a ShipIt job it left waiting must be removed first
    (`launchctl remove com.electron.biorouter.ShipIt`).

### Why Sections B and C still need a human and hardware

Everything that can be validated without minting an Apple-notarized binary and
installing it on a physical Mac has been validated above — including the real
electron-updater consumption of our manifest. The remainder (`scripts/release.sh
publish` of a signed/notarized release, then clicking **Restart & Update** on an
installed older build and watching the OS relaunch into the new version) requires
Apple Developer notarization credentials, ~10-minute platform builds, and a
Finder/Applications install — none of which exist in a CI/sandbox. Run them on a
release machine using the steps below.

## Pre-release checklist (Sections B–H)

Work through these for each release. The boxes below are intentionally
unticked — they are a template to fill in per release run, not a record of
past work. Cut a throwaway test release `X` that is **one patch newer** than an
installed build `X-1`, then proceed.

### Section B — release pipeline

Requires building a real signed and notarized release.

- [ ] `scripts/release.sh bump X` updates all 6 version-bearing files in lockstep (`scripts/check-version-consistency.sh` passes).
- [ ] `scripts/release.sh mac-arm64 X` and `mac-intel X` produce:
  - [ ] `ui/desktop/out/make/Biorouter-X-arm64.dmg` and `-x64.dmg`
  - [ ] `ui/desktop/out/make/zip/darwin/arm64/Biorouter-darwin-arm64-X.zip`
  - [ ] `ui/desktop/out/make/zip/darwin/x64/Biorouter-darwin-x64-X.zip`
- [ ] Each darwin zip contains `Biorouter.app/` at the **root** with a `Contents/_CodeSignature` (`unzip -l … | head`).
- [ ] The `.app` inside each zip is signed + **notarized**: `codesign -dv` and `xcrun stapler validate` pass on `out/Biorouter-darwin-<arch>/Biorouter.app`.
- [ ] `scripts/release.sh mac-manifest X` writes `out/make/latest-mac.yml`; it references **both** `Biorouter-darwin-arm64-X.zip` and `Biorouter-darwin-x64-X.zip` with correct sizes and base64 SHA-512.
- [ ] `scripts/release.sh verify X` reports all 9 file artifacts present + the `latest-mac.yml` arch-zip cross-check ✓, and verifies the local provenance manifest binds all 10 assets to the current source SHA.
- [ ] The draft contains **10** assets (5 GUI + 2 CLI + 2 darwin zips + `latest-mac.yml`). `scripts/release.sh publish X` must verify every uploaded SHA-256 digest against the local files before publication. Confirm with `gh release view vX --json assets --jq '.assets[].name'`.

### Section C — macOS GUI auto-update, the core flow (arm64 **and** Intel)

Install `X-1` into `/Applications`, launch it, then (with release `X` published):

- [ ] Within ~5s of launch, the lower-left **UPDATE** button appears without interrupting the user; clicking it opens **"Downloading update…"** with current vs new version and a live progress bar advancing 0→100%.
- [ ] During download the app stays fully usable (not blocked/frozen).
- [ ] When the download finishes, the modal says **"Preparing the update…"** for a few seconds while Squirrel.Mac stages it, then switches to **"Update ready to install"** with a **Restart & Update** button.
- [ ] Clicking **Restart & Update**: the app quits, the new `.app` is installed in place, and BioRouter **relaunches automatically** on version `X` — no Finder, no DMG, no drag-and-drop, no manual quit.
- [ ] After relaunch, **About / Settings shows version `X`**.
- [ ] Repeat the entire flow on an **Intel** Mac (or Apple Silicon under Rosetta) and confirm it pulls the `x64` zip (check the app log: `electron-updater` logs the chosen file URL — it must contain `x64`, not `arm64`).
- [ ] Apple Silicon pulls the `arm64` zip (log file URL contains `arm64`).

### Section D — macOS "Later" dismissal and the Settings path

- [ ] Clicking **Later** dismisses the modal; because `autoInstallOnAppQuit = true`, quitting the app then installs the staged update — next launch is on `X`.
- [ ] Per-version dismissal: dismissing version `X` does not re-nag for `X` on the next launch (localStorage `biorouter:update-modal-dismissed-version`), but the **ready-to-install** state still surfaces.
- [ ] Settings → **Check for Updates**: when up to date shows "Biorouter is up to date."; when `X` is available it downloads with a progress bar and then shows a **Restart & Update to X** button that performs the same one-click install.
- [ ] Tray icon shows the update badge while an update is available; the tray "Update Available…" / "Check for Updates" items open Settings → update section.

### Section E — terminal and CLI propagation

- [ ] **macOS:** before update, `which biorouter` resolves to a symlink into `/Applications/Biorouter.app/Contents/Resources/bin/biorouter`. After the one-click update + relaunch, run `biorouter --version` in a **new** terminal → it reports `X` automatically (the symlink target was replaced in place). No second click needed.
- [ ] If the terminal `biorouter` was installed standalone (deb/rpm) or copied (Windows), the existing **"Biorouter CLI Update"** card appears post-update and its button re-links to the bundled `X` binary (`biorouter setup-path`). Confirm this card's behavior is **unchanged** from before.
- [ ] `biorouterd --version` (the daemon bundled in the app) reports `X` after update — the GUI spawns the bundled daemon, so it always matches the app.
- [ ] Launching the CLI from the GUI ("Open in terminal") starts the `X` CLI.

### Section F — settings, config and data preservation

All of this must survive the update. Before updating, note the contents of
`~/.config/biorouter/`. After the one-click update + relaunch, verify **nothing
is lost or reset**:

- [ ] `~/.config/biorouter/config.yaml` — providers, selected default provider/model, and **extensions** list unchanged.
- [ ] Secrets still resolve from the OS keychain (no re-entry of API keys; at most one Keychain prompt — "Always Allow").
- [ ] `~/.config/biorouter/sessions/` — prior chat sessions still listed and openable.
- [ ] `~/.config/biorouter/workflows/` and `skills/` intact.
- [ ] Knowledge bases under `~/.config/biorouter/knowledge/` intact, including the active-KB pointer `.active-kb`.
- [ ] Window state, theme, menu-bar/dock icon prefs preserved.
- [ ] Llama Server downloaded models (in the data dir) are not re-downloaded.

> **Why.** The update replaces only the **app bundle**; all user state
> lives under `~/.config/biorouter` (and the OS credential store), which the
> updater never touches. Confirm regardless.

### Section G — fallback and failure modes

- [ ] **Release without the manifest** (e.g. an older release, or pre-this-change): `electron-updater` 404s on `latest-mac.yml`; the app falls back to `githubUpdater` which auto-downloads the `.dmg` to `~/Downloads` and shows the guided "Open Folder & Quit" dialog. The modal still appears and never crashes. (`UpdaterState.usingFallback === true`.)
- [ ] **Offline at launch**: no modal, no error spam; app behaves normally. Reconnect → next check surfaces the update.
- [ ] **Download interrupted** (kill network mid-download): the download resumes from where it stopped once the network is back (the log shows `[update] Download interrupted at … resuming`). Left down for over a minute, the modal shows an error with **Try again**, which resumes it. A background error **after** a completed download does **not** hide the ready-to-install state, but an **install** failure does, with its reason.
- [ ] **Corrupt/altered zip** (tamper with one byte so SHA-512 mismatches the manifest): `electron-updater` rejects the update (signature/checksum failure) and does **not** install — surfaced as an error, app stays on `X-1`.
- [ ] **Same version** (`latest-mac.yml` version == installed): no update offered; Settings shows "up to date".
- [ ] **Downgrade** (publish an older version as latest): not offered (`allowDowngrade` is off by default).
- [ ] Modal can be opened/closed repeatedly without duplicate event listeners (the `onUpdaterEvent` disposer is called on unmount).

### Section H — regression on unchanged surfaces

- [ ] Dependency checker still runs ~4s after window ready and prompts for missing deps exactly as before.
- [ ] No change to onboarding, provider configuration, or any non-update UI.
- [ ] First-run (fresh install, no prior version) shows no spurious update modal.

### Sign-off

Tick these once per release run, after working through Sections B–H above.

- [ ] Section A (automated) fully green, pre-existing failure count unchanged.
- [ ] Sections C–F verified on **macOS arm64**.
- [ ] Sections C–F verified on **macOS Intel**.
- [ ] Section E CLI propagation verified.
- [ ] Section G fallback/failure modes verified.
- [ ] Windows/Linux assisted-download path spot-checked (see the platform support matrix).

## Related documentation

- [BioRouter v1.86.0 release notes](notes/v1.86.0.md) — the release that shipped the one-click auto-update flow this checklist verifies.
- [Cross-compiling locally with `cross`](local-cross-compilation.md) — sibling release-process guide for building binaries for other architectures.
- [Installation and setup](../getting-started/installation.md) — the per-platform install paths an update has to replace cleanly.
- [Environment variables](../configuration/environment-variables.md) — reference for `BIOROUTER_UPDATE_FEED_URL` and the other updater knobs used above.
- [Dashboard mode removal record](../history/dashboard-mode/README.md) — explains why the `DashboardProvider` and `App.test.tsx` names in Section A's baseline no longer exist.
