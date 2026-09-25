import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { sidebarCopy } from '../sidebar/copy';
import {
  alice,
  bob,
  channelReady,
  connection,
  currentCrew,
  ids,
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

  it('keeps a row in the host’s sidebar until the joiner is in one of their teams (Q3-52)', async () => {
    // The Let in dialog says "you can close this"; a host who did is brought back here, not by a
    // toast that is gone in seconds.
    const daemon = installDaemon();
    renderCrew();
    await channelReady();
    const joinedList = () =>
      screen.queryByRole('list', { name: sidebarCopy.section.joined }) as HTMLElement | null;
    expect(joinedList()).toBeNull();

    daemon.state.snapshot = richSnapshot({ principals: [alice, bob, dave], pending_joins: [] });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() => expect(joinedList()).not.toBeNull());
    const row = within(joinedList() as HTMLElement).getByRole('listitem');
    expect(row).toHaveTextContent('Dave Kim (@dave) · joined');
    // Said once, politely, and not as a second "joined": the toast already said that.
    await waitFor(() =>
      expect(document.querySelector('[data-crew-sidebar-announcer]')).toHaveTextContent(
        'Dave Kim (@dave) isn’t in any of your teams yet.'
      )
    );

    // The host's only team is one click away.
    fireEvent.click(
      within(row).getByRole('button', { name: 'Add Dave Kim (@dave) to Analysis Lab' })
    );
    await waitFor(() =>
      expect(currentCrew().ui.dialog).toEqual({
        kind: 'add-people',
        target: 'team',
        targetId: ids.team,
      })
    );
    act(() => currentCrew().closeDialog());

    // Once Dave is in the team, the row goes.
    const base = richSnapshot();
    daemon.state.snapshot = richSnapshot({
      principals: [alice, bob, dave],
      pending_joins: [],
      teams: base.teams.map((team) => ({ ...team, members: [...team.members, dave.id] })),
    });
    act(() => daemon.emit(stateFrame(daemon)));
    await waitFor(() => expect(joinedList()).toBeNull());
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
