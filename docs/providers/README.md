# Model provider integration references

This folder holds the maintainer-facing integration references for individual LLM
providers: how a provider module is wired into the provider registry and factory, every
surface in the app where a user can select it, the commands that verify the integration
still works, and the provider-specific gotchas that are easy to rediscover the hard way.
Each document records the wiring so a maintainer can re-derive it from code, and each is
explicit that the authoritative model lists and defaults live in the Rust source rather
than on the page.

Come here when you are **changing or verifying a provider integration** — adding a
provider module, touching `crates/biorouter/src/providers/factory.rs`, or re-running the
checks after a catalog or context-limit change. If you instead want to *use* a provider —
pick one, find the credential it needs, or switch models — read
[Choosing a model provider](../getting-started/choosing-a-model-provider.md); if you want
the credential variable itself among all other settings, read
[Environment variables](../configuration/environment-variables.md). Note that this folder
is not an inventory of every shipping provider: BioRouter has 40+ provider modules under
`crates/biorouter/src/providers/`, and only the few named below — the two reference pages in
this folder, plus the bundled Llama Server and the two coding-agent providers in the
subdirectories — are documented here. Every other provider is documented only by its own
source module.

## Documents

| Document | What it covers |
| --- | --- |
| [Xiaomi MiMo provider](xiaomi-mimo.md) | The integration reference for the `xiaomi_mimo` provider — registry wiring, the `XIAOMI_MIMO_API_KEY` / `XIAOMI_MIMO_HOST` contract with its regional endpoints, every selection surface, and the verification commands. Current, written 2026-06-19 against v1.85.3. |
| [z.ai (GLM) provider](zai-glm.md) | The integration reference for the `zai` provider serving the GLM model family — registry wiring, the `ZAI_API_KEY` / `ZAI_HOST` contract on the OpenAI-compatible surface, every selection surface, and the verification commands. Current, written 2026-06-19 against v1.85.3. |

The two provider references deliberately share a section structure so their surfaces and
checks can be compared line for line.

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
- **[`llama-server/`](llama-server/README.md)** — verification material
  for the bundled **Llama Server** (`llamacpp`) local provider and its managed sidecar.
  It has its own index and currently holds a single document, the
  [Llama Server model catalog QA checklist](llama-server/model-catalog-qa-checklist.md).
  That checklist — plus the automated
  `llamacpp_survey` harness that executes most of it — walks every model in the catalog
  for availability, correctness, tool-calling, streaming, speed, and robustness, and
  records the security invariants the sidecar must not regress. It is **superseded in
  part**: the test procedure still holds, but the catalog contents, the `--ctx-size`
  default, and the thinking-control flag have all changed since it was written, so
  re-derive the model list and launch flags from the `llamacpp.rs` and
  `llamacpp_sidecar.rs` sources before running it.

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
