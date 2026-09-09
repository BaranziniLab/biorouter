---
name: knowledge-ingest-biookf
description: "The loop for ingesting a source into a BioOKF knowledge base, including the decision procedure for typing an entity as one of the 28 node types and choosing one of the 35 edge predicates with its required provenance triplet. Load this whenever you are ingesting a paper, trial, dataset or biomedical document into a BioOKF-format base, or writing or fixing typed pages and edges in one."
---

# Ingesting into a BioOKF knowledge base

This is the loop for a base whose `format` is `biookf`. Confirm before you start —
`kb_read_page { path: "schema.md" }` states the format and carries the full vocabulary,
generated for this build. If the base is plain **OKF**, use **knowledge-ingest-okf**
instead; typing an OKF base against a biomedical vocabulary produces a base of `Other`.

You own `knowledge/`, `index.md` and `log.md`. **Never write under `raw/`**.

Three constraints define this profile, and they apply to every page:

1. `type` is one of **28** controlled values.
2. an edge's `predicate` is one of **35** (24 forward-only, plus `not_` on 11 of them).
3. every edge carries `knowledge_level`, `agent_type` and `primary_source`.

Nothing rejects a page on read — but `kb_lint` flags every unknown value, and a base
full of flagged values is not exchangeable, which is the entire reason to be in this
profile rather than OKF.

## The loop, per source

**1 — Land the source and read it.** `kb_add_raw_source`, then `kb_read_page` on
`raw/<source-id>/source.md`. Read all of it. Read the figures too where you can: axes,
labels and legends carry claims, and a figure-derived edge is stamped with the same
`primary_source` as a text-derived one.

**2 — Create the source node FIRST.** Every edge you write next must cite it by
`identifier`, so it has to exist. Only four types may be cited as a `primary_source`:
**Publication** (paper, preprint, review), **Study** (trial, cohort, GWAS), **Dataset**,
**Agent** (an external authority such as HGNC or DrugBank). `Population`,
`GeographicLocation`, `Concept` and `Other` are context, not evidence.

```yaml
---
type: Publication
identifier: Chen 2020 (IL-6 and severe COVID-19)
xref: [PMID:32504360]
raw_source: [raw/pmid-32504360/original.pdf]
---
```

`raw_source` is what anchors the chain to immutable bytes. A `sources[]` entry is **not**
a node and cannot be traversed — it does not substitute.

**3 — Type each entity** using the decision procedure below.

**4 — Search before creating.** `kb_search` for the `identifier` you are about to mint.
Reuse and extend; never fork a near-duplicate. When the page exists, add a `reported_in`
edge to this source, merge in genuinely new synonyms / `xref` / description, and delete
nothing.

**5 — Validate, then write.** `kb_validate_page` with the draft and its intended path,
fix what it reports, then `kb_write_page` to `knowledge/<lowercased type>/<slug>.md`.
Do this per page, not per batch — the validator is where an invented predicate, a
missing triplet, an unresolved `object` and a duplicate `identifier` are caught, and
fixing one page costs a great deal less than discovering it after twelve.

**6 — Write the edges.** See "Choosing a predicate" below. Put every number the source
reports on the edge, not only in the prose.

**7 — Bookkeep.** Update `index.md`, then `kb_append_log` with `kind: "ingest"` and a
`delta` like `+11 nodes, +23 edges`. Then run `kb_lint` over the base and fix every
error before calling the source done.

## Typing an entity: the decision procedure

Work through these in order. Stop at the first one that answers.

**Step 1 — Is it evidence, or is it biology?** Anything that *reports* — a paper, a
trial, a database, an organization — is one of the 8 provenance-and-context types
(`Publication`, `Study`, `Dataset`, `Agent`, `Population`, `GeographicLocation`,
`Concept`, `Other`). Everything else is one of the 20 biomedical entity types.

**Step 2 — Classify by identity, not by role.** Type the thing by *what it is*, not by
what it is doing in this sentence. Aspirin **is** a `Molecule`; "aspirin treats
headache" is an edge, not a type. A gene used as a biomarker is still a `Gene`; the
biomarker relationship is a `measures` edge.

**Step 3 — Is it a thing at all, or a relationship?** A relationship between two
concepts is an **edge**, never a node. "IL-6 elevation in severe COVID" is an edge from
`Interleukin-6` to `COVID-19`. A measurement *value* is edge data. A variant's
consequence is edge data. If you find yourself minting a node whose name contains "in",
"with", "and" or "associated", stop — it is an edge.

**Step 4 — Disambiguate by use.** A word whose referent changes with context is typed by
its referent *here*. "BRCA1" is a `Gene` when you mean the locus and a `Molecule` when
you mean the protein; write the one the source means, and if the source means both,
that is two nodes joined by `encodes`.

**Step 5 — The Disease / Phenotype / BiomedicalMeasure trio.** These are three distinct
facets, not three names for one thing, and conflating them is the commonest typing
error:

- `Disease` — a named clinical entity: *type 2 diabetes*.
- `Phenotype` — an observable trait or sign: *hyperglycaemia*.
- `BiomedicalMeasure` — a quantity you can measure: *fasting plasma glucose*.

One node each, joined by edges (`has_phenotype`, `measures`), rather than one node
wearing three hats.

**Step 6 — Coarsest still-useful granularity.** Prefer the type that keeps the node
reusable, and put the specificity in `subtype` (free text, never validated). A
monoclonal antibody is a `Molecule` with `subtype: monoclonal-antibody`, not a type of
its own.

**Step 7 — Nothing fits.** Use `Other` and add a note saying what it is. Never invent a
29th type: an invented type is silently unexchangeable, whereas `Other` is honestly
unclassified. If you reach `Other` more than occasionally, the base is probably not
biomedical and should have been OKF.

