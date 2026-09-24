import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddPeopleDialog } from './AddPeopleDialog';
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

  it('sends the chosen person with their expected username, then says so off-screen', async () => {
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
        msg: addPeopleCopy.sent('Carol Diaz (@carol)'),
      })
    );
    expect(onClose).toHaveBeenCalled();
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

  it('says so when no one else has joined', async () => {
    const snapshot = makeSnapshot({ principals: [alice] });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    expect(await screen.findByText(addPeopleCopy.noOne('lab'))).toBeInTheDocument();
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
