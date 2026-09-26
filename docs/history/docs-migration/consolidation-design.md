# Docs consolidation design

> **What this is.** The design for merging BioRouter's two competing documentation trees — the hand-written `documentation/` folder and the Docusaurus-generated `docs/` site — into a single plain-markdown `docs/` folder, dropping the Docusaurus tooling, purging Goose/Block branding, and renaming `recipe` → `workflow` throughout.
> **Status:** Historical record — approved 2026-05-07 and executed. `documentation/` no longer exists and the target tree (`docs/getting-started/`, `docs/guides/`, `docs/extensions/`, `docs/troubleshooting/`) was built as specified. The companion task-by-task plan is [docusaurus-to-markdown-plan.md](docusaurus-to-markdown-plan.md).
> **Audience:** maintainers, and anyone tracing why a documentation page lives where it does.

Three terms this document assumes. **Goose** is the open source agent project whose design influenced BioRouter most; **Block** is the company that publishes Goose. Both appear throughout the old pages as branding that had to be replaced. **Docusaurus** is the React-based static-site generator that produced the old `docs/` tree — its output mixes JSX components into Markdown (MDX) and ships generated HTML, CSS, feeds and media alongside the source, none of which belongs in a plain documentation folder.

Despite the `-design` filename, the body below is a migration checklist: file-by-file move tables, a deletion list, the text transformations to apply, and the verification commands. The executable step-by-step version is the companion plan linked above.

> **Note.** The target paths in the tables below describe the layout as it stood after this migration, in May 2026. `docs/` was reorganized again in July 2026, so those paths are a record of the intended destination at the time rather than links you can follow today.

## Migration goal

Merge `documentation/` and `docs/` into a single, clean, plain-markdown `docs/` folder. Drop the Docusaurus infrastructure entirely. Purge all Goose/Block branding. Rename `recipe` → `workflow` throughout. Keep only built-in BioRouter extensions; discard all third-party MCP pages.

## Source inventory

### `documentation/` — 7 authoritative plain-markdown files (zero Goose references)

| File | Target |
| --- | --- |
| `architecture.md` | `docs/architecture/overview.md` |
| `data-privacy.md` | `docs/guides/data-privacy.md` |
| `extensions-skills-mcp.md` | `docs/guides/extensions-skills.md` |
| `installation-setup.md` | `docs/getting-started/installation.md` |
| `providers-and-models.md` | `docs/getting-started/providers.md` |
| `recipes.md` | `docs/guides/workflows/index.md` (recipe→workflow) |
| `schedulers.md` | `docs/guides/schedulers.md` |

### `docs/docs/` — selected pages to carry forward (strip JSX, fix branding)

| Source | Target |
| --- | --- |
| `docs/quickstart.md` | `docs/getting-started/quickstart.md` |
| `biorouter-architecture/extensions-design.md` | `docs/architecture/extensions-design.md` |
| `biorouter-architecture/error-handling.md` | `docs/architecture/error-handling.md` |
| `guides/config-files.md` | `docs/guides/config-files.md` |
| `guides/biorouter-cli-commands.md` | `docs/guides/cli-commands.md` |
| `guides/biorouter-permissions.md` | `docs/guides/permissions.md` |
| `guides/sessions/index.md` | `docs/guides/sessions.md` |
| `guides/context-engineering/index.md` | `docs/guides/context-engineering.md` |
| `guides/environment-variables.md` | `docs/guides/environment-variables.md` |
| `guides/security/index.md` | `docs/guides/security.md` |
| `guides/subagents.md` | `docs/guides/subagents.md` |
| `guides/tips.md` | `docs/guides/tips.md` |
| `guides/recipes/recipe-reference.md` | `docs/guides/workflows/reference.md` |
| `guides/recipes/subrecipes.md` | `docs/guides/workflows/subworkflows.md` |
| `guides/recipes/session-recipes.md` | `docs/guides/workflows/session-workflows.md` |
| `guides/recipes/storing-recipes.md` | `docs/guides/workflows/storing-workflows.md` |
| `troubleshooting/index.md` | `docs/troubleshooting/index.md` |
| `troubleshooting/known-issues.md` | `docs/troubleshooting/known-issues.md` |
| `troubleshooting/diagnostics-and-reporting.md` | `docs/troubleshooting/diagnostics.md` |
| `mcp/developer-mcp.md` | `docs/extensions/developer.md` |
| `mcp/computer-controller-mcp.md` | `docs/extensions/computer-controller.md` |
| `mcp/memory-mcp.md` | `docs/extensions/memory.md` |
| `mcp/tutorial-mcp.md` | `docs/extensions/tutorial.md` |
| `mcp/autovisualiser-mcp.md` | `docs/extensions/auto-visualiser.md` |
| `mcp/skills-mcp.md` | `docs/extensions/skills.md` |
| `mcp/extension-manager-mcp.md` | `docs/extensions/extension-manager.md` |
| `mcp/chatrecall-mcp.md` | `docs/extensions/chat-recall.md` |
| `mcp/code-execution-mcp.md` | `docs/extensions/code-execution.md` |
| `mcp/todo-mcp.md` | `docs/extensions/todo.md` |

## Target structure

```text
docs/
  getting-started/
    installation.md
    providers.md
    quickstart.md
  architecture/
    overview.md
    extensions-design.md
    error-handling.md
  guides/
    data-privacy.md
    extensions-skills.md
    schedulers.md
    config-files.md
    cli-commands.md
    permissions.md
    sessions.md
    context-engineering.md
    environment-variables.md
    security.md
    subagents.md
    tips.md
    workflows/
      index.md
      reference.md
      subworkflows.md
      session-workflows.md
      storing-workflows.md
  extensions/
    developer.md
    computer-controller.md
    memory.md
    tutorial.md
    auto-visualiser.md
    todo.md
    skills.md
    extension-manager.md
    chat-recall.md
    code-execution.md
  troubleshooting/
    index.md
    known-issues.md
    diagnostics.md
  superpowers/             (untouched)
    plans/
    specs/
```

