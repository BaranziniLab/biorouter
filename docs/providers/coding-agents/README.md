# Coding-agent providers

This folder documents the two providers that run inference on the user's **own vendor
subscription** by driving a coding-agent CLI the user already installed and signed in to:
`claude_code`, shown as **Claude Code**, and `codex`, shown as **Codex**. They are unlike
every other provider in the tree — there is no base URL and no API key, BioRouter never sees
or handles a credential, and the child process is a complete agent of its own whose tools
have to be switched off and replaced. That combination raises questions no other provider
reference has to answer: what the child may do, how BioRouter's own tools reach it, and
whether a consumer subscription may be used this way at all in a clinical-research setting.

Come here when you are **using, verifying or changing either provider**. The pages are
ordered from "make it work" to "why it is allowed", and the compliance page is not optional
reading for anyone deploying this at UCSF: a consumer Pro/Max or ChatGPT Plus plan carries
no Business Associate Agreement and no zero-data-retention agreement, so protected health
information must never reach these providers, and the page explains both the rule and the
gate that enforces it.

**They get every built-in capability, on the same terms as any other provider.**
Developer (shell, text editor, analyze, screen capture, image processor),
Computer Controller, Knowledge, Workspace, Skills, Extension Manager, Memory,
To Do, Chat Recall, Agent Drafter and Auto Visualiser all reach the child over
the bridge. What still does *not* is a **third-party** extension's tools, which
remain admitted case by case rather than wholesale.

⚠ **This changed, and the previous text described the old behaviour.** The
Developer roster used to be an empty list, and the reason given was that the
vendor CLI keeps its subscription credential on the host, so a host-reading tool
would let model-authored (possibly injected) input read it. That risk is real,
but withholding Developer did not answer it: the child already holds the vendor
credential it was spawned with, and every other public-tier model with Developer
enabled reads the same host. What the empty list actually produced was a coding
agent that could not `ls` a directory while the model beside it could — reported
repeatedly as a broken Developer capability rather than as a policy. The tools
are now bridged, and the bound is the same one that applies everywhere else:
`.biorouterignore`, the secret guard, the permission mode and its inspectors, and
privacy Gate C. See
[the tool bridge](tool-bridge.md#the-tools-do-not-have-to-be-an-extensions-109).

⚠ **Consequence worth stating plainly:** under `BIOROUTER_MODE: auto` the
permission inspector does not prompt, so a bridged `developer__shell` runs
without a confirmation step. That combination — a coding agent, full Developer,
and Auto mode — is the one to think about before pointing these providers at a
machine holding credentials you care about.

⚠ **On Codex, none of that reached the child from `codex-cli` 0.148.0 until
2026-09-11** (QA-E F1, measured on 0.153.4 with `claude` 2.1.266 alongside).
Every bridged call failed on its first attempt with the vendor's own sentence,
`MCP tool call requires approval, but approval policy is never`: the CLI began
answering its MCP tool-call approval itself under the `never` policy BioRouter
passed, so BioRouter was never asked. The thread now runs under Codex's granular
policy, which still refuses every child-local category inside the CLI and lets
only that one approval through, and BioRouter accepts it for its own bridge
alone — see
[why the policy is not `never`](child-agent-isolation.md#why-the-policy-is-not-never-qa-e-f1).

⚠ **Fixing the policy alone would not have brought the Codex shell back, and
both children read every tool result twice** (QA-E F4, same date and CLI
versions). The bridge handed the child every content block a tool returned,
including the copy a tool addresses only to the user: neither CLI filters by
audience, so the child's model read each shell result twice, and codex-cli
cannot parse the `priority` annotation on the shell's user copy, so every
`developer__shell` and `text_editor view` call failed there with `Unexpected
response type`. The child is now handed only what a model is sent, unannotated,
and the transcript stores the full result the bridge kept — see
[what the child is handed](tool-bridge.md#what-the-child-is-handed-and-what-the-transcript-keeps-qa-e-f4).

## Documents

| Document | What it covers |
| --- | --- |
| [How the coding-agent providers work](how-it-works.md) | The mechanism: what each provider spawns, which models each advertises and the CLI version each of those needs, where each vendor's credential lives, how the binary is found without spawning anything, how the conversation becomes one prompt, and how usage is accounted for a run that billed no tokens. |
| [Installing and signing in](installing-and-signing-in.md) | The user-facing setup: install each CLI, sign in by running the vendor's own command yourself, the four states the settings card can show, and the `CLAUDE_CODE_COMMAND` / `CODEX_COMMAND` escape hatch when the binary lives somewhere BioRouter does not search. |
| [The tool bridge](tool-bridge.md) | How the audited Workspace and Knowledge subsets, plus bounded workflow-owned tools, reach the child over MCP while BioRouter still executes them behind its gates; and why arbitrary host-reading extensions are deliberately excluded. |
| [What the child agent may not do](child-agent-isolation.md) | The isolation flags, each of which is security-relevant rather than hygiene: the hostile-fixture result behind `--setting-sources ""`, the measured MCP leak behind `--strict-mcp-config`, why `--tools ""` is not a substitute for either, and why `--bare` must never be passed. |
| [Compliance: vendor terms, BAA and PHI](compliance.md) | The most important page. What Anthropic's terms permit and forbid, the current state of subscription usage limits for `claude -p`, the unresolved OpenAI position, and why both providers are `ProviderTier::Public` so the privacy bind gate keeps them away from clinical sessions. |
| [Performance, limits and known gaps](performance-and-limits.md) | Measured latency and prompt overhead, the cost of a large tool surface, why conversation history is flattened rather than replayed, what the streaming path does and does not change, and the failure modes worth recognising. |
| [Streaming and tool-call parity](streaming-and-tool-call-parity.md) | **Status: Implemented.** The design record for streaming, mirrored tool calls, interactive approval, cancellation, and live steering. Its older phased analysis is retained as history; the "what shipped" section is the current inventory. |

Read [how it works](how-it-works.md) first if you are changing code, and
[installing and signing in](installing-and-signing-in.md) first if you are trying to get a
card out of "not signed in".

## Boundary with neighbouring folders

- The [providers index](../README.md) holds the per-provider integration references for
  ordinary API-key providers. This folder is a subdirectory of it because these two share a
  mechanism, not just a vendor.
- [Data privacy and PHI](../../security/data-privacy-and-phi.md) and
  [privacy tiers](../../security/privacy-tiers.md) own the institutional data-handling rules
  and the enforcement design. The [compliance page](compliance.md) here states how these two
  providers sit inside them; it does not restate the rules.
- [The coding-agent landscape](../../research/coding-agent-landscape/README.md) studies these
  vendors' agents as *external systems*. Nothing there describes BioRouter's own behaviour.

## Related documentation

- [Model provider integration references](../README.md) — the parent folder, and the
  references for the API-key providers.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md) — the
  user-facing survey of every provider, of which these two are the subscription-billed pair.
- [Data privacy and protected health information](../../security/data-privacy-and-phi.md) —
  which providers are acceptable for which data classification at UCSF.
- [Privacy tiers](../../security/privacy-tiers.md) — the capability/classification design
  whose bind gate keeps these providers out of private sessions.
