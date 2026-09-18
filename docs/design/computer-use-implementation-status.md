# Computer Use implementation status

Updated 2026-09-18. This branch integrates main through `cfb881e0` and includes native patch `7fd33dda`; package acceptance and live-test follow-up work remains in progress. This ledger tracks the full user-requested result. Source changes, passing unit tests, packaging, actual GUI behavior, and a pushed pull request are separate gates. An unchecked row remains required; unavailable platform evidence is not a pass.

| Requirement | Status and evidence to collect |
| --- | --- |
| Clean replacement: exactly ten native tools; no old scripting/control, Developer capture, hidden replay routes, or aliases | Implemented and audited in `computercontroller/`, Developer router, builtin registry and active fixtures; four tool-census regressions passed. No callable compatibility routes remain. |
| Native `screen_capture` in the same helper as the nine upstream tools | Pushed patch `ec1fc9d2dec0378f96ae3a2aee3373b5c83c06f1523ad54d4a32ec1fc67109be` adds exact-window/focus ownership, typed AX values and Windows real-button drag. Mac 185 non-live Swift tests pass. Latest live run proves typing, text values, click, drag and capture, but scalar readback and multi-page scroll behavior need correction. All five native target jobs and aggregate gate passed at `2f895299`; the subsequent scalar/scroll delta remains to validate. No xcap extraction or fallback. |
| Independent Web & Documents capability with five utilities | Implemented in `webdocuments/` with a separate five-tool registry and updated cache/resources, discovery and direct/nested utility contracts. All standard hosted checks pass at `65c51387`; full installed-package jobs remain running. |
| One approved grant per user request/chat/model/target; no per-action prompts | Implemented; 24 Computer Use tests passed. Actual Auto/Approve/SmartApprove interaction with native capture/input remains open. Chat mode still executes no tools; completion/drop revokes the grant. |
| Distinct private/public disclosure; unknown destination treated as public | Prompt/UI disclosure and provider hooks implemented. OpenAI-compatible, Anthropic, Ollama and Versa report actual HTTP(S) origins; Lead/Worker reports both. Exact resolved routes are hashed into consent identity, never displayed. Four focused destination regressions passed within the 24-test Computer Use suite. Private does not imply on-device. |
| Private/public chat isolation and handoff acknowledgement before capture | Isolation and handoff controls implemented with consent/runtime regressions; live two-chat observation and handoff acceptance remains open. Desktop remains physically shared. |
| Persistent chat runtime and non-pooling; single controller across backend processes | Runtime ownership and stale-state protections implemented and tested; live integrated two-chat/two-process acceptance remains open. |
| Stop/revoke/cancellation, exact process cleanup, bounded timeout, no mutation replay | Cleanup/cancellation protections implemented; focused late-result regression passed. Live Stop rejected subsequent native calls. The final-package repeat returned HTTP 200 in 5 ms, showed stopped in the UI, and rejected the next native action. An earlier delayed display did not recur; no speculative code fix was made. |
| Credentials stripped from descendants; observations not logged or shared through resource registries | Environment, logging and resource protections implemented; 72 sensitive-operation regressions passed. Bounded independent source review at `2f895299` found no actionable consent/isolation/legacy-route/prompt defects; live edge coverage remains separate. |
| Same sole native path in direct calls, coding-agent bridge, nested JavaScript, and delegated work | Bridge roster updated in `agents/agent.rs`; four census regressions and 24 Computer Use tests passed. Final integrated-path validation remains required. Delegation never silently inherits another chat's consent or observations. |
| All prompts, builtin contexts, active docs, workflows and fixtures reflect the new tools | Updated system/desktop/subagent guidance and snapshots, about-biorouter context, current docs and landing references, workflow fixture, bridge/module rosters, provider and Agent Drafter harnesses, and web tool discovery fixtures. Static legacy-reference scan clean in those active surfaces; 28 prompt-manager tests passed. All standard hosted checks pass at `65c51387`. |
| CLI/Electron/serve defaults preserve explicit disablement and restrictions | Default/restriction handling implemented; Luna verified both new capabilities enabled and no old desktop capability. UI regressions passed; live upgrade/opt-out acceptance remains open. |
| Setup/status/consent/revoke interfaces, target host, active indicator, doctor diagnostics | Source implemented: CLI/TUI poll host consent, render shared task disclosure, and revoke on Stop; doctor calls no-capture diagnostics. Serve uses interactive `--computer-use-approval` with a separate digest/header and bounded failed-key attempts. Tests added in CLI session and server auth/startup. Test-inclusive core/server/CLI check passed. Generated schema/client, TypeScript, scoped lint and 59 UI tests passed; Luna verified setup and per-task controls. Live approval and Stop behavior with real capture/input remain required. |
| Native helper pin, patch provenance, deterministic locator, no npm/runtime download | Implemented. Five target payloads built locally; all file/target hashes verified; 11 packaging regression tests passed. macOS ARM64/Intel and Linux ARM64 native protocol checks passed. Final local macOS app and isolated CLI installer/doctor receipts passed (below); release-installed platform coverage remains separate from helper-only checks. |
| Every supported release artifact contains matching helper, notices and dependencies | Build/staging/provenance integrated for all nine released archives: macOS ARM64/Intel DMG+updater ZIP, Windows x64 ZIP, Linux GUI/CLI DEB/RPM. Docker supports matching Linux x64/ARM64 helpers and dependencies. The final local macOS ARM64 app ZIP was fully extracted and passed ad hoc signature, dependency and backend-hash checks (receipt below). Release signing/notarization and other installed release artifacts remain unvalidated. |
| macOS signing/notarization/TCC identity and upgrade continuity | Local ARM64 outer app and both Mac helpers now pass Developer ID signature verification. Normal app startup works without a mock keychain. Both permissions now pass. TCC retained the old ad hoc requirement despite an enabled switch; an official reset scoped to Accessibility/org.biorouter.computer-use, re-addition and helper restart repaired it. Notarization, Intel app installation and minimum-OS behavior remain open. |
| Windows interactive desktop, UIA, DPI/multiple displays and capture/focus behavior | The wheel-coordinate defect is fixed; hosted real fixture passed vertical/horizontal scroll, text, key and capture checks. DPI/multiple displays and installed package evidence remain open. |
| Linux AT-SPI dependencies, X11 and Wayland honest capabilities | Real Linux x64 and ARM64 GTK/AT-SPI fixtures passed with the final patch: discovery/tree, editable text, accessibility click, independent text, scroll adjustment, nonblank window PNG, explicit unsupported Wayland doctor and capture refusal. Source DEB/RPM dependencies declared for GUI+CLI. Actual installed BioRouter packages and native Wayland coverage remain incomplete. |
| Real BioRouter driven by Luna: scrolling, web tasks, local application tasks | BioRouter native browser typing, click, scrolling, bottom-button click and navigation passed against a synthetic local fixture, independently verified by events. A second chat required a fresh grant. The final signed package then edited and saved the preopened TextEdit fixture through 19 native calls; independent disk verification found exactly the requested added line. Direct helper or external-driver success alone is insufficient. |
| Fixture-scoped self-test on rebuilt runtime | Required. `biorouter run --workflow biorouter-self-test.yaml`; record selected phases and actual outcomes. No-desktop/unobservable cases remain explicit gaps. |
| Formatting, build, targeted tests, clippy, generated schema and full project gates | Full `just check-everything` passed, including clippy and generated-schema gates. Final production CLI/daemon build, nine serve lifecycle tests and one parser test passed. All standard hosted gates pass at `65c51387`, including all five native targets and all three Rust platforms; full installed-package acceptance found a dependency-guard false positive; its correction and remaining Mac jobs are in progress. Receipt: `target/computer-use/evidence/ci-65c51387.json`. |
| Source reviewed and pull request pushed | Draft [PR #330](https://github.com/BaranziniLab/biorouter/pull/330) is pushed and reviewable. Source `2f895299` is pushed; final review, scalar/scroll corrections and remaining hosted gates stay open; a draft PR does not complete the other gates. |

Implementation owners should replace “In progress” or “Pending” with exact command/log/artifact evidence as it becomes available. Keep any unsupported environment/action explicit and resolve it before claiming the promised platform is supported. Final completion requires the user's full scope, including the pushed PR and Luna-driven real-app checks.

## September 18 live checkpoint

- Final native CI at `231daabc` passes all five targets and the aggregate gate. The full local project gate passes (`/tmp/biorouter-cu-231-final-gate.log`). The rebuilt Developer ID signed ARM64 app retains both OS permissions; ZIP extraction verifies 676 exact entries, contained dependencies and helper integrity. Signed backend program bytes match the production build, excluding only the code signature and its size fields. Receipts: `packaged-main-integrated-signing.json`, `packaged-macos-main231-archive.json` and `permission-main231-20260918.json` under `target/computer-use/evidence/`. Luna session `20260918_9` passes 16 native calls covering typing, text values, click, scroll, Home, secondary increment, drag and capture; exact three-page and unsupported scalar checks remain separate pending cases.
- Old Intel package run `35378834831` finished compilation and Forge packaging but failed because its Electron framework was unsigned. The harness now signs frameworks before nested apps and the parent, confines deep signing to Frameworks, and retains successful Rust caches after later failures. Twelve installed-readiness tests pass, including a real unsigned-framework failure/fix with unchanged Computer Use bytes; an actual copied Electron bundle also passes the fix. The known-broken queued package run was canceled before any installed-package job completed, and a corrected all-format run is required. No application source changed for this harness correction.
- Native patch `7fd33dda2fe55094ed05ac620da8e5f26eee49f94df90fc06b708e9fc31aef42` corrects scalar false success and coalesced page scrolling. Scalar setters write once and verify the value; unknown outcomes error without replay. Page scrolling waits for target-local geometry or axis-value progress, reports actual pages at a proven target-owned boundary, and stops on unknown progress. Oversized page counts are rejected before integer conversion. All 198 non-live Swift tests pass (`/tmp/cu-scroll-edge-final.log`); deliberate old-behavior mutations fail and independent review found no remaining actionable defect. Rebuilt package/live acceptance remains pending.
- Full local `just check-everything` also passed at native-fix checkpoint `2f895299`: `/tmp/biorouter-cu-2f895299-full-gate.log`. This includes clippy, UI checks, generated schema, version, branding and cross-drift gates. The subsequent native scalar/scroll delta requires its own final validation.
- Latest merged production CLI/daemon build passed in 19m08s; receipts: `/tmp/biorouter-cu-merged-release-build.log` and `target/computer-use/evidence/main-integrated-backends.json`. The main merge includes #329/#331 and passed the full local gate (`/tmp/biorouter-cu-main-merge-full-gate.log`), regenerated API client and seven preview tests. These new binaries have not yet completed packaged live acceptance.
- Mac helper checkpoint `43e7060f` completed CLI session `20260918_8`: background typing, text `set_value`, confirmation, secondary increment, drag and capture passed. Slider `set_value("60")` returned success without changing the control; a postcondition correction remains in progress. Fixture events independently prove scroll movement at 797, 1594, 2391 and 2645, even though the AX tree did not reveal movement. Multi-page requests advanced only one viewport and are under investigation. Receipt: `target/computer-use/evidence/mac-scroll-session8-attribution.json`.
- The earlier background typing defect is corrected by checking editable roles and verifying focused element ownership before reading or writing text. Exact captured-window activation is required for consented global fallback. Mac non-live regression suite: 185 passed, including deliberate focus-ownership and scalar-conversion negative checks. The first workflow run also used the invalid generic app name Browser; discovery guidance now requires the exact running app identity.
- Native CI run `35383167034` at `e05a1d75` passed both Mac and both Linux jobs but failed the new Windows independent drag assertion: the old message-based gesture produced no child events. Pushed source `2f895299` replaces it with explicit-consent, checked-target SendInput, reviewed without outstanding findings; actual Windows acceptance passed run `35385380547`: 12 held-button moves, released state and exact 120px displacement in the independent child control. Receipt: `target/package-acceptance-evidence/35385380547/windows-fixture/windows-fixture-independent-drag.json`. PowerShell syntax, compiled INPUT size, contract mocks, Go tests and Windows cross-build pass; these do not prove live Windows delivery.
- Linux ARM64 now passes independent drag acceptance: 13 held-button motion events, release, and 120px movement in a real GTK child control. Both platform fixtures reject incomplete/no-op drag receipts; Windows hosted execution of the independent drag assertion now passes at `2f895299`. Receipts: `target/computer-use/evidence/linux-drag-fixture.json` and `linux-drag-fixture-independent-drag.json`.
- Fresh public Codex chat `20260918_7` passed native approval and coding-agent bridge acceptance: disclosure showed destination/model/backend, observation/control scope, shared desktop and Stop limits; one native `get_app_state` read only the synthetic TextEdit document, then stopped. DB confirms public/Codex/gpt-6-astra and the native call; Luna confirmed UI behavior. Receipt: `target/computer-use/evidence/live-public-codex-handoff-20260918.json`. Separate Anthropic chat `20260918_6` showed provider disclosure but failed for insufficient credits before native execution.
- User-requested permission recheck again confirms Accessibility and Screen Recording true, helper integrity verified, no override and status ready. Receipt: `target/computer-use/evidence/permission-user-recheck-20260918.json`. No additional permission approval is required.
- Instrumented focused drag passed through the signed BioRouter CLI: seven native calls, trusted slider pointer-down/moves/up, value 20 to 84 and clean exit. Receipt: `target/computer-use/evidence/live-drag-instrumented-20260918.json`. Foreground telemetry exposed competing Electron launches during earlier attempts; those attempts do not establish a drag-event defect. Separately, source review found global input could raise a different window from the captured one, and capture selection could disagree with the AX tree. The window-identity correction passes 179 non-live Swift tests, including a fail-before/pass-after regression for Stop during blocking AX validation. All five payloads rebuilt and verified; the staged signed app retains both permissions, integrity and no override. Updated-helper live acceptance remains in progress.
- Full installed-package acceptance run `35378834831` builds actual release-format candidates at `65c51387`. Windows GNU compilation passed. Linux compiled and passed the glibc floor, then failed because the dependency guard missed multiline Forge arrays despite declared zlib dependencies. The guard correction passes positive and missing-dependency negative checks. Mac ARM64 DMG and ZIP builds plus actual installed locator/integrity checks passed; receipts are under `target/package-acceptance-evidence/35378834831/darwin-arm64/`. CI had no OS desktop permissions, so these are installed-package checks, not live input acceptance. Intel Mac remains running; Windows/Linux dependent package jobs were skipped after the Linux guard failure. Final-source package acceptance remains open.
- All applicable hosted checks pass at pushed source `a0b73cc2`, including Rust on three operating systems, cross-compiles, frontend, serve and all five native targets. Receipt: `target/computer-use/evidence/ci-a0b73cc2.json`. The full local project gate and production build also pass.
- Real BioRouter native actions produced eight independent browser-fixture events: page open, synthetic text entry, confirm click, three scroll positions, bottom confirmation and second-page navigation. Receipt: `target/computer-use/evidence/live-browser-and-stop-20260918.json`.
- Final signed package TextEdit test completed 19 native calls and saved the exact requested added line once. Independent disk evidence: `target/computer-use/evidence/live-textedit-final-package-20260918.json`.
- Separate same-provider chats required separate grants. This does not prove private-to-public provider handoff or complete observation isolation.
- Final-package Stop returned HTTP 200 in 5 ms, settled visibly, and rejected a subsequent native action. Receipt: `target/computer-use/evidence/live-stop-final-package-20260918.json`. An earlier display delay was not reproduced.
- Both OS permissions now pass, with integrity verified and no override: `target/computer-use/evidence/permission-after-scoped-reset-20260918.json`. A stale old ad hoc TCC requirement required an official Accessibility reset limited to the helper bundle ID, followed by re-addition of the signed helper. Other permission records were preserved.
- The signed ARM64 candidate ZIP passes extraction, exact inventory, signatures, helper and backend hashes, and dependency checks: `target/computer-use/evidence/packaged-macos-guidance-candidate-archive.json`. It is not notarized or published. Actual full-package installation evidence for the other platforms remains separate from the successful helper fixtures; a dedicated acceptance workflow is in progress.
- The first fixture-only CLI run performed 16 native calls, then received a provider HTTP 400 reporting a malformed `functio` key. A wire regression through the real Versa sender passed 18 requests, failed under deliberate key mutation, and passed after restoration. No production workaround was added; the cause of the original request rejection remains unproven. A fresh run did not reproduce it and completed 13 native calls, but the slider drag had no observed effect. The bounded self-test therefore remains **failed**, pending coordinate/runtime diagnosis and a successful rerun. Interactive `/exit` returning zero is not evidence of workflow success.
- Fixture-only, selected-phase and full-suite workflow rendering checks pass. The fixture-only mode does not load Developer or Web & Documents and forbids fallback scripts, configuration changes and unrelated app actions.

## Historical September 17 integration, PR and Luna receipts

These are observed intermediate results, not completion of the full request.

- Draft [PR #330](https://github.com/BaranziniLab/biorouter/pull/330) contains pushed source `31a9f116`; its hosted CI remains pending.
- Full `just check-everything` passed: `/tmp/biorouter-computer-use-check-everything.log`.
- Final production CLI/daemon build passed: `/tmp/biorouter-computer-use-production-final.log`.
- Serve lifecycle suite: **9 passed**, `/tmp/biorouter-computer-use-serve-lifecycle.log`; serve options/parser regression: **1 passed**, `/tmp/biorouter-computer-use-serve-options-test.log`.
- [Native CI run 35273394648](https://github.com/BaranziniLab/biorouter/actions/runs/35273394648), source `fd0736d1`: **all five target jobs and aggregate gate green**. This includes real Windows text/key/vertical and horizontal scroll/capture and Linux x64/ARM64 GTK fixtures; macOS protocol/contract checks do not imply user-approved live capture/input.
- The core/server/CLI test-inclusive check exited 0. Focused suites passed:
  **24 Computer Use**, **1 late-result**, **72 sensitive-operation**, and
  **4 tool-census** tests. The four provider destination regressions are included
  in the 24 Computer Use tests, not an additional count.
- The production debug CLI and daemon build exited 0; receipt:
  `/tmp/biorouter-computer-use-binaries.log`.
- All **435 OpenAPI references** resolve. Generated SDK, TypeScript checking,
  scoped lint, and **59 UI tests** passed.
- The exact Forge sequential development build exited 0; receipt:
  `/tmp/biorouter-computer-use-ui-build.log`.
- Luna inspected the real isolated GUI, PID **94914**, launched from the exact
  `.vite/build/main.js` entry. It verified Computer Use and Web & Documents defaults,
  absence of the old desktop capability, per-task controls, and setup reporting
  helper **0.3.5 / darwin-arm64** with both Accessibility and Screen Recording denied.
  CUA screenshot receipts in the Luna agent task are titled **Capture top capability
  controls** and **Capture setup denial and capabilities**; no disk screenshot was
  exposed. The OS permission question remains unanswered. No real Computer Use
  input or capture has occurred in this Luna run, so scrolling, web and local-app
  tasks and the resulting fix/rerun loop remain open.
- Historical initial hosted Rust suites recorded **4093 Ubuntu passes** and **4008 Windows
  passes**, each with the same sole stale web-discovery test failure. The corrected
  test preserves relevance ranking and excludes unrelated tools for focused queries;
  the broad `web` query now correctly permits the `webdocuments` namespace match.
  These initial failures are historical, not the latest CI snapshot. Final pushed-source CI at `31a9f116` remains pending.

## Prompt, documentation and harness validation receipts

- `git diff --check`: passed for the current shared worktree at this audit.
- `python3 -m py_compile scripts/agent-drafter-testdrive/audit_platform_integrations.py scripts/agent-drafter-testdrive/run.py`: passed.
- `bash -n scripts/test_providers.sh scripts/agent-drafter-apps/round.sh`: passed.
- PyYAML loading of `biorouter-self-test.yaml` and `landing/assets/ehr-diabetes-recipe.yaml`: passed; self-test includes native fixture parameter and both separate capability registrations.
- Active prompt, builtin context, current user-guide, landing and harness scans contain no instructions to call the removed script/control or Developer capture tools. Historical records and negative-removal/configuration tests retain names only as provenance or refusal evidence.
- First `CARGO_BUILD_JOBS=4 cargo test -p biorouter --lib agents::prompt_manager::tests --no-fail-fast` stopped at stale extracted DOCX/PDF module references. Those references were corrected by the native-tools lane. The resumed build was briefly paused for high host load, then completed successfully at reduced priority: **28 selected prompt-manager tests passed**, 0 failed, 0 ignored, 4065 filtered, including all three snapshots. Other source edits landed during compilation, so final consolidated verification must rebuild the latest tree.

Earlier focused receipts above retain their original scope. Full local project gates have now passed; latest-source hosted CI, installed release artifacts and real-app gates remain separate requirements.

## Observed native patch receipts

The native lane tested the `biorouter-1` patch against upstream OCU `0.3.5`, commit
`547b4ffb8ed731a8f16486e6d8a3b215484267d3`. The repository records the source pin and
prerequisites in `third_party/open-computer-use/pin.json`; the local test checkout was
`/tmp/ocu-biorouter-native-patch`. The final patch hash is
`12214baff26bcc4a3eb4b085a08004f74ba16e8b6a2d010c633be682fb38ab67`.
The local receipts below predate the final hosted run and are historical component
evidence; the green final-patch native CI run above supersedes the pending fixture status.

| Check | Observed receipt | Scope and remaining limit |
| --- | --- | --- |
| Swift capture/connection contract | `/tmp/ocu-native-swift-tests.log`: 5 tests, 0 failures. | Owner-scoped cancellation, disconnect handling, malformed capture arguments, fresh connection state, and passive diagnostics; no real GUI validation. |
| Swift existing regression suite | `/tmp/ocu-native-swift-regression-tests.log`: 163 tests, 0 failures. **168 Swift tests total** across the two selected suites. | macOS ARM64 local unit/regression evidence; excludes the separate live SkyClick suite and does not validate Intel packages or TCC upgrade behavior. |
| Linux Go runtime suite | `/tmp/ocu-native-linux-go-tests.log`: package result `ok`, 0.219 s. | Go-side runtime contract evidence from the local host; not a Linux desktop session. |
| Windows Go runtime suite | `/tmp/ocu-native-windows-go-tests.log`: package result `ok`, 0.213 s. | Go-side runtime contract evidence from the local host; not Windows UI Automation execution. |
| Linux Python runtime contract | `/tmp/ocu-native-linux-python-tests.log`: `python3 -m unittest -v runtime_test` completed with 12 tests, `OK`, exit 0. | Mocked GI/AT-SPI/GDK tests verify interface handling, passive diagnostics, missing dependencies, Wayland capture refusal, X11 image/error shapes, and focused scroll/key delivery with modifier cleanup; actual desktop receipts are recorded separately below. |
| macOS stdio initialization and tool roster | `/tmp/ocu-native-swift-protocol.json`: server `open-computer-use` 0.3.5, protocol `2025-03-26`, exactly ten expected tools including `screen_capture`. | Confirms native initialization and advertised contracts; no capture or input action was performed by this handshake. |

Windows/Linux window capture reads visible screen pixels and can include occluding windows.
Linux Wayland pixel capture currently returns an unsupported-environment error. These limits
are documented in the user-facing Computer Use guide and must not be described as complete
platform parity. Installed release artifacts, remaining platform edge cases, and Luna-driven BioRouter
tasks remain required above; Windows/Linux fixture passes are recorded separately.

## Native payload packaging and runtime receipts

A previously tested revision of `0002-native-capture-isolation.patch` had SHA-256
`20e89fcc73cd6ae81d226da87ac097e164230a816472e2f7631676771630ac96`.
This is historical receipt identity, not the final shipping hash. The final patch
hash is `12214baff26bcc4a3eb4b085a08004f74ba16e8b6a2d010c633be682fb38ab67`;
all five target jobs and the aggregate gate passed native CI run 35273394648.
The local build receipts below describe their recorded revision unless stated otherwise.
The original GTK fixture caught a real scroll failure: the helper ignored its
element target and used a keysym as a hardware keycode. Targeted focus and proper
AT-SPI key synthesis fixed it; the independent scroll assertion was retained.

- All five `scripts/computer-use-runtime.py build <target>` builds completed:
  `darwin-arm64`, `darwin-x64`, `win32-x64`, `linux-x64`, and container target
  `linux-arm64`. Builds used one worker at reduced priority.
- `scripts/computer-use-runtime.py verify <target>` passed for all five payloads at that tested revision.
  `target/computer-use/evidence/local-payload-builds-final.json` records actual
  executable/manifest digests, architecture output, source pin, and patch hashes.
- Both Mac payloads passed `scripts/test-computer-use-protocol.py`: version 0.3.5,
  protocol 2025-03-26, exactly ten tools. Intel execution was through local Rosetta;
  CI separately runs on the native `macos-26-intel` runner. No capture/input was
  performed by these protocol checks.
- The Mac bundles are ad hoc signed locally. A raw proxy-disabled passive doctor
  inherited parent permissions and reported ready; this is **not** the app-agent
  permission result. The actual launched BioRouter helper app-agent reported
  `os_permission_required`, Accessibility false, Screen Recording false, desktop
  available true. No permission was enabled automatically. Developer ID signing,
  notarization, TCC upgrade continuity, and user-approved GUI checks remain open.
- `scripts/test-computer-use-packaging.py`: 11 tests passed, including missing and
  mutated bytes, wrong target/architecture, extra foreign files, symlink rejection,
  stale pin, Node/Python verifier agreement, final ZIP tampering, and GUI+CLI
  dependency declarations. Bash, Python and Node syntax checks passed.
- The tested `target/computer-use/linux-arm64` payload passed
  `scripts/test-computer-use-linux-fixture.py` inside a Debian ARM64 container
  capped at one CPU and 2 GiB, with Xvfb, Openbox, GTK3 and a real user D-Bus/AT-SPI
  bus. Receipts: `target/computer-use/evidence/linux-arm64-fixture-final.json`,
  its sibling PNG, and `/tmp/biorouter-cu-linux-arm64-fixture-final.log`. This
  fixture independently observes actual application state; it is not merely
  Xvfb startup or a protocol handshake. The container never uses the host desktop.
- `computer-use-native.yml` requires all four release targets plus container ARM64.
  Native jobs have a 30-minute bound, fixtures five minutes, and the aggregate
  gate five minutes. Windows session 0 exits 77 and fails the required fixture
  gate instead of reporting desktop validation. Latest macOS and Linux hosted
  native jobs passed. A historical Windows fixture caught wheel delivery at
  coordinates (0, 0). The fix is included in the final patch; the final hosted
  Windows fixture passed text, key, vertical/horizontal scroll and capture checks.

Runtime artifacts remain under ignored `target/computer-use/`; the tested ARM64
Mac payload is also staged at `ui/desktop/src/computer-use`. No release version
was changed and no release artifact was published by the packaging lane.

The Linux x64 native executable also passed the same real GTK fixture under Docker
CPU emulation on the ARM64 host. Before this run, the container was disconnected
from its network and PATH was restricted to `/usr/bin:/bin`; the test explicitly
asserted that Node, npm, Go, Swift, and git were absent. Protocol and full fixture
checks still passed (exit 0). Receipt:
`target/computer-use/evidence/linux-x64-emulated-offline-fixture.json` and sibling
PNG/log. This demonstrates offline, toolchain-free native-helper operation, but
is distinguished from the native x64 hosted runner and final BioRouter package
installation tests.

## Final local macOS app archive

The corrected self-contained ARM64 app was packaged into a **301 MiB ZIP**, fully
extracted, and checked against the final production CLI/daemon hashes. Strict deep
ad hoc signature verification passed after extraction, and its dependencies are
self-contained. Receipt: `target/computer-use/evidence/packaged-macos-final-archive.json`;
archive SHA-256 `486d61ed8559d88cac03e7ab18de8d865dd9ee204dfb2b62a6880edfa28f8eac`.
The packaged doctor passed helper integrity checks without a helper-path override
and correctly reported TCC denied: `packaged-macos-doctor-final.json` in the same
evidence directory. The actual CLI setup-path installer passed with an isolated
temporary symlink target: `installed-macos-cli-doctor.json`. Dependency evidence
records 442 module entries (`packaged-dependency-self-contained.json`), 32 contained
links (`packaged-macos-filesystem-links.json`), and all 73 original npm bin links
preserved (`npm-bin-links-before-package.json`). Four packaging regressions passed
after fixing the earlier dependency-link defect. This proves local packaging and passive
diagnostics, not live Computer Use permission or control. Developer ID release
signing, notarization, installed-platform coverage and TCC upgrade continuity
remain open. No release artifact has been published.

Remaining platform edges are explicit: Windows drag, mixed DPI, multiple monitors,
occluded windows and secure-desktop behavior remain unvalidated; Linux mixed DPI,
multiple displays and drag remain unvalidated, and Wayland pixel capture remains
unsupported. These gaps do not negate the observed fixture passes or imply full
platform coverage.
