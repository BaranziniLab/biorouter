/**
 * What is left of the renderer's own view of skills.
 *
 * ⚠ **The filesystem scanner is gone** (#113). `loadSkillsFromDirs`,
 * `ALL_SKILL_DIRS` and `OTHER_SKILL_DIRS` walked three roots against the
 * backend's seven, so a skill bundled inside an installed extension was
 * loadable by the model and invisible in every interface surface that used
 * them — Settings, the composer picker, the `@`-mention list and the workflow
 * resource picker alike. They all read `useSkillCatalog` now, and the
 * `Skill`/`SkillBundle` shapes it produced are the generated `CatalogSkill` and
 * `CatalogBundle`.
 *
 * What remains is the two things a renderer legitimately does without asking
 * the daemon: parse the frontmatter of a file the user is *authoring*
 * (`CustomSkillModal`), and derive a folder name from it.
 *
 * ⚠ **The `BUILTIN_SKILL_NAMES` copy is gone too**, and deliberately. Its job
 * was to hide Delete and show the "Built-in" badge, and a hand-synced list is a
 * bad way to answer "did Biorouter put this here?" — it had already drifted
 * once, and the Skills pane offered a Delete that succeeded, toasted, and was
 * silently rewritten by the next startup. `CatalogSkill.builtin` answers it
 * from `is_builtin_skill_name`, in the process that owns the seeder.
 */
export const BIOROUTER_SKILLS_DIR = '~/.config/biorouter/skills';

/**
 * Read a single top-level frontmatter field, supporting both inline values
 * (`key: value`, optionally quoted) and YAML block scalars
 * (`key: >-` / `|` followed by indented continuation lines). The built-in
 * skills use a folded block scalar for `description`, so a naive
 * same-line-only parse would surface the literal `>-` indicator instead of the
 * text. Folded (`>`) blocks join lines with spaces (blank line → newline);
 * literal (`|`) blocks keep their newlines.
 */
function readFrontmatterField(fm: string, key: string): string | null {
  const lines = fm.split(/\r?\n/);
  const idx = lines.findIndex((l) => new RegExp(`^${key}:`).test(l));
  if (idx === -1) return null;

  const head = (lines[idx].match(new RegExp(`^${key}:\\s*(.*)$`))?.[1] ?? '').trim();

  // Block scalar indicator: `|` or `>`, optional chomping (+/-) / indent digit,
  // optional trailing comment.
  const block = head.match(/^([|>])[+-]?\d*\s*(#.*)?$/);
  if (block) {
    const folded = block[1] === '>';
    const body: string[] = [];
    for (let i = idx + 1; i < lines.length; i++) {
      const line = lines[i];
      if (line.trim() === '') {
        body.push('');
        continue;
      }
      // Continuation lines are indented; a column-0 line starts the next key.
      if (!/^\s/.test(line)) break;
      body.push(line.replace(/^\s+/, ''));
    }
    while (body.length && body[body.length - 1] === '') body.pop();
    if (!folded) return body.join('\n').trim() || null;
    let out = '';
    for (const l of body) {
      if (l === '') out += '\n';
      else out += (out && !out.endsWith('\n') ? ' ' : '') + l;
    }
    return out.trim() || null;
  }

  // Inline value — strip a single layer of matching surrounding quotes.
  const unquoted = head.replace(/^"([\s\S]*)"$/, '$1').replace(/^'([\s\S]*)'$/, '$1');
  return unquoted.trim() || null;
}

/**
 * Parse YAML frontmatter from a SKILL.md file.
 * Returns { name, description } if valid, null if missing or malformed.
 */
export function parseSkillFrontmatter(
  content: string
): { name: string; description: string } | null {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  if (!match) return null;
  const fm = match[1];
  const name = readFrontmatterField(fm, 'name');
  const description = readFrontmatterField(fm, 'description');
  if (!name || !description) return null;
  return { name, description };
}

/**
 * Derive a safe folder/file slug from a skill name or filename.
 * e.g. "My Skill!" → "my-skill"
 */
export function toSlug(input: string): string {
  return input
    .replace(/\.md$/i, '')
    .replace(/[^a-z0-9-_]/gi, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-|-$/g, '')
    .toLowerCase();
}

/**
 * A staging nonce left in an installed package's name by a shipped build.
 *
 * Two writers, two exact widths, and both are pinned rather than approximated:
 *
 * * **12 lowercase hex digits** — `crypto.randomBytes(6).toString('hex')`, what
 *   the desktop app's `registry:download` IPC handler prepended to the staged
 *   filename before `utils/registryDownload` moved it into the directory.
 * * **16 lowercase hex digits** — `format!("{nanos:x}")` over
 *   `SystemTime::now()` nanoseconds, what `biorouter serve`'s
 *   `POST /registry/download` prepended. Nanoseconds since the epoch have been
 *   16 hex digits since 2006 and stay 16 until 2554.
 *
 * Anchored and exact-width on purpose: this is "recognise the nonce this app
 * wrote", not "strip whatever precedes a dash", so `hi-c-analysis`,
 * `2024-cohort` and an unrelated `chip-single-cell` are all untouched.
 */
const STAGING_NONCE = /^(?:[0-9a-f]{12}|[0-9a-f]{16})-(?=.)/;

/**
 * The name a package installed by an older build *would* have had, or `null`
 * when it carries no staging nonce.
 *
 * ⚠ **An alias, never a replacement.** A bundle already on disk as
 * `d92c1c985d54-single-cell` keeps that name everywhere it is displayed and
 * removed — renaming an install directory would orphan its `skills-config.json`
 * entry, every session override keyed on the bundle name, and its
 * `removeSkillPackage` target. What the alias buys is the one question that was
 * answered wrongly: "is the marketplace's `single-cell` already installed?" —
 * yes, and the Browse modal stops offering it for the tenth time.
 */
export function withoutStagingNonce(packageName: string): string | null {
  const stripped = packageName.replace(STAGING_NONCE, '');
  return stripped === packageName ? null : stripped;
}
