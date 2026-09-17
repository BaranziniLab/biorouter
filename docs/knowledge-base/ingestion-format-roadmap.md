# Knowledge ingestion format roadmap

> **What this is.** A survey of the knowledge-base conversion pipeline as it stood on
> 2026-06-10, a licensed comparison of open-source document converters, and a phased plan
> to extend ingestion to PowerPoint, Excel/ODS, and a higher-fidelity PDF path.
> **Status:** Current — partially implemented. Phases 1, 2, 3 and 4.1 shipped and were
> verified on 2026-06-10; Phases 4.2, 4.3 and 5 are open work.
> **Audience:** developers working on the knowledge base and its conversion layer.

The knowledge base ingests a file, converts it deterministically to markdown, then lets a
bounded sub-agent digest that markdown into wiki pages. Conversion happens once and is
immutable, so whatever a converter loses is lost for good. This document asks what the
converters were losing in June 2026 and what to do about it.

**Phase numbering.** `Phase N` and `Phase N.M` are identifiers local to this document —
there is no external index. They are cited by the status block above and are the units the
work was tracked in. All phase definitions live under
[Plan for extending ingestion](#plan-for-extending-ingestion) below.

**Three genres under one heading**, in reading order:

| Section | Genre | Still actionable? |
|---|---|---|
| [Current architecture](#current-architecture-as-built) | Architecture survey, as built on 2026-06-10 | Reference; the known-gaps list is a snapshot of that date |
| [Tool research summary](#tool-research-summary) | Competitive research, June 2026 data | Reference; licensing conclusions need re-checking before reuse |
| [Plan for extending ingestion](#plan-for-extending-ingestion) | Forward plan | Phases 4.2, 4.3 and 5 only |

**What shipped on 2026-06-10:** xlsx/xls/ods via `calamine`; pptx via an in-house
converter with speaker notes; the PDF primary path upgraded to `pdf-inspector` with the
legacy chain kept as fallback; and a scanned-PDF quality banner in `source.md`. Evaluated
on real two-column papers (BERT, Attention), `pdf-inspector` ran 4–35× faster than
`pdf-extract` and recovered 20 headings and 100+ table rows where `pdf-extract` recovered
zero, with no content loss.

**What remains:** Phase 4.2 (vision-LLM fallback), Phase 4.3 (optional Docling sidecar),
and Phase 5 (long-tail formats).

**Original scope statement.** Expand knowledge-base ingestion beyond the then-current
PDF/HTML/DOCX/CSV set to PowerPoint (`.pptx`), Excel (`.xlsx`/`.xls`/`.ods`), and a
higher-fidelity PDF pipeline — based on a survey of the current architecture and a
comparison of open-source conversion tools.

---

## Current architecture (as built)

The ingestion pipeline is **conversion-then-digestion**:

```text
Frontend (Dropzone / paste / URL)
  → POST /knowledge/bases/{id}/ingest   (multipart or JSON, SSE response)
    → KnowledgeService::add_raw_source
        → convert::convert()             ← deterministic format → markdown
        → raw/{source-id}/{original.*, source.md, meta.yaml}  (immutable, git-committed)
    → ingest macro sub-agent (bounded LLM loop, kb_* tools, txn branch)
        → knowledge/ wiki pages + [[links]] + log.md          (one atomic commit)
```

Key property: **conversion quality bounds digestion quality.** The sub-agent only ever
sees `source.md`; whatever the converter destroys (tables, reading order, slide notes) is
unrecoverable downstream.

### Conversion dispatch as of 2026-06-10

`crates/biorouter-mcp/src/knowledge/convert/mod.rs` (`convert()`, `normalize_mime()`,
`guess_mime()`):

| Format | Converter | Crate(s) | Fidelity notes |
|---|---|---|---|
| PDF | `pdf.rs` | `pdf-extract` → `lopdf` → `python3 pdfminer` subprocess → lossy content-op scan | Text-layer only. No OCR, no tables, no headings, no multi-column reading order. |
| HTML | `html.rs` | `htmd` | Good. |
| DOCX | `docx.rs` | `docx-rs` | Text + basic structure. |
| CSV | `csv.rs` | `csv` | Markdown table. 8 MB cap. |
| MD / TXT | passthrough | — | — |
| URL | `url_fetch.rs` | `reqwest` → re-dispatch | — |

### Known gaps found during the 2026-06-10 review

> **Note.** This list is a snapshot of the state that motivated the plan below, not a live
> defect register. Source references name symbols and files rather than line numbers, which
> drift.

1. **`needs_llm_fallback` is dead code.** `convert/pdf.rs` sets it for scanned PDFs
   (fewer than 32 characters extracted), `Converted` carries it, and *nothing consumes it* —
   no UI warning, no alternate path. Scanned PDFs silently ingest as empty.
2. **`.xls` is mis-mapped.** `normalize_mime()` in `convert/mod.rs` maps
   `application/vnd.ms-excel` → `text/csv`, so a real legacy Excel binary would be fed to
   the CSV parser and produce garbage.
3. **`.pptx`/`.xlsx` are blocked at three layers:** the frontend extension allowlist in
   `ui/desktop/src/components/knowledge/IngestPanel/fileValidation.ts`, `guess_mime()`
   falling through to `text/plain`, and `convert()` bailing with `unsupported mime`.
4. **`umya-spreadsheet` 2.2.3 is already a workspace dependency** (used by
   `webdocuments/xlsx_tool.rs`) — XLSX support has zero-new-dependency options.

Lint (`macros/lint.rs`: deterministic scan plus optional sub-agent autofix), query
(`macros/query.rs`: BM25 `kb_search` plus page reads, optional file-as-page), and the CLI
(`biorouter knowledge ingest|query|lint|...`, MCP-free, calling the same macros) all
operate downstream of `source.md` and need **no changes** for new formats — that is the
payoff of the convert-then-digest design.

---

## Tool research summary

Full agent research reports, June 2026 data. License flags matter because BioRouter dmgs
and zips are distributed artifacts.

> **Warning.** The licensing and weight figures below were accurate as of June 2026 and
> have not been re-verified since. Re-check the license of any tool before adopting it.

### Shorthand used in the tool tables

| Term | Meaning |
|---|---|
| `olmOCR-bench`, `OmniDocBench` | Published third-party benchmark suites for document-to-markdown conversion quality. Scores quoted below come from those publications, not from BioRouter measurements. |
| `TableFormer` | The table-structure recognition model inside Docling. |
| `CRF image` | The GROBID Docker image, which uses conditional-random-field (classical sequence-labelling) models rather than a neural vision model. |
| `VLM` | Vision-language model — an LLM that reads page images directly. |

### PDF to markdown

| Tool | Lang | License | Weight | Quality vs `pdf-extract` |
|---|---|---|---|---|
| **pdf-inspector** (Firecrawl) | Rust, MIT | crate, no models | ~150 ms/doc | Headings (font-size), lists, tables (dual heuristic), **multi-column reading order**, bold/italic, plus a 10–50 ms text-vs-scanned classifier. Best lightweight upgrade. |
| **pdf_oxide** | Rust, MIT/Apache-2 | crate | very fast | Similar feature set, claims 5× faster than pdf-extract; young (Nov 2025), single author, 0.3.x churn — A/B candidate, not primary. |
| **Docling** (IBM/LF AI) | Python, **MIT** | sidecar; ~100s of MB models, CPU-capable | 2–6 s/page CPU | Layout ML + TableFormer + pluggable OCR + formula recognition. Best *cleanly licensed* heavyweight. `docling-serve` HTTP API fits biorouterd's architecture. |
| MinerU | Python, custom (Apache-base) | ~20 GB | — | OmniDocBench pipeline leader (tables, CJK, formulas). Too heavy + non-standard license. |
| Marker / Surya | Python, **GPL-3 + restricted weights** (<$2M commercial cap) | GBs, GPU-preferring | — | olmOCR-bench best pipeline (76.1) but license disqualifies distribution. |
| olmOCR | Python, Apache-2 | 7B VLM, ≥12 GB VRAM | — | Top open scores (82.4); cloud-scale only. |
| PyMuPDF4LLM / mutool | **AGPL** | — | — | Best no-ML quality, license disqualifies. |
| MarkItDown (PDF path) | Python, MIT | — | — | pdfminer plain text — **no better than what we have**. |
| GROBID (CRF image) | Java, Apache-2 | ~470 MB Docker, CPU | 2.5–10 PDF/s | Not a markdown converter; best-in-class scholarly **reference/metadata** extraction (TEI XML). Niche future add-on for biomedical citations. |
| extractous | Rust+GraalVM Tika | Apache-2 | large native libs | Plain text out, stale 18 mo. Pass. |

Benchmarks (olmOCR-bench, OmniDocBench): GPU VLMs > Marker/MinerU > Docling > heuristic
tools > plain text extraction. None of the published benchmarks cover the young Rust
crates — we need a small internal eval.

**Honest delta vs current method:** for digitally-born papers, body *words* mostly survive
today; the real losses are (a) multi-column reading order (pdf-extract interleaves columns
— actively corrupts meaning), (b) tables (destroyed; the single biggest faithfulness loss
for biomedical results), (c) headings (lost structure the sub-agent could use to split
pages), (d) scanned PDFs (currently 0% — common in older biomedical literature).
`pdf-inspector` recovers roughly the first three at zero weight; only the Docling/OCR tier
recovers scanned docs and complex tables/equations.

### XLSX to markdown

- **calamine** (2.3k★, MIT, pure Rust, active): the clear winner. Reads
  **xlsx/xlsm/xlsb/xls + ods**; deps are `zip` + `quick-xml` (already in tree);
  `worksheet_range()` returns **cached computed values** (what an LLM wants — no formula
  recalculation problem), `worksheet_formula()` available separately; lazy sheet loading,
  ~1.1M cells/s. Markdown glue is ~60–80 lines (same shape as `csv.rs`); prior art: the
  `madato` crate (copy the pattern, skip the dependency).
- `umya-spreadsheet` (already in tree): editing-oriented, heavier object model, slower
  reads, xlsx-only. Fine fallback; calamine is better *and* fixes `.xls`/`.ods` for free.
- markitdown/pandas sidecar: output essentially identical to the calamine path — not worth
  a Python runtime for this format. (pandoc **cannot** read xlsx at all.)

### PPTX to markdown

The sparse one — no Rust crate currently extracts **speaker notes**:

- **pptx-to-md** (MIT/Apache-2, deps already ~all in tree): slide text, lists, tables,
  images; **no speaker notes**, document-order only, 5★ single-author — vendor/fork
  material, not a load-bearing dependency.
- **undoc** (MIT, pure Rust, May 2026): pptx + xlsx + docx → md, tables, slide markers;
  notes undocumented; 20★, fast churn — re-evaluate in 6 months.
- **markitdown** (150k★, MIT, Python): the quality gold standard — titles, tables, charts,
  **speaker notes**, image alt-text, (top,left) reading-order sort. Cost: Python runtime
  plus python-pptx; viable only as an optional subprocess/sidecar, not bundled.
- pandoc: pptx is *writer-only* (issues #4252, #7621) — confirmed no help.
- **In-house extractor feasibility: high.** PPTX is a zip of XML. Slide text:
  `ppt/slides/slide{N}.xml` (`<a:t>` runs in `<a:p>` in `<p:txBody>`); tables:
  `<a:tbl>/<a:tr>/<a:tc>`; **speaker notes: `ppt/notesSlides/notesSlide{N}.xml`** (resolve
  via slide `_rels`); slide order: `ppt/presentation.xml` `sldIdLst`. ~300–500 lines with
  `zip` + `quick-xml` (both already deps) — and it is the only pure-Rust route to speaker
  notes, which for lab-meeting and conference decks often carry more knowledge than the
  slides.

Known limits of any pure-Rust pptx path: SmartArt text (`diagrams/data{N}.xml`), chart
data series, grouped/rotated shape ordering. Acceptable: the sub-agent digests content, it
does not reproduce layout.

---

## Plan for extending ingestion

Guiding principles:

- **Pure-Rust, in-process converters as the always-works default** (matching the
  pdf-extract/htmd/docx-rs/csv precedent; nothing new for users to install).
- **Optional high-fidelity sidecar later**, never bundled Python.
- Every converter returns `Converted { markdown, title, mime, needs_llm_fallback }` and is
  exercised by fixture tests under `convert/fixtures/`.

### Phase 1 — XLSX/XLS/ODS via calamine (small, ships first)

1. Add `calamine` to `biorouter-mcp/Cargo.toml`.
2. New `convert/xlsx.rs`: per sheet → `## <sheet name>` plus a markdown table from
   `worksheet_range()` (cached values); escape `|`; skip empty sheets; cap output (e.g.
   500 rows or ~200k cells per sheet, then `> … truncated: N more rows`) using
   `range.get_size()`; title from filename.
3. Dispatch in `convert/mod.rs`:
   - `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` → xlsx
   - **fix the `.xls` bug**: route `application/vnd.ms-excel` to calamine (it reads legacy
     xls) instead of the CSV parser; keep `text/csv` → `csv.rs`
   - `application/vnd.oasis.opendocument.spreadsheet` → calamine ods
   - extend `guess_mime()` for `.xlsx`/`.xlsm`/`.xls`/`.ods`.
4. Frontend: add extensions to `fileValidation.ts` (with a size cap — reuse the 8 MB
   CSV-class limit for spreadsheets) and to the Dropzone label; mirror the cap in
   `validate_ingest_upload()` in `crates/biorouter-server/src/routes/knowledge.rs`.
5. Tests: fixtures (multi-sheet xlsx, xls, ods, formula workbook → cached values,
   huge-sheet truncation) in `convert/fixtures/`; run
   `cargo test -p biorouter-mcp --lib knowledge::`.

No HTTP route shape changes, so no OpenAPI regeneration is needed (re-check if any request
fields are added).

### Phase 2 — PPTX via in-house extractor (zip + quick-xml)

1. New `convert/pptx.rs` (~400 lines):
   - slide order from `presentation.xml` plus rels;
   - per slide: `# Slide N — <title placeholder>`, paragraphs/lists from text bodies,
     `<a:tbl>` → markdown tables;
   - **speaker notes** from `notesSlides/` rels → `> **Notes:** …` block;
   - skip images/SmartArt with an explicit `*[figure omitted]*` marker so the sub-agent
     knows content was elided (a candidate for the Phase 4 vision path).
   - Reference implementations: the `pptx-to-md` crate (vendorable, compatible license)
     and markitdown's `_pptx_converter.py` (reading-order sort).
2. Dispatch plus `guess_mime()` for `.pptx`
   (`application/vnd.openxmlformats-officedocument.presentationml.presentation`); legacy
   `.ppt` → explicit error "please re-save as .pptx" (no viable pure-Rust OLE2 reader
   today; `litchi` is the crate to watch).
3. Frontend and server validation as in Phase 1 (25 MB cap).
4. Fixture tests: titled deck, deck with tables, deck with notes, empty deck.

### Phase 3 — PDF upgrade to structured markdown (pdf-inspector)

1. Internal eval first (cheap and decisive): ~25 PubMed Central PDFs (multi-column,
   results tables, references) through the current chain vs `pdf-inspector` vs `pdf_oxide`;
   score heading, table and reading-order retention; reuse the lint macro or a one-off
   harness for judging.
2. Adopt the winner (expected: `pdf-inspector`, MIT) as **primary** in
   `extract_pdf_text()`'s primary slot; keep the existing pdf-extract → lopdf → pdfminer →
   lossy chain as fallback (the `extract_pdf_text_with` plumbing already supports exactly
   this).
3. Use its text-vs-scanned classifier to set `needs_llm_fallback` deterministically instead
   of the fewer-than-32-characters heuristic.

### Phase 4 — make `needs_llm_fallback` real (scanned PDFs, figures)

Today the flag is produced and dropped. In order of effort:

**Phase 4.1 — surface it.** When set, emit an SSE warning event during ingest and prepend a
frontmatter note to `source.md` (`extraction_quality: poor — scanned or image-based`), so
users and the sub-agent stop silently ingesting empty sources.

**Phase 4.2 — vision-LLM fallback.** BioRouter already routes multimodal providers —
rasterize pages (`pdfium-render`, MIT-licensed bindings over BSD Pdfium) and have the
existing completer transcribe page images to markdown, bounded by page count. This handles
scanned biomedical PDFs with zero new user-visible dependencies beyond the bundled pdfium
dylib.

**Phase 4.3 — optional Docling sidecar.** Config-gated, with a user-installed
`docling-serve` URL in `config.yaml`: when configured, scanned or complex PDFs POST to it
for OCR plus TableFormer-grade conversion. Clean MIT license; never bundled. GROBID
(Apache-2, CPU CRF image) is a further optional add-on for reference and metadata
extraction if citation-graph features materialize.

### Phase 5 — cheap long-tail formats (opportunistic)

- `.epub`: zip plus the existing `htmd` (~50 lines).
- `.rtf`: the `rtf-parser` crate (pure Rust) → text.
- `.odp`: the same zip-plus-XML approach as pptx if demand appears.
- Plain images (`.png`/`.jpg` of figures and posters): the Phase 4.2 vision-LLM path makes
  this nearly free.

### Acceptance criteria

- `.xlsx`/`.xls`/`.ods`/`.pptx` drag-and-drop ingests end-to-end (GUI dropzone,
  `biorouter knowledge ingest --file`, URL path) with faithful tables, sheet names, and
  speaker notes in `raw/*/source.md`.
- No regression in the existing ~122 `knowledge::` tests plus ~19 route tests.
- Scanned-PDF ingestion either produces real text (Phase 4) or a visible quality warning —
  never a silent empty source.
- All new dependencies MIT/Apache-2; no Python or model downloads in the default install.

## Related documentation

- [Plan 1 — storage, git and graph](../history/knowledge-base-buildout/plan-1-storage-git-and-graph.md) — the conversion and raw-source foundation this roadmap extends; read it first for how `convert()` and `raw/` came to be.
- [Founding design](../history/knowledge-base-buildout/founding-design.md) — why the knowledge base is convert-then-digest at all.
- [Plan 3 — HTTP routes and export](../history/knowledge-base-buildout/plan-3-http-routes-and-export.md) — defines the `/knowledge/bases/{id}/ingest` SSE endpoint the new formats travel through.
- [Plan 4 — knowledge view and ingest](../history/knowledge-base-buildout/plan-4-knowledge-view-and-ingest.md) — the dropzone and `fileValidation.ts` allowlist that each new format must be added to.
