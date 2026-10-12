# Managed enterprise policy

> **What this is.** The reference for BioRouter's managed policy tier: an admin-owned config
> file that overrides user and project settings for permissions and hooks, including its
> per-OS location, the ownership check that decides whether it is trusted, and its YAML schema.
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

## What is implemented today

*For developers and administrators verifying a deployment.*

- **No managed file → zero change.** If the file is absent or untrusted, the managed inspector
  is skipped and hook resolution is byte-for-byte the previous behavior. Solo users pay
  nothing.
- **Managed deny works in every mode**, including `Auto`. It rides the existing escalation-only
  inspection merge, so no later inspector can lower it. A tool the user "Always allow"-ed
  silently becomes denied, surfaced as *"Blocked by your organization's managed policy."*
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
- [Security](README.md) — the rest of the security documentation.
