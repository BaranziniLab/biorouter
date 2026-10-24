# Integration status

This is a live progress record. It does not claim completion or final-main validation.

## Existing PR integration

- Merged after review and exact-head green CI: #433, #443, #450, #451, #432, #431, #430, #440.
- #432's closing reference was changed to Refs #424 before merge because it provides narrow installed-skill reads, not the full folder-authorization feature.
- #449: main conflict resolved at pushed head 52c496b2d, preserving content-free public failure counts and adding a privacy regression for nested/operation counts. All 22 focused issue-rendering tests and full `just check-everything` passed. Exact-head hosted CI remains in progress; not merged.
- #434: HOLD. Correction 5ae1a0b0c is pushed to the contributor branch. Independent review and a formal request-changes review found a Sonnet 5.5 preserved-thinking compatibility gap; a second review of the correction found no blocking issue. Canonical/dotted model matching, captured blocking/streaming request regressions, and annotation/provenance cleanup are included. Provider tests: 1250 passed, zero failed, one existing ignored. Context tests: four passed. Live integration test binary compiled; billed API calls were not repeated. Complete `just check-everything` passed after connecting the existing UI dependencies. Fresh exact-head CI was approved and started. User-facing review summary and GitHub description/comment retain the explicit merge hold. The PR is OPEN, with no auto-merge and no mergedAt. Do not merge even if CI passes.

## Remaining open issue coverage

| Issue | Work |
| --- | --- |
| #435, #438, #441 | #449 integration and revalidation |
| #445 | Existing audit fix plus Windows ACL and recoverable storage startup work |
| #446, #447 | Local audit changes, integration checks and final build verification |
| #452 | Directory-handle leaf filesystem plus actual editor/patch/undo consumers |
| #424 | Explicit external-folder authorization and automatic continuation |
| #437 | Trusted result-file handoff or intentional direct-read authorization |
| #436 | Native asynchronous wait/capacity/structured errors plus connector adoption |
| #453 | Private persisted Developer undo snapshots and safe legacy migration |

## Additional integration findings

- A new-parent identity race could associate undo history with a replacement directory after an edit. Directory-parent capture is now cached across write and history operations. All 21 leaf filesystem tests passed, including the deterministic rename/replacement regression.
- Post-edit diagnostics previously risked reopening file paths after scoped dispatch authority ended. The integrated design computes diagnostics while the file capability is alive and consumes bounded, one-use internal receipts tied to the session, tool and exact arguments. Its new tests remain pending execution.
- Independent review found implicit installed-skill admission reopened a pathname on each dispatch, potentially broadening authority after a link replacement. The integrated correction pins that authority. A separate first-call workspace seam is corrected by retaining the workspace at trusted binding, before file admission. These core regressions remain pending execution.
- Additional session-storage alias validation now checks existing database/WAL/SHM entries before SQLite or ACL repair touches them. This is configuration-hardening work; it is not evidence of an outside-user exploit against an already private parent. Its new core regressions remain pending execution.
- Central admission now checks stored scoped directory denials on complete original and discovered file plans; remembered allows cannot replace a folder grant. Folder approval rejects a changed disclosed root. Namespace normalization preserves Windows/macOS aliases while retaining Windows path refusals. Browser folder confirmation applies existing session reach. These core/server tests remain pending execution.

## Integration test checkpoints

- Leaf filesystem: 21 passed, zero failed (`/tmp/biorouter-integration-bounded-fs-final.log`).
- MCP file-access contract: 12 passed, zero failed (`/tmp/biorouter-integration-mcp-file-access-final.log`).
- First complete Developer run: 267 passed, 28 failed, one ignored. Failures exposed legitimate macOS path-alias interoperability, including the shell-history snapshot path.
- Corrective Developer run at 20bacc84d: 297 passed, zero failed, one ignored (`/tmp/biorouter-integration-mcp-developer-after-alias.log`). Captured initial aliases must name the same retained directory object; arbitrary external paths do not gain access. A subsequent source-only cleanup removes production dead-code warnings from legacy test validators.
- Initial core production compilation exposed a missing normal dependency on biorouter-sandbox. Added via `cargo add`; the full generation/check pipeline is being rerun. A compiler attempt is not a passing validation result.
- Related connector storage issues UCSFOMOPAgent #5 and CDWAgent #7 record conditional inherited-ACL exposure and UX-preserving recommendations. POSIX full suites passed at their documented checkpoints (68 and 79 cases, three skips each); final Windows additions, bundles, new-head CI and host acceptance remain separate and pending.

Final closure also requires the specified real app checks, exact final-main CI and clean branch/worktree inventory. The frozen audit evidence remains unchanged.
