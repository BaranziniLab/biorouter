---
name: knowledge-ingest-okf
description: "The loop for ingesting a source into an OKF knowledge base: turning a paper, document, page, transcript or pasted note into linked concept pages with provenance. Load this whenever the user asks to ingest, file, remember, add, capture or import something into a knowledge base that is in OKF format (the default), or when you are writing pages into one."
---

# Ingesting into an OKF knowledge base

This is the loop for a base whose `format` is `okf`. Check first — read the base's
`schema.md` — because a **BioOKF** base has a controlled vocabulary and a required
provenance triplet, and the skill for it is **knowledge-ingest-biookf**. If the base
has no `format`, stop and tell the user that its retired pre-OKF storage must be
purged by restarting Biorouter. Never create or extend legacy `title:` / `kind:`
pages or `[[wiki links]]`.

You own `knowledge/`, `index.md` and `log.md`. **Never write under `raw/`** — it holds
the original bytes and is read-only by design.

## The loop

**1 — Land the source.** `kb_add_raw_source` with the URL or the text. It files the
original under `raw/<source-id>/` with a derived `source.md` and a `meta.yaml`, and
classifies its credibility. It creates **no** knowledge pages; that is the rest of this
loop.

**2 — Read it.** `kb_read_page` on `raw/<source-id>/source.md`. Read the whole thing
before writing anything. Note what it is, who wrote it, and when — those become the
page's `sources[]` entry.

**3 — Decide what deserves a page.** A page is a **durable, reusable thing** that other
pages will want to point at: a concept, a method, a system, a person, a decision, a
dataset. It is not a fact, a number, a phrasing, or a relationship. "Postgres" is a
page; "Postgres is faster than SQLite for this workload" is a sentence on a page, with a
link.

A typical source yields a handful of pages, not thirty. If you cannot imagine a second
page linking to it, it is a paragraph, not a page.

**4 — Search before you create.** `kb_search` for each candidate. Reusing an existing
page and extending it is almost always right; forking a near-duplicate under a slightly
different name is the failure mode that quietly ruins a base. When the page exists,
**update** it: add the new source, merge genuinely new detail, and do not delete what is
already there.

**5 — Choose a `type` and keep using it.** OKF's vocabulary is open — `type` is any
non-empty string. That freedom is only useful if you are consistent, so:

- look at what the base already uses (`kb_list_pages` shows the directory names);
- reuse an existing type before coining a new one;
- coin in the singular, capitalized: `Method`, `Dataset`, `Decision`, `Person`;
- put the page at `knowledge/<lowercased type>/<slug>.md`.

**6 — Validate, then write.** Call `kb_validate_page` with the draft and the path you
are about to use, fix anything it reports, then `kb_write_page`. In OKF mode it is
checking the things that actually break a base: a parseable frontmatter block, a
non-empty `type`, footnotes that resolve, and `sources[]` entries that name a resource.

**7 — Link.** The graph is derived from links, so a page nobody links to is invisible in
it. Write links as ordinary markdown links to the target's path:

```markdown
Heart rate variability [falls with](/knowledge/concept/sympathetic-tone.md) rising
sympathetic tone.[^pmid-32504360]

[^pmid-32504360]: Task Force 1996, Circulation.
```

A link to a page that does not exist yet is legal and is recorded — create it when you
know enough to write it. Do not write untyped `[[double brackets]]`: pages that already
carry them are still read, permanently, and must not be rewritten, but do not write new
ones. (BioOKF has its own `[[predicate:: Object | key=value]]` inline edge form. That is
a different grammar and belongs only in a BioOKF base.)

**8 — Bookkeep.** Update `index.md` so the new pages are cataloged, then
`kb_append_log` with `kind: "ingest"` and a one-line summary plus a `delta` like
`+3 pages, +7 links`.

## The page

```yaml
---
type: Method
identifier: Heart rate variability
description: Beat-to-beat variation in heart rate.
tags: [autonomic, cardiology]
xref: [MESH:D006339]
status: stable
sources:
  - id: pmid-8598068
    resource: raw/pmid-8598068/original.pdf
    title: Heart rate variability standards of measurement
    author: "human:Task Force"
    last_modified: 1996-03-01
---

# Heart rate variability

Beat-to-beat variation in heart rate, used as a
[non-invasive readout](/knowledge/concept/autonomic-tone.md) of autonomic tone.[^pmid-8598068]

## Sources

- [Task Force 1996](/knowledge/source/pmid-8598068.md)

[^pmid-8598068]: Heart rate variability standards of measurement, Circulation 1996.
```

`type` is the one always-required key. `identifier` is what other pages resolve to, so
give every page one and keep it unique in the base. Omit anything you do not know rather
than inventing it — an invented `last_modified` is worse than an absent one, because a
reader cannot tell it was invented.

## Pitfalls

- **Writing a page per fact.** The commonest one. Facts are sentences with links.
- **Coining a new `type` per page.** Twelve types used once each is an open vocabulary
  used as a free-text field; the graph learns nothing from it.
- **Forking instead of reusing.** Search first, every time.
- **Prose that restates another page.** Link to it. If a fact lives elsewhere, the link
  *is* the statement.
- **Quoting.** Any YAML scalar containing `: ` (colon-space) must be quoted, or the
  frontmatter will not parse — `kb_validate_page` catches this before it costs you a
  write.
- **Skipping step 8.** A base whose `index.md` and `log.md` are stale is one nobody
  trusts a month later.