## Choosing a predicate

The 24 positives, by group:

- **structural** — `is_a`, `part_of`, `member_of`, `derives_from`
- **spatial** — `located_in`, `expressed_in`
- **molecular / functional** — `encodes`, `interacts_with`, `binds`, `regulates`,
  `catalyzes`, `converts_to`, `participates_in`
- **clinical / causal** — `causes`, `predisposes_to`, `treats`, `prevents`,
  `contraindicated_in`, `affects_response_to`, `has_phenotype`
- **measurement / association / provenance** — `measures`, `associated_with`,
  `used_to_study`, `reported_in`

Rules that decide the hard cases:

- **Direction is fixed. There are no inverse predicates.** Author `encodes` on the gene,
  never `encoded_by` on the protein. If the edge seems to run the wrong way, you are
  writing it on the wrong page.
- **Pick the most specific one that the source actually supports.**
  `associated_with` is the honest predicate for a correlation and the lazy one for
  everything else. If the source shows causation, say `causes`; if it shows a
  correlation, do not.
- **Negation is a claim, not an absence.** Eleven predicates take a `not_` prefix —
  `expressed_in`, `interacts_with`, `binds`, `regulates`, `causes`, `predisposes_to`,
  `treats`, `prevents`, `affects_response_to`, `has_phenotype`, `associated_with`. Use
  `not_treats` for an explicit negative finding, with its own full provenance. Never
  assert `<X>` and `not_<X>` between the same two nodes; that is a lint error, and the
  fix is to record the contradiction in prose and keep the better-evidenced edge.
- **`is_a` is for taxonomy, not for description.** Aspirin `is_a` NSAID. Aspirin is not
  `is_a` "useful drug".

## The provenance triplet

Every edge — including a `not_` edge and including `reported_in` — carries all three:

- `knowledge_level` — one of `knowledge_assertion`, `statistical_association`,
  `prediction`, `observation`, `not_provided`.
  Match it to what the source did: an assertion the authors make is
  `knowledge_assertion`; a correlation they measured is `statistical_association`; a
  model output is `prediction`. **Never silently elevate one to another** — that is how
  a base stops being trustworthy while still looking tidy.
- `agent_type` — one of `manual_agent`, `automated_agent`, `text_mining_agent`,
  `data_analysis_pipeline`, `computational_model`, `not_provided`.
  When you extracted the claim by reading the source, that is `text_mining_agent`.
- `primary_source` — the **`identifier` of a source node that exists in this bundle**.
  Not a CURIE, not `infores:…`, not a file path. If you want to cite HGNC, create HGNC
  once as an `Agent` node with its CURIE in `xref`, and cite that node's identifier.

## Sources are linked twice

Provenance runs through two mechanisms and a page needs both:

1. `primary_source` on every edge — which source attests *this* claim.
2. a `reported_in` edge from the page to that source node — the traversable link from a
   concept to where it was reported.

A source node's own `reported_in` edge cites **itself** as its `primary_source`. That
self-reference is the intended terminating case, not an error, and lint knows it.

## The numbers go on the edge

Any quantity the source reports belongs in the edge's own attributes, not only in the
prose: `effect_metric`, `effect_size`, `ci_lower`, `ci_upper`, `p_value`,
`adjusted_p_value`, `standard_error`, `sample_size`, `sensitivity`, `specificity`,
`auc`, `frequency`, `clinical_phase`, `response_direction`, `unit`. Write the value the
source gives, including `<0.001` — a number you cannot write exactly is still better
recorded than dropped.

Keep **context** separate from **measurement**: `species_context`, `sex`, `age_group`
and `timepoint` qualify the claim, they do not measure it. A p-value filed as context is
a category error a renderer cannot undo.

## A worked page

```yaml
---
type: Molecule
identifier: Tocilizumab
subtype: monoclonal-antibody
synonyms: [Actemra, RoActemra]
xref: [DRUGBANK:DB06273]
edges:
  - predicate: treats
    object: COVID-19
    knowledge_level: statistical_association
    agent_type: data_analysis_pipeline
    primary_source: RECOVERY trial
    effect_metric: relative_risk
    effect_size: 0.85
    ci_lower: 0.76
    ci_upper: 0.94
  - predicate: binds
    object: Interleukin-6 receptor
    knowledge_level: knowledge_assertion
    agent_type: manual_agent
    primary_source: DrugBank
  - predicate: reported_in
    object: RECOVERY trial
    knowledge_level: knowledge_assertion
    agent_type: text_mining_agent
    primary_source: RECOVERY trial
---

# Tocilizumab

An IL-6 receptor antagonist, trialled in severe COVID-19.
```

## Pitfalls

- **`identifier` is human-readable and bundle-unique, not a CURIE.** CURIEs go in
  `xref`. Two pages with the same `identifier` is an error; an `object` naming nothing
  is a dangling edge. `kb_validate_page` catches both before the write.
- **Only `edges:` entries carry a predicate and provenance.** Bundle-local markdown
  links and `[[…]]` links do become graph edges too, but *untyped* ones — so a
  relationship you want typed, attributed and queryable has to go in `edges:`. A typed
  entry also wins the tie-break against a prose link that restates it.
- **Quote any YAML scalar containing `: ` (colon-space)** or the frontmatter will not
  parse. `identifier: Chen 2020 (IL-6 and severe COVID-19)` is fine; anything with a
  colon-space needs quotes.
- **Do not write a `primary_source` you have not materialised.** A dangling
  `primary_source` is the same failure as a dangling `object`, one layer down.
- **Do not overwrite a page that contradicts the new source.** Add an
  `## Open contradictions` section naming both positions and their sources, or assert
  the `not_<X>` edge with its own provenance.
