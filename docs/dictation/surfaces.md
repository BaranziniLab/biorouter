# Dictation on every surface

> **What this is.** How local dictation works on each of Biorouter's 20 surfaces, S1 to S20: where audio is captured, how it travels, which process transcribes it, the controls and states a person sees, the operating system permissions involved, and the exact text shown when something is missing or impossible. It also holds the complete message catalog.
> **Status:** Current: the plan being built on branch `feat/local-dictation`, pull request by pull request in the order of the delivery plan
> **Audience:** developers of the desktop, `serve`, terminal, Crew, app, clipboard and ACP pull requests (PR-17 to PR-31; PR-32 is withdrawn); reviewers who check that no surface ships a dead control; operators of shared hosts who need to know where models go and what people will see.

This is one of the four area documents of the [implementation plan](implementation-plan.md). It elaborates the plan's decisions DD-1, DD-27 to DD-31 (the client side), DD-60 to DD-66, DD-69, DD-70, DD-72 to DD-88 and DD-92, and follows every other decision in the plan's register. The engine, the worker, the model catalog and the daemon side of the routes are in [engine and models](engine-and-models.md). The dictionary, its tiers, its sidebar page and its agent capability are in [dictionary](dictionary.md). Tests, CI jobs and the manual checklist are in [testing and release](testing-and-release.md). Where this document and the register disagree, the register wins.

## How to read this document

Load bearing claims carry a label:

| Label | Meaning |
|---|---|
| [M] | Measured on 2026-11-04 during this design, on real binaries, browsers or audio |
| [C] | Read in Biorouter's code at `main` (`52917f33e`, three commits after the v1.94.1 tag; the workspace version is 1.94.1); a line number is against `main` |
| [R] | Reported by a third party's source, documentation or issue tracker, not measured here |
| [I] | Inferred; the measurement that would settle it is named when it matters |

