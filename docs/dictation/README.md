# Voice dictation

This folder holds Biorouter's local voice dictation: the speech to text engine built into Biorouter's own binaries, the speech model a person chooses to install from the Baranzini Lab's GitHub, the personal dictionary that steers recognition toward their own terms (with its left sidebar page and its agent capability), and how dictation works on every surface, from the desktop composer to `biorouter serve`, the terminal, Jupyter AI and air gapped servers. Status: a plan; nothing in it is implemented yet. The documents target `main` at `52917f33e`, three commits after the v1.94.1 tag, and the [implementation plan](implementation-plan.md) is the one to read first: it holds the owner's requirements, the surface matrix, the decision register, the pull requests in order and the decisions only the owner can make.

## Documents

| Document | What it covers |
|---|---|
| [Implementation plan](implementation-plan.md) | The master plan: requirements R1 to R8, the 20 surfaces, the architecture, decisions DD-1 onward, pull requests PR-01 to PR-33, risks and owner decisions O-1 to O-33. |
| [Engine and models](engine-and-models.md) | The whisper.cpp engine and its worker process, the native build and platform floors, CPU and GPU backends, the model catalog and tiers, hosting in `BaranziniLab/biorouter-models`, installing on every OS and deployment, the administrator's switch, the daemon's dictation routes, and the removal of the inherited cloud dictation. |
| [Dictionary](dictionary.md) | The personal dictionary: data model and store, privacy tiers and the bias set, authority on each surface, biasing and correction, learning from edits, the `dictionary` capability and its tools, automatic extraction, the left sidebar page and the terminal commands. |
| [Surfaces](surfaces.md) | Dictation on each surface S1 to S20: capture, transport, where transcription runs, entry points, permissions, and the full catalog of messages a person sees. |
| [Testing and release](testing-and-release.md) | Tests per component and surface, CI jobs, release guards, fixtures, the real speech accuracy gate, the manual checklist and the measurements still to take. |

## Where neighbouring topics live

| Folder | What it holds instead |
|---|---|
| [Llama Server](../providers/llama-server/README.md) | The bundled llama.cpp sidecar for chat models. Dictation runs its own worker and never uses `llama-server`; the Llama Server catalog is the precedent for pinning local models by size and hash. |
| [Desktop UI](../desktop-ui/README.md) | The shell geometry, the composer card, the sidebar and shared controls that the microphone button and the Dictionary page follow. Dictation's own controls are specified here. |
| [CLI](../cli/README.md) | The command reference and the CLI checklist. The `biorouter dictation`, `dictate`, `transcribe` and `dictionary` commands are designed here and join the command reference when they ship. |
| [Deployment](../deployment/README.md) | `biorouter serve`, reverse proxies, JupyterHub and the CLI only Linux packages. How dictation behaves behind them is here; how to deploy them is there. |
| [Bundled extensions](../extensions/built-in/README.md) | Reference pages for each extension that ships with Biorouter. The `dictionary` capability gets its page there when it ships; its design is here. |
| [Security](../security/README.md) | Privacy tiers, managed policy, the secret guard. The dictionary follows those rules; the rules themselves live there. |
| [Voice dictation research](../research/voice-dictation/README.md) | Studies of other dictation tools and the speech model measurements behind the engine, the model tiers and the biasing design. |

## Related documentation

- [Implementation plan](implementation-plan.md): start here for the requirements, the decisions and the order of work.
- [Voice dictation research](../research/voice-dictation/README.md): the external survey and the measurements this design rests on.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules that decide which dictionary entries may shape a dictation.
- [Browser access](../deployment/browser-access.md): how `biorouter serve` is reached, which the browser surfaces depend on.
- [Documentation index](../README.md): the rest of Biorouter's documentation by topic.
