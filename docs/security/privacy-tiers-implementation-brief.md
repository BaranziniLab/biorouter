# Privacy tiers — implementation brief

> Computer-use references in historical investigations below are superseded by the [native Computer Use contract](../design/computer-use-integration-plan.md). Old script/control and Developer capture routes are removed; web/document tools now belong to `webdocuments`. Historical source paths and test receipts are not current executable guidance.


> **What this is.** The entry point for implementing [issue #56](https://github.com/BaranziniLab/biorouter/issues/56).
> It tells you where to work, which rulings are closed, which approaches are already dead and why,
> how to stage the work, and what verification actually proves. It does **not** replace
> [`privacy-tiers.md`](privacy-tiers.md) (the design) or
> [`privacy-tiers-execution-plan.md`](privacy-tiers-execution-plan.md) (the fifty-one task units) —
> it tells you how to use them and where they are known to be wrong.
> **Status:** **Executed** — the work this brief fronts is implemented for v1 on the
> `feat/privacy-tiers` branch, not merged to `main` as of 2026-08-05. It is retained as the record of
> which enforcement approaches are dead and why. **Narrowed by operator ruling on 2026-07-30 ([DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) and [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base)).**
> The plan it fronts had failed four independent adversarial review rounds before implementation
> began, and did change during it. Read [the brief is subject to change](#the-brief-is-subject-to-change)
> before anything else, and see the design's
> [What shipped, and what did not](privacy-tiers.md#what-shipped-and-what-did-not) for the outcome.
>
> ⚠ **Stage 3 is descoped.** The general filesystem barrier — the stage that failed review three times —
> is out of scope for v1; its tasks are marked `DEFERRED` in the plan and kept intact. Two new task
> units replace it in scope terms: **Task 29A** (the user-controlled knowledge-base tier) and
> **Task 30A** (the non-private-model disclosure, which is what makes the ruling's accepted risks
> acceptable).
> **Audience:** the engineer or agent implementing #56, and whoever reviews that work.

Privacy tiers give every model, session, extension and knowledge base one of two tiers. A session's
**capability** is the least-privileged model bound to it; its **classification** is the most
sensitive thing it has touched, ratcheted permanently. The single invariant is that a public-capability
model must never reach private material — not once, not read-only, not indirectly. ⚠ **That is the
goal, and it is not what v1 delivers**; DR-17 draws the enforced boundary at the agent-mediated
channels, and [what is actually true](#dr-3-says-not-indirectly-ar-11-concedes-an-indirect-path-and-dr-17-decides-which-one-binds)
says exactly where it falls. The invariant is easy to state and has proved genuinely hard to enforce: the design and the plan have been rewritten
ten times, and four independent review rounds each found real defects the previous round's authors
and reviewers had missed. This brief exists so that the fifth round does not repeat the first four.

---

## The brief is subject to change

In the operator's own words: **"the plan can be subject to change if problems arise or things that
can compromise the security arise."**

That is not a disclaimer. It is a working instruction, and the history says you will use it. Four
rounds of review have found four architecturally distinct bypasses, each one invisible to the round
before it. There is no reason to believe the fifth round's implementation is the first that will not
turn one up. Treat "I have found something that weakens the barrier" as an **expected** event with a
defined procedure, not as a failure to be worked around quietly.

### The amendment protocol

If implementation uncovers a bypass, an unsound assumption, an enforcement point the plan does not
cover, or anything else that would weaken the barrier:

1. **Stop.** Do not narrow the design to fit what you have already built, and do not carve out an
   exception so the current task can close. Every one of the four failures below was, at the moment
   of discovery, expressible as a small carve-out.
2. **Write the finding down** in the plan, in the task it belongs to, with the *measurement* that
   established it — a `file:line` in the real tree, a command and its output, or a test that fails.
   A finding without a measurement will be re-argued.
3. **Raise it** to the operator, naming which settled ruling it touches and what the options are.
4. **Do not silently narrow a settled ruling.** The rulings in
   [the settled rulings](#the-settled-rulings) are the operator's, not the plan's. If the tree cannot
   express one, that is a finding to raise, not a specification to soften.

Two precedents, both real and both in this document's history:

- The design's §9.5.3 once carried a carve-out reading *"what Layer A does not cover, deliberately:
  `kb_read_page`, `retrieve_memories`, `read_app`, `list_apps`."* That sentence **redefined DR-14
  rather than implementing it**, and the ninth round rejected it. The right response was Task 14E (a
  guard in each root's own resolver), not a shorter promise.
- The tenth-round revision note contains the sentence *"⚠ **Defined* was as far as that fix went;
  nothing emitted the field until the tenth round below."* A field was added to satisfy a type
  assertion, the gate went green, and the control it existed for was dead on both kernels for a full
  round. Every type assertion passed.

⚠ **A narrowing the operator makes is not the narrowing this rule forbids, and [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) is the
precedent.** The protocol above ends at step 3, *"raise it to the operator"* — and on 2026-07-30 the
operator answered by descoping the filesystem barrier. That is the protocol working, not a breach of
it. **The rule still binds every implementer:** no one may invoke DR-17 to descope anything DR-17 does
not name, and it names three things (files a private session left elsewhere, encryption at rest, the
general filesystem barrier) and nothing else.

**What must never be silently narrowed:** DR-1 through DR-18, and the two structural rulings that came
out of the review rounds — *the barrier sits at a choke point, never on an enumeration* (see
[the killed-approaches register](#the-killed-approaches-register)), and *the capability is sampled once
at the outermost entry and threaded* (O15).

---

## How to read the material

Read in this order. Do not start in the execution plan.

| # | Document | Why, and what to take from it |
|---|---|---|
| 1 | **This brief** | Where to work, what is settled, what is dead. |
| 2 | [`privacy-tiers.md`](privacy-tiers.md) (2,257 lines) | The *what* and *why*. §3 (settled requirements R1–R14), §4 (the two lattices), §7 (the capability matrix), §9 (enforcement), §9.5 (the read-deny). ⚠ **Every Rust line number in the design is stale** — it was verified against `main` at `708390d8`. Read it for the model, never for an anchor. |
| 3 | [`privacy-tiers-execution-plan.md`](privacy-tiers-execution-plan.md) (18,160 lines) | The *how*. Read its preamble in full — "Read this before you chase a line number", "Non-negotiable orderings" (O1–O16), "Departures from the design" (D1–D9), "Accepted risks" (AR-1–AR-15), "Which test filters are validated", "Decisions of record" (DR-1–DR-16), "Open questions" (1–24). Then read only the task units for the stage you are on. |

**The execution plan is detailed and it has known-wrong code in it.** Three separate review rounds
found prescribed snippets that cannot compile, and the plan itself has recorded some of the same
symbols as absent elsewhere on its own pages. Measured examples, all from round 3:

- Task 14D called `Agent::capability_tier()` and `Agent::working_dir()` — neither exists, and the plan
  had already recorded at its own line ~7005 that `Agent` has no capability method.
- The Developer re-check read `self.private_data` on `DeveloperServer`, which has no such field, in a
  handler (`text_editor`) that receives no `RequestContext` to derive one from.
- `expand_tilde(candidate)` was called by a module that neither defines nor imports it; the only such
  function is private to `crates/biorouter/src/security/policy/command.rs`.
- `Config::all_values().get(…)` does not compile — `all_values()` returns
  `Result<HashMap<…>, ConfigError>`.
- `tempfile` was prescribed for production `seatbelt.rs` while being a dev-dependency only.
- Task 12 put a method on `SessionStorage` and called it on `SessionManager`, which exposes only
  `storage()`.

Those six are repaired. **One is not, and it was found while writing this brief** — it is the live
example of what the rest of this section is about:

> **The contrast gate is already vacuous, and the corrected post-#56 number is 324.** Tasks 26 and 32
> expect `cd ui/desktop && node scripts/check-contrast.mjs` to print
> `OK — all 288 contrast assertions pass` *after* #56 lands, from a stated baseline of 252 plus Task
> 26's 36 new assertions. Measured on `main` today (`4e941619`), with no #56 code anywhere, it prints
> exactly that — **`OK — all 288 contrast assertions pass`**, exit 0.
>
> The cause is drift, not a bug. `check-contrast.mjs` gained **+73 lines** between the plan's anchor
> `9558c346` and `main`, and `grep -ci privacy` over it returns **0**, so none of that growth is #56's.
> The plan's post-#56 total is now the *pre*-#56 baseline. **That gate passes today, and it would still
> pass if Task 26's 36 assertions never landed.**
>
> **The arithmetic, every term of it measured today.** The script runs one identical block per
> family×mode **scope**, and there are six (Parchment / Alma Mater / Roche Limit × light / dark).
> Counting today's output by scope gives **48 in each of the six**, and 48 × 6 = **288**. The plan's
> anchor decomposed as 42 per scope × 6 = 252, so the drift is +6 per scope — a theme fix
> (`643a1f25`, the reference chip) that has nothing to do with #56. Task 26 adds **+2 per scope** (the
> hover ground, 2 text tokens) and **+4 per scope** (the four badge assertions), which is exactly the
> plan's own `+12` and `+24`. So the target is **(48 + 2 + 4) × 6 = 54 × 6 = 324.**
>
> **Correct Tasks 26 and 32 to 324 and record the correction in both.** Then re-measure the baseline
> on your own branch point before trusting even this number: it has already moved once for a reason
> unrelated to #56, and Stage 4 is the last stage to start.

The lesson is the general one, and it applies to snippets and to numbers nobody has checked yet:

> **The compiler is the truth. The plan is a strong hypothesis.** Where a prescribed snippet does not
> compile, fix it against the real signature — and **record the deviation in the task**, in the same
> commit. A silent fix means the next reviewer re-derives the same problem, and it also hides the case
> where the snippet was wrong because the *design* was wrong.

Same rule for line numbers. **The named symbol is the anchor; the line number is a hint.** The plan's
anchors are against `9558c346`; `main` has moved. Measured drift on files the plan anchors into,
`9558c346` → current `main`:

| File | Change |
|---|---|
| `crates/biorouter-mcp/src/developer/rmcp_developer.rs` | +892 lines (issues #64/#67/#68, the file-tool jail) |
| `crates/biorouter-mcp/src/memory/mod.rs` | +615 |
| `crates/biorouter-mcp/src/knowledge/macros/ingest.rs` | +442 |
| `crates/biorouter-mcp/src/developer/shell.rs` | +416 |
| `crates/biorouter-mcp/src/knowledge/macros/query.rs` | +128 |

Two concrete near misses that already exist: `MemoryRouter::get_memory_file` is `memory/mod.rs:434` on
`main` and `:336` in the plan; Landlock's write-only handling is `shell_sandbox/linux.rs:415`
(`AccessFs::from_write(abi)`) and `:399` in the plan. A near miss reads as a hit — that is how BR-71
lost time, and it is why every Files table in the plan states its base.

---

## Where to work

### Base: `main`

All three documents are on `main`. Confirm you have them before writing a line:

```bash
cd /Users/wgu/Desktop/BioRouter
git rev-parse --abbrev-ref HEAD          # main
git log --oneline -5 -- docs/security/privacy-tiers-execution-plan.md
# Every ruling through DR-18 must be present. Measured on main, 2026-07-31:
grep -c DR-16 docs/security/privacy-tiers-execution-plan.md   # 30
grep -c DR-17 docs/security/privacy-tiers-execution-plan.md   # 64
grep -c DR-18 docs/security/privacy-tiers-execution-plan.md   # 19
```

⚠ **Treat those three counts as "must be non-zero", not as equalities.** The plan is being amended
while you read this, so an exact match is not the test — a **zero** is, and a zero on DR-17 or DR-18
means your checkout predates the scope narrowing and every judgement you make from it will be wrong
about what is in scope. A stale checkout is the cheapest possible way to lose a day, and this feature
has already lost a ruling to one.

### `feat/privacy-tiers` is gone, and there is nothing to recover from it

Both the branch and the `/Users/wgu/Desktop/BioRouter-privacy` worktree have been **deleted** —
verified 2026-07-31: `git branch -a --list '*privacy*'` is empty (no local or remote ref) and the
worktree path does not exist. This subsection is history, not instruction. It is kept only so that
nobody who finds the name in an older document goes looking.

What was on it, and why none of it is wanted:

- It carried the **documents only** — no #56 production code at any point. That is the measurement
  behind the register's note that every killed approach was a document proposal.
- It was stale in both directions, `main` far ahead, and its copy of `privacy-tiers.md` was missing
  the whole two-layer read-deny rewrite (`35bf782e`, `6d6a7eca`, `72dc9de2`).
- Its one unmerged commit was a **settled operator ruling that never reached main**: `500e9b1d`
  recorded DR-16 (the user-only tier raise), and the merge that landed 18,000 lines of plan did not
  carry it. It was cherry-picked onto `main` before the branch was deleted; the ruling is on `main`
  now, which is what the `grep -c DR-16` check above confirms.

The lesson outlives the branch and is the reason it is still written down: **a ruling can be lost by
a merge**, and the only thing that catches it is grepping the tree you are about to work in for the
rulings you expect to find.

### One fresh branch and worktree per stage

Stages stay independently reviewable and revertible. Branch off `main`, in a worktree:

```bash
cd /Users/wgu/Desktop/BioRouter
git worktree add -b feat/privacy-tiers-stage1 ../BioRouter-privacy-s1 main
# later, each off the CURRENT main, after the previous stage merged:
git worktree add -b feat/privacy-tiers-stage2 ../BioRouter-privacy-s2 main
git worktree add -b feat/privacy-tiers-stage3 ../BioRouter-privacy-s3 main
git worktree add -b feat/privacy-tiers-stage4 ../BioRouter-privacy-s4 main
```

A stage merges to `main` **only after its own verification passes** — its phase gate, the full suite,
and an adversarial review of its diff. Conventional commits; **no `Co-Authored-By` trailers**, CI
rejects them. Do not push.

### Two hazards measured in this campaign

**Concurrent `cargo` across worktrees will OOM-kill builds mid-run, and a killed build reads like a
test failure.** There are nine worktrees on this machine already. Keep `CARGO_BUILD_JOBS` modest
(`CARGO_BUILD_JOBS=4` is a reasonable ceiling when anything else is compiling) and do not run several
stages' suites at once. If a suite dies with a signal rather than a failing assertion, suspect the
OOM killer before suspecting your code.

**BR-71 (issue #30) is being implemented concurrently on `feat/br71-workspace-control` and collides
with Stage 2.** Measured: that branch is 88 commits ahead of `main` and its diff touches
`crates/biorouter/src/agents/agent.rs`, `crates/biorouter/src/agents/extension_manager.rs`,
`crates/biorouter/src/agents/extension.rs`, `crates/biorouter-server/src/routes/session.rs` and the
whole new `crates/biorouter-server/src/workspace/` tree. Stage 2 threads a `CallCapability` through
`Agent::dispatch_tool_call` and `ExtensionManager::dispatch_tool_call` — the two functions BR-71
rewrites most. **Rebasing on `main` after BR-71 merges is cheaper than resolving that conflict twice.**
Two related facts already recorded: BR-71 also takes migration 17 for `parent_session_id` (O10 and
open question 12 make either merge order safe *in the database*, but the two diffs conflict textually
in `session_manager.rs`), and O16 sequences Task 14E relative to 14B.

---

## The settled rulings

Decided by the operator. **Not open for re-litigation in a PR.** If the implementation contradicts one,
the implementation is wrong; if the *tree* cannot express one, that is a finding to raise under
[the amendment protocol](#the-amendment-protocol), not a licence to narrow it.

The operative sentence of each is quoted verbatim below. The full row — the reasoning, the costs
accepted and the alternatives rejected — is in the plan's [Decisions of record](privacy-tiers-execution-plan.md#decisions-of-record);
read the full row before implementing against it.

| # | The ruling, verbatim |
|---|---|
| **DR-1** | "**Private models** are institutionally hosted (`versa_azure`, `versa_bedrock`) and user self-hosted (`llamacpp`, `ollama`). **Public** is everything hosted by an AI company or a large cloud — including `azure_openai`, `aws_bedrock`, `databricks` and `vertex`, whatever their names suggest." |
| **DR-2** | "**Two lattices, opposite directions.** CAPABILITY (what a session may DO) = the **least** privileged model bound to it… CLASSIFICATION (how sensitive its CONTENTS are) = the **most** sensitive thing it has touched, a permanent ratchet." |
| **DR-3** | "**A public model must never reach a private session.** Not once, not read-only, not indirectly. The converse is unrestricted: a private model may read anything." |
| **DR-4** | "**The ratchet fires on the first TURN and on a permitted private-extension dispatch — never on the bind.**" |
| **DR-5** | ~~"**Lineage decides write access.** Sessions the caller spawned get full control; everything else is read-only. Lineage is **one hop** — a grandchild is `other`."~~ **RETIRED.** Superseded by the ruling that an agent may inject a prompt into any conversation, child or unrelated, provided it does not cross the private/public boundary. WRITE ⇔ VIS. |
| **DR-6** | "**The BAAM registry is the sole grantor of a private badge, and anything not on BAAM is PUBLIC** (fail-open, by decision). The private set is exactly **`ucsfomopagent`** and **`cdwagent`**." |
| **DR-7** | "**`chatrecall` obeys the barrier**… **Side channels (existence, counts, timing) are explicitly out of scope**: no count padding, no constant-time responses, no decoys. Only content must not cross." |
| **DR-8** | "**Declassification is the user's alone** — an explicit deprivatise action in History. Nothing automatic, nothing an agent can invoke." |
| **DR-9** | Superseded by DR-15. The Gate-C-scoped opt-out is **retired**, not kept alongside the master switch. |
| **DR-10** | "**Fail directions differ by kind, deliberately.** Migration backfill → fail **open**. Runtime read of a missing/unparseable column → fail **closed**. Import with no tier → fail **closed**. Unknown provider → **Public**. Unlisted extension → **Public**. Any gate's lookup failing → refuse, encoded inside `Ok(..)`, never as `Err`." |
| **DR-11** | "**`medcp` stays callable by a public model**, and that is the accepted cost of DR-6." |
| **DR-12** | "**`spokeagent` is public.** SPOKE holds no patient data." |
| **DR-13** | "**A knowledge base ratchets on ingest**… A KB takes the tier of the most sensitive session that has ingested into it, and a public-capability session may not read *or write* a private KB." |
| **DR-14** | ⛔ **DEFERRED for v1 by DR-17 — do not implement Tasks 14A–14F, do not delete them.** "**A public-capability session's tools may not reach Biorouter's own private data, on by default, and the control is TWO layers.**… The entries are the four roots the operator named — the session store, the knowledge roots, the global memory root and the Agent Drafter app root — plus one file, `<config>/config.yaml`… Everything else on the filesystem stays readable and writable — this is **not** a general jail and must not become one. **Private-capability sessions are unaffected.**" |
| **DR-15** | "**One master toggle turns the entire privacy-tier feature off**, config key `BIOROUTER_PRIVACY_TIERS`, default `on`." ⚠ Session copy is **not** on the list of things it disables; the badges do **not** disappear, they restyle and suffix *— enforcement off*. |
| **DR-16** | "**Raising a session's capability to Private is the user's act alone. A model may never do it.**" ⚠ **DR-16 has no task written for it.** The ruling names a `Task 18A` that does not exist in the plan. See [Stage 4](#stage-4--the-master-toggle-the-user-only-tier-raise-and-the-ui). |
| **DR-17** | "The generic idea is to lock the sessions that are actually private or executed using a private model so that a public model or a non-private model cannot use the AI agent to easily get access to those sessions… we just need to make sure that the exact session logs and histories cannot be viewed by the public models. The public models cannot spin up private models to help them do their work of querying the sensitive databases using all of the different extensions that are only available for the private models… we don't have to enforce and encrypt every single step along the way. for now." ⚠ **Also a requirement, not only a descoping:** "still make sure that users understand the risks of using non-private models" — Task 30A, and it is what makes the accepted risks acceptable. |
| **DR-18** | "knowledge bases should also be able to be deemed private - as it is also a piece of biorouter component . please make sure that users can change the kb to be private or public and the private model generated kb will automatically be private until the user publicize it, and all the other guardrails will apply as well." ⚠ Task 29A, and it **resolves AR-1**. |

Two further rulings are structural rather than policy, came out of the review rounds, and carry the
same weight:

- **The barrier sits at a choke point, never on an enumeration.** Phrase every gate as *"every tool
  call passes through symbol X"*, never as *"the list of tools that read files"*. See the register
  below for why.
- **The capability is sampled once at the outermost entry and threaded** (O15). Four production entries
  reach a tool call — the agent loop, `POST /agent/call_tool`, the `execute_code` bridge, and
  `Agent::call_prefetch_tool`. Each captures one `CallCapability` (provider tier *and* master toggle,
  in one instant) and everything downstream takes it as a parameter.

### Two places the material contradicts itself, and what is actually true

Both were found by review of the first version of this brief. A contradiction left standing gets
resolved by whoever hits it first, in whichever direction lets their task close — so both are
resolved here, in writing.

#### DR-3 says "not indirectly", AR-11 concedes an indirect path, and DR-17 decides which one binds

DR-3 rules that a public model must never reach a private session, *"not once, not read-only, not
indirectly."* AR-11 measures that a tool running **inside** the daemon recovers the daemon's own API
secret — `ps -Ewww -p $PPID` on macOS (under a hardened, notarized binary, and under every
constructible sandbox profile, because `sysctl-read` is not gated) and `/proc/self/environ`
in-process on Linux — and then reads a private transcript from `GET /sessions/{id}/export`. As
written, both cannot be true.

**What is true after DR-17:** DR-3 remains the settled ruling and the goal the design is aimed at. It
is **not** the guarantee v1 delivers. DR-17 draws the v1 enforcement boundary at the
**agent-mediated** channels, and everything outside that boundary is an accepted, disclosed residual
rather than a defect to fix in this issue.

| | In scope for v1 — a defect if it leaks | Out of scope for v1 — disclosed, not mechanised |
|---|---|---|
| **Channel** | The tool-call choke point (Gate C), the tier and bind gates, the knowledge-base tool barrier (CP1–CP5, which DR-18 makes requirement R16), the app and worker seams, the extension catalog | The raw filesystem — `developer__shell` reads `sessions.db`, which carries a contentful FTS mirror of every message by design; the daemon's HTTP API once the secret is recovered in-process (`/sessions/{id}/export`, the `/knowledge/*` read routes, `GET /apps/{id}/export`, and `GET /diagnostics/{id}`, which is the widest — a zip of `session.json`, recent `logs/*.jsonl` and a verbatim `config.yaml`) |
| **Authority** | DR-3, DR-13, DR-18 | DR-17, disclosed by **Task 30A**, which is what makes the residual a considered tradeoff rather than an omission |

Two rules follow, and they are symmetrical. **Do not cite DR-3 to build the descoped stage** — the
filesystem barrier is out of v1 by operator ruling, not by oversight. **Do not cite DR-17 to weaken
an agent-mediated channel** — DR-17 names three things (files a private session left elsewhere,
encryption at rest, the general filesystem barrier) and nothing else.

⚠ Note what AR-11 *withdrew* when Stage 3 was descoped. Its earlier claim was that the second door
was "held by Layer A". Layer A is gone, so `POST /agent/call_tool` is now covered by **Gate C alone**
— a private extension is refused there, which is DR-17 requirement 2 — and the path-barrier half of
that coverage no longer exists. The register's *"a caller with the secret gets exactly what the chat
gets"* row still holds as a narrowing, and it is still not a closure: issue **#47** is the open item,
and #56 neither fixes nor depends on it.

#### "Never on an enumeration" versus a knowledge root with no resolver

The rule says phrase every gate as a choke point. The knowledge design is CP1–CP5 plus a grep, and
the plan concedes the root has no private resolver: `resolve_readable_path`
(`knowledge/store.rs:121`) has **3** call sites against roughly **40** direct filesystem reads in the
same module, and `KnowledgeService::root()` is `pub` (`service.rs:415`) because `routes/knowledge.rs`
legitimately joins off it at 7 sites.

**The rule stands; the design carries a stated exception.** Resolved in that direction, and here is
the distinction that makes it honest:

- **CP1–CP5 are not an enumeration.** They are five choke points on five *channels*. CP1 —
  `<KnowledgeServer as ServerHandler>::call_tool` — covers all nineteen `kb_*` tools **and the
  twentieth, the day it is written**, including the nine that take no `RequestContext`. CP2 is the
  `lock_kb` + `kb_root` prologue every macro shares; CP3 is `handle_kb_frame`, the single funnel its
  three call sites share; CP4 is `stage_full_payload`, the drafter's only door to KB content; CP5 is
  `Catalog::discover`. Each is "every call on this channel passes through symbol X". That is the rule
  being followed, not bent.
- **The exception is one level below them: the library API.** A new caller *inside* `biorouter-mcp`
  that joins off `root()` reaches KB content without passing any CP. Nothing in the type system stops
  it. Task 14E's grep was the guard, and DR-17 defers Task 14E — so, in the plan's own words on
  open question 22, **"with Task 14E deferred, nothing does"**. Task 10C's completeness test is the
  surviving guard, and it is a test rather than a type.

⚠ **DR-18 raises the cost of this exception and nothing has yet paid it.** The knowledge tool channel
is no longer a deferred nice-to-have; it is requirement R16 and it ships in Stage 1. So a shipping
requirement now rests on a convention a grep enforces. **The cheapest honest fix is the one open
question 22 already names**: make `root()` `pub(crate)` and give `biorouter-server` a narrower
accessor returning a *base's* directory rather than the tree's. That is a mechanical change, it is
the one piece of the deferred Task 14E worth reviving inside Stage 1, and it converts this exception
back into the rule. Until then, say "exception", never "choke point", when describing the knowledge
root.

---

## The killed-approaches register

This is the most valuable section in the brief, and a gap in it is the most expensive kind of gap
there is: the register is the only thing that stops a fifth attempt repeating the first four.
**Do not re-propose any of it.**

⚠ **What "killed" means here, stated precisely, because an earlier version of this section
overstated it.** Everything below was a **document proposal** — a design paragraph or a prescribed
task unit — that its own author and the previous round's reviewer had already accepted, and that a
later adversarial round then defeated on paper. **None of it was ever built.** This brief records
the measurement itself: `feat/privacy-tiers` carried the three documents and no #56 production code,
and each review round examined a diff in which only `privacy-tiers-execution-plan.md` had changed.
Nothing here "shipped past a green suite", because there has never been a suite to ship past.

That makes the register *more* useful, not less. These are not defects someone was careless enough
to ship. They are the designs a competent author reaches for **first** — which is exactly why they
will be reached for again, by you, unless you have read them here.

### The one lesson: every enforcement design that enumerates gets defeated one level down

Enumeration has now lost **three** times, at three different levels of abstraction, and each loss was
invisible to the level above it:

| Round | The enumeration | How it was defeated | Evidence |
|---|---|---|---|
| **1** | Enumerate **tools** — classify the tools that touch private data | Missed the arbitrary-execution builtins entirely. `developer__shell` runs any command, the shell is explicitly not jailed by the file tools' base, and the OS sandbox that could confine it defaults to **Off**. A public model never has to defeat a tool gate; it reads `sessions.db` — which carries a *contentful* FTS mirror of every message by design — straight off disk. | `shell_sandbox/mod.rs:271` (`_ => SandboxMode::Off`), `session_manager.rs:29`, `secret_guard.rs:33` (`DEFAULT_SECRET_PATTERNS` covers credentials, not the data roots) |
| **2** | Enumerate a **tool list** — guard `developer` and `computercontroller` | The OS sandbox cannot constrain tools that read files **in-process** inside `biorouterd`. `computercontroller__cache` accepts an arbitrary path and reads it with `tokio::fs::read_to_string`; the Agent Drafter readers do the same. **They *are* the daemon.** No sandbox the daemon installs on its children can constrain the daemon. Round 1 named the two servers; round 2 found `cache` *inside* one of them. | `computercontroller/mod.rs:1482`, `agent_drafter/store.rs:637`, `developer/text_editor.rs:641` |
| **3** | Enumerate **argument shapes** — guard any path-shaped argument at the choke point | Handlers compute their own paths. `read_app` receives an app id and a *relative* path; `ArtifactStore` supplies the denied root via `self.root.join(id)`. `export_app` reads the app root implicitly and writes to a caller-named destination — a copy primitive. And **only the Developer server receives the session cwd**, so every other built-in resolves a relative path against the *daemon's* cwd while the guard resolved it against the session's. All three pass the plan's own "surprise tool" test. | `agent_drafter/store.rs:447` (`fn dir`), `crates/biorouter-mcp/src/lib.rs:49`, `:77` |

Why a mechanical fix to the enumeration problem is impossible, measured: **125 `#[tool(…)]` declarations in
`crates/biorouter-mcp/src`**, and a `grep` for `fs::`/`File::open` structurally cannot find the
readers — `computercontroller__xlsx_tool` goes through `umya_spreadsheet`, `pdf_tool` through `lopdf`,
`datasql__data_query` through `sqlx`, and none of those lines contains an `fs::` token. A mechanical
extractor written for that survey silently dropped **the entire developer server**, because
`rmcp_developer.rs` contains an inner `#[cfg(test)]` and the extractor stopped there.

**What survives:** the choke point. `ExtensionManager::dispatch_tool_call`
(`extension_manager.rs:1438`) is where BR-23's SecretGuard argument scan has run for months; its own
comment calls it *"the single choke point every tool call flows through."* Measured on `main` today:
`grep -rn "\.call_tool(" --include='*.rs' crates/ | wc -l` → **10** total, of which exactly **one** is
a production dispatch into an MCP client, `extension_manager.rs:1562`, inside that function. That
count is a no-growth tripwire, not a measurement of #56 — any increase is a new bypass.

### Round 4 is a different failure, and counting it as a fourth enumeration hides what it teaches

Round 4 found a **privilege escalation**, not a fourth defeated enumeration. It belongs beside the
table above, not in it.

A caller could raise its **own** session to Private with one credential-free
`POST /agent/update_provider {provider:"llamacpp"}` (`routes/agent.rs`). No enumeration was defeated.
The **lattice** was: every gate in this feature can be individually correct and complete and still
enforce nothing, because the value they all branch on is a value their subject can set. That is a
different class of defect from "the list was short", and it needs a different kind of fix — which is
why the answer was a ruling (DR-16) rather than a better choke point.

What makes it sharper than an ordinary bypass: the rule that would stop it forbids *"switch this chat
to a private model"*, and that is **step 1 of the two-ways-out message in every refusal this feature
ships**. A blanket refusal would break the product's own remediation advice, so the fix has to
distinguish the **user's** act from the **model's** — which no HTTP route can do without something
that proves a human acted.

Recorded as [AR-15](privacy-tiers-execution-plan.md#ar-15--retired-by-dr-16--a-caller-holding-the-daemon-secret-can-raise-its-own-sessions-capability-with-no-credentials),
ruled on as **DR-16**, and ✅ **built and shipped** — Task 18A, commit `0757823f`, 2026-08-02. An
upward provider bind over HTTP now needs `X-User-Action` as well as the daemon secret, and **AR-15 is
retired**: it is the one accepted risk in this campaign that an implementation, rather than a scope
ruling, took off the list. (An earlier revision of this line said *"DR-16 still has no task written
for it"*; that was true when it was written and is not now.) See
[Stage 4](#stage-4--the-master-toggle-the-user-only-tier-raise-and-the-ui) and open questions 23–24,
which are the residuals — a daemon started with no key refuses every raise including the human's, and
the CLI has no proof at all.

### Individually killed approaches

| Approach | Why it is dead | Evidence |
|---|---|---|
| **The OS sandbox as *the* mechanism** (Layer B alone) | In-process readers are the daemon. Layer A (in-process, at the choke point) is **primary**; Layer B is defence in depth for spawned children only. This reframing also *shrinks* the cost: on a platform that cannot express the kernel deny, a public session loses the five **spawning** tools, not every file tool. | Plan §"DR-14 is two layers" |
| **Add the data roots to `DEFAULT_SECRET_PATTERNS`** (design §9.3 A2) | Two disqualifiers. It is **unconditional** — an always-on floor applied to every session, so it would hide the user's own KB and chat history from a **private** session too. And it would not close the read anyway: `candidate_is_denied` is lexical and existence-gated, so `sqlite3 "$(printf '%s' ~/.local/share/…/sessions.db)"` walks past it. | `secret_guard.rs:33`, and the design's own admission that "this raises the cost, it does not close the read" |
| **A shared `Arc<RwLock<Vec<PathBuf>>>` guard set before dispatch** | Cross-call mutable state, and dispatch deliberately permits overlapping calls (it returns the tool work as a boxed future). A public dispatch sets the guard; a provider swap plus a private dispatch clears it before the first builtin reads it; the public call then runs unrestricted. Replaced by: Layer A reads Gate C's own **local** and returns before the boxed future exists; Layer B carries a per-call boolean in `_meta`. | `extension_manager.rs:1531`, `:1544` |
| **Sampling the capability — or the master toggle — independently at each layer** | Round 3 counted **four** capability samples and three or four toggle reads, then built the interleaving: (1) call A starts **Public**, its `developer__shell` command constructing the denied path by runtime variable indirection — the exact case Layer B exists for; (2) the **Agent**-level policy samples Public and passes, because no literal denied path appears in the arguments; (3) the provider is swapped to **Private** before A reaches the manager — `Agent::dispatch_tool_call` has intervening `await`s (frontend resolution, vault application); (4) the **manager** samples again, sees Private, builds an empty path policy and hands Layer B `private_data_deny=false`; (5) A's public-authored shell runs unsandboxed. One call, evaluated under two capabilities, resolving to the *more* permissive one. The forced-overlap test could not expose it — it parked **after** the manager had already sampled. ⚠ A **third** read is the easy one to miss: Task 10 mandates one for built-in metadata, so a call admitted as Public can carry Private metadata after a swap. Replaced by O15 (sample once at each outermost entry, thread it by value), gated as a whole-tree count of absence because that is the only kind of gate that fails "threaded it but kept the sampler". | `agent.rs:2753`, `:2767`; round 3 §2 |
| **Gate C as a `ToolInspector`** | Invisible to three of the four production paths that reach the extension manager (`agent.rs`, `routes/agent.rs:1162`, `code_execution_extension.rs`, `Agent::call_prefetch_tool` — the last runs *before* the turn). | O7 |
| **Gate C in `Agent::dispatch_tool_call` instead of the manager's** | Same reason. The agent loop is one of four callers. | O7 |
| **Ratcheting at bind time** | Privatises a chat on a mis-click *and* still misses `POST /agent/call_tool`, which dispatches straight into the extension manager without touching the reply path. | DR-4, O5 |
| **Landlock for the Linux read-deny** | Landlock has no deny rule; the current backend handles **write** accesses only, leaving reads open. Granting the complement was tried and declined: anything created in an enumerated ancestor *after* the ruleset is built is unreadable for that command's lifetime — `cd ~ && mkdir out && echo x > out/f && cat out/f` fails. `bubblewrap` is the only Linux mechanism that can express it. | `shell_sandbox/linux.rs:415` (`AccessFs::from_write(abi)`), open question 17 |
| **`seatbelt::available()` as the capability probe** | It checks only that `/usr/bin/sandbox-exec` exists. Measured on this host: the file exists, and `sandbox-exec -p '(version 1)(allow default)' /bin/true` exits **71** with `sandbox_apply: Operation not permitted`. The probe must be a real negative-plus-positive **execution** test. | `seatbelt.rs:168` |
| **The bubblewrap `--tmpfs` write-deny claim, and its `rm -f` test** | `--tmpfs` is **writable**, so "neither readable nor writable" was false; and the test asserted that `rm -f` of a now-hidden file *fails*, when `rm -f` on an absent path exits 0. The prescribed implementation could not pass its own test on Linux under any correct implementation. | Plan Task 14A |
| **Canonicalize a `PathBuf`, then open it later across an `await`** | Rejected in **rounds 2 and 3**, and still the most natural thing to write. Validation returns a *path*, not a handle, and the open happens on the far side of an async boundary — so a background job can atomically swap a workspace symlink inside that window, and the **daemon**, not the sandboxed child, follows it into a denied root. Round 3 also caught the plan contradicting itself here: AR-9 claimed a sandboxed child could not create the symlink, while Task 14D correctly said it can, because *creating* a symlink never reads its target. The replacement is **resolve-and-open**: return an already-open handle, via `O_RESOLVE_BENEATH`/`O_NOFOLLOW_ANY` on macOS and `openat2(RESOLVE_BENEATH)` on Linux. The `canonicalize()` is precisely what makes this look safe — it is check-then-use wearing a resolver's clothes. | `rmcp_developer.rs:1193`, `:1216`; `computercontroller/mod.rs:1482`; rounds 2 §1 and 3 §3 |
| **Exempting `kb_get_active` / `kb_set_active` because "the caller already knows the id"** | False for a **no-argument** tool. `kb_get_active` serialised the whole selection — every visible base id plus the primary pointer — and the completeness test named it as exempt, i.e. blessed it. Fix: filter the **view**, never the store (`repair_decision` writes `next_ids.first()`, so filtering the store would re-point the user's primary as a side effect of a *read*). | `knowledge/server.rs:665`, `:691`, `:721`; `knowledge/service.rs:1635` |
| **The knowledge-archive provenance chain — three designs, one killed per round** | The longest-lived wrong idea in the campaign; each fix bought exactly one round. **(1) Trust the importer's tier.** The gate demanded only that the tier not travel, and the test exercised a *Private* importer alone — so private export → **public** import laundered a private base into a public one, and passed. **(2) A provenance marker, combined with an outside `dest_path`.** Honouring a caller-named destination for a private export drops a marked archive where a public shell can reach it; the shell strips the marker and imports it. Every archive behaviour test still passed, because forcing `<knowledge-root>/exports` was only an *observational grep*. **(3) Write outside, then move inside before returning.** The final-state assertion — no archive outside the root once the call returns — is satisfied, while a public-read window exists for the entire duration of the write. **A test that asserts a final state structurally cannot see a window.** ⚠ Round 3 found the user-route mirror test invalid for a second, independent reason: it drops its `TempDir`, supplies a `dest_path` query the route does not accept, and expects a file on disk that the real `GET` handler never writes — it returns archive **bytes**. It asserted an absence the code could not have produced, so it passed for the wrong reason and would have gone on passing. | Task 10A across rounds 1 §3, 2 §5 and 3 §7; `routes/knowledge.rs:1518` |
| **"A metadata regex or a call-site inventory proves completeness"** | Defeated in three successive forms. **(1)** The detector **excluded every `src/knowledge/` hit**, so it structurally could not see the active-pointer leak it was written to catch. **(2)** It **cannot distinguish filtered from unfiltered output** — an unchanged, still-leaking `kb_get_active` yields the same `.selection(` hit and the same expected count — and UFCS (`KnowledgeService::list_bases(&svc)`) evades a dot-call regex. **(3)** A **function item** — `let f = KnowledgeService::list_bases; f(&svc)` — and **direct registry/manifest loading** leak the same metadata while matching `META_RE` nowhere. **Say this in exactly these terms and do not soften them: these inventories are drift tripwires, not barrier proofs.** A tripwire tells you a *new* call site appeared beside the ones a human classified. It never tells you that classification was right, and it never tells you the set was complete. Anywhere the plan, a gate or a PR reports one as *coverage*, that is a finding to raise under [the amendment protocol](#the-amendment-protocol). The plan now concedes this of Task 10D itself — "a direct-call drift inventory" — and that is the ceiling for the whole class, not one task's caveat. | Task 10D across rounds 1 §3, 2 §5 and 3 §7 |
| **Sending all three `handle_kb_frame` call sites to `agent.provider()`** | The mid-turn site sits *after* `turn_agent` is bound, and `turn_agent` can be a worker. A private-main/public-worker pair was evaluated as private. It compiles because both agents are in scope. Only the mid-turn site takes `turn_agent`. | `routes/apps.rs:3541` (bind), `:3847` (mid-turn frame) |
| **Reading capability at `capability_report`'s existing position** | That position precedes `configure_main_provider`, so it reads the provider the session held *before* the app's own model was bound — wrong in both directions. The call moves **below** the bind. `configure_worker_agent` has the ordering right and the check missing. | `routes/apps.rs:1257` (report), `:1259` (bind), `:1553-1561` (worker) |
| **"The model has no way to authenticate to the daemon"** | False on both platforms. The daemon secret is recoverable from the daemon's own environment (`ps -Ewww -p $PPID` on macOS, `/proc/self/environ` in-process on Linux). Electron puts it there. Recorded as **AR-11**. | `auth.rs:80` (`check_token` — one machine-wide shared secret, byte-compared), `ui/desktop/src/biorouterd.ts:299` |
| **"Only disk and authenticated `GET /apps` enumerate app ids"** | False. `agent_drafter__list_apps` is a tool on a **public** extension that takes no path argument, so neither Layer A nor a filesystem deny can see it. Withdrawn in Task 14C; open questions 18 and 19 were corrected. | `agent_drafter/mod.rs:2636` → `store.rs:606` |
| **"A caller with the secret gets exactly what the chat gets"** | False. `CallToolRequest` carries a caller-supplied `session_id`; the route resolves *that* agent and dispatches through its manager, and `GET /sessions` enumerates ids. What the plan does instead: the route **declares Public** (`CallCapability::public_enforced()`), so naming a private session's id buys nothing. That is a **narrowing, not a closure** — it does not close "issue any tool call at all with the daemon secret" (AR-11), which is issue #47. | `routes/agent.rs:207-211`, `:1145`, `:1162`; `routes/session.rs:240` |
| **"Blocking `/config/upsert` makes the toggle Settings-only"** | False. `config.yaml` is an ordinary non-SecretGuard file that five tools can write; a public model edits it and the next daemon restart loads privacy tiers off. It joins DR-14's entries as a **file**. The HTTP channel to the same file is open question 24 and still unruled. | `secret_guard.rs:33`, `config_management.rs:174` |
| **Validating an output path as a permitted *directory*** | It constrains the argument and not the **filename the handler synthesises inside it**. Measured on `export_app`: a public model adds a top-level `config.yaml` to an Agent Drafter app (`update_app` accepts ordinary relative files), then calls `export_app` with `target_dir = <config directory>` — a directory the policy permits, because DR-14 denies the *file* `<config>/config.yaml` and deliberately allows its parent. The scanner sees only the permitted directory; the export preserves extra files and writes `target.join(rel)`, overwriting the master switch. ⚠ The plan's own test expected that target **refused** while its containment algorithm could match only the exact file or descendants of *directory* entries — **the test and the implementation were incompatible**, so whoever reached it first was going to "fix" one of them to agree with the other. Generalise it: a check on a path argument constrains that argument, never the handler's arithmetic on it. | `agent_drafter/mod.rs:2126`, `:2220`, `:2763`; `render.rs:939`; round 3 §4 |
| **A gate that prints a count and exits 0** | `grep -c '^MISSING' …; echo "expect: 0"` exits 0 whatever the count. **A gate a human must interpret is not a gate** — and this was the defect in the very task whose subject is *a command that reports success while doing nothing*. Every assertion must run through a `want`-style helper and the block must end `( exit "$rc" )`. | Plan Task 4b |
| **Gating a `cargo test` filter on "PASS"** | libtest prints `0 passed` and **exits 0** when a filter matches nothing. Every filter gate must assert a *count*, and where the module already has tests, a `pre + N` delta. `routes::agent` is **8** and `routes::session` is **20** on an untouched tree — "expect non-zero" is satisfied by a tree in which #56 added no route tests at all. | Plan §"Which test filters are validated" |
| **Keying a deferral allowlist on a filter *name*** | Excuses that filter in **every** package. `-p biorouter-mcp --lib privacy::refusal` names nothing and never will (`privacy/refusal.rs` is created in `biorouter`), yet it was reported DEFER because a filter of the same name was expected in a different crate. Key on the `(package, filter)` **pair**, and evidence every deferred row — searched over the plan **minus the table**, because otherwise each row witnesses itself inside its own heredoc. | Plan Task 4b |
| **Forbidding only the *trusting* literal in a provenance gate** | Hardcoding `Public` / `false` under-ratchets private callers and passes. Both directions must be forbidden, and each production caller needs a **behavioural** private/public matrix — a structural-only check is defeated by `caller_is_private: provider_name == Some("ollama")`, which keys on the requested name when `providers::create` can return something else. | Plan Tasks 10B, 11 |
| **A race test as N unconstrained spawns** | 200 spawns on a `current_thread` runtime under a conditional assertion that was **false for a correct implementation**. Replaced by forced interleavings behind `#[cfg(test)]` seams, with the seam placed *inside* the helper between the read and the write — a seam outside and before it passes a `SELECT` + unconditional `UPDATE` by refusing for the wrong reason. ⚠ Round 3 then defeated the *repaired* gate as well: put the required predicate in a **dead string**, write lowercase `select` plus an unconditional lowercase `update`, and place the seam before the hidden read — every case-sensitive structural check still passes. A structural check on SQL must assert the predicate is in the statement that **executes**, not that the text appears somewhere in the file. | Plan Task 12; round 3 §7 |
| **Verifying the master toggle against a hand-written list of enforcement points** | Round 2: the matrix carried ten rows and **omitted Gate F entirely** (it is implemented separately, in Task 18) and Task 23's **spawn matrix**; the structural inventory checked three symbols, so private-extension enablement, private server instructions and public-parent/private-child refusal could all stay armed with the feature "off" while every check passed. Round 3, against the *repaired* version, found four more defects: session-copy classification asserted identical on/off in a way that contradicts DR-15's own "no session-copy classification"; Layer B tested through an **undefined helper** rather than the spawned command; a refusal scanner demanding a **direct** toggle read while the prescribed central policy calls `PrivatePathPolicy::for_caller` and reads no toggle — so the closure rejected the very design it was closing; and the global atomic mutated with **no serialization and no RAII restoration**, leaving the matrix ending `false`, so parallel privacy tests disable one another. Replaced by two **inventory diffs**, the second starting from the **refusals**, so a gate nobody wired still fails. **A list you wrote is a list of what you remembered** — which is the same defect as the enumeration table above, one level up. | Task 30 across rounds 2 §1 and 3 §7 |

---

## Staging

Four stages. Value lands early; the part that has failed review three times is isolated at the end of
the enforcement work, where it can be re-planned without unwinding anything else.

⚠ **Stage 3 is now DESCOPED ([DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store)) and the isolation is what made that cheap.** Nothing in Stages 1,
2 or 4 has to be unwound. Stage 4 gains two task units: **29A** (the KB tier control, [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base)) and
**30A** (the disclosure, DR-17 requirement 3).

Within a stage, follow the plan's task units in order and honour the **non-negotiable orderings**
(O1–O16) — each one has a measured failure mode behind it, and O12/O13/O15/O16 are the ones that
decide whether a commit even compiles.

### Stage 1 — the tier model and the knowledge-base barrier

**Delivers:** the tier model and the knowledge-base barrier, which is the largest block of settled
*intent* in the plan — but read the next heading before you treat any of it as verified.

**Task units:** Phase 0 (Tasks 1–3), Phase 1 (Tasks 4, 4b, 5–9), and Phase 2's KB block (Tasks 10,
10A, 10B, 10C, 10D) plus Task 11 (Gate G).

⚠ **An earlier version of this brief called this whole stage "the parts Codex has confirmed sound".
That was false, and it is the most dangerous kind of error a brief can carry** — an implementer who
believes a gate is confirmed will not re-derive it, and three of these gates are known to pass a
wrong implementation. What Codex confirmed is a strict subset. It is listed exactly, with the round
that confirmed it, and everything else in the stage is **designed but unverified**.

#### Confirmed by review, and by which round

- **CP1 is valid** — round 1 (§1, "No defect found"). The hand-written
  `<KnowledgeServer as ServerHandler>::call_tool` is a faithful replacement of both
  rmcp-macro-generated methods: `ToolCallContext::new` is `pub`, `ToolRouter::call` is `pub`, and
  `CallToolRequestParams.arguments` is present before the tool body runs. A future incompatible rmcp
  change produces a **compile failure**, not a silent bypass. (`rmcp 0.14.0`, locked.) ⚠ Rounds 2 and
  3 did not re-examine it; if the lock moves off 0.14.0, this confirmation expires.
- **Task 10C — the KB pointer filtering (finding 2.2)** — round 2 (§4 and §5), re-affirmed round 3
  (§7). All three pointer fields filtered, the stored primary preserved, private and nonexistent
  targets made indistinguishable, error candidates filtered, private callers still working. This is
  the one gate a reviewer actively tried and **failed** to defeat: *"I could not construct the
  original pointer leak without failing the replacement tests."*
- **CP3 (finding 2.3)** — round 2 (§4), re-affirmed round 3 (§7). Both mixed main/worker directions
  are driven through the real socket, and only the mid-turn site is attributed to `turn_agent`.
- **CP5 (finding 2.4)** — round 2 (§4), re-affirmed round 3 (§7). Both global/manifest-provider
  inversions are exercised through `configure_agent` itself, plus the worker-specific grant.
- **The #57 child-environment strip** — round 2 (§3), "genuinely sound for the existing spawn paths
  the design relies on": foreground and background shells, `automation_script`, all three
  computer-control platforms, stdio/inline extensions, and the Agent Drafter smoke/esbuild children.
  This one is about code that already shipped, not a specification.

#### Designed but UNVERIFIED — round 3 built a passing wrong implementation for each

Treat every item here as work to re-derive, not work to transcribe. In each case the gate as written
runs green over an implementation the same round proved unsound.

- **Task 10A (the archive/provenance gate).** "Write outside, then move inside before returning"
  passes every assertion while opening a transient public-read window, and the user-route mirror test
  is invalid independently of that. See the [chain in the register](#the-killed-approaches-register).
- **Task 10B and Task 11 (Gate G, caller provenance).** A handler may call `paired`, ignore the tier
  it returns, and derive capability from the *requested provider name* — every structural count still
  passes. The CLI leg asserts `build_completer`'s returned tuple, not the handlers, and the plan
  concedes in its own text that no test would fail.
- **Task 10D (the metadata detector).** Function-item and direct-registry escapes; it is a drift
  tripwire, not a barrier proof, and the register now says so in those words.
- **Task 10 itself.** Round 3 found it mandates a **third** capability read, for built-in metadata —
  the sampling defect, inside the task that is supposed to establish the tier model.
- **Task 4b (the filter audit).** Round 3: the gate "treats any positive match count as success, so
  it still cannot enforce the claimed exact pre-counts". The baselines table below is only as good as
  the gate that checks it.

⚠ **A gap the plan admits and this brief previously buried: Task 10B covers `handle_ingest` with a
behavioural provenance row and leaves `handle_query`, `handle_lint` and `handle_ingest_conversation`
with none.** The plan states this deliberately — `handle_ingest` is "the one that writes content into
a base", and the other three are covered only structurally, by Step 5 (i) plus the ingest row's
existence as a pattern to copy. **But `query` writes too**, measured in
`crates/biorouter-mcp/src/knowledge/macros/query.rs`: its module doc says "optionally filing it as a
new knowledge page", `QueryArgs` carries `file_as_page: bool`, the sub-agent has `kb_write_page`, and
`commit_txn_if_a_page_was_filed` commits the transaction. So the stated reason — *the writer is the
one covered* — does not hold as written. Decide this consciously: add the three behavioural rows, or
record per handler why structural coverage suffices. Do not inherit it as settled.

**Depends on:** nothing outside `main`.

**Stop and re-plan if:** a sixth KB/content surface appears that none of CP1–CP5 covers (the review
sweep found none, but the sweep was over `KnowledgeService`, `store::*`, `kb_root`, `list_bases` and
`session_kb_ids` — a surface reached another way would be a new class); or if the ratchet's
`pre + N` arithmetic cannot be made to hold because Task 4b's measured baselines have drifted by more
than the tasks in this stage add.

### Stage 2 — capability threading

**Delivers:** `CallCapability`, sampled **once** at each of the four outermost entries and threaded
through Agent dispatch, the ExtensionManager policy, Gate C, built-in metadata and Layer B metadata.

**Task units:** Task 10's `CallCapability` half (it is written up there for ordering reasons), plus the
threading work O15 describes, plus Tasks 12–13 (Gates A and B) and 14 (Gate C).

**Why it is its own stage:** round 3 found the capability was sampled **four** times, not three, and the
master toggle three or four more. One call could pass Gate C with tiers ON and build an empty path
policy with tiers OFF. The attack is concrete: call A starts Public; the Agent-level policy samples
Public and passes because no literal denied path appears; the provider is swapped to Private across an
intervening `await`; the manager samples again, sees Private, and hands Layer B
`private_data_deny=false`. A's public-authored shell then runs unsandboxed.

**The shape that makes this checkable:** `capability_tier()` is **deleted as a shape**, not relocated.
The gate is a whole-tree count of absence (`grep -rn "capability_tier(" crates/` → 0), because that is
the only kind of gate that fails "threaded it but kept the sampler". `McpMeta` must be built **above**
the `async move` in `ExtensionManager::dispatch_tool_call` — inside it is execution time, on the far
side of an unbounded dispatch queue. `PrivatePathPolicy::for_call` must **not** re-read
`privacy_tiers_enabled()`.

**Depends on:** Stage 1's tier model (O1). **Conflicts with BR-71** — this is the stage to rebase.

**Stop and re-plan if:** a fifth production entry to a tool call turns up (the four are the agent loop,
`POST /agent/call_tool`, the `execute_code` bridge, and `Agent::call_prefetch_tool`); or if threading
the capability requires changing a signature that BR-71 is concurrently rewriting in an incompatible
way — in that case, rebase first and re-derive, do not thread twice.

### Stage 3 — the filesystem barrier ⛔ DESCOPED

> ⛔ **OUT OF SCOPE FOR v1 ([DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store)), 2026-07-30. Do not build this stage.** The operator descoped the
> general filesystem barrier: *"we don't have to enforce and encrypt every single step along the way.
> for now."* Its six task units carry the same banner in the plan and keep every measurement.
>
> **What that costs, stated:** requirement 1 (session logs and histories locked) is held on the
> **agent-mediated** channels only, so a public model with `developer__shell` can read `sessions.db`
> off disk. That is the accepted risk, and Task 30A's disclosure is the term on which it is accepted.
>
> **What it saves:** AR-6 is retired — a public-capability session keeps the shell on Windows and on
> bubblewrap-less Linux — and so are AR-9 and AR-10.
>
> **One thing here is NOT descoped.** *"CP1–CP4 are the knowledge root's door"* is the knowledge
> **tool** channel, and [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base) makes it a requirement (R16). It is Tasks 10A–10D in Stage 1, and it
> ships.
>
> Everything below is retained as the specification a revival starts from. The cheapest revival is
> Task 14B narrowed to one root — the session store — with Layer B left off.

**Delivers:** Layer A (the in-process path barrier at the dispatch choke point **and a guard in each
root's own resolver**) and Layer B (the OS read-deny sandbox, spawned children only).

**Task units:** 14A (Layer B mechanism), 14B (Layer A arguments), 14C (the daemon's own HTTP API),
14D (the readers that never reach the choke point), 14E (**the roots' own doors** — this is the one
that answers round 3), 14F (`export_app`'s write target).

**Be honest about this stage: it is the one that has failed review three times.** All three entries in
the enumeration table above are Stage 3's subject — round 4's escalation is Stage 4's, via DR-16.
Budget for a re-plan; do not budget for it going green first time.

**The two structural commitments, which are what round 3 bought:**

- **Layer A is not one check.** It is a check at the choke point (for caller-supplied paths) **plus a
  guard in each root's own resolver** (for handler-supplied paths). The roots' doors, measured:
  `ArtifactStore::dir` is already private (`agent_drafter/store.rs:447`);
  `MemoryRouter::get_memory_file` is private (`memory/mod.rs:434` on `main`); the session store is a
  sqlx pool nobody outside `session/` should hold. **Knowledge has no such door** —
  `resolve_readable_path` (`knowledge/store.rs:121`) has **3** call sites against roughly 40 direct
  filesystem reads in the same module, and `KnowledgeService::root()` is `pub`
  (`service.rs:415`) because `routes/knowledge.rs` legitimately joins off it at 7 sites. So CP1–CP4
  are the knowledge root's door, and a grep is what stops an eighth reader appearing beside them
  (open question 22, accepted).
- **The coverage gate must be phrased as a choke point, not a list.** The gate that makes "every tool
  call" checkable registers an in-process server **at test time** and asserts its invented
  `read_thing(path)` tool is refused. No list-based implementation passes it. A "surprise tool" test
  that supplies the full **absolute** denied path does not count — both implicit-root defects
  (`read_app`, `export_app`) pass that one.

**Depends on:** Stage 2 (Layer A reads the threaded `CallCapability`; Layer B carries it in `_meta`).
O16: Task 14E lands with or after 14B and before the Phase 2 gate; 14F depends on 14E.

**Known-open at the end of this stage, and accepted** — ⚠ **all three of these are now RETIRED by
[DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store), because the stage they belong to is descoped:** AR-9 (the symlink swap window — closed at the
open on both platforms via `O_RESOLVE_BENEATH`/`O_NOFOLLOW_ANY` on macOS and `openat2(RESOLVE_BENEATH)`
on Linux, both measured refusing a symlink traversal and a `..` escape; a **pre-planted hardlink**
still defeats Layer B, because both kernels match paths, not inodes); AR-10 (on Linux a deny root that
does not exist when a job starts stays visible to that job for its whole life); AR-6 (on a host that
cannot express the kernel deny a public session loses the five **spawning** tools).

**Stop and re-plan if:** a fifth *class* of read appears — anything that is neither a caller-supplied
path at the choke point nor a handler-supplied path at a root's resolver. That is the shape of all
four previous failures and it is the signal that the architecture, not the list, is wrong.

### Stage 4 — the master toggle, the user-only tier raise, and the UI

**Delivers:** DR-15's master toggle, DR-16's user-only tier raise, the badges, declassification, the
CLI surface, the marketplace, and the migration.

**Task units:** Phase 3 (21–25), Phase 4 (26–32) **including the two new ones, 29A and 30A**,
Phase 5 (33–37), Phase 6 (38–40) — **plus a task that does not exist yet**.

⚠ **Task 30A is not optional and must not be folded into Task 30.** It is DR-17 requirement 3 — the
disclosure that makes the ruling's accepted risks a considered tradeoff rather than an omission — and
it is the one privacy surface that must work with the master toggle **off**. Task 32's phase gate
asserts it. ⚠ **Task 29A depends on Task 18A's user-proof** ([DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base): the KB tier is user-only in
both directions), so where 18A is unresolved 29A fails closed rather than inventing a second
mechanism.

⚠ **DR-16 is a settled ruling with no task written for it.** The ruling names `Task 18A: the two HTTP
channels that raise a session's own tier, and the user-proof neither of them has`. That task must be
written before it is implemented, and writing it requires resolving two recorded open questions:

- **Open question 23** — what should a daemon that was handed no user-action key do? Four shipping
  launch paths supply none (`just run-server`, `just debug-server`, a hand-run `biorouterd agent`, any
  headless deployment). Failing closed there means a human at the keyboard cannot bind a private model
  over HTTP on the dev path this repo uses daily. An env-var escape hatch is **not** an alternative —
  AR-11 measured the daemon's environment to be recoverable, so a key delivered that way is a key the
  model has.
- **Open question 24** — does DR-16 extend to `POST /config/set_provider` and `POST /config/upsert`,
  which write `BIOROUTER_PROVIDER`, after which `restore_provider_from_session` falls back to it and a
  new session comes up private-capability with no `update_provider` call at all? DR-14 makes
  `config.yaml` a deny entry on the *filesystem* channel precisely because *"a master switch a public
  model can edit is not a switch"*; `/config/upsert` is the **HTTP** channel to the same file and no
  layer of DR-14 sees it.

**The toggle's own hazard:** DR-15's matrix covers **twenty** rows — ⚠ **eighteen under DR-17, which
removes Layer A's and Layer B's rows with the deferred stage** — nineteen enforcement points plus
the session-copy invariant — and an earlier version covered ten. Gate F's two channels, the spawn
matrix, Layer A's insertion points, the catalog, the export location and the visibility predicate could
all stay armed with the feature "off". It is closed at both ends by two inventory diffs, the second of
which starts from the **refusals**, so a gate nobody wired still fails. A crate-graph fact the plan
found late and that constrains the implementation: **`biorouter-mcp` cannot see `biorouter`**, so the
enabled-flag atomic lives in `biorouter-mcp` with a `biorouter` re-export.

**Depends on:** Stages 1–3 for everything it renders and everything it disables.

**Stop and re-plan if:** open questions 23 or 24 cannot be answered without an operator ruling — they
cannot, which is why they are questions. Raise them; do not invent an answer inside Task 18A.

---

## The verification regime

**A green suite is necessary and not sufficient.** Say it once and then behave as if it is true —
but note *why* it is true here, because the obvious argument is not available. This campaign has
produced no #56 code, so nobody can claim a green suite hid a defect in it. The real evidence is
about **gates**, and it is worse: three rounds found gates that *could not fail the wrong
implementation they named*, and in each case the same round then constructed that wrong
implementation. Those gates would have run green over a design already proved unsound.

A suite proves you did not break what exists. **A gate proves nothing at all unless you can name the
wrong implementation it rejects** — which is why item 5 of [the reviewer's
checklist](#the-reviewers-checklist) asks you to name one for every gate you write.

### Half one — the mechanical regime

Per task, per the plan: failing test first, implementation, gate, one commit. Per phase, the phase
gate (Tasks 3, 9, 20, 25, 32, 40).

**Baselines, measured (Task 4b Step 3, run against `main` at `89c1f026`).** These are the numbers every
`pre + N` assertion in the plan is arithmetic on. **Re-measure rather than trusting them** — a stale
figure is worse than none, because a "pre + N" assertion against it reads a shortfall as a pass.

```bash
# The five packages the plan filters on, plus biorouter-sandbox (the sixth, added late).
for p in biorouter biorouter-server biorouter-mcp biorouter-cli biorouter-sandbox; do
  cargo test -p "$p" --lib -- --list 2>/dev/null | sed -n 's/: test$//p' | sort > "/tmp/56-filters/$p.txt"
  echo "$p: $(wc -l < /tmp/56-filters/$p.txt) tests"
done
```

| Filter | Measured | Why it matters |
|---|---|---|
| `-p biorouter-mcp --lib knowledge::` | **190** | The plan once said "~122". A "pre + N" against 122 reads a shortfall as a pass. |
| `-p biorouter-mcp --lib memory::` | **10** | ⚠ *not* 12. The trailing `::` excludes two tests. The plan uses **both** spellings; they are different filters. |
| `-p biorouter-mcp --lib secret_guard::` | **19** | ⚠ *not* 20, same reason. "Expect the SAME count as before" is 19. |
| `-p biorouter --lib agents::extension_manager` | **37** | `::tests` is 33; the filter also catches `extension_manager_extension::tests` (4) by substring. |
| `-p biorouter --lib agents::agent` | **21** | Three test modules, none discoverable from the filter (`::tests` 14, `::rewrite_basis_tests` 2, `::stall_seam_tests` 5). |
| `-p biorouter-server --lib routes::agent` | **8** | Untouched baseline. Assert **strictly more**, never "non-zero". |
| `-p biorouter-server --lib routes::session` | **20** | Untouched baseline. Same rule. |
| `-p biorouter --lib agents::chatrecall_extension` | **0** | Genuinely zero — no `#[cfg(test)]` at all. Here a non-zero count *is* the assertion. |
| `-p biorouter --lib session::chat_history_search` | **0** | Same. |
| `-p biorouter-server --lib routes::apps` | **90** | |
| `-p biorouter-mcp --lib agent_drafter::` | **244** | |

41 `(package, filter)` pairs resolve today with a measured pre-count; 18 are deferred to the task that
creates them. **A filter in neither set is the BR-71 defect** — a command that prints `0 passed`, exits
0, and has been reported as verification for forty tasks. Task 20 re-runs the audit with a shrinking
deferred set; Task 40 re-runs it with an **empty** one.

**Release-gate commands** (Task 40): the full workspace suite, `cargo fmt --check`,
`./scripts/clippy-lint.sh`, `just check-everything`, `npx tsc --noEmit`, `npm run lint:check`,
`npm run test:run`, `node scripts/check-contrast.mjs`, `npm run themes -- --check`, and the named
integration targets. **Two expected pre-existing failures**, to be verified on a clean checkout before
dismissing: `providers::test_anthropic_provider` (calls the live API, fails on billing) and the
`SessionListView.test.tsx` isolation flake.

⚠ **`check-contrast.mjs`'s expected total is wrong in the plan. The corrected post-#56 target is
`OK — all 324 contrast assertions pass`**, derived and measured in
[how to read the material](#how-to-read-the-material): **288** on `main` today (48 per scope × 6
scopes) plus Task 26's **36** (2 hover-ground + 4 badge, per scope). The plan's `288` is the *pre*-#56
baseline, so as written the gate cannot fail. Re-measure the baseline on your own branch point before
asserting anything. The diagnostic reasoning in the plan still holds and is worth keeping: a total
*higher* than expected means `--background-medium` moved into `TEXT_GROUNDS` (and that run does not
print `OK` at all — three of its assertions fail AA at 3.75 / 4.45 / 4.28, and it exits 1); a total
short by exactly **10** or **20** means one of the two new blocks landed outside the per-scope loop and
ran once instead of six times (12 − 12/6 = 10; 24 − 24/6 = 20). Both wrong numbers read to a worker as
"the phase failed" when the phase succeeded, which is why the plan has quoted three different totals
and got two of them wrong.

⚠ `cargo test -p biorouter-mcp --test mcp_integration_test` is a cargo **hard error** — the file is
`crates/biorouter/tests/mcp_integration_test.rs`. Two invocations in an earlier draft had this wrong.

### Half two — the by-hand GUI scenarios

Nothing above renders a badge or clicks a button. These are run by hand, and they are the half that
catches the class of defect a jsdom test cannot see.

**Before launching anything, read
[`docs/desktop-ui/launching-the-dev-gui.md`](../desktop-ui/launching-the-dev-gui.md).** Five distinct
launcher failures produce symptoms that read as application bugs:

- `env -u ELECTRON_RUN_AS_NODE` — agent shells export it and Electron then exits instantly, no window,
  no error.
- Do **not** use `electron-forge start` — it reads stdin, so `< /dev/null` takes the app down.
- Pass `--config vite.renderer.config.mts` — a bare `npx vite` skips Tailwind and renders unstyled
  serif HTML that is fully functional.
- Set `BIOROUTER_NO_HMR=1` — any save under `ui/desktop/src` full-reloads the renderer and destroys
  the session under test.
- Verify with a **CDP screenshot**, never `screencapture` of the whole screen.

**Sandbox the config: `XDG_CONFIG_HOME=/tmp/privacy-check`.** Launching the dev GUI can wipe
`~/.config/biorouter`.

The scenarios, each with a screenshot as evidence:

1. A private chat's badge in **History, the chat header, the tab, and the sidebar** — four surfaces,
   all four checked.
2. Switching a private chat to a public model shows the Gate A repair card and **no success toast**.
   This is O3: `ui/desktop/src/components/ModelAndProviderContext.tsx:282` calls
   `updateAgentProvider` *without* `throwOnError` while `setConfigProvider` at `:294-300` has it
   (`:299`), so a Gate A refusal is discarded, execution continues, and a green success toast
   claims the switch worked while the session is still bound to the private model. **Gate A alone is
   worse than no Gate A.**
3. A private extension in the composer's selector is **visible-but-disabled with its reason** — not
   hidden, not silently failing.
4. The declassification dialog's phrase gate, Cancel focus, and the resulting *"Public — made public by
   you on …"* badge.
5. A full private → public declassification, end to end.
6. With the master toggle **off**: badges still render, restyled and suffixed *— enforcement off*,
   beside a persistent strip. A guardrail that vanishes when disabled cannot be noticed by the person
   who disabled it six months ago; a badge that still reads plain **Private** while nothing enforces it
   is a false statement.

Use the Versa models per standing policy (`versa_azure` / `gpt-5.5-2026-04-24`, and `versa_bedrock`
Opus 4.8); a local model only when local-model behaviour is itself under test. The operator's own
config has `cdwagent` and `ucsfomopagent` **disabled**, so seed a config in which something would
actually be refused — otherwise every scenario passes vacuously.

---

## Subagent guidance

**Where parallelism helps:**

- **Independent measurement.** "Sweep every caller of X and classify it" is the single highest-value
  subagent task in this campaign — it is what found `resolve_target_kb`, the third instance of a defect
  two detectors were blind to. Give it a symbol, a directory, and a demand for `file:line` evidence.
- **Adversarial review of a finished stage diff.** Independent, read-only, no shared context with the
  implementer. This is what has caught every real defect so far, and it should run at every phase gate,
  not only at Task 40 Step 5.
- **Phase 5 (`landing/`, Tasks 33–37).** O11: it is genuinely independent and may ship on any cadence.
  Enforcement runs off the compiled-in const, so the website blocks nothing.
- **Frontend work in Phase 4** (Tasks 26–28) against a backend that already exists.

**Where it actively hurts:**

- **Splitting one enforcement point across agents.** Every failure in the register above is a *seam*
  defect — a right value read in the wrong place, a check on the wrong side of a bind, a sampler left
  behind after threading. Seams are exactly what is lost when two agents each hold half a control.
  Stage 2 and Stage 3 are single-owner work.
- **Parallelising tasks that share a signature.** O13 exists because a previous draft left **nine
  consecutive commits** failing `cargo test`: one task changed three struct signatures and the only
  out-of-lib constructor was in no Files table, no `git add` and no run step. A task that changes a
  `pub` signature must run `cargo check --workspace --all-targets`, and every out-of-lib constructor of
  a changed type is a row in its Files table.
- **Running several stages' test suites at once.** Measured OOM hazard; see
  [where to work](#two-hazards-measured-in-this-campaign).
- **Trusting a subagent's count.** Ask for the command and its output, not the number. Three of the
  measured corrections in this campaign — `knowledge::` 190 vs 122, `memory::` 10 vs 12,
  `secret_guard::` 19 vs 20 — were counts a reasonable agent would have reported confidently.

---

## The reviewer's checklist

What the checking pass will verify. Pre-empt it.

1. **No enumeration.** Is any control phrased as a list of tools, servers, or argument names? Is the
   coverage gate phrased as *"every tool call passes through symbol X"*? Does a test-time in-process
   server with an invented path-taking tool get refused?
2. **One sample, threaded.** `grep -rn "capability_tier(" crates/` → 0. Is `McpMeta` built above the
   `async move`? Does `PrivatePathPolicy::for_call` re-read the toggle? Is there any path on which one
   call is evaluated under two different capabilities?
3. **The roots' doors.** Does every root have a resolver that enforces, or a documented reason it does
   not? Are `read_app`, `export_app`, `list_apps` and `computercontroller__cache` covered — the four
   that defeated round 3?
4. **Ordering.** Every read-deny emitted **after** the allow it subtracts from; every capability check
   **before** the early return it would otherwise sit behind. O14 names four such places and all four
   failures are silent: SBPL last-match-wins, bubblewrap later-option-wins, the `SandboxMode::Off` early
   return in `shell_sandbox_wrap`, and the `if jail_relaxed { return … }` in `resolve_path_jailed`
   (relaxed **is** Auto mode, the mode agents run in).
5. **Every gate can fail.** For each gate, name a plausible wrong implementation and show the gate
   rejecting it. Does the block exit non-zero? Does a `cargo test` line assert a count and not a pass?
   Is a deferral keyed on the `(package, filter)` pair? Is a provenance check two-directional?
6. **Deviations recorded.** Every place the plan's prescribed code did not compile, or its anchor did
   not resolve, is written into the task in the same commit. Silent fixes are a finding.
7. **The toggle's twenty rows** (eighteen under DR-17). Every enforcement point is in the matrix or the inventory, and session
   copy is asserted **identically in both columns** (DR-15: propagating a stamp is not classifying).
8. **The accepted risks are still the accepted risks.** AR-1 through AR-15 are rulings, not bug
   reports. A PR that closes one silently has changed a decision; a PR that *widens* one has introduced
   a defect. Either way it must say so. ⚠ **And a risk that has been closed must stop being described
   as open** — AR-1, AR-6, AR-9, AR-10 and AR-15 are all off the list now, each carrying a
   RESOLVED / RETIRED marker naming what closed it. A security document that overstates a weakness is
   wrong in the same way one that understates it is: it spends the reader's trust in the entries that
   are accurate.
9. **Commit hygiene.** Conventional commits, no `Co-Authored-By` trailers, one commit per task, nothing
   pushed.
10. **Docs.** Anything new under `docs/` carries the context header, sentence-case headings, a
    kebab-case filename and a `## Related documentation` close, and is indexed in
    [`docs/security/README.md`](README.md) and [`docs/organization.md`](../organization.md).

---

## Related documentation

- [Privacy tiers](privacy-tiers.md) — the design this brief fronts: the two lattices, the capability
  matrix, the five gates, and the cost.
- [Privacy tiers — implementation plan](privacy-tiers-execution-plan.md) — the fifty-one task units,
  the non-negotiable orderings, the decisions of record, the accepted risks and the open questions.
- [Multi-KB implementation plan](../knowledge-base/multi-kb-implementation-plan.md) — the "one axis,
  one pointer" visible-set model whose explicit-`kb_id` escape hatch the knowledge-base barrier
  qualifies.
- [Documentation style guide](../contributing/documentation-style.md) and
  [documentation organization](../organization.md) — both binding on anything this work adds to `docs/`.
- [Data privacy and patient data](data-privacy-and-phi.md) — the provider guidance this system enforces
  mechanically.
- [Secret storage](secret-storage.md) — the credential model, and the environment strip the review
  rounds confirmed sound.
- [BR-71 execution plan](../agent-loop/designs/br71-execution-plan.md) — the concurrent work Stage 2
  conflicts with.
- [Launching the dev GUI](../desktop-ui/launching-the-dev-gui.md) — required reading before any by-hand
  verification step.
