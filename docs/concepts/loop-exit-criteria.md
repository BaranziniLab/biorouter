# Loop exit criteria prototype

Local branch: `concept/agent-loop-exit-criteria`.

## Try it

1. Start the desktop UI using the repo's normal development commands.
2. Open **Exit criteria** in a chat composer (including a new, unsent chat).
3. Enable evaluation, write a criterion, and select a configured provider under
   **Public** or **Private**. Baseten has a dedicated decision catalog:
   `elephant-1`, `mammoth-1`, and `joey-1`. They use your existing
   `BASETEN_API_KEY` over the native `/v1/systemone` API, not chat completions.
   They are public hosted evaluators; no deployment is created by this change.
   Other providers retain their configured chat-model catalogs.
4. Choose **Evaluation timing**:
   - **Pre-send**: evaluate a bounded input excerpt before every main agent-model
     request. A failing check prevents that request and its tools from starting.
     The evaluator still receives the excerpt first. Preparation/helper calls,
     including earlier compaction and hooks, are outside this per-request gate.
   - **Post-response**: evaluate after the completed iteration, including its
     response and tools. A failure stops further iterations, but cannot retract
     streamed output or undo tool effects. Existing criteria default to this mode.
5. Choose a decision contract:
   - **Noul**: the probability of yes, from 0 to 1. Configure a threshold and
     interrupt either at/above it or below it. Equality belongs to the at/above
     side. This corrects the initial prototype's mistaken boolean interpretation.
   - **Boolean**: other providers retain the original exact `true`/`false`
     contract. Saved legacy Baseten boolean criteria use a 0.5 P(yes) cutoff;
     reopening them in the editor upgrades the controls to explicit Noul
     settings. Baseten's picker offers Noul, Choice, and Ranking.
   - **Choice**: provide 2–20 distinct strings and select the interrupting value.
   - **Ranking**: a numeric score in a configured range, interrupting strictly
     below a threshold. A score equal to the threshold continues. This is a
     scalar score, not a ranking of multiple candidates. With Baseten, provide
     2–10 ordered rubric descriptions, lowest first. The native `score` question
     uses a zero-based scale from 0 to the number of levels minus one and can
     return fractional scores. Other providers retain arbitrary numeric ranges.
6. Save, then send a message. New-chat criteria are persisted before that chat's
   first message is dispatched. Existing-chat changes apply at the next
   evaluation boundary.
7. On a failure, review the pause notice, adjust criteria or instructions, and
   explicitly send the next message. Saving settings never auto-resumes a turn.

## Contract and boundaries

Criteria live on the session row under `loop_exit_criteria.v1`. Drafts are scoped
by composer and kept in window session storage until adopted. API endpoints are
`GET` and `POST /agent/exit-criteria/{session_id}`; POST accepts
`{"criteria": <ExitCriteria or null>}`. A null criterion disables evaluation.

Each criterion stores a `timing` of `pre_send` or `post_response`. Missing timing
on older stored criteria defaults to `post_response`. Only the selected boundary
calls the evaluator; selecting Pre-send does not also run a post-response check.

Pre-send runs after the outgoing conversation is assembled but before the main
provider stream or tool bridge is opened. Post-response runs after tool results
are folded into the conversation, before another iteration or ordinary final-
answer exit gates. It evaluates a bounded recent
transcript excerpt that includes tool calls/results; it does not see the entire
workspace or full history. Generic chat evaluators must return exactly
`{"value": <typed result>, "reason": "..."}` without tool calls or markdown fences.
Native Baseten evaluators instead submit `state` and a named question to
`/v1/systemone`, read `answers.loop_exit`, and normalize the typed result for the
same threshold checks. They report the returned value, not an invented natural-
language rationale. Baseten's decision-only models are deliberately absent from
ordinary chat-model pickers.

These are per-iteration controls, not a universal network firewall. With
Post-response, streamed text has already been visible and tools have already
run; interrupting does not undo their side effects. With Pre-send, the main
request has not started, but preparation/helper traffic and earlier work are
not covered or undone. It uses the agent's existing resumable-for-user-input exit and
child supervision/settlement path, rather than feeding a correction back to the
agent automatically.

Errors (bad JSON, wrong type, unknown choice, out-of-range score, provider
failure, corrupt stored criteria, or a 30-second evaluator/provider-construction
timeout) pause the loop. User cancellation cancels the evaluation wait. Criteria
are off by default; disabled sessions do not make decision-model calls.

Privacy labels come from resolved provider instances, not editable model names.
Unresolved providers are unavailable in the picker. Private chats cannot use
public evaluators. The backend checks session classification and institutional
compatibility before saving and again before **every** model call, even when its
provider instance is cached. Cross-institution evaluation is conservatively
refused rather than adding a new consent mechanism in this prototype.

Each evaluation adds a model request and its latency/cost. Successful calls are
included in the reply budget and durable usage ledger without replacing the
agent's context-window gauge; malformed responses are billed as well. Timeouts
and failed calls cannot record usage the provider did not return. A native
System One adapter reuses the existing Baseten credential. No new credential or hosted deployment is created. Decision token usage is recorded;
this prototype does not supply decision-model dollar prices when the existing
pricing registry has no entry for them.

Do not use either timing option as a PHI disclosure firewall. Evidence is sent
to the selected evaluator even in Pre-send mode; other helper calls are outside
this gate, and the bounded excerpt can omit content. Use synthetic data when testing public
hosted evaluators.

## Validation

- `cargo test -p biorouter --test loop_exit_criteria --test baseten_decisions`
- `cd ui/desktop && npm run test:run -- src/components/bottom_menu/LoopExitCriteria.test.tsx src/store/loopExitCriteria.test.ts`
- `cargo fmt --all`
- `./scripts/clippy-lint.sh`
- `just generate-openapi`
- `biorouter run --workflow biorouter-self-test.yaml` (requires a configured model;
  use disposable fixtures and synthetic data only)


## Baseten API contract

The dedicated catalog uses these slugs:

| Picker name | Model API slug |
| --- | --- |
| elephant-1 | `decision-models/baseten-elephant-9b-latest` |
| mammoth-1 | `decision-models/baseten-mammoth-27b-latest` |
| joey-1 | `decision-models/baseten-joey-2b-latest` |

Contract sources: Baseten documentation pages
`inference/model-apis/decision-models` and `reference/inference-api/system-one`,
checked October 30, 2026. In System One, the question types are `noul`, `choice`,
and `score`; the UI's ordered ranking rubric maps to `score`. Private-provider
labels elsewhere in the picker do not make these public hosted endpoints private.


## Empty Baseten catalog after a development rebuild

The Electron app can reattach to a long-lived profile daemon after restarting.
Rebuilding a binary or closing Electron does not necessarily stop that service.
During development, both old and new binaries can report the same release
version, so version matching alone does not prove the new routes are loaded.

An HTTP 404 on `/agent/exit-criteria/models/baseten` means the attached service
lacks this route. Finish or cancel any active agent work before gracefully
restarting that profile's background service. The app can then reconnect using
the rebuilt binary. Reopen the editor or select **Retry model list**; the retry
preserves the unsaved criterion fields. No profile, chats, or API keys need to be
reset. Authentication errors (401/403) are reported separately from an old
backend. A failed catalog load never falls back to the general chat-model list.
