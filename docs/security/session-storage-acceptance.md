# Rebuilt Linux session-storage acceptance

The `Rebuilt session storage acceptance` workflow builds the actual `biorouterd` binary from the checked-out source with the repository's normal pinned Rust 1.92 toolchain. It tests the real daemon's SQLite history and restart behavior, rather than compiling an extracted permission helper. The build profile is `dev` with debug symbols disabled; it is not a newly shipped release artifact. Source preparation alone is not a passing runtime result.

The build runner and new runtime image both use Ubuntu 24.04. This avoids loading a host-built glibc 2.39 binary in an older Ubuntu 22.04 image. It does not change or verify the shipped package's separate glibc floor. The runtime includes Python and the daemon's expected shared libraries; each repetition records `ldd`, glibc and daemon-version output and fails on a loader error. Image ID, architecture, source SHA, daemon SHA-256 and compiler version are retained. The image's package installation needs setup connectivity; fixture containers run with networking disabled.

Each of three repetitions creates a fresh synthetic store in a fresh container. The owner is EUID 10001 and the observer is EUID 10002. The daemon runs as 10001. The container has a read-only root, all capabilities dropped, no-new-privileges, default seccomp, no ports, no default route, and bounded CPU, memory, PIDs and lifetime. Its only host bind is the read-only rebuilt daemon binary. Writable home and temporary storage are bounded tmpfs mounts. The home tmpfs is owned by 10001, so fixture creation and permission changes need no in-container root, chown capability or privileged setup.

The recorded cases check container isolation, a public synthetic control readable by both accounts, runtime compatibility, fresh owner API/SQLite access, fresh observer raw-file/SQLite denial, an intentionally permissive legacy-store positive control, preserved owner API/SQLite history after a daemon restart, and restored observer denial. For the legacy control, the stopped store is checkpointed by the owner and its synthetic database/sidecar modes are deliberately made readable. The observer must then recover the synthetic marker before repair; otherwise the fixture is inconclusive and fails. Restart must protect the same store while preserving the owner's title. This is a deliberately permissive synthetic control, not a new old-binary baseline run. It does not query a production database or use real records.

The ephemeral daemon secret is forwarded through subprocess environment variables and Docker's name-only `--env` forwarding. It is never a command-line value or a fixture/control file. The observer receives no daemon secret. Artifact logs redact the synthetic secret if it occurs in daemon output. No host home, source tree, credential store or Docker socket is mounted inside a fixture.

Every recorded case contains its observations, individual assertions and an explicit pass/fail outcome. A failed repetition preserves its phase, sanitized error, case records and available daemon logs; it is not dropped from the report. Cleanup is restricted to the generated container name with the expected assessment label. The workflow uploads the source/build/image/acceptance evidence even after failure. Preserve the initial failing run if a later source or harness correction passes.

To run this acceptance check after explicitly allocating local build/container resources:

```sh
cargo build --locked -p biorouter-server --bin biorouterd
docker build --pull -t biorouter-security-storage-runtime:local \
  -f scripts/security-assessment/session-storage-runtime.Dockerfile scripts/security-assessment
python3 scripts/security-assessment/session_storage_rebuilt.py \
  --daemon target/debug/biorouterd --image biorouter-security-storage-runtime:local \
  --source-commit "$(git rev-parse HEAD)" --repetitions 3 --output /tmp/new-session-storage-evidence
```

The output directory must be new. The helper's source SHA is operator-supplied; CI independently checks `git rev-parse HEAD` against the workflow SHA before building. On pull requests, it records both the tested merge commit and contributor head. Do not relabel this deterministic acceptance suite as independent attack trials or add its assertion count to the original audit denominator.

This lane does not validate Windows ACLs, macOS ACLs, unrestricted shell containment, same-UID hostile processes, administrators/root, descriptors opened before privacy repair, every filesystem, or a released application build. Existing Windows unit CI exercises its platform-specific storage tests without a redundant Windows daemon build in this workflow. Broader application and platform acceptance remains separate. The original `assessment-2026-10` logs and Dockerfile remain unchanged.
