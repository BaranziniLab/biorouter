# Docusaurus-to-markdown migration plan

> **What this is.** The task-by-task execution plan that folded BioRouter's Docusaurus-generated `docs/` site and the hand-written `documentation/` folder into one plain-markdown `docs/` tree — writing a migration script, running it over 36 files, deleting the Docusaurus infrastructure, and verifying the result.
> **Status:** Historical record — written 2026-05-07 and carried out. `documentation/` no longer exists, `docs/` is plain markdown in the getting-started / architecture / guides / extensions / troubleshooting layout this plan creates, and the throwaway `scripts/migrate-docs.py` was removed by Task 6 as designed. `scripts/verify-docs.sh` survives.
> **Audience:** maintainers and agents tracing how a documentation page reached its current path.

Two terms this plan assumes. **Docusaurus** is the React-based static-site generator that produced the old `docs/` tree; its output mixes JSX components into Markdown (MDX) and ships generated HTML, feeds and media beside the source. **Goose** is the open source agent project whose design influenced BioRouter most, published by the company **Block**. Both appear as branding in the old pages, and the migration replaces that branding with BioRouter.

The design this plan implements is [consolidation-design.md](consolidation-design.md), its sibling in this folder: that document holds the file-by-file move tables, the deletion list and the transformation rules, while this one holds the runnable steps.

> **Note.** Every command below hardcodes `/Users/wgu/Desktop/biorouter`, the author's checkout at the time of the run. To replay any step, substitute the path of your own clone.

> **Note.** Tasks 1 and 2 author `scripts/verify-docs.sh` and `scripts/migrate-docs.py` inline through `cat > … << 'EOF'` heredocs, so the script bodies below are a snapshot taken at authoring time, not the live source. `scripts/verify-docs.sh` has since been edited in the repository (its Goose check gained a `mongoose` false-positive filter) — treat the checked-in file as authoritative. `scripts/migrate-docs.py` was deleted by Task 6, Step 2, so the copy below is the only remaining record of it.

**How this plan was run:** as an agentic worker task, using the `superpowers:subagent-driven-development` sub-skill (`superpowers:executing-plans` was the alternative). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge `documentation/` and `docs/` into a single plain-markdown `docs/` folder, purging all Docusaurus infrastructure, Goose/Block branding, and recipe→workflow renaming.

**Architecture:** A Python migration script reads each source file, applies transformations (JSX stripping, branding replacement, recipe→workflow rename), and writes to the new location. A shell verification script acts as the test suite. All deletions happen after migration is confirmed complete.

**Tech Stack:** Python 3 (stdlib only), bash, standard Unix tools (`find`, `grep`, `rm`, `git`)

**Spec:** [consolidation-design.md](consolidation-design.md)

---

## File structure

```text
scripts/
  migrate-docs.py          CREATE — migration engine (JSX strip, branding, rename, copy)
  verify-docs.sh           CREATE — 7-criterion verification script (run before and after)
docs/
  getting-started/         CREATE — 3 files
  architecture/            CREATE — 3 files
  guides/                  CREATE — 12 files + workflows/ subfolder (5 files)
  extensions/              CREATE — 10 files
  troubleshooting/         CREATE — 3 files
  superpowers/             UNCHANGED
documentation/             DELETE after migration
docs/docs/                 DELETE after migration
docs/blog/                 DELETE
docs/community/            DELETE
docs/audio/                DELETE
docs/videos/               DELETE
docs/assets/               DELETE
docs/deeplink-generator/   DELETE
docs/recipe-generator/     DELETE
docs/prompt-library/       DELETE
docs/grants/               DELETE
docs/extension/            DELETE
docs/extensions/           NOTE: this is a different folder from docs/extensions/ we create — check carefully
docs/files/                DELETE
docs/v1/                   DELETE
docs/*.html                DELETE (404.html, index.html)
docs/sitemap.xml           DELETE (and all other Docusaurus root files)
```

