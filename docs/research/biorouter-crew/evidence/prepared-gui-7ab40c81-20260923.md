# Prepared development app refresh

The three existing isolated QA apps in
`/private/tmp/biorouter-crew-dev-apps-c857cc1b-1/` now embed the recorded
`7ab40c81` native CLI/daemon pair. The directory name reflects original
preparation history, not the refreshed binary source. The production desktop
source is unchanged between `c857cc1b` and `7ab40c81`; the only desktop change
is `src/daemonRuntime.regression.test.ts`.

| Embedded binary in each app | SHA-256 after app signing |
|---|---|
| biorouter | `ffa2f599621159be35a9efbf9c6f74c0ce5f3497d72da41d0d6bcece29c09624` |
| biorouterd | `4a1082ec9e1e0cc0b464c8cd608156a21cd24544c27cd87a2d42e7f84b16185c` |

Each hash matches the immutable source artifact. A whole-file comparison also
passed for the Alice copies. The immutable originals were not modified.
Before replacement the embedded CLI/daemon hashes were respectively
`d5cf729d5e5db26cd604cb59ef02ac8c7a50bbee3e09c29413998c4d9a6c02a8` and
`d79ade3fd2d2f757651ff6e6970c6cd416433eecd15865142c381a66f5468ef7`.

Only those embedded copies were replaced, then each app was ad-hoc signed with
`codesign --force --deep --sign - --timestamp=none` and passed
`codesign --verify --deep --strict`. Bundle IDs remain
`dev.biorouter.crew.qa.alice`, `.bob` and `.carol`; TeamIdentifier is unset.
This is a development signature, not notarization or a distribution signature.
Available disk space before copying was 211 GiB.

Luna performed no app launch, registration, native UI control, AWS operation,
JavaScript build or Cargo build. This is preparation and artifact verification;
mixed GUI/CLI and native file-dialog acceptance remain unexecuted pending the
specific native-app authorization.
