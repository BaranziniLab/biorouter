# Apps SDK

The Apps SDK is the contract behind **Biorouter applications** — each a TypeScript front-end (`src/main.ts` plus `index.html`) wired to a real per-app BioRouter agent over a WebSocket, authored through the Application Drafter tools and driven live by its own runtime agent. This folder holds the contract in three layers: what actually ships today, the design of record behind it, and the phased plan by which it lands.

Come here when you are authoring or editing an app, working on the SDK surface itself, or need to settle whether a given capability exists in this build. Two neighbours own adjacent ground. [`docs/agent-drafter/`](../agent-drafter/README.md) is the subsystem *map* — what an app is, how the pieces fit, and the frozen 100-spec corpus used to stress-test it; go there to understand Application Drafter as a whole or to run a test campaign. [`docs/history/apps-sdk-rfc-2026-06/`](../history/apps-sdk-rfc-2026-06/strategy-and-openai-comparison.md) holds the superseded June 2026 RFC that framed the layered-SDK direction; go there for the pre-v2 framing, not for current behaviour.

One rule governs reading these three together: **the reference is the authority whenever it and the other two disagree.** The design and the roadmap describe intent and sequence; only the reference is verified against the code.

## Documents

| Document | What it covers |
|---|---|
| [BioRouter Apps SDK v2 reference](sdk-reference.md) | The human-facing developer reference: the manifest schema, the `br.*` client runtime, the agent-facing `ui_*` tools and widget catalog, the WebSocket frame protocol, the security model, the export format and the test gates. Current, and the authority on shipped behaviour — it documents what actually ships in this build and gathers every partially realised piece into one "What is and is not shipped" table. |
| [BioRouter Apps SDK v2 design](v2-design.md) | The design of record: nine pillars that evolve Application Drafter from "apps that are mostly chatbots with a run button" into a real application SDK, with a component catalog, shared reactive state, platform APIs behind `br.*`, capability-gated security and multi-agent worker profiles. Current, authored 2026-07-12 after an adversarial review with the findings incorporated; it describes *intent*, so some pillars have shipped, some are landing, and some remain design-only. |
| [BioRouter Apps SDK v2 phase roadmap](v2-phase-roadmap.md) | The six-phase implementation plan — shared state document, catalog v2 with `ui_patch`, typed RPC and signals, `br.kb` and multi-agent profiles, theme packs, and hardening plus standalone export v2 — each phase independently shippable and mergeable. Current and partly executed: it records the *intended* sequence rather than the achieved state, and carries its own map of which phases have landed. |

## Version tokens

The three documents share a vocabulary worth knowing before you open any of them. **v1** is the original Application Drafter shape — a generated front-end that flattens its controls into one English prompt and streams markdown back. **v2** is the current, additive surface: typed actions, shared state, a component catalog, platform APIs and worker profiles. Every v1 manifest, frame and API still works unchanged, and v2 fields are all optional. **v2.1** labels work deliberately deferred out of v2.

## Related documentation

- [Application Drafter](../agent-drafter/README.md) — the subsystem map this folder is the territory for; start there if you want the shape of Application Drafter rather than the SDK contract.
- [BioRouter Apps platform design](../agent-drafter/apps-platform-design.md) — the subsystem overview that summarises these nine pillars, plus a retained historical record of the v1 redesign.
- [Application Drafter 100-app test-drive runbook](../agent-drafter/testing/app-test-drive-runbook.md) — how to exercise an authored app end-to-end in a browser, which is how the roadmap's phase gates are actually driven.
- [Auto Visualiser capability](../extensions/built-in/auto-visualiser.md) — the `render_*` tools that `ui_figure` embeds into an app panel.