> **Note.** The `docs/extensions/` line above flags a name collision and leaves it open. Task 4, Step 2 resolves it: a `docs/extensions/` directory already existed in the Docusaurus site, holding `index.html` and a `detail/` subdirectory. The migrated `.md` files land in the same directory, so that step deletes only the HTML artifacts and leaves the new Markdown in place. The folder is not deleted wholesale.

---

## Task 1: Write the verification script (tests first)

**Files:**

- Create: `scripts/verify-docs.sh`

- [ ] **Step 1: Create the verification script**

```bash
cat > /Users/wgu/Desktop/biorouter/scripts/verify-docs.sh << 'EOF'
#!/usr/bin/env bash
# Verification script for docs consolidation.
# All checks must pass (exit 0) when migration is complete.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
DOCS="$REPO/docs"
FAIL=0

check() {
  local desc="$1"; local cmd="$2"; local expect_empty="${3:-true}"
  echo -n "  CHECK: $desc ... "
  local result
  result=$(eval "$cmd" 2>/dev/null || true)
  if [ "$expect_empty" = "true" ] && [ -z "$result" ]; then
    echo "PASS"
  elif [ "$expect_empty" = "false" ] && [ -n "$result" ]; then
    echo "PASS"
  else
    echo "FAIL"
    [ -n "$result" ] && echo "    -> $result" | head -5
    FAIL=1
  fi
}

echo "=== BioRouter Docs Verification ==="

check "no .html files in docs/" \
  "find '$DOCS' -name '*.html' 2>/dev/null"

check "no .mp4/.mp3 files in docs/" \
  "find '$DOCS' \( -name '*.mp4' -o -name '*.mp3' \) 2>/dev/null"

check "no goose/geese references in markdown (outside superpowers/)" \
  "grep -ril 'goose\|geese' '$DOCS' --include='*.md' 2>/dev/null \
   | grep -v 'superpowers/' || true"

check "no recipe/recipes references in markdown (outside superpowers/)" \
  "grep -ril '\brecipe\b\|\brecipes\b' '$DOCS' --include='*.md' 2>/dev/null \
   | grep -v 'superpowers/' || true"

check "docs/docs/ directory does not exist" \
  "[ -d '$DOCS/docs' ] && echo 'exists' || echo ''" \
  "true"

check "documentation/ directory does not exist" \
  "[ -d '$REPO/documentation' ] && echo 'exists' || echo ''" \
  "true"

check "all files in docs/ (outside superpowers/) are .md" \
  "find '$DOCS' -not -path '*/superpowers/*' -type f ! -name '*.md' 2>/dev/null \
   | grep -v '/\.' || true"

echo ""
if [ "$FAIL" -eq 0 ]; then
  echo "ALL CHECKS PASSED"
  exit 0
else
  echo "SOME CHECKS FAILED"
  exit 1
fi
EOF
chmod +x /Users/wgu/Desktop/biorouter/scripts/verify-docs.sh
```

- [ ] **Step 2: Run verify script — expect failures (migration not done yet)**

```bash
cd /Users/wgu/Desktop/biorouter && bash scripts/verify-docs.sh || true
```

Expected output: several `FAIL` lines (HTML files exist, docs/docs/ exists, etc.)

---

## Task 2: Write the migration script

**Files:**

- Create: `scripts/migrate-docs.py`

- [ ] **Step 1: Create the script**

