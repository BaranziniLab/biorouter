# Process-global state in the Rust workspace

> **What this is.** The audit of every process-global value the `biorouter` crates read and write — environment variables, `Paths::*`, first-read-freezes statics and mutable registries — with a ledger of the places where a production reader can observe another test's write.
> **Status:** Current. The ledger is open: some rows are fixed, some are accepted, and some are still to do. Every count here was measured on 2026-09-09 against `main` at `f350cdfc`; re-measure with the recipes below rather than trusting a number.
> **Audience:** developers and agents working on the Rust crates, especially anyone diagnosing a test that fails in a subsystem their change never touched.

Four CI flakes fixed on 2026-09-08/09 shared one shape, and it is not the shape people look for. A test writes a process-global value and holds the right lock while doing it. The value is then read by *production* code that never asks for that lock. The lock is correct, the writer is correct, and an unrelated test fails anyway.

`env_lock::lock_env` and `serial_test::serial(key)` serialise only the callers that ask. So the natural fix — add a lock to the failing test — finds the lock already there and dead-ends. This page exists so the next person starts from the reader instead.

## The rule

> **A production reader of process-global state takes its value as an explicit input, sampled once by a caller that owns the decision.**

Everything else follows. A test then states its inputs instead of inheriting them, no lock is needed, and the value cannot change under a reader mid-decision.

Two remedies exist and **they are not interchangeable**:

| The reader resolves through | Remedy | Why |
|---|---|---|
| `Config::get_param` / `get_secret` | `config::with_config_overrides` | `get_param` consults the `CONFIG_OVERRIDES` task-local *before* the environment, so the override wins for the setting task and is invisible to every other one |
| a bare `std::env::var` | pass the value as an argument | the task-local is never consulted, so an override is a silent no-op |

⚠ **Getting this backwards produces a fix that does nothing and looks like one that works.** `agents/subagent_tool.rs` inserts `BIOROUTER_ALLOW_PROJECT_HOOKS` into a `with_config_overrides` map to unlock project hooks, but `hooks/mod.rs` reads that key with a bare `std::env::var`. The override never reaches it. The test still passes, because the refusal it asserts holds in both states — so nothing has ever reported it.

## What is banned, and what is not

- **No new locks.** `env-lock` is a single global mutex over the whole environment. Adding a holder serialises writers against writers, which was never the failure mode, and does nothing about the unlocked readers that are.
- **No new `#[serial]`.** An *unkeyed* `#[serial]` is worth even less than it looks: it excludes a test from the 31 other unkeyed ones and leaves it concurrent with the rest.
- **A source-scan guard beats a stress run.** The race needs an interleaving CI produces and a loaded laptop may never show, so twenty green runs are weak evidence. A source scan cannot flake. Give every scan a non-vacuity floor and prove it against a deliberately poisoned probe before trusting it.
- **Writing the environment is not banned outright.** A key that is *test-private* — namespaced so no production reader can resolve it — and restored on drop is safe without any lock, because there is no reader to race.

## Why this document exists rather than a fifth per-key guard

