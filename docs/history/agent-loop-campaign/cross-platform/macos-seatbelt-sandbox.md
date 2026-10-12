# macOS Seatbelt sandbox for the shell tool (BR-64)

> **What this is.** The design for BioRouter's first kernel-enforced containment of the
> developer shell tool: a macOS Seatbelt profile with injected writable roots and outbound
> network denied, kept deliberately separate from the approval policy.
> **Status:** Superseded — Slice 1 shipped as
> `crates/biorouter-sandbox/src/seatbelt.rs` during the 2026-07 agent-loop fix campaign, but
> the forward-looking Slice 3 (Linux and Windows backends) was replaced wholesale by BR-69,
> which generalized this single-backend model into a `ShellSandbox` trait now living at
> `crates/biorouter-sandbox/src/shell_sandbox/{macos,linux,windows}.rs`. **The current plan of
> record is [Linux and Windows sandboxing (BR-69)](../../../agent-loop/designs/linux-and-windows-sandboxing.md).**
> Read this document for the Seatbelt profile design and the reasoning behind the two-axis
> model; do not use its phasing.
> **Audience:** developers working on BioRouter's sandboxing and tool-execution containment.

Every BioRouter guardrail before this work sat *before* `spawn()` and was advisory: the
inspector chain could allow, ask, or deny a tool *request*, but once a request was approved
the shell tool spawned a full-privilege child with the user's uid, environment, network and
filesystem. This document specifies the first control that is enforced by the kernel rather
than by prompt compliance — and confines itself to macOS, where Seatbelt needs no new
dependency.

> **Identifier key.** `BR-NN` identifiers are proposals from the 67-item master list in
> [the agent-loop improvement proposals](../../agent-loop-review/improvement-proposals.md).
> `P-NN` identifiers are the numbered entries in the three lens reviews under
> [proposal lenses](../../agent-loop-review/proposal-lenses/README.md); a lens is one of
> **P** (performance), **R** (robustness), or **U** (ux). This document is BR-64, raised
> under the robustness lens as P-32.

| Field | Value |
|---|---|
| Proposal | BR-64 |
| Lens | R (robustness P-32) |
| Scope | macOS only. The H1 of the original document promised "OS-level sandbox for tool execution" generally; the design has always been Seatbelt-specific. |
| Shipped | Slice 1, during the [agent-loop fix campaign](../README.md) (wave 1, security cluster) |
| Superseded by | [Linux and Windows sandboxing (BR-69)](../../../agent-loop/designs/linux-and-windows-sandboxing.md) |

> **Warning — this design ships on one of three platforms.** The cross-platform audit
> recorded this as **GAP-4**: the shell sandbox exists on macOS only, so Linux and Windows
> users get no kernel containment from BR-64 at all. See the
> [platform parity audit](platform-parity-audit.md). BR-69 is the response.

> **Note.** Every `file:line` citation below was taken against the pre-campaign tree, before
> the 2026-07-13 integration merge. The file paths remain accurate; the line numbers have
> since moved. Treat the paths as authoritative and the line numbers as historical anchors.

## What this design borrows

The primary model is Codex CLI: macOS Seatbelt via `sandbox-exec -p` with writable roots
injected and network-outbound denied, and a Linux `codex-linux-sandbox` built from Landlock
(filesystem) + seccomp (blocks network syscalls) + bubblewrap namespaces. Codex also supplies
the **two-axis model** that separates *what is technically possible* (the OS sandbox) from
*when to ask* (the approval policy), and escalates to an approval prompt on a sandbox denial
rather than hard-failing. See the
[Codex CLI research note](../../../research/coding-agent-landscape/codex-cli.md) and the
[safety and guardrails comparison](../../agent-loop-review/competitive-comparison/safety-and-guardrails.md).
OpenHands (Docker/VM runtime), Gemini CLI (Seatbelt/container) and Claude Code (Bash sandbox)
round out the field.