```bash
cat > /Users/wgu/Desktop/biorouter/scripts/migrate-docs.py << 'PYEOF'
#!/usr/bin/env python3
"""
Migrate docs from Docusaurus/MDX to plain markdown.

Transformations applied to every file:
  1. Strip Docusaurus-only frontmatter keys
  2. Strip JSX/MDX (imports, component tags, admonitions wrappers)
  3. Replace Goose/Block branding → BioRouter
  4. Replace recipe/recipes → workflow/workflows
  5. Basic link cleanup (remove /BioRouter/ prefix, fix recipe→workflow paths)

Usage:
  python3 scripts/migrate-docs.py          # run migration
  python3 scripts/migrate-docs.py --dry-run  # preview without writing
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).parent.parent
DRY_RUN = '--dry-run' in sys.argv

# ── Frontmatter keys to remove ────────────────────────────────────────────────
STRIP_FM_KEYS = {
    'sidebar_label', 'sidebar_position', 'custom_edit_url',
    'pagination_prev', 'pagination_next', 'slug', 'displayed_sidebar',
    'tags', 'image', 'keywords', 'hide_title', 'hide_table_of_contents',
    'toc_min_heading_level', 'toc_max_heading_level',
}

# ── Branding replacements (order matters — longer patterns first) ──────────────
BRANDING = [
    # URLs first (before bare word replacements)
    ('block.gitmcp.io',          'github.com/BaranziniLab/biorouter'),
    ('block.xyz',                'https://github.com/BaranziniLab/biorouter'),
    ('block.github.io',          'https://github.com/BaranziniLab/biorouter'),
    ('sq.github.io/goose',       'https://github.com/BaranziniLab/biorouter'),
    # Bare word branding (case variants)
    (re.compile(r'\bGOOSE\b'),   'BIOROUTER'),
    (re.compile(r'\bGoose\b'),   'BioRouter'),
    (re.compile(r'\bgoose\b'),   'BioRouter'),
    (re.compile(r'\bGEESE\b'),   'BIOROUTER'),
    (re.compile(r'\bGeese\b'),   'BioRouter'),
    (re.compile(r'\bgeese\b'),   'BioRouter'),
    # Remove "by Block" / "from Block" attributions
    (re.compile(r'\s*[Bb]y\s+Block\b'),  ''),
    (re.compile(r'\s*[Ff]rom\s+Block\b'), ''),
    (re.compile(r',?\s*[Bb]lock\s+[Ii]nc\.?'), ''),
]

# ── Recipe → Workflow renames ─────────────────────────────────────────────────
WORKFLOW = [
    # Env vars and CLI flags first (exact string, no word-boundary needed)
    ('BIOROUTER_RECIPE_PATH',       'BIOROUTER_WORKFLOW_PATH'),
    ('BIOROUTER_RECIPE_GITHUB_REPO','BIOROUTER_WORKFLOW_GITHUB_REPO'),
    ('biorouter recipe validate',   'biorouter workflow validate'),
    ('biorouter run --recipe',      'biorouter run --workflow'),
    ('.biorouter/recipes/',         '.biorouter/workflows/'),
    ('~/.config/biorouter/recipes/','~/.config/biorouter/workflows/'),
    # Path segments in links
    ('/recipes/',                   '/workflows/'),
    ('/recipe-reference',           '/reference'),
    ('/session-recipes',            '/session-workflows'),
    ('/storing-recipes',            '/storing-workflows'),
    ('/subrecipes',                 '/subworkflows'),
    # YAML field names
    ('sub_recipes:',                'sub_workflows:'),
    ('sub_recipe:',                 'sub_workflow:'),
    # Plural forms first
    (re.compile(r'\bSubrecipes\b'),  'Subworkflows'),
    (re.compile(r'\bsubrecipes\b'),  'subworkflows'),
    (re.compile(r'\bSubrecipe\b'),   'Subworkflow'),
    (re.compile(r'\bsubrecipe\b'),   'subworkflow'),
    (re.compile(r'\bRecipes\b'),     'Workflows'),
    (re.compile(r'\brecipes\b'),     'workflows'),
    (re.compile(r'\bRecipe\b'),      'Workflow'),
    (re.compile(r'\brecipe\b'),      'workflow'),
]


def strip_frontmatter(text: str) -> str:
    """Remove Docusaurus-only frontmatter keys; preserve title/date/status."""
    if not text.startswith('---'):
        return text
    end = text.find('\n---', 3)
    if end == -1:
        return text
    fm_block = text[3:end]
    body = text[end + 4:]  # skip closing ---\n
    kept = []
    for line in fm_block.splitlines():
        key = line.split(':')[0].strip().lstrip('-').strip()
        if not key or key not in STRIP_FM_KEYS:
            kept.append(line)
    new_fm = '\n'.join(kept).strip()
    if new_fm:
        return f'---\n{new_fm}\n---\n{body}'
    return body.lstrip('\n')


def strip_jsx(text: str) -> str:
    """Remove JSX/MDX constructs; preserve substantive content."""
    # Remove import/export lines
    text = re.sub(r'^(?:import|export)\s+.+\n', '', text, flags=re.MULTILINE)

    # Convert Docusaurus admonitions: :::type[title]\ncontent\n::: → > **Type:** content
    def admonition_sub(m):
        kind = m.group(1).strip().split()[0].capitalize()
        content = m.group(2).strip()
        return f'\n> **{kind}:** {content}\n'
    text = re.sub(
        r':::(\w+[^\n]*)\n(.*?):::',
        admonition_sub,
        text,
        flags=re.DOTALL,
    )

    # Strip <details><summary>…</summary>…</details> — keep inner content
    text = re.sub(
        r'<details[^>]*>\s*<summary[^>]*>(.*?)</summary>(.*?)</details>',
        lambda m: f'\n**{m.group(1).strip()}**\n{m.group(2).strip()}\n',
        text,
        flags=re.DOTALL,
    )

    # Convert JSX heading/paragraph tags to markdown (e.g. from landing pages)
    text = re.sub(r'<h1[^>]*>(.*?)</h1>', r'# \1', text, flags=re.DOTALL)
    text = re.sub(r'<h2[^>]*>(.*?)</h2>', r'## \1', text, flags=re.DOTALL)
    text = re.sub(r'<h3[^>]*>(.*?)</h3>', r'### \1', text, flags=re.DOTALL)
    text = re.sub(r'<p[^>]*>(.*?)</p>', r'\1\n', text, flags=re.DOTALL)

    # Remove Tabs/TabItem wrappers — keep plain content inside TabItem
    text = re.sub(r'<Tabs[^>]*>', '', text)
    text = re.sub(r'</Tabs>', '', text)
    text = re.sub(r'<TabItem[^>]*>', '', text)
    text = re.sub(r'</TabItem>', '', text)

    # Remove self-closing JSX components (uppercase tag name)
    text = re.sub(r'<[A-Z][a-zA-Z]*(?:\s[^>]*)?\s*/>', '', text)

    # Remove opening/closing JSX component pairs with content
    text = re.sub(r'<[A-Z][a-zA-Z]*[^>]*>.*?</[A-Z][a-zA-Z]*>', '', text, flags=re.DOTALL)

    # Remove iframe embeds
    text = re.sub(r'<iframe[^>]*>.*?</iframe>', '', text, flags=re.DOTALL)

    # Remove remaining div/span with className
    text = re.sub(r'<(?:div|span)[^>]*className[^>]*>', '', text)
    text = re.sub(r'</(?:div|span)>', '', text)

    # Remove className attributes on remaining HTML tags
    text = re.sub(r'\s+className=(?:"[^"]*"|\{[^}]*\})', '', text)

    # Remove /BioRouter/ URL prefix (GitHub Pages base path, not needed in plain docs)
    text = re.sub(r'\(/BioRouter/', '(/', text)

    # Fix doc cross-links: /docs/guides/ → relative paths are fine as-is
    # (internal links are best-effort; broken ones can be fixed manually)

    # Collapse 3+ blank lines to 2
    text = re.sub(r'\n{3,}', '\n\n', text)
    return text.strip() + '\n'


def apply_subs(text: str, subs: list) -> str:
    for pattern, replacement in subs:
        if isinstance(pattern, re.Pattern):
            text = pattern.sub(replacement, text)
        else:
            text = text.replace(pattern, replacement)
    return text


def migrate(src: Path, dst: Path):
    text = src.read_text(encoding='utf-8')
    text = strip_frontmatter(text)
    text = strip_jsx(text)
    text = apply_subs(text, BRANDING)
    text = apply_subs(text, WORKFLOW)
    if DRY_RUN:
        print(f'  DRY-RUN: {src.relative_to(REPO)} -> {dst.relative_to(REPO)}')
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(text, encoding='utf-8')
    print(f'  {src.relative_to(REPO)} -> {dst.relative_to(REPO)}')


# ── Mapping: source path → destination path (both relative to REPO) ───────────
MAPPING = [
    # documentation/ → docs/ (already clean markdown, recipe→workflow only)
    ('documentation/architecture.md',           'docs/architecture/overview.md'),
    ('documentation/data-privacy.md',           'docs/guides/data-privacy.md'),
    ('documentation/extensions-skills-mcp.md',  'docs/guides/extensions-skills.md'),
    ('documentation/installation-setup.md',     'docs/getting-started/installation.md'),
    ('documentation/providers-and-models.md',   'docs/getting-started/providers.md'),
    ('documentation/recipes.md',                'docs/guides/workflows/index.md'),
    ('documentation/schedulers.md',             'docs/guides/schedulers.md'),

    # getting-started
    ('docs/docs/quickstart.md',                         'docs/getting-started/quickstart.md'),

    # architecture
    ('docs/docs/biorouter-architecture/extensions-design.md',  'docs/architecture/extensions-design.md'),
    ('docs/docs/biorouter-architecture/error-handling.md',     'docs/architecture/error-handling.md'),

    # guides — core
    ('docs/docs/guides/config-files.md',              'docs/guides/config-files.md'),
    ('docs/docs/guides/biorouter-cli-commands.md',    'docs/guides/cli-commands.md'),
    ('docs/docs/guides/biorouter-permissions.md',     'docs/guides/permissions.md'),
    ('docs/docs/guides/sessions/index.md',            'docs/guides/sessions.md'),
    ('docs/docs/guides/context-engineering/index.md', 'docs/guides/context-engineering.md'),
    ('docs/docs/guides/environment-variables.md',     'docs/guides/environment-variables.md'),
    ('docs/docs/guides/security/index.md',            'docs/guides/security.md'),
    ('docs/docs/guides/subagents.md',                 'docs/guides/subagents.md'),
    ('docs/docs/guides/tips.md',                      'docs/guides/tips.md'),

    # guides — workflows (formerly recipes)
    ('docs/docs/guides/recipes/recipe-reference.md',   'docs/guides/workflows/reference.md'),
    ('docs/docs/guides/recipes/subrecipes.md',         'docs/guides/workflows/subworkflows.md'),
    ('docs/docs/guides/recipes/session-recipes.md',    'docs/guides/workflows/session-workflows.md'),
    ('docs/docs/guides/recipes/storing-recipes.md',    'docs/guides/workflows/storing-workflows.md'),

    # troubleshooting
    ('docs/docs/troubleshooting/index.md',                    'docs/troubleshooting/index.md'),
    ('docs/docs/troubleshooting/known-issues.md',             'docs/troubleshooting/known-issues.md'),
    ('docs/docs/troubleshooting/diagnostics-and-reporting.md','docs/troubleshooting/diagnostics.md'),

    # extensions — built-in BioRouter extensions only
    ('docs/docs/mcp/developer-mcp.md',           'docs/extensions/developer.md'),
    ('docs/docs/mcp/computer-controller-mcp.md', 'docs/extensions/computer-controller.md'),
    ('docs/docs/mcp/memory-mcp.md',              'docs/extensions/memory.md'),
    ('docs/docs/mcp/tutorial-mcp.md',            'docs/extensions/tutorial.md'),
    ('docs/docs/mcp/autovisualiser-mcp.md',      'docs/extensions/auto-visualiser.md'),
    ('docs/docs/mcp/skills-mcp.md',              'docs/extensions/skills.md'),
    ('docs/docs/mcp/extension-manager-mcp.md',   'docs/extensions/extension-manager.md'),
    ('docs/docs/mcp/chatrecall-mcp.md',          'docs/extensions/chat-recall.md'),
    ('docs/docs/mcp/code-execution-mcp.md',      'docs/extensions/code-execution.md'),
    ('docs/docs/mcp/todo-mcp.md',                'docs/extensions/todo.md'),
]

if __name__ == '__main__':
    print(f'{"DRY-RUN: " if DRY_RUN else ""}Migrating {len(MAPPING)} files...')
    errors = []
    for src_rel, dst_rel in MAPPING:
        src = REPO / src_rel
        dst = REPO / dst_rel
        if not src.exists():
            print(f'  MISSING: {src_rel}')
            errors.append(src_rel)
            continue
        try:
            migrate(src, dst)
        except Exception as e:
            print(f'  ERROR: {src_rel}: {e}')
            errors.append(src_rel)
    print(f'\nDone. {len(MAPPING) - len(errors)}/{len(MAPPING)} files migrated.')
    if errors:
        print(f'Errors ({len(errors)}): {errors}')
        sys.exit(1)
PYEOF
chmod +x /Users/wgu/Desktop/biorouter/scripts/migrate-docs.py
```

