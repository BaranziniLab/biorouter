# Dictation test checklist

> **What this is.** The manual test pass for local dictation on each of its surfaces, S1 to S20, written to be run against a signed release build, the dated log of the measurements M-1 to M-22 that announcing version 1 of dictation depends on, and the draft release notes for the first release that carries dictation.
> **Status:** Current. Written 2026-11-10 on branch `feat/local-dictation`, after every code pull request of the plan was committed there (the last, PR-28 and PR-29, in `9a96e0132`) and before the release that carries dictation; version 1 is not announced. Each section ends with a status line, dated 2026-11-10, that says what evidence exists and what is still open; none of the twenty sections has passed on a signed build yet. Extend it when a behaviour changes; do not delete rows that still describe shipped behaviour.
> **Audience:** the maintainer who verifies a release that carries dictation, and reviewers who check that a surface's section passed before version 1 is announced.

Every row states what to do, what should happen, and what a failure looks like, because several of these fail by doing nothing visible, which reads as a pass. The **CI covers** column names the job or test that already holds a row's logic, so a reader knows the row checks the real device, account, browser or terminal, not the logic.

Identifiers follow the [implementation plan](../dictation/implementation-plan.md): S1 to S20 are its surfaces, PR-01 to PR-33 its pull requests (PR-07 and PR-32 withdrawn), O-1 onward the owner's decisions, MSG-… the exact sentences in the [message catalog](../dictation/surfaces.md#message-catalog), and M-1 to M-22 the measurements of [testing and release](../dictation/testing-and-release.md#measurements) (M-9 and M-13 withdrawn).

> **Warning.** Run this against an installed, signed build. A dev build uses a different configuration root, makes the launching terminal the process the macOS microphone prompt names, and is never notarized. A pass on a dev build is not a pass. Rows that say "dev app" are the only exception.

## What has to pass

Version 1 of dictation is announced only when all of these hold (DD-94, O-29):

