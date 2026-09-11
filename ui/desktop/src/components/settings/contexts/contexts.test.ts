import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  CONTEXTS,
  CONTEXT_IDS,
  contextConfigKey,
  isContextBundle,
  isContextSkill,
} from './contexts';

describe('contexts', () => {
  /**
   * ⚠ These ids are what enablement is keyed on, and a typo here is invisible:
   * the Context row would render and toggle, the skill would simply never be
   * filtered out, and the counts this exists to fix would stay wrong. Pinned
   * against the Rust source of truth — `BUILTIN_SKILLS` and `KNOWLEDGE_BUNDLE`
   * in `skills_extension.rs`.
   *
   * ⚠ **Four skill names and one BUNDLE directory.** `knowledge-bases` is not a
   * skill: it is the directory holding the four `KNOWLEDGE_SKILLS` plus
   * `update-soul`, which used to be a Context of its own labelled "Updates".
   * Five rows over nine shipped skills.
   *
   * ⚠ Sorted, so `develop-biorouter` sits before its two longer namesakes.
   * The three are distinct skills, not one with variants: this one is about
   * changing Biorouter's own source, the others about authoring a skill and
   * packaging an extension.
   */
  it('names exactly the five rows that actually ship', () => {
    expect([...CONTEXT_IDS].sort()).toEqual([
      'about-biorouter',
      'develop-biorouter',
      'develop-biorouter-extension',
      'develop-biorouter-skill',
      'knowledge-bases',
    ]);
  });

  it('recognises a shipped skill and leaves a user skill alone', () => {
    expect(isContextSkill('about-biorouter')).toBe(true);
    // The trap: a user-installed skill whose name merely resembles one.
    expect(isContextSkill('about-biorouter-notes')).toBe(false);
    expect(isContextSkill('single-cell')).toBe(false);
  });

  /**
   * ⚠ **A bundle member is a Context through its bundle, never its own name.**
   * `update-soul` and the four knowledge skills each carry their own
   * frontmatter `name:`, and none of them is in `CONTEXT_IDS` any more. A
   * name-only filter — which is what this function used to be — puts all five
   * back in the composer picker, the `@`-mention list and the chip count, while
   * the single Settings switch appears to work.
   */
  it('treats a member of a Context bundle as a Context', () => {
    for (const member of [
      'update-soul',
      'knowledge-lint',
      'knowledge-choose-a-format',
      'knowledge-ingest-okf',
      'knowledge-ingest-biookf',
    ]) {
      expect(isContextSkill(member), `${member} without its bundle`).toBe(false);
      expect(isContextSkill(member, 'knowledge-bases'), `${member} in its bundle`).toBe(true);
    }
    // A member of some OTHER bundle is not swept up.
    expect(isContextSkill('brainstorming', 'superpowers')).toBe(false);
    expect(isContextSkill('anything', null)).toBe(false);
    expect(isContextSkill('anything', undefined)).toBe(false);
  });

  it('recognises a Context bundle row and leaves an installed package alone', () => {
    expect(isContextBundle('knowledge-bases')).toBe(true);
    expect(isContextBundle('superpowers')).toBe(false);
    // A member name is not a bundle name.
    expect(isContextBundle('knowledge-lint')).toBe(false);
  });

  /**
   * ⚠ The key must not collide with, or be routed into, `skills-config.json`'s
   * `disabled[]`. That array is honoured by `handle_load_skill`, which refuses
   * a disabled skill outright, while the system prompt unconditionally tells
   * the model to load `about-biorouter` — so a Context disabled through that
   * path would make the agent report a load failure on every turn.
   */
  it('derives a config key that is a plain identifier', () => {
    expect(contextConfigKey('about-biorouter')).toBe('context_about_biorouter');
    expect(contextConfigKey('knowledge-bases')).toBe('context_knowledge_bases');
    for (const c of CONTEXTS) {
      expect(contextConfigKey(c.id)).toMatch(/^context_[a-z0-9_]+$/);
    }
  });

  it('gives every context a label and a description', () => {
    for (const c of CONTEXTS) {
      expect(c.label.length).toBeGreaterThan(0);
      expect(c.description.length).toBeGreaterThan(0);
      expect(c.description).not.toContain('—');
    }
  });
});

