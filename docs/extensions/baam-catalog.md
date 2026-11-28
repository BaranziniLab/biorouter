# The BAAM catalog

> **What this is.** How the tracking system behind BAAM (the Biorouter AI Agent Marketplace) works: the three catalog files in `landing/catalog/`, the checks that hold them to the page, and how to add, update, merge, retire or rename an extension or a skill.
> **Status:** Current. Added 2026-11-22 with the BAAM refresh: 41 extension rows (28 active) and 144 skill rows (132 active).
> **Audience:** anyone who changes a BAAM card, releases an extension or a skill, or reviews such a change.

## Why there is a catalog

`landing/baam.html` holds the cards, and `landing/registry.json` is generated from them. Neither can say what an item used to be, what replaced it, what job it owns, or where its content came from, and `registry.json` cannot grow a field: every daemon since v1.90.0 parses registry v2 with `deny_unknown_fields` and falls back to a stale copy when it meets one. So every extra fact lives beside the page, in `landing/catalog/`, and a check holds the two together.

## The three files

| File | What it holds |
| --- | --- |
| `landing/catalog/extensions.json` | One row per BAAM extension, active or not: id, display name, repository, group (`ucsf`, `tools` or `adapters`), version, status, aliases, job, licence, privacy and affiliation, the release (tag, asset, SHA-256), bundled skills, third-party components, boundaries with look-alikes and a history. Retired rows say why and what to use instead. A row whose id Biorouter renamed locally records the new id in `renamed_locally_to`. |
| `landing/catalog/skills.json` | A mirror of `catalog.json` in BaranziniLab/biorouter-skills, byte for byte, with one line added: `source_commit`, the commit it was copied from. Skills are tracked where their content lives, so a pull request there changes a skill and its row together. |
| `landing/catalog/jobs.json` | The job vocabulary, the jobs Biorouter itself owns (built-in extensions and skills), the required jobs with their owner or the round that plans one (the gap list), and the look-alike screen settings. |

A row's `status` is `active`, `retired`, or `superseded_by:<id>` (for a skill, also `superseded_by:builtin:<name>`). An id never moves: a merged or retired item keeps its row, and its id stays reserved.

## The rules

The shared rules are in `landing/scripts/lib/catalog-rules.mjs`, a verbatim copy of `scripts/lib/catalog-rules.mjs` in biorouter-skills. Change both together.

- **D1, one job, one owner.** No two active items share a job, and none claims a job in `jobs.json` `builtin`. A bundled adapter skill's job ends in `.tool-usage` and names the BAAM skill that holds the method.
- **D2, one name.** Ids, display names, aliases and Biorouter's own skill and extension names (read from this repository's source) are one namespace. A retired row may hand its name to the active item that took over its job with `retired.name_passes_to`, as UCSFHPCAgent hands its name to the `ucsf-hpc` skill.
- **D3, look-alikes.** Two active extensions whose text scores at or above `similarity_threshold` (token-set Jaccard over display name, summary and keywords) need a `distinct_from` boundary on one of them. Skills are screened in biorouter-skills, which reads each `SKILL.md`.
- **D4, redirects are complete.** A `superseded_by` target exists, is active and lists the old id, and the old id's bundled skills, in `aliases`. On the page, the target extension's card says "Replaces <old name>", and the target skill's card carries the old id in `data-tags`, so a search for the old name finds it.
- **G1, gaps.** Every required job has an active owner, or a `planned` entry, which prints a warning.

`baseline_violations` in either catalog lists failures that exist on purpose, each owned by a work package. An entry whose work package is in `done_work_packages`, or one that no longer fails, is itself a failure.

## What the check compares with the page

`node landing/scripts/check-catalog.mjs` reads the catalogs, `landing/registry.json` and the parts of `landing/baam.html` the registry does not carry. It fails when:

- a card has no active row, or an active row has no card;
- a card's name, group or category, licence, privacy, affiliation, version or download URL differs from its row (an extension's download is `https://github.com/<repository>/releases/download/<release.tag>/<release.asset>`);
- an extension row's `summary` or `keywords` differ from its card's description and tags;
- a skill card's `data-version` differs from its row's version;
- a superseded or retired row is missing from the Moved and retired list (`<details id="baam-moved">`), or appears twice, or the list names anything else;
- a hand-written count on the page (the tab counts, `count-adapters`, `count-core`, `count-dev`, `count-bio`, the packaged skills note) differs from what the catalogs give;
- a row whose id or `manifest_name` Biorouter's rename map (`crates/biorouter/src/config/extension_renames.json`) renames does not record the new id in `renamed_locally_to` and in `aliases`, or a row records a rename the map does not hold;
- a rule above fails.

`build-registry.mjs` runs the same check on the registry it has just built, before it writes or compares anything, so the three generated outputs can never be regenerated while a catalog disagrees with the page.

