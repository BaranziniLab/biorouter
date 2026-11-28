# Provenance and credits

> **What this is.** How every BAAM skill and extension records what it borrowed and who made it, how that one record becomes every credit line people read, where each piece lives, and the checks that hold them together.
> **Status:** Current. The rule dates from 2026-11-27, and both BAAM catalogs record it in format 2. Each skill folder's `PROVENANCE.md` and each extension repository's README section arrive with that item's next release; the checks PV16 and PV17 are planned.
> **Audience:** anyone who adds, updates or reviews a BAAM skill or extension, and anyone who brings outside material into one.

The rule: every skill and extension says what it borrowed (text, code, data or ideas), from whom and under which licence, and credits the people behind that work. An item made entirely by the lab says "Made by the Baranzini Lab (Wanjun Gu, UCSF)." Every item says "Curated and organized by Wanjun Gu, Baranzini Lab, UCSF." People can see all of it on BAAM (the Biorouter AI Agent Marketplace, `landing/baam.html`), and the repositories document it.

Five principles follow from the rule:

- **One record per item, written once.** The catalog row is the source of truth. Every line and file a person reads is generated from it, and a `--check` fails when a generated copy differs.
- **Originality first.** The first words a person reads say whose work it is: "Made by", "Adapted from", "From", "Runs" or "Bridges to".
- **Credit is broader than licence.** Ideas, papers, datasets and services are credited even where no licence asks for it. Licence files are owed only where material is redistributed.
- **Never guessed.** A name is credited only as the source itself publishes it: its licence holder, its `CITATION.cff`, its README's author list, the commit authors of the files taken. Where the licence names no holder, the record says so and credits the publisher.
- **Short where it is seen, complete one click away.** One line on a card; the full record in the opened detail panel and in the item's own files.

## The four origins

Every row has one of four origins. It describes the item as it ships now. Ask the questions in order; the first yes decides.

| Question | Origin | Example from today's records |
| --- | --- | --- |
| Is the main content or the main job outside work kept largely as published? | third-party | `superpowers`: "From superpowers by Jesse Vincent (MIT), packaged for Biorouter by the Baranzini Lab." BioMCPAgent: "Runs BioMCP 0.9.3 by Ian Maurer (MIT); extension by the Baranzini Lab." IDCAgent: "Bridges to IDC MCP server by Imaging Data Commons; extension by the Baranzini Lab." |
| Does it contain outside text, code or data, changed for Biorouter? | adapted | `replication-package-audit`: "Adapted from medsci-skills by Aperivue (MIT) and 9 more." |
| Was it written here, with no outside text, code or close paraphrase, but does its current design, structure, method or scope follow named outside work? | ideas-derived | `empirical-research-router`: "Made by the Baranzini Lab (Wanjun Gu, UCSF), with ideas from Auto-Empirical Research Skills (Bryce Wang, CoPaper.AI)." |
| None of the above. | in-house | CDWAgent and `scientific-research`: "Made by the Baranzini Lab (Wanjun Gu, UCSF)." |

How each question is answered:

- **Third-party, for a skill:** at least 90% of the item's lines that derive from a source are unchanged, and lines written for Biorouter are under 10% of the item, by a body diff against the source at its pinned commit (front matter removed, whitespace collapsed). A person confirms every row the diff classifies. `superpowers` keeps 6,454 of the 6,504 lines it takes from its source, and Biorouter wrote 41 more.
- **Third-party, for an extension:** an outside program does the main job and its tool list is the program's own; the lab's part is the launcher, the policy and the packaging. Its line reads "Runs" when the extension bundles or launches the program, and "Bridges to" when the program is a service its publisher hosts and the extension forwards calls to it. A program the package manager installs and the extension then runs is recorded as both `dependency` and `launched`, as BioMCPAgent's is; `dependency` alone never leads a line.
- **Adapted:** a close paraphrase counts as text. Open Computer Use Agent is adapted because it ships Open Computer Use changed by local patches.
- **What never changes the origin:** libraries an item depends on, services it calls, data it reads and tools or methods it teaches are credited in the record and leave the origin alone. A source the lab owns (`owner: "lab"`) is not outside work, so taking from it never moves an item out of in-house: Broccolito/gptr behind GptrAgent and `high-performance-r`, and BaranziniLab/UCSFHPCAgent behind `ucsf-hpc`.
- **Earlier versions:** a source that shaped only an earlier version is recorded with `applies_to: "earlier"` and shown under "Earlier versions". It never changes today's origin. IDCAgent and MarkItDownAgent carry such an entry.

