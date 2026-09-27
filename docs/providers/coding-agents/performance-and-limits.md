# Performance, limits and known gaps

> **What this is.** What these two providers cost in latency and prompt tokens, measured rather than
> estimated; why conversation history is flattened into one prompt each turn; what the streaming path
> does and does not cover; and the failure modes worth recognising before diagnosing them as
> something else.
> **Status:** Current. The figures below are single-machine measurements taken during
> implementation, quoted so a later change can be compared against something; re-measure before
> treating any of them as a budget.
> **Audience:** developers working on the coding-agent providers, and users wondering why the first
> token takes longer than with an API provider.

A turn on these providers is a process launch, not an HTTP request. That is the whole explanation for
most of what follows, and it is a fixed cost that no amount of prompt tuning removes.

## Latency: most of it is not the model

| Measurement | Value |
| --- | --- |
| A `claude -p` turn, wall clock | ~5.3 s |
| …of which was actually API time | ~1.8 s |
| A `codex exec` turn, wall clock | ~3.1 s |
| A cold `claude` start, warm dev machine | ~3.5 s |
| Probe ceiling, per CLI | 20 s |
| Turn ceiling, both providers | None, unless `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS` sets one |

So roughly 3.5 of the 5.3 seconds is process start, not inference. Two consequences worth
internalising:

- **Short turns feel disproportionately slow**, and long turns barely notice the overhead. These
  providers suit substantial questions, not rapid-fire exchanges.
- **The 20-second probe ceiling is generous on purpose.** A first run after a CLI update can be
  slower than a cold start, and reporting "not installed" for a slow-but-working binary is worse than
  waiting.

There is no turn ceiling by default. A coding-agent turn can legitimately run for a long time, for
example while it supervises a delegated task, so a turn ends when the child finishes or when the
user presses Stop, not when a clock runs out. A wedged child can always be stopped: Stop drops the
turn, which drops the child process, and `kill_on_drop(true)` reaps it. An operator who also wants a
hard resource ceiling sets `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS` to a positive number of
seconds, in `config.yaml` or the environment. A missing, invalid or zero value keeps the default.
Both providers first shipped with a fixed ceiling; it was removed on 2026-08-26, in favour of this
default.

## Prompt overhead: replacing the vendor prompt is the single biggest win

| Prompt | Tokens per call |
| --- | --- |
| Claude Code's default system prompt | 25,022 |
| BioRouter's system prompt, replacing it | 1,527 |
| Codex's own default preamble | ~15,000 |

