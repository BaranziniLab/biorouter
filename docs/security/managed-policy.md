# Managed enterprise policy

> **What this is.** The reference for BioRouter's managed policy tier: an admin-owned config
> file that overrides user and project settings for permissions, hooks and local dictation,
> including its per-OS location, the ownership check that decides whether it is trusted, and
> its YAML schema.
> **Status:** Current. Phase 1 is shipped and described below under
> [what is implemented today](#what-is-implemented-today); later phases are listed under
> [roadmap](#roadmap-phase-2-and-later). One gap is load-bearing: **ownership verification is
> not enforced on Windows.**
> **Audience:** administrators deploying BioRouter to a lab or institutional fleet (file
> location, deployment, schema), and developers working on the permission and hooks subsystem
> (behaviour notes, roadmap). Each section says which.
> **Identifier key.** `BR-NN` identifiers are proposals from the 67-item master list in
> [the agent-loop improvement proposals](../history/agent-loop-review/improvement-proposals.md).
> This feature is BR-65; the implementation design is
> [managed policy tier](../agent-loop/designs/managed-policy-tier.md).

BioRouter's hooks and permissions are normally configured per user (global
`~/.config/biorouter/config.yaml`) and, opt-in, per project (`.biorouter/hooks.yaml`). Both
tiers are owned by the user, so a lab or institutional deployment has no way to enforce a rule
the user cannot turn off. The **managed policy tier** adds a trusted, admin-owned layer that
wins over both.

## What the tier governs

- **Permissions** — force certain tools to be denied, to require approval, or to be
  pre-approved, regardless of the user's own `permission.yaml` or the active
  [permission mode](permission-modes.md), including `Auto`.
- **Hooks** — mandatory lifecycle hooks that always run and cannot be disabled, plus an
  override that forces or forbids project hooks org-wide. See the
  [hooks reference](../agent-loop/hooks/hooks-reference.md) for the hook schema this reuses.
- **Dictation**: a `dictation:` block turns local voice dictation off, or narrows it to
  recordings only, to an allowed set of speech models, to a folder of models the
  administrator fills, or to a dictionary the agent cannot write. See
  [the dictation block](#the-dictation-block).

Precedence is:

```text
Default (built-in)  <  User (global config)  <  Project (opt-in)  <  Managed (admin)
```

Managed **deny** and **ask** always win. Managed **allow** is applied as the permission
baseline (ahead of the user's table), so it can pre-approve a tool the user marked "Never
Allow" — but a higher managed **deny** or a security inspector can still escalate it. `deny`
is checked before `ask` before `allow`.

## File location

*For administrators.*

The managed file is read from an admin-owned, per-OS location. There is **no**
environment-variable override in production (an env var is user-settable and would defeat the
tamper model):

| OS | Path |
|---|---|
| macOS | `/Library/Application Support/Biorouter/managed-policy.yaml` |
| Linux | `/etc/biorouter/managed-policy.yaml` |
| Windows | `%ProgramData%\Biorouter\managed-policy.yaml` |

Deploy it via mobile device management (MDM) tooling such as Jamf (macOS), a package postinstall
or Ansible (Linux), or Group Policy or an installer (Windows).

## Ownership verification

*For administrators.*

Before the file is parsed, BioRouter verifies that neither the file nor its parent directory
can be rewritten by a non-privileged user (mirroring Gemini CLI's ownership verification
against privilege escalation):

- **Unix** — the file and its parent directory must be owned by `root` (uid 0) **or** the
  current user, and must **not** be group- or world-writable (`mode & 0o022 == 0`). Install
  with, for example:

  ```bash
  sudo install -o root -g wheel -m 0755 -d /etc/biorouter
  sudo install -o root -g wheel -m 0644 managed-policy.yaml /etc/biorouter/
  ```

- **Windows** — `%ProgramData%` is admin-writable-only by default, so phase 1 trusts the
  location; deep access control list (ACL) owner verification is a planned follow-up.

> **Warning — Windows.** Because the Windows path is trusted by location rather than verified,
> the ownership guarantee this tier's trust model rests on is absent on that platform. Treat a
> Windows deployment as relying on the default `%ProgramData%` ACLs being intact. See
> [roadmap](#roadmap-phase-2-and-later) and the
> [managed policy tier design](../agent-loop/designs/managed-policy-tier.md).

A file that fails the check (untrusted owner, world-writable, unreadable, or malformed YAML) is
**ignored with a warning** and the agent runs normally. This is a deliberate
availability-over-strictness choice: a bad MDM push must not brick every lab machine.

Dictation is the one exception. A file that exists but cannot be trusted, read or parsed turns
**dictation off** while the rest of the policy is ignored as described above, because an
administrator who shipped a broken file most likely meant to restrict something. See
[how the block fails](#how-the-block-fails).

## Schema

*For administrators.*

```yaml
# Managed hooks: identical schema to the user `hooks:` map. They always run and
# resolve BEFORE user/project hooks. A managed Stop/PreToolUse block cannot be
# cleared by a user hook (merge takes the most-restrictive decision).
hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "/usr/local/lib/biorouter/managed-shell-guard.sh"
  Stop:
    - hooks:
        - type: prompt
          prompt: "Block stopping until the audit log has been written."

# Override the user's `allow_project_hooks` opt-in.
#   false -> forbid project hooks org-wide (even if the user set true)
#   true  -> force project hooks on (even if the user did not opt in)
#   (omit) -> leave the user/env value untouched
allow_project_hooks: false

permissions:
  # Force auto-approve (applied as the permission baseline). Wins over a user
  # "Never Allow" for the same tool.
  allow:
    - "developer__text_editor"
  # Force an approval prompt (escalation; non-bypassable).
  ask:
    - "memory__.*"          # exact tool name or anchored regex (matcher.rs)
  # Force a hard denial (escalation; non-bypassable, works even in Auto mode).
  deny:
    - "developer__shell"

  # Reserved for future command-level policy (BR-20/BR-21). Parsed but inert in
  # this release, so a managed file can carry these forward without breaking
  # older binaries.
  command_rules: []
```

Tool names in `allow`/`ask`/`deny` are matched exactly or as an anchored regex, using the same
matcher the hooks engine uses (`crates/biorouter/src/hooks/matcher.rs`), so `a|b` alternation
and `foo__.*` both work.

## The dictation block

*For administrators.*

The `dictation:` block governs local voice dictation (DD-90 in the
[dictation implementation plan](../dictation/implementation-plan.md)). Every key only
narrows, and writing `true` is the same as leaving the key out:

```yaml
dictation:
  enabled: false               # no capture or transcription anywhere, no downloads, no dictionary
  capture: false               # no live microphones; transcribing a recording still works
  model_downloads: false       # only models already in models_dir are used
  allowed_models: [whisper-small.en-q8_0]
  models_dir: /var/lib/biorouter/dictation/models   # read only, still hash checked
  external_engine: false       # parsed, no effect until an engine address exists
  dictionary: false            # no dictionary edits, sidebar row, capability, extractor or bias
  dictionary_agent_writes: false   # the agent's dictionary tools are not offered
```

| Key | When `false` (for `allowed_models` and `models_dir`, when set) |
|---|---|
| `enabled` | Dictation is off on every surface: no microphone, no transcription, no model install, no dictionary. Every other key reads as `false` too. Deleting a model and exporting the dictionary still work |
| `capture` | Live microphones are off. Transcribing a recorded file still works |
| `model_downloads` | Nothing is downloaded. `models_dir` is the only source of speech models, so set it too: without it no folder supplies a model |
| `allowed_models` | Only the listed model ids may be installed, selected or used. An empty list allows none |
| `models_dir` | A read only folder of speech models, searched first. Files there are still checked against the catalog's hashes. It must be an absolute path |
| `external_engine` | Parsed and accepted, with no effect in this release: there is no off machine engine yet. Accepting it keeps a later policy file from turning dictation off on this release |
| `dictionary` | No dictionary edits, sidebar row, agent capability, chat extractor or recognition bias. Transcription still works, with no bias terms. Implies `dictionary_agent_writes: false` |
| `dictionary_agent_writes` | The agent's `dictionary__*` tools are denied and the chat extractor is off. The person's own edits still work |

The refusals people see use these sentences:

| When | Sentence |
|---|---|
| `enabled: false` | Dictation is turned off on this computer by your organization's managed policy. |
| `capture: false` | Your organization's managed policy turns off live microphone dictation on this computer. Transcribing a recording still works. |
| `model_downloads: false` | Your organization's managed policy turns off model downloads on this computer. Speech models come only from \<models_dir>; ask your administrator to place one there. |
| `allowed_models` excludes a model | Your organization's managed policy allows only these speech models here: \<ids>. |
| A `dictionary__*` tool under `enabled`, `dictionary` or `dictionary_agent_writes: false` | Blocked by your organization's managed policy. |

### How the block fails

The block is parsed apart from the rest of the file, and it **fails closed** for dictation
alone:

| File state | Dictation | The rest of the policy |
|---|---|---|
| No file | Unmanaged: everything allowed | Inert |
| A trusted file with no `dictation:` key | Unmanaged | Applies |
| A valid block | Each key narrows as written | Applies |
| An empty `dictation:`, an unknown key, a value of the wrong type (including an empty value), a relative `models_dir`, or a block that is not a mapping | **Off**, with a warning naming the key | Applies |
| A file that is untrusted, unreadable or not valid YAML, or whose existence cannot be checked | **Off**, with a warning | Ignored, as before |

A key this release does not know therefore turns dictation off rather than being ignored, so a
policy written for a newer release never grants more than its author meant. An empty
`dictation:` (also `dictation: ~` or `dictation: null`) turns dictation off for the same reason.
It is what a block whose keys lost their indentation looks like: YAML then reads those keys as
top-level keys, which the file ignores, so `enabled: false` written without its indentation
would otherwise have done nothing. Because the block
is kept as raw YAML when the file is parsed, a block this release cannot understand never fails
the file, and `permissions.deny` keeps its force.

Write the switches as `true` or `false`. The file is read as YAML 1.2, where `yes`, `no`, `on`
and `off` are text rather than switches, so `enabled: yes` is a value of the wrong type and
turns dictation off.

`POST /agent/call_tool` and the dictation doors read, verify and parse the file again for each
request, so a change applies at the next request without a restart. A chat's agent reads the
file when the agent is built, so its tool rules change when that agent is next created, for
example after a restart. A broken file repeats its warning each time it is read.

The switch prevents accidents; it is not a boundary. `BIOROUTER_PATH_ROOT` relocates the
policy file for everything the person runs, and on Windows the file's location is trusted
rather than verified.

## What is implemented today

*For developers and administrators verifying a deployment.*

- **No managed file → zero change.** If the file is absent or untrusted, the managed inspector
  is skipped and hook resolution is byte-for-byte the previous behavior. Solo users pay
  nothing.
- **Managed deny works in every mode**, including `Auto`. It rides the existing escalation-only
  inspection merge, so no later inspector can lower it. A tool the user "Always allow"-ed
  silently becomes denied, surfaced as *"Blocked by your organization's managed policy."*
- **Managed deny also holds for a direct call through `POST /agent/call_tool`**, the route
  that dispatches a tool without the agent loop and therefore without any inspector. Before
  the dictation block shipped, a denied tool ran there. A managed `ask` is not refused at that
  route, because its callers include the person's own actions in the desktop app.
- **Known gap: a script a person runs through that route.** When a person runs an
  `execute_code` script through `POST /agent/call_tool`, the tool calls inside the script are
  not yet checked against managed deny, so a script that calls `developer__shell` under
  `deny: [developer__shell]` still runs it. A script the agent runs is not affected: its calls
  go through the agent's inspectors, the managed one included.
- **The dictation block is parsed and enforced at the tool level today.** The `dictionary__*`
  tools are denied while the block keeps the agent out of the dictionary, and the managed
  inspector runs even when the file failed to load, so that denial holds in the agent loop and
  for direct calls through `POST /agent/call_tool`, with the script gap above. The dictation routes,
  the command line and the desktop app read the same resolved rules
  (`ManagedPolicy::dictation()` in `crates/biorouter/src/managed/dictation.rs`) as each of
  them lands.
- **The managed file is read-only** to BioRouter; the agent never writes it. No session,
  `permission.yaml`, or `.biorouter/hooks.yaml` state changes.
- **`permissions.command_rules` is parsed but inert** — see the roadmap below.

## Roadmap (phase 2 and later)

*For developers.* Nothing in this section is shipped.

- Windows ACL owner verification (`Administrators`/`SYSTEM`).
- A `biorouter policy show` subcommand to print the resolved managed layer and its trust status
  for deployment verification.
- BR-20 (catastrophic denylist) and BR-21 (argv/prefix command policy) evaluate
  `permissions.command_rules` inside the managed inspector.

## Related documentation

- [Permission modes](permission-modes.md) — the user-owned tier this policy overrides,
  including `Auto`.
- [Hooks reference](../agent-loop/hooks/hooks-reference.md) — the hook schema the `hooks:` key
  above reuses, and the user and project tiers it outranks.
- [Managed policy tier design](../agent-loop/designs/managed-policy-tier.md) — the BR-65
  implementation design, source-file map, and phase plan.
- [Configuration file reference](../configuration/config-file-reference.md) — the user and
  project config files that sit below this tier.
- [Dictation engine and models](../dictation/engine-and-models.md): the design of the
  administrator's switch, and every door that reads the `dictation:` block.
- [Security](README.md) — the rest of the security documentation.
