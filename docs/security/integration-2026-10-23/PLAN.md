# Security integration and verification plan

User intent: complete every currently open BioRouter issue, audit all existing PRs, implement missing changes, merge validated PRs into main, drive the real app, and retain reviewer-ready evidence. The user explicitly confirmed the broader repository-wide scope.

## Order and ownership

1. Review all ten existing open PRs and their exact-head CI/local-check evidence; merge dependency parents first. Keep the expanded #424 open after the narrower installed-skill read fix.
2. Integrate #433 -> #443 -> #450/#451, #431 -> #449, and independent #430/#432/#440. Resolve #449 using content-free public bug-report summaries. Review fork #434, run its checks and prepare corrections, but keep it unmerged: the user explicitly reserved its merge decision for later.
3. Extend and integrate the local audit changes for #445/#446/#447. Session storage must have Unix/Windows protection and actionable startup failure; the final built application needs separate-account verification.
4. Build #452 in two coordinated lanes: shared handle-backed actual I/O and all MCP editor/file/patch/history/undo consumers. Every patch child is planned before effects; unchecked pathname opens are not an acceptable fallback.
5. Build #424/#437 in a third coordinated lane: explicit folder-scoped human authorization, once/session/deny, automatic single continuation, revocation and all shared-dispatch paths. Retain secret and administrator restrictions. Resolve browser trusted-approval support explicitly.
   Implement #436's shared asynchronous-job wait/capacity/error contract and adoption in the related OMOP/CDW connectors, using synthetic/offline fixtures and no clinical database queries.
6. Verify each implementation with appropriate focused tests, full repository checks before pushing, exact-head hosted CI, independent review and real workflows. Fix/retest newly found defects.
7. Merge in dependency order, verify final main CI and built-source identity, then drive an isolated synthetic-data real Electron app with this chat's model. Distinguish scripted backend fixtures from real-provider behavior.
8. Update issue status and manuscript/supplement integration evidence only for demonstrated outcomes. Preserve the frozen baseline audit archive. Close findings after their acceptance criteria pass.
9. Clean merged upstream/local task branches and managed worktrees only after preserving necessary evidence/unpushed work. Check every leftover ref for unique commits; never discard unrelated unmerged work to satisfy a branch count.

## Resource controls

The initial host has high load, about9GiB used swap, roughly249GiB free internal disk, and no thermal warnings. Four independent edit/read lanes are active; no worker may start heavy builds until the coordinator grants a serialized slot. Use the existing shared Rust target and UI dependencies where compatible. At most two internally parallel local jobs, and preferably one under current pressure. Monitor CPU/load, memory/swap, disk and thermal state periodically.

WD_BLACK has approximately5TiB free and is authorized for temporary backups. Preserve synthetic evidence and source first. Delete only confirmed task-owned rebuildable artifacts; no blanket cargo clean, unrelated targets, user data, credential stores or Docker volumes.

## Current evidence policy

Merged dependency PR checks are not final integrated-build evidence. A passing test executable that merely records attack success is not a denial assertion. Require explicit expected outcomes and unchanged outside canaries. No issue is closed solely because a PR exists or CI is green.

Current local audit fixes originate at5c5dd82b96c9432e65ec1a114b68c52ab1698323. Future integration evidence belongs in this directory and does not rewrite original raw observations.
