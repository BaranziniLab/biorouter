# Built apps desktop followup

Completed in `/Users/wgu/Desktop/BioRouter/.claude/worktrees/built-apps-desktop-followup`,
branch `codex/built-apps-desktop-followup`, based on the hardening handoff commit `a423670d5`.
The root coordinator owns integration, the full suites, real Electron verification and pushes.
This lane did not start an app or run Cargo.

## Findings closed

- The managed app preview canceled the SDK figure shell request. Its request filter now permits
  exactly `/apps/<this-id>/figure-frame` as a GET subframe on the attached daemon's origin.
  The same path remains forbidden as a top-level document, fetch, script, image or WebSocket,
  and another app's figure path and another port are refused.
- `MCPUIResourceRenderer` now awaits `openInBrowser` for URI-list cards. A rejected local app
  browser launch displays the shared failure toast.
- Generic `write-file` now resolves the window's trusted working directory in the main process,
  passes `allowedFileRoots` to a testable writer, and checks canonical containment before it
  creates any directory or file. Symlink escapes and prefix siblings are refused. Writes still
  use an exclusive, unique temporary file and atomic rename. The two current callers now read
  the returned boolean, so a refusal is an error instead of a saved/exported success message.
- The export proof finding was already fixed in the handoff tree: `apps:export` passes the
  attached window's `daemon.userActionKey`; the source guard and export behavior tests pass.
  Shipped-skill filtering was also already present in `referencedSkills`.
  A late export review found the seeded `knowledge-bases` bundle was missing from the list;
  the UI mirror now omits that bundle too, with the existing full-export dialog regression
  expanded to cover it. Shipped skill aliases are filtered case-insensitively, matching the
  Rust export filter and case-insensitive recipient filesystems. This mirror matches the
  export lane's Rust shipped-skill list.
- While reviewing the proof path, the two renderer authentication getters were found to read
  current settings even though the window remained attached to its earlier daemon. After a
  Settings > External Backend change, the build-chat metadata read could carry the wrong proof.
  Both getters now return the owning window's attached daemon key, and consult settings only
  before attachment or on a surface with no attached window. An empty attached key remains
  empty. The window identity comes from `BrowserWindow.fromWebContents(event.sender)`.

## Verification

Used the existing desktop dependencies through a `node_modules` symlink to the main checkout.
The final targeted run used the repository's cached Hermit Node `v24.10.0` executable directly:

```sh
/Users/wgu/Library/Caches/hermit/pkg/node-24.10.0/bin/node node_modules/vitest/vitest.mjs run \
  src/utils/writeFileWithinRoots.test.ts src/utils/managedAppPreviewPolicy.test.ts \
  src/utils/appExport.test.ts src/components/MCPUIResourceRenderer.test.tsx \
  src/components/applications/buildChat.test.ts \
  src/components/applications/ApplicationsView.test.tsx \
  src/components/applications/ExportAppDialog.test.tsx \
  src/components/workflows/WorkflowsView.test.tsx \
  src/components/settings/chat/BioRouterHintsModal.test.tsx \
  src/utils/attachedDaemonKey.test.ts --maxWorkers=1
```

Result: **10 files passed, 213 tests passed, zero failed**. The unchanged failure-path tests
in ApplicationsView and WorkflowsView print their expected diagnostics. The first run found
two new test-harness mistakes (a card title selector and missing ResizeObserver for a dropdown),
both corrected before the final complete targeted run.

Final Node 24 typecheck and targeted ESLint cover every changed source/test file. Prettier was
applied to changed files; the final diff check is clean. The root coordinator must still run
the integrated full UI suite and lint/format checks and exercise the real Electron UI.

## Precise limits and remaining integration work

The optional forge-proof build-chat identity proposal is deferred by the coordinator because
it requires a trusted core session-store binding stamped by `create_app`, schema/API design and
legacy handling. The current frontend is explicit about its timestamp heuristic: it catches
a chat born after the app or last active before the app, but a hand-edited legacy manifest can
point at another local chat active across the claimed creation time. Export stripping of
`session_id` remains the protection against copied author chat IDs. The 34 buildChat tests and
37 ApplicationsView tests cover deleted, replaced, unavailable, pending, subagent-parent,
proof-bearing metadata reads and revalidation-before-navigation behavior; they do not claim
cryptographic identity proof.

After adding the seeded-bundle mirror entry, the nine ExportAppDialog tests passed again on
Node 24 with one worker; targeted lint and formatting checks passed for the two changed files.

File writes remain bounded to the trusted allowlist. Choosing a destination outside home,
temp, user data, the sandbox root or the current window's trusted working directory can be
refused; the UI now reports this honestly. Canonical containment checks handle stable symlink
escapes. They do not claim a capability against a separate local process that swaps parent
directories between checks and filesystem operations.

Real Electron verification should include a figure in an agentic managed preview, local app
URI-card failure feedback, deleted-build-chat disabled feedback, all three native export modes,
and a hints/workflow write both within and outside the roots. CI/release/live status belongs
to the integrated coordinator report, not this targeted test result.

## Independent security followup review

Source-reviewed security checkpoint `4ef194521ef1500be670d239da0f8cb5d2aeb6ae` and followup
`5f31a2ada35e6c455ab35e2b0f17ed138e83a8c3` in the security worktree, without builds or tests.
The new outer socket lifetime covers setup, consult and inline knowledge-base awaits; turn
and consulted-worker child tokens inherit cancellation. Token, app UID and retirement lifetime
come from one cached generation, and authentication compares that generation with the current
manifest before upgrade. Unknown socket requests do not allocate cached app state.

The review found a reconnect race in `UiBridge::detach`: its old atomic generation check could
pass before a concurrent attach, then clear the fresh sender and cancel the new connection's
parked tools. The security followup replaces that generation with a mutex held for the entire
attach/detach lifecycle and adds a forced-interleaving regression. The followup also gives the
shell its actual request URI instead of a default URI, so authority-only requests cannot bypass
the secure-address check. Both fixes were re-reviewed; no actionable issue remains in that
reviewed scope. The root coordinator still owns execution of the new Rust regressions.

For the forwarded HTTPS path, the secure-context gate requires both browser-owned Fetch
Metadata headers, and the document gate checks their navigation values. A page-set
`X-Forwarded-Proto` alone cannot admit an old plain-HTTP LAN app's fetch. Existing submitted
knowledge-ingest background jobs intentionally outlive the socket loop; this review does not
claim they are canceled when the app is deleted.
