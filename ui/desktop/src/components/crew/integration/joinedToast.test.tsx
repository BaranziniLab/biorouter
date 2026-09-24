import { act, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  alice,
  bob,
  channelReady,
  connection,
  currentCrew,
  installDaemon,
  richSnapshot,
  renderCrew,
  type ScriptedDaemon,
} from './harness';

const toasts = vi.hoisted(() => ({ toastSuccess: vi.fn() }));
vi.mock('../../../toasts', async () => {
  const actual = await vi.importActual<typeof import('../../../toasts')>('../../../toasts');
  return { ...actual, toastSuccess: toasts.toastSuccess };
});
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

const dave = {
  id: '5e4d3c2b-1a09-4f8e-9d7c-6b5a4f3e2d1c',
  uid: 70304,
  username: 'dave',
  nickname: 'Dave Kim',
  display_name: 'Dave Kim',
  active: true,
};

/** The observer's next state frame, as the daemon sends it when the workspace changes. */
function stateFrame(daemon: ScriptedDaemon) {
  return {
    type: 'state',
    connection_id: connection.id,
    connection_mode: 'private',
    connection_policy_epoch: 1,
    connection_institution_id: connection.institution_id,
    snapshot: daemon.state.snapshot,
    runs: daemon.state.runs,
    cursor: null,
  };
}

describe('the host hears when someone joins (a result that happens off-screen)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('says once who joined, and nothing for the people already there', async () => {
    const daemon = installDaemon();
    renderCrew();
    await channelReady();
    expect(toasts.toastSuccess).not.toHaveBeenCalled();

    daemon.state.snapshot = richSnapshot({ principals: [alice, bob, dave], pending_joins: [] });
    act(() => daemon.emit(stateFrame(daemon)));

    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({ msg: 'Dave Kim (@dave) joined lab' })
    );
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);

    // The same membership again announces nothing.
    act(() => daemon.emit(stateFrame(daemon)));
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);
  });

  it('tells only the host', async () => {
    const daemon = installDaemon({
      snapshot: richSnapshot({ actor: bob, pending_joins: [] }),
    });
    renderCrew();
    await channelReady();

    daemon.state.snapshot = richSnapshot({ actor: bob, principals: [alice, bob, dave] });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() => expect(currentCrew().snapshot?.principals).toHaveLength(3));
    await act(async () => {});
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
  });
});