A 16x reduction on Claude Code, from one flag. This is why `--system-prompt` **replaces** rather
than `--append-system-prompt` adding to the default, and why Codex's `baseInstructions` is set at
`thread/start` — see
[the flags that are not optional](child-agent-isolation.md#the-system-prompt-is-replaced-not-appended).

## Tool-schema cost, and why it is cheaper than it looks

A 60-tool surface measured about **5.2k prompt tokens on Claude — and they were cache reads**, not
fresh input. The tool list is a stable prefix, so both vendors' prompt caching absorbs most of its
cost after the first turn.

That is the same reason re-sending the conversation each turn is affordable; see below. It is also
why the usage figures are reported with the cache buckets kept separate: the live context gauge counts
the cached prefix as occupancy, while the billed total does not double-count it. The bucket
arithmetic differs per vendor and is described in
[usage accounting](how-it-works.md#usage-accounting-for-a-turn-that-billed-no-tokens).

## Why history is flattened rather than replayed

The `Provider` trait hands every call the entire conversation; both CLIs take a prompt. BioRouter
flattens the conversation into one prompt each turn. Two better-looking alternatives were tried and
rejected on evidence:

**Replay over `--input-format stream-json`.** That channel is an *interactive* one, not history
replay: each user message it receives starts its own complete turn, and an injected assistant message
is ignored outright. Measured — a three-message replay produced **two separate answers** rather than
one continued conversation.

**`--session-id` then `--resume`.** This does work across processes and is a legitimate later
optimisation. It is not the default because it moves the authoritative history **into the child**,
where BioRouter's compaction, message editing and `.biorouterignore`-driven redaction cannot reach
it. The two copies would silently diverge exactly when a long session gets compacted — the moment
you can least afford it. Flattening keeps BioRouter's transcript authoritative, which is the property
the `Provider` contract already assumes.

The cost is re-sending the conversation every turn, and prompt caching of the stable prefix puts the
marginal cost far below the naive reading.

Two details of the flattening are worth knowing when output looks odd:

- A lone user message is passed through **verbatim**. Wrapping a single question in scaffolding
  measurably degrades the answer.
- With history, earlier turns go inside a `<conversation_history>` block and the live instruction
  follows *outside* it with no role label. Without that separation both models tend to summarise the
  transcript instead of answering it. Tool traffic is included as text, capped at 4,000 characters
  per result, so one huge SQL or shell result cannot evict the actual conversation.

## Streaming: what is live, and what it does not change

Both providers stream. Each overrides `supports_streaming()` to return true
(`crates/biorouter/src/providers/claude_code.rs:764`, `crates/biorouter/src/providers/codex.rs:999`)
and implements `stream()` (`claude_code.rs:794`, `codex.rs:1017`), so the agent takes the streaming
branch and text appears as the model writes it rather than when the child exits. Tool calls stream
with it: each one the child makes is mirrored into the transcript as an ordinary tool card the
moment it is made, and resolves to success or failure in place — see
[the mirror](tool-bridge.md#the-mirror-how-a-bridged-call-becomes-a-visible-card).

The two providers differ only in what they decode:

- **Claude Code** is invoked with `--output-format stream-json` plus `--include-partial-messages`
  and `--verbose` (`claude_code.rs:815-823`). The CLI then emits raw Anthropic Messages events
  wrapped in a `stream_event` envelope, and `coding_agent/claude_stream.rs` unwraps them into the
  same `providers::formats::anthropic` decoder the Anthropic provider uses — for text and thinking
  only, because that decoder would otherwise mint tool requests the agent loop would dispatch a
  second time.
- **Codex** decodes the `item/agentMessage/delta`, `item/reasoning/*` and tool-item notifications
  the app server was already sending and the old fold discarded (`coding_agent/codex_stream.rs`).

What streaming does **not** change:

- **The process launch is still there.** Streaming removes the wait for the whole turn, not the
  ~3.5 s cold start before the first token can exist. Short turns still feel disproportionately
  slow.
- **The optional turn ceiling covers both paths.** When `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS`
  sets one, the blocking path's timeout wraps an await the streaming path never reaches, so each
  `stream()` applies the same limit inside itself, read from the same
  `coding_agent::turn_timeout()`. With the key unset, which is the default, neither path has one,
  and Stop is how a wedged turn ends.
- **A lead/worker pair streams only if both halves do.** `LeadWorkerProvider` forwards
  `supports_streaming()` as the **conjunction** of the two providers
  (`crates/biorouter/src/providers/lead_worker.rs:410-412`), so pairing a coding agent with a
  non-streaming provider gives a blocking turn.

Two limits remain, and both are explicit rather than accidental:

- **A bridged call needing human approval parks for the real user decision.** The bridge keeps the
  request leased while the desktop prompt is active, then returns the accepted result or an
  explicit refusal/expiry. The wait is bounded below the per-call transport deadline so an expired
  prompt becomes a result the child can act on, not a generic MCP timeout.
- **Unexpected Codex-local tool events remain visible.** Process feature gates and the read-only
  sandbox make these unreachable in normal operation. The decoder still mirrors one
  with a `child` marker so an upstream isolation regression cannot be mistaken for a bridged,
  policy-checked call.

Cancellation retains a hard backstop: dropping the provider stream aborts the reader task, which
drops the child, which `kill_on_drop(true)` reaps. Live Codex steering first sends
`turn/interrupt`, then starts the user's replacement instruction on the same thread; this is
separate from the hard-stop path and preserves the partial conversation.

## Failure modes worth recognising

| What you see | What it usually is |
| --- | --- |
| "not installed" on a machine where the terminal finds the binary | The desktop app's truncated `PATH`. Pin the path — [when BioRouter cannot find the binary](installing-and-signing-in.md#when-biorouter-cannot-find-the-binary). |
| An authentication error naming `apiKeyHelper` after a CLI upgrade | `assert_subscription_auth` fired: the run would have been billed to something other than the subscription. Most likely an `apiKeyHelper` in a Claude Code settings file, or `-p` having defaulted to `--bare`. |
| A card stuck on "indeterminate" | The probe ran but could not be understood — malformed `claude auth status` output, an `auth.json` with no `auth_mode`, or an expired refresh token behind a well-formed file. The reason is on the card. |
| The child says it cannot do something and asks you to approve it | Since #107 this should not happen: a bridged call routed to `needs_approval` [raises a real dialog and waits](tool-bridge.md#a-call-needing-approval-is-put-to-a-person-and-the-call-waits-107). If you see it anyway, the request expired, was dismissed, or the turn ended before you answered — the child's text says which, and a chat reply cannot approve it. Re-ask. |
| The child has no tools at all | No bridge was established — typically a CLI process with no HTTP server. The turn still answers from the conversation; this is deliberate degradation, not an error. |
| An empty response with nothing in the logs | For Codex, the app server exited before a terminal frame; the error carries a bounded tail of the child's stderr. For Claude Code, stderr is drained concurrently and included in the failure. |
| A turn that fails saying it "did not finish within" a number of seconds | `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS` is set and the turn ran past it. The child is killed rather than left holding the session. Raise the value, or remove the key so turns run until they finish. |
| "The operation timed out" on a slow Biorouter tool | The child's own per-call MCP deadline abandoned the request. Biorouter configures it (`timeout` for Claude Code, `tool_timeout_sec` for Codex) — see [the child's per-call deadline](tool-bridge.md#the-childs-per-call-deadline-is-configured-not-discovered-110) — so seeing this means the field was not honoured. A tool that waits should have clamped to `bridge::bridged_call_budget()` and returned a partial result instead. |
| A watch that reports a shorter wait than you asked for | Working as intended (#110). The wait was clamped to fit the transport, and the reply names both numbers. Watch again; the completions already reported are not repeated. |

## Related documentation

- [How the coding-agent providers work](how-it-works.md) — the mechanism these costs come from,
  including the flattening and the usage arithmetic.
- [The tool bridge](tool-bridge.md) — the grant lifetime and the construction-time rule a `stream()`
  obeys, the mirror that makes bridged calls visible, and what a large tool surface buys.
- [Streaming and tool-call parity](streaming-and-tool-call-parity.md) — the design record behind the
  streaming path, including what shipped and what was deliberately deferred.
- [What the child agent may not do](child-agent-isolation.md) — the flags behind the prompt saving.
- [Installing and signing in](installing-and-signing-in.md) — the setup-side failure modes in the
  table above.
- [Common problems and fixes](../../troubleshooting/common-problems-and-fixes.md) — general app and
  daemon troubleshooting.
