# Coding-agent parity in Crew

> **What this is.** The design contract that lets Claude Code and Codex chats use Crew exactly as chats on any other provider do: connect to a channel, run a Crew task, and use the person tools. Crew decides who may read a workspace by matching the chat's privacy to the workspace's privacy, the same way for every provider, and never by the kind of provider.
> **Status:** Proposed. Ruled by the owner on 2026-11-12, overruling the recommendation to keep these chats away from Crew channels and tasks. Built on branch `claude/crew-coding-parity-37b543`, stacked on the account ownership work (#475) and mixed provider tiers (#484). This touches privacy and permission code, so it needs human review before it lands (CP-8 lists the enforcement points).
> **Audience:** Developers and reviewers of Crew and of the coding-agent providers. Users read the [Crew user manual](../README.md).

## Why

Claude Code (`claude_code`) and Codex (`codex`) run inference on the person's own vendor subscription by driving the vendor's command-line agent, which reaches Biorouter's tools over one MCP relay, the tool bridge ([coding-agent providers](../../providers/coding-agents/README.md)). Both are `ProviderTier::Public`.

Until this change Crew refused both providers by kind, whatever the workspace:

| Door | Where | What it said |
|---|---|---|
| Grant, model switch, every model request in a granted chat | `CrewManager::check_provider_binding` | "Crew cannot bind a provider with unscoped external tools" |
| Run admission (a chat's grant and a task) | `CrewManager::checked_run_admission` | "Crew cannot admit providers with external tools outside its scoped capability boundary" |
| Task start | `routes/crew.rs` `start_owned_run` | "This provider controls external tools that Crew cannot isolate." |
| Task model from a chat | `crew_tools/tasks.rs` `task_model` | "Crew can't run a task on Claude Code or Codex. Choose another model." |
| Task model defaults | CLI `usable_task_providers`, desktop `crewTaskModelUsable` | both providers left out of the list |
| Coding-agent bridge | `CODING_AGENT_BRIDGE_ALLOWED_CREW_TOOLS` and its empty-roster note | the grant's worker tools withheld; "A chat on Claude Code or Codex cannot be connected to a Crew channel" |

All four Rust refusals asked `Provider::uses_tool_bridge()`, which is a name match on the two providers (`coding_agent::bridge::provider_uses_bridge`). The reason given was that the vendor's own agent runs tools Crew cannot isolate. That reason no longer holds for the bridged chats Biorouter starts: the vendor's own tools are switched off by the isolation flags every bridged chat already depends on (CP-7), and every tool the vendor agent can reach is one of Biorouter's, dispatched through the same gates as any other provider's call.

The owner's ruling, in their words: "If the crew workspace is built in a way that it's supposed to be private, then only private models can use it. If the crew workspace is built in a way that's supposed to be public, then any public providers should be able to get access to it."

## Decisions

### CP-1: Crew admits a model by privacy, the same for every provider

A Crew door asks one question: may a model of this privacy read this Crew place? It never asks which provider the model is.

- **The model's privacy** is the chat's DR-33 tier and affiliation ([privacy tiers](../../security/privacy-tiers.md), DR-33): the chat's provider slot, whose `tier()` is the least private of its model and its decision models. A call's capability (`CallCapability`) carries the same pair, sampled from that slot. A Crew task's model is a fresh provider with no decision models, so its tier is its own.
- **The place's privacy** is what the workspace and the grant say: the connection's mode (Public or Private), whether the grant admitted a public model (`public_provider`), whether the chat was already private when it was granted (`origin_restricted`), which institutions own what it reads, and, at admission, whether any channel it reads is restricted.

The outcomes, for any provider:

| Workspace | Public model (Claude Code, Codex, a public model of any provider, or a chat made public by a decision model) | Private model (Versa and other institutional or local models) |
|---|---|---|
| Private | Refused: "This chat's Crew context is private, so a public model can't read it." | Admitted, when the model's affiliation matches the workspace's institution |
| Public | Admitted, with no access to the server (public models never get a work folder or full access) and only public-safe channels | Admitted. A chat that was already private when granted connects in private personal mode: the workspace must carry its institution's label, and everything the chat posts there is restricted, so no public model ever reads it |

The last cell is unchanged by this work and holds for every private model: a private chat's own content never reaches a public model through a public workspace, because the broker restricts what such a run posts (`personal_mode`, `restricted`). A private chat on an unlabeled public workspace is refused by the broker ("workspace institution must be labelled").

### CP-2: one function decides, and every door asks it

`crew::model_match::check(model, place)` is the only place a Crew door compares a model's privacy with a Crew place. The tier and affiliation checks that were written out in four places now live there, with their sentences unchanged:

| Door | Caller | Place |
|---|---|---|
| A tool call in a granted chat, direct or bridged | `CrewManager::check_dispatch` (Crew's dispatch check, `agents/crew_extension.rs`) | the chat's grant |
| Every model request and provider use in a granted chat | `check_provider_dispatch`, `check_provider_use` | the chat's grant |
| A grant, a restore and a model switch | `check_provider_binding` | the chat's grant |
| A run is asked for (a chat's `/crew` grant, a Crew task from the Crew view, the CLI or a chat's `tasks_ask`) | `checked_run_admission` | the connection, before anything is sent |
| The workspace answers its snapshot | `institution::admission` | the workspace's mode and restricted context |
| The run is created | `begin_run_with_policy` | the institutions that own what it reads |

`model_match::check` takes a `ModelPrivacy` (a tier and an affiliation), never a provider, so it cannot ask what kind of provider it is looking at. The person tools' read scope (`crew_tools`), which narrows what a public model may read rather than admitting a run, asks `ModelPrivacy::is_public` on the call's capability. A source scan (`no_crew_door_asks_the_provider_kind`) fails when any Crew file (the core, the server's Crew routes and person tools, and the Crew capability's dispatch) calls `uses_tool_bridge` or names `claude_code` or `codex`, or compares a tier with `ProviderTier::Public` outside `model_match.rs`.

### CP-3: no refusal names a provider kind

Every refusal a coding-agent chat can now meet is a tier or affiliation mismatch, worded for the person, and the same sentence a chat on any public provider reads. The sentences are the existing `crew_public_model_refused` and `crew_institution_mismatch` ones ([agents and chat access](../agents-and-chat-access.md), refusals table). The three kind refusals and the task model's "Crew can't run a task on Claude Code or Codex" are deleted, not reworded.

### CP-4: what a bridged coding agent is offered

The Crew row of `CODING_AGENT_BRIDGE_POLICIES` admits all four Crew tools: the person tools `crew__read` and `crew__change`, and the grant's worker tools `crew__connections` and `crew__request`. The row says what may cross; the chat's own roster decides what does, by the rule every provider's chat follows (`CrewToolStanding::offers`):

- a person's own chat with no grant gets the person tools (owner ruling D3, unchanged);
- a chat that holds a grant, and a Crew task's chat, get the two worker tools and nothing else of Crew;
- an app's chat, a subagent and a scheduled run get none.

The bridge takes Crew tools only from that roster, never from the registry (unchanged). The empty-roster note that told a bridged model "a chat on Claude Code or Codex cannot be connected to a Crew channel" is deleted: a bridged chat with no Crew tool now reads what any chat reads in the same state.

**A grant's tool limit holds on the bridge.** A chat that holds a Crew grant may call only the worker tools and the checklist (`authorize_session_tool`, gate B-1). The bridge used to add tools it recovered from the registry (Developer, Knowledge and the others) after the roster, so a granted Claude Code chat with Developer enabled would have been offered `developer__shell` and then refused at dispatch. The bridge plan now applies the same rule to every candidate (`a_granted_coding_agent_chat_is_bridged_only_what_its_grant_allows`), so a granted bridged chat is offered exactly what the same chat on another provider is offered. Dispatch still asks gate B-1 again for every bridged call, and the Crew tool asks its standing a third time.

### CP-5: a bridged call carries the chat's tier

The bridge plan's capability is the lesser of the chat's provider slot (its DR-33 tier, decision models included) and the provider rebuilt for one iteration of the reply loop: the slot whenever it is Public, the iteration's provider while the slot is Private. So a chat a decision model made public bridges as public, and a slot switched to a private model while a public child runs never widens what that child may read. This rule landed with the mixed provider tiers work (#484, `prepare_coding_agent_bridge_plan`, held by `the_turn_provider_never_decides_a_tier`); parity relies on it and adds `a_bridge_plan_carries_the_chats_tier_not_the_iteration_providers`. The two are equal for Claude Code and Codex, which are Public with no affiliation and cannot be lowered further, but Crew's dispatch check reads this capability, so it must be the chat's tier if a private provider ever drives the bridge.

### CP-6: long Crew work never outlasts a bridged call

A bridged call must answer within the bridge's budget (`child_tool_call_budget`, 600 seconds; a parked approval card waits at most 570). No Crew call blocks that long, by construction, and a test pins it (`every_crew_wait_fits_inside_a_bridged_call`):

- every request to the workspace is bounded by the transport's 45-second exchange deadline (`transport::REQUEST_DEADLINE`);
- a command on the server (`remote.shell`, `remote.execute`) waits at most 20 seconds and then answers its job, which the model checks with `remote.job_status`; long work runs with `detach` (the server's `MAX_WAIT`);
- a file copy (`remote.fetch_blob`, `remote.share_file`) settles for at most 20 seconds and then answers "Still copying" with its job;
- the person tools already cut every card and job wait to the call's remaining time (`within_call`), and `tasks_watch` waits at most 20 seconds.

So a long Crew run returns a handle and is polled, the way `workspace_watch` and the Developer shell's background jobs are, and the bridge's 570-second card limit is never the thing that ends one. A Crew task on Claude Code or Codex is a whole agent turn of its own with the same tool budget as any task (`TASK_MAX_TURNS`, `TASK_MAX_TOOL_CALLS`).

### CP-7: what keeps the vendor's own tools off

Crew's reason for refusing these providers was that their own agent runs tools Crew cannot isolate. In a chat Biorouter starts, the vendor's agent has no tools of its own. These flags are the boundary, they are already what every bridged chat relies on, and each is pinned by a test:

| Provider | What is switched off | Where | Test |
|---|---|---|---|
| Claude Code | every built-in tool: `--tools ""`, or only `ToolSearch` (the schema loader) when a bridge is offered | `claude_code.rs` `base_args` | `the_isolation_flags_are_always_present`, `a_bridged_child_keeps_only_the_schema_loader` |
| Claude Code | hooks and settings from the working directory: `--setting-sources ""` | same | `the_isolation_flags_are_always_present` |
| Claude Code | the person's own MCP servers: `--strict-mcp-config` with only Biorouter's relay | same | same |
| Claude Code | `--bare` is never passed (it would drop the system prompt override) | same | `bare_mode_is_never_requested` |
| Codex | shell, unified exec, browser, computer use, apps, plugins, multi-agent, image tools and the rest of `DISABLED_CHILD_FEATURES`, each passed as `--disable` | `codex.rs` `app_server_command` | `an_isolated_codex_home_carries_auth_but_never_user_mcp_config` |
| Codex | the person's own configuration and MCP servers: `--strict-config` and a `CODEX_HOME` holding only `auth.json` | same | same |
| Codex | writes, web search and every approval but MCP elicitation: a read-only sandbox, `web_search = "disabled"`, the granular approval policy | `codex.rs` `thread_params`, `approval_policy` | `thread_params_are_locked_down`, `the_approval_policy_lets_only_mcp_elicitations_through` |

Crew now relies on these too, so `crew_relies_on_the_coding_agent_isolation_flags` (in each provider) asserts the subset Crew's argument needs: no built-in tool beyond the schema loader, no tool from the working directory's settings, no MCP server but Biorouter's, and Codex's shell and exec features off. A vendor CLI that stops honouring a flag is a defect in the bridge for every chat, not a Crew-only one; Codex's unknown-feature retry already warns when a build drops a feature name.

What these flags do not do: they do not stop the vendor's own service from seeing what the chat sends it. That is what the tier is for. Both providers are Public (no BAA, no zero-retention agreement), so they reach only public workspaces and public-safe channels.

### CP-8: what changes, and the enforcement points for review

Enforcement points a reviewer should read, all in `crates/biorouter/src` unless named:

1. `crew/model_match.rs`: the one tier and affiliation decision (CP-1, CP-2).
2. `crew/mod.rs`: `check_tier`, `check_provider_binding`, `checked_run_admission` and `begin_run_with_policy` now ask it; the two `uses_tool_bridge` refusals are gone.
3. `crew/institution.rs`: `admission` asks it for the workspace's mode and restricted context.
4. `crates/biorouter-server/src/routes/crew.rs` `start_owned_run`: the task-start kind refusal is gone; admission decides.
5. `crates/biorouter-server/src/routes/crew_tools/tasks.rs` `task_model`: no provider is left out of a chat's task model.
6. `agents/agent.rs`: the Crew bridge row admits the worker tools; the bridge plan keeps a grant's tool limit (its capability rule, CP-5, came with #484); the Crew empty-roster note is gone.
7. `agents/agent.rs` `provider_for_crew_grant`: a chat whose slot is empty is judged with its stored decision models folded in (DR-33), so a chat made public by a decision model is never granted a private workspace as if it were private.

Surfaces: the desktop's Crew task model list and the CLI's task defaults offer Claude Code and Codex like any configured provider; the docs say what each workspace admits by privacy.

Unchanged, and the same for every provider: a Crew grant binds the chat to one model (`crew_model_fixed`); a Crew-scoped chat may use only a decision model that runs on this computer (`CrewUseGate`), so a chat made public by a Baseten decision model can join a public workspace but its decision model does not run there; a public model never gets server access.

## Tests

- `crew::model_match` tests: `one_rule_decides_every_crew_door` (the CP-1 table for a public and a private model against a private and a public workspace, a grant, a restricted origin and institution ownership) and `no_refusal_names_a_provider_kind`.
- `no_crew_door_asks_the_provider_kind`: the CP-2 source scan.
- `claude_code_and_codex_meet_crew_as_public_models` and `versa_and_a_chat_made_public_by_its_decision_model_meet_the_same_rule`, with the real providers: a public workspace admits `claude_code` and `codex` at the binding, a bridged call and run admission; a private workspace refuses them with `crew_public_model_refused`; Versa is admitted to both; and Versa with a Baseten decision model (DR-33) is refused a private workspace and admitted to a public one.
- Bridge (`agents/agent.rs`): `a_coding_agent_is_offered_crew_by_its_chats_own_roster` (an ungranted chat is never offered the worker tools, and a bridged chat's Crew section equals a direct chat's), `a_granted_coding_agent_chat_is_bridged_only_what_its_grant_allows` (exactly the worker tools and the checklist, never a registry tool, and a bridged call outside the grant refused at dispatch), `a_coding_agent_child_is_bridged_the_person_tools_only_in_a_persons_chat` (now with the granted case) and `a_bridge_plan_carries_the_chats_tier_not_the_iteration_providers` (CP-5).
- `every_bundled_capability_is_bridged_or_withheld_on_purpose`: Crew is bridged, with all four tools, and no capability carries an empty-roster note.
- `every_crew_wait_fits_inside_a_bridged_call` (CP-6) and `crew_relies_on_the_coding_agent_isolation_flags` (CP-7).
- Live, on a scratch server: a public workspace with Claude Code and Codex (connect a chat to a channel, run a task), a private workspace's refusal for both, and Versa in both.

## Related documentation

- [Agents and chat access](../agents-and-chat-access.md): what a chat and a task can do in Crew, for users
- [Privacy and security](../privacy-and-security.md): Crew's private and public boundaries
- [Account ownership](account-ownership.md): the person tools and owner ruling D3
- [Coding-agent providers](../../providers/coding-agents/README.md) and [the tool bridge](../../providers/coding-agents/tool-bridge.md)
- [Privacy tiers](../../security/privacy-tiers.md): DR-33 and the decision-model tier
