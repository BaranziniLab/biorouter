# Dictation on every surface

> **What this is.** How local dictation works on each of Biorouter's 20 surfaces, S1 to S20: where audio is captured, how it travels, which process transcribes it, the controls and states a person sees, the operating system permissions involved, and the exact text shown when something is missing or impossible. It also holds the complete message catalog.
> **Status:** Current: the record of local dictation as it was built on branch `feat/local-dictation`, PR-01 to PR-33 (PR-07 and PR-32 withdrawn), with every code pull request committed by 2026-11-10 and before the release that carries it. Design rationale and the measurements taken while building it are kept with their dates; "As built" notes say where the code differs from the design.
> **Audience:** developers who maintain the desktop, `serve`, terminal, Crew, app, clipboard and ACP surfaces (PR-17 to PR-31; PR-32 was withdrawn); reviewers who check that no surface ships a dead control; operators of shared hosts who need to know where models go and what people will see.

This is one of the four area documents of the [implementation plan](../history/local-dictation-2026-11/implementation-plan.md). It elaborates the plan's decisions DD-1, DD-27 to DD-31 (the client side), DD-60 to DD-66, DD-69, DD-70, DD-72 to DD-88 and DD-92, and follows every other decision in the plan's register. The engine, the worker, the model catalog and the daemon side of the routes are in [engine and models](engine-and-models.md). The dictionary, its tiers, its sidebar page and its agent capability are in [dictionary](dictionary.md). Tests, CI jobs and the manual checklist are in [testing and release](testing-and-release.md). Where this document and the register disagree, the register wins.

## How to read this document

Load bearing claims carry a label:

| Label | Meaning |
|---|---|
| [M] | Measured on 2026-11-04 during this design, on real binaries, browsers or audio |
| [C] | Read in Biorouter's code at `main` (`52917f33e`, three commits after the v1.94.1 tag; the workspace version is 1.94.1); a line number is against `main`. Citations added after a pull request landed name the file and symbol on `feat/local-dictation` at that pull request; the symbol is the anchor, not the line |
| [R] | Reported by a third party's source, documentation or issue tracker, not measured here |
| [I] | Inferred; the measurement that would settle it is named when it matters |