This complements — it does not replace — **BR-20** (always-on catastrophic denylist),
[**BR-21**](../../../agent-loop/designs/command-policy-engine.md) (auditable command policy engine) and
[**BR-65**](../../../agent-loop/designs/managed-policy-tier.md) (managed tier). Those are the *auditable allow/ask/deny
catalog*: what the model is *permitted* to ask for. BR-64 is the *kernel-enforced
containment*: what the process is *technically able* to do. The BR-21 design says the same
thing from the other side, calling an OS sandbox "complementary, not a replacement."

---

## The problem, grounded in code

BioRouter has **no process isolation at all**. Every guardrail sits *before*
`spawn()` and is advisory:

1. **The only enforcement is prompt-gated permission.** The tool-call gauntlet
   (Q6 in the
   [guardrails and permissions review](../../agent-loop-review/subsystem-reviews/guardrails-and-permissions.md))
   is Security → Permission →
   Repetition → Hooks inspectors, all of which run in the agent loop *before*
   the tool executes and can only Allow / Ask / Deny the *request*. Once a
   request is approved (or auto-approved in `Auto` mode), the developer shell
   tool spawns a full-privilege child with the user's own uid, environment,
   network, and filesystem — see `execute_shell_command`
   (`crates/biorouter-mcp/src/developer/rmcp_developer.rs:1283-1319`), which
   calls `configure_shell_command`
   (`crates/biorouter-mcp/src/developer/shell.rs:109-142`) →
   `tokio::process::Command::new(shell).arg("-c").arg(command).spawn()`. There
   is no `sandbox-exec`, no namespace, no seccomp, no Landlock anywhere on that
   path.

2. **So autonomy is bounded by prompt compliance, not the kernel.** In `Auto`
   mode the `PermissionInspector` returns `Allow` for everything
   (`permission_inspector.rs:121-122`); the regex scanner is
   `SECURITY_PROMPT_ENABLED=false` by default and, when on, only *asks* (gap #3 in the
   [guardrails and permissions review](../../agent-loop-review/subsystem-reviews/guardrails-and-permissions.md)).
   A prompt-injected or
   simply mistaken command that slips past the (now BR-20-gated) catastrophic
   denylist runs with the operator's full authority — it can read `~/.ssh`,
   exfiltrate over the network, or write outside the project.

3. **The comparison table is blunt about it.** The
   [safety and guardrails comparison](../../agent-loop-review/competitive-comparison/safety-and-guardrails.md)
   records "OS-level sandbox: **none**" for BioRouter, against Codex (Seatbelt/Landlock/token),
   Gemini (Seatbelt/container), OpenHands (Docker/VM), Claude Code (Bash
   sandbox). BioRouter is one of only three comparators (with Pi, Aider)
   with zero isolation.

There *is* a `biorouter-sandbox` crate, but it is **container/subprocess** level
(`LocalProcessSandbox` = a path-jail + timeout with *no* isolation, its own
docstring says so; `DockerSandbox` = the `docker` CLI), and it is wired only
into the BRSDK-app `compute`/`files` capabilities and the app path-jail
(`developer/jail.rs`, `compute_server/mod.rs`) — **not** the main-agent developer
shell tool. It gives us the `SandboxSpec` vocabulary (writable roots, network
policy, timeout) to reuse but no *kernel* backend.

---

## Design

Adopt Codex's model in three properties:

1. **Native OS enforcement** of a default-deny profile: full filesystem *read*,
   *writes* confined to injected writable roots (the session working dir + the
   temp dir), **network denied** by default.
2. **Off by default, opt-in per platform.** Kernel sandboxes are platform-
   specific and can break legitimate tool access (the proposal's own stated
   risk); shipping them on-by-default would regress every existing workflow.
   Gate behind `BIOROUTER_SHELL_SANDBOX` (env), off unless explicitly set.
