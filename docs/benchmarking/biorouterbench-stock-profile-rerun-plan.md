# BioRouterBench stock profile rerun plan

> **What this is.** The concrete plan for rerunning BioRouterBench 1.1.0 with Biorouter in its
> shipped configuration (stock system prompt, default extension roster, compaction on) beside
> Codex and Claude Code, and for comparing Biorouter 1.94.5 with 1.94.6 on the same footing. It
> is the deliverable of issue [#541](https://github.com/BaranziniLab/biorouter/issues/541).
> **Status:** Current. Proposed on 2026-11-19 and not yet run: no number in this document is a
> result of the rerun. Code facts were read at the `v1.94.5` tag; model ids and prices were
> checked against OpenRouter's public catalog on 2026-11-19.
> **Audience:** whoever runs, reviews or pays for the rerun: benchmark operators, Biorouter
> maintainers, and agents picking up #541.

BioRouterBench 1.1.0 compared three agent harnesses on 62 bioinformatics tasks. Codex and
Claude Code kept their native prompts and tools, while Biorouter 1.93.4 ran a minimal profile: a
272 character system prompt, one tool (`developer__shell`), compaction off and its loop helpers
off. Biorouter scored below both in 22 of 24 paired contrasts, and the harness review in
[#534](https://github.com/BaranziniLab/biorouter/issues/534) found that no arm varied that
profile, so nobody can say how much of the gap is the profile and how much is the product. This
plan defines the run that separates the two, and uses the same run to measure what the 1.94.6
fixes (#535 to #540, #542 to #549) change.

## Terms used here

- **1.1.0** is the published BioRouterBench 1.1.0 study: Biorouter 1.93.4, Codex 0.157.1,
  Claude Code 2.1.296, one judge (Claude Opus 5.5).
- **P01 to P15** are the proposals of the harness review, filed as issues #535 to #549 under the
  tracking issue #534. This plan is P07 (#541).
- **Configuration** is one model at one effort level, for example GPT-6 Luna at low.
- **Block** is one task, configuration and rep. Every arm runs every block of its grid, so arms
  are compared block by block.
- **Cell** is one arm on one block: one container, one attempt, two deliverables
  (`/app/answer.txt` and `/app/trace.md`).
- **The relay** is the 1.1.0 study proxy (`study_proxy.py`) that every model request passes
  through. **The worker** is the 1.1.0 cell runner (`worker.py`, `answer.py`), **the classifier**
  is `classify.py`, and `native_config.py` builds each harness's command and configuration. All
  four live in `benchmarking/bbfinish/` in the private manuscript repository. This plan names the
  changes they need and does not reproduce their contents.
- **The review folder** is the harness review's working folder named in #534. It is not in any
  repository; everything this plan needs from it is listed under [Tasks](#tasks).
- **Arm ids** (CX, CC, BM5, BS5, BS6, BG6, BD6, AN) and **contrast ids** (C1 to C7) are defined
  in [Arms and contrasts](#arms-and-contrasts).

## What the rerun must answer

1. **Profile effect.** At a fixed binary (1.94.5), how much do Biorouter's mechanical outcomes
   and score change between the 1.1.0 minimal profile and the stock profile? This decides
   whether the Luna low collapse (34 of 186 Biorouter cells with no answer files in 1.1.0) and
   the timeout excess (47 against 14 for Codex and 39 for Claude Code) were configuration
   artifacts.
2. **Release effect.** With the stock profile, what changes between 1.94.5 and 1.94.6, measured
   first on the mechanism each fix targets and only then on score?
3. **Remaining gap.** How far is stock Biorouter 1.94.6 from Codex and Claude Code, per
   configuration and pooled, once the configuration confounds are gone?

The rerun does not rank models, does not update Codex or Claude Code, and does not change the
tasks.

## Arms and contrasts

Every arm runs in the answering image the 1.1.0 worker selects for the task, with its digest
recorded in the manifest. The review's smoke runs used `bbopus-local:cc2.1.296` (Ubuntu 24.04,
carrying Codex 0.157.1, Claude Code 2.1.296 and Biorouter 1.93.4). Where an arm needs another
Biorouter version, its binary is bind mounted over `/usr/bin/biorouter`. "Full grid" is the
1.1.0 OpenRouter grid: 62 tasks, nine configurations, three reps, 1,674 cells per arm.

| Id | Harness | Profile | Grid | Role |
| --- | --- | --- | --- | --- |
| CX | Codex 0.157.1 | native, 1.1.0 configuration unchanged | full | comparator |
| CC | Claude Code 2.1.296 | native, 1.1.0 configuration unchanged | full | comparator |
| BM5 | Biorouter 1.94.5 | minimal, ported to 1.94.5 | full | baseline for the profile effect |
| BS5 | Biorouter 1.94.5 | stock | full | profile effect, and baseline for the release effect |
| BS6 | Biorouter 1.94.6 | stock | full | release effect, and the headline Biorouter column |
| BG6 | Biorouter 1.94.6 | stock plus the deliverable gate of #535 | Luna low, GLM low, DeepSeek medium | labelled feature arm, never part of the like for like comparison; dropped if #535 does not ship in 1.94.6 |
| BD6 | Biorouter 1.94.6 | stock with Code Execution off | Luna low, GLM low, medium and high, DeepSeek medium and high | optional and exploratory: direct tools against Code Mode |
| AN | the three 1.1.0 harnesses (Biorouter 1.93.4 minimal, Codex, Claude Code) | 1.1.0 unchanged, through the 1.1.0 relay | anchor subset | shows the rerun reproduces 1.1.0 |

Each contrast is the Biorouter arm minus the other arm, paired on block.

| Id | Contrast | Question | Role |
| --- | --- | --- | --- |
| C1 | BS5 against BM5 | profile effect at a fixed binary | primary |
| C2 | BS6 against BS5 | release effect under the stock profile | primary |
| C3 | BS6 against CX | remaining gap to Codex | primary |
| C4 | BS6 against CC | remaining gap to Claude Code | primary |
| C5 | BG6 against BS6 | what the deliverable gate adds | secondary |
| C6 | BD6 against BS6 | Code Mode against direct tools | secondary, only if BD6 runs |
| C7 | BM5 against the 1.1.0 Biorouter cells | drift from 1.93.4 to 1.94.5 under the minimal profile, confounded with time | descriptive only |

C1 changes the prompt and the roster together, deliberately: the stock prompt is rendered from
the roster that is loaded, and a stock prompt over a cut roster names tools the model cannot call
(the second correction in #541). C2 compares two releases, so it measures a bundle; the
[mechanism counters](#mechanism-counters-for-the-release-comparison) attribute its parts.

## The Biorouter profiles

### Stock profile (BS5, BS6)

The rule is a fresh install's defaults, changed only where a headless benchmark cell forces it.
Each change is stated here and reported beside every score.

| Setting | Stock profile | Where the default comes from (`v1.94.5`) |
| --- | --- | --- |
| System prompt | Biorouter's own, rendered from the loaded roster; `BIOROUTER_SYSTEM_PROMPT_FILE_PATH` unset | [`prompt_manager.rs`](../../crates/biorouter/src/agents/prompt_manager.rs) |
| Task text | the task `README.md` on stdin, `--instructions -` | `parse_run_input` in [`cli.rs`](../../crates/biorouter-cli/src/cli.rs) |
| Extra system text | through `--system`, only under the task-level instructions rule below | `parse_run_input` |
| Extensions | every default-on extension except three that cannot work in a cell (next table) | `PLATFORM_EXTENSIONS` in [`extension.rs`](../../crates/biorouter/src/agents/extension.rs); `inject_builtin_extensions` in [`config/extensions.rs`](../../crates/biorouter/src/config/extensions.rs) |
| Code Execution (Code Mode) | on, as shipped | same |
| Compaction | on: threshold 0.8, eager compaction on, the summary written by the session's own model | `DEFAULT_COMPACTION_THRESHOLD`, `DEFAULT_EAGER_COMPACTION_ENABLED`, `BIOROUTER_COMPACT_USE_FAST_MODEL` in [`context_mgmt/mod.rs`](../../crates/biorouter/src/context_mgmt/mod.rs) |
| Stall check | on | `StallCheckConfig` in [`stall.rs`](../../crates/biorouter/src/agents/stall.rs) |
| Done gate, self critique | off | [`done_gate.rs`](../../crates/biorouter/src/agents/done_gate.rs), [`self_critique.rs`](../../crates/biorouter/src/agents/self_critique.rs) |
| MOIM, the per-request context block | on and anchored | `ANCHORED_KEY` in [`moim.rs`](../../crates/biorouter/src/agents/moim.rs) |
| Turn and tool call caps | none: no `--max-turns`, `BIOROUTER_MAX_TOOL_CALLS` unset (0, unlimited), so the relay's 200 request cap is the only cap, as for Codex and Claude Code | `DEFAULT_MAX_TURNS`, `DEFAULT_MAX_TOOL_CALLS` in [`agent.rs`](../../crates/biorouter/src/agents/agent.rs) |
| Effort | the cell's level, set by the relay as for the other two harnesses | relay change R3 |
| Permission mode | `BIOROUTER_MODE=auto`, the counterpart of the bypass flags Codex and Claude Code ran with in 1.1.0 | |
| Session store | `BIOROUTER_PATH_ROOT=/state/biorouter`, on its own mount outside `/app` and outside the task's scratch quota | removes the cause of the two exit 78 cells of 1.1.0 |
| Keyring | `BIOROUTER_DISABLE_KEYRING=true` | |

**The task-level instructions rule.** #541 proposes passing the four sentences of the 1.1.0
minimal prompt through `--system`. That is fair only if Codex and Claude Code received the same
sentences in 1.1.0. Before Stage 0, read `build_spec` in `native_config.py` and give all three
harnesses the same task-level text: if Codex and Claude Code received only the task `README.md`,
Biorouter receives only the README too. Record the outcome in the run manifest. Stage 0 checks
that the task text bytes in the first request are the same for the three harnesses.

**Extensions turned off, and why.**

| Extension | Default | Stock profile | Reason |
| --- | --- | --- | --- |
| `computercontroller` (Biorouter Copilot) | on | off | It controls a desktop through a native helper. A Linux container has none, its server returns errors in an unsupported environment, and every observation needs a human grant that a headless cell cannot give. |
| `crew` | on | off | It needs a saved connection to a Crew server. A cell has none, so its tools can only refuse. |
| `decisions` | on | off | Its tools ask a separately installed decision model, on the same computer or on Baseten. None exists in a cell, and one that did would be a second provider outside the relay's cap and journal. |
| `chatrecall`, `dictionary` | off | off | default |
| `developer`, `webdocuments`, `autovisualiser`, `memory`, `knowledge`, `agent_drafter`, `todo`, `workspace`, `extensionmanager`, `skills`, `code_execution` | on | on | default |

**What the model sees directly.** With Code Execution on, the model's own roster holds the Code
Execution tools (`execute_code`, `read_module`, `search_modules`, `wait_for`, `watch_job`) and the
families `survives_code_execution_filter` keeps
([`reply_parts.rs`](../../crates/biorouter/src/agents/reply_parts.rs)): Todo, Workspace
(including the sub-agent tool, which `BIOROUTER_MODE=auto` turns on), Memory, the platform tools,
the final output tool and the few tools a script may not run, such as deleting a knowledge base.
The developer tools (`shell`, `text_editor`, `shell_status`, `shell_kill`) and the other
extensions' tools are reached from inside an `execute_code` script. That is how Biorouter ships,
so it is what BS5 and BS6 measure; BD6 measures the alternative. Nobody writes the roster by
hand: Stage 0 captures it from the first request, and the relay enforces it from then on (R2).

> **Why the full default roster, not the four developer tools.** #541 proposes the roster
> `shell`, `shell_status`, `shell_kill`, `text_editor`. That fixes the 1.1.0 mismatch (a prompt
> naming tools the roster hid), but it is still a cut roster, so the comparison would still be
> against a Biorouter nobody runs. The default roster removes the mismatch at its source,
> because the stock prompt is rendered from what is loaded. Stage 0 checks that claim: every
> tool the rendered prompt names must be reachable, directly or through `execute_code`.

The stock profile's `config.yaml`, written to `/state/biorouter/config/config.yaml` before the
cell starts. Everything it does not name takes its default.

```yaml
extensions:
  computercontroller:
    enabled: false
    type: builtin
    name: computercontroller
  crew:
    enabled: false
    type: platform
    name: crew
    description: ""
  decisions:
    enabled: false
    type: platform
    name: decisions
    description: ""
```

### Minimal profile on 1.94.5 (BM5)

The 1.1.0 `minimal_configuration()` from `native_config.py`, its prompt file and its command,
unchanged, except for four changes that 1.94.x forces or that #541 asks of every new arm:

- `decisions` disabled in `config.yaml`, with the same entry as above. It became a default-on
  extension after 1.93.4, the minimal configuration does not name it, and the relay's denylist
  does not drop its tools.
- `BIOROUTER_MOIM_ANCHORED=false`, so one MOIM block sits after the latest tool results, where
  the relay expects it and strips it (see R4 for the byte change).
- `BIOROUTER_PATH_ROOT=/state/biorouter`, with the prompt file path moved with it
  (`BIOROUTER_SYSTEM_PROMPT_FILE_PATH=/state/biorouter/config/minimal-system.md`).
- The relay overwrites the `reasoning_effort` 1.94.x sends for OpenAI family models (R3).

`--max-turns 200` stays, as in 1.1.0. It equals the relay's cap, and the classifier now labels
Biorouter's own cap stop as a limit stop (K1), so the two caps are equivalent.

### Gate arm (BG6) and direct tools arm (BD6)

- **BG6** is BS6 plus the deliverable gate for `/app/answer.txt` and `/app/trace.md`, through the
  interface 1.94.6 ships for #535 (the proof of concept used `--require-file` and
  `BIOROUTER_RUN_REQUIRE_FILES`; use what `biorouter run --help` and the 1.94.6 release notes
  say). The gate adds an instruction the other harnesses do not get, so BG6 is reported only as
  a Biorouter feature arm.
- **BD6** is BS6 with one more entry in `config.yaml`, so every tool is called directly:

```yaml
  code_execution:
    enabled: false
    type: platform
    name: code_execution
    description: ""
```

## Comparators and the anchor

CX and CC run the 1.1.0 commands that `native_config.py` builds, unchanged:

```bash
codex exec --strict-config --ignore-rules --dangerously-bypass-approvals-and-sandbox --json -
```

with the 1.1.0 `config.toml` (the relay as model provider, the cell's `model_reasoning_effort`,
web search disabled, multi-agent off, `auto_compact_token_limit` at the route guard), and

```bash
claude --print --effort "$LEVEL" --output-format stream-json --setting-sources "" \
  --strict-mcp-config --mcp-config '{"mcpServers":{}}' --disable-slash-commands \
  --dangerously-skip-permissions
```

with `CLAUDE_CODE_SUBAGENT_MODEL` set to the cell's model and
`CLAUDE_CODE_MAX_OUTPUT_TOKENS=32768`, as in 1.1.0.

The anchor (AN) reruns part of 1.1.0 exactly: 20 tasks drawn from the 59 graded ones with a seed
recorded in the manifest; Luna low (the largest Biorouter deficit), GLM medium and DeepSeek
medium; three reps; all three 1.1.0 harnesses, including Biorouter 1.93.4 from the image under
its 1.1.0 minimal profile and store location. AN goes through a separate relay instance at its
1.1.0 commit and is classified with the 1.1.0 classifier. That is 180 cells per harness and 540
in all.

## Models

| 1.1.0 key | OpenRouter id | Effort levels | Upstream pin, as in 1.1.0 | Model page price, input / output per million tokens |
| --- | --- | --- | --- | --- |
| m01 | `openai/gpt-6-luna` | low, medium, high | OpenAI | $0.10 / $0.50 |
| m03 | `z-ai/glm-5.3-flash` | low, medium, high | Novita (fp8) | $0.04 / $0.08 |
| m04 | `openai/gpt-6.1-sol` | medium | OpenAI | $2 / $10 |
| m07 | `deepseek/deepseek-v4-flash` | medium, high | GMICloud (fp8) | $0.09 / $0.18 |

Each id was checked on 2026-11-19 against OpenRouter's public catalog
(`GET https://openrouter.ai/api/v1/models`) and its endpoint list
(`GET https://openrouter.ai/api/v1/models/<id>/endpoints`): it exists, lists tool calling and
reasoning, carries no expiration date, and its 1.1.0 upstream is still among its endpoints. A
pinned upstream can charge more than the model page price, which is why the budget uses
measured 1.1.0 costs instead. Before Stage 0, compare each id with the slug `native_config.py`
records for its model key. A mismatch stops the plan until it is resolved, because a renamed id
is a different model for this purpose.

**Claude models (optional Stage 4).** `anthropic/claude-sonnet-5.5` ($2 / $10) and
`anthropic/claude-haiku-5.5` ($0.10 / $0.50) at medium, through the relay, for CX, CC and BS6,
so that the Biorouter column measures Biorouter's own loop. The 1.1.0 Claude plan arm is not
rerun as a headline: Biorouter ran its `claude_code` provider there, so it measured Claude
Code's loop driving Biorouter's tools. If it is ever rerun, it carries that label.

**Judges.** The primary judge is Claude Opus 5.5, exactly as in 1.1.0: the study's grading
prompt, schema and scorer, medium effort, a tool-less `claude -p` with the 1.1.0 flags. The second
judge is `google/gemini-4-argon` ($2 / $10) through OpenRouter with the same prompt, schema and
scorer. It belongs to a family with no model under test.

## Tasks

- **Task set.** The 62 BioRouterBench 1.1.0 tasks, with questions and inputs at the sha256
  values the 1.1.0 inputs manifest records (`bbfinish/vendor/inputs/<task>.json`). The primary
  analysis uses the same 59 graded tasks as 1.1.0. Nothing about the tasks changes.
- **Reps.** Three per block, as in 1.1.0. Rep disagreement was higher for Biorouter than for the
  other two in 1.1.0 (#534, #541), and three is the floor #541 asks for.
- **Pilot set (Stage 1).** The eight smoke tasks of #534 (`da-9-7`, `da-5-3`, `da-4-6`,
  `da-15-8`, `da-14-8`, `da-8-3`, `da-9-1`, `br-ext-4`), plus up to ten Luna low tasks chosen
  by one rule: tasks whose three 1.1.0 Biorouter reps on Luna low all ended `missing_files`, in
  task id order from `cells.csv`. The rule is fixed here, so the selection cannot follow the
  pilot's results.
- **Mechanism panel (Stage 1).** The five synthetic tasks of the review (`syn-bg`, `syn-fg`,
  `syn-bgjob`, `syn-early`, `syn-cap`), each built to trigger one harness behaviour. They and the
  review's counter script (`syn_report.py`) are in the review folder only, so commit them to the
  benchmark repository, with their expected answers, before Stage 1. `syn-cap` passes
  `--max-turns 4` by design: it tests how a cap stop is reported, not the cap.

## Environment

- **Where.** AWS EC2, x86_64, in the region and on the instance type the 1.1.0 OpenRouter arm
  used; read both from the bbfinish run records and write them into the manifest. Native
  x86_64 matters because timeouts at 3,600 s are a primary endpoint: the review's local smoke
  runs used amd64 emulation on Apple Silicon, and their wall times cannot be compared. No timed
  cell runs on a laptop.
- **Costing assumption.** If the 1.1.0 type is not recorded, use `r7i.16xlarge` (64 vCPU,
  512 GiB): eight cells at once at 7 CPUs and 56 GB each, leaving 8 vCPU for Docker and the
  relay. Four such hosts give 32 cells at once.
- **Cell isolation.** One fresh container per cell with the 1.1.0 `docker_argv` flags:
  `--platform linux/amd64 --init --user 10001:10001 --workdir /app --cpus 7 --memory 56g
  --read-only --cap-drop ALL --security-opt no-new-privileges`, plus 1.1.0's `--pids-limit` and
  `--ipc none`. Mounts: the cell's work folder at `/app` on the per-cell 20 GiB scratch image,
  as in 1.1.0; `README.md` and `data/` read only; and, new for the Biorouter arms, the cell's
  state folder at `/state` on its own 4 GiB image. State is fresh per cell, so no memory,
  knowledge base, skill or session carries from one cell to the next.
- **Network.** The 1.1.0 `benchmark-isolated` Docker network, whose only route is the relay, for
  every arm. A harness that tries to reach anything else fails the same way as the others.
  Stage 0 records every such attempt Biorouter makes (its stderr and the network's DNS log), so
  a blocked startup call that changes behaviour is fixed before Stage 1, not found in the
  results.
- **Scheduling.** All arms of a block run inside one 24 hour window, in an order drawn with a
  seed derived from the block id, so drift at an upstream provider lands on every arm alike.
  Infrastructure failures (container start, host loss, relay restart) are retried as new
  attempts, as in 1.1.0. Agent outcomes are never retried.
- **Binaries.** Biorouter 1.94.5 and 1.94.6 come from the GitHub release assets
  `biorouter-cli_<version>_amd64.deb` (release profile). Extract `/usr/bin/biorouter`, record its
  sha256 and the image digest in the manifest, and never use a `quick` or debug build. Stages 0
  and 1 may use a release profile build of the 1.94.6 draft's target commit; Stage 2 uses the
  published asset and repeats Stage 0 on it first.

A BS6 cell, as the worker must start it (`$IMAGE`, `$MODEL`, `$CELL`, `$BIN` and `$RELAY_URL`
come from the manifest; the 1.1.0 values of `HOME`, the Python and R library paths and
`TMPDIR=/app/tmp` are passed unchanged and not repeated here):

```bash
docker run --rm --platform linux/amd64 --init --user 10001:10001 --workdir /app \
  --cpus 7 --memory 56g --read-only --cap-drop ALL --security-opt no-new-privileges \
  --network benchmark-isolated \
  --mount "type=bind,src=$CELL/app,dst=/app" \
  --mount "type=bind,src=$CELL/app/README.md,dst=/app/README.md,readonly" \
  --mount "type=bind,src=$CELL/app/data,dst=/app/data,readonly" \
  --mount "type=bind,src=$CELL/state,dst=/state" \
  --mount "type=bind,src=$BIN/biorouter-1.94.6,dst=/usr/bin/biorouter,readonly" \
  --env MODEL="$MODEL" \
  --env BIOROUTER_PATH_ROOT=/state/biorouter \
  --env BIOROUTER_MODE=auto \
  --env BIOROUTER_DISABLE_KEYRING=true \
  --env OPENROUTER_HOST="$RELAY_URL" \
  --env OPENROUTER_API_KEY=relay-placeholder \
  "$IMAGE" \
  sh -c 'exec biorouter run --provider openrouter --model "$MODEL" --output-format stream-json --instructions - < /app/README.md'
```

BS5 is the same with the 1.94.5 binary. BM5 runs the 1.1.0 command,
`biorouter run --provider openrouter --model "$MODEL" --max-turns 200 --output-format stream-json --instructions /app/README.md`,
with the 1.1.0 environment and the four changes listed under its profile.

### Credentials

- **One key per stage.** Create an OpenRouter key for each stage with an OpenRouter credit limit
  equal to that stage's budget, so an overrun stops at the provider rather than on an invoice.
  Revoke it when the stage ends.
- **Only the relay holds it.** Store the key as an AWS SSM Parameter Store SecureString that only
  the relay host's instance role can read. The relay reads it into memory at start through the
  AWS SDK. It is never an environment variable in a host shell, never on disk and never inside a
  harness container.
- **Containers get a placeholder.** Biorouter gets `OPENROUTER_API_KEY=relay-placeholder` and
  `OPENROUTER_HOST` pointing at the relay; Codex and Claude Code get their 1.1.0 equivalents. The
  relay puts the real credential on the upstream request.
- **Logs hold metadata only.** The relay journal records sizes, model, upstream, finish reason
  and timings, and redacts `authorization`, `x-api-key` and `api-key` headers.
- **Scan every batch.** After each batch, search everything it produced (relay journal,
  `native.stdout`, Biorouter session stores, task folders, archives) for the key's value and for
  the `sk-or-` prefix. A hit stops the run and revokes the key, and the affected archives are
  deleted, not redacted.
- **Judges.** The Opus judge runs as in 1.1.0, through Claude Code's own sign-in on the grading
  machine, with no key in any file. The Gemini judge uses its own OpenRouter key under the rules
  above.
- **Nothing in the repository.** No credential, parameter value or account identifier goes into
  the benchmark repository, this document, the manifest (which names parameters only) or any
  issue.

## Relay and classifier changes

Every change below goes into the benchmark repository. None is a Biorouter change.

| Id | Change | Applies to | Why |
| --- | --- | --- | --- |
| R1 | A relay mode per arm. `minimal` keeps 1.1.0 behaviour (MOIM stripped, shell-only roster, requests with no tools denied); `native` is unchanged for CX and CC; `stock` forwards MOIM blocks, the declared roster and Biorouter's own requests with no tools (compaction summaries, stall checks) on the cell's model. Every request of a cell, sub-agents included, counts against the one 200 request cap, for every harness. | all | the stock profile must reach the model as shipped |
| R2 | Assert the forwarded tool list against the declared roster: on every request in `minimal` mode; in `stock` mode exactly on the first request, with any later change (an extension loaded during the run) logged as an event, not rejected. | Biorouter arms | replaces the 1.1.0 denylist (`DROP_TOOLS`), which misses tools added after 1.93.4 such as `decisions__*` |
| R3 | Delete Biorouter's `reasoning_effort` and set the cell's `reasoning.effort`, as for the other harnesses; the `reasoning_conflict` check becomes an overwrite. | Biorouter arms on 1.94.x | 1.94.x sends `reasoning_effort: medium` for `openai/` models (`create_request` in [`formats/openai.rs`](../../crates/biorouter/src/providers/formats/openai.rs)), which the 1.1.0 relay refuses |
| R4 | Accept the 1.94.x MOIM block, which ends with one newline where 1.93.4's ended with two. | `minimal` mode | otherwise every 1.94.x request fails `exactly_one_whole_moim_required` |
| R5 | Reject a request for any model other than the cell's as `model_not_in_route`, and count it apart from failures. | all | Biorouter's stall check asks OpenRouter's fast model (`google/gemini-3.5-flash`) first and falls back to the session's model on an error (`complete_fast` in [`providers/base.rs`](../../crates/biorouter/src/providers/base.rs)); the 1.1.0 relay already rejected Claude Code's auxiliary requests |
| R6 | Answer `GET /api/v1/models` from a snapshot of OpenRouter's catalog taken at Stage 0. | all | a harness that asks for model metadata gets the same answer for the whole run |
| R7 | Forward `reasoning_details` in assistant messages. | Biorouter arms on 1.94.x | 1.94.x replays OpenRouter reasoning (see #548) |
| R8 | Redact credentials in the journal, as described under Credentials. | all | |

| Id | Classifier change | Why |
| --- | --- | --- |
| K1 | Label Biorouter's own cap stops as limit stops: from the `stop_reason` of the `complete` event where the binary reports one (#540), otherwise from the stop text ("reached my action limit") or a final `tool_calls` finish at the cap. | 1.1.0 filed 10 such stops as `missing_files` (#541) |
| K2 | Label a final dispatch with no output, or a `content_filter` finish, as an upstream fault. | moves 1 to 2 cells; the same rule for every harness |
| K3 | Give Biorouter's repetition guard stop and stall check stop their own agent stop classes. | they are product decisions to end a run and should be visible as such |
| K4 | Flag a Biorouter run that ends while a `watch_job` it started is still pending. | see the `watch_job` row under [Risks](#risks) |

The new classes change labels, not completeness: a cell missing a deliverable still scores 0. AN
keeps the 1.1.0 classifier; K1 to K4 are applied to it afterwards only as a relabelling report.

## Stages and commands

### Stage 0: compatibility and plumbing, no model calls

Build in the benchmark repository: `stock_configuration()` beside `minimal_configuration()` in
`native_config.py`; R1 to R8; K1 to K4; a stub upstream that speaks each upstream protocol the
relay uses and plays a scripted exchange (for a Biorouter stock arm: a
`code_execution__read_module` call for each module the rendered prompt names, then a tool call,
then a reply carrying `reasoning_details`, then a final text; for the other arms: a tool call,
then a final text); and a checker that reads the relay journal.

Then, for each Biorouter binary (1.94.5, 1.94.6), each profile it runs (minimal, stock, gate,
direct tools) and each model family (`openai/gpt-6-luna` and `z-ai/glm-5.3-flash`), start the
cell's real container command against the relay with the stub as its upstream. Run CX and CC the
same way through the changed relay and through the 1.1.0 relay, and diff what each forwards.

Pass criteria, all required for every combination:

1. No relay rejects other than `model_not_in_route`.
2. The first request's tool list equals the profile's declared roster. For the stock profile
   that list becomes the declared roster in the manifest, and it holds no `computercontroller__`,
   `crew__` or `decisions__` tool.
3. Every tool the rendered stock prompt names appears in the declared roster or in a
   `read_module` listing from the scripted exchange. The prompt's sha256 is recorded.
4. The task text bytes in the first request are the same for the three harnesses.
5. The upstream requests carry exactly one effort value, equal to the cell's level, and the
   32,768 token output cap.
6. CX's and CC's forwarded requests are the same through both relays, apart from journal
   redaction.
7. The stream's `complete` event arrives and the exit code is recorded; on 1.94.6 its
   `stop_reason` field is present if #540 shipped.
8. The context window Biorouter uses for each model (from the session's stored model
   configuration) is recorded beside the catalog's value.
9. The secret scan finds nothing.

Cost: no model spend; about two to three working days of engineering.

### Stage 1: pilot

- **Real tasks:** the pilot set, Luna low, three reps, arms CX, CC, BM5, BS5 and BS6: up to 270
  cells.
- **Mechanism panel:** the five synthetic tasks, GLM-5.3 Flash at medium, three reps, arms BM5,
  BS5, BS6 and BG6: 60 cells.
- **Measure:** mechanical endpoints and mechanism counters only, no judge.
- **Go to Stage 2 when:** no arm has more than 2% infrastructure failures, relay rejects other
  than `model_not_in_route` are zero, the secret scan is clean, and every counter can be computed
  for every arm. The direction or size of an outcome difference never decides whether Stage 2
  runs, so the full run cannot be selected on a favourable pilot.
- **Re-derive the budget** from the pilot's measured cost per cell before Stage 2.
- **Key limit:** $50. **Wall time:** about half a day on one host.

### Stage 2: main grid

- CX, CC, BM5, BS5 and BS6 on the full grid (8,370 cells), BG6 on its three configurations
  (558), and AN (540): 9,468 cells. BD6 (1,116) runs only if the budget allows after the rest.
- **Freeze before the first cell:** this plan's commit, the manifest (binary and image digests,
  declared rosters, prompt hashes, task hashes, model ids and upstream pins, seeds, relay and
  classifier commits, which 1.94.6 issues shipped according to its release notes) and the
  analysis script. Commit them to the benchmark repository and cite the manifest's commit in the
  results.
- **Key limit:** $1,600.

### Stage 3: grading

- **Primary judge** on every graded cell of Stage 2: 9,468 cells, or 10,584 with BD6.
- **Second judge** on a stratified sample: 20 tasks (a seed of its own, independent of the
  anchor's), rep 1, all nine configurations, arms CX, CC, BM5, BS5 and BS6: 900 cells.
- **Blinding.** Both judges see `answer.txt` and `trace.md` after one redaction applied the same
  way to every harness (harness and product names, tool and extension names such as
  `developer__shell`, `execute_code`, `apply_patch` and `Bash`, and paths that name a harness's
  state folder), in shuffled order, with no arm label. Commit the redaction list with the
  manifest.

### Stage 4: Claude models through the relay (optional)

- Sonnet 5.5 and Haiku 5.5 at medium, all 62 tasks, three reps, arms CX, CC and BS6: 1,116 cells.
- Runs only after Stage 0 passes for each harness on each model.
- **Key limit:** $600.

## Metrics

### Primary endpoints

| Endpoint | Definition | Source |
| --- | --- | --- |
| Incomplete rate | share of cells without both deliverables present and non-empty, or ended by the 3,600 s wall | worker outcome |
| Missing files by class | text only stop, Biorouter cap stop (K1), relay cap stop, upstream fault (K2), guard or stall stop (K3), other | classifier |
| Timeouts | cells ended by the 3,600 s wall | worker |
| Foreground kills | developer `shell` calls killed at the 240 s foreground budget, called directly or inside `execute_code` | tool results in the session store |
| Requests per cell | model requests the relay forwarded, sub-agent and auxiliary ones included | relay journal |
| Non-model seconds per request | cell wall time minus the summed upstream request time, divided by the requests | worker and relay journal |

An `execute_code` result records the tool calls its script made, up to 64 per run
(`MAX_TOOL_CALL_RECORDS` in
[`code_execution_extension.rs`](../../crates/biorouter/src/agents/code_execution_extension.rs)).
The counters read those records as well as direct calls, and report how many runs reached the
record cap.

### Mechanism counters for the release comparison

Each row is counted only if the issue shipped in 1.94.6.

| Issue | What 1.94.6 changes | Counter |
| --- | --- | --- |
| #535 (P01) | an opt-in deliverable gate | BG6 only: gate re-prompts, and cells that end on the gate's exit code |
| #536 (P02) | the foreground shell returns when the shell exits | shell calls held more than 10 s after their command line put a job in the background |
| #537 (P03) | a 240 s kill returns the output so far and is not labelled retryable | kills whose result carries output; kills labelled retryable |
| #538 (P04) | background jobs stay readable without `shell_status` | background starts whose output was read later |
| #539 (P05) | a headless prompt block replaces the interactive one | first request size; text only stops before request 25 |
| #540 (P06) | a stop reason in the `complete` event, and a non-zero exit on limit stops | share of cells with a `stop_reason`; exit codes |
| #542 (P08) | one retry on in-stream error frames and empty replies | runs ended by an in-stream error or an empty reply |
| #543 (P09) | a headless auto-deny says what happened | tool results telling the model the user declined |
| #544 (P10) | a custom prompt composes with a small core | not measured: it changes only a prompt override, which no 1.94.6 arm uses |
| #545 (P11) | a unique bare tool name is repaired | "not found" results for a bare tool name |
| #546 (P12) | a quoted here-document is read as data | secret guard refusals of a here-document write |
| #547 (P13) | one chance to act on a repetition deny | runs ended by the repetition guard |
| #548 (P14) | streamed `reasoning_details` fragments are merged | replayed fragments per assistant message, from the relay journal |
| #549 (P15) | a full disk degrades instead of ending the run with exit 78 | exit 78 cells; expected zero in every arm, since the store is off the task disk |

### Secondary endpoints

- Judge score, 0 to 100, with an incomplete cell scored 0 (the 1.1.0 rule).
- Cost per cell from the relay's usage records, wall time per cell, first request input tokens,
  tool count on the first request, trace length, and compactions per cell.

### The capability set beside every score

Every results table carries, for each arm: the prompt's source and sha256, the median first
request input tokens, the number of directly callable tools, whether sub-agents, web tools and
compaction were available and how often each was used or fired, the turn cap, the effort route
and the output cap.

## Statistical comparison

- **Pairing and clusters.** Every contrast pairs on block. The 59 graded tasks are the clusters.
- **Binary endpoints** (incomplete, missing files, timeouts): the risk difference with a 95%
  percentile bootstrap interval that resamples tasks (10,000 resamples), and an exact McNemar
  test on discordant blocks, per configuration and pooled.
- **Score:** the mean paired difference with the same task cluster bootstrap, per configuration,
  pooled over the eight configurations other than Luna low (1.1.0 also pooled without Luna
  low), pooled over all nine, and on the blocks every compared arm completed.
- **Counts per cell** (requests, kills, non-model seconds): the ratio of means with the task
  cluster bootstrap.
- **Multiplicity.** Holm correction over the four primary contrasts (C1 to C4), for the pooled
  incomplete rate and the pooled score: eight tests. Everything else is an estimate with its
  interval and carries no claim.
- **Judge offset.** On the second judge sample, take each arm's mean of (second judge minus
  primary judge), and the difference of those means between the two arms of each primary
  contrast, with the task cluster bootstrap. A harness dependent offset is present when that
  interval excludes 0. Agreement is reported as mean absolute error and weighted kappa on ten
  point bins (the 1.1.0 retest of the primary judge: 2.39 and 0.85).
- **Noise thresholds** (from #541). Mechanical endpoints are decided first. A score difference
  of 5 points or more counts when its interval excludes 0 over at least 125 paired blocks. A
  smaller difference counts only when its interval excludes 0 over at least 350 paired blocks,
  the second judge's estimate has the same sign, and the difference is larger than any harness
  offset measured between those two arms.

Power, from the 1.1.0 paired block standard deviation of about 20 points, at 80% power and a two
sided 5% level, before allowing for task clustering (which the bootstrap does, and which widens
these):

| Comparison | Paired blocks | Smallest detectable difference |
| --- | --- | --- |
| one configuration | 177 | about 4.2 points |
| pooled over eight configurations | 1,416 | about 1.5 points |
| anchor, pooled | 180 | about 4.2 points |
| missing files, 18% against 5% | about 90 cells per arm needed | Luna low has 186 per arm |

## Budget and time

Assumptions:

- Cost per cell is the 1.1.0 measurement from #534, in USD per cell for Codex, minimal
  Biorouter and Claude Code: GPT-6.1 Sol 0.39, 0.17, 0.98; GPT-6 Luna medium 0.035, 0.014, 0.119;
  GLM-5.3 Flash medium 0.047, 0.029, 0.047; DeepSeek V4 Flash 0.070, 0.071, 0.099. The medium
  figure stands in for the other levels of the same model.
- A stock profile cell costs what a Claude Code cell costs. Its first request (Biorouter's own
  estimate for its default prompt and tool schemas is about 31,000 to 40,000 tokens, in
  `CLAUDE.md`'s Llama Server notes) is close to Claude Code's 33,300. Stage 0 measures the real
  size and Stage 1 the real cost.
- Stage 4 prices Sonnet 5.5 like GPT-6.1 Sol and Haiku 5.5 like GPT-6 Luna medium, the models
  with the same list prices.
- A graded cell costs the judge about 23,000 input and 2,000 output tokens, as in 1.1.0.
- The mean cell takes about 8 minutes. 1.1.0 medians run from under 3 minutes (Biorouter on
  Luna medium, 161 s) to about 20 minutes on the slowest configuration, and about 2% of cells
  ran the full hour (100 timeouts in 5,022 cells).
- Hosts are `r7i.16xlarge` at about $4.23 an hour on demand in us-east-1, eight cells per host.
  Check the price on the day.

| Stage | Cells | Model spend | Key limit | Host hours | Wall time |
| --- | --- | --- | --- | --- | --- |
| 0 | none | none | none | a few | two to three working days of engineering |
| 1 | 330 | about $25 | $50 | about 6 | half a day, plus a day to review |
| 2 required | 9,468 | about $1,240 | $1,600 | about 160 | about 40 hours on four hosts |
| 2 optional, BD6 | 1,116 | about $85 | $150 | about 19 | about 5 hours on four hosts |
| 3, primary judge | 9,468 | none through the Claude plan (about 19 hours at three at a time), or about $1,250 through the API | none, or a $1,400 API spend limit | none | about a day |
| 3, second judge | 900 | about $60 | $150 | none | about 2 hours |
| 4 optional | 1,116 | about $490 | $600 | about 19 | about a day |
| Analysis and report | | | | | two days |

Required total: about $1,600 of model spend with a 25% contingency, about $750 of EC2 (host
hours plus storage for the cell archives) and $60 to $1,310 for judging, depending on the
primary judge's route: about $2,400 to $3,700. The optional BD6 arm and Stage 4 together add
about $740 of model spend and EC2, plus about $300 if their cells are judged through the API.
Calendar time is about two working weeks.

## Risks

| Risk | Effect | Mitigation |
| --- | --- | --- |
| Models and upstreams drifted since 1.1.0 | the rerun cannot be pooled with the published cohort | AN with an explicit reproduction criterion (V3); every arm runs at the same time, so contrasts inside the rerun stay valid even if AN fails |
| The Luna low collapse does not reproduce under BM5 | C1 cannot test the configuration artifact question for Luna low | report it as untestable on today's models; the review's smoke runs on other models did not reproduce it either (#534) |
| 1.94.6 ships only part of #535 to #549 | C2 measures a smaller bundle | list the shipped issues in the manifest from the 1.94.6 release notes and count only those; drop BG6 if #535 did not ship |
| Code Mode routes the shell through JavaScript | a model that writes poor scripts does worse than with direct tools; this is the product as shipped | BD6 measures it if the budget allows; `execute_code` error rates are reported per arm |
| `watch_job` in a headless run | Code Execution's `watch_job` ends the turn and resumes the chat when the job finishes. No handling for it was found in `crates/biorouter-cli`, so a headless run may exit while a watched job still runs, leaving files missing. Not verified. | K4 flags it; `syn-bgjob` exercises it in Stage 1; if it happens, file a product issue instead of changing the profile |
| Biorouter reads GLM-5.3 Flash's window as 200,000 tokens | `z-ai/glm-5.3-flash` has no exact entry in `MODEL_CONTEXT_WINDOWS` ([`model.rs`](../../crates/biorouter/src/model.rs)), so the `glm-5` substring fallback gives 200,000 where OpenRouter lists 1,048,576. At the 0.8 threshold, compaction can start near 160,000 tokens on GLM cells; 1.1.0's largest Biorouter request, on any model, was 189,825 tokens. | shipped behaviour, so the stock profile keeps it; compactions are counted per configuration, Stage 0 records the window (criterion 8), and the mismatch goes to a product issue |
| A default extension misbehaves without its prerequisites | noise in the Biorouter arms | three are off with reasons; Stage 0 records blocked connections; extension load errors are counted |
| The relay changes create an asymmetry | a biased comparison | each mode is checked in Stage 0, and CX's and CC's forwarded requests are diffed against the 1.1.0 relay |
| The stock profile costs more than assumed | budget overrun | per stage key credit limits; the budget is re-derived from Stage 1 |
| Rate limits at a pinned upstream (HTTP 429) | retries, longer cells, aborted runs | the 1.1.0 relay retries; concurrency is capped per upstream; an abort after a rate limit is classified as upstream (in the review's smoke runs a 1.94 build aborted after eight retries on HTTP 429; the evidence is on #542) |
| The primary judge scores one harness's answers differently | a gap of 2 to 4 points can be the judge, not the harness: a small Sonnet 5.5 calibration in 1.1.0 hinted at such an offset for Claude Code's answers (#534), and Stage 4's Claude answers come from the judge's own family | the second judge comes from a family not under test, and the offset analysis measures the effect |
| A credential reaches an archive | key exposure | placeholder keys in containers, metadata-only journals, a scan after every batch, revocation |
| Emulation or a different host type | wall times and timeouts shift | native x86_64 on the 1.1.0 instance type; AN checks timeouts too |

## Acceptance criteria

The run is valid when all of these hold:

| Id | Criterion |
| --- | --- |
| V1 | Stage 0 passes for every binary, profile and model family used, and is repeated on the published 1.94.6 binary before its first Stage 2 cell. |
| V2 | The secret scan finds nothing in any batch. |
| V3 | For each harness, the 1.1.0 mean on the anchor's cells lies inside the 95% bootstrap interval of AN's pooled mean score, and AN's missing file and timeout counts lie inside the 95% binomial intervals of 1.1.0's rates on those cells. If V3 fails, the results are reported as a self-contained comparison and not pooled with 1.1.0. |
| V4 | At least 98% of each arm's planned cells end with a final attempt. |
| V5 | All arms of every block ran within the same 24 hour window. |
| V6 | Every forwarded request of a configuration was served by its pinned upstream. |

What the results may conclude, mechanical endpoints first:

| Id | Rule |
| --- | --- |
| D1 | The Luna low collapse is a configuration artifact if BM5 reproduces it (at least 10% of its Luna low cells missing files) and BS5's rate is at most 5%, with C1 significant after Holm. If BM5 does not reproduce it, the results say the question cannot be tested on today's models. |
| D2 | A release change (C2) on a mechanical endpoint counts when its interval excludes 0; a score change follows the noise thresholds. A rise of more than 2 percentage points in the pooled incomplete rate (upper interval bound), or of more than 20% in tool calls per completed cell or cost per cell, from 1.94.5 to 1.94.6 is reported as a regression in 1.94.6. |
| D3 | The remaining gap (C3, C4) is reported per configuration and pooled, on all blocks and on the blocks all arms completed, each with its interval and its capability set. No single headline number appears without its interval. |
| D4 | #541 is done when V1 to V6 hold, a results document reporting D1 to D3 sits beside this plan in `docs/benchmarking/`, and the run's data, manifest and scripts are committed to the benchmark repository. |

## Out of scope

- A composed custom prompt (P10, #544) as a benchmark profile. A later run can use it once it
  ships.
- Newer Codex or Claude Code versions. Updating the comparators is a separate study.
- The 1.1.0 Claude plan arm as a headline.
- Any Biorouter code change. What the rerun finds becomes issues.

## Related documentation

- [Benchmarking](README.md): this folder's index, and where the results will go.
- [Code Execution capability](../extensions/built-in/code-execution.md): Code Mode, which the
  stock profile keeps on.
- [Subagents](../agent-loop/subagents.md): the delegation that `BIOROUTER_MODE=auto` turns on in
  the stock profile.
- [Environment variables](../configuration/environment-variables.md): `BIOROUTER_PATH_ROOT`,
  `BIOROUTER_AUTO_COMPACT_THRESHOLD` and the other keys named here.
- [biorouter CLI command reference](../cli/command-reference.md): `biorouter run` and its flags.
- [Coding-agent providers](../providers/coding-agents/README.md): why the 1.1.0 Claude plan arm
  measured Claude Code's loop and not Biorouter's.
- [Issue #534](https://github.com/BaranziniLab/biorouter/issues/534): the harness review, its
  evidence and the proposals P01 to P15 (issues #535 to #549).
