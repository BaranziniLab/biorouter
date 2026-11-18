# Skill packages

> **What this is.** How BioRouter imports a skill, or a coordinated package of skills, from a repository URL, a `.zip`, the marketplace, the CLI or an agent — and why all five go through one pipeline. Reference for the importer introduced by issue #115.
> **Status:** Current. The importer, the `/skills/packages/*` routes, the `importSkillPackage` tool, the Add Skill URL field and the CLI's `skill install` are shipped.
> **Audience:** contributors, and anyone whose import came out wrong.

## The failure this replaces

Given a repository URL such as `https://github.com/heygen-com/hyperframes`,
BioRouter had no repository-aware installer. An agent asked to "install the
skills from this repo" improvised with shell and file copies, and produced one
unrelated top-level skill per `SKILL.md`.

For a coordinated package that discards **behaviour**, not merely grouping:

- the entry-point relationship — HyperFrames' `hyperframes` skill is a mandatory
  router that is read first and installs the workflows on demand;
- the core-versus-on-demand distinction its manifest declares;
- the package's identity, so it cannot be updated or removed as one thing;
- twenty separate rows in the interface where there should be one.

The ZIP path was no better. The daemon and the CLI each recognised three shapes
— `SKILL.md`, `<skill>/SKILL.md`, `<bundle>/<skill>/SKILL.md` — by counting
slashes, and read no manifest at all. A normal GitHub source archive is
`<repo>-<ref>/skills/<name>/SKILL.md`, which matches none of them.

## One pipeline, five callers

`crates/biorouter/src/agents/skill_package/`:

| File | What it does |
|---|---|
| `source.rs` | A repository URL, a direct archive URL, or a local `.zip` → archive bytes. Holds the host allowlist. |
| `archive.rs` | Read the zip safely, inflating only the files a plan reads; strip the wrapper directory a source archive carries. |
| `manifest.rs` | The detection ladder. |
| `plan.rs` | Structure → a reviewable `ImportPlan`. |
| `install.rs` | Atomic install, the package record, removal. |
| `pending.rs` | Plans waiting for an answer. |

Everything that installs a skill calls it:

- **Add skill** (`AddSkillModal.tsx`, opened from Skills → Add → From a repository or .zip…) — a URL field and a one-line drop zone;
- **the marketplace** (`baam/installSkill.ts`);
- **the agent**, through the `importSkillPackage` MCP tool;
- **the CLI**, `biorouter skill install <path-or-url>`;
- **HTTP**, `POST /skills/packages/{preview,install,remove}`.

The two parallel archive parsers those surfaces used to share between them — in
`routes/shell.rs` and in `commands/skill.rs` — were **deleted**, not kept in
step. So was the `/headless/skills/extract-zip` route and its IPC channel.

## The detection ladder

Explicit metadata beats shape, in this order:

1. `biorouter-package.json` — the record this importer writes, so an installed
   package round-trips;
2. `.codex-plugin/plugin.json` — its `skills` path;
3. `.claude-plugin/plugin.json`;
4. `skills-manifest.json`;
5. structural inference.

⚠ **The ladder merges; it does not pick one winner.** A real repository carries
complementary files rather than competing ones — HyperFrames' plugin file
supplies the package name, version and skills path, while its skills manifest
supplies the router and the groups. Reading only the higher rung dropped the
groups on the floor, and the package installed with no core/on-demand
distinction at all: present, but silently poorer than its source. Priority is
still absolute per field, and the reported `evidence` is the highest rung that
contributed.

⚠ **A malformed manifest is an error, not "no manifest".** Falling through would
install a package that declares an identity under an inferred one.

### A shared name prefix is never the signal

Several valid HyperFrames members — `media-use`, `slideshow`,
`product-launch-video`, `faceless-explainer` — do not begin with
`hyperframes-`. Detection uses structure and manifests, and **every component
keeps its declared name exactly**: the frontmatter `name` is the identifier every
enablement surface keys on, so a prefix would be both a lie about the
component's identity and useless as a detector.

## Stripping the wrapper directory

GitHub source archives wrap everything in one directory. Two rules, in
`archive.rs`:

- **A download from a code host** passes `WrapperHint::SourceArchive`: the single
  common root *is* the wrapper, whatever it is called. Knowing this rather than
  predicting the name matters — codeload's directory is `<repo>-<branch>` with
  `/` folded to `-`, and for a tag it also drops a leading `v`, so
  `refs/tags/v1.0` unpacks into `repo-1.0`.
- **Anything else** (`WrapperHint::Infer`) strips only when stripping *reveals*
  package structure: a manifest, a `skills/` directory, or a root `SKILL.md`.
  That is what keeps a genuine bundle archive — `pack/alpha/SKILL.md`,
  `pack/beta/SKILL.md` — from being unwrapped into two unrelated skills.

## How large a source may be

The limit is 256 MB (`MAX_ARCHIVE_BYTES` in `archive.rs`), and it counts two
things: the archive as it downloads, and the files read out of it. Only the
files a plan can use are read, and only they count:

- every file in a folder that holds a `SKILL.md`, which is a skill and its own
  `references/`, `scripts/` and assets;
- the shared `assets/`, `references/` and `scripts/` folders at the root or
  beside the skills, which a bundle keeps;
- the four manifests the detection ladder reads at the root.

Everything else in a repository is never inflated: its docs, its registry, its
build output, a zip bomb in a folder no skill lives in. The names are read from
the archive's central directory first, and the wrapper is decided on all of
them, so leaving the rest out changes no plan. The fixture matrix runs every
shape through both readers and checks the plans are the same.

HyperFrames' default branch is the case this is for. On 2026-11-17 it was a
146 MB download that expands to 327 MB, mostly `docs/`, `registry/` and
`packages/`; its skill folders hold about 14 MB. Inflating every file refused
the import with a raw byte count. Reading only the skills, it imports.

When a source is still too large, the refusal says so in one sentence and
names two ways on: import a release tag (`…/tree/<tag>`), which is usually a
smaller archive, or download the repository and drop a `.zip` of only its
skills folder into Add skill. A `…/tree/<branch>/skills` URL does not help:
GitHub serves a whole repository as one archive, and the importer reads
`main/skills` as the name of a ref, which does not exist.

## Where components live

A manifest's `skills` path wins. Otherwise the components root is the directory
the `SKILL.md` folders **share as their parent** — `pack` for a bundle archive,
`skills` for a plugin repository, `""` for skills loose at the archive root.

Components are found exactly one level below that root, never at any depth: a
skill's own `references/` or `scripts/` folder may itself hold a `SKILL.md`, and
a deeper match would promote a support file to a component.

## Ambiguity is a question, not a default

The plan carries an `ambiguity` when — and only when — **no manifest spoke and
the components sit loose at the archive root**. That shape is equally one
package and a folder somebody zipped.

A named parent directory is *not* ambiguous: whoever put `alpha` and `beta`
inside `superpowers/` already said they belong together, and that includes
`skills/`. Neither is anything a manifest defined, which is why an explicit
manifest installs as a bundle **without making the user approve all twenty
children**.

Answering it:

| Surface | How |
|---|---|
| Add skill | "Install as one bundle" / "Install separately" |
| HTTP | a `200` with `status: "needsChoice"` and a `planId`; POST again with `choice` |
| The agent | `status: "needsChoice"` plus instructions to ask the user — it must not choose for them |
| CLI | the question is printed and the command stops; `--as bundle` or `--as individual` answers it |

⚠ **A `needsChoice` is a 200, not a 4xx.** It is a legitimate outcome the caller
is expected to act on, and an agent that saw an error would reasonably retry the
same call rather than asking the question it was handed.

⚠ **The answer installs the archive that was previewed.** The parked plan holds
the resolved bytes: a branch moves, and a preview saying "20 skills, entry point
`hyperframes`" followed by an install of something else makes the preview a
decoration. The store is bounded (15 minutes, 8 plans), because a plan can hold
up to 256 MB of skill files and a forgotten preview must not pin one.

## What lands on disk

```
~/.config/biorouter/skills/hyperframes/
  biorouter-package.json      ← the record the catalog reads
  skills/hyperframes/SKILL.md ← the router; its own name, not a prefix
  skills/media-use/SKILL.md
  skills/slideshow/SKILL.md
  …
```

