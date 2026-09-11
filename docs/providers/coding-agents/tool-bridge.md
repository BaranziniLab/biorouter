# The tool bridge

> **What this is.** How BioRouter gives subscription-authenticated coding agents reviewed built-in
> tools, admitted session extensions, and bounded workflow tools. It explains the gate stack,
> capability URL, catalog refresh, mirrored tool cards and approval flow.
> **Status:** Current.
> **Audience:** developers working on the coding-agent providers, the extension layer, or the
> daemon's routes.

`claude` and `codex` are complete agents: they run their own loop and execute their own file and
shell tools. BioRouter switches those off, because a tool the child runs itself is invisible to
BioRouter's inspectors, permission modes, `.biorouterignore` and vault — see
[what the child agent may not do](child-agent-isolation.md). The bridge restores only the reviewed
surface with every gate intact. Built-in tools come from the explicit
`CODING_AGENT_BRIDGE_POLICIES` allowlist in
[`agent.rs`](../../../crates/biorouter/src/agents/agent.rs). Attached ordinary MCP extensions
are included only after the existing tool, privacy-tier and extension-reach filters admit them.
A public provider still cannot discover or invoke a private extension through this bridge.

## MCP is the mechanism, not one option among several

There is exactly one channel that returns a tool *result* into a live turn of either CLI: an MCP
server the child itself calls.

Neither CLI's permission protocol has an outcome meaning "the host already ran this, here is the
result". Claude Code's `can_use_tool` resolves only to allow — optionally with rewritten input — or
deny. Codex's approvals are approve or deny. So the obvious design, intercepting the child's own
tool call and answering it with BioRouter's result, is not expressible in either protocol. The
child has to *call BioRouter*, and MCP is the call.

```text
MCP tools/list  <-  the session's tool set, exactly as the model would see it
MCP tools/call   ->  the inspector stack, then ExtensionManager::dispatch_tool_call
```

## The tools do not have to be an extension's (#109)

`tools/call` lands on a `BridgeToolDispatch`, not on a hardcoded
`ExtensionManager`. A chat turn's dispatcher wraps the session's extension manager and the one
audited platform ingest macro; a bounded workflow has its own small surface with its own
dispatcher, and those tools are in no extension at all. The knowledge ingest macro is the case that forced it: its
`KbToolDispatch` carries the git transaction every write in the run must land on,
so a call routed to the `knowledge` extension instead would commit somewhere else.

What that unlocked is the whole of #109. Before it, the only caller that knew how
to give a coding-agent provider tools was `Agent::reply`; every other agentic loop
— the knowledge macros, scheduled workflows, bounded sub-agents — called
`Provider::complete` directly with a `tools` argument `claude_code` and `codex`
**discard**. The failure was silent and expensive: the model produced a complete,
correct plan with every call written out as prose, invented its own
`<tool_response>OK</tool_response>` replies to continue against, and wrote
nothing, after a full run the user paid for. The Knowledge UI's answer was a
hardcoded provider denylist, now deleted.

[`ProviderToolTurnContext`](../../../crates/biorouter/src/providers/tool_turn.rs)
is the primitive: issue a one-turn grant over the workflow's own tools, scope the
URL around the call, drop the lease when it returns. It prefers `stream()` when
the provider offers it — not for latency, but because the mirrored pairs
recording what the child ran exist only on that path, so a workflow driven
through `complete_with_model` would execute every tool correctly and be unable to
say afterwards which ones. `ProviderTurn::pending_tool_calls()` excludes the
mirrored requests, so a caller's loop cannot dispatch a call the child already
ran.

⚠ **An empty `ToolInspectionManager` is not "allow everything".** A grant refuses
a call whose verdict is "no decision was reached", so a context built without
inspectors refuses every bridged call. `for_workflow` therefore installs a real
stack — managed policy, security, sensitive ops, permission. The security and
sensitive-ops inspectors escalate regardless of mode, which is what keeps its
`BioRouterMode::Auto` a statement about ordinary authorised steps rather than a
blanket grant.

⚠ **A workflow with no HTTP server refuses instead of degrading.** A *chat* turn
with no bridge runs the child tool-less, and that is right: an answer from the
conversation beats a failed turn. A workflow that *is* a sequence of tool calls
has no such fallback — it would narrate and write nothing — so
`ProviderToolTurnContext` fails before the model runs, naming the provider and
the way out.

## It is generic, and that is the point

Nothing in the bridge knows anything about any individual tool, and no tool needs per-tool work to
become available to a child. That falls out of both sides already speaking MCP: BioRouter's tools
*are* `rmcp::model::Tool`, and `ExtensionManager::dispatch_tool_call` already takes MCP's own
`CallToolRequestParams`. The bridge is a relay between two things that already fit.

The consequence worth stating plainly: the relay is generic, but the subscription boundary is an
allowlist. Built-in policy, enabled capability state and current tool availability must all agree.
An ordinary extension grant also pins its exact configuration and tool subset; dispatch rechecks
that the grant still matches the attached extension. Loading a different server under the same
name cannot inherit the old grant. The bridge enforces its advertised list again at `tools/call`,
so an unadvertised name cannot be invoked directly to bypass this boundary.

