import { act, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { HistoryEntry } from '../../../api/types.gen';
import { ChangeLogDrawer } from './ChangeLogDrawer';

/**
 * QA 2026-09-10 F13: after digesting a paragraph into a new base the Change log
 * listed only `create knowledge base …`, while `git log` held the two `[ingest]`
 * commits that wrote every page. The history route reads git and was right; the
 * drawer — mounted with the Knowledge view and merely hidden — had read it once,
 * before the digest, and never again.
 *
 * `ChangeLogDrawer.test.tsx` stubs `useHistory` to pin the drawer's markup; this
 * file runs the real hook over a mocked route, because the defect was in WHEN
 * the hook reads.
 */

const mocks = vi.hoisted(() => ({
  listHistory: vi.fn(),
  restoreState: vi.fn(),
  primaryKbId: 'probe' as string | null,
}));

vi.mock('../../../api', () => ({
  listHistory: mocks.listHistory,
  restoreState: mocks.restoreState,
}));

vi.mock('../KnowledgeContext', () => ({
  useKnowledge: () => ({ primaryKbId: mocks.primaryKbId, triggerGraphRefresh: vi.fn() }),
}));

vi.mock('../../../toasts', () => ({ toastError: vi.fn() }));

function commit(sha: string, kind: HistoryEntry['kind'], summary: string): HistoryEntry {
  return { commit_sha: sha.padEnd(40, '0'), kind, summary, timestamp: '2026-09-10T19:30:00Z' };
}

const CREATED = commit('2dc4141', 'manual', 'create knowledge base probe');
const INGESTED_SOURCE = commit('0058ad9', 'ingest', 'ingested pasted-knowledge-b2c5a1');
const INGESTED_PAGES = commit('6f5b82e', 'ingest', 'ingest pasted-knowledge-b2c5a1');

/** What `git log` in the base says, newest first. Tests move it. */
let gitLog: HistoryEntry[] = [];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.primaryKbId = 'probe';
  gitLog = [CREATED];
  mocks.listHistory.mockImplementation(() => Promise.resolve({ data: [...gitLog] }));
});

function drawer(open: boolean) {
  return (
    <ChangeLogDrawer
      open={open}
      onOpenChange={() => undefined}
      onPreview={() => undefined}
      onRestored={() => undefined}
    />
  );
}

function listedSummaries() {
  return within(screen.getByRole('dialog'))
    .queryAllByText(/knowledge base probe|pasted-knowledge/)
    .map((node) => node.textContent);
}

describe('ChangeLogDrawer — what it lists is what git holds', () => {
  it('lists the ingest commits a digest made while the drawer was shut', async () => {
    // The view mounts with the drawer shut; the base is brand new.
    const { rerender } = render(drawer(false));

    // A digest lands: two `[ingest]` commits on top of the create.
    gitLog = [INGESTED_PAGES, INGESTED_SOURCE, CREATED];

    rerender(drawer(true));

    await waitFor(() =>
      expect(listedSummaries()).toEqual([
        'ingest pasted-knowledge-b2c5a1',
        'ingested pasted-knowledge-b2c5a1',
        'create knowledge base probe',
      ])
    );
  });

  it('reads again every time it is opened', async () => {
    const { rerender } = render(drawer(true));
    await waitFor(() => expect(listedSummaries()).toEqual(['create knowledge base probe']));

    rerender(drawer(false));
    gitLog = [INGESTED_PAGES, CREATED];
    rerender(drawer(true));

    await waitFor(() =>
      expect(listedSummaries()).toEqual([
        'ingest pasted-knowledge-b2c5a1',
        'create knowledge base probe',
      ])
    );
  });

  // Nothing to show while shut, so nothing is read while shut.
  it('does not read while it is shut', async () => {
    render(drawer(false));
    await Promise.resolve();
    expect(mocks.listHistory).not.toHaveBeenCalled();
  });

  // The subject can change while a read is out. The older base's answer must
  // not land over the newer one's.
  it("never shows another base's history that answered late", async () => {
    const slow = deferred<{ data: HistoryEntry[] }>();
    mocks.listHistory.mockImplementationOnce(() => slow.promise);
    const { rerender } = render(drawer(true));

    mocks.primaryKbId = 'other';
    const otherLog = [commit('aaaaaaa', 'manual', 'create knowledge base probe-two')];
    mocks.listHistory.mockImplementation(() => Promise.resolve({ data: otherLog }));
    rerender(drawer(true));
    await waitFor(() => expect(listedSummaries()).toEqual(['create knowledge base probe-two']));

    // Let the late answer land completely — resolution, state update, render.
    await act(async () => {
      slow.resolve({ data: [INGESTED_PAGES, INGESTED_SOURCE, CREATED] });
      await slow.promise;
      await new Promise((settle) => setTimeout(settle, 0));
    });
    expect(listedSummaries()).toEqual(['create knowledge base probe-two']);
  });
});
