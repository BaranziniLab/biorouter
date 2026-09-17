# Computer Use implementation status

Updated 2026-09-17. This ledger tracks the full user-requested result. Source changes, passing unit tests, packaging, actual GUI behavior, and a pushed pull request are separate gates. An unchecked row remains required; unavailable platform evidence is not a pass.

| Requirement | Status and evidence to collect |
| --- | --- |
| Clean replacement: exactly ten native tools; no old scripting/control, Developer capture, hidden replay routes, or aliases | In progress. `computercontroller/`, Developer router, builtin registry, active fixtures and repository-wide legacy-reference audit. |
| Native `screen_capture` in the same helper as the nine upstream tools | Native patch frozen; Swift contract/regression tests, both Go suites, and ten-tool macOS protocol handshake passed (receipts below). Actual display/window/list-only behavior and OS permissions still require GUI validation. No xcap extraction or fallback. |
| Independent Web & Documents capability with five utilities | In progress. `webdocuments/`, builtin registry, cache/resources and direct/nested utility tests. |
| One approved grant per user request/chat/model/target; no per-action prompts | In progress. Consent dispatch tests and actual Auto/Approve/SmartApprove UI/CLI receipts. Chat mode still executes no tools; completion/drop revokes the grant. |
| Distinct private/public disclosure; unknown destination treated as public | Prompt/UI disclosure and provider hooks implemented. OpenAI-compatible, Anthropic, Ollama and Versa report actual HTTP(S) origins; Lead/Worker reports both. Exact resolved routes are hashed into consent identity, never displayed. Four focused destination regressions added; execution pending. Private does not imply on-device. |
| Private/public chat isolation and handoff acknowledgement before capture | In progress. Prove no shared screenshots, trees, elements, errors, pending results, or grants; desktop remains physically shared. |
| Persistent chat runtime and non-pooling; single controller across backend processes | In progress. Runtime/ownership tests, two-chat and two-process evidence, and stale-state rejection after handoff/restart. |
| Stop/revoke/cancellation, exact process cleanup, bounded timeout, no mutation replay | In progress. Descendant cleanup and late-result rejection tests plus actual UI Stop behavior. |
| Credentials stripped from descendants; observations not logged or shared through resource registries | In progress. Environment, logging, resource, and privacy regression evidence. |
| Same sole native path in direct calls, coding-agent bridge, nested JavaScript, and delegated work | Bridge roster updated in `agents/agent.rs`; parity and consent tests pending. Delegation never silently inherits another chat's consent or observations. |
| All prompts, builtin contexts, active docs, workflows and fixtures reflect the new tools | Updated system/desktop/subagent guidance and snapshots, about-biorouter context, current docs and landing references, workflow fixture, bridge/module rosters, provider and Agent Drafter harnesses, and web tool discovery fixtures. Static legacy-reference scan clean in those active surfaces; final full audit and runtime tests pending. |
| CLI/Electron/serve defaults preserve explicit disablement and restrictions | In progress. Backend config tests, obsolete restriction diagnostics, fresh/upgrade profile tests; no renderer default migration may undo an opt-out. |
| Setup/status/consent/revoke interfaces, target host, active indicator, doctor diagnostics | Source implemented: CLI/TUI poll host consent, render shared task disclosure, and revoke on Stop; doctor calls no-capture diagnostics. Serve uses interactive `--computer-use-approval` with a separate digest/header and bounded failed-key attempts. Tests added in CLI session and server auth/startup. Execution, generated API/client checks, and UI verification remain required. |
| Native helper pin, patch provenance, deterministic locator, no npm/runtime download | In progress. Manifest/handshake/hash tests and clean-PATH installed execution required. |
| Every supported release artifact contains matching helper, notices and dependencies | In progress. macOS ARM64/Intel DMG+updater ZIP, Windows x64 ZIP, Linux desktop and CLI DEB/RPM; containers and source installation behavior audited separately. |
| macOS signing/notarization/TCC identity and upgrade continuity | Pending final signed artifact evidence, including Intel execution evidence and minimum-OS behavior. |
| Windows interactive desktop, UIA, DPI/multiple displays and capture/focus behavior | Pending interactive Windows fixture and packaged artifact evidence. A startup/version smoke alone does not pass. |
| Linux AT-SPI dependencies, X11 and Wayland honest capabilities | Python/Go contract tests pass; actual desktop fixtures and packaged dependency evidence pending. X11 requires GI, AT-SPI and GDK 3. Wayland pixel capture is explicitly unsupported, so full Wayland coverage is not complete. Xvfb startup alone is insufficient. |
| Real BioRouter driven by Luna: scrolling, web tasks, local application tasks | Required. Run against rebuilt BioRouter using its new Computer Use capability, record concrete task outcomes and screenshots/state, and fix every discovered integration bug before rerunning. Direct helper or external-driver success alone is insufficient. |
| Fixture-scoped self-test on rebuilt runtime | Required. `biorouter run --workflow biorouter-self-test.yaml`; record selected phases and actual outcomes. No-desktop/unobservable cases remain explicit gaps. |
| Formatting, build, targeted tests, clippy, generated schema and full project gates | Required. `cargo fmt`, build, affected Rust/UI suites, `./scripts/clippy-lint.sh`, `just check-everything`, cross-target/hosted CI. Record exact commands, selected test counts and exits. |
| Source reviewed and pull request pushed | Required by user. Record branch, final source commit, remote PR URL, reviewable diff and hosted CI. Do not mark complete for a local uncommitted implementation. |

