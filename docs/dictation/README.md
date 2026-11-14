# Voice dictation

This folder holds Biorouter's local voice dictation: the speech to text engine built into Biorouter's own binaries, the speech model a person chooses to install from the Baranzini Lab's GitHub, the personal dictionary that steers recognition toward their own terms (with its left sidebar page and its agent capability), and how dictation works on every surface that offers it, from the desktop composer to `biorouter serve`, the terminal and air gapped servers. Status: the record of local dictation as it was built on branch `feat/local-dictation`. Every code pull request, PR-01 to PR-31, is committed there as of 2026-11-10 (PR-07 and PR-32 withdrawn by the owner), and PR-33 turns these documents from plan into that record; dictation has not shipped in a release yet, and version 1 is not announced. The [implementation plan](../history/local-dictation-2026-11/implementation-plan.md) is the one to read first: it holds the owner's requirements, the surface matrix, the decision register, the pull requests as they landed and every owner decision, the last ones from 2026-11-10. The design cites Biorouter's code at `main` `52917f33e`, three commits after the v1.94.1 tag, where it was written; "As built" notes say where the code differs.

## Documents

| Document | What it covers |
|---|---|
| [Engine and models](engine-and-models.md) | The whisper.cpp engine and its worker process, the native build and platform floors, CPU and GPU backends, the model catalog and tiers, hosting in `BaranziniLab/biorouter-models`, installing on every OS and deployment, the administrator's switch, the daemon's dictation routes, and the removal of the inherited cloud dictation. |
| [Dictionary](dictionary.md) | The personal dictionary: data model and store, privacy tiers and the bias set, authority on each surface, biasing and correction, learning from edits, the `dictionary` capability and its tools, automatic extraction, the left sidebar page and the terminal commands. |
| [Surfaces](surfaces.md) | Dictation on each surface S1 to S20 (S13, Jupyter AI, is not offered by the owner's decision O-30): capture, transport, where transcription runs, entry points, permissions, and the full catalog of messages a person sees. |
| [Testing and release](testing-and-release.md) | Tests per component and surface, CI jobs, release guards, fixtures, the accuracy gates on public recordings (PriMock57 and the Fareez interviews) and synthetic speech, and the definition of every measurement. The manual pass and the dated measurement log are in the [dictation test checklist](../releases/dictation-test-checklist.md). |

## Where neighbouring topics live

| Folder | What it holds instead |
|---|---|
| [Llama Server](../providers/llama-server/README.md) | The bundled llama.cpp sidecar for chat models. Dictation runs its own worker and never uses `llama-server`; the Llama Server catalog is the precedent for pinning local models by size and hash. |
| [Desktop UI](../desktop-ui/README.md) | The shell geometry, the composer card, the sidebar and shared controls that the microphone button and the Dictionary page follow. Dictation's own controls are specified here. |
| [CLI](../cli/README.md) | The command reference and the CLI checklist. The `biorouter dictation`, `dictate`, `transcribe` and `dictionary` commands are designed here; all four, and Ctrl+T and `/dictate` inside a session, are in the [command reference](../cli/command-reference.md) as they behave. |
| [Deployment](../deployment/README.md) | `biorouter serve`, reverse proxies, JupyterHub and the CLI only Linux packages. How dictation behaves behind them is here; how to deploy them is there. |
| [Bundled extensions](../extensions/built-in/README.md) | Reference pages for each extension that ships with Biorouter. The `dictionary` capability has its page there, [dictionary](../extensions/built-in/dictionary.md); its design is here. |
| [Releases](../releases/README.md) | The [dictation test checklist](../releases/dictation-test-checklist.md): the manual pass per surface S1 to S20, the dated measurement log, and the draft release notes for the first release that carries dictation. |
| [Security](../security/README.md) | Privacy tiers, managed policy, the secret guard. The dictionary follows those rules; the rules themselves live there. |
| [Voice dictation research](../research/voice-dictation/README.md) | Studies of other dictation tools and the speech model measurements behind the engine, the model tiers and the biasing design. |
| [Dictation build record](../history/local-dictation-2026-11/README.md) | The implementation plan, now a historical record: requirements R1 to R8, the surfaces, decisions DD-1 onward, privacy rules, measurements, pull requests PR-01 to PR-33 (PR-07 and PR-32 withdrawn), risks, and the owner's decisions O-1 to O-45, from 2026-11-05 to the merge on 2026-11-14. Look a decision up there; this folder says how dictation works now. |

## Related documentation

- [Dictation build record](../history/local-dictation-2026-11/README.md): the implementation plan, with the requirements, every decision and the pull request that built each part.
- [Dictation test checklist](../releases/dictation-test-checklist.md): the manual pass per surface and the dated measurement log.
- [Voice dictation research](../research/voice-dictation/README.md): the external survey and the measurements this design rests on.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules that decide which dictionary entries may shape a dictation.
- [Browser access](../deployment/browser-access.md): how `biorouter serve` is reached, which the browser surfaces depend on.
- [Documentation index](../README.md): the rest of Biorouter's documentation by topic.
