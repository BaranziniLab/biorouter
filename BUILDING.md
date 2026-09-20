# BioRouter Build Guide

This guide covers how to build BioRouter for all supported platforms from a macOS Apple Silicon development machine. Follow the steps in order — some steps depend on previous ones.

> **The supported path is [`scripts/release.sh`](scripts/release.sh)**, documented in [RELEASE.md](RELEASE.md). It encodes every step below as a resumable phase, including the invariants that are easy to get wrong by hand (the pinned Linux cross-compile image, the packaged artifact names, the exact asset list). Use this document as the manual/debugging reference for when a phase fails or you need to understand what one does.
>
> **Version placeholder:** File names below use `<version>` for the workspace version. The single source of truth is `[workspace.package].version` in `Cargo.toml`; `scripts/check-version-consistency.sh` fails CI if any of the other five copies drift from it. Substitute the real version when you build — never hardcode one.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Step 1 — Build the Rust Binary (macOS ARM64)](#step-1--build-the-rust-binary-macos-arm64)
3. [Step 2 — Build macOS Apple Silicon App (Signed + Notarized)](#step-2--build-macos-apple-silicon-app-signed--notarized)
4. [Step 3 — Build macOS Intel App (Signed + Notarized)](#step-3--build-macos-intel-app-signed--notarized)
5. [Step 4 — Build Linux App via Docker (.deb + .rpm)](#step-4--build-linux-app-via-docker-deb--rpm)
6. [Step 5 — Restore macOS ARM Binary After Linux Build](#step-5--restore-macos-arm-binary-after-linux-build)
7. [Step 6 — Build Windows App](#step-6--build-windows-app)
8. [Step 7 — Build macOS DMG Installers](#step-7--build-macos-dmg-installers)
9. [Output Artifacts](#output-artifacts)
10. [One-Time Setup](#one-time-setup)
11. [Troubleshooting](#troubleshooting)

---

## Prerequisites

Before building, ensure the following are installed and configured:

| Tool | Install |
|------|---------|
| **Rust 1.92** (with `cargo`) | https://rustup.rs — the channel is pinned in `rust-toolchain.toml`, so rustup selects it automatically |
| **Node.js 24.x** (not newer) | https://nodejs.org — `ui/desktop/package.json` declares `engines: { "node": "^24.0.0" }`, and hermit pins 24.10.0 |
| **npm** | bundled with Node.js |
| **Docker Desktop** | https://www.docker.com/products/docker-desktop (required for Linux build) |
| **Xcode Command Line Tools** | `xcode-select --install` |
| **Go 1.26.8** | https://go.dev/dl. Builds the Biorouter Copilot helper for the Windows and Linux targets; this is the version `.github/workflows/computer-use-native.yml` pins |
| **Swift 6.2+ / Xcode** | Required for the two macOS Biorouter Copilot helper targets; `computer-use-runtime.py` refuses to build them anywhere but macOS |
| **Python 3** | Runs `scripts/computer-use-runtime.py`, the helper build driver |
| **git** | The helper build stages the vendored source and applies its patches with `git apply` |

> **Node 24.x means 24.x, not "24 or newer."** Under Node 26 `electron-forge package` exits 0 having produced no `.app` — a silent no-op that looks like a build succeeding — and the `appdmg` / `macos-alias` native modules the DMG maker needs do not build at all.

Also required for macOS signed builds — see [One-Time Setup](#one-time-setup):
- Apple Developer certificate imported into your keychain
- Apple app-specific password (from appleid.apple.com)

All commands below assume you start from the repo root **with the hermit toolchain activated** — it is what pins Node 24, and every packaging step below depends on it:
```bash
cd /path/to/biorouter
source bin/activate-hermit
```

---

## Step 1 — Build the Rust Binary (macOS ARM64)

This compiles the native macOS ARM64 backend binary. It must be done before any Electron build.

```bash
cargo build --release
```

Output: `target/release/biorouter` (CLI) and `target/release/biorouterd` (daemon) — the desktop bundle requires **both**.

Then copy **both** binaries into the Electron app's bin directory. `just copy-binary` copies `biorouter` and `biorouterd` and re-signs them with the Developer ID (so Keychain grants survive rebuilds):

```bash
just copy-binary
```

Verify the CLI is the right architecture:
```bash
file ui/desktop/src/bin/biorouter
# Expected: Mach-O 64-bit executable arm64
```

> **Note:** Packaging requires **both** `biorouter` and `biorouterd` in `src/bin/` — `prepare-platform-binaries.js` aborts the build if either is missing (it also fetches the `llamacpp/llama-server` sidecar automatically). The ARM64 binaries are gitignored (they exceed GitHub's 100MB limit) and must be rebuilt on each machine.

### Build the Biorouter Copilot helper for the target you are packaging

The helper's build paths, scripts and staged directories keep the `computer-use` spelling
(`vendor/computer-use/`, `target/computer-use/`, `scripts/computer-use-runtime.py`,
`stageComputerUse()`). Those are on-disk and provenance identifiers, not display names, so the
rename to Biorouter Copilot left them alone deliberately.

`stageComputerUse()` is the **first** statement of `prepare-platform-binaries.js`, so every packaging
command below (Steps 2, 3, 4, 6 and 7, and any bare `npm run bundle:*`) dies before it copies a
single binary unless `target/computer-use/<target>/manifest.json` already exists. Nothing in the
`Justfile` produces it, so a local packaging run needs the command below run by hand. The automated
paths that build it for you are `scripts/release.sh`, the
`.github/workflows/computer-use-native.yml` workflow, the root `Dockerfile` and
`scripts/computer-use-package-acceptance.py`. Packaging bundles the helper, so there is no way to
skip it.

```bash
# Run from the repo root, once per target you intend to package.
python3 scripts/computer-use-runtime.py build darwin-arm64   # Step 2 and the arm64 DMG
python3 scripts/computer-use-runtime.py build darwin-x64     # Step 3 and the Intel DMG
python3 scripts/computer-use-runtime.py build linux-x64      # Step 4
python3 scripts/computer-use-runtime.py build win32-x64      # Step 6
```

The upstream source is vendored in full at [`vendor/computer-use/source/`](vendor/computer-use), at the commit pinned in [`pin.json`](vendor/computer-use/pin.json), so this step needs no network and no upstream change can affect it. The vendored tree stays pristine: the reviewed patches in `vendor/computer-use/patches/` are applied to a throwaway copy at build time, never to the tree itself. Pass `--source <clone>` to build against an external checkout instead, which is how you try a candidate upstream before vendoring it.

---

## Step 2 — Build macOS Apple Silicon App (Signed + Notarized)

Requires the ARM64 binary from Step 1 to be in `ui/desktop/src/bin/biorouter`.

```bash
cd ui/desktop

APPLE_ID=<apple-id> \
APPLE_APP_SPECIFIC_PASSWORD=<app-specific-password> \
npm run bundle:default
```

What this does:
1. Strips Windows binaries from `src/bin/` via `prepare-platform-binaries.js`
2. Builds and packages the Electron app for `darwin/arm64`
3. Signs all binaries with the Developer ID certificate
4. Submits to Apple's notarization service and staples the ticket
5. Creates the final distributable zip with `ditto`

**Verify notarization:**
```bash
spctl --assess --verbose out/Biorouter-darwin-arm64/Biorouter.app
# Expected: accepted  source=Notarized Developer ID
```

Output: `out/Biorouter-darwin-arm64/Biorouter.zip` — a build **intermediate**, not a release asset.

> **The packaged product is named `Biorouter`, not `BioRouter`.** `productName` in `ui/desktop/package.json` is `Biorouter`, and every packaged path, `.app`, dmg, deb, rpm and zip follows it (`scripts/check-brand-consistency.sh` enforces this). The `just` recipes and the Rust binaries are unaffected — only the packaged names use this spelling.

---

## Step 3 — Build macOS Intel App (Signed + Notarized)

Intel Mac cross-compilation requires the `x86_64-apple-darwin` Rust target. Install it once:

```bash
rustup target add x86_64-apple-darwin
```

Then cross-compile the Rust binary for Intel:

```bash
cargo build --release --target x86_64-apple-darwin
```

Output: `target/x86_64-apple-darwin/release/biorouter` and `.../biorouterd`.

Copy **both** Intel binaries into the bin directory (replacing ARM). `just copy-binary-intel` copies `biorouter` and `biorouterd` from the Intel target:

```bash
just copy-binary-intel
```

Verify:
```bash
file ui/desktop/src/bin/biorouter
# Expected: Mach-O 64-bit executable x86_64
```

Now build the Intel Electron app:

```bash
cd ui/desktop

APPLE_ID=<apple-id> \
APPLE_APP_SPECIFIC_PASSWORD=<app-specific-password> \
npm run bundle:intel
```

**Verify notarization:**
```bash
spctl --assess --verbose out/Biorouter-darwin-x64/Biorouter.app
# Expected: accepted  source=Notarized Developer ID
```

Output: `out/Biorouter-darwin-x64/Biorouter_intel_mac.zip` — again a build intermediate, not a release asset.

After this step, restore the ARM binary so subsequent builds aren't broken:

```bash
just copy-binary
```

---

## Step 4 — Build Linux App via Docker (.deb + .rpm)

The Linux build runs entirely inside Docker — no Linux machine needed. Docker Desktop must be running.

Run it through the recipe, not by hand:

```bash
just make-ui-linux
```

That does both stages: it sources `scripts/cross-env.sh` and cross-compiles the Rust backend for `x86_64-unknown-linux-gnu`, then runs `ui/desktop/scripts/build-linux-deb.sh` in the container image pinned by digest in `ui/desktop/scripts/linux-native-baseline.json` (`node:24-bullseye`, glibc 2.31) to produce the `.deb` and `.rpm`. `scripts/release.sh linux-backend <version>` is the stage-A-only equivalent, and it wipes the target dir first so nothing stale survives.

> **Never inline the cross-compile image into a command.** The glibc floor lives in exactly one place — `LINUX_RUST_IMG` in [`scripts/cross-env.sh`](scripts/cross-env.sh), pinned to `rust:1.92-bullseye` (glibc 2.31). The rolling `rust:latest` is now trixie (glibc 2.39) and produces a Linux backend that will not start on Debian 12, Ubuntu 22.04, or RHEL/Rocky 9. This recipe used to pin `rust:latest` and silently raised the floor; `scripts/check-no-cross-drift.sh` (part of `just check-everything`) and `scripts/check-glibc-floor.sh` now exist to stop it drifting back. A hand-rolled `docker run` bypasses both gates.

The build caches the Cargo registry in a Docker volume between runs. On the first run this takes ~5-10 minutes; subsequent runs are much faster.

Verify the Linux binary was produced:
```bash
ls -lh target/x86_64-unknown-linux-gnu/release/biorouter
# Expected: ~100-115MB ELF 64-bit binary
```

The `build-linux-deb.sh` stage:
1. Installs `fakeroot`, `dpkg`, and `rpm` inside the container
2. Runs `npm ci` and swaps the macOS ARM binary in `src/bin/` for the Linux x64 binary
3. Runs `electron-forge make` for `linux/x64` with the `maker-deb` + `maker-rpm` targets

Outputs:
- `ui/desktop/out/make/deb/x64/biorouter_<version>_amd64.deb`
- `ui/desktop/out/make/rpm/x64/Biorouter-<version>-1.x86_64.rpm`

---

## Step 5 — Restore macOS ARM Binary After Linux Build

The Linux Docker build replaces `src/bin/biorouter` with the Linux x64 binary. **Always restore the ARM binary before the Windows build or any future macOS work:**

```bash
just copy-binary
```

Also, the Docker `npm ci` inside the container corrupts the local `node_modules` (missing macOS ARM native modules). Fix this every time after a Linux Docker build:

```bash
cd ui/desktop
rm -rf node_modules
npm ci
cd ../..
```

> **`npm ci`, never `npm install`.** `package-lock.json` is a tracked file, and `install` rewrites it. The next Linux or Windows Docker build runs `npm ci` inside the container, which refuses a lockfile that disagrees with `package.json`, so deleting the lockfile here leaves the tree dirty and breaks the next cross build.

---

## Step 6 — Build Windows App

The Windows Rust binaries (`biorouter.exe` and `biorouterd.exe`) are **not** checked into the repo — they must be cross-compiled from macOS/Linux inside Docker before packaging. `just make-ui-windows` runs the whole flow: it calls `just release-windows` (a Docker `rust:latest` cross-compile to `target/x86_64-pc-windows-gnu/release/`, producing both `.exe`s plus the required mingw runtime DLLs), copies them into `src/bin/`, then runs the Electron bundle.

```bash
just make-ui-windows
```

Under the hood `npm run bundle:windows` (via `prepare-platform-binaries.js`):
1. Builds the Vite main process bundle
2. Copies the **supporting** Windows runtime files (`uv.exe`/`uvx.exe`, `git/`, `.cmd` shims, and the mingw `.dll`s) from `src/platform/windows/bin/` into `src/bin/` — that directory holds only these support files, **not** the Rust `.exe`s
3. Removes the macOS binaries from `src/bin/`
4. Fetches the `llamacpp/llama-server.exe` sidecar and verifies `biorouter.exe` + `biorouterd.exe` are present (packaging aborts if either is missing)
5. Runs `electron-forge make` for `win32/x64`

Outputs (every win32 maker runs, so both are produced):
- `out/make/zip/win32/x64/Biorouter-win32-x64-<version>.zip`
- `out/make/squirrel.windows/x64/Biorouter-Setup-<version>.exe`, the installer that updates an existing install in place. The updater matches that exact filename, so a missing or misnamed one sends Windows back to the assisted download.

After this, restore the ARM binary again:
```bash
just copy-binary
```

---

## Step 7 — Build macOS DMG Installers

DMG files provide a standard macOS drag-to-install experience (open DMG → drag app to Applications). Both builds are signed and notarized.

### Apple Silicon DMG

Ensure the ARM64 binary is in `src/bin/` (it should be after Step 5/6):

```bash
file ui/desktop/src/bin/biorouter
# Must show: Mach-O 64-bit executable arm64
```

```bash
cd ui/desktop

APPLE_ID=<apple-id> \
APPLE_APP_SPECIFIC_PASSWORD=<app-specific-password> \
npm run bundle:dmg
```

Output: `out/make/Biorouter-<version>-arm64.dmg`

### Intel DMG

Swap in the Intel binaries first:

```bash
just copy-binary-intel
```

```bash
cd ui/desktop

APPLE_ID=<apple-id> \
APPLE_APP_SPECIFIC_PASSWORD=<app-specific-password> \
npm run bundle:intel-dmg
```

Output: `out/make/Biorouter-<version>-x64.dmg`

Restore the ARM binary after:

```bash
just copy-binary
```

---

## Output Artifacts

A release carries **exactly 11 assets** — the list `release_assets()` in `scripts/release.sh` prints, and which `draft` refuses to proceed without:

| Platform | File | Location |
|----------|------|----------|
| macOS Apple Silicon (DMG) | `Biorouter-<version>-arm64.dmg` | `ui/desktop/out/make/` |
| macOS Intel (DMG) | `Biorouter-<version>-x64.dmg` | `ui/desktop/out/make/` |
| macOS Apple Silicon (auto-update zip) | `Biorouter-darwin-arm64-<version>.zip` | `ui/desktop/out/make/zip/darwin/arm64/` |
| macOS Intel (auto-update zip) | `Biorouter-darwin-x64-<version>.zip` | `ui/desktop/out/make/zip/darwin/x64/` |
| macOS (auto-update manifest) | `latest-mac.yml` | `ui/desktop/out/make/` |
| Windows x64 | `Biorouter-win32-x64-<version>.zip` | `ui/desktop/out/make/zip/win32/x64/` |
| Linux Ubuntu / Pop!_OS (GUI) | `biorouter_<version>_amd64.deb` | `ui/desktop/out/make/deb/x64/` |
| Linux Fedora / RHEL (GUI) | `Biorouter-<version>-1.x86_64.rpm` | `ui/desktop/out/make/rpm/x64/` |
| Linux headless CLI (deb) | `biorouter-cli_<version>_amd64.deb` | `dist/cli/` |
| Linux headless CLI (rpm) | `biorouter-cli-<version>-1.x86_64.rpm` | `dist/cli/` |
| Windows x64 (installer) | `Biorouter-Setup-<version>.exe` | `ui/desktop/out/make/squirrel.windows/x64/` |

**Do not upload** `out/Biorouter-darwin-arm64/Biorouter.zip` or `out/Biorouter-darwin-x64/Biorouter_intel_mac.zip`. Those unversioned `ditto` archives are build intermediates.

### The macOS auto-update manifest

`latest-mac.yml` is generated by `scripts/release.sh mac-manifest <version>` (also run automatically by `draft`) from the two versioned darwin zips. It is load-bearing: without it, electron-updater 404s and the in-app one-click "Restart & Update" silently degrades to the assisted GitHub-download fallback. electron-updater picks the architecture from the `arm64`/`x64` token in the zip filename, so both clients share one manifest.

---

## One-Time Setup

These steps only need to be done once per development machine.

### Import the Apple Developer Certificate

The `.p12` file and full instructions live in `notarization/` (the whole directory is gitignored, keep it local): `notarization/UCSF-AppleDeveloper-Main_Application.p12` and `notarization/APPLE_DEVELOPER_NOTES.md`.

```bash
security import notarization/UCSF-AppleDeveloper-Main_Application.p12 \
  -k ~/Library/Keychains/login.keychain-db \
  -P "<p12-passphrase>" \
  -T /usr/bin/codesign \
  -T /usr/bin/productbuild
```

### Seed the Notarization Credentials into the Keychain

`scripts/release.sh` resolves the notarization credentials in this order: the `APPLE_ID` /
`APPLE_APP_SPECIFIC_PASSWORD` environment variables → the macOS Keychain →
`notarization/APPLE_DEVELOPER_NOTES.md`. The Keychain is the preferred store — encrypted at rest, no
plaintext on disk — and it is what lets an unattended `scripts/release.sh mac-arm64 <version>` run
with no environment variables set. Seed it once:

```bash
security add-generic-password -s biorouter-notarization -a APPLE_ID -w <apple-id> -A -U
security add-generic-password -s biorouter-notarization -a APPLE_APP_SPECIFIC_PASSWORD -w <password> -A -U
```

### Install the Apple Developer ID G2 Intermediate Certificate

Required for a complete trust chain:

```bash
curl -s -o /tmp/DeveloperIDG2CA.cer \
  "https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer"
security import /tmp/DeveloperIDG2CA.cer \
  -k ~/Library/Keychains/login.keychain-db
```

### Verify the Signing Identity

```bash
security find-identity -v -p codesigning
# Should show: "Developer ID Application: University of California at San Francisco (F3YYBXAFJ8)"
```

### Install Node Dependencies

```bash
cd ui/desktop
npm ci
```

### Install the Intel Rust Target (for Intel macOS builds)

```bash
rustup target add x86_64-apple-darwin
```

---

## Troubleshooting

### `@rollup/rollup-darwin-arm64` missing after Linux Docker build

The Docker `npm ci` overwrites local `node_modules` with Linux versions, breaking macOS builds.

**Fix:**
```bash
cd ui/desktop
rm -rf node_modules
npm ci
```

`npm ci`, never `npm install`: `install` rewrites the tracked `package-lock.json`, and the container's own `npm ci` then refuses it.

### `401 Unauthorized` during notarization

The app-specific password must be generated for the personal Apple ID used for notarization (not the UCSF email). See `notarization/APPLE_DEVELOPER_NOTES.md` (gitignored, kept local) for the account details and how to generate a replacement if needed.

### `cannot find -lxcb` or `cannot find -lbz2` in Linux Docker build

The cross-compilation environment needs AMD64 dev headers. `scripts/cross-env.sh` installs them (via `dpkg --add-architecture amd64`) as part of the pinned recipe — if you see these errors, you are almost certainly running a hand-rolled `docker run` instead of `just make-ui-linux` / `scripts/release.sh linux-backend`.

### Keychain access dialog during signing

Click **Always Allow** when macOS prompts for `codesign` keychain access. To suppress this prompt permanently after importing the certificate:

```bash
security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
  -k "<your-login-keychain-password>" \
  ~/Library/Keychains/login.keychain-db
```

### `biorouter` binary is the wrong architecture

If you accidentally run a macOS build after a Linux Docker build, the binary in `src/bin/` may be the Linux ELF binary. Always check:

```bash
file ui/desktop/src/bin/biorouter
```

If it shows `ELF 64-bit` instead of `Mach-O`, restore it:

```bash
just copy-binary
```
