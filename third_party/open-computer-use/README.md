# BioRouter native Computer Use payload

`pin.json` pins the MIT-licensed upstream commit. `patches/` is applied in lexical
order to a fresh detached checkout; a dirty local source clone cannot affect a
build. Every payload contains the upstream license, notice, source pin, patch
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