Implementation owners should replace “In progress” or “Pending” with exact command/log/artifact evidence as it becomes available. Keep any unsupported environment/action explicit and resolve it before claiming the promised platform is supported. Final completion requires the user's full scope, including the pushed PR and Luna-driven real-app checks.

## Prompt, documentation and harness validation receipts

- `git diff --check`: passed for the current shared worktree at this audit.
- `python3 -m py_compile scripts/agent-drafter-testdrive/audit_platform_integrations.py scripts/agent-drafter-testdrive/run.py`: passed.
- `bash -n scripts/test_providers.sh scripts/agent-drafter-apps/round.sh`: passed.
- PyYAML loading of `biorouter-self-test.yaml` and `landing/assets/ehr-diabetes-recipe.yaml`: passed; self-test includes native fixture parameter and both separate capability registrations.
- Active prompt, builtin context, current user-guide, landing and harness scans contain no instructions to call the removed script/control or Developer capture tools. Historical records and negative-removal/configuration tests retain names only as provenance or refusal evidence.
- First `CARGO_BUILD_JOBS=4 cargo test -p biorouter --lib agents::prompt_manager::tests --no-fail-fast` stopped at stale extracted DOCX/PDF module references. Those references were corrected by the native-tools lane. The resumed build was briefly paused for high host load, then completed successfully at reduced priority: **28 selected prompt-manager tests passed**, 0 failed, 0 ignored, 4065 filtered, including all three snapshots. Other source edits landed during compilation, so final consolidated verification must rebuild the latest tree.

Pending focused checks include final-tree prompt verification, actual builtin bridge roster parity, code-execution discovery, the nested HTTP failure regression, and module import invariants. Full project, package and real-app gates above remain separate requirements.

## Frozen native patch receipts

The native lane froze the `biorouter-1` patch against upstream OCU `0.3.5`, commit
`547b4ffb8ed731a8f16486e6d8a3b215484267d3`. The repository records the source pin and
prerequisites in `third_party/open-computer-use/pin.json`; the local test checkout was
`/tmp/ocu-biorouter-native-patch`.

| Check | Observed receipt | Scope and remaining limit |
| --- | --- | --- |
| Swift capture/connection contract | `/tmp/ocu-native-swift-tests.log`: 5 tests, 0 failures. | Owner-scoped cancellation, disconnect handling, malformed capture arguments, fresh connection state, and passive diagnostics; no real GUI validation. |
| Swift existing regression suite | `/tmp/ocu-native-swift-regression-tests.log`: 163 tests, 0 failures. **168 Swift tests total** across the two selected suites. | macOS ARM64 local unit/regression evidence; excludes the separate live SkyClick suite and does not validate Intel packages or TCC upgrade behavior. |
| Linux Go runtime suite | `/tmp/ocu-native-linux-go-tests.log`: package result `ok`, 0.219 s. | Go-side runtime contract evidence from the local host; not a Linux desktop session. |
| Windows Go runtime suite | `/tmp/ocu-native-windows-go-tests.log`: package result `ok`, 0.213 s. | Go-side runtime contract evidence from the local host; not Windows UI Automation execution. |
| Linux Python runtime contract | `/tmp/ocu-native-linux-python-tests.log`: `python3 -m unittest -v runtime_test` completed with 8 tests, `OK`, exit 0. | Mocked GI/AT-SPI/GDK tests verify interface handling, passive diagnostics, missing dependencies, Wayland capture refusal, and X11 image/error shapes; no actual Linux desktop. |
| macOS stdio initialization and tool roster | `/tmp/ocu-native-swift-protocol.json`: server `open-computer-use` 0.3.5, protocol `2025-03-26`, exactly ten expected tools including `screen_capture`. | Confirms native initialization and advertised contracts; no capture or input action was performed by this handshake. |

Windows/Linux window capture reads visible screen pixels and can include occluding windows.
Linux Wayland pixel capture currently returns an unsupported-environment error. These limits
are documented in the user-facing Computer Use guide and must not be described as complete
platform parity. Installed artifacts, native GUI fixtures, and Luna-driven BioRouter tasks
remain required above.
