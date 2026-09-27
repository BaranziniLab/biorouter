import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';

/**
 * What the daemon answers when a Crew grant restricts the chat, as the
 * generated client throws it under `throwOnError`: the parsed plain-text body.
 * Mirrored from `CREW_EXPORT_REFUSAL` in
 * `crates/biorouter/src/session/session_manager.rs`.
 */
const CREW_EXPORT_REFUSAL =
  'Crew context cannot be exported without its channel permissions. Share an authorized ' +
  'message or attachment from Crew instead.';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  exportSession: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: mocks.exportSession,
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const crewChat = {
  id: 'crew-task',
  name: 'Crew task',
  working_dir: '/tmp',
  created_at: '2026-09-27T12:00:00Z',
  updated_at: '2026-09-27T12:00:00Z',
  extension_data: {},
  message_count: 2,
} as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [crewChat] } });
});

async function clickExport() {
  const user = userEvent.setup();
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await user.click(await screen.findByRole('button', { name: 'Export Crew task' }));
}

describe('SessionListView export refusals', () => {
  it("shows the daemon's sentence when a Crew chat's export is refused, and downloads nothing", async () => {
    mocks.exportSession.mockRejectedValue(CREW_EXPORT_REFUSAL);
    const createObjectURL = vi.fn();
    Object.defineProperty(URL, 'createObjectURL', { value: createObjectURL, configurable: true });

    await clickExport();

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: "Couldn't export this chat",
        msg: CREW_EXPORT_REFUSAL,
      })
    );
    expect(mocks.exportSession).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { session_id: 'crew-task' },
        headers: { 'X-User-Action': 'test-proof' },
      })
    );
    expect(createObjectURL).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('says the export failed in words, not a status, when the daemon gives no sentence', async () => {
    // An empty error body is thrown as `{}` by the generated client.
    mocks.exportSession.mockRejectedValue({});

    await clickExport();

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: "Couldn't export this chat",
        msg: 'The chat could not be read for export. Nothing was downloaded.',
      })
    );
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });
});
