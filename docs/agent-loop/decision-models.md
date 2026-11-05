# Decision models

> **What this is.** The guide to decision models in Biorouter: fast models that return a typed answer (yes or no, one of named options, a level on a scale) with the probability of every outcome, instead of text. It covers where they come from, the four ways a chat uses them (monitors, the agent's `decide` tool, phase gates, and a decision model as the chat's own model), how accurate a screen is and how to check one, the decision log and the rule that keeps thousands of decisions out of the transcript, usage and billing, the privacy rules, the settings, the routes, and the CLI and terminal UI.
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

Every decision also says whether its probabilities are **calibrated**: true for a decision model's own distribution, and for token log-probabilities that covered at least 90% of the probability mass; false for a number a model wrote. Calibrated says only where the numbers come from. They are not validated on your data: no calibration error figures are published for any of these models, fitting calibration to your own data is not built, and the same note can score 0.36 under one wording of a question and 0.94 under another (see [Accuracy, and checking a screen](#accuracy-and-checking-a-screen)). The `decision_models` tool, the CLI and the desktop say the same words: System One gives "the model's own probabilities, not validated on your data".

## Where decision models come from

| Provider id | What it is | Tier | How it is found |
|---|---|---|---|
| `baseten_decision` | Baseten's decision models (`joey-1`, `elephant-1`, `mammoth-1`) over the System One API, `POST /v1/systemone` | Public, no affiliation | The `decision-models/` entries of Baseten's live catalog, else the three known slugs, when `BASETEN_API_KEY` is saved. The host is `BASETEN_DECISION_HOST` (default `https://inference.baseten.co`). Each has a 16,384-token window. |
| `ollama_decision` | A local Ollama (0.35 or later) model whose `/api/show` capabilities include `decision`, for example `tev1:0.8b`, over the same System One API | Private on a loopback host (the self-hosted rule); Public for a cloud tag or a model `/api/show` says is served remotely | `OLLAMA_HOST`, the way Ollama's own client reads it. A model's window is its Modelfile's `num_ctx`, else its trained window, else 8,192 tokens. |

Two fallbacks let a chat model answer decision questions when no decision model is available, and every decision records which adapter produced it:

1. **Log-probabilities.** Each option is mapped to one token (`yes`/`no`, or `A`, `B`, ...), the model is asked for one token at temperature 0, and the probability of each option is summed over its spelling variants and renormalised. Implemented for Ollama's `/api/chat` and for the OpenAI chat format. A backend that drops `logprobs` silently is never read as a certain answer; the decision falls back to the next adapter and says why.
2. **Verbalized.** The model writes an answer and a number as structured output. These probabilities are always marked uncalibrated, and a phase gate refuses them unless the gate allows it.

**Measured on `tev1:0.8b`** (Ollama 0.40.14, an Apple Silicon laptop, 2026-11-04): about 22 ms for one question on new text once the model is loaded, about 15 ms per question when several share one text, 1.6 s for the first call including the load, and identical answers on repeat runs. The vendor latencies published for Baseten's models (20 to 49 ms) were not verified; the [live Baseten test](#for-developers) prints them.

**`tev1:0.8b`'s window is 2,048 tokens**, so chunks for it must be short; `decision_models` reports each model's window. Two numbers are easy to mix up here:

