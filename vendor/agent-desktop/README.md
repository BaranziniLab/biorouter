# Vendored agent-desktop

Biorouter Copilot's engine. `source/` holds
[agent-desktop](https://github.com/vercel-labs/agent-desktop) (Vercel Labs, Apache
License 2.0) at the tag and commit `pin.json` names, unmodified, limited to the files
a build reads. A build needs no copy of the upstream repository, so a repository that
is deleted, force pushed or changed cannot affect Biorouter, and the exact source
compiled into a shipped helper is reviewable in this repository's history. The design
is [Biorouter Copilot on agent-desktop](../../docs/design/copilot-agent-desktop.md)
(AD-01 to AD-03).

| Path | What it is |
|---|---|
| `source/` | upstream's files at the pin, never edited by hand |
| `pin.json` | repository, tag, commit, version, JSON contract version, the five build targets and their Rust triples, the macOS bundle name and identifier, minimum macOS, glibc floor, Linux package dependencies |
| `source-manifest.json` | the SHA-256 of every file under `source/`, one digest over the whole tree, and the subset that is vendored with the reason for each part |
| `LICENSE` | upstream's Apache License 2.0 text, byte identical to `source/LICENSE` |
| `DEPENDENCIES.txt` | every crates.io crate `Cargo.lock` locks, with its version, licence and the targets that compile it; shipped in each payload as `DEPENDENCIES.agent-desktop.txt` |

There are **no Biorouter patches**. Every Biorouter rule (consent, refusals, argument
checks, output framing) lives in the wrapper in `crates/biorouter-mcp/src/computer_use/`.
A `patches/` folder is not created until a patch is needed.

## What is in `source/`

`Cargo.toml`, `Cargo.lock`, `LICENSE`, `clippy.toml`, `deny.toml`, `crates/` (the core
crate, the macOS, Windows and Linux adapters, and `crates/ffi`, which the workspace
lists as a member, so cargo cannot load the workspace without it), `src/` (the
binary), `skills/` and `providers/macos/` (both embedded with `include_str!` by the
core crate, so it does not compile without them), and three test fixtures the core
crate's unit tests read. Left out: `apps/`, `assets/`, `docs/`, `npm/`, `probes/`,
`benchmarks/`, the rest of `tests/`, and upstream's `rust-toolchain.toml`, which would
make rustup install 1.89.0 instead of building with Biorouter's toolchain. The
vendoring script's `INCLUDED` list is the authority, and the manifest records it.

## Checks

```sh
./scripts/check-vendored-computer-use.sh
```

It verifies every file against the manifest in both directions (a missing, changed or
unrecorded file fails), the tree digest, that `pin.json` and the manifest name the same
release, that no file lies outside the vendored subset, that `DEPENDENCIES.txt` lists
exactly the crates `Cargo.lock` locks, that `Cargo.lock` locks no network client
(AD-20), that every vendored file is committable, and that none is subject to
end-of-line conversion. The Justfile, `rust.yml`'s guards job and AGENTS.md run it.

## Building

```sh
python3 scripts/computer-use-runtime.py build darwin-arm64     # on macOS
python3 scripts/computer-use-runtime.py build darwin-x64       # on macOS
bash scripts/build-computer-use-helper-cross.sh linux-x64      # Docker, rust:1.92-bullseye
bash scripts/build-computer-use-helper-cross.sh win32-x64      # Docker, the same image with mingw
python3 scripts/computer-use-runtime.py verify darwin-arm64
python3 scripts/computer-use-runtime.py contract darwin-arm64  # runs the binary, no desktop access
```

The build verifies the vendored tree before and after cargo runs, builds with the
repository's own Rust toolchain (`RUSTUP_TOOLCHAIN` from `rust-toolchain.toml`),
`--locked`, with `--remap-path-prefix` so no local path is embedded, and
`MACOSX_DEPLOYMENT_TARGET=13.0` on macOS. Dependencies come from crates.io, checked
against `Cargo.lock`'s checksums. For a build with no network:

```sh
python3 scripts/computer-use-runtime.py vendor-deps            # once, with network
python3 scripts/computer-use-runtime.py build linux-x64 --offline-deps target/agent-desktop-vendor
```

Release builds sign the macOS bundle with `--signing-identity` (Developer ID, hardened
runtime, timestamp) before the payload manifest is written; development builds are
signed ad hoc, so macOS permission grants follow the code hash and must be given again
after a rebuild.

## Moving the pin

```sh
git clone https://github.com/vercel-labs/agent-desktop.git /path/to/agent-desktop
git -C /path/to/agent-desktop checkout <new tag>
$EDITOR vendor/agent-desktop/pin.json           # upstream_tag, upstream_commit, upstream_version, contract_version
python3 scripts/vendor-computer-use-source.py --from /path/to/agent-desktop
python3 scripts/vendor-computer-use-source.py --dependencies   # needs the crates or the network
python3 scripts/check-computer-use-upstream.py  # what changed between the two tags
```

Then rebuild every target and run the wrapper's contract tests, which fail when a
command or flag the wrapper uses has changed. Read upstream's `CHANGELOG.md` between
the two tags first: a JSON contract version change is a wrapper change, not a pin
change.
