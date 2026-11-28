import { describe, expect, it } from 'vitest';
import {
  applyOptimistically,
  bundleCatalogEntryKey,
  CATALOG_CHANGED_EVENT,
  movedChoicesNotice,
  pickerBundles,
  resolveSkillKeys,
  skillCatalogToggleKey,
  standaloneSkills,
} from './useSkillCatalog';
import { SKILLS_COPY } from './copy';
import type { CatalogBundle, CatalogSkill, CatalogView } from '../../api';

const state = (effective: boolean, machineEnabled = true) => ({
  machineEnabled,
  session: 'default' as const,
  sessionViaBundle: false,
  hiddenContext: false,
  effective,
});

const skill = (name: string, bundle: string | null, effective = true): CatalogSkill => ({
  name,
  description: '',
  slug: name,
  directory: `/skills/${name}`,
  sourceRoot: '/skills',
  source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
  bundle,
  builtin: false,
  state: state(effective),
});

const bundle = (name: string, members: string[]): CatalogBundle => ({
  name,
  displayName: name,
  directory: `/skills/${name}`,
  sourceRoot: '/skills',
  source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
  skills: members,
  package: null,
  builtin: false,
  state: state(true),
});

const view: CatalogView = {
  generation: 3,
  roots: [],
  skills: [skill('solo', null), skill('media-use', 'hyperframes'), skill('other', null)],
  bundles: [bundle('hyperframes', ['media-use'])],
};

describe('applyOptimistically', () => {
  it('moves only the entries the toggle named, and their bundle members', () => {
    const next = applyOptimistically(view, ['hyperframes'], false, true);
    expect(next.bundles[0].state.effective).toBe(false);
    expect(next.skills.find((s) => s.name === 'media-use')!.state.effective).toBe(false);
    expect(next.skills.find((s) => s.name === 'solo')!.state.effective).toBe(true);
    expect(next.skills.find((s) => s.name === 'other')!.state.effective).toBe(true);
  });

  /**
   * A per-chat toggle changes `workspace_skills/v1` and nothing else. Guessing
   * at `machineEnabled` here would have the interface claim, for one frame,
   * that a chat-scoped click had rewritten the machine-wide preference every
   * other chat and the CLI share.
   */
  it('never claims a per-chat toggle changed the machine-wide preference', () => {
    const next = applyOptimistically(view, ['solo'], false, true);
    const solo = next.skills.find((s) => s.name === 'solo')!;
    expect(solo.state.effective).toBe(false);
    expect(solo.state.machineEnabled).toBe(true);
  });

  it('does move the machine-wide answer when the toggle was machine-wide', () => {
    const next = applyOptimistically(view, ['solo'], false, false);
    const solo = next.skills.find((s) => s.name === 'solo')!;
    expect(solo.state.machineEnabled).toBe(false);
  });

  it('leaves the source catalog untouched, so a refusal can restore it', () => {
    const before = JSON.stringify(view);
    applyOptimistically(view, ['hyperframes', 'solo'], false, true);
    expect(JSON.stringify(view)).toBe(before);
  });

  it('keeps a same-named bundle toggle shared across physical roots', () => {
    const duplicate = {
      ...view,
      bundles: [
        bundle('pack', ['alpha']),
        { ...bundle('pack', ['beta']), sourceRoot: '/project/.biorouter/skills' },
      ],
    };

    const next = applyOptimistically(duplicate, ['pack'], false, true);
    expect(next.bundles.map((entry) => entry.state.effective)).toEqual([false, false]);
  });
});

describe('same-named bundle identities', () => {
  it('uses distinct physical row keys but keeps one shared toggle key', () => {
    const first = bundle('pack', ['alpha']);
    const second = {
      ...bundle('pack', ['beta']),
      sourceRoot: '/project/.biorouter/skills',
    };

    expect(bundleCatalogEntryKey(first)).not.toBe(bundleCatalogEntryKey(second));
    expect(
      [first, second].map((entry) =>
        skillCatalogToggleKey({
          kind: 'bundle',
          key: bundleCatalogEntryKey(entry),
          bundle: entry,
          enabled: true,
        })
      )
    ).toEqual(['pack', 'pack']);
  });
});

describe('CATALOG_CHANGED_EVENT', () => {
  it('is the name worktree 4 publishes', () => {
    expect(CATALOG_CHANGED_EVENT).toBe('catalog:changed');
  });
});

