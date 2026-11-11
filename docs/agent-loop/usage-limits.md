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
  before every reply, and it survives a reload. Which warnings were given is kept on the
  chat's row (`extension_data`, `usage_limit_notice/v1`), keyed by the limit's scope,
  axis and value (and the month, for the monthly limit); a limit that changes, is lifted,
  or is back under 80% after it was raised is forgotten there, so its next crossing warns
  again. Writing the chat's own limit clears the record.
- **When the next request would pass a limit**, the turn ends before it, so the last
  request no longer runs whole past the limit. The stored notice says what is used, the
  limit, what the next request is estimated at and how much is left. When one request
  alone is estimated past the whole limit (a limit smaller than one request), it says so:
  "One request is estimated at $0.0086, more than the whole limit of $0.005, so no request
  fits under it."
- **At 100%** the turn ends there. No further model request is made, and no wrap-up
  request either, because that would spend past the limit too. The chat stores a notice
  that names the limit, the limit's size and the amount used, and says where to raise or
  lift it: the cost in the composer for a chat limit, Settings > App > Usage for the
  default or the monthly limit.
- **When a turn ends having reached a limit** (a request billed more than its estimate),
  or leaving too little for the next request, the same stop notice is stored as the turn
  ends, so the reason is in the transcript after a reload rather than only when the next
  message is refused. A turn that crossed 80% stores the warning the same way.
- **A message sent to a chat at a limit** is stored and answered with the same notice. No
  model request is made, and automatic compaction does not run, since a compaction is a
  model request.
- **A message typed while a stopped turn ends** (a steer) does not resume it. It is
  stored and marked unanswered, as with the per-reply budget.
- **Raising or lifting the limit** lets the next message run normally. The monthly limit
  also clears itself on the 1st of the next month, local time.

Other spending doors follow the same rule:

- **Subagents.** A chat at its limit, or any chat once the month is at its limit, starts
  no subagent; the tool call is refused with a sentence telling the model not to retry. A
  running subagent is held to its parent chat's limit at each of its own model requests.
- **Scheduled runs.** A run that would start while the month is at its limit is refused
  before it creates anything. The refusal shows as the schedule's last error, and the
  firing is given back, so a schedule with a run count does not use up its runs. A run
  that reaches a limit part-way (the default per-chat limit applies to each run's own
  chat, and the month can fill while it runs) is a failed run, not a success: its stop
  shows as the schedule's last error, the last-run time does not move, and the firing is
  given back the same way.
- **Overflow compaction.** When a request overflows the model's window and the chat is
  already at a limit, the chat is not compacted and the turn ends.
- **Decision models.** A monitor, a `decide` batch or a phase gate is counted, through the
  usage ledger, but never blocked by a usage limit: it is a side call, and stopping it
  part-way would leave a monitor's verdict unknown.
- **Workflow generation.** The model call that turns a chat into a workflow is added to
  the chat's usage, so it counts too.
- **Chat titles.** A chat at a limit is not named by a model. After a message the limit
  stopped, a chat still called "New chat" gets the first words of its first message as
  its title, and a chat that already has a title keeps it. The check is the same one the
  turn makes before each model request (`SessionManager::maybe_update_name`).

## How the next request is estimated

`usage_limit::project_request` estimates the next request from the ledger
(`SessionManager::last_request_usage`, the chat's own last turn request, never a side call
or a decision):

- **Inside a turn**, the previous request's billed prompt is scaled by how much the
  context manager's size estimate of the request grew since it was sent
  (`context_mgmt::estimate_request_size`, held per chat on the agent), so a large tool
  result counts before it is sent.
- **At a reply's first request, in the composer and in the prologue**, the estimate is the
  previous prompt plus its answer, which joins the history. The three use the same rule, so
  they agree on whether the chat is paused.
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

A missing, zero or negative value is unlimited. `config.yaml` can still be edited by
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
the turn). A composer with no chat yet reads the monthly state from `GET /usage/summary`
and holds Send while the monthly limit is reached. Under the monthly limit's row, Settings
shows what the month used against it and whether it is near or reached.

## Where the code is

- `crates/biorouter/src/agents/usage_limit.rs`: the stored override, the resolver, the
  comparison, the next-request estimate (`project_request`, `estimate_next_request`,
  `check_next`), the warning record (`announce_warning_once`), the dollar formatter, the
  notice texts and the per-reply warn and stop state.
- `crates/biorouter/src/session/session_manager/spend.rs`: `SessionManager::spend_snapshot`,
  the one spend read every check uses, and `last_request_usage`, the request an estimate
  starts from.
- `crates/biorouter/src/agents/agent.rs`: `Agent::usage_limit_gate` (before every model
  request, with the estimate, and before an overflow compaction, without it),
  `Agent::usage_limit_paused_notice` (the prologue) and `Agent::usage_limit_turn_end_notice`
  (the end of a turn the limit did not stop).
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