3. **Escalate-to-approval on denial, never silent breakage** — the Codex
   two-axis property. (Slice 2: a sandbox-denied exit is fed back as a
   `RequireApproval` "re-run without sandbox?" rather than an opaque failure.
   Slice 1 keeps the denial visible in stderr, which the model already sees.)

Slice 1 (this change) delivers property (1) + (2) for **macOS** only, because
Seatbelt needs no new dependency (`/usr/bin/sandbox-exec` ships with macOS) and
is a pure string-generation + argv-wrap problem — the smallest correct,
independently-valuable cut. Linux (Landlock+seccomp+bubblewrap) and Windows
(restricted token / AppContainer) are later slices with real new deps.

### Module layout

New module in the existing leaf crate (natural home; `biorouter-mcp` already
depends on `biorouter-sandbox`, and OS-sandbox logic belongs with the other
sandbox backends, not scattered in the Developer server):

| File | Responsibility |
|------|----------------|
| `crates/biorouter-sandbox/src/seatbelt.rs` (new) | `SeatbeltPolicy` — writable roots + network flag → SBPL profile string + `sandbox-exec` argv wrap; `available()` host check. Pure + unit-tested; one live macOS enforcement test. |
| `crates/biorouter-sandbox/src/lib.rs` (change) | `pub mod seatbelt;` |
| `crates/biorouter-mcp/src/developer/shell.rs` (change) | `configure_shell_command` consults the gate and, when enabled on a capable host, rewrites the spawned program to `sandbox-exec … -- <shell>`. New pure helper `shell_sandbox_enabled(&str)`. |

### The Seatbelt profile (SBPL)

Modeled on Codex's `seatbelt_base_policy.sbpl` (itself derived from Chromium's
`common.sb`): `(deny default)`, allow read-only file ops everywhere, allow
`process-exec`/`process-fork`/`signal(self)` so children inherit the policy,
allow `/dev/null` writes, allow a bounded `sysctl-read` set that dyld/libSystem
need, and — the enforcement — a `file-write*` block whose subpaths are the
**parameterized** writable roots (`(subpath (param "WRITABLE_ROOT_0"))` …), with
paths passed as `-DWRITABLE_ROOT_n=…` CLI params so a path can never inject SBPL
syntax. **Network is denied by omission** (no `(allow network*)`) unless
`allow_network` is set, matching Codex's `network-outbound` deny.

If there are *zero* writable roots the write block is omitted entirely (an
`(allow file-write*)` with no filter would allow *all* writes — the opposite of
the intent).

### Key APIs

```rust
// crates/biorouter-sandbox/src/seatbelt.rs
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

pub struct SeatbeltPolicy {
    pub writable_roots: Vec<PathBuf>,
    pub allow_network: bool,
}
impl SeatbeltPolicy {
    pub fn new(writable_roots: Vec<PathBuf>) -> Self;   // network denied
    pub fn with_network(self, allow: bool) -> Self;
    pub fn profile(&self) -> String;                    // the SBPL text
    /// (program, wrapper_args) — wrapper_args ends `… -- <program>`, ready to
    /// have the program's own args appended by the caller.
    pub fn wrap(&self, program: &str) -> (String, Vec<String>);
}
/// macOS + `/usr/bin/sandbox-exec` present.
pub fn available() -> bool;
```

### Control flow: shell spawn, revised

```text
execute_shell_command  (rmcp_developer.rs:1283)
  └─ configure_shell_command(shell_config, command, working_dir)   [shell.rs:109]
       gate = BIOROUTER_SHELL_SANDBOX   (default off)
       if shell_sandbox_enabled(gate) && seatbelt::available():
           roots  = [working_dir?, env::temp_dir()]
           policy = SeatbeltPolicy::new(roots)
                        .with_network(BIOROUTER_SHELL_SANDBOX_NETWORK truthy)
           (program, prefix) = policy.wrap(&shell_config.executable)
           // sandbox-exec -p PROFILE -DWRITABLE_ROOT_0=… -- <shell>
       else:
           (program, prefix) = (shell_config.executable, [])
       Command::new(program)
           .current_dir(working_dir).<pipes/env/kill_on_drop/process_group>
           .args(prefix).args(shell_config.args).arg(command)
```

