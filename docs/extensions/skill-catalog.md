# The skill catalog

> **What this is.** How BioRouter decides which skills exist, which of them are on, and how a per-chat choice differs from a machine-wide one. Reference for the daemon-served catalog introduced by issue #113.
> **Status:** Current. The catalog, the `/skills/*` routes and the composer's per-chat toggles are shipped. Superseded names resolve to their replacement since 2026-11-28 (J-P06).
> **Audience:** contributors, and anyone debugging a skill that is installed but not usable.

## The one-sentence version

There is one catalog, the daemon owns it, and everything — the model's
`listSkills`, the composer's picker, Settings, the CLI — reads that one answer.

## Why it had to become one thing

Before this, three surfaces answered "which skills exist?" independently, and
they disagreed:

| Surface | Roots scanned |
|---|---|
| The agent (`skills_extension.rs`) | seven kinds, including `~/.config/biorouter/extensions/<name>/skills` |
| The desktop picker (`skillUtils.ts`) | three |
| `biorouter skill list` | the agent's, by calling into it |

So a skill bundled inside an installed extension — BiorOffice's Word, Excel and
PowerPoint skills; MarkItDown's converter — was loadable by the model, listed by
the CLI, and had **no row in the interface**. Toggling it was not possible
because there was nothing to toggle.

A second scanner with a different root list is not a bug you fix once. The lists
drift again the next time a root is added. So the renderer no longer scans: it
fetches.

## Where each piece lives

| Concern | Home |
|---|---|
| The root list, with provenance | `crates/biorouter/src/agents/skill_catalog.rs` — `roots()` |
| Discovery and bundle derivation | `SkillCatalog::scan` |
| The enablement rule | `skill_catalog::compose_state`, asked through `skill_catalog::Composition` |
| Superseded names | `crates/biorouter/src/agents/skill_redirects.rs` |
| Machine-wide preference | `~/.config/biorouter/skills-config.json`, `disabled[]` |
| Per-chat deviation | the session row, `extension_data` key `workspace_skills/v1` |
| HTTP surface | `crates/biorouter-server/src/routes/skills.rs` |
| Interface | `ui/desktop/src/components/skills/useSkillCatalog.ts` |

Every interface surface that lists skills reads that one hook: Settings
(`SkillsView`), the composer picker (`BottomMenuSkillSelection`), the
`@`-mention list (`MentionPopover`) and the workflow resource picker. The
renderer's own scanner — `loadSkillsFromDirs`, `ALL_SKILL_DIRS`,
`OTHER_SKILL_DIRS` — is **deleted**, along with `skillUtils`'
`BUILTIN_SKILL_NAMES`: "did Biorouter put this here?" is answered by
`CatalogSkill.builtin`, in the process that owns the seeder.

`SkillsClient::get_default_skill_directories()` still exists, and the CLI still
calls it, but it is now the paths-only view of `roots()`. **Adding a root means
editing `roots()` and nothing else.**

## The five roots, in override order

Later wins, so a skill of the same frontmatter `name` further down shadows one
above it.

1. `~/.claude/skills`
2. `~/.config/agents/skills`
3. `~/.config/biorouter/skills`
4. `~/.config/biorouter/extensions/<extension>/skills`, one root per installed
   extension, sorted so two extensions shipping the same skill name resolve the
   same way on every start
5. the working directory's `.claude/skills`, `.biorouter/skills`, `.agents/skills`

A skill's identity is the `name:` in its frontmatter, never its directory name.

## Two scopes, and why they must not be confused

**Machine-wide** is `skills-config.json`. It is shared with
`biorouter skill enable/disable` and with every other window, so writing to it
from a chat would change every other conversation.

**Per-chat** is `workspace_skills/v1` on the session row: an `add` list and a
`remove` list, each holding skill names or bundle names. It is a *deviation*
from the machine-wide answer, not a copy of it, so a machine-wide change still
reaches a chat that has no opinion about that skill.

Precedence, most specific first. One function, `compose_state`, holds it, and
both the model's filter and the interface's switch read it:

