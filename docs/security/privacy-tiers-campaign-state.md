# Where the privacy campaign stands

> Computer-use references in historical investigations below are superseded by the [native Computer Use contract](../design/computer-use-integration-plan.md). Old script/control and Developer capture routes are removed; web/document tools now belong to `webdocuments`. Historical source paths and test receipts are not current executable guidance.


> **What this is.** A continuity record for the privacy-tiers campaign (issue #56) and the work that
> travelled with it: what shipped, what the system actually guarantees, what is left, and the
> constraints someone resuming this will otherwise rediscover the hard way.
> **Status:** Current — the campaign is unfinished. Move this to `docs/history/` when it concludes.
> **Audience:** anyone picking this work up, in this session or a later one.

Read [the execution plan](privacy-tiers-execution-plan.md) for the task-level detail. Read **this**
first, because the plan describes intent and this describes reality — and they have disagreed more
than once.

## 1. The most important thing: what this system actually is

⚠ **Privacy tiers are a safety boundary, not a security boundary. Do not describe them as the
latter, in documentation, in the UI, or to a user.**

This was settled by the operator on 2026-08-05 after tracing the bypasses end to end. It is the single
most important fact in this document, because a reader who gets it wrong will license behaviour the
system cannot back.

**What it reliably prevents — mistakes by a cooperative actor:**

- A researcher who has forgotten which model the chat is bound to.
- An agent that helpfully reaches for the OMOP or CDW connector because the task mentioned patients.
- A chat that drifts from public to private without anyone noticing.
- Private conversation history being fed to a public model through Biorouter's own APIs — chat
  recall, cross-session ingest, the workspace tools, summaries, auto-naming, copy, diverge, import.

Those are the overwhelming majority of real incidents, and against them the barrier holds.

**What it does not prevent — a determined or injected path.** There are three known bypasses, and they
share **one root cause: every barrier sits above the filesystem, and the filesystem is open**
([DR-17](privacy-tiers-execution-plan.md) descoped the general filesystem barrier for v1).

| Bypass | Cost to an agent with shell access |
| --- | --- |
| Read files a private session wrote | Free — no barrier exists |
| Read `~/.config/biorouter/sessions/sessions.db` | Free — one filesystem inspector exists and it guards the *memory* store, not sessions |
| Reach a private extension (OMOP, CDW) | **Two file edits** — rename the entry in `config.yaml`, delete `extension-provenance.json`. No rebuild, no reinstall |

⚠ **The third is a difference in kind, not degree.** The first two leak data already written. The
third grants *live query access to EHR data from a public model*, which is the one thing the tiering
is otherwise good at refusing. It is documented as a residual in
`crates/biorouter/src/privacy/extensions.rs` and must be named in user-facing docs rather than
discovered.

**The threat that makes this matter more than "a user could cheat":** prompt injection. A document
ingested into a private session can carry instructions a later public session acts on. Against a
cooperative agent the barrier works; injection is precisely what removes the cooperation.

**Operator's ruling, 2026-08-05:** ship it as safety, documented honestly, with the two-edit bypass
named. Encryption of the session store and OS-level read-deny are **deferred, not dropped** — see §6.

## 2. What the barrier does hold, precisely

Verified against the code, not the plan. A public-capability model:

- **Cannot call** a private extension's tools — `dispatch_tool_call` returns `Err`, not a prompt.
- **Cannot see** them — `allowed_extension_keys` filters them out of the tool list entirely.
- **Cannot attach** one — 409, and deliberately with **no user-proof branch**: attaching a private
  extension to a public chat *"is not a raise the user can authorize either."* Even the human at the
  keyboard cannot approve it; the route is to switch the chat's model.

⚠ **Permission modes are not a lever on any of this.** The privacy gates never consult
`PermissionMode` — zero references. Autonomous mode, always-allow rules and approval settings do not
open the barrier. The **only** switch that disables it is the master toggle
(`privacy_tiers_enabled()`, default `true`), which now lives in a protected store rather than
`config.yaml` and requires a user-action proof.

⚠ **Classification fails open.** An extension absent from the compiled `PRIVATE_EXTENSIONS` snapshot
resolves Public. The guarantee is exactly as good as the tagging, which today is `cdwagent` and
`ucsfomopagent`, both `ucsf`.

## 3. The model, in one page

**Two lattices, plus a third axis.**

- **Tier** — *how sensitive?* `Public` / `Private`. **Capability** is the *least* privileged model
  bound; **classification** is the *most* sensitive thing touched, and it is a permanent ratchet.
- **Affiliation** — *under whose agreements?* `Local` / `Institution(id)` for models; `Any` /
  `Institutions({…})` for extensions. Added because **HIPAA compliance does not transfer between
  institutions**: a UCSF-approved model has no blanket permission over another site's PHI.

⚠ **`Local` is the TOP of the affiliation lattice, not a peer of the institutions.** A local model
reaches everything private, because no disclosure occurs and there is nothing for an agreement to
govern. The natural equality-based implementation breaks exactly this case and passes every other
test.

**Eight gates** (A–H) on the distinct reach paths: bind, turn, dispatch, chat recall, discovery,
extension channels, cross-session ingest, alternate providers.

## 4. Decision records — the index

All live in [the execution plan](privacy-tiers-execution-plan.md). They are **binding**; do not
reinterpret one to widen or narrow what it names.

| DR | Ruling |
| --- | --- |
| 14 | The filesystem barrier is two layers; the OS sandbox is the second |
| 16 | Raising an existing session's tier is user-only |
| 17 | **Scope:** narrows the plan to the session store; defers the filesystem barrier |
| 18 | A knowledge base's tier is user-controllable |
| 19 | **User warns, agent never** — the governing asymmetry |
| 20 | A system password gates declassification |
| 21 | An app session's capability is fixed at creation |
| 22 | The master switch does not live in `config.yaml` |
| 23 | An extension's tier is re-derived, never stored locally |
| 24 | All three platforms get a real system-auth prompt |
| 25 | CLI diverge stays ungated — it discloses nothing |
| 26 | **Affiliation is a third axis** |
| 27 | Three mixing modes: `open` / `standard` / `strict` |
| 28 | **Capability governs reach**, and exporting is a declassifying act |

## 5. State of the tree, 2026-08-05

| Branch | Ahead of main | Contents |
| --- | --- | --- |
| `feat/privacy-tiers` | **358 commits** | The whole campaign. Worktree `/Users/wgu/Desktop/BioRouter-privacy` |
| `docs/campaign-documentation` | **15 commits** | Workspace-control docs, v1.89.0 release notes, root-Markdown refresh. Worktree `/Users/wgu/Desktop/BioRouter-docs` |
| `worktree-heatmap-responsive-home` | — | The Astryx design + heatmap work, **unmerged and gated** |

All pushed. Everything through the affiliation axis, the session-addressing barrier, the grant UI,
the system-password wiring, the mixing modes, the closed private list, the fail-safe default and the
extensible affiliation model has **landed**.

## 6. What is left, in order

1. **Privacy user docs + the full release gate** — running at the time of writing.
2. **Three deferred-then-queued items** (`priv-p8`): the session-store inspector, the CLI capability
   work that inverts the reach gate from proof-of-human to capability, and export gating.
3. **Release blockers** — the desktop daemon is unreachable from the CLI (ephemeral port + per-launch
   random secret); the CLI tells users to run `biorouter sessions watch`, which is not a registered
   command; `open_tab` reports false success; `close_tab` returns success with `wait_result: false`;
   `activate_tab` has zero emitters; the unbounded pending queue; empty subagents invisible to
   `session list`.
4. **Merge** both branches to main, delete branches and worktrees.
5. **At-merge documentation** — the README's sensitive-data section (a 13-point spec exists),
   `SECURITY.md`, the docs table, Key Features, and moving the privacy docs off `Status: Proposed`.
6. **Bump 1.88.6 → 1.89.0** via `scripts/release.sh bump minor`; retro-tag v1.88.6 (it shipped
   untagged, as did v1.88.4 and v1.87.2).
7. **Build all four platforms**, one at a time, sign and notarize both macOS dmgs.
8. **Test the release artifacts**, not the dev build.
9. **STOP.** Do not publish. Do not begin Astryx without an explicit signal.

**Deferred, not dropped:** encrypting the session store (≈4–6 tasks, complicated by an FTS index that
would hold plaintext beside encrypted content) and extending the shipped `ShellSandbox` with
read-deny (cheap on macOS Seatbelt, expensive on Linux Landlock which is allowlist-based, impossible
on Windows which ships an honest no-containment tier).

## 6b. ⚠ Merge preconditions — nothing in the tree enforces these

Found by verification on 2026-08-06. Both are ordering hazards with no automated guard, so they live
here rather than in a commit message nobody will re-read.

**1. `docs/campaign-documentation` must NOT merge before, or without, `feat/privacy-tiers`.**

The README on the docs branch carries two references that resolve **only** on the privacy branch:

- an anchor into `data-privacy-and-phi.md#the-provider-guidance-on-this-page-is-now-enforced` — that
  heading exists only on `feat/privacy-tiers`;
- a link to `docs/security/privacy-tiers-migration.md`, which is **absent** from the docs branch.

Merge the docs branch first and `main` ships a README with a broken anchor and a dead link, on the
page a security reviewer reads first. The content itself is correct — it is the sanctioned
"lands *with* the merge" kind — but the ordering is load-bearing and unguarded.

**2. Both branches edit `data-privacy-and-phi.md` in the same header region, and the second one
conflicts.**

- The **docs** branch replaces the "no recorded last-reviewed date" / provider-drift block with a
  dated review **and the Llama Server correction** (its local-model guidance previously named only
  Ollama, predating the bundled sidecar).
- The **privacy** branch still carries the old block, plus three new sections.

⚠ **A careless resolution silently drops the Llama Server correction** — taking the privacy branch's
side of that hunk looks clean and loses it. The privacy branch has three "Llama Server" mentions in
that file; the docs branch has the corrected tables. Check for both after resolving.

## 7. Constraints and lessons that cost time to learn

**The machine.** The binding resource is file-event and log throughput through a serialised security
path, not CPU or RAM. A 16-core / 128 GB box panicked with `userspace watchdog timeout: no successful
checkins from logd`. **One Rust-compiling worktree**, `CARGO_BUILD_JOBS=2`. Halt below 150 GB free.
Cargo never collects `target/debug/deps`, so each rebuild leaves the prior ~300 MB test binary behind
— this campaign needed four sweeps. `rm -rf target/debug/incremental` is free; deduping superseded
hashed binaries recovers tens of GB more.

**Codex is gone.** Out of credits since 2026-08-03, and the invocation path is **removed** from every
harness rather than left to fail over. Measured provenance: **32 of 166 review verdicts were Codex
(19%)**; the rest were labelled Claude fallbacks, mostly from a 30-hour window when a malformed
`[agents]` key left the reviewer dead while returning a verdict-less "Waiting…" that scored as a pass.
A review with no verdict now fails closed.

**The pattern that keeps producing defects: the mechanism built, the entry point never wired.** Three
instances — the knowledge backfill was unreachable and its notice never rendered; the system-password
prompter had no callers; the cross-affiliation grant had no UI, making a "warning" a hard block. A
code review passes all three, because every unit is correct and nothing calls it. **Drafting
user-facing prose found the third**, by trying to write "here is how you accept this warning" and
finding no answer.

**Enumeration loses.** #56's design was defeated three times by enumerating rather than structurally
closing (tool name → tool list → argument shape). Standing bar for a gate: **it must FAIL a plausible
wrong implementation**, not merely pass the happy path.

**Verify a completeness test can fail.** Delete an entry, watch it fail, restore it, record the
observed failure. This plan has shipped a grep gate, a file-exists check and a cargo filter matching
nothing — each passed by accident.

## 8. Open questions for the operator

- Whether the CLI's four remote-control subcommands keep their refusal, or move to capability (the
  DR-28(a) work in `priv-p8` assumes capability).
- Whether encryption returns to scope for a later release, and whether it is filed as a tracked issue
  rather than living only in a plan document.
- Whether the macOS-only sandbox read-deny is worth taking alone, given Windows gets nothing.

## Related documentation

- [Privacy tiers execution plan](privacy-tiers-execution-plan.md) — every task and decision record.
- [Privacy tiers](privacy-tiers.md) — the design this campaign executes.
- [Data privacy and patient data](data-privacy-and-phi.md) — provider guidance. ⚠ Carries no
  last-reviewed date and its local-model guidance predates the bundled Llama Server; fix before
  pointing a PHI reader at it as authoritative.
- [Linux and Windows sandboxing](../agent-loop/designs/linux-and-windows-sandboxing.md) — the shipped
  `ShellSandbox`, currently write-confinement and network only.
