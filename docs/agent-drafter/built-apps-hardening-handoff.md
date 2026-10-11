# Built apps hardening: handoff

Status: implementation and native acceptance complete, October 11, 2026. Current CI and artifact
verification are recorded in the linked PR checks and final PR verification record.

Continue in `/Users/wgu/Desktop/BioRouter/.claude/worktrees/biorouter-built-apps-testing-cc226e`,
on `claude/biorouter-built-apps-testing-cc226e`. The upstream base is `30e1f0622`.
[PR #392](https://github.com/BaranziniLab/biorouter/pull/392) carries the integrated change. Commits are pushed
periodically; inspect `git status`, `git log` and the PR head before resuming. The main checkout
and the original QA profile have been preserved.

The authoritative acceptance status is the
[verification ledger](built-apps-hardening-verification.md). A source fix, a partial test run,
a past agent report or a successful earlier CI head does not establish final acceptance.

## What changed

The browser launch failure came from sending Chrome to Electron's secret-only local relay.
Launch now mints through that relay and opens the daemon's own URL in the default browser.
The managed in-chat preview also discovers and rebinds to the daemon origin. Failed opening
shows a useful error.

The build-chat button checks that its chat still exists before opening it. A deleted chat
leaves a disabled button and a visible reason instead of a persistent dead tab. Exported
manifests carry no source chat identifier.

Launcher, Full and bundled-daemon export requests now reach a shared Rust ZIP implementation.
It exports raw entry HTML, preserves binary assets and executable launchers, excludes working
files and secrets, and includes only selected custom skills and authorized knowledge bases.
First-run launchers import included knowledge through the authenticated daemon API, preserve
privacy/ownership rules, bind allocated IDs and retry from per-archive receipts. Recipient
synchronization preserves independent edits and distinguishes competing export folders. Node
and PowerShell normalize Windows short/long aliases of the same physical export folder.

The desktop and Linux CLI packages ship pinned, integrity-checked esbuild. Build staging handles
CSS, SDK import variants and failed builds without corrupting the app. Authored static scripts
and handlers work under the app policy; injected markup cannot reuse an authored handler.
Figures use a dedicated same-app frame that the desktop preview permits.

Lifecycle fixes bind runtime tokens to an app generation, cancel deleted apps' sockets and turns,
serialize bridge ownership, invoke deletion hooks on reset, and reject case aliases. Plain HTTP
on a network address refuses app pages and the secret-bearing serve shell; loopback and correctly
configured HTTPS remain supported.

## Review dispositions and evidence

All 21 findings from the three interrupted reviews have implementation dispositions in these
reports. They also cover the small followups named in the original handoff:

- [Security and lifecycle](built-apps-security-followup.md)
- [Export and recipient synchronization](built-apps-export-followup.md)
- [Build and browser serving](built-apps-build-followup.md)
- [Desktop navigation, authentication and file writes](built-apps-desktop-followup.md)

The original audit and reports remain at `/Users/wgu/biorouter-runs/built-apps-qa/notes/`.
Those private notes are historical evidence; their open-finding lists predate the followups.
The sanitized reports above are durable repository documentation.

The first combined Rust run exposed an HTTP seeded-bundle filtering bug and a Node-server
fixture that could fall back to a real daemon when a Hermit interpreter shim failed. It also
hit test-invocation issues: a shared outer profile conflicted with per-process sandbox guards,
and sandboxed chmod removed special permission bits. These results are diagnostic history,
not a passing full suite. The ledger records the clean rerun and subsequent checks.

## Acceptance and safe test routing

Live tests use only `/private/tmp/built-apps-final-qa/runs/gui`, an isolated copy of the original
QA profile. Recipient tests use fresh `BIOROUTER_PATH_ROOT` directories with the real `HOME`
unchanged. Test input is fictional; the configured provider is Versa Azure with GPT-5.5.
Do not print or commit credentials. The original profile and installed app are not test targets.

Rebuild the branch's daemon and Electron app before testing. A debug build may use the
`biorouter/privacy-test-auth` seam for this controlled profile; a release must omit it.
The ledger records actual native folder-picker exports, default-browser opening, build-chat
states, managed preview figures, all three recipient modes, forced source rebuilds, binary bytes,
exclusions, real knowledge-tool retrieval and provider responses. Full and fat recipient replays
reused their daemon without duplicate knowledge imports. Keep CI evidence tied to its exact head.
Run the formatter, strict clippy and `just check-everything` before pushing source changes.

## Deliberate design limits

Per-app origins and storage separation, app privacy classification and trusted identity stamps
for legacy build chats require separate designs. The browser policy relies on modern navigation
metadata and opener support; legacy loopback clients without those headers remain supported.
Exports target the exporting daemon's OS and architecture and require the recipient's own
provider configuration. Extension registry references are recorded rather than universally
bundled. Concurrent local parent-directory swaps and already-submitted background jobs need
separate containment designs. These are documented limits, not claims of completed isolation.

See [browser access](../deployment/browser-access.md),
[serve decisions](../deployment/serve-decisions.md),
[privacy tiers](../security/privacy-tiers.md),
[apps platform design](apps-platform-design.md), and the
[SDK reference](../apps-sdk/sdk-reference.md).
