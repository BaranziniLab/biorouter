import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import type { CrewController } from '../state/types';
import { nameSuggestionCopy } from './copy';
import { readJoinContext, resetJoinContextForTests, updateJoinContext } from './joinContext';
import { LEGACY_JOIN_STATUS } from './JoinStatusCard';
import { NameSuggestionNote } from './NameSuggestionNote';
import { useJoinProbe } from './useJoinProbe';
import { fakeConnection, fakeSnapshot, makeCrew, renderWithCrew } from './testCrew';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

function Probe() {
  useJoinProbe();
  return null;
}

const UNKNOWN_DEVICE =
  'unauthorized: unknown device Your unsent draft is retained for this channel. Retry to verify access before sending.';

function renderProbe(overrides: Partial<CrewController> = {}) {
  const connection = fakeConnection();
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection,
    connections: [connection],
    ...overrides,
  });
  return { crew, view: renderWithCrew(<Probe />, crew) };
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  resetJoinContextForTests();
});

describe('useJoinProbe', () => {
  it('reports the join status of a computer the workspace refused as an unknown device', async () => {
    mocks.crewHttp.mockResolvedValue({ status: 'invited', code: '7QK2M9XA3JTPWZ4D' });
    const { crew } = renderProbe({ refreshError: UNKNOWN_DEVICE });
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('invited'));
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/join',
      'GET',
      undefined,
      expect.any(AbortSignal)
    );
  });

  it('asks right after Join saved the connection, before any refusal', async () => {
    updateJoinContext('conn-1', { joining: true });
    mocks.crewHttp.mockResolvedValue({ status: 'not_invited' });
    const { crew } = renderProbe();
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith('not_invited'));
  });

  it('sends a computer the workspace does not know to the token path on an older server', async () => {
    mocks.crewHttp.mockResolvedValue({ status: 'unsupported' });
    const { crew } = renderProbe({ refreshError: UNKNOWN_DEVICE });
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith(LEGACY_JOIN_STATUS));
  });

  it('does the same when the background service predates the join route', async () => {
    mocks.crewHttp.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    const { crew } = renderProbe({ refreshError: UNKNOWN_DEVICE });
    await waitFor(() => expect(crew.setJoinStatus).toHaveBeenCalledWith(LEGACY_JOIN_STATUS));
  });

  it('never sends a member whose updates failed for another reason to the join screen', async () => {
    const { crew } = renderProbe({ refreshError: 'Crew observation failed.' });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(mocks.crewHttp).not.toHaveBeenCalled();
    expect(crew.setJoinStatus).not.toHaveBeenCalled();
  });

  it('asks nothing of a disconnected, verified or already reported connection', async () => {
    renderProbe({
      refreshError: UNKNOWN_DEVICE,
      connection: fakeConnection({ status: 'disconnected' }),
    });
    renderProbe({ refreshError: UNKNOWN_DEVICE, joinStatus: 'invited' });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });

  it('forgets the joining flag once the workspace verifies this computer', async () => {
    updateJoinContext('conn-1', { joining: true });
    renderProbe({
      snapshot: fakeSnapshot(),
      observedPrivacy: {
        connectionId: 'conn-1',
        mode: 'private',
        institutionId: 'ucsf',
        policyEpoch: 1,
      },
    });
    await waitFor(() => expect(readJoinContext('conn-1').joining).toBe(false));
  });
});

describe('NameSuggestionNote', () => {
  const verified = {
    connectionId: 'conn-1',
    snapshot: fakeSnapshot(),
    observedPrivacy: {
      connectionId: 'conn-1',
      mode: 'private' as const,
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
  };

  it('offers the server-account name once after joining, and applies it only on Use', async () => {
    updateJoinContext('conn-1', { suggestName: true });
    const crew = makeCrew({
      ...verified,
      request: vi.fn().mockResolvedValue({ full_name: 'Bob Lee' }) as CrewController['request'],
    });
    renderWithCrew(<NameSuggestionNote />, crew);

    expect(
      await screen.findByText(nameSuggestionCopy.prompt('Bob Lee', 'lab'))
    ).toBeInTheDocument();
    expect(crew.request).toHaveBeenCalledWith('profile.suggest', {}, expect.anything());
    expect(crew.mutate).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: nameSuggestionCopy.use }));
    await waitFor(() =>
      expect(crew.mutate).toHaveBeenCalledWith('profile.update', {
        nickname: 'Bob Lee',
        avatar: null,
      })
    );
    await waitFor(() => expect(readJoinContext('conn-1').suggestName).toBe(false));
  });

  it('opens Edit profile instead, and offers nothing to someone who already chose a name', async () => {
    updateJoinContext('conn-1', { suggestName: true });
    const crew = makeCrew({
      ...verified,
      request: vi.fn().mockResolvedValue({ full_name: 'Bob Lee' }) as CrewController['request'],
    });
    const view = renderWithCrew(<NameSuggestionNote />, crew);
    fireEvent.click(await screen.findByRole('button', { name: nameSuggestionCopy.edit }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'edit-profile' });
    view.unmount();

    updateJoinContext('conn-1', { suggestName: true });
    const named = fakeSnapshot({
      actor: { id: 'p-bob', uid: 1001, username: 'bob', nickname: 'Robert' },
    });
    const other = makeCrew({ ...verified, snapshot: named });
    renderWithCrew(<NameSuggestionNote />, other);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(other.request).not.toHaveBeenCalled();
    expect(screen.queryByRole('status')).toBeNull();
  });
});
