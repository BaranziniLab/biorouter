# Making a Release

Biorouter releases are cut locally with [`scripts/release.sh`](scripts/release.sh), not by a
GitHub Action. The script encodes the whole pipeline — version bump → compile all four backends →
sign and notarize both macOS dmgs → package Windows and Linux → verify → draft the GitHub release —
so a human or an agent can reproduce a release exactly. Publication is deliberately left as a
separate, gated step.

Run everything under the hermit toolchain, which pins Node 24 (the macOS dmg maker's native
modules do not build on newer Node):

```bash
source bin/activate-hermit
```

## One-shot release

```bash
scripts/release.sh all patch          # next patch, computed from Cargo.toml's current version
scripts/release.sh all 1.88.7         # the same thing, spelled explicitly
```

`all` runs every build and verify phase and **stops at the draft release**. It does not publish:
publication is a separate, gated step (see `draft` and `publish` below). When `all` finishes it
prints the command to run next.

## Phase-by-phase (resumable)

Each phase is a separate subcommand, so a failed release can be resumed rather than restarted.
Resuming has preconditions. Every phase after `backends` refuses to run unless the tree is
completely clean (untracked files included), `Cargo.toml` is still at the release version, and
`HEAD` is exactly the commit `backends` built from, so a commit made mid-release forces a rebuild
from the final commit. `draft` and `publish` additionally require `HEAD` to equal `origin/main`,
with `origin/main` already at this version.

| Phase | What it does |
| ----- | ------------ |
| `bump <ver\|major\|minor\|patch>` | Bump the version in the 5 release files and refresh `Cargo.lock` |
| `backends <ver>` | Compile release backends for all 4 targets (mac arm64, mac x64, windows-gnu, linux-gnu) |
| `linux-backend <ver>` | Rebuild just the Linux backend from scratch (re-runnable) |
| `mac-arm64 <ver>` | Package, sign, and **notarize** the Apple Silicon `.dmg` |
| `mac-intel <ver>` | Package, sign, and **notarize** the Intel `.dmg` |
| `windows <ver>` | Package the Windows `.zip` **and** the Squirrel `Biorouter-Setup-<ver>.exe` installer (the installer is what makes a Windows update in-place; the updater matches that exact filename) |
| `linux <ver>` | Package the GUI `.deb` + `.rpm` — **run this last**, it leaves `node_modules` Linux-flavored |
| `cli-linux <ver>` | Build the headless CLI-only `.deb` + `.rpm` (`biorouter` + `biorouterd`) |
| `mac-manifest <ver>` | Generate `latest-mac.yml` for electron-updater (also run by `draft`) |
| `verify <ver>` | Assert all 10 artifacts are present, both macOS apps are stapled and Gatekeeper-accepted, the Intel bundle is really x86_64, and `latest-mac.yml` (if generated) names both arch zips — then run `scripts/check-brand-consistency.sh`, `scripts/check-auth-helper-bundled.sh` against each built `.app`, and `scripts/smoke-test-release-artifacts.sh` |
| `draft <ver>` | Generate `latest-mac.yml`, assert all 11 release assets exist, and `gh release create --draft` with the notes |
| `publish <ver>` | Re-run `verify`, require the release to already exist as a draft **and** a green `release-artifact-smoke.yml` run for this version, then `gh release edit --draft=false` |
| `all <ver>` | Every build + verify phase, ending at the draft release — publication stays a separate step |

A `verify` failure is not always about a missing artifact: it also fails on brand drift (a
`productName` or logo mismatch caught by `check-brand-consistency.sh`), on a macOS app whose auth
helper is not bundled, on an unstapled app, and on a failing release smoke test. The message names
the failing check.

The native Windows smoke workflow is a **hard gate between `draft` and `publish`**. `publish`
looks for a successful `release-artifact-smoke.yml` run titled exactly `Release artifact smoke
v<ver>`; without one it aborts. Run that workflow against the draft's Windows zip first.

For an agent-orchestrated run where each phase is a verified subagent that stops on the first
failure, use the `release` workflow in [`.claude/workflows/release.js`](.claude/workflows/release.js).

## Version numbers

The version lives in **one** source of truth: `[workspace.package].version` in `Cargo.toml`. The CLI,
the daemon, and the core library all inherit it, so the three Rust binaries can never disagree.
`bump` writes the number into five files. One is `Cargo.toml` itself; the other four hold six copies
of it between them, because two of those four carry it twice:

- `ui/desktop/package.json`
- `ui/desktop/package-lock.json` (2 occurrences: `.version` and `.packages[''].version`)
- `ui/desktop/openapi.json` (`.info.version`)
- `README.md` — the version badge, **both** its shields.io URL and its `alt` text

**Never hand-edit a version file** — always use `scripts/release.sh bump <ver>`.
`scripts/check-version-consistency.sh` (run by `just check-everything` / `just check-versions`) fails
CI if any of them drift from `Cargo.toml`. The README badge matters most here: it is the version most
visitors actually see, and it sat three minors stale before it was brought under this tooling.

### Choosing the version

`bump` (and `all`) accept a literal `X.Y.Z` or one of three keywords that compute the next version
from the tree's current one:

| Argument | From 1.88.6 |
| -------- | ----------- |
| `major` | 2.0.0 |
| `minor` | 1.89.0 |
| `patch` | 1.88.7 |

`minor-minor` is accepted as an alias for `patch`. A leading `v` is stripped, so `v1.88.7` works. A
**backwards** bump is refused outright — electron-updater compares versions, so a lower number ships
an update clients will reject.

The keywords are valid **only** for `bump` and `all`. Every other phase refuses them, because those
phases run against a tree `bump` has already rewritten and a keyword would resolve one step too far.
After a keyword bump the script prints the resolved number to pass to the later phases.

## Release notes

Per-version notes live at `docs/releases/notes/v<ver>.md`, one file per version. `draft` (which `all`
runs) reads that exact path and aborts if it is missing, so write the notes before drafting.

## After a release

The Linux packaging phase leaves a Linux-flavored `node_modules`. Restore a mac-native tree before
building again:

```bash
cd ui/desktop && rm -rf node_modules && npm ci
```

`npm ci`, not `npm install` — the committed `package-lock.json` is the reproducible source, and it is
what every automated path uses (`cmd_all`, the dmg-dep check, and the Linux phase's own closing
advice). `npm install` is free to move a dependency, which is exactly the drift this restore is
meant to undo.

## Further reading

`CLAUDE.md` carries the long-form version of this pipeline, including every hard-won invariant
(cross-compile link fixes, the one-platform-at-a-time staging rule, the Linux glibc baseline, the
exact set of release assets, and the notarization setup). `scripts/release.sh` itself documents the
same invariants inline.
