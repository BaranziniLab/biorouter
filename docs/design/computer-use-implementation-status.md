# Computer Use implementation status

Updated 2026-09-17. This ledger tracks the full user-requested result. Source changes, passing unit tests, packaging, actual GUI behavior, and a pushed pull request are separate gates. An unchecked row remains required; unavailable platform evidence is not a pass.

| Requirement | Status and evidence to collect |
| --- | --- |
| Clean replacement: exactly ten native tools; no old scripting/control, Developer capture, hidden replay routes, or aliases | In progress. `computercontroller/`, Developer router, builtin registry, active fixtures and repository-wide legacy-reference audit. |
| Native `screen_capture` in the same helper as the nine upstream tools | Native implementation and prior-revision Swift contract/regression tests, both Go suites, and ten-tool macOS protocol handshake passed (receipts below). Final patch freeze awaits the Windows fixture fix. Actual display/window/list-only behavior and OS permissions still require GUI validation. No xcap extraction or fallback. |
| Independent Web & Documents capability with five utilities | In progress. `webdocuments/`, builtin registry, cache/resources and direct/nested utility tests. |
| One approved grant per user request/chat/model/target; no per-action prompts | In progress. Consent dispatch tests and actual Auto/Approve/SmartApprove UI/CLI receipts. Chat mode still executes no tools; completion/drop revokes the grant. |
| Distinct private/public disclosure; unknown destination treated as public | Prompt/UI disclosure and provider hooks implemented. OpenAI-compatible, Anthropic, Ollama and Versa report actual HTTP(S) origins; Lead/Worker reports both. Exact resolved routes are hashed into consent identity, never displayed. Four focused destination regressions passed within the 24-test Computer Use suite. Private does not imply on-device. |
| Private/public chat isolation and handoff acknowledgement before capture | In progress. Prove no shared screenshots, trees, elements, errors, pending results, or grants; desktop remains physically shared. |
| Persistent chat runtime and non-pooling; single controller across backend processes | In progress. Runtime/ownership tests, two-chat and two-process evidence, and stale-state rejection after handoff/restart. |
| Stop/revoke/cancellation, exact process cleanup, bounded timeout, no mutation replay | In progress. Descendant cleanup and late-result rejection tests plus actual UI Stop behavior. |
| Credentials stripped from descendants; observations not logged or shared through resource registries | In progress. Environment, logging, resource, and privacy regression evidence. |
| Same sole native path in direct calls, coding-agent bridge, nested JavaScript, and delegated work | Bridge roster updated in `agents/agent.rs`; four census regressions and 24 Computer Use tests passed. Final integrated-path validation remains required. Delegation never silently inherits another chat's consent or observations. |
| All prompts, builtin contexts, active docs, workflows and fixtures reflect the new tools | Updated system/desktop/subagent guidance and snapshots, about-biorouter context, current docs and landing references, workflow fixture, bridge/module rosters, provider and Agent Drafter harnesses, and web tool discovery fixtures. Static legacy-reference scan clean in those active surfaces; final full audit and runtime tests pending. |
| CLI/Electron/serve defaults preserve explicit disablement and restrictions | In progress. Backend config tests, obsolete restriction diagnostics, fresh/upgrade profile tests; no renderer default migration may undo an opt-out. |
| Setup/status/consent/revoke interfaces, target host, active indicator, doctor diagnostics | Source implemented: CLI/TUI poll host consent, render shared task disclosure, and revoke on Stop; doctor calls no-capture diagnostics. Serve uses interactive `--computer-use-approval` with a separate digest/header and bounded failed-key attempts. Tests added in CLI session and server auth/startup. Test-inclusive core/server/CLI check passed. Generated schema/client, TypeScript, scoped lint and 59 UI tests passed; Luna verified setup and per-task controls. Live approval and Stop behavior with real capture/input remain required. |
| Native helper pin, patch provenance, deterministic locator, no npm/runtime download | Implemented. Five target payloads built locally; all file/target hashes verified; 11 packaging regression tests passed. macOS ARM64/Intel and Linux ARM64 native protocol checks passed. Final installed BioRouter and clean-PATH receipts remain separate from helper-only checks. |
| Every supported release artifact contains matching helper, notices and dependencies | Build/staging/provenance integrated for all nine released archives: macOS ARM64/Intel DMG+updater ZIP, Windows x64 ZIP, Linux GUI/CLI DEB/RPM. Docker supports matching Linux x64/ARM64 helpers and dependencies. Final BioRouter package extraction/signing/install evidence remains required; helper builds alone do not pass that gate. |
| macOS signing/notarization/TCC identity and upgrade continuity | Pending final signed artifact evidence, including Intel execution evidence and minimum-OS behavior. |
| Windows interactive desktop, UIA, DPI/multiple displays and capture/focus behavior | Hosted interactive fixture verified text, F6 and vertical scrolling, but exposed wheel delivery at coordinates (0, 0); fix and rerun are in progress. DPI/multiple displays and installed package evidence remain open. A startup/version smoke alone does not pass. |
| Linux AT-SPI dependencies, X11 and Wayland honest capabilities | Real Linux ARM64 GTK/AT-SPI fixture passed against the tested packaged helper revision: discovery/tree, editable text, accessibility click, independent text, scroll adjustment, nonblank window PNG, explicit unsupported Wayland doctor and capture refusal. Source DEB/RPM dependencies declared for GUI+CLI. Actual installed BioRouter packages and native Wayland coverage remain incomplete. |
| Real BioRouter driven by Luna: scrolling, web tasks, local application tasks | Partially observed. Luna verified the rebuilt isolated GUI, default capabilities, setup denial and per-task controls (receipts below). The OS permission question is unanswered; no real Computer Use input/capture, scrolling, web task or local-app task has passed yet. Direct helper or external-driver success alone is insufficient. |
| Fixture-scoped self-test on rebuilt runtime | Required. `biorouter run --workflow biorouter-self-test.yaml`; record selected phases and actual outcomes. No-desktop/unobservable cases remain explicit gaps. |
| Formatting, build, targeted tests, clippy, generated schema and full project gates | Selected checks and exact CLI/daemon/Forge builds passed (receipts below). Hosted Rust discovery regression was corrected and awaits rerun. Final-tree `./scripts/clippy-lint.sh`, `just check-everything`, consolidated suites and all required hosted gates remain required. |
| Source reviewed and pull request pushed | Draft [PR #330](https://github.com/BaranziniLab/biorouter/pull/330) is pushed and reviewable. Final source commit, review and all required hosted CI remain open; a draft PR does not complete the other gates. |

Implementation owners should replace “In progress” or “Pending” with exact command/log/artifact evidence as it becomes available. Keep any unsupported environment/action explicit and resolve it before claiming the promised platform is supported. Final completion requires the user's full scope, including the pushed PR and Luna-driven real-app checks.

## Latest integration, PR and Luna receipts

These are observed intermediate results, not completion of the full request.

- Draft [PR #330](https://github.com/BaranziniLab/biorouter/pull/330) is pushed.
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
- Initial hosted Rust suites recorded **4093 Ubuntu passes** and **4008 Windows
  passes**, each with the same sole stale web-discovery test failure. The corrected
  test preserves relevance ranking and excludes unrelated tools for focused queries;
  the broad `web` query now correctly permits the `webdocuments` namespace match.
  Hosted reruns remain pending; neither initial suite is recorded as green.

## Prompt, documentation and harness validation receipts

- `git diff --check`: passed for the current shared worktree at this audit.
- `python3 -m py_compile scripts/agent-drafter-testdrive/audit_platform_integrations.py scripts/agent-drafter-testdrive/run.py`: passed.
- `bash -n scripts/test_providers.sh scripts/agent-drafter-apps/round.sh`: passed.
- PyYAML loading of `biorouter-self-test.yaml` and `landing/assets/ehr-diabetes-recipe.yaml`: passed; self-test includes native fixture parameter and both separate capability registrations.
- Active prompt, builtin context, current user-guide, landing and harness scans contain no instructions to call the removed script/control or Developer capture tools. Historical records and negative-removal/configuration tests retain names only as provenance or refusal evidence.
- First `CARGO_BUILD_JOBS=4 cargo test -p biorouter --lib agents::prompt_manager::tests --no-fail-fast` stopped at stale extracted DOCX/PDF module references. Those references were corrected by the native-tools lane. The resumed build was briefly paused for high host load, then completed successfully at reduced priority: **28 selected prompt-manager tests passed**, 0 failed, 0 ignored, 4065 filtered, including all three snapshots. Other source edits landed during compilation, so final consolidated verification must rebuild the latest tree.

Pending focused checks include final-tree prompt verification, actual builtin bridge roster parity, code-execution discovery, the nested HTTP failure regression, and module import invariants. Full project, package and real-app gates above remain separate requirements.

## Observed native patch receipts

The native lane tested the `biorouter-1` patch against upstream OCU `0.3.5`, commit
`547b4ffb8ed731a8f16486e6d8a3b215484267d3`. The repository records the source pin and
prerequisites in `third_party/open-computer-use/pin.json`; the local test checkout was
`/tmp/ocu-biorouter-native-patch`. Final patch freeze and hash confirmation remain
pending the Windows fixture correction and rebuilt payload validation.

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
platform parity. Installed artifacts, native GUI fixtures, and Luna-driven BioRouter tasks
remain required above.

## Native payload packaging and runtime receipts

A previously tested revision of `0002-native-capture-isolation.patch` had SHA-256
`20e89fcc73cd6ae81d226da87ac097e164230a816472e2f7631676771630ac96`.
This is historical receipt identity, not the final shipping hash. The latest helper
hash and payload rebuild receipts remain pending the final Windows fix and freeze.
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
  native jobs passed. The Windows interactive fixture passed text, F6 and vertical
  scrolling, then identified wheel delivery at coordinates (0, 0); correction and
  rerun remain required before declaring the Windows native gate passed.

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
