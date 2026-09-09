---
name: knowledge-lint
description: "Reading a knowledge-base lint report and fixing what it found: the rule ids and severities kb_lint returns, what each one means, and the repair for it. Load this when the user asks to lint, check, tidy, audit or repair a knowledge base, when kb_validate_page or kb_lint reports diagnostics you need to act on, or before exporting or sharing a base."
---

# Linting a knowledge base

Two tools report the same kind of finding at two scopes:

- **`kb_validate_page`** — one draft, before you write it. Cheap, and where most of
  these should be caught.
- **`kb_lint`** — the whole base. Run it after a batch of edits, and before exporting or
  sharing.

Both return **diagnostics**. Each carries five things, four of them always:

| field | what it is |
| --- | --- |
| `rule` | a stable id like `biookf.edge.missing_primary_source`. Match on this, never on the message — messages get reworded. |
| `severity` | `error`, `warning` or `info`. |
| `subject` | the page or edge the finding is about. Never empty. |
| `path` | the page's bundle-relative path, when the finding has one. Absent for a base-wide finding, and for a draft that has no path yet. |
| `message` | what is wrong, in a sentence. |

The rule id's prefix says which layer objected, and that tells you how to read it:

- **`kb.`** — Biorouter's own housekeeping. Untidy, not invalid.
- **`okf.`** — OKF v0.2 conformance. The file does not meet the format's own rules.
- **`biookf.`** — the biomedical profile's vocabulary and provenance rules. These only
  appear in a BioOKF base.

## Nothing here rejects anything

An `error` means "this page is not conformant". It does **not** mean the page will stop
being read, rendered or linked — it will. So do not "fix" a diagnostic by deleting
content or by loosening the page until the report goes quiet. Fix the thing it names, or
decide deliberately not to and say why.

Two corollaries worth internalising:

- A base that predates both formats is not linted at all — `kb_lint` **refuses** it,
  because that retired pre-OKF storage is purged on startup. `kb_read_page` and
  `kb_search` still work on one, so the content is recoverable; you just cannot lint
  or repair. Tell the user to restart Biorouter.
- `kb_lint` caps how many diagnostics it returns. If `total` exceeds the length of
  `items`, fix a batch and run it again rather than assuming you have seen everything.

## The housekeeping rules (`kb.`)

| rule | means | fix |
| --- | --- | --- |
| `kb.orphan` | no other page links to this one | link it from a hub page, or fold it into the page it belongs to, or delete it if it says nothing |
| `kb.contradiction` | the page declares `contradiction: true` | resolve it: say which source wins and why, or record both positions under `## Open contradictions` |
| `kb.stale_source` | ingested over 90 days ago and still referenced by nothing | either ingest it properly into pages, or accept it — an unused source is not a bug, only a loose end |
| `kb.missing_concept_page` | a source page links to a target no page carries | create the page, or fix the link if the target already exists under another name |

Orphans are the one to take seriously: a page nothing links to is invisible in the graph
and will not be found by anyone navigating it.

## The format rules (`okf.`)

| rule | means | fix |
| --- | --- | --- |
| `okf.frontmatter.unparseable` | the `---` block is not valid YAML | usually an unquoted scalar containing `: ` (colon-space), or an unterminated block |
| `okf.frontmatter.absent` | the file has no `---` block | add one; every non-reserved `.md` is a concept document |
| `okf.type.missing` | no non-empty `type` | add one — it is the single always-required key |
| `okf.source.resource_missing` | a `sources[]` entry names no `resource` | point it at `raw/<id>/…` or the URL it came from |
| `okf.generated.by_missing` | a `generated` block with no `by` | name the producer, or drop the block |
| `okf.footnote.unresolved` | a `[^id]` in the body with no matching `sources[].id` | add the source entry, or remove the footnote |
| `okf.verified.bare_mapping` | `verified` is a single mapping, not a list | informational; a list is the canonical shape |
| `okf.attestation.unchecked` | the page declares an attested computation | informational: this build does not verify them, and says so rather than implying it did |
| `okf.index.frontmatter` | `index.md`'s frontmatter is missing or malformed | a warning about the bundle's own scaffold rather than about a concept page |
| `okf.log.date_heading` | `log.md` carries an entry with no `## YYYY-MM-DD` heading | give the entry a date heading |

## The profile rules (`biookf.`) — BioOKF bases only

**Vocabulary**

- `biookf.type.missing` — the page has no `type` at all. Every page in this profile needs
  one of the 28; this is the distinct case from an invented one.
- `biookf.type.invalid` — the `type` is not one of the 28. Re-run the typing decision
  procedure in **knowledge-ingest-biookf**; do not invent a 29th type. If genuinely
  nothing fits, `Other` plus a note is the honest answer.
