# Secret storage

> **What this is.** How BioRouter stores provider API keys and other secrets in the operating
> system's native credential store, what to expect on each platform, why macOS asks for your
> password and how many times, and the escape hatches when the credential store is unavailable
> or unwanted.
> **Status:** Current.
> **Audience:** end users on the first two sections; developers and maintainers on
> [developer builds](#developer-builds-rebuilds-re-prompt-unless-you-sign),
> [tests](#tests-never-use-the-credential-store) and
> [background](#background-why-one-password-cannot-cover-both-binaries). Each section says
> which.

BioRouter stores provider API keys and other secrets in the operating system's native
credential store: the **macOS Keychain**, the **Windows Credential Manager**, or the **Linux
Secret Service** (GNOME Keyring / KWallet). While the credential store answers, secrets never
touch disk in plaintext, custody stays with the OS, and access is gated by the same mechanisms the
rest of the platform uses. Where no credential store answers, or you opt out, secrets go to a
plaintext file instead, which Biorouter creates readable only by your account (see
[Escape hatches](#escape-hatches)). The provider screens say so in one sentence: "Keys are kept
in your system keychain when one is available, otherwise in a private file in Biorouter's
settings folder."

BioRouter reads the credential store **once per process** and caches secrets in memory for the
rest of the run, so a single authorization covers an entire session — you will never see
back-to-back identical prompts. This matters mainly on macOS, which is the only platform that
prompts at all.

## Platform behaviour at a glance

*For end users.*

| Platform | Store | Prompts | Notes |
|---|---|---|---|
| macOS | Keychain | Up to once per binary, ever | See [macOS](#macos-the-keychain-password-prompt) below |
| Windows | Credential Manager | None | Credentials are protected per-user by the Windows Data Protection API (DPAPI). BioRouter transparently splits large secret sets across multiple credentials to stay under Windows' 2560-byte per-credential limit. |
| Linux | Secret Service | None on a desktop session | Desktop sessions unlock the Secret Service with your login. On headless machines, SSH sessions, the Windows Subsystem for Linux (WSL), or systems without a Secret Service daemon, BioRouter automatically falls back to file storage (`~/.config/biorouter/secrets.yaml`). |

## macOS: the Keychain password prompt

*For end users.*

When a BioRouter binary reads the `biorouter` Keychain item for the first time, macOS itself
shows a password dialog — the OS asking you to authorize *that specific executable* to read the
item. Two facts shape the whole experience:

- **"Allow" authorizes a single access; "Always Allow" is permanent.** Always Allow adds the
  binary to the Keychain item's access control list, so it is never asked again. Always click
  **Always Allow**.
- **Authorization is per binary.** The desktop app's backend (`biorouterd`) and the CLI
  (`biorouter`) are separate executables, so macOS tracks a separate grant for each.

### How many times will I type my password?

- **Desktop-only usage: zero prompts.** The binary that *creates* a Keychain item is trusted
  for it automatically. If you enter your API keys through the desktop app, `biorouterd`
  created the item and never prompts.
- **Adding the CLI (or vice versa): one prompt, once.** The other binary needs its own grant:
  type your password once and click **Always Allow**. Release builds are Developer ID-signed,
  so the grant survives app updates.

So in the worst case — using both the GUI and the CLI, with secrets created by the other one —
your password is needed **twice, ever**, once per binary. Usually it is once or not at all.

## Escape hatches

*For end users and developers.*

| Variable | Effect |
|---|---|
| `BIOROUTER_DISABLE_KEYRING=true` | Store secrets in plaintext `~/.config/biorouter/secrets.yaml` instead of the OS credential store |
| `<KEY_NAME>` (e.g. `OPENAI_API_KEY`) | Any secret can be supplied directly via the environment; this always wins over the credential store |

The `secrets.yaml` path above is the macOS and Linux location. It sits alongside `config.yaml`;
for the per-platform config directory, including Windows, see the
[configuration file reference](../configuration/config-file-reference.md).

On macOS and Linux, Biorouter writes `secrets.yaml` with mode 0600 (readable and writable by your
account only), whatever your umask, and creates a missing folder with mode 0700. A `secrets.yaml`
that is readable by others is tightened to 0600 before the next write. Anyone who can act as your
account, or as root, can still read the file, so prefer the credential store where one exists.

Crew device keys are the exception: they never go to `secrets.yaml`. On a computer with no
credential store, Crew keeps them in an encrypted vault instead; see
[Keep device keys in an encrypted vault](../crew/command-line.md#keep-device-keys-in-an-encrypted-vault).

## Developer builds: rebuilds re-prompt unless you sign

*For developers working on BioRouter.*

`cargo build` produces ad-hoc-signed binaries whose identity changes on every build, so the
Keychain treats each rebuild as a brand-new app and re-prompts even after Always Allow.
`just copy-binary` (used by `just run-ui` and `just run-dev`) automatically re-signs the freshly
built `biorouter`/`biorouterd` with the Developer ID certificate when it is present in your
keychain, giving dev builds a stable identity — Always Allow then sticks across rebuilds.
Without the certificate, set `BIOROUTER_DISABLE_KEYRING=true` during development to use
plaintext `secrets.yaml` instead. Running the tests needs neither; see the next section.

## Tests never use the credential store

*For developers working on BioRouter.*

`cargo test` never reads or writes the macOS Keychain, the Windows Credential Manager or the
Linux Secret Service, and needs no environment variable for that. A test binary is ad-hoc signed
and gets a new identity on every build, so a Keychain read from one used to raise a password
dialog on every run (12 in one day on 2026-11-12, every one from a `target/debug/deps/` test
binary), and a dialog nobody answers blocks the test forever.

Every test binary that links the `biorouter` crate replaces the store before `main`:

- `biorouter::keep_this_test_binary_off_the_os_credential_store!()` installs a store kept in the
  test process's memory as the `keyring` crate's default. Every keyring entry in the process then
  reads and writes there: the secrets blob behind `Config`, Crew device keys, and any
  `Config::new(path, service)` a test builds. The macro also adds a test to that binary which
  fails if an entry would still be built on the OS store.
- The test sandboxes of `biorouter`, `biorouter-cli` and `biorouter-server` (`src/test_sandbox.rs`,
  declared by each library's test binary, by `biorouterd`'s, and by every `tests/*.rs` of the CLI
  and the server) invoke the macro. They also set `BIOROUTER_DISABLE_KEYRING=true` when it is
  unset, so `Config::global()` keeps its secrets in the sandbox's own `secrets.yaml` under its
  temporary `BIOROUTER_PATH_ROOT`, as every CI test step does, and a process a test starts
  inherits the same.
- Every `tests/*.rs` of `biorouter` and `biorouter-acp`, and the `biorouter-acp` library, invoke
  the macro directly. These binaries set no temporary root of their own, so they leave the
  variable alone and their secrets stay in memory.

Two tests in `crates/biorouter/src/config/test_binary_credentials.rs` keep this true.
`every_test_binary_that_links_biorouter_keeps_off_the_os_credential_store` fails when a new test
file, crate or binary target forgets the macro, and names it.
`only_test_code_keeps_a_binary_off_the_os_credential_store` fails when anything under a crate's
`src/` invokes it outside a `#[cfg(test)]` item. No environment variable and no Cargo feature can
switch the store off, so shipped binaries, and the development binaries the desktop app runs,
use the credential store exactly as described above.

The store replacement reaches only the test process itself. A test that starts a real `biorouter`
or `biorouterd` starts another process: under the CLI and server sandboxes it inherits
`BIOROUTER_DISABLE_KEYRING=true`, and anywhere else the test passes the variable on the child's
command itself, as the CLI's `serve_lifecycle` tests do.

A test that needs a real provider key takes it from the environment, never from the credential
store.

## Background: why one password cannot cover both binaries

*For maintainers. Nothing here changes what you do; it explains why the per-binary grant cannot
be removed.*

This is a deliberate macOS security boundary, not a BioRouter limitation. The Keychain
identifies an app by its code signature (its *designated requirement*) and records consent per
app, per item. If authorizing `biorouterd` also authorized `biorouter`, any program on the
system could ride along on another app's grant — so the OS simply does not offer a supported
way for one executable's authorization to extend to another. Apple's sanctioned mechanism for
sharing (keychain access groups) requires moving to the iOS-style data-protection keychain with
provisioning-profile entitlements, which the Rust `keyring` ecosystem doesn't support today.

## Related documentation

- [Configuration file reference](../configuration/config-file-reference.md) — the per-platform
  config directory and the `secrets.yaml` file this page's escape hatch writes.
- [Environment variables](../configuration/environment-variables.md) — the full reference for
  `BIOROUTER_DISABLE_KEYRING` and the provider key variables.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — where the API
  keys stored here come from.
- [Process-global state in tests](../testing/process-global-state.md) — the other per-binary
  test sandbox rules, which the test credential store sits beside.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md) — for
  credential-store failures that this page does not cover.
- [Security](README.md) — the rest of the security documentation.