Identifiers: R1 to R8 are the owner's requirements; S1 to S20 the canonical surfaces; DD the plan's decisions; PR-01 to PR-33 and A-1 its pull requests (PR-07 and PR-32 withdrawn); O the owner's decisions of 2026-11-05 and the plan's defaults; M-1 to M-22 the measurements still to take (M-9 and M-13 withdrawn); MSG the messages in the [message catalog](#message-catalog); P-1 to P-16 the privacy invariants. Code is cited as a file plus a symbol, with paths from the repository root. A file that does not exist yet is written as a code span, never as a link.

Two rules govern every section below. First, a control, command or route appears only in the pull request that makes it work end to end, and until PR-21 lands the `serve` gates, Biorouter's pages hide every dictation control when `isBrowserSurface()` is true (DD-94). Second, a surface that cannot capture audio never shows a control that does nothing: it shows a specific alternative and a sentence from the catalog (R8, and the SD-8 rule "a control that can never work here says so, before it is touched").

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

- Biorouter keeps audio in memory on the transcribing host: no temporary file, no log line, no diagnostics entry, no session row (P-1). Logs carry durations, byte counts and model ids only (DD-93). A reverse proxy in front of `serve` is outside Biorouter: nginx with its stock settings writes each chunk to a temporary file [M], so the [proxy guidance](#biorouter-serve-s5-to-s7) makes the setting that keeps chunks in its memory a requirement.
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
| S15 Agent Drafter apps | Never the app page; the trusted renderer for the preview | Serving daemon's worker | That daemon's folder | Public by default; the desktop preview takes private terms for one recording after MSG-PRIVATE-TERMS (O-8) | **Dictate into this app**; **Dictate to clipboard** | MSG-APPS, MSG-APPS-BROWSER | PR-03 (headers), PR-28 |
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

Every Biorouter page records, uploads and inserts with the same modules, added in PR-17 and kept free of React so Crew's composer, the clipboard capture window and the preview bar import them unchanged:

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

DD-64. The steps, all inside the click or keydown handler that starts dictation, which runs when the phase is `idle` or the status has not loaded yet. Steps 1 to 3 run synchronously at the top of the handler, before any `await`, including the wait for the [status store](#the-status-store):

1. Check, before any request or prompt: `!window.isSecureContext || !navigator.mediaDevices?.getUserMedia` puts the button in `unavailable` with MSG-INSECURE. Chromium exposes the `AudioWorkletNode` constructor on an insecure origin while `ctx.audioWorklet` is missing, so testing the constructor is not enough [M].
2. Call `getUserMedia({audio: {deviceId, echoCancellation: false, noiseSuppression: <per viewer, default false>, autoGainControl: false}})`. `deviceId` is `{exact: id}` when the person chose a device in Settings. Automatic gain control stays off because Chromium's gain control on Windows rewrites the system microphone volume [R].
3. Create `new AudioContext({sampleRate: 16000, latencyHint: 'interactive'})` and call `resume()` inside the same gesture. WebKit's `resume()` never settled without a gesture [M]. Whether the gesture survives an `await` on a network request was not measured [I], which is why nothing is awaited before this step.
4. Load the worklet with `import workletUrl from './pcmCapture.worklet.js?url&no-inline'`, starting `addModule` in the same synchronous block as steps 2 and 3, so the device, the context and the module come up in parallel. `vite.renderer.config.mts` also sets `build.assetsInlineLimit` to a function that returns `false` for `*.worklet.js`.
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
  readonly timings: CaptureTimings;         // ms from the call: stream, module, connected, live, first chunk
  stop(): Promise<void>;                    // flushes the last partial chunk, ends tracks, starts closing the context
  cancel(): void;
  readonly ended: Promise<'stopped' | 'track-ended'>;
  readonly closed: Promise<void>;           // the context is closed; nothing on the stop path waits for it
}
export function startCapture(opts: CaptureOptions): Promise<Capture>; // call inside the gesture
export function prewarmCapture(): void;     // enumerateDevices(), at most every 30 s; opens nothing
```

**Warming before the press.** The first `getUserMedia` in a window pays for the browser's first device enumeration. `prewarmCapture()` calls `enumerateDevices()`, which opens no device, shows no microphone indicator and asks for no permission, and `noteDictationInterest()` in the [status store](#the-status-store) calls it when a composer's field takes focus or the pointer reaches a microphone, at most every 30 s. Measured in the `dictation-capture-electron` probe on an Apple Silicon Mac (Electron 39.8.10, the fake device, 2026-11-05, a fresh profile per run from the live check on) [M]: press to `getUserMedia` resolving took 209 to 463 ms on the first press in a fresh window (five runs) and 28 to 38 ms after the warm-up (thirteen runs; 64 ms once, while two other Electron apps, a vitest run and a cargo build shared the machine), and press to the first audio frame in the worklet 30 to 40 ms (108 ms in that run); warming an `AudioContext` instead only reached 91 ms. With the warm-up, the first 100 ms chunk arrives 126 to 154 ms after the press (195 ms in the loaded run). On stop the worklet's last partial chunk arrives 0.5 to 0.8 ms after a stop that is not an input event, but 0.7 to 10.2 ms after a click (eleven stops in six runs, then eighteen in the live check, fifteen of them 5 to 10 ms) [M], because Chromium renders the frame that follows an input event before it delivers the worklet's answer to the page, so the wait depends on where the click falls between frames [I]. That wait is the client's floor on the stop path: the tail cannot go up before it arrives. A composer that renders on the stop adds to it only what its render takes beyond that wait: with a stand in render of 10 ms on every change of the session's snapshot (`BIOROUTER_DICTATION_CAPTURE_RENDER_MS=10`) the last chunk came 11 ms after the click, whether the flush was posted before the render or after it, since the page takes the answer only once the render is done [M]. So the card's render on stop must stay well inside one frame.

What the measurements showed [M]:

- Vite 7.3.1 inlines assets under 4,096 B as `data:` URLs, the worklet included, and the renderer's CSP (`script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'`) blocks a `data:` or `blob:` worklet; dictation then dies on the first click with `AbortError: Unable to load a worklet's module` in Electron, Chromium and Firefox. `?url&no-inline` emits `assets/<name>-<hash>.js`, which loads in 1 to 2 ms from a packaged `file://` app (plain folder and `app.asar`) and in every browser.
- Capture into a 16 kHz context gave RMS 0.35353 against 0.35355 expected and 440.1 Hz for a 440 Hz tone in Electron 39.8.10 and Chromium 145; WebKit 26.0, fed a synthetic 48 kHz stream, gave RMS 0.35329 and 441.5 Hz. Firefox 146.0.1 throws `NotSupportedError` on a cross rate `createMediaStreamSource`; the native rate path works there.
- An unconnected `AudioWorkletNode` is still processed. `channelCount: {ideal: 1}` does not give mono; `channelCountMode: 'explicit'` does. The resampler overshoots peaks by up to about 5 %, hence the clamp.
- A worklet in a hidden, never shown Electron window delivered 375 render quanta per second, still at 350 s.
- Speech through the whole client path, with a recorded clip as the fake device (`BIOROUTER_DICTATION_CAPTURE_SPEECH_WAV`, `c01_Samantha.wav` and `s09_Daniel.wav` from the bench, 2026-11-05): after its first 130 ms, the audio the loopback stand in received matched the clip at one constant lag, 36.9 dB signal to error (38.1 dB below 6 kHz), with no seam at a 100 ms chunk or a 2 s `PUT` boundary and the level within 0.03 dB. Chromium's resampling between the clip and the device rate cut the band above 6 kHz (energy 0.72 of the clip's at 6 to 7.2 kHz, 0.12 at 7.2 to 8 kHz). whisper small.en and large-v3-turbo q5_0 transcribed the received audio word for word as they transcribed the clip. The first 28 ms the worklet received after `onLive` (450 or 451 samples) were digital zeros, the clip's first 39 ms never reached it, and in one run, on a loaded machine, a 10 ms glitch 119 ms in replaced 8.75 ms with zeros and repeated 1.3 ms. All of that falls before anyone could start speaking after the card changes.

### The client side of a stream

DD-27 to DD-31. `components/dictation/transport.ts` is the only stream client, and it reaches `/dictation/streams` only through `components/dictation/dictationApi.ts`, so the composer test factories that mock `../api` need no new export.

| Route | Body | Answer |
|---|---|---|
| `POST /dictation/streams` | JSON at most 8 KiB `{sample_rate, format: "s16le", language?, target_kind: "chat" \| "new_chat" \| "editor" \| "crew" \| "external", session_id?, new_chat_provider?, include_private_terms?, context?}` (`context` at most 2 KiB, ranking only, never logged; `new_chat_provider` only with `new_chat`; `include_private_terms` only with `crew` or `external`) | 201 `{stream_id, max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600}`; 409 `noModel`; 429 `tooManyStreams` with `Retry-After`; 403 with the policy sentence |
| `PUT /dictation/streams/{id}/audio?seq=N` | `application/octet-stream`, raw s16le, at most 524,288 B | 200 `{received_s, partial?, queue_position}`; 409 `outOfOrder {expected_seq}`; 410 expired or cancelled; 413 |
| `POST /dictation/streams/{id}/finish` | none | 200 transcript, or 202 `{pending: true}` after 3 s; 500 `engineFailed` when the worker exits or misses its deadline |
| `GET /dictation/streams/{id}` | none | as finish, answered at once |
| `DELETE /dictation/streams/{id}` | none | 204; queued segments dropped |

Every route takes `X-Secret-Key` through `check_token`. Stream creation also sends `await userActionHeaders()`: the user action proof on the desktop, and on `serve` an `X-Caller-Provider` naming the host's configured provider [C `ui/desktop/src/utils/userAction.ts` `userActionHeaders`]. As decided by the owner on 2026-11-05 (O-17), the proof is not required to create a stream; it only decides whether private entries may bias (DD-29).

What each composer sends:

| Where the button is | `target_kind` | `session_id` | `context` |
|---|---|---|---|
| `ChatInput` in an existing chat (tab, split pane) | `chat` | the chat's id | the draft before the anchor, then the visible transcript tail, cut to 2 KiB on a character boundary |
| `ChatInput` on Home or a new tab, before the chat exists | `new_chat`, with `new_chat_provider` set to the tab's provider pick when it has one | none | the draft before the anchor |
| The message editor (`UserMessage.tsx`) | `editor` | the chat's id | the message text before the anchor |
| Crew's channel composer and **Ask my agent** | `crew`, with `include_private_terms: true` only for a recording the person started with **Include my private terms** | none | the draft before the anchor |
| The clipboard capture window, the preview bar | `external`, with `include_private_terms: true` under the same rule | none | none |

The daemon decides the target tier once per stream (DD-30): with `session_id`, `session_reach` runs first and a refusal degrades silently to public bias with a byte identical response; `new_chat` uses the tier the new chat's bind would produce, never a declared tier ([engine and models](engine-and-models.md)); `crew` and `external` are public unless the body carries `include_private_terms: true` from a caller with the proof and private standing, which the renderer sends only after the person confirms MSG-PRIVATE-TERMS for that recording (O-8). The provider the new chat will bind is `new_chat_provider` when the caller holds the user action proof, and `BIOROUTER_PROVIDER` otherwise: an unproven caller's `new_chat_provider` is ignored, never refused, so the answer reveals nothing. The private draft flag (DD-71) still warns if that text is then sent to a public model.

The stream, step by step:

1. **Create** when capture starts, in parallel with `getUserMedia`. A 409 `noModel` opens the setup popover; a 403 shows the policy sentence from the body; a 429 shows MSG-SERVER-BUSY with `<n>` counted down from `Retry-After` (at most 30 s), then tries again. It tries again on every 429 while the person speaks, since the audio is buffered meanwhile and waiting costs no speech, and for up to 30 s after the stop. A 429 after that ends the dictation with MSG-SERVER-BUSY-FAILED and **Retry**, and the audio is kept for 5 minutes, as for MSG-ENGINE-FAILED.
2. **Buffer** the worklet's 100 ms chunks in memory. The client keeps every chunk until `finish` answers with text, so a failed stream can be replayed; after a failure it keeps them for at most 5 minutes, for **Retry**, and drops them on the next dictation, on cancel or on unmount.
3. **Upload** every 2 to 3 s (64,000 to 96,000 B at 16 kHz), never more than `max_chunk_bytes`. Exactly one `PUT` is in flight, with a 10 s timeout. On a timeout the same `seq` is sent again with the same bytes; the daemon treats a repeated `seq` as idempotent. The sixth timeout in a row on one chunk (60 s, the daemon's idle expiry) ends the dictation with MSG-LOST, and so does a fourth network error, 5xx or 429 answer in a row on a chunk the next step cannot shrink. The daemon never answers a `PUT` with 429, so one comes from a proxy that rate limits, and it is waited out like any other transient failure, for at most 10 s at a time. Capture keeps buffering while a `PUT` waits. A failure in this step, or in steps 4, 5 and 7, ends the dictation when it happens, also while the person is still speaking (`onFailed`): the microphone goes off and the card shows the notice, rather than recording into a stream that can take nothing more until the stop.
4. **Shrink** on a 413, or on a network error for a chunk over 64 KiB: halve the chunk size and resend, with a floor of 64 KiB. A 413 at 64 KiB ends the dictation with MSG-413, because the proxy in front cannot pass even the smallest chunk. After a network error the first, larger body may have arrived with only its answer lost; the daemon then ignores the halved repeat of that `seq` and answers with the full chunk's `received_s`, and the client skips the half it would otherwise send twice.
5. **Reorder** on 409 `outOfOrder`: resend from `expected_seq`.
6. **Wait visibly.** When a `PUT` or a poll has had no answer for 1.5 s, the card shows MSG-WAIT until it answers.
7. **Recover** from a 410 that the person did not cause (the 60 s idle expiry after a long stall): open one new stream and replay the buffered audio from `seq=0`. A second failure ends the dictation in the `error` phase with MSG-LOST and inserts nothing.
8. **Preview** partial text: when a `PUT` answer carries `partial`, show it in a muted preview line inside the card, never in the field (DD-31).
9. **Cap**: show MSG-CAP-WARN 30 s before `max_seconds` (at 570 s of 600), then stop, insert and show MSG-CAP-STOPPED at 600 s. The cap is a stop, never an error.
10. **Finish** on stop: upload the remainder, then `POST …/finish`. A 200 carries the transcript; a 202 means poll `GET /dictation/streams/{id}` every 500 ms, each answered at once. No request is ever held open.
11. **Engine failure**: a 500 `engineFailed` from `finish` or a poll (the worker crashed, or missed its result deadline after the GPU retry) ends the dictation in the `error` phase with MSG-ENGINE-FAILED and its **Retry** action in the card's notice row. **Retry** opens a new stream and replays the buffered chunks; nothing is inserted until it succeeds.
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

The `PUT` is a hand written `fetch(getApiUrl(…))` (`ui/desktop/src/config.ts` `getApiUrl`) with `X-Secret-Key` and `Content-Type: application/octet-stream`, because the body is raw bytes. A packaged `file://` page sent it with no CORS preflight [M]; a `serve` page is same origin.

Measured in the `dictation-capture-electron` probe against a loopback stand in for the daemon that holds each `PUT` 15 ms (Apple Silicon, Electron 39.8.10, 2026-11-05, six runs of a 3 s dictation, then nine more in the live check, two of them of 5 s speech clips) [M]: the create left the page 1 to 23 ms after the press, after capture had started; on stop, the final `PUT` left 6 to 10 ms after the click, `finish` 25 to 29 ms after it, and the text was in the field 1 or 2 ms after the transcript arrived. So the client adds about 10 ms and one `PUT` round trip to the daemon's own time. No run sent a CORS preflight.

Why there is no long lived dictation request: the renderer already holds up to five long requests by design, two observer streams (`MAX_LIVE_OBSERVER_STREAMS = 2` at `crates/biorouter-server/src/state.rs:879`), two long polls and one `/reply` [C]. Electron, Chromium, Firefox and WebKit allow six HTTP/1.1 connections per origin; with five held a `PUT` took 2 to 10 ms, and with six held it waited for a long poll to end [M]. Over HTTP/2 no engine of the three stalled, and with ten streams held a `PUT` took 7 to 17 ms [M]. Uploads of 64 to 512 KiB took about 3 to 14 ms through nginx across the three engines [M]. M-5 measures the real app with two windows and two `serve` tabs.

### Inserting text

DD-63. One function places the transcript, for every Biorouter composer:

1. **Anchor.** When recording starts, remember the field's selection (`selectionStart`, `selectionEnd`). Every later change to the prose maps the anchor through the same single region diff the correction ledger uses (`locateEdit`). An untouched selection is replaced; a selection the person typed into collapses to its mapped end.
2. **Splice** with `spliceDictation`: collapse runs of whitespace, strip a leading `/` (a spoken "slash bug" never becomes a command), add a leading space unless the text before is whitespace or an opening bracket or quote, add a trailing space only in the middle of text before a word, change no case.
3. **Undoable path.** If the field owns focus (`document.activeElement` is the field, the body, or inside its `.biorouter-composer-card`) and no IME composition is running: `field.setSelectionRange(anchor.start, anchor.end)`, then `document.execCommand('insertText', false, inserted)`, then check `field.value.slice(start, start + inserted.length) === inserted`. During a composition, wait for `compositionend`.
4. **State path.** Otherwise (focus elsewhere, `execCommand` returned `false`, or the check failed): compute the new value from the composer's current prose and set it through the composer's state, as the `onQuotedText` effect in `ChatInput.tsx` does through `setComposerText` [C]. This path is not undoable.
5. **Always** call `setHasUserTyped(true)`, so history recall cannot replace the box; never call `checkForMentionOrSlash`, so a spoken "@" opens no menu; leave the caret after the inserted text.
6. **Composer gone.** If the composer unmounted (tab switch, split, leaving Home), splice into the stored draft at the mapped anchor with `insertIntoComposerDraft(key, at, text)`. Never `giveBackToComposer`, which puts returned text before the draft.
7. **Find the field by reference.** `ChatInput` already holds its textarea in a local `textAreaRef` (`useRef`, attached to the `<textarea>`) [C]; it passes that ref to `DictationButton` as a new prop. Never look the field up by `#dynamic-textarea`: that id is a literal, so a split view repeats it in each pane [C].

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

`components/dictation/dictationStore.ts` is a `useSyncExternalStore` singleton over `GET /dictation/status` (shape in [engine and models](engine-and-models.md)). It sends no request when `ChatInput` mounts, because 23 test files render the real composer under an offline fetch guard [C `ui/desktop/src/test/setup.ts`]. It loads when a composer's textarea first gains focus, on the first `pointerenter`, focus or click of any microphone, and when the Settings section or the Dictionary page mounts (`noteDictationInterest()`, which also warms the device list, see [capture](#capture-in-a-biorouter-page)); refreshes on window focus and after install, select or delete; and polls every 1.5 s only while `install_job` runs. A focused textarea is where ⌘⇧D is pressed, so the chord rarely meets a store that has not loaded.

Until it has loaded, the button shows the `idle` look. A click or chord then runs steps 1 to 3 of [capture](#capture-in-a-biorouter-page) at once, inside the gesture, and only then waits for the one status request. If the phase it reports is not `idle`, the handler stops every track, closes the context and acts on the real phase (the setup popover, or the reason sentence); no stream is opened before the answer. Each status read has a 5 s deadline (`STATUS_TIMEOUT_MS`): a daemon that takes the connection and never answers would otherwise hold that press in Transcribing until Escape, and every later read of the window with it. After the deadline the store reads `failed`, the phase `idle`, the stream opens, and its create's own answer (409 `noModel`, 403, 404) decides. The interface learns policy only from `status.policy`, never from `GET /config`.

Where the daemon is decides what the card says about it (DD-92). `dictationDaemonPlace()` in the store answers `this-computer` only when `main.ts` told the window it started the daemon itself (`BIOROUTER_LOCAL_BACKEND`), `external-backend` on S17 and `serve` in a browser, each with the host the daemon reports for itself (`status.host`), or the address the API calls go to. MSG-ENGINE-FAILED fills `<where>` from it, MSG-SETUP fills its own `<where>` (the forms with "on", `setupWhereOnServer` in `copy.ts`), and the setup popover picks its privacy line by its `kind`. A phase decision fills MSG-INSECURE with the page's own address and MSG-REMOTE-OLD with the daemon's and this app's version even when its caller passes nothing, so no sentence reaches a person with a placeholder in it.

Per viewer conveniences live in `localStorage` key `biorouter:dictation-viewer`, every read and write wrapped in try and catch: `{deviceId, noiseReduction, showButton, ackHosts[], policyOffHint}`. `ackHosts[]` remembers each host whose MSG-DISCLOSE-HOST was shown and each `http:` backend URL whose MSG-REMOTE-HTTP was confirmed; `policyOffHint` hides the button at the next launch when the last status said policy forbids dictation. The daemon checks policy on every request regardless.

## The desktop composer button in every composer

### Placement

DD-60. One component, `components/dictation/DictationButton.tsx`, a ghost round 28 px `Button` (`size="sm" shape="round" variant="ghost"`), `type="button"`, with `onMouseDown` calling `preventDefault()` so the field keeps focus and its accent edge. The glyph is `export const Mic = light(_Mic)` in `components/icons/app-icons.tsx`.

| Composer | File | Position | `target.kind` |
|---|---|---|---|
| Chats, Home, split panes, new tabs (one `ChatInput`) | `ui/desktop/src/components/ChatInput.tsx` | between the model slot (`:3060`) and `{sendOrStop}` (`:3074`); **Send** stays the row's last child, because the footer inset and `composerGeometry.browser.test.ts` are derived from its edge [C] | `chat`, or `new_chat` without a session |
| The message editor | `ui/desktop/src/components/UserMessage.tsx` | in the right aligned footer (**Cancel**, **Save as new**, **Save** at `main` [C]), immediately left of **Save**, the editor's send action, so **Save** stays last | `editor` |
| Crew's channel composer | `ui/desktop/src/components/crew/composer/Composer.tsx` | after `.crew-compose-spacer`, before `.crew-compose-send` | `crew` |
| Crew's **Ask my agent** box | `ui/desktop/src/components/crew/pane/AgentTaskPane.tsx` | immediately left of the submit button **Start my agent and allow posting here**, in the pane's `flex justify-end` row [C] | `crew` |

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

At the narrowest pane (360 px), the row keeps room for more than 12 px of the model chip's name with the microphone added [I], which `composerCard.browser.test.tsx` already measures at 760, 440 and 360 px.

### Before a model is installed

DD-61 and O-9. The button shows from the first launch in a muted `not-installed` look. A click opens the setup popover; it never starts a download by itself. The per viewer switch **Show the microphone button** in Settings hides it; ⌘⇧D still starts dictation, and the recording pill takes the button's place until the dictation ends. When managed policy sets `dictation.enabled: false` or `dictation.capture: false`, the button is removed from every composer once the status store knows, and Settings shows MSG-POLICY or MSG-POLICY-CAPTURE in its policy note.

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
| `GET /dictation/status` answers 404 or 405 (S17: a backend from before dictation) | `unavailable` | MSG-REMOTE-OLD |
| `status.policy.enabled` or `status.policy.capture` not allowed | button removed | Settings shows MSG-POLICY or MSG-POLICY-CAPTURE |
| `status.engine.compiled` false | `unavailable` | MSG-NO-ENGINE |
| `status.engine.state` is `unsupported_cpu` | `unavailable` | desktop: MSG-CPU then MSG-CPU-DESKTOP; Intel build under Rosetta on macOS 12 to 14 (any version before 15): MSG-CPU-ROSETTA; `serve`: MSG-CPU-SERVER |
| no installed model, `status.install_job` running | `installing` | none |
| no installed model, `status.install.allowed` true | `not-installed` | none |
| no installed model, install refused | `unavailable` | `status.install.reason` (MSG-INSTALL-LOCKED, MSG-POLICY, MSG-POLICY-DOWNLOADS, MSG-POLICY-MODELS), or MSG-REMOTE-NO-KEY on S17 |
| otherwise (a usable model, even while another one installs) | `idle` | none |

"No installed model" means `status.active_model` names no installed model. The `installing` row comes before `not-installed`: with the order reversed, the first install would show `not-installed` until it finished, and the progress ring after **Install** would never appear (`resolveDictationPhase` in `dictationStore.ts`).

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

`dictationPhase` from `onPhaseChange` joins both submit paths of `ChatInput.tsx` and the condition that disables **Send** [C]: Enter in `handleKeyDown` is handled by the gesture rules before the steer, queue and `performSubmit` calls; `onFormSubmit`, which **Send** reaches as a `type="submit"` button of the composer's form, refuses while arming, recording or transcribing; and `isSubmitButtonDisabled` also turns true then, so **Send** shows as disabled with the tooltip "Finish dictating first" (arming or recording) or "Transcribing…". Stop for a running turn stays available, and dictation is allowed while a turn runs; after the insert, Enter queues as usual.

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
| **Restart & update** while dictating | the renderer has set `dictation:busy`; the restart asks MSG-RESTART-BUSY |

`dictation:busy` stays set while any composer in the window still waits for its text, including a pane that a press in another pane stopped, which finishes in the background. A recording kept for **Retry** does not hold it. The main process keeps the record in `utils/dictationBusy.ts`, which the updater imports, and drops a window's entry when the window closes or its renderer crashes; every new document (a reload, a crash recovery) sends `false` from its preload first, so no window stays busy for a dictation that is gone. Two windows may each record; the daemon queues their streams.

### The setup popover and disclosure

`components/dictation/SetupPopover.tsx`, anchored to the microphone (`side="top"`), in the idiom of the Llama Server inline card. It holds MSG-SETUP with `<where>` filled in ("on this computer", or "on the Biorouter server at \<host>"), the model's name, size and folder, the privacy line, **Install**, **Not now** and the link **More in Settings**; after an install, the link **Edit your dictionary**. Under **Install** sits the checkbox **Suggest dictionary words from my chats**, ticked and visible, with MSG-SETUP-SUGGEST below it (DD-56; the box arrives with PR-23). Progress continues in the microphone's ring and in Settings; completion shows no toast. There is no new onboarding step.

The privacy line is visible text, never an InfoTip (DD-92):

| Where the daemon is | Line |
|---|---|
| The desktop's own daemon | MSG-DISCLOSE-LOCAL |
| An external backend (S17) | MSG-DISCLOSE-REMOTE |
| A `serve` browser (S5 to S8, S16) | MSG-DISCLOSE-HOST, also shown once in the card before the first recording to a new host |

In MSG-DISCLOSE-HOST, `<host>` is `status.host`, followed by `location.host` in parentheses when they differ, because neither side can tell an SSH tunnel from a local daemon. No surface says "on this computer" when the daemon is elsewhere, and no build before PR-01 says Biorouter does not send the audio anywhere else. The sentence speaks for Biorouter only: on S6 the audio also crosses the operator's reverse proxy, which the [proxy guidance](#biorouter-serve-s5-to-s7) keeps from writing it to disk.

### The private draft flag

DD-71. `ComposerDraft` gains a memory only flag set when an inserted transcript reports `bias_scope: "private"`, and cleared when the draft is sent or emptied. `bias_scope: "private"` means private entries were eligible, not that a private term is in the text, and the messages say so. The check runs inside `releaseComposerText(reason)` ([dictionary](dictionary.md)), so it covers all three exits that hand the text to a model: `performSubmit`, `handleInterruptionAndQueue` (queue) and `handleSteerFromComposer` (steer) [C `ChatInput.tsx`]. Before such a draft leaves for a chat whose model is public, judged by the daemon's instance tier for that chat (the bound provider's `tier()`, never a declared tier), the composer asks MSG-PRIVATE-DRAFT (**Send anyway**, **Cancel**); before the model switcher moves that chat to a public model, it asks MSG-PRIVATE-DRAFT-SWITCH (**Switch anyway**, **Cancel**). The person's say so proceeds. No agent path sees or answers either question.

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

Errors appear in the card's notice row, never as toasts. The row clears when the next dictation starts.

### Message editor and sending

DD-69 and DD-70. Before the message editor gets a microphone, PR-04 deletes the two `logInfo` calls in `UserMessage.tsx` that write message text to the packaged log: `initializeEditMode` (`:208`, the edit mode content) and the change handler (`:256`, the changed content) [C]. Dictated text reaches the editor through the change handler, so `:256` is the one dictation would otherwise log. A third call, in `handleKeyDown` (`:310`), logs each key name with its Meta and Ctrl state [C]; dictated text never passes through it, and DD-69 leaves it alone. No composer, on any surface, sends dictated text by itself, and no setting adds an automatic send.

## Desktop permissions and packaging per OS

DD-65. The renderer is the only desktop process that records, and its grant is narrow.

### Every desktop OS

- `utils/permissionPolicy.ts` `isAllowedRendererPermission` already grants `media` only when every requested type is `'audio'` and the origin is the app's own, "for dictation" [C]. PR-04 adds `details.isMainFrame === true`, passed by `installSessionHooks` in `main.ts`, so no subframe can ever get the microphone. Both handlers received `isMainFrame: true` for the main frame, and a denial rejects with `NotAllowedError: Permission denied` [M].
- App previews and the embedded browser keep denying every permission [C `ui/desktop/src/utils/managedAppPreviewSession.ts`].
- IPC, each handler checking that the sender is a window's main frame:

| Channel | Direction | Payload | Answer |
|---|---|---|---|
| `dictation:mic-status` | renderer to main | none | `systemPreferences.getMediaAccessStatus('microphone')` on macOS and Windows; `'unknown'` on Linux |
| `dictation:mic-request` | renderer to main | none | macOS `systemPreferences.askForMediaAccess('microphone')` on the first click while `not-determined`; elsewhere `true` |
| `dictation:open-mic-settings` | renderer to main | none | opens `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` (macOS) or `ms-settings:privacy-microphone` (Windows); no deep link on Linux |
| `dictation:busy` | renderer to main | boolean | gates **Restart & update** behind MSG-RESTART-BUSY |
| `dictation:lock-screen` | main to renderer | none | stop and insert |
| `dictation:insert-into-preview` | renderer to main | `{viewId, text}` | S15, shipped only if M-15 passes |
| `dictionary:add-term` | main to renderer | `{text, x, y}` | opens the term dialog prefilled (the context menu door); the renderer resolves the chat at that point, and the add names it as `origin_session_id`, or sends `origin_unresolved: true` for a transcript it cannot resolve, so a private term takes that chat's institutions (O-36) |

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

DD-66. One section, `SETTINGS_SECTIONS.dictation = { id: 'settings-dictation', tab: 'models' }`, mounted in `settings/models/ModelsSection.tsx` between `<LocalModelInventory/>` and the danger zone, built from new `settings/models/SpeechModelSection.tsx` and `speechModelStore.ts` (the `llamaServerStore` pattern: progress survives navigation, polling only during an install). There is no fourth tab [C `ui/desktop/src/components/settings/SettingsView.tsx:33`]. The setup popover's **More in Settings** navigates with the section key `dictation`.

| Row | Contents | Notes |
|---|---|---|
| Privacy line | MSG-DISCLOSE-LOCAL, MSG-DISCLOSE-REMOTE or MSG-DISCLOSE-HOST, as visible text | chosen by where the daemon is |
| One row per catalog model | display name, size, languages, suitability, "Recommended for this computer"; **Install**, **Use**, **Remove**, **Cancel**, **Update** | install sends `userActionHeaders()`; a refused install shows `status.install.reason` in place of the button; MSG-TURBO-CPU before turbo on a CPU host; MSG-DISK, MSG-HASH, MSG-OFFLINE on failure; a copy found in the system, managed or env folder shows its location in place of **Remove** |
| **Model folder** | `status.models_dir` with the account (`valueMono`); **Show in Finder** or **Show in Explorer**; **Rescan** | the reveal button only on the desktop with a loopback daemon; **Rescan** is `POST /dictation/models/rescan` and is the air gapped path |
| **Language** | `en` (default), `auto`, or an ISO 639-1 code | written through `POST /dictation/models/select`; a `.en` model refuses other values with MSG-LANGUAGE-EN |
| **Microphone** (per viewer) | System default plus devices, **Test microphone** | the test shows a 5 s level meter; nothing is recorded or sent |
| **Shortcut** | read only "⌘⇧D · tap to start or stop, hold to talk" | Ctrl+Shift+D off macOS; hidden on a `serve` browser until M-18 records that the page receives the chord, then shown per browser |
| **Show the microphone button**, **Reduce background noise** (per viewer) | two switches | noise reduction maps to `noiseSuppression` |
| **Dictate into other apps** (desktop only) | the switch, the shortcut picker or MSG-GNOME, MSG-WINDOW-OPEN on Windows and Linux, MSG-NO-TRAY while the tray is absent | MSG-CLIPBOARD-EXPLAIN shows when the switch turns on (S18) |
| **Test speed** | `POST /dictation/bench`, shows the real time factor | a factor above 0.5 marks a model `too_slow` |
| **Edit your dictionary** | link to `/dictionary` | reveals the sidebar group by the current route rule |
| Managed policy | a new `privacy/ManagedPolicyNote.tsx`, in the pattern of the existing `privacy/HostManagedModelNote.tsx` [C]; locked rows disabled with the policy sentence | shown when `status.policy.managed` |

On a `serve` browser the reveal button is hidden, **Dictate into other apps** is absent, the **Shortcut** row waits for M-18, and the install text says the download goes to the server. On S17 the rows act on the backend host, and the **Model folder** row names that host's folder and account.

## Surfaces one by one

### Desktop app (S1 to S4)

The four desktop surfaces share the capture, transport, insert, composer, permission and Settings sections above. Each subsection lists what differs. A model installed by the app and a model installed by the bundled CLI land in the same per user folder, so the two share one copy.

#### S1 desktop app, macOS Apple Silicon

| Part | Design |
|---|---|
| Capture | The renderer in every composer; the hidden capture window for **Dictate to clipboard** (S18); the main renderer for **Dictate into this app** (S15) |
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

`serve` runs the same renderer, built from the same source by `build:web`, in a browser, so the composer, Settings and the Dictionary page work as on the desktop, with these differences. PR-21 removes the `isBrowserSurface()` gates that PR-18 to PR-20 add, in the same pull request that lands everything in this section.

**Headers (PR-03, before any served microphone).** App pages share the shell's origin (SD-14), and a browser grants the microphone per origin, so agent written app code would otherwise inherit the person's grant without a prompt. No response sends a `Permissions-Policy` header at `main` [C, searched]; PR-03 adds one at each function below, and `APP_PERMISSIONS_POLICY` is a new constant in `routes/apps.rs`.

| Response | Header PR-03 adds | Where it goes (existing functions at `main`) |
|---|---|---|
| The shell | `Permissions-Policy: microphone=(self)` | `crates/biorouter-server/src/routes/web_ui.rs` `index` (`:346`) |
| App pages | `Permissions-Policy: microphone=(), camera=(), display-capture=()` (`APP_PERMISSIONS_POLICY`) | `routes/apps.rs` `app_page_response` (`:183`) |
| App files under `dist/` and `assets/` | the same | `with_app_file_csp` (`:154`) |
| Figure frames | the same | `figure_frame` (`:5608`) |
| The launch bounce | the same | `launch_bounce` (`:429`) |

Chromium enforces the header; Firefox and Safari ignore it [R]. That residual is recorded in SD-16, and `docs/deployment/browser-access.md` tells Firefox and Safari users to allow the microphone for one visit only.

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

**The install lock.** On the keyless `serve` daemon, a browser may install a pinned model with the secret (DD-20, O-2). Three switches turn that off, composing as the most restrictive: `biorouter serve --no-dictation-install` (which sets `BIOROUTER_DICTATION_BROWSER_INSTALL=false` in the environment of the daemon it starts), the env variable itself, and managed policy. When locked, `status.install` is `{allowed: false, reason}` with MSG-INSTALL-LOCKED, and the microphone and the Settings rows show that sentence before any click.

**Standing.** A `serve` browser holds no proof; `userActionHeaders()` states the host's configured provider in `X-Caller-Provider` [C]. On a host configured with a private model that gives private standing, so dictation into a private chat there uses private entries; on a host configured with a public model it uses public entries only. The Dictionary page on `serve` disables publicize and privatize before the click with MSG-DICTIONARY-HOST-TIER; raising `agent_writes` and turning chat learning or suggestions on with MSG-DICTIONARY-HOST-SETTINGS, which names `biorouter dictionary settings` (DD-75); and, without private standing, **Only in private chats** with MSG-DICTIONARY-HOST-PRIVATE ([dictionary](dictionary.md) has the full rules).

**The shared origin notice.** MSG-SHARED-ORIGIN shows once per origin before the first recording, on every `serve` origin and in every browser. App pages share the shell's origin (SD-14), and PR-03's `Permissions-Policy` header keeps an app page itself and the frames it holds away from the microphone in Chromium, but not a persistent grant: a same origin popup that an app page opens (`window.open('about:blank')`, or any same origin document served without the header) records in Chromium 157, measured on 2026-11-05 [M], and Firefox and Safari do not enforce the header at all. A path prefix, such as JupyterHub's `/user/<name>/biorouter/`, widens the grant further to every page on the hub. So the notice always recommends a one time grant, and per app origins remain the lasting fix (SD-16).

**The chord.** Browsers bind ⌘⇧D to their own commands (Chrome and Firefox bookmark all tabs; Safari adds to the Reading List [R]), and M-18 measures whether a page can take it. Until M-18 shows the page receives the chord, a `serve` browser's tooltip reads "Dictate" without it, Settings hides the **Shortcut** row, and the field's key handler does not claim the chord.

**Copilot.** Biorouter Copilot places no restriction on dictation, decided by the owner on 2026-11-05 (O-15, DD-88): an agent holding a Copilot grant on Biorouter's own window, or on a browser that shows a `serve` tab, can start the microphone by clicking it or pressing the dictation chord, and can then press **Send** as it can for typed text. What limits it is Copilot's per task grant (one grant per task, chat, model and target), the composer's visible recording state, the operating system's microphone indicator, and Stop and revoke. On the browser, the origin's existing microphone grant opens the microphone with no prompt. This residual is recorded in SD-16 and P-7; it concerns S5 on the person's own machine and S6 when the browser runs where Copilot does.

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

**SD-16**, a new record in `docs/deployment/serve-decisions.md` (PR-03 starts it, PR-21 completes it): dictation on `serve` needs no proof; private bias standing comes from the proof or a stated `X-Caller-Provider`, never from the operator's tier that SD-10 gives listings; model install is admitted unless the operator locks it; app pages deny the microphone; the Firefox and Safari residual; the residual that any holder of a keyless daemon's secret, an agent shell that recovers it included, can install a pinned model and make a browser's dictionary writes; the Copilot residual (DD-88, O-15); the ACP harvesting residual.

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
| Residuals | Copilot holding a grant on the browser that shows this page can click the microphone, with no refusal (DD-88, O-15; recorded in SD-16) |
| Release | PR-21 |
| Acceptance | The `rust.yml` `serve` job: Playwright Chromium (16 kHz) and Firefox (the native rate fallback) against `serve` on loopback with the debug test model show MSG-NOTHING, and the daemon received 32,000 B per second of audio; M-8 (Safari at `127.0.0.1`, including the first click and the first ⌘⇧D before the status store has loaded, which must reach `recording`) recorded |

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
| Acceptance | The `serve` job's `nginx:stable` container under `/user/a/biorouter/`: the worklet resolves, a 1 MiB plus 2 B `PUT` gets 413 and halves, and with `client_body_buffer_size 512k;` the nginx log holds no `buffered to a temporary file` line; a build output check that the worklet is an `assets/*.js` file referenced by the entry chunk; M-5 (two tabs while a turn streams, then behind HTTP/2 nginx) and M-17 recorded |

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

**Commands** (the full tree, including `biorouter dictation …` and `biorouter dictionary …`, is in [engine and models](engine-and-models.md) and [dictionary](dictionary.md)):

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

In every front end Enter sends only after the text is inserted and the person presses it again; text dictated while the agent streams goes into the composer and Enter queues it. While recording or transcribing, Ctrl+C cancels the dictation only, never the turn or the session; other keys are ignored and the hint says how to stop. `BIOROUTER_CLI_DICTATION_KEY` (default `t`, meaning Ctrl+T; empty disables) clashing with `BIOROUTER_CLI_NEWLINE_KEY` disables dictation's key with one startup note, MSG-KEY-CLASH. Slash commands: `/dictate`, `/dictate status`, `/dictate install [MODEL]`, `/dictate retry`, `/dictionary add`, `/dictionary review`. In the TUI and the classic REPL a private `/dictionary add` carries the session's institutions, read in process, as every term the person adds inside a private chat does, the context menu door included, while a term added with no chat carries none (O-36; [private rows and institutions](dictionary.md#private-rows-and-institutions), [in a session](dictionary.md#in-a-session)). Hold to talk waits for a later release (see [what waits](#what-waits-for-a-later-release)).

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
biorouter transcribe memo.m4a --format json            # public only; schema 1
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
| Messages | MSG-AGENT-SHELL, MSG-NO-TTY; `transcribe` JSON errors carry `error_code` |
| Release | PR-12 (`transcribe`), PR-25 (`dictate`) |
| Acceptance | `dictation_cli.rs` runs `transcribe jfk.wav --format json` with the `for-tests` model and asserts exit 3 and `error_code: "no_speech"`; an agent shell `transcribe --session <private id>` on reversed speech returns no private term |

### S12 ACP clients

Zed, JetBrains and other ACP editors render their own composer. None of the ACP clients checked (Zed, JetBrains AI Assistant, Jupyter AI's ACP client) sends audio: Zed only converts audio blocks between protocol versions and never builds one, JetBrains AI Assistant shows no sign of audio, and Jupyter AI's ACP client sends text and resource blocks only [R]. Biorouter advertises `.audio(false)` [C `crates/biorouter-acp/src/server.rs:755`]. Zed's request for dictation is the open issue zed#16410 [R]; no JetBrains issue was found. Biorouter owns no microphone control there, so there is nothing dead to show. The path is **Dictate to clipboard** and paste, and the editor learns it in the product, not only from the docs: at session start the ACP server advertises one available command, `/dictate`, through `AvailableCommandsUpdate` (in the pinned `agent-client-protocol-schema` 0.10.5 [R]). Invoking it answers MSG-ACP-DICTATE locally, with no model turn and no provider call. The Jupyter AI persona is an ACP client too, launched as `[BIOROUTER_EXECUTABLE, "acp"]` with nothing that names it [C `integrations/jupyter-ai/jupyter_ai_biorouter/persona.py`], and the owner excluded Jupyter from dictation (O-30, S13). Whether Jupyter AI shows ACP available commands is unmeasured [I], so PR-30 checks it first; if it does, the persona names itself at launch and the server leaves `/dictate` out of its sessions.

| Part | Design |
|---|---|
| Capture | None in the editor; the desktop's hidden capture window or the CLI capture child |
| Transcription | The desktop daemon's worker, or `biorouter dictate --copy` |
| Models | The local folder |
| Bias | Public only inside ACP (O-28 keeps it so when audio blocks arrive); the clipboard routes it relies on follow S18 |
| Entry points | Tray **Dictate to clipboard**, the global shortcut, or `biorouter dictate --copy`, then paste; the editor's `/dictate` command explains this with MSG-ACP-DICTATE |
| In the turn | `ContentBlock::Audio(..) => ()` at `server.rs:428` [C], today a silent drop, becomes a text part with MSG-ACP-NOTE |
| Docs | MSG-ACP in `docs/cli/command-reference.md` (`### acp`) |
| Dictionary | stdio offers the `dictionary` capability in suggest form only, with no extractor; tool answers append MSG-TOOL-REVIEW-HINT; `--ws` refuses `--with-builtin dictionary` and never takes audio |
| Release | PR-30 (note and `/dictate`), PR-28 and PR-25 (clipboard) |
| Acceptance | An audio block yields the note and no audio; `on_initialize` still advertises `audio(false)`; `/dictate` appears in the editor's command list and answers MSG-ACP-DICTATE without calling the provider; Jupyter AI's chat shows no `/dictate` (O-30); a `--ws` agent loads no `dictionary` |

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
| The desktop preview | **Dictate into this app** in `artifacts/PreviewBar.tsx`, shipped only if M-15 passes |
| A browser, with no Biorouter app of the person's own | dictate in Biorouter's own composer and copy the text, or use the system's own dictation (MSG-APPS-BROWSER) |

**Dictate into this app.** A microphone icon button in the preview bar's actions (`PreviewBarActions`) for an app artifact, in the desktop app only: it needs IPC to main, so it is not rendered when `isBrowserSurface()` is true, and a `serve` browser gets MSG-APPS-BROWSER instead. The preview session keeps denying every permission [C `managedAppPreviewSession.ts`], so the trusted main renderer records with the composer's capture and transport (`target_kind: "external"`), then sends IPC `dictation:insert-into-preview {viewId, text}`. Main calls `view.webContents.focus()` and `view.webContents.insertText(text)` on that preview's `WebContentsView`; the app receives text only, and the text goes into whatever field the app had focused. Before enabling the button, main asks the preview, through `executeJavaScriptInIsolatedWorld` with a fixed expression, whether an editable element is focused; when none is, the button stays present and disabled, and its tooltip is MSG-APPS [I]. M-15 measures whether `insertText` reaches the focused field after the main renderer took focus; if it fails, PR-28 ships without the button, S15 relies on **Dictate to clipboard**, and the SDK reference carries MSG-APPS-NO-PREVIEW in place of MSG-APPS, because MSG-APPS names the button.

| Part | Design |
|---|---|
| Transcription | The serving daemon's worker |
| Models | That daemon's folder |
| Bias | Public by default; the desktop preview's **Dictate into this app** takes private terms for one recording after MSG-PRIVATE-TERMS (O-8) |
| Docs | `docs/apps-sdk/sdk-reference.md` carries MSG-APPS (or MSG-APPS-NO-PREVIEW if M-15 fails) for people with the Biorouter app and MSG-APPS-BROWSER for people who reach an app only through a browser |
| Release | PR-03 (headers), PR-28 (the preview button if M-15 passes; the clipboard path either way) |
| Acceptance | A Playwright Chromium test grants the microphone to the shell origin, opens an app page on the same origin and gets `NotAllowedError`; M-15 recorded; `target_kind: "external"` on every preview request; a vitest case shows **Dictate into this app** disabled with the MSG-APPS tooltip when nothing editable in the preview is focused, and not rendered on `isBrowserSurface()` |

`br.dictate()` and a `capabilities.dictation` switch for apps wait for per app origins (see [what waits](#what-waits-for-a-later-release)).

### S16 Docker image

DD-80 and O-14. The image becomes `serve` capable; it never contains a speech model. Today the builder lacks `cmake` and builds only `--package biorouter-cli`, so the image has no daemon and no web bundle [C `Dockerfile`].

Changes in PR-31: `cmake` in the builder (PR-09 already needs it); build `--package biorouter-cli --package biorouter-server`; a `node:24` stage runs `npm ci && npm run build:web` in `ui/desktop`; copy `biorouterd` beside `biorouter` and the bundle to `/usr/share/biorouter/web`; `EXPOSE 8765`. `GGML_NATIVE=OFF` with an explicit ISA matters most here, because Docker builds are native and would otherwise carry the build machine's instructions.

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
| Plain `http:` | MSG-REMOTE-HTTP once per backend URL for a non loopback `http:` address; the person confirms and proceeds (O-18); the confirmation is kept in `ackHosts[]` |
| No key on a keyed backend | Install disabled before the click with MSG-REMOTE-NO-KEY (O-17) |
| A backend from before dictation | The laptop and the backend can run different versions here, and only here. When `GET /dictation/status` answers 404 or 405, the microphone is `unavailable` with MSG-REMOTE-OLD; when `/dictionary/settings` answers 404, the **Dictionary** page shows the same sentence and disables its controls |
| The laptop's policy | `utils/managedDictationPolicy.ts` reads the laptop's managed policy file at the `Paths::managed_policy_path` locations (with `js-yaml`, already a dependency [C]) only when `externalBiorouterd.enabled`. It honours only `dictation.enabled: false` and `dictation.capture: false`; either makes `isAllowedRendererPermission` refuse `media`, removes the microphone, disables the clipboard switch, and shows MSG-POLICY or MSG-POLICY-CAPTURE. A Rust test pins the TypeScript path table against `Paths::managed_policy_path` |
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

Without the key, a keyed backend's microphone works with public entries only and installing is disabled, with MSG-REMOTE-NO-KEY, as decided by the owner on 2026-11-05 (O-17).

The laptop's CLI has its own folder and needs its own `biorouter dictation install`; `status` names the host and account, so the two are not confused.

### S18 other applications: dictate to clipboard

DD-86, O-8 and O-19. Two forms in v1 bring Biorouter's engine and dictionary to any application, with public terms unless the person includes private ones for one recording. Neither pastes for the person, and neither sends anything.

**The desktop form** (`ui/desktop/src/utils/dictationAnywhere.ts`, PR-28):

1. **Turn it on.** **Settings > Models > Speech models > Dictate into other apps**. Turning the switch on shows MSG-CLIPBOARD-EXPLAIN and, on macOS, calls `askForMediaAccess('microphone')` so the TCC prompt comes while the person looks at Biorouter. On Windows and Linux the row shows MSG-WINDOW-OPEN. The switch and the chosen shortcut live in `<config>/dictation/anywhere.json`, written only by Electron main and read by main at launch to build the tray item and register the shortcut (DD-88). The file is outside `config.yaml`, so `/config/upsert` cannot turn the global shortcut on, and outside renderer `localStorage`, which main could not read at launch; like the privacy master switch record, a shell can still edit it (accident prevention, not a boundary).
2. **Start.** The tray item **Dictate to clipboard**, added to `updateTrayMenu` in `ui/desktop/src/utils/autoUpdater.ts` while policy allows, or the global shortcut. Below that tray item, **Include my private terms** starts one recording with private terms after MSG-PRIVATE-TERMS ([including private terms](#including-private-terms-for-one-recording)). The tray exists only while **Show in menu bar** is on in **Settings > App**, and `createTray` turns that setting off when the icon file is missing or the tray cannot be created [C `main.ts` `createTray`, `disableTray`]. While the tray is absent, the row shows MSG-NO-TRAY and the switch turns on only together with a shortcut, so the feature always has a way in. While the switch is off, the tray item opens Settings at that row instead of recording; with no model installed, it opens the Speech models section. The shortcut is off until the person picks one (`CommandOrControl+Alt+Shift+D` suggested); `globalShortcut.register()`'s result is checked, and `false` shows MSG-SHORTCUT-TAKEN. On GNOME Wayland the picker is replaced by MSG-GNOME. Toggle only: Electron reports no key up.
3. **Record.** Main shows a recording tray image and the items **Stop and copy** and **Cancel**, starts `powerSaveBlocker.start('prevent-app-suspension')`, and opens a hidden capture window (`show: false`, `focusable: false`, `skipTaskbar: true`, `backgroundThrottling: false`) on the renderer partition, loading `#/dictation-capture`. That page runs the composer's capture and transport with `target_kind: "external"`. Deadlines come from worklet counts or main process timers, because hidden pages slow `setInterval` to once per second.
4. **Stop and copy.** A second trigger or **Stop and copy** finishes the stream. Main writes with `clipboard.writeText`, then compares `readText()`. A match shows the notification MSG-CLIPBOARD-COPIED (`<key>` is ⌘V or Ctrl+V); a mismatch (a Wayland clipboard that needs focus) opens a focused review card holding **Copy**. `no_speech` shows MSG-CLIPBOARD-NOTHING and leaves the clipboard unchanged; a result made only of dictionary words shows MSG-CLIPBOARD-ECHO and copies nothing.
5. **Errors.** A microphone refusal opens the review card with the mapped sentence and its action (MSG-MIC-DENIED-MAC, MSG-MIC-DENIED-WIN, MSG-NO-DEVICE, MSG-MIC-BUSY). An engine failure opens it with MSG-ENGINE-FAILED and **Retry**.
6. **Afterwards.** The tray image returns, the power save blocker stops, and the capture window idles, then closes. The last transcript stays in main's memory for at most 5 minutes, for the review card only. Notifications carry counts, never text.

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
| An off machine engine (`BIOROUTER_DICTATION_ENGINE_URL`) | Not needed by R1 to R8; it would send audio and terms to a host the tier rules call Public (O-12, DD-87). The managed `external_engine` key is parsed now as a no op | The local engine on every surface |

## Message catalog

Renderer copy lives in `ui/desktop/src/components/dictation/copy.ts` and `components/dictionary/copy.ts`; CLI copy in `crates/biorouter-cli/src/dictation/messages.rs`; both are pinned by snapshot tests. Until that CLI file exists, PR-10's status lines and PR-11's model command sentences (MSG-SYSTEM-ROOT to MSG-LIST-NOTES) are in `crates/biorouter-cli/src/commands/dictation.rs`, the installer's own sentences (MSG-TLS to MSG-REMOVE-FAILED, with MSG-HASH and MSG-DISK) in `crates/biorouter-dictation/src/models.rs`, and the engine's diagnostics (MSG-DIAG-EXIT to MSG-DIAG-CANCELLED) are `DictationError`'s text in `crates/biorouter/src/dictation/service.rs`, each pinned by a test there. The other dictation documents quote these by id. Text in angle brackets is filled in unless the row says it stays as written; bold marks a button or link label. Every sentence a person sees is in this table; a pull request that needs a new one adds its row here first. "Biorouter does not … send it anywhere else" (MSG-DISCLOSE-HOST) speaks for Biorouter only, not for a proxy in front of it, and is true only after PR-01 removes the inherited cloud dictation; no document may say it of an earlier build.

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
| MSG-STATUS-POLICY | managed by your organization; dictation is allowed | The `Policy:` line of `biorouter dictation status` and `doctor` when a managed file narrows nothing; it reads `not managed` without one, and the narrowing sentence (MSG-POLICY, MSG-POLICY-CAPTURE, MSG-POLICY-DOWNLOADS) when one applies |
| MSG-DIAG-EXIT | The dictation worker stopped with exit code \<code>. (after a signal: The dictation worker was stopped.) | `status --verbose`, `Worker hello:`; the detail behind an engine failure. This row and the six below it are `DictationError`'s text, pinned by `the_error_sentences_are_the_catalogs` in `crates/biorouter/src/dictation/service.rs` |
| MSG-DIAG-TIME | The dictation worker did not answer in time (\<phase>). | As MSG-DIAG-EXIT; \<phase> stays as written: `hello`, `load`, `transcribe`, `vad_segments` or `shutdown` |
| MSG-DIAG-LOAD | The speech model could not be loaded: \<reason>. | As MSG-DIAG-EXIT; \<reason> is the worker's fixed phrase, such as `model file size does not match` or `voice detector could not be loaded` |
| MSG-DIAG-WORKER | worker \<program> speaks protocol \<n>; this build speaks 1 | As MSG-DIAG-EXIT, for a `BIOROUTER_DICTATION_WORKER` of another build. The same diagnostic, which names the worker binary and nothing else, says `worker <program> sent a frame that could not be read: <reason>`, `worker <program> sent a message that could not be read` or `worker <program> did not start with hello`; and before a spawn, `BIOROUTER_DICTATION_WORKER must be an absolute path` or `the running executable could not be found` |
| MSG-DIAG-FAILED | Dictation failed: \<detail>. | As MSG-DIAG-EXIT; \<detail> is a fixed phrase: `the worker could not start (<error kind>)`, `the dictation supervisor has stopped`, `dictation needs an async runtime`, `a model path is not valid UTF-8`, or the worker's own phrase for an internal error (`no model loaded`, `unsupported message`, `decoding failed`, `voice detection failed`) |
| MSG-DIAG-AUDIO | The audio could not be used. | As MSG-DIAG-EXIT, for audio the worker refused |
| MSG-DIAG-CANCELLED | Dictation was cancelled. | As MSG-DIAG-EXIT, for a request cancelled before it answered |
| MSG-NO-MODEL-CLI | Dictation needs a speech model on this computer (\<host>, account \<user>). Install \<model> (\<size>, from BaranziniLab's GitHub release)? Run `biorouter dictation install`, or type `/dictate install`. | CLI with no model |
| MSG-INSTALL-LOCKED | Dictation is not set up on this server. Ask the person who runs it to run `biorouter dictation install`. | `serve` microphone and Settings when installs are locked |
| MSG-INSTALL-AGENT | Speech models are installed by a person from Biorouter's Settings; an agent cannot install one. | Install route refusal without the proof on a keyed daemon |
| MSG-INSTALL-SHELL | Install the speech model yourself: run this in a terminal, or use Settings > Models > Speech models. | `biorouter dictation install` under `BIOROUTER_TERMINAL=1` |
| MSG-OFFLINE | This computer cannot reach Biorouter's model repository. On a computer that can, run `biorouter dictation urls` and download the files it lists, copy them here, then run `biorouter dictation install --from <file>` (or put them in \<folder> and choose Rescan). | Settings, popover, CLI install |
| MSG-HASH | The downloaded file did not match Biorouter's pin, so nothing was installed. | Settings, popover, CLI install |
| MSG-DISK | Installing \<model> needs \<x> GB free on \<volume>; \<y> GB is available. | Settings, popover, CLI install |
| MSG-TURBO-CPU | This model takes about \<t> s to transcribe \<a> s of speech on this computer. Install it anyway? (buttons **Install anyway**, **Cancel**) | Settings before turbo on a CPU host |
| MSG-LANGUAGE-EN | \<model> understands only English. Choose a multilingual model for other languages, or set the language to English. | **Language** row; CLI `--language` |
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
| MSG-DETECTOR-MISSING | \<model> needs the voice detector \<detector>, which is not installed on this computer. Run `biorouter dictation install <model>`. | CLI `use` (exit 2) and `warm-up` (exit 1) naming a speech model whose detector is gone; `install <model>` then fetches only the detector. The text is `DictationError::DetectorNotInstalled` in `crates/biorouter/src/dictation/service.rs`, pinned by `the_model_sentences` there |
| MSG-CHANGE-SHELL | Change speech models yourself: run this in a terminal, or use Settings > Models > Speech models. | CLI `remove` and `use` under `BIOROUTER_TERMINAL=1` (exit 2) |
| MSG-CONFIRM-TERMINAL | Installing a speech model needs your confirmation at a terminal. To install without one, run: biorouter dictation install --yes \<model> (for an import, `--from <path>` in place of \<model>; for `remove`: Removing a speech model needs your confirmation at a terminal. To remove it without one, run: biorouter dictation remove --yes \<model>) | CLI without a terminal and without `--yes` (exit 2) |
| MSG-CONFIRM-INSTALL | Download and install \<model>? | CLI `install`, at a terminal |
| MSG-TOO-SLOW | \<model> is too slow for dictation on a computer with \<n> cores; after you install it, `biorouter dictation bench` measures it. Install it anyway? | CLI `install` before a bench exists, in place of MSG-TURBO-CPU |
| MSG-BELOW-MINIMUM | \<model> needs at least \<n> GB of memory, and this computer has less. Install it anyway? | CLI `install` |
| MSG-ALREADY | \<model> is already installed (\<location>, \<path>). (for `--system` and `--from`: \<model> is already installed in \<folder>.) | CLI `install` |
| MSG-INSTALLED | Installed \<model> (\<path>). | CLI `install` and `install --from` |
| MSG-WARMING | Preparing \<model> for its first use in the background. | CLI `install` on a Metal host |
| MSG-WARM-UP-SYSTEM | Each account's first dictation prepares \<model> for that account. To prepare it now, run `biorouter dictation warm-up <model>` as that account. | CLI `install` with `--system`, or run as root or Administrator, on a Metal host: the shaders compile into each account's own Metal cache, so nothing is started |
| MSG-RETIRED | \<id> is retired; Biorouter uses \<model> in its place. | Every CLI model command given a retired id |
| MSG-IMPORT-CONFIRM | Install the speech model files in \<from> into \<folder>? | `install --from`, at a terminal |
| MSG-IMPORT-REFUSED | \<path> is \<model>, which was not installed: \<the policy's sentence> | `install --from` when `allowed_models` leaves the model out |
| MSG-IMPORT-UNKNOWN | \<path> is not a Biorouter model file, so it was not installed. | `install --from` |
| MSG-IMPORT-NONE | No file in \<from> matches a Biorouter model. | `install --from` (exit 1) |
| MSG-WAITING-LOCK | Another Biorouter process is installing \<model>; waiting for it to finish. | CLI `install` |
| MSG-PROGRESS | Downloading \<model>… · Checking \<model> against its pin… · Downloaded \<model> | CLI `install`: the plain lines without a terminal, and the progress bar's label and last line on one |
| MSG-REMOVE-CONFIRM | Remove \<model> from \<folder>? | CLI `remove`, at a terminal |
| MSG-REMOVED | Removed \<model> from \<folder>. | CLI `remove` |
| MSG-REMOVE-ELSEWHERE | \<model> is installed in the \<location> folder (\<path>), not in \<folder>. | CLI `remove` (exit 1) |
| MSG-REMOVE-MISSING | \<model> is not installed. | CLI `remove` (exit 1) |
| MSG-USE | Dictation now uses \<model>. | CLI `use` |
| MSG-USE-NOT-SAVED | The choice could not be saved: \<error>. | CLI `use` (exit 1) |
| MSG-VERIFY | `ok      <model> (<location>, <path>)`, with `, an earlier revision` for a superseded copy · `FAILED  <model> (<location>, <path>): the file does not match its pin`, with `, so it was deleted` in this account's folder · `FAILED  <model> (<location>, <path>): <error>` · \<n> model files failed verification. ("1 model file" for one) | CLI `verify`; the last sentence ends it with exit 1 |
| MSG-NOTHING-INSTALLED | No speech model is installed. | CLI `verify` with nothing to check |
| MSG-BENCH | \<model> on \<backend>, \<n> threads: \<a> s of speech in \<t> s (real time factor \<r>, \<suitability>). | CLI `bench` |
| MSG-LIST-NOTES | recommended for this computer · installed (\<location>, \<path>) · not installed · in use · a newer revision exists · not allowed by your organization · required · retired model · not a Biorouter model file | CLI `list`, the notes after each row |
| MSG-SERVER-MEMORY | This server does not have enough free memory to load \<model> (\<x> GB needed, \<y> GB free). Try again later, or ask the administrator to install the Light model. | Composer card on S5 to S8 and S17 |
| MSG-ENGINE-FAILED | Transcription failed on \<where>, so no text was produced. Your recording is kept for 5 minutes. (button **Retry**) | Composer card, clipboard review card; \<where> is "this computer" or "the Biorouter server at \<host>" |
| MSG-ENGINE-FAILED-CLI | Transcription failed. Type /dictate retry within 5 minutes to try the same recording again. | TUI, REPL, shared prompt; `dictate` and `transcribe` print the first sentence and exit 1 |
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
| MSG-SHARED-ORIGIN | Your browser's microphone permission covers every page at \<origin>, including pages that code or an agent wrote there, such as Biorouter apps and notebook outputs. Choose Allow this time unless you trust every page there. | Once per origin before the first recording, on every `serve` origin (SD-16) |
| MSG-413 | The proxy in front of Biorouter refused the audio upload (413). Its request size limit is below 64 KB. | Composer card |
| MSG-WAIT | Waiting for a free connection to Biorouter | Composer card after 1.5 s without an answer |
| MSG-SERVER-BUSY | The server is busy transcribing for other people. Trying again in \<n> seconds. | Composer card while a 429 at stream creation is waited out |
| MSG-SERVER-BUSY-FAILED | The server is still busy transcribing for other people, so nothing was inserted. Your recording is kept for 5 minutes. (button **Retry**) | Composer card, when stream creation still answers 429 30 s after the stop |
| MSG-RECORDING-FAILED | Biorouter could not read the recording, so nothing was inserted. Try again. | Composer card, when the `MediaRecorder` fallback cannot decode what it recorded |
| MSG-NOTHING | Nothing was heard, so nothing was inserted. | Composer card |
| MSG-SILENT | The microphone sent only silence. Check that it is not muted. | Composer card; CLI on Linux |
| MSG-NO-AUDIO-YET | No audio detected. Check that your microphone is not muted. | Composer card and TUI while recording |
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
| MSG-APPS | Apps cannot use the microphone themselves. Click into the app's text field, then use Dictate into this app, or use Dictate to clipboard in the Biorouter app (or `biorouter dictate --copy`) and paste. Your system's own dictation also works, without your Biorouter dictionary. | Preview bar tooltip; SDK reference |
| MSG-APPS-NO-PREVIEW | Apps cannot use the microphone themselves. Use Dictate to clipboard in the Biorouter app (or `biorouter dictate --copy`) and paste. Your system's own dictation also works, without your Biorouter dictionary. | SDK reference, in place of MSG-APPS if M-15 fails |
| MSG-APPS-BROWSER | Apps cannot use the microphone. To dictate here, dictate in Biorouter's own composer and copy the text, or use your system's own dictation. | SDK reference |
| MSG-GNOME | GNOME on Wayland does not let Biorouter set a keyboard shortcut that works in other apps. Use Dictate to clipboard in Biorouter's tray menu (GNOME shows it with the AppIndicator extension), or the microphone button in Biorouter's own composer. | Settings, in place of the shortcut picker |
| MSG-WINDOW-OPEN | On Windows and Linux, Dictate to clipboard works while a Biorouter window is open. It can be minimized. | Settings, under the clipboard switch |
| MSG-SHORTCUT-TAKEN | Another app is already using \<shortcut>. Choose a different shortcut. | Settings |
| MSG-NO-TRAY | Biorouter's menu bar icon is off, so Dictate to clipboard is not in its menu. Choose a shortcut below, or turn on Show in menu bar in Settings > App. | Settings, under **Dictate into other apps**, while the tray is absent |
| MSG-CLIPBOARD-EXPLAIN | Dictate to clipboard puts the text on your system clipboard. After that it is outside Biorouter: any program on this computer can read it, including an agent with shell access in any chat; clipboard managers may keep it; Universal Clipboard or Windows cloud clipboard may sync it to your other devices; and the app you paste into decides where it goes. Your private dictionary terms are used only for a recording you start with Include my private terms. | Settings, once, when the switch turns on |
| MSG-CLIPBOARD-COPIED | Copied \<n> words. Paste with \<key>. | Notification |
| MSG-CLIPBOARD-NOTHING | Nothing was heard, so the clipboard was not changed. | Notification |
| MSG-CLIPBOARD-ECHO | Only dictionary words came back, which usually means the recording was silent. Nothing was copied. | Notification |
| MSG-NO-CLIPBOARD | No clipboard is reachable here, so the text is printed below. Copy it from the terminal. | `dictate --copy` (exit 5) |
| MSG-CLIPBOARD-WAIT | Copied. Waiting up to 60 s for you to paste… | `dictate --copy` on Linux |
| MSG-REMOTE-HTTP | Dictation sends your voice to \<host> without encryption, as your typed messages already are. Use an https address for this server to protect it. Continue? | S17, once per `http:` backend URL |
| MSG-REMOTE-NO-KEY | Dictation on \<host> uses only your public dictionary terms until you enter this server's User Action Key in Settings > App > Use external server. Installing models there also needs the key. | S17, keyed backend without the key (O-17) |
| MSG-DISCLOSE-LOCAL | Speech is transcribed by Biorouter on this computer. Audio is not saved. | Settings, popover (the desktop's own daemon) |
| MSG-DISCLOSE-REMOTE | Audio is transcribed on \<host>, the computer running Biorouter's service. Models install there, not on this device. | Settings, popover, `ExternalBackendSection.tsx` (S17) |
| MSG-DISCLOSE-HOST | Your voice is sent to the Biorouter server on \<host> and turned into text there. Biorouter does not save it or send it anywhere else. The text appears here for you to check before you send it. | Settings, popover, and once in the card (`serve`) |
| MSG-SETUP | Dictation turns your speech into text \<where>. It needs a speech model: \<name> (\<size>), downloaded once from the Baranzini Lab's model repository on GitHub into \<folder>. Audio is not saved. (where is "on this computer" or "on the Biorouter server at \<host>"; buttons **Install**, **Not now**; link **More in Settings**) | Setup popover |
| MSG-SETUP-SUGGEST | The agent and Biorouter can propose terms they see in your chats. Biorouter also looks for terms in every message you write, on this computer. A term goes into your dictionary when you ask the agent to add it, or agree when it offers; everything else waits for you to accept it. (shown under **Suggest dictionary words from my chats**, which starts ticked, O-33; the middle sentence is left out when `learn_from_chats` starts off; the last sentence above is the `on_request` variant, the default the owner decided on 2026-11-05 (O-3); when `agent_writes` is `suggest` the last sentence reads "Nothing is added until you accept it.", and when it is `add` it reads "The agent's terms are added at once, and you can undo each one; everything else waits for you to accept it.") | Setup popover, Settings |
| MSG-PRIVATE-DRAFT | This message was dictated with your private dictionary terms turned on, so it may contain them, and \<model> is a public model. Send it anyway? (buttons **Send anyway**, **Cancel**) | Composer, before a send, a queue or a steer |
| MSG-PRIVATE-DRAFT-SWITCH | This draft was dictated with your private dictionary terms turned on, so it may contain them. If you switch this chat to \<model>, a public model, the draft goes there with your next message. Switch anyway? (buttons **Switch anyway**, **Cancel**) | Model switcher |
| MSG-PRIVATE-TERMS | Your private dictionary terms may appear in this text, and Biorouter cannot keep them away from public models or from other institutions' models once you paste or post it. Include them? (buttons **Include**, **Public terms only**; at a terminal, `[y/N]`) | After **Include my private terms** (Crew, **Dictate into this app**, the tray) and `biorouter dictate --private-terms`; one recording |
| MSG-PRIVATE-TERMS-REFUSED | Private dictionary terms need you at your own terminal and a private model configured on this computer, so `--private-terms` was refused. Run the command without it to dictate with public terms. | `biorouter dictate --private-terms` when DD-77 does not admit it (exit 2) |
| MSG-PUBLICIZE-WARN | This term came from a private chat. Making it public lets it appear when you dictate into chats on public models. (buttons **Make public**, **Cancel**) | Dictionary page |
| MSG-ROW-PRIVATE | Used only in private chats | Dictionary page, the start of line two of a private row with no institution: a term added with no chat, or from a private chat whose model and connectors name no institution (O-6, O-32, O-35, O-36) |
| MSG-ROW-PRIVATE-INSTITUTION | Used only in private chats on \<institution> models and on local models (a row that names two or more institutions reads "Tied to \<institutions>; used only in private chats on local models") | Dictionary page, the start of line two of a private row that carries institutions; \<institution> is the institution's published name (else its id); in the two or more form, \<institutions> is each institution's published name (else its id) joined with "and" (O-32, O-35) |
| MSG-ROW-PRIVATE-LOCAL | Used only in private chats on local models | Dictionary page, the start of line two of a private row whose institution list is empty: an import that could not be read, a write whose chat union named more than 8 institutions, or a write whose chat record or chat model could not be read (O-32, O-35) |
| MSG-EXPORT-PRIVATE | This file leaves Biorouter. It will include your private terms. (buttons **Export with private terms**, **Export public terms only**) | Dictionary page export |
| MSG-DICTIONARY-AGENT | Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms. | Dictionary route refusals to an agent |
| MSG-DICTIONARY-HOST-TIER | Do this on the server: `biorouter dictionary <verb> <term>` (as the service user). | Dictionary page on `serve`, before the click; \<verb> is `publicize` for **Make public…** and `privatize` for **Make private…**; the route's 403 leaves \<term> as written |
| MSG-DICTIONARY-HOST-SETTINGS | Do this on the server: `biorouter dictionary settings <option>` (as the service user). | Dictionary page on `serve`, before the click, for raising **Agent's terms** or turning chat learning or suggestions on; \<option> is `--agent-writes <mode>`, `--learn-from-chats on` or `--suggest-from-chats on`, from the control; `authority.raise_settings.reason` |
| MSG-DICTIONARY-HOST-PRIVATE | Private terms are added on the server: `biorouter dictionary add --private <term>` or `biorouter dictionary import <file>` (as the service user). | Dictionary page on `serve` without private standing, before the click on **Only in private chats**; the 403 for a private add or import |
| MSG-DICTIONARY-FILE | The dictionary file at \<path> could not be read, so nothing was changed. Fix or remove it, then try again. | Dictionary page, CLI |
| MSG-NOT-FOUND | No dictionary entry with that id. | Dictionary routes |
| MSG-SAME-LETTERS | A term with the same letters exists: \<written> | Term dialog, CLI add |
| MSG-TERM-TOO-LONG | Use a term of at most 64 characters. | Term dialog, dictionary routes, CLI add (code `too_long`) |
| MSG-TERM-WORDS | Use a term of at most six words. | As above (code `too_many_words`) |
| MSG-TERM-PLAIN | Use one line of plain text. | As above (codes `line_break`, `control_character`, `angle_bracket`) |
| MSG-TERM-ADDRESS | A dictionary term cannot be a web address, an email address or a file path. | As above (codes `url`, `email`, `path`) |
| MSG-TERM-IDENTIFIER | This looks like an identifier or a key, so it was not added. | As above (codes `digits_only`, `long_hex`, `secret_shaped`) |
| MSG-DICTIONARY-CAP | Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one. | As above (code `cap`) |
| MSG-DICTIONARY-STALE | This term changed since you opened it. Reload it and try again. | As above (code `stale`) |
| MSG-EMPTY | No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1. | Dictionary page empty state |
| MSG-TOOL-SUGGESTED | Recorded \<n> suggestions for the person's review in Dictionary. Nothing changes in dictation until they accept. | `dictionary__add_terms` result |
| MSG-TOOL-ADDED | Added \<n> terms to the dictionary. They are marked for the person's review. | `dictionary__add_terms` result |
| MSG-TOOL-REVIEW-HINT | Review them with `biorouter dictionary review` on this computer, or in Dictionary in Biorouter. | Appended on ACP and the CLI |
| MSG-SERVE-TUNNEL | Browsers refuse the microphone and this page on http://\<lan addr>:\<port>. On your own computer run `ssh -N -L <port>:127.0.0.1:<port> <host>`, then open http://127.0.0.1:\<port>/?t=\<token>. | `serve` banner: a non loopback bind without `--public-url`, outside a container; the port, token and host filled in, and no `?t=` part under `--no-token` |
| MSG-SERVE-CONTAINER | This runs in a container. On the computer that publishes its port, open http://127.0.0.1:\<host port>/?t=\<token>, where \<host port> is the port published for container port \<port> (with Docker: -p 127.0.0.1:\<host port>:\<port>). Browsers refuse the microphone on other plain http addresses. | `serve` banner: a non loopback bind without `--public-url`, inside a container; the token and \<port> filled in, \<host port> left as written |
| MSG-CREW-TIP | Dictate. The text goes into the box for you to check; nothing is posted until you press Send. | Crew microphone tooltip |
| MSG-RESTART-BUSY | Dictation in progress. Restart anyway? | **Restart & update** while dictating |
| Existing 403, unchanged (`web_ui.rs` `insecure_address`) | `Open Biorouter through localhost or HTTPS. Use an SSH tunnel, or an HTTPS reverse proxy that passes the browser's Sec-Fetch-* headers through, and a current browser that sends them.` | S7 |
| Existing Crew refusal (`CrewNeedsDesktop.tsx`) | Crew needs the Biorouter desktop app | Crew in a `serve` browser |

## Related documentation

- [Implementation plan](implementation-plan.md): the requirements, the decision register this document follows, and the pull requests named in each Release row.
- [Engine and models](engine-and-models.md): the worker, the model catalog, the installer, the managed policy block and the daemon side of the `/dictation/*` routes.
- [Dictionary](dictionary.md): the store, the bias set, authority per surface, the **Dictionary** page and the `dictionary` capability that several surfaces above refer to.
- [Testing and release](testing-and-release.md): the tests, CI jobs and the per surface manual checklist behind each Acceptance row.
- [Browser access](../deployment/browser-access.md): how `biorouter serve` is reached and proxied, which S5, S6 and the proxy guidance build on.
- [Serve decisions](../deployment/serve-decisions.md): SD-8, SD-14 and SD-15, and the SD-16 record this work adds.
- [Headless Linux deployment](../deployment/headless-linux.md): the systemd service account and the plain HTTP address that S7 and S8 correct.
- [Jupyter AI persona](../integrations/jupyter-ai-persona.md): the integration where dictation is not offered, by the owner's decision O-30 (S13).
- [Crew](../crew/README.md): the workspace whose composers S14 adds microphones to, and the manual its new strings join.
- [Apps SDK reference](../apps-sdk/sdk-reference.md): where MSG-APPS and MSG-APPS-BROWSER are published for app authors.
- [CLI command reference](../cli/command-reference.md): the `serve` and `acp` sections that gain dictation lines.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules behind every Bias row, a safety boundary against mistakes by cooperative actors, not a security boundary against a determined adversary.
- [Managed policy](../security/managed-policy.md): the policy file whose `dictation:` block every surface honours.
- [Shared controls](../desktop-ui/shared-controls.md): the fill and focus rules the microphone button and its pill follow.
- [Dictation tools survey](../research/voice-dictation/dictation-tools-survey.md): how other dictation tools handle capture, gestures and lost speech, which several choices here answer.
