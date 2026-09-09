# How this documentation is organized

> **What this is.** The rules for deciding **where a document goes**, **what it is called**,
> and **when a new folder is justified** — the sorting system for `docs/`. Read it before
> adding a document, creating a folder, or moving anything.
> **Status:** Current. This is the governing convention for the tree; it is meant to grow as
> the product does.
> **Audience:** anyone adding documentation — humans and agents alike. Agents working in this
> repository should treat it as binding.

The tree was reorganized on 2026-07-18 from a flat pile into topic folders. That reorganization
was not the point; keeping it coherent as the product grows is. This page exists so the next
person — or the next agent, on a feature or a bug fix six months from now — files a document
the same way, without having to reverse-engineer the pattern or ask.

Its companion is the [documentation style guide](contributing/documentation-style.md), which
governs what a document *looks like* on the inside. This page governs where it *lives*.

---

## 1. The one question that decides everything

Before anything else, answer this:

> **Does this document tell a reader how the system works now, or does it record work that was
> done?**

That single distinction is the tree's backbone.

| Answer | Where it goes | What it is |
|---|---|---|
| How the system works **now** | a subsystem folder at the top level | living documentation |
| A record of work that **was done** — a plan that ran, a review that concluded, a campaign that finished, a design that shipped | `history/<campaign>/` | a historical record |

Everything else in this document is elaboration on that split.

**Why it matters more than naming.** Before the reorganization, 137 of 233 files were records
of completed work sitting beside living guidance, with nothing distinguishing them. A reader
could not tell whether a plan had been executed, whether a design described shipped code or an
intention, or whether a checklist still needed running. Filenames did not fix that. The split
does.

> **The test for "living".** If the code changed tomorrow, would this document need to be
> edited? If yes, it is living. If it would instead become *a record of what things used to
> be*, it belongs in `history/`.

### The status line is the contract

Every document declares which side it is on, in its header:

```markdown
> **Status:** Current.
> **Status:** Historical record — <what completed, and when>.
> **Status:** Superseded — <what changed, and where the current truth lives>.
```

Those are the only three values. If you need a fourth, you have found something this system
does not model — say so explicitly in the document and flag it, rather than inventing a verb.
Two real cases that look like exceptions but are not:

- *"Partially implemented."* That is `Current` with open work named in the status sentence.
- *"Superseded in part."* That is `Current` with a warning; use it when a live document has a
  section that has rotted, and say which section.

