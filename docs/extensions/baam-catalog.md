# The BAAM catalog

> **What this is.** How the tracking system behind BAAM (the Biorouter AI Agent Marketplace) works: the three catalog files in `landing/catalog/`, the files generated from them, the checks that hold them to the page, and how to add, update, merge or retire an extension or a skill.
> **Status:** Current. Added 2026-11-22 with the BAAM refresh; provenance records added 2026-11-28. 41 extension rows (28 active) and 145 skill rows (133 active), both catalogs in format 2.
> **Audience:** anyone who changes a BAAM card, releases an extension or a skill, or reviews such a change.

## Why there is a catalog

`landing/baam.html` holds the cards, and `landing/registry.json` is generated from them. Neither can say what an item used to be, what replaced it, what job it owns, or where its content came from, and `registry.json` cannot grow a field: every daemon since v1.90.0 parses registry v2 with `deny_unknown_fields` and falls back to a stale copy when it meets one. So every extra fact lives beside the page, in `landing/catalog/`, and a check holds the two together. Credits travel the same way: `landing/credits.json`, generated from the catalogs, carries what the registry cannot.

## The three files

| File | What it holds |
| --- | --- |
| `landing/catalog/extensions.json` | One row per BAAM extension, active or not: id, display name, repository, group (`ucsf`, `tools` or `adapters`), version, status, aliases, job, licence, privacy and affiliation, the release (tag, asset, SHA-256), bundled skills, a provenance record (`source` and `contributors`), boundaries with look-alikes and a history. Retired rows say why and what to use instead. |
| `landing/catalog/skills.json` | A mirror of `catalog.json` in BaranziniLab/biorouter-skills, byte for byte, with one line added: `source_commit`, the commit it was copied from. Each row carries the same provenance record. Skills are tracked where their content lives, so a pull request there changes a skill and its row together. |
| `landing/catalog/jobs.json` | The job vocabulary, the jobs Biorouter itself owns (built-in extensions and skills), the required jobs with their owner or the round that plans one (the gap list), and the look-alike screen settings. |

A row's `status` is `active`, `retired`, or `superseded_by:<id>` (for a skill, also `superseded_by:builtin:<name>`). An id never moves: a merged or retired item keeps its row, and its id stays reserved.

`extensions.json` and `skills.json` say `"format": 2`, which means every row records its provenance: the origin (`in-house`, `ideas-derived`, `adapted` or `third-party`), every source with how it was used, the ideas, data and services it credits, and its contributors. [Provenance and credits](provenance.md) describes the record and the lines generated from it. `jobs.json` stays at format 1.

## The generated files

`node landing/scripts/build-registry.mjs` writes five files in three directories, together, under one lock. Never edit them by hand.

| File | What it is |
| --- | --- |
| `landing/registry.json` | The marketplace catalog the app reads, from the page's static cards. |
| `ui/desktop/src/components/baam/registry.fallback.json` | The app's bundled copy of the registry. |
| `crates/biorouter/src/privacy/registry_private.rs` | The private extension set compiled into the CLI and the daemon. |
| `landing/credits.json` | One credits entry for every catalog row, active or not: its credit line, origin sentence, sources, ideas, data and services, people, earlier versions and links to its licence files and full record. |
| `ui/desktop/src/components/baam/credits.fallback.json` | The app's copy of `credits.json`, byte for byte the same. |

`node landing/scripts/write-credits.mjs` writes the credit text into `landing/baam.html`: the line under every card's description, the curator line of each shelf and of the footer, and each skill card's `data-version` and `data-license` from its row.

## The rules

The shared rules are in `landing/scripts/lib/catalog-rules.mjs`, a verbatim copy of `scripts/lib/catalog-rules.mjs` in biorouter-skills. Change both together.

