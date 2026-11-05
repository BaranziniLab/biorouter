# Decision models

> **What this is.** The guide to decision models in Biorouter: fast models that return a typed answer (yes or no, one of named options, a level on a scale) with the probability of every outcome, instead of text. It covers where they come from, the four ways a chat uses them (monitors, the agent's `decide` tool, phase gates, and a decision model as the chat's own model), the decision log and the rule that keeps thousands of decisions out of the transcript, the privacy rules, the settings, the routes, and the CLI and terminal UI.
> **Status:** Current.
> **Audience:** end users who want a decision model to watch, screen or gate a task, and developers working on the agent loop, a decision adapter or a surface that shows decisions.

A language model writes prose, which is slow and hard to act on when the question has a fixed set of answers. "Does this clinical note state the patient has multiple sclerosis?" has two answers. A decision model reads the note and returns `yes` with P(yes) = 0.97 in tens of milliseconds. That speed changes what is practical: an agent can screen six hundred notes in a few seconds and then read only the ones that matter, a monitor can check the conversation after every step of a long task, and a phase of delegated work can be held back until a decision model is confident it met its goal.

Albert Lee's prototype in PR #469 (Baseten System One exit criteria) was the first decision-model integration in Biorouter. Its typed question kinds and their validation, the rule that a probability equal to the threshold passes, the System One request builders, the refusal to echo a backend's error body, and the idea of evaluating the agent's progress tail all carry over into what is described here.

## What a decision is

A **question** has a name (letters, digits, `_` or `-`), instructions, and a kind:

| Kind | Answers | Probabilities keyed by |
|---|---|---|
| yes/no (System One's `noul`) | `yes` or `no` | `yes`, `no` |
| choice | one of 2 to 26 named options, each with an optional description the model reads | the option names |
| score | one of 1 to 10 ordered levels, lowest first | the level numbers, from `0` |

A **decision** keeps the whole distribution, the answer it points to, and two numbers that are easy to confuse:

- **The probability of an answer** (P(yes), P(an option), or P(a level or higher) for a score). Thresholds act on this, and only on this.
- **Confidence**, how concentrated the distribution is: (max p minus 1/k) divided by (1 minus 1/k) for k outcomes, which is |2p minus 1| for a yes/no question. Zero is uniform and one is certain. It is not the probability of anything, and Biorouter computes it the same way for every adapter.

A **threshold** marks what passes: P(yes) at or above it for yes/no, P(a named option) for a choice, P(the named level or any above it) for a score. A probability equal to the threshold passes. For a score the threshold uses the summed level probabilities, never the expected level, because an expected level of 1.0 can hide a distribution split between 0 and 2.

Every decision also says whether its probabilities are **calibrated**: true for a decision model's own distribution, and for token log-probabilities that covered at least 90% of the probability mass; false for a number a model wrote. Even calibrated values are the model's probabilities, not measured accuracy on your data. No calibration error figures are published for any of these models, and fitting calibration to your own data is not built.

## Where decision models come from

| Provider id | What it is | Tier | How it is found |
|---|---|---|---|
| `baseten_decision` | Baseten's decision models (`joey-1`, `elephant-1`, `mammoth-1`) over the System One API, `POST /v1/systemone` | Public, no affiliation | The `decision-models/` entries of Baseten's live catalog, else the three known slugs, when `BASETEN_API_KEY` is saved. The host is `BASETEN_DECISION_HOST` (default `https://inference.baseten.co`). Each has a 16,384-token window. |
| `ollama_decision` | A local Ollama (0.35 or later) model whose `/api/show` capabilities include `decision`, for example `tev1:0.8b`, over the same System One API | Private on a loopback host (the self-hosted rule); Public for a cloud tag or a model `/api/show` says is served remotely | `OLLAMA_HOST`, the way Ollama's own client reads it |

Two fallbacks let a chat model answer decision questions when no decision model is available, and every decision records which adapter produced it:

1. **Log-probabilities.** Each option is mapped to one token (`yes`/`no`, or `A`, `B`, ...), the model is asked for one token at temperature 0, and the probability of each option is summed over its spelling variants and renormalised. Implemented for Ollama's `/api/chat` and for the OpenAI chat format. A backend that drops `logprobs` silently is never read as a certain answer; the decision falls back to the next adapter and says why.
2. **Verbalized.** The model writes an answer and a number as structured output. These probabilities are always marked uncalibrated, and a phase gate refuses them unless the gate allows it.

**Measured on `tev1:0.8b`** (Ollama 0.40.14, an Apple Silicon laptop, 2026-11-04): about 22 ms for one question on new text once the model is loaded, about 15 ms per question when several share one text, 1.6 s for the first call including the load, and identical answers on repeat runs. Its default window is 2,048 tokens, so chunks for it must be short; `decision_models` reports each model's window. The vendor latencies published for Baseten's models (20 to 49 ms) were not verified.

**Which model a decision uses**, first match wins:

1. the model the use names (a monitor's, a gate's, or the `decide` tool's `model` argument, which must be one `decision_models` lists);
2. the chat's default decision model (`defaultModel` in its decision settings);
3. `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL` (the model may be written `provider/model`);
4. the first available model this chat may use, local models first.

## Four ways a chat uses decision models

### Monitors: a decision model watching the chat

A monitor asks one question about a rolling window of the conversation while the agent works, and acts when the answer reaches its threshold. A chat takes at most 8. Each one has:

- a **question** (any kind) and an optional **model**;
- a **timing**: `post_iteration` (after every model reply and its tool results) or `turn_end` (once, as the turn is about to finish);
- a **window**: the newest entries of the conversation (at most 48), a character budget (at most 12,000), and whether tool calls and results are included. The window is built from stored messages, which have already passed the secret-output guard;
- a **threshold** on P(yes) or on a named option, a number of **consecutive** evaluations at or above it before it fires (1 to 5), and an optional **re-arm** level: after firing, the monitor latches and fires again only after the probability has dropped below it;
- an **action**: `stop` ends the turn and hands control back to you, `notify` shows one notice and carries on, `steer` tells the agent (a stored note it reads on its next step) and carries on;
- an **error policy**: `notify` (the default; after three errors in a row the monitor is off for the rest of the turn) or `pause`, which treats an error as firing with `stop`, for safety criteria;
- a cap on evaluations per turn (at most 200).

Monitors that share a timing, a model and a window are asked in one request. A window that has not changed since the last evaluation is not asked again. A message you type while the turn runs wins over a monitor's stop: the stop is skipped and your message is answered. Monitors are set in the desktop's Monitors dialog, with `biorouter session monitors`, or through `PUT /sessions/{session_id}/decision-settings`.

### The `decide` tool: screening a large input

The `decisions` platform extension gives the main agent two tools. It is on by default, and the delegation gate does not count it as a capability you granted (see [Workspace control](workspace-control.md#when-delegation-is-allowed)), so enabling it never makes a chat able to delegate by itself.

**`decide`** asks up to 16 questions of up to 200 chunks of text in one call. The agent splits a large input along natural boundaries (one clinical note, one function or file, one section), gives each chunk an id, and puts every question for a chunk in the same call, because the questions share the chunk and it is read once. One request goes to the decision model per chunk, several at a time (by default 8 for a remote model and 2 for a local one, `BIOROUTER_DECISION_MAX_CONCURRENT` to change it), and a chunk that fails never fails the batch.

| Argument | What it takes |
|---|---|
| `questions` | 1 to 16, name to `{instructions, type, options, levels, threshold, pass_option, at_least_level}`. `type` is `yes_no` (the default), `choice` or `score`; a choice's threshold needs `pass_option`, a score's needs `at_least_level` (0 is the first level). |
| `chunks` | 1 to 200 `{id, text}`. Ids are unique in the call; a chunk with no text is refused. |
| `model` | optional `provider/model`, one `decision_models` lists for this chat |

The tool's description teaches the model the limits that matter: a decision model only picks among the answers offered and never extracts a value, quotes the text or explains itself, so to get a value the agent asks whether a chunk states one and then reads those chunks itself; and probabilities can be wrong, so the agent reads the chunks it relies on and looks at the uncertain ones.

**What `decide` returns is one result per call, whatever its size.** Per question it gives how many chunks passed, how many answered yes (yes/no), how many were uncertain (P(yes) between 0.4 and 0.6), the count of each answer (choice and score), the highest probability, and the ids that passed or were uncertain, at most 40 of each (as few as 10 when a call asks many questions) with a count of the rest. Errors are grouped by message with the chunks they hit. A batch of at most 20 chunks and few questions also gets one compact line per chunk, because an agent conditioning its work on a handful of decisions needs each probability. A call in which no chunk was decided is reported as a tool error. The result names a batch id, and its structured content (`kind: "decide_batch"`) carries the same aggregates for the tool card. Every individual call, with the exact text it read, is in the decision history.

**An example.** A private chat holds 600 neurology notes. Baseten's models are Public and would be refused, so the decision model is a local one such as `ollama_decision/tev1:0.8b`. The agent calls `decide` three times, 200 notes each, with:

```json
{"questions": {
   "has_ms": {"instructions": "Does this note state the patient has multiple sclerosis?", "threshold": 0.9},
   "edss_stated": {"instructions": "Is an EDSS score stated in this note?", "threshold": 0.8},
   "course": {"instructions": "Which MS course is described?", "type": "choice",
              "options": {"rrms": "Relapsing-remitting", "spms": "Secondary progressive",
                          "ppms": "Primary progressive", "none": "Not stated"}}},
 "chunks": [{"id": "note-0001", "text": "Neuro f/u. 34F, RRMS dx 2019 ..."}]}
```

It forms the cohort from the `has_ms` passes, reads only the `edss_stated` notes in full to extract the EDSS value (EDSS runs 0 to 10 in steps of 0.5, which is 21 values, too many for a score question), and reports the notes in the uncertain band for review.

**`decision_models`** lists the models `decide` can use: each one's `provider/model` name, tier, whether it runs on this computer, its adapter, the question kinds it takes and its window. A model this chat may not use is listed with the reason, so the agent can say why rather than try it.

### Phase gates: holding back a phase until it met its goal

A phase of an orchestration plan may carry a decision gate: 1 to 8 yes/no questions, a threshold (default 0.8), a number of attempts (1 to 5, default 3), an optional model, and whether uncalibrated probabilities are accepted (default no). When the agent completes the phase, a decision model reads the phase's goal, its hand-off and its subagents' result lines, and the phase completes only when every question reaches the threshold. A failed gate keeps the phase in progress and hands the failing questions and their probabilities back to the agent to fix; when its attempts are used up the phase is blocked. An evaluation that fails with an error never passes and uses no attempt, and two in a row block the phase. The whole rule, with the state the gate reads and how it shows, is in [Orchestration phases](orchestration-phases.md#decision-gates).

### A decision model as the chat's own model

A decision model can be picked as a chat's main model, like any model; the model picker groups decision models separately. Such a chat has no language-model loop. You write the question and its options (or a question on the first line of your message) and paste the text; each message is asked the question, and the reply is one message whose text reads everywhere (`Yes · P(yes) 0.93 · tev1:0.8b`) and from which the desktop draws a decision card with a bar per option, the threshold, the model and whether the probabilities are calibrated. The question is stored on the chat as `decision_chat.v1`. A browser session (`biorouter serve`) cannot change a chat's model, so it cannot start a decision chat, but it shows the cards, the decision history and the gates.

## The decision log, and why the transcript never lists decisions

Decision calls can number in the thousands in one chat: a monitor after every iteration, a `decide` call over hundreds of chunks, a gate retried. So a decision call is never its own transcript message. Every call is one row of the per-chat `decisions` table, which records the origin (`monitor`, `tool`, `gate`, `chat` or `adhoc`), what asked (the monitor's id, the `decide` batch, the phase id) and its label, the turn, the batch and chunk, the model, its tier and adapter, the questions, the decisions with their probabilities, the outcome (`passed`, `failed`, `decided`, `error` or `refused`), the exact context sent (capped at 64 KiB), the latency, the token usage, and whether privacy was enforced.

Every surface reads aggregates and shows decision activity once per source:

| Where | What it shows |
|---|---|
| A `decide` call | One tool card summarising the batch, with a link to the history filtered to it. |
| A turn that ran monitors | One inline marker ("Monitors checked 18 times this turn, last P(form reached) 0.31") and one notice for each monitor that fired. Nothing while nothing fires. |
| A gated phase | A chip on the phase row: passed, failed with attempt n of m, or blocked. |
| The chat summary | A Decisions section with one row per source and criterion: count, last answer, a probability bar, state. |
| The decision history | The only place individual calls are listed: a paged list with filters (source, criterion, phase, turn, outcome, probability range, errors) and a detail pane with the exact context, the questions, every option's probability, the threshold, the model, the latency and the usage. |
| History | A per-chat count on the chat's row, never child rows. |
| The CLI and the terminal UI | One summary line per source per turn; `biorouter session decisions` pages and filters. |

Deleting a chat deletes its decision rows. A tool batch's rows carry the batch id as both `batch_id` and `origin_id`, and the label `decide`.

## Privacy

A decision hands chat content to a model the chat's row never records, so it is checked by its own privacy gate (Gate H, the alternate-provider rule) before anything is sent, every time, through the one decision service:

- **A private chat never reaches a public decision model.** Baseten's models are Public, so a private EHR chat uses a local Ollama decision model or another private one. The refusal comes before the request.
- **The classification is re-read before each chunk**, so a chat raised to Private in the middle of a batch stops reaching a public model at the next chunk.
- **Affiliation is checked beside the tier**: a model covered by one institution does not read another institution's private data.
- **A public chat may use a private (local) decision model.** Its answer lands back in the same chat and raises nothing. Whether to refuse that direction too is an open owner decision; it would also keep public chats off local decision models.
- **A Crew-scoped chat cannot use a decision model**, because a Crew grant fixes the one model its content may reach.
- **The master switch is sampled once per call** and threaded through, never re-read: on a tool path that is the tool call's capability. With the switch off, rows record `privacy_enforced = 0`.
- **A phase gate checks against the parent chat's tier**, and it reads only results the parent may hold: a subagent more private than its parent has its result withheld from the gate exactly as from the hand-off.
- **Every route that names a chat asks the same reach decision** as the other chat routes, and writing a chat's decision settings needs the proof that a person acted, because the settings name where the chat's content goes.
- **The log holds what was sent**, under the chat's own tier, and is deleted with the chat. Baseten's model cards state zero data retention by default; whether Baseten signs a BAA is not established, so it stays Public. Decision export is not built.

A decision model can only move probabilities among the answers offered; it never writes instructions. The only things a decision can cause are the bounded actions above: a monitor's stop, notice or steer, and a gate's pass or fail. The full gate list is in [Privacy tiers](../security/privacy-tiers.md).

## Settings

Per chat, in the chat's `extension_data`, written one key per transaction:

| Key | Holds |
|---|---|
| `decision_settings.v1` | `defaultModel` and `monitors` (at most 8) |
| `decision_chat.v1` | The question (and threshold) a decision chat asks |
| `orchestration_plan.v1` | Each phase's gate and the record of its evaluations |

Configuration keys:

| Key | What it does |
|---|---|
| `BIOROUTER_DECISION_PROVIDER`, `BIOROUTER_DECISION_MODEL` | The default decision model when a use names none |
| `BIOROUTER_DECISION_TIMEOUT` | Seconds a remote decision may take (default 10; a local model's first call gets at least 30, because it includes loading) |
| `BIOROUTER_DECISION_MAX_CONCURRENT` | Requests in flight per model |
| `BASETEN_API_KEY`, `BASETEN_DECISION_HOST` | Baseten's key and host |
| `OLLAMA_HOST` | Where the local Ollama is |

`BASETEN_DECISION_HOST` names where content goes, so it is a destination key: changing it over HTTP needs the proof that a person acted.

## Routes

| Route | What it does |
|---|---|
| `GET /decisions/models` | The decision models available now, with each one's tier and, for a named chat, why the chat may not use it |
| `POST /decisions/decide` | An ad hoc decision; with a session id it is checked against that chat and logged |
| `GET /sessions/{session_id}/decisions` | A page of the chat's decision log, filtered |
| `GET /sessions/{session_id}/decisions/summary` | The aggregates every surface draws: per source and criterion, and per recent turn |
| `GET /sessions/{session_id}/decisions/{decision_id}` | One decision with the exact context it was sent |
| `GET`, `PUT /sessions/{session_id}/decision-settings` | The chat's default model and monitors; `PUT` needs the proof |
| `GET`, `PUT /sessions/{session_id}/decision-chat` | A decision chat's question |

## The CLI and the terminal UI

- `biorouter decide --ask "<question>" [--model provider/model] [--choice name=description ...] [--levels a,b,c] [--threshold p] [--batch dir] [--session id] [--json]` asks one question about a text (stdin, `--text`, or one per file of a directory). Without `--session` nothing is logged, and a public model is named on stderr before the text is sent.
- `biorouter models decision list` lists the decision models available now, local first.
- `biorouter session decisions <session_id>` prints the aggregates first and then pages the calls, with filters for the source, the batch, the turn, the outcome and errors.
- `biorouter session monitors <session_id> list|add|remove` reads and writes a chat's monitors.
- The terminal UI shows one summary line per source per turn, draws a decision chat's replies as compact bars, and opens a `/decisions` overlay with the chat's aggregates and, on Enter, one call's detail.

The options in full are in the [CLI command reference](../cli/command-reference.md).

## For developers

| Path | What is there |
|---|---|
| `crates/biorouter/src/decision/` | `types.rs` (questions, decisions, the confidence and threshold arithmetic), `service.rs` (`DecisionService`: resolution, Gate H, size checks, concurrency, deadlines, the log sink), `log.rs` (the `decisions` table and its aggregates), `settings.rs`, `monitor.rs`, `window.rs`, `turns.rs` (the turn each chat is in, so every decision of a turn shares a turn id) |
| `crates/biorouter/src/providers/decision/` | `system_one.rs`, `logprob.rs`, `verbalized.rs`, `catalog.rs` (discovery and tiers), `chat.rs` (a decision model as a chat's provider) |
| `crates/biorouter/src/agents/decisions_extension.rs` | The `decide` and `decision_models` tools |
| `crates/biorouter/src/agents/orchestration_plan.rs` | Phase gates: validation, the gate's state, evaluation and the verdict transition |
| `crates/biorouter-server/src/routes/decisions.rs` | The routes |

Every decision goes through `DecisionService::decide`, `decide_many` or `decide_batch`; never call an adapter directly, because the privacy gate, the size check and the log live in the service. A tool or gate builds its `DecisionSession` from the calling chat with `DecisionSession::load(session_manager, session_id, cap.enforced())`, then sets the origin, its label and `decision::turns::current_turn(session_id)`. Tests use a fake `DecisionModelSource` (`agents::decisions_extension::test_support`) and never reach a real model:

```bash
cargo test -p biorouter --lib -- decision:: decisions_extension orchestration_plan
cargo test -p biorouter --lib -- workspace_extension::tests::a_gated
```

## Related documentation

- [Orchestration phases](orchestration-phases.md): phase plans, the completion barrier and the decision gate on a phase.
- [Workspace control](workspace-control.md): the delegation gate the `decisions` extension does not count toward.
- [Privacy tiers](../security/privacy-tiers.md): Gate H and the other gates a decision passes.
- [CLI command reference](../cli/command-reference.md): `decide`, `models decision list`, `session decisions` and `session monitors` in full.
- [Decision models in the desktop app](../desktop-ui/decision-models.md): the Decisions section, the decision history view, the Monitors dialog and the decision card.
