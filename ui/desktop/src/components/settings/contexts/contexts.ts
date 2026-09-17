/**
 * Contexts: the skills that ship with Biorouter rather than being installed.
 *
 * They are skills mechanically — same `SKILL.md`, same loader, same
 * `/skill:<name>` reference — but not skills from the user's point of view.
 * The user did not choose them, cannot delete them (the seeder rewrites them on
 * every start), and did not mean them when they asked how many skills are
 * loaded. Counting them made "5 skills enabled" mostly a statement about the
 * install.
 *
 * ⚠ **Disabling a Context must NOT go through `skills-config.json`.** That
 * file's `disabled[]` array is honoured by `handle_load_skill`, which refuses a
 * disabled skill outright (`skills_extension.rs:704-713`) — while
 * `prompts/system.md:31-34` unconditionally instructs the model to load
 * `about-biorouter`. Route a Context through that array and turning one off
 * makes the agent inject "Could not load this selected skill" into every
 * prompt. Enablement therefore lives in ordinary config keys, so a disabled
 * Context stops being *surfaced* without becoming *unloadable*.
 *
 * ⚠ The Rust side keeps its own list (`skills_extension.rs::context_ids`, i.e.
 * `BUILTIN_SKILLS` plus `KNOWLEDGE_BUNDLE`). The two are hand-synced but not
 * unasserted: `contexts.test.ts` parses the Rust source and fails if this copy
 * disagrees with it (#77). Rust owns the truth — the seeder writes those files
 * — so a mismatch is fixed by moving the TS list, not the assertion.
 *
 * ⚠ **A Context id is not always a skill name.** One row may stand for a whole
 * BUNDLE: `knowledge-bases` covers the four knowledge-format skills plus
 * `update-soul`, which are seeded into `<skills root>/knowledge-bases/<name>/`
 * and reach the renderer with `CatalogSkill.bundle` set. So every filter here
 * takes the bundle as well as the name — `isContextSkill(name, bundle)` for a
 * skill row, `isContextBundle(name)` for a bundle row. Filtering on the name
 * alone leaves a bundle row in the composer picker and the `@`-mention list,
 * and — worst of the three — inside "Enable all", which writes to
 * `skills-config.json` and would make `handle_load_skill` refuse a Context.
 */

export interface ContextMeta {
  /**
   * The identifier enablement is keyed on: a skill's frontmatter `name:` (which
   * is also its directory name), or a BUNDLE's directory name.
   */
  id: string;
  label: string;
  description: string;
}

/**
 * ⚠ Context rows may cover individual skills or bundles. Every id must exist on the Rust side
 * before it is listed here: `BUILTIN_SKILLS` (`skills_extension.rs`) carries
 * the individual skill names, and `KNOWLEDGE_BUNDLE` carries one *bundle* directory whose
 * five members are the four `KNOWLEDGE_SKILLS` plus `update-soul` (a Rust
 * string in `soul.rs` rather than an `include_str!`). A Context whose `SKILL.md`
 * — or whose bundle directory — does not ship renders and toggles while
 * pointing at nothing, so add it on the Rust side first.
 */
export const CONTEXTS: readonly ContextMeta[] = [
  {
    id: 'office-word',
    label: 'Word documents',
    description: 'Read, create and edit Word documents. Loaded only for relevant document tasks.',
  },
  {
    id: 'office-powerpoint',
    label: 'PowerPoint',
    description: 'Create and edit slide decks. Loaded only for presentation tasks.',
  },
  {
    id: 'office-excel',
    label: 'Excel spreadsheets',
    description: 'Work with spreadsheets, formulas and charts. Loaded only for spreadsheet tasks.',
  },
  {
    id: 'office-pdf',
    label: 'PDF documents',
    description: 'Read, create and process PDFs and forms. Loaded only for PDF tasks.',
  },
  {
    id: 'about-biorouter',
    label: 'About Biorouter',
    description: 'What Biorouter is and how its pieces fit together.',
  },
  {
    id: 'develop-biorouter',
    label: 'Develop Biorouter',
    description: 'Work on the Biorouter codebase: layout, commands and checks.',
  },
  {
    id: 'develop-biorouter-extension',
    label: 'Develop Biorouter Extension',
    description: 'Build, package and publish an extension.',
  },
  {
    id: 'develop-biorouter-skill',
    label: 'Develop Biorouter Skill',
    description: 'Write, test and package a skill.',
  },
  {
    id: 'knowledge-bases',
    label: 'Knowledge',
    description:
      'Build and maintain knowledge bases: pick a format, ingest sources, read a lint report, and keep the personal base current as a chat goes on.',
  },
];

export const CONTEXT_IDS: ReadonlySet<string> = new Set(CONTEXTS.map((c) => c.id));

/**
 * Is this skill a shipped Context rather than one the user installed?
 *
 * Pass the skill's `bundle` — a member of a Context bundle is a Context, and
 * its own name is not in `CONTEXT_IDS`.
 */
export function isContextSkill(name: string, bundle?: string | null): boolean {
  return CONTEXT_IDS.has(name) || (bundle != null && CONTEXT_IDS.has(bundle));
}

/** Is this bundle row a shipped Context rather than an installed package? */
export function isContextBundle(name: string): boolean {
  return CONTEXT_IDS.has(name);
}

/** The config key holding one Context's enablement. Default on when absent. */
export function contextConfigKey(id: string): string {
  return `context_${id.replace(/-/g, '_')}`;
}