- [ ] **Step 2: Dry-run to verify all source files exist**

```bash
cd /Users/wgu/Desktop/biorouter && python3 scripts/migrate-docs.py --dry-run
```

Expected: 36 `DRY-RUN:` lines, no `MISSING:` lines.

If any `MISSING:` lines appear, check the source path against the actual file location with `find docs/docs -name "<filename>"`.

---

## Task 3: Run the migration

**Files:**

- Creates all 36 target `.md` files under `docs/`

- [ ] **Step 1: Run migration**

```bash
cd /Users/wgu/Desktop/biorouter && python3 scripts/migrate-docs.py
```

Expected: 36 lines of `source -> dest`, then `Done. 36/36 files migrated.`

- [ ] **Step 2: Spot-check three representative output files**

```bash
# 1. extensions-skills.md — should have no JSX, no Goose references
head -40 /Users/wgu/Desktop/biorouter/docs/guides/extensions-skills.md

# 2. guides/workflows/reference.md — verify recipe→workflow rename
grep -i "recipe\|goose" /Users/wgu/Desktop/biorouter/docs/guides/workflows/reference.md | head -10

# 3. extensions/developer.md — verify JSX stripped, GooseBuiltinInstaller gone
grep -i "GooseBuiltinInstaller\|className\|import " /Users/wgu/Desktop/biorouter/docs/extensions/developer.md | head -10
```

