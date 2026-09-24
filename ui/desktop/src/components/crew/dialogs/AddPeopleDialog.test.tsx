import type * as React from 'react';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Invitation, Snapshot } from '../crewApi';
import { CrewControllerProvider, useCrew } from '../state/CrewControllerContext';
import { AddPeopleDialog, type AddPeopleDialogProps } from './AddPeopleDialog';
import { addPeopleCopy } from './copy';
import {
  alice,
  bob,
  carol,
  dan,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { DIRECT_ADD_CAPABILITY } from './people';
import { TransferOwnershipDialog } from './TransferOwnershipDialog';

const toasts = vi.hoisted(() => ({ toastSuccess: vi.fn() }));
vi.mock('../../../toasts', () => toasts);

installResizeObserverStub();

async function openPicker(name: RegExp = /^Person/) {
  fireEvent.click(await screen.findByRole('button', { name }));
  return screen.findByRole('listbox');
}

/** Each row's person, as rendered (the avatar is decorative and aria-hidden). */
const optionNames = (listbox: HTMLElement) =>
  within(listbox)
    .queryAllByRole('option')
    .map((option) => option.querySelector('[data-person-context]')?.textContent);

afterEach(() => vi.clearAllMocks());

const HOUR = 3600;
function invitation(kind: 'team' | 'channel', target: string, principal: string): Invitation {
  return {
    id: `inv-${kind}-${target}-${principal}`,
    kind,
    target_id: target,
    principal_id: principal,
    inviter_id: alice.id,
    expires_at: Date.now() / 1000 + HOUR,
  };
}

const methods = {
  id: 'channel-methods',
  team_id: 'team-1',
  name: 'methods',
  created_by: alice.id,
  owner_id: alice.id,
  members: [alice.id],
  archived: false,
  classification: 'restricted' as const,
};

/**
 * The harness's controller as the broker's hello would extend it. It reads the controller from
 * context, so errors and pending keys stay live.
 */
function WithDirectAdd({ children }: { children: React.ReactNode }) {
  const crew = useCrew();
  return (
    <CrewControllerProvider
      controller={{ ...crew, capabilities: ['unique_names_v1', DIRECT_ADD_CAPABILITY] }}
    >
      {children}
    </CrewControllerProvider>
  );
}

/** The dialog under a broker that says it adds members directly (`direct_add_v1`). */
function renderDirect(
  props: Omit<AddPeopleDialogProps, 'onClose'>,
  options: { snapshot?: Snapshot; answer?: unknown } = {}
) {
  const onClose = vi.fn();
  const view = renderWithCrew(
    <WithDirectAdd>
      <AddPeopleDialog {...props} onClose={onClose} />
    </WithDirectAdd>,
    {
      snapshot: options.snapshot,
      request: (method) =>
        method === 'team.add_member' || method === 'channel.add_member' ? options.answer : {},
    }
  );
  return { ...view, onClose };
}

describe('AddPeopleDialog and PersonPicker', () => {
  it('offers a team only people who are not members and not already invited', async () => {
    const snapshot = makeSnapshot({
      // Bob and Carol are members; Dan has a live invitation; Eve has an expired one.
      principals: [
        ...makeSnapshot().principals,
        { id: 'person-eve', uid: 1004, username: 'eve', nickname: 'Eve Park' },
      ],
      invitations: [
        {
          id: 'inv-1',
          kind: 'team',
          target_id: 'team-1',
          principal_id: dan.id,
          inviter_id: alice.id,
          expires_at: Date.now() / 1000 + 3600,
        },
        {
          id: 'inv-2',
          kind: 'team',
          target_id: 'team-1',
          principal_id: 'person-eve',
          inviter_id: alice.id,
          expires_at: Date.now() / 1000 - 10,
        },
      ],
    });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    expect(
      await screen.findByRole('dialog', { name: 'Add people to Analysis Lab' })
    ).toBeInTheDocument();
    const listbox = await openPicker();
    expect(optionNames(listbox)).toEqual(['Eve Park (@eve)']);
  });

  it('offers a channel only members of its team who are not in the channel', async () => {
    renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />
    );
    expect(
      await screen.findByRole('dialog', { name: 'Add people to #general' })
    ).toBeInTheDocument();
    const listbox = await openPicker();
    // Bob is in #general; Dan is not in the team at all.
    expect(optionNames(listbox)).toEqual(['Carol Diaz (@carol)']);
  });

  it('shows every row with its @username and searches display names and usernames', async () => {
    const snapshot = makeSnapshot({
      teams: [{ ...makeSnapshot().teams[0], members: [alice.id] }],
    });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    const listbox = await openPicker();
    expect(optionNames(listbox)).toEqual([
      'Bob Lee (@bob)',
      'Carol Diaz (@carol)',
      'Dan Wu (@dan)',
    ]);
    const search = screen.getByRole('combobox', { name: addPeopleCopy.search });
    fireEvent.change(search, { target: { value: 'diaz' } });
    expect(optionNames(listbox)).toEqual(['Carol Diaz (@carol)']);
    fireEvent.change(search, { target: { value: '@da' } });
    expect(optionNames(listbox)).toEqual(['Dan Wu (@dan)']);
    fireEvent.change(search, { target: { value: 'zed' } });
    expect(within(listbox).getByText(addPeopleCopy.noMatch('zed'))).toBeInTheDocument();
  });

  it('sends the chosen person with their expected username, then says they still have to accept', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={onClose} />
    );
    const add = await screen.findByRole('button', { name: 'Add' });
    expect(add).toBeDisabled();
    const listbox = await openPicker();
    fireEvent.click(within(listbox).getByRole('option', { name: /Carol Diaz/ }));
    expect(screen.getByRole('button', { name: /^Person/ })).toHaveTextContent(
      'Carol Diaz (@carol)'
    );
    await act(async () => {
      fireEvent.click(add);
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'invitation.create')).toEqual([
        {
          kind: 'channel',
          target_id: 'channel-general',
          principal_id: carol.id,
          expected_username: 'carol',
        },
      ])
    );
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Invited. Carol Diaz (@carol) will see it in Crew and needs to accept.',
      })
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('names who is invited to a team and has not accepted, instead of hiding them', async () => {
    // Bob and Carol are members; Dan was invited and has not accepted.
    const snapshot = makeSnapshot({ invitations: [invitation('team', 'team-1', dan.id)] });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    expect(
      await screen.findByText(addPeopleCopy.allInvited('lab', 'Analysis Lab', '@dan'))
    ).toBeInTheDocument();
    // The host still has somewhere to go.
    fireEvent.click(screen.getByRole('button', { name: addPeopleCopy.inviteToWorkspace('lab') }));
  });

  it('lists a channel’s waiting invitees beside the picker', async () => {
    const snapshot = makeSnapshot({
      teams: [{ ...makeSnapshot().teams[0], members: [alice.id, bob.id, carol.id, dan.id] }],
      invitations: [invitation('channel', 'channel-general', dan.id)],
    });
    renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />,
      { snapshot }
    );
    expect(await screen.findByText(addPeopleCopy.waiting('@dan'))).toBeInTheDocument();
    const listbox = await openPicker();
    expect(optionNames(listbox)).toEqual(['Carol Diaz (@carol)']);
  });

  it('says a channel is empty because its team is, names the team’s waiting invitees, and offers the team', async () => {
    // P0-2: the team invitations were never accepted, so no one else is in the team — never
    // "No one else has joined lab", which was false.
    const snapshot = makeSnapshot({
      teams: [{ ...makeSnapshot().teams[0], members: [alice.id] }],
      channels: [{ ...makeSnapshot().channels[0], members: [alice.id] }],
      invitations: [invitation('team', 'team-1', bob.id), invitation('team', 'team-1', carol.id)],
    });
    const { crew } = renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />,
      { snapshot }
    );
    expect(
      await screen.findByText(
        `${addPeopleCopy.noOneInTeam('Analysis Lab')} ${addPeopleCopy.waitingToAccept('@bob and @carol')}`
      )
    ).toBeInTheDocument();
    expect(screen.queryByText(addPeopleCopy.noOne('lab'))).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: addPeopleCopy.addToTeam('Analysis Lab') }));
    expect(crew.current().ui.dialog).toEqual({
      kind: 'add-people',
      target: 'team',
      targetId: 'team-1',
    });
  });

  it('says so when everyone is already here', async () => {
    const snapshot = makeSnapshot({
      channels: [{ ...makeSnapshot().channels[0], members: [alice.id, bob.id, carol.id] }],
    });
    renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />,
      { snapshot }
    );
    expect(await screen.findByText(addPeopleCopy.allInTeam('Analysis Lab'))).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Person/ })).toBeNull();
  });

  it('says so when no one else has joined, and offers the host the way to invite them', async () => {
    const snapshot = makeSnapshot({ principals: [alice] });
    const { crew } = renderWithCrew(
      <AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />,
      { snapshot }
    );
    expect(await screen.findByText(addPeopleCopy.noOne('lab'))).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Invite people to lab…' }));
    expect(crew.current().ui.dialog).toEqual({ kind: 'invite-people' });
  });

  it('offers a member who is not the host no invite button', async () => {
    const snapshot = makeSnapshot({ actor: bob, principals: [alice, bob] });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    expect(await screen.findByText(addPeopleCopy.allInWorkspace('lab'))).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Invite people to/ })).toBeNull();
  });
});