| Command | What it adds |
| --- | --- |
| `node landing/scripts/check-catalog.mjs` | The check above. |
| `node landing/scripts/check-catalog.mjs --skills-repo <biorouter-skills checkout>` | Compares the mirror with that checkout's `catalog.json` at `source_commit`. |
| `node landing/scripts/check-catalog.mjs --remote` | Compares the mirror with the copy on GitHub at `source_commit`. The landing deploy runs this. |
| `node --test landing/scripts/check-catalog.test.mjs` | The calibration of D3 against the baseline catalog, and one mutant per rule. |

The Frontend workflow and `just check-registry` run the check and its tests; the landing deploy runs them with `--remote` before it publishes.

## Common changes

### Release a new version of an extension

1. Release it in its own repository (tag `vX.Y.Z`, equal to `manifest.version`, with `<id>.brxt`, `<id>-X.Y.Z.brxt` and `SHA256SUMS`). A replacement is a minor version bump.
2. In `landing/baam.html`, change the card's version in `.ext-org` and its download link, and its description if the release changed what it does.
3. In `landing/catalog/extensions.json`, change `version`, `release.tag`, `release.sha256` (the line for `<id>.brxt` in `SHA256SUMS`), `summary` and `keywords` to match the card, and add a `history` entry.
4. Update the row's tool list in `landing/baam-details.js`.
5. Run `node landing/scripts/build-registry.mjs`, then `--check` and the check above.

### Release a new version of a skill

Release it in biorouter-skills, which updates `catalog.json`. Then run `node landing/scripts/sync-skill-catalog.mjs --skills-repo <checkout>` (it copies `catalog.json` at the checkout's `origin/main`), set the card's `data-version` and, if they changed, its licence and description, and regenerate the registry.

### Add an extension or a skill

Add the card, then the row (for a skill, in biorouter-skills, then sync). Give it a job that no active item owns. If the check reports a look-alike, either merge the two or write a `distinct_from` boundary that says what each one does that the other does not. A private extension also needs an entry in `EXPECTED_PRIVATE_EXTENSIONS` in `landing/scripts/build-registry.mjs`, which is security code and needs a person's review.

### Merge or retire an item

1. Cut its final release (a sunset that still works and names its replacement, or a tombstone with one status tool), a minor version bump.
2. Set its row's `status` to `superseded_by:<id>` or `retired` (with `retired.reason` and `retired.use_instead`). For a merge, add the old id, old display name and old bundled skill names to the target's `aliases`, and say "Replaces <old name>" on the target card (for a skill, add the old id to the target card's `data-tags`).
3. Remove its card and its `baam-details.js` entry, and add one `<li data-moved-id data-moved-kind data-moved-to data-moved-name data-moved-to-name>` line to the Moved and retired list. A search for the old id or name then shows where it went.
4. Never delete its releases: installed copies and older app builds still resolve them.

### Rename an extension id

An extension's id is the prefix of its tools, its config key and its install folder, so renaming it is a Biorouter change first and a BAAM change later. [Extension id renames](extension-renames.md) explains the whole mechanism; the steps are:

1. Add the rename to `crates/biorouter/src/config/extension_renames.json` (`from`, `to` and a `note` saying who approved it) and the same pair to `EXTENSION_RENAMES` in `ui/desktop/src/utils/extensionRenames.ts`. Never remove a row, and never chain: an id renamed twice goes straight to its newest id.
2. For a private extension, add the new id to `EXPECTED_PRIVATE_EXTENSIONS` in `landing/scripts/build-registry.mjs` with the affiliation the old id has, and keep the old id. The generator emits the new id from the map. This is security code and needs a person's review.
3. On the row whose id or `manifest_name` is the old id, set `renamed_locally_to` to the new id and add the new id to `aliases`. Change nothing else: the id, status and release stay, and `registry.json`, its desktop fallback and `landing/baam.html` do not change, because a client older than the rename reads the live registry and cannot install a bundle under the new name safely.
4. Run `node landing/scripts/build-registry.mjs --check` and the check above.
5. Move BAAM to the new id in a later pull request, once a release carrying the rename has shipped, the extension's repository publishes a bundle under the new name, and the owner judges older clients rare. For `gptragent` to `gptr`, that pull request and its file list are written down in [The gptr turn-on pull request](extension-renames.md#the-gptr-turn-on-pull-request).

## Related documentation

- [The BRXT adapter kit](brxt-adapter-kit.md)
- [Installing an extension, and where its credentials go](installing-an-extension.md)
- [Extension id renames](extension-renames.md)
- [Skill packages](skill-packages.md)
- [The skill catalog](skill-catalog.md)
- `landing/README.md` and `landing/scripts/build-registry.mjs`
- `README.md` of BaranziniLab/biorouter-skills, section "The catalog"
