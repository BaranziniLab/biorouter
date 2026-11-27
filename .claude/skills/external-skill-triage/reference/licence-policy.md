# Licence policy for outside material

> **What this is.** What Biorouter may take from an outside skill, plugin or extension under each licence, how to check the licence that really applies, and how attribution is recorded in biorouter-skills.
> **Status:** Current, as applied in the 2026-11 triage campaigns. The owner decides anything this page leaves open.
> **Audience:** triage readers, planners, implementers and the licence reviewers of each item.

## Classes

| Licence found | Class | What Biorouter may do |
|---|---|---|
| MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, 0BSD, CC0 | permissive | Adapt with attribution (below) |
| CC BY 4.0 | attribution only | Adapt only after an owner decision, keeping the attribution; in 2026-11 the owner held CC BY prose until decided |
| CC BY-SA, CC BY-NC, CC BY-NC-SA, CC BY-NC-ND | share alike or non-commercial | Ideas only |
| GPL, AGPL, and any copyleft the owner has not cleared | copyleft | Ideas only |
| No licence, NOASSERTION, proprietary, source available, a custom licence with restrictions | none or restrictive | Ideas only |

Examples that came up: a vendor's document skills ship their own restrictive `LICENSE.txt` inside a permissively licensed repo, and an automated science project uses a custom licence; both are ideas only.

## What "ideas only" allows

- Allowed: methods, structure, the list of checks a method needs, facts, and citations of primary sources, written in original wording from those primary sources.
- Not allowed: text, prompts, code, figure scripts, templates, tables, checklists, close paraphrase, or the source's own section order refilled with reworded sentences.
- If an item's scope would in practice need copying (for example reproducing a figure script set), rescope it or drop it. The value and licence critic looks for exactly this.
- The per item reviewer runs a similarity check of the new text against every ideas only source it names.

## Find the licence that really applies

1. **Per item beats per repo.** A `LICENSE` file inside a skill or plugin folder, a `license:` field in frontmatter, or a per skill licence table overrides the repository licence. Prose and scripts can differ (for example scripts Apache-2.0, prose CC BY 4.0).
2. **Follow the lineage.** Read the README credits and look for "based on", "adapted from" and files copied from elsewhere. When the upstream itself adapted someone else's material, the most restrictive licence in the chain governs those parts. In 2026-11 an MIT research skill pack had adapted two files from unlicensed and non-commercial sources, which made those two files ideas only.
3. **Vendor derived copies stay restricted.** A permissive repo that carries copies of a restrictively licensed vendor skill does not make that skill reusable.
4. **Relicensing by a contributor's statement is weak.** Treat such material as ideas only unless the original author's grant is clear.
5. **Read the NOTICE.** Apache NOTICE files must be kept; some forbid a name for derivatives (one review tool's NOTICE forbids its name on derived work).
6. **Quotations and instruments.** Never carry direct quotations of named authors, and never reproduce a published instrument under NC-ND terms (for example a risk of bias tool); cite it instead.
7. **Data terms.** A tool that wraps a database also carries the provider's terms (non-commercial APIs, CC BY-SA data). Record them per component.
8. **Pin the commit.** Record the exact commit taken; `scripts/upstream-drift.mjs` reports later upstream changes against it.

## Attribution mechanics in biorouter-skills

`phi-deidentification` is the model to copy. For an adapted item:

- The row's `source` block: `origin: "adapted"`, one entry per upstream in `upstreams` (`repo`, `path`, `ref`, `commit`, `licence`, `copyright`, `taken`), `maintained_here`, and `changes`, which states what Biorouter changed.
- The row's `licence` is the SPDX expression of the archive, for example `Apache-2.0 AND MIT`.
- `notice_file: "LICENSE-UPSTREAM"`, a file in the skill folder holding each upstream copyright line and full licence text; for an Apache-2.0 upstream it also carries the upstream NOTICE content.
- The `license` line of every `SKILL.md` of the item matches the row.
- The repository `NOTICE` is generated from the rows by `node scripts/render-readme.mjs`. Never edit it by hand; `--check` fails when it drifts. BSD-3-Clause entries get the non-endorsement line from the generator.
- An ideas only source is still named in the row's sources with its reuse class, so a reviewer can run the similarity check.

## Never

- Copy from any repository of the project Biorouter was derived from or its publisher, or name them anywhere.
- Claim a licence the source does not state; when unsure, the item is ideas only until the owner decides.

## Related documentation

- [../SKILL.md](../SKILL.md): rule 5 and the planning stage.
- [security-checklist.md](security-checklist.md): item 13 covers third party terms.
- `README.md` of BaranziniLab/biorouter-skills, section "The catalog": the row format.