Everything downstream (env injection, PATH extension, process-group kill,
cancellation) is unchanged: `sandbox-exec` execs the shell in the same process
group, so `kill_process_group` still reaps the whole tree.

---

## Alternatives considered

- **Wrap in the `biorouter-sandbox` `SandboxClient` trait / `LocalProcessSandbox`
  instead.** That trait is async and returns a captured `ExecOutput`; the
  developer shell tool needs *streaming* output, real-time cancellation, and a
  process group for kill — it drives `tokio::process::Command` directly. Routing
  it through the trait would be a large refactor of `execute_shell_command` for
  no isolation gain (LocalProcessSandbox has *no* isolation by its own
  docstring). Wrapping the argv at the existing spawn site is the minimal
  correct change. The trait/Docker backend remains the path for the *app*
  compute capability.
- **Docker/VM (OpenHands model) for the desktop shell.** Heavyweight; requires a
  running Docker daemon most desktop users don't have; breaks the "run a command
  in my repo" expectation. Codex's per-command OS sandbox is the right fit for a
  local research tool. Docker stays available via the existing `DockerSandbox`
  for app compute.
- **On by default.** Rejected for Slice 1: Seatbelt denials break legitimate
  workflows (a build that writes to `~/.cache`, a tool that needs network) and
  the graceful escalation (property 3) isn't wired yet, so an on-by-default
  profile would surface as opaque failures. Ship off, prove enforcement, then
  wire escalation and flip the default per-mode.