Identifiers: R1 to R8 are the owner's requirements; S1 to S20 the canonical surfaces; DD the plan's decisions; PR-01 to PR-33 and A-1 its pull requests (PR-07 and PR-32 withdrawn); O the owner's decisions of 2026-11-05 and the plan's defaults; M-1 to M-22 the measurements, dated in the [checklist's log](../releases/dictation-test-checklist.md#measurement-log) (M-9 and M-13 withdrawn); MSG the messages in the [message catalog](#message-catalog); P-1 to P-16 the privacy invariants. Code is cited as a file plus a symbol, with paths from the repository root. A file that does not exist yet is written as a code span, never as a link.

Two rules govern every section below. First, a control, command or route appears only in the pull request that makes it work end to end, and until PR-21 landed the `serve` gates, Biorouter's pages hid every dictation control when `isBrowserSurface()` was true (DD-94). Second, a surface that cannot capture audio never shows a control that does nothing: it shows a specific alternative and a sentence from the catalog (R8, and the SD-8 rule "a control that can never work here says so, before it is touched").

## The rule that reaches every surface

DD-1 has two halves: **capture where the person is, transcribe where the agent and the dictionary live.**

1. Only a process in front of the person opens a microphone: the desktop renderer, a browser tab, or a capture child of `biorouter` at a person's terminal. A daemon never opens one. On a server it would record the wrong room, and on macOS the permission system would charge whichever process started the daemon [I]. `biorouterd` links no capture code at all (P-10).
2. The process that owns the target chat transcribes, with its own account's model and dictionary: the daemon for Biorouter's pages and `biorouter` for the terminal front ends. The bias set is therefore decided from the target chat's tier on the same host, and only text crosses into a chat.
3. Every transcription runs the same Rust pipeline from the leaf crate `crates/biorouter-dictation`, linked into both binaries, so no surface depends on a package another surface lacks.
4. One dictionary per account, at `<config>/dictionary/`, shared by the daemon, the CLI and the `dictionary` capability under one file lock.
5. Composers Biorouter does not own (ACP editors, app pages opened in a browser, other applications) get **Dictate to clipboard**, from the desktop tray or `biorouter dictate --copy`.

Where audio is captured:

| Capture | How | Surfaces |
|---|---|---|
| A Biorouter page (the Electron renderer, or a browser tab on `serve`) | `getUserMedia` into a 16 kHz `AudioContext` and an AudioWorklet (`components/dictation/capture.ts`) | S1 to S6, S8 and S16 through `serve`, S14 (desktop), S15 (desktop preview), S17, S18 (desktop) |
| A capture child of `biorouter` | `biorouter __br-dictation-worker capture`, miniaudio over CoreAudio, WASAPI, PulseAudio or ALSA | S9, S14 (CLI pipe), S18 (CLI), S19 |
| None | the surface has no microphone or no Biorouter control | S7, S10, S11 (live), S12, S13 (not offered, O-30), S16 (inside the container), S20 inherits its surface |

How audio travels:

| From | To | Transport |
|---|---|---|
| A Biorouter page | its daemon | the chunked HTTP stream `/dictation/streams` (DD-27), the same code on the desktop and `serve`, because both build one renderer source with one Vite config and `utils/surface.ts` tells them apart at run time [C `ui/desktop/package.json` `build:web`] |
| The capture child | its parent `biorouter` | length prefixed frames on the child's stdout; nothing leaves the process tree |

Which process transcribes:

| Process | Worker | Surfaces |
|---|---|---|
| `biorouterd` (the desktop's daemon, a `serve` daemon, a backend host, the Docker image) | `biorouterd __br-dictation-worker engine` | S1 to S6, S8 (browser part), S14 desktop, S15, S16 `serve`, S17, S18 desktop; S20 as its surface |
| `biorouter` (TUI, REPL, `dictate`, `transcribe`) | `biorouter __br-dictation-worker engine` | S7 (`transcribe` on the host), S8 to S11, S14 CLI, S16 `transcribe`, S18 CLI, S19; S20 as its surface |

Three things hold everywhere:

- Biorouter keeps audio in memory on the transcribing host: no temporary file, no log line, no diagnostics entry, no session row (P-1). Logs carry only a digest of the stream's id, its target kind and bias scope, model ids, the compute backend, a warm up's reason, the sample rate, the segment count and durations, never audio or text (DD-93, [engine and models](engine-and-models.md#logging)). A reverse proxy in front of `serve` is outside Biorouter: nginx with its stock settings writes each chunk to a temporary file [M], so the [proxy guidance](#biorouter-serve-s5-to-s7) makes the setting that keeps chunks in its memory a requirement.
- Nothing is sent automatically, on any surface (DD-70). Dictation inserts text for the person to check; **Send**, Enter or `y` is always the person's act.
- The transcribing process decides which dictionary entries may bias a request, from the target, never from the client (DD-30, P-4). Targets that leave Biorouter or reach other people are public by default: Crew, the clipboard, the app preview and `dictate` take private terms only for one recording a person confirms after MSG-PRIVATE-TERMS, never for an agent (O-8). ACP and `transcribe` without an admitted `--session` are public only, because no proof of a person reaches their transcription (P-5). Jupyter AI has no dictation (O-30). A private entry carries the institutions of the chat it came from, those behind the chat's model and those whose connectors the chat touched (O-35, O-36), and shapes only dictation aimed at a model every one of them admits (O-32, P-16).

## The surface matrix

The condensed form of the plan's matrix. **Bias** says which dictionary entries may shape the result: public entries are always eligible; private entries only when the transcribing process judges the target private capable and the caller has private standing (the user action proof, a stated private capability, or a person at an interactive terminal), or, for Crew, the clipboard, the app preview and `dictate`, when the person includes them for one recording after MSG-PRIVATE-TERMS (O-8); with the privacy master switch off every entry is eligible. A private entry that carries one institution is eligible only when the target chat's model is covered by that institution's agreements and no other's, or is a local model; one that carries two or more institutions is eligible only for a local model (O-32, O-35, P-16). Messages are named by id; their text is in the [message catalog](#message-catalog).

| Surface | Capture | Transcription | Model folder and installer | Bias | Entry point | When it cannot work | Release |
|---|---|---|---|---|---|---|---|
| S1 desktop app, macOS Apple Silicon | Renderer, every composer; hidden window for the clipboard | App's `biorouterd` worker, Metal | Per user folder; popover, Settings or CLI; default `whisper-large-v3-turbo-q5_0` | Chat tier (the desktop holds the proof); Crew, clipboard, preview public unless the person includes private terms for one recording (O-8) | Microphone left of **Send**; ⌘⇧D; tray (while **Show in menu bar** is on) | MSG-MIC-DENIED-MAC, MSG-MIC-RESTRICTED, MSG-NO-DEVICE, MSG-POLICY, MSG-NO-TRAY | PR-19 (Settings PR-18, sidebar PR-20, clipboard PR-28) |
| S2 desktop app, macOS Intel | As S1 | `biorouterd` worker, CPU, `x86-64-v3` | As S1; default `whisper-small.en-q8_0` | As S1 | As S1 | MSG-CPU with MSG-CPU-DESKTOP, MSG-CPU-ROSETTA, MSG-TURBO-CPU | PR-19 |
| S3 desktop app, Windows x64 | As S1 (WASAPI through Chromium) | `biorouterd.exe` worker, static mingw, CPU | `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\<id>\` | As S1 | As S1 with Ctrl+Shift+D; tray while a window is open | MSG-MIC-DENIED-WIN, MSG-WINDOW-OPEN, MSG-CPU, MSG-CPU-DESKTOP | PR-19 (clipboard PR-28) |
| S4 desktop app, Linux deb and rpm | As S1 (PulseAudio or PipeWire through Chromium) | `biorouterd` worker, CPU | Per user folder, or `/var/lib/biorouter/dictation/models` | As S1 | As S1; tray in place of the shortcut on GNOME Wayland | MSG-NO-DEVICE, MSG-MIC-BUSY, MSG-GNOME, MSG-WINDOW-OPEN, MSG-CPU | PR-19 (clipboard PR-28) |
| S5 `serve`, browser on the same machine | Browser at the printed `http://127.0.0.1:8765/?t=…` address | Host's `biorouterd` worker | Host account; browser installs on the secret unless locked (O-2) | Private only for a private target with standing | Served microphone and **Dictionary** page | MSG-MIC-BROWSER, MSG-INSTALL-LOCKED, MSG-DISCLOSE-HOST, MSG-CPU-SERVER | PR-21 |
| S6 `serve` behind an HTTPS proxy | Browser over HTTPS, also under a JupyterHub prefix | Per user `biorouterd` worker | Per account, `--system`, or managed `models_dir` | As S5 | As S5 | MSG-SHARED-ORIGIN, MSG-413, MSG-WAIT, MSG-SERVER-BUSY, MSG-SERVER-MEMORY | PR-21 |
| S7 `serve` at a plain HTTP address | None (insecure origin) | Not reached; tunnel to S5, or `transcribe` on the host | Host account | n/a | `ssh -N -L 8765:127.0.0.1:8765 <host>`, then the printed address with 127.0.0.1 as its host, keeping its `?t=` part | Existing 403; MSG-INSECURE; MSG-SERVE-TUNNEL in the banner | PR-21 |
| S8 CLI only Linux packages on a server | None on the server; the browser through S5 or S6 | `/usr/bin/biorouterd` worker; `/usr/bin/biorouter transcribe` | Service account's folder, or `--system` | As S5; `transcribe` under DD-77 | Served microphone; `biorouter transcribe <file>` | Over SSH: MSG-LINUX-NOAUDIO, then MSG-SSH-SERVICE when the service runs, else MSG-SSH-ROUTES; at a local console or in any other terminal on the host outside SSH: MSG-LINUX-NOAUDIO then MSG-LINUX-ROUTES; MSG-OTHER-ACCOUNT, MSG-CPU-SERVER, MSG-CPU-VM | PR-11, PR-12, PR-21 |
| S9 interactive CLI, local | Capture child | `biorouter` worker, in process | Per user folder; `dictation install` or `/dictate install` | Session tier (TUI, REPL); stored tier (shared prompt, attach) | Ctrl+T, `/dictate`, `biorouter dictate` | MSG-TCC-ZERO, MSG-WIN-ZERO, MSG-NO-MODEL-CLI, MSG-TOO-SHORT | PR-25, PR-26 |
| S10 CLI over SSH or in a container | None; refused before a device opens | Remote worker (`serve`, `transcribe`), or the laptop's | Remote account, or the laptop's | Remote as S5; clipboard routes public | `serve` and a tunnel; paste; `transcribe` | MSG-SSH-ROUTES, MSG-SSH-SERVICE, MSG-SSH-MAC, MSG-CONTAINER, MSG-SYNC-HINT | PR-25 (messages), PR-21 (served route) |
| S11 noninteractive CLI | None | `biorouter transcribe FILE\|-` | Account folder; `install --yes <id>` | `transcribe`: public only, unless DD-77 admits `--session`; `dictate`: public unless a person at the terminal passes `--private-terms` (O-8) | `transcribe`; `"$(biorouter dictate)"` from a person's terminal | MSG-AGENT-SHELL, MSG-NO-TTY | PR-12, PR-25 |
| S12 ACP clients | None in the editor | Clipboard routes: desktop daemon or CLI worker | Local folder | Public only inside ACP (O-28); the clipboard routes follow S18 | Tray, global shortcut, `dictate --copy`, then paste; the editor's `/dictate` command | MSG-ACP-DICTATE from `/dictate`; MSG-ACP-NOTE in the turn; MSG-ACP in the docs | PR-30 (note, `/dictate`), PR-28 and PR-25 (clipboard) |
| S13 Jupyter AI persona chat | None: not offered, decided by the owner on 2026-11-05 (O-30) | None | None | n/a | None: Biorouter adds no control there, so there is no dead control | None | None (PR-32 withdrawn) |
| S14 Crew | Desktop renderer; CLI capture child | Member's own daemon or CLI worker | Member's folder | Public by default; private terms for one recording after MSG-PRIVATE-TERMS (O-8, O-21) | Microphone before Crew's **Send** and immediately left of the **Ask my agent** pane's submit button; the CLI pipe | MSG-CREW-TIP; "Crew needs the Biorouter desktop app" | PR-27 (CLI pipe PR-25) |
| S15 Agent Drafter apps | Never the app page; for the desktop preview, the hidden capture window main opens, as for S18 | Serving daemon's worker | That daemon's folder | Public by default; the desktop preview takes private terms for one recording after MSG-PRIVATE-TERMS (O-8) | **Dictate into this app**; **Dictate to clipboard** | MSG-APPS, MSG-APPS-BROWSER | PR-03 (headers), PR-28 |
| S16 Docker image | None in the container; the host's browser through `serve` | Container's `biorouterd` worker, or `transcribe` | Data volume, or a read only mount | As S5; `transcribe` public | `docker run … serve`; `docker run … transcribe` | MSG-CONTAINER; MSG-SERVE-CONTAINER in the banner | PR-12, PR-31 |
| S17 desktop app, external or remote backend | Laptop renderer | Backend host's worker | Backend host's folder | With that server's key, as S1; without it, public only | Same microphone and sidebar | MSG-DISCLOSE-REMOTE, MSG-REMOTE-HTTP, MSG-REMOTE-NO-KEY, MSG-REMOTE-OLD, MSG-POLICY | PR-29 |
| S18 other applications | Hidden capture window; CLI capture child | Desktop daemon or CLI worker | Local folder | Public by default; private terms for one recording after MSG-PRIVATE-TERMS, never for an agent (O-8) | Tray, global shortcut, `dictate --copy` | MSG-CLIPBOARD-EXPLAIN, MSG-SHORTCUT-TAKEN, MSG-GNOME, MSG-WINDOW-OPEN, MSG-NO-TRAY, MSG-NO-CLIPBOARD | PR-28 (desktop), PR-25 (CLI) |
| S19 WSL | WSL2 with WSLg: capture child through WSLg's PulseAudio; WSL2 without WSLg and WSL1: none | WSL's `biorouter` worker, CPU | WSL account; `--from /mnt/c/…` | As S9 | As S9; `serve` from a Windows browser; the Windows app | MSG-WSL2-NOPULSE, MSG-WSL2-NOWSLG, MSG-WSL1 | PR-25, PR-26 |
| S20 air gapped or offline hosts | As its surface | Local worker | `dictation urls`, `install --from`, **Rescan**, managed `models_dir` | As its surface | As its surface | MSG-OFFLINE | PR-11 (CLI), PR-18 (**Rescan**) |

Every surface also honours the administrator's managed `dictation:` block (DD-90): MSG-POLICY when dictation is off, MSG-POLICY-CAPTURE when only live capture is off, MSG-POLICY-DOWNLOADS or MSG-POLICY-MODELS when only installing is narrowed. Hosts whose processor lacks AVX2, FMA, F16C or BMI2 (or `dotprod` on Linux arm64) are covered in [hosts without AVX2](#hosts-without-avx2).

### Where speech models go, per deployment

This table is the plan's O-1 table, reproduced verbatim. Where speech models go, per OS and deployment (O-1). In every case the folder is created by the installer, shown with the account in **Settings > Models > Speech models** (**Model folder** row) and printed by `biorouter dictation status`.

| Deployment | Account that owns the files | Folder | How a person installs |
|---|---|---|---|
| Desktop app, macOS (S1, S2) | The person | `~/.local/share/biorouter/dictation/models/<id>/` | Setup popover or Settings; or `biorouter dictation install` |
| Desktop app, Linux (S4) | The person | `~/.local/share/biorouter/dictation/models/<id>/` | As above |
| Desktop app, Windows (S3) | The person | `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\<id>\` | As above |
| Any of the above with `BIOROUTER_PATH_ROOT` set | The person | `<root>/data/dictation/models/<id>/` | As above |
| `biorouter serve` (S5 to S7) | The account that runs `serve` | That account's data folder, as above | The browser's Settings with the secret (O-2), or `biorouter dictation install` on the host |
| CLI only package run by systemd (S8) | The service account, `biorouter` | The service account's data folder, `~biorouter/.local/share/biorouter/dictation/models/` | `sudo -u biorouter -H biorouter dictation install` |
| Any server, shared by every account | root installs; every account reads | `/var/lib/biorouter/dictation/models/` (Linux), `/Library/Application Support/Biorouter/dictation/models/` (macOS), `%ProgramData%\Biorouter\dictation\models\` (Windows) | `sudo biorouter dictation install --system` |
| A managed fleet | The administrator | The managed `dictation.models_dir`, read only, still hash checked | Files placed there by the administrator |
| Docker (S16) | `biorouter` in the container | A data volume at `/home/biorouter/.local/share/biorouter`, or a read only mount at `/var/lib/biorouter/dictation/models` | `docker run … <image> dictation install`, or the browser's Settings through `serve` |
| WSL (S19) | The WSL Linux account | `~/.local/share/biorouter/dictation/models/` inside WSL | `biorouter dictation install`, or `--from /mnt/c/…` to reuse a Windows side download |
| External backend (S17) | The account that runs the backend | The backend host's folder, not the laptop's | Settings on the laptop with that server's key, or the host CLI |
| Air gapped hosts (S20) | As the surface | As the surface | `biorouter dictation install --from <file\|dir>`, or files placed in the folder and **Rescan** |

The lookup order, the writers and the installer are specified in [engine and models](engine-and-models.md). No speech model is ever part of an install or an update (R6, DD-19).

## Common pieces: browser capture, transport and inserting text

Every Biorouter page records, uploads and inserts with the same modules, added in PR-17 and kept free of React so Crew's composer and the hidden capture window import them unchanged. The hidden capture window records for both **Dictate to clipboard** (S18) and **Dictate into this app** in the live preview (S15); there is no preview bar:

```text
ui/desktop/src/components/dictation/
  capture.ts              getUserMedia, AudioContext, worklet, fallbacks
  pcmCapture.worklet.js   processor 'biorouter-pcm-capture' (plain JS)
  dictationApi.ts         the one module that spells the /dictation/* routes
  transport.ts            the chunked stream client
  dictationStore.ts       status store over GET /dictation/status
  dictationSession.ts     which composer owns the microphone
  micErrors.ts            error to sentence map
  copy.ts                 every string in this document's catalog for the renderer
ui/desktop/src/utils/dictatedText.ts      spliceDictation, insertDictatedText, mapAnchor
ui/desktop/src/utils/dictationLedger.ts   locateEdit and the correction ledger's spans
ui/desktop/src/utils/composerDrafts.ts    insertIntoComposerDraft (new)
ui/desktop/src/utils/micPermissionSettings.ts   OS settings URLs (not the Copilot helper) and the fake microphone switches
ui/desktop/src/utils/dictationBusy.ts     the main process's record of dictating windows, for the update restart
```

`dictationApi.ts` was written by hand against the shapes in [engine and models](engine-and-models.md) before PR-15's routes and the generated client existed; a source guard keeps every `/dictation/` path in that one file, so moving the JSON calls onto the generated client changes only it.

### Capture in a Biorouter page

DD-64. The steps, all inside the click or keydown handler that starts dictation, which runs when the phase is `idle` or the status has not loaded yet. Steps 1 and 2 run synchronously at the top of the handler, before any `await`, including the wait for the [status store](#the-status-store), and so does step 3 where it must run inside the gesture:

1. Check, before any request or prompt: `!window.isSecureContext || !navigator.mediaDevices?.getUserMedia` puts the button in `unavailable` with MSG-INSECURE. Chromium exposes the `AudioWorkletNode` constructor on an insecure origin while `ctx.audioWorklet` is missing, so testing the constructor is not enough [M].
2. Call `getUserMedia({audio: {deviceId, echoCancellation: false, noiseSuppression: <per viewer, default false>, autoGainControl: false}})`. `deviceId` is `{exact: id}` when the person chose a device in Settings. Automatic gain control stays off because Chromium's gain control on Windows rewrites the system microphone volume [R].
3. Start the 16 kHz context: the one `prewarmCapture()` made before the press, with the worklet module loaded and the context suspended (see [the context before the press](#the-context-before-the-press)), or else a new `new AudioContext({sampleRate: 16000, latencyHint: 'interactive'})`. Where a context may start without a gesture, it is resumed, or made, only once `getUserMedia` has resolved, because a context made or resumed beside `getUserMedia` holds the device back [M]: in the desktop app, whose Electron starts a context without a gesture, and in a browser page whose prepared context started on its own (Chromium and Firefox do after the page's first gesture). Elsewhere `resume()` runs inside the same gesture, before any `await`: WebKit's `resume()` never settled without a gesture [M]. Whether the gesture survives an `await` on a network request was not measured [I], which is why nothing is awaited before this step there.
4. Load the worklet with `import workletUrl from './pcmCapture.worklet.js?url&no-inline'`, starting `addModule` as the context is made, before the press for a prepared one. `vite.renderer.config.mts` also sets `build.assetsInlineLimit` to a function that returns `false` for `*.worklet.js`.
5. Connect `createMediaStreamSource(stream)` to `new AudioWorkletNode(ctx, 'biorouter-pcm-capture', {channelCount: 1, channelCountMode: 'explicit'})`, never to `ctx.destination`. The processor downmixes, clamps, converts to Int16, and posts chunks of 1,600 frames (100 ms at 16 kHz) with their RMS and peak.
6. Fallbacks, in this order: a `NotSupportedError` from `createMediaStreamSource` (measured in Firefox 146 [M]; Bugzilla 1674892 fixes it in Firefox 148 [R]) closes the context and retries at the device's native rate, declaring that `sample_rate` to the daemon, which resamples; any `addModule` rejection (`AbortError` or `SecurityError`), or a worklet node that cannot be created once the module loaded (an `InvalidStateError` for a processor that did not register), switches to `MediaRecorder`, decoded on stop with `decodeAudioData` and `OfflineAudioContext(1, n, 16000)`, then uploaded as PCM chunks with no live meter (a recording it cannot decode ends with MSG-RECORDING-FAILED, never MSG-TOO-SHORT); an `OverconstrainedError` for a stale `deviceId` retries by label, then the default device.
7. Every exit (stop, cancel, error, unmount, track `ended`) stops every track and closes the context. The operating system's microphone indicator is the proof.

```ts
// ui/desktop/src/components/dictation/capture.ts
export interface CaptureOptions {
  deviceId?: string;          // per viewer; stale ids fall back by label, then default
  deviceLabel?: string;       // that device's label, for the fallback
  noiseSuppression: boolean;  // "Reduce background noise", default false
  onChunk(pcm: Int16Array, level: { rms: number; peak: number }): void; // 100 ms each
  onLive?(): void;            // the first render quantum that carries the microphone's input
}
export interface Capture {
  readonly sampleRate: number;              // 16000, or the device rate on the Firefox fallback
  readonly mode: 'worklet' | 'recorder';    // 'recorder' has no live level
  readonly timings: CaptureTimings;         // ms from the call: stream, module, connected, live, first chunk; at the stop, the backlog and the wait for it
  stop(): Promise<void>;                    // hands over the last partial chunk, ends tracks, starts closing the context
  readonly lastChunkDelivered: boolean;     // set inside stop() unless it waits for audio recorded before the stop
  cancel(): void;
  readonly ended: Promise<'stopped' | 'track-ended'>;
  readonly closed: Promise<void>;           // the context is closed; nothing on the stop path waits for it
}
export function startCapture(opts: CaptureOptions): Promise<Capture>; // call inside the gesture
export function prewarmCapture(): void;     // enumerateDevices() at most every 30 s, and one suspended 16 kHz context with the module loaded; opens no microphone
```

**Warming before the press.** The first `getUserMedia` in a window pays for the browser's first device enumeration. `prewarmCapture()` calls `enumerateDevices()`, which opens no device, shows no microphone indicator and asks for no permission, and `noteDictationInterest()` in the [status store](#the-status-store) calls it when a composer's field takes focus or the pointer reaches a microphone, and the enumeration runs at most every 30 s. Measured in the `dictation-capture-electron` probe on an Apple Silicon Mac (Electron 39.8.10, the fake device, 2026-11-05, a fresh profile per run from the live check on) [M]: press to `getUserMedia` resolving took 209 to 463 ms on the first press in a fresh window (five runs) and 28 to 38 ms after the warm-up (thirteen runs; 64 ms once, while two other Electron apps, a vitest run and a cargo build shared the machine), and press to the first audio frame in the worklet 30 to 40 ms (108 ms in that run); warming an `AudioContext` instead only reached 91 ms. With the warm-up, the first 100 ms chunk arrives 126 to 154 ms after the press (195 ms in the loaded run). On stop the worklet's last partial chunk arrives 0.5 to 0.8 ms after a stop that is not an input event, but 0.7 to 10.2 ms after a click (eleven stops in six runs, then eighteen in the live check, fifteen of them 5 to 10 ms) [M], because Chromium renders the frame that follows an input event before it delivers the worklet's answer to the page, so the wait depends on where the click falls between frames [I]. That wait is the client's floor on the stop path: the tail cannot go up before it arrives. A composer that renders on the stop adds to it only what its render takes beyond that wait: with a stand in render of 10 ms on every change of the session's snapshot (`BIOROUTER_DICTATION_CAPTURE_RENDER_MS=10`) the last chunk came 11 ms after the click, whether the flush was posted before the render or after it, since the page takes the answer only once the render is done [M]. So the card's render on stop had to stay well inside one frame.

#### The context before the press

As built in PR-19's live check (2026-11-07), `prewarmCapture()` also makes the capture context before the press: a 16 kHz `AudioContext` with the worklet module loaded, then suspended, so it renders nothing and holds no audio device, and no microphone indicator shows. It makes one when none is waiting and no capture is live (a context made beside a live microphone, as the pointer reaches the microphone to stop, would land on the stop path), never again in a page whose module failed to load, and in a browser only once the page has had a user gesture, because a context made before one starts suspended with a console warning. The session calls it again once a dictation has ended in a composer that is still there, after the capture's context closed and the card rendered, because the next press may follow with nothing in between that notes interest: the field keeps its focus and the pointer stays on the microphone. The press takes that context and resumes it after `getUserMedia` resolves where step 3 allows, inside the gesture otherwise; a press that finds none makes one the same way. A press (the microphone, or the chord as the first key) notes interest only after the capture has started, or that interest would make the context inside the press, beside `getUserMedia`.

Why [M]: in the dev app on an Apple Silicon Mac (Electron 39.8.10, the fake device, real presses through the Chrome DevTools Protocol, eight presses each, load average 15 to 35), press to the first audio frame took 88 to 100 ms when the press made the context and loaded the module beside `getUserMedia` (the first PR-19 draft; `getUserMedia` itself 87 to 99 ms), 44 to 53 ms when it resumed a context made before the press beside `getUserMedia` (`getUserMedia` 44 to 48 ms), and 24 to 35 ms, median 27, when it resumed that context once `getUserMedia` had resolved (`getUserMedia` 21 to 32 ms), which is what ships; with nothing prepared, the context made after the stream gave 28 to 36 ms (once 104 ms). `getUserMedia` alone, with no context anywhere, took 23 to 25 ms. No long task ran during the press, so the cost is the context and the device starting together, not the development build. Through the real composer on Home, eight dictations at load average 17 to 25 [M]: press to the first audio frame 37 to 52 ms, median 45, with `getUserMedia` resolving 35 to 50 ms after the press, because the press's own render (the composer disables Send while arming) holds the main thread 23 to 31 ms after the press in the development build. In the production build of the `dictation-capture-electron` probe, two runs the same day [M]: 22 and 28 ms with the context prepared, 25 and 26 ms in its hidden window, which prepares none, and 34 and 43 ms through the session in its one whole dictation, which prepares none either.

As built in PR-19, the stop no longer asks the worklet for its audio. The worklet posts each render quantum's samples (128 frames, 8 ms at 16 kHz) as it makes them, stamped with the context frame they start at, and `capture.ts` assembles the 100 ms chunks itself, so on stop the page usually holds the audio already: `stop()` hands over the last partial chunk before it returns, and the `flush` message only tells the worklet to end before the context closes, off the path. What the page has not received at the stop is bounded by how late the main thread takes the worklet's messages, not by one quantum: messages posted while it was busy (a render, a long task) wait in its queue, and an input event is handled ahead of them. So the stop reads the context's clock, and the stamps say how much audio recorded before that moment is still on its way. Up to 16 ms of it (`STOP_BACKLOG_SLACK_MS`, two quanta) is dropped and the stop does not wait; more is taken as it arrives, until the stamps reach the stop's frame, the worklet's `final` answer comes (the port keeps order, so nothing it posted is still to come), or 250 ms pass, and only then is the last chunk handed over and `lastChunkDelivered` set; `timings.stopBacklogMs` and `timings.stopWaitMs` record both. Measured in Chrome 157 with the fake microphone, `capture.ts` and the worklet served from 127.0.0.1, every stop a real mouse press (2026-11-07, Apple Silicon) [M]: at 50 presses on an idle page nothing was on its way 48 times and one quantum (8 ms) twice, within the slack, so no stop waited; after a long task of 40, 150 or 400 ms just before the press, 40, 144 to 152 and 400 ms of audio was on its way (ten presses each), all of which the stop of the first PR-19 draft discarded, and the stop now waited 0.4 to 0.8 ms for it (17.8 ms once, at 400 ms) and lost none. The `dictation-capture-electron` probe, run the same day unloaded and again with a stand in render of 60 ms on every snapshot change, still measured `stop()` returning the last chunk 0.3 and 0.4 ms after the click and `finish` leaving 0 to 1 ms after it, with samples for 99.7 % of the wall time [M]. The session then starts the stream's `finish` before it tells the card Transcribing, and `dictationApi.ts` sends that request in the first microtask (the secret is kept once read), so the request is on the wire before the render the publish schedules. The chat composer also does not render on the stop unless its placeholder must change: it subscribes to its gate (free, holding, or listening into an empty field), and the microphone and Send's reason draw Transcribing on their own. Measured in the `dictation-capture-electron` probe against its loopback stand in (2026-11-07, Apple Silicon, load average about 50) [M]: `stop()` returned the last chunk 0.3 ms after the click, `finish` left 1 ms and reached the stand in 2 ms after it, and the worklet's answer to the flush, which used to be the floor, came 9 ms after the click with nothing waiting for it. In the dev app, with real clicks on the real composer (debug daemon, Metal, the same load) [M]: `finish` left 0.6 to 2.7 ms after the stop in six dictations, one of them stopped with the chord (PR-15's live check measured 10 ms to the daemon), no `PUT` followed the stop, and the composer's Transcribing commit came 3.2 to 5.3 ms after the stop once the composer subscribed to its gate (13.5 ms before that change, in the development build).

What the measurements showed [M]:

- Vite 7.3.1 inlines assets under 4,096 B as `data:` URLs, the worklet included, and the renderer's CSP (`script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'`) blocks a `data:` or `blob:` worklet; dictation then dies on the first click with `AbortError: Unable to load a worklet's module` in Electron, Chromium and Firefox. `?url&no-inline` emits `assets/<name>-<hash>.js`, which loads in 1 to 2 ms from a packaged `file://` app (plain folder and `app.asar`) and in every browser.
- Capture into a 16 kHz context gave RMS 0.35353 against 0.35355 expected and 440.1 Hz for a 440 Hz tone in Electron 39.8.10 and Chromium 145; WebKit 26.0, fed a synthetic 48 kHz stream, gave RMS 0.35329 and 441.5 Hz. Firefox 146.0.1 throws `NotSupportedError` on a cross rate `createMediaStreamSource`; the native rate path works there.
- An unconnected `AudioWorkletNode` is still processed. `channelCount: {ideal: 1}` does not give mono; `channelCountMode: 'explicit'` does. The resampler overshoots peaks by up to about 5 %, hence the clamp.
- A worklet in a hidden, never shown Electron window delivered 375 render quanta per second, still at 350 s.
- Speech through the whole client path, with a recorded clip as the fake device (`BIOROUTER_DICTATION_CAPTURE_SPEECH_WAV`, `c01_Samantha.wav` and `s09_Daniel.wav` from the bench, 2026-11-05): after its first 130 ms, the audio the loopback stand in received matched the clip at one constant lag, 36.9 dB signal to error (38.1 dB below 6 kHz), with no seam at a 100 ms chunk or a 2 s `PUT` boundary and the level within 0.03 dB. Chromium's resampling between the clip and the device rate cut the band above 6 kHz (energy 0.72 of the clip's at 6 to 7.2 kHz, 0.12 at 7.2 to 8 kHz). whisper small.en and large-v3-turbo q5_0 transcribed the received audio word for word as they transcribed the clip. The first 28 ms the worklet received after `onLive` (450 or 451 samples) were digital zeros, the clip's first 39 ms never reached it, and in one run, on a loaded machine, a 10 ms glitch 119 ms in replaced 8.75 ms with zeros and repeated 1.3 ms. All of that falls before anyone could start speaking after the card changes.

### The client side of a stream

DD-27 to DD-31. `components/dictation/transport.ts` is the only stream client, and it reaches `/dictation/streams` only through `components/dictation/dictationApi.ts`. That module calls the generated functions it uses by name and reads them only when it sends a request, so a composer test that mocks `../api` with a factory lacking them still renders.

| Route | Body | Answer |
|---|---|---|
| `POST /dictation/streams` | JSON at most 8 KiB `{sample_rate, format: "s16le", language?, target_kind: "chat" \| "new_chat" \| "editor" \| "crew" \| "external", session_id?, new_chat_provider?, new_chat_model?, new_chat_decision_models?, include_private_terms?, context?}` (`context` at most 2 KiB, ranking only, never logged; `new_chat_provider`, `new_chat_model` and `new_chat_decision_models` only with `new_chat`; `include_private_terms` only with `crew` or `external`) | 201 `{stream_id, max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600}`; 409 `noModel`; 429 `tooManyStreams` with `Retry-After`; 403 with the policy sentence; 503 `serverMemory` |
| `PUT /dictation/streams/{id}/audio?seq=N` | `application/octet-stream`, raw s16le, at most 524,288 B | 200 `{received_s, partial?, queue_position}`; 409 `outOfOrder {expected_seq}`; 410 expired or cancelled; 413; 500 `engineFailed` once a decode failed while the person spoke |
| `POST /dictation/streams/{id}/finish[?seq=N]` | none, or the last chunk with its `seq` | 200 transcript, or 202 `{pending: true}` after 3 s; 500 `engineFailed` when the worker exits or misses its deadline; for the chunk, the `PUT`'s 409 and 413 |
| `GET /dictation/streams/{id}?wait_ms=3000` | none | as finish, held until the transcript is ready, at most 3 s |
| `DELETE /dictation/streams/{id}` | none | 204; queued segments dropped, the audio and the transcript zeroed |

Every route takes `X-Secret-Key` through `check_token`. Stream creation also sends `await userActionHeaders()`: the user action proof on the desktop, and on `serve` an `X-Caller-Provider` naming the host's configured provider [C `ui/desktop/src/utils/userAction.ts` `userActionHeaders`]. As decided by the owner on 2026-11-05 (O-17), the proof is not required to create a stream; it only decides whether private entries may bias (DD-29).

What each composer sends:

| Where the button is | `target_kind` | `session_id` | `context` |
|---|---|---|---|
| `ChatInput` in an existing chat (tab, split pane) | `chat` | the chat's id | the draft before the anchor, cut to its last 2 KiB on a character boundary (as built in PR-19; the visible transcript tail is not sent, because `ChatInput` holds only the transcript's length) |
| `ChatInput` on Home or a new tab, before the chat exists | `new_chat`, with `new_chat_provider` set to the tab's provider pick when it has one, and `new_chat_decision_models` set to every decision model the chat's draft monitors name when it has a draft (DR-33) | none | the draft before the anchor |
| The message editor (`UserMessage.tsx`) | `editor` | the chat's id, from its `sessionId` prop; as built in PR-19 its one caller, `ProgressiveMessageList.tsx`, does not pass it yet (outside PR-19's files), so the editor's streams name no chat and get public terms only, the safe side of DD-30 | the message text before the anchor, cut to its last 2 KiB of UTF-8 on a character boundary, as `ChatInput`'s |
| Crew's channel composer and **Ask my agent** | `crew`, with `include_private_terms: true` only for a recording the person started with **Include my private terms** | none | the draft before the anchor |
| The hidden capture window, for **Dictate to clipboard** (S18) and **Dictate into this app** in the live preview (S15) | `external`, with `include_private_terms: true` under the same rule | none | none |

The daemon decides the target tier once per stream (DD-30): with `session_id`, `session_reach` runs first and a refusal degrades silently to public bias with a byte identical response; `new_chat` uses the tier the new chat's bind would produce, never a declared tier ([engine and models](engine-and-models.md)); `crew` and `external` are public unless the body carries `include_private_terms: true` from a caller with the proof and private standing, which the renderer sends only after the person confirms MSG-PRIVATE-TERMS for that recording (O-8). The provider the new chat will bind is `new_chat_provider` when the caller holds the user action proof, and `BIOROUTER_PROVIDER` otherwise: an unproven caller's `new_chat_provider` is ignored, never refused, so the answer reveals nothing. A new chat's decision models count toward its tier as they will once it starts (DR-33): a draft that names a public decision model, such as `joey-1` on Baseten beside Versa, makes the target public, and a draft sent without the proof makes it public too ([dictionary](dictionary.md), "A new chat's decision models"). The private draft flag (DD-71) still warns if that text is then sent to a public model.

The stream, step by step:

1. **Create** when capture starts, in parallel with `getUserMedia`. A 409 `noModel` opens the setup popover; a 403 shows the policy sentence from the body; a 429 shows MSG-SERVER-BUSY with `<n>` counted down from `Retry-After` (at most 30 s), then tries again. It tries again on every 429 while the person speaks, since the audio is buffered meanwhile and waiting costs no speech, and for up to 30 s after the stop. A 429 after that ends the dictation with MSG-SERVER-BUSY-FAILED and **Retry**, and the audio is kept for 5 minutes, as for MSG-ENGINE-FAILED.
2. **Buffer** the worklet's 100 ms chunks in memory. The client keeps every chunk until `finish` answers with text, so a failed stream can be replayed; after a failure it keeps them for at most 5 minutes, for **Retry**, and drops them on the next dictation, on cancel or on unmount.
3. **Upload** every 2 to 3 s (64,000 to 96,000 B at 16 kHz), never more than `max_chunk_bytes`. Exactly one `PUT` is in flight, with a 10 s timeout. On a timeout the same `seq` is sent again with the same bytes; the daemon treats a repeated `seq` as idempotent. The sixth timeout in a row on one chunk (60 s, the daemon's idle expiry) ends the dictation with MSG-LOST, and so does a fourth network error, 5xx or 429 answer in a row on a chunk the next step cannot shrink. The daemon never answers a `PUT` with 429, so one comes from a proxy that rate limits, and it is waited out like any other transient failure, for at most 10 s at a time. Capture keeps buffering while a `PUT` waits. A failure in this step, or in steps 4, 5 and 7, ends the dictation when it happens, also while the person is still speaking (`onFailed`): the microphone goes off and the card shows the notice, rather than recording into a stream that can take nothing more until the stop.
4. **Shrink** on a 413, or on a network error for a chunk over 64 KiB: halve the chunk size and resend, with a floor of 64 KiB. A 413 at 64 KiB ends the dictation with MSG-413, because the proxy in front cannot pass even the smallest chunk. After a network error the first, larger body may have arrived with only its answer lost; the daemon then ignores the halved repeat of that `seq` and answers with the full chunk's `received_s`, and the client skips the half it would otherwise send twice.
5. **Reorder** on 409 `outOfOrder`: resend from `expected_seq`.
6. **Wait visibly.** When a `PUT` or a held read has had no answer for 1.5 s, the card shows MSG-WAIT until it answers.
7. **Recover** from a 410 that the person did not cause (the 60 s idle expiry after a long stall): open one new stream and replay the buffered audio from `seq=0`. A second failure ends the dictation in the `error` phase with MSG-LOST and inserts nothing.
8. **Preview** partial text: when a `PUT` answer carries `partial`, show it in a muted preview line inside the card, never in the field (DD-31).
9. **Cap**: show MSG-CAP-WARN 30 s before `max_seconds` (at 570 s of 600), then stop, insert and show MSG-CAP-STOPPED at 600 s. The cap is a stop, never an error.
10. **Finish** on stop: wait for a `PUT` still in flight, then `POST …/finish?seq=N` carrying the remainder when it fits in one chunk (a remainder over the chunk size goes up as `PUT`s first), so a stop costs one round trip (PR-15). A finish that times out is sent again with the same `seq` and bytes, which the daemon answers as a repeat; a 413 on the carried chunk halves it and sends the rest as `PUT`s, and a 409 `outOfOrder` resends from `expected_seq`. A 200 carries the transcript; a 202 means read `GET /dictation/streams/{id}?wait_ms=3000`, which the daemon holds until the transcript is ready, one read after another with no sleep between them; only a failed read backs off by 500 ms per failure. No request is held longer than `finish` itself.
11. **Engine failure**: a 500 `engineFailed` from `finish` or a poll (the worker crashed, or missed its result deadline after the GPU retry) ends the dictation in the `error` phase with MSG-ENGINE-FAILED and its **Retry** action in the card's notice row. A `PUT` answers the same 500 once a segment's decode failed while the person still speaks, because the stream then takes no more audio; the client says so at once through `onFailed`, as in step 3, and never resends it as a passing 5xx. **Retry** opens a new stream and replays the buffered chunks; nothing is inserted until it succeeds.
12. **Cancel** with `DELETE`; queued segments are dropped and nothing is inserted.

```ts
// ui/desktop/src/components/dictation/transport.ts
export type DictationTargetKind = 'chat' | 'new_chat' | 'editor' | 'crew' | 'external';
export interface StreamRequest {
  sample_rate: number;            // 16000, or the declared native rate
  format: 's16le';
  language?: string;
  target_kind: DictationTargetKind;
  session_id?: string;            // 'chat' and 'editor' only
  new_chat_provider?: string;     // 'new_chat' only; honoured only with the user action proof
  new_chat_model?: string;        // 'new_chat' only, beside new_chat_provider, under the same rule
  new_chat_decision_models?: { provider: string; model: string; adapter?: string }[]; // 'new_chat' only: the draft's decision models (DR-33); only more public
  include_private_terms?: boolean; // 'crew' and 'external' only; sent only after MSG-PRIVATE-TERMS, for one recording
  context?: string;               // at most 2 KiB, ranking only
}
export interface Transcript {
  text: string; raw_text: string;
  changes: { start: number; end: number; from: string; to: string; stage: string; entry_id?: string }[];
  segments: { t0: number; t1: number; text: string }[];
  model: string; audio_s: number; elapsed_ms: number;
  bias_scope: 'public' | 'private';
  retried: null | { reason: string };
  no_speech: boolean;
  timings?: { segments: number; wait_ms: number; tail_ms: number; tail_audio_s: number };
}
export interface DictationStream {
  readonly id: string | null;                      // a replay or a retry replaces it
  readonly ready: Promise<void>;                   // the daemon accepted the stream
  readonly audioSeconds: number;
  push(pcm: Int16Array): void;                     // buffers; uploads on its own clock
  finish(): Promise<Transcript>;
  retry(): Promise<Transcript>;                    // after engineFailed or a busy server: a new stream, the same audio
  cancel(): Promise<void>;                         // DELETE, nothing inserted
  release(): void;                                 // DELETE after the text went in
  onPartial(cb: (text: string) => void): () => void;
  onWaiting(cb: (waiting: boolean) => void): () => void;   // drives MSG-WAIT
  onCapWarning(cb: () => void): () => void;                 // 30 s before max_seconds
  onCapReached(cb: () => void): () => void;                 // at max_seconds: stop and insert
  onBusy(cb: (seconds: number) => void): () => void;        // a 429 at creation: MSG-SERVER-BUSY
  onFailed(cb: (error: DictationStreamError) => void): () => void; // the upload failed while recording
}
// Returns at once, so audio can be pushed while the daemon answers the create; `ready` settles with it.
export function createDictationStream(req: StreamRequest, options?: { signal?: AbortSignal }): DictationStream;
export function openDictationStream(req: StreamRequest, signal: AbortSignal): Promise<DictationStream>;
```

The status read, stream creation, the held read and the delete go through the client generated from the daemon's OpenAPI document (PR-15), given the same daemon address `getApiUrl` gives; the shapes `dictationApi.ts` declares are checked against the generated ones by the type check. The `PUT` and `finish`, whose bodies are raw bytes, are a hand written `fetch(getApiUrl(…))` (`ui/desktop/src/config.ts` `getApiUrl`) with `X-Secret-Key` and `Content-Type: application/octet-stream`. A packaged `file://` page sent them with no CORS preflight [M]; a `serve` page is same origin.

Measured in the `dictation-capture-electron` probe against a loopback stand in for the daemon that holds each `PUT` 15 ms (Apple Silicon, Electron 39.8.10, 2026-11-05, six runs of a 3 s dictation, then nine more in the live check, two of them of 5 s speech clips) [M]: the create left the page 1 to 23 ms after the press, after capture had started; on stop, the final `PUT` left 6 to 10 ms after the click, `finish` 25 to 29 ms after it, and the text was in the field 1 or 2 ms after the transcript arrived. So the client then added about 10 ms and one `PUT` round trip to the daemon's own time. No run sent a CORS preflight. With `finish` carrying the last chunk (PR-15; the same probe, 2026-11-06, a 3 s dictation, load average above 20) [M]: one `PUT` while the person spoke and none after the stop; `finish` left the page 1 ms after the stop with the last 32,768 B, reached the stand in 2 ms after it, and the text was in the field 4 ms after the stop. Press to the first audio frame was 36.5 ms in that run.

The same probe against the real routes (PR-15's live check, 2026-11-06, load average about 10): with `BIOROUTER_DICTATION_CAPTURE_DAEMON` set, the stand in records the same times and passes every request on to a running dev `biorouterd` with both catalog models and the 14 benchmark terms, so the fake microphone's audio reaches a real model [M]. For two speech clips of 5.1 and 6.1 s on each model, press to the first audio frame (the card showing recording) took 30 to 48 ms; on stop, the worklet's last chunk came 8 ms after the click and `finish` reached the daemon 10 ms after it, carrying 22 to 56 KB of tail; the text was in the field 1 or 2 ms after the transcript. Stop to text in the field was 148 and 197 ms on `small.en` q8_0 and 318 and 330 ms on turbo `q5_0`, of which the daemon's own `finish` to transcript was 128, 177, 298 and 311 ms, with no held read and no wait for the load. With the dictionary the second clip came out as "EGLN1 encodes PHD2, which hydroxylates HIF-2α under normal oxygen."

In the dev app with the microphone of PR-19 (real clicks on the real composer on Home, the fake device playing a bench clip, the debug daemon on Metal, 2026-11-07, load average about 50 from other builds) [M]: stop to text in the field was 117.5 ms for 4.3 s of audio and 191 and 249 ms for 6.5 s on `small.en` q8_0 (the daemon's own `finish` to transcript 105, 176 and 229 ms), and 480 and 598 ms on turbo `q5_0` (462 and 580 ms, slower than PR-15's 298 and 311 ms because of the load). The client's share, stop to the answer less the daemon's time, was 10.8 to 18.8 ms, of which `finish` leaving took 0.6 to 2.7 ms; the transcript to the text in the field took 1.4 to 1.7 ms; and one ⌘Z after each insert removed exactly the dictated words and left "Check the " the person had typed. Press to the recording look took 102 to 139 ms in that development build, against 34 to 69 ms in the probe's production build. Most of that difference was the press making the 16 kHz context and loading the worklet beside `getUserMedia`, not the development build: with [the context made before the press](#the-context-before-the-press) and resumed after the stream, press to the first audio frame in the same development build was 37 to 52 ms [M].

Why there is no long lived dictation request: the renderer already holds up to five long requests by design, two observer streams (`MAX_LIVE_OBSERVER_STREAMS = 2` at `crates/biorouter-server/src/state.rs:879`), two long polls and one `/reply` [C]. Electron, Chromium, Firefox and WebKit allow six HTTP/1.1 connections per origin; with five held a `PUT` took 2 to 10 ms, and with six held it waited for a long poll to end [M]. Over HTTP/2 no engine of the three stalled, and with ten streams held a `PUT` took 7 to 17 ms [M]. Uploads of 64 to 512 KiB took about 3 to 14 ms through nginx across the three engines [M]. M-5 measures the real app with two windows and two `serve` tabs.

### Inserting text

DD-63. One function places the transcript, for every Biorouter composer:

1. **Anchor.** When recording starts, remember the field's selection (`selectionStart`, `selectionEnd`). A caret strictly inside a word, between two letters or digits, moves to the nearer end of that word, its end on a tie, with an apostrophe, hyphen or underscore between letters counted as part of the word, so the text never splits a word: PR-19's live check found "Call th|e OMOP" coming out as "Call th … e OMOP" [M]. A selection, and a caret at a word's edge, stay as they are. Every later change to the prose maps the anchor through the same single region diff the correction ledger uses (`locateEdit`). An untouched selection is replaced; a selection the person typed into collapses to its mapped end.
2. **Splice** with `spliceDictation`: collapse runs of whitespace, strip a leading `/` (a spoken "slash bug" never becomes a command), add a leading space unless the text before is whitespace or an opening bracket or quote, add a trailing space only in the middle of text before a word, change no case.
3. **Undoable path.** If the field owns focus (`document.activeElement` is the field, the body, or inside its `.biorouter-composer-card`) and no IME composition is running: `field.setSelectionRange(anchor.start, anchor.end)`, then `document.execCommand('insertText', false, inserted)`, then check `field.value.slice(start, start + inserted.length) === inserted`. During a composition, wait for `compositionend`.
4. **State path.** Otherwise (focus elsewhere, `execCommand` returned `false`, or the check failed): compute the new value from the composer's current prose and set it through the composer's state, as the `onQuotedText` effect in `ChatInput.tsx` does through `setComposerText` [C]. This path is not undoable.
5. **Always** call `setHasUserTyped(true)`, so history recall cannot replace the box; never call `checkForMentionOrSlash`, so a spoken "@" opens no menu; leave the caret after the inserted text.
6. **Composer gone.** If the composer unmounted (tab switch, split, leaving Home), splice into the stored draft at the mapped anchor with `insertIntoComposerDraft(key, at, text)`. Never `giveBackToComposer`, which puts returned text before the draft.
7. **Find the field by reference.** `ChatInput` already holds its textarea in a local `textAreaRef` (`useRef`, attached to the `<textarea>`) [C]; it passes that ref to `useDictationComposer` (`components/dictation/useDictationComposer.ts`), which attaches the composer to the session and hands the microphone its control. Never look the field up by `#dynamic-textarea`: that id is a literal, so a split view repeats it in each pane [C].

```ts
// ui/desktop/src/utils/dictatedText.ts
export interface Anchor { start: number; end: number }
export function spliceDictation(value: string, anchor: Anchor, text: string):
  { value: string; inserted: string; caret: number };
export function insertDictatedText(
  field: HTMLTextAreaElement | null,
  anchor: Anchor,
  text: string,
  setByState: (value: string, caret: number) => void,
  options?: { composing?: boolean },  // true takes the state path; the session waits for compositionend first
): 'undoable' | 'state' | 'draft';     // 'draft': no field; the caller splices into the stored draft
export function mapAnchor(anchor: Anchor, before: string, after: string, caretAfter?: number): Anchor;
// ui/desktop/src/utils/composerDrafts.ts
export function insertIntoComposerDraft(key: string, at: number | Anchor, text: string): void;
```

Measured on Electron 39.8.10 with React 19.2.4 [M]: `execCommand('insertText')` returns `true`, fires `input` and React's `onChange`, and one ⌘Z (through the Edit menu or `webContents.undo()`) removes exactly the dictated words; a `setSelectionRange` before each insert keeps two dictations as two undo steps; any later programmatic `setState` that changes the value wipes the whole undo history; `setRangeText` fires no `input` event and is unusable; `execCommand` writes into whatever editable element has focus, which is why step 3 checks focus. No copy may promise "⌘Z removes the last dictation" without that qualification.

### The status store

`components/dictation/dictationStore.ts` is a `useSyncExternalStore` singleton over `GET /dictation/status` (shape in [engine and models](engine-and-models.md)). It sends no request when `ChatInput` mounts, because 23 test files render the real composer under an offline fetch guard [C `ui/desktop/src/test/setup.ts`]. It loads when a composer's textarea first gains focus after the composer mounted (the mount's own autofocus does not count, so mounting the composer sends nothing), on the first keydown in the field, on the first `pointerenter`, focus or click of any microphone, and when the Settings section or the Dictionary page mounts (`noteDictationInterest()`, which also warms the device list, see [capture](#capture-in-a-biorouter-page)); refreshes on window focus and after install, select or delete; and polls every 1.5 s only while `install_job` runs. A focused textarea is where ⌘⇧D is pressed, so the chord rarely meets a store that has not loaded.

Until it has loaded, the button shows the `idle` look. A click or chord then runs steps 1 to 3 of [capture](#capture-in-a-biorouter-page) at once, inside the gesture, and only then waits for the one status request. If the phase it reports is not `idle`, the handler stops every track, closes the context and acts on the real phase (the setup popover, or the reason sentence); no stream is opened before the answer. Each status read has a 5 s deadline (`STATUS_TIMEOUT_MS`): a daemon that takes the connection and never answers would otherwise hold that press in Transcribing until Escape, and every later read of the window with it. After the deadline the store reads `failed`, the phase `idle`, the stream opens, and its create's own answer (409 `noModel`, 403, 404) decides. The interface learns policy from `status.policy`, never from `GET /config`. On the desktop each status read also asks Electron main for this computer's own policy (`laptopPolicy`, through the `dictationAnywhereGet` entry the **Dictate into other apps** row reads), which main rereads from the managed file only for an external backend; for the app's own daemon the two agree, and main answers no hidden or review window.

Where the daemon is decides what the card says about it (DD-92). `dictationDaemonPlace()` in the store answers `this-computer` only when `main.ts` told the window it started the daemon itself (`BIOROUTER_LOCAL_BACKEND`), `external-backend` on S17 and `serve` in a browser, each with the host the daemon reports for itself (`status.host`), or the address the API calls go to. MSG-ENGINE-FAILED fills `<where>` from it, MSG-SETUP fills its own `<where>` (the forms with "on", `setupWhereOnServer` in `copy.ts`), and the setup popover picks its privacy line by its `kind`. A phase decision fills MSG-INSECURE with the page's own address and MSG-REMOTE-OLD with the daemon's and this app's version even when its caller passes nothing, so no sentence reaches a person with a placeholder in it.

Per viewer conveniences live in `localStorage` key `biorouter:dictation-viewer`, every read and write wrapped in try and catch: `{deviceId, noiseReduction, showButton, ackHosts[], policyOffHint}`. `ackHosts[]` remembers each host whose MSG-DISCLOSE-HOST was shown and a SHA-256 identity of each confirmed backend URL (including its query); no URL credentials or query values are stored there; `policyOffHint` records whether the last status said policy forbids dictation; nothing hides the microphone on it, because under policy the microphone stays in place, disabled (see [before a model is installed](#before-a-model-is-installed)). The daemon checks policy on every request regardless.

## The desktop composer button in every composer

### Placement

DD-60. One component, `components/dictation/DictationButton.tsx`, a ghost round 28 px `Button` (`size="sm" shape="round" variant="ghost"`), `type="button"`, with `onMouseDown` calling `preventDefault()` so the field keeps focus and its accent edge. The glyph is `export const Mic = light(_Mic)` in `components/icons/app-icons.tsx`.

| Composer | File | Position | `target.kind` |
|---|---|---|---|
| Chats, Home, split panes, new tabs (one `ChatInput`) | `ui/desktop/src/components/ChatInput.tsx` | between the model slot (`:3060`) and `{sendOrStop}` (`:3074`); **Send** stays the row's last child, because the footer inset and `composerGeometry.browser.test.ts` are derived from its edge [C] | `chat`, or `new_chat` without a session |
| The message editor | `ui/desktop/src/components/UserMessage.tsx` | in the right aligned footer (**Cancel**, **Save as new**, **Save** at `main` [C]), immediately left of **Save**, the editor's send action, so **Save** stays last | `editor` |
| Crew's channel composer | `ui/desktop/src/components/crew/composer/Composer.tsx` | after `.crew-compose-spacer`, before `.crew-compose-send` | `crew` |
| Crew's **Ask my agent** box | `ui/desktop/src/components/crew/pane/AgentTaskPane.tsx` | immediately left of the submit button **Start my agent and allow posting here**, in the pane's `flex justify-end` row [C] | `crew` |

A decision chat (a chat whose model is a decision model) keeps the microphone in the same place, immediately left of **Send**, even where its controls row hides other controls, because dictation is input rather than a model feature (owner decision, 2026-11-12). A subagent's tab mounts the same `ChatInput` on the desktop; in a `biorouter serve` browser that tab shows its read-only reason in place of the composer, so it has no microphone there.

```tsx
// ui/desktop/src/components/dictation/DictationButton.tsx
export type DictationPhase =
  | 'unavailable' | 'not-installed' | 'installing' | 'idle'
  | 'arming' | 'recording' | 'transcribing' | 'error';
export interface DictationButtonProps {
  textAreaRef: React.RefObject<HTMLTextAreaElement | null>;  // ChatInput's own local ref; never #dynamic-textarea
  onInsert: (transcript: Transcript, anchor: Anchor) => void;
  onPhaseChange: (phase: DictationPhase) => void;            // feeds the submit gates
  draftKey: string;                                          // for insertIntoComposerDraft
  target: { kind: 'chat' | 'new_chat' | 'editor' | 'crew'; sessionId?: string };
  setView: (view: View, options?: ViewOptions) => void;      // "More in Settings", "/dictionary"
}
```

At the narrowest pane (360 px), the row keeps room for more than 12 px of the model chip's name with the microphone added, which `composerCard.browser.test.tsx` measures at 760, 440 and 360 px with the real microphone in the row (PR-19) [M].

As built in PR-19, the button takes one `control` from `useDictationComposer` rather than the props above: the composer's key, its gate and the press, stop, cancel, key and interest handlers. The target is built at the press from the composer's own state (`chat` with the chat's id, or `new_chat` with the tab's provider and model when it has its own pick), and the insert goes through the composer the hook attached, so the button needs no `onInsert` or `onPhaseChange`:

```tsx
// ui/desktop/src/components/dictation/useDictationComposer.ts
export function useDictationComposer(options: {
  key: string;                                   // the draft key, or a key unique in this window
  textAreaRef: React.RefObject<HTMLTextAreaElement | null>;
  storesDraft: boolean;
  target: () => DictationTarget;                 // read at the press
  setByState: (value: string, caret: number) => void;
  isComposing?: () => boolean;
  draftPrefix?: () => number;                    // a `/bug ` chip before the field's text
  onInserted?: (event: DictationInserted) => void;
  isEmpty?: () => boolean;                       // for "Listening…"
}): DictationComposerControl;                    // gate, holdsComposer, listening, press, stop, cancel, handleKeyDown, noteInterest
// ui/desktop/src/components/dictation/DictationButton.tsx
export function DictationButton(props: { control: DictationComposerControl; setView: SetView }): JSX.Element | null;
export function DictationNotices(props: { dictationKey: string }): JSX.Element;
```

### Before a model is installed

DD-61 and O-9. The button shows from the first launch in a muted `not-installed` look. A click opens the setup popover; it never starts a download by itself. The per viewer switch **Show the microphone button** in Settings hides it; ⌘⇧D still starts dictation, and the recording pill takes the button's place until the dictation ends. A chord that finds no model shows the button with its setup popover, and the button goes again when the popover closes, so the press never silently does nothing (PR-19). The Settings switch itself was not built by 2026-11-10: `copy.ts` holds its label and the button reads the viewer field, but no Settings row writes it, so it waits for a later release (see [Settings](#settings) and [what waits](#what-waits-for-a-later-release)). When managed policy sets `dictation.enabled: false` or `dictation.capture: false`, the button stays in every composer once the status store knows, disabled in the `unavailable` look, with the policy's own sentence as its name and tooltip: the daemon's `reason` for the refused door (MSG-POLICY, or MSG-POLICY-CAPTURE when only live capture is off), and the catalog's sentence only when the daemon gave none. On S17 the daemon's policy is the backend host's, so the store also reads this computer's own policy from Electron main and shows it the same way, with the catalog's sentence (2026-11-13). A control that can never work here says why before it is touched, as the owner ruled on 2026-11-12; until then the button was removed. A press there starts nothing, and the chord starts nothing and shows the same sentence in the notice row. Settings shows MSG-POLICY or MSG-POLICY-CAPTURE in its policy note.

### Phases

The phase is carried as `data-dictation-phase` on the button and styled in authored CSS in `components/composer/composer.css`, because a newly written Tailwind class may never generate under `BIOROUTER_NO_HMR` [C].

| Phase | Look | Name or tooltip | Click |
|---|---|---|---|
| `unavailable` | disabled ghost microphone (inside a span, so the tooltip still opens) | the reason sentence | none |
| `not-installed` | muted ghost microphone | **Set up dictation** | setup popover |
| `installing` | microphone with a progress ring | "Downloading speech model… \<n>%" | reopens the popover |
| `idle` | ghost microphone | "Dictate" plus the chord | start |
| `arming` | spinner after 150 ms, `aria-busy` | "Starting microphone…" | cancel |
| `recording` | 28 px pill: 4 or 5 accent level bars and `m:ss` (the timer dropped under a 440 px pane) | "Stop dictation, 0:07"; tooltip "Stop and insert · Esc cancels" | stop and insert |
| `transcribing` | spinner in the pill | "Transcribing…" | none (Escape cancels) |
| `error` | idle look plus one in card notice row with one action | the mapped sentence | as idle |

Which phase the button takes, checked in this order:

| Condition | Phase | Reason sentence |
|---|---|---|
| `!window.isSecureContext \|\| !navigator.mediaDevices?.getUserMedia` | `unavailable` | MSG-INSECURE |
| `status.policy.enabled` not allowed | `unavailable` | `status.policy.enabled.reason` (MSG-POLICY), or MSG-POLICY when it is missing; Settings shows the same in its policy note |
| `status.policy.capture` not allowed | `unavailable` | `status.policy.capture.reason` (MSG-POLICY-CAPTURE), or MSG-POLICY-CAPTURE when it is missing; Settings shows the same in its policy note |
| this computer's own managed policy turns dictation or live capture off (S17; Electron main reads it, `laptopPolicy` in the store) | `unavailable` | MSG-POLICY, or MSG-POLICY-CAPTURE when only live capture is off |
| `GET /dictation/status` answers 404 or 405 (S17: a backend from before dictation) | `unavailable` | MSG-REMOTE-OLD |
| `status.engine.compiled` false | `unavailable` | MSG-NO-ENGINE |
| `status.engine.state` is `unsupported_cpu` | `unavailable` | desktop: MSG-CPU then MSG-CPU-DESKTOP; Intel build under Rosetta on macOS 12 to 14 (any version before 15): MSG-CPU-ROSETTA; `serve`: MSG-CPU-SERVER |
| no installed model, `status.install_job` running | `installing` | none |
| no installed model, `status.install.allowed` true | `not-installed` | none |
| no installed model, install refused | `unavailable` | `status.install.reason` (MSG-INSTALL-LOCKED, MSG-POLICY, MSG-POLICY-DOWNLOADS, MSG-POLICY-MODELS), or MSG-REMOTE-NO-KEY on S17 |
| otherwise (a usable model, even while another one installs) | `idle` | none |

"No installed model" means `status.active_model` names no installed model. The two `status.policy` rows need a status, which a 404 or 405 does not give, so they never meet the MSG-REMOTE-OLD row; this computer's own policy comes before it because updating that server would not help here. The `installing` row comes before `not-installed`: with the order reversed, the first install would show `not-installed` until it finished, and the progress ring after **Install** would never appear (`resolveDictationPhase` in `dictationStore.ts`).

There is no fifth composer card edge and no square glyph: a square would read as a second Stop beside the turn's Stop [C `ui/Stop.tsx`]. The level bars are driven from the worklet's 100 ms RMS through CSS custom properties in a `requestAnimationFrame` loop, with no React render per frame, and are static under `prefers-reduced-motion`. The level is therefore not part of the session's React snapshot: the loop reads `getDictationLevel()` (or `subscribeDictationLevel`), and the snapshot changes only with the activity, a notice, the partial line, MSG-WAIT or the whole second of the timer, at most twice per second of speech, so a person typing while dictating pays no render per chunk. A visually hidden `role="status"` announces Recording, Transcribing, Inserted and errors. The placeholder becomes "Listening…" only when the prose is empty; the field stays editable throughout. `arming` lasts until the worklet reports the first render quantum that carries the microphone's input (`onLive`), so the person never speaks into a microphone that is not yet live [R: the Codex desktop app lost a median 587 ms of speech by showing "listening" early]. The source node's first 28 ms after that quantum are still zeros (measured with a speech clip as the fake device [M]), well under anyone's reaction to the card, so no speech is lost to them. That is 90 to 120 ms sooner than waiting for the first 100 ms chunk (31 to 40 ms against 126 to 154 ms after the press, measured in the `dictation-capture-electron` probe [M]).

Results that are not text:

| Result | What the card shows | Stream |
|---|---|---|
| Under 0.5 s of audio | MSG-TOO-SHORT | `DELETE`, nothing transcribed |
| Every sample exactly zero | MSG-SILENT | `DELETE` |
| 3 s below the noise floor while recording | MSG-NO-AUDIO-YET, recording continues | kept |
| `no_speech: true` | MSG-NOTHING | finished |
| 500 `engineFailed` | MSG-ENGINE-FAILED with **Retry** | ended; the chunks kept for 5 minutes |
| A 429 at creation still 30 s after the stop | MSG-SERVER-BUSY-FAILED with **Retry** | never created; the chunks kept for 5 minutes |
| A second failure after a 410 | MSG-LOST | ended |
| The `MediaRecorder` fallback cannot decode its recording | MSG-RECORDING-FAILED | `DELETE`, nothing transcribed |

### Gestures

DD-62. A click toggles. ⌘⇧D on macOS and Ctrl+Shift+D elsewhere, matched on `event.code === 'KeyD'` in the focused field's own `onKeyDown` (so it acts on the focused pane only), with `event.repeat` and IME composition ignored: a tap toggles, and a hold of 500 ms or more is hold to talk, stopped by the key up of D or a modifier. A window `blur` or `visibilitychange` during a hold stops and inserts. Enter while recording stops and inserts and never sends; Enter while transcribing does nothing; the next Enter sends. Escape while arming, recording or transcribing cancels, with `preventDefault()` so the sidebar overlay's Escape handler skips it. No spoken word sends or submits anything. Helpers `getDictationShortcutText()` and `getDictationAriaKeyShortcuts()` go beside `getSteerShortcutText` in `utils/keyboardShortcuts.ts`, and the button carries `aria-keyshortcuts`. The 500 ms threshold exists because the Codex desktop app's 150 ms threshold turned a 179 ms click into hold to talk and discarded the take [R]; M-18 measures tap durations.

### Submit gates

`dictationPhase` from `onPhaseChange` joins both submit paths of `ChatInput.tsx` and the condition that disables **Send** [C]: Enter in `handleKeyDown` is handled by the gesture rules before the steer, queue and `performSubmit` calls; `onFormSubmit`, which **Send** reaches as a `type="submit"` button of the composer's form, refuses while arming, recording or transcribing; and `isSubmitButtonDisabled` also turns true then, so **Send** shows as disabled with the tooltip "Finish dictating first" (arming or recording) or "Transcribing…". When a usage limit also holds the chat, that tooltip still wins while dictation holds the composer, and the usage limit notice above the text says why Send stays off afterwards; inserted text never sends itself, so a paused chat keeps it as a draft. Stop for a running turn stays available, and dictation is allowed while a turn runs; after the insert, Enter queues as usual.

### Ownership and lifecycle

`components/dictation/dictationSession.ts`, a per window module store, records which composer key owns the microphone, so speech is never lost and the microphone is never left on:

| Event | What happens |
|---|---|
| Dictation starts in pane B while pane A records | A stops, and A's text is inserted into A; edits in A meanwhile move its anchor. A press in B that cannot start (B's phase is not `idle`) leaves A recording |
| Pane A's background dictation fails | A's notice, with **Retry** and the kept audio where it has one, shows in A's card once B has ended (at once if nothing else holds the card), unless B ended with a failure of its own, which keeps the card |
| The composer unmounts while arming, recording or transcribing | capture stops, the stream finishes, and the text goes into the stored draft at the mapped anchor; during the first use macOS prompt, the microphone is never opened |
| Screen lock (main forwards `powerMonitor` `lock-screen` as `dictation:lock-screen`) | stop and insert |
| A track ends (headset unplugged) | stop and insert what was captured |
| 30 s before `max_seconds` | MSG-CAP-WARN in the card; at the cap, stop, insert and show MSG-CAP-STOPPED |
| **Restart & update** while dictating | the renderer has set `dictation:busy`; the restart asks MSG-RESTART-BUSY, with **Restart anyway** and **Cancel**, Cancel the default (as built in PR-19 in `utils/dictationBusy.ts` `restartAllowedWhileDictating`, asked by the updater's install and by the update notification's click) |

`dictation:busy` stays set while any composer in the window still waits for its text, including a pane that a press in another pane stopped, which finishes in the background. A recording kept for **Retry** does not hold it. The main process keeps the record in `utils/dictationBusy.ts`, which the updater imports, and drops a window's entry when the window closes or its renderer crashes; every new document (a reload, a crash recovery) sends `false` from its preload first, so no window stays busy for a dictation that is gone. Two windows may each record; the daemon queues their streams.

### The setup popover and disclosure

`components/dictation/SetupPopover.tsx`, anchored to the microphone (`side="top"`), in the idiom of the Llama Server inline card. It holds MSG-SETUP with `<where>` filled in ("on this computer", or "on the Biorouter server at \<host>"), the model's name, size and folder, the privacy line, **Install**, **Not now** and the link **More in Settings**; after an install, the link **Edit your dictionary**, absent under managed `dictionary: false` as the sidebar row is. Under **Install** sits the checkbox **Suggest dictionary words from my chats**, ticked and visible, with MSG-SETUP-SUGGEST below it (DD-56; the box arrives with PR-23). Progress continues in the microphone's ring and in Settings; completion shows no toast. There is no new onboarding step. As built in PR-20, **Edit your dictionary** shows once the status store holds a usable model and no install runs, beside **More in Settings**; it closes the popover and sets the hash route to `/dictionary`.

The privacy line is visible text, never an InfoTip (DD-92):

| Where the daemon is | Line |
|---|---|
| The desktop's own daemon | MSG-DISCLOSE-LOCAL |
| An external backend (S17) | MSG-DISCLOSE-REMOTE |
| A `serve` browser (S5 to S8, S16) | MSG-DISCLOSE-HOST, also shown once in the card before the first recording to a new host |

In MSG-DISCLOSE-HOST, `<host>` is `status.host`, followed by `location.host` in parentheses when they differ, because neither side can tell an SSH tunnel from a local daemon. No surface says "on this computer" when the daemon is elsewhere, and no build before PR-01 says Biorouter does not send the audio anywhere else. The sentence speaks for Biorouter only: on S6 the audio also crosses the operator's reverse proxy, which the [proxy guidance](#biorouter-serve-s5-to-s7) keeps from writing it to disk.

### The private draft flag

DD-71. As built in PR-19 the flag is not a field of `ComposerDraft` but a memory only set keyed by the composer's dictation key (`components/dictation/privateDraft.ts`), so it never reaches the stored draft; it is set when an inserted transcript reports `bias_scope: "private"`, and cleared when the draft is sent or emptied. The session sets it itself for text that goes into the stored draft of a composer that unmounted while the person spoke (a tab or chat switch), since no composer is there to set it; and "emptied" means the box and the stored draft both, so a composer mounting under the key cannot clear it for one render that shows an older, emptier copy (PR-19 review). `bias_scope: "private"` means private entries were eligible, not that a private term is in the text, and the messages say so. The send, queue and steer entry points ask before `releaseComposerText(reason)` hands their ledger away ([dictionary](dictionary.md)), covering all three exits that hand the text to a model: `performSubmit`, `handleInterruptionAndQueue` (queue) and `handleSteerFromComposer` (steer) [C `ChatInput.tsx`]. Before such a draft leaves for a chat whose model is public, judged by the daemon's instance tier for that chat (the bound provider's `tier()`, never a declared tier), the composer asks MSG-PRIVATE-DRAFT (**Send anyway**, **Cancel**); before the model switcher moves that chat to a public model, it asks MSG-PRIVATE-DRAFT-SWITCH (**Switch anyway**, **Cancel**). The person's say so proceeds. No agent path sees or answers either question. As built in PR-22, the question sits at the entry of Enter's send, queue and steer and of the form's submit; a model tier that is not known to be private asks (the stricter reading); and MSG-PRIVATE-DRAFT-SWITCH is asked for an unsent chat (Home and new tabs), whose pick reaches the chip through `onChooseForUnsentChat`, while a chat that exists is switched inside `SwitchModelModal` (`settings/models/subcomponents/`), which has no hook before its switch and is outside PR-19's files, so for an existing chat the switch asks nothing and the send's MSG-PRIVATE-DRAFT is the only question; no text leaves unasked. Adding a before switch hook there is a follow up. The two questions are titled "Send to a public model?" and "Switch to a public model?".

### Including private terms for one recording

O-8, DD-30, DD-83, DD-86. Crew's two boxes, the preview's **Dictate into this app** and **Dictate to clipboard** dictate with public terms by default, because the text goes to colleagues or to a destination Biorouter cannot classify. A person may include their private terms for one recording; an agent never can.

| Where | How the person asks | What happens |
|---|---|---|
| Crew's microphones, **Dictate into this app** | the button's context menu (right click, Shift+F10 or the Menu key) holds **Include my private terms** | MSG-PRIVATE-TERMS opens (**Include**, **Public terms only**); **Include** starts a recording whose stream carries `include_private_terms: true`, **Public terms only** starts an ordinary one |
| **Dictate to clipboard** | the tray menu holds **Include my private terms** below **Dictate to clipboard** | the review card shows MSG-PRIVATE-TERMS first, then records as above; the global shortcut always records with public terms |
| `biorouter dictate` | `--private-terms` | admitted only when `prompt_can_run()` is true, `BIOROUTER_TERMINAL` is unset and `caller_capability()` returns Private, after MSG-PRIVATE-TERMS and a `y` (DD-77); otherwise MSG-PRIVATE-TERMS-REFUSED and exit 2, before any device opens |

The choice lasts one recording and is never remembered. The item is offered only where the caller holds the user action proof and private standing (the desktop's own daemon, or S17 with that server's **User Action Key**); elsewhere it is absent. The daemon admits `include_private_terms` only with the proof, so no agent path sets it, and ACP never offers it. Included private terms cover every private row, whatever institutions it carries, decided by the owner on 2026-11-05 (O-37, DD-36).

### Error map

`components/dictation/micErrors.ts`, from the `getUserMedia` error and the operating system status from `dictation:mic-status`:

| Error | OS status | Sentence | Action |
|---|---|---|---|
| `NotAllowedError` | `denied`, macOS | MSG-MIC-DENIED-MAC | **Open settings** |
| `NotAllowedError` | any status, Windows | MSG-MIC-DENIED-WIN | **Open settings** |
| `NotAllowedError` | `restricted`, macOS | MSG-MIC-RESTRICTED | none |
| `NotAllowedError` | `granted`, macOS | MSG-MIC-GRANTED-FAILED | none |
| `NotAllowedError` | `unknown` (Linux desktop) | MSG-MIC-BUSY | none |
| `NotAllowedError` | a browser surface | MSG-MIC-BROWSER | none |
| `NotFoundError` | any | MSG-NO-DEVICE | none |
| `NotReadableError`, `AbortError` | any | MSG-MIC-BUSY | none |
| `OverconstrainedError` | any | none; retry by label, then the default device; MSG-NO-DEVICE if both fail | none |
| insecure context | any | MSG-INSECURE | none |

Errors appear in the card's notice row, never as toasts. The row clears when the next dictation starts, or with its **Dismiss** link (PR-19). A chord pressed while the microphone is `unavailable` shows the reason in the same row, since the chord has no tooltip of its own.

### Message editor and sending

DD-69 and DD-70. Before the message editor gets a microphone, PR-04 deletes the two `logInfo` calls in `UserMessage.tsx` that write message text to the packaged log: `initializeEditMode` (`:208`, the edit mode content) and the change handler (`:256`, the changed content) [C]. Dictated text reaches the editor through the change handler, so `:256` is the one dictation would otherwise log. A third call, in `handleKeyDown` (`:310`), logs each key name with its Meta and Ctrl state [C]; dictated text never passes through it, and DD-69 leaves it alone. No composer, on any surface, sends dictated text by itself, and no setting adds an automatic send. As built in PR-19, the editor's microphone sits immediately left of **Save**, and **Save** and **Save as new** wait while dictation holds the editor; Enter stops and inserts there too, and Escape cancels the dictation, not the edit. `UserMessage` takes the chat's id as an optional `sessionId`; until `ProgressiveMessageList` passes it, the editor's stream names no chat and the daemon gives it public terms only.

## Desktop permissions and packaging per OS

DD-65. A Biorouter renderer is the only desktop process that records, and its grant is narrow: a window's own composer, or the hidden capture window main opens for **Dictate to clipboard** (S18) and **Dictate into this app** (S15).

### Every desktop OS

- `utils/permissionPolicy.ts` `isAllowedRendererPermission` already grants `media` only when every requested type is `'audio'` and the origin is the app's own, "for dictation" [C]. PR-04 adds `details.isMainFrame === true`, passed by `installSessionHooks` in `main.ts`, so no subframe can ever get the microphone. Both handlers received `isMainFrame: true` for the main frame, and a denial rejects with `NotAllowedError: Permission denied` [M].
- App previews and the embedded browser keep denying every permission [C `ui/desktop/src/utils/managedAppPreviewSession.ts`].
- IPC. Every renderer to main `dictation:*` handler in `main.ts` opens with one sender check and runs nothing before it, not even an argument check. `isDictationSender` admits the main frame of a window in `windowMap`. `isDictationCaptureSender` and `isDictationReviewSender` admit the main frame of the hidden capture window and of the review card, the two windows `createAnywhereWindow` opens on Biorouter's own pages; each refuses a navigation off the app's origin and every `window.open`, so its main frame stays Biorouter's renderer. `components/dictation/dictationSourceGuards.test.ts` holds every handler to its row below, and fails on a channel it does not list.

| Channel | Direction | Sender | Payload | Answer |
|---|---|---|---|---|
| `dictation:mic-status` | renderer to main | a window, or the capture window | none | `systemPreferences.getMediaAccessStatus('microphone')` on macOS and Windows; `'unknown'` on Linux |
| `dictation:mic-request` | renderer to main | a window, or the capture window | none | macOS `systemPreferences.askForMediaAccess('microphone')` on the first click while `not-determined`; elsewhere `true` |
| `dictation:open-mic-settings` | renderer to main | a window | none | opens `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` (macOS) or `ms-settings:privacy-microphone` (Windows); no deep link on Linux |
| `dictation:show-models-folder` | renderer to main | a window | the folder the daemon reports, as a hint | opens this account's speech model folder (Settings, **Model folder**) |
| `dictation:busy` | renderer to main | a window | boolean | gates **Restart & update** behind MSG-RESTART-BUSY |
| `dictation:lock-screen` | main to renderer | | none | stop and insert |
| `dictation:anywhere:settings-ready` | renderer to main | a window | none | the window is listening, so main can send it to the **Dictate into other apps** row (S18) |
| `dictation:anywhere:get` | renderer to main | a window | none | the **Dictate into other apps** settings and what limits them |
| `dictation:anywhere:set` | renderer to main | a window | `{enabled, shortcut, disclosureShown}` | writes `anywhere.json` and answers the settings now in force |
| `dictation:anywhere:preview-focused` | renderer to main | a window | `{viewId}` | whether an editable element in that app preview has focus (S15) |
| `dictation:anywhere:preview-start` | renderer to main | a window | `{viewId, privateConfirmed}` | starts a **Dictate into this app** recording in the capture window; main inserts the text into that preview (S15) |
| `dictation:anywhere:capture-command` | main to renderer, the capture window only | | start (with source, `privateConfirmed` and a generation), stop or cancel | the capture page records or stops |
| `dictation:anywhere:capture-ready` | renderer to main | the capture window | none | the capture page is listening |
| `dictation:anywhere:capture-reply` | renderer to main | the capture window | started, finished (with the transcript), failed or cancelled, with its generation | main copies the text, inserts it into the preview or opens the review card |
| `dictation:anywhere:review` | main to renderer, the review card only | | the card's message and text | the card shows them |
| `dictation:anywhere:review-get` | renderer to main | the review card | none | the card's message, and its text until it expires |
| `dictation:anywhere:review-copy` | renderer to main | the review card | none | copies the card's text and reads it back |
| `dictation:anywhere:private-choice` | renderer to main | the review card | boolean | the answer to MSG-PRIVATE-TERMS for the tray's **Include my private terms** |
| `dictionary:add-term` | main to renderer | | `{text, x, y}` | opens the term dialog prefilled (the context menu door); the renderer resolves the chat at that point, and the add names it as `origin_session_id`, or sends `origin_unresolved: true` for a transcript it cannot resolve, so a private term takes that chat's institutions (O-36) |

- The OS settings URLs live in the new `utils/micPermissionSettings.ts`, not in the Copilot helper, which refuses on an external backend and off macOS [C `utils/copilotPermissionSettings.ts`]; the microphone is always local to the app.
- Test switches: when `!app.isPackaged && process.env.BIOROUTER_TEST_FAKE_MIC_WAV`, `main.ts` adds `--use-fake-device-for-media-stream --use-file-for-fake-audio-capture=<wav>%noloop` (plus `--disable-features=AudioServiceSandbox` on macOS, added to any features already disabled, where the sandboxed audio service otherwise cannot read the WAV and the track is digital silence, RMS 0 [M]; the fake device flag alone was not shown to cause it). It adds them as the module loads, before `ready`, like its Playwright debugging switch, not in `appMain`, which runs after `ready`; the list comes from `fakeMicrophoneSwitches` in `utils/micPermissionSettings.ts`, which the `dictation-capture-electron` probe bundles too. `--use-fake-ui-for-media-stream` is forbidden by a source guard on any use under `ui/desktop/` (a comment may name it to say it is not used, as PR-03's `serve` spec does): it bypassed the permission handler entirely, with zero handler calls and audio plus video granted under a deny all policy [M].
- Biorouter Copilot places no restriction on dictation, decided by the owner on 2026-11-05 (O-15, DD-88): an agent holding a Copilot grant on Biorouter's own window, or on a browser that shows a `serve` tab, can start the microphone by clicking it or pressing the dictation chord, and can then press **Send** as it can for typed text. What limits it is Copilot's per task grant (one grant per task, chat, model and target), the composer's visible recording state, the operating system's microphone indicator, and Stop and revoke.

### macOS (S1, S2)

- PR-04 adds `NSMicrophoneUsageDescription = 'Biorouter uses the microphone only while you dictate, and does not save the recording.'` to the existing `extendInfo` block of `forge.config.ts`, pinned by a new `styles/forgeMacUsage.test.ts` beside `styles/forgeWin32Metadata.test.ts`. `forge.config.ts` declares no such key at `main` [C], so Electron's stock `Info.plist` string "This app needs access to the microphone" is what a prompt would show [R Electron's `Info.plist`].
- PR-04 adds `osxSign.optionsForFile`, which pins `com.apple.security.device.audio-input` for the app, `Biorouter Helper.app`, `Contents/Resources/bin/biorouter` and `Contents/Resources/bin/biorouterd`. At `main`, `forge.config.ts` passes the legacy keys `entitlements` and `entitlements-inherit` [C], which `@electron/osx-sign` 1.3.3 never reads, so the shipped app carries the default entitlement set of `@electron/osx-sign`, which includes audio input only by accident [M]; under the hardened runtime a missing entitlement means no prompt and every request denied.
- `cmd_verify` in `scripts/release.sh` and `smoke_mac` in `scripts/smoke-test-release-artifacts.sh` check the entitlement on all four and the plist string with `plutil -extract NSMicrophoneUsageDescription raw`.
- The first click while the status is `not-determined` calls `dictation:mic-request`, so the prompt comes from a person's click and names Biorouter. A denial shows MSG-MIC-DENIED-MAC with **Open settings**; macOS applies a changed grant only after a restart, which the sentence says.
- A dev build launched from a terminal makes the terminal the responsible process for the permission prompt [R for the mechanism; I for each host]; the runtime probe could not exercise it, because its responsible process was already granted. Test permission behaviour with `open -n` on a signed build (M-6 records the denial name and message on a signed app).

### Windows (S3)

- Desktop apps get no per app prompt; the switch "Let desktop apps access your microphone" governs them, and per app prompts for desktop apps exist only in Insider builds [R]. Which status `getMediaAccessStatus` reports when that switch is off is not verified: Electron maps `DeniedByUser` to `denied` and `DeniedBySystem` to `restricted`, and answers `granted` when its WinRT query fails [R Electron's `electron_api_system_preferences_win.cc`]. So on Windows every `NotAllowedError` gives MSG-MIC-DENIED-WIN with **Open settings** (`ms-settings:privacy-microphone`), whatever the status, and checklist row S3.2 records the status the switch produces on Windows 11.
- Automatic gain control stays off, so dictation never rewrites the system microphone volume [R].
- Squirrel's **Restart & update** asks MSG-RESTART-BUSY while `dictation:busy` is set.

### Linux (S4)

- Chromium reaches PulseAudio or PipeWire with no operating system prompt; `dictation:mic-status` answers `unknown`.
- Failures map to MSG-NO-DEVICE or MSG-MIC-BUSY. There is no settings deep link, so no **Open settings** button.
- The deb and rpm need no new dependency for capture: Chromium already talks to the sound server.

## Settings

DD-66. One section, `SETTINGS_SECTIONS.dictation = { id: 'settings-dictation', tab: 'models' }`, mounted in `settings/models/ModelsSection.tsx` between `<LocalModelInventory/>` and the danger zone, built from `settings/models/SpeechModelSection.tsx` and `speechModelStore.ts` (the `llamaServerStore` pattern: progress survives navigation, polling only during an install). There is no fourth tab [C `ui/desktop/src/components/settings/SettingsView.tsx:33`]. The setup popover's **More in Settings** navigates with the section key `dictation`.

As built in PR-18 [C]: the status itself, an install's progress included, is the status store's (`components/dictation/dictationStore.ts`), a module singleton that re-reads `GET /dictation/status` every 1.5 s only while `install_job` runs, so leaving Settings mid download loses nothing; `speechModelStore.ts` holds the rest at module level (each row's request in flight and its refusal, the language being saved, the last **Rescan** result, the last **Test speed** run with the model it was for and its refusal, and the times **Test speed** measured from this window, per model). The section reads the status once when it opens. Every model route goes through `dictationApi.ts`'s `dictationModelsApi`, which sends `userActionHeaders()` on install, cancel, delete, select and rescan, and the secret alone on bench. After each change the status is read again, so a row shows what the daemon holds; a read that shows no running job after a `202` started before the install did, and one more read follows. The InfoTips of the section and of its **Model folder**, **Language**, **Microphone** and **Speed** rows are catalog sentences too, MSG-SETTINGS-HELP to MSG-SPEED-HELP.

| Row | Contents | Notes |
|---|---|---|
| Privacy line | MSG-DISCLOSE-LOCAL, MSG-DISCLOSE-REMOTE or MSG-DISCLOSE-HOST, as visible text in a note at the top of the section | chosen by `dictationDaemonPlace` |
| Managed policy | `privacy/ManagedPolicyNote.tsx`, in the pattern of `privacy/HostManagedModelNote.tsx` [C]: the daemon's `reason` for each refused door of `status.policy` (MSG-POLICY alone when dictation is off; otherwise MSG-POLICY-CAPTURE, MSG-POLICY-DOWNLOADS and MSG-POLICY-MODELS as they apply), the catalog sentence only when a reason is missing; locked rows disabled | shown when `status.policy.managed` and something is narrowed; a managed file that narrows nothing shows no note |
| One row per catalog model | display name over one facts line: size, "English only" for a `.en` id or "Many languages", "Recommended for this computer", "A newer revision exists", the folder of a copy this account did not install, and one warning word (`too_slow`, `below_minimum`, `unsupported_cpu`, or "Not allowed by your organization"); **Install** (with a 4px bar and **Cancel** while it runs, and the download's percentage and megabytes in the facts line), **Update**, **Use**; **Remove** in the row's `⋯` and right click menu, with "In use" or "Installed" at rest in its place | one install at a time, so every other **Install** waits; a refused install that holds for every model and that the policy note does not say (MSG-NO-ENGINE, the processor's sentence, MSG-INSTALL-AGENT, MSG-INSTALL-LOCKED) shows `status.install.reason` once, above the rows, in place of every **Install**; MSG-TURBO-CPU (with a measured time) or MSG-TOO-SLOW-SETTINGS before a `too_slow` model, MSG-BELOW-MINIMUM-SETTINGS before a `below_minimum` one; the download's megabytes count the model's own bytes, so its total is the size the facts line showed (the job also fetches the 885,098 B voice detector first, which made turbo's "574 MB" row read "of 575 MB" [M]), and its percentage is the whole job's, as the composer's; a failed install's sentence (MSG-DISK, MSG-HASH, MSG-OFFLINE, MSG-TLS, MSG-DOWNLOAD-FAILED, MSG-INSTALL-CANCELLED) replaces the facts line until the model is installed, and does not come back when that model is removed later (the daemon keeps its last job until the next install, so a cancelled job reappeared after **Rescan** and **Remove** [M]); a refused **Use** stays on its row until that model is in use or the language is saved again; MSG-REMOVE-CONFIRM before **Remove**, or for the model in use, which the daemon replaces at once, MSG-REMOVE-CONFIRM-ACTIVE naming the model that takes its place or MSG-REMOVE-CONFIRM-LAST; a copy in the system, managed or env folder names that folder in place of **Remove** |
| **Model folder** | `status.models_dir` in the mono face, then the account, on the row's second line (a path in a 40 % value column lost its end, the part that names it); **Show in Finder**, **Show in Explorer** or **Show in folder**; **Rescan**, whose result follows on a third line (MSG-RESCAN-ADDED, MSG-RESCAN-NONE, MSG-RESCAN-UNKNOWN) | the reveal button only for the daemon this app started (`BIOROUTER_LOCAL_BACKEND`); main's `dictation:show-models-folder` checks the same thing from its own record, then derives this account's folder itself by the daemon's rule (`<root>/data/dictation/models` under `BIOROUTER_PATH_ROOT`, the local app folder's `data\dictation\models` on Windows, `$XDG_DATA_HOME/biorouter/dictation/models` or `~/.local/share/biorouter/dictation/models` elsewhere; `ui/desktop/src/utils/dictationModelsFolder.ts`) and reads the renderer's `status.models_dir` only as a hint that must name that folder, so the one folder it can create or open is that one; it creates it when missing (the air gapped path puts files there before any install), and opens it only when its real path is a plain folder; **Rescan** is `POST /dictation/models/rescan` with no argument and is the air gapped path |
| **Language** | Detect automatically (`auto`), English (`en`, the default), then every other code of `WHISPER_LANGUAGES` by its English name, from a table in `speechModelStore.ts` (Electron's reduced ICU names 97 of the 99 codes through `Intl.DisplayNames` and answers `bo` and `ba` with the code [M]) | written through `POST /dictation/models/select`; a `.en` model refuses every other value, `auto` included, so while one is in use the menu holds English alone (and the saved value when it is another), disabled when English is chosen, with MSG-LANGUAGE-EN-ONLY under it, or MSG-LANGUAGE-EN while the saved language is one it refuses; a refusal shows MSG-LANGUAGE-EN with the display name; a test holds the list to `protocol.rs` |
| **Microphone** (per viewer) | System default plus the devices `enumerateDevices()` lists (without Chromium's `default` and `communications` aliases), **Test microphone** | the choice is `deviceId` and `deviceLabel` in `biorouter:dictation-viewer`; the test opens the chosen device through `capture.ts` for 5 s, shows a 4px level meter on a -60 to 0 dBFS scale, reads only the level and drops the samples (nothing is recorded or sent), ends the tracks at 5 s or when the page closes, says "Sound detected" or MSG-NO-AUDIO-YET, and a failure to open gives the error map's sentence with **Open settings** where it has one; on macOS the first test asks `dictation:mic-request` first, as the composer's first press does |
| **Speed** (**Test speed**) | `POST /dictation/bench` on the model in use, then MSG-BENCH for the model in use only: a time measured for a model no longer in use or no longer installed is not shown, and comes back when that model is in use again; a run in progress shows no earlier line | held until a model is in use, while an install runs, and while a dictation in this window arms, records or transcribes (`isDictationRunning` in `dictationSession.ts`), because the bench's two decodes run on the same worker, served round robin beside the dictation's segments; a factor above 0.5 marks a model `too_slow` |
| **Shortcut** | read only "⌘⇧D · tap to start or stop, hold to talk" | not built yet: PR-19 shipped the chord but not this row, because its files do not include `SpeechModelSection.tsx`. PR-20 edits that file for **Edit your dictionary** but owns neither `settings/models/copy.ts` nor the dictation copy that would hold the row's words, so it left the row out; a follow up adds it with its copy; Ctrl+Shift+D off macOS; hidden on a `serve` browser until M-18 records that the page receives the chord, then shown per browser. Still not built on 2026-11-10, after PR-28 and PR-29 edited the section, so it waits for a later release |
| **Show the microphone button**, **Reduce background noise** (per viewer) | two switches | not built yet, as the row above: PR-19 shipped the viewer fields they write (`showButton`, `noiseReduction` in `biorouter:dictation-viewer`) and the microphone and capture that read them; PR-20 left the switches out for the same reason as the **Shortcut** row; noise reduction maps to `noiseSuppression`, which the microphone test already reads. Still not built on 2026-11-10 (`copy.ts` holds `showButton` and `reduceNoise`, and no Settings component renders them), so they wait for a later release |
| **Dictate into other apps** (desktop only) | the switch, the shortcut picker or MSG-GNOME, MSG-WINDOW-OPEN on Windows and Linux, MSG-NO-TRAY while the tray is absent | PR-28; MSG-CLIPBOARD-EXPLAIN shows when the switch turns on (S18). The preview's **Dictate into this app** does not need this switch (O-42) |
| **Edit your dictionary** | link to `/dictionary` | PR-20; reveals the sidebar group by the current route rule. As built: a **Dictionary** row after **Speed**, with MSG-DICTIONARY-HELP as its InfoTip and a secondary **Edit your dictionary** button that sets the hash route (the section holds no router); absent under managed `dictionary: false`, as the sidebar row is |

Until PR-21, `SpeechModelSection` rendered nothing on `isBrowserSurface()`; since PR-21 it renders there with the host wording.

Measured in the dev app on an Apple Silicon Mac (debug daemon, Metal, 2026-11-07) [M]: the section showed the status 28 to 335 ms after it opened. **Install** of `whisper-small.en-q8_0` answered in under 0.2 s and showed its count at the next 1.5 s poll; a quit at 67 % (a 177,567,497 B partial) followed by **Install** in the relaunched app resumed at 68 % and reached "In use" 15.2 s after the click, of which checking the pin took about 3 s; the daemon's warm up after the install was ready in 251 ms and decoded in 77 ms, so the first dictation does not pay the load. **Rescan** placed the voice detector in 168 ms and turbo in 7.9 s (hashing 574 MB in a debug build), and turbo's warm up after it was ready in 411 ms and decoded in 259 ms. **Test speed** read 0.09 s for 5.5 s of speech on `small.en` (factor 0.02) and 0.27 s on turbo (0.05). **Use** reached "In use" in 123 to 126 ms. **Test microphone** drew its meter 73 ms after the click and its first level at 282 ms, on the fake device. One daemon finding: until the installer's first byte, `GET /dictation/status` reports a downloading job with `bytes_done` equal to `bytes_total` (`InstallJob::view` in `routes/dictation.rs` counts every file as done while `progress.id` is still empty), so the row says "Downloading…" without a count until the count is below the total.

On a `serve` browser the reveal button is hidden, **Dictate into other apps** is absent, the **Shortcut** row waits for M-18, and the install text says the download goes to the server. On S17 the rows act on the backend host, and the **Model folder** row names that host's folder and account.

## Surfaces one by one

### Desktop app (S1 to S4)

The four desktop surfaces share the capture, transport, insert, composer, permission and Settings sections above. Each subsection lists what differs. A model installed by the app and a model installed by the bundled CLI land in the same per user folder, so the two share one copy.

#### S1 desktop app, macOS Apple Silicon

| Part | Design |
|---|---|
| Capture | The renderer in every composer; the hidden capture window for **Dictate to clipboard** (S18) and for **Dictate into this app** (S15) |
| Transport | The chunked stream to the app's own `biorouterd` on loopback |
| Transcription | `biorouterd __br-dictation-worker engine` on Metal, threads `min(8, available_parallelism)`, halved while Llama Server or a loopback Ollama serves |
| Models | `~/.local/share/biorouter/dictation/models/<id>/`; default `whisper-large-v3-turbo-q5_0` (574,041,195 B) with `silero-vad-v6.2.0` (885,098 B); the Light choice is `whisper-small.en-q8_0` (264,477,561 B) |
| Bias | Public entries always; private entries when the target chat is private capable, since the desktop always holds the proof; Home and new tabs judged by the provider the new chat will bind (`new_chat_provider`); Crew, clipboard and preview public unless the person includes private terms for one recording (O-8) |
| Entry points | Microphone left of **Send** in every composer; ⌘⇧D; tray **Dictate to clipboard** while **Show in menu bar** is on; the global shortcut the person turns on; **Dictionary** in the sidebar; **Settings > Models > Speech models** |
| Permissions | TCC prompt on the first click, with the pinned usage string |
| Messages | MSG-MIC-DENIED-MAC, MSG-MIC-RESTRICTED, MSG-MIC-GRANTED-FAILED, MSG-NO-DEVICE, MSG-MIC-BUSY, MSG-NOTHING, MSG-SILENT, MSG-TOO-SHORT, MSG-ENGINE-FAILED, MSG-POLICY, MSG-RESTART-BUSY, MSG-NO-TRAY |
| Release | PR-19 (Settings PR-18, sidebar PR-20, clipboard PR-28) |
| Acceptance | A dev app run with `BIOROUTER_NO_HMR=1` and the fake device shows one ⌘Z removing exactly the dictated words in the real `ChatInput`, and nothing sent without Enter; M-4 (a signed, notarized app on macOS 12 and 13 transcribes once on Metal), M-6 (TCC denial on a signed app) and M-7 (a real microphone) recorded; checklist section S1 passes |

#### S2 desktop app, macOS Intel

| Part | Design |
|---|---|
| Capture, transport, entry points, permissions | As S1 |
| Transcription | `biorouterd` worker on the CPU, built for `x86-64-v3`; when `sysctl.proc_translated == 1` the parent sets `ROSETTA_ADVERTISE_AVX=1` for the worker |
| Models | As S1; the default is `whisper-small.en-q8_0`; without a **Test speed** record, turbo is `too_slow` on fewer than 16 cores (DD-22), and after a record it is `recommended` only at a real time factor of 0.5 or less; it installs after MSG-TURBO-CPU |
| Messages | As S1, plus MSG-CPU then MSG-CPU-DESKTOP on a processor without AVX2 (a 2013 Mac Pro), and MSG-CPU-ROSETTA for the Intel build under Rosetta on macOS 12 to 14 (any version before 15) |
| Release | PR-19 |
| Acceptance | `smoke_mac` runs the self test for x64 under `arch -x86_64` with `ROSETTA_ADVERTISE_AVX=1` on macOS 15 or later; M-10 records latency on an Intel Mac; checklist section S2 passes |

#### S3 desktop app, Windows x64

| Part | Design |
|---|---|
| Capture | As S1; Chromium captures through WASAPI |
| Transcription | `biorouterd.exe __br-dictation-worker engine`, a static mingw build on the CPU; system DLL imports only |
| Models | `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\<id>\`: local, not roaming, and outside Squirrel's `app-<version>` folder, so updates never touch it; default `whisper-small.en-q8_0` |
| Entry points | As S1 with Ctrl+Shift+D; **Dictate to clipboard** from the tray (while **Show in menu bar** is on) or the chosen shortcut, while a Biorouter window is open, minimized included, because closing the last window quits the app on Windows [C `ui/desktop/src/main.ts` `window-all-closed`] |
| Permissions | The Windows privacy switch, not a prompt |
| Messages | MSG-MIC-DENIED-WIN with **Open settings**, MSG-NO-DEVICE, MSG-MIC-BUSY, MSG-WINDOW-OPEN under the clipboard switch, MSG-CPU and MSG-CPU-DESKTOP, MSG-RESTART-BUSY |
| Release | PR-19 (clipboard PR-28) |
| Acceptance | The `windows-runtime` job of `release-artifact-smoke.yml`, the native Windows publish gate, runs the self test; M-1 (a native Windows run of the static mingw worker with a real model) recorded; checklist section S3 passes on the zip and on the Squirrel installer, and row S3.2 records the status the privacy switch produces |

The desktop clipboard path cannot mark dictated text as excluded from the Windows cloud clipboard. Electron 39's `clipboard.write()` takes text, bookmark, RTF, HTML and image and has no privacy flag, and `clipboard.writeBuffer()` commits one custom format on its own, emptying the clipboard first, so the exclusion format cannot sit beside the text [R Electron 39.8.10 `electron_api_clipboard.cc`; I for the conclusion]. MSG-CLIPBOARD-EXPLAIN says that clipboard text may sync; `biorouter dictate --copy` does exclude it (S18).

#### S4 desktop app, Linux deb and rpm

| Part | Design |
|---|---|
| Capture | As S1; Chromium reaches PulseAudio or PipeWire, with no OS prompt |
| Transcription | `biorouterd` worker on the CPU, static libstdc++, no symbol above `GLIBC_MAX` (2.31; the spike measured 2.30) |
| Models | `~/.local/share/biorouter/dictation/models/<id>/`; an administrator's `/var/lib/biorouter/dictation/models` is found too |
| Entry points | As S1 with Ctrl+Shift+D. On GNOME Wayland, Electron 39 cannot register a global shortcut (Chromium excludes GNOME from the GlobalShortcuts portal [R]), so the picker shows MSG-GNOME and the tray item is the way in; GNOME shows the tray only with the AppIndicator extension. **Dictate to clipboard** works only while a window is open |
| Permissions | None to grant; a busy or missing device is the only failure |
| Messages | MSG-NO-DEVICE, MSG-MIC-BUSY, MSG-GNOME, MSG-WINDOW-OPEN, MSG-CPU and MSG-CPU-DESKTOP |
| Release | PR-19 (clipboard PR-28; the GNOME Wayland shortcut waits for Electron 43 [R]) |
| Acceptance | `smoke_deb` and `smoke_rpm` in `scripts/smoke-test-release-artifacts.sh` run the self test; M-7 (PipeWire and PulseAudio) and M-12 (hidden window capture with PipeWire, Wayland clipboard focus, KDE portal shortcut) recorded; checklist section S4 passes |

### `biorouter serve` (S5 to S7)

`serve` runs the same renderer, built from the same source by `build:web`, in a browser, so the composer, Settings and the Dictionary page work as on the desktop, with these differences. PR-21 opens their browser routes and controls while keeping the host's model folder reveal and desktop shortcut hidden.

**Headers (PR-03, before any served microphone).** App pages share the shell's origin (SD-14), and a browser grants the microphone per origin, so agent written app code would otherwise inherit the person's grant without a prompt. No response sends a `Permissions-Policy` header at `main` [C, searched]; PR-03 adds one at each function below, and `APP_PERMISSIONS_POLICY` is a new constant in `routes/apps.rs`.

| Response | Header PR-03 adds | Where it goes (existing functions at `main`) |
|---|---|---|
| The shell | `Permissions-Policy: microphone=(self)` | `crates/biorouter-server/src/routes/web_ui.rs` `index` (`:346`) |
| App pages | `Permissions-Policy: microphone=(), camera=(), display-capture=()` (`APP_PERMISSIONS_POLICY`) | `routes/apps.rs` `app_page_response` (`:183`) |
| App files under `dist/` and `assets/` | the same | `with_app_file_csp` (`:154`) |
| Figure frames | the same | `figure_frame` (`:5608`) |
| The launch bounce | the same | `launch_bounce` (`:429`) |

Chromium enforces the header; Firefox and Safari ignore it [R]. That residual is recorded in SD-17, and `docs/deployment/browser-access.md` tells Firefox and Safari users to allow the microphone for one visit only.

**The banner (PR-21).** `print_banner` in `crates/biorouter-cli/src/commands/serve.rs` gains one readiness line:

```text
  Dictation: ready (whisper-small.en-q8_0)
  Dictation: no speech model; run `biorouter dictation install`
  Dictation: unavailable on this processor
```

For a non loopback bind, `print_banner` prints the block "From another machine on this network:" with a plain HTTP LAN address that the shell refuses [C]. PR-21 replaces that block by the case:

| Bind and flags | What the banner prints in place of the LAN block |
|---|---|
| Non loopback, no `--public-url`, not in a container | MSG-SERVE-TUNNEL, with the port, the token and this machine's name filled in |
| Non loopback, no `--public-url`, in a container (`/.dockerenv`, `/run/.containerenv` or `KUBERNETES_SERVICE_HOST`) | MSG-SERVE-CONTAINER, with the container port and the token filled in |
| Non loopback with `--public-url` (a proxy on another machine or pod, such as a Kubernetes ingress or a JupyterHub single user pod) | nothing: the existing proxied lines already name the address to open |

The banner knows the port and the token, so both messages name the full address to open, `?t=` part included (without it when `serve` runs with `--no-token`). The shell answers an address without its token with a 401, "This link needs its access token" [C `web_ui.rs` `unauthorized`], so no message in this document names a bare `http://127.0.0.1:8765`.

The readiness line checks managed live-capture policy before model inventory. When capture is denied, it gives the managed capture reason even if a model is installed. When no model is usable and managed policy forbids downloads, it gives the administrator and allowed-folder reason instead of an install command.

**The install lock.** On the keyless `serve` daemon, a browser may install a pinned model with the secret (DD-20, O-2). Three switches turn that off, composing as the most restrictive: `biorouter serve --no-dictation-install` (which sets `BIOROUTER_DICTATION_BROWSER_INSTALL=false` in the environment of the daemon it starts), the env variable itself, and managed policy. When locked, `status.install` is `{allowed: false, reason}` with MSG-INSTALL-LOCKED, and the microphone and the Settings rows show that sentence before any click.

**Standing.** A `serve` browser holds no proof; `userActionHeaders()` states the host's configured provider in `X-Caller-Provider` [C]. On a host configured with a private model that gives private standing, so dictation into a private chat there uses private entries; on a host configured with a public model it uses public entries only. The Dictionary page on `serve` disables publicize and privatize before the click with MSG-DICTIONARY-HOST-TIER; raising `agent_writes` and turning chat learning or suggestions on with MSG-DICTIONARY-HOST-SETTINGS, which names `biorouter dictionary settings` (DD-75); and, without private standing, **Only in private chats** with MSG-DICTIONARY-HOST-PRIVATE ([dictionary](dictionary.md) has the full rules). The settings response carries separate `write` and `delete` allowances: managed `dictionary: false` forbids adds and edits but still permits delete, dismiss and export for a caller the corresponding route admits. A browser missing the delete allowance leaves row delete, dismiss and bulk delete unavailable.

**The shared origin notice.** MSG-SHARED-ORIGIN shows once per origin before the first recording, on every `serve` origin and in every browser. The composer microphone and Settings' Test microphone each wait for its acknowledgement; the acknowledgement click starts capture while it still has browser user activation. `startDictation` also refuses a direct browser start until the origin was acknowledged. The viewer records the acknowledgement for that origin in local storage, with an in-memory fallback when storage is blocked. App pages share the shell's origin (SD-14), and PR-03's `Permissions-Policy` header keeps an app page itself and the frames it holds away from the microphone in Chromium, but not a persistent grant: a same origin popup that an app page opens (`window.open('about:blank')`, or any same origin document served without the header) records in Chromium 157, measured on 2026-11-05 [M], and Firefox and Safari do not enforce the header at all. A path prefix, such as JupyterHub's `/user/<name>/biorouter/`, widens the grant further to every page on the hub. So the notice always recommends a one time grant, and per app origins remain the lasting fix (SD-17).

**The chord.** Browsers bind ⌘⇧D to their own commands (Chrome and Firefox bookmark all tabs; Safari adds to the Reading List [R]), and M-18 measures whether a page can take it. Until M-18 shows the page receives the chord, a `serve` browser's tooltip reads "Dictate" without it, Settings hides the **Shortcut** row, and the field's key handler does not claim the chord.

**Copilot.** Biorouter Copilot places no restriction on dictation, decided by the owner on 2026-11-05 (O-15, DD-88): an agent holding a Copilot grant on Biorouter's own window, or on a browser that shows a `serve` tab, can start the microphone by clicking it or pressing the dictation chord, and can then press **Send** as it can for typed text. What limits it is Copilot's per task grant (one grant per task, chat, model and target), the composer's visible recording state, the operating system's microphone indicator, and Stop and revoke. On the browser, the origin's existing microphone grant opens the microphone with no prompt. This residual is recorded in SD-17 and P-7; it concerns S5 on the person's own machine and S6 when the browser runs where Copilot does.

**Phones and tablets.** An HTTPS `serve` is reachable from a phone or tablet, and the composer then shows a microphone there. iOS Safari and Android Chrome (the 16 kHz context, the worklet, the gesture rules, the narrow layout) are untested [I]; checklist row S6.6 and M-8 cover them before any document claims they work.

**Several people on one host.** One `serve` daemon serves one operator; everyone who opens its address is the same user [C `docs/deployment/browser-access.md`]. Hubs that start one daemon per user load one worker per active user, 0.6 to 0.9 GB each, freed after 300 s idle [I]. whisper.cpp has no memory mapped load path, so disk can be shared through `--system` or managed `models_dir` but RAM cannot [R]. Each daemon allows `max_streams = 8` and 32 queued segments, then answers 429 with `Retry-After` (MSG-SERVER-BUSY), and checks `MemAvailable` and cgroup `memory.max` before a load (MSG-SERVER-MEMORY).

**Proxy guidance**, added to `docs/deployment/browser-access.md` in PR-21:

| Setting | Why |
|---|---|
| Request body limit of at least 512 KiB plus headers (nginx's 1 MiB default is enough) | chunks are at most 524,288 B; nginx answered a readable 413 for 1 MiB plus 2 B in every browser engine [M] |
| HTTP/2 between browser and proxy | removes the six connection stall when more than one tab is open [M] |
| `client_body_buffer_size 512k;` (or `proxy_request_buffering off;`) on the Biorouter location, required | keeps each audio chunk in nginx's memory; with the stock 16 KiB buffer nginx wrote every chunk above 16 KiB to a temporary file on disk and logged `a client request body is buffered to a temporary file`, 154 times in one session [M]. Every dictation chunk is 64 to 512 KiB, and the documented deployment puts nginx on the transcribing host [C `docs/deployment/headless-linux.md`] |
| `c.JupyterHub.subdomain_host` | gives each user's server its own origin, so a microphone grant stops covering the whole hub |
| Pass the browser's `Sec-Fetch-*` headers | already required by SD-15; dictation adds nothing |

M-17 runs the upload ladder through CHP, `jupyter-server-proxy`, `ingress-nginx`, Caddy and Traefik before the proxy section names them, and records for each the setting that keeps request bodies off its disk.

**SD-17**, a new record in `docs/deployment/serve-decisions.md` (PR-03 starts it, PR-21 completes it): dictation on `serve` needs no proof; private bias standing comes from the proof or a stated `X-Caller-Provider`, never from the operator's tier that SD-10 gives listings; model install is admitted unless the operator locks it; app pages deny the microphone; the Firefox and Safari residual; the residual that any holder of a keyless daemon's secret, an agent shell that recovers it included, can install a pinned model and make a browser's dictionary writes; the Copilot residual (DD-88, O-15); the ACP harvesting residual.

#### S5 `serve`, browser on the same machine

| Part | Design |
|---|---|
| Capture | The person's browser at the address `serve` prints, `http://127.0.0.1:8765/?t=<token>` (`DEFAULT_PORT` in `commands/serve.rs` [C]). `http://127.0.0.1` and `http://localhost` are secure contexts with `getUserMedia` and `audioWorklet` in Chromium 145, Firefox 146 and WebKit 26 [M]; the browser prompts once per origin |
| Transport | The chunked stream, same origin |
| Transcription | The host's `biorouterd` worker |
| Models | The account that runs `serve`; the browser's Settings installs there unless locked, or `biorouter dictation install` on the host |
| Bias | Public; private only for a private target with private standing (a private host's browser states `X-Caller-Provider`); an identical response shape either way |
| Entry points | The served composer's microphone; the served **Dictionary** page; Settings |
| Messages | MSG-MIC-BROWSER, MSG-INSTALL-LOCKED, MSG-DISCLOSE-HOST (once per host), MSG-SHARED-ORIGIN (once per origin), MSG-CPU-SERVER, MSG-NOTHING, MSG-WAIT, MSG-ENGINE-FAILED |
| Residuals | Copilot holding a grant on the browser that shows this page can click the microphone, with no refusal (DD-88, O-15; recorded in SD-17) |
| Release | PR-21 |
| Acceptance | The `rust.yml` `serve` job: Playwright Chromium (16 kHz) and Firefox (the native rate fallback) against `serve` on loopback with the debug test model show MSG-NOTHING, and the daemon received 32,000 B per second of audio; M-8 (Safari at `127.0.0.1`, including the first acknowledged click before the status store has loaded) recorded. The browser does not claim ⌘⇧D. |

#### S6 `serve` behind an HTTPS reverse proxy, including a JupyterHub path prefix

| Part | Design |
|---|---|
| Capture | The browser over HTTPS, a secure context; the shell admits the address through Fetch Metadata [C `crates/biorouter-server/src/auth.rs` `reached_over_a_secure_context`]. The worklet resolves under a stripped `/user/a/biorouter/` prefix and under a deep fall through path because it is a real asset file referenced relative to its chunk [M] |
| Transport | The chunked stream through the proxy; at most 524,288 B per `PUT` |
| Transcription | The per user `biorouterd` worker |
| Models | Per account, or `sudo biorouter dictation install --system`, or managed `models_dir` on hubs with one daemon per user |
| Bias | As S5 |
| Entry points | As S5 |
| Messages | MSG-SHARED-ORIGIN (once per origin), MSG-413, MSG-WAIT, MSG-SERVER-BUSY, MSG-SERVER-MEMORY, MSG-DISCLOSE-HOST, MSG-ENGINE-FAILED |
| Phones and tablets | Untested; checklist row S6.6 (iPhone Safari and Android Chrome: dictate once, deny once) before any claim |
| Release | PR-21 |
| Acceptance | The standalone nginx driver under `/user/a/biorouter/`: the worklet resolves, nginx itself rejects a 1 MiB plus 2 B `PUT` with 413, the production transport halves after a 413, and with `client_body_buffer_size 512k;` the nginx log holds no `buffered to a temporary file` line. The `serve` job checks that the worklet is an `assets/*.js` file referenced by an entry chunk. M-5 (two tabs while a turn streams, then behind HTTP/2 nginx) and M-17 remain separate checks. |

#### S7 `serve` at a plain HTTP network address

No microphone is possible here: browsers expose none on an insecure origin [M], and the shell is refused before any composer renders, with the existing 403 from `insecure_address` in `crates/biorouter-server/src/routes/web_ui.rs` [C], unchanged. DD-79 adds no new refusal.

| Part | Design |
|---|---|
| Capture | None in the browser |
| Working paths | An SSH tunnel turns this into S5: `ssh -N -L 8765:127.0.0.1:8765 <host>`, then open the address `serve` printed with 127.0.0.1 as its host, keeping its `?t=` part (MSG-SERVE-TUNNEL prints it whole); `biorouter transcribe <file>` on the host handles recordings |
| Messages | The existing 403: `Open Biorouter through localhost or HTTPS. Use an SSH tunnel, or an HTTPS reverse proxy that passes the browser's Sec-Fetch-* headers through, and a current browser that sends them.` Behind a proxy that rewrites `Host` to loopback, the shell renders but the page is insecure, and the composer shows MSG-INSECURE before any click. The banner prints MSG-SERVE-TUNNEL for a non loopback bind without `--public-url` |
| Docs | `docs/deployment/headless-linux.md` stops handing users a plain HTTP LAN address (`http://<host>:8765/?t=…` today [C]) and gives the tunnel and the HTTPS proxy instead |
| Release | PR-21 (banner and docs; the alternative) |
| Acceptance | Banner snapshots for a loopback bind, a `0.0.0.0` bind, a `0.0.0.0` bind inside a container, and `--host 0.0.0.0 --public-url https://hub.example/user/a/biorouter/` (no tunnel advice); checklist section S7 confirms the tunnel path end to end with the tokened address |

### S8 CLI only Linux packages on a server

The `biorouter-cli` deb and rpm need no new package entry, `Depends` or helper: the engine is inside `/usr/bin/biorouter` and `/usr/bin/biorouterd`, and the dictation interface is inside the web bundle at `/usr/share/biorouter/web` [C `packaging/biorouter-cli.yaml`].

| Part | Design |
|---|---|
| Capture | None on the server. The person's browser through `serve` (S5 with a tunnel, or S6 behind a proxy) |
| Transcription | `/usr/bin/biorouterd __br-dictation-worker engine` for the served browser; `/usr/bin/biorouter transcribe` for files |
| Models | The service account's folder through `sudo -u biorouter -H biorouter dictation install`, or `sudo biorouter dictation install --system` into `/var/lib/biorouter/dictation/models` |
| Bias | As S5 in the browser; `transcribe` public only unless DD-77 admits the session |
| Entry points | The served microphone; `biorouter transcribe <file>` |
| Messages | `biorouter dictate` on the server over SSH: MSG-LINUX-NOAUDIO, then MSG-SSH-SERVICE when a `biorouter serve` service runs there, else MSG-SSH-ROUTES; at the local console or in any other terminal on the host outside SSH: MSG-LINUX-NOAUDIO, then MSG-LINUX-ROUTES; all before any model prompt; installing as the wrong account: MSG-OTHER-ACCOUNT; MSG-CPU-SERVER, then MSG-CPU-VM when the CPUID hypervisor bit is set |
| Release | PR-11 (install), PR-12 (`transcribe`), PR-21 (served microphone) |
| Acceptance | In both package smoke containers (`debian:bookworm-slim`, `rockylinux:9`), the self test prints `"selftest":"ok"` and `biorouter dictation status --format json` reports no installed model and `capture.available=false`, with `reason="not_compiled"` from PR-10 and `reason="no_audio_system"` from PR-25, which brings capture and its audio system probe; `strings biorouterd \| grep -c libpulse` is 0 (P-10) |

The service account trap: the systemd unit runs as `User=biorouter` [C `docs/deployment/headless-linux.md`], so a model an operator installs as themself lands where the served browser never looks. Every host side `dictation` and `dictionary` command prints the account and folder it acts on. `status` and `install` print MSG-OTHER-ACCOUNT when a `biorouterd` owned by another account is running, found by a scan of `/proc` [I: a host that mounts `/proc` with `hidepid` hides other accounts' processes]; the message names that process's owner, never a fixed account name. The same scan gives `dictate` over SSH its MSG-SSH-SERVICE variant: when a `biorouter serve` process runs there, its port comes from its command line (8765 when it names none), and the person tunnels to it rather than starting a second `serve`, which would collide on the port (`preflight_port` [C `serve.rs`]) and run as the wrong account. The service's token sits in the root only `/etc/biorouter/env` [C `docs/deployment/headless-linux.md`], so the message sends the person to whoever runs the service for it. `docs/deployment/headless-linux.md` names the service account in every command, including `sudo -u biorouter -H biorouter dictionary review`.

### The terminal (S9 to S11)

The CLI transcribes in its own process tree on every front end, including the shared daemon prompt and `session attach`; only text reaches a daemon (DD-72). Capture runs in a child, `biorouter __br-dictation-worker capture`, so a driver crash or ALSA's stderr never reaches the TUI (DD-73).

**Commands** (the full tree, including `biorouter dictation …` and `biorouter dictionary …`, is in [engine and models](engine-and-models.md) and [dictionary](dictionary.md); the commands that have shipped are in the [command reference](../cli/command-reference.md) as they behave today):

```text
biorouter dictate     [--copy] [--no-history] [--format text|json] [--seconds N] [--session ID]
                      [--private-terms] [--language L] [--model M] [--no-dictionary] [--no-review] [--yes]
biorouter transcribe  FILE|-  [--format text|json|srt|vtt] [--output FILE] [--session ID]
                      [--context-file PATH] [--language L] [--model M] [--no-dictionary]
biorouter dictation devices  [--format text|json]
biorouter dictation test     [--seconds N]
```

`--seconds N` is a cap of at most 600, never a way around the terminal check. `--yes` on `dictate` skips the review, like `--no-review`. `dictate --file PATH` is a hidden alias of `transcribe PATH`.

**Detection order**, in `crates/biorouter-cli/src/dictation/environment.rs`, run before any device opens. The heuristics choose the wording; apart from SSH on macOS, they never block a device that works. A host that can never capture live answers with its routes before any model prompt, so nobody downloads a 264 to 574 MB model only to be refused on the next run.

| Step | Check | Message | Exit |
|---|---|---|---|
| 1 | Built without the `dictation` feature | MSG-NO-ENGINE | 2 |
| 2 | Managed policy: `enabled: false`; or `capture: false` for live capture | MSG-POLICY or MSG-POLICY-CAPTURE | 2 |
| 3 | `BIOROUTER_TERMINAL=1` (every agent shell sets it [C `crates/biorouter-mcp/src/developer/shell.rs`]) | MSG-AGENT-SHELL | 2 |
| 4 | No terminal: `prompt_can_run()` is false [C `crates/biorouter-cli/src/commands/needs_terminal.rs:49`] | MSG-NO-TTY | 2 |
| 5 | macOS over SSH (`SSH_CONNECTION`, `SSH_CLIENT`, `SSH_TTY`) | MSG-SSH-MAC, then MSG-SSH-ROUTES | 2 |
| 6 | CPU gate | MSG-CPU with the missing features; under Rosetta on macOS 12 to 14 (any version before 15), MSG-CPU-ROSETTA | 2 |
| 7 | The capture child, asked only to list its backends and input devices without opening one, finds no backend or no device | the wording comes from the environment, in this order: over SSH (variables, or an `sshd` or `sshd-session` ancestor, because tmux keeps stale variables): the cause line (MSG-LINUX-NOAUDIO on Linux), then MSG-SSH-SERVICE when a `biorouter serve` process runs on this host, else MSG-SSH-ROUTES, then MSG-SYNC-HINT; a container (`/.dockerenv`, `/run/.containerenv`, `KUBERNETES_SERVICE_HOST`, or a container runtime named in `/proc/1/cgroup`): MSG-CONTAINER; WSL1: MSG-WSL1; WSL2 without WSLg (no `/mnt/wslg/PulseServer`): MSG-WSL2-NOWSLG; WSL2 with WSLg but without `libpulse.so.0`: MSG-WSL2-NOPULSE; other Linux: MSG-LINUX-NOAUDIO, then MSG-LINUX-ROUTES; otherwise MSG-NO-DEVICE | 2 |
| 8 | No installed model | MSG-NO-MODEL-CLI | 2 |
| 9 | The chosen device fails to open | MSG-NO-DEVICE when it is gone, MSG-MIC-BUSY otherwise | 2 |
| 10 | The device opened, but every sample is exactly zero for 1.5 s | macOS MSG-TCC-ZERO; Windows MSG-WIN-ZERO; Linux MSG-SILENT | 3 |

Listing devices opens no input stream, so step 7 shows no permission prompt [I]; it runs in the capture child, behind `CaptureConsent`, after steps 3 and 4.

```rust
// crates/biorouter-cli/src/dictation/environment.rs
pub enum CaptureVerdict {
    NotCompiled,
    PolicyOff { capture_only: bool },
    AgentShell,
    NoTerminal,
    MacOverSsh,
    CpuUnsupported { rosetta: bool },
    NoCapture(NoCaptureReason),   // step 7, checked before NoModel
    NoModel,
    Ready,
}
pub enum NoCaptureReason {
    Ssh { service_port: Option<u16> },  // Some: a `biorouter serve` runs here (MSG-SSH-SERVICE)
    Container, Wsl1, Wsl2NoWslg, Wsl2NoPulse, LinuxNoAudio, NoDevice,
}
pub fn detect(env: &Environment, policy: &DictationPolicy, status: &ServiceStatus) -> CaptureVerdict;

// crates/biorouter-cli/src/dictation/consent.rs: the only file that constructs it,
// pinned by the test capture_consent_is_minted_in_one_file
pub struct CaptureConsent;
impl CaptureConsent {
    pub fn at_this_terminal() -> Result<Self, CaptureVerdict> {
        if std::env::var_os("BIOROUTER_TERMINAL").is_some_and(|v| v == "1") {
            return Err(CaptureVerdict::AgentShell);
        }
        if !crate::commands::needs_terminal::prompt_can_run() {
            return Err(CaptureVerdict::NoTerminal);
        }
        Ok(CaptureConsent)
    }
}
```

**The capture child.** Started only with a `CaptureConsent`. Control frames `ready{backend, device, rate}`, `level{peak, rms}` every 100 ms, `error{code}` and `stopped{frames}`; audio frames `[u32 len][f32 LE]`, 16 kHz mono, a 100 ms period. Stdin end of file makes it exit within 2 s; the parent keeps the last 4 KiB of its stderr. miniaudio is compiled with only CoreAudio, WASAPI, PulseAudio and ALSA (the Null backend, which silently records silence, is compiled out [M]), and on Linux the sound libraries are loaded at run time, so `DT_NEEDED` stays inside `BASE_LIBS`. `BIOROUTER_DICTATION_RECORDER` (env only, Linux) names a command that writes raw s16le 16 kHz mono to stdout, as the escape hatch for unusual audio setups; it still runs only behind `CaptureConsent`. `biorouter dictation devices` lists input devices, and `BIOROUTER_CLI_DICTATION_DEVICE` chooses one.

**Front ends** (DD-74, PR-26):

| Front end | Start | Stop and insert | Cancel | Where the text goes |
|---|---|---|---|---|
| TUI (`session/tui/{mod.rs,app.rs}`) | Ctrl+T or `/dictate`, intercepted before the streaming queue | Enter or Ctrl+T again | Escape or Ctrl+C | `App::insert_transcript(text, anchor)` at the cursor remembered at start (or the current cursor if the input changed), one leading space when the previous character is not whitespace, no trailing space, never through `paste` |
| Classic REPL (`session/input.rs`) | Ctrl+T (a `DictateHandler: ConditionalEventHandler` returns `Cmd::Interrupt`, overriding rustyline's transpose) or `/dictate` | Enter or Ctrl+T | Escape or Ctrl+C | `readline_with_initial(prompt, (left + text, right))` reopens the line for editing; recording draws one `\r` status line on stderr |
| Shared daemon prompt (`commands/shared_conversation.rs`) | Ctrl+T or `/dictate` inside `terminal_input`, which PR-26 gives a new `initial` argument (at `main` it is `terminal_input(prompt, hidden, limit)` [C]) | Enter or Ctrl+T | Escape or Ctrl+C | the prompt reopens prefilled through `initial` |
| `session attach` (`commands/session_watch.rs`) | the exact line `/dictate` | the next Enter (attach reads stdin line by line [C `session_watch.rs`]) | Ctrl+C | printed, then `Send? [y/N]`, default No, because a send steers a live turn; `/dictate` itself is never posted |

In every front end Enter sends only after the text is inserted and the person presses it again; text dictated while the agent streams goes into the composer and Enter queues it. While recording or transcribing, Ctrl+C cancels the dictation only, never the turn or the session; other keys are ignored and the hint says how to stop. `BIOROUTER_CLI_DICTATION_KEY` (default `t`, meaning Ctrl+T; empty disables) clashing with `BIOROUTER_CLI_NEWLINE_KEY` disables dictation's key with one startup note, MSG-KEY-CLASH. In the TUI, Ctrl+B, Ctrl+C, Ctrl+D and Ctrl+O already belong to the subagent pane, stopping and quitting, so a dictation key set to one of those letters is not offered there and the hints leave it out. While a recording runs its keys come first, ahead of the subagent pane; the chord starts a recording from the focused pane as typing does, and a subagent's chat opened from the pane is read only, so nothing is dictated into it. Slash commands: `/dictate`, `/dictate status`, `/dictate install [MODEL]`, `/dictate retry`, `/dictionary add`, `/dictionary review`. In the TUI and the classic REPL a private `/dictionary add` carries the session's institutions, read in process, as every term the person adds inside a private chat does, the context menu door included, while a term added with no chat carries none (O-36; [private rows and institutions](dictionary.md#private-rows-and-institutions), [in a session](dictionary.md#in-a-session)). Hold to talk waits for a later release (see [what waits](#what-waits-for-a-later-release)).

**TUI strings** (in `crates/biorouter-cli/src/dictation/messages.rs`, pinned by snapshot tests):

```text
◌ Starting microphone…                                  until the first frame with a peak above zero
● REC 0:07 ▂▅▇▅▂  Enter insert · Esc cancel             5 cells of ▁▂▃▄▅▆▇█ on a 100 ms clock
◌ Transcribing 0:07…                                    with the existing spinner
^T dictate                                              in the hints, only when compiled in, allowed and installed
3 dictionary suggestions; /dictionary review            start line, when suggestions wait
```

The terminal title gets a `●` prefix and a space while the device is open and is restored on stop. Under `NO_COLOR` the colour goes and `REC` and the timer stay.

**Limits.** 600 s per recording, with MSG-CAP-WARN at 570 s and MSG-CAP-STOPPED at the stop; under 0.5 s gives MSG-TOO-SHORT; exact zeros for 1.5 s give the permission message of step 10; below the noise floor for 3 s shows MSG-NO-AUDIO-YET and keeps recording; three failures within 10 s pause dictation for the session with MSG-PAUSED. After a failed transcription the front ends print MSG-ENGINE-FAILED-CLI, and the audio stays in memory for `/dictate retry` for at most 5 minutes, and is dropped on the next recording, on cancel or on quit; it is never written to disk.

**`dictate` reviews by default** (DD-76): the transcript is shown on stderr; Enter uses it, `e` edits it (`readline_with_initial` on the terminal), Escape discards it (exit 4). Only the final text or JSON goes to stdout, so `"$(biorouter dictate)"` and pipes are reviewed on the terminal before anything leaves.

**Exit codes** for `dictate` and `transcribe`: 0 ok; 1 generic failure, including a failed transcription (the first sentence of MSG-ENGINE-FAILED-CLI on stderr); 2 declined to start (no terminal, agent shell, policy off, no microphone here, not compiled, unsupported processor, no model, `--private-terms` not admitted); 3 nothing heard (no speech, an echo only, too short, or exact zeros); 4 cancelled by the person; 5 `--copy` could not reach a clipboard and printed the text instead.

**Which entries bias a CLI transcription** (DD-77):

| CLI path | Private entries may bias when |
|---|---|
| TUI, classic REPL | the session is private capable |
| Shared daemon prompt, `session attach` | the chat's stored tier is private and the run is at an interactive terminal; a chat id the local store does not hold (a daemon whose store is elsewhere, as in the external backend setup of [workspace control](../agent-loop/workspace-control.md)), or whose tier cannot be read, gets public entries only |
| `transcribe` without `--session` | never: public only |
| `dictate` to stdout, `dictate --copy`, without `--session` | only with `--private-terms`, at an interactive terminal (`prompt_can_run()`), with `BIOROUTER_TERMINAL` unset and `caller_capability()` returning Private, after MSG-PRIVATE-TERMS and a `y` (O-8) |
| `--session <id>` | the chat is private **and** `prompt_can_run()` **and** `caller_capability()` returns `ProviderTier::Private` [C `crates/biorouter-cli/src/session/privacy.rs:65`; it returns Public when nothing is configured or the provider is unknown] **and** `BIOROUTER_TERMINAL` is unset |
| An agent's shell (`BIOROUTER_TERMINAL=1`) | never, whatever the flags |
| The privacy master switch off | every entry |

Context passed for ranking: the text before the cursor (at most 1,000 characters) in the TUI and REPL; `--context-file` (at most 4 KB) for `transcribe`.

**macOS permission for the CLI.** Capture runs in the CLI's own child, so the terminal app is the responsible process and its name is in the prompt. v1 makes no AVFoundation call and embeds no `__info_plist`; a denial is detected by the zero rule of step 10, whose sentence names the terminal app from `__CFBundleIdentifier` (or `TERM_PROGRAM`). `biorouter dictation test` is the first run path, so the prompt appears when the person chose to test. In the desktop app's own terminal dock, the Biorouter app is presumably the responsible process [I]; checklist row S9.7 records which app the prompt names there; the desktop sets no `BIOROUTER_TERMINAL` [C, no hit under `ui/desktop/src`], so a person there is never mistaken for an agent. `Contents/Resources/bin/biorouter` carries the pinned `audio-input` entitlement.

#### S9 interactive CLI on a local machine

| Part | Design |
|---|---|
| Capture | The capture child |
| Transport | Frames on the child's stdout into the parent's memory |
| Transcription | The `biorouter` process's own worker, `biorouter __br-dictation-worker engine`; Metal on Apple Silicon |
| Models | The same per user folder as the desktop app; `biorouter dictation install` or `/dictate install` |
| Bias | TUI and REPL: the session's tier (a person at the terminal has standing); shared prompt and attach: the chat's stored tier, read offline, and public only when the local store does not hold the chat |
| Entry points | Ctrl+T and `/dictate` in all four front ends; `biorouter dictate` |
| Permissions | macOS: the terminal app's TCC grant; Windows: the desktop apps switch; Linux: none |
| Messages | MSG-TCC-ZERO, MSG-WIN-ZERO, MSG-SILENT, MSG-NO-MODEL-CLI, MSG-TOO-SHORT, MSG-NO-AUDIO-YET, MSG-NO-DEVICE, MSG-MIC-BUSY, MSG-ENGINE-FAILED-CLI, MSG-CAP-WARN, MSG-CAP-STOPPED, MSG-PAUSED, MSG-KEY-CLASH |
| Release | PR-25 (`dictate`, capture, environment sentences), PR-26 (front ends) |
| Acceptance | On Linux `readelf -d biorouter` shows no `libasound` or `libpulse`; real microphone rows (macOS Terminal and iTerm2, Windows 11, Ubuntu 24.04 with PipeWire) deliver frames within 2 % of wall time at a 100 ms period and the first frame within 300 ms (M-7); checklist rows for Terminal, iTerm2, VS Code's terminal, tmux and Windows Terminal, a terminal whose `Info.plist` lacks `NSMicrophoneUsageDescription` (row S9.3), and the desktop app's terminal dock (row S9.7) |

#### S10 CLI over SSH or in a container with no microphone

The remote host has no microphone the person can reach, so live capture is refused before any device opens, with three routes that work:

1. Run `biorouter serve` on the remote host and keep it running, then on the laptop run `ssh -N -L 8765:127.0.0.1:8765 <host>` and open the address `serve` printed, `http://127.0.0.1:8765/?t=<token>`, keeping its `?t=` part. This is S5 on the laptop's browser, with the remote host's model and dictionary. When a `biorouter serve` service already runs there (S8), tunnel to its port instead, with MSG-SSH-SERVICE. VS Code and Cursor Remote may forward the port without the `ssh` command [I]; checklist row S10.5 records whether they do, and until it passes no message says so.
2. Dictate on the laptop with `biorouter dictate --copy` (or the desktop's **Dictate to clipboard**) and paste into the remote TUI, which accepts bracketed paste [C `session/tui/mod.rs`].
3. Copy a recording to the host and run `biorouter transcribe <file>`.

Two machines have two dictionaries. `biorouter dictionary export - | ssh <host> biorouter dictionary import -` copies the laptop's terms (MSG-SYNC-HINT).

| Part | Design |
|---|---|
| Capture | None on the remote host |
| Transcription | The remote host's worker (`serve`, `transcribe`), or the laptop's for the clipboard route |
| Models | The remote host's account, or the laptop's |
| Bias | The remote dictionary as S5; the clipboard routes public unless the person includes private terms for one recording (O-8) |
| Messages | MSG-SSH-ROUTES or MSG-SSH-SERVICE, MSG-SSH-MAC then the routes, MSG-CONTAINER, MSG-SYNC-HINT; all before any model prompt |
| Release | PR-25 (messages), PR-21 (the served route) |
| Acceptance | The environment table tests (SSH variables, a stale tmux variable, `/.dockerenv`, `KUBERNETES_SERVICE_HOST`, and no model plus SSH giving MSG-SSH-ROUTES, exit 2 and no install prompt); checklist section S10 confirms the tunnel route from a laptop with the tokened address |

#### S11 noninteractive CLI

`run`, `session send`, workflows, scripts and `crew send` have no person at a terminal, so live capture is refused (MSG-NO-TTY, or MSG-AGENT-SHELL in an agent's shell, both exit 2). O-20 keeps it that way: a capture with no terminal is what an agent's tool call looks like, and hotkey wrappers use the desktop's tray and global shortcut instead. `biorouter transcribe FILE|-` works anywhere: cron, an agent's shell, scripts, Docker.

```bash
biorouter transcribe memo.mp3 --format json            # public only; schema 1
biorouter run -t "$(biorouter dictate)"                 # reviewed on the terminal first
biorouter session send <id> "$(biorouter dictate)"      # stdin belongs to --user-action-key-stdin
biorouter dictate | biorouter crew send <channel> --input -
biorouter crew ask <channel> "$(biorouter dictate)"   # reviewed before it starts a task
biorouter dictation install --yes whisper-small.en-q8_0 # provisioning; refused in an agent's shell
```

| Part | Design |
|---|---|
| Capture | None |
| Transcription | `biorouter transcribe` in its own worker |
| Models | The account's folder; `biorouter dictation install --yes <id>`, refused when `BIOROUTER_TERMINAL=1` with MSG-INSTALL-SHELL |
| Bias | `transcribe`: public only, and `--session <id>` admits private entries only under DD-77; `dictate`: public unless a person at the terminal passes `--private-terms` (O-8); an agent's shell is public whatever the flags |
| Messages | MSG-AGENT-SHELL, MSG-NO-TTY; `transcribe`: MSG-TRANSCRIBE-NOTHING to MSG-TRANSCRIBE-FAILED, MSG-TOO-SHORT, MSG-LANGUAGE-UNKNOWN, MSG-NO-MODEL, MSG-UNKNOWN-MODEL, MSG-NOT-INSTALLED and MSG-DETECTOR-MISSING, and its JSON errors carry `error_code` |
| As built (PR-12 to PR-14b) | Exit codes: 0 text written; 1 the input or the output failed (`unsupported_format`, `too_long`, `not_found`, `unreadable`, `unwritable`) or the engine failed (`engine_failed`); 2 refused before anything was attempted (no engine, the processor, policy, no model, an unknown or uninstalled model, a language the model cannot take); 3 nothing heard (`no_speech`, `too_short`); a recording under half a second or of exact zeros is answered before a model is needed, and `no_speech` also comes after the decode when the model hears nothing; 4 stopped with Ctrl-C (`cancelled`). `--language auto --format json` reports `"language": "auto"`, not the language the model detected, because the worker's result carries none. Each one shot `transcribe` loads its own model, so running many at once is slower than running them in turn: 8 at once took 16 s together, about 2 s each, against about 2.6 s for the eight run in turn at 0.33 s each (`small.en`, Metal) [M, 2026-11-06]; the daemon's streams share one worker |
| Release | PR-12 (`transcribe`), PR-25 (`dictate`) |
| Acceptance | `dictation_cli.rs` runs `transcribe jfk.wav --format json` with the `for-tests` model and asserts exit 3 and `error_code: "no_speech"`, also with `BIOROUTER_TERMINAL=1` and no terminal (`transcribe_works_in_an_agent_shell`); from PR-14, an agent shell `transcribe --session <private id>` on reversed speech returns no private term |

### S12 ACP clients

Zed, JetBrains and other ACP editors render their own composer. None of the ACP clients checked (Zed, JetBrains AI Assistant, Jupyter AI's ACP client) sends audio: Zed only converts audio blocks between protocol versions and never builds one, JetBrains AI Assistant shows no sign of audio, and Jupyter AI's ACP client sends text and resource blocks only [R]. Biorouter advertises `.audio(false)` [C `crates/biorouter-acp/src/server.rs:755`]. Zed's request for dictation is the open issue zed#16410 [R]; no JetBrains issue was found. Biorouter owns no microphone control there, so there is nothing dead to show. The path is **Dictate to clipboard** and paste, and the editor learns it in the product, not only from the docs: at session start the ACP server advertises one available command, `/dictate`, through `AvailableCommandsUpdate` (in the pinned `agent-client-protocol-schema` 0.10.5 [R]). Invoking it answers MSG-ACP-DICTATE locally, with no model turn and no provider call. The Jupyter AI persona is an ACP client too, launched at `main` as `[BIOROUTER_EXECUTABLE, "acp"]` with nothing that named it [C `integrations/jupyter-ai/jupyter_ai_biorouter/persona.py`], and the owner excluded Jupyter from dictation (O-30, S13). As built in PR-30 (`999ea27f9`): Jupyter AI's ACP client 0.1.5 turns available commands into slash completions (read from the pinned wheel, not yet seen live), so the persona sets `BIOROUTER_ACP_CLIENT=jupyter-ai` in its child's environment and the server leaves `/dictate` out for that client (`client_hides_dictate` in `server.rs`). `/dictate` is announced after both a new and a loaded stdio session. Checklist row S12.7 checks this in JupyterLab; the persona's Python tests run in no workflow.

| Part | Design |
|---|---|
| Capture | None in the editor; the desktop's hidden capture window or the CLI capture child |
| Transcription | The desktop daemon's worker, or `biorouter dictate --copy` |
| Models | The local folder |
| Bias | Public only inside ACP (O-28 keeps it so when audio blocks arrive); the clipboard routes it relies on follow S18 |
| Entry points | Tray **Dictate to clipboard**, the global shortcut, or `biorouter dictate --copy`, then paste; the editor's `/dictate` command explains this with MSG-ACP-DICTATE |
| In the turn | An audio block, which `main` dropped silently at `server.rs:428` [C], becomes a text part with MSG-ACP-NOTE, and the note is sent to the editor; a prompt that holds only audio is answered locally with the note and no model turn (PR-30) |
| Docs | MSG-ACP in `docs/cli/command-reference.md` (`### acp`) |
| Dictionary | stdio offers the `dictionary` capability in suggest form only, with no extractor; tool answers append MSG-TOOL-REVIEW-HINT; `--ws` refuses `--with-builtin dictionary` and never takes audio |
| Release | PR-30 (note and `/dictate`, `999ea27f9`), PR-28 (`9a96e0132`) and PR-25 (clipboard) |
| Acceptance | An audio block yields the note and no audio; `on_initialize` still advertises `audio(false)`; `/dictate` appears in the editor's command list and answers MSG-ACP-DICTATE without calling the provider; Jupyter AI's chat shows no `/dictate` (O-30); a `--ws` agent loads no `dictionary`. On 2026-11-10 the ACP library and server tests pass in CI (`rust.yml` runs 42944486393 and 42949720898); the live Zed and JupyterLab rows (S12.1, S12.6, S12.7) are open |

PR-30 also fixes the stale `resolve_ws_token` comment, which says the desktop sets `BIOROUTER_ACP_WS_TOKEN`; nothing under `ui/desktop/src` does [C]. Never add private `_…/dictation/*` methods, never take audio on `--ws` (an untrusted page could send near silent clips and read bias terms back through the prompt echo [I]), and never install models over ACP.

### S13 Jupyter AI persona chat

Not offered, decided by the owner on 2026-11-05 (O-30, DD-81): Biorouter builds no microphone or dictation feature for Jupyter, and Jupyter AI dictation is deprecated. Biorouter adds no control to Jupyter AI's chat, so there is no dead control and no message to show there (R8). There is no labextension, no Jupyter server handler and no JupyterLab dictionary panel; PR-32 and M-13 are withdrawn. `biorouter serve` behind JupyterHub's proxy (S6) is Biorouter's own page, not a Jupyter feature.

### S14 Crew

DD-83 and O-21. Two microphones, both in the desktop app: in Crew's channel composer before `.crew-compose-send`, and in the **Ask my agent** pane immediately left of its submit button **Start my agent and allow posting here**. They use the same capture, transport and insert as `ChatInput`, with `target_kind: "crew"`. Each member's own daemon transcribes, and audio never crosses Crew's broker.

| Part | Design |
|---|---|
| Capture | The desktop renderer; the CLI capture child for `biorouter dictate` |
| Transcription | The member's own local daemon, or the CLI's worker |
| Models | The member's own folder |
| Bias | Public by default: the text goes to colleagues, and no Biorouter tier describes them; private terms only for one recording the person starts with **Include my private terms** after MSG-PRIVATE-TERMS (O-8, O-21) |
| Entry points | The two microphones; `biorouter dictate \| biorouter crew send <channel> --input -`, reviewed on the terminal before it posts; `biorouter crew ask <channel> "$(biorouter dictate)"` [C `commands/crew/args.rs` `Ask`], reviewed on the terminal before it starts a task |
| Sending | The end of a dictation never calls `submitNow` or `sendNow`. A dictated `@agent …` starts nothing without Enter, because "Enter is the consent" [C `Composer.tsx`]. `useRestoredDraftCaret` must not move the caret after an `execCommand` insert. Never an audio attachment: a channel's files are readable by every member |
| Learning | None from Crew boxes in v1; Crew scoped chats already permit only `crew__` and `todo__` tools [C `crates/biorouter/src/crew/mod.rs:4234`], so they cannot call `dictionary__*` |
| Messages | Tooltip, always: MSG-CREW-TIP. In a `serve` browser Crew is already refused with "Crew needs the Biorouter desktop app" [C `ui/desktop/src/components/crew/CrewNeedsDesktop.tsx`] |
| Docs | MSG-CREW-TIP and every other new quoted string gets its line in `docs/crew/messages-and-files.md`, and `node scripts/check-crew-manual.mjs` passes |
| Release | PR-27 (CLI pipe PR-25) |
| Acceptance | The end of a dictation never calls `submitNow` or `sendNow`; a dictated `@agent …` starts nothing without Enter; the draft's idempotency key is unchanged; a browser test shows the caret stays after the insert; checklist section S14 passes by hand |

### S15 Agent Drafter apps

DD-84 and O-16. No app page gets a microphone in v1: on the shared origin, a grant given to any page would be reachable by agent written code without a fresh prompt, which would let an agent open the microphone.

| Where the app runs | What a person gets |
|---|---|
| Served app pages and exported apps in a browser | `Permissions-Policy: microphone=()` on every app response (PR-03); **Dictate to clipboard** and paste. In Chromium a page that calls `getUserMedia` gets `NotAllowedError` |
| The desktop preview | **Dictate into this app** in the live page toolbar, `artifacts/WebPagePreview.tsx`, shipped only if M-15 passes; shown while a field in the app has focus, and working whether or not **Dictate into other apps** is on (O-42) |
| A browser, with no Biorouter app of the person's own | dictate in Biorouter's own composer and copy the text, or use the system's own dictation (MSG-APPS-BROWSER) |

**Dictate into this app.** A microphone icon button in the live page toolbar (`WebPagePreview.tsx`) for an app artifact, in the desktop app only: it needs IPC to main, so it is not rendered when `isBrowserSurface()` is true; a `serve` browser has no button, and the SDK reference gives people there MSG-APPS-BROWSER. The preview session keeps denying every permission [C `managedAppPreviewSession.ts`], so the app page never records. Pressing the button sends `dictation:anywhere:preview-start {viewId, privateConfirmed}`. Main records in the hidden capture window, as **Dictate to clipboard** does (S18), on the same stream (`target_kind: "external"`), and when the transcript comes back it calls `view.webContents.focus()` and `view.webContents.insertText(text)` on that preview's `WebContentsView`. The app receives text only, and the text goes into whatever field the app had focused; when no field is focused at the end, main opens the review card with the text instead. Twice a second the toolbar asks main (`dictation:anywhere:preview-focused {viewId}`), and main asks the preview with a fixed expression whether an editable element is focused. Main runs that expression with `executeJavaScriptInIsolatedWorld` in a world of its own, `DICTATION_FOCUS_WORLD_ID` in `utils/embeddedBrowser.ts`, and never in world 0, the page's own main world, where the app's script could redefine `document.activeElement` or `instanceof` and answer for itself; only a literal `true` counts as focused. The button is rendered only while a field is focused, so with nothing focused the toolbar has no button and MSG-APPS appears only in the SDK reference (owner decision, 2026-11-10). Main asks again when the button is pressed. The button does not depend on **Dictate into other apps**: that switch governs the tray item and the global shortcut, so a preview dictation starts with the switch off and goes on if the switch is turned off while it records (owner decision, 2026-11-10). M-15 measures whether `insertText` reaches the app's focused field after the press on the toolbar button moved focus to Biorouter's own window; if it fails, PR-28 ships without the button, S15 relies on **Dictate to clipboard**, and the SDK reference carries MSG-APPS-NO-PREVIEW in place of MSG-APPS, because MSG-APPS names the button.

| Part | Design |
|---|---|
| Transcription | The serving daemon's worker |
| Models | That daemon's folder |
| Bias | Public by default; the desktop preview's **Dictate into this app** takes private terms for one recording after MSG-PRIVATE-TERMS (O-8) |
| Docs | `docs/apps-sdk/sdk-reference.md` carries MSG-APPS (or MSG-APPS-NO-PREVIEW if M-15 fails) for people with the Biorouter app and MSG-APPS-BROWSER for people who reach an app only through a browser |
| Release | PR-03 (headers), PR-28 (the preview button if M-15 passes; the clipboard path either way) |
| Acceptance | A Playwright Chromium test grants the microphone to the shell origin, opens an app page on the same origin and gets `NotAllowedError`; M-15 recorded; `target_kind: "external"` on every preview request; vitest cases show **Dictate into this app** absent while nothing editable in the preview is focused, present while a field is, starting with **Dictate into other apps** off, and not rendered on `isBrowserSurface()` |

`br.dictate()` and a `capabilities.dictation` switch for apps wait for per app origins (see [what waits](#what-waits-for-a-later-release)).

### S16 Docker image

DD-80 and O-14. The image is `serve` capable; it never contains a speech model. At `main` the builder lacked `cmake` and built only `--package biorouter-cli`, so the image had no daemon and no web bundle [C `Dockerfile`].

As built in PR-31 (`8c865d11d`), and checked against `BUILDING_DOCKER.md`: `cmake` in the builder (PR-09 already needed it); `--package biorouter-cli --package biorouter-server`; a `node:24-bookworm` stage runs `npm ci && npm run build:web` in `ui/desktop`; `biorouterd` beside `biorouter` in `/usr/local/bin` and the bundle at `/usr/share/biorouter/web`; `EXPOSE 8765`. The native image gate passed on 2026-11-10 with a `linux/arm64` image on the maintainer's Mac; the amd64 image (M-16 amd64) waits until `docker-image.yml` is on `main`, and no image is published ([checklist S16](../releases/dictation-test-checklist.md#s16-docker-image)). `GGML_NATIVE=OFF` with an explicit ISA matters most here, because Docker builds are native and would otherwise carry the build machine's instructions.

```bash
# serve, published on the host's loopback only
docker run -p 127.0.0.1:8765:8765 \
  -v biorouter-data:/home/biorouter/.local/share/biorouter \
  -v biorouter-config:/home/biorouter/.config/biorouter \
  <image> serve --host 0.0.0.0

# install a model into the data volume
docker run --rm -v biorouter-data:/home/biorouter/.local/share/biorouter \
  -v biorouter-config:/home/biorouter/.config/biorouter \
  <image> dictation install --yes whisper-small.en-q8_0

# transcribe a recording, biased by the dictionary in the config volume
docker run --rm -v "$PWD:/data" -v biorouter-data:/home/biorouter/.local/share/biorouter \
  -v biorouter-config:/home/biorouter/.config/biorouter \
  <image> transcribe /data/memo.wav
```

The `serve` banner inside the container prints MSG-SERVE-CONTAINER with the token filled in: the address to open on the computer that runs Docker is `http://127.0.0.1:<published port>/?t=<token>`, not the container's own address.

An administrator can build their own image with a model. The image ends with `USER biorouter` [C `Dockerfile`], and `--system` needs root, so the derived image switches back and forth:

```text
FROM <image>
USER root
RUN biorouter dictation install --system --yes whisper-small.en-q8_0
USER biorouter
```

| Part | Design |
|---|---|
| Capture | None in the container (Docker Desktop's virtual machine has no microphone); the host's browser through `serve` |
| Transcription | The container's `biorouterd` worker, or `biorouter transcribe` |
| Models | A data volume at `/home/biorouter/.local/share/biorouter`, or a read only mount at `/var/lib/biorouter/dictation/models` |
| Bias | As S5; `transcribe` public only |
| Messages | `biorouter dictate` inside the container: MSG-CONTAINER. The `serve` banner for the `0.0.0.0` bind prints MSG-SERVE-CONTAINER, never MSG-SERVE-TUNNEL, so the person opens the published `http://127.0.0.1:8765/?t=<token>` with the token the banner shows |
| Release | PR-12 (`transcribe`, once PR-09 builds the engine in the image), PR-31 (`serve`) |
| Acceptance | `docker build` on amd64 and arm64 (nightly and on dispatch, O-24); inside the container the self test exits 0 and `biorouter dictation status` reports no model; a Playwright Chromium page at the published, tokened address the banner names reports `isSecureContext` and completes one fake microphone dictation (M-16); a banner snapshot inside a container shows MSG-SERVE-CONTAINER |

### S17 desktop app against an external or remote backend

DD-85. With **Use external server** on in **Settings > App**, the laptop records and the backend host transcribes, with the backend's model and dictionary.

| Part | Design |
|---|---|
| Capture | The laptop's renderer, a `file://` secure context whatever the backend URL; the laptop's own OS permission applies |
| Transport | The chunked stream to the backend URL; `connect-src` already admits the external origin [C `main.ts` `buildConnectSrc`]; `X-Secret-Key` is the configured **Secret Key**, and the proof is the configured **User Action Key**, or empty [C `ExternalBackendSection.tsx` labels; `main.ts` `getUserActionKey`] |
| Transcription | The backend host's `biorouterd` worker |
| Models | The backend host's folder; Settings installs there with that server's **User Action Key** on a keyed daemon, or with the secret on a keyless one; the **Model folder** row names the backend's folder and account, and the reveal button is hidden unless the backend is on loopback |
| Bias | The backend's dictionary under the caller's standing: with the key, as S1; without it, public only (O-17) |
| Entry points | The same microphone and sidebar |
| Disclosure | MSG-DISCLOSE-REMOTE in Settings, in `ExternalBackendSection.tsx` and in the setup popover |
| Plain `http:` | MSG-REMOTE-HTTP once per backend URL for a non loopback `http:` address; the person confirms and proceeds (O-18); a SHA-256 identity of the complete effective URL is kept in `ackHosts[]` |
| No key on a keyed backend | Install disabled before the click with MSG-REMOTE-NO-KEY (O-17) |
| A backend from before dictation | The laptop and the backend can run different versions here, and only here. When `GET /dictation/status` answers 404 or 405, the microphone is `unavailable` with MSG-REMOTE-OLD; when `/dictionary/settings` answers 404, the **Dictionary** page shows the same sentence and disables its controls |
| The laptop's policy | Electron main rereads the laptop policy for each media permission check and request, and before a hidden clipboard capture starts. `utils/managedDictationPolicy.ts` reads the laptop's managed policy file at the `Paths::managed_policy_path` locations (with `js-yaml`, already a dependency [C]) only when `externalBiorouterd.enabled`. It honours only `dictation.enabled: false` and `dictation.capture: false`; either makes `isAllowedRendererPermission` refuse `media`, leaves the microphone disabled with MSG-POLICY or MSG-POLICY-CAPTURE as its reason (owner ruling, 2026-11-12), disables the clipboard switch, and shows MSG-POLICY or MSG-POLICY-CAPTURE. A Rust test pins the TypeScript path table against `Paths::managed_policy_path` |
| Release | PR-29 |
| Acceptance | Tests for loopback, remote `https:`, remote `http:`, no key, and a backend that answers 404 on `/dictation/status` (MSG-REMOTE-OLD, microphone `unavailable`, Dictionary controls disabled); a staged laptop policy refuses `media`; M-20 (latency and uploads to a LAN `biorouterd` behind nginx) recorded; checklist row S17.5 against an older `biorouterd` |

The laptop's policy locations, mirrored in `utils/managedDictationPolicy.ts`:

```ts
// ui/desktop/src/utils/managedDictationPolicy.ts
// Mirrors Paths::managed_policy_path; a Rust test reads this table and compares.
export function managedPolicyPath(env: NodeJS.ProcessEnv, platform: NodeJS.Platform): string | null {
  const root = env.BIOROUTER_PATH_ROOT?.trim();
  if (root) return path.join(root, 'managed', 'managed-policy.yaml');
  if (platform === 'darwin') return '/Library/Application Support/Biorouter/managed-policy.yaml';
  if (platform === 'linux') return '/etc/biorouter/managed-policy.yaml';
  if (platform === 'win32') return env.ProgramData ? path.join(env.ProgramData, 'Biorouter', 'managed-policy.yaml') : null;
  return null;
}
export function readLaptopDictationPolicy(): { enabled: boolean; capture: boolean }; // fails closed on a parse error
```

The desktop's hidden Dictate to clipboard window attaches to that same external backend. Before main requests the external backend's dictation status or opens a new hidden capture window, it shows MSG-DISCLOSE-REMOTE and, for a non loopback `http:` URL, MSG-REMOTE-HTTP. The confirmation is remembered by backend identity in that viewer's `ackHosts[]`; a cancelled dialog starts no capture or upload. Main checks the laptop policy again after the dialog. Changing the backend query asks again; URL credentials and query values are never written to `ackHosts[]`. Screen lock invalidates a pending hidden confirmation, even when the native dialog is still open. A configured external backend without its User Action Key may still dictate with public terms, but the private tray item is disabled.

Without the key, a keyed backend's microphone works with public entries only and installing is disabled, with MSG-REMOTE-NO-KEY, as decided by the owner on 2026-11-05 (O-17).

As built in PR-29 (`9a96e0132`, with PR-28), the laptop's policy was read by Electron main only, for the media permission handler and for the **Dictate into other apps** row (`anywhereSnapshot` in `main.ts`), so under a laptop policy alone the composer microphone looked ready and a press met the permission handler's refusal: its disabled look with the policy's sentence (2026-11-12) came from the daemon's `status.policy`, which on S17 is the backend host's. That gap is closed (2026-11-13): the status store reads the laptop's policy from main with every status read, and so again when the window regains focus, and the microphone is `unavailable` under it with MSG-POLICY or MSG-POLICY-CAPTURE as its name and tooltip (`dictationStore.ts` `resolveDictationPhase`). Checklist row S17.4 checks it by hand.

The laptop's CLI has its own folder and needs its own `biorouter dictation install`; `status` names the host and account, so the two are not confused.

### S18 other applications: dictate to clipboard

DD-86, O-8 and O-19. Two forms in v1 bring Biorouter's engine and dictionary to any application, with public terms unless the person includes private ones for one recording. Neither pastes for the person, and neither sends anything.

**The desktop form** (`ui/desktop/src/utils/dictationAnywhere.ts`, PR-28):

1. **Turn it on.** **Settings > Models > Speech models > Dictate into other apps**. Turning the switch on shows MSG-CLIPBOARD-EXPLAIN and, on macOS, calls `askForMediaAccess('microphone')` so the TCC prompt comes while the person looks at Biorouter. On Windows and Linux the row shows MSG-WINDOW-OPEN. The switch and the chosen shortcut live in `<config>/dictation/anywhere.json`, written only by Electron main and read by main at launch to build the tray item and register the shortcut (DD-88). The file is outside `config.yaml`, so `/config/upsert` cannot turn the global shortcut on, and outside renderer `localStorage`, which main could not read at launch; like the privacy master switch record, a shell can still edit it (accident prevention, not a boundary).
2. **Start.** The tray item **Dictate to clipboard**, added to `updateTrayMenu` in `ui/desktop/src/utils/autoUpdater.ts` while policy allows, or the global shortcut. Below that tray item, **Include my private terms** starts one recording with private terms after MSG-PRIVATE-TERMS ([including private terms](#including-private-terms-for-one-recording)). The tray exists only while **Show in menu bar** is on in **Settings > App**, and `createTray` turns that setting off when the icon file is missing or the tray cannot be created [C `main.ts` `createTray`, `disableTray`]. While the tray is absent, the row shows MSG-NO-TRAY and the switch turns on only with a registered shortcut, so the feature always has a way in. On GNOME Wayland without a tray, the switch stays off because global shortcuts cannot register there. While the switch is off, the tray item opens Settings at that row instead of recording; for a new window main waits for its exact renderer to install the navigation listener before sending the destination, and a newer tray intent supersedes the pending one; with no model installed, it opens the Speech models section. A delayed shortcut save cannot turn the feature back on after the person turns the switch off. Turning the switch off invalidates any pending tray or shortcut start as soon as main receives the request, before its settings file write, so a delayed status answer cannot open the microphone. It leaves a dictation started from **Dictate into this app** running, because the switch does not govern the preview button (S15); the tray's **Stop and copy** and **Cancel** still end that dictation. If saving fails, dictation stays off for this session and the row reports the failure; the person must check the setting after restarting Biorouter. The shortcut is off until the person picks one (`CommandOrControl+Alt+Shift+D` suggested); `globalShortcut.register()`'s result is checked, and `false` shows MSG-SHORTCUT-TAKEN. On GNOME Wayland the picker is replaced by MSG-GNOME. Toggle only: Electron reports no key up.
3. **Record.** Main shows a recording tray image and the items **Stop and copy** and **Cancel**, starts `powerSaveBlocker.start('prevent-app-suspension')`, and opens a hidden capture window (`show: false`, `focusable: false`, `skipTaskbar: true`, `backgroundThrottling: false`) on the renderer partition, loading `#/dictation-capture`. That page runs the composer's capture and transport with `target_kind: "external"`. Deadlines come from worklet counts or main process timers, because hidden pages slow `setInterval` to once per second.
4. **Stop and copy.** A second trigger or **Stop and copy** finishes the stream. Main writes with `clipboard.writeText`, then compares `readText()`. A match shows the notification MSG-CLIPBOARD-COPIED (`<key>` is ⌘V or Ctrl+V); a mismatch (a Wayland clipboard that needs focus) opens a focused review card holding **Copy**, which reads the clipboard back too and shows MSG-CLIPBOARD-COPY-FAILED when it did not keep the text. `no_speech` or an empty final transcript shows MSG-CLIPBOARD-NOTHING and leaves the clipboard unchanged. A nonempty final transcript made only of complete eligible dictionary terms shows MSG-CLIPBOARD-ECHO and copies nothing. The daemon derives that boolean from the same stream's filtered entries and final corrected text. A successful nonempty result after an echo retry still copies unless it is dictionary only. An older backend without the boolean follows MSG-REMOTE-OLD and leaves the clipboard unchanged.
5. **Errors.** A microphone refusal opens the review card with the mapped sentence and its action (MSG-MIC-DENIED-MAC, MSG-MIC-DENIED-WIN, MSG-NO-DEVICE, MSG-MIC-BUSY). An engine failure opens it with MSG-ENGINE-FAILED and **Retry**. As built in PR-28 (`9a96e0132`), a capture that fails with no mapped sentence opens the card with MSG-CLIPBOARD-FAILED, written in `main.ts` (`showAnywhereReview`); checklist row S18.10 checks it. Since 2026-11-13 (owner decision), a tray or shortcut dictation whose transcription failed in a way the stream can send again (`engineFailed`, or a server still busy after the stop) keeps its recording, as the composer does: the hidden page keeps the stream with the audio already sent (`keepsFailedRecording` in `utils/dictationAnywhere.ts`), main keeps the capture window open, and the card offers **Retry** beside MSG-CLIPBOARD-FAILED. **Retry** hides the card and sends the same audio into a new stream, and a success writes the clipboard exactly as a first one does. The hold ends after 5 minutes (`HELD_FOR_RETRY_MS`, the composer's `KEEP_FAILED_MS`), when **Retry** runs, when the card closes or shows anything else, when a new dictation starts and when the switch turns off; the page then cancels the stream, and the card keeps the sentence without **Retry**. A refusal before capture, a microphone that never started and a preview dictation (whose field may hold other text by then) keep the plain sentence. Checklist row S18.11 checks it.
6. **Afterwards.** The tray image returns, the power save blocker stops, and the capture window idles, then closes. The last transcript stays in main's memory for at most 5 minutes, for the review card only. Main enforces that deadline for every review copy and fetch, clears the renderer at the same deadline, and closes the hidden capture window after 15 seconds idle, but never while it holds a recording for **Retry**. An unexpected capture-window close cancels the operation and stops its power blocker. Notifications carry counts, never text.

On Windows and Linux the tray item and shortcut work only while a Biorouter window is open (minimized is fine), because closing the last window quits the app there [C `main.ts` `window-all-closed`]. On macOS the menu bar icon keeps the app alive while it is shown; without it, closing the last window quits, as on Windows and Linux (`process.platform !== 'darwin' || !tray` [C]). The feature is absent on `serve`, where `renderer.tsx`'s stand in reports it unavailable. The desktop path cannot attach the Windows cloud clipboard exclusion to the text with Electron 39 (see S3) [R].

**The CLI form**: `biorouter dictate --copy`, with the capture child and the review rule, through the helper `crates/biorouter-cli/src/clipboard.rs` (which also fixes the TUI's copy, whose `write_clipboard` drops the clipboard at once [C `session/tui/mod.rs`]):

| Platform | Behaviour |
|---|---|
| macOS | set the text and exit |
| Windows (native) | set the text with arboard's `SetExtWindows::exclude_from_cloud()` (the `CanUploadToCloudClipboard` format) and exit |
| Linux | `set().wait_until(now + 60 s)`; after 300 ms prints MSG-CLIPBOARD-WAIT; Ctrl+C ends the wait; the text is also on stdout |
| WSL | pipes the text to `powershell.exe -NoProfile -Command Set-Clipboard`, which sets no cloud exclusion |
| No display | prints the text with MSG-NO-CLIPBOARD and exits 5 |

`--no-history` adds `exclude_from_history()`; by default a dictated copy stays in local history like a typed one (O-19). The TUI holds one clipboard lease for its whole lifetime.

| Part | Design |
|---|---|
| Bias | Public by default; private terms only for one recording the person starts with **Include my private terms** (desktop) or `--private-terms` (CLI) after MSG-PRIVATE-TERMS, never for an agent (O-8) |
| Rules for both forms | nothing echoes the transcript; no synthetic paste; no automatic send; MSG-CLIPBOARD-EXPLAIN once; no daemon route and no tool starts capture or sets `include_private_terms`; managed policy disables all of it |
| Release | PR-28 (desktop), PR-25 (CLI) |
| Acceptance | The state machine tests in `dictationAnywhere.test.ts` (shortcut taken, policy off, no model, an echo only result, a read back mismatch opening the review card, no tray: MSG-NO-TRAY and the switch refused without a shortcut); notifications carry counts only; checklist row S18.8 turns off **Show in menu bar** and checks the shortcut path; M-12 (hidden capture with a real microphone on Windows 11 and PipeWire, the worklet from `app.asar` in the hidden window, the TCC prompt from the hidden window); GNOME Wayland shows MSG-GNOME |

### S19 WSL

| Part | Design |
|---|---|
| Capture | WSL2 with WSLg: the CLI capture child loads `libpulse.so.0` at run time and talks to WSLg's PulseAudio server (`/mnt/wslg/PulseServer`). WSL2 without WSLg (Windows 10 builds before WSLg, or `guiApplications=false` in `.wslconfig`) and WSL1: none |
| Transcription | WSL's `biorouter` worker, Linux on the CPU |
| Models | The WSL account's `~/.local/share/biorouter/dictation/models/`; `biorouter dictation install --from /mnt/c/…` reuses a download made on the Windows side, after the hash check |
| Bias | As S9 |
| Entry points | As S9; or `biorouter serve` inside WSL, opened from a Windows browser at the address it prints (`http://127.0.0.1:8765/?t=<token>`), which is S5 through WSL's localhost forwarding [I]; or the Windows desktop app (S3) |
| Detection | `WSL_DISTRO_NAME` set; WSLg when `/mnt/wslg/PulseServer` exists; WSL1 when `/proc/sys/kernel/osrelease` lacks `microsoft-standard` |
| Messages | MSG-WSL2-NOPULSE when WSLg is there but the PulseAudio client library is missing; MSG-WSL2-NOWSLG when WSLg is absent or turned off, because no PulseAudio server exists to reach and installing `libpulse0` would not help; MSG-WSL1 |
| Clipboard | `biorouter dictate --copy` goes through `powershell.exe Set-Clipboard` |
| Release | PR-25, PR-26 |
| Acceptance | Environment table tests for WSLg, WSL2 without WSLg and WSL1; checklist section S19 records a WSLg dictation, and row S19.7 records MSG-WSL2-NOWSLG with `guiApplications=false` |

### S20 air gapped or offline hosts

Nothing needs a network at run time: capture, transcription and the dictionary are all local. Only the model download does, and it has an offline path (DD-24).

1. On a computer that can reach GitHub, run `biorouter dictation urls`, which prints each file's URL, size and SHA-256, and download the files it lists.
2. Carry the files to the offline host.
3. Run `biorouter dictation install --from <file|dir>`, which identifies each file by its SHA-256 against the catalog compiled into Biorouter, whatever its name. On the desktop or `serve`, put the files in the folder that **Model folder** shows and press **Rescan** (`POST /dictation/models/rescan`).
4. Fleets put the files in the managed `dictation.models_dir`, read only and still hash checked.

No HTTP route takes a filesystem path. An install that cannot reach the repository shows MSG-OFFLINE with `<folder>` filled in.

| Part | Design |
|---|---|
| Capture, transcription, bias, entry points | As the surface it is |
| Messages | MSG-OFFLINE; MSG-POLICY-DOWNLOADS when managed `model_downloads: false` refuses a download; MSG-POLICY-MODELS when `allowed_models` excludes the request |
| Release | PR-11 (CLI), PR-18 (**Rescan**) |
| Acceptance | `--from` with a renamed file installs by hash; `model_downloads: false` refuses a download with MSG-POLICY-DOWNLOADS while `--from` into `models_dir` works; checklist section S20 installs on a host with no route to GitHub |

## Hosts without AVX2

DD-9. On x86, the engine is built for `x86-64-v3` and needs AVX2, FMA, F16C and BMI2; Linux arm64 needs `dotprod`. The parent checks CPUID before starting a worker, so an unsupported processor gets a sentence and never a crash; the worker's self test exits 20 with MSG-CPU. Under Rosetta, AVX2 is hidden but works, so the parent sets `ROSETTA_ADVERTISE_AVX=1` for the worker when `sysctl.proc_translated == 1` [R for macOS 15 and later]. `status.engine.state` is `unsupported_cpu` with `missing_cpu_features`, every model's suitability is `unsupported_cpu`, and installs are refused before any download.

| Where | What the person sees | The route that still works |
|---|---|---|
| Desktop app (S2, S3, S4) | MSG-CPU, then MSG-CPU-DESKTOP, as the microphone's reason and in Settings | the system's own dictation, without the Biorouter dictionary; or connect the app to a Biorouter server that supports dictation (S17) |
| Intel build under Rosetta on macOS 12 to 14 (any version before 15) | MSG-CPU-ROSETTA | install the Apple Silicon build |
| `serve` browser and other servers (S5 to S8, S16) | MSG-CPU-SERVER; on a Linux virtual machine, MSG-CPU-VM after it; the `serve` banner says "Dictation: unavailable on this processor" | the person's Biorouter app on a computer whose processor supports dictation, with **Dictate to clipboard**; the system's own dictation |
| CLI (S9 to S11, S19) | MSG-CPU, exit 2 | as above |

A second helper build for such hosts is O-11, decided after the census M-11 of lab hosts.

## What waits for a later release

Each item below has a working path today, so no surface is left without one (R8).

| Later | Why it waits | Today's path |
|---|---|---|
| Transcribing ACP audio blocks (`audio(true)` on stdio only, transcribed in `on_prompt` before `convert_acp_prompt_to_message`, labelled "(dictated)", public only per O-28) | None of the ACP clients checked (Zed, JetBrains AI Assistant, Jupyter AI's ACP client) sends audio [R] | **Dictate to clipboard** or `biorouter dictate --copy`, then paste; `/dictate` in the editor answers MSG-ACP-DICTATE; MSG-ACP-NOTE in the turn |
| `br.dictate()` and `br.dictation.attach()` in app pages, behind a deny by default `capabilities.dictation {enabled, max_seconds ≤ 120}`, `microphone=(self)` only for a declaring app, recording only from a user gesture and never from a `ui_*` tool, public only, text only | On the shared origin, agent written code could reach a granted microphone; per app origins (SD-14's follow up) come first (O-16) | **Dictate into this app** in the desktop preview if M-15 passes; **Dictate to clipboard** |
| Hold to talk in the terminal | On Unix it needs crossterm's `REPORT_EVENT_TYPES` flag, which only terminals with the kitty keyboard protocol honour; on Windows crossterm always reports key release [R crossterm 0.28.1 `event.rs`]. The TUI already pushes `DISAMBIGUATE_ESCAPE_CODES` and keeps only `KeyEventKind::Press` [C `session/tui/mod.rs`, three filters]. The flag does not break key repeat by itself: with it pushed, auto repeat arrives as `KeyEventKind::Repeat` [R crossterm `parse.rs`], which those filters drop, so each must first accept `Press` or `Repeat` (DD-74) | Ctrl+T toggles |
| A GNOME Wayland global shortcut | Electron 39 cannot register one on GNOME; GNOME works from Electron 43 with a reverse DNS `desktopName` [R] | The tray item (with the AppIndicator extension) and the composer microphone; MSG-GNOME |
| Push to talk and synthetic paste for the clipboard | Electron reports no key up for global shortcuts, and synthetic paste needs Accessibility permission | Toggle, then the person pastes |
| The **Shortcut**, **Show the microphone button** and **Reduce background noise** rows of **Settings > Models > Speech models** | Not built by 2026-11-10: PR-19 shipped the chord and the viewer fields, and no later pull request owned the rows ([Settings](#settings)) | ⌘⇧D works without the row; the viewer fields keep their defaults (button shown, noise reduction off) |
| An off machine engine (`BIOROUTER_DICTATION_ENGINE_URL`) | Not needed by R1 to R8; it would send audio and terms to a host the tier rules call Public (O-12, DD-87). The managed `external_engine` key is parsed now as a no op | The local engine on every surface |

## Message catalog

Renderer copy lives in `ui/desktop/src/components/dictation/copy.ts` and `components/dictionary/copy.ts`; CLI copy in `crates/biorouter-cli/src/dictation/messages.rs`; both are pinned by snapshot tests. That CLI file holds `transcribe`'s sentences from PR-12 (MSG-TRANSCRIBE-NOTHING to MSG-TRANSCRIBE-FAILED, with MSG-TOO-SHORT), pinned by `the_transcribe_sentences_are_the_catalogs`; PR-10's status lines and PR-11's model command sentences (MSG-SYSTEM-ROOT to MSG-LIST-NOTES) are still in `crates/biorouter-cli/src/commands/dictation.rs`, the installer's own sentences (MSG-TLS to MSG-REMOVE-FAILED, with MSG-HASH and MSG-DISK) in `crates/biorouter-dictation/src/models.rs`, and the engine's diagnostics (MSG-DIAG-EXIT to MSG-DIAG-CANCELLED) are `DictationError`'s text in `crates/biorouter/src/dictation/service.rs`, each pinned by a test there. The dictionary commands' sentences (MSG-DICTIONARY-TTY to MSG-DICTIONARY-FOLDER) are in `crates/biorouter-cli/src/commands/dictionary.rs` and the store's in `crates/biorouter-dictation/src/store/`, pinned by `the_dictionary_sentences_are_the_catalogs` in the first file (the validation sentences also by `every_code_has_its_catalog_sentence` in `store/validate.rs`). The daemon's own sentences are in `crates/biorouter-server/src/routes/dictation.rs` (MSG-INSTALL-AGENT, MSG-INSTALL-LOCKED) and `routes/dictionary.rs` (MSG-DICTIONARY-AGENT, MSG-DICTIONARY-HOST-TIER, MSG-DICTIONARY-HOST-SETTINGS, MSG-DICTIONARY-HOST-PRIVATE, MSG-SAME-LETTERS), and the hidden self test's in `crates/biorouter-dictation/src/worker/self_test.rs` (MSG-SELFTEST). Four tests read this table itself, so a row's text column must stay word for word what the code prints: `the_route_sentences_are_the_catalogs` in `routes/dictionary.rs`, `the_offline_sentence_is_the_catalogs` in `crates/biorouter/src/dictation/mod.rs` (MSG-OFFLINE, which the daemon's install job and the CLI print alike), `ui/desktop/src/components/dictation/copy.test.ts`, which reads every row the renderer's dictation copy names, and `ui/desktop/src/components/dictionary/copy.test.ts`, which does the same for the Dictionary page's copy (PR-20) and also checks that every validation code the store answers has its sentence. A code comment that names a sentence names it by the id of its row here. The other dictation documents quote these by id. Text in angle brackets is filled in unless the row says it stays as written; bold marks a button or link label. Every sentence a person sees is in this table; a pull request that needs a new one adds its row here first. "Biorouter does not … send it anywhere else" (MSG-DISCLOSE-HOST) speaks for Biorouter only, not for a proxy in front of it, and is true only after PR-01 removes the inherited cloud dictation; no document may say it of an earlier build.

Two groups of message ids were retired on 2026-11-05 and are not reused: the Copilot refusal message (O-15) and the Jupyter AI messages (O-30).

| Id | Text | Shown where |
|---|---|---|
| MSG-POLICY | Dictation is turned off on this computer by your organization's managed policy. | Every surface: Settings policy note, CLI (exit 2), tray, `/dictation/*` 403 |
| MSG-POLICY-CAPTURE | Your organization's managed policy turns off live microphone dictation on this computer. Transcribing a recording still works. | As MSG-POLICY, when only `capture` is off |
| MSG-POLICY-DOWNLOADS | Your organization's managed policy turns off model downloads on this computer. Speech models come only from \<folder>; ask your administrator to place one there. | Settings, popover, CLI install, install route 403, when `model_downloads: false` refuses a download |
| MSG-POLICY-MODELS | Your organization's managed policy allows only these speech models here: \<models>. | Settings, popover, CLI install, install route 403, when `allowed_models` excludes the request |
| MSG-NO-ENGINE | This build of Biorouter has no local dictation engine. | Microphone tooltip, Settings, CLI step 1 |
| MSG-CPU | This processor does not support the instructions local dictation needs (\<missing>). Your system's own dictation still works, without your Biorouter dictionary. | Desktop, CLI, the worker self test; \<missing> is `missing_cpu_features`, for example AVX2, FMA, F16C, BMI2 on x86 or dotprod on Linux arm64 |
| MSG-CPU-DESKTOP | To use your Biorouter dictionary, connect this app to a Biorouter server whose processor supports dictation, in Settings > App > Use external server. | After MSG-CPU on the desktop |
| MSG-CPU-SERVER | This server's processor cannot run local dictation (it lacks \<missing>). Your system's own dictation still works, without your Biorouter dictionary. If you have the Biorouter app on a computer whose processor supports dictation, Dictate to clipboard works there. | `serve` browser, other servers |
| MSG-CPU-VM | If this is a virtual machine, ask its administrator to expose the host CPU model (for example `host-passthrough`). | After MSG-CPU-SERVER on a Linux virtual machine |
| MSG-CPU-ROSETTA | This is the Intel build of Biorouter running under Rosetta, which cannot run local dictation on this version of macOS. Install the Apple Silicon build of Biorouter. | Intel build under Rosetta on macOS 12 to 14 (any version before 15) |
| MSG-STATUS | `Engine:` · `Processor:` · `Backend:` · `Worker:` · `Speech models:` · `Model folder:` · `Policy:` · `Microphone:`, with `--verbose` `Worker hello:` · `Backends:` · `CPU features:` · `System info:` · `Worker exit:` · `Worker stderr:` | The labels of `biorouter dictation status` and of `doctor`'s **Dictation** section, under the heading **Dictation**. Their values: `whisper.cpp <commit>, ggml <version> (<isa>)` or MSG-NO-ENGINE; `supported` or MSG-CPU (with MSG-CPU-ROSETTA or MSG-CPU-VM); `<backend>, <n> threads` or `none`; `none`, `not started`, `stopped`, `starting`, `ready (rss <size>)` or `error`; each speech model as `<id> (<location>, in use, voice detector missing)` with the notes that apply, joined with `; `, or `none installed`; `<folder> (account <user>)`; MSG-STATUS-POLICY's forms; `not part of this build` until PR-25. The text is built in `crates/biorouter-cli/src/commands/dictation.rs` |
| MSG-STATUS-POLICY | managed by your organization; dictation is allowed | The `Policy:` line of `biorouter dictation status` and `doctor` when a managed file narrows nothing; it reads `not managed` without one, and the narrowing sentence (MSG-POLICY, MSG-POLICY-CAPTURE, MSG-POLICY-DOWNLOADS) when one applies |
| MSG-DIAG-EXIT | The dictation worker stopped with exit code \<code>. (after a signal: The dictation worker was stopped.) | `status --verbose`, `Worker hello:`; the detail behind an engine failure. This row and the six below it are `DictationError`'s text, pinned by `the_error_sentences_are_the_catalogs` in `crates/biorouter/src/dictation/service.rs` |
| MSG-DIAG-TIME | The dictation worker did not answer in time (\<phase>). | As MSG-DIAG-EXIT; \<phase> stays as written: `hello`, `load`, `transcribe`, `vad_segments` or `shutdown` |
| MSG-DIAG-LOAD | The speech model could not be loaded: \<reason>. | As MSG-DIAG-EXIT; \<reason> is the worker's fixed phrase, such as `model file size does not match` or `voice detector could not be loaded` |
| MSG-DIAG-WORKER | worker \<program> speaks protocol \<n>; this build speaks 1 | As MSG-DIAG-EXIT, for a `BIOROUTER_DICTATION_WORKER` of another build. The same diagnostic, which names the worker binary and nothing else, says `worker <program> sent a frame that could not be read: <reason>`, `worker <program> sent a message that could not be read` or `worker <program> did not start with hello`; and before a spawn, `BIOROUTER_DICTATION_WORKER must be an absolute path` or `the running executable could not be found` |
| MSG-DIAG-FAILED | Dictation failed: \<detail>. | As MSG-DIAG-EXIT; \<detail> is a fixed phrase: `the worker could not start (<error kind>)`, `the dictation supervisor has stopped`, `dictation needs an async runtime`, `a model path is not valid UTF-8`, or the worker's own phrase for an internal error (`no model loaded`, `unsupported message`, `decoding failed`, `voice detection failed`) |
| MSG-DIAG-AUDIO | The audio could not be used. | As MSG-DIAG-EXIT, for audio the worker refused |
| MSG-DIAG-CANCELLED | Dictation was cancelled. | As MSG-DIAG-EXIT, for a request cancelled before it answered |
| MSG-SELFTEST | The self test accepts only pinned files: the fixtures and the speech model catalog. The model or the voice detector file is missing, unreadable or not pinned. · The sample could not be read: \<why>. · This build lacks instruction set features it was configured with: \<features>. · The self test could not load its model: \<error>. · The voice detector did not find speech in the sample, or found speech in silence. · Decoding the sample with the fixture model returned text. · Decoding the sample failed: \<error>. | The hidden self test (`__br-dictation-worker self-test`) on stderr, one sentence per failed stage, beside its one JSON line on stdout; a refused processor gets MSG-CPU instead. Seen in package smokes, `scripts/dictation-self-test.sh` and CI logs |
| MSG-NO-MODEL-CLI | Dictation needs a speech model on this computer (\<host>, account \<user>). Install \<model> (\<size>, from BaranziniLab's GitHub release)? Run `biorouter dictation install`, or type `/dictate install`. | The terminal front ends and `dictate` with no model (PR-25, PR-26); until they land, the commands that exist print MSG-NO-MODEL |
| MSG-NO-MODEL | Dictation needs a speech model on this computer (\<host>, account \<user>). Run `biorouter dictation install`. | CLI `transcribe` and `bench` when no speech model is installed in any folder (exit 2); `DictationError::NoModel`'s text in `crates/biorouter/src/dictation/service.rs`. The daemon answers it as `409 noModel` with the caller's install reason, or an empty message when installing is allowed, so the interface opens the setup popover |
| MSG-UNKNOWN-MODEL | There is no speech model named \<id>. Run `biorouter dictation list` to see the models. | CLI `install`, `remove`, `use`, `urls`, `warm-up` and `transcribe --model` given an id the catalog does not know, and `use` given the voice detector's id (exit 2); `DictationError::UnknownModel`'s text. The daemon answers an unknown id with 422 |
| MSG-NOT-INSTALLED | \<model> is not installed on this computer. Run `biorouter dictation install <model>`. | CLI `use` and `transcribe --model` naming a catalog speech model with no verified copy in any folder (exit 2), and `warm-up` (exit 1); `DictationError::NotInstalled`'s text. The daemon answers it as `409 noModel`, as MSG-NO-MODEL |
| MSG-INSTALL-LOCKED | Dictation is not set up on this server. Ask the person who runs it to run `biorouter dictation install`. | `serve` microphone and Settings when installs are locked |
| MSG-INSTALL-AGENT | Speech models are installed by a person from Biorouter's Settings; an agent cannot install one. | Install route refusal without the proof on a keyed daemon |
| MSG-INSTALL-SHELL | Install the speech model yourself: run this in a terminal, or use Settings > Models > Speech models. | `biorouter dictation install` under `BIOROUTER_TERMINAL=1` |
| MSG-OFFLINE | This computer cannot reach Biorouter's model repository. On a computer that can, run `biorouter dictation urls` and download the files it lists, copy them here, then run `biorouter dictation install --from <file>` (or put them in \<folder> and choose Rescan). | Settings, popover, CLI install |
| MSG-HASH | The downloaded file did not match Biorouter's pin, so nothing was installed. | Settings, popover, CLI install |
| MSG-DISK | Installing \<model> needs \<x> GB free on \<volume>; \<y> GB is available. | Settings, popover, CLI install |
| MSG-TURBO-CPU | This model takes about \<t> s to transcribe \<a> s of speech on this computer. Install it anyway? (buttons **Install anyway**, **Cancel**) | Settings before a `too_slow` model (turbo on a CPU host) for which **Test speed** measured a time from this window; \<t> is that decode's seconds and \<a> the clip's 5.5 s. `GET /dictation/status` carries no bench record, so without such a measurement Settings asks MSG-TOO-SLOW-SETTINGS |
| MSG-TOO-SLOW-SETTINGS | \<model> may be too slow for dictation on this computer. After you install it, Test speed measures it. Install it anyway? (buttons **Install anyway**, **Cancel**) | Settings before a `too_slow` model with no measured time, in place of MSG-TURBO-CPU, as the terminal's MSG-TOO-SLOW names the rule rather than a time (PR-18) |
| MSG-BELOW-MINIMUM-SETTINGS | \<model> needs more memory than this computer has, so it may load slowly or not at all. Install it anyway? (buttons **Install anyway**, **Cancel**) | Settings before a `below_minimum` model (DD-22); the terminal's MSG-BELOW-MINIMUM names the gigabytes, which the status does not carry (PR-18) |
| MSG-LANGUAGE-EN | \<model> understands only English. Choose a multilingual model for other languages, or set the language to English. | **Language** row; CLI `--language`. The daemon answers it as `409 languageUnsupported` with the catalog id (`DictationError::EnglishOnly`); Settings fills it with the display name of the model the refusal is about, on the **Language** row while the saved language is one the English only model in use refuses, and on a model row whose **Use** it refused (PR-18) |
| MSG-LANGUAGE-EN-ONLY | \<model> understands only English. To dictate in another language, use a multilingual model. | Settings, under the **Language** row while an English only model is in use with English chosen; the menu then holds English alone and is disabled (PR-18) |
| MSG-LANGUAGE-UNKNOWN | Biorouter does not know the language "\<code>". Use a two letter code such as en or de, or auto to let the model decide. | CLI `--language` and `transcribe` (exit 2), and `BIOROUTER_DICTATION_LANGUAGE` in `config.yaml`, both read in lower case; \<code> as written, quoted. `DictationError::LanguageUnsupported`'s text in `crates/biorouter/src/dictation/service.rs`, pinned by `the_error_sentences_are_the_catalogs` there |
| MSG-OTHER-ACCOUNT | The Biorouter service on this computer runs as `<service user>`; models you install as `<user>` are not visible to it. Run: `sudo -u <service user> -H biorouter dictation install` | CLI `status` and `install` on a service host; \<service user> is the owner of the `biorouterd` the `/proc` scan found |
| MSG-TLS | The secure connection to \<host> could not be verified, so nothing was downloaded. | Settings, popover, CLI install; a TLS failure (DD-21), told by type and never retried. This row and the ten below it are the installer's own text in `crates/biorouter-dictation/src/models.rs`, pinned by `the_installer_sentences_are_the_catalogs` in `crates/biorouter-cli/src/commands/dictation.rs` |
| MSG-DOWNLOAD-FAILED | The download kept failing, so it stopped. Run the install again to continue where it stopped. | Settings, popover, CLI install: a host that serves the bytes answered, and five attempts in a row added nothing; the partial is kept |
| MSG-HTTP | The model repository answered HTTP \<status>. | CLI install; an answer the installer does not follow (a 4xx other than 403, 404, 408, 410, 425 and 429) |
| MSG-DOWNLOAD-REFUSED | The download was refused: \<why>. | CLI install; \<why> is a fixed phrase: `more than 5 redirects`, `a redirect without a location`, `<host> is not an https address` or `<host> is not one of GitHub's asset hosts` |
| MSG-INSTALL-CANCELLED | The install was cancelled. | Settings, CLI (Ctrl-C during `install`, exit 1; the partial stays and the next install resumes from it) |
| MSG-WRITE | The model could not be written: \<error>. | CLI install and `install --from`, for a write that failed with the volume not full (a full one gives MSG-DISK) |
| MSG-INSTALL-FAILED | The install failed: \<detail>. | CLI install; \<detail> is a fixed phrase: `no HTTP client (<error>)`, `a catalog URL does not parse`, or the error of a task of the installer's own that failed |
| MSG-UNSAFE-FOLDER | Nothing was installed in the shared model folder, because \<path> \<problem>. Make each folder on that path a real folder that only root or an Administrator can change, then try again. | `install --system` and `remove --system` (exit 2); \<problem> is `is not owned by root`, `can be changed by accounts other than root`, `is a link or a junction` or `is not a folder` |
| MSG-READ | \<path> could not be read: \<error>. | `install --from` (exit 1) |
| MSG-REMOVE-BUSY | \<model> is being installed by another Biorouter process; try again when it finishes. | CLI `remove` while another process holds the model's lock (exit 1) |
| MSG-REMOVE-FAILED | \<model> could not be removed: \<error>. | CLI `remove` (exit 1) |
| MSG-SYSTEM-ROOT | Installing into the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation install --system`, or leave out --system to install for your account. | CLI `install --system` (exit 2). This row and the rest of PR-11's CLI rows below are in `crates/biorouter-cli/src/commands/dictation.rs`, pinned by `the_model_command_sentences_are_the_catalogs` there |
| MSG-SYSTEM-ROOT-REMOVE | Removing a model from the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation remove --system`. | CLI `remove --system` (exit 2) |
| MSG-NO-SHARED-FOLDER | This computer has no shared model folder. | CLI `--system` on a platform that has none (exit 2) |
| MSG-NO-RECOMMENDED | This computer has no recommended speech model. | CLI `install` with no model named, on a host class with no default (exit 2) |
| MSG-DETECTOR-ONLY | \<model> is installed with every speech model; install a speech model instead. | CLI `install` given the voice detector's id (exit 2) |
| MSG-DETECTOR-NEEDED | \<model> needs \<detector>; it is removed with the last speech model that needs it. | CLI `remove` given the voice detector's id while a speech model in that folder is there or is being installed by another process (exit 2) |
| MSG-DETECTOR-MISSING | \<model> needs the voice detector \<detector>, which is not installed on this computer. Run `biorouter dictation install <model>`. | CLI `use` (exit 2), `transcribe` with or without `--model` (exit 2, `error_code` `not_installed`) and `warm-up` (exit 1) for a speech model whose detector is gone; the daemon answers it as `409 noModel`, as MSG-NO-MODEL; `install <model>` then fetches only the detector. The text is `DictationError::DetectorNotInstalled` in `crates/biorouter/src/dictation/service.rs`, pinned by `the_model_sentences` there |
| MSG-CHANGE-SHELL | Change speech models yourself: run this in a terminal, or use Settings > Models > Speech models. | CLI `remove` and `use` under `BIOROUTER_TERMINAL=1` (exit 2) |
| MSG-CONFIRM-TERMINAL | Installing a speech model needs your confirmation at a terminal. To install without one, run: biorouter dictation install --yes \<model> (for an import, `--from <path>` in place of \<model>; for `remove`: Removing a speech model needs your confirmation at a terminal. To remove it without one, run: biorouter dictation remove --yes \<model>) | CLI without a terminal and without `--yes` (exit 2) |
| MSG-CONFIRM-INSTALL | Download and install \<model>? | CLI `install`, at a terminal, after the heading **Install a speech model** and one line per file (`<id>  <size> B  sha256 <digest>`), the source and the target folder with its account |
| MSG-TOO-SLOW | \<model> is too slow for dictation on a computer with \<n> cores; after you install it, `biorouter dictation bench` measures it. Install it anyway? | CLI `install` before a bench exists, in place of MSG-TURBO-CPU |
| MSG-BELOW-MINIMUM | \<model> needs at least \<n> GB of memory, and this computer has less. Install it anyway? | CLI `install` |
| MSG-ALREADY | \<model> is already installed (\<location>, \<path>). (for `--system` and `--from`: \<model> is already installed in \<folder>.) | CLI `install` |
| MSG-INSTALLED | Installed \<model> (\<path>). | CLI `install` and `install --from` |
| MSG-SUGGEST-KEPT | This install keeps your earlier choice about suggesting dictionary words from your chats; to change it, run `biorouter dictionary settings --suggest-from-chats` with on or off, or use Suggest dictionary words from my chats on the Dictionary page. | CLI `install` and `install --from` on stderr, after a verified install given `--suggest-from-chats` or `--no-suggest-from-chats` once the activation marker records a choice (DD-56); the flag changes nothing then |
| MSG-WARMING | Preparing \<model> for its first use in the background. | CLI `install` on a Metal host |
| MSG-WARM-UP-SYSTEM | Each account's first dictation prepares \<model> for that account. To prepare it now, run `biorouter dictation warm-up <model>` as that account. | CLI `install` with `--system`, or run as root or Administrator, on a Metal host: the shaders compile into each account's own Metal cache, so nothing is started |
| MSG-RETIRED | \<id> is retired; Biorouter uses \<model> in its place. | Every CLI model command given a retired id |
| MSG-IMPORT-CONFIRM | Install the speech model files in \<from> into \<folder>? | `install --from`, at a terminal |
| MSG-IMPORT-REFUSED | \<path> is \<model>, which was not installed: \<the policy's sentence> | `install --from` when `allowed_models` leaves the model out |
| MSG-IMPORT-UNKNOWN | \<path> is not a Biorouter model file, so it was not installed. | `install --from` |
| MSG-IMPORT-NONE | No file in \<from> matches a Biorouter model. | `install --from` (exit 1) |
| MSG-WAITING-LOCK | Another Biorouter process is installing \<model>; waiting for it to finish. | CLI `install` |
| MSG-PROGRESS | Downloading \<model>… · Checking \<model> against its pin… · Downloaded \<model> | CLI `install`: the plain lines without a terminal, and the progress bar's label and last line on one |
| MSG-REMOVE-CONFIRM | Remove \<model> from \<folder>? | CLI `remove`, at a terminal; the dialog of Settings' **Remove** for a model that is not in use, with the model's display name and `status.models_dir` |
| MSG-REMOVE-CONFIRM-ACTIVE | Remove \<model> from \<folder>? It is the model in use, so dictation will switch to \<next>. | The dialog of Settings' **Remove** for the model in use, which the daemon replaces at once: \<next> is the display name of the model `active_model` in `crates/biorouter/src/dictation/service.rs` picks next (this computer's default, then the first usable model in catalog order), as `modelAfterRemoving` in `speechModelStore.ts` reads it from the status (PR-18) |
| MSG-REMOVE-CONFIRM-LAST | Remove \<model> from \<folder>? It is the model in use and no other installed model can take its place, so dictation will not work until you install one. | As MSG-REMOVE-CONFIRM-ACTIVE, when no other installed model that managed policy admits can take its place (PR-18) |
| MSG-REMOVED | Removed \<model> from \<folder>. | CLI `remove` |
| MSG-REMOVE-ELSEWHERE | \<model> is installed in the \<location> folder (\<path>), not in \<folder>. | CLI `remove` (exit 1) |
| MSG-REMOVE-MISSING | \<model> is not installed. | CLI `remove` (exit 1) |
| MSG-USE | Dictation now uses \<model>. | CLI `use` |
| MSG-USE-NOT-SAVED | The choice could not be saved: \<error>. | CLI `use` (exit 1) |
| MSG-VERIFY | `ok      <model> (<location>, <path>)`, with `, an earlier revision` for a superseded copy · `FAILED  <model> (<location>, <path>): the file does not match its pin`, with `, so it was deleted` in this account's folder · `FAILED  <model> (<location>, <path>): <error>` | CLI `verify`, one line per file |
| MSG-VERIFY-FAILED | \<n> model files failed verification. (for one: 1 model file failed verification.) | CLI `verify`, after the lines of MSG-VERIFY when any failed; it ends the command with exit 1 |
| MSG-NOTHING-INSTALLED | No speech model is installed. | CLI `verify` with nothing to check |
| MSG-BENCH | \<model> on \<backend>, \<n> threads: \<a> s of speech in \<t> s (real time factor \<r>, \<suitability>). | CLI `bench`; Settings **Test speed**, under the **Speed** row, with the model's display name, `Metal` or `CPU`, and the suitability in words; only for the model in use, and not while a new run is in progress |
| MSG-RESCAN-ADDED | Installed \<models> from the model folder. | Settings after **Rescan** placed files; \<models> are display names joined with `, ` (PR-18) |
| MSG-RESCAN-NONE | No new model file was found in \<folder>. | Settings after **Rescan** placed nothing and found nothing it could not identify |
| MSG-RESCAN-UNKNOWN | Not Biorouter model files, so not installed: \<names>. | Settings after **Rescan**, for the file names no pin matched (`unrecognized`), after MSG-RESCAN-ADDED when both apply |
| MSG-SETTINGS-REFUSED | The Biorouter service refused this (\<code>, HTTP \<status>). | Settings, for a refusal of a model route whose body carries no sentence (`installBusy`, `notDeletable`, `badRequest`); a refusal with a sentence shows that sentence |
| MSG-SETTINGS-UNREACHABLE | The Biorouter service did not answer. Try again. | Settings, when a model route or the status read got no answer (button **Retry** for the status read) |
| MSG-SETTINGS-HELP | Models that turn your speech into text for dictation. Each one downloads once, from the Baranzini Lab's model repository on GitHub. | Settings, the InfoTip of the **Speech models** heading (PR-18) |
| MSG-FOLDER-HELP | Where this account's speech models are kept. On a computer without internet access, copy the model files here and choose Rescan. | Settings, the InfoTip of the **Model folder** row |
| MSG-LANGUAGE-HELP | The language you dictate in. Detect automatically lets a multilingual model decide each time. | Settings, the InfoTip of the **Language** row |
| MSG-MIC-HELP | The microphone this window records from. Test microphone shows its level for 5 seconds; nothing is recorded or sent. | Settings, the InfoTip of the **Microphone** row |
| MSG-SPEED-HELP | Transcribes a 5.5 s sample with the model in use. A model that needs more than half the sample's length is marked slow. | Settings, the InfoTip of the **Speed** row |
| MSG-LIST-NOTES | recommended for this computer · installed (\<location>, \<path>) · not installed · in use · a newer revision exists · not allowed by your organization · required · retired model · not a Biorouter model file | CLI `list`, under the heading **Speech models**, the notes after each row |
| MSG-SERVER-MEMORY | This server does not have enough free memory to load \<model> (\<x> GB needed, \<y> GB free). Try again later, or ask the administrator to install the Light model. | Composer card on S5 to S8 and S17 |
| MSG-ENGINE-FAILED | Transcription failed on \<where>, so no text was produced. Your recording is kept for 5 minutes. (button **Retry**) | Composer card, clipboard review card; \<where> is "this computer" or "the Biorouter server at \<host>" |
| MSG-ENGINE-FAILED-CLI | Transcription failed. Type /dictate retry within 5 minutes to try the same recording again. | TUI, REPL, shared prompt; `dictate` prints the first sentence and exits 1; `transcribe` prints MSG-TRANSCRIBE-FAILED, which starts with it, and exits 1 |
| MSG-TRANSCRIBE-NOTHING | No speech was heard in that recording, so nothing was transcribed. | `transcribe` (exit 3, `no_speech`): no speech, an echo only, or exact zeros. This row and the seven below it are in `crates/biorouter-cli/src/dictation/messages.rs`, pinned by `the_transcribe_sentences_are_the_catalogs` there |
| MSG-TRANSCRIBE-CANCELLED | Transcription was cancelled. | `transcribe` stopped with Ctrl-C (exit 4) |
| MSG-TRANSCRIBE-TOO-LONG | That recording is longer than 2 hours, or larger than 200 MB on standard input. Split it into 16 kHz mono parts first, for example: ffmpeg -i \<file> -ar 16000 -ac 1 -f segment -segment_time 3600 part%02d.wav | `transcribe` (exit 1, `too_long`); \<file> is the file as named, and stays as written for standard input. Each part, an hour of 16 kHz mono, is about 115 MB, under both limits |
| MSG-TRANSCRIBE-UNSUPPORTED | Biorouter cannot read \<kind> recordings. Convert it to WAV first, for example: ffmpeg -i \<file> -ar 16000 -ac 1 \<name>.wav (with no kind to name: Biorouter cannot read this recording. … ; on standard input the example is `ffmpeg -i <file> -ar 16000 -ac 1 -f wav - \| biorouter transcribe -`) | `transcribe` (exit 1, `unsupported_format`); \<kind> is `Opus`, `WebM` or the file's extension, such as `.txt` (DD-78; M4A, AAC and Ogg Vorbis are read since O-39, and Apple Lossless since O-41), and no kind is named for a file whose extension is one Biorouter reads (`.wav`, `.aiff`, `.flac`, `.mp3`, `.m4a`, `.mp4`, `.aac`, `.ogg` and their variants) but whose contents it cannot, such as an ADPCM WAV from a voice recorder or a damaged `.m4a`; \<name> is the file's name without its extension, with `-16k` added when that extension is `.wav`, so the example never writes over the file it reads |
| MSG-TRANSCRIBE-NOT-FOUND | There is no file at \<file>. | `transcribe` (exit 1, `not_found`) |
| MSG-TRANSCRIBE-UNREADABLE | \<file> could not be read. (on standard input: Standard input could not be read.) | `transcribe` (exit 1, `unreadable`) |
| MSG-TRANSCRIBE-UNWRITABLE | The transcript could not be written to \<path>. (to standard output: The transcript could not be written to standard output.) | `transcribe --output`, and a failed write to standard output (exit 1, `unwritable`); a reader that closed the pipe early is not a failure |
| MSG-TRANSCRIBE-FAILED | Transcription failed. \<detail> | `transcribe` (exit 1, `engine_failed`): MSG-ENGINE-FAILED-CLI's first sentence, then the engine's diagnostic, one of MSG-DIAG-EXIT to MSG-DIAG-CANCELLED, which names no transcript and no audio |
| MSG-LOST | Dictation lost its connection to Biorouter, so nothing was inserted. Try again. | Composer card: a second failure following a 410, the sixth timeout in a row on one chunk, the fourth network error, 5xx or 429 in a row, or no answer at all; shown when it happens, also while recording |
| MSG-CAP-WARN | Dictation stops in 30 seconds. What you said so far will be inserted. | Composer card and the terminal front ends, at 570 s |
| MSG-CAP-STOPPED | Dictation stopped at the 10 minute limit and inserted what you said. | Composer card and the terminal front ends, at 600 s |
| MSG-PAUSED | Dictation failed three times in a row, so it is paused for this session. Type /dictate status to see why. | Terminal front ends, after three failures within 10 s |
| MSG-KEY-CLASH | Ctrl+\<key> dictation is off because BIOROUTER_CLI_NEWLINE_KEY uses the same key. Set BIOROUTER_CLI_DICTATION_KEY to another letter. | Terminal front ends, one note at start |
| MSG-REMOTE-OLD | The Biorouter service on \<host> is a version without local dictation. Update it to \<this app's version> or later, or turn off Use external server in Settings > App. | S17: microphone tooltip and the **Dictionary** page when `/dictation/status` or `/dictionary/settings` answers 404 or 405 |
| MSG-MIC-DENIED-MAC | Microphone access is off for Biorouter. Turn it on in System Settings > Privacy & Security > Microphone, then restart Biorouter. (button **Open settings**) | Composer card, clipboard review card (macOS) |
| MSG-MIC-DENIED-WIN | Microphone access is off for Biorouter. Turn on “Let desktop apps access your microphone” in Settings > Privacy & security > Microphone. (button **Open settings**) | Composer card, clipboard review card (Windows) |
| MSG-MIC-RESTRICTED | Your organization manages microphone access on this computer. | Composer card |
| MSG-MIC-GRANTED-FAILED | Biorouter could not open the microphone although macOS allows it. Restart Biorouter and try again. | Composer card (macOS) |
| MSG-MIC-BROWSER | Allow the microphone for this site in your browser's site settings, then try again. | Composer card on `serve` |
| MSG-NO-DEVICE | No microphone found. Connect one or make it the system default, then try again. | Composer card, review card, CLI |
| MSG-MIC-BUSY | The microphone is in use by another app, or the system blocked it. | Composer card, review card |
| MSG-INSECURE | Dictation needs a secure address. Your browser blocks the microphone on http://\<host>. Open Biorouter through an SSH tunnel (then it is localhost) or through an HTTPS address. | Microphone tooltip on an insecure page |
| MSG-SHARED-ORIGIN | Your browser's microphone permission covers every page at \<origin>, including pages that code or an agent wrote there, such as Biorouter apps and notebook outputs. Choose Allow this time unless you trust every page there. | Once per origin before the first recording, on every `serve` origin (SD-17) |
| MSG-413 | The proxy in front of Biorouter refused the audio upload (413). Its request size limit is below 64 KB. | Composer card |
| MSG-WAIT | Waiting for a free connection to Biorouter | Composer card after 1.5 s without an answer |
| MSG-SERVER-BUSY | The server is busy transcribing for other people. Trying again in \<n> seconds. | Composer card while a 429 at stream creation is waited out |
| MSG-SERVER-BUSY-FAILED | The server is still busy transcribing for other people, so nothing was inserted. Your recording is kept for 5 minutes. (button **Retry**) | Composer card, when stream creation still answers 429 30 s after the stop |
| MSG-RECORDING-FAILED | Biorouter could not read the recording, so nothing was inserted. Try again. | Composer card, when the `MediaRecorder` fallback cannot decode what it recorded |
| MSG-NOTHING | Nothing was heard, so nothing was inserted. | Composer card |
| MSG-SILENT | The microphone sent only silence. Check that it is not muted. | Composer card; CLI on Linux |
| MSG-NO-AUDIO-YET | No audio detected. Check that your microphone is not muted. | Composer card and TUI while recording; Settings' **Test microphone** when its 5 s heard nothing above about -40 dBFS |
| MSG-TOO-SHORT | That recording was under half a second, so nothing was transcribed. | Composer card, CLI |
| MSG-AGENT-SHELL | Dictation records your microphone, so only you can start it, from your own terminal. Biorouter does not start it from an agent's shell. | CLI step 3 (exit 2) |
| MSG-NO-TTY | Live dictation needs an interactive terminal. To transcribe a recording instead, run: biorouter transcribe \<file> | CLI step 4 (exit 2) |
| MSG-SSH-MAC | macOS only lets apps in the logged in desktop session use the microphone, and this is an SSH session. | CLI step 5 |
| MSG-SSH-ROUTES | This computer has no microphone you can reach over SSH. To dictate with this computer's model and dictionary, run `biorouter serve` here and keep it running, then on your laptop run `ssh -N -L 8765:127.0.0.1:8765 <host>` and open the address `serve` prints, including its ?t= part. Or dictate on your own computer with `biorouter dictate --copy` and paste, or copy a recording here and run `biorouter transcribe <file>`. | CLI over SSH |
| MSG-SSH-SERVICE | A Biorouter service already runs on this computer, on port \<port>. To dictate with it, on your laptop run `ssh -N -L <port>:127.0.0.1:<port> <host>` and open http://127.0.0.1:\<port>/?t=\<token>, with the token from whoever runs the service. Or dictate on your own computer with `biorouter dictate --copy` and paste, or copy a recording here and run `biorouter transcribe <file>`. | CLI over SSH, in place of MSG-SSH-ROUTES, when the `/proc` scan finds a `biorouter serve` process; \<port> from its command line, \<token> left as written |
| MSG-SYNC-HINT | To use your laptop's dictionary terms here, run on your laptop: `biorouter dictionary export - \| ssh <host> biorouter dictionary import -` | CLI over SSH, after the routes |
| MSG-CONTAINER | This container has no microphone. Transcribe a recording with `biorouter transcribe <file>`, or run `biorouter serve` and dictate from your browser. | CLI in a container (`/.dockerenv`, `/run/.containerenv`, `KUBERNETES_SERVICE_HOST`, or a container runtime in `/proc/1/cgroup`) |
| MSG-LINUX-NOAUDIO | No audio system is running here (no PulseAudio, PipeWire or ALSA capture device). | CLI on Linux without a sound server |
| MSG-LINUX-ROUTES | To dictate with this computer's model and dictionary, run `biorouter serve` here and open it from your own computer through an SSH tunnel or an HTTPS address, or transcribe a recording with `biorouter transcribe <file>`. | CLI on Linux without a sound server, outside SSH, a container and WSL, always after MSG-LINUX-NOAUDIO |
| MSG-WSL2-NOPULSE | WSL delivers audio through WSLg's PulseAudio server. Install the PulseAudio client library (`sudo apt install libpulse0`), then try again. | CLI in WSL2 with WSLg, when `libpulse.so.0` is missing |
| MSG-WSL2-NOWSLG | This WSL has no audio, because WSLg is not available or is turned off. Use Biorouter on Windows, run `biorouter serve` here and open the address it prints in a Windows browser, or transcribe a recording with `biorouter transcribe <file>`. | CLI in WSL2 without `/mnt/wslg/PulseServer` |
| MSG-WSL1 | WSL1 has no audio. Use Biorouter on Windows, or WSL2 with WSLg. | CLI in WSL1 |
| MSG-TCC-ZERO | macOS gave Biorouter only silence, which usually means \<terminal app> is not allowed to use the microphone. Turn it on in System Settings > Privacy & Security > Microphone, then quit and reopen it. If it is not listed: `tccutil reset Microphone <bundle id>`. If \<terminal app> never asks, it cannot use the microphone on macOS: run Biorouter in Terminal, iTerm2, Ghostty or VS Code instead, or use Dictate to clipboard in the Biorouter app. | CLI on macOS, step 10 |
| MSG-WIN-ZERO | Windows gave Biorouter only silence. Turn on “Let desktop apps access your microphone” in Settings > Privacy & security > Microphone. | CLI on Windows, step 10 |
| MSG-ACP-NOTE | (An audio clip was attached here. Biorouter does not take audio from this editor, so the clip was left out. To dictate, use Dictate to clipboard in the Biorouter app or run `biorouter dictate --copy`, then paste. Your system's own dictation also works, without your Biorouter dictionary.) | In an ACP turn |
| MSG-ACP | Editors that connect over ACP do not send audio, so there is no microphone button in their chat. To dictate with Biorouter's speech model and dictionary, use Dictate to clipboard in the Biorouter app, or run `biorouter dictate --copy` in a terminal, then paste. Your system's own dictation also works, without your Biorouter dictionary. | `docs/cli/command-reference.md` |
| MSG-ACP-DICTATE | To dictate, use Dictate to clipboard in the Biorouter app or run `biorouter dictate --copy`, then paste. Your system's own dictation also works, without your Biorouter dictionary. | The answer to the editor's `/dictate` command, with no model turn |
| MSG-APPS | Apps cannot use the microphone themselves. Click into the app's text field, then use Dictate into this app, or use Dictate to clipboard in the Biorouter app (or `biorouter dictate --copy`) and paste. Your system's own dictation also works, without your Biorouter dictionary. | SDK reference (the preview button is absent while nothing is focused, so no tooltip carries it) |
| MSG-APPS-NO-PREVIEW | Apps cannot use the microphone themselves. Use Dictate to clipboard in the Biorouter app (or `biorouter dictate --copy`) and paste. Your system's own dictation also works, without your Biorouter dictionary. | SDK reference, in place of MSG-APPS if M-15 fails |
| MSG-APPS-BROWSER | Apps cannot use the microphone. To dictate here, dictate in Biorouter's own composer and copy the text, or use your system's own dictation. | SDK reference |
| MSG-GNOME | GNOME on Wayland does not let Biorouter set a keyboard shortcut that works in other apps. Use Dictate to clipboard in Biorouter's tray menu (GNOME shows it with the AppIndicator extension), or the microphone button in Biorouter's own composer. | Settings, in place of the shortcut picker |
| MSG-WINDOW-OPEN | On Windows and Linux, Dictate to clipboard works while a Biorouter window is open. It can be minimized. | Settings, under the clipboard switch |
| MSG-SHORTCUT-TAKEN | Another app is already using \<shortcut>. Choose a different shortcut. | Settings |
| MSG-NO-TRAY | Biorouter's menu bar icon is off, so Dictate to clipboard is not in its menu. Choose a shortcut below, or turn on Show in menu bar in Settings > App. | Settings, under **Dictate into other apps**, while the tray is absent |
| MSG-CLIPBOARD-EXPLAIN | Dictate to clipboard puts the text on your system clipboard. After that it is outside Biorouter: any program on this computer can read it, including an agent with shell access in any chat; clipboard managers may keep it; Universal Clipboard or Windows cloud clipboard may sync it to your other devices; and the app you paste into decides where it goes. Your private dictionary terms are used only for a recording you start with Include my private terms. | Settings, once, when the switch turns on |
| MSG-CLIPBOARD-COPIED | Copied \<n> words. Paste with \<key>. | Notification |
| MSG-CLIPBOARD-NOTHING | Nothing was heard, so the clipboard was not changed. | Notification |
| MSG-CLIPBOARD-FAILED | Biorouter could not finish dictation. Try again. | The **Dictate to clipboard** review card, when a capture fails with no mapped sentence; hard coded in `main.ts` (`showAnywhereReview`). After a transcription failure the stream can send again, with **Retry** while the recording is held (5 minutes); without **Retry** otherwise |
| MSG-CLIPBOARD-ECHO | Only dictionary words came back, which usually means the recording was silent. Nothing was copied. | Notification, external clipboard stream with `dictionary_only: true` |
| MSG-CLIPBOARD-COPY-FAILED | The clipboard did not keep the text. Select it above and press \<key>. | The **Dictate to clipboard** review card, when its **Copy** finds the clipboard does not hold the text; \<key> is ⌘C or Ctrl+C |
| MSG-NO-CLIPBOARD | No clipboard is reachable here, so the text is printed below. Copy it from the terminal. | `dictate --copy` (exit 5) |
| MSG-CLIPBOARD-WAIT | Copied. Waiting up to 60 s for you to paste… | `dictate --copy` on Linux |
| MSG-REMOTE-HTTP | Dictation sends your voice to \<host> without encryption, as your typed messages already are. Use an https address for this server to protect it. Continue? | S17, once per `http:` backend URL |
| MSG-REMOTE-NO-KEY | Dictation on \<host> uses only your public dictionary terms until you enter this server's User Action Key in Settings > App > Use external server. Installing models there also needs the key. | S17, keyed backend without the key (O-17) |
| MSG-DISCLOSE-LOCAL | Speech is transcribed by Biorouter on this computer. Audio is not saved. | Settings, popover (the desktop's own daemon) |
| MSG-DISCLOSE-REMOTE | Audio is transcribed on \<host>, the computer running Biorouter's service. Models install there, not on this device. | Settings, popover, `ExternalBackendSection.tsx` (S17) |
| MSG-DISCLOSE-HOST | Your voice is sent to the Biorouter server on \<host> and turned into text there. | Settings, popover, and once in the card (`serve`); the one line shown at rest |
| MSG-DISCLOSE-HOST-DETAIL | Biorouter does not save it or send it anywhere else. The text appears here for you to check before you send it. | The InfoTip beside MSG-DISCLOSE-HOST (owner rule "no walls of text", Phase C ruling F-13, 2026-11-14) |
| MSG-SETUP | Dictation turns your speech into text \<where>. It needs a speech model: \<name> (\<size>), downloaded once from the Baranzini Lab's model repository on GitHub into \<folder>. Audio is not saved. (where is "on this computer" or "on the Biorouter server at \<host>"; buttons **Install**, **Not now**; link **More in Settings**) | Setup popover |
| MSG-SETUP-SUGGEST | The agent can propose terms it sees in your chats. A term can go into your dictionary when you ask the agent to add it, or reply to its offer naming that term, even if you decline; everything else waits for you to accept it. (the InfoTip of **Suggest dictionary words from my chats**, which starts ticked, O-33, by the owner's rule of 2026-11-13 against walls of text; the sentence on the chat extractor it used to carry moved to that feature's own switch, **Suggest words from my messages**; the last sentence above is the `on_request` variant, the default the owner decided on 2026-11-05 (O-3, O-34); when `agent_writes` is `suggest` the last sentence reads "Nothing is added until you accept it.", and when it is `add` it reads "The agent's terms are added at once, and you can undo each one; everything else waits for you to accept it.") | Setup popover, Settings |
| MSG-PRIVATE-DRAFT | This message was dictated with your private dictionary terms turned on, so it may contain them, and \<model> is a public model. Send it anyway? (buttons **Send anyway**, **Cancel**) | Composer, before a send, a queue or a steer |
| MSG-PRIVATE-DRAFT-SWITCH | This draft was dictated with your private dictionary terms turned on, so it may contain them. If you switch this chat to \<model>, a public model, the draft goes there with your next message. Switch anyway? (buttons **Switch anyway**, **Cancel**) | Model switcher |
| MSG-PRIVATE-TERMS | Your private dictionary terms may appear in this text, and Biorouter cannot keep them away from public models or from other institutions' models once you paste or post it. Include them? (buttons **Include**, **Public terms only**; at a terminal, `[y/N]`) | After **Include my private terms** (Crew, **Dictate into this app**, the tray) and `biorouter dictate --private-terms`; one recording |
| MSG-PRIVATE-TERMS-REFUSED | Private dictionary terms need you at your own terminal and a private model configured on this computer, so `--private-terms` was refused. Run the command without it to dictate with public terms. | `biorouter dictate --private-terms` when DD-77 does not admit it (exit 2) |
| MSG-PUBLICIZE-WARN | This term came from a private chat. Making it public lets it appear when you dictate into chats on public models. (buttons **Make public**, **Cancel**) | Dictionary page; CLI `dictionary publicize`, on stderr before the typed term; only for a private row that shows it came from a chat: a row from the agent or the chat extractor, or a person's add or a learned correction that names its chat (`suggested_in`) or one or more institutions, which only a chat's stamp gives such a row, and never a row the person made private (`provenance: privatized_by_user`) or imported (PR-14b; since PR-16 the rule is `Entry::came_from_a_chat` in `crates/biorouter-dictation/src/store/schema.rs`, which the CLI calls and the routes report as `EntryView.came_from_a_chat` to a caller with private standing) |
| MSG-PUBLICIZE-WARN-NO-CHAT | This term is private. Making it public lets it appear when you dictate into chats on public models. (buttons **Make public**, **Cancel**) | As MSG-PUBLICIZE-WARN, for every other private row: one the person added as private with no chat, made private, or imported, and one whose chat cannot be told from its fields (PR-14b) |
| MSG-ROW-PRIVATE | Used only in private chats | Dictionary page, the start of line two of a private row with no institution: a term added with no chat, or from a private chat whose model and connectors name no institution (O-6, O-32, O-35, O-36) |
| MSG-ROW-PRIVATE-INSTITUTION | Used only in private chats on \<institution> models and on local models (a row that names two or more institutions reads "Tied to \<institutions>; used only in private chats on local models") | Dictionary page, the start of line two of a private row that carries institutions; \<institution> is the institution's published name (else its id); in the two or more form, \<institutions> is each institution's published name (else its id) joined with "and" (O-32, O-35) |
| MSG-ROW-PRIVATE-LOCAL | Used only in private chats on local models | Dictionary page, the start of line two of a private row whose institution list is empty: an import that could not be read, a write whose chat union named more than 8 institutions, or a write whose chat record or chat model could not be read (O-32, O-35) |
| MSG-EXPORT-PRIVATE | This file leaves Biorouter. It will include your private terms. (buttons **Export with private terms**, **Export public terms only**) | Dictionary page export; CLI `dictionary export --include-private`, with the same two answers |
| MSG-DICTIONARY-AGENT | Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms. | Dictionary route refusals to an agent; CLI dictionary writes, `import` included, under `BIOROUTER_TERMINAL=1` (exit 2) |
| MSG-DICTIONARY-HOST-TIER | Do this on the server: `biorouter dictionary <verb> <term>` (as the service user). | Dictionary page on `serve`, before the click; \<verb> is `publicize` for **Make public…** and `privatize` for **Make private…**; the route's 403 leaves \<term> as written |
| MSG-DICTIONARY-HOST-SETTINGS | Do this on the server: `biorouter dictionary settings <option>` (as the service user). | Dictionary page on `serve`, before the click, for raising **Agent's terms** or turning chat learning or suggestions on; \<option> is `--agent-writes <mode>`, `--learn-from-chats on` or `--suggest-from-chats on`, from the control; `authority.raise_settings.reason` |
| MSG-SCAN-OFF | Turn on Suggest words from my messages first, then find words in your recent chats. | `POST /dictionary/suggestions/scan` and `biorouter dictionary suggestions scan` while `learn_from_chats` is off (409 `suggestions_off`, exit 1); `suggestion_scan::MSG_SCAN_OFF` |
| MSG-SCAN-POLICY | Your organization turned off adding words to the dictionary automatically. | The same, under managed `dictionary: false` or `dictionary_agent_writes: false` (403 `policy`) |
| MSG-SCAN-UNREADABLE | Biorouter could not read your dictionary, so it did not look for words. | The same, when the store cannot be read (503 `unreadable`) |
| MSG-NO-SUGGESTIONS | No suggestions to review. | `biorouter dictionary suggestions` at a terminal when nothing waits (stderr) |
| MSG-NO-SUGGESTION | There is no suggestion \<name> to review. | `biorouter dictionary suggestions keep` or `dismiss` for a name or id that matches no waiting row (exit 1) |
| MSG-SCAN-TURN-ON | To turn it on here, run `biorouter dictionary settings --learn-from-chats on`. | `biorouter dictionary suggestions scan`, under MSG-SCAN-OFF |
| MSG-SCAN-REVIEW | Review them with `biorouter dictionary suggestions`. | `biorouter dictionary suggestions scan`, after a scan that found words |
| MSG-SCAN-REVIEW-IN-SESSION | Review them with /dictionary review. | `/dictionary scan` in the TUI or the classic chat, after a scan that found words |
| MSG-SCAN-STARTED-IN-SESSION | Looking for words in your recent chats. Esc stops it. | The TUI's note for `/dictionary scan`, replaced in place by MSG-SCAN-PROGRESS as it reads |
| MSG-DICTIONARY-HOST-PRIVATE | Private terms are added on the server: `biorouter dictionary add --private <term>` or `biorouter dictionary import <file>` (as the service user). | Dictionary page on `serve` without private standing, before the click on **Only in private chats**; the 403 for a private add or import |
| MSG-DICTIONARY-FILE | The dictionary file at \<path> could not be read, so nothing was changed. Fix or remove it, then try again. | Dictionary page, CLI |
| MSG-NOT-FOUND | No dictionary entry with that id. | Dictionary routes; CLI `--id`, `accept` and `dismiss` (exit 1) |
| MSG-SAME-LETTERS | A term with the same letters exists: \<written> | Term dialog, CLI add |
| MSG-TERM-TOO-LONG | Use a term of at most 64 characters. | Term dialog, dictionary routes, CLI add (code `too_long`) |
| MSG-TERM-WORDS | Use a term of at most six words. | As above (code `too_many_words`) |
| MSG-TERM-PLAIN | Use one line of plain text. | As above (codes `line_break`, `control_character`, `angle_bracket`) |
| MSG-TERM-ADDRESS | A dictionary term cannot be a web address, an email address or a file path. | As above (codes `url`, `email`, `path`) |
| MSG-TERM-IDENTIFIER | This looks like an identifier or a key, so it was not added. | As above (codes `digits_only`, `long_hex`, `secret_shaped`) |
| MSG-TERM-FORMS | Use at most eight spoken forms and sixteen misrecognitions for a term. | As above (code `too_many_forms`, also for more than eight context cues) |
| MSG-DICTIONARY-CAP | Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one. | As above (code `cap`) |
| MSG-DICTIONARY-STALE | This term changed since you opened it. Reload it and try again. | As above (code `stale`) |
| MSG-DICTIONARY-DUPLICATE | That spelling is already a term here, so the edit was not made. | Term dialog, dictionary routes, CLI `dictionary edit` (exit 1): an edit to a spelling another row of the same tier has |
| MSG-DICTIONARY-WRITE | The dictionary could not be written: \<error>. | Dictionary routes, CLI: a write that failed on disk; \<error> is the operating system's description, or `the store could not be reached` when the routes could not open the store at all (500 `write_failed`). This row and the rows above it from MSG-TERM-TOO-LONG, with MSG-DICTIONARY-FILE, MSG-NOT-FOUND, MSG-IMPORT-NOT-EXPORT, MSG-IMPORT-UNPARSABLE and MSG-IMPORT-ROW-UNREADABLE, are the store's text in `crates/biorouter-dictation/src/store/` |
| MSG-EMPTY | No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1. | Dictionary page empty state; CLI `dictionary list` at a terminal, on stderr |
| MSG-DICTIONARY-HELP | Words dictation should spell your way, such as names, gene symbols and acronyms. A private term is used only when you dictate into a private chat. | Dictionary page, the band's InfoTip; the **Dictionary** row in **Settings > Models > Speech models** (PR-20). This row and the rows below it to MSG-EXPORT-WRITE-FAILED are the page's own text in `ui/desktop/src/components/dictionary/copy.ts`, pinned by its `copy.test.ts` |
| MSG-DICTIONARY-NO-MATCH | No terms match this filter. | Dictionary page, a filter that matches nothing |
| MSG-DELETE-TERM | Dictation stops using \<term>, and Biorouter does not suggest it again. (buttons **Delete**, **Cancel**) | Dictionary page, before **Delete…** on a row; the delete writes a tombstone for the term's letters in its tier |
| MSG-DELETE-ALL | This deletes the \<n> terms listed here, and Biorouter does not suggest them again. It cannot be undone. (button **Delete all terms**, after typing `delete`) | Dictionary page, **Delete all terms…**; on `serve` it deletes only the rows that browser can list |
| MSG-DICTIONARY-TOO-LARGE | That list is larger than 1 MB or 5,000 rows. Split it and add each part. | **Paste a list…** and **Import a file…**, before the request when the text is over either limit, and for the route's 413 `too_large`, which carries no sentence |
| MSG-ADD-TERMS-HELP | Paste one term per line, such as UCSF or EPAS1 = epas one, or drop a .txt, .csv, .tsv or .json file. Terms you already have gain the new spoken forms; nothing is deleted. | **Paste a list…**, the field's InfoTip |
| MSG-IMPORT-PLAN | \<new> new · \<existing> already in your dictionary · \<skipped> skipped | **Paste a list…**, the daemon's dry run; \<existing> counts `merged` and `duplicate` together, \<skipped> the `invalid` rows |
| MSG-IMPORT-SKIPPED | Line \<line>: \<sentence> | **Paste a list…**, one per skipped row; \<sentence> is its code's row among MSG-TERM-TOO-LONG to MSG-TERM-FORMS, MSG-DICTIONARY-CAP or MSG-IMPORT-ROW-UNREADABLE |
| MSG-SPOKEN-FORMS-HELP | How you say the term, one way per line, at most eight. Leave it empty when dictation already hears the term. | Term dialog, the **Spoken as** InfoTip |
| MSG-MATCH-HELP | Also fix near misses corrects close misspellings, and Exact spelling only corrects only what you listed. Help recognition only never changes the text after transcription. | Term dialog, the **Match** InfoTip under **More options** |
| MSG-REVIEW-HELP | A kept word starts shaping dictation. A dismissed one is not suggested again. | **Review all…**, under its title |
| MSG-LEARN-FROM-CHATS-HELP | Biorouter looks for terms such as EPAS1 or UCSF in the messages you type in chats on public models, on this computer, and suggests a word once it shows up in two chats. You keep or dismiss each one, and a dismissed word is not suggested again. | Dictionary page, the InfoTip of **Suggest words from my messages** in the settings at the page's foot, and the same switch in **Settings > Models > Speech models** (user run suggestions, owner, 2026-11-13); `learn_from_chats` starts off until the person turns it on (O-44) |
| MSG-SCAN-PROGRESS | Reading chat \<read> of \<total>, \<words> found so far | Dictionary page, under the switch while **Find words in my recent chats** runs, beside **Stop**; \<read> is the chat being read, \<words> "1 word" or "12 words" |
| MSG-SCAN-FINISHED | Read \<chats>. \<words> to review. | Dictionary page, when the scan read every chat it meant to; \<chats> "40 chats", \<words> "3 new words" |
| MSG-SCAN-FINISHED-NONE | Read \<chats>. No new words found. | Dictionary page, when the scan ended with nothing new |
| MSG-SCAN-STOPPED | Stopped after \<read> of \<chats>. \<words> to review. | Dictionary page, after **Stop**; the words found until then are kept |
| MSG-SCAN-STOPPED-NONE | Stopped after \<read> of \<chats>. No new words found. | Dictionary page, after **Stop** with nothing new |
| MSG-SCAN-FAILED | Biorouter could not finish reading your chats. | Dictionary page, when the scan could not read the chats or write the dictionary; muted ink, not red |
| MSG-SUGGESTIONS-BACK-OFF | You dismissed most suggestions, so Biorouter now suggests fewer. (buttons **Turn off suggestions**, **Keep suggesting**) | Dictionary page, one neutral line above the suggestions with its two actions, while `suggestions.offer_turning_off` is true: at least 20 recent decisions, most of them **Dismiss** (owner rule of 2026-11-13: one line, the rest in an InfoTip); `biorouter dictionary suggestions` prints it with the command that turns suggestions off |
| MSG-SUGGESTIONS-BACK-OFF-HELP | A word now has to show up in \<n> chats before it is suggested. Turning suggestions off stops them until you turn them on again. | The InfoTip beside MSG-SUGGESTIONS-BACK-OFF, named "About fewer suggestions"; \<n> is `suggestions.required_chats` |
| MSG-PRIVATIZE-CONFIRM | Dictation then uses this term only when you dictate into private chats. (buttons **Make private**, **Cancel**) | Dictionary page, before **Make private…** on a public row; there is no warning the other way round, because a private term reaches fewer models |
| MSG-EXPORT-SAVED | Saved \<file> to \<folder>. | Dictionary page, after the export file is written through the app's Save window; a browser download claims nothing |
| MSG-EXPORT-WRITE-FAILED | The file could not be written to \<folder>. Nothing was saved. | Dictionary page, when the chosen file could not be written |
| MSG-DICTIONARY-TTY | Changing the dictionary needs you at an interactive terminal. To add terms from a file or a pipe, use `biorouter dictionary import`. | CLI: a dictionary write other than `import` without a terminal (exit 2). This row and the CLI rows below it to MSG-DICTIONARY-FOLDER are in `crates/biorouter-cli/src/commands/dictionary.rs`, pinned with the store's dictionary sentences by `the_dictionary_sentences_are_the_catalogs` there |
| MSG-EXPORT-PRIVATE-REFUSED | Exporting private terms needs you at your own terminal and a private model configured on this computer. Run the command without --include-private to export public terms. | CLI `dictionary export --include-private` where DD-77 does not admit it (exit 2) |
| MSG-SPOKEN-ONE-TERM | Use --spoken with one term at a time. | CLI `dictionary add` with several terms and `--spoken` (exit 2) |
| MSG-IMPORT-FORMAT | Name the format with --format txt, csv, tsv or json. | CLI `dictionary import` from standard input that is not an export, or from a file whose extension names no format (exit 2) |
| MSG-NO-TERM | There is no term \<term> in the dictionary. (naming a tier: There is no \<tier> term \<term> in the dictionary.) | CLI `dictionary edit`, `remove`, `publicize` and `privatize` (exit 1); \<tier> is `public` or `private`, the tier `publicize` or `privatize` moves a term from |
| MSG-TYPED-MISMATCH | That did not match \<term>, so nothing was changed. | CLI `dictionary publicize` and `privatize`, when the typed answer is not the term (exit 1) |
| MSG-IMPORT-NOT-FOUND | There is no file at \<file>. | CLI `dictionary import` (exit 1) |
| MSG-IMPORT-UNREADABLE | \<file> could not be read. (on standard input: Standard input could not be read.) | CLI `dictionary import` (exit 1) |
| MSG-IMPORT-TOO-LARGE | \<file> is larger than 16 MB; split it and import each part. (on standard input: Standard input is larger than 16 MB; split it and import each part.) | CLI `dictionary import` (exit 1) |
| MSG-IMPORT-NOT-TEXT | \<file> is not UTF-8 text. (on standard input: Standard input is not UTF-8 text.) | CLI `dictionary import` (exit 1) |
| MSG-IMPORT-NOT-EXPORT | That file is not a Biorouter dictionary export. | CLI `dictionary import` of JSON that is not an export (exit 1); the Dictionary page's import |
| MSG-IMPORT-UNPARSABLE | That file could not be read: \<detail>. | CLI `dictionary import` of CSV or TSV the reader cannot parse (exit 1); \<detail> is the CSV reader's own description |
| MSG-IMPORT-ROW-UNREADABLE | This row could not be read. | The import report's line for a row with code `unreadable`: a JSON entry that is not an object, or whose `written`, `tier`, `spoken_forms`, `misrecognitions` or `note` this version cannot read |
| MSG-IMPORT-REPORT | `Imported: <n> new, <n> merged, <n> duplicate, <n> invalid.` (with `--dry-run`, `Dry run:` in place of `Imported:`) · `line <n>: <code>: <sentence>` · `Private terms received: <n>.` | CLI `dictionary import`: one `line` per refused row, \<sentence> its row among MSG-TERM-TOO-LONG to MSG-TERM-FORMS, MSG-DICTIONARY-CAP or MSG-IMPORT-ROW-UNREADABLE; the last line without a terminal, counting the rows the file marks private, which are never reported as new or already present |
| MSG-EXPORT-UNWRITABLE | \<path> could not be written: \<error>. | CLI `dictionary export FILE` (exit 1) |
| MSG-CLEAR | This deletes \<n> terms from \<folder>. (for one: This deletes 1 term from \<folder>.) | CLI `dictionary clear`, on stderr, before the typed `delete <n>` |
| MSG-DICTIONARY-DONE | Added \<term> (\<tier>). · \<term> is already in the dictionary (\<tier>). · Changed \<term>. · Removed \<term>. · Accepted \<term>. · Dismissed \<term>. · \<term> is now \<tier>. · Kept \<n>. Dismissed \<n>. · Deleted \<n> terms. (for one: Deleted 1 term.) · Wrote \<path>. | CLI dictionary commands, one line per change; \<tier> is `public` or `private`. A command that names several terms prints each refused one as `<term>: <sentence>` and exits 1 |
| MSG-DICTIONARY-NOTHING | Nothing was changed. · Nothing was exported. · Nothing to review. · There are no terms to delete. | CLI dictionary commands, after a cancelled question or with nothing to do |
| MSG-DICTIONARY-ASK | \<term> is both a public and a private term. Which one? · Choose the terms to keep · Dismiss the rest? · Change this setting? · Type \<term> to make it \<tier>, or leave blank to cancel · Type delete \<n> to confirm, or leave blank to cancel | CLI dictionary questions at a terminal: twins, `review`, `settings` raising a setting, `publicize` and `privatize`, `clear` |
| MSG-DICTIONARY-FOLDER | Dictionary: \<folder> (account \<user>) | CLI, on stderr before every dictionary command, so stdout stays clean for `list` and `export` |
| MSG-TOOL-SUGGESTED | Recorded \<n> suggestions for the person's review in Dictionary. Nothing changes in dictation until they accept. | `dictionary__add_terms` result |
| MSG-TOOL-ADDED | Added \<n> terms to the dictionary. They are marked for the person's review. | `dictionary__add_terms` result |
| MSG-TOOL-EXISTS | 1 term was already in Dictionary. · \<n> terms were already in Dictionary. | `dictionary__add_terms` result for visible existing rows; singular for one, plural otherwise |
| MSG-TOOL-UNCHANGED | Some terms could not be recorded. | `dictionary__add_terms` result for skipped terms or existing rows hidden from the caller; names no row or count |
| MSG-TOOL-REVIEW-HINT | Review them with `biorouter dictionary review` on this computer, or in Dictionary in Biorouter. | Appended on ACP and the CLI |
| MSG-SERVE-TUNNEL | Browsers refuse the microphone and this page on http://\<lan addr>:\<port>. On your own computer run `ssh -N -L <port>:127.0.0.1:<port> <host>`, then open http://127.0.0.1:\<port>/?t=\<token>. | `serve` banner: a non loopback bind without `--public-url`, outside a container; the port, token and host filled in, and no `?t=` part under `--no-token` |
| MSG-SERVE-CONTAINER | This runs in a container. On the computer that publishes its port, open http://127.0.0.1:\<host port>/?t=\<token>, where \<host port> is the port published for container port \<port> (with Docker: -p 127.0.0.1:\<host port>:\<port>). Browsers refuse the microphone on other plain http addresses. | `serve` banner: a non loopback bind without `--public-url`, inside a container; the token and \<port> filled in, \<host port> left as written |
| MSG-CREW-TIP | Dictate. The text goes into the box for you to check; nothing is posted until you press Send. | Crew microphone tooltip |
| MSG-RESTART-BUSY | Dictation in progress. Restart anyway? | **Restart & update** while dictating |
| Existing 403, unchanged (`web_ui.rs` `insecure_address`) | `Open Biorouter through localhost or HTTPS. Use an SSH tunnel, or an HTTPS reverse proxy that passes the browser's Sec-Fetch-* headers through, and a current browser that sends them.` | S7 |
| Existing Crew refusal (`CrewNeedsDesktop.tsx`) | Crew needs the Biorouter desktop app | Crew in a `serve` browser |

## Learning from corrections

PR-22 keeps dictated spans in the composer's renderer memory, including drafts restored after a tab switch and recordings that finish after unmount. Editing those spans can offer short correction pairs after a send, queue drain, steer or ordinary message editor save is accepted. A refused, failed or cancelled send offers nothing. The request carries the actual destination chat, including one just created from Home. Programmatic text replacements cannot teach from an older span.

**Save as new chat**, `/diverge` and `/crew` drop the ledger in v1. The editor's new chat callback does not expose its destination, so it cannot stamp a correction there. No transcript or ledger is attached to outgoing messages or navigation. The daemon checks the pairs again before the dictionary changes. See [Learning from the person's edits](dictionary.md#learning-from-the-persons-edits) for the filters and the second distinct sighting rule.

## Related documentation

- [Implementation plan](../history/local-dictation-2026-11/implementation-plan.md): the requirements, the decision register this document follows, and the pull requests named in each Release row.
- [Engine and models](engine-and-models.md): the worker, the model catalog, the installer, the managed policy block and the daemon side of the `/dictation/*` routes.
- [Dictionary](dictionary.md): the store, the bias set, authority per surface, the **Dictionary** page and the `dictionary` capability that several surfaces above refer to.
- [Testing and release](testing-and-release.md): the tests, CI jobs and the per surface manual checklist behind each Acceptance row.
- [Browser access](../deployment/browser-access.md): how `biorouter serve` is reached and proxied, which S5, S6 and the proxy guidance build on.
- [Serve decisions](../deployment/serve-decisions.md): SD-8, SD-14 and SD-15, and the SD-17 record this work adds.
- [Headless Linux deployment](../deployment/headless-linux.md): the systemd service account and the plain HTTP address that S7 and S8 correct.
- [Jupyter AI persona](../integrations/jupyter-ai-persona.md): the integration where dictation is not offered, by the owner's decision O-30 (S13).
- [Crew](../crew/README.md): the workspace whose composers S14 adds microphones to, and the manual its new strings join.
- [Apps SDK reference](../apps-sdk/sdk-reference.md): where MSG-APPS and MSG-APPS-BROWSER are published for app authors.
- [CLI command reference](../cli/command-reference.md): the `serve` and `acp` sections that gain dictation lines.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules behind every Bias row, a safety boundary against mistakes by cooperative actors, not a security boundary against a determined adversary.
- [Managed policy](../security/managed-policy.md): the policy file whose `dictation:` block every surface honours.
- [Shared controls](../desktop-ui/shared-controls.md): the fill and focus rules the microphone button and its pill follow.
- [Dictation tools survey](../research/voice-dictation/dictation-tools-survey.md): how other dictation tools handle capture, gestures and lost speech, which several choices here answer.
