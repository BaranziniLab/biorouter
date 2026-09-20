# Installing an extension, and where its credentials go

> **What this is.** How a `.brxt` extension gets installed from each of the four surfaces that can do it, and the design of the credential path — why a secret never enters the conversation, and what that costs.
> **Status:** Current.
> **Audience:** contributors, and operators deciding how to configure extensions safely.

Four surfaces install extensions: **Browse Extensions** in the desktop app, the **Add Extension** file drop beside it, `biorouter extension install` at a terminal, and an agent asked to do it in chat. They are one transaction with four front doors — [`crates/biorouter/src/extension_install/`](../../crates/biorouter/src/extension_install/) — and before that existed they were three implementations that disagreed on the one thing that matters.

---

## What an install actually does

```
download → validate → extract → uv sync → credentials → register → attach
```

Only the fifth step can stop and wait for a person, and every step after a failure has to undo the ones before it. That is why these are one type ([`ExtensionInstallTransaction`](../../crates/biorouter/src/extension_install/transaction.rs)) rather than a sequence a caller composes: without the undo, a machine is left with a half-registered extension, an orphaned `~/.config/biorouter/extensions/<name>/` tree, and a credential in the keychain for something that is not installed.

The states an install can end in are all reportable, to a person and to a model:

| State | Means |
|---|---|
| `attached` | Registered, and live in the chat that asked for it. |
| `installed` | Registered, but not attached — nothing asked, or the hot-load failed and the extension is still correct for the next chat. |
| `needsCredentials` | Required values are missing and this run may not ask. **Nothing was registered.** The key *names* are reported so an operator can configure them and re-run. |
| `cancelled` | A person declined. Nothing was registered. |
| `failed` | Something broke. Everything this run created was undone. |

Rollback is scoped to the run's own work. A tree that already existed is not deleted — a failed upgrade that removes a working extension is worse than a failed upgrade — and a credential the machine already held is not revoked, because other extensions may share it.

> **A cancelled credential step leaves a claim on disk, and the claim is what makes it findable again.** The extracted tree and its built Python environment survive, and beside them — in `extension-installs/`, a *sibling* of the extensions directory so a bundle cannot forge one — sits an [`InstallClaim`](../../crates/biorouter/src/extension_install/claim.rs) naming the extension, its install directory, and which keys are still needed. No values, ever.
>
> ⚠ **This used to say the record lived in a process-global map and that "retrying does not re-download or rebuild". Both were wrong.** The map was write-only and died with the process, so quitting the app left the tree unreachable and invisible on every surface — which is the whole reason the claim is now written to disk. And a resume *does* re-download and re-extract; what survives the round trip is the built `.venv`, which is the expensive part. Finish a claimed install with `biorouter extension configure <name>`, which prompts with echo off.

---

## The credential path

**A credential goes from the trusted surface to the OS credential store, and nothing else ever sees it.** Not the model, not the provider, not a session row, not a log line, not a process argument, not a diagnostics bundle.

```
 install ──park(Secrets{keys, destination})──▶ card in the chat   (KEY NAMES ONLY)
                                                   │
        user types into Biorouter's own dialog ────┘
                                                   │
        POST /action-required/secrets  (X-User-Action)
                                                   ▼
                                          store the values
                                            ├─ declared secret → OS credential store
                                            ├─ ordinary setting → config.yaml
                                            └─ release the install with the NAMES
 install ◀──── configuredKeys: ["SPOKEAGENT_PASSCODE"] ─────┘
```

### Why the obvious implementation is wrong

BioRouter already had a mechanism for asking the user something mid-turn: MCP elicitation. Reusing it and giving the form `type="password"` inputs looks like a two-line change, and it fails on its first turn — invisibly.

`createElicitationResponseMessage` builds an `elicitationResponse` whose `user_data` is marked `agentVisible: true`, and `Agent::reply` forwards that whole object to the waiting request. Masking the characters would hide the secret from **the person typing it** and from nobody else: it would still be serialised into the transcript, persisted to the session row, replayed into the next prompt, and flattened into a child agent's transcript.

