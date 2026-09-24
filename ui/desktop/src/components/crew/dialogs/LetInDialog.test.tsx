import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { useState, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError, type PendingJoin, type Snapshot } from '../crewApi';
import { CrewControllerProvider, useCrew } from '../state/CrewControllerContext';
import { addPeopleCopy, letInCopy } from './copy';
import { alice, bob, makeSnapshot, renderWithCrew, requestsFor } from './dialogsTestHarness';
import { LetInDialog } from './LetInDialog';
import { DIRECT_ADD_CAPABILITY } from './people';

const eve = { id: 'person-eve', uid: 1004, username: 'eve', nickname: 'Eve Park' };

function renderLetIn(join: PendingJoin, options: { joined?: boolean; refuse?: string } = {}) {
  const snapshot = makeSnapshot({
    principals: options.joined ? [...makeSnapshot().principals, eve] : makeSnapshot().principals,
    pending_joins: [join],
  });
  const onClose = vi.fn();
  const view = renderWithCrew(<LetInDialog username={join.username} onClose={onClose} />, {
    snapshot,
    request: (method, params) => {
      // The broker refuses a first approval; one that asks to replace it is accepted.
      if (options.refuse && method === 'enrollment.approve' && params.replace !== true)
        throw new CrewHttpError(options.refuse, 400, 'crew_request_refused');
      return {};
    },
  });
  return { ...view, onClose };
}

/**
 * The dialog under a controller whose snapshot and capabilities a test can change after render, as
 * the observer does: `update` swaps in the next snapshot.
 */
function renderLive(
  initial: Snapshot,
  options: {
    capabilities?: string[];
    request?: (method: string, params: Record<string, unknown>) => unknown;
  } = {}
) {
  const live: { set: (snapshot: Snapshot) => void } = { set: () => {} };
  function Live({ children }: { children: ReactNode }) {
    // From context, so the harness's errors and pending keys stay live.
    const crew = useCrew();
    const [snapshot, setSnapshot] = useState(initial);
    live.set = setSnapshot;
    return (
      <CrewControllerProvider
        controller={{ ...crew, snapshot, capabilities: options.capabilities ?? null }}
      >
        {children}
      </CrewControllerProvider>
    );
  }
  const view = renderWithCrew(
    <Live>
      <LetInDialog username="eve" onClose={vi.fn()} />
    </Live>,
    { snapshot: initial, request: options.request }
  );
  return { ...view, update: (next: Snapshot) => act(() => live.set(next)) };
}

/** The harness snapshot plus a #methods channel in team-1; `pending_joins` defaults to eve's. */
function withMethods(overrides: Partial<Snapshot> & { methodsMembers?: string[] } = {}): Snapshot {
  const { methodsMembers = [alice.id], ...rest } = overrides;
  const base = makeSnapshot();
  return makeSnapshot({
    pending_joins: [{ username: 'eve', full_name: 'Eve Park' }],
    ...rest,
    channels: [
      ...base.channels,
      {
        id: 'channel-methods',
        team_id: 'team-1',
        name: 'methods',
        created_by: alice.id,
        owner_id: alice.id,
        members: methodsMembers,
        archived: false,
        classification: 'restricted',
      },
    ],
  });
}

/** A broker that adds members directly, and answers `team.add_member` for eve into #methods. */
function renderDirectAdd(snapshot: Snapshot) {
  return renderLive(snapshot, {
    capabilities: ['unique_names_v1', DIRECT_ADD_CAPABILITY],
    request: (method) =>
      method === 'team.add_member'
        ? {
            team_id: 'team-1',
            principal_id: eve.id,
            added_channels: ['channel-methods'],
            already_member: false,
          }
        : {},
  });
}

async function approveWith(code: string, who = 'Eve') {
  fireEvent.change(await screen.findByLabelText(letInCopy.code(who)), { target: { value: code } });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: `Let ${who} in` }));
  });
}

const CODE = '7QK2M9XA3JTPWZ4D';
/** The broker's literal refusal (`broker/join.rs`, `approve`). */
const ALREADY_APPROVED =
  'already_approved: You already let a device in for @eve. Replace the code only if they sent you a new one.';
const code = () => screen.getByLabelText(letInCopy.code('Eve'));

afterEach(() => vi.clearAllMocks());