1. the session `add` list names the skill
2. the session `remove` list names the skill
3. the session `add` list names the skill's **bundle**
4. the session `remove` list names the skill's bundle
5. `skills-config.json` names the skill or its bundle
6. a Context switched off in Settings → Contexts is off. ⚠ A Context id may
   name a **bundle**: `knowledge-bases` covers the four knowledge-format skills
   plus `update-soul`, so this step tests a skill's bundle as well as its own
   name, exactly as step 5 does

Steps 5 and 6 are the machine-wide answer, and they decide only a chat with no
opinion of its own. So a Context follows the same rule as every other skill: a
per-chat `add` turns on a Context switched off in Settings, in that chat only,
and a per-chat `remove` turns off one that Settings has on (owner ruling,
2026-11-13). The composer's Skills list offers these switches under **Built-in
contexts**. Before that ruling the Settings switch was tested first, so a
per-chat grant could not put a Context back, and the per-chat switch moved
without changing anything. A Context switched off in Settings stays loadable by
exact name either way (see [built-in/skills.md](built-in/skills.md)); one a chat
switched off is refused to that chat's `loadSkill`, like any skill a chat
switched off.

⚠ Steps 3 and 4 are why a per-chat bundle toggle persists the **bundle's** name
rather than expanding its members at click time: a bundle that later gains a
skill stays covered, where an expanded list would silently stop covering it.

## Superseded and retired names

The BAAM catalog never reuses a name. When a skill or a bundle component is
merged into another, its row keeps the old name with the status
`superseded_by:<target>`, and its folder holds a generated **redirect stub**: a
`SKILL.md` that keeps the old `name:` and carries the status in its frontmatter.
A retired skill's folder holds the same kind of stub with `status: "retired"`.

```yaml
name: taste-skill
description: "Moved: this skill is now part of Frontend Design (frontend-design). Use frontend-design instead."
metadata:
  version: "1.1.0"
  status: "superseded_by:frontend-design"
```

The target is a skill or bundle id (`frontend-design`, `systems-biology`), a
component as `<bundle>/<component name>` (`data-visualization/data-visualization-genome-tracks`),
or a shipped skill as `builtin:<name>`. A target outside that id grammar (word
characters, `-`, `/`, `:` and `.`, at most 200 characters) makes the file an
ordinary skill, so no package can put prose into a notice. A plain id that names
both a bundle and a member of the same name means the bundle. That frontmatter
is the only place the app learns a redirect; it reads no network and no registry
field. A merge of a merge is followed to the last replacement, and a cycle, or a
chain that ends at a retired stub, resolves to nothing.

### Where the replacement is installed, the old name is the replacement

One module, `skill_redirects`, decides it, and every surface asks it.

**Saved choices carry, most specific first.** For a replacement `R` in bundle
`B`, a chat's choices decide in this order, and the first level that says
anything wins:

1. an entry naming `R` itself;
2. an entry naming an old name that resolves to `R`;
3. an entry naming `B`;
4. an entry naming the bundle an old name sat in, when the component moved to
   another bundle (W63 and W77 move perturb-seq, atlas-mapping and others this
   way).

Within level 2 or level 4 a removal beats an add, because the choices come from
different names and failing closed keeps content the person switched off from
coming back under the new name. A carried bundle choice applies to the moved
component only, never to the rest of `B`. Machine-wide, `disabled[]` can only
switch things off, so an entry naming an old name, or the other bundle it sat
in, switches `R` off. A chat can still switch `R` on; machine-wide, turning the
old bundle on is the only way, and `biorouter skill enable` says so rather than
reporting `R` as on.