- **D1, one job, one owner.** No two active items share a job, and none claims a job in `jobs.json` `builtin`. A bundled adapter skill's job ends in `.tool-usage` and names the BAAM skill that holds the method.
- **D2, one name.** Ids, display names, aliases and Biorouter's own skill and extension names (read from this repository's source) are one namespace. A retired row may hand its name to the active item that took over its job with `retired.name_passes_to`, as UCSFHPCAgent hands its name to the `ucsf-hpc` skill.
- **D3, look-alikes.** Two active extensions whose text scores at or above `similarity_threshold` (token-set Jaccard over display name, summary and keywords) need a `distinct_from` boundary on one of them. Skills are screened in biorouter-skills, which reads each `SKILL.md`.
- **D4, redirects are complete.** A `superseded_by` target exists, is active and lists the old id, and the old id's bundled skills, in `aliases`. On the page, the target extension's card says "Replaces <old name>", and the target skill's card carries the old id in `data-tags`, so a search for the old name finds it.
- **G1, gaps.** Every required job has an active owner, or a `planned` entry, which prints a warning.
- **PV1 to PV13, provenance.** Every row has an origin that agrees with its sources, complete sources, a licence that names what ships, contributors, and a credit line of at most 120 characters. The row rules are the second half of `catalog-rules.mjs`; this repository runs them over the extension rows, and biorouter-skills runs them, with its folder rules, over the skill rows. [Provenance and credits](provenance.md) lists each rule.

`baseline_violations` in either catalog lists failures that exist on purpose, each owned by a work package. An entry whose work package is in `done_work_packages`, or one that no longer fails, is itself a failure.

## What the check compares with the page

`node landing/scripts/check-catalog.mjs` reads the catalogs, `landing/registry.json` and the parts of `landing/baam.html` the registry does not carry. It fails when:

- a card has no active row, or an active row has no card;
- a card's name, group or category, licence, privacy, affiliation, version or download URL differs from its row (an extension's download is `https://github.com/<repository>/releases/download/<release.tag>/<release.asset>`);
- an extension row's `summary` or `keywords` differ from its card's description and tags;
- a skill card's `data-version` differs from its row's version;
- a superseded or retired row is missing from the Moved and retired list (`<details id="baam-moved">`), or appears twice, or the list names anything else;
- a hand-written count on the page (the tab counts, `count-adapters`, `count-core`, `count-dev`, `count-bio`, the packaged skills note) differs from what the catalogs give;
- **PV14, the page's credits:** a card has no `<p class="item-credit">` right after its description, has two, or says something other than its row's credit line; a shelf or the footer lacks its one curator line, or a shelf's line is not its last element; a credit or curator element sits outside the card, shelf or footer that should hold it; credit wording sits anywhere else on the page, in a shown attribute such as `title` or `alt`, or in the panel text of `landing/baam-details.js` (PV13's phrases, the extension lines' "extension by the Baranzini Lab", the origin sentences and the lead of every generated line); `landing/credits.json` or its app copy differs from what the catalogs generate; or an active row has no entry in `credits.json`;
- **PV15, the extension rows' credits:** a third-party extension names no program it bundles, launches or forwards calls to; an adapted or third-party row names no outside source it copies, bundles or launches; a bundled or launched source lacks a name, a version or commit, a licence, a credit or a `what`; a data or service entry lacks a name, an http(s) address or terms; a row still carries `third_party`, which format 2 folds into `source.upstreams`; or an active row's summary or card text carries an extension line or other credit wording that PV13's shared phrases miss;
- a rule above fails.

`build-registry.mjs` runs the same check on the registry it has just built, before it writes or compares anything, so the five generated outputs can never be regenerated while a catalog disagrees with the page.

| Command | What it adds |
| --- | --- |
| `node landing/scripts/check-catalog.mjs` | The check above. |
| `node landing/scripts/check-catalog.mjs --skills-repo <biorouter-skills checkout>` | Compares the mirror with that checkout's `catalog.json` at `source_commit`, and `landing/scripts/lib/catalog-rules.mjs` with that checkout's `scripts/lib/catalog-rules.mjs` at the same commit. |
| `node landing/scripts/check-catalog.mjs --remote` | The same two comparisons against the copies on GitHub at `source_commit`. The landing deploy runs this. |
| `node --test landing/scripts/check-catalog.test.mjs` | The calibration of D3 against the baseline catalog, and one mutant per rule, PV14 and PV15 included. |
| `node landing/scripts/write-credits.mjs` | Writes the credit lines, the curator lines and the skill cards' `data-version` and `data-license` into `landing/baam.html`. |
| `node landing/scripts/write-credits.mjs --check` | Writes nothing; fails and names the first sites that differ. |
| `node landing/scripts/build-registry.mjs --check` | Writes nothing; fails when any of the five generated files differs from what the page and the catalogs give. |