Expected for check 2: no output (no recipe/goose hits).
Expected for check 3: no output (no JSX artifacts).

- [ ] **Step 3: Check landing pages that were pure JSX — may need a content line**

```bash
wc -l /Users/wgu/Desktop/biorouter/docs/guides/context-engineering.md \
       /Users/wgu/Desktop/biorouter/docs/guides/sessions.md \
       /Users/wgu/Desktop/biorouter/docs/troubleshooting/index.md
```

If any file is under 5 lines, view it and add a one-sentence intro describing what the section covers:

```bash
# Example fix for a nearly-empty landing page:
# cat docs/guides/sessions.md
# If sparse, prepend a heading and intro:
# echo -e "# Sessions\n\nSessions are your continuous interactions with BioRouter. Each session maintains context and conversation history.\n" | cat - docs/guides/sessions.md > /tmp/s.md && mv /tmp/s.md docs/guides/sessions.md
```

- [ ] **Step 4: Commit migrated files**

```bash
cd /Users/wgu/Desktop/biorouter
git add docs/getting-started/ docs/architecture/ docs/guides/ docs/extensions/ docs/troubleshooting/
git commit -m "docs: migrate 36 files to plain markdown, purge Goose branding and recipe terminology"
```

---

## Task 4: Delete all Docusaurus artifacts

