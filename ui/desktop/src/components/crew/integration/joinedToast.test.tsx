import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { sidebarCopy } from '../sidebar/copy';
import { forgetJoinerNames } from '../sidebar/sidebarView';
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

/**
 * A joiner who chose no name yet: a new principal's nickname is its username, so the snapshot
 * names him `@crew_jack` alone (naming D2).
 */
const jack = {
  id: '6f5e4d3c-2b1a-4098-8e7d-6c5b4a3f2e1d',
  uid: 70305,
  username: 'crew_jack',
  nickname: 'crew_jack',
  active: true,
};

describe('the host hears when someone joins (a result that happens off-screen)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetJoinerNames();
  });
  afterEach(() => forgetJoinerNames());

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

  it('names a joiner who chose no name by the server-account name he waited under (Q4-42)', async () => {
    // Inside Let in he is "Jack"; the toast and the sidebar row about the same event said
    // "@crew_jack". His waiting row — the one place that name comes from — is gone from the very
    // snapshot that shows him joined, so the name is kept from the views before it.
    const daemon = installDaemon({
      snapshot: richSnapshot({
        pending_joins: [{ username: 'crew_jack', full_name: 'Jack Moreno' }],
      }),
    });
    renderCrew();
    await channelReady();

    daemon.state.snapshot = richSnapshot({ principals: [alice, bob, jack], pending_joins: [] });
    act(() => daemon.emit(stateFrame(daemon)));

    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Jack Moreno (@crew_jack) joined lab',
      })
    );
    expect(toasts.toastSuccess).toHaveBeenCalledTimes(1);
    const joined = await screen.findByRole('list', { name: sidebarCopy.section.joined });
    expect(within(joined).getByRole('listitem')).toHaveTextContent(
      'Jack Moreno (@crew_jack) · joined'
    );
  });

  it('says @username when no verified view ever named the joiner’s server account', async () => {
    const daemon = installDaemon({ snapshot: richSnapshot({ pending_joins: [] }) });
    renderCrew();
    await channelReady();

    daemon.state.snapshot = richSnapshot({ principals: [alice, bob, jack], pending_joins: [] });
    act(() => daemon.emit(stateFrame(daemon)));

    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({ msg: '@crew_jack joined lab' })
    );
    const joined = await screen.findByRole('list', { name: sidebarCopy.section.joined });
    expect(within(joined).getByRole('listitem')).toHaveTextContent('@crew_jack · joined');
    expect(joined).not.toHaveTextContent('Jack Moreno');
  });

  it('keeps the name the joiner chose over the one on their server account', async () => {
    const daemon = installDaemon({
      snapshot: richSnapshot({
        pending_joins: [{ username: 'dave', full_name: 'David Kim' }],
      }),
    });
    renderCrew();
    await channelReady();

    daemon.state.snapshot = richSnapshot({ principals: [alice, bob, dave], pending_joins: [] });
    act(() => daemon.emit(stateFrame(daemon)));

    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({ msg: 'Dave Kim (@dave) joined lab' })
    );
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
