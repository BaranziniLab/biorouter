# Docs migration

This folder records how BioRouter came to have a single plain-markdown `docs/` tree. In May 2026 the repository carried two competing documentation trees — a hand-written `documentation/` folder and a Docusaurus-generated `docs/` site — and both documents here describe the work that merged them, dropped the Docusaurus tooling, purged the Goose/Block branding the old pages carried, and renamed `recipe` to `workflow` throughout. **It happened.** The design was approved on 2026-05-07 and executed the same day: `documentation/` no longer exists, `docs/` is plain markdown, the throwaway `scripts/migrate-docs.py` was deleted as the plan specified, and `scripts/verify-docs.sh` survives in the repository. Both files are historical records, kept for provenance rather than as instructions to follow.

Come here when you want to know *why* a documentation page lives where it does, or where a page came from before the migration — the design carries file-by-file move tables covering all 36 migrated pages. Do not come here for the current layout of `docs/` or for the conventions a new page should follow. The target paths in both documents describe the tree as it stood in May 2026, and `docs/` was reorganized again in July 2026, so those paths are a record of intent rather than links you can follow today. The live authority on documentation conventions is [`docs/contributing/documentation-style.md`](../../contributing/documentation-style.md); the live tree is the rest of `docs/` itself. For the neighbouring archive of repository housekeeping, see [`branch-merge-2026-07/`](../branch-merge-2026-07/README.md), which records merges rather than documentation moves.

## Documents

| Document | What it covers |
|---|---|
| [Docs consolidation design](consolidation-design.md) | The design for merging the two documentation trees into one plain-markdown `docs/` folder — file-by-file move tables, the deletion list, the text transformations to apply, and the verification commands. Despite the `-design` filename, the body is a migration checklist. Approved 2026-05-07 and executed. |
| [Docusaurus-to-markdown migration plan](docusaurus-to-markdown-plan.md) | The task-by-task execution plan that implements that design: writing a verification script and a Python migration engine, running it over 36 files, deleting the Docusaurus infrastructure, and verifying the result. Written 2026-05-07 and carried out. Holds the only surviving copy of the deleted `scripts/migrate-docs.py`. |

The two are a pair: the design holds the mapping and the rules, the plan holds the runnable steps. Read the design first if you are tracing a page's origin, the plan first if you are reconstructing how the move was performed.

## Related documentation

- [Historical records](../README.md) — the parent archive index, which places this folder among the other 24 topic folders and explains how to read a document's `Status:` line.
- [Documentation style guide](../../contributing/documentation-style.md) — the current conventions for every file under `docs/`, which supersede any formatting guidance implied here.
- [Open documentation issues](../../contributing/open-documentation-issues.md) — the live list of documentation gaps, as opposed to this folder's closed migration work.
- [Branch merge 2026-07](../branch-merge-2026-07/README.md) — the other repository-housekeeping record in this archive, covering the July 2026 branch and pull-request merge campaign.