**Files:**

- Deletes: `docs/blog/`, `docs/community/`, `docs/audio/`, `docs/videos/`, `docs/assets/`, and all other Docusaurus-generated content
- Deletes: `docs/docs/` (the old Docusaurus source directory entirely)
- Deletes: `documentation/` (merged content now lives in `docs/`)

- [ ] **Step 1: Delete Docusaurus-generated directories**

```bash
cd /Users/wgu/Desktop/biorouter

# Large Docusaurus sections
rm -rf docs/blog \
       docs/community \
       docs/audio \
       docs/videos \
       docs/assets \
       docs/deeplink-generator \
       docs/recipe-generator \
       docs/prompt-library \
       docs/grants \
       docs/extension \
       docs/files \
       docs/v1

# Docusaurus-only root-level files
rm -f docs/404.html \
      docs/index.html \
      docs/.nojekyll \
      docs/sitemap.xml \
      docs/robots.txt \
      docs/atom.xml docs/atom.css docs/atom.xsl \
      docs/rss.xml  docs/rss.css  docs/rss.xsl \
      docs/llms.txt \
      docs/servers.json \
      docs/inkeepChatButton.js \
      docs/inkeepSearchBar.js
```

- [ ] **Step 2: Clean up Docusaurus HTML inside docs/extensions/ (the directory already existed)**

