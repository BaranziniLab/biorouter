import { act, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Channel, Team } from '../crewApi';
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
import { forgetOwnRenames, noteOwnRename } from './useChannelAddedToast';

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
        msg: layoutCopy.channelAdded('Alice Chen (@alice)', '#plots'),
      })
    );
    // M11: the adder by the one rule for inline text, as the joined toast names people.
    expect(layoutCopy.channelAdded('Alice Chen (@alice)', '#plots')).toBe(
      'Alice Chen (@alice) added you to #plots'
    );
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
        msg: layoutCopy.channelAdded('Alice Chen (@alice)', '#methods'),
      })
    );
  });

  // M11: a direct team add brought #general (and ticked channels) and was announced as "added you
  // to #general", which every team has.
  it('says once that they were added to a team, naming the team, not its #general', async () => {
    const daemon = installDaemon({ snapshot: richSnapshot({ actor: bob }) });
    renderCrew();
    await channelReady();
    const imaging: Team = {
      id: '7f6a5b4c-3d2e-4f1a-8b9c-0d1e2f3a4b5c',
      name: 'Imaging Core',
      created_by: general.owner_id,
      members: [general.owner_id, bob.id],
      general_channel_id: '7a1c2a3b-0000-4000-8000-0000000c0010',
    };
    const imagingGeneral = channel({
      id: imaging.general_channel_id,
      team_id: imaging.id,
      name: 'general',
      created_by: general.owner_id,
      owner_id: general.owner_id,
      members: [general.owner_id, bob.id],
    });
    const scans = channel({
      id: '7a1c2a3b-0000-4000-8000-0000000c0011',
      team_id: imaging.id,
      name: 'scans',
      created_by: general.owner_id,
      owner_id: general.owner_id,
      members: [general.owner_id, bob.id],
    });
    const base = richSnapshot({ actor: bob });
    daemon.state.snapshot = richSnapshot({
      actor: bob,
      teams: [...base.teams, imaging],
      channels: [general, methods, imagingGeneral, scans],
    });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: layoutCopy.teamAdded('Alice Chen (@alice)', 'Imaging Core'),
      })
    );
    expect(layoutCopy.teamAdded('Alice Chen (@alice)', 'Imaging Core')).toBe(
      'Alice Chen (@alice) added you to Imaging Core'
    );
    await act(async () => {});
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);
  });

  it('names the team of a channel whose name another of their teams also has', async () => {
    const imaging: Team = {
      id: '7f6a5b4c-3d2e-4f1a-8b9c-0d1e2f3a4b5c',
      name: 'Imaging Core',
      created_by: general.owner_id,
      members: [general.owner_id, bob.id],
      general_channel_id: '7a1c2a3b-0000-4000-8000-0000000c0010',
    };
    const imagingGeneral = channel({
      id: imaging.general_channel_id,
      team_id: imaging.id,
      name: 'general',
      created_by: general.owner_id,
      owner_id: general.owner_id,
      members: [general.owner_id, bob.id],
    });
    const base = richSnapshot({ actor: bob });
    const daemon = installDaemon({
      snapshot: richSnapshot({
        actor: bob,
        teams: [...base.teams, imaging],
        channels: [general, methods, imagingGeneral],
      }),
    });
    renderCrew();
    await channelReady();
    // Imaging Core gains a #methods, as Analysis Lab has, and Bob is in it.
    const imagingMethods = channel({
      id: '7a1c2a3b-0000-4000-8000-0000000c0012',
      team_id: imaging.id,
      name: 'methods',
      created_by: general.owner_id,
      owner_id: general.owner_id,
      members: [general.owner_id, bob.id],
    });
    daemon.state.snapshot = richSnapshot({
      actor: bob,
      teams: [...base.teams, imaging],
      channels: [general, methods, imagingGeneral, imagingMethods],
    });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: layoutCopy.channelAdded('Alice Chen (@alice)', 'Imaging Core / #methods'),
      })
    );
  });

  // M12: a rename overwrote the name and nobody was told; the old name then failed in the CLI.
  it('says once that a channel or team they are in was renamed, but not to whoever renamed it', async () => {
    forgetOwnRenames();
    const daemon = installDaemon({ snapshot: richSnapshot({ actor: bob }) });
    renderCrew();
    await channelReady();
    const base = richSnapshot({ actor: bob });
    daemon.state.snapshot = richSnapshot({
      actor: bob,
      teams: [{ ...base.teams[0], name: 'Analysis Group' }],
      channels: [{ ...general, name: 'lobby' }, methods],
    });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: layoutCopy.renamed('#general', '#lobby'),
      })
    );
    expect(toasts.toastSuccess).toHaveBeenCalledWith({
      msg: layoutCopy.renamed('Analysis Lab', 'Analysis Group'),
    });
    expect(layoutCopy.renamed('#general', '#lobby')).toBe('#general is now #lobby');
    // #methods, which Bob is not in, is never announced; each rename once.
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(2);
    act(() => daemon.emit(stateFrame(daemon)));
    await act(async () => {});
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(2);

    // Renamed on this computer: the person who did it is not told again.
    noteOwnRename(general.id);
    daemon.state.snapshot = richSnapshot({
      actor: bob,
      teams: [{ ...base.teams[0], name: 'Analysis Group' }],
      channels: [{ ...general, name: 'front-desk' }, methods],
    });
    act(() => daemon.emit(stateFrame(daemon)));
    await act(async () => {});
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(2);
  });

  /**
   * MSG2-N8: the views compared were one Crew screen's, so an add made while the person was on
   * Home was never said: the first view after they came back had nothing to compare with.
   */
  it('says it when they come back to Crew after being added while away', async () => {
    const daemon = installDaemon({ snapshot: richSnapshot({ actor: bob }) });
    const first = renderCrew();
    await channelReady();
    first.unmount();

    // On Home meanwhile: Alice adds Bob to #plots.
    const plots = channel({
      created_by: general.owner_id,
      owner_id: general.owner_id,
      members: [general.owner_id, bob.id],
    });
    daemon.state.snapshot = richSnapshot({ actor: bob, channels: [general, methods, plots] });
    renderCrew();
    await channelReady();
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: layoutCopy.channelAdded('Alice Chen (@alice)', '#plots'),
      })
    );
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);
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
