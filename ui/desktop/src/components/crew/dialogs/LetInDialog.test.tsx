import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError, type PendingJoin } from '../crewApi';
import { letInCopy, workspaceSettingsCopy } from './copy';
import { bob, makeSnapshot, renderWithCrew, requestsFor } from './dialogsTestHarness';
import { LetInDialog } from './LetInDialog';

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

  it('warns about a different-code device before anything is typed', async () => {
    renderLetIn({ username: 'eve', full_name: 'Eve Park', mismatched_attempts: 2 });
    const warning = await screen.findByText(workspaceSettingsCopy.otherDevice('eve'));
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
    expect(await screen.findByText(letInCopy.approved('Eve'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('7QK2');
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
    expect(await screen.findByText(letInCopy.approved('Eve'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('7QK2');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toHaveFocus());

    const add = screen.getByRole('button', { name: 'Add Eve to Analysis Lab' });
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
    expect(
      await screen.findByText(letInCopy.addedToTeam('Eve', 'Analysis Lab'))
    ).toBeInTheDocument();
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
    expect(within(dialog).getByRole('button', { name: 'Add Eve to Analysis Lab' })).toBeDisabled();
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
    await screen.findByText(letInCopy.approved('Bob'));
    expect(screen.queryByRole('button', { name: /^Add Bob to/ })).toBeNull();
    expect(bob.id).toBe('person-bob');
  });
});