| Path | What happens |
|---|---|
| A chat's `add` or `remove`, and `skills-config.json`'s `disabled[]` | Carried as above. |
| The old name's row | Shows the replacement's `state`, so either switch moves both. It also carries `supersededBy`, `replacement`, and `carried`, the saved choices made before the move that now decide the replacement. |
| A new choice naming the old name: `setSkillEnabled`, `POST /skills/session`, `workspace_set_tools`, a spawn's `add_skills` and `remove_skills`, `biorouter skill enable/disable`, the desktop's switches | Saved under the replacement. Turning the replacement on machine-wide also clears the old names in `disabled[]` that would keep it off. `POST /skills/session` returns the notice in `notices`. |
| `loadSkill` | Loads the replacement's instructions behind one line naming the move, and the replacement's own switch decides. For a whole-bundle replacement it returns the stub's pointer, and the line names the bundle's skills, because `loadSkill` loads skills. |
| A workflow's `skills:` naming the old name | Inlines the replacement's instructions; a replacement that is switched off refuses the run with the move named. |
| A workflow's `skills:` naming a bundle that holds a stub | A component merged into another member of the same bundle is dropped, since its replacement is inlined as a member. A component moved to another bundle is replaced by what it moved into, judged by that skill's own switch. A component whose replacement is not installed refuses the run, naming where to get it. A retired component is left out with a notice. |
| The chat's skill inventory | Lists the old name once, under moved names, rather than as a second skill. |
| `biorouter skill list` | Marks a moved row with an arrow and the name it moved into. |

### Where the replacement is not installed

The stub is an ordinary pointer with a switch of its own, and nothing is
carried. A workflow naming it, or naming a bundle that holds it, is refused
with the replacement named and where to add it, and `loadSkill` and
`setSkillEnabled` say the same in one line. When the chain passes through other
stubs, the line names the last target, the one that is missing.

### Retired names

A retired stub never resolves, because a retired skill moved no content
anywhere. A choice saved under it decides only its own pointer, whose text names
an alternative, and `loadSkill` returns that text behind a line saying it is
retired. A workflow that names a retired skill is refused, quoting the stub's
own description. A workflow that names a bundle holding a retired component
runs without it and says so: its publisher removed the content, which is what
deleting the folder would have done, and there is nothing to install that would
bring it back.

### One sentence, shown on every surface

`Redirect::notice` writes it, for example
`'taste-skill' has moved into 'frontend-design', so 'frontend-design' is used in its place.`
It appears in tool results, the rendered workflow skill block (which asks the
model to mention it to the person once, so the workflow can be updated), the
refusals, the target tab's change notice from `workspace_set_tools`, and the
terminal. Settings › Skills and the composer's Skills list show one muted line
when a saved choice was carried.

### Removing a stub keeps what it carried

A carried choice lives as long as the stub that carries it. So every removal
through Biorouter (Settings, `biorouter skill remove`, `removeSkillPackage`)
goes through `skill_package::remove_keeping_choices`, which first writes each
choice the stub was carrying under the replacement's own name: in every chat
that saved one, inside the housekeeping scope so no chat moves under Last
activity, and in `skills-config.json`. Removing a bundle that held a moved
component, or a stub in the middle of a chain, is handled the same way. A stub
deleted by hand ends its redirect with nothing pinned, because nothing runs to
do it, and so does a stub inside an extension's skills folder when the
extension is uninstalled; the catalog's stubs install into the skills folder,
not an extension's.

### One name, one meaning

A live skill always beats a stub of the same name, whatever the root order, so
a stub can never redirect a name that has live content. The importer refuses to
install a live skill under a name an installed stub carries, unless the install
replaces the very package that holds the stub (an update or a reinstall), and
names the stub's location so the person can rename the new skill or remove the
stub first.

### Why the stub, and why components carry no aliases

A name with no stub cannot be resolved: a folder deleted from a bundle leaves a
saved choice that names nothing, which is how a chat silently lost a skill
before this. So a superseded component keeps its folder, holding the generated
stub, and its name is never reused. The importer keeps component folders and
drops other files at a bundle's root, so the stub survives a marketplace
install.

Components carry no `aliases` (B1 rule 10), and this holds for W53, W63 and
W77. The redirect lives on the old row, never on the replacement, for three
reasons:

1. The old bundle is the only thing the person is sure to have installed. A
   component that moved to another bundle cannot be found through an alias on
   a target in a bundle that is not installed.
2. A stub folder survives the importer; a redirect file at a bundle's root
   would not.
3. One format serves top-level items and components, and a name keeps exactly
   one owner.