So the values do not travel on the conversation transport at all. [`SecretRequestCard`](../../ui/desktop/src/components/SecretRequestCard.tsx) takes no `append` and no `onSubmit` — it cannot put anything into the conversation even if it tried — and there is deliberately **no** `SecretResponse` sibling to `ElicitationResponse` for one to be serialised into.

### The guarantee is enforced, not documented

- `UserActionOutcome::SecretsConfigured` has no field a value can sit in.
- `PendingUserActions::resolve` **refuses** a value-bearing answer to a credential card and leaves the caller parked, so the property holds for surfaces that module has never heard of.
- The card carries `{key, label, description, required}` and no `secret` flag — it is the *ask*, and widening it would be one more place a value could be attached. The writer tells a credential from a setting using the manifest's declaration, registered beside the park.
- A key the card never asked for is **dropped**, not written. A form post does not get to choose what lands in the keychain.
- `SubmitSecretsRequest` hand-writes `Debug` to print key names, because a derive would put a passcode into any tracing line or test failure that formatted the body.

### Proof of user

`POST /action-required/secrets` requires DR-16's `X-User-Action` header. The model reaches the same daemon over the same HTTP with the same secret key; without the proof it could satisfy its own credential card with a value it invented and drive the install past the one step that exists to involve a person.

Two consequences worth knowing:

- **The refusal is written for a model to read.** It forecloses a retry and never suggests asking for the value in chat, because that is the exact failure this feature exists to end.
- **A daemon started without a user-action key cannot accept credentials over HTTP.** That includes `biorouter serve` (browser access), which spawns its daemon with `Stdio::null()` on purpose — see [SD-1](../deployment/serve-decisions.md). Configure those at a terminal with `biorouter extension install` or `biorouter extension configure`, which prompt with echo off.

---

## Per-surface behaviour

### Desktop — Browse Extensions