/**
 * ⚠ The section's placement is otherwise unprotected: `SettingsView.test.tsx`
 * asserts the App tab's order and nothing asserts the Chat tab's, so a
 * reordering would go unnoticed. This pins the sequence at the source, which is
 * the only place it can be checked without a layout engine.
 */
describe('Settings -> Chat section order', () => {
  it('puts Contexts after Capabilities and Memory, and before App SDK', () => {
    const src = readFileSync(join(__dirname, '..', 'chat', 'ChatSettingsSection.tsx'), 'utf8');
    const at = (needle: string) => {
      const i = src.indexOf(needle);
      expect(i, `${needle} missing from ChatSettingsSection`).toBeGreaterThan(-1);
      return i;
    };
    // Capabilities owns the switch that turns memory on, which is why Memory
    // sits directly under it; Contexts goes after that pair.
    expect(at('<CapabilitiesSection />')).toBeLessThan(at('<MemorySection />'));
    expect(at('<MemorySection />')).toBeLessThan(at('<ContextsSection />'));
    // ⚠ `>App SDK<`, not `App SDK`. The bare string also matches the comment
    // in ChatSettingsSection explaining this very ordering, which sits ABOVE
    // the section — so the loose form failed on correct code. Third time this
    // repo has produced a guard that matched its own prose.
    expect(at('<ContextsSection />')).toBeLessThan(at('>App SDK<'));
  });
});

/**
 * ⚠ **Copies of this list exist and nothing asserted they agree.**
 *
 * Rust owns the truth. It says it with three names, answering three different
 * questions, and conflating any two is how this area has broken before:
 *
 * * `context_ids()` — `BUILTIN_SKILLS` ++ `KNOWLEDGE_BUNDLE`. What Settings
 *   offers as a switch. `CONTEXTS` here mirrors exactly this.
 * * `is_builtin_skill_name()` — over `shipped_skills()` (`BUILTIN_SKILLS` ++
 *   `KNOWLEDGE_SKILLS`) plus `SOUL_SKILL_DIR`. Every SKILL Biorouter seeds,
 *   which is what hiding Delete and showing the Built-in badge mirror.
 * * `is_shipped_entry_name()` — the above plus the bundle DIRECTORY, for the
 *   surfaces that enumerate directory entries rather than skills.
 *
 * ⚠ **There are TWO hand-synced copies, not three.** `skillUtils.BUILTIN_SKILL_NAMES`
 * is gone: it answered "did Biorouter put this here?", which the daemon answers
 * directly as `CatalogSkill.builtin` (and `CatalogBundle.builtin` for a bundle
 * row). A pinning test over a list nothing reads guards nothing, so what is
 * asserted below is that the copy has not come back.
 *
 * ⚠ **The extractors below must see every list Rust seeds from.** They once
 * sliced `BUILTIN_SKILLS` alone, so the four knowledge skills were invisible to
 * the census — which is exactly why the Skills pane came to offer a working
 * Delete on a seeded skill. Every array and constant the seeder reads is parsed
 * here, and each extractor asserts it found something, so a rename that makes a
 * regex stop matching fails loudly instead of silently narrowing the census.
 *
 * Reading the Rust source is the only way to check this from here, and it is
 * the same approach `measures.test.ts` takes for a value only the source can
 * settle.
 */
