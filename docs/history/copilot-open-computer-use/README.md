# Biorouter Copilot on Open Computer Use

> **What this is.** The records of Biorouter Copilot's first native engine, Open Computer Use: the
> plan that chose it, the evidence ledger that tracked it, and the Windows verification procedure.
> **Status:** Historical record. Copilot ran on Open Computer Use 0.3.5 from 1.91 through 1.94.5. On
> 2026-11-20 the owner replaced the engine with agent-desktop, and 1.94.6 retired Open Computer
> Use: its vendored tree, patches and scripts were deleted.
> **Audience:** anyone tracing why Copilot's consent, isolation and packaging look the way they do.

Biorouter Copilot (extension key `computercontroller`) observes and controls native apps. Its first
engine was a pinned copy of Open Computer Use, a Swift, Go, Python and PowerShell helper with five
Biorouter patches, spoken to over MCP. Its consent card, its per chat isolation and its packaging
rules were set here, and most of them carried over to agent-desktop unchanged; [Biorouter Copilot on
agent-desktop](../../design/copilot-agent-desktop.md) names each one it keeps or revises.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Integration plan](integration-plan.md) | The plan of record of 2026-09-17: one native Copilot capability with ten tools, the consent and isolation rules, distribution in every package, and the removal of the older scripting and capture tools. |
| [Implementation status](implementation-status.md) | The evidence ledger: every gate of the plan with its receipts, including the macOS permission findings AD-04 still cites. |
| [Windows verification](windows-verification.md) | The procedure for verifying the retired engine on a real Windows desktop, and why a hosted runner could not reach its ready state. |

## Related documentation

- [Biorouter Copilot on agent-desktop](../../design/copilot-agent-desktop.md): the current design, which replaced this engine
- [Biorouter Copilot fixture acceptance](../../testing/copilot-fixture-acceptance.md): how the current engine is driven against real fixture apps
- [Biorouter Copilot capability](../../extensions/built-in/computer-controller.md): the user facing page
- [Historical records index](../README.md): the archive this folder belongs to