- `biookf.predicate.invalid` — the predicate is not one of the 35. Pick the closest legal
  one that the source actually supports; the message names candidates.
- `biookf.edge.not_negatable` — you prefixed `not_` onto one of the 13 predicates that
  do not take it. Only the 11 effect predicates are negatable.
- `biookf.alias.*` — a deprecated spelling that still reads. Update it to the current
  one; it is a warning, not a break.

**Identity**

- `biookf.identifier.missing` — every page in this profile needs one; it is the key
  every edge joins on.
- `biookf.identifier.duplicate` — two pages claim the same name. Merge them. This is the
  one to fix first, because every edge into that name is now ambiguous.
- `biookf.identifier.opaque` — the identifier is a CURIE or a code, not a human-readable
  name. CURIEs belong in `xref`.

**Edges and provenance**

- `biookf.edge.object_missing` / `biookf.edge.object_unresolved` — the edge points at
  nothing. Create the target page, or fix the name. Note that during an ingest this is
  often *temporary*: create the target and re-lint.
- `biookf.edge.missing_knowledge_level` / `missing_agent_type` /
  `missing_primary_source` — the provenance triplet is incomplete. All three are
  required on every edge, including `reported_in` and including negated edges.
- `biookf.edge.invalid_knowledge_level` / `invalid_agent_type` — not one of the legal
  values. Do not elevate a `statistical_association` to a `knowledge_assertion` to make
  the message go away.
- `biookf.edge.primary_source_unresolved` — the cited source node does not exist.
  Materialise it as a real page; a `sources[]` entry is not a node.
- `biookf.edge.primary_source_not_source` — you cited a node that is not a
  `Publication`, `Study`, `Dataset` or `Agent`. Only those four are evidence.
- `biookf.edge.primary_source_not_provided` — the placeholder is still there. Replace it
  with the real source, or accept it deliberately if the claim genuinely has none.
- `biookf.edge.domain` / `biookf.edge.range` — the predicate does not accept a node of
  that type on that end. Usually the typing is wrong, not the predicate; re-check step 5
  of the typing procedure (the Disease / Phenotype / BiomedicalMeasure trio is the
  commonest cause).
- `biookf.edge.contradiction` — the base asserts both `<X>` and `not_<X>` between the
  same two nodes. Keep the better-evidenced edge and record the disagreement in prose.
- `biookf.source.unanchored` — a source node with no `raw_source`. Point it at the bytes
  under `raw/`, which is what makes the provenance chain end somewhere real.

**Evidence quality** — read off the credibility verdict the classifier wrote into
`raw/<id>/meta.yaml` at ingest. A source the classifier never saw is never flagged, so
silence here means "no signal", not "checked and fine".

- `biookf.source.retracted` — a claim rests on a source marked RETRACTED. Do not delete
  the source page: it is a real thing that was really published. Re-source every claim
  citing it from the current literature, or withdraw those claims and say in prose that
  the finding was retracted.
- `biookf.source.not_scholarly` — a `knowledge_assertion` (the strongest knowledge
  level: "this is established") rests on a web page or a personal communication. Two
  honest fixes: cite the primary literature, or lower the edge's `knowledge_level` to
  what the source actually supports. Deleting the warning by raising the tier is not one
  of them.

## Working through a report

1. **Errors first, then warnings, then infos.** The report is already ordered that way.
2. **`biookf.identifier.duplicate` before anything else** — every other finding on those
   pages may be a consequence of it.
3. **Then the dangling references** (`object_unresolved`, `primary_source_unresolved`,
   `kb.missing_concept_page`). Creating one missing page often clears several findings
   at once.
4. **Validate each repair before writing it** with `kb_validate_page`, so you do not
   trade one finding for another.
5. **Re-run `kb_lint`** and check `total`, not just the length of `items`.
6. **Log it.** `kb_append_log` with `kind: "lint"` and a delta saying what you fixed.

## Autofix

**The `kb_lint` tool never edits anything.** It reports; you fix, with `kb_write_page`.

There *is* an autofix — a sub-agent that repairs the mechanical findings (missing links,
missing pages, alias updates) and commits only if pages actually changed — but it lives
on the two surfaces that name a provider: `biorouter kb lint --fix` in the terminal, and
the "Check for problems" panel in the Knowledge view. It is not reachable as a tool,
because a tool that sometimes writes could not be classified as reading or writing, and
that classification is what keeps a private base out of a public chat. Suggest the CLI
command to the user if the report is long and mechanical; do the judgment calls —
contradictions, re-typing, which source wins — yourself either way.
