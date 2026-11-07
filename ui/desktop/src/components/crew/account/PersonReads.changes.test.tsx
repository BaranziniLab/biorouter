import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeConnection, makeCrew, renderWithCrew } from '../onboarding/testCrew';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { readsCopy } from './copy';
import { PersonReads } from './PersonReads';

/**
 * "Reads as you" follows Crew's change feed (WI-15): a chat that revokes its own reading, from the
 * chat, drops the row here without a click.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const lab = fakeConnection({ id: 'conn-1', name: 'lab' });

let reading = true;
const readCalls = () =>
  mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections/conn-1/grants');

beforeEach(() => {
  reading = true;
  mocks.crewHttp.mockReset();
  mocks.crewHttp.mockImplementation(async (path: string) =>
    path === '/connections/conn-1/grants'
      ? {
          grants: [],
          replaced_grants: [],
          person_reads: reading
            ? [{ session_id: 'chat-1', connection_id: 'conn-1', granted_at: 1_700_000_000 }]
            : [],
        }
      : {}
  );
});

const change = (connectionId: string) => ({
  revision: 4,
  truncated: false,
  changes: [
    {
      revision: 4,
      kind: 'grants_revoke',
      connection_id: connectionId,
      channel_id: null,
      session_id: 'chat-1',
      run_id: null,
      job_id: null,
    },
  ],
});

describe('PersonReads and the change feed', () => {
  it('drops a read consent the chat revoked', async () => {
    renderWithCrew(
      <PersonReads sessionId="chat-1" />,
      makeCrew({ connections: [lab], connectionId: 'conn-1', connection: lab })
    );
    expect(await screen.findByText(readsCopy.reads('lab'))).toBeInTheDocument();
    reading = false;
    dispatchCrewChanged(change('conn-1'));
    await waitFor(() => expect(screen.queryByTestId('crew-person-reads')).toBeNull());
    expect(readCalls()).toHaveLength(2);
  });

  it('reads nothing for a change on a workspace it does not list', async () => {
    renderWithCrew(
      <PersonReads sessionId="chat-1" />,
      makeCrew({ connections: [lab], connectionId: 'conn-1', connection: lab })
    );
    await screen.findByText(readsCopy.reads('lab'));
    dispatchCrewChanged(change('conn-9'));
    await new Promise((resolve) => setTimeout(resolve, CREW_CHANGED_DEBOUNCE_MS * 2));
    expect(readCalls()).toHaveLength(1);
  });
});
