import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Invitation, PendingJoin } from '../crewApi';
import { acceptKey, AttentionSections } from './AttentionSections';
import { sidebarCopy } from './copy';
import { alice, bob, makeController, makeSnapshot, renderWithCrew } from './sidebarTestUtils';

const carol = { id: 'person-carol-0000', uid: 1002, username: 'carol', nickname: 'Carol Diaz' };

function invitation(overrides: Partial<Invitation> = {}): Invitation {
  return {
    id: 'invitation-1',
    kind: 'team',
    target_id: 'team-imaging-0000',
    principal_id: bob.id,
    inviter_id: alice.id,
    expires_at: 4_000_000_000,
    target_name: 'Imaging Core',
    ...overrides,
  };
}

/** Bob, a member (not the host), with the given invitations. */
function asBob(invitations: Invitation[], pending_joins?: PendingJoin[]) {
  return makeController({
    snapshot: makeSnapshot({
      actor: bob,
      principals: [alice, bob, carol],
      invitations,
      ...(pending_joins ? { pending_joins } : {}),
    }),
    isHost: false,
  });
}

function section(name: string) {
  return screen.getByRole('list', { name });
}

describe('Invitations', () => {
  it('lists each invitation to me by name, from its inviter, with a small Accept', () => {
    renderWithCrew(<AttentionSections />, asBob([invitation()]));
    expect(screen.getByRole('heading', { name: sidebarCopy.section.invitations })).toHaveClass(
      'text-caps'
    );
    const row = within(section(sidebarCopy.section.invitations)).getByRole('listitem');
    expect(row).toHaveTextContent('Imaging Core');
    expect(row).toHaveTextContent('from Alice Chen (@alice)');
    const accept = within(row).getByRole('button', { name: 'Accept invitation to Imaging Core' });
    expect(accept).toHaveTextContent(sidebarCopy.invitation.accept);
    // No machine string on the default path.
    expect(row.textContent).not.toMatch(/invitation-1|team-imaging|person-/);
  });

  it('names a channel invitation by #slug', () => {
    renderWithCrew(
      <AttentionSections />,
      asBob([invitation({ kind: 'channel', target_name: 'methods' })])
    );
    expect(screen.getByRole('button', { name: 'Accept invitation to #methods' })).toBeVisible();
  });

  it('accepts through the controller, as the same broker call the old view made', async () => {
    const controller = asBob([invitation()]);
    renderWithCrew(<AttentionSections />, controller);
    fireEvent.click(screen.getByRole('button', { name: 'Accept invitation to Imaging Core' }));
    await waitFor(() =>
      expect(controller.mutate).toHaveBeenCalledWith('invitation.accept', {
        invitation_id: 'invitation-1',
      })
    );
    expect(controller.act).toHaveBeenCalledWith(
      'global',
      acceptKey('invitation-1'),
      expect.any(Function)
    );
  });

  it('never shows an ID for an invitation the broker did not name yet', () => {
    renderWithCrew(
      <AttentionSections />,
      asBob([invitation({ target_name: undefined, inviter_id: 'person-unknown-0000' })])
    );
    const row = within(section(sidebarCopy.section.invitations)).getByRole('listitem');
    expect(row).toHaveTextContent(sidebarCopy.invitation.untitled);
    expect(row).toHaveTextContent('from Unknown member');
    expect(
      within(row).getByRole('button', { name: 'Accept invitation from Unknown member' })
    ).toBeInTheDocument();
    expect(row.textContent).not.toMatch(/person-|team-imaging/);
  });

  it('names the inviter from the broker’s projection when the directory lacks them', () => {
    renderWithCrew(
      <AttentionSections />,
      asBob([
        invitation({
          inviter_id: 'person-gone-0000',
          inviter: { username: 'dana', display_name: 'Dana Wu' },
        }),
      ])
    );
    expect(screen.getByText('Dana Wu')).toBeInTheDocument();
  });

  it('shows only invitations addressed to me that have not expired', () => {
    renderWithCrew(
      <AttentionSections />,
      asBob([
        invitation({ id: 'mine' }),
        invitation({ id: 'to-carol', principal_id: carol.id, target_name: 'Carol team' }),
        invitation({ id: 'expired', expired: true, target_name: 'Old team' }),
      ])
    );
    const rows = within(section(sidebarCopy.section.invitations)).getAllByRole('listitem');
    expect(rows).toHaveLength(1);
    expect(screen.queryByText('Carol team')).toBeNull();
    expect(screen.queryByText('Old team')).toBeNull();
  });

  it('disables Accept while it runs, and while the view is only the last verified copy', () => {
    const controller = asBob([invitation()]);
    const view = renderWithCrew(<AttentionSections />, {
      ...controller,
      isPending: (key) => key === acceptKey('invitation-1'),
    });
    expect(
      screen.getByRole('button', { name: 'Accept invitation to Imaging Core' })
    ).toBeDisabled();

    view.update({
      ...controller,
      snapshot: null,
      observedPrivacy: null,
      lastVerified: {
        connectionId: controller.connectionId,
        snapshot: controller.snapshot!,
        observedPrivacy: controller.observedPrivacy!,
        runs: [],
        labels: null,
        teamId: '',
        channelId: '',
        messages: [],
      },
    });
    expect(
      screen.getByRole('button', { name: 'Accept invitation to Imaging Core' })
    ).toBeDisabled();
  });
});