/**
 * ⚠ **A Context that is a BUNDLE has to be filtered as a bundle.** Every
 * Context filter used to test a skill's own `name`, and a bundle row carries
 * none of its members' names — so promoting the knowledge skills to a bundle
 * put a row back into the composer picker, the `@`-mention list and the
 * workflow resource picker at once, with `activeCount` up by one to match.
 *
 * The composer one is the dangerous one: its rows feed "Enable all", which
 * writes `skills-config.json`, and `handle_load_skill` refuses anything listed
 * there — while the system prompt asks for `about-biorouter` on every turn.
 * That is the failure `contexts.ts` was written to make impossible.
 */
describe('Context bundles are not picker rows', () => {
  const contextView: CatalogView = {
    generation: 1,
    roots: [],
    skills: [
      skill('solo', null),
      skill('media-use', 'hyperframes'),
      skill('knowledge-lint', 'knowledge-bases'),
      skill('update-soul', 'knowledge-bases'),
    ],
    bundles: [
      bundle('hyperframes', ['media-use']),
      bundle('knowledge-bases', ['knowledge-lint', 'update-soul']),
    ],
  };

  it('drops the Context bundle and keeps an installed package', () => {
    expect(pickerBundles(contextView).map((b) => b.name)).toEqual(['hyperframes']);
  });

  /**
   * The members were already excluded by the `!skill.bundle` clause, so this
   * would pass with the Context filter deleted. It is here to pin that the two
   * clauses agree: nothing about the knowledge bundle reaches a skill row.
   */
  it('drops the Context bundle members from the standalone list', () => {
    expect(standaloneSkills(contextView).map((s) => s.name)).toEqual(['solo']);
  });

  /**
   * ⚠ **A member that reaches the renderer WITHOUT its bundle is not covered,
   * and this pins that rather than pretending otherwise.**
   *
   * It is reachable: the migration logs-and-continues if it cannot move a flat
   * directory, discovery keys by frontmatter name so the flat copy can win the
   * race, and a project-local `.claude/skills/knowledge-lint` shadows the
   * bundled one outright (project roots sort last in `roots()`).
   *
   * `isContextSkill('knowledge-lint', null)` is `CONTEXT_IDS.has('knowledge-lint')`,
   * which is false — the Context row names the BUNDLE. So such a skill is a
   * standalone picker row with the Knowledge switch showing off. An earlier
   * draft of this test asserted the opposite and passed, because its fixture
   * was a skill NAMED `knowledge-bases`, which no code path produces.
   *
   * The fix belongs in the migration, not in a name-matching special case here:
   * a stray copy that shadows the bundled one is a *different* skill on disk
   * and the renderer cannot tell. Asserted so the gap is visible and a future
   * change to it is deliberate.
   */
  it('cannot cover a member that arrives without its bundle', () => {
    const stray: CatalogView = {
      ...contextView,
      skills: [skill('solo', null), skill('knowledge-lint', null)],
      bundles: [],
    };
    expect(standaloneSkills(stray).map((s) => s.name)).toEqual(['solo', 'knowledge-lint']);
  });
});

/**
 * J-P06: `viz-old-tracks` was merged into `viz-new-tracks`; its folder holds a
 * redirect stub the daemon reports with `replacement`.
 */
describe('superseded skill names', () => {
  const stub = (carried: CatalogSkill['carried'] = []): CatalogSkill => ({
    ...skill('viz-old-tracks', 'viz'),
    supersededBy: 'viz/viz-new-tracks',
    replacement: 'viz-new-tracks',
    carried,
  });
  const moved = (carried?: CatalogSkill['carried']): CatalogView => ({
    ...view,
    skills: [...view.skills, skill('viz-new-tracks', 'viz'), stub(carried)],
  });

  it('saves a switch named by the old name under its replacement', () => {
    expect(resolveSkillKeys(moved(), ['viz-old-tracks'], false)).toEqual({
      keys: ['viz-new-tracks'],
      cleared: [],
    });
  });

  /**
   * The machine-wide trap: `disabled[]` still listing the old name would keep
   * the replacement off after its own switch was turned on.
   */
  it('clears the old names when the replacement is switched on machine-wide', () => {
    expect(resolveSkillKeys(moved(), ['viz-new-tracks'], true)).toEqual({
      keys: ['viz-new-tracks'],
      cleared: ['viz-old-tracks'],
    });
    expect(resolveSkillKeys(moved(), ['solo'], true)).toEqual({ keys: ['solo'], cleared: [] });
  });

  it('says in one line when a saved choice was carried, and only then', () => {
    expect(movedChoicesNotice(moved())).toBeNull();
    expect(movedChoicesNotice(moved(['sessionRemoved']))).toBe(
      SKILLS_COPY.movedChoices([['viz-old-tracks', 'viz-new-tracks']])
    );
  });
});