- **Bake writable paths into the profile string.** Rejected: a path with SBPL
  metacharacters could break or subvert the profile. `sandbox-exec -D` params
  (Codex's approach) keep the profile static and the paths data.
- **Do it in `validate_shell_command` (`rmcp_developer.rs:1240`).** That only
  covers the built-in shell and is a *pre-flight text check*, not runtime
  containment. The spawn site is where enforcement must live.

---

## Migration and compatibility

- **Default off.** With `BIOROUTER_SHELL_SANDBOX` unset, `configure_shell_command`
  is byte-for-byte the old behavior (same program, same args). Zero migration,
  zero behavior change for existing users.
- **Opt-in values.** `BIOROUTER_SHELL_SANDBOX` ∈ `{1,true,on,seatbelt}` enables
  it; anything else (incl. unset) is off. `BIOROUTER_SHELL_SANDBOX_NETWORK`
  truthy re-allows outbound network inside the sandbox (for workflows that must
  fetch). On non-macOS hosts the gate is a silent no-op (Slice 1 has no Linux
  backend), so the same env can be set fleet-wide without breaking Linux.
- **Graceful unavailability.** If the flag is on but `sandbox-exec` is missing,
  we log once and run **unsandboxed** (fail-open) rather than refuse to run any
  command — Slice 1 is opt-in hardening, not a hard gate. (Slice 2's escalation
  can make this fail-closed-to-approval.)
- **No persisted state; no config-file schema change** (env only, matching
  BR-21's `SECURITY_COMMAND_POLICY` and the `SECURITY_PROMPT_*` precedent). A
  typed `Settings`/`config.yaml` field + `BioRouterMode` coupling is a later
  slice.

---

## Test plan

Unit (`cargo test -p biorouter-sandbox seatbelt`):
- `profile()` contains `(version 1)`, `(deny default)`, `(allow file-read*)`.
- Network **denied** by default → no `(allow network*)`; `with_network(true)` →
  contains it.
- One writable root → profile references `WRITABLE_ROOT_0` and `wrap()` emits
  `-DWRITABLE_ROOT_0=<path>`; two roots → `_0` and `_1`.
- **Zero roots → no `(allow file-write*` block** (no unrestricted-write escape).
- `wrap()` returns `SANDBOX_EXEC` as program and args end with `--`, `<program>`.

Live macOS enforcement (`#[cfg(target_os = "macos")]`, guarded by `available()`):
- A write *inside* the writable root succeeds; a write *outside* it (e.g. under
  `$HOME`) exits non-zero — proves kernel enforcement, the whole point of BR-64.
- A plain `echo`/read command still runs 0 under the profile (no false break).

Shell integration (`cargo test -p biorouter-mcp developer::shell` /
`shell_sandbox`):
- `shell_sandbox_enabled` accepts `1/true/on/seatbelt` (case-insensitive,
  trimmed) and rejects `""/0/off/no`.
- With the gate unset, `configure_shell_command`'s program is the shell (not
  `sandbox-exec`).

---

## Phasing

> **Note.** Slice 1 shipped as written. Slices 2–4 were **not** executed as described:
> BR-69 replaced the single-backend model with a `ShellSandbox` trait and its own phasing.
> Treat everything below Slice 1 as the original intent, not as the current plan.

- **Slice 1 (this change, S/M). Shipped.** macOS Seatbelt module + shell-spawn wrap,
  off-by-default env gate, unit + live-enforcement tests. Independently valuable:
  a macOS user (or a UCSF fleet via env) gets kernel-enforced write-confinement +
  network-deny on the shell tool today.
- **Slice 2 (M):** escalate-to-approval on a sandbox-denied exit (the Codex
  two-axis property) — a denial becomes a `RequireApproval("re-run without
  sandbox?")` inspection result instead of an opaque failure; a `SandboxSpec`
  derived from `BioRouterMode`.
- **Slice 3 (L). Superseded by BR-69.** Linux `codex-linux-sandbox`-style backend (Landlock + seccomp
  + bubblewrap) behind the same gate; wire the Computer-Controller script exec
  and third-party MCP spawn paths.
- **Slice 4 (M):** typed `Settings`/`config.yaml` surface + GUI toggle +
  managed-tier ([BR-65](../../../agent-loop/designs/managed-policy-tier.md)) pin so an admin can *force* the sandbox on.

## Open questions, with the recommendation that was taken

Each of these was recorded with a recommendation rather than left blocking, and the
recommendation is what shipped in Slice 1.

1. **Env var vs `config.yaml` for Slice 1.** *Recommendation taken:* env var
   (`BIOROUTER_SHELL_SANDBOX`), matching BR-21's `SECURITY_COMMAND_POLICY` and
   the `SECURITY_PROMPT_*` precedent; a typed config field is Slice 4 so it can
   land with the managed-tier pin and the mode coupling in one coherent surface.
2. **Fail-open vs fail-closed when `sandbox-exec` is absent but the flag is on.**
   *Recommendation taken:* fail-open (run unsandboxed, log once) for Slice 1
   because escalation isn't wired yet and Slice 1 is opt-in hardening; Slice 2's
   approval escalation flips this to fail-closed-to-approval.
3. **Default writable roots.** *Recommendation taken:* session working dir + the
   process temp dir only (Codex's `workspace-write` shape). `~/.cache`,
   `~/.config` etc. are deliberately *not* writable by default; a workflow that
   needs them opts out via the flag until Slice 2 adds per-root config.

---

## Related documentation

- [Linux and Windows sandboxing (BR-69)](../../../agent-loop/designs/linux-and-windows-sandboxing.md) — the successor design that supersedes this one's cross-platform phasing.
- [Platform parity audit](platform-parity-audit.md) — GAP-4, which records that this design ships on one of three platforms.
- [Wave 1 security report](../wave-reports/wave-1-security.md) — the implementation record for Slice 1.
- [Command policy engine (BR-21)](../../../agent-loop/designs/command-policy-engine.md) — the auditable allow/ask/deny catalog this containment layer complements.
- [Safety and guardrails comparison](../../agent-loop-review/competitive-comparison/safety-and-guardrails.md) — the table showing BioRouter had zero OS isolation before this work.
