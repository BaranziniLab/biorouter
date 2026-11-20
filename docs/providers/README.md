# Model provider integration references

This folder holds selected model and integration references for individual LLM
providers: model defaults and capabilities, provider wiring and selection surfaces,
verification commands, and provider-specific limits. Provider source, bundled
catalogues and supported live discovery determine the advertised model lists and
defaults; each reference explains the relevant behavior and links to its evidence.

Come here when you are **changing or verifying a provider integration** — adding a
provider module, touching `crates/biorouter/src/providers/factory.rs`, or re-running the
checks after a catalog or context-limit change. If you instead want to *use* a provider —
pick one, find the credential it needs, or switch models — read
[Choosing a model provider](../getting-started/choosing-a-model-provider.md); if you want
the credential variable itself among all other settings, read
[Environment variables](../configuration/environment-variables.md). Note that this folder
is not an inventory of every shipping provider. It covers the references below, the
two local providers (the bundled Llama Server and Ollama), and the two coding-agent
providers in the subdirectories. Other providers are documented by their source
modules under `crates/biorouter/src/providers/`.

## Documents

| Document | What it covers |
| --- | --- |
| [Anthropic and Claude Code models](anthropic-models.md) | The ordered Claude 5.5 models, optional Fable tier, Claude Code version requirements, saved selections, model retirement and signed thinking history. Current, verified 2026-11-17. |
| [Prompt caching on Amazon Bedrock and Versa Bedrock](bedrock-prompt-caching.md) | Where the Converse `cachePoint` checkpoints go, which Claude models get them, how a gateway that refuses them is resent once without them and remembered, and the `BEDROCK_PROMPT_CACHING` switch. Current, written 2026-11-19 (#527). |
| [Baseten provider](baseten.md) | The integration reference for the `baseten` declarative provider: the bundled catalog, the keyless feed that keeps its models and prices current between releases, the daily job that publishes that feed with a key held in a GitHub Environment, where Baseten appears in the app, its known limits, and the verification commands. Current, written 2026-10-19, revised 2026-10-20. |
| [Content-filter refusals](content-filter-refusals.md) | What Biorouter says when Azure's or an OpenAI-compatible provider's content filter blocks a request or a reply (a policy refusal that names the flagged categories and is not retried), what the Responses route says, and why a filter that failed to run no longer ends the turn. Shared by Azure OpenAI, Versa API Azure, OpenAI and the custom providers on OpenAI's format. Current (November 2026). |
| [Google Gemini provider](google.md) | Gemini 4 model defaults and thinking behavior, native GenerateContent signatures, tool identities, and fixture-based verification. Current, verified 2026-11-17. |
| [OpenAI API models](openai.md) | Current flagship and optional model tiers, Responses tool support, output budgets, reasoning controls, and the separate Codex account catalogue. Current, verified 2026-11-17. |
| [Stream faults, empty replies and streamed reasoning](stream-faults-and-empty-replies.md) | How the OpenAI-compatible decoder classifies an error inside a stream, the one corrective attempt an empty reply gets, and how OpenRouter's streamed `reasoning_details` fragments are merged before replay. Current, written 2026-11-19 (#542, #548). |
| [Xiaomi MiMo provider](xiaomi-mimo.md) | The integration reference for the `xiaomi_mimo` provider — registry wiring, the `XIAOMI_MIMO_API_KEY` / `XIAOMI_MIMO_HOST` contract with its regional endpoints, every selection surface, and the verification commands. Current, written 2026-06-19 against v1.85.3. |
| [z.ai (GLM) provider](zai-glm.md) | The integration reference for the `zai` provider serving the GLM model family — registry wiring, the `ZAI_API_KEY` / `ZAI_HOST` contract on the OpenAI-compatible surface, every selection surface, and the verification commands. Current, written 2026-06-19 against v1.85.3. |

The Xiaomi MiMo and z.ai (GLM) references deliberately share a section structure so their
surfaces and checks can be compared line for line.

## Subdirectories

- **[`coding-agents/`](coding-agents/README.md)** — the two providers that run inference on
  the user's **own vendor subscription** by driving a coding-agent CLI the user installed and
  signed in to: `claude_code` (shown as **Claude Code**) and `codex` (**Codex**). They share
  a mechanism rather than just a vendor, which is why they are documented together: no API
  key, no base URL, a credential BioRouter never touches, and a child process that is itself
  a complete agent whose own tools must be switched off and replaced over MCP. The folder has
  its own index and holds seven documents covering the mechanism, user setup, the generic tool
  bridge, the child-isolation flags, the vendor-terms and PHI compliance position, the
  measured performance limits, and the streaming and tool-call parity design record. **Read
  [the compliance page](coding-agents/compliance.md) before using either provider for
  research data** — a consumer subscription carries no BAA and no zero-data-retention
  agreement, so both providers are `ProviderTier::Public` and the privacy bind gate refuses
  them to any session classified private.
- **[`llama-server/`](llama-server/README.md)**: the bundled **Llama Server** (`llamacpp`)
  local provider and its managed sidecar. It has its own index and holds two documents: the
  [model catalog](llama-server/model-catalog.md), which describes the two models it offers
  (Qwen3.8 27B, the default, and Gemma 4 31B), what a name from an earlier catalog does, how
  the sidecar finds a copy on disk, and its context, request and thinking policy; and the
  [model catalog QA checklist](llama-server/model-catalog-qa-checklist.md), which with the
  `llamacpp_integration` suite and the `llamacpp_survey` harness verifies each model's
  availability, tool calling at the size of a real first turn, streaming and speed, and records
  the security invariants the sidecar must not regress. Both were rewritten on 2026-10-10 for
  llama.cpp build b11429.
- **[Local inference validation record](../history/local-inference-2026-10/local-inference-validation.md)**: the historical source-scoped local and
  live validation record for the local inference fix. The privacy-corrected code passes six compilation
  boundaries, 9,328 workspace tests, 21 local gates and five fresh-executable checks. Interim H1
  passes all 16 CI checks with the original Windows failure retained. Gemma G/R6 and historical
  Qwen/Gemma model-quality, interruption and workflow failures remain explicit; final roster outcomes and
  the expected archive destination/completion-proof procedure are recorded separately. Final-head status is in
  [PR #393's checks](https://github.com/BaranziniLab/biorouter/pull/393/checks).
- **[Local inference handoff record](../history/local-inference-2026-10/local-inference-handoff.md)**: implementation, investigation and
  delivery references for Ollama context truncation, Llama Server hardening and tool-call
  robustness, including source/binary lineage and current quality/coverage limitations.
- **[`ollama/`](ollama/README.md)**: the **Ollama** (`ollama`) local provider, which drives an
  Ollama server the user runs. It has its own index and holds two documents:
  [context window and settings](ollama/context-window-and-settings.md), on the native
  `/api/chat` request, how the context window is chosen and why a cut prompt is refused rather
  than answered (the cause of the 2026-10 "model returns JSON instead of doing the task"
  report), and every `OLLAMA_*` setting; and
  [tool-call protocols](ollama/tool-call-protocols.md), on how Ollama reads each model's tool
  calls, what 12 models returned on the wire, which local models work as an agent and which to
  avoid, and the guards on recovering a call a model wrote as text.

## Related documentation

- [Choosing a model provider](../getting-started/choosing-a-model-provider.md) — the
  user-facing side of this folder: which providers exist, their credentials and default
  models. Itself superseded in part, so check the source module list for the live set.
- [Environment variables](../configuration/environment-variables.md) — where every
  provider's API key and host override sit among all other configuration variables.
- [Secret storage](../security/secret-storage.md) — how provider API keys are held in the
  OS credential store rather than in plaintext.
- [biorouter CLI command reference](../cli/command-reference.md) — the `configure` and
  `run --provider` commands that appear in each provider's surfaces table.
