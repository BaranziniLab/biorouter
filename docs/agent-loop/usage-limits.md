# Usage limits

> **What this is.** How the per-chat limit, the default per-chat limit and the monthly limit stop a chat from spending past what you set, in billed tokens or priced dollars.
> **Status:** Current.
> **Audience:** end users, and developers working on the agent loop or the usage routes

A long agent task can make hundreds of model requests, and every one is billed for the
whole conversation it carries. A usage limit stops a chat before the request that would
spend past it. All limits are off (unlimited) by default. They are separate from the
per-reply budget (BR-35, `BIOROUTER_REPLY_BUDGET_*`), which bounds one reply and starts
again at zero on the next one; a usage limit counts everything a chat or a month has
used.

## The three limits

| Limit | Where it is set | What it counts |
| --- | --- | --- |
| Chat limit | The cost at the lower right of the composer | This chat and every subagent it started |
| Default per-chat limit | Settings > App > Usage | The same, for every chat that has no limit of its own |
| Monthly limit | Settings > App > Usage | Every chat, deleted chats included, in the current LOCAL calendar month |

A deleted chat still counts, in tokens and in dollars, but its dollars are priced again at
the model's base rate. Deleting a chat folds its usage into one total per day and model,
which keeps no request's size, so a request that was billed at a model's long-context rate
(past a prompt of 200,000 or 272,000 tokens, depending on the model) counts at the base
rate from then on. Deleting such a chat can therefore lower the month's dollar figure, and
can lift a monthly dollar limit that had been reached. Token figures do not change.

Each limit can be in dollars, in tokens, or both. The editors start in dollars whenever
the chat's model (or, in Settings, the current model) has a price, and in tokens when it
has none, saying that a dollar limit cannot stop an unpriced model. Settings > App > Show
costs hides the running cost, not the limit: a dollar limit can be set and is shown with
that switch off. Dollars are priced the same way the Usage panel prices them, cache
included. Tokens are the billed total of every request: fresh input, output, cache reads
and cache writes.

A dollar limit keeps every decimal it was given, up to four: `0.013` is stored as
`0.013`, never rounded to a cent, and an amount with a fifth decimal is refused with a
message rather than rounded.

Every surface prints dollars one way (`formatUsd` in `ui/desktop/src/utils/formatUsd.ts`,
`format_usd` in `usage_limit.rs`, which both run the cases in
`ui/desktop/src/utils/formatUsdCases.json`): two decimals from $1 up, and below $1 three
significant digits but never fewer than three decimals, so $0.0085 reads `$0.0085` in the
cost, in its per-model row and in a notice. Where a used amount and its limit are read
side by side, both take the fewest decimals that tell them apart, so `$0.177 of $0.18`
never reads `$0.18 of $0.18`.

A chat's limit has three states:

- **Default.** The chat uses the default per-chat limit, whatever it is when the check
  runs. Changing the default changes every chat that has no limit of its own.
- **Unlimited.** The chat has no per-chat limit, even when a default is set. Use this to
  lift the limit for one chat.
- **Its own limit.** The chat's own tokens, dollars or both. It replaces the default on
  both axes: an axis the chat leaves empty is unlimited for that chat.

The monthly limit applies on top of any chat limit. Lifting a chat's limit never lifts
the month.

## What happens at a limit

Before every model request in a reply, Biorouter compares what was used, plus an
estimate of the request about to be sent, with the limits in force.

- **At 80%** of any limit, the chat stores one notice that starts "This chat is near its
  usage limit" or "Near the monthly usage limit". It is given once per crossing, not
  before every reply, and it survives a reload. Each limit warns on its own: a month that
  was already warned about does not hide a chat crossing 80% of its own limit later.
  Which warnings were given is kept on the chat's row (`extension_data`,
  `usage_limit_notice/v1`), keyed by the limit's scope, axis and value (and the month,
  for the monthly limit); a limit that changes, is lifted, or is back under 80% after it
  was raised is forgotten there, so its next crossing warns again. Writing the chat's own
  limit clears the record. A subagent's warning about its parent chat's limit is stored
  in the parent chat, where the record is kept and where you read.
