# OS-level tool sandboxing on Linux and Windows (BR-69)

> **What this is.** The design generalizing the macOS-only Seatbelt sandbox into one
> `ShellSandbox` trait with three backends — Landlock + seccomp on Linux with a bubblewrap
> fallback, and an honest "no containment" tier on Windows — plus the capability reporting
> that tells a user which tier they actually got.
> **Status:** Current, partly implemented. Slices 0, 1, 2 and 4 shipped as commit
> `2d16ff0a` in Wave 3: `crates/biorouter-sandbox/src/shell_sandbox/{mod,macos,linux,windows}.rs`
> exist and `biorouter doctor` reports the tier. Slice 3 (real Windows containment) and the
> CI enforcement section remain the plan for that work — the campaign's outcome report
> calls the Windows tier the weakest part of the sandbox story, and the Linux and Windows
> arms were never compiled on the dev host. See the per-slice status in
> [Effort and phasing](#effort-and-phasing).
> **Audience:** developers working on tool sandboxing and shell execution.

BioRouter ships on macOS, Windows and Linux, but before this work its only OS-level
sandbox for the shell tool was macOS Seatbelt, and on the other two platforms the gate
was a no-op that logged a warning and ran the command anyway. This design replaces the
mechanism-named call site with a capability-named one, adds a real Linux backend, states
plainly what Windows can and cannot enforce, and makes the resulting tier visible to the
user instead of buried in a daemon log.

> **Identifier key.** *BR-NN* is a proposal from the agent-loop review's master list,
> defined in [improvement proposals](../../history/agent-loop-review/improvement-proposals.md);
> this design is BR-69. *Lens* records which review raised it — **R** = robustness, and
> *P-32* is that proposal's number within the robustness lens review.

> **Note — what this extends.** BR-64 (macOS Seatbelt, Slice 1 shipped; designed in
> [macOS Seatbelt sandbox](../../history/agent-loop-campaign/cross-platform/macos-seatbelt-sandbox.md)) — this is BR-64's Slice 3
> generalized. It complements BR-20 (catastrophic denylist) and BR-21 (policy engine),
> which this design shows are POSIX-only and therefore near-vacuous on Windows.

> **Overlap to be aware of.** The Problem section's account of the POSIX-only command
> safety net restates the premise of
> [Cross-platform command safety (BR-68)](../../history/agent-loop-campaign/cross-platform/command-safety.md), and its Slice 0 overlaps
> that design's rule work. BR-68 is the authority on the rules; this document is the
> authority on containment. The underlying findings (GAP-1, GAP-4) come from the
> [platform parity audit](../../history/agent-loop-campaign/cross-platform/platform-parity-audit.md).

## Contents

- [The problem, grounded in code](#the-problem-grounded-in-code)
- [Design](#design) — Slice 0 rules, Slice 1 the trait, Slice 2 Linux, Slice 3 Windows, Slice 4 reporting
- [Alternatives considered](#alternatives-considered)
- [Migration and compatibility](#migration-and-compatibility)
- [Test plan](#test-plan)
- [Effort and phasing](#effort-and-phasing)
- [Open questions](#open-questions-recommendation-taken-recorded-not-blocking)
- [Sources](#sources)
- [Related documentation](#related-documentation)

## The problem, grounded in code

BioRouter ships on macOS (dmg), Windows (zip) and Linux (deb/rpm). Its security
posture does not.

### The only OS sandbox is macOS-only, and it fails open everywhere else

`crates/biorouter-sandbox/src/seatbelt.rs:168`:

```rust
pub fn available() -> bool {
    cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).exists()
}
```

The single call site, `shell_sandbox_wrap` in
`crates/biorouter-mcp/src/developer/shell.rs:125-155`, hard-codes that one
backend:

```rust
if !biorouter_sandbox::seatbelt::available() {
    tracing::warn!(
        "BIOROUTER_SHELL_SANDBOX is set but no OS sandbox is available on this host; \
         running the shell tool unsandboxed"
    );
    return None;          // ← fail-open on Linux and Windows, always
}
```

So on Linux and Windows the gate is a **no-op that logs a warning**. A Linux or
Windows user who deliberately sets `BIOROUTER_SHELL_SANDBOX=1` — a UCSF fleet
admin pushing it via env, say — gets **zero** containment and a `warn!` line
buried in the daemon log they never read. BR-64's own design acknowledges this
("On non-macOS hosts the gate is a silent no-op", `BR-64-design.md:203`, now
[macOS Seatbelt sandbox](../../history/agent-loop-campaign/cross-platform/macos-seatbelt-sandbox.md)) and
defers it to "Slice 3 (L)". This is that slice.

The abstraction is also wrong-shaped: `shell.rs` names the *mechanism*
(`seatbelt::available()`, `SeatbeltPolicy`), not the *capability*. Adding Linux
today means a second `#[cfg]` ladder at the call site, and Windows a third.

### The command safety net (BR-20/BR-21) is POSIX-only — verified

This is the finding that matters most, and it is worse than "the sandbox is
missing." Every rule in **both** layers matches Unix syntax exclusively.

> **Note.** [Cross-platform command safety (BR-68)](../../history/agent-loop-campaign/cross-platform/command-safety.md) works this same
> finding in far more depth and is the design that fixed it. The summary here exists so
> this document's Slice 0 argument stands on its own.

`crates/biorouter/src/security/patterns.rs:463` — the always-on, non-bypassable
BR-20 `CATASTROPHIC_RULES`, all eight of them:

| Rule | Matches | Windows equivalent it does **not** match |
|------|---------|------------------------------------------|
| `rm_rf_root` | `rm -rf /` | `Remove-Item -Recurse -Force C:\`, `del /f /s /q C:\*`, `rd /s /q C:\` |
| `mkfs_device` | `mkfs.ext4 /dev/sda1` | `format C: /q`, `Format-Volume` |
| `dd_raw_disk` | `dd of=/dev/sda` | `diskpart` → `clean`, `Clear-Disk` |
| `fork_bomb` | `:(){ :\|:& };:` | `while($true){Start-Process powershell}` |
| `chmod_777_root` | `chmod -R 777 /` | `icacls C:\ /grant Everyone:F /T` |
| `curl_pipe_root_shell` | `curl … \| sudo sh` | `iwr … \| iex`, `Invoke-Expression (New-Object Net.WebClient).DownloadString(…)` |
| `system_power_off` | `shutdown`/`reboot`/`halt` | `Stop-Computer`, `shutdown /s /t 0` (`shutdown` collides by name only) |
| `git_push_force_protected` | (git — portable) | — |

And the BR-21 policy engine's embedded baseline
(`crates/biorouter/src/security/policy/baseline.policy.yaml`) is the same story —
its `path_glob`s are literally `"/etc/**"`, `"/usr/**"`, `"/dev/**"`, and its
`binary:` lists are `[rm]`, `[dd]`, `[wipefs]`, `[chmod]`, `[chown]`,
`[shutdown, reboot, halt, poweroff]`. No `path_glob` can ever match `C:\Windows`;
no `binary` list contains a PowerShell cmdlet.

Confirmed by grep — **zero** hits across `crates/biorouter/src/security/` for
`Remove-Item`, `del /`, `rd /s`, `format c:`, `diskpart`, `reg delete`,
`cipher /w`.

Meanwhile `crates/biorouter-mcp/src/developer/shell.rs:35-67`
(`detect_windows_shell`) correctly selects `pwsh` → `powershell` → `cmd.exe`. So
BioRouter **knowingly runs PowerShell on Windows and then inspects the command
with a regex set that only understands `sh`.** On Windows the agent's floor is
not "weak" — it is *absent*, in both layers, while the code reports "Allow" with
full confidence.

### BR-37's process-group kill is dual-armed, but the Windows arm is untested and structurally weaker

Verified in `crates/biorouter-mcp/src/developer/background.rs:343-360` and
`shell.rs:210-258`: both `kill_process_group` implementations do have
`#[cfg(unix)]` (`libc::kill(-pid, SIGTERM)` → sleep → `SIGKILL`) and
`#[cfg(windows)]` (`taskkill /F /T /PID`) arms. The dual-arming claim holds. Two
real caveats, though:

- **The Unix arm has a true process group to kill** — `configure_shell_command`
  calls `command_builder.process_group(0)` under `#[cfg(unix)]`
  (`shell.rs:198-201`). The Windows arm has **no equivalent**: no
  `CREATE_NEW_PROCESS_GROUP`, no Job Object. `taskkill /T` therefore walks the
  live *parent-PID chain*, which (a) misses grandchildren re-parented when an
  intermediate process exits, and (b) is vulnerable to PID reuse. A Job Object
  with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is the correct primitive and is
  *exactly* what the Windows sandbox tier below provides — so BR-69 fixes BR-37's
  Windows weakness as a side effect.
- **Every test in `background.rs` is `#[cfg(unix)]`** (lines 680, 694, 704, 753…).
  The Windows kill path and the orphan reaper have **no** coverage.

### Where each platform stands

| Platform | OS sandbox | Catastrophic denylist (BR-20) | Policy engine (BR-21) | Process-tree kill (BR-37) |
|---|---|---|---|---|
| macOS | Seatbelt (opt-in) | effective | effective | process group ✅ |
| Linux | **none** (warn + run) | effective | effective | process group ✅ |
| Windows | **none** (warn + run) | **vacuous** | **vacuous** | `taskkill /T`, untested |

---

## Design

Four pieces, in dependency order. **Slice 0 is the cheapest and the highest
value** — do not let the kernel work delay it.

### Slice 0 — Close the POSIX-only rule gap (no sandbox involved)

> **Shipped.** Delivered by BR-68 rather than by this item — see
> [Cross-platform command safety](../../history/agent-loop-campaign/cross-platform/command-safety.md), commit `651acff0`. The
> `platform:` field described below shipped as the richer `platforms` + `shells` pair.

Independent of everything below, and worth shipping alone. Both rule layers gain
Windows coverage, gated so they cost nothing on POSIX hosts:

- `patterns.rs`: add `CatastrophicRule`s for `remove_item_recurse_force`,
  `del_slash_s_root`, `rd_slash_s_root`, `format_volume`, `diskpart_clean`,
  `iwr_pipe_iex` / `downloadstring_iex`, `icacls_everyone_full_root`,
  `stop_computer`. Matchers are functions already (`matcher: is_rm_rf_root`), so
  this is additive — no engine change.
- `baseline.policy.yaml`: a `windows:` rule block with `binary: [Remove-Item, …]`
  and drive-letter `path_glob`s (`C:\\**`, `%SystemRoot%\\**`). This requires the
  policy engine's tokenizer/path-canonicalizer to learn Windows argv rules
  (backslash paths, `/f` style flags, case-insensitive binaries) — the one
  non-trivial bit. Rules keep their embedded `tests:` so
  `PolicyEngine::run_self_tests()` guards them in CI.
- **A rule set must declare which shell family it targets.** A `platform:
  posix|windows|any` field means the PowerShell rules never fire against a
  `bash` command string (and vice-versa), so we do not import false positives.

Rationale for going first: it is pure-Rust, cross-platform, testable on the mac
dev box, and it turns Windows' safety net from *absent* to *present*. The kernel
sandbox below is defense-in-depth on top of it — not a substitute.

### Slice 1 — One trait, three backends

> **Shipped** as commit `2d16ff0a`: `crates/biorouter-sandbox/src/shell_sandbox/{mod,macos,linux,windows}.rs`.

Generalize `SeatbeltPolicy` into a mechanism-neutral policy + a backend trait, in
`crates/biorouter-sandbox/`.

```rust
// crates/biorouter-sandbox/src/shell_sandbox/mod.rs  (new)

/// What the sandbox should permit. Mechanism-neutral; every backend maps this
/// onto its own primitives. This is the existing SeatbeltPolicy, renamed and
/// de-mechanised — same two fields, same defaults.
#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy {
    /// Directories the sandboxed process may write to (subpaths included).
    /// Everything else on the filesystem is readable but not writable.
    pub writable_roots: Vec<PathBuf>,
    /// When false (the default), outbound network is denied.
    pub allow_network: bool,
}

/// Which enforcement mechanism actually took effect. Reported to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxTier {
    /// Kernel-enforced write confinement AND network deny.
    Full,
    /// Kernel-enforced write confinement; network deny NOT enforced.
    /// (e.g. Landlock present but seccomp blocked by a restrictive container.)
    WriteOnly,
    /// Process containment + resource caps only. No write confinement,
    /// no network deny. Honest name for the unprivileged Windows tier.
    ContainmentOnly,
    /// Nothing. The command runs with the user's full authority.
    None,
}

/// A human-readable, user-facing capability report. `mechanism` is the concrete
/// backend ("seatbelt", "landlock+seccomp", "bubblewrap", "job-object");
/// `degradations` explains every capability we asked for and did not get.
#[derive(Debug, Clone)]
pub struct SandboxReport {
    pub tier: SandboxTier,
    pub mechanism: &'static str,
    pub degradations: Vec<String>,
}

/// The argv rewrite a backend performs. Deliberately identical in shape to
/// today's `SeatbeltPolicy::wrap` -> (program, prefix_args), because ALL THREE
/// backends can be expressed as "replace the program with a wrapper that
/// re-execs it" (see "Why a helper binary" below). The caller appends the
/// program's own args after `prefix_args`, exactly as it does today.
pub struct Wrapped {
    pub program: String,
    pub prefix_args: Vec<String>,
}

pub trait ShellSandbox: Send + Sync {
    /// Probe this host. MUST be a real capability probe, never a version guess.
    fn probe(&self) -> SandboxReport;

    /// Rewrite `program` so it runs under `policy`. Only called when
    /// `probe().tier != SandboxTier::None`.
    fn wrap(&self, policy: &SandboxPolicy, program: &str) -> Result<Wrapped, SandboxError>;
}

/// The one entry point the rest of the codebase uses. Returns the best backend
/// compiled in for this target, or a `NullSandbox` whose probe() reports
/// `tier: None`. There is exactly one `#[cfg]` ladder in the tree, and it is here.
pub fn detect() -> Box<dyn ShellSandbox>;
```

`shell.rs` then calls **one** API and never names a mechanism:

```rust
fn shell_sandbox_wrap(program: &str, working_dir: Option<&Path>)
    -> Option<(String, Vec<String>)>
{
    let mode = SandboxMode::from_env();            // off | auto | strict
    if mode == SandboxMode::Off { return None; }

    let backend = biorouter_sandbox::shell_sandbox::detect();
    let report  = backend.probe();

    if report.tier == SandboxTier::None {
        return mode.on_no_sandbox(&report);        // auto -> warn+None; strict -> Err
    }
    // ... build roots (working_dir + temp_dir), same as today ...
    let w = backend.wrap(&policy, program).ok()?;
    Some((w.program, w.prefix_args))
}
```

#### How `seatbelt.rs` refactors with **zero** macOS behavior change

The refactor is mechanical and provably behavior-preserving:

1. `SeatbeltPolicy` keeps its two fields, `new()`, `with_network()`, `profile()`
   and `wrap()` **verbatim** — the SBPL text, the `-DWRITABLE_ROOT_n=` params, the
   `--` separator, the `canonicalize()` fallback, all untouched. All five existing
   unit tests and both live-enforcement tests compile and pass unchanged.
2. A thin adapter is added *beside* it:

```rust
pub struct SeatbeltSandbox;

impl ShellSandbox for SeatbeltSandbox {
    fn probe(&self) -> SandboxReport {
        if seatbelt::available() {
            SandboxReport { tier: SandboxTier::Full, mechanism: "seatbelt", degradations: vec![] }
        } else {
            SandboxReport { tier: SandboxTier::None, mechanism: "seatbelt", degradations: vec![] }
        }
    }
    fn wrap(&self, policy: &SandboxPolicy, program: &str) -> Result<Wrapped, SandboxError> {
        // SandboxPolicy IS SeatbeltPolicy's two fields. Same call, same argv.
        let p = SeatbeltPolicy::new(policy.writable_roots.clone())
            .with_network(policy.allow_network);
        let (program, prefix_args) = p.wrap(program);
        Ok(Wrapped { program, prefix_args })
    }
}
```

3. `detect()` returns `Box::new(SeatbeltSandbox)` under `cfg(target_os="macos")`.

The argv handed to `tokio::process::Command` on macOS is **byte-for-byte what it
is today**. A regression test asserts exactly that (see Test plan).

### Slice 2 — Linux: Landlock (+ seccomp), bubblewrap fallback

> **Shipped** as commit `2d16ff0a` (`shell_sandbox/linux.rs`), but **not verified**: the
> Linux arm was never compiled on the macOS dev host, only type-checked by flipping `cfg`
> gates. See the [verification report](../../history/agent-loop-campaign/cross-platform/parity-verification-report.md).

This is the platform where a real, unprivileged kernel sandbox exists.

#### Why a helper binary (and not `pre_exec`)

Seatbelt is an *external wrapper program*. Landlock and seccomp are **in-process
syscalls that must run in the child, after `fork`, before `exec`.** The two
options:

- `CommandExt::pre_exec` — runs the closure in the forked child. It is `unsafe`
  and must be **async-signal-safe**: in a child forked from a multi-threaded
  process (which `biorouterd` emphatically is — it is a Tokio runtime), any
  `malloc` can deadlock on an allocator lock a *different* thread held at fork
  time. `rust-landlock`'s ruleset construction allocates and opens paths. This is
  a real, intermittent, un-debuggable hang.
- **Re-exec ourselves as a helper** — this is what OpenAI's `codex-linux-sandbox`
  does, and it is what we should do. `wrap()` returns
  `(current_exe(), ["__br-sandbox", "--writable-root", "/w", "--deny-net", "--", "/bin/bash"])`.
  The helper process (a fresh, single-threaded `main`) applies
  `PR_SET_NO_NEW_PRIVS` → Landlock ruleset → seccomp filter to *itself*, then
  `execve`s the program. No fork-safety hazard, and — critically — **it collapses
  to the exact same `(program, prefix_args)` shape as Seatbelt**, so the trait
  above needs no `pre_exec` escape hatch and `configure_shell_command` needs no
  restructuring.

The helper entry point is a shared function called at the very top of `main()` in
both binaries:

```rust
// biorouter-cli/src/main.rs and biorouter-server/src/main.rs, first line of main:
biorouter_sandbox::shell_sandbox::linux::run_helper_if_invoked();  // never returns if it is the helper
```

It is a hidden argv marker (`__br-sandbox`), not a clap subcommand, so it never
appears in `--help` and cannot collide with a user command. `current_exe()` is
resolved once and cached; if it is unreadable (a deleted/replaced binary), the
backend reports `tier: None` rather than guessing.

#### Detection: probe the ABI, never `uname`

The single most common bug here is gating on kernel version. **Landlock being in
the kernel is not enough — it must also be enabled in the boot-time `lsm=` list.**
Debian/Ubuntu enable it; several enterprise distros (SELinux-first, e.g. RHEL/Rocky
9 depending on point release) do **not**, and WSL2 kernels historically did not.
A `kernel >= 5.13` check would confidently produce a sandbox that silently
enforces nothing.

The correct probe is the ABI-version syscall, which returns the supported ABI
level or fails with `ENOSYS`/`EOPNOTSUPP`:

```rust
landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)
// -> Ok(abi) | Err(ENOSYS)      kernel built without Landlock
//            | Err(EOPNOTSUPP)  present but not enabled in `lsm=`
```

`rust-landlock` exposes this via `ABI::new_current()` / the `CompatLevel`
machinery. We use `CompatLevel::BestEffort` for *rule* features (so an ABI-1
kernel still gets write confinement, just without the ABI-2+ refinements) but we
treat **write confinement itself as a hard requirement** — if the ruleset cannot
restrict `LANDLOCK_ACCESS_FS_WRITE_FILE` / `MAKE_*` / `REMOVE_*`, we do not claim
a tier.

ABI floor: **ABI 1 (kernel 5.13)** gives us filesystem write confinement, which is
the core guarantee. Later ABIs add refinements (ABI 2 = `REFER`/rename across
roots, ABI 3 = truncate, ABI 4 = TCP bind/connect restrictions) that we take
opportunistically via best-effort. Note ABI 4's network scoping only covers TCP
bind/connect — it does **not** replace seccomp (no UDP, no raw sockets), so
seccomp stays the network primitive.

#### Network deny: seccomp-bpf on `socket(2)` domain

seccomp cannot dereference pointers, so it cannot inspect a `sockaddr` passed to
`connect(2)`. The robust primitive is to filter `socket(2)` on its **scalar
`domain` argument**:

- Deny (`EPERM`, not `SIGSYS` — a clean `errno` lets programs print a sane error
  instead of dying on a signal): `AF_INET`, `AF_INET6`, `AF_PACKET`, `AF_NETLINK`
  (route/xfrm subsets), and `socketcall` on 32-bit arches.
- **Allow `AF_UNIX`.** Codex learned this the hard way: without it, ordinary
  shell machinery (D-Bus, X11, `getaddrinfo` via nscd, syslog) breaks and the
  sandbox looks like a BioRouter bug.
- Also deny `connect`/`sendto`/`sendmsg`/`bind`/`listen`/`accept*` as a
  belt-and-braces measure against an **inherited** AF_INET fd. In practice Rust
  sets `CLOEXEC` on its sockets, and the helper `execve`s, but `biorouterd` holds
  real network sockets and the cost of the extra rules is zero.

Crate choice: **`seccompiler`** (Firecracker's, pure Rust, Apache-2.0) over
`libseccomp-rs`. `libseccomp-rs` links the C `libseccomp` and needs
`libseccomp-dev` at build time — that would break the pinned
`rust:1.92-bullseye` cross-compile container the release pipeline depends on
(`LINUX_RUST_IMG` in `scripts/release.sh`). `seccompiler` and `rust-landlock` are
both pure-Rust syscall wrappers and cross-compile cleanly. Both go behind
`[target.'cfg(target_os = "linux")'.dependencies]` so the mac and Windows builds
never see them.

#### Degradation ladder on Linux

`probe()` walks it and reports honestly:

| Condition | Tier | `mechanism` | Reported degradation |
|---|---|---|---|
| Landlock ABI ≥ 1 **and** seccomp installable | `Full` | `landlock+seccomp` | — |
| Landlock ABI ≥ 1, seccomp refused (e.g. a container with `no-new-privs` already pinned oddly, or a `SCMP` blocked by an outer sandbox) | `WriteOnly` | `landlock` | "outbound network is NOT blocked" |
| No Landlock, but `bwrap` on `PATH` | `Full` | `bubblewrap` | "using bubblewrap namespaces (`--ro-bind / /` + `--bind` writable roots + `--unshare-net`)" |
| Neither | `None` | `none` | "kernel too old / Landlock not in `lsm=`, and bubblewrap not installed — install `bubblewrap` for a sandbox" |

Bubblewrap is a genuinely good fallback (`--ro-bind / /`, `--bind` each writable
root, `--unshare-user --unshare-pid`, `--unshare-net` when network is denied) and
— conveniently — is *also* a pure argv wrapper, so it drops straight into
`Wrapped` with no helper. We look it up on `PATH` and do **not** vendor it. We
never *require* it: a missing `bwrap` is a reported degradation, not an error.

> Deliberate non-goal: we do not adopt Codex's current bwrap-*first* ordering.
> Codex bundles bwrap; we do not, and requiring users to `apt install bubblewrap`
> for a sandbox that Landlock can provide with no dependency is the wrong default
> for a research tool. Landlock first, bwrap as fallback.

### Slice 3 — Windows: the honest tier

> **Still the plan.** `shell_sandbox/windows.rs` exists and reports an honest tier, but
> the campaign's [outcome report](../../history/agent-loop-campaign/outcome-report.md)
> calls the Windows tier the weakest part of the sandbox story, and the Windows arm was
> never compiled on the dev host. Treat the Job Object / restricted-token work below as
> outstanding.

**Say the hard thing plainly: there is no unprivileged, general-purpose,
kernel-enforced write-confinement sandbox on Windows that can wrap an arbitrary
developer shell command without breaking it.** Anyone who tells you otherwise is
selling an AppContainer that cannot run `git`. The options, with their real
limits:

| Mechanism | Write confinement | Network deny | Needs admin | Why it fails as a general shell wrapper |
|---|---|---|---|---|
| **Job Object** | ✗ | ✗ | no | Caps processes/memory/CPU and gives a real kill-the-tree primitive. Not an access-control mechanism at all. |
| **Restricted token + Low integrity level** | *partial* | ✗ | no | Low-IL blocks writes to medium-IL objects (most of the user's files) via the mandatory integrity policy. But it blocks *reads* of nothing and **cannot** stop a write to any object whose ACL already grants `Everyone` write. And to let the agent write to its own project dir you must `icacls /setintegritylevel Low` **the user's project directory** — a persistent, visible ACL mutation of *their* files. That is a hostile side effect for a research tool. |
| **AppContainer / LPAC** | ✓ (real) | ✓ (real — omit the `internetClient` capability and outbound network is denied *in the kernel*, the one unprivileged network-deny primitive Windows has) | no | The blast radius is the problem: an AppContainer process gets a fresh package SID with access to almost nothing. `git`, `node`, `python`, `cargo`, MSVC — all routinely break on registry, named-pipe, and `%LOCALAPPDATA%` denials. Granting each back means ACLing the user's toolchain with the package SID. |
| **Windows Firewall rules** | n/a | ✓ | **yes** | This is how Codex does Windows network-deny — it creates dedicated `CodexSandboxOffline`/`CodexSandboxOnline` **users** and firewall rules. Requires administrator. A UCSF-managed laptop user does not have it. |
| **Windows Sandbox (WDAG)** | ✓ | ✓ | Pro/Enterprise | A full VM. Seconds of startup per invocation, no shared filesystem by default. Unusable for per-command wrapping. |

Therefore the design ships **two Windows tiers and advertises them accurately**:

**Tier W1 — `ContainmentOnly` (default; ship this).**
Helper (`__br-sandbox`, same marker, Windows arm) creates:
- a **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, an active-process
  cap, a memory ceiling, and `JOB_OBJECT_UILIMIT_*` restrictions, then assigns
  the child to it;
- a **restricted token** (`CreateRestrictedToken` with `DISABLE_MAX_PRIVILEGE` +
  deny-only SIDs for the local Administrators group) and launches the shell with
  it, at Low integrity;
- **no** ACL mutation of the user's directories.

`probe()` reports `tier: ContainmentOnly, mechanism: "job-object"`, with
degradations `["writes are NOT confined", "outbound network is NOT blocked"]`.
It is not a sandbox and we will not call it one. What it genuinely buys:
resource caps, no privilege escalation, and — the BR-37 fix — a **real
kill-the-whole-tree primitive** that `taskkill /T` only approximates.

**Tier W2 — `Full` via AppContainer (opt-in, later, `BIOROUTER_SHELL_SANDBOX=appcontainer`).**
Real write confinement + real kernel network deny, at the cost of explicitly
ACL-granting the app-container SID onto the writable roots and (inevitably) onto
whichever toolchain paths the user's workflow needs. Correct for a locked-down
"run untrusted code" mode; wrong as a default. Ships only after the W1 plumbing
and the Slice 0 rules are in, and only behind an explicit opt-in with a
documented "this will break tools" warning.

**And the ordering that matters:** on Windows, **Slice 0 (the PowerShell rule
gap) is worth more than Slice 3.** A user is far likelier to be saved by
`Remove-Item -Recurse -Force C:\` being *denied* than by an AppContainer that
they turned off because `git` stopped working. Ship the rules first.

### Slice 4 — Capability reporting and config

> **Shipped** as commit `2d16ff0a`: `biorouter doctor` gained a Shell-sandbox section that
> reports the tier.

#### `BIOROUTER_SHELL_SANDBOX` grows a three-valued mode

```rust
pub enum SandboxMode { Off, Auto, Strict }
```

| Value | Meaning |
|---|---|
| unset, `0`, `off`, `no`, `false` | **Off.** Byte-for-byte today's unsandboxed path. Still the default. |
| `1`, `true`, `on`, `auto`, `seatbelt` | **Auto.** Use the best available tier. If the tier is `None` (or degraded), **warn once and run anyway** — today's fail-open. The legacy `seatbelt` value maps here, so existing configs keep working. |
| `strict` | **Strict.** Require `tier == Full`. If the host cannot provide it, **refuse to run the command**: the shell tool returns a tool *error* whose text names the missing capability and how to fix it ("Landlock is not enabled in this kernel's `lsm=` list; install `bubblewrap` or set `BIOROUTER_SHELL_SANDBOX=auto`"). |

**What `strict` means when no sandbox exists is the crux, so state it flatly:
`strict` refuses to run.** A mode named "strict" that silently degrades to
"unsandboxed" is worse than no mode at all — it is a false assurance, and it is
precisely the failure BR-64's Linux/Windows no-op already commits. `strict` is
the mode a fleet admin (BR-65 managed tier) pins so that a Windows box, which
*cannot* reach `Full`, **fails loudly** instead of quietly running the agent with
the operator's full authority. `WriteOnly` does not satisfy `strict` either;
`BIOROUTER_SHELL_SANDBOX=strict-write` is the escape hatch if that proves too
sharp (see Open questions).

`BIOROUTER_SHELL_SANDBOX_NETWORK` is unchanged (truthy → `allow_network`), and
`SandboxPolicy.allow_network` is what every backend consumes.

#### Users must be able to tell which tier they got

Today the only signal is a `tracing::warn!` inside the daemon. Three surfaces:

1. **The tool result.** When the sandbox is on, the shell tool's response prepends
   one line to the *assistant-visible* text — `[sandbox: landlock+seccomp — writes
   confined to /work, /tmp; network denied]` or `[sandbox: none — command ran with
   full user authority]`. The model sees it, so it can reason about a denial
   instead of retrying blindly. It is also the thing a user screenshots in a bug
   report.
2. **A `sandbox_status` line in `biorouter doctor`** (and a chip in the GUI's
   settings/security panel), rendering the `SandboxReport` verbatim: tier,
   mechanism, and every degradation string.
3. **One `tracing::info!` at startup**, not a `warn!` buried at first use.

The `degradations: Vec<String>` field exists precisely so these surfaces never
have to *infer* what was lost from a tier enum.

---

## Alternatives considered

- **Keep `#[cfg]`-laddering inside `shell.rs`.** Rejected. It already reads
  `seatbelt::available()` by name; adding two more mechanisms puts three kernel
  APIs and two helper protocols into an MCP tool's spawn function. `detect()`
  keeps exactly one `#[cfg]` ladder, in the sandbox crate, where the tests live.
- **Apply Landlock/seccomp via `CommandExt::pre_exec` instead of a helper
  binary.** Rejected — the fork-in-a-multithreaded-Tokio-process allocator
  deadlock is real and intermittent, and (secondarily) `pre_exec` would force the
  trait to carry a non-argv escape hatch, breaking the clean
  `(program, prefix_args)` shape that Seatbelt and bubblewrap already fit. The
  helper re-exec is what Codex settled on for the same reasons.
- **Gate Landlock on kernel version (`uname` ≥ 5.13).** Rejected, and worth
  calling out because it is the obvious wrong answer: Landlock must also be listed
  in the boot-time `lsm=` parameter, which several enterprise distros and older
  WSL2 kernels do not do. A version check would report `Full` on a host enforcing
  nothing — the worst possible outcome for a security feature. Probe the ABI
  syscall.
- **Bubblewrap first (Codex's current ordering), or vendor `bwrap`.** Rejected as
  the default. Codex bundles the binary; we would have to ship it in the deb/rpm
  and the zip, and it needs user namespaces (which some hardened kernels and most
  container-in-container CI disable). Landlock needs no dependency at all. bwrap
  stays as the fallback for pre-5.13 / `lsm=`-less hosts.
- **`libseccomp-rs` over `seccompiler`.** Rejected: it links C `libseccomp`, which
  would add a `libseccomp-dev` build dep to the pinned `rust:1.92-bullseye` cross
  container and a runtime `.so` dep to the deb/rpm. `seccompiler` is pure Rust.
- **Docker/container-per-command (OpenHands model) as the Linux backend.**
  Rejected for the same reason BR-64 rejected it: it requires a daemon most
  desktop/HPC users don't have, and it breaks "run this in my repo." `DockerSandbox`
  already exists in this crate for the *app compute* capability; that is its right
  scope.
- **Ship the Windows Job Object tier as "the Windows sandbox."** Rejected —
  emphatically. It confines nothing. Calling it a sandbox would give a Windows
  user a green checkmark for a protection they do not have, which is worse than
  the honest red X they have today. It reports `ContainmentOnly`.
- **Claim Windows parity by requiring admin (Codex's sandbox-user + firewall
  model).** Rejected for the default path: UCSF-managed laptops do not grant
  local admin. It could return as an opt-in enterprise tier if a site *does* have
  admin.

---

## Migration and compatibility

- **Default is still off.** `BIOROUTER_SHELL_SANDBOX` unset ⇒ `configure_shell_command`
  produces the identical program and argv it does today, on all three platforms.
  Zero migration, zero behavior change.
- **macOS is byte-for-byte unchanged.** `SeatbeltPolicy`, its SBPL text, its
  `-D` params and its `--` separator are untouched; `SeatbeltSandbox` is a thin
  adapter over the same `wrap()`. A golden-argv regression test pins this
  (below). Existing `BIOROUTER_SHELL_SANDBOX=seatbelt` configs keep working —
  the value maps to `Auto`.
- **Linux/Windows users who already set the flag** go from "silently no-op" to
  "actually sandboxed" — a behavior change, but strictly in the direction they
  asked for, and `auto` still fails open on hosts that cannot enforce. Called out
  in release notes.
- **New deps are target-gated**: `landlock` + `seccompiler` only under
  `cfg(target_os = "linux")`, `windows` crate only under `cfg(windows)`. The macOS
  build's dependency graph does not change. Both Linux crates are pure Rust, so
  the pinned `rust:1.92-bullseye` glibc-2.31 cross-build and the `cli-linux`
  smoke test are unaffected.
- **The helper marker (`__br-sandbox`) must be wired into both `main()`s**
  (`biorouter` and `biorouterd`) before Slice 2 is enabled; if the marker is
  absent from the running binary, `probe()` detects that (it self-tests by
  invoking `current_exe() __br-sandbox --selftest`, cached once per process) and
  reports `tier: None` rather than producing a wrapper that would exec into a
  normal CLI startup. This is the one genuinely dangerous failure mode in the
  design and it gets an explicit runtime check.
- **No config-file schema change** in this slice (env only, matching BR-21's
  `SECURITY_COMMAND_POLICY` precedent). The typed `Settings` field + GUI toggle +
  BR-65 managed pin land together, as BR-64 Slice 4 already planned.
- **Slice 0's new rules can produce new denials** on Windows for commands that
  previously ran. That is the point, but it means the Windows rule set ships with
  the same embedded-`tests:` discipline and a `not_matches:` list padded with
  ordinary developer commands (`Remove-Item .\build -Recurse`, `del /q *.obj`) to
  keep false positives out.

---

## Test plan

### Pure / cross-platform (runs on the mac dev box, every CI job)

`cargo test -p biorouter-sandbox shell_sandbox`:
- `SandboxPolicy` defaults: `allow_network == false`; zero writable roots is
  representable and every backend must treat it as "no writes anywhere" (the
  BR-64 `zero_roots_emits_no_unrestricted_write_block` invariant, generalized —
  each backend gets this test).
- `SandboxMode::from_env` parses `off/auto/strict` plus every legacy truthy value
  (`1/true/on/seatbelt`), case-insensitive and trimmed; unknown values are `Off`,
  never a silent `Auto`.
- **Golden-argv macOS regression:** with a fixed policy, `SeatbeltSandbox::wrap`'s
  `(program, prefix_args)` equals `SeatbeltPolicy::wrap`'s output *exactly*. This
  is the test that guarantees "no macOS behavior change" and it runs on every
  platform (pure string construction).
- `NullSandbox::probe()` reports `tier: None` with a non-empty, actionable
  `degradations` entry.

`cargo test -p biorouter security::policy` / `security::patterns` (Slice 0):
- Every new Windows catastrophic rule has `matches:`/`not_matches:` embedded, and
  `PolicyEngine::run_self_tests()` enforces them at load — the existing mechanism,
  so a broken Windows rule fails CI on a mac.
- Cross-family isolation: a PowerShell rule must **not** fire on
  `rm -rf ./build`, and a POSIX rule must not fire on `Remove-Item .\build`.

### Real enforcement, per OS

The pattern (already established by BR-64's `seatbelt_enforces_write_confinement`)
is: **spawn a real command under the real backend and assert the kernel stopped
it.** String assertions cannot validate a sandbox. Each test guards on
`probe().tier` and `eprintln!`-skips otherwise, so it is green-and-honest on a
host that cannot enforce.

**macOS** — the two existing BR-64 tests, unchanged, plus one new:
- write inside the writable root succeeds; write outside exits non-zero ✅ (exists)
- benign `echo`/read still runs ✅ (exists)
- **network deny** (new; BR-64 never tested it): `curl -m 2 https://example.com`
  under the profile exits non-zero, and does so *with* `allow_network` set to
  false only. Uses a loopback listener rather than the real internet so it is
  hermetic and offline-safe.

**Linux** (`#[cfg(target_os = "linux")]`, gated on `probe().tier`):
- **Write confinement:** `sh -c 'echo x > $OUTSIDE'` exits non-zero and the file
  does not exist; the same write to a writable root succeeds. Asserted separately
  for the `landlock` and the `bubblewrap` backends by forcing each
  (`BIOROUTER_SHELL_SANDBOX_BACKEND=landlock|bubblewrap`, a test-only override).
- **Network deny:** a test binds a loopback TCP listener in the parent, then the
  sandboxed child attempts to connect to it. Under `Full`, `connect` fails with
  `EPERM`. Under `allow_network = true`, it succeeds. Hermetic — no internet.
- **AF_UNIX still works:** the child successfully `socket(AF_UNIX)`s and connects
  to a temp-dir socket. This is the regression that catches an over-broad seccomp
  filter — the exact failure Codex hit — and without it a "passing" network test
  can ship a sandbox that breaks every shell on the box.
- **Degradation honesty:** with Landlock available but seccomp forced to fail
  (test override), `probe()` reports `WriteOnly` and a non-empty degradation, and
  the write test still passes while the network test is *expected* to fail-open.
- **Helper self-test:** `current_exe() __br-sandbox --selftest` exits 0 from both
  `biorouter` and `biorouterd`; a binary without the marker makes `probe()` return
  `None`.

**Windows** (`#[cfg(windows)]`):
- Job Object: a shell that spawns a detached grandchild is fully reaped when the
  job handle closes — **the BR-37 gap**. This is a real assertion `taskkill /T`
  would fail.
- Process/memory caps are enforced (a fork-bomb-ish loop is capped, not fatal to
  the host).
- `probe()` reports `ContainmentOnly` with *both* degradation strings present. A
  test asserts the Windows backend **never** reports `Full` in W1 — the guard
  against someone later "upgrading" the label without upgrading the mechanism.
- `strict` mode on Windows returns a tool **error**, not a silent run.

### CI (the dev box is a mac — this is the part that must not be hand-waved)

> **Outstanding.** This subsection is the part of the design that has not been carried
> out: the Linux and Windows arms were never compiled on the dev host, so the enforcement
> tests below have never actually run. The generic cross-compile gate that landed
> alongside is [BR-70](../../history/agent-loop-campaign/cross-platform/ci-gate.md); the sandbox-specific jobs described here are still to
> be built.

The Linux enforcement tests are worthless if they only ever skip. Three jobs:

1. **`linux-sandbox` job on `ubuntu-24.04`** (GitHub-hosted). Ubuntu ships
   Landlock enabled in `lsm=`, so `probe()` returns `Full` and the enforcement
   tests **actually run**. A guard step asserts the tier *before* the test job:
   `cargo run -q --bin biorouter -- doctor --sandbox` must print
   `tier=Full mechanism=landlock+seccomp`, and the job **fails** if it prints
   `None`. Without this guard a kernel/runner change would silently turn the whole
   suite into skips and nobody would notice — the single most likely way this
   feature rots.
2. **Docker matrix** for the degradation ladder, since one runner cannot exhibit
   all of them:
   - `debian:12` + `--security-opt seccomp=unconfined` (Landlock present) → `Full`.
   - a container run **without** Landlock exposure → asserts the `bwrap` fallback
     path (`apt install bubblewrap`) reaches `Full` via namespaces.
   - a container with neither → asserts `tier: None` **and** that `auto` warns and
     runs while `strict` refuses. This is the "no sandbox here" contract, tested.
   Note the containers must run with enough privilege for user namespaces
   (bwrap) — `--privileged` is *not* needed for Landlock, which is the point.
3. **`windows-latest` job** for the Job Object / restricted-token tests and the
   `never reports Full` assertion.

Cross-compile guard: the existing `cli-linux` clean-Debian/Rocky smoke test
already catches a glibc regression; add a line to it that runs
`biorouter doctor --sandbox` inside the clean container, which is exactly where a
`libseccomp`-style runtime `.so` dep would blow up. (It won't, because
`seccompiler` is pure Rust — but that is a claim worth having a test for, since
Rocky 9 is precisely the distro where Landlock may be absent *and* the runtime
deps matter.)

---

## Effort and phasing

The `Status` column records where each slice stands today; the rest of the table is the
design's original estimate.

| Slice | Scope | Size | Independently valuable? | Status |
|---|---|---|---|---|
| **0** | Windows/PowerShell rules in `patterns.rs` + `baseline.policy.yaml`; `platform:` field on rules; tokenizer learns Windows argv | **M** | **Yes — biggest single win.** Turns Windows' safety net from *absent* to *present*, with no kernel work and no new deps. Ship first, alone if need be. | Shipped, via [BR-68](../../history/agent-loop-campaign/cross-platform/command-safety.md) (`651acff0`) |
| **1** | `ShellSandbox` trait + `SandboxPolicy`/`SandboxTier`/`SandboxReport`; `detect()`; `SeatbeltSandbox` adapter; `shell.rs` calls one API; `SandboxMode` (off/auto/strict); tier reporting in tool result + `doctor` | **M** | Yes — macOS users get `strict` and a visible tier; the codebase gets the seam. No behavior change. | Shipped (`2d16ff0a`) |
| **2** | Linux: `__br-sandbox` helper in both `main()`s; `landlock` + `seccompiler`; ABI probe; bubblewrap fallback; the enforcement + AF_UNIX + degradation tests; the CI matrix | **L** | Yes — Linux (deb/rpm, and the UCSF HPC/Linux users) gets a real kernel sandbox for the first time. | Shipped (`2d16ff0a`), not compiled or enforcement-tested on the dev host |
| **3** | Windows W1: Job Object + restricted token + Low IL via the same helper; honest `ContainmentOnly` reporting; fixes BR-37's Windows kill gap | **M** | Yes — real process-tree kill + resource caps, honestly labelled. | Outstanding — the weakest tier per the campaign outcome report |
| **4** | Windows W2 (AppContainer, opt-in); typed `Settings`/`config.yaml` + GUI toggle; BR-65 managed-tier pin of `strict`; BR-64 Slice 2's escalate-to-approval-on-denial wired to `SandboxReport` | **L** | Yes — completes the two-axis model. | Tier reporting shipped in `biorouter doctor` (`2d16ff0a`); AppContainer and the config surface outstanding |

Slices 0 and 1 are independent and can run in parallel. 2 and 3 both depend on 1
(the trait) and are independent of each other. 4 depends on all.

---

## Open questions (recommendation taken; recorded, not blocking)

> **Note.** These were recorded at design time, each with the author's recommendation.
> Most of the design has since shipped without them being resolved in this document, so
> check the code before assuming any recommendation below was adopted.

1. **Does `strict` accept `WriteOnly`?** *Recommendation taken:* **no** — `strict`
   requires `Full`. A mode whose entire purpose is "do not run unless contained"
   should not accept a tier that leaves the network open, because network egress
   is the exfiltration channel that matters for prompt injection. If real fleets
   find this too sharp (a hardened container that blocks our seccomp install would
   make every command fail), add `strict-write` rather than weakening `strict`.
2. **Should the sandbox default flip to `auto` once Linux lands?** *Recommendation:*
   not in this change. BR-64 deliberately shipped off-by-default because denials
   break legitimate builds (`~/.cache`, `~/.cargo`) and the escalate-to-approval
   path (BR-64 Slice 2) is not wired. Flip the default only after Slice 4 turns a
   denial into an approval prompt instead of an opaque failure. Until then, `auto`
   opt-in.
3. **Default writable roots — still just `[working_dir, temp_dir]`?**
   *Recommendation taken:* yes, unchanged from BR-64, for cross-platform
   consistency. But Linux will surface the pain faster than macOS did (`~/.cargo`,
   `~/.cache`, `~/.npm`, and on HPC the module/conda prefix). Slice 4's config
   surface should ship `BIOROUTER_SHELL_SANDBOX_WRITABLE_ROOTS` (colon/semicolon
   separated) at the same time, or the first Linux user with a Rust build will
   just turn the sandbox off — the worst outcome.
4. **Read confinement (not just write)?** *Recommendation:* out of scope, matching
   Codex and BR-64 — full-filesystem *read* is allowed. Note honestly that this
   means the sandbox does **not** stop the agent reading `~/.ssh/id_rsa`; it stops
   it *exfiltrating* the key (network deny) and *destroying* your files (write
   confinement). If read confinement is wanted, Landlock supports it natively and
   Seatbelt does too — it is a policy change, not an architecture change — but it
   breaks vastly more tools than write confinement does.
5. **Who else spawns processes?** The legacy Computer Controller server
   (`computercontroller/platform/{macos,windows,linux}.rs`, since removed when
   Biorouter Copilot replaced the legacy computer tools) and third-party MCP
   extension spawns are **not** covered by this design, exactly as they were not
   covered by BR-64. Once the trait exists (Slice 1), routing them through it is
   mechanical, and the legacy Computer Controller script-exec path was the
   obvious next consumer. Worth a follow-up ticket rather than expanding this one.
6. **Does the helper re-exec interact with `process_group(0)` / `kill_on_drop`?**
   Believed no — the helper `execve`s in place, so the pid the parent holds *is*
   the shell's pid and it is already in the new process group set by
   `configure_shell_command`. This should be asserted by a test in Slice 2
   (kill a sandboxed background job and confirm the tree dies), not assumed.

---

## Sources

> **Note.** This design carries no authoring date; it was written before the Wave 3
> cross-platform cluster landed (verified 2026-07-13). The Landlock ABI levels and the
> Codex sandbox internals cited below move upstream — re-verify them before relying on
> any specific version claim.

- [openai/codex — `codex-rs/linux-sandbox/README.md`](https://github.com/openai/codex/blob/main/codex-rs/linux-sandbox/README.md) — helper-binary architecture, `PR_SET_NO_NEW_PRIVS` + seccomp net filter, bubblewrap `--ro-bind` / `--unshare-net`, Landlock as legacy fallback.
- [Inside the Codex Sandbox: Platform-Specific Implementation on macOS, Linux and Windows](https://codex.danielvaughan.com/2026/04/08/codex-sandbox-platform-implementation/) — Landlock+seccomp split, `AF_UNIX` exemption, and the Windows restricted-token / sandbox-user / firewall model (and its admin requirement).
- [`rust-landlock` crate docs](https://landlock.io/rust-landlock/landlock/) and [`ABI` enum](https://landlock.io/rust-landlock/landlock/enum.ABI.html) — `CompatLevel::BestEffort` vs `HardRequirement`; ABI 1 = 5.13, ABI 2 = 5.19, ABI 3 = 6.2, ABI 4+ (TCP scoping).
- [Landlock — The Linux Kernel documentation](https://docs.kernel.org/userspace-api/landlock.html) and [`landlock(7)`](https://man7.org/linux/man-pages/man7/landlock.7.html) — the `LANDLOCK_CREATE_RULESET_VERSION` ABI probe; unprivileged, `lsm=`-gated.
- [openai/codex issue #1039 — WSL: seccomp/landlock combination not supported](https://github.com/openai/codex/issues/1039) — real-world evidence that a kernel-version check is insufficient.
- [Launch an AppContainer — Win32](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer) and [`windows::Win32::System::JobObjects`](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/System/JobObjects/index.html) — the two Windows primitives and their scope.
- [Chromium sandbox design](https://chromium.googlesource.com/chromium/src/+/b4730a0c2773d8f6728946013eb812c6d3975bec/docs/design/sandbox.md) — restricted token + job object + integrity level, and the `Everyone`-writable-ACL limitation.

## Related documentation

- [macOS Seatbelt sandbox (BR-64)](../../history/agent-loop-campaign/cross-platform/macos-seatbelt-sandbox.md) — the macOS-only predecessor this design generalizes, and the source of `SeatbeltPolicy`.
- [Cross-platform command safety (BR-68)](../../history/agent-loop-campaign/cross-platform/command-safety.md) — the companion design that delivered Slice 0's rule work; rules there, containment here.
- [Platform parity audit](../../history/agent-loop-campaign/cross-platform/platform-parity-audit.md) — GAP-4, the finding that the sandbox existed on one of three shipped platforms.
- [Cross-platform cluster verification report](../../history/agent-loop-campaign/cross-platform/parity-verification-report.md) — the gate record for commit `2d16ff0a`, and the caveat that the Linux and Windows arms were never compiled.
- [Cross-platform CI verification gate (BR-70)](../../history/agent-loop-campaign/cross-platform/ci-gate.md) — the compile gate this design needed as a venue for its Linux-only kernel code.