## The record

The row's `source` and `contributors` hold the record, in the same shape in both catalogs. A catalog in this shape says `"format": 2` at its top. The fields:

| Field | What it holds |
| --- | --- |
| `source.origin` | `in-house`, `ideas-derived`, `adapted` or `third-party`. |
| `source.upstreams[]` | Every source whose material the item contains or runs: `kind` (`repository`, `package`, `program`, `guideline`, `paper`), `repo` or `url` (or a bare `doi` for a publication), `display`, `path`, `commit` or `version`, `licence`, `copyright` (the verbatim line from the source's licence, or null), `copyright_note`, `credit`, `people`, `use`, `what`, `files`, `via`, `upstream_notice`, `owner` (`external` or `lab`), `taken` and `cite`. |
| `source.ideas_from[]` | Outside work the item follows without copying it: `kind`, `name`, `url` or `cite`, `credit`, `people`, `what` (what was followed), `evidence` (how the absence of copied text was established, such as a count of shared six-word runs) and `applies_to` (`current` or `earlier`). Only `current` entries make an item ideas-derived. |
| `source.data_and_services[]` | Data the item reads and services it calls: `kind` (`dataset` or `service`), `name`, `url`, `terms`, `credit`, `cite` where the provider asks for one, and `how` it is used. BioMCPAgent lists eleven. |
| `source.references[]` | Optional: tools and methods a skill teaches, or the lab's own work an item is designed around, each with a `name` and a `url` or `cite`. It credits and never changes the origin. CDWAgent lists MedCP and UCSFOMOPAgent here. |
| `source.refused_sources[]` | Sources considered and left out, with a `repo` or `url` and the `reason`. They stay in the catalog and never reach the page. |
| `source.changes` | What Biorouter changed. Required for adapted and third-party rows ("Packaging only" when that is all). |
| `source.content_moved_to` | On a merged or retired row only: where its content lives now (an item id, `builtin:<name>`, a `<bundle>/<folder>` path, or null). The row keeps the provenance of its last real content. |
| `source.built_with` | Extensions only: the kit or template a release was built with, such as `brxt-adapter-kit 1.0.1`. |
| `contributors[]` | The people who made the Biorouter side, each `{ name, affiliation, role }`. Every row has at least one. Co-authors come from the item's own README or `pyproject.toml`; CDWAgent names Gianmarco Bellucci and Wanjun Gu. |
| `components` | Bundles only: a component may carry its own `origin`, `licence` and `sources` (indexes into the row's upstreams), and `written_for_biorouter` for files written here inside a copied component. |
| `history[]` | Each entry may add `origin` and `licence`, so a row records that a version had another origin. |

`use` says how a source reaches the item. It decides which sources set the origin, which owe licence files, and which source a line leads with:

| `use` | Meaning | What follows |
| --- | --- | --- |
| `text`, `code`, `data` | Copied into the item. | Sets the origin (adapted or third-party); licence text and copyright line ship with the item. |
| `bundled` | A program shipped unchanged in the bundle. | Licence text ships; leads a "Runs" line. |
| `bundled-modified` | A program or its source shipped changed. | Sets the origin; licence text ships; leads a "Runs" line. |
| `dependency` | Installed by the package manager. | Credited; nothing is redistributed. |
| `launched` | Fetched and run at run time, such as an `npx` package. | Credited; leads a "Runs" line. |
| `hosted` | Run by its publisher on its own servers; the extension forwards calls to it. | Extensions only; leads a "Bridges to" line when nothing else is run. |

A source's own sources are separate upstreams joined by `via` (Open Computer Use contains Cua Driver and yabai), each with its own licence and credit.

## The generated lines

The card line and the sentence a full record opens with come from `creditLine(row)` and `originSentence(row)` in `landing/scripts/lib/catalog-rules.mjs`, a file both repositories share, so the page, the skills repository and the app print the same words.

| Origin | Card line | Origin sentence of the full record |
| --- | --- | --- |
| in-house | Made by the Baranzini Lab (Wanjun Gu, UCSF). | Original work of the Baranzini Lab. |
| ideas-derived | Made by the Baranzini Lab (Wanjun Gu, UCSF), with ideas from {name} ({credit}). | Written by the Baranzini Lab. The sources below shaped its design; none of their text or code is included. |
| adapted | Adapted from {name} by {credit} ({licence}). | Includes text or code from the sources below, changed for Biorouter. Their licences and copyright lines ship with it. |
| third-party skill | From {name} by {credit} ({licence}), packaged for Biorouter by the Baranzini Lab. | The sources below, kept largely as published and packaged for Biorouter. Their licences and copyright lines ship with it. |
| third-party extension | Runs {name} {version} by {credit} ({licence}); extension by the Baranzini Lab. | The program below does the work; the Baranzini Lab wrote the extension around it. |
| third-party extension that forwards to a hosted service | Bridges to {name} by {credit}; extension by the Baranzini Lab. | The service below, run by its publisher, does the work; the Baranzini Lab wrote the extension that forwards calls to it. |

- **The lead source** is the first outside source in record order of the kind the origin names: for an adapted item, one it copies from; for a third-party item, one it is made of, runs or forwards calls to; for an ideas-derived item, its first current idea. For an extension, a program it runs comes before one it forwards calls to. Entries that name one repository count as one source, with all of its licences.
- **More than one source:** the line names the lead source and adds "and N more" ("Adapted from medsci-skills by Aperivue (MIT) and 9 more.").
- **At most 120 characters** (`CREDIT_LINE_MAX`). When a line passes it, the "and N more" tail is dropped first; a line still too long fails PV13, and the fix is a shorter `display` for the source. The longest line today is 117 characters.
- **Co-authors** of an in-house item are not on the card; the fixed line stays, and the full record's People line names them.
- **The ideas-derived credit** in parentheses appears only when it differs from the source's name.

The curator line, "Curated and organized by Wanjun Gu, Baranzini Lab, UCSF." (`CURATED_BY`), is never on a card. It appears once at the end of each BAAM shelf (Extensions, Workflows and Skills), once in the BAAM footer, in every opened extension or skill panel, at the end of every full record, and, from each item's next release, in its skills' front matter. In the app, the marketplace rows will show it beside the card line once the app reads `credits.json`.

A copyright line for the lab's own work reads `Copyright <year> Baranzini Lab, University of California, San Francisco` (`labCopyrightLine`). PV13 flags a curator line, a "Made by the Baranzini Lab" line or a lab copyright line written any other way in a skill's files.

## Where each piece lives

| Piece | Skills | Extensions |
| --- | --- | --- |
| Source of truth | The row in `catalog.json` of BaranziniLab/biorouter-skills, mirrored byte for byte into `landing/catalog/skills.json` | The row in `landing/catalog/extensions.json` |
| Published for the page and the app | `landing/credits.json` and its app copy `ui/desktop/src/components/baam/credits.fallback.json` | the same two files |
| Card line | `<p class="item-credit">`, the next element after the card's description in `landing/baam.html`, so it shows without JavaScript and in the page source | the same, on every extension card |
| Shelf and footer lines | `<p class="shelf-curator">`, the last element of each shelf, and `<span class="foot-curator">` in the footer's brand column | the same |
| Panel row | "Provenance and credits", in every opened skill panel | the same, in every opened extension panel |
| Full record | `PROVENANCE.md` in the item's folder, generated by `scripts/render-readme.mjs`; `code-simplifier` and `playground` carry one, and the others arrive with each item's next release. Until then, the "Provenance and credits" section of the biorouter-skills README. | A generated `## Provenance and credits` section in the extension repository's README, which arrives with each repository's next update |
| Licence texts | `LICENSE-UPSTREAM`, the row's `notice_file`: a per-file list, then each source's copyright line, notice paragraph and full licence text | `NOTICE` in the repository, naming every bundled, modified, launched or dependency component with its copyright and licence; it arrives complete with each repository's next update |
| Biorouter's own licence | The archive builder adds the repository's `LICENSE` as `<id>/LICENSE`, from each item's next release | `LICENSE` in the repository |
| Machine-readable copy | `metadata.origin`, `metadata.credit` and `metadata.curated_by` in every `SKILL.md`, written by `render-readme.mjs` from each item's next release | none yet |

`landing/registry.json` does not change: every daemon since v1.90.0 parses registry v2 with `deny_unknown_fields` (`crates/biorouter/src/marketplace.rs`), so a new field would make it fall back to a stale copy. Credits travel beside it.

### The credits file

`landing/credits.json` (version 1) has one entry for every row of both catalogs, active or not, keyed by the registry id. `build-registry.mjs` writes it and its app copy, byte for byte the same, in the same locked step as `registry.json`, and its `--check` compares both. The contract is `landing/scripts/lib/credits.mjs`. Nothing in the app reads the copy yet.

```json
{
  "version": 1,
  "source": "https://biorouter.ucsf.edu/baam",
  "curated_by": "Curated and organized by Wanjun Gu, Baranzini Lab, UCSF.",
  "made_by_lab": "Made by the Baranzini Lab (Wanjun Gu, UCSF).",
  "skills_commit": "<the mirror's source_commit>",
  "extensions": { "<id>": "<entry>" },
  "skills": { "<id>": "<entry>" }
}
```

An entry holds `status`, `origin`, `line` (the card line), `summary` (the origin sentence), `licence`, `sources`, `ideas` (the current ones), `data_and_services`, `people`, `written_for_biorouter` (a bundle's components written here), `earlier` (versions with another origin, and ideas that shaped only an earlier version), `moved_to`, `licence_files` and `record`.

- `licence_files` links a skill's `LICENSE-UPSTREAM` at the mirrored commit (or the skills repository's `LICENSE` when the row has no notice file), and an extension's `NOTICE` at its release tag.
- `record` links the skills README section `#provenance-and-credits` at the mirrored commit. It is null for extensions until their repositories carry the generated section.
- Every `url` in an entry is an http(s) address or null. A reader allows unknown fields and checks `version`.

### The panel row

`landing/baam.js` builds the "Provenance and credits" row of an opened panel from `credits.json`, with text nodes only: no value from a record ever passes through `innerHTML`, and a link is made only from an http(s) address. The row holds the origin sentence; up to three sources (or ideas), each with its name, version or short commit, licence, what was used and the credit, behind "Show all N sources" when there are more; what it builds on or the ideas it follows; a bundle's components written for Biorouter; the data and services, and the earlier versions, each folded; the People line; the curator line; and the links "Licence files" and, for skills, "Full record". When `credits.json` fails to load, the row shows the card's line and the curator line.

Search does not read credit text: a search for a source's name finds no card that names it only in its credit line.

## Licence files by licence

| How the source is used | Licence | What ships with the item |
| --- | --- | --- |
| text, code, data, bundled, bundled-modified | MIT, ISC, BSD-2-Clause, BlueOak-1.0.0 | The verbatim copyright line and the licence text. |
| the same | BSD-3-Clause | As above, plus the non-endorsement sentence. |
| the same | Apache-2.0 | The licence text; the source's `NOTICE` verbatim when it has one (the row sets `upstream_notice`); a change notice in changed files. |
| the same | CC-BY-4.0 | The title, the author, the licence link and a statement of changes. |
| the same | CC0-1.0 | Nothing is required; it is credited anyway. |
| dependency, launched, hosted | any | Credit in the record; no licence text, since nothing is redistributed. |
| ideas only | any, including CC BY-SA, non-commercial and AGPL | Credit in `ideas_from`; no licence file, and no text, code, table or close paraphrase. |

Material that is copied or shipped may carry only MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, BlueOak-1.0.0, CC0-1.0 or CC-BY-4.0 (`ALLOWED_LICENCES`).

## The checks

The rules that read only rows are in `landing/scripts/lib/catalog-rules.mjs`, a verbatim copy of biorouter-skills `scripts/lib/catalog-rules.mjs`. The rules that read a skill's folder are in biorouter-skills `scripts/check-catalog.mjs`. The page and extension rules are in `landing/scripts/check-catalog.mjs`, which runs the row rules over the extension rows (biorouter-skills runs them over the skill rows).

| Rule | Where | What fails |
| --- | --- | --- |
| PV1 | rows | A row, active or not, without one of the four origins or the format 2 fields; a moved or retired row without `content_moved_to`. |
| PV2 | rows | An in-house row with an outside source it copies, or a current `ideas_from`. |
| PV3 | rows | An ideas-derived row without a current `ideas_from`, or with an outside source it copies. |
| PV4 | rows | An adapted row without an outside source it copies, or with an empty `changes`. |
| PV5 | rows | A third-party row without an outside source it copies, bundles, launches or forwards to, or with an empty `changes`. |
| PV6 | rows | A source without a `repo` or `url`, a `credit` and a `what`; a copied or bundled source without a commit or version, an allowed licence, and a copyright line or a `copyright_note`; an idea, data entry, reference or refused source that is incomplete. |
| PV7 | skill folders | A notice file missing a copied source's copyright line, a licence's full text, the BSD sentence, or an Apache source's `NOTICE`. |
| PV8 | rows and skill folders | A row licence that omits a copied source's licence, or Apache-2.0 for the lab's text; a component or front matter licence that differs from its own. |
| PV9 | skill folders | A folder whose `PROVENANCE.md`, or a moved row's stub, differs from the generator's. |
| PV10 | skill folders | A `SKILL.md` whose `metadata.origin`, `metadata.credit` or `metadata.curated_by` differs from the generated value. |
| PV11 | rows and skill folders | A bundle component with no lineage; a file written for Biorouter that its host does not list, or that is not in the folder. |
| PV12 | rows | A row with no contributor, or a contributor without a name or a role. |
| PV13 | rows and skill folders | A line over 120 characters; card text that carries credit; process wording in shown text; a fixed text written differently from its constant. |
| PV14 | the page | `landing/credits.json` or its app copy differing from the generator's output; an active row with no entry; a card whose credit line is missing, doubled, misplaced or different; a shelf or the footer without its one curator line; credit wording written by hand anywhere else on the page, read as a browser shows it (character references and no-break spaces included). |
| PV15 | extension rows | A third-party extension with no outside program it bundles, launches or forwards calls to, which its "Runs" or "Bridges to" line must name; an adapted or third-party row with no outside source it copies, bundles or launches; a bundled or launched source without a name, a version or commit, a licence, a credit and a `what`; a data or service entry without a name, an http(s) address and terms; a row that still carries `third_party`. |
| PV16 | planned | An extension repository README without the section, the generated line and the curator line; a `NOTICE` that omits a component. |
| PV17 | planned | An adapter bundle whose README section or `NOTICE` differs from its spec. |

Each rule in force has a mutant test that fails it. In this repository, run from the root after `source bin/activate-hermit`:

```bash
node landing/scripts/write-credits.mjs --check
node landing/scripts/build-registry.mjs --check
node landing/scripts/check-catalog.mjs
node landing/scripts/check-catalog.mjs --skills-repo <biorouter-skills checkout>
node --test landing/scripts/check-catalog.test.mjs
node --test landing/scripts/credits.test.mjs
node --test landing/scripts/build-registry.test.mjs
node --test landing/scripts/baam-credits.browser.test.mjs
```

The browser test needs `ui/desktop/node_modules` (`cd ui/desktop && npm ci`) and Playwright's Chromium. In biorouter-skills:

```bash
node scripts/check-catalog.mjs
node scripts/render-readme.mjs --check
node scripts/write-stubs.mjs --check
node --test scripts/
```

A skill row whose folder files wait for its next release is listed in biorouter-skills `provenance-baseline.json`. Only rows that existed when the rules landed may be listed there; a row added later passes every rule.

## Adding or updating an item

### A skill

1. In biorouter-skills, write the row's `source` and `contributors` in `catalog.json` (`README.md` there, "Contributing a skill", has the steps): the origin by the four questions; each source with its `use`, `what`, `files`, commit, licence, credit and people; each idea with `what` and `evidence`; `changes`; and a role for each contributor.
2. For copied material, add `LICENSE-UPSTREAM` to the folder, as the table above asks, and name it in `notice_file`.
3. Run `node scripts/render-readme.mjs`, which writes the README tables, `NOTICE`, the item's `PROVENANCE.md` and its `SKILL.md` credit keys. Run the checks, then release the item.
4. In this repository, run the one command, with a commit that is already on GitHub. A new skill needs its card in `landing/baam.html` first, because the registry check refuses an active row with no card:

   ```bash
   node landing/scripts/sync-skill-catalog.mjs --skills-repo <biorouter-skills checkout> --commit <sha>
   ```

   It refuses a catalog that is not in format 2. It writes the mirror, copies `catalog-rules.mjs` from the same commit, then runs `write-credits.mjs` (every card line, the curator lines, and each skill card's `data-version` and `data-license`) and `build-registry.mjs` (the registry and the credits files), and stops at the first failure.
5. Run the checks above and commit every file it wrote.

### An extension

1. With its card in `landing/baam.html`, edit its row in `landing/catalog/extensions.json`: the origin by the four questions; each source with its `use`, version or commit, licence, copyright, credit, people and `what`; every service it calls and dataset it reads, with terms and a citation where one is asked for; its contributors; and for an adapter, `built_with`.
2. Run `node landing/scripts/write-credits.mjs`, which writes its card line.
3. Run `node landing/scripts/build-registry.mjs`, which writes the registry and the credits files.
4. Run the checks above.
5. In the extension's own repository, keep `NOTICE` naming every component it bundles, modifies, launches or depends on, with its copyright and licence. The generated README section follows at the repository's next update.

[The BAAM catalog](baam-catalog.md) has the release steps for both.

## Bringing in outside material

Every outside source is adapted, used for ideas only, or refused, and the record says which.

- **Adapt.** Copy only under an allowed licence, from the source itself at a pinned commit, never from someone else's snapshot of it. Record the upstream with `use`, `what`, `files`, the commit, the credit and the people as the source publishes them, and ship the licence files. The item becomes adapted, or third-party when the body diff says so. Copying text into an in-house or ideas-derived item changes its origin in the same change; PV2 and PV3 fail otherwise.
- **Ideas only.** Any licence is fine, including CC BY-SA, non-commercial and AGPL, because nothing is copied: no text, code, table or close paraphrase. Record it in `ideas_from` with `what` (what was followed), `evidence` (how you established that no text was taken) and `applies_to: "current"`. The item becomes ideas-derived.
- **Refuse.** A source considered and left out, such as a vendor's copy of another project, non-commercial material or AGPL code, is recorded in `source.refused_sources` with its `repo` or `url` and the reason. It stays in the catalog and never reaches the page.
- **Sources of sources.** When a source adapts another one, record both, joined by `via`, each with its own licence and credit, rather than describing the chain in prose.

## Credits and install provenance

Biorouter uses the word provenance for two different things, so the code keeps them apart.

| Aspect | Credits (this page) | Install provenance |
| --- | --- | --- |
| The question | Whose work is in this item? | Which registry entry did this installed extension come from? |
| Where it lives | The catalog rows, `landing/credits.json` and its app copy | `extension-provenance.json` in the person's config folder, written when a `.brxt` is installed |
| What it holds | Origins, sources, licences, people, the generated lines | The registry id, the download URL, the bundle's SHA-256 and the time it was recorded, keyed by the config entry's name |
| Who reads it | BAAM's cards and detail panels; the app's marketplace views once they read the app copy | The privacy tier: `classify_extension` (`crates/biorouter/src/privacy/extensions.rs`) re-derives an installed extension's tier and affiliation from the registry by the id the store records (`crates/biorouter/src/privacy/provenance.rs`) |

People read "Provenance and credits". Code, routes and the published file say `credits`, so a credits change never reaches the privacy code and a privacy change never reaches the credit lines.

## Related documentation

- [The BAAM catalog](baam-catalog.md): the catalog files, every check that compares them with the page, and the release, add, merge and retire steps.
- [The BRXT adapter kit](brxt-adapter-kit.md): how an adapter's record relates to the kit it is built with.
- [Installing an extension, and where its credentials go](installing-an-extension.md): how a `.brxt` install works on each surface.
- [Privacy tiers](../security/privacy-tiers.md): the install provenance record and the tier it re-derives.
- `README.md` of BaranziniLab/biorouter-skills, sections "The catalog", "Contributing a skill" and "Provenance and credits".