The `docs/extensions/` directory exists in the Docusaurus site and contains `index.html` and a `detail/` subdirectory. Our migrated `.md` files now live alongside those. Remove only the HTML artifacts; leave the `.md` files intact.

```bash
find /Users/wgu/Desktop/biorouter/docs/extensions -name "*.html" -delete
find /Users/wgu/Desktop/biorouter/docs/extensions -name "detail" -type d -exec rm -rf {} + 2>/dev/null || true
```

- [ ] **Step 3: Delete docs/docs/ entirely (old Docusaurus source)**

```bash
rm -rf /Users/wgu/Desktop/biorouter/docs/docs
```

- [ ] **Step 4: Delete documentation/ (all content merged into docs/)**

```bash
rm -rf /Users/wgu/Desktop/biorouter/documentation
```

- [ ] **Step 5: Check for any remaining non-superpowers directories**

```bash
ls /Users/wgu/Desktop/biorouter/docs/
```

Expected: only `getting-started/  architecture/  guides/  extensions/  troubleshooting/  superpowers/`

If any unexpected directory remains (e.g. `docs/markdown-page/`, `docs/recipes/`, `docs/extensions/` from Docusaurus), remove them:

```bash
# Example: if docs/extensions/ from the old Docusaurus site still exists
# alongside our new docs/extensions/, it won't because rm -rf docs/docs removed
# the source. But if there are lingering top-level folders, delete them:
# rm -rf docs/<unexpected-folder>
```

- [ ] **Step 6: Commit deletions**

```bash
cd /Users/wgu/Desktop/biorouter
git add -A
git commit -m "docs: delete Docusaurus infrastructure, blog, audio, video, third-party MCP pages"
```

---

## Task 5: Run verification and fix any remaining issues

**Files:**

- No new files — fix any issues found in the migrated `.md` files

- [ ] **Step 1: Run the full verification suite**

```bash
cd /Users/wgu/Desktop/biorouter && bash scripts/verify-docs.sh
```

Expected: `ALL CHECKS PASSED` with all 7 checks showing `PASS`.

- [ ] **Step 2: Fix any remaining Goose/Block references**

If check 3 (goose/geese) fails, find and fix the specific file:

```bash
grep -rn "goose\|geese\|Goose\|Geese" /Users/wgu/Desktop/biorouter/docs --include="*.md" \
  | grep -v superpowers/
```

For each hit, open the file and replace manually. Common missed cases:

- `**BioRouter Prompt:**` ← was `**goose Prompt:**` in `subagents.md`
- URLs containing `goose` in GitHub links