describe('AddPeopleDialog, adding directly (direct_add_v1)', () => {
  it('adds someone to the team with the channels chosen, and says what they can now see', async () => {
    // Dan's old invitation is no reason to keep him out: he can be added now.
    const snapshot = makeSnapshot({
      channels: [...makeSnapshot().channels, methods],
      invitations: [invitation('team', 'team-1', dan.id)],
    });
    const { crew, onClose } = renderDirect(
      { target: 'team', targetId: 'team-1' },
      {
        snapshot,
        answer: {
          team_id: 'team-1',
          principal_id: dan.id,
          added_channels: ['channel-methods'],
          already_member: false,
        },
      }
    );
    const listbox = await openPicker();
    expect(optionNames(listbox)).toEqual(['Dan Wu (@dan)']);
    fireEvent.click(within(listbox).getByRole('option', { name: /Dan Wu/ }));

    const channels = screen.getByRole('group', { name: addPeopleCopy.channels });
    const general = within(channels).getByRole('checkbox', { name: /#general/ });
    expect(general).toBeChecked();
    expect(general).toBeDisabled();
    expect(within(channels).getByRole('checkbox', { name: /#methods/ })).toBeChecked();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'team.add_member')).toEqual([
        {
          team_id: 'team-1',
          principal_id: dan.id,
          expected_username: 'dan',
          channel_ids: ['channel-methods'],
        },
      ])
    );
    expect(requestsFor(crew, 'invitation.create')).toEqual([]);
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Added. Dan Wu (@dan) can now see #general and #methods.',
      })
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('leaves out a channel the host unchecks', async () => {
    const snapshot = makeSnapshot({ channels: [...makeSnapshot().channels, methods] });
    const { crew } = renderDirect(
      { target: 'team', targetId: 'team-1' },
      { snapshot, answer: { added_channels: [], already_member: false } }
    );
    fireEvent.click(within(await openPicker()).getByRole('option', { name: /Dan Wu/ }));
    fireEvent.click(screen.getByRole('checkbox', { name: /#methods/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'team.add_member')).toEqual([
        { team_id: 'team-1', principal_id: dan.id, expected_username: 'dan' },
      ])
    );
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Added. Dan Wu (@dan) can now see #general.',
      })
    );
  });

  it('adds a team member straight into a channel', async () => {
    const { crew } = renderDirect(
      { target: 'channel', targetId: 'channel-general' },
      { answer: { already_member: false } }
    );
    fireEvent.click(within(await openPicker()).getByRole('option', { name: /Carol Diaz/ }));
    expect(screen.queryByRole('group', { name: addPeopleCopy.channels })).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.add_member')).toEqual([
        { channel_id: 'channel-general', principal_id: carol.id, expected_username: 'carol' },
      ])
    );
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Added. Carol Diaz (@carol) can now see #general.',
      })
    );
  });

  it('shows a refusal in the dialog and adds no one', async () => {
    renderWithCrew(
      <WithDirectAdd>
        <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />
      </WithDirectAdd>,
      {
        request: (method) => {
          if (method === 'channel.add_member')
            throw new Error('forbidden: channel owner or host required');
          return {};
        },
      }
    );
    fireEvent.click(within(await openPicker()).getByRole('option', { name: /Carol Diaz/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'forbidden: channel owner or host required'
    );
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
  });
});

describe('TransferOwnershipDialog', () => {
  it('offers the channel members, preselects a successor and sends the expected username', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(
      <TransferOwnershipDialog channelId="channel-general" successorId={bob.id} onClose={onClose} />
    );
    expect(
      await screen.findByRole('dialog', { name: 'Transfer ownership of #general' })
    ).toBeInTheDocument();
    expect(screen.getByText('They’ll need to accept.')).toBeInTheDocument();
    const picker = screen.getByRole('button', { name: /^New owner/ });
    expect(picker).toHaveTextContent('Bob Lee (@bob)');
    fireEvent.click(picker);
    expect(optionNames(await screen.findByRole('listbox'))).toEqual(['Bob Lee (@bob)']);
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Offer ownership' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.transfer')).toEqual([
        { channel_id: 'channel-general', successor_id: bob.id, expected_username: 'bob' },
      ])
    );
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({
        msg: 'Ownership offered to Bob Lee (@bob)',
      })
    );
  });
});
