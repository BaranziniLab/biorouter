# Integration status

This is a live progress record. It does not claim completion or final-main validation.

## Existing PR integration

- Merged after review and exact-head green CI: #433, #443, #450, #451, #432, #431, #430, #440, #449.
- #432's closing reference was changed to Refs #424 before merge because it provides narrow installed-skill reads, not the full folder-authorization feature.
- #449: merged as 18858616228c9e8d84166fe686ce7ced84a3b11d after all 17 exact-head checks passed at 52c496b2d. This preserves content-free public failure counts and adds a privacy regression for nested/operation counts. All 22 focused issue-rendering tests and full `just check-everything` passed before pushing. #435, #438 and #441 are closed. The integration branch includes this main merge; final-main verification remains pending.
- #434: HOLD. Correction 5ae1a0b0c is pushed to the contributor branch. Independent review and a formal request-changes review found a Sonnet 5.5 preserved-thinking compatibility gap; a second review of the correction found no blocking issue. Canonical/dotted model matching, captured blocking/streaming request regressions, and annotation/provenance cleanup are included. Provider tests: 1250 passed, zero failed, one existing ignored. Context tests: four passed. Live integration test binary compiled; billed API calls were not repeated. Complete `just check-everything` passed after connecting the existing UI dependencies. Fresh exact-head CI was approved and started. User-facing review summary and GitHub description/comment retain the explicit merge hold. The PR is OPEN, with no auto-merge and no mergedAt. Do not merge even if CI passes.

## Remaining open issue coverage

| Issue | Work |
| --- | --- |
| #435, #438, #441 | Closed through #449; included in combined integration checks |
| #445 | Existing audit fix plus Windows ACL and recoverable storage startup work |
| #446, #447 | Local audit changes, integration checks and final build verification |
| #452 | Directory-handle leaf filesystem plus actual editor/patch/undo consumers |
| #424 | Explicit external-folder authorization and automatic continuation |
| #437 | Trusted result-file handoff or intentional direct-read authorization |
| #436 | Native asynchronous wait/capacity/structured errors plus connector adoption |
| #453 | Private persisted Developer undo snapshots and safe legacy migration |

## Additional integration findings

- A new-parent identity race could associate undo history with a replacement directory after an edit. Directory-parent capture is now cached across write and history operations. All 21 leaf filesystem tests passed, including the deterministic rename/replacement regression.
- Post-edit diagnostics previously risked reopening file paths after scoped dispatch authority ended. The integrated design computes diagnostics while the file capability is alive and consumes bounded, one-use internal receipts tied to the session, tool and exact arguments. All 13 focused diagnostic tests passed; agent and dispatcher receipt tests also passed within the 55-test core checkpoint.
- Independent review found implicit installed-skill admission reopened a pathname on each dispatch, potentially broadening authority after a link replacement. The integrated correction pins that authority. A separate first-call workspace seam is corrected by retaining the workspace at trusted binding, before file admission. Their focused core regressions passed.
- Additional session-storage alias validation now checks existing database/WAL/SHM entries before SQLite or ACL repair touches them. This is configuration-hardening work; it is not evidence of an outside-user exploit against an already private parent. Eight session-storage integration tests passed on macOS. Native Windows runtime acceptance of the new ACL migration guard remains pending.
- Central admission now checks stored scoped directory denials on complete original and discovered file plans; remembered allows cannot replace a folder grant. Folder approval rejects a changed disclosed root. Namespace normalization preserves Windows/macOS aliases while retaining Windows path refusals. These core tests passed. Browser folder confirmation now applies session reach; the private/unknown-session server regression and separate proof-header library test both passed.
- Windows private-storage migration now checks every retained descendant and directory ACL before any inheritable permission repair. Foreign mutation rights, unrecognized ACL forms, reparse points, hardlinks and ordinary shared-group ownership cause an actionable refusal. Independent static review found no blocker; Windows execution remains pending. Same-user/admin behavior, historical open foreign handles and transactional migration are outside the established guarantee. Retained handles can briefly conflict with concurrent Windows file replacement during initialization; no measured failure is claimed.
- Normal shared-daemon startup now displays fixed recovery guidance for recognized private-history refusal without echoing arbitrary stderr, private paths or secrets. Windows already uses the per-window error path. A browser field-label issue and an older footer assertion were corrected after the first combined UI run failed three cases.

## Integration test checkpoints

- Leaf filesystem: 21 passed, zero failed (`/tmp/biorouter-integration-bounded-fs-final.log`).
- MCP file-access contract: 12 passed, zero failed (`/tmp/biorouter-integration-mcp-file-access-final.log`).
- First complete Developer run: 267 passed, 28 failed, one ignored. Failures exposed legitimate macOS path-alias interoperability, including the shell-history snapshot path.
- Corrective Developer run at 20bacc84d: 297 passed, zero failed, one ignored (`/tmp/biorouter-integration-mcp-developer-after-alias.log`). Captured initial aliases must name the same retained directory object; arbitrary external paths do not gain access. A subsequent source-only cleanup removes production dead-code warnings from legacy test validators.
- Initial core production compilation exposed a missing normal dependency on biorouter-sandbox. Added via `cargo add`; production/schema/client generation then passed. An additional folder display-path regression was corrected by avoiding an empty suffix/trailing slash for existing files.
- Core folder authorization/security/storage integration checkpoint: 24 passed, zero failed. Core authority/skills/diagnostic/privacy unit checkpoint at 1dcadde20: 55 passed, zero failed. Smart Approve: 16 passed, zero failed. These are regression tests, not additional adversarial-trial counts.
- Combined UI checkpoint at 672655ad5: 86 passed, zero failed across seven focused files. The preceding run had 82 passed and three failed; both logs are retained. Server proof checks: one binary/session-reach test and one library/header-separation test passed. Production/schema/client generation after the main merge passed.
- Rebuilt Linux acceptance is prepared as a separate hosted workflow: real current daemon, three fresh two-UID SQLite/history/restart fixtures, no-network/read-only/capability-dropped containers, explicit outcomes and source/binary/image attribution. Static review caught the rename route's empty successful response; the corrected harness preserves strict JSON reads and adds six passing offline controls. No rebuilt-Linux runtime pass is claimed before the hosted workflow executes.
- Related connector storage issues UCSFOMOPAgent #5 and CDWAgent #7 record conditional inherited-ACL exposure and UX-preserving recommendations. The latest reviewed heads f9a6e40f0e9c92a47fec4b56214474b847c214fe and ded5d6eeb56c833ce15bc1c63cf474c350c48acb passed all six checks each, including ten native Windows controls per connector. Their latest local suites, deterministic bundle hashes and prior failures retain separate provenance in the connector evidence record. Host app acceptance has not been repeated on these heads, and both connector PRs remain open.

Final closure also requires the specified real app checks, exact final-main CI and clean branch/worktree inventory. The frozen audit evidence remains unchanged.
