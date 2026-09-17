# Built-in office context compatibility

## Scope

Provide original Biorouter instructions for Word, PowerPoint, Excel, and PDF work.
Each context is seeded by the existing built-in skill registry and exposed under
Settings → Chat → Contexts. No new service, native renderer, or Python runtime is
bundled. The contexts operate with the tools and permissions available to the agent.

## Comparison of the inspected packages

| Format | Anthropic local package | Codex local package | Biorouter implementation |
| --- | --- | --- | --- |
| Word | docx-js and packaged XML helpers | Host runtime, document helpers, render workflow | python-docx for ordinary documents; explicit OOXML/native-app limits |
| PowerPoint | Packaged PPTX authoring/editing helpers | Host presentation runtime and artifact tools | python-pptx and optional LibreOffice rendering |
| Excel | Python spreadsheet workflows and recalculation helpers | Artifact Tool and host spreadsheet APIs | openpyxl plus real-engine recalculation |
| PDF | Python tools, OCR, PDF operations and helpers | Python tools, host operation markers and citations | pypdf/reportlab/pdfplumber with local rendering |

The inspected local vendor packages include host-specific dependencies and license
restrictions on redistribution. This change includes neither vendor instructions
nor scripts, assets, or private runtimes. The new contexts are original instructions
using public Python library interfaces. It does not claim that a vendor package has
been ported wholesale or that native Office applications are bundled.

Codex operation markers, workspace dependency loaders, special citation directives,
and Claude-specific tool/identity assumptions are not Biorouter interfaces. Outputs
use Biorouter's file links and available Developer tools. When enabled, existing
Computer Controller DOCX/XLSX/PDF tools handle supported lightweight operations;
the contexts describe their limits and use portable libraries for richer authoring. Dependencies are discovered
and installed only as needed in isolated environments through normal permissions.

## Loading contract

Only identifiers and concise routing guidance appear at session startup. Full bodies
arrive through loadSkill for relevant work, explicit skill references, or a workflow's
skills list. Selection by the model is conditional guidance, not a hard keyword gate.
It supports natural language and multiple formats without making ordinary words
like “word” inject a document workflow automatically.

Hidden contexts receive no routing hint. Per-chat revocation and tool-roster checks
remain intact. Built-in identity prevents package removal and ordinary skill counts
continue to exclude contexts. Existing contexts and user-authored skills are preserved.

## Supported baseline and limits

- Create/read/edit ordinary DOCX paragraphs, tables, styles, and page settings.
- Create/read/edit PPTX slides, editable text, tables, charts, and notes.
- Create/read/edit XLSX data, formulas, formatting, validation, and charts.
- Create/read and perform ordinary PDF page/form operations.
- Inspect outputs after reopening, recalculate formulas using an available engine,
  and render previews when a renderer and image-view tool are available.

Macros, SmartArt, complex tracked changes, advanced native Excel features, XFA forms,
signatures, and secure redaction need specialized paths and explicit verification.
Library round trips and LibreOffice rendering do not prove native Office fidelity.

## Verification

- Rust skills tests cover seeding, discovery, on-demand loading, compact startup
  routing, and session revocation. Existing workflow/context-state tests cover the
  shared workflow and settings paths.
- Desktop context tests compare the UI registry with the Rust shipped registry.
- `uv run --with python-docx --with python-pptx --with openpyxl --with reportlab
  --with pypdf python scripts/test-office-contexts.py --output <new-directory>`
  executes the context examples, reopens files, edits them, checks recalculation
  and renders previews. It requires LibreOffice and Poppler on PATH.
- `biorouter run --workflow biorouter-self-test.yaml --params test_phases=office
  --params workspace_dir=<test-directory>` exercises the configured model and real
  Biorouter tool loop. Set `--params office_python=<venv-python>` to use a prepared
  environment, and run from the test workspace so file tools stay within scope.
  Inspect tool traces and output artifacts independently.
- Fresh-session positive requests should load only the relevant context. An unrelated
  request should load none. Model-directed behavior needs live checks in addition
  to deterministic tests.
