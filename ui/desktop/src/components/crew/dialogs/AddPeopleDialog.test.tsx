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

/** Each row's person, as rendered (the avatar is decorative and aria-hidden). */
const optionNames = (listbox: HTMLElement) =>
  within(listbox)
    .queryAllByRole('option')
    .map((option) => option.querySelector('[data-person-context]')?.textContent);

/** The checklist of people who can be added, and each row's person as rendered. */
const checklist = () => screen.findByRole('list', { name: addPeopleCopy.people });
const rowNames = (list: HTMLElement) =>
  within(list)
    .queryAllByRole('listitem')
    .map((row) => row.querySelector('[data-person-context]')?.textContent);
/** Tick a person by name, as a person would: their row's checkbox. */
async function tick(name: RegExp) {
  fireEvent.click(within(await checklist()).getByRole('checkbox', { name }));
}
async function add(name: string | RegExp = /^Add/) {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name }));
  });
}

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
  options: {
    snapshot?: Snapshot;
    answer?: unknown;
    request?: (method: string, params: Record<string, unknown>) => unknown;
  } = {}
) {
  const onClose = vi.fn();
  const view = renderWithCrew(
    <WithDirectAdd>
      <AddPeopleDialog {...props} onClose={onClose} />
    </WithDirectAdd>,
    {
      snapshot: options.snapshot,
      request:
        options.request ??
        ((method) =>
          method === 'team.add_member' || method === 'channel.add_member' ? options.answer : {}),
    }
  );
  return { ...view, onClose };
}

const eve = { id: 'person-eve', uid: 1004, username: 'eve', nickname: 'Eve Park' };

describe('AddPeopleDialog and its checklist', () => {
  it('offers a team only people who are not members and not already invited', async () => {
    const snapshot = makeSnapshot({
      // Bob and Carol are members; Dan has a live invitation; Eve has an expired one.
      principals: [...makeSnapshot().principals, eve],
      invitations: [
        invitation('team', 'team-1', dan.id),
        { ...invitation('team', 'team-1', eve.id), expires_at: Date.now() / 1000 - 10 },
      ],
    });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    expect(
      await screen.findByRole('dialog', { name: 'Add people to Analysis Lab' })
    ).toBeInTheDocument();
    expect(rowNames(await checklist())).toEqual(['Eve Park (@eve)']);
  });

  it('offers a channel only members of its team who are not in the channel', async () => {
    renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />
    );
    expect(
      await screen.findByRole('dialog', { name: 'Add people to #general' })
    ).toBeInTheDocument();
    // Bob is in #general; Dan is not in the team at all.
    expect(rowNames(await checklist())).toEqual(['Carol Diaz (@carol)']);
  });

  it('shows every row with its @username and searches display names and usernames', async () => {
    const snapshot = makeSnapshot({
      teams: [{ ...makeSnapshot().teams[0], members: [alice.id] }],
    });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    const list = await checklist();
    expect(rowNames(list)).toEqual(['Bob Lee (@bob)', 'Carol Diaz (@carol)', 'Dan Wu (@dan)']);
    const search = screen.getByRole('searchbox', { name: addPeopleCopy.search });
    fireEvent.change(search, { target: { value: 'diaz' } });
    expect(rowNames(list)).toEqual(['Carol Diaz (@carol)']);
    fireEvent.change(search, { target: { value: '@da' } });
    expect(rowNames(list)).toEqual(['Dan Wu (@dan)']);
    fireEvent.change(search, { target: { value: 'zed' } });
    expect(screen.getByText(addPeopleCopy.noMatch('zed'))).toBeInTheDocument();
  });

  it('invites the ticked person with their expected username, says they still have to accept, and stays open', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={onClose} />
    );
    const submit = await screen.findByRole('button', { name: 'Add' });
    expect(submit).toBeDisabled();
    await tick(/Carol Diaz/);
    expect(submit).toBeEnabled();
    await add('Add');
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
    expect(
      await screen.findByText('Invited @carol. They’ll see it in Crew and need to accept.')
    ).toBeInTheDocument();
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    // No one is left to add: one Done, no disabled Add, and it takes the focus the Add had.
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).queryByRole('button', { name: /^Add/ })).toBeNull();
    const done = within(dialog).getByRole('button', { name: 'Done' });
    await waitFor(() => expect(done).toHaveFocus());
    fireEvent.click(done);
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

  it('lists a channel’s waiting invitees beside the checklist', async () => {
    const snapshot = makeSnapshot({
      teams: [{ ...makeSnapshot().teams[0], members: [alice.id, bob.id, carol.id, dan.id] }],
      invitations: [invitation('channel', 'channel-general', dan.id)],
    });
    renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={vi.fn()} />,
      { snapshot }
    );
    expect(await screen.findByText(addPeopleCopy.waiting('@dan'))).toBeInTheDocument();
    expect(rowNames(await checklist())).toEqual(['Carol Diaz (@carol)']);
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
});