- [ ] **Step 3: Fix any remaining recipe references**

If check 4 (recipe) fails:

```bash
grep -rn "\brecipe\b\|\brecipes\b" /Users/wgu/Desktop/biorouter/docs --include="*.md" \
  -i | grep -v superpowers/
```

Fix each file. Common missed cases:

- YAML code blocks inside markdown (the Python script does replace inside code blocks)
- Table cells containing `recipe` as part of a longer word like `recipe-generator` (already deleted) or `recipe.yaml` examples — replace those with `workflow.yaml`

- [ ] **Step 4: Check for leftover JSX artifacts**

```bash
grep -rn "className\|import React\|<Card\|<Tabs\|<TabItem\|<GooseBuiltin" \
  /Users/wgu/Desktop/biorouter/docs --include="*.md" | grep -v superpowers/
```

For each hit, open the file and remove the remaining JSX manually.

- [ ] **Step 5: Verify target directory tree looks correct**

```bash
find /Users/wgu/Desktop/biorouter/docs -type f -name "*.md" \
  | grep -v superpowers/ | sort
```

Expected: 36 files across `getting-started/`, `architecture/`, `guides/`, `guides/workflows/`, `extensions/`, `troubleshooting/`

- [ ] **Step 6: Commit fixes**

```bash
cd /Users/wgu/Desktop/biorouter
git add docs/
git commit -m "docs: fix remaining branding, JSX, and recipe references after migration"
```

---

## Task 6: Final verification and cleanup

- [ ] **Step 1: Run verification one final time**

```bash
cd /Users/wgu/Desktop/biorouter && bash scripts/verify-docs.sh
```

Expected: `ALL CHECKS PASSED`

- [ ] **Step 2: Remove the migration script (no longer needed)**

```bash
rm /Users/wgu/Desktop/biorouter/scripts/migrate-docs.py
```

Keep `scripts/verify-docs.sh` — it's useful for future audits.

- [ ] **Step 3: Final commit**

```bash
cd /Users/wgu/Desktop/biorouter
git add -A
git commit -m "docs: remove migration script, consolidation complete"
```

- [ ] **Step 4: Confirm final structure**

```bash
find /Users/wgu/Desktop/biorouter/docs -type d | sort
```

Expected directories:

```text
docs/
docs/architecture
docs/extensions
docs/getting-started
docs/guides
docs/guides/workflows
docs/superpowers
docs/superpowers/plans
docs/superpowers/specs
docs/troubleshooting
```

---

## Spec coverage checklist

Each requirement from [consolidation-design.md](consolidation-design.md), mapped to the task that satisfies it.

| Spec requirement | Task |
| --- | --- |
| Drop Docusaurus entirely — no HTML files | Task 4 + verify check 1 |
| No audio/video files | Task 4 + verify check 2 |
| Purge Goose/Block branding | Tasks 2–3 (migration script) + verify check 3 |
| recipe → workflow rename | Tasks 2–3 (migration script) + verify check 4 |
| Delete `docs/docs/` | Task 4, Step 3 |
| Delete `documentation/` | Task 4, Step 4 |
| Only built-in extensions kept | `MAPPING` in Task 2 (10 files) |
| `superpowers/` untouched | Not in any delete command |
| All files are `.md` | verify check 7 |

## Related documentation

- [Docs consolidation design](consolidation-design.md) — the design this plan implements, with the full file-by-file move tables and transformation rules
- [System overview](../../architecture/system-overview.md) — the architecture page produced by the first entry in this plan's `MAPPING`
- [Workflows](../../workflows/README.md) — the section created by the recipe→workflow rename in Tasks 2–3
- [Extensions and skills guide](../../extensions/extensions-and-skills-guide.md) — one of the 36 migrated pages, carried over from `documentation/extensions-skills-mcp.md`
- [Troubleshooting](../../troubleshooting/README.md) — the troubleshooting section this migration created