So W77's "old names as aliases" for the prefixed generic names and the
single-bundle pipelines becomes the same thing: each old component name is a
superseded row with a stub, never an alias on the new row. Top-level items keep
`aliases` for BAAM search, which the app does not read.

An old full copy installed before its stub was released keeps loading its old
text until it is reinstalled, because the registry cannot tell the app that an
id moved.

A stub whose name is also an installed bundle's name is not a redirect, because
a choice naming that word has always meant the bundle.

A redirect is honoured from any installed `SKILL.md`, so it is as trustworthy
as the package that installed the stub, whose text already tells the model to
load its target. It grants nothing the model could not already do with
`setSkillEnabled` in its own chat, and every carried choice is shown.

## Hot-loading: a skill installed mid-conversation

`SkillsClient` used to discover skills in its constructor and hold that map for
the life of the process, so a skill installed afterwards could never become
loadable in an existing chat. It now reads the process-global catalog on every
access, and a refresh reaches **every live conversation at once**.

The catalog rescans when either of two things changes:

* the **root set** — recomputed on every read, because installing an extension
  adds a root; or
* the **modification time** of any root or any bundle directory. Bundle
  directories are watched as well as roots because creating
  `<bundle>/<child>/` bumps the bundle's mtime and not the root's.

⚠ mtime has one-second granularity on some filesystems, so a write in the same
second as the last scan can be missed. Every change BioRouter makes itself calls
`skill_catalog::invalidate()` or `refresh()`, which closes that window. It stays
open only for a write by another process — `biorouter skill install` at a
terminal — and the interface's explicit refresh covers that: the composer's menu
asks the daemon to rescan each time it opens.

## Reacting to an extension install (`catalog:changed`, #112)

Installing an extension can add a whole skill root, so both halves of the
interface subscribe to the machine-wide inventory-changed event:

* **Rust** — anything that changes skills on disk calls
  `skill_catalog::invalidate()` (drop the snapshot) or `refresh()` (rescan and
  publish). That is the entry point for a `CatalogChanged` subscriber.
* **Renderer** — `useSkillCatalog` listens for the `catalog:changed` `window`
  event and rescans.

⚠ **The renderer keys off `revision` and reads nothing else from the payload.**
The event carries a `skills[]` list, and a consumer that repaired its inventory
from that list rather than refetching would drift the first time two events
raced. A monotonic revision that has advanced means "you are stale"; the answer
to being stale is to refetch.

⚠ **Only the newest read answers.** `useSkillCatalog` numbers every read it issues
(the first one, a chat switch, a rescan) and commits an answer, a failure or the end
of `loading` only if it is still the newest. Without that, the read for the chat the
hook served before could land after the next chat's and replace its view, and the
composer's Skills list would show the old chat's skills, then correct itself. A
write made through `setEnabled` is not a read: it serialises through its own queue
and commits the daemon's answer to the write it made.

## The HTTP surface

All three require `X-Secret-Key`, like every other route.

| Route | What it does |
|---|---|
| `GET /skills/catalog?session_id=&refresh=` | The catalog, composed for one conversation. Omit `session_id` for the machine-wide view a new chat would start with. |
| `POST /skills/session` | `{ sessionId, add[], remove[] }`. Persists the deviation and returns the catalog **read back after the write**, so the caller renders confirmed state rather than its own guess. |
| `POST /skills/refresh` | Rescan unconditionally and publish. What an install calls. |

There is deliberately no `X-User-Action` gate. Proof-of-user exists for raises
the *model* must not perform alone; enabling a skill in your own chat grants
nothing the machine-wide catalog did not already hold, and requiring the proof
would break browser access outright, since `biorouter serve` installs no digest
(see [serve decisions](../deployment/serve-decisions.md), SD-1).

## The rule the interface is held to

**A switch shows confirmed backend state, never local intent.** The composer's
per-chat branch once wrote a `Map` in React state and raised a green toast
saying the skill was "enabled for this chat" — no request, no write, no catalog
refresh, no live agent. The switch moved, the toast was green, and the next turn
saw exactly what it saw before.

