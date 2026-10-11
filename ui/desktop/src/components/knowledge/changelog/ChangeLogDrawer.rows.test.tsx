import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HistoryEntry } from '../../../api/types.gen';
import { knowledgeCopy } from '../copy';
import { ChangeLogDrawer } from './ChangeLogDrawer';

/**
 * V3-10: a change-log entry rests as its kind, its time and its summary. Preview (an icon) and
 * a `⋯` holding "Restore…" are a RowActions cluster, shown on hover or focus (the rule is
 * `.br-row-actions` in main.css; jsdom draws no CSS, so these pin the markup it hangs on). The
 * newest commit has no Restore: restoring to the state the base is already in restores nothing.
 */

const mocks = vi.hoisted(() => ({
  history: [] as HistoryEntry[],
  restore: vi.fn(),
  triggerGraphRefresh: vi.fn(),
}));

vi.mock('../KnowledgeContext', () => ({
  useKnowledge: () => ({ primaryKbId: 'kb-1', triggerGraphRefresh: mocks.triggerGraphRefresh }),
}));

vi.mock('../hooks/useHistory', () => ({
  useHistory: () => ({
    history: mocks.history,
    loading: false,
    error: null,
    refresh: vi.fn(),
    restore: mocks.restore,
  }),
}));

vi.mock('../../../toasts', () => ({ toastError: vi.fn() }));

function commit(sha: string, kind: HistoryEntry['kind'], summary: string): HistoryEntry {
  return { commit_sha: sha.padEnd(40, '0'), kind, summary, timestamp: '2026-09-10T19:30:00Z' };
}

const NEWEST = commit('aaaaaaa', 'ingest', 'ingest pasted notes');
const OLDER = commit('bbbbbbb', 'link', 'linked two pages');
const OLDEST = commit('ccccccc', 'manual', 'create knowledge base soul');

beforeEach(() => {
  vi.clearAllMocks();
  mocks.history = [NEWEST, OLDER, OLDEST];
  mocks.restore.mockResolvedValue('ddddddd');
});
afterEach(() => cleanup());

function drawer(props: { onPreview?: (sha: string) => void; onRestored?: () => void } = {}) {
  return render(
    <ChangeLogDrawer
      open
      onOpenChange={() => undefined}
      onPreview={props.onPreview ?? (() => undefined)}
      onRestored={props.onRestored ?? (() => undefined)}
    />
  );
}

const entryRow = (summary: string) =>
  screen.getByText(summary).closest('.biorouter-list-row') as HTMLElement;

describe('ChangeLogDrawer entries', () => {
  it('shows no labelled Preview or Restore button on an entry', () => {
    drawer();
    const row = entryRow('linked two pages');
    // The two buttons are an icon and a ⋯, named but not spelled out beside the summary.
    expect(within(row).queryByText('Preview')).toBeNull();
    expect(within(row).queryByText(/^Restore/)).toBeNull();
    expect(within(row).getByRole('button', { name: knowledgeCopy.drawers.preview })).toBe(
      row.querySelector('.br-row-actions [data-row-action]')
    );
    // Both live in the cluster the hover rule reveals, and the time shares its slot.
    expect(row.querySelector('.br-row-actions-slot .br-row-actions-meta')).not.toBeNull();
  });

  it('previews through the icon action', async () => {
    const onPreview = vi.fn();
    drawer({ onPreview });
    await userEvent.click(
      within(entryRow('linked two pages')).getByRole('button', {
        name: knowledgeCopy.drawers.preview,
      })
    );
    expect(onPreview).toHaveBeenCalledWith(OLDER.commit_sha);
  });

  it('offers Restore… behind the ⋯ of an older entry, and asks first', async () => {
    const onRestored = vi.fn();
    drawer({ onRestored });
    const row = entryRow('linked two pages');
    await userEvent.click(within(row).getByRole('button', { name: 'More actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Restore…' }));

    // Nothing is restored until the dialog is confirmed.
    expect(mocks.restore).not.toHaveBeenCalled();
    const dialog = await screen.findByRole('dialog', { name: 'Restore knowledge base?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Restore' }));
    await waitFor(() => expect(mocks.restore).toHaveBeenCalledWith(OLDER.commit_sha));
    await waitFor(() => expect(onRestored).toHaveBeenCalled());
  });

  it('offers no Restore on the newest commit, even when the filter hides the rest', async () => {
    drawer();
    const head = entryRow('ingest pasted notes');
    expect(within(head).queryByRole('button', { name: 'More actions' })).toBeNull();
    // Preview is still there.
    expect(
      within(head).getByRole('button', { name: knowledgeCopy.drawers.preview })
    ).toBeInTheDocument();

    // Hide every kind but `link`: the newest commit is not among them, and no other entry is
    // promoted to head by the filter.
    for (const name of [knowledgeCopy.changeKind.ingest, knowledgeCopy.changeKind.manual]) {
      await userEvent.click(screen.getByRole('button', { name }));
    }
    expect(screen.queryByText('ingest pasted notes')).toBeNull();
    expect(
      within(entryRow('linked two pages')).getByRole('button', { name: 'More actions' })
    ).toBeInTheDocument();
  });
});

describe('ChangeLogDrawer kind chips', () => {
  const chip = (name: string) => screen.getByRole('button', { name });

  it('rest in the ordinary fill while nothing is filtered, and take the tint once narrowed', async () => {
    drawer();
    const kinds = Object.values(knowledgeCopy.changeKind) as string[];
    expect(kinds).toHaveLength(7);
    for (const name of kinds) {
      expect(chip(name)).toHaveAttribute('aria-pressed', 'true');
      expect(chip(name).className).not.toMatch(/tint-selected/);
    }

    await userEvent.click(chip(knowledgeCopy.changeKind.manual));
    expect(chip(knowledgeCopy.changeKind.manual)).toHaveAttribute('aria-pressed', 'false');
    expect(chip(knowledgeCopy.changeKind.manual).className).not.toMatch(/tint-selected/);
    // What is still shown now reads as selected against what is not.
    expect(chip(knowledgeCopy.changeKind.ingest).className).toMatch(/tint-selected/);

    // Back to all on: quiet again.
    await userEvent.click(chip(knowledgeCopy.changeKind.manual));
    expect(chip(knowledgeCopy.changeKind.ingest).className).not.toMatch(/tint-selected/);
  });
});
