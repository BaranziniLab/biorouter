import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { rejoinCopy } from '../account/copy';
import { resetBuildBrokerVersionForTests } from '../account/brokerVersion';
import { markNeedsRejoin, needsRejoin, resetRejoinMarksForTests } from '../account/rejoinMarks';
import type { CrewConnection } from '../crewApi';
import { resetSetupStartedHereForTests } from '../onboarding/setup';
import { fakeConnection, fakeSnapshot, makeCrew, renderWithCrew } from '../onboarding/testCrew';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { ConnectionBar } from './ConnectionBar';

/**
 * Add this computer again follows Crew's change feed (WI-15): when this computer is added again
 * somewhere else (a chat's `crew__change rejoin`, another window, `biorouter crew rejoin`), the
 * note this window put up after Keys removed it goes away without a click here.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});
vi.mock('../auth/SignInDialog', () => ({ SignInDialog: () => null }));

function renderBar(connection: CrewConnection) {
  const crew = makeCrew({
    connectionId: connection.id,
    connection,
    connections: [connection],
    snapshot: fakeSnapshot(),
    observedPrivacy: {
      connectionId: connection.id,
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    screen: 'channel',
  });
  return renderWithCrew(<ConnectionBar />, crew);
}

const changed = (kind: string, connectionId: string | null, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind,
      connection_id: connectionId,
      channel_id: null,
      session_id: null,
      run_id: null,
      job_id: null,
    },
  ],
});

const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  mocks.crewHttp.mockReset();
  mocks.crewHttp.mockImplementation(async (path: string) => (path === '/setup' ? [] : {}));
  resetRejoinMarksForTests();
  resetSetupStartedHereForTests();
  resetBuildBrokerVersionForTests();
});

describe('ConnectionBar and the change feed', () => {
  it('drops Add this computer again once this computer was added again elsewhere', async () => {
    markNeedsRejoin('conn-1');
    renderBar(fakeConnection());
    expect(await screen.findByTestId('crew-rejoin')).toHaveTextContent(
      rejoinCopy.removedHere('lab')
    );
    dispatchCrewChanged(changed('rejoin', 'conn-1'));
    await waitFor(() => expect(screen.queryByTestId('crew-rejoin')).toBeNull());
    expect(needsRejoin('conn-1')).toBe(false);
  });

  it('keeps it for a rejoin of another connection, and for any other change', async () => {
    markNeedsRejoin('conn-1');
    renderBar(fakeConnection());
    await screen.findByTestId('crew-rejoin');
    dispatchCrewChanged(changed('rejoin', 'conn-9'));
    dispatchCrewChanged(changed('connect', 'conn-1', 2));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(needsRejoin('conn-1')).toBe(true);
    expect(screen.getByTestId('crew-rejoin')).toBeInTheDocument();
  });
});