- **When the next request would pass a limit**, the turn ends before it, so the last
  request no longer runs whole past the limit. The stored notice says what is used, the
  limit, what the next request is estimated at and how much is left (a monthly notice
  leaves out what is used and what is left, see below). When one request
  alone is estimated past the whole limit (a limit smaller than one request), it says so:
  "One request is estimated at $0.0086, more than the whole limit of $0.005, so no request
  fits under it."
- **At 100%** the turn ends there. No further model request is made, and no wrap-up
  request either, because that would spend past the limit too. The chat stores a notice
  that names the limit, the limit's size and, for a chat limit, the amount used, and says
  where to raise or lift it: the cost in the composer for a chat limit, Settings > App >
  Usage for the default or the monthly limit.
- **A monthly notice names the monthly limit, never the month's total.** The total sums
  every chat, private and deleted ones included, and the notice is stored in whichever
  chat reached the limit, which may be a public one. So it says the monthly limit is near
  (at least 80%) or used up, or that the next request is estimated past what is left, but
  not how much was used, a percent of it or how much is left. Settings > App > Usage shows
  the exact figures, and the routes give them only to a request that carries the
  user-action proof.
- **When a turn ends having reached a limit** (a request billed more than its estimate),
  or leaving too little for the next request, the same stop notice is stored as the turn
  ends, so the reason is in the transcript after a reload rather than only when the next
  message is refused. A turn that crossed 80% stores the warning the same way.
- **A message sent to a chat at a limit** is stored and answered with the same notice. No
  model request is made, and automatic compaction does not run, since a compaction is a
  model request. `/compact` typed into such a chat gets the same notice and compacts
  nothing.
- **Model calls at the end of a turn** are held to the same check: the self-critique pass,
  the `/goal` judge and every prompt Stop hook are each a model request, so none runs once
  the chat or the month is at a limit or the next request is estimated past one. The turn
  finishes, and its end stores the stop notice. Command Stop hooks still run, since they
  make no model request. A `/goal` whose judge did not run is not cleared; the chat says
  it stays active.
- **A message typed while a stopped turn ends** (a steer) does not resume it. It is
  stored and marked unanswered, as with the per-reply budget.
- **Raising or lifting the limit** lets the next message run normally. The monthly limit
  also clears itself on the 1st of the next month, local time.

A limit is checked against billed spend, which a request adds only once its response
lands. Requests that run at the same time do not see each other: parallel subagents of one
chat, or several chats against the monthly limit, can each pass the check and together
spend past a limit by up to about one request each. Nothing reserves a request's estimate
while it runs.

Other spending doors follow the same rule:

- **Subagents.** A chat at its limit, or any chat once the month is at its limit, starts
  no subagent; the tool call is refused with a sentence telling the model not to retry. A
  running subagent is held to its parent chat's limit at each of its own model requests.
- **Scheduled runs.** A run that would start while the month is at its limit is refused
  before it creates anything. The refusal shows as the schedule's last error, and the
  firing is given back, so a schedule with a run count does not use up its runs. A run
  that a limit stops part-way (the default per-chat limit applies to each run's own
  chat, and the month can fill while it runs) is a failed run, not a success: its stop
  shows as the schedule's last error, the last-run time does not move, and the firing is
  given back the same way. A run that finishes its work and only then reaches a limit, or
  leaves too little for one more request, is a success: its transcript ends on the same
  stop notice, so the scheduler asks the agent whether the limit cut the reply short
  (`Agent::usage_limit_interrupted_reply`) rather than reading the transcript.
