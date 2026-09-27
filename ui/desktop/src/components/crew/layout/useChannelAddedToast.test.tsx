import { act, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Channel } from '../crewApi';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  bob,
  channelReady,
  connection,
  currentCrew,
  general,
  installDaemon,
  methods,
  richSnapshot,
  renderCrew,
  type ScriptedDaemon,
} from '../integration/harness';
import { layoutCopy } from './copy';

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

const channel = (overrides: Partial<Channel>): Channel => ({
  ...methods,
  id: '7a1c2a3b-0000-4000-8000-0000000c0009',
  name: 'plots',
  ...overrides,
});

/** Q2-63: someone adds you to a channel while you are elsewhere, and nothing said so. */
describe('the person hears when someone adds them to a channel (Q2-63)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('says once who added them, and nothing for the channels already there', async () => {
    const daemon = installDaemon({ snapshot: richSnapshot({ actor: bob }) });
    renderCrew();
    await channelReady();
    expect(toasts.toastSuccess).not.toHaveBeenCalled();

    const plots = channel({
      created_by: general.owner_id,
      owner_id: general.owner_id,
      members: [general.owner_id, bob.id],
    });
    daemon.state.snapshot = richSnapshot({ actor: bob, channels: [general, methods, plots] });
    act(() => daemon.emit(stateFrame(daemon)));

    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: layoutCopy.channelAdded('@alice', '#plots'),
      })
    );
    expect(layoutCopy.channelAdded('@alice', '#plots')).toBe('@alice added you to #plots');
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);

    // The same channels again announce nothing.
    act(() => daemon.emit(stateFrame(daemon)));
    await act(async () => {});
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);
  });

  it('says it when someone adds them to a channel that already existed', async () => {
    const daemon = installDaemon({ snapshot: richSnapshot({ actor: bob }) });
    renderCrew();
    await channelReady();
    // #methods was listed with Bob not in it; now he is.
    daemon.state.snapshot = richSnapshot({
      actor: bob,
      channels: [general, { ...methods, members: [...methods.members, bob.id] }],
    });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: layoutCopy.channelAdded('@alice', '#methods'),
      })
    );
  });

  it('says nothing for a channel they made themselves', async () => {
    const daemon = installDaemon();
    renderCrew();
    await channelReady();
    const mine = channel({
      created_by: currentCrew().snapshot!.actor.id,
      owner_id: currentCrew().snapshot!.actor.id,
      members: [currentCrew().snapshot!.actor.id],
    });
    daemon.state.snapshot = richSnapshot({ channels: [general, methods, mine] });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() => expect(currentCrew().snapshot?.channels).toHaveLength(3));
    await act(async () => {});
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
  });
});
