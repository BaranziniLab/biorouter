# Desktop UI testing and debugging

This folder covers exercising the BioRouter Electron desktop app as a running
program: how to launch the dev GUI and drive it from a terminal, and what behavior to
check once it is in front of you. Come here when you need to reproduce a desktop bug by
clicking through the real app, when you are about to run a manual QA pass on a chat
surface, or when you are writing automated tests and need the behavioral spec they
encode — and when the app misbehaves on a platform you cannot run, which in practice
means Windows, where CI is the only place the code executes.

This is not where the desktop UI is designed. Visual and interaction
design explorations live in `docs/design/` (theme studios, the home-screen and
UI-cohesion redesigns); what this folder does hold, beside the debugging guides, is the
handful of **rules the GUI is built to** and that its tests enforce — one display surface
for an artifact, one visual vocabulary for Settings; how the Electron main process, daemon, and renderer fit
together is in [the architecture overview](../architecture/system-overview.md); the
terminal surfaces — the CLI and the TUI — have their own guides and QA script in
`docs/cli/`; and removed desktop features are archived under `docs/history/`. If you
arrived looking for one of those, leave now.

## Documents

| Document | What it covers |
|----------|----------------|
| [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) | The procedure for starting the Electron dev GUI from an agent shell or CI step, and the five failure modes that make a working app look broken — `ELECTRON_RUN_AS_NODE`, forge's stdin EOF, a bare `npx vite` serving the app with no CSS, full-screen screenshots, and an invisible NSAlert that deadlocks the dev bundle. Current. |
| [Renderer testing traps](renderer-testing-traps.md) | Ways a frontend test passes while the code it covers is broken: a `vi.fn` spy makes a floating promise unobservable, `throwOnError` is off by default so an HTTP 500 resolves, and a request-generation counter keyed on "newest issued" discards results it should keep. Current. |
| [Debugging the dev GUI with agent-browser](agent-browser-debugging.md) | How to drive the Electron dev GUI from an ordinary terminal with the `agent-browser` CLI over the Chrome DevTools Protocol, and why this repo exposes that protocol on port 9333 rather than the Playwright default of 9222. Current. |
| [Debugging Biorouter on Windows](debugging-on-windows.md) | How to diagnose a Windows-only failure from a Mac, when nobody on the project develops on Windows. The first question is whether the job failed or **hung** — a hung `test (windows-latest)` looks like a slow one and hides every test behind it, and the tell is the job's duration against its own history (20–41 min is normal; 120+ is a hang). Covers why a Windows test hangs rather than fails, what can actually be run from a Mac and what only CI can answer, the Windows traps that have bitten this repo (an open file cannot be deleted; `HOME` is undefined), and the five currently-open Windows defects with what has already been ruled out for each. Current; open-defect snapshot 2026-08-20. |
| [When the app "stops scaling with the window"](window-scaling-regressions.md) | The recurring regression where resizing stops changing the layout: the one product cause (a fixed pixel cap, which no jsdom test can catch) and the four impostors that produce the identical symptom — CDP viewport emulation pinning `innerWidth`, an `osascript` resize that silently no-ops, a dead daemon leaving a blank page with no layout to reflow, and the SIGKILL that follows copying a code-signed binary in place. Current. |
| [The startup freeze, and main-thread blocking generally](startup-freeze-and-main-thread-blocking.md) | Why the app froze a few seconds after every launch (#88): a `spawnSync` on the Electron main thread, not CPU contention. How to tell a blocked event loop from a slow one, the two instruments that measure it, the A/B that closed it, and the four things this symptom is routinely mistaken for. Current. |
| [How an Auto Visualiser figure's libraries reach the renderer](artifact-cdn-assets.md) | The two-sided mechanism behind `BIOROUTER_AUTOVIS_CDN`: a figure may never touch the network at display time, so the Electron main process pre-fetches each pinned CDN URL and inlines it before the CSP applies. The two invariants that keeps alive — every emitted URL is on the desktop list, and each is emitted as a `<script src=…>` a classic-script rewriter can replace — plus how to read a "library failed to load". Current. |
| [Where a generated artifact is displayed](artifact-display-surfaces.md) | The rule that a figure, an app card or any generated artifact has exactly ONE display surface — the artifact side panel — on all three transcript surfaces (live chat, saved session, shared session), and why that is enforced by a required prop rather than by convention. Covers what the removed inline renderer actually cost (a second CSP, a second action channel, a second resize contract, a fabricated session id), what was deleted with it, and what deliberately stayed (MCP Apps are a different feature). Current. |
| [The preview panel](preview-panel/README.md) | The working documents for expanding the artifact side panel: a measured survey of every render branch, image list and guard as it stands, and the plan to widen it along five axes — more image formats, the Office gap around the renderers that already ship, live websites in their own native view, an annotation channel back into the chat, and agent access to what the panel is showing. Plan **executed**; the implementation record covers what shipped, what was verified against real Electron, and what is still open. |
| [The provider catalog](provider-catalog.md) | The one surface listing every provider the daemon serves: three tabs carrying §14.5's privacy taxonomy, institutions named from the daemon's affiliation payload rather than from a literal, AI agents ahead of the API providers, and a default tab computed from the bound provider / a subscription-ready CLI / Local. Also the first-run screen, which is the same component in `mode="onboarding"`, the `BIOROUTER_ONBOARDING_SKIPPED` escape from it, and the composer's no-model state that makes that escape honest. Current. |
| [The settings visual vocabulary](settings-visual-vocabulary.md) | The nine rules the Settings view (Models, Chat, App) is built to, and the two primitives they lean on: a row's fill never depends on its state, rows are direct children of their list, one note shape with a ceiling on it, type roles rather than sizes, and the button ladder. Covers why six of them are asserted at the source rather than in a render test — jsdom never runs Tailwind, and two of the defects only appear in the cascade. Current. |
| [Diverge behavior checklist](diverge-behavior-checklist.md) | A catalog of 68 user actions for Diverge — the feature that branches a conversation into a new session — each paired with the behavior BioRouter must exhibit, serving as both a manual QA script and the spec the automated tests encode. Current; last revised 2026-07-18, when the dashboard-canvas items were deleted alongside dashboard mode itself. |

The checklist and the agent-browser guide are meant to be used together: the checklist
marks each item **[T]** (covered by an automated test) or **[UI]** (requires driving the
real app), and the agent-browser guide is how you drive the app for every `[UI]` item.

## Related documentation

- [CLI QA checklist](../cli/qa-checklist.md) — the sibling manual test script covering the terminal surfaces, where this folder covers the desktop ones.
- [Environment variables](../configuration/environment-variables.md) — reference for `PLAYWRIGHT_CDP_PORT`, `BIOROUTER_PATH_ROOT`, and the other knobs the dev-GUI launch depends on.
- [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md) — what to collect once you have reproduced a GUI failure here.
- [Dashboard mode history](../history/dashboard-mode/README.md) — the archived removal record for the desktop feature whose items were deleted from the Diverge checklist.
