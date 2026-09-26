# Crew Linux artifact portability

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

For an ordinary-user SSH installation, copy the qualified executable to the target account and install it with `install -m 0755 biorouter-crew "$HOME/.local/bin/biorouter-crew"` after creating `$HOME/.local/bin`. The desktop uses that explicit location. A system CLI-package installation can be copied from `/usr/bin/biorouter-crew` into that location; alternatively extract the `.deb` without installing it, using `dpkg-deb --extract package.deb staging`, and copy `staging/usr/bin/biorouter-crew`. Neither approach requires administrator access. Preserve executable permissions when extracting a CI artifact archive.

The libc floor qualifies binary loading, not every operation on every host. Crew's broker/bridge require Linux Unix-socket peer credentials. Remote execution additionally requires fully enforced Landlock ABI v3 and seccomp; unsupported kernels or runtime policy must refuse execution. File operations, filesystem ownership, advisory locking, durable rename/fsync, Unix sockets, and NFS/shared-home behavior still need qualification on the actual host. This change does not claim universal kernel or NFS support.

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
