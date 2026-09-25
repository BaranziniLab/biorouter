import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { hideOthers } from 'aria-hidden';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Invitation, PendingJoin } from '../crewApi';
import type { CrewController } from '../state/types';
import {
  acceptKey,
  AttentionSections,
  waitingChanges,
  waitingStates,
  type WaitingState,
} from './AttentionSections';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
import {
  alice,
  bob,
  connection,
  makeController,
  makeSnapshot,
  renderWithCrew,
  secondConnection,
} from './sidebarTestUtils';

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
  it('lists each invitation to me by name, from its inviter, with a small Join', () => {
    renderWithCrew(<AttentionSections />, asBob([invitation()]));
    expect(screen.getByRole('heading', { name: sidebarCopy.section.invitations })).toHaveClass(
      'text-caps'
    );
    const row = within(section(sidebarCopy.section.invitations)).getByRole('listitem');
    expect(row).toHaveTextContent('Imaging Core');
    expect(row).toHaveTextContent('from Alice Chen (@alice)');
    // One verb for one action (T-41): the main area's button says "Join Imaging Core" too.
    const accept = within(row).getByRole('button', { name: 'Join Imaging Core' });
    expect(accept).toHaveTextContent('Join');
    expect(accept).not.toHaveTextContent(/Accept/);
    // An sm button (32px, its own type), not an xs height carrying md text.
    expect(accept.className).toContain('h-control-sm');
    expect(accept.className).not.toContain('h-control-compact');
    // No machine string on the default path.
    expect(row.textContent).not.toMatch(/invitation-1|team-imaging|person-/);
  });

  it('names a channel invitation by #slug', () => {
    renderWithCrew(
      <AttentionSections />,
      asBob([invitation({ kind: 'channel', target_name: 'methods' })])
    );
    expect(screen.getByRole('button', { name: 'Join #methods' })).toBeVisible();
  });

  it('accepts through the controller, as the same broker call the old view made', async () => {
    const controller = asBob([invitation()]);
    renderWithCrew(<AttentionSections />, controller);
    fireEvent.click(screen.getByRole('button', { name: 'Join Imaging Core' }));
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
      within(row).getByRole('button', { name: 'Join, invited by Unknown member' })
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
    expect(screen.getByRole('button', { name: 'Join Imaging Core' })).toBeDisabled();

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
    expect(screen.getByRole('button', { name: 'Join Imaging Core' })).toBeDisabled();
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

  it('says whose turn it is while a joiner has not sent a code yet (Q2-42)', () => {
    renderWithCrew(<AttentionSections />, asHost());
    const rows = within(section(sidebarCopy.section.waiting)).getAllByRole('listitem');
    // "@bob · invited", then what happens next — the joiner sends a code, then the host acts.
    expect(rows[0].querySelector('[data-crew-waiting-state="invited"]')).toHaveTextContent(
      `${sidebarCopy.waiting.separator} ${sidebarCopy.waiting.invited}`
    );
    expect(rows[0]).toHaveTextContent(`@bob · Bob Lee (name on the server account) · invited`);
    const next = rows[0].querySelector('[data-crew-waiting-next]') as HTMLElement;
    expect(next).toHaveTextContent('Let in… when they send their code');
    // The button that acts on it is described by it.
    expect(
      within(rows[0]).getByRole('button', { name: 'Let @bob in' })
    ).toHaveAccessibleDescription(sidebarCopy.waiting.nextStep);
    // A code already entered is not "invited", and has no next step to wait for.
    expect(rows[2].querySelector('[data-crew-waiting-state="invited"]')).toBeNull();
    expect(rows[2].querySelector('[data-crew-waiting-next]')).toBeNull();
  });

  it('says nothing about a code for an invitation that ran out', () => {
    renderWithCrew(
      <AttentionSections />,
      makeController({
        snapshot: makeSnapshot({ pending_joins: [{ username: 'gail', expired: true }] }),
      })
    );
    const row = within(section(sidebarCopy.section.waiting)).getByRole('listitem');
    expect(row).not.toHaveTextContent(sidebarCopy.waiting.nextStep);
    expect(row).not.toHaveTextContent(sidebarCopy.waiting.invited);
  });

  it('opens Let in for that username', () => {
    const controller = asHost();
    renderWithCrew(<AttentionSections />, controller);
    fireEvent.click(screen.getByRole('button', { name: 'Let @erin in' }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'let-in', username: 'erin' });
  });

  it('warns under the row when a computer with a different code tried to join', () => {
    renderWithCrew(<AttentionSections />, asHost());
    const rows = within(section(sidebarCopy.section.waiting)).getAllByRole('listitem');
    expect(rows[1]).toHaveTextContent(sidebarCopy.waiting.otherDevice('erin'));
    expect(rows[0]).not.toHaveTextContent(sidebarCopy.waiting.otherDevice('bob'));
  });

  it('words the warning for the host’s own typo as well as another computer (T-13)', () => {
    // The broker cannot tell the two apart at approve time, so the sentence must cover both
    // and say what to do about each — never only "something else tried to join".
    expect(sidebarCopy.waiting.otherDevice('erin')).toBe(
      'A computer trying to join as @erin showed a different code. Check the code @erin sent ' +
        'you; if you typed it wrong, let them in again with the right code. Don’t approve a ' +
        'code you didn’t get from @erin.'
    );
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

  it('says "Code entered", never "Approved", once the host entered a code, with nothing to press', () => {
    renderWithCrew(<AttentionSections />, asHost());
    const rows = within(section(sidebarCopy.section.waiting)).getAllByRole('listitem');
    // The broker compares the code only when the joiner's computer checks in (T-13).
    expect(sidebarCopy.waiting.approved).toBe('Code entered');
    expect(rows[2]).toHaveTextContent('Code entered');
    expect(rows[2]).not.toHaveTextContent(/Approved/);
    expect(within(rows[2]).queryByRole('button')).toBeNull();
  });

  it('keeps Let in… beside "Code entered" after a different code, so a typo can be fixed', () => {
    const controller = makeController({
      snapshot: makeSnapshot({
        pending_joins: [{ username: 'hana', approved: true, mismatched_attempts: 1 }],
      }),
    });
    renderWithCrew(<AttentionSections />, controller);
    const row = within(section(sidebarCopy.section.waiting)).getByRole('listitem');
    expect(row).toHaveTextContent(sidebarCopy.waiting.approved);
    expect(row).toHaveTextContent(sidebarCopy.waiting.otherDevice('hana'));
    // The warning says "let them in again with the right code": the way to do it is right here.
    fireEvent.click(within(row).getByRole('button', { name: 'Let @hana in' }));
    expect(controller.openDialog).toHaveBeenCalledWith({ kind: 'let-in', username: 'hana' });
  });
});

describe('waitingChanges', () => {
  const state = (overrides: Partial<WaitingState> = {}): WaitingState => ({
    approved: false,
    expired: false,
    mismatches: 0,
    ...overrides,
  });

  it('says nothing about the first view: it is the baseline, not news', () => {
    expect(waitingChanges(null, new Map([['bob', state({ mismatches: 3 })]]))).toEqual([]);
  });

  it('reports someone new waiting, a code entered and a new different-code attempt', () => {
    const before = new Map([
      ['bob', state()],
      ['erin', state({ approved: true, mismatches: 1 })],
    ]);
    const after = new Map([
      ['bob', state({ approved: true })],
      ['erin', state({ approved: true, mismatches: 2 })],
      ['finn', state()],
    ]);
    expect(waitingChanges(before, after)).toEqual([
      { kind: 'code-entered', username: 'bob' },
      { kind: 'mismatch', username: 'erin' },
      { kind: 'waiting', username: 'finn' },
    ]);
  });

  it('repeats nothing that did not change, and stays quiet about expired and departed rows', () => {
    const before = new Map([
      ['bob', state({ approved: true, mismatches: 1 })],
      ['gail', state()],
    ]);
    expect(waitingChanges(before, new Map(before))).toEqual([]);
    // gail's invitation ran out; bob left the list (he joined, and the joined toast says so).
    expect(waitingChanges(before, new Map([['gail', state({ expired: true })]]))).toEqual([]);
  });

  it('reads the broker’s counts defensively', () => {
    const states = waitingStates({
      pending_joins: [
        { username: 'bob', mismatched_attempts: 2.7, approved: true },
        { username: 'erin', mismatched_attempts: -1 },
        { username: 'finn', mismatched_attempts: Number.NaN },
        null as unknown as PendingJoin,
      ],
    });
    expect(states.get('bob')).toEqual({ approved: true, expired: false, mismatches: 2 });
    expect(states.get('erin')?.mismatches).toBe(0);
    expect(states.get('finn')?.mismatches).toBe(0);
    expect(states.size).toBe(3);
  });
});

describe('announcing Waiting to join (T-17)', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  const polite = () => document.querySelector('[data-crew-sidebar-announcer]') as HTMLElement;
  const alerts = () => screen.queryAllByRole('alert');

  function host(pending: PendingJoin[], extra: Partial<CrewController> = {}) {
    return makeController({ snapshot: makeSnapshot({ pending_joins: pending }), ...extra });
  }

  function renderAnnounced(controller: CrewController) {
    return renderWithCrew(
      <SidebarAnnouncer>
        <AttentionSections />
      </SidebarAnnouncer>,
      controller
    );
  }

  it('says nothing about the people already waiting when the sidebar opens', () => {
    renderAnnounced(host([{ username: 'erin', mismatched_attempts: 2 }]));
    expect(polite()).toHaveTextContent('');
    expect(alerts()).toEqual([]);
  });

  it('announces someone new waiting, politely, with the workspace’s name', () => {
    const view = renderAnnounced(host([]));
    view.update(host([{ username: 'bob', full_name: 'Bob Lee' }]));
    expect(polite()).toHaveTextContent('@bob is waiting to join Fixture.');
    expect(alerts()).toEqual([]);
  });

  it('announces a code entered, then says it again when it happens again', () => {
    const view = renderAnnounced(host([{ username: 'bob' }]));
    view.update(host([{ username: 'bob', approved: true }]));
    const first = polite().firstElementChild;
    expect(polite()).toHaveTextContent(
      'Code entered for @bob; joins when their computer confirms.'
    );
    // Cancelled and invited again: the same sentence is a NEW message, in its own keyed node,
    // so a screen reader hears it again rather than seeing an unchanged text node.
    view.update(host([]));
    view.update(host([{ username: 'bob' }]));
    view.update(host([{ username: 'bob', approved: true }]));
    expect(polite()).toHaveTextContent(
      'Code entered for @bob; joins when their computer confirms.'
    );
    expect(polite().firstElementChild).not.toBe(first);
  });

  it('raises the different-code warning as an alert, once per new attempt', () => {
    const view = renderAnnounced(host([{ username: 'erin', approved: true }]));
    expect(alerts()).toEqual([]);

    view.update(host([{ username: 'erin', approved: true, mismatched_attempts: 1 }]));
    expect(screen.getByRole('alert')).toHaveTextContent(
      sidebarCopy.waiting.alertOtherDevice('erin')
    );
    const raised = screen.getByRole('alert');

    // A re-render with the same count is not a new attempt.
    view.update(host([{ username: 'erin', approved: true, mismatched_attempts: 1 }]));
    expect(screen.getByRole('alert')).toBe(raised);

    view.update(host([{ username: 'erin', approved: true, mismatched_attempts: 2 }]));
    expect(screen.getByRole('alert')).not.toBe(raised);
    expect(screen.getAllByRole('alert')).toHaveLength(1);
  });

  it('raises the warning even while a modal hides the rest of the page (Q2-47)', () => {
    const view = renderAnnounced(host([{ username: 'erin', approved: true }]));
    const hostSpan = document.querySelector('[data-crew-sidebar-alert]') as HTMLElement;
    // `hideOthers` keeps an element that carries `aria-live`: that is what keeps the host exposed.
    expect(hostSpan).toHaveAttribute('aria-live', 'assertive');
    // A modal opens (Radix calls aria-hidden's `hideOthers` on its content)…
    const dialog = document.createElement('div');
    dialog.setAttribute('role', 'dialog');
    document.body.appendChild(dialog);
    const undo = hideOthers(dialog);
    try {
      expect(hostSpan.closest('[aria-hidden="true"]')).toBeNull();
      // …and a different code lands while it is open: still an alert, still reachable.
      view.update(host([{ username: 'erin', approved: true, mismatched_attempts: 1 }]));
      expect(screen.getByRole('alert')).toHaveTextContent(
        sidebarCopy.waiting.alertOtherDevice('erin')
      );
      // The host holds no controls, so keeping it exposed exposes nothing else.
      expect(hostSpan.querySelector('button, a, input, [tabindex]')).toBeNull();
    } finally {
      undo();
      dialog.remove();
    }
  });

  it('clears both regions after a while, so a later message is heard as new', () => {
    vi.useFakeTimers();
    const view = renderAnnounced(host([{ username: 'erin' }]));
    view.update(host([{ username: 'erin', mismatched_attempts: 1 }, { username: 'finn' }]));
    expect(polite()).toHaveTextContent('@finn is waiting to join Fixture.');
    expect(screen.getByRole('alert')).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    expect(polite()).toHaveTextContent('');
    expect(alerts()).toEqual([]);
  });

  it('does not repeat the joined toast when someone leaves the list by joining', () => {
    const view = renderAnnounced(host([{ username: 'bob', approved: true }]));
    view.update(host([]));
    expect(polite()).toHaveTextContent('');
    expect(alerts()).toEqual([]);
  });

  it('compares only verified views, and starts over for another connection', () => {
    const pending: PendingJoin[] = [{ username: 'bob' }];
    const view = renderAnnounced(host([]));
    // A re-verifying view (the last verified copy) is not news.
    view.update(
      makeController({
        snapshot: null,
        observedPrivacy: null,
        effectivePrivacy: null,
        lastVerified: {
          connectionId: connection.id,
          snapshot: makeSnapshot({ pending_joins: pending }),
          observedPrivacy: {
            connectionId: connection.id,
            mode: 'private',
            institutionId: 'ucsf',
            policyEpoch: 1,
          },
          runs: [],
          labels: null,
          teamId: '',
          channelId: '',
          messages: [],
        },
      })
    );
    expect(polite()).toHaveTextContent('');

    // Another workspace's list is its own baseline: nobody there "started waiting" just now.
    const other = { ...secondConnection, status: 'connected' as const };
    view.update(
      makeController({
        connections: [connection, other],
        connectionId: other.id,
        connection: other,
        snapshot: makeSnapshot({ pending_joins: pending }),
        observedPrivacy: {
          connectionId: other.id,
          mode: 'private',
          institutionId: 'ucsf',
          policyEpoch: 1,
        },
      })
    );
    expect(polite()).toHaveTextContent('');
  });
});

it('renders nothing when there is nothing to attend to', () => {
  const { container } = renderWithCrew(<AttentionSections />);
  expect(container).toBeEmptyDOMElement();
});