- **Crew tasks.** A task runs in a chat of its own, so its spend counts toward that
  chat and the month, never the chat that asked for it. While the month is at its limit
  no task starts, from Crew or from a chat. A chat at its own limit also cannot start one
  with `tasks_ask`; it gets no approval card. Both refusals carry `crew_usage_limit_reached`.
  A task that a limit stops part way ends as failed with the usage-limit error
  (`CREW_TASK_STOPPED_AT_LIMIT`) and posts no result, the way a limit-stopped scheduled run
  is recorded; Crew asks the same question the scheduler does
  (`Agent::usage_limit_interrupted_reply`).
- **Overflow compaction.** When a request overflows the model's window and the chat is
  already at a limit, the chat is not compacted and the turn ends.
- **Side checks.** The stall check, the permission judge, MCP sampling and knowledge
  macros are counted, as side calls charged to the chat they serve, if any (a subagent's
  to its parent), and to the month, but a limit does not stop them. Each serves a request the
  gate already admitted, or work outside a turn, and the next request of the turn still
  meets the gate.
- **Decision models.** A monitor, a `decide` batch or a phase gate is counted, through the
  usage ledger, but never blocked by a usage limit: it is a side call, and stopping it
  part-way would leave a monitor's verdict unknown. A phase gate that the chat's own model
  judges, because the chat has no decision model it may use or its decision model could not
  be reached, is counted the same way, as a side call on the chat (`SideCall::GateJudge`),
  and is not stopped either. So is a decision whose model is under a chat provider's name
  (`side:decision:<row id>`, `SideCall::Decision`), once per decision row.
- **Workflow generation.** The model call that turns a chat into a workflow is added to
  the chat's usage, so it counts too.
- **Chat titles.** A chat at a limit is not named by a model. After a message the limit
  stopped, a chat still called "New chat" gets the first words of its first message as
  its title, and a chat that already has a title keeps it. The check is the same one the
  turn makes before each model request (`SessionManager::maybe_update_name`). Below a
  limit, the naming call is billed to the chat (`side:session_name`) and sees at most the
  first 1,000 characters of each of the first three user messages, 3,000 in all
  (`providers::base::session_name_context`): sending the whole message billed a
  93K-token first message's title at 92,745 input tokens.

In short, a limit holds these model calls: every request of a turn, a message sent to a
paused chat, automatic, overflow and background compaction, `/compact`, the self-critique
pass, the `/goal` judge and prompt Stop hooks, starting a subagent and each of its
requests, a chat's title, the start of a scheduled run (monthly limit), and the start of a
Crew task (the month, and the asking chat for `tasks_ask`). It does not hold decision models,
a phase gate judged by the chat model, workflow generation, the stall check, the permission
judge, MCP sampling or knowledge macros, which are counted but not stopped, nor
prompt hooks on events other than Stop (for example `UserPromptSubmit` or `PreToolUse`).

## How the next request is estimated

`usage_limit::project_request` estimates the next request from the ledger
(`SessionManager::last_request_usage`, the chat's own last turn request, never a side call,
a decision or a compaction):

- **Inside a turn**, the previous request's billed prompt is scaled by how much the
  context manager's size estimate of the request grew since it was sent
  (`context_mgmt::estimate_request_size`, held per chat on the agent), so a large tool
  result counts before it is sent.
- **At a reply's first request**, the estimate is the previous prompt plus its answer,
  which joins the history. In the reply loop it also adds the context manager's size of
  what came after that answer (the new message, an attachment, a pasted document), so a
  large paste is counted before it is sent. Only what was added is estimated this way,
  not the whole request, because that estimate counts high by design. The composer and
  the prologue cannot see an unsent message, so they use the previous prompt plus its
  answer alone. The loop is therefore at least as strict: a chat they show open can still
  stop at its first request when the new message is large, never the other way round.
- **After a compaction**, the request before it carried the history the compaction
  replaced, and the compaction's own request (the old history in, the summary out) is not
  the next request's shape either. Until a request after the compaction is billed, the
  next one is estimated like a chat's first: the reply loop uses the context manager's
  size of it, and the composer and the prologue compare the spend so far alone. A
  compaction's ledger rows carry event keys that start with `compaction:`
  (`session_manager::compaction_event_key`).