A components folder a manifest declares (`skills/` here) is kept, because
members and the package's shared assets refer to one another by relative path.
The record lists each component's directory, and the catalog finds the members
through it (`package_component_skill_files`). A folder inferred from structure
alone is the package folder itself, so a plain `pack/alpha/SKILL.md` bundle
installs as `pack/alpha/SKILL.md`, not `pack/pack/alpha`.

A **single** skill installs at `~/.config/biorouter/skills/<slug>/SKILL.md` with
no record. One skill in one folder is not a package of one, and installing it as
a bundle would put it a level deeper than every other single-skill install.

The record carries the package id and display name, its version, the source URL
/ ref / resolved commit, the installer, the timestamp, the entry point, the
groups and the component list. `skill_catalog` reads it into
`CatalogBundle.package`, which is what lets the picker show one expandable row
named "HyperFrames" over one line, "5 skills · 0.8.12", which opens to show the
entry point and each component with its group.

## Atomicity

A package is many files across many directories, and writing them in place means
a failure halfway through leaves a directory that *looks* installed, is missing
components, and shadows whatever was there before.

So: the package is written to a sibling staging directory, **verified by reading
the tree back** (every declared component must have a `SKILL.md` on disk whose
frontmatter name matches), and swapped in with two renames within one directory
— which is where `rename` is atomic. A failure before the swap leaves the
previous install untouched; one between the renames puts it back. Removal
renames aside first, so the directory leaves the catalog's view in one step
rather than emptying out under a scan in flight.

A duplicate component name is **fatal**, not a warning: a skill's identity is its
frontmatter `name`, so of two components declaring `same-name` one would
permanently shadow the other, silently.

## Where a package may come from

`ALLOWED_HOSTS` in `source.rs` — `biorouter.ucsf.edu`, `github.com`,
`objects.githubusercontent.com`, `raw.githubusercontent.com`,
`codeload.github.com` — over `https` only. The same list the marketplace
download path uses, so a package source and a marketplace asset are held to one
rule. A URL is an instruction to fetch *and then write what comes back*, which
is why this is an allowlist rather than a scheme check.

## Tests

```bash
cargo test -p biorouter --lib -- skill_package
cargo test -p biorouter-server --lib -- routes::skills
cd ui/desktop && npx vitest run src/components/skills src/components/baam/installSkill.test.ts
```

The Rust fixtures build real ZIPs and run the real pipeline end to end, because
every defect this replaces lived in a seam between two steps — a test that hands
`plan_from_entries` a hand-built entry list jumps straight over the wrapper
directory, which is the one that mattered most.

⚠ **Run the live test after touching `archive.rs`, `manifest.rs` or `plan.rs`:**

```bash
cargo test -p biorouter --test skill_package_live -- --ignored
```

It downloads the real HyperFrames repository (about 146 MB), so it is
`#[ignore]`d, and it is the only test that can fail for the reason the fixtures
cannot. A fixture is a
statement about what you *think* the world looks like, and a complete synthetic
matrix passed while **three** real defects survived, all three found in one live
run:

1. `skills-manifest.json`'s `skills` is a **map** keyed by component name, not
   an array. A `Vec` did not merely miss the names — serde failed the whole
   document, and importing HyperFrames was refused with "skills-manifest.json
   is not valid JSON".
2. The human-readable name lives in the plugin manifest's `interface` block,
   not at the top level.
3. Package identity was resolved *before* the single-skill collapse, so one
   skill in a named folder with no manifest was refused with "this package has
   no name" — about a package the user never asked for. Every fixture reached
   that shape through `strip_wrapper`, which unwraps it into the root-`SKILL.md`
   case and jumps over the branch.

A later run failed for a reason no fixture could show: the repository grew past
the size limit with files no skill uses (see "How large a source may be").

## Related documentation

- [The skill catalog](skill-catalog.md) — what happens after an install: discovery, enablement, and per-chat state
- [Skills capability](built-in/skills.md) — the user-facing guide to skills
- [Extensions, skills, and MCP agents](extensions-and-skills-guide.md) — the other things that can be installed