So: the toggle is optimistic for one frame, then replaced by the daemon's
answer; a refusal restores the previous catalog and raises an **error** toast
carrying the daemon's own words; and no success toast is raised on a failed
write. The refusal message travels back in the mutation's *result* rather than
in hook state, because a caller reading it from hook state reads its own stale
render closure — which is how every failure once reported "The change was not
saved." with the reason dropped.

## What Settings shows, and what it will not delete

Rows are grouped by where the skill came from: **Biorouter Skills**, one group
per installed extension (**From BiorOffice**), **Skills From Other Agents**, and
**From This Project**.

A package is one expandable row carrying its display name, version and entry
point, opening to its components with their groups — not N unrelated rows.

⚠ **Two kinds of skill have no Delete control.** One Biorouter ships and
re-seeds on every start, and one an installed extension supplies. In both cases
the delete would succeed, toast, and be silently undone. That is the lesson
`BUILTIN_SKILL_NAMES` was originally written for, applied to a second case the
list never covered.

Deleting goes through the importer's remover
(`POST /skills/packages/remove`), which renames the directory aside before
deleting it. The root it deletes under is chosen from
`skill_catalog::roots()` — a path a caller invents matches nothing and is
refused, which is what makes it safe for this handler to cover
`~/.claude/skills` and a project directory as well as Biorouter's own.

## What the model sees, and why it carries provenance (#168)

`skills__searchSkills` returns a projection of the same catalog, one object per
skill. Alongside `name`, `description` and `bundle` each row carries:

| Field | Meaning |
|---|---|
| `builtin` | Biorouter seeded this skill and re-seeds it on every start. Same answer as the `builtin` on a catalog row. |
| `source` | The kind of root it was discovered under — `biorouter`, `extension`, `claudeHome`, `agentsHome`, `project`. |
| `extension` | The owning extension's directory name. Present only when `source` is `extension`. |
| `removable` | Whether `removeSkillPackage` accepts this skill at all. |
| `removalTarget` | The exact name to pass to `removeSkillPackage`. Present only when `removable`, and for a bundle member it is the **bundle**, not the skill. |

⚠ **These are derived from the catalog, never from a second list of names.**
`builtin` is `is_builtin_skill_name` — the very predicate the removal preflight
asks — and `source` comes from `skill_catalog::roots()`. A hand-maintained copy
is the thing that drifts.

The projection used to stop at `bundle`, which left a caller no way to tell a
package it may uninstall from one Biorouter ships or an extension supplies. Since
`removeSkillPackage` validates the **whole** batch before removing anything, a
single shipped name rejected all of them — and the rejection named one offender,
so the only route to a valid batch was to re-send it once per offender. It now
reports every rejected target grouped by reason **and** the targets that would
have been removed, so one retry is enough. The all-or-nothing semantics are
unchanged: a batch with any bad name still removes nothing.

`removable` is false for everything outside Biorouter's own skills root, because
that is the only root `removeSkillPackage` deletes under. An extension's skill is
uninstalled by removing the extension.

## How a query is matched

`skills__searchSkills` with no `query` pages the catalog alphabetically. With a
query it **ranks** the skills this conversation has enabled, through
`catalog_search::rank` — the same matcher behind `searchMarketplaceSkills` and
`search_marketplace_extensions`, whose module doc is the specification:

- A skill is returned when it matches **any** word of the query, not all of
  them. Skills matching the most words come first; ties go to where a word
  matched — the skill's name, then its bundle's name, then its description —
  and then to alphabetical order. A skill holding the whole query as written
  outranks all of those.
- A word under three characters matches whole words only, so `r` finds the R
  language rather than every word with an r in it. Filler (`for`, `about`,
  `skill`) is dropped, and a plural falls back to its singular.
- A word matches **inside** a longer word only when it is four characters or
  half that word: `omics` finds `transcriptomics` and `rna` finds `scRNA`, but
  `gen` no longer finds `…Agent` and `lab` no longer finds `BaranziniLab`. Three
  characters buried in a long word is a morpheme rather than a search, and on the
  marketplace catalog it returned nearly everything — `lab` matched 37 of 37
  extensions, `ing` 88 of 129 skills.