Add downloads the bundle inside the installer, so progress, failure, **Retry** and **Back to marketplace** are all one surface. Local-file controls never appear on this route (issue #116). If the manifest declares values, the configuration step opens; if it declares none, the button says **Install extension** and installs.

### Desktop — Add Extension (local `.brxt`)

Keeps the drag/drop and file picker, including after a bundle is loaded, so a mis-picked file can be replaced. A path preloaded over IPC (double-clicking a `.brxt` in Finder) is still this route — a preloaded path means "a file is chosen", never "this came from the marketplace".

### CLI

```bash
biorouter extension install ./spokeagent-0.4.1.brxt
```

At a terminal, missing values are prompted for with **echo off** for anything the manifest declares secret. Unattended, the install stops with a non-zero exit naming the keys it needs and registers nothing.

> ⚠ **This used to warn and register anyway.** A human reading the warning scroll past might catch it; an agent reads the exit code, reports success, and leaves an extension that starts and cannot authenticate. There is no install-it-broken path any more.

For unattended runs:

```bash
printf '%s\n' 'SPOKEAGENT_PASSCODE=…' | biorouter extension install ./bundle.brxt --secret-stdin
```

`--secret KEY=VALUE` still works and now says out loud that the value is in your shell history and visible to `ps`. Do not put a credential in a command-line argument.

To re-enter credentials later without reinstalling:

```bash
biorouter extension configure spokeagent
```

Already-configured keys are listed **by name** and never read back to pre-fill a field — reading one back would put it on screen, which is the one thing an echo-off prompt exists to prevent.

### From a chat

The agent calls `install_extension` with an exact trusted BAAM registry id.
BioRouter resolves the download URL itself, validates the model's eligibility and
asks for approval of that exact package descriptor. It never accepts an
agent-supplied download URL or shells out to install. Credentials are entered in
BioRouter's own dialog; the agent receives key names and status, never values.

A public-model chat cannot install a private connector through this tool: it is
refused before approval or download, independently of the diagnostic privacy
toggle. Attach performs its own reach checks as well. For otherwise authorized
installs, an attach failure is reported separately from installation; an
installed-only result is not evidence that the extension's tools are callable.

---

## Staying current: `CatalogChanged` (issue #112)

An install that succeeds on disk is useless if nothing notices. Four inventories read the extension map — `ConfigContext.extensionsList` in the renderer, the Settings list, the composer's picker, and the running agent's `ExtensionManager` — and each used to be repaired by whichever code path happened to write. An install from *outside* the GUI repaired none of them, which is why two correctly installed extensions could not be attached to the chat that had just asked for them.

There is now one event, and every inventory invalidates from it.

```
  set_extension / remove_extension / set_extension_enabled   (this process)
  config.yaml changed underneath us                          (any process)
                         │
             CatalogEvents::global().publish(..)  → revision += 1
                         │
        GET /catalog/changes?since=N   (long poll, parks ~25s)
                         │
       ConfigContext ──┬── Settings list
                       ├── composer picker
                       └── window `catalog:changed`  → non-React consumers
```

### What is in it

`CatalogChanged` carries a monotonic `revision`, a `reason`, and per-extension rows keyed by `name_to_key(name)` — the join every surface already uses, and the only identifier that survives a display-name change. A row carries the normalized `config` so a consumer can repair its row without a refetch, `enabled`, and `bundledSkillIds`: the skills that extension's bundle contains.

> ⚠ **The revision is the contract, not the payload.** A consumer that applies `changes` and never refetches drifts the first time two changes race. `truncated` means the client fell further behind than the daemon's buffer holds, and it is an order to refetch, not a warning: applying a partial history and believing yourself current is the same stale-inventory bug one layer down.

### Three things worth knowing

- **The watcher is what makes a CLI install visible.** `biorouter extension install` in another terminal writes `config.yaml` from a different process and reaches none of the in-process choke points. `spawn_config_watcher` stats the file every two seconds and reports only what this process did *not* do — a plain `stat`, not a filesystem-notification API, because it behaves identically on every platform and over the network mounts a shared config directory sometimes lives on.
- **An identical rewrite publishes nothing.** `syncBundledExtensions` and the capability migrations re-save entries at every startup; announcing those would have every client refetch its whole inventory on every launch.
- **A daemon restart needs no special case.** The revision resets to 0, so a client holding a higher number sees a *lower* one come back and refetches.

### Offered, not attached

A running chat snapshots the extensions it started with. When an extension appears from somewhere else — another terminal, another window — the row appears in the composer's picker and a toast says so, and the click is the user's. An agent asked to install one *in* a chat attaches it itself, because there the user did ask.

`bundledSkillIds` states what the bundle **contains**, not what has been installed to the skills directory — the Rust install path does not install bundled skills today (the Electron one does). Treat it as "look here", never as "these are present".

## Tests

```bash
cargo test -p biorouter --lib -- extension_install
cargo test -p biorouter --test extension_install_secrets
cargo test -p biorouter --lib -- catalog::
cargo test -p biorouter --test catalog_inventory
cargo test -p biorouter-server --lib -- routes::catalog
cd ui/desktop && npx vitest run src/utils/catalogSubscription.test.ts
cargo test -p biorouter-server --lib -- routes::action_required
cargo test -p biorouter-cli --lib -- commands::extension
cd ui/desktop && npx vitest run src/components/SecretRequestCard.test.tsx
```

`extension_install_secrets` is the one that matters, and every test in it asserts an **absence**. That is the point: an implementation that renders the dialog perfectly, stores the value correctly and *also* writes it into `config.yaml` would pass every functional test this feature could have. The surfaces it checks are the plaintext config entry, the published card, the parked call's outcome and its `Debug` rendering, a diagnostics bundle, the session store (the card is ephemeral, so it is never written), and the widest outbound provider payload there is — the coding-agent transcript flattener.

The desktop suite adds a structural assertion for the same reason: `types/message.ts` must define no `createSecretResponseMessage`, because that one line beside `createElicitationResponseMessage` would look like the obvious next step and would put the credential straight back on the transport.

## Related documentation

- [Extensions, skills, and MCP agents](extensions-and-skills-guide.md) — the end-user guide to all three extension kinds.
- [Extension Manager](built-in/extension-manager.md) — the built-in that hosts `install_extension`.
- [Secret storage](../security/secret-storage.md) — where a credential actually lives once stored.
- [Privacy tiers](../security/privacy-tiers.md) — Gate F1, and why it lands on the attach.
- [Browser access decisions](../deployment/serve-decisions.md) — SD-1, and why a browser daemon cannot accept credentials over HTTP.