- **Before a chat's first request**, the prompt is the context manager's size estimate.
- The previous request's cache read and cache write shares and its output carry over. A
  chat's first request takes them from the model's last request in any chat
  (`last_request_usage_for_model`), or assumes no cache.
- Dollars are priced at the bound model's rates (its long-context rate past the threshold),
  or the previous request's model when the bound one has no price. An estimate with no
  price is never compared with a dollar limit.

## When a dollar limit cannot fire

A dollar limit compares priced spend. If nothing a chat used is priced (a local model, a
model missing from the price list), its dollar figure is unknown, not zero, and a dollar
limit over it never fires. The usage-limit route says so with `chatDollarLimitInert` and
`monthDollarLimitInert`. Set a token limit if you need a limit that always applies. When
only part of the spend is priced, the priced part is compared, which is a lower bound.

## Who may change a limit

Only the person at the keyboard. Both writes take the user-action proof on a daemon that
holds a key, so a model cannot raise or lift its own limit over HTTP; the refusal tells it
to stop and ask. On a `biorouter serve` daemon, which can never check a proof, the reach
gate is the whole gate, as for every other per-chat setting.

The default and monthly limits live in the `usage:` section of `config.yaml`:

```yaml
usage:
  monthly_token_limit: 66000000
  monthly_dollar_limit: 250.0
  default_session_token_limit: 5000000
  default_session_dollar_limit: 20.0
```

A missing, zero or negative value is unlimited. Each key is read on its own, so one value
that is not a number leaves only its own limit unset and the others in force; the log
says which key, once. A token limit written as a decimal or in exponent form (`6.6e7`) is
rounded to a whole number, and a number written as a string (`'5_000_000'`, `'$250'`) is
read. A comma is not read either way, since `0,50` could mean fifty cents or fifty
dollars.

**Upgrading.** Before usage limits, `monthly_token_limit` and `monthly_dollar_limit` only
drew the Usage panel's gauge. They are now enforced: a `usage:` section you already had
pauses every chat once the month reaches it. To keep using the app past it, raise or lift
the limit in Settings > App > Usage, or remove the keys from `config.yaml`.

`config.yaml` can still be edited by
anything with a shell on this machine, the agent's own shell included (DR-17), so these
limits stop accidental spend; they are not a barrier against a determined local
process. Over HTTP the section takes the user-action proof on every route that writes
it: `PUT /usage/limits`, and `POST /config/upsert` or `POST /config/remove` with the key
`usage`, which answer `403` without it.

A chat's own limit is stored on its row, in `extension_data` under `usage_limit/v1`.
Changing it is a deliberate change to the chat, so it counts as activity and moves the
chat in the sidebar (coordinator ruling 16).

## Routes

| Route | Gate | What it does |
| --- | --- | --- |
| `GET /sessions/{session_id}/usage-limit` | `session_reach` | The chat's effective limit and where it came from, its own setting, the defaults, the monthly limit, what was used, and `paused`, `reason` and `warn`. `paused` also holds when the next request is estimated to pass a limit: `pausedByEstimate` is then `true`, and `nextRequest` (`tokens`, `usd`) is the estimate. The month's totals (`used.monthTokens`, `used.monthUsd`) sum every chat, private ones included, so they are `null` unless the request carries the user-action proof |
| `PUT /sessions/{session_id}/usage-limit` | `session_reach`, then the user-action proof | `{ "mode": "inherit" \| "unlimited" \| "limit", "maxTokens"?, "maxUsd"? }`; answers in the `GET` shape. A subagent's chat is refused: set the limit on its parent |
| `PUT /usage/limits` | The user-action proof | `{ monthlyTokenLimit?, monthlyDollarLimit?, defaultSessionTokenLimit?, defaultSessionDollarLimit? }`; a field left out is kept and `null` is unlimited |
| `GET /usage/summary` | The user-action proof | Month to date against the monthly limits, plus the default per-chat limits, and the monthly limit's state as the reply loop reads it: `monthlyReached`, `monthlyNear`, `monthlyUsedTokens`, `monthlyUsedUsd`. A new chat's composer reads `monthlyReached` to hold Send before the chat exists, and Settings shows the monthly limit's progress from these |

