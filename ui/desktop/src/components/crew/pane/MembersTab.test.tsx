import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  alice,
  bob,
  carol,
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  methods,
  renderCrew,
} from '../channel/crewTestHarness';
import { membersCopy } from './copy';
import { MembersTab } from './MembersTab';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
const ghost = '6f1c2a3b-0000-4000-8000-000000000000';

function Members() {
  return (
    <div data-testid="members">
      <MembersTab />
    </div>
  );
}

async function shown() {
  await waitFor(() => expect(currentCrew().channel?.id).toBe(general.id));
  return screen.getByTestId('members');
}

/** Each row's text as a reader gets it: decorative avatars (aria-hidden) left out. */
function rows(tab: HTMLElement) {
  return within(within(tab).getByRole('list', { name: membersCopy.listLabel('#general') }))
    .getAllByRole('listitem')
    .map((item) => {
      const copy = item.cloneNode(true) as HTMLElement;
      copy.querySelectorAll('[aria-hidden="true"]').forEach((node) => node.remove());
      return copy.textContent;
    });
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
});

describe('MembersTab', () => {
  it('lists the channel’s members — not the team’s people — owner first, with markers', async () => {
    renderCrew(Members);
    const tab = await shown();
    expect(within(tab).getByText(membersCopy.count(2))).toBeInTheDocument();
    expect(rows(tab)).toEqual(['Alice Chen (@alice) · youOwner', 'Bob Lee (@bob)']);
    expect(tab.textContent).not.toContain('Carol');
    expect(tab.textContent).not.toMatch(UUID);
  });

  it('lets the owner add people, make someone owner, or remove them — through dialogs', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();

    await user.click(within(tab).getByRole('button', { name: membersCopy.addPeople }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'add-people',
      target: 'channel',
      targetId: general.id,
    });

    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    await user.click(await screen.findByRole('menuitem', { name: membersCopy.makeOwner }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'transfer-ownership',
      channelId: general.id,
      successorId: bob.id,
    });

    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    await user.click(await screen.findByRole('menuitem', { name: membersCopy.remove('#general') }));
    expect(currentCrew().ui.dialog).toEqual({
      kind: 'confirm',
      confirm: { action: 'remove-channel-member', channelId: general.id, principalId: bob.id },
    });
  });

  it('offers Copy username before Copy person ID on the owner’s own row (T-33)', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();
    const more = within(tab).getByRole('button', { name: membersCopy.more('Alice Chen (@alice)') });
    await user.click(more);
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyUsername, membersCopy.copyPersonId]);
    // The username goes without its `@`, as Workspace settings copies it.
    await user.click(within(menu).getByRole('menuitem', { name: membersCopy.copyUsername }));
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe('alice'));

    await user.click(more);
    await user.click(await screen.findByRole('menuitem', { name: membersCopy.copyPersonId }));
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe(alice.id));
  });

  it('puts the owner’s actions on someone else after the copies', async () => {
    const user = userEvent.setup();
    renderCrew(Members);
    const tab = await shown();
    await user.click(within(tab).getByRole('button', { name: membersCopy.more('Bob Lee (@bob)') }));
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([
      membersCopy.copyUsername,
      membersCopy.copyPersonId,
      membersCopy.makeOwner,
      membersCopy.remove('#general'),
    ]);
  });

  it('never draws a ⋯ that holds only Copy person ID (T-33)', async () => {
    const user = userEvent.setup();
    // A member who is not the owner, looking at a named member, a former one and an unknown one.
    installObserver({
      snapshot: makeSnapshot({
        actor: bob,
        principals: [alice, bob],
        former_principals: [{ id: carol.id, username: 'carol', display_name: 'Carol Diaz' }],
        channels: [{ ...general, members: [alice.id, bob.id, carol.id, ghost] }, methods],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    const list = within(tab).getByRole('list', { name: membersCopy.listLabel('#general') });
    const items = within(list).getAllByRole('listitem');
    expect(items).toHaveLength(4);

    // The unknown member has nothing to copy but an ID, so the row has no menu at all.
    const unknown = items.find((item) => item.textContent?.includes('Unknown member'));
    expect(unknown).toBeDefined();
    expect(within(unknown as HTMLElement).queryByRole('button')).toBeNull();

    // Every menu that is drawn holds something besides the ID.
    const triggers = within(list).getAllByRole('button', { name: /^More actions for/ });
    expect(triggers).toHaveLength(3);
    for (const trigger of triggers) {
      await user.click(trigger);
      const menu = await screen.findByRole('menu');
      const entries = within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent);
      expect(entries).not.toEqual([membersCopy.copyPersonId]);
      expect(entries[0]).toBe(membersCopy.copyUsername);
      await user.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    }
  });

  it('tells a member who cannot add people who can, instead of showing nothing', async () => {
    const user = userEvent.setup();
    installObserver({ snapshot: makeSnapshot({ actor: bob }) });
    renderCrew(Members);
    const tab = await shown();
    expect(within(tab).queryByRole('button', { name: membersCopy.addPeople })).toBeNull();
    expect(within(tab).getByText(/^Ask/)).toHaveTextContent(
      'Ask Alice Chen (@alice) to add people.'
    );
    await user.click(
      within(tab).getByRole('button', { name: membersCopy.more('Alice Chen (@alice)') })
    );
    const menu = await screen.findByRole('menu');
    expect(within(menu).queryByRole('menuitem', { name: membersCopy.makeOwner })).toBeNull();
    expect(within(menu).queryByRole('menuitem', { name: /Remove from/ })).toBeNull();
  });

  it('names former and unknown members without ever printing an ID', async () => {
    const user = userEvent.setup();
    installObserver({
      snapshot: makeSnapshot({
        principals: [alice, bob],
        former_principals: [{ id: carol.id, username: 'carol', display_name: 'Carol Diaz' }],
        channels: [{ ...general, members: [alice.id, carol.id, ghost] }, methods],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    expect(within(tab).getByText(membersCopy.count(3))).toBeInTheDocument();
    const list = rows(tab);
    expect(list).toContain('Carol Diaz (@carol) · former member');
    expect(list).toContain('Unknown member');
    expect(tab.textContent).not.toMatch(UUID);
    // A former member is not offered Make owner or Remove; their ID stays behind Copy, after the
    // username, so the menu never holds the ID alone.
    await user.click(
      within(tab).getByRole('button', {
        name: membersCopy.more('Carol Diaz (@carol) · former member'),
      })
    );
    const menu = await screen.findByRole('menu');
    expect(
      within(menu)
        .getAllByRole('menuitem')
        .map((item) => item.textContent)
    ).toEqual([membersCopy.copyUsername, membersCopy.copyPersonId]);
  });

  it('shows channel invitations the viewer sent as muted invited rows', async () => {
    installObserver({
      snapshot: makeSnapshot({
        invitations: [
          {
            id: 'invitation-1',
            kind: 'channel',
            target_id: general.id,
            principal_id: carol.id,
            inviter_id: alice.id,
            expires_at: Date.now() / 1000 + 3600,
          },
          {
            id: 'invitation-2',
            kind: 'channel',
            target_id: general.id,
            principal_id: carol.id,
            inviter_id: alice.id,
            expires_at: 1,
            expired: true,
          },
        ],
      }),
    });
    renderCrew(Members);
    const tab = await shown();
    expect(rows(tab)).toEqual([
      'Alice Chen (@alice) · youOwner',
      'Bob Lee (@bob)',
      `Carol Diaz (@carol)${membersCopy.invited}`,
    ]);
    // The count is the channel's members; an invitation is not a member yet.
    expect(within(tab).getByText(membersCopy.count(2))).toBeInTheDocument();
  });
});