describe('AddPeopleDialog with no one left to add (QA Q2-22)', () => {
  it('lists who is in already, host and you first, names the workspace’s invitees, and shows one Done', async () => {
    const zed = { id: 'person-zed', uid: 1009, username: 'zed', nickname: 'Aaron Zed' };
    const snapshot = makeSnapshot({
      actor: bob,
      principals: [alice, bob, carol, zed],
      channels: [
        {
          ...makeSnapshot().channels[0],
          owner_id: bob.id,
          members: [carol.id, zed.id, bob.id, alice.id],
        },
      ],
      teams: [{ ...makeSnapshot().teams[0], members: [alice.id, bob.id, carol.id, zed.id] }],
      pending_joins: [
        { username: 'crew_frank' },
        { username: 'old', expired: true },
        { username: 'carol', add_device: true },
      ],
    });
    const onClose = vi.fn();
    renderWithCrew(
      <AddPeopleDialog target="channel" targetId="channel-general" onClose={onClose} />,
      { snapshot }
    );
    const dialog = await screen.findByRole('dialog', { name: 'Add people to #general' });
    expect(within(dialog).getByText(addPeopleCopy.allInTeam('Analysis Lab'))).toBeInTheDocument();
    // The message is the dialog's description (QA Q2-28).
    expect(dialog).toHaveAccessibleDescription(addPeopleCopy.allInTeam('Analysis Lab'));
    const members = within(dialog).getByRole('region', {
      name: addPeopleCopy.alreadyInPlace('#general'),
    });
    expect(
      within(members)
        .getAllByRole('listitem')
        .map((row) => row.querySelector('[data-person-context]')?.textContent)
    ).toEqual([
      expect.stringContaining('Alice Chen'),
      expect.stringContaining('Bob Lee'),
      expect.stringContaining('Aaron Zed'),
      expect.stringContaining('Carol Diaz'),
    ]);
    expect(
      within(dialog).getByText('Invited to lab, not joined yet: @crew_frank.')
    ).toBeInTheDocument();
    // One Done, and no Add to press, disabled or otherwise.
    expect(within(dialog).queryByRole('button', { name: /^Add/ })).toBeNull();
    expect(within(dialog).queryByRole('button', { name: 'Cancel' })).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Done' }));
    expect(onClose).toHaveBeenCalled();
  });

  it('tells someone who may not add people here who may, with one Done', async () => {
    // Bob is neither Analysis Lab's owner nor the host.
    const snapshot = makeSnapshot({ actor: bob, principals: [alice, bob, carol, dan] });
    renderDirect({ target: 'team', targetId: 'team-1' }, { snapshot });
    const dialog = await screen.findByRole('dialog', { name: 'Members of Analysis Lab' });
    const text = 'Only @alice or the host can add people to Analysis Lab.';
    expect(within(dialog).getByText(text)).toBeInTheDocument();
    expect(dialog).toHaveAccessibleDescription(text);
    expect(within(dialog).queryByRole('list', { name: addPeopleCopy.people })).toBeNull();
    expect(within(dialog).queryByRole('button', { name: /^Add/ })).toBeNull();
    expect(within(dialog).getByRole('button', { name: 'Done' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Invite people to/ })).toBeNull();
  });

  it('shows someone who may not add people to a team its members, then who may (QA Q3-44)', async () => {
    // Bob is neither Analysis Lab's owner nor the host: the team menu's "Members of…" opens this.
    const snapshot = makeSnapshot({ actor: bob, principals: [alice, bob, carol, dan] });
    renderDirect({ target: 'team', targetId: 'team-1' }, { snapshot });
    const dialog = await screen.findByRole('dialog', {
      name: addPeopleCopy.membersOf('Analysis Lab'),
    });
    const list = within(dialog).getByRole('list', {
      name: addPeopleCopy.membersOf('Analysis Lab'),
    });
    // Host, you, then by name.
    expect(
      within(list)
        .getAllByRole('listitem')
        .map((row) => row.querySelector('[data-person-context]')?.textContent)
    ).toEqual([
      expect.stringContaining('Alice Chen'),
      expect.stringContaining('Bob Lee'),
      expect.stringContaining('Carol Diaz'),
    ]);
    expect(within(list).getAllByRole('listitem')[1]).toHaveTextContent('you');
    // The list first, then the muted line saying who may add people, and no Note box.
    const who = within(dialog).getByText('Only @alice or the host can add people to Analysis Lab.');
    expect(who).toHaveClass('text-text-muted');
    expect(list.compareDocumentPosition(who) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(dialog).queryByRole('heading', { name: /^Already in/ })).toBeNull();
    // A single Done, and nothing to add with.
    expect(
      within(dialog)
        .getAllByRole('button')
        .map((button) => button.textContent)
    ).toEqual(['Done', expect.anything()]);
    expect(within(dialog).queryByRole('checkbox')).toBeNull();
  });

  it('keeps Add people for the team’s owner and for the host', async () => {
    // Alice is the host and Analysis Lab's owner.
    renderDirect({ target: 'team', targetId: 'team-1' });
    expect(
      await screen.findByRole('dialog', { name: addPeopleCopy.titleTeam('Analysis Lab') })
    ).toBeInTheDocument();
    // The host who does not own the team adds people too, under a broker that adds directly.
    const hosted = makeSnapshot({
      teams: [{ ...makeSnapshot().teams[0], created_by: carol.id }],
    });
    renderDirect({ target: 'team', targetId: 'team-1' }, { snapshot: hosted });
    expect(
      await screen.findAllByRole('dialog', { name: addPeopleCopy.titleTeam('Analysis Lab') })
    ).toHaveLength(2);
  });

  it('says only the owner invites under an older broker', async () => {
    const snapshot = makeSnapshot({ actor: bob, principals: [alice, bob] });
    renderWithCrew(<AddPeopleDialog target="team" targetId="team-1" onClose={vi.fn()} />, {
      snapshot,
    });
    expect(
      await screen.findByText('Only @alice can add people to Analysis Lab.')
    ).toBeInTheDocument();
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
    expect(rowNames(await checklist())).toEqual(['Dan Wu (@dan)']);
    await tick(/Dan Wu/);

    const channels = screen.getByRole('group', { name: addPeopleCopy.channels });
    const general = within(channels).getByRole('checkbox', { name: /#general/ });
    expect(general).toBeChecked();
    expect(general).toBeDisabled();
    expect(within(channels).getByRole('checkbox', { name: /#methods/ })).toBeChecked();

    await add('Add');
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
    expect(
      await screen.findByText('Added @dan to Analysis Lab. They can now see #general and #methods.')
    ).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('leaves out a channel the host unchecks', async () => {
    const snapshot = makeSnapshot({ channels: [...makeSnapshot().channels, methods] });
    const { crew } = renderDirect(
      { target: 'team', targetId: 'team-1' },
      { snapshot, answer: { added_channels: [], already_member: false } }
    );
    await tick(/Dan Wu/);
    fireEvent.click(screen.getByRole('checkbox', { name: /#methods/ }));
    await add('Add');
    await waitFor(() =>
      expect(requestsFor(crew, 'team.add_member')).toEqual([
        { team_id: 'team-1', principal_id: dan.id, expected_username: 'dan' },
      ])
    );
    expect(
      await screen.findByText('Added @dan to Analysis Lab. They can now see #general.')
    ).toBeInTheDocument();
  });

  it('adds a team member straight into a channel', async () => {
    const { crew } = renderDirect(
      { target: 'channel', targetId: 'channel-general' },
      { answer: { already_member: false } }
    );
    await tick(/Carol Diaz/);
    expect(screen.queryByRole('group', { name: addPeopleCopy.channels })).toBeNull();
    await add('Add');
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.add_member')).toEqual([
        { channel_id: 'channel-general', principal_id: carol.id, expected_username: 'carol' },
      ])
    );
    expect(await screen.findByText('Added @carol to #general.')).toBeInTheDocument();
  });

  it('shows a refusal in words, in the dialog, and adds no one', async () => {
    renderDirect(
      { target: 'channel', targetId: 'channel-general' },
      {
        request: (method) => {
          if (method === 'channel.add_member')
            // The broker's literal refusal (`mutate_channel_add_member`).
            throw new Error(
              "forbidden: Only the channel's owner or the workspace host can add people to it."
            );
          return {};
        },
      }
    );
    await tick(/Carol Diaz/);
    await add('Add');
    // Its sentence, without the code: written for a person, shown to one.
    expect(
      await screen.findByText(
        "Couldn’t add @carol: Only the channel's owner or the workspace host can add people to it."
      )
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('forbidden:');
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
    // Carol can still be picked again.
    expect(rowNames(await checklist())).toEqual(['Carol Diaz (@carol)']);
  });
});

describe('AddPeopleDialog, several people at once (QA Q2-05)', () => {
  /** Four people in the team and none in #methods but Alice. */
  const four = () =>
    makeSnapshot({
      principals: [...makeSnapshot().principals, eve],
      teams: [
        { ...makeSnapshot().teams[0], members: [alice.id, bob.id, carol.id, dan.id, eve.id] },
      ],
      channels: [...makeSnapshot().channels, methods],
    });

  it('adds everyone Select all ticks, one request each, with one summary, and stays open', async () => {
    const { crew, onClose } = renderDirect(
      { target: 'channel', targetId: 'channel-methods' },
      { snapshot: four(), answer: { already_member: false } }
    );
    const list = await checklist();
    expect(rowNames(list)).toHaveLength(4);
    fireEvent.click(screen.getByRole('checkbox', { name: addPeopleCopy.selectAll(4) }));
    await add(addPeopleCopy.addMany(4));
    await waitFor(() => expect(requestsFor(crew, 'channel.add_member')).toHaveLength(4));
    expect(requestsFor(crew, 'channel.add_member')).toEqual(
      [bob, carol, dan, eve].map((person) => ({
        channel_id: 'channel-methods',
        principal_id: person.id,
        expected_username: person.username,
      }))
    );
    const summary = await screen.findByText('Added @bob, @carol, @dan and @eve to #methods.');
    expect(summary.closest('[role="status"]')).not.toBeNull();
    expect(screen.getAllByText(/^Added /)).toHaveLength(1);
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog', { name: 'Add people to #methods' })).toBeInTheDocument();
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
  });

  it('keeps adding after one refusal, and says who could not be added and why', async () => {
    const { crew } = renderDirect(
      { target: 'channel', targetId: 'channel-methods' },
      {
        snapshot: four(),
        request: (method, params) => {
          if (method === 'channel.add_member' && params.principal_id === carol.id)
            throw new Error('forbidden: principal unavailable');
          return method === 'channel.add_member' ? { already_member: false } : {};
        },
      }
    );
    await checklist();
    fireEvent.click(screen.getByRole('checkbox', { name: addPeopleCopy.selectAll(4) }));
    await add(addPeopleCopy.addMany(4));
    await waitFor(() => expect(requestsFor(crew, 'channel.add_member')).toHaveLength(4));
    const summary = await screen.findByText(/^Added @bob, @dan and @eve to #methods\. /);
    expect(summary).toHaveTextContent(/Couldn’t add @carol: /);
    // The one refused stays on the list, ticked, to try again; the others have gone.
    expect(rowNames(await checklist())).toEqual(['Carol Diaz (@carol)']);
    expect(screen.getByRole('checkbox', { name: /Carol Diaz/ })).toBeChecked();
    expect(screen.getByRole('button', { name: 'Add' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument();
  });

  it('still adds one person, as ever, when only one is ticked, and carries on from the search', async () => {
    const { crew } = renderDirect(
      { target: 'channel', targetId: 'channel-methods' },
      { snapshot: four(), answer: { already_member: false } }
    );
    await tick(/Dan Wu/);
    const submit = screen.getByRole('button', { name: 'Add' });
    expect(submit).toBeEnabled();
    submit.focus();
    await add('Add');
    await waitFor(() =>
      expect(requestsFor(crew, 'channel.add_member')).toEqual([
        { channel_id: 'channel-methods', principal_id: dan.id, expected_username: 'dan' },
      ])
    );
    expect(await screen.findByText('Added @dan to #methods.')).toBeInTheDocument();
    // The Add that had focus is disabled now that no one is ticked: focus moves to the search.
    expect(submit).toBeDisabled();
    await waitFor(() =>
      expect(screen.getByRole('searchbox', { name: addPeopleCopy.search })).toHaveFocus()
    );
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
