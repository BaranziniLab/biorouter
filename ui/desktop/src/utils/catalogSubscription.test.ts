import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CatalogDelta } from '../api';
import {
  CATALOG_CHANGED_EVENT,
  changedExtensionKeys,
  newlyInstalledExtensions,
  subscribeToCatalog,
} from './catalogSubscription';

vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../api')>()),
  catalogChanges: vi.fn(),
}));

function delta(revision: number, keys: string[], over: Partial<CatalogDelta> = {}): CatalogDelta {
  return {
    revision,
    changes: keys.map((key, i) => ({
      revision: revision - keys.length + i + 1,
      reason: 'install' as const,
      extensions: [
        {
          key,
          name: key,
          change: 'added' as const,
          enabled: true,
          bundledSkillIds: [],
        },
      ],
      skills: [],
    })),
    truncated: false,
    ...over,
  };
}

/**
 * A poll queue: each call returns the next scripted delta, then parks forever so
 * the loop cannot spin past the script and swamp the test.
 */
function scripted(deltas: CatalogDelta[]) {
  const seen: number[] = [];
  let i = 0;
  const poll = vi.fn(async (since: number) => {
    seen.push(since);
    if (i < deltas.length) return deltas[i++];
    await new Promise(() => {});
    return undefined;
  });
  return { poll, seen };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

/**
 * The loop keeps a small floor under one iteration (`MIN_INTERVAL_MS`) so a
 * subscription answered without parking cannot become a busy wait on the
 * daemon. These tests are about *sequencing*, not wall-clock, so they hand it a
 * sleep that resolves at once — the ordering they assert is unchanged, and they
 * do not have to sit through the floor or encode its value.
 */
const immediate = () => Promise.resolve();

afterEach(() => vi.clearAllMocks());

describe('subscribeToCatalog (issue #112)', () => {
  it('starts from zero and follows the revision it is handed', async () => {
    const onChange = vi.fn();
    const { poll, seen } = scripted([delta(1, ['bioroffice']), delta(2, ['markitdown'])]);

    const stop = subscribeToCatalog({ onChange, poll, sleep: immediate });
    await flush();
    await flush();
    stop();

    expect(seen.slice(0, 3)).toEqual([0, 1, 2]);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('dispatches a window event so non-React consumers can hear it', async () => {
    const heard = vi.fn();
    window.addEventListener(CATALOG_CHANGED_EVENT, heard);
    const { poll } = scripted([delta(1, ['bioroffice'])]);

    const stop = subscribeToCatalog({ onChange: () => {}, poll });
    await flush();
    stop();
    window.removeEventListener(CATALOG_CHANGED_EVENT, heard);

    expect(heard).toHaveBeenCalledTimes(1);
    const event = heard.mock.calls[0][0] as CustomEvent<CatalogDelta>;
    expect(event.detail.revision).toBe(1);
  });

  /**
   * A timeout is the common case — the daemon parks ~25s and answers with the
   * same revision. Treating that as a change would make every idle app refetch
   * its whole inventory twice a minute.
   */
  it('a poll that timed out with nothing to report is not a change', async () => {
    const onChange = vi.fn();
    const { poll } = scripted([delta(0, []), delta(0, [])]);

    const stop = subscribeToCatalog({ onChange, poll });
    await flush();
    await flush();
    stop();

    expect(onChange).not.toHaveBeenCalled();
  });

  /**
   * ⚠ The daemon's revision resets to 0 when it restarts. A client holding a
   * higher number and no rule for this would park forever on a revision the
   * daemon needs minutes of activity to climb back to — the app would look
   * exactly as stale as before this feature existed.
   */
  it('treats a revision going backwards as a restart, and refetches', async () => {
    const onChange = vi.fn();
    const { poll, seen } = scripted([
      delta(7, ['bioroffice']),
      // Daemon restarted: an empty catalogue at revision 0.
      delta(0, []),
      delta(1, ['markitdown']),
    ]);

    const stop = subscribeToCatalog({ onChange, poll, sleep: immediate });
    await flush();
    await flush();
    await flush();
    stop();

    expect(seen.slice(0, 4)).toEqual([0, 7, 0, 1]);
    expect(onChange).toHaveBeenCalledTimes(3);
  });

  /**
   * `truncated` means the daemon's buffer dropped changes this client never
   * saw. There is nothing to apply, and the client must still be told to look.
   */
  it('a truncated delta is a change even with no rows in it', async () => {
    const onChange = vi.fn();
    const { poll } = scripted([{ revision: 99, changes: [], truncated: true }]);

    const stop = subscribeToCatalog({ onChange, poll });
    await flush();
    stop();

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0][0].truncated).toBe(true);
  });

  it('backs off on a failed poll instead of spinning', async () => {
    const onChange = vi.fn();
    const sleep = vi.fn(async () => {});
    let calls = 0;
    const poll = vi.fn(async () => {
      calls += 1;
      if (calls === 1) throw new Error('daemon restarting');
      if (calls === 2) return delta(1, ['bioroffice']);
      await new Promise(() => {});
      return undefined;
    });

    const stop = subscribeToCatalog({ onChange, poll, sleep, retryDelayMs: 1 });
    await flush();
    await flush();
    stop();

    expect(sleep).toHaveBeenCalledWith(1);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  /**
   * The guard that matters is the one AFTER the await: a poll parked when the
   * provider unmounted resolves into a component that is gone, and applying it
   * would set state on a dead tree.
   */
  it('a poll that resolves after unsubscribing changes nothing', async () => {
    const onChange = vi.fn();
    let release: (d: CatalogDelta) => void = () => {};
    const poll = vi.fn(
      () =>
        new Promise<CatalogDelta>((resolve) => {
          release = resolve;
        })
    );

    const stop = subscribeToCatalog({ onChange, poll });
    await flush();
    stop();
    release(delta(1, ['bioroffice']));
    await flush();
    await flush();

    expect(poll).toHaveBeenCalledTimes(1);
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe('reading a delta', () => {
  it('names every extension key a delta touched, once', () => {
    const d = delta(2, ['bioroffice', 'bioroffice']);
    expect(changedExtensionKeys(d)).toEqual(['bioroffice']);
  });

  /** An extension installed but left disabled is not something to offer. */
  it('only reports additions that are actually enabled', () => {
    const d = delta(1, ['bioroffice']);
    d.changes![0].extensions![0].enabled = false;
    expect(newlyInstalledExtensions(d)).toEqual([]);

    d.changes![0].extensions![0].enabled = true;
    expect(newlyInstalledExtensions(d)).toEqual([{ key: 'bioroffice', name: 'bioroffice' }]);
  });

  /**
   * ⚠ **A first run is not six installs.** The daemon writes its own bundled
   * baseline into the catalogue the first time it starts, and every entry
   * arrives as `added` + `enabled` — so a brand-new user, who had set nothing
   * up, met a stack of "Extension installed. Turn it on for this chat…" toasts
   * naming `developer`, `computercontroller`, `autovisualiser`, `memory`,
   * `knowledge` and `agent_drafter`. The notification exists for an install
   * made somewhere else that this chat can now opt into; Biorouter's own
   * baseline is never that.
   */
  it("does not announce Biorouter's own bundled extensions", () => {
    const d = delta(1, ['developer']);
    d.changes![0].extensions![0].config = {
      name: 'developer',
      description: 'Developer tools',
      type: 'builtin',
      bundled: true,
    } as never;
    expect(newlyInstalledExtensions(d)).toEqual([]);

    // A platform extension is the same case by a different config shape: only
    // Biorouter can register an in-process server.
    const p = delta(2, ['workspace']);
    p.changes![0].extensions![0].config = {
      name: 'workspace',
      description: 'Workspace Control',
      type: 'platform',
    } as never;
    expect(newlyInstalledExtensions(p)).toEqual([]);
  });

  it('still announces a third-party install, and one with no config at all', () => {
    // The control. Silencing the baseline must not silence the case the
    // notification was written for.
    const d = delta(3, ['bioroffice']);
    d.changes![0].extensions![0].config = {
      name: 'bioroffice',
      description: 'Office documents',
      type: 'stdio',
      cmd: 'bioroffice',
      args: [],
    } as never;
    expect(newlyInstalledExtensions(d)).toEqual([{ key: 'bioroffice', name: 'bioroffice' }]);

    // No config: fail towards NOTIFYING. One extra notification is a smaller
    // harm than silently swallowing a real third-party install.
    const bare = delta(4, ['mystery']);
    expect(newlyInstalledExtensions(bare)).toEqual([{ key: 'mystery', name: 'mystery' }]);
  });

  /** A toggle is not an install; offering to attach one would be noise. */
  it('does not treat an enable as a new install', () => {
    const d = delta(1, ['bioroffice']);
    d.changes![0].extensions![0].change = 'enabled';
    expect(newlyInstalledExtensions(d)).toEqual([]);
  });
});