describe('LetInDialog', () => {
  it('names the joiner by @username and the name on their server account, and focuses the code', async () => {
    renderLetIn({ username: 'eve', full_name: 'Eve Park' });
    const dialog = await screen.findByRole('dialog', { name: 'Let @eve into lab' });
    expect(dialog).toHaveTextContent('Eve Park (name on the server account)');
    await waitFor(() => expect(code()).toHaveFocus());
    expect(code()).toHaveAttribute('autocomplete', 'one-time-code');
    expect(screen.getByText(letInCopy.helper('Eve'))).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Let Eve in' })).toHaveAttribute('type', 'submit');
  });

  it('accepts a pasted code with hyphens, spaces or lower case, and sends it normalized', async () => {
    const { crew } = renderLetIn({ username: 'eve', full_name: 'Eve Park' });
    const field = await screen.findByLabelText(letInCopy.code('Eve'));
    fireEvent.change(field, { target: { value: '7qk2 m9xa-3jtp wz4d' } });
    expect(field).toHaveValue('7QK2-M9XA-3JTP-WZ4D');
    expect(field).toBeValid();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Eve in' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.approve')).toEqual([{ username: 'eve', code: CODE }])
    );
  });

  it('reads Crockford lookalikes as the broker does and refuses U before sending', async () => {
    const { crew } = renderLetIn({ username: 'eve' });
    const field = await screen.findByLabelText(letInCopy.code('@eve'));
    fireEvent.change(field, { target: { value: 'io00-l000-0000-0000' } });
    expect(field).toHaveValue('1000-1000-0000-0000');
    expect(field).toBeValid();

    fireEvent.change(field, { target: { value: '7QK2-M9XA-3JTP-WZ4U' } });
    expect(field).toBeInvalid();
    fireEvent.click(screen.getByRole('button', { name: 'Let @eve in' }));
    expect(
      await screen.findByText('Device codes never contain the letter U. Check the code.')
    ).toBeInTheDocument();
    expect(requestsFor(crew, 'enrollment.approve')).toEqual([]);
  });

  it('warns about a different-code device before anything is typed, and says what to do', async () => {
    renderLetIn({ username: 'eve', full_name: 'Eve Park', mismatched_attempts: 2 });
    // The host's own typo reads as a check to make, not as an impostor (QA T-13).
    const warning = await screen.findByText(letInCopy.mismatch('eve'));
    expect(warning).toHaveTextContent('If you typed it wrong, enter it again and choose Replace.');
    expect(code()).toHaveValue('');
    // The warning comes before the field in reading order.
    expect(warning.compareDocumentPosition(code()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('never renders a code it was not given', async () => {
    const { container } = renderLetIn({ username: 'eve', full_name: 'Eve Park' });
    await screen.findByLabelText(letInCopy.code('Eve'));
    const text = document.body.textContent ?? '';
    expect(text).not.toMatch(/[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}-/);
    expect(code()).toHaveValue('');
    expect(container).toBeTruthy();
  });

  it('says a device was already let in, and replaces the code only when asked', async () => {
    const { crew } = renderLetIn(
      { username: 'eve', full_name: 'Eve Park' },
      { refuse: ALREADY_APPROVED }
    );
    fireEvent.change(await screen.findByLabelText(letInCopy.code('Eve')), {
      target: { value: CODE },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Eve in' }));
    });
    expect(await screen.findByText(letInCopy.alreadyApproved('eve'))).toBeInTheDocument();
    expect(screen.getByText(letInCopy.replaceHelp)).toBeInTheDocument();
    expect(requestsFor(crew, 'enrollment.approve')).toEqual([{ username: 'eve', code: CODE }]);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: letInCopy.replace }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.approve')).toEqual([
        { username: 'eve', code: CODE },
        { username: 'eve', code: CODE, replace: true },
      ])
    );
    expect(await screen.findByText(letInCopy.approved('@eve'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('7QK2');
  });

  it('keeps Let in disabled until the field holds a whole code', async () => {
    const { crew } = renderLetIn({ username: 'eve', full_name: 'Eve Park' });
    const field = await screen.findByLabelText(letInCopy.code('Eve'));
    const submit = screen.getByRole('button', { name: 'Let Eve in' });
    expect(submit).toBeDisabled();
    fireEvent.change(field, { target: { value: '7QK2-M9XA' } });
    expect(submit).toBeDisabled();
    // A short code is not called wrong while it is still being typed…
    expect(screen.queryByText('A device code has 16 letters and numbers.')).toBeNull();
    // …but Return says why nothing happened.
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(
      await screen.findByText('A device code has 16 letters and numbers.')
    ).toBeInTheDocument();
    fireEvent.change(field, { target: { value: CODE } });
    expect(submit).toBeEnabled();
    expect(requestsFor(crew, 'enrollment.approve')).toEqual([]);
  });

  it('says the code was saved, never "Approved", then that they joined once they have', async () => {
    const pending = makeSnapshot({ pending_joins: [{ username: 'eve', full_name: 'Eve Park' }] });
    const { update } = renderLive(pending);
    await approveWith(CODE);
    expect(await screen.findByText(letInCopy.approved('@eve'))).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/Approved|checks in/);

    update(makeSnapshot({ principals: [...pending.principals, eve], pending_joins: [] }));
    expect(await screen.findByText(letInCopy.joined('@eve', 'lab'))).toBeInTheDocument();
    expect(screen.queryByText(letInCopy.approved('@eve'))).toBeNull();
  });

  it('brings the mismatch back after saving, with a way to enter the code again', async () => {
    const pending = makeSnapshot({ pending_joins: [{ username: 'eve', full_name: 'Eve Park' }] });
    const { update } = renderLive(pending);
    await approveWith(CODE);
    await screen.findByText(letInCopy.approved('@eve'));

    // The joiner's computer showed a different code from the one the host typed.
    update(
      makeSnapshot({
        pending_joins: [{ username: 'eve', full_name: 'Eve Park', mismatched_attempts: 1 }],
      })
    );
    const warning = await screen.findByRole('alert');
    expect(warning).toHaveTextContent(letInCopy.mismatch('eve'));
    fireEvent.click(within(warning).getByRole('button', { name: letInCopy.enterAgain }));
    expect(await screen.findByLabelText(letInCopy.code('Eve'))).toHaveValue('');
  });

  it('reads an older daemon’s envelope the same way, and drops Replace once the code changes', async () => {
    const { crew } = renderLetIn(
      { username: 'eve', full_name: 'Eve Park' },
      {
        refuse: `Crew broker refused request: ${JSON.stringify({
          code: 'already_approved',
          message: ALREADY_APPROVED,
        })}`,
      }
    );
    const field = await screen.findByLabelText(letInCopy.code('Eve'));
    fireEvent.change(field, { target: { value: CODE } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Eve in' }));
    });
    expect(await screen.findByText(letInCopy.alreadyApproved('eve'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('Crew broker refused request');
    expect(screen.getByRole('button', { name: letInCopy.replace })).toBeInTheDocument();

    // A different code is a new approval, not a replacement of the one refused.
    fireEvent.change(field, { target: { value: '1000-1000-0000-0000' } });
    expect(screen.queryByRole('button', { name: letInCopy.replace })).toBeNull();
    expect(screen.queryByText(letInCopy.alreadyApproved('eve'))).toBeNull();
    expect(requestsFor(crew, 'enrollment.approve')).toEqual([{ username: 'eve', code: CODE }]);
  });

  it('shows any other refusal in the broker’s own sentence', async () => {
    renderLetIn(
      { username: 'eve', full_name: 'Eve Park' },
      { refuse: 'not_invited: @eve has no pending invitation. Invite them first.' }
    );
    fireEvent.change(await screen.findByLabelText(letInCopy.code('Eve')), {
      target: { value: CODE },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Eve in' }));
    });
    expect(
      await screen.findByText('@eve has no pending invitation. Invite them first.')
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: letInCopy.replace })).toBeNull();
  });

  it('after approval, drops the code and offers one button per team the host created', async () => {
    const { crew } = renderLetIn({ username: 'eve', full_name: 'Eve Park' }, { joined: true });
    fireEvent.change(await screen.findByLabelText(letInCopy.code('Eve')), {
      target: { value: CODE },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Eve in' }));
    });
    expect(await screen.findByText(letInCopy.approved('@eve'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('7QK2');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toHaveFocus());

    // An older broker invites: the button and its outcome both say so (QA P0-2).
    const add = screen.getByRole('button', { name: 'Invite @eve to Analysis Lab' });
    await act(async () => {
      fireEvent.click(add);
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'invitation.create')).toEqual([
        {
          kind: 'team',
          target_id: 'team-1',
          principal_id: eve.id,
          expected_username: 'eve',
        },
      ])
    );
    expect(await screen.findByText(letInCopy.addedToTeam('@eve'))).toBeInTheDocument();
    expect(letInCopy.addedToTeam('@eve')).toBe(
      'Invited. @eve will see it in Crew and needs to accept.'
    );
  });

  it('adds the person straight into the team and the channels chosen, when the broker can', async () => {
    const { crew } = renderDirectAdd(
      withMethods({ principals: [...makeSnapshot().principals, eve] })
    );
    await approveWith(CODE);
    const dialog = await screen.findByRole('dialog');
    const channels = within(dialog).getByRole('group', { name: addPeopleCopy.channels });
    const general = within(channels).getByRole('checkbox', { name: /#general/ });
    expect(general).toBeChecked();
    expect(general).toBeDisabled();
    expect(within(channels).getByRole('checkbox', { name: /#methods/ })).toBeChecked();

    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Add @eve to Analysis Lab' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'team.add_member')).toEqual([
        {
          team_id: 'team-1',
          principal_id: eve.id,
          expected_username: 'eve',
          channel_ids: ['channel-methods'],
        },
      ])
    );
    expect(requestsFor(crew, 'invitation.create')).toEqual([]);
    expect(
      await screen.findByText('Added. @eve can now see #general and #methods.')
    ).toBeInTheDocument();
  });

  it('keeps the "Added." line once the next state frame shows the person in the team', async () => {
    // A fresh joiner: not a member when the code is saved, so the team waits for them.
    const { update } = renderDirectAdd(withMethods());
    await approveWith(CODE);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByRole('button', { name: 'Add @eve to Analysis Lab' })).toBeDisabled();

    // They join; the observer's frame names them, and the team control comes alive.
    const joined = withMethods({
      principals: [...makeSnapshot().principals, eve],
      pending_joins: [],
    });
    update(joined);
    expect(await screen.findByText(letInCopy.joined('@eve', 'lab'))).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Add @eve to Analysis Lab' }));
    });
    const added = 'Added. @eve can now see #general and #methods.';
    expect(await screen.findByText(added)).toBeInTheDocument();

    // The broker put them in the team, and the next frame (at most 2 s later) says so. The
    // outcome is what the host needs to read, so it must not go with the team's button (P0-2).
    const inTeam = () =>
      withMethods({
        principals: [...makeSnapshot().principals, eve],
        pending_joins: [],
        teams: makeSnapshot().teams.map((team) =>
          team.id === 'team-1' ? { ...team, members: [...team.members, eve.id] } : team
        ),
        methodsMembers: [alice.id, eve.id],
      });
    update(inTeam());
    expect(within(dialog).getByText(added).closest('[role="status"]')).not.toBeNull();
    expect(within(dialog).getByText(letInCopy.joined('@eve', 'lab'))).toBeInTheDocument();
    // Nor with any frame after it.
    update(inTeam());
    expect(within(dialog).getByText(added)).toBeInTheDocument();
  });

  it('keeps Add to team waiting until the person has joined', async () => {
    renderLetIn({ username: 'eve', full_name: 'Eve Park' });
    fireEvent.change(await screen.findByLabelText(letInCopy.code('Eve')), {
      target: { value: CODE },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Eve in' }));
    });
    const dialog = await screen.findByRole('dialog');
    expect(
      within(dialog).getByRole('button', { name: 'Invite @eve to Analysis Lab' })
    ).toBeDisabled();
    expect(within(dialog).getByText(letInCopy.addAfterJoin('Eve'))).toBeInTheDocument();
  });

  it('offers no team the person is already in', async () => {
    const snapshot = makeSnapshot({ pending_joins: [{ username: 'bob' }] });
    renderWithCrew(<LetInDialog username="bob" onClose={vi.fn()} />, { snapshot });
    fireEvent.change(await screen.findByLabelText(letInCopy.code('Bob')), {
      target: { value: CODE },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Let Bob in' }));
    });
    await screen.findByText(letInCopy.approved('@bob'));
    expect(screen.queryByRole('button', { name: /to Analysis Lab$/ })).toBeNull();
    expect(bob.id).toBe('person-bob');
  });
});
