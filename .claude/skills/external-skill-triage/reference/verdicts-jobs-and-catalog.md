# Verdicts, jobs and the catalog rules

> **What this is.** How a triage reader turns an outside item into a verdict against Biorouter's catalog, how jobs decide "one strong skill per job", and the catalog rules and conventions an implemented item must pass.
> **Status:** Current for the catalog format in biorouter-skills and `landing/catalog/` as of 2026-11.
> **Audience:** deep readers, planners, critics, implementers and the integrator.

## Where the catalog lives

| What | Where | Read it how |
|---|---|---|
| Every BAAM skill and bundle | `catalog.json` in BaranziniLab/biorouter-skills | From a fresh clone or worktree at the remote main, never the owner's local checkout |
| Its mirror in the site | `landing/catalog/skills.json` | Same bytes plus `source_commit` |
| Extensions | `landing/catalog/extensions.json` | From a biorouter worktree at `origin/main` |
| The job vocabulary, built-in jobs, required jobs | `landing/catalog/jobs.json` | Same |
| Names Biorouter ships itself | `builtin-skill-names.json` in biorouter-skills | Same |

## Verdicts

A deep reader gives each item one verdict: `new-skill`, `enrich-existing`, `new-extension`, `enrich-extension` or `skip` (definitions in [../SKILL.md](../SKILL.md), stage 3). The plan then turns accepted verdicts into work items of type `new`, `enrich`, `merge`, `retire` or `fix`.

Quality is scored 1 to 5 on method depth, correctness, safety, and whether it would actually help a researcher working with an agent. Generic advice with little method scores 2 or less, however polished.

An `enrich-existing` verdict must name the target id and the exact content to add (a mode, a reference file, a checker script, a set of fixes). "Could improve X" is not a verdict an implementer can act on.

## One strong skill per job

- A job is a dotted `<domain>.<task>` key from the vocabulary in `jobs.json`. A skill names a method; an extension names access to a service; a bundled adapter skill's job ends in `.tool-usage` and names the BAAM skill that holds the method.
- Before proposing a new skill, find the job's owner: the `job` of every active row, `builtin` in `jobs.json` (jobs Biorouter itself owns), and `required` (the gap list).
- Prefer enriching the owner. Fold the runners-up into the winner as references or modes rather than as more skills.
- Watch the winner's size. In 2026-11 one skill already held 398 files and 7.3 MB, so its plan gave it a size budget and moved a new mode elsewhere.
- A script two skills need has one owner; the other carries a byte identical copy that the catalog check verifies, because `build-zip.mjs` has no shared file mechanism.
- New extensions are proposed for service access (a database, an API, a desktop app), not for method; the method stays in a skill the extension's bundled skill points at.

## The rules that will fire

The catalog check enforces these; the wording lives in biorouter-skills `README.md` ("The catalog") and [`docs/extensions/baam-catalog.md`](../../../../docs/extensions/baam-catalog.md).

| Rule | What it means for a triage item |
|---|---|
| D1 | No two active items own one job, and none claims a built-in job |
| D2 | Ids, display names, aliases and built-in names are one namespace |
| D3 | Look-alike text needs a merge or a `distinct_from` boundary |
| D4 | A merged item points at an active target that lists the old id in `aliases` |
| D5 | Component names are unique; look-alike components across bundles record a boundary |
| D6 | No active skill is a template shell copied from five or more others |
| G1 | Every required job has an owner or a plan |

Conventions learned the hard way in 2026-11:

- **Aliases are for top level items only.** A component name never moves and is never reused; a merged component keeps its row with `superseded_by:<target>`.
- **Use exact component names.** Some components' frontmatter names are prefixed with their bundle (for example `database-access-open-literature-discovery`); a plan that drops the prefix names nothing.
- **A required job lands with its owner change.** An item that claims a job listed in `required` ships together with the `jobs.json` owner change in the same BAAM pull request, or G1 fails.
- **Check every proposed id** against ids, aliases, component names and built-in names before it reaches the plan.
- **Keywords decide look-alikes.** Keep a new item's keywords clear of a neighbour's terms, or write a boundary that says what each one does that the other does not.
- **Boundaries are reciprocal when both sides are close.** Write the line on both rows when each could be mistaken for the other.

## Versions

- A fix is a patch; new content is a minor version; replacing an existing skill or extension is a minor version (owner, 2026-11-21).
- Raise the row's `version` and `metadata.version` in every `SKILL.md` of the item, and add a `history` entry with the date and a one line note (for a security sensitive item, name the review).
- A merge or retirement also raises the old item's version, and its folder becomes the generated redirect stub (`node scripts/write-stubs.mjs`).

## Auditing Biorouter's own catalog

A triage is a good moment to audit the catalog itself. Split it into slices (for example by category, about 20 items each), give each slice to one Opus agent that reads every `SKILL.md` in its slice and the summary, job and boundaries of all items, and ask for duplicates, unclear boundaries, thin skills, stale content (confirmed by reading, not by fetching), broken structure and gaps, each with a concrete fix. The planner treats these findings like any repo report.

## Related documentation

- [../SKILL.md](../SKILL.md): the stages that use these verdicts.
- [implement-release-list.md](implement-release-list.md): how rows are staged and merged.
- [`docs/extensions/baam-catalog.md`](../../../../docs/extensions/baam-catalog.md): the catalog files and common changes.