The Frontend workflow and `just check-registry` run the check and its tests, and `node --test landing/scripts/credits.test.mjs`, which also holds the committed page to what `write-credits.mjs` writes; the landing deploy runs them with `--remote` before it publishes. The Frontend workflow, `just check-shelf` and the landing deploy also run `node --test landing/scripts/baam-credits.browser.test.mjs`, which reads every card's credit line, the curator lines and an opened Provenance and credits row in Chromium.

## Common changes

### Release a new version of an extension

1. Release it in its own repository (tag `vX.Y.Z`, equal to `manifest.version`, with `<id>.brxt`, `<id>-X.Y.Z.brxt` and `SHA256SUMS`). A replacement is a minor version bump.
2. In `landing/baam.html`, change the card's version in `.ext-org` and its download link, and its description if the release changed what it does.
3. In `landing/catalog/extensions.json`, change `version`, `release.tag`, `release.sha256` (the line for `<id>.brxt` in `SHA256SUMS`), `summary` and `keywords` to match the card, and add a `history` entry with its `origin` and `licence`.
4. Bring the row's provenance up to date: in `source.upstreams`, the version or commit of every program it bundles, launches or depends on, with its licence and copyright line; every service or dataset it now reaches in `source.data_and_services`; and for an adapter, `source.built_with`.
5. Update the row's tool list in `landing/baam-details.js`.
6. Run `node landing/scripts/write-credits.mjs`, then `node landing/scripts/build-registry.mjs`, then both with `--check`, and the check above.

### Release a new version of a skill

Release it in biorouter-skills, which updates `catalog.json` and the item's provenance record. Then run one command:

```bash
node landing/scripts/sync-skill-catalog.mjs --skills-repo <checkout> --commit <sha>
```

It copies `catalog.json` and `scripts/lib/catalog-rules.mjs` at that commit (the checkout's `origin/main` when `--commit` is left out; the commit must already be on GitHub, or the deploy cannot check the mirror), then runs `write-credits.mjs` and `build-registry.mjs`, so the card lines, each skill card's `data-version` and `data-license`, the registry and the credits files all follow, and it stops at the first failure. Change the card's description by hand if it changed, then run the checks.

### Add an extension or a skill

Add the card, then the row (for a skill, in biorouter-skills, then sync). Write its provenance record in the same change: for an extension, `source` and `contributors` in its row; for a skill, the same in biorouter-skills, with a `LICENSE-UPSTREAM` for any copied material. [Provenance and credits](provenance.md) has the steps. Then run `node landing/scripts/write-credits.mjs` (for a skill, the sync runs it), which gives the new card its credit line. Give it a job that no active item owns. If the check reports a look-alike, either merge the two or write a `distinct_from` boundary that says what each one does that the other does not. A private extension also needs an entry in `EXPECTED_PRIVATE_EXTENSIONS` in `landing/scripts/build-registry.mjs`, which is security code and needs a person's review.

### Merge or retire an item

1. Cut its final release (a sunset that still works and names its replacement, or a tombstone with one status tool), a minor version bump.
2. Set its row's `status` to `superseded_by:<id>` or `retired` (with `retired.reason` and `retired.use_instead`), and `source.content_moved_to` to where its content went (the target id or a path inside it, or null when nothing took it over). The row keeps the provenance of its last real content. For a merge, add the old id, old display name and old bundled skill names to the target's `aliases`, and say "Replaces <old name>" on the target card (for a skill, add the old id to the target card's `data-tags`).
3. Remove its card and its `baam-details.js` entry, and add one `<li data-moved-id data-moved-kind data-moved-to data-moved-name data-moved-to-name>` line to the Moved and retired list. A search for the old id or name then shows where it went.
4. Never delete its releases: installed copies and older app builds still resolve them.

## Related documentation

- [Provenance and credits](provenance.md)
- [The BRXT adapter kit](brxt-adapter-kit.md)
- [Installing an extension, and where its credentials go](installing-an-extension.md)
- [Skill packages](skill-packages.md)
- [The skill catalog](skill-catalog.md)
- `landing/README.md` and `landing/scripts/build-registry.mjs`
- `README.md` of BaranziniLab/biorouter-skills, sections "The catalog" and "Provenance and credits"
