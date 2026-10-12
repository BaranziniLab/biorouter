import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CatalogSkill, CatalogView } from '../../api';
import { CATALOG_CHANGED_EVENT, useSkillCatalog } from './useSkillCatalog';

/**
 * Only the newest read may answer (WS-COMPOSER-5, V2-R7-3).
 *
 * `reload` used to commit whatever answered, in arrival order. The composer's
 * mount can bind another chat while the previous chat's read is still out (a tab
 * that keeps its mount, or a fast switch), and that older read would land after
 * the next chat's and replace its view. `loading` went false at the same moment,
 * while the newer read was still outstanding.
 *
 * The daemon is a set of promises the test settles by hand, so an order of
 * arrival is a thing the test chooses rather than a race it hopes to win.
 */

const mocks = vi.hoisted(() => ({
  skillCatalogHandler: vi.fn(),
  refreshSkillCatalog: vi.fn(),
  setSessionSkills: vi.fn(),
}));

vi.mock('../../api', () => ({
  skillCatalogHandler: (...args: unknown[]) => mocks.skillCatalogHandler(...args),
  refreshSkillCatalog: (...args: unknown[]) => mocks.refreshSkillCatalog(...args),
  setSessionSkills: (...args: unknown[]) => mocks.setSessionSkills(...args),
}));

vi.mock('../../store/skillOverrides', () => ({
  loadSkillOverrides: vi.fn(async () => true),
  saveSkillOverrides: vi.fn(async () => undefined),
  setSkillOverride: vi.fn(),
  isSkillEnabled: () => true,
  getSkillOverrides: () => new Map<string, boolean>(),
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: vi.fn(async () => ({})),
}));

function skill(name: string): CatalogSkill {
  return {
    name,
    description: `${name} description`,
    slug: name,
    directory: `/skills/${name}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    bundle: null,
    builtin: false,
    state: {
      machineEnabled: true,
      session: 'default',
      sessionViaBundle: false,
      hiddenContext: false,
      effective: true,
    },
  };
}

function view(generation: number, ...names: string[]): CatalogView {
  return { generation, roots: [], skills: names.map(skill), bundles: [] };
}

/** One read the daemon has been asked for and has not answered yet. */
interface Pending {
  query: { session_id?: string };
  answer: (next: CatalogView) => Promise<void>;
  fail: (message: string) => Promise<void>;
}

/** Every read the hook asks for is held here until the test settles it. */
function holdReads(): Pending[] {
  const held: Pending[] = [];
  const hold = (options: { query: { session_id?: string } }) =>
    new Promise((resolve, reject) => {
      held.push({
        query: options.query,
        answer: async (next) => {
          await act(async () => {
            resolve({ data: next });
          });
        },
        fail: async (message) => {
          await act(async () => {
            reject(new Error(message));
          });
        },
      });
    });
  mocks.skillCatalogHandler.mockImplementation(hold);
  mocks.refreshSkillCatalog.mockImplementation(hold);
  return held;
}

const names = (hook: { current: { skills: CatalogSkill[] } }) =>
  hook.current.skills.map((s) => s.name);

beforeEach(() => {
  vi.clearAllMocks();
});

describe('useSkillCatalog: only the newest read answers', () => {
  it('keeps the next chat’s view when the previous chat’s read lands after it', async () => {
    const reads = holdReads();
    const { result, rerender } = renderHook(({ id }) => useSkillCatalog(id), {
      initialProps: { id: 'chat-a' as string | null },
    });
    expect(reads.map((r) => r.query.session_id)).toEqual(['chat-a']);

    rerender({ id: 'chat-b' });
    expect(reads.map((r) => r.query.session_id)).toEqual(['chat-a', 'chat-b']);

    await reads[1].answer(view(2, 'b-skill'));
    expect(names(result)).toEqual(['b-skill']);
    expect(result.current.loading).toBe(false);

    // The older read lands now. It belongs to the chat this hook served before.
    await reads[0].answer(view(1, 'a-skill'));
    expect(names(result), 'the view is still the next chat’s').toEqual(['b-skill']);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('stays loading until the newest read answers, whichever read lands first', async () => {
    const reads = holdReads();
    const { result, rerender } = renderHook(({ id }) => useSkillCatalog(id), {
      initialProps: { id: 'chat-a' as string | null },
    });
    rerender({ id: 'chat-b' });

    // Chat A's read lands while chat B's is still out.
    await reads[0].answer(view(1, 'a-skill'));
    expect(result.current.loading, 'B has not answered, so the hook is still loading').toBe(true);
    expect(names(result), 'A’s view is not committed under B').toEqual([]);

    await reads[1].answer(view(2, 'b-skill'));
    expect(result.current.loading).toBe(false);
    expect(names(result)).toEqual(['b-skill']);
  });

  it('does not let the previous chat’s failed read set an error or end loading', async () => {
    const reads = holdReads();
    const { result, rerender } = renderHook(({ id }) => useSkillCatalog(id), {
      initialProps: { id: 'chat-a' as string | null },
    });
    rerender({ id: 'chat-b' });

    await reads[0].fail('chat A went away');
    expect(result.current.error).toBeNull();
    expect(result.current.loading).toBe(true);

    await reads[1].answer(view(2, 'b-skill'));
    expect(result.current.error).toBeNull();
    expect(result.current.loading).toBe(false);
    expect(names(result)).toEqual(['b-skill']);
  });

  it('still reports the newest read’s own failure', async () => {
    const reads = holdReads();
    const { result, rerender } = renderHook(({ id }) => useSkillCatalog(id), {
      initialProps: { id: 'chat-a' as string | null },
    });
    rerender({ id: 'chat-b' });

    await reads[1].fail('the daemon is not there');
    expect(result.current.error).toBe('Could not read the skill catalog: the daemon is not there');
    expect(result.current.loading).toBe(false);

    // And the older read, answering later, does not clear it or replace the view.
    await reads[0].answer(view(1, 'a-skill'));
    expect(result.current.error).toBe('Could not read the skill catalog: the daemon is not there');
    expect(names(result)).toEqual([]);
  });

  it('applies the same rule to a rescan that overtakes the first read for one chat', async () => {
    const reads = holdReads();
    const { result } = renderHook(() => useSkillCatalog('chat-a'));
    expect(reads).toHaveLength(1);

    // An extension install announces itself while the first read is still out.
    act(() => {
      window.dispatchEvent(new CustomEvent(CATALOG_CHANGED_EVENT, { detail: { revision: 1 } }));
    });
    expect(reads).toHaveLength(2);
    expect(mocks.refreshSkillCatalog).toHaveBeenCalledTimes(1);

    await reads[1].answer(view(2, 'old-skill', 'new-skill'));
    await reads[0].answer(view(1, 'old-skill'));
    expect(names(result), 'the rescan that saw the install stays').toEqual([
      'old-skill',
      'new-skill',
    ]);
    expect(result.current.loading).toBe(false);
  });

  it('settles an ordinary single read as before', async () => {
    const reads = holdReads();
    const { result } = renderHook(() => useSkillCatalog('chat-a'));
    expect(result.current.loading).toBe(true);

    await reads[0].answer(view(1, 'only'));
    expect(names(result)).toEqual(['only']);
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });
});