1. Every section S1 to S20 other than S13 passes on its surface with a signed build of the release being announced. S13's section states the owner's decision O-30 and holds no rows.
2. M-1 to M-8, M-10, M-12, M-15 to M-20 and M-22 are recorded with dates in the [measurement log](#measurement-log). M-11, M-14 and M-21 feed later decisions; M-9 and M-13 are withdrawn.
3. The speech model releases in `BaranziniLab/biorouter-models` pass `gh release verify-asset`, and `dictation-model-pins.yml` is green.

No accuracy figure about the product goes into these rows, the release notes, the announcement or the interface (DD-95). A row that records a transcript records whether the expected spelling appeared, not a score.

## How to read a status line

Each section ends with a status line dated 2026-11-10. It names the evidence that exists and what is still open, with these words:

| Word | Meaning |
|---|---|
| CI | A hosted run of a named workflow on a named commit, with the result GitHub recorded |
| dispatched | A hosted run that was started on a named commit and had no result when the line was written |
| dev app | An unsigned development build on the maintainer's Apple Silicon Mac |
| live | A real model turn, on `versa_azure` GPT-5.5 unless the line says otherwise |
| synthetic | Audio fed from a WAV file through a fake device, not a microphone |
| physical microphone | A real microphone in front of a person |

None of these is a pass of a row on a signed build. A section passes only when a person runs its rows on the surface and records the result.

## CI on the branch

The commits of 2026-11-10 and their hosted runs, as GitHub recorded them; the results for `9a96e0132` and `8955295b5` were read at 00:10 PST on 2026-11-11, after both runs finished. A green run belongs to its own commit only; a later green run covers the earlier commits only because they are its ancestors.

| Commit | What it holds | `rust.yml` | `frontend.yml` |
|---|---|---|---|
| `50e79b308` | PR-23, the `dictionary` capability | 42924892586 cancelled: eight of nine jobs green, and `test (ubuntu-latest)` stopped at its 90 minute limit while the ACP test `test_acp_load_session_is_not_activity` hung | 42924904901 green |
| `999ea27f9` | PR-30, ACP | 42926830786 failed one Ubuntu test, `no_console_window_census`, fixed in `8558ccf61` | none |
| `8c865d11d` | PR-31, the Docker image | none of its own; its native gate ran locally (S16) | none |
| `8558ccf61` | the census fix | 42934104077 failed the `serve` job's Firefox teardown, fixed in `07bb853ec` | none |
| `65ba7dae2` | ACP answers a failed session load instead of closing the connection, and bounds every ACP server test | 42944486393 green on all nine jobs | 42944496975 green |
| `491246227` | PR-24, the chat extractor | 42949720898 green on all nine jobs | 42949725205 green |
| `9a96e0132` | PR-28 and PR-29, with the tool row layout fix `30a13154b` | 42975902058 green on all nine jobs | 42975905256 failed: seven of eight jobs green, and `Unit tests (vitest)` failed two tests that passed in 42949725205, `main.appLaunch.test.ts` (two `windowDaemons.set` calls in `main.ts` where it expects one) and `tabSwitchFrames.browser.test.tsx` (seven switches on **Settings > App** where it expects six) |
| `8955295b5` | the two test fixes, no other change | none of its own; no Rust file changed after `9a96e0132` | 42982772021 green on all eight jobs |

So every code pull request on the branch has a fully green `rust.yml` run: PR-23, PR-30, PR-31 and PR-24 through 42949720898 at `491246227`, and PR-28 and PR-29 through 42975902058 at `9a96e0132`. Their `frontend.yml` run 42975905256 failed two tests that had not been updated for them: the second `windowDaemons.set` is PR-28's hidden capture and review windows, which reuse the daemon their owner window was attached with (`createAnywhereWindow` in `main.ts`), and the seventh switch is PR-29's **Server** section (`ExternalBackendSection.tsx`). `8955295b5` updates both tests and changes no product code; its `frontend.yml` run 42982772021 is green on all eight jobs.

## Before you start

- [ ] Note the build: version, platform, architecture, and whether it is signed and notarized.
- [ ] Use an installed, signed build, never a dev build, except where a row says "dev app".
- [ ] Use a fresh operating system account, or run `tccutil reset Microphone com.electron.biorouter` on macOS.
- [ ] Configure two providers: one private (Llama Server, or a private Versa model) and one public.
- [ ] Keep privacy tiers on (**Settings > App > Privacy**).
- [ ] Have a built in microphone and a Bluetooth headset.
- [ ] Add `UCSF` and `EPAS1` as public entries, and `EGLN1` from a private chat, so it is private.
- [ ] Keep a short recording, `memo.mp3`, that says those terms. A phone's `memo.m4a` converts with `ffmpeg -i memo.m4a memo.mp3`; keep the M4A too, for S8.5.

## S1 desktop app, macOS Apple Silicon

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S1.1 | On a fresh account, open a chat and click the microphone left of **Send**. | A muted microphone; the setup popover with MSG-SETUP, **Install**, **Not now**, **More in Settings**, and **Suggest dictionary words from my chats** ticked. Nothing downloads. | A download starts on the click; no microphone. | `DictationButton.test.tsx` |
| S1.2 | In the packaged app, press **Install**; quit at about half; reopen and press **Install** again; then press **Show in Finder** in Settings. | Progress in the ring; the second install resumes; **Settings > Models > Speech models** shows `whisper-large-v3-turbo-q5_0` in use, the folder `~/.local/share/biorouter/dictation/models/` and the account; **Show in Finder** opens that folder; the Components group opens once and shows **Dictionary**; no toast. | The download restarts from zero; files elsewhere; Finder opens nothing; no **Dictionary** row. | installer tests, `dictationModelsFolder.test.ts` |
| S1.3 | Click the microphone. | macOS asks with "Biorouter uses the microphone only while you dictate, and does not save the recording." | Electron's default text; no prompt and silence. | `forgeMacUsage.test.ts`, `cmd_verify` |
| S1.4 | Allow, say "Ask SPOKE which drugs target EPAS1 at UCSF", click again; then press ⌘Z once. | The text appears at the caret with `EPAS1` and `UCSF` spelled as in the dictionary; nothing is sent; one ⌘Z removes exactly the dictated words. | Sent without Enter; inserted into another field; ⌘Z removes more or less. | Electron job, the dev app run |
| S1.5 | Run `tccutil reset Microphone com.electron.biorouter`, relaunch, click, and choose Don't Allow. | MSG-MIC-DENIED-MAC; **Open settings** opens Privacy & Security > Microphone; after allowing and restarting, dictation works. Record the error name and the TCC log line (M-6). | A silent failure; a generic error; the wrong pane. | none |
| S1.6 | Tap ⌘⇧D; hold it a second while speaking; press Escape during a recording; press Enter during a recording. | A tap toggles; a hold records while held; Escape discards; Enter stops and inserts and does not send. | Enter sends; Escape inserts. | `DictationButton.test.tsx` |
| S1.7 | Record, then lock the screen. Record again and choose **Restart & update**. | The words land in the draft on lock; the update asks MSG-RESTART-BUSY. | Speech lost; a restart without asking. | none |
| S1.8 | In a chat on the private provider, dictate "EGLN1", then switch the chat to the public model. In a chat on the public model, dictate "EGLN1". | For an unsent chat on Home or a new tab, MSG-PRIVATE-DRAFT-SWITCH, and **Switch anyway** proceeds; in a chat that exists, MSG-PRIVATE-DRAFT asks at the send. In the public chat the private entry never shapes the spelling. | No warning; a private entry shapes a public chat's dictation. | `ChatInput.dictation.test.tsx`, `ModelsBottomBar.privateDraft.test.tsx`, `bias.rs` |
| S1.9 | On a macOS 12 virtual machine, launch the notarized build, install the Light model and dictate once; repeat on macOS 13. | The app starts and transcribes on Metal (M-4). | dyld refuses the app; the worker crashes. | `check-macos-deployment.sh` |
| S1.10 | On a fresh account with the Components group collapsed, install a speech model (or put its files in the model folder and press **Rescan**); open **Dictionary**, add EPAS1 with **Add a term…**, and dictate a sentence with EPAS1 into a new chat. | The group opens once when the model is in use, with **Dictionary** after **Knowledge**; the term's row appears; the dictated text spells `EPAS1`, and the row then reads "1 use". Collapse the group and relaunch: it stays collapsed. | The group never opens, or opens at every launch; the term changes nothing. | `AppSidebar.test.tsx`, `DictionaryView.test.tsx` |
| S1.11 | In a split with a public chat on the left (current) and a private chat on the right, right click a misspelled word, then a selected word, in the right pane's transcript; then do the same in a shared transcript. | The menu reads **Add to spelling dictionary** and **Add to Biorouter dictionary…**; the second opens the term dialog with the word and **Only in private chats** ticked in both places; the new row is private, with the right chat's institutions or MSG-ROW-PRIVATE-LOCAL for the shared transcript. | The old "Add to dictionary" label; the box unticked; a public row. | `contextMenu.test.ts`, `TermDialog.test.tsx` |

Known behaviour, kept by the owner on 2026-11-10 (O-43): a dictation into a chat whose agent is not loaded yet leaves out private terms that carry an institution, because the target's model is unstated until the agent is loaded. Private terms that carry no institution still apply. Do not record this as a failure of S1.4 or S1.8.

**Status, 2026-11-10.** Not passed on a signed build. Committed: Settings > Models > Speech models (PR-18, `7632bb8b2`, frontend run 42364926887 green), the composer microphone (PR-19, `dc375d3fc`; frontend run 42380873522 green after the follow up `84ed0aaa9`), the Dictionary page (PR-20, `74f653837`, frontend run 42389187127 green), correction learning (PR-22, `6ee9c2c71`, Rust run 42430350294 and frontend run 42430352489 green), and the `dictionary` capability (PR-23, `50e79b308`; see [CI on the branch](#ci-on-the-branch)), which adds S1.1's **Suggest dictionary words from my chats** box. Dev app, 2026-11-07: the Settings half of S1.2 (install, quit at 67 %, resume at 68 %, "In use" 15.2 s after the click), and S1.10 (Rescan to "In use" 3.7 s, the row on screen 1 ms later; "… target E-PAS1." before the term, "… target EPAS1." after). Synthetic, 2026-11-07: correction learning on q8 Metal (timings in the measurement log). Open: every row on the packaged, signed and notarized app; S1.3 and S1.5 with a fresh TCC state (M-6); S1.9 on macOS 12 and 13 (M-4); S1.11, which needs a person or full screen control, because Electron's native menu cannot be opened over CDP and the background desktop control tool refuses right click menus; every row with a physical microphone.

## S2 desktop app, macOS Intel

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S2.1 | On an Intel Mac with AVX2, fewer than 16 cores and no `bench` record, open the setup popover. | `whisper-small.en-q8_0` (264 MB) is the offered default; turbo is marked `too_slow`. | Turbo offered by default. | suitability tests |
| S2.2 | In Settings press **Install** on turbo; after it installs, press **Use** and **Test speed**; remove turbo and press **Install** again. | Before the first install, MSG-TOO-SLOW-SETTINGS (no time exists yet; **Test speed** measures only the model in use); after the measurement, MSG-TURBO-CPU with the measured time when the factor is above 0.5, and no question when it is not (turbo is then rated by its record); **Install anyway** works each time. Record both models' real time factor at 4 and 8 threads (M-10). | Turbo installs without a warning. | `speechModelStore.test.ts` |
| S2.3 | On Apple Silicon with macOS 15 or later, install the Intel build and dictate. | Dictation works under Rosetta. | Exit 20; a crash. | `smoke_mac` x64 |
| S2.4 | On Apple Silicon with macOS 12 to 14 (any version before 15), open the Intel build's popover. | MSG-CPU-ROSETTA; no download offered. | A download, then a failure. | `src/cpu.rs` table |
| S2.5 | On a processor without AVX2 (a 2013 Mac Pro, or a VM with a masked CPU model). | MSG-CPU, then MSG-CPU-DESKTOP, before any download. | A download, then a crash. | `check-dictation-isa.sh` |

**Status, 2026-11-10.** Not passed. Only source review, unit tests and CI cross builds exist for this surface. No Intel Mac, Rosetta or masked CPU run has been recorded. Open: every row, and M-10 through S2.2.

## S3 desktop app, Windows x64

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S3.1 | Install with `Biorouter-Setup-<ver>.exe` and install the model. | Files under `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\`, the local data folder, not under `%APPDATA%\UCSF\biorouter\data\` (Roaming, where Biorouter's other data lives) or `app-<version>`. | Files in Roaming or inside the app folder. | path agreement test |
| S3.2 | Turn off "Let desktop apps access your microphone" and click. | MSG-MIC-DENIED-WIN; **Open settings** opens `ms-settings:privacy-microphone`. Record which `getMediaAccessStatus` value the switch produces on Windows 11 (`denied` or `restricted`). | Silence; a generic error. | `micErrors.test.ts` |
| S3.3 | Let another app hold the microphone exclusively and click; later unplug a USB headset while recording. | MSG-MIC-BUSY; unplugging stops and inserts what was heard. | A hang; lost speech. | none |
| S3.4 | Update through the installer; then run the zip build. | The model stays installed and in use in both. | The update removes the model. | none |
| S3.5 | Use Ctrl+Shift+D; use the tray's **Dictate to clipboard** with the window minimized; then close the last window. | Both work while a window exists; MSG-WINDOW-OPEN under the switch; closing the last window quits. | The tray item does nothing while minimized. | `dictationAnywhere.test.ts` |
| S3.6 | On an account whose name holds characters outside ASCII, install and dictate. | It works. | The model fails to load. | `windows-native-dictation` |

**Status, 2026-11-10.** Not passed. The engine's native Windows measurement M-1 is recorded (CI, 2026-11-06, `rust.yml` run 42280907734 at `78d5ca159`: the self test, `status --verbose`, and a model install and `transcribe` under a folder name with a character outside ASCII on a hosted runner), and `windows-native-dictation` was green again in 42949720898 at `491246227`. That holds S3.6's logic on a runner, not on the packaged app. S3.5's **Dictate to clipboard** and tray item are committed with PR-28 in `9a96e0132` and have not run in a packaged app. That commit's `rust.yml` run 42975902058 is green on all nine jobs, and its `frontend.yml` run 42975905256 failed two unit tests, both fixed in `8955295b5` (tests only), whose `frontend.yml` run 42982772021 is green on all eight jobs ([CI on the branch](#ci-on-the-branch)). Open: every row on a Squirrel install and on the zip, with a physical microphone.

## S4 desktop app, Linux deb and rpm

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S4.1 | Ubuntu 24.04 (PipeWire), Ubuntu 22.04 (PulseAudio), Fedora (rpm): install, install the model, dictate. | No OS prompt; text inserted. Record throughput on each (M-7). | Silence; dropped audio. | Electron job (macOS only) |
| S4.2 | With no microphone; then with the microphone held by another app. | MSG-NO-DEVICE; MSG-MIC-BUSY. | A spinner that never ends. | `micErrors.test.ts` |
| S4.3 | On GNOME Wayland open **Dictate into other apps**. | MSG-GNOME in place of the shortcut picker; the tray item works with the AppIndicator extension. | A shortcut that registers and never fires. | none |
| S4.4 | As root run `biorouter dictation install --system`; remove the person's copy. | Status shows `location: system`; dictation works. | The system copy is not found. | lookup order tests |
| S4.5 | Upgrade the package. | The model stays. | The upgrade removes it. | R6 guard |

**Status, 2026-11-10.** Not passed. Package checks and the lookup order tests exist; no native Linux desktop run is recorded. S4.3's row is committed with PR-28 in `9a96e0132` and has not been seen on GNOME. That commit's `rust.yml` run 42975902058 is green on all nine jobs, and its `frontend.yml` run 42975905256 failed two unit tests, both fixed in `8955295b5` (tests only), whose `frontend.yml` run 42982772021 is green on all eight jobs ([CI on the branch](#ci-on-the-branch)). Open: every row on the three distributions with a physical microphone (M-7).

## S5 `biorouter serve`, browser on the same machine

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S5.1 | Start `biorouter serve` with no model. | The banner says "Dictation: no speech model; run `biorouter dictation install`". | No dictation line. | banner snapshots |
| S5.2 | Open the URL in Chrome and install from Settings. | The text says the download goes to the server; MSG-DISCLOSE-HOST on first use; the model lands in the host account's folder; **Show in Finder** is absent. | "On this computer" wording; a Finder button. | `serve` job |
| S5.3 | Dictate in Chrome, Firefox and Safari 27 at `http://127.0.0.1:8765`; deny once in each. | Text inserted; a denial shows MSG-MIC-BROWSER. Record Safari (M-8). | Safari never asks. | `serve` job (Chromium, Firefox) |
| S5.4 | On **Dictionary**, open the menu of a private row; then open **Add** on a host whose operator's provider is public. | **Make public…** is disabled with MSG-DICTIONARY-HOST-TIER; **Only in private chats** is disabled with MSG-DICTIONARY-HOST-PRIVATE; raising **Agent's terms** is disabled with MSG-DICTIONARY-HOST-SETTINGS. | The click fails after the fact. | vitest |
| S5.5 | Restart with `biorouter serve --no-dictation-install`. | Install refused with MSG-INSTALL-LOCKED; an installed model still works. | Install proceeds. | `dictation_no_user_key.rs` |
| S5.6 | Press ⌘⇧D in the served composer in each browser. | Record whether the page receives the chord and which tap durations register (M-18). | n/a | none |

**Status, 2026-11-10.** Not passed on a signed build. PR-21 is committed (`814a14749`); `rust.yml` run 42451102000 on that commit was green, including the `serve` job that dictates in Chromium and Firefox with a fake audio device, and the `serve` job was green again in 42949720898 at `491246227`. S5.4's **Agent's terms** control arrived with PR-23 (`50e79b308`). Synthetic, report file dated 2026-11-07: a served dictation on q8 Metal through loopback nginx (timings in the measurement log). Open: Safari (M-8), the chord (M-18), S5.4 by hand, and every row with a physical microphone.

## S6 `serve` behind an HTTPS reverse proxy or a JupyterHub prefix

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S6.1 | Behind nginx over HTTPS with HTTP/2 at `/user/<name>/biorouter/`, dictate. | MSG-SHARED-ORIGIN once before the first recording; the worklet loads; text inserted. | The microphone fails on the first click everywhere. | `serve` job (nginx prefix) |
| S6.2 | Set `client_max_body_size 32k`, dictate; then set it to `128k`. | At 32k, MSG-413; at 128k, dictation works with smaller chunks. | A silent failure. | `serve` job (413 halving) |
| S6.3 | Repeat S6.1 behind `configurable-http-proxy` and `jupyter-server-proxy`. | Record each proxy's result and the setting that keeps request bodies off its disk (M-17). | n/a | none |
| S6.4 | Have four people dictate at once on one host. | All finish; MSG-SERVER-BUSY under load, then a retry; MSG-SERVER-MEMORY when memory is short. | A stream that never finishes. | `dictation_routes.rs` |
| S6.5 | Two `serve` tabs in one browser while a reply streams, then dictate in the second. | Record PUT timings; MSG-WAIT appears if a PUT is held (M-5). | n/a | transport tests |
| S6.6 | On an iPhone in Safari and an Android phone in Chrome, open the HTTPS `serve` address; dictate once, then deny once. | Text inserted; the denial shows MSG-MIC-BROWSER (M-8). | No prompt; a dead microphone. | none |

**Status, 2026-11-10.** Not passed. The rows' logic is held by the `serve` job (green in run 42451102000 at `814a14749` and in 42949720898 at `491246227`) and `dictation_routes.rs`. Synthetic, report file dated 2026-11-07: a served dictation on q8 Metal behind HTTPS prefix nginx on the maintainer's Mac (timings in the measurement log). Open: S6.3 and the other proxy families (M-17), S6.5 while a real turn streams (M-5), S6.6 on phones (M-8), and S6.4 with four people.

## S7 `serve` at a plain HTTP network address

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S7.1 | Start `biorouter serve --host 0.0.0.0` and open `http://<LAN address>:8765`. | The existing refusal text, unchanged; the banner prints MSG-SERVE-TUNNEL, not a plain HTTP LAN URL. | A composer with a dead microphone; a LAN URL in the banner. | banner snapshots |
| S7.2 | Run `ssh -N -L 8765:127.0.0.1:8765 <host>` and open the address `serve` printed, with its `?t=` part. | Dictation works as in S5. | n/a | none |
| S7.3 | Behind a proxy that rewrites `Host` and serves plain HTTP. | The composer shows MSG-INSECURE and the microphone is unavailable. | A microphone that fails on click. | `dictationStore.test.ts` |

**Status, 2026-11-10.** Not passed. The banner snapshots and `dictationStore.test.ts` hold the logic (PR-21). Open: every row on a real LAN host, an SSH tunnel and a rewriting proxy.

## S8 CLI only Linux deb and rpm on a server

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S8.1 | On a server whose systemd unit runs as `biorouter`, run `sudo -u biorouter -H biorouter dictation install`. | `dictation status` shows account `biorouter` and `~biorouter/.local/share/biorouter/dictation/models/`. | The model installs under root. | package smoke |
| S8.2 | Run `biorouter dictation install` as another user. | MSG-OTHER-ACCOUNT. | A model the service never sees. | none |
| S8.3 | Run `sudo biorouter dictation install --system`. | `/var/lib/biorouter/dictation/models/`, folders 0755 and files 0644; the service finds it. | Files readable only by root. | none |
| S8.4 | Run `biorouter dictate` on the server over SSH, then at its local console. | Over SSH: MSG-LINUX-NOAUDIO, then MSG-SSH-SERVICE when the `biorouter serve` service runs (MSG-SSH-ROUTES otherwise); at the console: MSG-LINUX-NOAUDIO, then MSG-LINUX-ROUTES; exit 2 and no device opened in both. | ALSA noise on the terminal; a recording of silence. | environment tests |
| S8.5 | Run `biorouter transcribe memo.mp3`, then `biorouter transcribe memo.m4a` with an AAC memo and with a Voice Memos recording made with Lossless on, then a recording in Opus or WebM. | The text on stdout for the MP3, for an AAC `memo.m4a` (O-39, PR-12b) and for an Apple Lossless one (O-41, decided by the owner on 2026-11-13); for Opus or WebM, exit 1 with MSG-TRANSCRIBE-UNSUPPORTED and its `ffmpeg` line. | An AAC or Apple Lossless M4A refused; a crash on Opus. | `dictation_cli.rs` |
| S8.6 | On a VM with a masked CPU model. | MSG-CPU-SERVER, then MSG-CPU-VM. Run the census command on lab hosts (M-11). | A crash with code 132. | `check-dictation-isa.sh` |

**Status, 2026-11-10.** Not passed. Package smoke and `dictation_cli.rs` hold the logic. PR-12b (`bd985898a`, `rust.yml` run 42387673290 green) made M4A, AAC and Ogg Vorbis readable, which changed S8.5 from the refusal the plan first wrote; Apple Lossless, which PR-12b refused as O-41's default, decodes since the owner decided O-41 on 2026-11-13. Open: every row with the CLI only packages on a real server whose service runs as a systemd account; S8.6 on a VM with a masked CPU model; M-11, which feeds O-11.

## S9 interactive CLI on a local machine

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S9.1 | In the TUI press Ctrl+T, speak, press Enter. | `◌ Starting microphone…`, then `● REC` with the timer and level bars, then `◌ Transcribing`; the text at the cursor; not sent. | A literal `t` typed; the text sent. | `draw_input` snapshots |
| S9.2 | Repeat in the classic REPL, the shared daemon prompt, and `biorouter session attach` with `/dictate`. | The REPL dictates instead of transposing; `attach` asks `Send? [y/N]` and defaults to No. | A transposition; an automatic send. | pseudo terminal test |
| S9.3 | On macOS try Terminal, iTerm2, Ghostty, VS Code's terminal and tmux; deny in one. | The prompt names the terminal app; a denial shows MSG-TCC-ZERO naming it. Record tmux attribution. | Silence with no message. | controller tests |
| S9.4 | On Windows try Windows Terminal and conhost with the privacy switch off. | MSG-WIN-ZERO. | Silence. | controller tests |
| S9.5 | Run `biorouter dictation test --seconds 10` on macOS, Windows 11, Ubuntu 24.04 (PipeWire) and 22.04 (PulseAudio). | Frames within 2 % of wall time; the first frame in under 300 ms (M-7). | Dropped frames; a long start. | none |
| S9.6 | Press Ctrl+C during a recording while a reply streams. | Only the dictation is cancelled. | The reply is cancelled. | `App` unit tests |
| S9.7 | On macOS, dictate from a terminal app whose `Info.plist` lacks `NSMicrophoneUsageDescription`. | No prompt ever appears; MSG-TCC-ZERO with its sentence for a terminal that never asks. | Silence with no message. | controller tests |
| S9.8 | Dictate with Ctrl+T in the desktop app's terminal dock. | It records; record which app the TCC prompt names. | n/a | none |

**Status, 2026-11-10.** Not passed on a signed build. PR-25 (final `488349de0`, `rust.yml` run 42422228488 green on nine jobs) and PR-26 (`79b20ba2f`, `rust.yml` run 42455226208 green on nine jobs) are committed. Report files dated 2026-11-07: synthetic capture drove the TUI and the classic REPL on Metal, and one physical microphone (a C920) recorded a first frame of 203 ms; the timings and their boundaries are in the measurement log. Open: S9.3 and S9.7 across terminal apps, S9.4 on Windows, S9.5 on four operating system families (M-7), S9.8 in the terminal dock.

## S10 CLI over SSH or in a container with no microphone

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S10.1 | `ssh <Linux host>`, then `biorouter dictate`, with and without a model installed there. | MSG-SSH-ROUTES (MSG-SSH-SERVICE when a `biorouter serve` runs there); exit 2; no device opened; no install prompt when no model is installed. | A recording of silence; an offer to download a model. | environment tests |
| S10.2 | `ssh <Mac>`, then `biorouter dictate`. | MSG-SSH-MAC, then the routes. | A TCC prompt on the remote screen. | environment tests |
| S10.3 | `docker run -it <image> dictate`. | MSG-CONTAINER. | ALSA noise. | package smoke |
| S10.4 | Follow the tunnel route, opening the address `serve` printed with its `?t=` part; then run `biorouter dictionary export - \| ssh <host> biorouter dictionary import -`. | The laptop's browser dictates with the host's model and dictionary; the terms appear on the host. | n/a | CLI import tests |
| S10.5 | Run `biorouter serve` in a VS Code Remote SSH terminal and in Cursor. | Record whether each forwards the port and opens a working address. | n/a | none |

**Status, 2026-11-10.** Not passed. The environment table tests of PR-25 hold the refusals offline. S10.3's image is committed with PR-31 (`8c865d11d`) and its native arm64 gate passed on 2026-11-10 (S16); no image is published, so S10.3 needs a locally built image. Open: every row over real SSH sessions to a Linux host and a Mac, in a real container, and in the two editors.

## S11 noninteractive CLI

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S11.1 | `biorouter transcribe memo.mp3 --format json`. | Schema 1 with `bias_scope: "public"`. | A count of entries in the output. | `dictation_cli.rs` |
| S11.2 | `biorouter run -t "$(biorouter dictate)"`. | The dictated text is shown for review before `run` starts. | `run` starts with unreviewed text. | review test |
| S11.3 | Ask the agent in a chat to run `biorouter dictate` in its shell. | MSG-AGENT-SHELL; exit 2. | A microphone opens. | `capture_consent_is_minted_in_one_file` |
| S11.4 | Run `biorouter dictate < /dev/null` from a script. | MSG-NO-TTY; exit 2. | A recording starts. | environment tests |
| S11.5 | `biorouter dictate \| biorouter crew send <channel> --input -`. | Reviewed on the terminal before anything is posted. | A post without review. | review test |
| S11.6 | `biorouter crew ask <channel> "$(biorouter dictate)"`. | Reviewed on the terminal before the task starts. | A task starts with unreviewed text. | review test |

**Status, 2026-11-10.** Not passed. `dictation_cli.rs`, the review tests and `capture_consent_is_minted_in_one_file` hold the logic (PR-12, PR-25). Open: every row on an installed CLI; S11.3 in a live chat.

## S12 ACP clients

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S12.1 | In Zed and in a JetBrains IDE (stdio), look at the agent chat. | No microphone and no dead control. | n/a | `on_initialize` test |
| S12.2 | Send an audio block from any client that can, once alone and once with text. | MSG-ACP-NOTE in the reply both times; the audio alone gets no model turn; with text, the turn runs on the text with the note in place of the clip. | The clip vanishes silently; a model turn for audio alone. | ACP audio tests in `crates/biorouter-acp/src/server.rs` |
| S12.3 | Use the tray's **Dictate to clipboard** (or `biorouter dictate --copy`) and paste into the editor. | The text pastes, with public terms unless the person chose **Include my private terms** (or `--private-terms`) and confirmed MSG-PRIVATE-TERMS. | Private terms without the confirmation. | `bias.rs` |
| S12.4 | Ask the agent to add a dictionary term. | A suggestion, and the answer names `biorouter dictionary review`. | An active row. | capability tests |
| S12.5 | Connect a `--ws` client. | No dictionary tools offered. | Tools offered. | ACP `--ws` test |
| S12.6 | In Zed, open the agent's command list and run `/dictate`. | `/dictate` is listed and answers MSG-ACP-DICTATE without calling the provider. | No command; a model turn. | ACP `/dictate` test |
| S12.7 | In JupyterLab, open Jupyter AI's chat with the Biorouter persona and its command list. | No `/dictate` and no dictation control (O-30). | `/dictate` listed. | none: the persona test in `integrations/jupyter-ai/tests/test_persona.py` (PR-30) runs in no workflow, so only this row checks it |

**Status, 2026-11-10.** Not passed. PR-30 is committed (`999ea27f9`): MSG-ACP-NOTE as a text part in place of an audio block, an audio only prompt answered locally with the note and no model turn, `/dictate` in the available commands after a new and a loaded stdio session, and the Jupyter AI persona setting `BIOROUTER_ACP_CLIENT=jupyter-ai` in its child's environment so the server leaves `/dictate` out. Its own `rust.yml` run failed on a census test fixed in `8558ccf61`, and the ACP library and server tests passed in the all green runs 42944486393 and 42949720898 ([CI on the branch](#ci-on-the-branch)). The Python persona tests were not run on the build machine (their dependencies are not installed there) and no workflow runs them. S12.4 and S12.5 rest on PR-23 (`50e79b308`); S12.3 runs through `biorouter dictate --copy` (PR-25) and the tray form (PR-28, `9a96e0132`). Open: S12.1 and S12.6 in Zed and a JetBrains IDE, S12.7 in JupyterLab, all by hand.

## S13 Jupyter AI persona chat

Withdrawn by the owner's decision O-30 (2026-11-05). Biorouter builds no microphone or dictation feature for Jupyter and adds no control to Jupyter AI's chat, so no microphone control is expected and this section holds no rows. PR-32 and M-13 are withdrawn with it. DD-94 counts this section as passed by that ruling. Row S12.7 checks the one thing a person could see: that Jupyter AI's chat lists no `/dictate`.

**Status, 2026-11-10.** Passed by the owner's ruling; nothing to run.

## S14 Crew

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S14.1 | In a channel composer, hover the microphone. | MSG-CREW-TIP. | No tooltip. | `check-crew-manual.mjs` |
| S14.2 | Dictate "@agent summarize the thread". | The text in the box; nothing posted and no agent started until Enter. | An agent run starts. | Crew composer tests |
| S14.3 | Dictate "EGLN1" in the box; then choose **Include my private terms** from the microphone's context menu, choose **Include** in MSG-PRIVATE-TERMS, and dictate it again. | The first time the private entry never shapes the spelling; the second time it may, for that recording only. | Private terms without the confirmation; the choice remembered. | `bias.rs`, Crew composer tests |
| S14.4 | Use the **Ask my agent** box. | As S14.2. | As S14.2. | Crew composer tests |
| S14.5 | Open Crew in a `serve` browser. | "Crew needs the Biorouter desktop app". | A dead microphone. | none |

**Status, 2026-11-10.** Not passed on a signed build. PR-27 is committed at `a4a6983f5`: frontend run 42477730958 green on its second attempt, the no AI coauthor check 42477662502 green, and `rust.yml` run 42477723459 green on all nine jobs. Synthetic, report file dated 2026-11-07: a local fallback on q8 Metal drove the channel composer and the **Ask my agent** box (timings in the measurement log). Open: every row with a physical microphone, the remote Crew manual step on a real workspace, and S14.5 in a `serve` browser.

## S15 Agent Drafter apps

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S15.1 | Open a built app's page in a `serve` browser and call `getUserMedia` from its console in Chrome, Firefox and Safari. | Chrome refuses; record Firefox and Safari, the documented residual (SD-17). | Chrome grants it. | `serve` job (Chromium) |
| S15.2 | In the desktop preview, click into an app's text field, then press **Dictate into this app**. Do it once with **Dictate into other apps** on and once with it off. | The text lands in the field both times (M-15, O-42). If M-15 failed, the button is absent. | Text in the wrong place; nothing happens while the switch is off. | `dictationAnywhere.test.ts` (the switch only) |
| S15.3 | With nothing in the app focused, look at the live page toolbar; then click into a text field. | **Dictate into this app** is absent while nothing is focused and appears once a field has focus (O-42). The SDK reference carries MSG-APPS (MSG-APPS-NO-PREVIEW if M-15 failed). | A disabled button with no reason; a button that does nothing. | `WebPagePreview.dictation.test.tsx` |
| S15.4 | Open an exported app in a browser; use **Dictate to clipboard** and paste. | The text pastes. | n/a | none |

**Status, 2026-11-10.** Not passed. PR-03's headers are committed (`7936f100f`), and `ui/desktop/tests/e2e/serve-app-microphone.spec.ts` checks Chromium's refusal against a real `biorouter serve`. PR-28 is committed in `9a96e0132` (`rust.yml` run 42975902058 green on all nine jobs; `frontend.yml` run 42975905256 failed two unit tests, one of them PR-28's `main.appLaunch.test.ts`, both fixed in `8955295b5`, whose `frontend.yml` run 42982772021 is green on all eight jobs, see [CI on the branch](#ci-on-the-branch)): **Dictate into this app** sits in the live page toolbar (`artifacts/WebPagePreview.tsx`), records in the hidden capture window, is shown only while a field in the app has focus, and starts whether or not **Dictate into other apps** is on, as the owner decided on 2026-11-10 (O-42). M-15 is open: if it fails, the preview button is removed and S15.2 and S15.3 check its absence. Open: every row in a packaged app, and M-15.

## S16 Docker image

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S16.1 | Run the documented `docker run -p 127.0.0.1:8765:8765 …` command, open `http://127.0.0.1:8765/?t=<token>` as MSG-SERVE-CONTAINER in the banner says, install from Settings and dictate. | The model lands in the data volume; dictation works (M-16). | An insecure context; the model inside the container layer. | Docker workflow |
| S16.2 | Recreate the container with the same volume. | The model is still installed. | A second download. | none |
| S16.3 | Mount a model folder read only at `/var/lib/biorouter/dictation/models`. | Found as `location: system`. | Not found. | lookup order tests |
| S16.4 | `docker run … <image> transcribe /data/memo.wav`. | The text. | n/a | Docker workflow |

**Status, 2026-11-10.** Not passed. PR-31 is committed (`8c865d11d`: the `Dockerfile`, `BUILDING_DOCKER.md`, `.dockerignore`, the nightly and dispatch `docker-image.yml`, and its two check scripts). Its native Docker gate passed on 2026-11-10 on the maintainer's Apple Silicon Mac (run root `sandboxes/pr31-full-r6-20261110b` in the maintainer's run folder): a native full build, a `linux/arm64` image, both container self tests, the status with no model, a model install from the pinned cache, a JFK transcription, the `serve` shell and banner, a secure context, and a dictation in Chrome with a fake microphone on `whisper-small.en-q8_0` on the container's CPU (timings in the measurement log). That is synthetic audio on arm64. The amd64 image (M-16 amd64) waits until `docker-image.yml` reaches `main`, because a workflow cannot be dispatched before it exists on the default branch (the dispatch answered HTTP 404). No image is published. Open: S16.1 to S16.4 by hand, the amd64 image, and M-16.

## S17 desktop app against an external backend

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S17.1 | In Settings > App > **Use external server**, point at a LAN `biorouterd` behind nginx over HTTPS with its **User Action Key**; dictate in a private chat. | MSG-DISCLOSE-REMOTE; dictation works with private terms. Record latency and upload timings (M-20). | "On this computer" wording. | external backend tests |
| S17.2 | Remove the key. | MSG-REMOTE-NO-KEY; dictation goes on with public terms only (O-17); install disabled before the click. | Install fails after the click. | external backend tests |
| S17.3 | Point at a plain `http:` backend. | MSG-REMOTE-HTTP once per URL; **Continue** proceeds. | A refusal; a warning every time. | external backend tests |
| S17.4 | Stage a laptop managed policy with `dictation.capture: false`. | The microphone stays in every composer, disabled, with MSG-POLICY-CAPTURE as its name and tooltip, and a press starts nothing; the **Dictate into other apps** switch is disabled; Settings shows MSG-POLICY-CAPTURE; a page that calls `getUserMedia` gets `NotAllowedError`. | The backend's policy decides instead; the microphone is gone, or looks ready and fails only on the press. | policy path test |
| S17.5 | Point at a `biorouterd` older than local dictation. | MSG-REMOTE-OLD on the microphone and the **Dictionary** page; their controls disabled. | A dead microphone; a raw 404. | external backend tests |

**Status, 2026-11-10.** Not passed. PR-29 is committed with PR-28 in `9a96e0132` (`rust.yml` run 42975902058 green on all nine jobs; `frontend.yml` run 42975905256 failed two unit tests, one of them PR-29's `tabSwitchFrames.browser.test.tsx`, which found the **Server** section's switch it did not expect; both are fixed in `8955295b5`, whose `frontend.yml` run 42982772021 is green on all eight jobs, see [CI on the branch](#ci-on-the-branch)). The laptop's policy reaches main's permission handler and the **Dictate into other apps** row (`anywhereSnapshot` in `main.ts`); an earlier review of the draft found that the composer microphone still showed under a laptop policy. On 2026-11-12 the owner ruled that a policy that forbids live capture leaves the microphone in place, disabled, with the policy's sentence as its reason, rather than removing it. The composer did that first for the daemon's `status.policy`, which on S17 is the backend host's policy. Since 2026-11-13 the status store also reads this computer's own policy from main, through the same `dictationAnywhereGet` the Settings row uses, with every status read and so again when the window regains focus, and the microphone is `unavailable` under it with MSG-POLICY or MSG-POLICY-CAPTURE as its name and tooltip (`dictationStore.test.ts`, `DictationButton.test.tsx`). The gap is closed in the source; the row has not been run by hand. Open: every row, and M-20 behind nginx on a LAN.

## S18 other applications (Dictate to clipboard)

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S18.1 | Turn on **Dictate into other apps**. | MSG-CLIPBOARD-EXPLAIN once; on macOS the microphone prompt. | No explanation. | `dictationAnywhere.test.ts` |
| S18.2 | From the tray choose **Dictate to clipboard**, speak, choose **Stop and copy**, paste into TextEdit, Word or Notepad. | MSG-CLIPBOARD-COPIED; the text pastes; the notification shows counts only. | Words in the notification. | `dictationAnywhere.test.ts` |
| S18.3 | Record silence. | MSG-CLIPBOARD-NOTHING; the clipboard is unchanged. | The clipboard is overwritten. | `dictationAnywhere.test.ts` |
| S18.4 | Choose a shortcut another app holds. | MSG-SHORTCUT-TAKEN. | A shortcut that never fires. | `dictationAnywhere.test.ts` |
| S18.5 | On native Windows with the cloud clipboard on, run `biorouter dictate --copy`. | The text does not reach another device. | It syncs. | clipboard helper tests |
| S18.6 | With the packaged deb on Ubuntu 26.04 GNOME, Fedora and KDE Plasma 6 (X11 and Wayland), and on a Windows 11 laptop. | Record hidden window capture with a real microphone, clipboard focus and the KDE portal shortcut (M-12). | n/a | none |
| S18.7 | `biorouter dictate --copy` over SSH with no display; then on a Linux desktop. | MSG-NO-CLIPBOARD and exit 5; MSG-CLIPBOARD-WAIT and a paste within 60 s works. | The text is lost when the CLI exits. | clipboard helper tests |
| S18.8 | Turn off **Show in menu bar** in Settings > App, then open **Dictate into other apps**. | MSG-NO-TRAY; a shortcut is required. | A switch that starts nothing. | `dictationAnywhere.test.ts` |
| S18.9 | On native Windows with the cloud clipboard on, use the tray's **Dictate to clipboard**. | MSG-CLIPBOARD-EXPLAIN was shown when the switch was turned on, and syncing to another device is expected: the desktop form cannot set the exclusion in Electron 39. | The switch turned on without MSG-CLIPBOARD-EXPLAIN. | `dictationAnywhere.test.ts` |
| S18.10 | Make a capture fail after it started, for example by unplugging the only microphone during a recording. | The review card says MSG-CLIPBOARD-FAILED; the clipboard is unchanged. | A card with no sentence; the clipboard overwritten. | none |
| S18.11 | From the tray or the shortcut, make the transcription fail after **Stop and copy** with a 500 `engineFailed` (for example, kill the `__br-dictation-worker engine` process while it transcribes on the CPU), then choose **Retry** on the card. Repeat, and wait 5 minutes instead. | The card says MSG-CLIPBOARD-FAILED with **Retry**; **Retry** hides the card and copies the text with MSG-CLIPBOARD-COPIED. After 5 minutes the card keeps the sentence without **Retry**. | No **Retry**; a **Retry** that does nothing; **Retry** still offered after 5 minutes. | `DictationCapturePage.test.tsx`, `dictationAnywhereMain.test.ts` |
| S18.12 | Where the clipboard does not keep the text (a Wayland clipboard that needs focus), choose **Copy** on the review card. | The text is copied, or the card says MSG-CLIPBOARD-COPY-FAILED. | **Copy** looks done and the clipboard is unchanged. | `DictationReviewCard.test.tsx` |

**Status, 2026-11-10.** Not passed. `biorouter dictate --copy` and the clipboard helper are committed with PR-25, so S18.5 and S18.7 can run on an installed CLI. The desktop rows are committed with PR-28 in `9a96e0132` (`rust.yml` run 42975902058 green on all nine jobs; `frontend.yml` run 42975905256 failed two unit tests, both fixed in `8955295b5`, whose `frontend.yml` run 42982772021 is green on all eight jobs, see [CI on the branch](#ci-on-the-branch)) and have not been seen in a packaged app. A capture that fails with no mapped sentence shows the hard coded MSG-CLIPBOARD-FAILED (`showAnywhereReview` in `main.ts`). Since 2026-11-13 (owner decision), a tray or shortcut dictation whose transcription failed in a way the stream can send again keeps its recording for 5 minutes in the hidden capture window, and the card offers **Retry** while it is held (S18.11); the card's **Copy** reports a copy the clipboard did not keep with MSG-CLIPBOARD-COPY-FAILED (S18.12). Both are checked by unit tests only. Open: every row, and M-12 on Windows 11, PipeWire, GNOME and KDE with a physical microphone.

## S19 WSL

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S19.1 | In WSL2 with WSLg and `libpulse0`, run `biorouter dictate`. | It records and transcribes. | Silence. | environment tests |
| S19.2 | Remove `libpulse0`; run it again. | MSG-WSL2-NOPULSE. | A generic error. | environment tests |
| S19.3 | In WSL1. | MSG-WSL1. | A hang. | environment tests |
| S19.4 | `biorouter dictation install --from /mnt/c/Users/<you>/Downloads/`. | It installs by hash. | A second download. | `--from` tests |
| S19.5 | `biorouter dictate --copy`, then paste in a Windows app. | The text pastes through `Set-Clipboard`. | n/a | clipboard helper tests |
| S19.6 | Run `biorouter serve` in WSL and open the address it prints, with its `?t=` part, in a Windows browser (`localhost` in place of `127.0.0.1` works too). | Dictation works as in S5. | n/a | none |
| S19.7 | In WSL2 with `guiApplications=false` in `.wslconfig`, run `biorouter dictate`. | MSG-WSL2-NOWSLG; exit 2. | A generic error. | environment tests |

**Status, 2026-11-10.** Not passed. The environment table tests of PR-25 hold the WSL messages. Open: every row in real WSL2 with WSLg, WSL2 without it, and WSL1.

## S20 air gapped or offline hosts

| # | Do this | Expect | Failure looks like | CI covers |
|---|---|---|---|---|
| S20.1 | With the network off, run `biorouter dictation install`. | MSG-OFFLINE. | A hang. | installer tests |
| S20.2 | On a connected machine run `biorouter dictation urls`, download the files, carry them over and run `biorouter dictation install --from <dir>`. | Installed by hash, renamed files included; a wrong file gives MSG-HASH. | A renamed file refused. | `--from` tests |
| S20.3 | On the desktop, copy the files into the model folder (**Show in Finder**) and press **Rescan**. | The model is listed as installed. | n/a | rescan route test |
| S20.4 | Stage managed `model_downloads: false` with a `models_dir`. | Only that folder is used; nothing is offered for download. | A download starts. | `dictation_policy_routes.rs` |
| S20.5 | Dictate with the network off while watching outgoing connections. | It works and nothing connects out. | Any connection attempt. | none |

**Status, 2026-11-10.** Not passed. The installer, `--from`, rescan and policy route tests hold the logic, and `the_offline_sentence_is_the_catalogs` pins MSG-OFFLINE. Open: every row on an offline machine, and S20.5 with an outgoing connection monitor.

## Measurement log

One row per measurement, with the date, the build, the machine, the result and who took it (O-29). Announcing version 1 needs M-1 to M-8, M-10, M-12, M-15 to M-20 and M-22. M-11, M-14 and M-21 feed later decisions; M-9 and M-13 are withdrawn by the owner's decisions O-23 and O-30 (2026-11-05). What each measurement is and how to take it is in [testing and release](../dictation/testing-and-release.md#measurements). Nothing here is an accuracy claim about the product (DD-95).

| Id | Measurement | Blocks | Recorded | Build and machine | Result | Taken by |
|---|---|---|---|---|---|---|
| M-1 | Native Windows run of the static mingw worker | announcing v1 | 2026-11-06 | `rust.yml` dispatched run 42280907734 at `78d5ca159` (PR-09 to PR-13); hosted Windows runner | Green on every job: the native self test, `status --verbose`, and the `whisper-small.en-q8_0` install and JFK `transcribe` under `RUNNER_TEMP\dictation-José-m1`. PR-09's own acceptance run, 42185126727, was green after one rerun of an unrelated flake. `windows-native-dictation` was green again in 42949720898 at `491246227` on 2026-11-10. Refresh on the release's own commit and package | CI |
| M-2 | Real x86 Linux (not Rosetta or qemu) | announcing v1 | not recorded | n/a | Older hosted cross build evidence exists; the self test and JFK run on a hosted amd64 runner and the real model run on a lab server are open | n/a |
| M-3 | The engine under MSVC (`/MD`) | PR-09 merge | 2026-11-05 (decision), 2026-11-06 (runs) | The Windows `test` leg; the runs named under M-1 | Decided in PR-09: the leg builds the engine (`MSVC_NATIVE = true` in `crates/biorouter-dictation/build.rs`), and the runs under M-1 were green with it. Refresh on the release's own commit | CI |
| M-4 | A signed, notarized, engine linked app on macOS 12 and 13; Metal under the hardened runtime; a first transcription | the first app release that carries PR-09 | not recorded | n/a | Open. During the design, Metal transcribed under the hardened runtime on ad hoc signed copies with `osx-sign`'s default entitlements, with none and with an empty set, cold shader compile included; the notarized app on macOS 12 and 13 is the open part (row S1.9) | n/a |
| M-5 | Connection pool: two windows and two `serve` tabs while a turn streams; then behind HTTP/2 nginx | announcing v1 | not recorded | n/a | Open (row S6.5 and a desktop twin) | n/a |
| M-6 | The TCC denial's error name and message on a signed app | announcing v1 | not recorded | n/a | Open (row S1.5) | n/a |
| M-7 | Real microphones per OS family, app and CLI; PipeWire, PulseAudio and ALSA throughput; CLI time to first frame under 300 ms | announcing v1 | partial, report file dated 2026-11-07 | PR-25 gate, M4 Metal debug build | One physical microphone (C920): first frame 203 ms. Synthetic first frames 17 to 36 ms are a different boundary. Open: every OS family, the app, and the throughput rows (S4.1, S9.5) | the PR-25 gate run |
| M-8 | Safari at `127.0.0.1` and behind HTTPS, first click and first ⌘⇧D before the status store loads; iOS Safari; MSG-SHARED-ORIGIN once per origin; the Firefox ESR fallback before 148 | announcing v1 | not recorded | n/a | Open (rows S5.3, S6.1, S6.6) | n/a |
| M-9 | Withdrawn by O-23 (2026-11-05): the lab recording set | nothing | n/a | n/a | Withdrawn. Biorouter records no real voices for evaluation, and no accuracy figure is published (DD-95) | n/a |
| M-10 | Native x86 latency and real speech accuracy of the CPU models, 4 and 8 threads, on an Intel Mac, a Windows laptop and a Linux server | per hardware defaults; announcing v1 | not recorded | n/a | Open (row S2.2) | n/a |
| M-11 | Lab host ISA census | O-11 | not recorded | n/a | Not taken; feeds O-11 (a helper build for hosts without AVX2) | n/a |
| M-12 | Hidden window capture with a real microphone on Windows 11 and PipeWire; Wayland clipboard focus; the KDE portal shortcut | announcing **Dictate to clipboard** on Linux | not recorded | n/a | Open (row S18.6); PR-28 is committed in `9a96e0132` and has not run on those systems; that commit's `rust.yml` run 42975902058 is green on all nine jobs, and its `frontend.yml` run 42975905256 failed two unit tests, both fixed in `8955295b5` (tests only), whose `frontend.yml` run 42982772021 is green on all eight jobs | n/a |
| M-13 | Withdrawn by O-30 (2026-11-05): audio decoding for a Jupyter AI microphone | nothing | n/a | n/a | Withdrawn with S13 | n/a |
| M-14 | `available_parallelism()` under a cgroup v2 quota; worker memory; four concurrent streams | several decoder states in a later release | not recorded | n/a | Not taken; feeds a later release | n/a |
| M-15 | `webContents.insertText` into the focused field of an app preview after the main renderer took focus | shipping **Dictate into this app** | not recorded | n/a | Open (row S15.2); PR-28 is committed in `9a96e0132`, whose `rust.yml` run 42975902058 is green on all nine jobs and whose `frontend.yml` run 42975905256 failed two unit tests, both fixed in `8955295b5` (tests only), whose `frontend.yml` run 42982772021 is green on all eight jobs | n/a |
| M-16 | Docker `serve` end to end | the Docker documentation | partial, 2026-11-10 | PR-31 at `8c865d11d`, native `linux/arm64` image on the maintainer's Apple Silicon Mac | The native gate passed end to end with a fake microphone in Chrome on the container's CPU (S16 status; timings below). The amd64 image is a separate proof and is open until `docker-image.yml` is on `main`; no image is published | the PR-31 gate run |
| M-17 | Proxies other than nginx, and each one's setting that keeps request bodies off disk | the proxy section of browser access | not recorded | n/a | Open (row S6.3) | n/a |
| M-18 | Whether browsers let a page prevent ⌘⇧D; tap durations on the chord | the tooltip on `serve`; the 500 ms threshold | not recorded | n/a | Open (row S5.6) | n/a |
| M-19 | Long dictation: 60 to 180 s of continuous real speech through the streaming path; boundary word loss and Stop to text time on Metal and an x86 CPU | announcing v1 | partial | PR-15 | PR-15 measured synthetic clips end to end only (60 s, 10 minutes and a live 120 s; the forced 25 s cuts left the text length unchanged). Real conversation is open, and `long_dictation_replays_in_real_time` is not built | PR-15's run |
| M-20 | External backend: latency and uploads to a LAN `biorouterd` behind nginx | announcing S17 | not recorded | n/a | Open (row S17.1); PR-29 is committed in `9a96e0132`, and no LAN nginx run has been made; that commit's `rust.yml` run 42975902058 is green on all nine jobs, and its `frontend.yml` run 42975905256 failed two unit tests, both fixed in `8955295b5` (tests only), whose `frontend.yml` run 42982772021 is green on all eight jobs | n/a |
| M-21 | Extractor precision over the person written messages of 200 of the owner's sessions, offline and local | the default of `learn_from_chats` | not run, by decision | n/a | PR-24 (`491246227`) ships `learn_from_chats` off, with the reason recorded at `LEARN_FROM_CHATS_DEFAULT` in `crates/biorouter-dictation/src/store/schema.rs`: the precision run waits for the owner's own labels on candidate tokens from eligible chats (O-44, 2026-11-10). Turning it on later needs eligible sessions of the owner's only, private model and patient data sessions excluded, the owner's labels on candidate tokens only (never whole messages), aggregate reports with no remote data, and precision of 0.8 or more at `seen ≥ 2` | n/a |
| M-22 | The DD-44 channel decision on public real speech and synthetic speech | the DD-44 default; announcing v1 | 2026-11-06 | PR-14's code on top of `78d5ca159`, `m22_arms` in `crates/biorouter/tests/dictation_accuracy.rs` (a debug test binary); Metal on one Apple M4 Max shared with other work | The declared fallback ships: the token prompt alone. `temperature_inc` 0.2 and timestamps on stay. Tables in [speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md#the-m-22-channel-decision-2026-11-06). Recheck its release gate before announcing | the maintainer's run |

### Latency measured while the pull requests were built

These are accepted measurements from the build of PR-21 to PR-31, copied with their boundaries as the handoff and progress records of 2026-11-10 give them. Values are milliseconds unless marked. Load is the reported host load, not a measurement of transcription speed. Synthetic and physical microphone rows are separate, and none is an accuracy result. Do not compare one boundary with another: a worklet chunk is not an HTTP upload, a first PUT is not first audio, and a whole agent turn is neither. The report files named in the last column are in the maintainer's run folder, not in the repository.

The first column is the modification date of the report file the row comes from, which is when the report was written and not necessarily when the run was made. The rows dated 2026-11-10 come from the run folder's progress record of that day and carry the date of the run.

| Report file date | PR, backend and model | Load | Measured boundary and value | Evidence and limit |
| --- | --- | --- | --- | --- |
| 2026-11-07 | PR25 M4 Metal debug, small.en | 6.37, 12.79, 17.33 across runs | Stop to text 106 to 116; synthetic first frame 17 to 36; physical C920 first frame 203; `afplay` duration 8.781 seconds | `codex-progress/pr25-gate/pr25-gate-final.json` and `codex-progress/pr25-gate/remaining-live/results.json`. First frame is capture timing; `afplay` duration is stimulus length, not latency. |
| 2026-11-07 | PR25 M4 Metal debug, turbo | Same reported PR25 load set | Stop to text 282 to 300 | Same PR25 gate reports. Do not pool with small.en. |
| 2026-11-07 | PR22 q8 Metal, correction learning | 5.55 | Worklet first chunk 26, per chunk 30.7, stop pointerdown to visible text 120.3 | `codex-progress/pr22-live-final-luna.json`. Learned after two corrections. Worklet chunk is not HTTP upload or first microphone PCM. |
| 2026-11-07 | PR22 Send bubble, same live run | 5.55 | Send pointerdown to user bubble baseline 98.1, corrected 103.4 | Same report. Each is one observation; do not infer a distribution or ASR cost. |
| 2026-11-07 | PR21 q8 Metal, loopback nginx | 6.25 | First worklet message 27.9; first HTTP PUT 2019.3; stop to text 166 | `codex-progress/pr21-gate-final-luna.json`. The PUT includes a different boundary and initial upload path. |
| 2026-11-07 | PR21 q8 Metal, HTTPS prefix nginx | 4.97 and 6.25 in the PR21 run | First worklet message 27.8; first HTTP PUT 2018.8; stop to text 182 | Same report. Do not compare the PUT number as if it were first PCM or worklet latency. |
| 2026-11-07 | PR26 classic REPL, q8 Metal | 5.83/3.87/3.50 before, 4.76/3.75/3.47 after | Dispatch to first audio 1507; observer file 1610.3; stop to input 157.6 | `codex-progress/pr26-gate-final-luna.json` and `codex-progress/pr26-live-classic-q8-luna.json`. Observer file time is not the actual first audio receipt. |
| 2026-11-07 | PR26 TUI, q8 Metal | 3.34/3.71/3.56 before, 3.45/3.73/3.58 after | Dispatch 31; actual PCM receipt 34.1; stop to input 150.5 | `codex-progress/pr26-gate-final-luna.json`. Terminal boundary differs from browser worklet timing. |
| 2026-11-07 | PR27 Crew local fallback, q8 Metal | 5.26 to 9.58 | Channel press to first PCM 90.1, stop to text 139.9; task press to first PCM 23.6, stop to text 161.7 | `codex-progress/pr27-implementation-live-report.json`. Synthetic local fallback passed; remote Crew manual and physical mic remain open. |
| 2026-11-08 | PR23 agent turn, versa_azure GPT-5.5 | Not an ASR load comparison | Whole User turn 5682.2 and 4744.5 | `codex-progress/pr23-gui-positive-and-routing-luna.json`. These span the model turn and tool path. They are neither ASR latency nor `dictionary__add_terms` tool time. |
| 2026-11-10 | PR23 private chat stream, small.en q8 Metal, debug build | Not recorded | `EPAS1` spelled in 2 of 2 dictations into the targeted private chat and in 0 of 2 into a public chat; stop to text 111 to 138 | The PR-23 live check. Counts whether a private term shaped dictation only where its tier allows; a count of spellings, not an accuracy figure. |
| 2026-11-10 | PR23 store of 5,000 public rows, r15 debug binaries | Not recorded | `dictionary__list_terms` tool frame gap 110.9; `dictionary__add_terms` tool frame gap 151.2; whole turn 17,284.5; import 554.7; request to response headers 3.1 | Sandbox `r15perfb`. Measured on r15, before the `on_request` fix the same run found (`person_named_term_in_messages`, committed in `50e79b308`), so these timings are of the earlier source. A tool frame gap is the time between the tool's request and result frames on the stream; the turn is a whole agent turn, not tool time. |
| 2026-11-10 | PR31 native Docker gate, `linux/arm64` container, `whisper-small.en-q8_0` on CPU | 18 | Press to first frame 33; stop to text 877 | Run root `sandboxes/pr31-full-r6-20261110b`, receipt SHA-256 `e9456d90…`. Chrome with a fake microphone against `serve` in the container; the probe times from the button's pointerdown, because the composer starts there. CPU inside the container, not Metal, so do not compare it with the Metal rows. Not amd64. |

The dev app measurements of Settings and of the Dictionary page on 2026-11-07 are in the S1 status line and in [surfaces](../dictation/surfaces.md#settings).

### Recording a new measurement

Add the date, the build (version, commit, signed or not), the machine, the result and who took it to the measurement's row. Name the boundary the number measures, keep a physical microphone apart from synthetic audio, and give one observation as one observation rather than a range. Move the row's state from "not recorded" or "partial" only when the measurement's own definition in [testing and release](../dictation/testing-and-release.md#measurements) is met.

## Release notes draft

The text below is the draft of the user facing notes for the first release that carries local dictation. Release notes under `docs/releases/notes/` are frozen records of releases as they shipped, so the draft stays here until that release is cut. At release:

1. Copy the text into `docs/releases/notes/v<version>.md`, give it the heading `# Biorouter v<version>`, raise every `###` heading below to `##`, and add its row to the [release notes index](notes/README.md).
2. Replace every `vVERSION` in the links below with the release's tag, and put the version in the note at the top of the website's Dictation section (`landing/docs.html`, `#app-dictation`).
3. Keep the **Dictate into this app** sentence only if M-15 passed; otherwise delete it, and the **Built apps** item of the website's Dictation section (`docs/website/pages/app.html`), because the preview button is removed.
4. Fill in the measured growth of `biorouter` and `biorouterd` per platform from the release's own build (`dictation_size_ceiling` in `scripts/smoke-test-release-artifacts.sh`).
5. Before release, name the supported way to remove one stored secret, checked in the build. Do not suggest deleting the `biorouter` Keychain or Credential Manager entry, which holds every key.
6. Never add an accuracy figure (DD-95). The announcement of version 1 of dictation is separate and waits for [What has to pass](#what-has-to-pass).

This release adds local dictation: speech to text that runs on the computer running Biorouter, with a personal dictionary that spells your terms your way. You can dictate in the desktop app's composers, in the terminal and in `biorouter serve` browsers. No speech model comes with the app or with an update; you install one when you want dictation. The release also removes the cloud dictation route Biorouter inherited, which sent audio to OpenAI or ElevenLabs.

### Dictate in the app, the terminal and the browser

- **Desktop composer.** A microphone sits immediately left of **Send** in every composer: chats, Home, split panes, new tabs and the message editor. Click it, or press ⌘⇧D (Ctrl+Shift+D on Windows and Linux): a tap starts and stops a recording, a hold of half a second or more records while you hold. Enter stops and inserts, Escape discards. The text lands at the caret as one undo step, and nothing is ever sent until you press **Send**.
- **Crew.** The channel composer and the **Ask my agent** box have the same microphone. The text goes into the box for you to check; nothing is posted and no agent starts until you press Send.
- **Terminal.** In the interactive session, the classic prompt, the shared daemon prompt and `biorouter session attach`, press Ctrl+T or type `/dictate`; `attach` asks `Send? [y/N]` before anything is sent. `biorouter dictate` records from your terminal's microphone and shows the text for review before it goes to standard output, so `biorouter run -t "$(biorouter dictate)"` sends nothing you have not seen. `biorouter dictate --copy` puts it on the clipboard instead.
- **Recordings.** `biorouter transcribe <file>` turns a recording into text on this computer. It reads WAV, AIFF, FLAC, MP3, M4A and MP4 audio (AAC or Apple Lossless), raw AAC and Ogg (Vorbis or FLAC), so a phone's or a Mac's voice memo needs no conversion, a Lossless one included. Opus and WebM are refused with an `ffmpeg` line that converts them first.
- **`biorouter serve`.** A browser at the address `serve` prints dictates with the host's model and dictionary. The first recording says that audio is transcribed on the server. `biorouter serve --no-dictation-install` stops browsers from installing models.
- **Other applications.** Turn on **Dictate into other apps** in **Settings > Models > Speech models**, and **Dictate to clipboard** in the tray menu, or a global shortcut you choose there, records and copies the text for you to paste into any app. The tray item is there only while **Settings > App > General > Show in menu bar** is on; the shortcut works without it. On Windows and Linux it works while a Biorouter window is open, minimized included. In the desktop preview of a built app, **Dictate into this app** types into the field you clicked; it is shown while a field in the app has focus, and works whether or not **Dictate into other apps** is on.
- **External backends.** Pointed at another computer's Biorouter service, the app dictates with that computer's model and dictionary and says so first. Without that server's **User Action Key**, dictation there uses only your public terms.
- **Editors over ACP.** Zed and other ACP clients offer no microphone; `/dictate` in the editor's command list explains how to dictate with **Dictate to clipboard** or `biorouter dictate --copy`. An audio clip an editor attaches is left out of the turn with a note that says so.
- **Docker.** The image can run `biorouter serve` for a browser on the same computer, and keeps models in its data volume, never in the image.

Jupyter AI gets no dictation: Biorouter adds no microphone control there.

### Install a speech model when you want one

Nothing downloads until you ask. The first click on the microphone opens a short setup with **Install** and **Not now**. You can also install from **Settings > Models > Speech models**, or from a terminal:

```bash
biorouter dictation install
```

- With no model named, Biorouter installs the one suited to the computer: `whisper-large-v3-turbo-q5_0` (574 MB) on Apple Silicon, `whisper-small.en-q8_0` (264 MB) elsewhere, each with the `silero-vad-v6.2.0` voice detector (885 KB).
- Models come from the Baranzini Lab's own repository, `BaranziniLab/biorouter-models` on GitHub, and every file is checked against a SHA-256 pin built into Biorouter before anything reads it. A download that stops resumes from where it stopped.
- They install into your own data folder: `~/.local/share/biorouter/dictation/models/` on macOS and Linux, `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\` on Windows. An update never touches them. **Settings** and `biorouter dictation status` show the folder and the account.
- On a server, install as the account the service runs as (`sudo -u biorouter -H biorouter dictation install`), or into the shared folder every account reads with `sudo biorouter dictation install --system`.
- Without a network, `biorouter dictation urls` on a connected computer lists the files to download; carry them over and run `biorouter dictation install --from <folder>`, or put them in the model folder and press **Rescan** in Settings.
- **Remove** in Settings, or `biorouter dictation remove <model>`, deletes a model; the voice detector goes with the last one.

### Your dictionary

- Add the words dictation should spell your way, such as UCSF or EPAS1, on the new **Dictionary** page in the sidebar's Components group, with **Add to Biorouter dictionary…** in the right click menu of a chat, or with `biorouter dictionary add`. Import and export move terms between computers.
- When you correct a dictated word the same way twice, Biorouter learns it for a term you already have.
- **Suggest dictionary words from my chats**, beside **Install** in the setup, turns on the agent's `dictionary` capability for new chats. A term the agent adds goes live at once only when your latest message asked for it, or answered the agent's message that offered it; every other term waits under **Suggested** for you. Biorouter checks where the term appears, not whether your answer agreed, so look over the **Added by the agent** group, where **Keep** and **Undo** sit on each row.
- **Also find terms in my messages automatically**, at the foot of the **Dictionary** page, starts off. Turned on, it looks for terms in your messages, on this computer, and suggests them for you to accept; it starts off because how well it picks terms is not checked yet.

### Privacy

Dictation follows Biorouter's private and public rules. Like those rules everywhere in Biorouter, they keep cooperative people and agents from mistakes; they are not a defence against a determined attacker.

- **Audio.** Audio stays in memory in Biorouter's processes on the computer that transcribes, and is never written to a file, a log or a chat. On `serve`, Docker and an external backend, that computer is the server, and Biorouter says so before the first recording. A reverse proxy in front of `serve` is yours to configure; [browser access](https://github.com/BaranziniLab/biorouter/blob/vVERSION/docs/deployment/browser-access.md) names the setting that keeps uploads off its disk.
- **Text.** The transcript goes into the composer and nowhere else. Notifications show counts, never words.
- **Private terms.** A term you add from a private chat is private unless you untick **Only in private chats**, and a private term shapes dictation only into chats on private models. It also carries the institutions of the chat it came from, so it never shapes dictation aimed at another institution's model. Crew, the clipboard and the app preview use only public terms unless you choose **Include my private terms** for one recording after a warning; editors over ACP use public terms only.
- **Sending.** If a draft you dictated with private terms is about to go to a public model, Biorouter asks first.
- **What a shell can still do.** An agent with a shell can read the dictionary file. An agent you granted Biorouter Copilot on Biorouter's own window can press the microphone, as it can press any other control. On a `serve` host with no user action key, anyone who holds the daemon secret can install a pinned model and edit the dictionary rows a browser may edit.
- **Your organization.** A managed `dictation:` policy can turn dictation off, turn off live microphones only, stop downloads, limit the models, or turn off the dictionary. A key Biorouter does not recognize turns dictation off rather than on.

### The inherited cloud dictation is removed

- `POST /audio/transcribe` (OpenAI `whisper-1`), `POST /audio/transcribe/elevenlabs` and `GET /audio/config` are gone. They forwarded audio to a cloud service on the daemon secret alone, with no privacy tier and no off switch, and no screen in the app used them.
- A saved `ELEVENLABS_API_KEY` stays in Biorouter's credential store, untouched and unused; Biorouter does not delete credentials on your behalf. `OPENAI_API_KEY` is the OpenAI provider's own key and works as before.
- The old dictation settings saved in the app's local storage are cleared once.

### Requirements and compatibility

- `biorouter` and `biorouterd` now declare macOS 12 as their minimum, which the app already required.
- Local dictation needs a processor with AVX2, FMA, F16C and BMI2 on x86, which processors before Haswell and some virtual machines lack. Biorouter checks before any download and says what is missing instead of crashing; your system's own dictation still works there, without your Biorouter dictionary.
- The Intel Mac build runs dictation under Rosetta on macOS 15 and later; on macOS 12 to 14, install the Apple Silicon build.
- Each of `biorouter` and `biorouterd` is larger by the engine built into it (fill in the measured growth, step 4 above). Releases still carry exactly 11 assets, and no speech model is one of them.

### Licenses

`NOTICE` gains four entries: **whisper.cpp** (MIT, the ggml authors), the speech engine built into `biorouter` and `biorouterd`, vendored at a pinned commit; **miniaudio** (MIT No Attribution), the terminal's microphone capture, which arrives inside the vendored whisper.cpp tree; the **English Speller Database** word list, which keeps dictionary correction off ordinary words; and **Speech models downloaded on request**: the Whisper weights (MIT, OpenAI), their ggml conversions (MIT, the ggml authors) and the **Silero** VAD v6.2.0 voice detector (MIT, Silero Team). Each model's license texts are written beside its installed files.

### What waits for a later release

- Transcribing audio that an ACP editor sends; none of the editors checked sends any.
- A microphone inside built app pages.
- Hold to talk in the terminal; Ctrl+T toggles.
- A global shortcut on GNOME under Wayland; the tray item and the composer microphone work there.
- Push to talk and automatic paste for **Dictate to clipboard**.
- The **Shortcut**, **Show the microphone button** and **Reduce background noise** rows of **Settings > Models > Speech models**; the chord works without them, the button shows and noise reduction stays off.

[Local dictation](https://github.com/BaranziniLab/biorouter/blob/vVERSION/docs/dictation/README.md) · [Dictation surfaces and messages](https://github.com/BaranziniLab/biorouter/blob/vVERSION/docs/dictation/surfaces.md) · [Dictation commands](https://github.com/BaranziniLab/biorouter/blob/vVERSION/docs/cli/command-reference.md#dictation)

## Related documentation

- [Testing and release](../dictation/testing-and-release.md): the tests per component, the CI jobs, the definition of every measurement and the announcement rule this checklist serves.
- [Surfaces](../dictation/surfaces.md): the behaviour each row expects and the exact text of every MSG id.
- [Implementation plan](../dictation/implementation-plan.md): the surfaces, the pull requests and the owner's decisions the rows follow.
- [Dictionary](../dictation/dictionary.md): the privacy invariants P-1 to P-16 behind the private term rows.
- [Privacy and workspace test checklist](privacy-and-workspace-test-checklist.md): the checklist whose shape this one follows, for the privacy tiers dictation's rows rely on.
- [CLI QA checklist](../cli/qa-checklist.md): the terminal checklist that points here for dictation.
- [Speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md): where M-22's tables and later Tier 2 tables go.
- [Release notes](notes/README.md): where the draft above goes when the release is cut.
- [Releases](README.md): the folder this checklist belongs to.
