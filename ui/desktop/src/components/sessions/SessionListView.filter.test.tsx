/**
 * History's search is the band's `FilterInput` (spec 3.4 and 2.6): it narrows
 * the list to the chats whose name, folder or id contain the text, it shows
 * every match at once, and ⌘F focuses it. It used to be `SearchView`, the
 * transcript's find bar, whose counter stepped through painted marks; that bar
 * is the transcript's find overlay now and is no longer a list filter.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';
import { HISTORY_NO_MATCH_TITLE } from './copy';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const session = (id: string, name: string, workingDir = '/Users/someone/Desktop'): Session =>
  ({
    id,
    name,
    working_dir: workingDir,
    created_at: '2026-09-12T12:00:00Z',
    updated_at: '2026-09-12T12:00:00Z',
    extension_data: {},
    message_count: 2,
  }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({
    data: {
      sessions: [
        session('20260912_1', 'Desktop cleanup'),
        session('20260912_2', 'Desktop icons'),
        session('20260912_3', 'Variant calling', '/Users/someone/genomics'),
      ],
    },
  });
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

async function renderHistory() {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText('Desktop cleanup');
  return screen.getByRole('searchbox', { name: 'Search history' });
}

const rowNames = () =>
  screen
    .getAllByRole('button', { name: /^Open chat / })
    .map((button) => button.getAttribute('aria-label')?.replace('Open chat ', ''));

describe('SessionListView — the band filter', () => {
  it('narrows the list to the chats whose name matches, ignoring case', async () => {
    const filter = await renderHistory();
    fireEvent.change(filter, { target: { value: 'ICONS' } });

    await waitFor(() => expect(rowNames()).toEqual(['Desktop icons']));
  });

  it('matches the folder a chat ran in, and its id', async () => {
    const filter = await renderHistory();
    fireEvent.change(filter, { target: { value: 'genomics' } });
    await waitFor(() => expect(rowNames()).toEqual(['Variant calling']));

    fireEvent.change(filter, { target: { value: '20260912_1' } });
    await waitFor(() => expect(rowNames()).toEqual(['Desktop cleanup']));
  });

  it('says so when nothing matches, and Escape brings every chat back', async () => {
    const filter = await renderHistory();
    fireEvent.change(filter, { target: { value: 'no such chat' } });
    expect(await screen.findByRole('heading', { name: HISTORY_NO_MATCH_TITLE })).toBeVisible();

    fireEvent.keyDown(filter, { key: 'Escape' });
    await waitFor(() => expect(rowNames()).toHaveLength(3));
  });

  it('is focused by ⌘F while History is open', async () => {
    const filter = await renderHistory();
    expect(document.activeElement).not.toBe(filter);
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    expect(document.activeElement).toBe(filter);
  });

  it('sits in the band, beside the title', async () => {
    const filter = await renderHistory();
    const band = filter.closest('[data-band]') as HTMLElement;
    expect(band).not.toBeNull();
    expect(within(band).getByRole('heading', { level: 1 })).toHaveTextContent('Chat history');
  });
});