describe('Waiting to join', () => {
  const pending: PendingJoin[] = [
    { username: 'bob', full_name: 'Bob Lee' },
    { username: 'erin', full_name: 'Erin Park', mismatched_attempts: 2 },
    { username: 'finn', approved: true },
  ];

  function asHost() {
    return makeController({ snapshot: makeSnapshot({ pending_joins: pending }) });
  }

  it('lists each joiner @username first, then the name on the server account, with Let in…', () => {
    renderWithCrew(<AttentionSections />, asHost());
    const rows = within(section(sidebarCopy.section.waiting)).getAllByRole('listitem');
    expect(rows[0]).toHaveTextContent('@bob · Bob Lee (name on the server account)');
    const username = rows[0].querySelector('[data-person-part="username"]');
    expect(username).toHaveTextContent('@bob');
    expect(username).toHaveClass('font-mono');
    expect(within(rows[0]).getByRole('button', { name: 'Let @bob in' })).toHaveTextContent(
      sidebarCopy.waiting.letIn
    );
  });

  it('opens Let in for that username', () => {
    const controller = asHost();
    renderWithCrew(<AttentionSections />, controller);
    fireEvent.click(screen.getByRole('button', { name: 'Let @erin in' }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'let-in', username: 'erin' });
  });

  it('warns under the row when a device with a different code tried to join', () => {
    renderWithCrew(<AttentionSections />, asHost());
    const rows = within(section(sidebarCopy.section.waiting)).getAllByRole('listitem');
    expect(rows[1]).toHaveTextContent(sidebarCopy.waiting.otherDevice('erin'));
    expect(rows[0]).not.toHaveTextContent(sidebarCopy.waiting.otherDevice('bob'));
  });

  it('shows an expired join as expired, to invite again, never to let in', () => {
    const controller = makeController({
      snapshot: makeSnapshot({
        pending_joins: [
          { username: 'gail', approved: true, expired: true, mismatched_attempts: 1 },
        ],
      }),
    });
    renderWithCrew(<AttentionSections />, controller);
    const row = within(section(sidebarCopy.section.waiting)).getByRole('listitem');
    expect(row).toHaveTextContent(
      `${sidebarCopy.waiting.expired} ${sidebarCopy.waiting.separator} ${sidebarCopy.waiting.inviteAgain}`
    );
    expect(row).not.toHaveTextContent(sidebarCopy.waiting.approved);
    expect(within(row).queryByRole('button', { name: 'Let @gail in' })).toBeNull();
    // The different-code warning still says someone tried.
    expect(row).toHaveTextContent(sidebarCopy.waiting.otherDevice('gail'));

    fireEvent.click(within(row).getByRole('button', { name: 'Invite @gail again' }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'invite-people' });
  });

  it('shows an approved joiner as approved, with nothing to press', () => {
    renderWithCrew(<AttentionSections />, asHost());
    const rows = within(section(sidebarCopy.section.waiting)).getAllByRole('listitem');
    expect(rows[2]).toHaveTextContent(sidebarCopy.waiting.approved);
    expect(within(rows[2]).queryByRole('button')).toBeNull();
  });
});

it('renders nothing when there is nothing to attend to', () => {
  const { container } = renderWithCrew(<AttentionSections />);
  expect(container).toBeEmptyDOMElement();
});
