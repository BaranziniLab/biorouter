import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { fakeConnection, makeCrew, renderWithCrew } from '../onboarding/testCrew';
import { readsCopy } from './copy';
import { PersonReads } from './PersonReads';

/**
 * The chats that read a workspace as the person (account ownership AO-12's read consent), in the
 * chat access pane: one row per workspace, each with Revoke (`DELETE /crew/sessions/{session}/reads
 * /{connection}`).
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const lab = fakeConnection({ id: 'conn-1', name: 'lab' });
const imaging = fakeConnection({ id: 'conn-2', name: 'imaging' });

function grants(connectionId: string, sessions: string[]) {
  return {
    grants: [],
    replaced_grants: [],
    person_reads: [
      ...sessions.map((session) => ({
        session_id: session,
        connection_id: connectionId,
        granted_at: 1_700_000_000,
        session_name: 'Methods review',
      })),
      // Another connection's row on this list is never this connection's.
      { session_id: 'chat-1', connection_id: 'conn-9', granted_at: 1 },
    ],
  };
}

function renderReads(sessionId: string | null = 'chat-1') {
  return renderWithCrew(
    <PersonReads sessionId={sessionId} />,
    makeCrew({ connections: [lab, imaging], connectionId: 'conn-1', connection: lab })
  );
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (method === 'GET' && path === '/connections/conn-1/grants')
      return grants('conn-1', ['chat-1', 'chat-2']);
    if (method === 'GET' && path === '/connections/conn-2/grants')
      return grants('conn-2', ['chat-2']);
    if (method === 'DELETE') return null;
    return {};
  });
});

describe('PersonReads', () => {
  it('lists the workspaces this chat reads as the person, and nothing of other chats', async () => {
    renderReads();
    const section = await screen.findByTestId('crew-person-reads');
    expect(within(section).getByText(readsCopy.reads('lab'))).toBeVisible();
    expect(within(section).queryByText(readsCopy.reads('imaging'))).toBeNull();
    expect(within(section).getAllByRole('listitem')).toHaveLength(1);
  });

  it('takes a consent back with Revoke', async () => {
    renderReads();
    const section = await screen.findByTestId('crew-person-reads');
    fireEvent.click(within(section).getByRole('button', { name: readsCopy.revokeLabel('lab') }));
    await waitFor(() =>
      expect(mocks.crewHttp).toHaveBeenCalledWith('/sessions/chat-1/reads/conn-1', 'DELETE')
    );
    expect(await screen.findByText(readsCopy.revoked('lab'))).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: readsCopy.revokeLabel('lab') })).toBeNull();
  });

  it('says so when there was nothing left to take back, and keeps a refusal visible', async () => {
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (method === 'GET') return path.includes('conn-1') ? grants('conn-1', ['chat-1']) : {};
      throw new CrewHttpError('No such consent.', 404, 'crew_not_found');
    });
    renderReads();
    const section = await screen.findByTestId('crew-person-reads');
    fireEvent.click(within(section).getByRole('button', { name: readsCopy.revokeLabel('lab') }));
    expect(await screen.findByText(readsCopy.gone)).toBeInTheDocument();
  });

  it('shows nothing without a chat or a consent', async () => {
    const { container } = renderReads('chat-3');
    await waitFor(() => expect(mocks.crewHttp).toHaveBeenCalledTimes(2));
    expect(container).toBeEmptyDOMElement();
  });
});
