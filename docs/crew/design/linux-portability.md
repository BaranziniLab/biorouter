# Crew Linux artifact portability

> **What this is.** How Crew's Linux broker is built for the release target, which glibc floor and checks qualify it, how every Biorouter build carries it for the setup service to install, how to install it in an ordinary account without administrator access, and the bounded rootless smoke of 2026-09-22.
> **Status:** Current. The build, staging and qualification steps describe the scripts as they are on 2026-10-16; the smoke section is a dated record of one run.
> **Audience:** Maintainers who build or package the Crew broker, and hosts who install it by hand.

Crew uses the repository's existing Linux release target: **x86_64-unknown-linux-gnu**, built through `scripts/cross-env.sh` with the pinned `rust:1.92-bullseye` image and a **glibc 2.31** floor. A binary built in `rust:latest` is not a supported portable release artifact: its newer libc imports can prevent startup on Debian 11 even when the Rust source is unchanged. No Linux ARM release artifact is qualified by this integration.

The Linux release backend, nightly cross-build, and package cross-build now explicitly select `biorouter-crew`. The Linux backend CI artifact archive includes it alongside the CLI and daemon. Both glibc-symbol and dynamic-library dependency checks include it. CLI `.deb` and `.rpm` packages install it at `/usr/bin/biorouter-crew`; the GUI needs no local broker because its connection manager invokes the broker on the SSH host.

From the repository root, the existing full package-backend command is:

```bash
bash scripts/build-computer-use-package-cross.sh x86_64-unknown-linux-gnu
```

For a broker-only build using the same pinned toolchain and a separate target directory:

```bash
bash <<'BASH'
set -euo pipefail
source scripts/cross-env.sh
cross_linux 'cargo build --locked --release -p biorouter-crew -j 2' /usr/src/myapp/target/crew-portable
BASH
```

The result is `target/crew-portable/x86_64-unknown-linux-gnu/release/biorouter-crew`. Qualify that exact artifact with `readelf --version-info` (all imported `GLIBC_*` versions must be at most 2.31), inspect `readelf -d` for runtime dependencies, and exercise startup on Debian 11 before calling it portable. The full package-backend command runs the shared artifact checks automatically; it expects all three binaries. These commands are documented here, not evidence that the new artifact has passed validation.

## The bundled broker

Since the [automatic setup](open-workspaces-and-automatic-setup.md) change (OW-9), every desktop build and both CLI packages carry the exact Linux broker their setup service installs on a server, so a setup needs no network to find it:

| Where | Path |
|---|---|
| Desktop app (every platform) | `crew-broker/linux-x86_64/{biorouter-crew,manifest.json}` beside `biorouterd`, staged at `ui/desktop/src/bin/crew-broker/linux-x86_64/` |
| CLI `.deb` and `.rpm` | `/usr/libexec/biorouter/crew-broker/linux-x86_64/` |
| Development | Any folder named by `BIOROUTER_CREW_BROKER_DIR`, holding `biorouter-crew` and usually its `manifest.json` |

`scripts/stage-crew-broker.sh <release-dir-or-binary> <dest-dir>` stages one, and writes `manifest.json` from the staged bytes, never by hand: `version`, `target`, `glibc_max`, `sha256`, `size`, `protocol`, `capabilities`, `source_sha` and `source_dirty`. It refuses, before anything lands at the destination, a file that is not an x86-64 ELF executable, a broker that needs a glibc newer than the floor (`scripts/check-glibc-floor.sh`), one built without join-by-name (`scripts/check-crew-broker-join.sh`), one whose `--version` is not this tree's Cargo version, and a destination that holds anything an earlier staging did not write. It swaps the staging in with two renames. On a host that is not Linux x86_64 it runs the broker inside the glibc 2.31 baseline image, so it needs Docker.