- **2,048** is what Ollama reports (`num_ctx 2048` in the model's `/api/show` parameters, though the model was trained on 262,144). It is what Biorouter lists for the model and what the size check before sending holds every request to.
- **8,192** (`OLLAMA_DEFAULT_DECISION_CONTEXT` in `crates/biorouter/src/providers/decision/catalog.rs`) is only what Biorouter assumes for an Ollama decision model whose `/api/show` states neither a `num_ctx` nor a trained window. The model registry also carries 8,192 for `tev1:0.8b`, because `tests/context_windows.rs` requires a window for every advertised model; no gauge shows it, because a decision chat draws none.

**Which model a decision uses**, first match wins:

1. the model the use names (a monitor's, a gate's, or the `decide` tool's `model` argument, which must be one `decision_models` lists);
2. the chat's default decision model (`defaultModel` in its decision settings);
3. `BIOROUTER_DECISION_PROVIDER` / `BIOROUTER_DECISION_MODEL` (the model may be written `provider/model`);
4. the first listed model this chat may use, in this order: local models first; among the local Ollama models the shipped default `tev1:0.8b` (`OLLAMA_DEFAULT_DECISION_MODEL`) first and the rest by name; Baseten's smallest first (`joey-1`, `elephant-1`, `mammoth-1`). A model Ollama cannot load (next section) stays listed, after every model that loads, with its reason, and the automatic choice skips it.

The order is `order_ollama_entries` in `catalog.rs`. It replaced a sort by name alone, which made the choice an accident of spelling and put a copy Ollama refuses ahead of the default. `biorouter decide` asks the service for its model (`DecisionService::model_entry_for`), which applies the order the daemon's decisions use, so a terminal and the desktop never pick different models.

When the automatic choice finds nothing, the answer depends on why. If models are listed but a privacy rule rules out every one for this chat (a private chat with only Baseten's listed, say), the decision is **refused**, as it is for a named public model: 403 on a route, a refused row with the lock styling, and a `notify` monitor is off for the rest of the turn at once. If nothing is listed, it is **no model**: 404, an error row, and a `notify` monitor is off for the rest of the turn only after three errors in a row. A listed model Ollama cannot load does not count as listed, and its sentence is added to the answer.

### A local copy Ollama cannot load

Ollama can list a model it then refuses to load: a copy in a format a newer Ollama no longer supports. On the machine this feature was built on, `clef-flash:latest` is such a copy (built from a GGUF file on 2026-10-03, and kept as it is). Ollama 0.40.14 lists it with the `decision` capability and answers every request for it with HTTP 400 and a message to re-download it. Before the fix it sorted ahead of `tev1:0.8b`, so every decision that named no model failed, with advice about the window that did not apply. The rule now, for any such model:

- **It is found at discovery, before any decision.** Biorouter asks the Ollama server about each decision-capable model once, with a System One request whose questions are empty: `{"model": "<name>", "state": "", "questions": {}}`. Ollama checks the format first, so a copy it cannot load gets the format refusal, and a copy it can load gets a complaint about the empty questions. Neither loads the model (measured on Ollama 0.40.14: `/api/ps` listed nothing before or after, and each answer took about 5 to 7 ms), and the request carries no text. The answer is remembered per server, model and `/api/tags` digest for the life of the process, so a re-pulled copy is asked again. A timeout, an unreachable server, a 404 or a server fault is never remembered and never marks a model. Cloud models are not asked.
- **It stays listed, last, with the fix.** Its reason reads: "Ollama decision model `<model>` cannot be loaded: Ollama says this copy is in a format it no longer supports. Re-download it with `ollama pull <model>`, or remove it with `ollama rm <model>`." `biorouter models decision list --format json` carries it as `loadRefusal`, and `decision_models` and `GET /decisions/models` give it as the reason the model is unavailable.
- **The automatic choice skips it.** A use that names the model still opens it, and fails with the same sentence rather than the generic one about the window or a malformed question.
- **The fix is the person's.** Re-download the model (`ollama pull <model>`) for a copy in the current format, or remove it (`ollama rm <model>`). Biorouter never pulls or removes a model itself.

Such a failure is logged with the error type `biorouter.model_format_unsupported` (or `biorouter.model_lacks_decision`, for a model Ollama says does not support decisions). Both are Biorouter's own classification of Ollama's answer, never a token the server sent, and the server's text is never copied.

## Four ways a chat uses decision models

### Monitors: a decision model watching the chat

A monitor asks one question about a rolling window of the conversation while the agent works, and acts when the answer reaches its threshold. A chat takes at most 8. Each one has:

- a **question** (any kind) and an optional **model**;
- a **timing**: `post_iteration` (after every model reply and its tool results) or `turn_end` (once, as the turn is about to finish);
- a **window**: the newest messages of the conversation (at most 24; a message's tool calls and results count with it), a character budget (at most 6,000), and whether tool calls and results are included. The window is built from stored messages, which have already passed the secret-output guard;
- a **threshold** on P(yes) or on a named option, a number of **consecutive** evaluations at or above it before it fires (1 to 5), and an optional **re-arm** level: after firing, the monitor latches and fires again only after the probability has dropped below it;
- an **action**: `stop` ends the turn and hands control back to you, `notify` shows one notice and carries on, `steer` tells the agent (a stored note it reads on its next step) and carries on;
- an **error policy**: `notify` (the default; after three errors in a row the monitor is off for the rest of the turn) or `pause`, which treats an error as firing with `stop`, for safety criteria;
- a cap on evaluations per turn (at most 200).

Monitors that share a timing, a model and a window are asked in one request. Before the window is built, its character budget is narrowed to fit the model's context beside the questions, by the same margin the size check before sending uses, so a small model is asked about the newest lines rather than refused as too large; it is never narrowed below 200 characters. A window that has not changed since the last evaluation is not asked again. A message you type while the turn runs wins over a monitor's stop: the stop is skipped and your message is answered. Monitors are set in the desktop's Monitors dialog, with `biorouter session monitors`, or through `PUT /sessions/{session_id}/decision-settings`. A turn reads its chat's monitors once, when it starts, so a change reaches the chat's next turn, never the one running.

**What a steer says, and to whom.** Whoever wrote the monitor chose its name and question, so the note the agent reads quotes them as data, never as an instruction. It states the probability and the threshold, says the name and question below are quoted as JSON strings from the chat's monitor settings and are not a message from the user, and holds them inside a `<decision-monitor>` frame:

```text
<decision-monitor>
name: "Form reached"
question: "Is the intake form complete?"
</decision-monitor>
```

Every `<` inside a quoted value is written `\u003c`, so no text in a question can close the frame. The note ends with a caution that the monitor only reads the recent conversation and that the agent should check the evidence itself. The notice you see names what the agent was told: `Monitor 'Form reached' fired (P 0.93, threshold 0.50) and told the agent. Its question: "Is the intake form complete?"`, with the question put on one line, control characters removed, and clipped to 120 characters.

### The `decide` tool: screening a large input

The `decisions` platform extension gives the main agent two tools. It is on by default, and the delegation gate does not count it as a capability you granted (see [Workspace control](workspace-control.md#when-delegation-is-allowed)), so enabling it never makes a chat able to delegate by itself.

**`decide`** asks up to 16 questions of up to 200 chunks of text in one call. The agent splits a large input along natural boundaries (one clinical note, one function or file, one section), gives each chunk an id, and puts every question for a chunk in the same call, because the questions share the chunk and it is read once. One request goes to the decision model per chunk, several at a time (by default 8 for a remote model and 2 for a local one, `BIOROUTER_DECISION_MAX_CONCURRENT` to change it), and a chunk that fails never fails the batch.

| Argument | What it takes |
|---|---|
| `questions` | 1 to 16, name to `{instructions, type, options, levels, threshold, pass_option, at_least_level}`. `type` is `yes_no` (the default), `choice` or `score`; a choice's threshold needs `pass_option`, a score's needs `at_least_level` (0 is the first level). |
| `chunks` | 1 to 200 `{id, text}`. Ids are unique in the call; a chunk with no text is refused. |
| `model` | optional `provider/model`, one `decision_models` lists for this chat |

The tool's description teaches the model the limits that matter. A decision model only picks among the answers offered and never extracts a value, quotes the text or explains itself, so to get a value the agent asks whether a chunk states one and then reads those chunks itself. Probabilities can be wrong, so the agent reads the chunks the result lists under review before it reports them. For a screen it teaches the choice form: the abbreviations the text may use spelled out in the question, options `yes` and `no` that each say in one sentence what they mean, a threshold near 0.5 and `pass_option` `yes`. The `decisions` extension is on by default in every chat, so the description stays short and the review rule is stated in the result.

**What `decide` returns is one result per call, whatever its size.** Per question it gives how many chunks passed, how many answered yes (yes/no), how many are to review, the count of each answer (choice and score), the highest probability, and the ids that passed or are to review, at most 40 of each (as few as 10 when a call asks many questions) with a count of the rest. Errors are grouped by message with the chunks they hit. A batch of at most 20 chunks and few questions also gets one compact line per chunk, because an agent conditioning its work on a handful of decisions needs each probability. A call in which no chunk was decided is reported as a tool error. The result names a batch id, and its structured content (`kind: "decide_batch"`) carries the same aggregates for the tool card, with each question's review list as `review`, `reviewIds` and `moreReview`. Every individual call, with the exact text it read, is in the decision history.

**The review list.** A chunk is to review when its pass probability is neither a clear no nor a clear pass: from 0.2 (`REVIEW_FLOOR`) up to, but not including, the larger of the question's threshold and 0.8 (`CLEAR_PASS`), or up to 0.8 when the question has no threshold. The probability read is the threshold's own when the question has one (P(yes), P(pass_option), or P(at_least_level or higher)), and P(yes) for a yes/no question without one. A yes/no question always has a review list; a choice or score has one only with a threshold, and its count is otherwise `null`. The rule is `needs_review` in `crates/biorouter/src/agents/decisions_extension.rs`.

- Each question's paragraph says, for example, `3 to review (P from 0.20 up to 0.90: not a clear no, not a clear pass)`, then lists the ids on a `review:` line.
- A per-chunk line never says a bare fail, which an agent reads as a no. It says `pass`, `pass, review` (a pass under 0.8), `below threshold`, `below threshold, review`, or `review` for a yes/no question with no threshold.
- The result ends: "A chunk listed under review is unresolved, not a no: read it before you report it, or report it as unresolved. A decision model can also be confidently wrong, so check a few passes."
- The band is fixed, with no setting and no argument, because the agent that reads the list would otherwise choose how long it is.

The rule replaced a fixed band of P(yes) 0.4 to 0.6 that ignored the threshold. At a 0.9 threshold that band listed 2 of the 4 true notes the bare question below missed, and printed the other two as `fail`, which an agent reads as a no. The new rule lists all four.

**An example.** A private chat holds 600 neurology notes. Baseten's models are Public and would be refused, so the decision model is a local one such as `ollama_decision/tev1:0.8b`. The agent calls `decide` three times, 200 notes each, with:

```json
{"questions": {
   "has_ms": {"type": "choice",
              "instructions": "Does this note state the patient has multiple sclerosis (MS, RRMS, SPMS or PPMS)?",
              "options": {"yes": "The patient has multiple sclerosis (MS, RRMS, SPMS or PPMS).",
                          "no": "The patient does not have multiple sclerosis."},
              "threshold": 0.5, "pass_option": "yes"},
   "edss_stated": {"instructions": "Is an EDSS (Expanded Disability Status Scale) score stated in this note?",
                   "threshold": 0.5},
   "course": {"instructions": "Which MS course is described?", "type": "choice",
              "options": {"rrms": "Relapsing-remitting", "spms": "Secondary progressive",
                          "ppms": "Primary progressive", "none": "Not stated"}}},
 "chunks": [{"id": "note-0001", "text": "Neuro f/u. 34F, RRMS dx 2019 ..."}]}
```

It reads the notes listed under review for `has_ms` before it reports them, builds the cohort from the `has_ms` passes, and reports any note it cannot resolve as unresolved. It reads only the `edss_stated` notes in full to extract the EDSS value (EDSS runs 0 to 10 in steps of 0.5, which is 21 values, too many for a score question). `course` has no threshold, so it gives counts and no review list. The example's note scores 0.53 on the bare question, which an earlier version of this example asked at threshold 0.9, so it failed there; the choice form gives it 0.88.

**`decision_models`** lists the models `decide` can use: each one's `provider/model` name, tier, whether it runs on this computer, its adapter (System One is "the model's own probabilities, not validated on your data"; log-probabilities are "token log-probabilities of a chat model, not validated on your data"), the question kinds it takes and its window. A model this chat may not use is listed with the reason (a privacy rule, or a copy Ollama cannot load), so the agent can say why rather than try it.

### Phase gates: holding back a phase until it met its goal

A phase of an orchestration plan may carry a decision gate: 1 to 8 yes/no questions, a threshold (default 0.8), a number of attempts (1 to 5, default 3), an optional model, and whether uncalibrated probabilities are accepted (default no). When the agent completes the phase, a decision model reads the phase's goal, its hand-off and its subagents' result lines, and the phase completes only when every question reaches the threshold. A failed gate keeps the phase in progress and hands the failing questions and their probabilities back to the agent to fix; when its attempts are used up the phase is blocked. An evaluation that fails with an error never passes and uses no attempt, and two in a row block the phase. The whole rule, with the state the gate reads and how it shows, is in [Orchestration phases](orchestration-phases.md#decision-gates).

### A decision model as the chat's own model

A decision model can be picked as a chat's main model, like any model; the model picker groups decision models separately. Such a chat has no language-model loop. You write the question and its options (or a question line at the start of your message, beginning with `Q:` or `?`, as in `Q: Is the form signed?`) and paste the text; each message is asked the question, and the reply is one message whose text reads everywhere (`Yes · P(yes) 0.93 · tev1:0.8b`) and from which the desktop draws a decision card with a bar per option, the threshold, the model and whether the probabilities are calibrated. The question is stored on the chat as `decision_chat.v1`. A browser session (`biorouter serve`) cannot change a chat's model, so it cannot start a decision chat, but it shows the cards, the decision history and the gates.

- **A decision chat on a local model is private before its first answer.** The decision model is the chat's bound provider, so Gate B (the privacy check at the start of every turn, which raises a chat to its model's tier) raises the chat before the question is asked: Private on a loopback Ollama, with the reason `turn:ollama_decision`. No decision raises a chat's classification; this raise comes from the bind.
- **There is no context to measure.** Each message is asked on its own, and nothing adds to the chat's token total. So no surface draws a context gauge for a decision chat: the desktop composer shows the folder and the cost only, and the terminal UI and the classic CLI print no context meter. A typed `/compact` answers that there is no context to compact, instead of asking the decision model to summarise. The only limit is per message, and a message too long for the model is refused with a sentence that names the model's window.
- **A failed or refused question still names its log row.** The chat chooses the request id before it asks (`DecisionService::decide_as`), so the reply carries `decisionId` whatever the outcome, and the desktop and terminal draw it as a Not decided or Refused card with a link to the row. Only a question that fails validation, which writes no row, names none.

## Accuracy, and checking a screen

A decision model's probability depends on the wording as much as on the text. The figures below were measured on `tev1:0.8b` (Ollama 0.40.14, 2026-11-05) on synthetic notes; the bare question's figures repeated exactly on a second run. The bare question is "Does this note state the patient has multiple sclerosis?"; the choice form is `has_ms` in the example above, and the column shows its P(yes).

| Note | Has MS | Bare question | Choice form |
|---|---|---|---|
| "34F, RRMS dx 2019, EDSS 2.0, on ocrelizumab." | yes | 0.36 | 0.94 |
| "Neuro f/u. 34F, RRMS dx 2019 on ocrelizumab..." | yes | 0.53 | 0.88 |
| "52M, SPMS, EDSS 6.5, walks with a cane." | yes | 0.38 | 0.91 |
| "61M, PPMS dx 2017 on ocrelizumab, EDSS 5.0." | yes | 0.40 | 0.85 |
| "MS, stable on natalizumab." | yes | 0.48 | 0.65 |
| "PMH: HTN, T2DM, RRMS." | yes | 0.34 | 0.59 |
| "...MRI normal. MS ruled out." | no | 0.00 | 0.00 |
| "40F, migraine. FHx MS (mother)..." | no | 0.88 | 0.58 |
| "27F, ?MS, referred for MRI brain and LP." | no | 0.04 | 0.04 |
| "33M, NMOSD, AQP4+, on rituximab." | no | 0.08 | 0.03 |

At a 0.9 threshold the bare question passed 1 of the first 5 true notes it was tried on: only "45F, relapsing MS since 2015, new T2 lesion." (1.00), which the choice form was not run on. The choice form at 0.5 passed all six true notes in the table. What the measurements teach:

- **Spell out the abbreviations in the question itself.** With them only in the option descriptions, three of the true notes scored lower (0.76, 0.67 and 0.54).
- **Say what each answer means, in one short sentence.** The yes/no kind carries no meaning for each answer, which is why the choice form with options `yes` and `no` is taught. A question that also listed exclusions (suspected, ruled out, family history) lowered some true notes, one to 0.27.
- **Keep a screen's threshold near 0.5 and read the review list.** A high threshold such as 0.9 fails true cases on a small model.
- **Check some passes.** The family-history note scored 0.88 on the bare question: a confident wrong yes that no probability rule catches. Under the choice form it scored 0.58, a pass under 0.8, so it is on the review list.

No larger local decision model and none of Baseten's were tried on these notes, so whether size fixes the family-history case is not known.

**To check a screen before relying on it:**

1. Label 30 to 50 chunks by hand, covering the hard cases: abbreviations, negations ("ruled out"), suspicion ("?MS"), family history, and diseases that look alike (CIS, NMOSD).
2. Run the same questions, threshold and model over them.
3. Count the misses at the threshold, and how many of them the review list caught.
4. Repeat after changing a question, the threshold or the model.

## The decision log, and why the transcript never lists decisions

Decision calls can number in the thousands in one chat: a monitor after every iteration, a `decide` call over hundreds of chunks, a gate retried. So a decision call is never its own transcript message. Every call is one row of the per-chat `decisions` table, which records the origin (`monitor`, `tool`, `gate`, `chat` or `adhoc`), what asked (the monitor's id, the `decide` batch, the phase id) and its label, the turn, the batch and chunk, the model, its tier and adapter, the questions, the decisions with their probabilities, the outcome (`passed`, `failed`, `decided`, `error` or `refused`), the exact context sent (capped at 64 KiB), the latency, the token usage, and whether privacy was enforced.

Every surface reads aggregates and shows decision activity once per source:

| Where | What it shows |
|---|---|
| A `decide` call | One tool card summarising the batch, with a link to the history filtered to it. |
| A turn that ran monitors | One inline marker ("Monitors checked 18 times this turn, last P(form reached) 0.31") and one notice for each monitor that fired. Nothing while nothing fires. |
| A gated phase | A chip on the phase row: passed, failed with attempt n of m, or blocked. |
| The chat summary | A Decisions section with one row per source and criterion: count, last answer, a probability bar, state. |
| The decision history | The only place individual calls are listed: a paged list with filters (source, criterion, phase, turn, outcome, probability range, errors) and a detail pane with the exact context, the questions, every option's probability, the threshold, the model and its tier, the latency and the usage. |
| History | A per-chat count on the chat's row, never child rows. |
| The CLI and the terminal UI | One summary line per source per turn; `biorouter session decisions` pages and filters. |

Deleting a chat deletes its decision rows; its decision spend stays counted (see [Usage and billing](#usage-and-billing)). A row's id is the request id every decision of that request carries, so a decision chat's reply names its row as `decisionId` with no lookup, and `GET /sessions/{session_id}/decisions?decisionId=` returns that one row. A failure row gets an id of its own, unless its caller chose the id before asking, as a decision chat does.

A batch stopped before any chunk was sent (no model the chat may use, a privacy refusal, a question kind the model cannot take) is one row, not one per chunk. So is a batch stopped partway, when the chat turns private while it runs: the chunks not yet asked are not asked, each reports the refusal in the tool's result, and one refused row names them ("Not sent: 460 of 500 chunks were stopped partway through the batch, when a privacy or Crew rule refused this chat's content to the decision model. Chunks: ..."). The turn's tally counts it once.

A tool batch's rows carry the batch id as both `batch_id` and `origin_id`, and the label `decide`. The summary groups them by label, so every `decide` call in a chat is one source (and one line per turn) with no origin id, while monitors stay one source per monitor id and gates one per phase id; the history still filters one batch with `batchId`.

## Usage and billing

Every decision call about a chat (a monitor check, one chunk of a `decide` batch, a gate, a decision chat's reply, an ad hoc decision that names a chat) is added to the usage ledger once, beside its log row. The ledger row is keyed `decision:<row id>`, so a retried write bills once, and it is filed under the decision provider and the model as requested (a Baseten slug such as `decision-models/baseten-joey-2b-latest`, which is what the price table is keyed by). The write happens where the log row is written, so it covers the desktop's daemon, the CLI's interactive sessions, `biorouter decide --session` and `biorouter acp` alike. A request that fails after the model answered, for example with an answer the service cannot use, keeps its usage.

Where the numbers appear:

- **The Usage panel and `biorouter usage`** count decision tokens and priced dollars in the totals and against the monthly limit, and show them on their own line ("Decision models: 620 requests, 1.2M tokens, $0.12"). They are requests, never turns: a bucket's turn count leaves them out. A local decision model has no meter, so its line reads "not metered", its requests count as $0, and they do not mark the totals as partial ("at least"). That is a deliberate exception, because a local model is the automatic choice and would otherwise mark almost every month; a plain Ollama chat model still marks them, and so does a Baseten decision model with no known price, which Baseten still bills.
- **The chat's own cost under the composer** reads the same ledger, so it counts priced decision calls.
- **Home** counts decision tokens in its heatmap and token tiles, lists decision models by their short names (`joey-1`) in its Models tab after the chat models, and never picks one as the Favorite model.
- **A deleted chat's** decision spend stays in the anonymous per-day total the ledger keeps for deleted chats, as its turns' does.

What the write never touches, whoever asks:

- **The context gauge.** The columns the gauge reads (`total_tokens`, `input_tokens`, `output_tokens`) are left alone; only the chat's accumulated counters move.
- **The chat's place in the sidebar.** The write never stamps `updated_at`, which the sidebar sorts by and which only a change made to a chat moves (R-30), so asking a decision model about a chat, even 1,000 chunks from outside any turn, never moves it. A decision made inside a turn needs no stamp of its own, because the turn's message and token writes move the chat already.
- **A scheduled chat's link** to its schedule (`schedule_id`).

**Prices.** Baseten bills its decision models per million input tokens: `joey-1` $0.01, `elephant-1` $0.035, `mammoth-1` $0.10, output $0.00 (an answer is about one token per question). For 600 notes of about 2,000 tokens, that is about $0.012 on `joey-1` and $0.12 on `mammoth-1`. The bundled table (`BASETEN_DECISION_PRICING` in `crates/biorouter/src/providers/pricing.rs`) is dated 2026-11-04 (`BUNDLED_DECISION_PRICING_AS_OF` in `live_catalog.rs`), and the live Baseten feed's decision price wins only when it was fetched on or after that date. A model's alias (`joey-1`, which a decision chat binds) and its slug (which the log records) price the same. `ollama_decision` is never priced: a local model has no meter, and a cloud tag bills an ollama.com subscription, not tokens. Neither decision provider ever takes a price from the general model catalog.

**What is not counted.**

- A decision with no chat (`biorouter decide` without `--session`, `POST /decisions/decide` without a `sessionId`), because ledger rows belong to a chat. So `biorouter decide` without `--session` ends with one stderr line saying what the run used (`usage: 2 requests on baseten_decision/mammoth-1: 1,200,000 input and 3 output tokens, about $0.12. Without --session this is not recorded in Biorouter's usage.`; a local model's line has no price). Each decision's `usage` is also in the route's answer and in the CLI's `--json` output.
- A request that times out. The provider may bill it, and there is no usage to report.
- Cached input. System One reports only input tokens and the feed lists cached input at $0.00, so a cached shared prefix is billed here at the full input rate, a small overstatement. Baseten's flex and priority tiers are not modelled; Biorouter sends neither.

### The reply budget

A reply's dollar cap (`BIOROUTER_REPLY_BUDGET_USD`, the per-reply budget known as BR-35) counts the priced decision calls the reply made: its monitors, its `decide` chunks and its gates, added at the reply's next iteration boundary (`BudgetTracker::record_external_usd` in `crates/biorouter/src/agents/budget.rs`). Dollars only: a decision call never counts toward the token cap (`BIOROUTER_REPLY_BUDGET_TOKENS`), which is set for the chat's own context and which a single 1.2M-token batch would trip. An unpriced local decision call adds nothing, and it does not switch the dollar cap off for the reply.

## Privacy

A decision hands text to a model the chat's row never records, so it is checked by its own privacy gate (Gate H, the alternate-provider rule) before anything is sent, every time, through the one decision service:

- **A private chat never reaches a public decision model.** Baseten's models are Public, so a private EHR chat uses a local Ollama decision model or another private one. The refusal comes before the request.
- **The classification is re-read before each chunk**, so a chat raised to Private in the middle of a batch stops reaching a public model at the next chunk, and the chunks not yet asked are not sent.
- **Affiliation is checked beside the tier**: a model covered by one institution does not read another institution's private data.
- **A public chat may use a private (local) decision model** (owner ruling, 2026-11-05). The model reads only text the chat's public model already has: a monitor's window is the stored conversation, the `decide` tool's chunks are text the chat model wrote into its call, and a gate reads the hand-off and its children's result lines. Sending that to this computer discloses nothing more. Nothing but probabilities over the answers the chat offered comes back into the chat or its model, so no decision call raises the chat's classification. The one free text a decision can produce is a logprob model's verbalized fallback, which keeps up to 2,000 characters of the model's reply in the log row's raw JSON, never in the transcript. Refusing this direction would push the same text to a hosted model off this computer, and raising the chat to Private would stop its public model running the next turn. A local decision model does not make a public chat safe for patient data: put patient data in a private chat.
- **A decision chat on a local model is private**, by the bind rather than by the decision: see [A decision model as the chat's own model](#a-decision-model-as-the-chats-own-model).
- **An ad hoc decision about a chat carries the caller's text**, not the chat's. `POST /decisions/decide` with a `sessionId` and `biorouter decide --session` send text the caller supplies (a request body, stdin, `--text`, the files of `--batch`). It is checked against the named chat's tier and kept in that chat's decision history, at that chat's reach. For a chat that is not private, with privacy tiers on, that means public chats can read it, so the CLI says so on stderr before sending and the route's answer says so in its `note`. A public model is named on stderr before sending, with `--session` as without it. The CLI resolves its model once and pins it for every request of the run, so a long `--batch` never moves from a local model to a hosted one partway; if the pinned model goes away, the run fails.
- **A Crew-scoped chat cannot use a decision model**, because a Crew grant fixes the one model its content may reach.
- **The master switch is sampled once per call** and threaded through, never re-read: on a tool path that is the tool call's capability. With the switch off, rows record `privacy_enforced = 0`.
- **A phase gate checks against the parent chat's tier**, and it reads only results the parent may hold: a subagent more private than its parent has its result withheld from the gate exactly as from the hand-off.
- **Every route that names a chat asks the same reach decision** as the other chat routes. Writing a chat's decision settings or a decision chat's question over HTTP needs the proof that a person acted on a daemon that holds a user-action key (the desktop's). On a daemon that holds none (`biorouter serve`, a hand-run `biorouterd`) no request can carry the proof, so the reach gate is the whole HTTP gate there, steer monitors included (owner ruling, 2026-11-05), except that a subagent's chat is refused with the sentence `/reply` and `/agent/stop` give there. `biorouter session monitors` with no reachable daemon writes the store directly, under the tier rule alone, on any daemon. Every write reaches the chat's next turn, never the one running, and a steer's notice shows the person the question the agent was told. [SD-11](../deployment/serve-decisions.md#sd-11--stop-works-on-a-daemon-with-no-key-steering-does-not-and-a-subagents-tab-stays-the-persons) lists these doors beside the others that reach a running chat.
- **The log holds what was sent**, under the chat's own tier, and is deleted with the chat. For a monitor, the `decide` tool, a gate or a decision chat that is the chat's own content; for an ad hoc decision it is the caller's text. Baseten's model cards state zero data retention by default; whether Baseten signs a BAA is not established, so it stays Public. Decision export is not built.

A decision model can only move probabilities among the answers offered; it never writes instructions. The only things a decision can cause are the bounded actions above: a monitor's stop, notice or steer, and a gate's pass or fail. Gate H is asked again before every call whatever a setting names, so no setting can carry a private chat to a public model. The full gate list is in [Privacy tiers](../security/privacy-tiers.md).

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

`BASETEN_DECISION_HOST` names where content goes, so it is a destination key: changing it over HTTP needs the proof that a person acted. The `decide` tool's review band has no key (see [the review list](#the-decide-tool-screening-a-large-input)).

## Routes

| Route | What it does |
|---|---|
| `GET /decisions/models` | The decision models available now, with each one's tier and, for a named chat, why the chat may not use it (a privacy rule, or a copy Ollama cannot load) |
| `POST /decisions/decide` | An ad hoc decision. With a session id it is checked against that chat and logged in its history, and for a chat that is not private the answer's `note` says the text is kept where public chats can read it |
| `GET /sessions/{session_id}/decisions` | A page of the chat's decision log, filtered |
| `GET /sessions/{session_id}/decisions/summary` | The aggregates every surface draws: per source and criterion, and per recent turn |
| `GET /sessions/{session_id}/decisions/{decision_id}` | One decision with the exact context it was sent |
| `GET`, `PUT /sessions/{session_id}/decision-settings` | The chat's default model and monitors. `PUT` needs the proof on a daemon that holds a key; on one that holds none it is refused only for a subagent's chat |
| `GET`, `PUT /sessions/{session_id}/decision-chat` | A decision chat's question; `PUT` is gated like the settings |

A decision refused by a privacy or Crew rule answers 403, including the case where every listed model is ruled out for the chat; no model at all answers 404.

## The CLI and the terminal UI

- `biorouter decide --ask "<question>" [--model provider/model] [--choice name=description ...] [--levels a,b,c] [--threshold p] [--batch dir] [--session id] [--json]` asks one question about a text (stdin, `--text`, or one per file of a directory). It chooses its model once, by the order above, and uses it for every request of the run. Without `--session` nothing is logged and no tier is enforced. A public model is named on stderr before the text is sent. With `--session` naming a chat that is not private (privacy tiers on), a second line says the text is kept in that chat's decision history, where public chats can read it. Without `--session`, a last line on stderr gives the run's requests, tokens and, for a priced model, dollars, because nothing in Biorouter records them.
- `biorouter models decision list` lists the decision models available now, in the order the automatic choice tries them, with a copy Ollama cannot load last.
- `biorouter session decisions <session_id>` prints the aggregates first and then pages the calls, with filters for the source, the batch, the turn, the outcome and errors.
- `biorouter session monitors <session_id> list|add|remove` reads and writes a chat's monitors.
- The terminal UI shows one summary line per source per turn, draws a decision chat's replies as compact bars, and opens a `/decisions` overlay with the chat's aggregates by source; Enter on a source shows that source's last calls, and Esc goes back. In a decision chat neither the terminal UI nor the classic CLI prints a context meter.

The options in full are in the [CLI command reference](../cli/command-reference.md).

## For developers

| Path | What is there |
|---|---|
| `crates/biorouter/src/decision/` | `types.rs` (questions, decisions, the confidence and threshold arithmetic), `service.rs` (`DecisionService`: resolution, Gate H, size checks, concurrency, deadlines, the log sink), `log.rs` (the `decisions` table, its aggregates, and the usage ledger write), `settings.rs`, `monitor.rs` (including the steer note and notices), `window.rs`, `turns.rs` (the turn each chat is in, so every decision of a turn shares a turn id) |
| `crates/biorouter/src/providers/decision/` | `system_one.rs`, `logprob.rs`, `verbalized.rs`, `catalog.rs` (discovery, the load probe, the order and tiers), `chat.rs` (a decision model as a chat's provider) |
| `crates/biorouter/src/agents/decisions_extension.rs` | The `decide` and `decision_models` tools, and the review rule |
| `crates/biorouter/src/agents/orchestration_plan.rs` | Phase gates: validation, the gate's state, evaluation and the verdict transition |
| `crates/biorouter/src/session/session_manager.rs` | `SessionManager::apply_side_usage_event`, the ledger write that never touches the gauge, the schedule link or `updated_at` |
| `crates/biorouter-server/src/routes/decisions.rs` | The routes |

Every decision goes through `DecisionService::decide`, `decide_as`, `decide_many` or `decide_batch`; never call an adapter directly, because the privacy gate, the size check and the log live in the service. A tool or gate builds its `DecisionSession` from the calling chat with `DecisionSession::load(session_manager, session_id, cap.enforced())`, then sets the origin, its label and `decision::turns::current_turn(session_id)`. Tests use a fake `DecisionModelSource` (`agents::decisions_extension::test_support`) and never reach a real model:

```bash
cargo test -p biorouter --lib -- decision:: decisions_extension orchestration_plan
cargo test -p biorouter --lib -- workspace_extension::tests::a_gated
cargo test -p biorouter --test decision_system_one
cargo test -p biorouter --test decision_baseten_config
cargo test -p biorouter-server --lib -- routes::decisions
cargo test -p biorouter-server --test decision_settings_no_user_key
```

`decision_system_one` covers both System One servers against local mock servers. Most answers were written from Baseten's and Ollama's documentation; Baseten's refusal of a missing key and Ollama's format refusal are the bodies the real servers returned. It also covers the load probe and the order of a listing that holds a copy Ollama refuses. `decision_baseten_config` is its own binary because it sets `BASETEN_API_KEY` and `BASETEN_DECISION_HOST` in the environment.

**The live Baseten test** makes real, billed calls, so every test in it is `#[ignore]`d and CI runs none of them. Run it from the worktree root, in a shell where you exported `BASETEN_API_KEY`:

```bash
cargo test -p biorouter --test decision_baseten_live -- --ignored --test-threads=1 --nocapture
```

The binary runs in its own sandbox (a temporary `BIOROUTER_PATH_ROOT`, the keyring off) and reads the key from the environment only. A test that needs the key and finds none prints `SKIPPED <test>: BASETEN_API_KEY is not set...` and passes, so read the output. Two tests need no key: one checks that a wrong key is refused without being echoed, and one reads the published Baseten feed and checks it lists the three decision models at 16,384 tokens. `BASETEN_DECISION_HOST` points the run at another host, and one thread keeps a Basic account under its 15 requests a minute.

## Related documentation

- [Orchestration phases](orchestration-phases.md): phase plans, the completion barrier and the decision gate on a phase.
- [Workspace control](workspace-control.md): the delegation gate the `decisions` extension does not count toward.
- [Privacy tiers](../security/privacy-tiers.md): Gate H and the other gates a decision passes.
- [Serve decisions, SD-11](../deployment/serve-decisions.md#sd-11--stop-works-on-a-daemon-with-no-key-steering-does-not-and-a-subagents-tab-stays-the-persons): who may write a chat's monitors on a daemon with no user-action key, and why.
- [The sidebar chat list](../desktop-ui/sidebar-chat-list.md): what moves a chat, and why a decision's usage never does.
- [CLI command reference](../cli/command-reference.md): `decide`, `models decision list`, `session decisions` and `session monitors` in full.
- [Decision models in the desktop app](../desktop-ui/decision-models.md): the Decisions section, the decision history view, the Monitors dialog, the decide card and the decision card.
