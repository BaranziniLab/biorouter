# How the coding-agent providers work

> **What this is.** The mechanism behind the two subscription-billed providers — what each one
> spawns, which models each advertises and the CLI version each of those needs, where each vendor's
> credential lives and who reads it, how the CLI is located without spawning anything, how a
> BioRouter conversation becomes one prompt, how a turn streams and its tool calls are mirrored into
> the transcript, and how usage is accounted for a turn that billed no tokens.
> **Status:** Current.
> **Audience:** developers working on the provider layer, and maintainers verifying the
> integration.

Every other provider in BioRouter is an HTTP client: a base URL, a credential, a request. These
two are not. `claude_code` and `codex` spawn a coding-agent CLI that the **user** installed and
signed in to, and that CLI resolves its own credential and bills the user's own plan. BioRouter
never sees, stores, brokers, proxies or transmits the credential — it starts a process. That one
fact drives every design decision on this page, and the reasoning for why it is also the
compliance boundary is on [the compliance page](compliance.md).

## The two providers

| | Claude Code | Codex |
| --- | --- | --- |
| Registry id | `claude_code` | `codex` |
| Display name | **Claude Code** | **Codex** |
| Executable | `claude` | `codex` |
| Config key naming the executable | `CLAUDE_CODE_COMMAND` | `CODEX_COMMAND` |
| Surface driven | `claude -p` (headless) | `codex app-server` (JSON-RPC over stdio) |
| Default model | `claude-fable-5-1` | `gpt-6-astra` |
| Advertised models | `claude-fable-5-1`, `claude-opus-5`, `claude-sonnet-5` (1,000,000 each), `claude-haiku-4-5` (200,000) — all accept images | `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5` (1,050,000 each), `gpt-5.3-codex-spark` (400,000, **text only** — no images) |
| Unlisted models | Accepted — a user may type an alias such as `sonnet` by hand | Accepted |
| Vendor documentation | [Claude Code headless mode](https://code.claude.com/docs/en/headless) | [Codex CLI](https://developers.openai.com/codex/cli) |
| Privacy tier | `Public`, not `runs_locally` | `Public`, not `runs_locally` |
| Turn timeout | 30 minutes | 30 minutes |

> **A note on the name.** The provider is shown as **Claude Code**, which is a deliberate deviation
> from Anthropic's Agent SDK branding guidelines: those permit "Claude Agent", "Claude", or
> "*product* Powered by Claude", and list "Claude Code" and "Claude Code Agent" as not permitted for
> a third-party product label. BioRouter shipped it as "Claude Agent" first for exactly that reason
> and changed it on the maintainer's instruction, because it is what users of this tool actually
> call it. It is recorded here so a future reader knows it was a decision rather than an oversight.
>
> Only the label changed. The registry id stays `claude_code` because BioRouter's pricing table keys
> on it, and a rename there would re-open the fabricated-pricing bug described under
> [usage accounting](#usage-accounting-for-a-turn-that-billed-no-tokens). Both the label and the id
> are pinned by unit tests, so neither drifts silently.

The model lists are advertised as concrete ids rather than the CLIs' `sonnet`/`opus` aliases.
An alias has no entry in `MODEL_CONTEXT_WINDOWS`, so it would silently take the 128k default and
the settings UI would display the wrong context window for a 1M-token model. Both spellings work
when a user types one; only the advertised set is pinned. The authoritative catalogue is in
[`crates/biorouter/src/providers/claude_code.rs`](../../../crates/biorouter/src/providers/claude_code.rs)
and [`crates/biorouter/src/providers/codex.rs`](../../../crates/biorouter/src/providers/codex.rs),
not on this page.

## Which models you can pick depends on the CLI version

Both catalogues track the vendor's current lineup, and the newest model in each needs a recent CLI.
The floors are the vendor's, not BioRouter's:

| Model | Minimum CLI | What an older CLI says |
| --- | --- | --- |
| `claude-fable-5-1` | `claude` **2.1.251** | `400 Claude Code 2.1.235 does not support this model; version 2.1.251 or newer is required. Run 'claude update', or update the Claude desktop app, then try again.` |
| `gpt-6-astra` | `codex-cli` **0.153.4** | `The 'gpt-6-astra' model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again.` |

BioRouter surfaces that text **verbatim** and stops the turn. It does not quietly retry on an older
model: the vendor's message already names the fix, and answering from a model you did not pick is
worse than an error you can act on. Every other advertised model in both catalogues answered on the
older CLI as well as the newer one, so the gate is per-model, not per-catalogue.

> **Note.** The two sources for the Claude floor disagree by four patch versions — Anthropic's
> model-configuration page says v2.1.255, the API's own 400 says 2.1.251. BioRouter documents the
> number the API returns, because that is the one a user will read on their own screen.

Aliases still work, and on a CLI new enough for Fable 5.1 they land where you would expect: `fable`
and `best` both resolve to `claude-fable-5-1`, `opus` to `claude-opus-5`, `sonnet` to
`claude-sonnet-5`, and `haiku` to `claude-haiku-4-5-20251001`. Advertising concrete ids rather than
these aliases is the context-window decision above, not a claim that aliases are unsupported.

> **Why `claude-sonnet-4-6` is no longer advertised.** A bare `claude-sonnet-4-6` gets a
> **200,000**-token window on a Max plan, not the 1M this table used to claim: the million needs
> the `[1m]` suffix *and* usage credits. What this change does is take the id out of the
> **picker**, and nothing more. Typed by hand it still runs, and its gauge still reads 1M, because
> the window comes from `MODEL_CONTEXT_WINDOWS` in
> [`crates/biorouter/src/model.rs`](../../../crates/biorouter/src/model.rs), which is keyed by
> model name alone: the anthropic, Bedrock and Databricks providers serve that id at a genuine 1M,
> and one shared entry cannot say "1M there, 200k here". Keeping the claim that narrow is the
> point — an id nobody is offered is better than one offered with a number the CLI does not
> honour, and the typed case stays wrong until the window becomes a per-provider fact.
> `claude-fable-5` left for a duller reason — Fable 5.1 supersedes it at the same tier — and it
> too remains usable by hand.

On the Codex side, `gpt-5.4` and `gpt-5.4-mini` were **retired by OpenAI on 2026-08-31**, with
`gpt-5.6-terra` and `gpt-5.6-luna` named as their replacements. They no longer appear in
`model/list`, and a turn on one fails with
`400 The '<id>' model is not supported when using Codex with a ChatGPT account`. `gpt-5.3-codex`
was rejected by that same 400 and is listed as deprecated by OpenAI; the surviving member of that
family is the text-only research preview `gpt-5.3-codex-spark`, which is a **different id** rather
than a rename BioRouter could have followed.

Measured on 2026-09-08 against `claude` 2.1.235 and 2.1.260, and `codex-cli` 0.147.0 and 0.153.4,
by running every id through the CLI itself. The Claude windows are what the CLI reports back in
`modelUsage`; Codex's `model/list` carries no context-window field on either version, so those
windows come from OpenAI's published model pages and only the *existence* and effort ladder of each
id is a local measurement.

## Where each credential lives, and who reads it

BioRouter reads neither. It asks each CLI what state the CLI believes it is in.

| | Claude Code | Codex |
| --- | --- | --- |
| Credential store on macOS | The **Keychain**, service `Claude Code-credentials` | `~/.codex/auth.json` |
| Credential store elsewhere | `~/.claude/.credentials.json` (Linux and Windows only) | `~/.codex/auth.json` |
| Override for the store's location | — | `CODEX_HOME` |
| How BioRouter reads the state | `claude auth status`, which emits JSON | Reads **only** the `auth_mode` field of `auth.json`, then cross-checks liveness with `codex login status` |
| The subscription value | `authMethod` is `claude.ai` | `auth_mode` is `chatgpt` |

The macOS detail matters when debugging: on a Mac there is no `~/.claude/.credentials.json` to
inspect, so its absence says nothing about whether the user is signed in. Codex's `auth.json` is
read for one field and no others — not touching the tokens is what makes "credentials never pass
through BioRouter" a property of the code rather than an intention. For a turn, BioRouter points
`CODEX_HOME` at an ephemeral directory containing only a filesystem link to that `auth.json`.
Codex therefore retains subscription authentication without loading the user's `config.toml` or
its personal MCP servers.

## Finding the CLI without spawning anything

Three questions are deliberately kept apart, because they cost very different amounts:

| Question | Cost | Where it is asked |
| --- | --- | --- |
| Where is the binary? | A few `stat` calls | `resolve_binary`, called from each provider's `from_env` |
| Which version is it? | One process spawn | `probe`, called from the HTTP route and the CLI |
| Is the user signed in, and to what? | One process spawn or one file read | `probe` |

⚠ **The split is load-bearing.** `GET /config/providers` constructs **every** configured
provider under a three-second timeout in order to sample its tier and affiliation. A `from_env`
that spawned `claude auth status` would slow — or time out — the whole settings page, so nothing
in the spawning half may be reached from it. The probe's own ceiling is 20 seconds, which is
generous on purpose: a cold `claude` start measured about 3.5 seconds on a warm dev machine, and
the first run after an update can be slower.

Resolution uses BioRouter's augmented search path (`SearchPaths`, with the npm prefixes added)
rather than a bare `Command::new("claude")`. The reason is a real failure mode: `biorouterd` is
launched by the Electron main process with a `PATH` of roughly `<dir of biorouterd>:<inherited>`,
and a GUI app's inherited `PATH` on macOS excludes `/opt/homebrew/bin`, `~/.local/bin` and every
npm prefix — so the naive spawn reports "not installed" on a machine where the user's terminal
finds the binary instantly. A path the user pins in `CLAUDE_CODE_COMMAND` or `CODEX_COMMAND`
wins outright, which is the escape hatch for the toolchain managers `SearchPaths` does not know
about; see [installing and signing in](installing-and-signing-in.md#when-biorouter-cannot-find-the-binary).

The child also gets the augmented `PATH`, not just the resolved absolute path: both CLIs shell
out to `git`, `ripgrep` and `node` on their own account, and `codex` in particular is an npm shim
that execs a sibling native binary.

## What each provider actually runs

### Claude Code: `claude -p`

BioRouter drives the CLI rather than `@anthropic-ai/claude-agent-sdk`, and that is a decision
rather than an omission. Anthropic's headless documentation states that it covers using the Agent
SDK via the CLI (`claude -p`) — `-p` **is** the SDK's CLI form. The SDK itself is a library for
Python and TypeScript only, and its own overview directs another language to run the CLI as a
subprocess with `-p` and `--output-format json`. BioRouter is Rust, so this is the documented
route. The SDK would also be worse here: it directs third-party developers to API-key
authentication under Anthropic's Commercial Terms, and the npm package ships nothing but the same
binary anyway.

One turn is one `claude -p` invocation. Every argument the builder produces is fixed except the
output format — the streaming path appends `--include-partial-messages` and `--verbose` afterwards
and changes nothing else — and the
prompt goes on **stdin**, never in `argv` — a flattened conversation can exceed the platform's
argv limit. The full argument list and the security reasoning for four of the flags are on
[what the child agent may not do](child-agent-isolation.md).

There are two output formats, and the ordinary chat path takes the streaming one.
`--output-format json` gives one result object per turn: BioRouter scans the emitted lines for the
`system`/`init` frame (which carries `apiKeySource`), any `system`/`api_retry` frame (which
carries an error category), and the `result` object (which carries the answer text and the
authoritative usage). `--output-format stream-json`, with `--include-partial-messages` and
`--verbose`, gives the same information as a stream of framed events — see
[a turn streams](#a-turn-streams-and-its-tool-calls-are-mirrored) below. In both cases stderr is
drained **concurrently** with stdout, so the CLI's own diagnostic survives a failure and a chatty
child cannot deadlock on a full pipe.

### Codex: `codex app-server`

`codex exec --json` is the obvious choice and it is the wrong one, for a reason that only appears
once tools are involved. `exec` has no channel for answering an approval, so the moment the agent
wants to call a tool the call fails with "user cancelled MCP tool call" — verified — and the only
ways around it are `--approve-for-me`, which forces a workspace-write sandbox, or
`--dangerously-bypass-approvals-and-sandbox`. Both hand the child more authority than BioRouter
wants it to have.

`codex app-server` speaks newline-delimited JSON-RPC 2.0 over stdio and routes every approval
back to the host as a **server-originated request** that blocks the turn until it is answered.
That is the shape BioRouter needs, because the decision stays here. The transport is therefore
genuinely bidirectional: a client that only reads responses deadlocks the first time the agent
wants to do anything. Inbound messages are classified exactly as the protocol defines them — an
`id` with no `method` is a response, a `method` with an `id` is a request that must be answered,
a `method` with no `id` is a notification.

A turn is `initialize` → `initialized` → `thread/start` → `turn/start`, then a pump that reads
notifications and answers requests until `turn/completed` or `turn/failed` arrives. Two details
are easy to get wrong and are pinned by tests:

- `turn/start` is acknowledged immediately and the turn continues as notifications, so the pump
  must run **alongside** the awaited request, not after it.
- The advisory error notification's method is the bare literal `error`, not `thread.error`. It
  breaks the dotted convention every sibling notification follows, so a match written from the
  type names alone misses it. It is recorded but is not fatal — only `turn/completed` or
  `turn/failed` ends the turn.

`initialize` identifies BioRouter honestly, as `clientInfo.name = "biorouter"`. Some harnesses
shape that field to look like the vendor's own first-party client; doing so is exactly what the
vendors' terms target, so BioRouter says who it is.

`app-server` also exposes `account/read`, `account/rateLimits/read` and `model/list`, and its
whole protocol can be regenerated with `codex app-server generate-json-schema --out DIR` rather
than reverse-engineered.

## A turn streams, and its tool calls are mirrored

Both providers implement `Provider::stream()` and report `supports_streaming() == true`
(`crates/biorouter/src/providers/claude_code.rs:764,794`,
`crates/biorouter/src/providers/codex.rs:999,1017`), so the agent takes the streaming branch. What
the user sees is what an API provider gives: markdown appearing as the model writes it, and a tool
card for every call the child makes — visible the moment it is made, resolving to success or
failure, expandable to its arguments and its result.

**Text and thinking.** Claude Code's `stream_event` frames wrap raw Anthropic Messages-API events,
so `coding_agent/claude_stream.rs` unwraps them and feeds the decoder the Anthropic provider already
uses. Codex's `item/agentMessage/delta` and `item/reasoning/*` notifications are decoded by
`coding_agent/codex_stream.rs`. Every chunk of one answer carries a stable message id, which is what
merges the chunks into a single row instead of one row per token.

**Tool calls: the mirror.** Neither child's calls can be dispatched by the agent loop, because the
child already ran them — a bridged call executed on BioRouter's side of
[the tool bridge](tool-bridge.md), behind the inspectors, permission mode, `.biorouterignore`, vault
and privacy Gate C, before the frame describing it ever reached this crate. So the provider *mirrors*
the call instead: it mints an already-resolved `ToolRequest` / `ToolResponse` message pair, of
exactly the kind every API provider produces, which the existing tool-card components render with no
frontend change.

Each half of the pair carries a marker — the reserved `biorouterProviderExecuted` key in the
per-tool provider metadata, valued `bridged` or `child`
(`crates/biorouter/src/providers/coding_agent/mirror.rs:63`). The agent loop honours it by
persisting and yielding the message and dispatching nothing
(`crates/biorouter/src/agents/agent.rs:7182-7202`). Without the marker the loop would dispatch the
mirrored request — a second, real execution of a call that already ran, because
`categorize_tool_requests` filters on message content and never reads metadata. The full reasoning,
the `bridged`/`child` distinction and the remaining gaps are on
[the tool bridge](tool-bridge.md#the-mirror-how-a-bridged-call-becomes-a-visible-card).

⚠ **The bridge URL is read at construction time, never from a poll.** `Agent::reply` scopes the URL
around the call that *builds* the stream, not around consuming it, so both `stream()`
implementations read the URL and spawn the child before returning the stream. The same applies to
the turn ceiling: the blocking path's 30-minute timeout wraps an await the streaming path never
reaches, so each `stream()` carries its own deadline (`claude_code.rs:891-902`, `codex.rs:649-659`).

## The conversation becomes one prompt

The `Provider` trait hands every call the entire conversation; both vendor CLIs take a *prompt*.
BioRouter flattens: a lone user message is passed through verbatim, and anything earlier is
wrapped in a `<conversation_history>` block with the live instruction following it, outside the
block and without a role label. Tool traffic is included as text (`[called tool: …]`,
`[tool result: …]`, capped at 4,000 characters per result) because the child otherwise looks up
what BioRouter already knows.

Replaying history as turns was measured and does not work, and `--session-id`/`--resume` works
but would move the authoritative transcript into the child. Both results, and the cost of
re-sending each turn, are on
[performance, limits and known gaps](performance-and-limits.md#why-history-is-flattened-rather-than-replayed).

## Keeping the run on the subscription

A stray credential in the daemon's environment would silently reroute the run onto a metered API
account, and the failure is quiet — which is what makes it dangerous. Claude Code's documented
auth precedence has seven levels, and `ANTHROPIC_API_KEY` outranks the subscription OAuth token;
in `-p` mode it is used with no approval prompt. Measured on a dev box: with `ANTHROPIC_API_KEY`
set to a bogus value the run reported `apiKeySource: "ANTHROPIC_API_KEY"` and still succeeded.

Two independent defences follow from that, and both are required.

1. **Remove the possibility.** Every child — including the probes — is configured through
   `configure_subscription_child`, which strips the credentials that could divert it. The list is
   grouped by what each group reroutes *to*: first-party API keys (`ANTHROPIC_API_KEY`,
   `ANTHROPIC_AUTH_TOKEN`, `OPENAI_API_KEY`, `CODEX_API_KEY`), base-URL redirection
   (`ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`, `OPENAI_API_BASE`), Claude Code's alternate-backend
   switches (`CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY`), and the AWS, Google Vertex and
   Azure credential families behind them. The same function then applies
   `prepare_agent_child_command`, which removes the **daemon's** own secrets — in particular
   `BIOROUTER_SERVER__SECRET_KEY`, without which the child would be a fully authenticated client
   of BioRouter's REST API.
2. **Assert what happened.** Each provider asks its own child, at turn time, which account the run
   would be billed to, and stops the turn if the answer is not the subscription. Reaching either
   assertion is a real defect rather than a user error, because defence 1 should have made it
   impossible — but the two failure modes are separate credentials, so both are checked.

   - **Claude Code**: `system/init`'s `apiKeySource` must be `none` or absent. Anything else stops
     the turn with an authentication error naming the likely cause, an `apiKeyHelper` in a Claude
     Code settings file, which outranks subscription sign-in.
   - **Codex**: one `account/read` round trip on the app-server connection the turn is already
     holding, taken **before** `thread/start`, so a refused turn costs no tokens at all. The
     account's `type` must be `chatgpt`; `apiKey`, `amazonBedrock` and any future type stop the
     turn. Deliberately read from the running app server rather than from `~/.codex/auth.json` —
     the file is what the server *would* load, but the process is what actually bills, and
     `CODEX_HOME`, a login after the file was read, or an install authenticating some other way
     all separate the two.

   Both fail **open on silence**: a missing `apiKeySource`, a missing `account`, an app server too
   old to know `account/read`, or one that ignores the method until a ten-second timeout expires,
   all continue the turn. A check that could not be obtained is not evidence of a metered run, and
   turning "nothing" into a refusal would take a working CLI away from the user over a suspicion.
   The metered case reports itself explicitly; it is never inferred from silence.

⚠ **`configure_subscription_child` must be called LAST**, after every `.env()`, `.envs()`,
`.arg()` and `.current_dir()` on the command. Both scrubs manipulate the same environment map
that `.env()` writes, so a later `.env()` re-admits what was removed.

The scrub is deliberately **not** "every credential-looking variable". Over-stripping has its own
regression history here — a truncated `PATH` once broke every Homebrew binary — and an
extension's declared credential is none of BioRouter's business. A unit test asserts both halves:
every listed key is removed, and `SPOKEAGENT_PASSCODE`, `PATH` and `HOME` survive untouched.

The probes run under the **same** scrubbed environment as a real turn, which is the difference
between reporting what is stored and reporting what will happen: `claude auth status` answers
"claude.ai" even when a stray `ANTHROPIC_API_KEY` is exported, so probing with the ambient
environment would describe a credential BioRouter's own runs will never use.

## Usage accounting for a turn that billed no tokens

Both providers set the `provider` field on the usage row they return. Left unset, accounting
falls back to the model name and `canonical_model_pricing` invents a per-token catalogue price
for a run that billed a subscription. `pricing::blocks_fallback_pricing` lists `claude_code` and
`codex` for the same reason, and it is why the registry ids may not be renamed.

The two vendors report token counts under different conventions, and BioRouter's four buckets
must not overlap:

- **Claude Code** reports the Anthropic shape, where `input_tokens` already excludes both cache
  buckets — exactly BioRouter's invariant, so nothing is subtracted.
- **Codex** follows OpenAI's convention, where `input_tokens` is the whole prompt count and
  `cached_input_tokens` is a cached *subset* of it. The cached part is subtracted out, otherwise
  the billed total double-counts every cached token and stops reconciling with a vendor bill.

In both cases the `total_tokens` reported for the live context gauge deliberately *includes* the
cached prefix, because that is context occupancy rather than a bill.

## Errors are typed, not collapsed

Setup failures and turn failures are mapped onto typed provider errors so the retry layer can
tell a credential problem it must not retry from a blip it should: `authentication_failed`,
`oauth_org_not_allowed` and `billing_error` become authentication errors, `rate_limit` becomes a
rate-limit error, `overloaded`/`server_error` become server errors, `max_output_tokens` becomes a
context-length error. A missing CLI stays an execution error, because no retry and no credential
change fixes it. Every setup error names the exact command the user should run — that is the whole
reason the setup errors are built separately from the generic mapper. The four states and their
messages are on [installing and signing in](installing-and-signing-in.md).

## Thinking effort climbs the CLI's own ladder

Both CLIs expose a taller reasoning scale than the API providers do —
`low, medium, high, xhigh, max` — so these two providers do not reuse the shared
`ReasoningEffort::provider_effort()` helper, which stops at `high` because `high` is the top of the
*API* scale. They climb their own ladder instead:

| BioRouter `/effort` | Claude Code | Codex |
| --- | --- | --- |
| `quick` | `low` | `low` |
| *default* | `high` | `high` |
| `deep` | `max` | the model's top rung — `max` on `gpt-6-astra` and the 5.6 family, `xhigh` elsewhere |

Three things follow that are worth knowing before you rely on it.

**The default is no longer silent.** Everywhere else in BioRouter the default effort means "say
nothing and let the model choose". Here it emits `high`, so every turn — including from someone who
has never typed `/effort` — asks for more reasoning than the vendor default (`medium` on
`gpt-6-astra`, `low` on `gpt-5.6-sol`). That is deliberate: a coding agent is what you reach for
when the work is hard. It also costs thinking tokens against your own subscription on every turn,
which is the trade being made.

**Codex's ladder is per-model, so `deep` lands differently.** Four of the six advertised Codex
models advertise `max` and get it — `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra` and
`gpt-5.6-luna`; `gpt-5.5` and `gpt-5.3-codex-spark` stop at `xhigh`. Sending an unadvertised rung
is *accepted* rather than rejected — measured on `gpt-5.5`, which has no `max` — so the failure
mode is a silent clamp or a silent ignore, and there is no way to tell which from the outside. An
ignore would fall back to the model's default and quietly deliver *less* than `deep` asked for, so
BioRouter sends only what the model advertises.

That rule is also why the hidden review model `codex-auto-review` is not on the `max` list. It
answers a turn on every CLI version probed, but it appears in no `model/list` response, so its
ladder cannot be read — and a rung nobody can verify is exactly the silent-clamp case above.

**`deep` is not `ultra`.** Three Codex models advertise an `ultra` rung above `max` — `gpt-6-astra`,
`gpt-5.6-sol` and `gpt-5.6-terra` — and Claude Code has no such level at all (passing one makes the
CLI warn and fall back to its default, which is a silent downgrade rather than an error). `deep` is
the strongest *ordinary* tier; the delegating mode
above it is not something `/effort deep` should buy without being asked for.

## Using one for a single task, without rebinding the chat

A coding agent does not have to become the whole conversation's provider. A chat bound to any
other model can hand one task to `claude_code` or `codex` by spawning a subagent and naming the
provider in the spawn's settings:

```json
{
  "instructions": "Refactor the cohort-loading module and run its tests.",
  "settings": { "provider": "claude_code" }
}
```

This works through machinery that already existed rather than through anything added for these
providers. `TaskConfig` carries an `Arc<dyn Provider>`, and `subagent_tool::apply_settings_overrides`
resolves the named provider and applies the privacy rules to the pair before the child's session row
is written.

There is deliberately **no** separate `delegate_to_claude_agent` tool, and the reason is recorded in
the agent's own code: `Agent::subagent_tool_enabled` exists because when two delegation mechanisms
are armed at once the model reaches for the more general one, and the declared alternative becomes
dead configuration. A second tool covering the same ground would reproduce that, so delegation stays
one mechanism with a provider argument.

The privacy consequence follows automatically and is worth stating, because it is the case a reader
will want to check. Both providers are `Public`, so a **private** parent naming either one as its
child's provider is refused outright — not flagged for approval, refused. A subagent spawn arrives
as tool arguments the model wrote, so a private parent handing private-origin prompt text to a
public model it chose itself is an agent-initiated disclosure with no human in the loop to escalate
to, and `apply_settings_overrides` runs in-process inside the parent's turn where there is nothing to
escalate to. The refusal is the same one that governs every other public model, which is why it
needed no new rule here — and why there is no second list of provider names that a future provider
could be forgotten from.

## Related documentation

- [Installing and signing in](installing-and-signing-in.md) — the setup path, the four card
  states, and the `GET /coding_agents/status` route behind them.
- [The tool bridge](tool-bridge.md) — how BioRouter's own tools reach the child once the child's
  own tools are off.
- [What the child agent may not do](child-agent-isolation.md) — the isolation flags and the
  measurements behind them.
- [Compliance: vendor terms, BAA and PHI](compliance.md) — why the credential handling above is
  the compliance boundary, and why both providers are `Public`.
- [Performance, limits and known gaps](performance-and-limits.md) — latency, prompt overhead, and
  what the streaming path does and does not cover.
- [Streaming and tool-call parity](streaming-and-tool-call-parity.md) — the design record behind the
  streaming path and the mirror, and what was deliberately deferred.
- [Model provider integration references](../README.md) — the parent folder and the API-key
  provider references.
- [Subagents](../../agent-loop/subagents.md) — the delegation mechanism the section above
  uses, and the settings a spawn accepts.
- [Environment variables](../../configuration/environment-variables.md) — where the two command
  keys sit among all other configuration.