`scripts/build-crew-broker-linux.sh <out-dir>` cross-builds the broker alone through `cross_linux` in `scripts/cross-env.sh`, the one cross-build definition, on the pinned bullseye image, then stages it. Its cargo target directory persists on a Docker volume (`CREW_BROKER_TARGET_VOLUME`, default `biorouter-crew-broker-target`, or `CROSS_TARGET_MOUNT`), so a rebuild takes about a minute; that cache is safe only because staging reads the glibc floor off the bytes it stages. `just stage-crew-broker` runs it into the desktop's `src/bin`:

```bash
scripts/build-crew-broker-linux.sh ui/desktop/src/bin/crew-broker/linux-x86_64
```

A release stages the broker that `scripts/release.sh linux-backend` built, and refuses a bundle whose staged manifest names another SHA-256 than the one recorded for that release; `BIOROUTER_REQUIRE_CREW_BROKER=1` makes `prepare-platform-binaries.js` and forge refuse a bundle without one. A development package without a broker builds with a warning, and its setup service answers `crew_install_artifact_unavailable`. macOS signing skips the ELF (`/crew-broker/` in `forge.config.ts`), and the app's sealed resources cover it instead. Rust builds are not reproducible, so the broker in the mac bundles and the one in the Linux and Windows bundles, built separately from one commit, have different digests; each manifest describes its own copy.

The setup service checks the file against its manifest and the daemon's own version before sending it, sends it over the SSH session's standard input, and checks size, SHA-256 and `--version` again on the server before an atomic rename. See [How Biorouter installs Crew](../administration.md#how-biorouter-installs-crew).

## Install by hand

For an ordinary-user SSH installation, copy the qualified executable to the target account and install it with `install -m 0755 biorouter-crew "$HOME/.local/bin/biorouter-crew"` after creating `$HOME/.local/bin`. The desktop uses that explicit location. A system CLI-package installation can be copied from `/usr/bin/biorouter-crew` into that location; alternatively extract the `.deb` without installing it, using `dpkg-deb --extract package.deb staging`, and copy `staging/usr/bin/biorouter-crew`. Neither approach requires administrator access. Preserve executable permissions when extracting a CI artifact archive.

## What the glibc floor qualifies

The libc floor qualifies binary loading, not every operation on every host. Crew's broker/bridge require Linux Unix-socket peer credentials. Folder-mode remote execution additionally requires fully enforced Landlock ABI v3 and seccomp; unsupported kernels or runtime policy refuse execution up front, with a sentence that names full access. Full access (OW-6) runs commands as the member's own account through their own bridge, unconfined, so it needs neither: on a kernel older than 6.2, such as Ubuntu 22.04's 5.15, it is the only level that runs commands. File operations, filesystem ownership, advisory locking, durable rename/fsync, Unix sockets, and NFS/shared-home behavior still need qualification on the actual host. This change does not claim universal kernel or NFS support.

## Bounded rootless smoke evidence (2026-09-22)

The pinned artifact
`target/crew-portable/x86_64-unknown-linux-gnu/release/biorouter-crew`
(SHA-256
`2a72d02df3d57b9785305d3bdf978d24f4e2cb248652ba8a459983ca8335594d`) was
mounted read-only into a disposable `rust:1.92-bullseye` container with
`--platform linux/amd64` and run as ordinary UID `65532`. A synthetic
root-owned `/etc/machine-id`, private HOME, private state parent, and a
synthetic 32-byte bootstrap public key were used. `start` returned a PID;
`status` verified the matching workspace, UID, socket, and process metadata.
The bridge then performed a real Unix-socket `hello`, returning the private
mode, workspace identity, host UID 65532, and Crew capabilities.

The lifecycle `stop` path was attempted with the exact status PID in both the
default container and with Docker seccomp unconfined. Both emulated x86_64
runs returned `unsupported: safe stop requires Linux pidfd_open`; this is a
limitation of the macOS x86_64 emulation/kernel path, so exact-pid stop
qualification remains pending a native Linux kernel. No remote execution or
network qualification is claimed by this smoke.

## Related documentation

- [Crew design](README.md): the other Crew design and reference documents
- [Administration](../administration.md): installing `biorouter-crew` on a server, for hosts and IT staff
- [Crew build campaign](../../history/biorouter-crew/README.md): the evidence behind these steps