**The status line and the folder must agree.** A file marked `Historical record` sitting in a
living folder is a filing error, and so is a `Current` file under `history/`. This is the most
common way the tree drifts, and it is mechanically checkable — see [§8](#8-checks-before-you-commit).

---

## 2. Living documentation: file it by subsystem

Top-level folders name **subsystems and user-facing concerns**, not document types. There is no
`guides/`, no `reference/`, no `plans/` — a reader looking for how permissions work should not
have to guess whether permissions are a guide or a reference.

Current top-level areas:

| Folder | Scope |
|---|---|
| `getting-started/` | installing, first run, choosing a provider, chat basics, usage habits |
| `architecture/` | how the system is put together, across crates |
| `agent-loop/` | the reasoning loop: accepted designs, hooks, subagents, context engineering |
| `agent-drafter/` | the apps platform — the agent that builds apps |
| `apps-sdk/` | the SDK those apps are built against |
| `extensions/` | the extension system, and `built-in/` for each shipped extension |
| `integrations/` | adapters that let another host application run a Biorouter agent |
| `knowledge-base/` | personal knowledge bases |
| `providers/` | LLM provider integrations |
| `security/` | permission modes, secrets, managed policy, data handling |
| `workflows/` | workflow authoring, storage, scheduling |
| `cli/` | the `biorouter` command-line interface |
| `configuration/` | config files and environment variables |
| `desktop-ui/` | the Electron GUI |
| `deployment/` | reaching Biorouter through a browser — on your own machine or on a shared host |
| `releases/` | release engineering, plus `notes/` per shipped version |
| `troubleshooting/` | when something is broken |
| `design/` | visual and interaction design, including HTML studios |
| `research/` | studies of systems **outside** this repo |
| `testing/` | the properties tests must hold, and the shared state they must not inherit |
| `contributing/` | how to work on the documentation itself |
| `history/` | records of completed work (see [§3](#3-history-records-of-work-that-was-done)) |

**Adding to this list is expected**, not exceptional. See [§5](#5-when-to-create-a-new-folder).

### Choosing between two plausible folders

Ask **what the reader was looking for when they arrived**, not what the document is about in
the abstract. Some judgement calls already made, as worked examples:

- *Permission modes* → `security/`, not `configuration/`. Someone reading it is deciding how
  much autonomy to grant, not editing a config file.
- *The agent-drafter apps platform* and *the apps SDK* are separate folders. One is the map
  (how the platform behaves), the other the territory (the API surface). Each README states the
  boundary and links the other.
- *Nine studies of external coding agents* → `research/`, not `history/`. They document other
  people's systems, so they are reference material, not a record of our work — and they do not
  go stale when our code changes.
- *The Auto Visualiser tool-selection fixtures* stay in `crates/.../tests/fixtures/`, because
  they are test inputs a run reads, not prose. Their README follows this system and is linked
  from `extensions/built-in/auto-visualiser.md`. **Documentation lives with what it documents
  when the artifact is code; only prose belongs under `docs/`.**

---

## 3. `history/`: records of work that was done

One folder per **campaign** — a coherent piece of work with a beginning and an end. Not one
folder per document, and never a `misc/`.

A campaign folder holds everything belonging to that effort, together: the design, the plan
that executed it, the verification reports, the raw data, the outcome. Keeping them together is
the point — it is what makes a decision auditable years later.

```text
history/<campaign>/
    README.md              what the campaign was, when it ran, and how it ended
    <design>.md            what was intended
    <plan>.md              how it was to be executed
    wave-reports/          per-stage verification, if the campaign had stages
    data/                  raw JSON/CSV the reports cite
    authoring-logs/        machine-generated run logs
```

**A history README must answer, in its first paragraph:** *did this actually happen, and does
it still matter?* Give the date range, and say how it ended — shipped, superseded, abandoned.
If the feature was later removed, say where the current truth lives now.

Work that was **abandoned or removed** still belongs here. `history/dashboard-mode/` holds four
design/plan pairs for a feature that was built and then deleted, plus the removal record. That
is not clutter; it is the answer to "why isn't there a dashboard, and was it tried?"

**Nothing is deleted for being stale.** Label it and move it. The only things removed during
the 2026-07-18 cleanup were two *generated* files — single-page HTML views stitched from
Markdown that still exists in the tree — because keeping a derived copy of a corpus is not
preservation, it is duplication.

---

## 4. Naming

| Rule | Example |
|---|---|
| `kebab-case.md`; no `ALL_CAPS` except `README.md` | `secret-storage.md` |
| Name the **purpose**, not the process that produced it | `FINDINGS.md` → `audit-findings-register.md` |
| Do not encode the doc type in the name | `hooks-reference.md`, not `hooks-guide-reference-doc.md` |
| ISO date **suffix** only when the date is load-bearing | `desktop-reliability-2026-07.md` |
| A folder name is a topic, singular and specific | `llama-server/`, not `misc-providers/` |
| Keep identifiers that are cited elsewhere | `BR-43` stays in the body; the filename says `shadow-git-checkpoints.md` |

On that last row: ticket-style identifiers (`BR-NN`, `spec-NNN`, wave names) are often cited
from source comments and other documents, so they must survive. Put the **subject** in the
filename and keep the identifier in the document, with a one-line key in the header explaining
the scheme.

---

## 5. When to create a new folder

Create one when **both** are true:

1. You can name the topic in two or three words without using "and", "misc", or "other".
2. Either it already has two or more documents, or you can state plainly what else will land
   there.

A folder holding a **single document is fine** if its topic is precise. `deployment/`,
`knowledge-base/`, and `history/notification-redesign/` each hold one document and are correct:
a precise one-document topic is strictly better than a vague bucket. What is *not* fine is a
folder containing only a `README.md` that redirects elsewhere — that is a signpost, not a
topic, and its content belongs in the place it points to.

**Never create:** `misc/`, `other/`, `general/`, `stuff/`, `notes/`, `temp/`, or a folder named
after a document type (`plans/`, `specs/`, `reports/`). If work does not fit an existing topic,
it needs a *new specific topic*, not a bucket. Buckets are where documents go to be lost.

### Adding a subsystem

New feature area, expected to accumulate documentation:

1. Create `docs/<subsystem>/` with a `README.md` index.
2. Add a row to the by-topic table in [`docs/README.md`](README.md).
3. Add a row to the table in [§2](#2-living-documentation-file-it-by-subsystem) of this page.
4. Cross-link the neighbours whose boundary it touches, in both directions.

### Starting a campaign

Multi-stage work with a defined end — a migration, an audit, a fix programme:

1. Create `docs/history/<campaign>/` at the point the work **concludes**, not when it starts.
2. While in flight, keep the plan in the relevant living folder, marked `Current`.
3. On completion, move the material into the campaign folder, restatus it to
   `Historical record — <outcome>, <date>`, and write the README that says how it ended.
4. If the work changed how the system behaves, update the **living** documentation too. The
   history folder records what was done; it does not substitute for the guide someone will
   actually read.

That last step is the one most often skipped, and it is the one that matters.

---

## 6. Every folder carries a `README.md`

An index that lists the folder's files with a one-line description of each, drawn from reading
them. It also states the folder's **boundary with its neighbours**, so a reader who has landed
in the wrong place can leave quickly.

An index must also name non-Markdown contents and say what they are for. If the folder holds an
HTML studio, the index says it needs a browser to be useful.

---

## 7. HTML, data, and other non-Markdown

| Kind | Where | Rule |
|---|---|---|
| Prose | `.md` | the default; anything that is headings, paragraphs, lists, tables and code is Markdown |
| Genuinely visual pages | `.html` beside the topic's Markdown | only when the page **renders** something Markdown cannot: live theme tokens, colour swatches, logo studios, interactive component galleries, real mockups |
| Visual page carrying heavy prose | both | keep the HTML, add a Markdown companion that carries the reasoning; the companion points at the HTML and marks visual-only content with `> **Rendered in the HTML.**` |
| Raw data cited by a report | `data/` inside the campaign folder | JSON/CSV the reports reference |
| Machine-generated logs | `authoring-logs/` inside the campaign folder | |
| Images and icons | `assets/` inside the topic folder | |
| Test fixtures | **not** in `docs/` | they live with the tests that read them; their README follows this system and is linked from the relevant doc |

> **Warning.** Do not check in a **generated** document. If a file is assembled from other files
> by a script, keep the sources and the script, not the output. Two such files were deleted in
> the 2026-07-18 cleanup after measurement showed 96.6% of one was a verbatim copy of Markdown
> already in the tree. A derived view is not a document.

---

## 8. Checks before you commit

Mechanical, and worth running:

```bash
# every folder that holds documents has an index
for d in $(find docs -type d); do
  ls "$d"/*.md >/dev/null 2>&1 && [ ! -f "$d/README.md" ] && echo "no index: $d"
done

# no ALL_CAPS filenames outside README.md
find docs -name '*.md' | grep -E '/[A-Z_]+\.md$' | grep -v README.md

# nothing loose at the root (README.md and this file are the only two)
find docs -maxdepth 1 -type f
```

By judgement:

- **Status matches folder.** Nothing marked `Historical record` or `Superseded` sits in a living
  folder; nothing `Current` sits under `history/`.
- **Every link resolves.** The tree is kept at zero broken links; do not be the one who breaks it.
- **You updated the folder's `README.md` table.** A document nobody can find from its index is
  a document nobody will read.
- **You updated referrers.** Source comments, `CLAUDE.md`, `README.md`, and the `Justfile` all
  cite documentation paths. If you moved a file, `grep -rn "docs/" --include='*.rs'` and friends
  before you finish. A Rust comment pointing at a moved file rots silently — nothing will ever
  flag it.

---

## 9. When the rules do not fit

They will not, eventually. When that happens:

- **Do not** invent a status verb, a bucket folder, or a private convention in one corner of
  the tree.
- **Do** write the document, file it in the closest honest place, and record the mismatch in
  [open documentation issues](contributing/open-documentation-issues.md) with what you found and
  why the system did not accommodate it.
- If the same mismatch appears twice, that is the system being wrong, not the document. Extend
  this page.

This page is expected to change. Adding a top-level area, a new campaign shape, or a new content
kind is normal maintenance — amend the relevant section and say in the commit message what
category of thing prompted it.

## Related documentation

- [Documentation index](README.md) — the entry point; its by-topic table is the live version of §2
- [Documentation style guide](contributing/documentation-style.md) — what a document looks like inside, once you know where it goes
- [Open documentation issues](contributing/open-documentation-issues.md) — where an unresolved conflict or a mismatch with this system gets recorded
- [`history/`](history/README.md) — the records of completed work, and the shape a campaign folder takes