- The conversation's switches apply **first**: a skill turned off here is never
  scored, returned or counted.

A ranked page is the listing's page plus three fields. `terms` is what the query
was read as, each row adds `matchedTerms`, and a search that matches nothing adds
`guidance` — how many skills are enabled in this conversation, and to try a
shorter term or list them all. A query with no letter or digit in it is the
listing.

⚠ **The search used to keep a skill only when it contained every word of the
query.** With a ggplot skill and an R-scripting skill installed,
`R scripting ggplot visualization` returned `total: 0` — QA finding F5, which the
marketplace search had too, through different code. It now returns the ggplot
skill (three of the four words), the R-scripting skill (two), then any skill
matching one. Add a catalog search by calling `rank` with that catalog's fields,
never by writing another matcher.

The Skills page filter reads the same rules with one addition that the model's
search does not have. A filter is typed into, so the last word, while nothing
follows it, also matches the start of a word in a skill's name (or in a package's
member names), ranked below the whole word. `gg` finds `ggplot`, `r` lists
`r-scripting` first and `ralph` after it, and `gg ` with a space holds the word
to whole words again. Descriptions are never read by prefix. The composer's skill
picker keeps the whole-word floor, because its Enable all button writes every row
the filter leaves on screen. The rule lives in `typeahead` of `baam/search.ts`
and is asked for by `skills/SkillsView.tsx` alone.

## Debugging a skill that is installed but not usable

1. `biorouter skill list` — if it is absent, discovery never saw it. Check the
   frontmatter parses (`name:` and `description:` both present).
2. `GET /skills/catalog` — if the CLI sees it and this does not, the daemon is
   holding a stale snapshot; `POST /skills/refresh`.
3. If the catalog lists it with `state.effective: false`, read the rest of
   `state`: `machineEnabled: false` means `skills-config.json`, `session:
   "removed"` means this chat, and `sessionViaBundle: true` means the chat
   turned off the bundle rather than the skill.
4. `hiddenContext: true` means Settings → Contexts, and the skill is still
   loadable by exact name. That asymmetry is intentional. It decides
   `effective` only while `session` is `"default"`: a chat that added the
   Context has it on whatever Settings says. For a bundle member the switch is
   the **bundle's**: `knowledge-bases` is one Context row over five skills.
5. `builtin: true` — on a skill row or a bundle row — means Biorouter seeded it,
   so no surface offers a Delete, and `skill_package::remove` **and `install`**
   both refuse that name under Biorouter's own skills root. It is restored on
   every start; disable it instead.
6. `replacement` on a row means the skill is a redirect stub and its state is
   its replacement's. `carried` names the saved choice made before the move
   that decides it: `sessionAdded` and `sessionRemoved` are this chat's choice
   for the old name, `sessionBundleAdded` and `sessionBundleRemoved` its choice
   for the bundle the old name sat in, and `machineDisabled` and
   `machineBundleDisabled` the same two in `skills-config.json`. Turn the
   replacement, or its own bundle, on or off directly in a chat; a direct
   choice wins.
7. ⚠ **A knowledge skill disabled machine-wide before it became a bundle member
   is honoured but no longer reachable from the interface.** Those four were
   ordinary picker rows once, so `skills-config.json` may name one directly.
   Nothing writes that entry any more and no surface renders the member, so the
   Knowledge Context reads ON while that one skill stays out of the model's
   catalog. The entry is deliberately **not** discarded on upgrade — silently
   overruling an explicit choice is the defect, not the fix — and the terminal
   still shows and clears it:

   ```bash
   biorouter skill list          # the member shows ○
   biorouter skill enable knowledge-lint
   ```

## Related documentation

- [Skills capability](built-in/skills.md) — the user-facing guide to what a skill is and where to get one
- [Extensions, skills, and MCP agents](extensions-and-skills-guide.md) — installing and authoring
- [Skill packages](skill-packages.md) — how a skill or package gets onto disk in the first place
- [Workspace control](../agent-loop/workspace-control.md) — `workspace_set_tools`, the model's own route to the same per-session state