The audit was deferred by name in [PR #198](https://github.com/BaranziniLab/biorouter/pull/198), the last of the four fixes:

> The guard is scoped to `BIOROUTER_SESSION_BLOB_LAZY_LOAD`. `model.rs`'s `no_test_poisons_a_shared_setting` covers five more keys on a different principle (invalid values only). Every *other* config key an unguarded reader resolves live is uncovered; a general "no test writes a config key an unlocked reader consults" audit is a larger piece of work than this fix.

It is the only one of the four that defers the general audit. #188 and #197 record follow-ups about config I/O, and #193 has no follow-ups section at all.

## The blind spot that sets the size of the problem

`Config::get_param` (`crates/biorouter/src/config/base.rs:1094`) resolves in three rungs — the task-local override, then `std::env::var` of the upper-cased key (`:1102`), then the config file. **Every config key is therefore also an environment key.** The measured surface:

| Measure | Count |
|---|---|
| `.get_param` call sites | 195 |
| `.get_secret` call sites | 50 |
| Distinct literal config keys | 141 |
| Call sites whose key is not a literal | 43 |

No literal-key scan can see any of those 43, and no scan of `env::var(` can see any of the 195. A guard's key table is a list of *measured hazards*, never a closed set.

## Inventory

All counts are static, measured 2026-09-09 over `crates/biorouter/src` unless a row says otherwise.

### Environment reads

| Shape | Count | Notes |
|---|---|---|
| Literal `env::var("KEY")` | 88 sites, 58 distinct keys, 48 files | 75 production, 13 test |
| `env::var(CONST)` or a computed key | 21 sites | invisible to any literal-key scan |
| `Paths::*` calls | 99 sites, 46 of them `config_dir()` | every one re-reads `BIOROUTER_PATH_ROOT` |

`Paths::get_dir` (`crates/biorouter/src/config/paths.rs:7`, the read at `:18`) has no cache, no static and no lock, so **`BIOROUTER_PATH_ROOT` is re-read on every `Paths::` call**. Against that sit 33 locked test writers of the same key. `managed_policy_path()` (`:113-114`) reads the key directly and so lacks `get_dir`'s blank-value filter.

The four highest-traffic literal keys are `BIOROUTER_PATH_ROOT` (6 sites), `PATH` (7), `HOME` (6) and `BIOROUTER_CONTEXT_LIMIT` (2).

### Production code that writes the environment

Five sites, and they are not a test concern — they mutate the environment of a multi-threaded daemon.

| Site | What it writes |
|---|---|
| `providers/bedrock.rs:79` | every `AWS_*` config value **and secret**, from `from_env` |
| `providers/bedrock.rs:90-92` | an unlocked read-then-write promoting `AWS_ENDPOINT_URL_BEDROCK` |
| `providers/sagemaker_tgi.rs:52` | the same `AWS_*` dump |
| `config/base.rs:1504` | `BIOROUTER_DISABLE_KEYRING=1` on keyring fallback; races `Config::default`'s read at `:225`, which `GLOBAL_CONFIG` then freezes |
| `agents/test_sandbox.rs:37` | safe twice over — a `#[ctor]` that runs before `main`, in a module gated at its declaration site |

The two `AWS_*` dumps leak credentials into every subprocess spawned afterwards, which is the exact unsoundness the `CONFIG_OVERRIDES` task-local was introduced to avoid.

### Test writers

| Shape | Count |
|---|---|
| `env_lock::lock_env` call sites | 67, over 26 named keys plus one dynamic (`BIOROUTER_PATH_ROOT` alone: 33) |
| `temp_env::` call sites | 3, all in `config/base.rs` |
| Bare `set_var` / `remove_var` in tests | 22 sites |
| `#[serial…]` attributes | 129 real: 98 keyed, 31 unkeyed |
| `Config::global().set_param` in tests | 0 — already the right pattern |

> **Note.** A grep for `#[serial` returns 138. Nine of those are `#[serial_test::parallel]`, which asserts the opposite.

### Statics frozen at first read

Nine were verified, and **none has a `#[cfg(test)]` reset hook**: `model.rs` `PREDEFINED_MODELS`, `observability/phase_timing.rs`, `observability/loop_safety.rs`, `agents/tool_dispatch_limits.rs` `TOOL_SEMAPHORE`, `agents/subagent_tool.rs` `SUBAGENT_SEMAPHORE`, `config/base.rs` `GLOBAL_CONFIG`, `session/session_manager.rs` `SESSION_STORAGE`, `config/permission.rs`, `execution/manager.rs`.

These are order-dependence, not tearing, so **no lock can fix them**. The remedy is an explicit input or a test-only reset. `loop_safety`'s static freezes a *config* read, which by the rung order above is an environment read.

### Mutable registries

| Registry | State |
|---|---|
| `USER_PROOF_AVAILABLE` (`crates/biorouter/src/pending_user_action.rs:346`) | 6 unsynchronised production readers in the lib. The only production writer is the daemon (`crates/biorouter-server/src/commands/agent.rs:136`); the only test writer is `crates/biorouter/tests/bug_report_agent_loop.rs`, a **separate binary**. Latent in `--lib`, not live. |
| `privacy::crossing::CROSSED` (`:51`) | genuinely mutex-guarded, so not this shape. Two `workspace_extension.rs` tests call `reset_for_test()` outside that mutex, and `reset_for_test` is `#[cfg(test)]`, so `tests/workspace_crossing_disclosure.rs` can write the ledger but never clear it. |

## The ledger

Verdicts: **fixed**, **live** (a reader can observe another test's write today), **latent** (the same shape, no writer or no sensitive reader today), **accepted** (deliberate, with the reason).

| State | Reader | Writer | Verdict |
|---|---|---|---|
| `BIOROUTER_MAX_TOKENS` | `ModelConfig::new` via `get_param` | the config layer itself | **fixed** — PR #188 |
| `config.yaml` via a shared `config.tmp` | `load_values_with_recovery` | every thread in a start-up storm | **fixed** — PR #197 |
| `BIOROUTER_PATH_ROOT` at write time | `AgentManager::new`'s spawned seeding | any test holding `env_lock` | **fixed for the spawned seeders** — PR #193 |
| `BIOROUTER_SESSION_BLOB_LAZY_LOAD` | `Agent::platform_tool_gates` via `get_param` | two `#[serial]` tests | **fixed** — PR #198 |
| `TEST_KEY`, `API_KEY`, `PROVIDER`, `PORT`, `ENABLED`, `CONFIG`, `TEST_PRECEDENCE` | none in production today; `get_param` upper-cases, so any `get_param("provider")` would resolve one | 11 unguarded bare `set_var` in 3 `config/base.rs` tests; `TEST_KEY` was never removed at all | **fixed** — namespaced to `BIOROUTER_TEST_CONFIG_*` and restored on drop |
| `OSV_ENDPOINT` | `OsvChecker::new` (`agents/extension_malware_check.rs:18`), reached in production from `extension_manager.rs:972` on every Stdio extension install | 3 tests, RAII-restored but unkeyed `#[serial]` | **live** |
| `BIOROUTER_TOOL_CALL_BATCHING` | `providers/base.rs:1236`, bare `env::var`, once per streamed turn | `formats/anthropic.rs:1745` under `#[serial(tool_call_batching_env)]`, a key only 3 tests hold | **live** — 5 flag-sensitive tests hold no key |
| `BIOROUTER_ALLOW_PROJECT_HOOKS` | `hooks/mod.rs:214`, bare `env::var` | `providers/bedrock.rs:1147`, **never removed** | **latent** — no `.biorouter/hooks.yaml` in-tree, so no reader is sensitive today |
| `BIOROUTER_ALLOW_PROJECT_HOOKS` override | `hooks/mod.rs:214` | `agents/subagent_tool.rs:5663` via `with_config_overrides` | **live defect, not a race** — the override is a no-op, so that arm does not test its own unlock |
| `HOME` | `security/policy/command.rs:846`, `policy/target.rs:125` | `knowledge/conversation_ingest.rs:806` | **latent** — `global_memory.rs` is already mitigated by `pinned_store_root()` |
| `BIOROUTER_PATH_ROOT` (the general case) | ~46 live `Paths::config_dir()` readers | 33 `lock_env` writers | **open** — deferred, see below |
| `CLAUDE_THINKING_ENABLED`, `CLAUDE_THINKING_BUDGET` | 6 bare `env::var` sites on the request-format path | none in `crates/` | **latent** |
| `BEDROCK_OPERATION_TIMEOUT_SECS` | `get_param` first, then env | none | **accepted** — the mitigated shape, and the model the two live rows should follow |
| `pending_user_action::USER_PROOF_AVAILABLE` | 6 lib readers | none in `--lib` | **latent** |
| `SkillsClient::new` resolving `Paths::config_dir()` | `agents/skills_extension.rs:807` | — | **accepted** — PR #193 calls the synchronous constructor read "the property we want": the root a client seeds into is the one that was ambient when it was built |
| `AWS_*` written by production | any `env::var` reader in the process | `providers/bedrock.rs:79`, `sagemaker_tgi.rs:52` | **open** — not a test hazard; recorded here because it is the same mechanism |

### `<config>/skills`, spelled eleven ways

`skills_extension::skills_root(config_dir)` (`:228`) exists and has three callers. Independently, `Paths::config_dir().join("skills")` is spelled eleven more times — nine in production — and only a doc comment ties them together.

Production: `agents/skill_catalog.rs:155`, `agents/skills_extension.rs:379`, `:512`, `:830`, `agents/skill_package/install.rs:75`, `:85`, `:278`, `biorouter-cli/src/commands/skill.rs:38` (a second `fn skills_root`), `biorouter-cli/src/session/completion.rs:86`. Tests: `skills_extension.rs:4291`, `:5700`, `knowledge/conversation_ingest.rs:666`.

⚠ `skills_extension.rs:830` is **not** a second read inside `SkillsClient::new`, as the follow-up note claimed. It is `add_missing_shipped_skills`, a separate `pub(crate) fn` reached from `SkillCatalog::scan` — so the read lands on a later, unowned code path, which is the hazard `:807` is accepted for *not* being.

### Deferred: the `Paths::config_dir()` readers

The ~46 live `config_dir()` readers are **not** attempted here. Each needs its own judgement — is this read synchronous and owned by the caller, like `SkillsClient::new`, or does it land on a detached path, like the seeders `knowledge::soul` fixed? A blanket rewrite would be wrong in both directions. The per-site judgement is still to be made.

## Re-measuring

Every count above is static and will drift. These are the recipes, run from the repository root.

```bash
grep -rn 'Paths::[a-z_]*(' crates/biorouter/src --include='*.rs'
```

```bash
grep -rn 'env::var\(_os\)\?(\s*"' crates/biorouter/src --include='*.rs'
```

```bash
grep -rn 'std::env::\(set_var\|remove_var\)' crates/biorouter/src --include='*.rs'
```

```bash
grep -rn 'config_dir()' crates/ --include='*.rs' | grep 'join("skills")'
```

> **Warning.** Grading a site as production or test by its position relative to the file's own first `#[cfg(test)]` misgrades every module gated at its *declaration site* — `#[cfg(test)] mod tests;` in a `lib.rs` or `mod.rs` — because the file it names contains no `#[cfg(test)]` of its own. `agents/test_sandbox.rs` is one such file, and the rule calls its environment writes production.

## Traps when writing a guard for any of this

- **Slice on the first `#[cfg(test)]` only where the convention holds.** It does in `model.rs` (881), `agents/platform_tools.rs` (569) and `knowledge/soul.rs` (1004), where the first `#[cfg(test)]` opens a test module that runs to the end of the file. It does **not** in `agents/skills_extension.rs`, whose first `#[cfg(test)]` is an inline attribute on a function at line 744, so a slice there discards about 2300 lines of production code and reports a false clean. It does not in `providers/base.rs` either: four test modules, with production code between them.
- **Strip comments before matching.** A guard that explains the shape it forbids will otherwise report itself as its own first offender. `model.rs`'s scan does *not* strip them and survives only because its comments spell placeholder keys.
- **Give every key its own non-vacuity floor.** A single global floor lets most rows rot silently behind the one key that is still written.
- **Count file mentions, not matches.** A presence-judged key is healthy at zero matches, so a floor on matches is unsatisfiable for exactly the rows that most need one.
- **Insert a test after the previous test's closing brace**, never anchored on a `fn` line — an insertion between `#[test]` and its function silently unregisters the neighbour, and cargo says nothing.

## Related documentation

- [How this documentation is organized](../organization.md) — where a document like this one belongs
- [Secret storage](../security/secret-storage.md) — the credential path two of the production environment writers above reach into
- [System overview](../architecture/system-overview.md) — the crate boundaries the audit walks