## What gets deleted

All of the following are removed from `docs/`:

- All generated HTML files (`*.html`) and `*/index.html` subdirectories
- `docs/blog/` — blog posts not relevant to app docs
- `docs/community/` — external community pages
- `docs/audio/` — `elevenlabs-mcp-demo.mp3` (third-party demo, not built-in)
- `docs/videos/` — all `.mp4` files (hero videos, demo clips)
- `docs/assets/` — Docusaurus CSS, hashed images, media assets
- `docs/deeplink-generator/`, `docs/recipe-generator/`, `docs/prompt-library/`
- `docs/grants/`, `docs/extension/`, `docs/files/`, `docs/v1/`
- `docs/inkeepChatButton.js`, `docs/inkeepSearchBar.js`
- `docs/sitemap.xml`, `docs/robots.txt`, `docs/atom.xml`, `docs/atom.css`, `docs/atom.xsl`
- `docs/rss.xml`, `docs/rss.css`, `docs/rss.xsl`
- `docs/.nojekyll`, `docs/llms.txt`, `docs/servers.json`
- `docs/docs/mcp/` — all 50+ third-party MCP pages (not built-in to BioRouter)
- `docs/docs/blog/`, `docs/docs/experimental/`, `docs/docs/tutorials/`
- `docs/docs/guides/` subdirectories not in the carry-forward list
- `documentation/` folder entirely (all content merged into `docs/`)

## Content transformations

Applied uniformly to every file carried forward.

### Strip JSX and MDX

Remove all React component syntax and Docusaurus-specific markup:

- `<Card ... />`, `<div className={...}>`, `className={styles.xxx}`
- YouTube `<iframe>` embeds and `<div className="video-container">` wrappers
- Import statements (`import Card from ...`, `import styles from ...`)
- Frontmatter fields only used by Docusaurus (`sidebar_label`, `sidebar_position`, `custom_edit_url`, `pagination_prev`, `pagination_next`)

Replace card grids with plain markdown link lists. Drop embedded videos entirely (link to external resource in text if essential).

### Replace Goose and Block branding

| Find (case-insensitive) | Replace with |
| --- | --- |
| `goose` / `Goose` / `GOOSE` | `BioRouter` |
| `geese` / `Geese` / `GEESE` | `BioRouter` |
| `block.xyz`, `block.github.io` | `https://github.com/BaranziniLab/biorouter` |
| `sq.github.io/goose` | `https://github.com/BaranziniLab/biorouter` |
| "Block" as a company name | remove sentence or rewrite without company attribution |

### Rename recipe to workflow

| Find | Replace |
| --- | --- |
| `recipe` / `Recipe` | `workflow` / `Workflow` |
| `recipes` / `Recipes` | `workflows` / `Workflows` |
| `sub_recipe` | `sub_workflow` |
| `subrecipe` / `subrecipes` | `subworkflow` / `subworkflows` |
| `BIOROUTER_RECIPE_PATH` | `BIOROUTER_WORKFLOW_PATH` |
| `BIOROUTER_RECIPE_GITHUB_REPO` | `BIOROUTER_WORKFLOW_GITHUB_REPO` |
| `biorouter recipe validate` | `biorouter workflow validate` |
| `biorouter run --recipe` | `biorouter run --workflow` |
| `.biorouter/recipes/` | `.biorouter/workflows/` |
| `~/.config/biorouter/recipes/` | `~/.config/biorouter/workflows/` |

### Clean up paths and links

- Update all internal doc cross-links to reflect new file paths
- Remove links to deleted pages (blog posts, external tools, Docusaurus-only routes)

## Out of scope

- Custom/domain-specific extensions: OMOP agent, SPOKE agent, CDW agent — not included
- Experimental features (`biorouter-mobile`, `vs-code-extension`) — not carried forward
- Tutorial pages — dropped (can be recreated fresh if needed)
- `docs/superpowers/` — untouched; plans and specs remain as-is

## Verification criteria

These were written before the migration ran, as the checks it had to satisfy afterwards:

1. `find docs/ -name "*.html"` → zero results
2. `find docs/ -name "*.mp4" -o -name "*.mp3"` → zero results
3. `grep -ri "goose\|geese" docs/ --include="*.md"` → zero results (outside `superpowers/`)
4. `grep -ri "recipe" docs/ --include="*.md"` → zero results (outside `superpowers/`)
5. `ls docs/docs/mcp/` → directory does not exist
6. `ls documentation/` → directory does not exist
7. Every file in `docs/` (outside `superpowers/`) is a `.md` file

> **Note.** This document does not record the run's output. The seven criteria were encoded as a runnable script, `scripts/verify-docs.sh`, which is still in the repository; see Task 1 of [docusaurus-to-markdown-plan.md](docusaurus-to-markdown-plan.md) for how it was built and Task 5 for how failures were worked through.

## Related documentation

- [Docusaurus-to-markdown migration plan](docusaurus-to-markdown-plan.md) — the task-by-task execution of this design, including the migration script and verification suite
- [System overview](../../architecture/system-overview.md) — the page `documentation/architecture.md` became
- [Workflows](../../workflows/README.md) — where `documentation/recipes.md` landed after the recipe→workflow rename
- [Troubleshooting](../../troubleshooting/README.md) — the carried-forward troubleshooting section
- [Extension trait design](../legacy-architecture/extension-trait-design.md) — one of the Docusaurus architecture pages carried forward here