Delegated children have a separate, narrower runtime profile. A parent's ordinary extension
grant does not automatically transfer to a child. The source's child-grant policy, rather than
the parent's requested extension names, determines what the child actually receives.

## Catalog changes within a user request

An extension may attach successfully while the coding agent still holds the tool catalog from
the beginning of its provider request. Returning the new tool names in an install report does
not update that client's callable schema. In Codex 0.147.0, the
[tool-list notification handler](https://github.com/openai/codex/blob/rust-v0.147.0/codex-rs/rmcp-client/src/logging_client_handler.rs)
logs the notification without refreshing the tool list. A live Playwright install reproduced
this gap: the report listed `playwrightagent__browser_tabs`, but the model could not call it.

After a successful bridged Extension Manager or Skills catalog mutation, the old chat dispatcher
stops admitting further calls. The agent loop waits for all observed parallel tool calls to
settle, stops the old provider stream, and revokes its lease. It preserves completed tool records,
rebuilds the tools and system prompt from the current session, and resumes the same user task
under a fresh grant. The continuation explicitly says not to repeat completed operations.
Failed mutations and native child tools do not trigger this refresh.

The immutable-grant checks, inspectors, privacy capability and approval path remain in force.
This compatibility path costs another provider request and context replay per catalog-changing
step. Native mid-request catalog refresh is a future optimization, not a reason to bypass the
bridge or widen an existing grant. User-driven changes made outside the manager during an active
provider request still require separate live validation.

That external-toggle gap is also visible in the source: the provider wake loop listens for
output, cancellation, elicitation and steering, but not catalog changes. The GUI can update an
agent's Extension Manager while the request still holds its original prompt and tools. Dispatch
revalidates revoked tools, but that refusal is not a refreshed prompt. Closing this gap needs a
coordinated, tool-safe refresh boundary; restarting from a bare catalog event could interrupt
an admitted operation before its result is recorded. This remains a required runtime fix and
live test, not a completed prompt-audit item.

Regression coverage includes `mirrored_catalog`, `bridged_catalog_mutation`, the direct-provider
catalog refresh test, and the Codex MCP-error flag test. Before declaring end-to-end success,
run the self-test's dynamic-extension scenario in the isolated development app: start detached,
attach, invoke a real extension tool, detach, and verify the final tool inventory. An install
report or a passing mocked-provider test alone is not sufficient.

The 2026-08-30 isolated desktop regression passed this sequence with Codex: one install,
one `browser_tabs` call, and one detach, with the visible extension count changing 0 → 1 → 0.
The earlier failure in the same test chat had installed and detached without invoking the tool.
A separate read-only task identified Soul as the chat's primary base and listed its six pages;
this checks that existing test chat, not every fresh-profile or ingestion scenario.

A later isolated Spoke run completed marketplace approval, user-entered credential configuration,
one live `get_spoke_schema`, one read-only `query_spoke` returning three Gene records, and detach.
Its follow-up explicitly forbade reattachment: the model correctly reported the tools unavailable,
made no additional Spoke call, and the popup showed Spoke off. The installed package was retained.
That run also exposed four pointless name guesses before installation; an absent installed entry
must route through discovery and approved installation, not spelling retries.
The manager's schema, tool descriptions and not-found response now require the exact installed
name from its inventory, distinguish that name from a marketplace registry id, and point missing
entries toward approved installation. The recovery test failed before this wording change;
private-extension admission still precedes the not-found branch.

The installed `anti-ai-writing` skill was hot-loaded, read and applied to a synthetic scheduling
decision memo, then hot-unloaded in the same user task. The memo correctly compared $4 versus
$4.50 per daily minute saved and the $60 incremental cost for 10 extra daily minutes. The skill
count fell from four to three after unload. Permanent package removal remains a separate check.

A subsequent removal returned `status: removed`; the desktop displayed “allowed once” without
the test operator clicking approval. The approval actor still needs confirmation, so that run
does not prove the intended cancellation scenario. Its read-only follow-up did prove removal:
installed-skill search returned zero matches, one exact `loadSkill` call failed with “Skill not
found”, the picker showed “No skills found”, and neither the package directory nor its removal
staging directory remained. Spoke stayed installed. The chat retained its inert
`workspace_skills.v1.remove` entry for the uninstalled skill and its historical transcript; this
is not a zero-residue purge of prior preferences or records.

Codex's standard MCP result object is decoded as a complete `CallToolResult`, not serialized
into a text block containing another result. This preserves content types, structured data, error
state and display metadata. ⚠ Since QA-E F4 it is the *fallback*: the child is handed only the
model's view of a result, so where the bridge kept its own full record of the call, that record —
audience annotations included — is what gets stored; see
[what the child is handed](#what-the-child-is-handed-and-what-the-transcript-keeps-qa-e-f4). To Do update results carry the updated task's
id, text and status so the activity row can name the work rather than only its numeric id.
The subsequent live read-only checklist showed “Starting ‘Confirm the primary knowledge base’”
and “Marking ‘Examine its page index read-only’ complete”, alongside “Listing pages in Soul”.
Older saved results without the task payload retain the action-and-id fallback.
Marketplace labels retain semantic versions, including prerelease and build suffixes:
“Installing Spoke Agent v0.4.1” instead of “Installing Spokeagent 0 4 1”. Recognized compact
agent names are expanded without splitting ordinary names such as “Reagent”. Both cases have
fail-first regressions, and the Spoke version label was checked in the running desktop UI.
The desktop extension count classifies capability names directly, without waiting for the global
catalog; a session response arriving first must not temporarily inflate it with built-in tools.
The separate total-tool count also refetches on catalog changes, cancels superseded queries,
and clears when switching to a chat whose agent is not ready. Its regressions cover attach,
detach, out-of-order responses, chat switching and listener cleanup; three failed before the fix.

The checkpoint's full workspace test run completed with 6,909 passed, zero failed, 44 ignored,
and the paid Anthropic provider test explicitly excluded. The full desktop run passed all 3,788
tests across 369 files. After the follow-up exact-name guidance and word-boundary changes, all
38 Extension Manager tests and 45 activity-card tests passed, as did desktop type checking.
These counts are automated coverage, not a substitute for the remaining live lifecycle and
mid-request UI-toggle checks above.

For scripted Electron testing, keep `npm run start-gui` attached to a terminal. The non-interactive
launcher exited before the application main process started during this validation; a terminal
launch worked. Use the repository's pinned runtime and an isolated `BIOROUTER_PATH_ROOT` with
`BIOROUTER_DISABLE_KEYRING=true`. Do not launch the bare Electron bundle before the app is ready,
which opens Electron's welcome window rather than BioRouter.

Verified against a 60-tool surface — both CLIs accepted a 73-character prefixed tool name, a schema
using `$defs`/`$ref`/`oneOf`, an image result, and a `ui://` embedded resource, all passed through
unchanged. ⚠ Two things that probe did not exercise have since been measured, and both changed what
the bridge returns (QA-E F4, 2026-09-11): neither CLI filters a result by audience, and codex-cli
0.153.4 cannot parse a block carrying `priority`. A block addressed only to the user — a `ui://`
figure, the shell's reformatted copy — is therefore no longer handed to the child at all, and the
annotations are stripped from what is; the transcript keeps the full result. See
[what the child is handed](#what-the-child-is-handed-and-what-the-transcript-keeps-qa-e-f4).

## What still fires on a bridged call

A grant is a snapshot taken when the provider request starts, not a handle back to the agent: the provider is
called from inside the agent's own stack and cannot hold a reference to it, and a grant that
outlived its turn would be a capability with no owner. The snapshot carries the session, the
permission mode, the extension manager, the inspection manager, the conversation, the already
filtered tool set, the turn's privacy capability, a request-scoped child of the turn's cancellation token, the
session's hooks manager, and the app's secret vault when there is one. The last three are there
because a bridged call is a tool call: without the token nothing the child started is reachable by
Stop, without the hooks manager a `PreToolUse` rewrite cannot be collected, and without the vault a
`{{vault:NAME}}` reaches the tool as a literal string.

| Gate | Where it applies to a bridged call |
| --- | --- |
| Tool discovery / privacy Gate E | Inherited. The advertised set is the one `filter_tools` produced for the model — already tier-filtered and reach-filtered — and `tools/list` serves it verbatim, so there is no second policy here to drift from the first. |
| Tool inspectors (command policy, sensitive ops, everything in the inspector stack) | Run on every call, against the conversation snapshot and the session's permission mode. |
| Permission mode | The inspectors' permission decision is honoured: denied is refused, "no decision was reached" is refused too (an absent decision must never read as approval), and `needs_approval` is [put to a person](#a-call-needing-approval-is-put-to-a-person-and-the-call-waits-107) rather than refused. |
| Privacy Gate C | `dispatch_tool_call` is the one choke point every tool call passes through, and a bridged call goes through it with the turn's `CallCapability`. |
| `PreToolUse` hook rewrites | Applied and then **re-judged**. The hooks have already run inside the inspector pass, so their `updatedInput` is collected and applied, and every inspector except the hook one re-runs on the rewritten arguments — otherwise a hook would be a hole straight through the security and permission gates, which only ever saw what the child's model asked for. The rewrite is taken scoped to this call's own request id, because the staging buffer is per session and bridged calls run concurrently. |
| Host file containment | The built-in policy and admitted extension configuration determine the surface. ⚠ The bridge no longer adds a Developer-specific working-directory jail: the editor used to be confined to the session working directory here and nowhere else, which stopped nothing once `developer__shell` was bridged beside it (a child holding a shell reads the same path by typing `cat`) and only made the editor stricter under these two providers than under every other one. Containment is therefore what it is on the ordinary path — `.biorouterignore`, the secret guard, the inspectors and the permission mode — not a second jail on one tool. Granting an ordinary extension is not an OS sandbox for that extension's process. There is no process-global Auto-mode relaxation for another route or session to inherit. |
| `.biorouterignore`, vault, session working directory | Whatever BioRouter's dispatcher and inspectors already enforce, because BioRouter is the process executing the tool. A `{{vault:NAME}}` in the arguments is resolved on the leaf dispatch path, after the call has been judged and immediately before it runs — the same position the agent's own path uses, so the inspectors and the user's hooks never see the decrypted secret. |
| Cancellation | The grant passes its request-scoped child token to `dispatch_tool_call`. Both Stop and lease revocation therefore reach parking Workspace/Knowledge calls; delegated background children have their own visible session and cancellation route so they can survive one provider invocation while the parent continues supervising them. |

⚠ **The inspector pass is why this is not a thin proxy onto `ExtensionManager`.**
`POST /agent/call_tool` *is* that thin proxy, and its own comment records the cost: it bypasses the
agent loop and therefore every `ToolInspector`. A child agent's tool calls are model-initiated and
must be inspected exactly like the parent model's.

The privacy capability is **sampled once**, when the grant is issued, and threaded from there. A
gate on this path asks the sampled capability rather than re-reading the master switch — a second
read is precisely the race `CallCapability` exists to close.

⚠ **One residual, recorded rather than implied away.** A hook's `additionalContext` and
`systemMessage` have **nowhere to go**
on this path — the model that made the call lives in another process, and the tool result is data
rather than a channel for out-of-band prose — so the bridge drops its own staged entries
deliberately, rather than leaving them for the session's next ordinary turn to inject into an
unrelated transcript.

## A call needing approval is put to a person, and the call waits (#107)

If the permission inspector routes a call to `needs_approval`, the bridge raises a real approval
request and parks the child's HTTP call until somebody answers.

**What this replaced.** Until #107 the bridge returned a refusal telling the child's model to ask
the user in words:

```text
`<tool>` needs a person's approval, and this turn has no way to ask for one.
Tell the user what you wanted to run and why, and let them approve it.
```

The model did exactly that, and the sentence was false in a way nothing on screen revealed: **no
request id had been minted**, so no dialog opened, there was nothing for a client to post to, and
the word "approve" typed into the chat resolved nothing. A retry hit the identical refusal. The
turn could only end in confusion — which is what the issue reported.

### The card is the agent's own card

[`crate::pending_user_action`](../../../crates/biorouter/src/pending_user_action.rs) is the one
registry both approval mechanics now route through. It publishes into the **same session-scoped
queue the agent loop already drains** (`ActionRequiredManager`), carrying the same
`ActionRequired::ToolConfirmation` payload — including BR-63's risk grade and preview — that
`handle_approval_tool_requests` yields on the agent's own path.

Three things follow, and each is the reason for the choice:

- **The desktop needed no change.** It draws the dialog it already had, and
  `POST /action-required/tool-confirmation` resolves it through a fallthrough in
  `Agent::handle_confirmation`: if the agent's own `pending_confirmations` map has no entry for the
  id, the decision is relayed to the process-global registry. One route, one dialog, two mechanics.
- **One wake source.** A second queue would make every agent loop race two notifies, which is the
  exact shape that made #40 a cross-session prompt leak.
- **The reply loop had to learn to drain during the provider call.** On a coding-agent turn the
  child is blocked on the bridge's HTTP response, which is itself parked on the card — so the
  provider stream yields *nothing* until a person answers, and a drain placed after the next stream
  item could never run. `Agent::reply` now races `stream.next()` against `request_arrived` with the
  same `next_batch_wake` the tool-batch loop uses. It is not gated to coding agents: any provider
  call is an await an extension's elicitation can land inside, and surfacing it during the call is
  strictly earlier than surfacing it afterwards.

### Every way out is bounded

| Release | Mechanism |
| --- | --- |
| The user decides | Allow (`AllowOnce` / `AlwaysAllow`) runs the call; Deny returns a refusal result. A **dismissal** (`Cancel`) is relayed as `Cancelled`, not as a denial — dismissing a card is not judging the call, and recording it as one would teach the permission store something the user never said. |
| Turn cancel | The grant carries **the turn's own** cancel token, so Stop, `AppState::cancel_turn` and the websocket `TurnGuard` all reach a parked call. |
| Lease drop | The grant's nonce is the park's *owner*; `BridgeLease::drop` calls `cancel_owner`, so a turn that ended by panic or early return cannot leave a child blocked on a response nobody will answer. |
| Session deleted | `DELETE /sessions/{id}` calls `cancel_session`. A card belonging to a deleted chat can never be answered — the surface it would be drawn on is gone. |
| TTL | `approval_ttl()`, deliberately **shorter than the child's own per-call deadline**. |

⚠ **The TTL is bounded by the transport, not by `BIOROUTER_CONFIRMATION_TIMEOUT_SECS`.** The
agent's own prompt may wait an hour because nothing is holding a socket open. This one cannot,
because a child CLI is: both CLIs apply a hard per-call wall clock (issue #110 measured Claude
Code's at ~60 s) and abandon the request when it elapses. Waiting past it does not give the user
more time — it converts a card they could still answer into "The operation timed out", a transport
failure the model may retry, producing a *second* card for the same call. So the park fits inside
`bridge::child_tool_call_budget()` and always answers with a result.

### Headless is explicit, not a slow refusal

A run with nobody to ask — `biorouter run -p`, a piped stdin, a scheduled job —
does not park for the full TTL and then time out. The CLI's existing
`headless_auto_decision` already answers a tool-confirmation card immediately
with `DenyOnce` when stdin is not a terminal, and it reaches a *bridged* prompt
through the same `Agent::handle_confirmation` fallthrough the desktop uses. So
the child gets a refusal within milliseconds, saying it was not approved.

That matters because of what the alternative looked like. The old text told the
model to ask the user in prose, and in a headless run there was no user, no
dialog, and no request id — the model asked, nothing answered, and the turn ended
having quietly done less than it reported.

### The refusal texts never invite an unanswerable question

Whatever happens, the child gets an MCP tool **result** with `isError`, not a JSON-RPC error: a
JSON-RPC error is a transport failure the child may retry or treat as a broken server, whereas
`isError` is a result the model reads and acts on. And on every non-approval path the text says
plainly that a chat message cannot approve it, because by then the request id is gone. That
property is asserted (`no_outcome_claims_a_chat_message_can_approve`), not merely intended — the
old wording is precisely what turned a missing dialog into a loop of polite, futile requests.

### Concurrent asks cannot resolve each other

Every park mints its own uuid and its own `oneshot`. A decision for an id nobody is waiting on is
dropped and reported as `Unknown`, never re-aimed at whichever call happens to be parked now. Both
child CLIs issue parallel `tools/call`, so this is the ordinary case rather than a corner one — it
is BR-62's property for the agent's own path, extended to this one.

### Nested approvals end with their provider request

Manager tools can park their own mandatory approval after the bridge's inspection pass. Those
inner waits do not carry the bridge nonce, so cancelling only nonce-owned cards left them alive
after the provider request ended. An issued bridge now owns a child of the user turn's
cancellation token. Revoking its lease cancels that child, including nested waits and dispatched
tools, without cancelling the parent turn or a successor request. A retained grant also refuses
new calls after revocation, before inspection and again before dispatch.

The nested-approval and request-isolation tests failed before this change. Coverage also checks
that a retained grant cannot dispatch after lease drop, and the existing real-process
cancellation test continues to exercise the dispatch wiring.

### Approval labels require acknowledgement

The desktop disables decision buttons while posting an answer and only records the requested
decision after the server returns `delivered`. Network failures, missing acknowledgements and
unavailable user-action proof leave the card retryable without storing an approval. Expired
requests and decisions already answered on another surface show neutral terminal states, not
the approval the current click attempted. Five acknowledgement regressions failed before the
change; all 22 confirmation-card tests then passed, including duplicate-click, retry and
navigation coverage. This fixes misleading presentation, not the unresolved attribution of
the live package-removal approval described above.

### Secret-safe requests

The same registry carries `UserActionRequest::Secrets`: a request for credentials that a *trusted
surface* collects and writes straight to the keyring. The parked caller learns only which keys were
configured, because `UserActionOutcome::SecretsConfigured` has no field a value could sit in, the
published card (`ActionRequiredData::SecretRequest`) carries key names and labels only, and
`resolve` **refuses** a data-bearing outcome for a secrets request rather than letting a mis-wired
route smuggle one into the transcript. A secret that never enters the conversation transport cannot
be persisted into a session row, replayed into a later prompt, or flattened into a child agent's
transcript.

### What is not persisted

An approval or credential card is a decision prompt, not a record. The drain skips writing it to
history: an answered card means nothing, and a persisted one reopens the session showing a
live-looking dialog for a call that finished long ago, routed to a request id that no longer
exists. Elicitations keep being persisted, because their *answer* is part of the conversation and
the response row references them.

## The mirror: how a bridged call becomes a visible card

For a long time a bridged call was completely invisible. It ran on a different task from the turn
that issued the grant, nothing on that path yielded an agent event or persisted a message, and the
GUI showed a spinner and then an answer — with no record that a tool had run at all.

The mirror closes that. As the child's stream reports each call, the provider mints an ordinary
**`ToolRequest` / `ToolResponse` message pair** carrying the call id, the tool name with the
`mcp__biorouter__` prefix stripped, the arguments and the result. Those are the same message types
every API provider produces, so the existing tool cards render them with no frontend change: a
skeleton the moment the tool's name is known, then a loading card, then a green or red card that
expands to the exact arguments and output. The pair persists like any other tool traffic, so
reopening the session shows the same cards, and the transcript flattener carries it into the next
turn's prompt as `[called tool: …]` / `[tool result: …]` — which is what stops the child re-running
lookups it has already done.

A failed call is recorded as a **successful transport carrying `isError: true`**, not as a
transport-level error. That is the shape the card reads to colour itself, and it keeps the failure
text readable in the card body instead of collapsing it to an error string
(`crates/biorouter/src/providers/coding_agent/mirror.rs:417-450`).

### Why the pair carries a marker

Each mirrored `ToolRequest` and `ToolResponse` carries a reserved key,
`biorouterProviderExecuted`, in the per-tool provider metadata the types already had
(`mirror.rs:63`). Its value is `bridged` for a call that ran on BioRouter's side of the bridge, and
`child` for one that ran inside the child's own sandbox.

The marker is not decoration. **An unmarked `ToolRequest` in a turn's response is dispatched by the
agent loop**, and `categorize_tool_requests` filters on message content only — it never reads
metadata. So a mirrored pair without the marker is either a `Tool '…' not found` error row (with the
bridge prefix intact) or a genuine **second execution** of a call the bridge already ran. A shell
command run twice is not a display glitch. The loop's one new branch asks
`mirror::contains_provider_executed` and, for a message carrying any mirrored content, persists and
yields it while dispatching nothing (`crates/biorouter/src/agents/agent.rs:7182-7202`).

Two design choices are worth knowing before touching this:

- **The predicate is "any", not "all".** A message that somehow mixed marked and unmarked content
  dispatches nothing at all. The worst case is then a card whose tool did not run — visible, and a
  decoder bug — rather than a command that ran twice, which is invisible and unrecoverable.
- **The marker is deliberately *not* a `MessageProvenance` variant.** That type is BR-71's
  security-purposed cross-session stamp, whose presence already means something specific to merge
  boundaries and the subagent surfaces, and whose unknown kinds degrade to `None`. Overloading a
  security signal with a display one would also lose the stamp silently on an older reader. The
  metadata home needed no schema change: `metadata` was already a free-form object on both types in
  the generated OpenAPI schema, so no client regeneration was required.

### `bridged` and `child` are different guarantees

| Marker | What ran, and under what | Where it comes from |
| --- | --- | --- |
| `bridged` | A BioRouter tool, executed by BioRouter's dispatcher behind every gate in the table above. | Claude Code's `tool_use`/`tool_result` frames; Codex `mcpToolCall` items whose server is `biorouter`. |
| `child` | An unexpected child-local execution that passed **none** of BioRouter's gates. Codex's local model tools are disabled; retaining this marker makes an upstream isolation regression or an unexpected MCP server visible instead of misattributing it to the bridge. | Codex `commandExecution` / `fileChange` items, and unexpected `mcpToolCall` items from any other server (`crates/biorouter/src/providers/codex.rs`). |

Showing a `child` call is a deliberate honesty choice: it happened whether or not BioRouter drew it,
and hiding it would be worse. It is not an endorsement, and the distinction is real rather than
cosmetic. ⚠ **The GUI does not yet draw a label separating the two** — the marker rides in the
persisted metadata, but a card reading `exec` looks like any other card today. Until that label
lands, read `exec` and `apply_patch` cards on a Codex turn as child-executed.

### What the child is handed, and what the transcript keeps (QA-E F4)

A BioRouter tool can return the same output twice, addressed to two readers: `developer__shell`
returns one text block with `audience: ["assistant"]` and a second, reformatted one with
`audience: ["user"]` and `priority: 0.0`; `text_editor view` returns the file to the assistant as an
embedded resource and a numbered rendering to the user; an Auto Visualiser figure is a `ui://`
resource for the user beside a one-line label for the assistant. Every provider formatter sends the
model only the blocks addressed to it (`providers::formats::audience::is_for_model`), and the GUI
shows only the ones addressed to the user.

The bridge used to hand the child **every** block, and three things followed, all measured on
2026-09-11 against `claude` 2.1.266 and `codex-cli` 0.153.4:

| What | Consequence |
| --- | --- |
| Neither CLI filters by audience | The child's model read every shell result twice, in the live turn. |
| Claude Code drops every annotation when it echoes a result (`tool_result.content` and `tool_use_result` alike), and rewrites an embedded resource as `[Resource from biorouter at <uri>] <text>` | The mirror stored two unlabelled blocks, so the card read "2 results" and the next turn's transcript repeated the output again. |
| codex-cli cannot parse a result whose block carries `priority` — `priority: 0.0` alone fails the call with `Unexpected response type`; `audience` alone does not | Once F1 was fixed, every `developer__shell` and `text_editor view` call on Codex still failed. |

So the bridge now answers the child with `bridge::child_view` of the result: the model-facing blocks
only, with their annotations removed — after the filter they tell a model nothing, and removing them
is what makes the result parse on Codex. The bridge is, in effect, these two providers' formatter.

The full result is not thrown away. `BridgeGrant::call_for_child` keeps it, keyed by the child's own
id for the call, which both CLIs put in the `tools/call` request's `_meta` and repeat on the frame that
reports the result:

| CLI | `_meta` key | Same id as |
| --- | --- | --- |
| Claude Code 2.1.266 | `claudecode/toolUseId` | the stream's `tool_use` / `tool_result` id |
| codex-cli 0.153.4 | `callId` | the `mcpToolCall` item id |

When the provider mirrors the call, it takes that record (`bridge::take_recorded_result`) and stores
**it** rather than the child's lossy echo — so the transcript holds exactly what the tool returned,
annotations included, the card counts one user-facing result, and the next turn's prompt carries one
model-facing copy. The transcript flattener applies the same `is_for_model` filter the formatters do,
reading text resources as well as text, so an annotated result is flattened once.

⚠ **The record is stored only when the echo shows the child received it**
(`mirror::recorded_if_received`): the error flag must agree, and every text BioRouter sent must
appear in what the child echoed. A child whose call timed out (#110), or whose CLI truncated a large
result, worked from something else, and the transcript records what the child actually saw. A CLI
that stops sending its call id degrades the same way — to the echo of the model-facing view — rather
than failing.

### The mirror and the approval card are different things

The mirror draws what *happened*: a `ToolRequest`/`ToolResponse` pair, green or red, after the fact.
The approval card is raised *before* anything happens and is answerable. A call that is refused —
by policy, by the user, or because the request expired — still shows as a red mirrored card naming
the tool, so a turn that quietly did less than the user thought is still visible in the transcript.

## The child's per-call deadline is configured, not discovered (#110)

Both CLIs apply a **hard per-call wall clock** to an MCP `tools/call` and abandon
the request when it elapses. Their defaults are far below what Biorouter's slower
tools need: issue #110 measured Claude Code's at "almost exactly 60 seconds"
against a `workspace_watch` whose schema advertises waits of up to 600. Every one
of those died at 60 with **"The operation timed out"** — a transport failure the
model may retry, rather than the non-error partial report the handler was about
to return. The handler was correct the whole time; it simply never got to answer.

So the deadline is set explicitly, per server, on both sides:

| CLI | Field | Unit | Where |
| --- | --- | --- | --- |
| Claude Code | `timeout` | milliseconds | the `--mcp-config` server entry. Its own help calls it a "hard wall-clock limit per call; **progress notifications do not extend it**". |
| Codex | `tool_timeout_sec` | seconds | the `thread/start` config override, beside the URL. `startup_timeout_sec` covers the initial connect. |

`bridge::CHILD_TOOL_CALL_TIMEOUT` is what both are given: 31 minutes, just above
the providers' own 30-minute turn ceiling so the enclosing turn always ends
first. `bridge::child_tool_call_budget()` is the shorter ten-minute budget for
parking tools that can return a partial result, such as approval and watch.

**A parking tool that waits must clamp to the budget.** `BridgeGrant::call` publishes it
in a task-local for the duration of the call, readable as
`bridge::bridged_call_budget()`; absent means this is not a bridged call and
nothing is holding a socket open. `workspace_watch` reads it, shortens its wait,
and **says both numbers** — the effective wait and the one that was asked for —
because a caller told only "still running" after 50 s will read that as the answer
to its 600-second question, and either abandon a subagent that is working fine or
fall back to polling transcripts, which is the behaviour the tool exists to
replace. Coding-agent subagent delegation returns a visible background child
session immediately. The outer BioRouter agent has an exit gate: while any such
child is running it injects a supervision continuation instead of accepting a
final answer. Claude Code or Codex must call `workspace_watch` and
`workspace_read_conversation` until the child finishes. This crosses provider
invocation timeouts without detaching the work from the parent lifecycle, and a
user can steer or stop the child from its tab or the CLI throughout.

**And a parked tool must be cancellable.** With the deadline raised, a
`workspace_watch` can legitimately hold the child's request for ten minutes — so
a Stop that did not reach it would keep a cancelled turn alive for the whole
wait. It now honours the turn's token and reaps the watcher tasks holding its
event-ring subscriptions; see
[cancelling a watch](../../agent-loop/workspace-control-tools.md#cancelling-a-watch).

⚠ **Raising the deadline is not the whole fix, and neither half is optional.**
The clamp is what keeps the guarantee when the configuration is not honoured — an
older CLI, a future one that renames the field, a user's own `MCP_TOOL_TIMEOUT`.
Without it, a regression in either vendor turns straight back into "The operation
timed out". Without the raised deadline, every long tool is clamped to a minute.
The E2E tests in `tool_bridge_routes.rs` drive a deliberately 90-second tool
through both real CLIs, so a dropped field fails there rather than in a user's
session.

## Transport: loopback HTTP, and the URL is the credential

The bridge is an HTTP endpoint on the daemon — `POST /tool_bridge/{nonce}` — rather than a spawned
stdio MCP server. That avoids inventing a second process and a socket, and it means the child talks
to the **live** `ExtensionManager` rather than to a copy. Both CLIs accept a remote MCP server on
loopback HTTP; that was verified before the design was chosen.

The capability cannot live in a header. Claude Code will send an `Authorization` header from its
config file, but **Codex sends none at all** — observed as `auth=None` on every request it made. A
header scheme would authenticate one client and not the other. So the capability lives in the URL
path, as an unguessable single-turn nonce that both CLIs transmit by construction:

| Property | Value, and why |
| --- | --- |
| Form | 32 hex characters from a v4 UUID — long enough that guessing is not a strategy. |
| Lifetime | One turn. The lease revokes the grant on drop, so a panicking or early-returning turn cannot leave a capability behind. |
| Scope | One session's already-filtered tool set, behind the full gate stack. Far narrower than the daemon's REST API. |
| Not the daemon secret | The child's environment is scrubbed of `BIOROUTER_SERVER__SECRET_KEY`, and the bridge route deliberately has no secret-key gate. |
| Handling | The route must not log the path, and an unknown nonce answers identically to any other unusable one, so it cannot be used as an oracle for which nonces exist. |

How the URL reaches each CLI:

```json
// Claude Code: written to a 0600 temp file, passed as --mcp-config
{ "mcpServers": { "biorouter": { "type": "http", "url": "<bridge url>" } } }
```

```json
// Codex: thread/start's `config` override map, i.e. mcp_servers.<name>.url
{ "mcp_servers": { "biorouter": { "url": "<bridge url>" } } }
```

Claude Code's configuration is a **file**, not an inline JSON string, even though the CLI accepts
both: the URL carries the turn's capability and `argv` is readable by any process running as the
same user. `NamedTempFile` creates it 0600, and the handle is deliberately bound for the whole run —
dropping it deletes the file, and a child that started a moment later would find no configuration.

Claude Code's invocation also gains `--permission-mode bypassPermissions` when a bridge is
present. That looks alarming and is correct here: with the built-ins off, the only tools that exist
are BioRouter's, and each one is inspected and permission-checked on BioRouter's side of the bridge
before it runs. Leaving the child to prompt instead would stall the turn, since a `-p` session has
nobody to ask.

## The protocol surface

| JSON-RPC method | Behaviour |
| --- | --- |
| `initialize` | Answers protocol version `2024-11-05`, a `tools` capability, and `serverInfo` naming `biorouter` and its version. |
| `tools/list` | The grant's tool set, verbatim: name, description, input schema. |
| `tools/call` | Validated (`name` required, `arguments` must be an object or absent), then run through the gate stack above. |
| `server/discover`, `ping` | An empty result. Claude Code probes `server/discover` before `initialize`; an empty result is a clean "nothing extra to discover" rather than an error it would log. |
| Anything else | JSON-RPC `-32601`. |
| A notification (no `id`) | `202 Accepted` with no body. Answering a notification with a JSON-RPC envelope is a protocol error some clients reject. |
| An unknown or expired nonce | JSON-RPC `-32001`, "this tool bridge is no longer active; its turn has finished" — the same answer for a well-formed miss and for a malformed nonce. |

## Lifecycle, and running with no bridge at all

The daemon publishes its base URL once it has bound a port; grants live in a process-global map
keyed by nonce, and only `claude_code` and `codex` are ever issued one. `live_grants()` exists for
an operator looking for a leak, not for tests — the map is process-global, so a count would measure
other work in the same process.

If there is **no base URL** — a CLI process with no HTTP server — the providers run the child
**tool-less** rather than failing. That is the correct degradation: there would be nothing for the
child to connect to, and a turn that can answer from the conversation should still answer.

⚠ **The bridge URL rides a task-local, and it must be read at construction time.** The `Provider`
trait has no session in scope — `complete_with_model` receives a system prompt, messages and tools
and nothing else — so `Agent::reply` scopes the URL around the call it makes into the provider.

The scope wraps the awaited call that *builds* the response, not the consumption of what that call
returns. A `stream()` implementation therefore reads the URL and spawns the child inside the scope,
**before returning the stream**; a poll of the returned stream may not read it, because by then the
scope is gone. Both providers do exactly that, and each says so in its own `stream()` header
(`crates/biorouter/src/providers/coding_agent/bridge.rs:625`,
`crates/biorouter/src/providers/claude_code.rs:770-777`). The lease itself is not the constraint —
`Agent::reply` binds it before the scope and it lives to the end of that loop iteration, which
outlasts stream consumption.

## Where the code is

| Concern | File |
| --- | --- |
| Grants, leases, the nonce, the task-locals, the transport budget; the child's view of a result (`child_view`) and the full result kept for the transcript (`call_for_child`, `take_recorded_result`) | [`crates/biorouter/src/providers/coding_agent/bridge.rs`](../../../crates/biorouter/src/providers/coding_agent/bridge.rs) |
| Running one provider turn with Biorouter tools, from anywhere | [`crates/biorouter/src/providers/tool_turn.rs`](../../../crates/biorouter/src/providers/tool_turn.rs) |
| Parking a call on a person: approval, elicitation, secret-safe credentials | [`crates/biorouter/src/pending_user_action.rs`](../../../crates/biorouter/src/pending_user_action.rs) |
| The queue the card is published on, and the loop's wake seam | [`crates/biorouter/src/action_required_manager.rs`](../../../crates/biorouter/src/action_required_manager.rs) |
| The mirror marker, the request/response pair builders, and when the kept result is stored instead of the child's echo (`recorded_if_received`) | [`crates/biorouter/src/providers/coding_agent/mirror.rs`](../../../crates/biorouter/src/providers/coding_agent/mirror.rs) |
| The loop branch that persists a mirrored pair without dispatching it | [`crates/biorouter/src/agents/agent.rs`](../../../crates/biorouter/src/agents/agent.rs) |
| The HTTP/JSON-RPC endpoint | [`crates/biorouter-server/src/routes/tool_bridge.rs`](../../../crates/biorouter-server/src/routes/tool_bridge.rs) |
| Handing the URL to Claude Code | [`crates/biorouter/src/providers/claude_code.rs`](../../../crates/biorouter/src/providers/claude_code.rs) |
| Handing the URL to Codex | [`crates/biorouter/src/providers/codex.rs`](../../../crates/biorouter/src/providers/codex.rs) |

MCP remains the implemented return channel for both coding-agent providers. Any vendor-specific
replacement must preserve the same inspected dispatch, grant revocation and mirrored-result
contracts; its availability should be verified against the installed CLI version.

## Related documentation

- [What the child agent may not do](child-agent-isolation.md) — the other half of the argument: the
  child's own tools are off, which is why the bridge exists.
- [How the coding-agent providers work](how-it-works.md) — the mechanism the bridge sits inside.
- [Permission modes](../../security/permission-modes.md) — the modes whose decisions the bridge
  honours.
- [Privacy tiers](../../security/privacy-tiers.md) — Gate C, Gate E and the `CallCapability` the
  grant samples once.
- [Extensions](../../extensions/README.md) — the tools that become available to a child for free.
- [Performance, limits and known gaps](performance-and-limits.md) — what a large tool surface costs
  in prompt tokens, and what the streaming path does and does not cover.
- [Streaming and tool-call parity](streaming-and-tool-call-parity.md) — the design record behind the
  mirror, including the two alternatives that were rejected.