describe('the built-in skill list, across every copy', () => {
  const rustSrc = (...parts: string[]) =>
    readFileSync(
      join(__dirname, '..', '..', '..', '..', '..', '..', 'crates', 'biorouter', 'src', ...parts),
      'utf8'
    );

  /**
   * The `("<name>", include_str!(...))` pairs of one `&[(&str, &str)]` array.
   *
   * rustfmt breaks each pair across lines, so the name and `include_str!` are
   * not adjacent. Matching on the name that PRECEDES an `include_str!` is what
   * survives that, and it still cannot pick up an unrelated string.
   */
  const seededArray = (identifier: string): string[] => {
    const src = rustSrc('agents', 'skills_extension.rs');
    const at = src.indexOf(`${identifier}: &[(&str, &str)] = &[`);
    expect(at, `${identifier} not found in skills_extension.rs`).toBeGreaterThan(-1);
    const block = src.slice(at, src.indexOf('];', at));
    const names = [...block.matchAll(/"([a-z0-9-]+)",\s*\n?\s*include_str!/g)].map((m) => m[1]);
    expect(names.length, `${identifier} yielded no skills`).toBeGreaterThan(0);
    return names;
  };

  const rustConst = (file: string[], identifier: string): string => {
    const match = rustSrc(...file).match(
      new RegExp(`${identifier}:\\s*&str\\s*=\\s*"([a-z0-9-]+)"`)
    );
    expect(match, `${identifier} not found in ${file.join('/')}`).not.toBeNull();
    return match![1];
  };

  const builtinSkills = () => seededArray('BUILTIN_SKILLS');
  const knowledgeSkills = () => seededArray('KNOWLEDGE_SKILLS');
  const knowledgeBundle = () => rustConst(['agents', 'skills_extension.rs'], 'KNOWLEDGE_BUNDLE');
  const soulSkill = () => rustConst(['knowledge', 'soul.rs'], 'SOUL_SKILL_DIR');

  /** What Rust offers as a Settings switch: `context_ids()`. */
  const rustContextIds = () => [...builtinSkills(), knowledgeBundle()].sort();

  /**
   * Every SKILL Biorouter seeds — `is_builtin_skill_name()`. Larger than the
   * Contexts, and made of different things: five of these nine are reachable
   * only through the bundle.
   */
  const shippedNames = () => [...builtinSkills(), ...knowledgeSkills(), soulSkill()].sort();

  it('CONTEXTS names exactly what Rust offers as a Context', () => {
    expect([...CONTEXT_IDS].sort()).toEqual(rustContextIds());
  });

  /**
   * ⚠ The census must be **larger** than the Context list, and every skill in
   * it must be covered — by its own row if it is flat, by its bundle's row if
   * it is a member. Asserting only `CONTEXTS === context_ids()` would pass
   * while the seeder wrote skills no surface here had ever heard of, which is
   * the state this area shipped in once already.
   *
   * ⚠ **Each name is checked against the row that is supposed to cover IT.**
   * The obvious spelling — `isContextSkill(n) || isContextSkill(n, bundle)` —
   * is vacuous: the second call is `CONTEXT_IDS.has(n) || CONTEXT_IDS.has(bundle)`,
   * and the bundle is a Context, so it returns true for every string in
   * existence. It was written that way here first, and it passed for
   * `not-a-skill-at-all`.
   */
  it('every skill Rust seeds is covered by the Context row that owns it', () => {
    const flat = builtinSkills();
    const members = [...knowledgeSkills(), soulSkill()];
    const bundle = knowledgeBundle();
    expect([...flat, ...members].length).toBeGreaterThan(CONTEXT_IDS.size);

    for (const name of flat) {
      expect(isContextSkill(name), `${name} is seeded but no Context row names it`).toBe(true);
    }
    for (const member of members) {
      // Covered ONLY through the bundle — a member listed in CONTEXTS as well
      // would give the user two switches for one thing, the narrower of which
      // the bundle switch silently overrides.
      expect(isContextSkill(member), `${member} is a Context in its own right`).toBe(false);
      expect(
        isContextSkill(member, bundle),
        `${member} is not covered by its bundle's Context row`
      ).toBe(true);
    }
    // The predicate is not a tautology: something Rust does not seed is not a
    // Context, whatever bundle you hand it.
    expect(isContextSkill('single-cell', 'superpowers')).toBe(false);
  });

  it('the renderer keeps no second list of the skills Rust seeds', () => {
    const source = readFileSync(join(__dirname, '..', '..', 'skills', 'skillUtils.ts'), 'utf8');
    for (const name of [...shippedNames(), knowledgeBundle()]) {
      expect(
        source.includes(`'${name}'`),
        `skillUtils.ts names '${name}' again — read CatalogSkill.builtin instead`
      ).toBe(false);
    }
  });
});