The renderer recognises the stored notices by their prefixes, which are mirrored in
`ui/desktop/src/components/conversation/usageLimitNotice.ts`; a Rust test reads that file.

In the desktop app a chat's own limit is set from the cost under its composer (the cost
card's Limit section), and the monthly and default limits in Settings > App > Usage; see
[Shared controls](../desktop-ui/shared-controls.md#the-cost-cards-limit-section). The
composer reads the chat's limit when it opens, after every turn, when a stored limit
notice lands and whenever the chat's recorded usage moves (a title's side call lands after
the turn, and the post-turn naming poll carries the chat's row counters into the live ones).
The card's "Cost of this chat" is every request the chat made, its side calls (the title,
stall checks, judges, hooks, sampling, knowledge macros) included, and it is read again each
time the card opens, with the Limit section, so the two never count different requests; the
limit line adds the chat's subagents. The cost on the footer line is also read again a few times
in the five minutes after each turn and when the window comes back, so a side call billed after
the turn, such as a background compaction, shows without opening the card. A composer with no chat yet reads the monthly state
from `GET /usage/summary` and holds Send while the monthly limit is reached. Under the
monthly limit's row, Settings shows what the month used against it and whether it is near
or reached.

## Where the code is

- `crates/biorouter/src/agents/usage_limit.rs`: the stored override, the resolver, the
  comparison, the next-request estimate (`project_request`, `estimate_next_request`,
  `check_next`), the warning record (`announce_warning_once`), the dollar formatter, the
  notice texts and the per-reply warn and stop state.
- `crates/biorouter/src/session/session_manager/spend.rs`: `SessionManager::spend_snapshot`,
  the one spend read every check uses, and `last_request_usage`, the request an estimate
  starts from. It runs before every model request, so it reads through indexes: the month
  through `idx_token_events_ts` (a `ts` lower bound a day before the local 1st, then the
  local date), and a chat with its subagents through `idx_token_events_session` and
  `idx_sessions_parent`. The last is created on open, with no migration arm.
- `crates/biorouter/src/agents/agent.rs`: `Agent::usage_limit_gate` (before every model
  request, with the estimate, and before an overflow compaction, without it),
  `Agent::usage_limit_paused_notice` (the prologue), `Agent::usage_limit_turn_end_notice`
  (the end of a turn the limit did not stop) and `Agent::usage_limit_hold` (the check the
  prologue, the end-of-turn model calls and `/compact` ask).
- `crates/biorouter/src/config/usage.rs`: the `usage:` section and its merge write.
- `crates/biorouter-server/src/routes/usage_limit.rs` and `routes/usage.rs`: the routes.

Tests: `cargo test -p biorouter --lib -- usage_limit spend config::usage`,
`cargo test -p biorouter --test usage_limit_agent_loop`,
`cargo test -p biorouter-server --lib -- routes::usage routes::usage_limit`, and in
`ui/desktop`, `npx vitest run src/utils/formatUsd.test.ts src/components/bottom_menu
src/components/settings/usage src/components/usage src/components/ChatInput.usageLimit.test.tsx`.

## Related documentation

- [Long-running tasks](long-running-tasks.md): turn and tool-call limits, the per-reply
  budget, and what keeps a long task affordable.
- [Decision models](decision-models.md): how decision calls are billed and counted.
- [Steering a running turn](steering-a-running-turn.md): what happens to a message typed
  while a turn is ending.
- [Subagents](subagents.md): what a subagent spends and how it is started.
- [Shared controls](../desktop-ui/shared-controls.md): the cost card's Limit section and the
  composer notice.
