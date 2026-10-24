# Scoped defensive audit completion record

The defensive audit described in AUDIT-INTENT.md is complete for the selected cases and configurations. Three findings received local source changes and passing focused regressions. The original shared checkout and the released distribution were not modified. See RESULTS.md for the case counts, provenance, recommendations, and limits.

## Completed checks

- Final release-profile build of the selected core and editor integration targets: passed.
- Core approval/session regressions: 19 tests across three suites passed.
- Scope and legacy-configuration regressions: two tests passed.
- Public editor replay: three successful processes, each with seven unsafe-path denials and four valid-use controls; no outside file or directory creation.
- Sandbox file API replay: three runs, each with seven denials and one valid-use control.
- Linux production directory guard, extracted verbatim: three two-UID fixtures passed; foreign-owned and final-symlink directories refused. This was a helper check, not a full rebuilt Linux/SQLite application replay.
- Focused production/integration-target Clippy: passed with warnings denied.
- Non-inheritable-socket gate: passed across 17 production targets.
- Rust formatting and tracked diff whitespace check: passed.
- Four manuscript DOCX packages: ZIP/XML integrity passed. Every rendered page received visual review; repaired figure pages and subsection flow received a final review.

Raw logs, command/binary execution records, and structured verification summaries are retained. Earlier aborted builds, a missing-PATH setup attempt, and the failed broader all-target release lint attempt remain distinct from the final successful checks.

## Delivery and integration status

The source changes are on the local branch `codex/security-reviewer-assessment`, based on frozen source commit `f73ebf74c578717a7af64b221e57358a63dda8e3`. GitHub issues #445, #446, and #447 remain open and include post-change results plus recommendations. Nothing has been pushed, merged, or released as part of this completion step.

The manuscript deliverables are in `/Users/wgu/Desktop/biorouter-manuscript/manuscript/v1.1`. The v0.4.2 source remains unchanged. Supplementary Data S1 and its raw-data archive report bounded evidence, rather than a general security pass rate or a certification.

Before integration, run the full repository `just check-everything`/CI checks. The attempted all-target release-profile lint encountered a pre-existing debug-only helper referenced by unit tests; its failure is retained in verification/release-all-targets-limit.md. A new distributed application build has not been validated.

Remaining unassessed areas include concurrent path replacement, Windows ACL parity, full Docker command containment, browser-engine behavior, live-model classification behavior, and exhaustive extension/configuration coverage. Issue #446 recommends directory-handle-relative I/O; issue #445 notes startup handling and symlink-relocated history compatibility. These limits remain visible in the manuscript and issues.
