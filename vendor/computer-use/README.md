# BioRouter native Computer Use payload

`source/` is a COMPLETE copy of the MIT-licensed upstream project at the commit
`pin.json` names, committed to this repository. A build reads it and needs no
network, so an upstream repository that is deleted, force-pushed or altered
cannot affect BioRouter, and the exact bytes that go into a shipped helper are
reviewable in this repository's own history.

`source/` is **pristine** — the patches are not applied to it. `patches/` is
applied in lexical order to a throwaway copy at build time. Keeping the two apart
is what makes an upstream update tractable: replace the tree wholesale, re-apply
the reviewed patches on top, and a conflict is a real conflict rather than a
merge of our own edits with themselves.

`source-manifest.json` carries a SHA-256 of every vendored file plus one digest
over the whole tree, and the build verifies it before the bytes become build
input — in both directions, so an unrecorded file sitting in the tree is refused
as loudly as a modified one. `scripts/check-vendored-computer-use.sh` runs that
check in CI, and also asserts every manifest file is committable: repository-wide
`.gitignore` rules (`*.png`, `.agents/`, `.mcp.json`) matched 15 of them when the
tree was first vendored, and a tree that verifies locally while those files never
reach a fresh clone is worse than one that fails outright.

⚠ **Ten upstream files are deliberately NOT vendored**, listed under `excluded` in
the manifest with the reason. They are assets upstream extracted from another
vendor's shipped application for reverse-engineering notes. MIT covers upstream's
own work; it cannot relicense somebody else's artwork. They are documentation
references, not build inputs — nothing under `apps/` reads them — and a build is
byte-identical with and without them (verified).

### Checking for upstream changes

```sh
python3 scripts/check-computer-use-upstream.py          # summary
python3 scripts/check-computer-use-upstream.py --json   # machine-readable
```

Read-only: it never edits the pin, the tree, or the patches. The number it exists
to give you is not "N commits behind" but **which upstream files our patches also
touch**, because those are what will conflict on re-vendoring. The update
procedure is in that script's header.

### Updating the vendored tree

```sh
git clone <repository> /tmp/ocu && git -C /tmp/ocu checkout <new commit>
$EDITOR vendor/computer-use/pin.json          # commit + version
python3 scripts/vendor-computer-use-source.py --from /tmp/ocu
python3 scripts/computer-use-runtime.py build <target>  # patches re-apply here
```

A patch that no longer applies fails the build rather than being skipped. Bump
`patch_revision` when you rework one, and rebuild every target. Every payload contains the upstream license, notice, source pin, patch
hashes, target, launch arguments, and SHA-256 hashes of every shipped file.

Build prerequisites are Python 3, git, Go 1.26.8 for Windows/Linux, and Swift 6.2+
on macOS for the two Mac targets. They are **build** dependencies; installed
BioRouter never runs npm, git, Go, Swift, or a network installer for this feature.

```sh
python3 scripts/computer-use-runtime.py build darwin-arm64
python3 scripts/computer-use-runtime.py build darwin-x64
python3 scripts/computer-use-runtime.py build win32-x64
python3 scripts/computer-use-runtime.py build linux-x64
python3 scripts/computer-use-runtime.py verify linux-x64
python3 scripts/test-computer-use-protocol.py target/computer-use/linux-x64
```

`--source /path/to/upstream-clone` avoids a network clone while still checking out
the exact pinned commit and applying only the reviewed patches. Build concurrency
defaults to two workers; `BIOROUTER_BUILD_JOBS` can set a lower or higher bound.
Source and Swift intermediate trees use `.noindex` suffixes on macOS.

For Mac releases, pass `--signing-identity` with the BioRouter Developer ID
identity. Local builds otherwise use ad hoc signing, which does not promise TCC
permission continuity. The stable release bundle ID is
`org.biorouter.computer-use`. Each architecture has a distinct output directory;
the helper app contains the original BioRouter cursor and icon. Forge preserves
its completed signature instead of re-signing it after its manifest is hashed.
The outer app signature then seals both helper and manifest. The helper requires
macOS 14 or newer; this does not change the minimum OS of unrelated BioRouter
features. No upstream extracted cursor asset is copied.

When `WINDOWS_CERTIFICATE_FILE` is configured, Windows helper signing uses
`osslsigncode` and the protected password file named by
`BIOROUTER_WINDOWS_SIGN_PASSWORD_FILE`. It fails if configured signing cannot be
completed; the final signed bytes are hashed. Windows requires the signed-in
user's interactive session and Windows PowerShell with UI Automation support.

Forge stages the verified target under `resources/computer-use` (Mac:
`Contents/Resources/computer-use`) outside `app.asar`; this includes the bundled
CLI and updater ZIP. The CLI DEB/RPM and Docker image install the same payload at
`/usr/libexec/biorouter/computer-use`. Linux package dependencies explicitly
install distro Python, PyGObject, AT-SPI2, and GTK3. A Go executable alone is
insufficient. Bare archives/source installs must install those dependencies from
`pin.json`. Headless containers do not acquire a desktop by installing libraries.
The Docker image builds matching amd64 or arm64 payloads; Linux ARM64 is a
container target in addition to the four released desktop/CLI targets. Flatpak is not in the supported release matrix; its
sandbox desktop integration requires separate validation.

`prepare-platform-binaries.js` replaces staging only from a verified, matching
target. Forge verifies again after packaging. The release script inspects the
actual DMG, both updater ZIPs, Windows ZIP, and GUI/CLI DEB/RPM archives and records
the embedded manifest digest in release provenance. Missing files, architecture
drift, stale patches, added foreign files, and post-signing changes fail the gate.

`computer-use-native.yml` builds all four release targets plus Linux ARM64 for containers, verifies their complete
manifests, and exercises the ten-tool stdio handshake. This is protocol/build
evidence; it does not claim OS permission, interactive desktop, Wayland, or macOS
upgrade/TCC acceptance. Such platform acceptance remains separate from a green
build or an empty desktop list.

The Windows native CI job requires an interactive WinForms fixture run. Session
0 writes `desktop_unavailable` evidence and exits 77, failing the required gate;
an unavailable desktop is never counted as UI Automation acceptance. The fixture
independently verifies text/value changes, a key event, accessibility click, real
scroll offset, app discovery, snapshots, and PNG capture. Drag, mixed DPI, multiple displays,
occlusion, secure desktop, macOS TCC continuity, Linux X11/Wayland interactive
acceptance, and installed-package upgrade tests still require their respective
platform evidence; this fixture does not claim them.

Linux CI runs `test-computer-use-linux-fixture.py` on x64 and ARM64 in an isolated
Xvfb display, a real user D-Bus, Openbox, and GTK3. It must discover the fixture
through AT-SPI, change an editable field, invoke its button, independently read
the fixture result, move its actual scroll adjustment, and decode a nonblank
window screenshot. A synthetic Wayland declaration must produce an explicit
unsupported doctor state and reject capture without image content. This checks
the unsupported contract; it does not validate a native Wayland compositor.
