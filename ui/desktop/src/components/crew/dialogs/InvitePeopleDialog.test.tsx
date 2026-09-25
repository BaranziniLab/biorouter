import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { STALE_DAEMON_MESSAGE } from '../api/errors';
import type { CrewConnection } from '../crewApi';
import { expiryCopy, inviteCopy } from './copy';
import {
  connection,
  installResizeObserverStub,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { InvitePeopleDialog, withoutLeadingAt } from './InvitePeopleDialog';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

installResizeObserverStub();

const LINE = 'brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ';
const MESSAGE = `Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this whole message.\n${LINE}`;

/** A day from now, in the broker's unit: Unix seconds. */
const EXPIRES_AT = Math.floor(Date.now() / 1000) + 24 * 60 * 60;

/** The contract's expiry wording, spelled out here rather than read from the code under test. */
const expiresText = (seconds: number) =>
  `expires ${new Intl.DateTimeFormat(undefined, {
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(seconds * 1000))}`;

function renderInvite(
  refuse?: string,
  options: { connections?: CrewConnection[]; expiresAt?: number } = {}
) {
  const onClose = vi.fn();
  const view = renderWithCrew(<InvitePeopleDialog onClose={onClose} />, {
    connections: options.connections,
    request: (method, params) => {
      if (method !== 'enrollment.invite') return {};
      // What the daemon answers a broker refusal with: its text, as `CrewHttpError.message`.
      if (refuse) throw new CrewHttpError(refuse, 400, 'crew_request_refused');
      if ('uid' in params) return { invitation: 'token-secret-value' };
      return {
        username: 'bob',
        full_name: 'Bob Lee',
        add_device: params.add_device === true,
        join_id: 'join-1',
        expires_at: options.expiresAt ?? EXPIRES_AT,
      };
    },
  });
  return { ...view, onClose };
}

async function invite(username: string) {
  fireEvent.change(await screen.findByLabelText('Username'), { target: { value: username } });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Invite' }));
  });
}

beforeEach(() => {
  mocks.crewHttp.mockImplementation(async (path: string) =>
    path.startsWith('/connections/conn-1/invitation') ? { message: MESSAGE, line: LINE } : {}
  );
});
afterEach(() => vi.clearAllMocks());

describe('InvitePeopleDialog', () => {
  it('has one visible field, focused, with the @ as an adornment', async () => {
    renderInvite();
    expect(await screen.findByRole('dialog', { name: 'Invite people to lab' })).toBeInTheDocument();
    const username = screen.getByLabelText('Username');
    await waitFor(() => expect(username).toHaveFocus());
    expect(username).toBeRequired();
    expect(screen.queryByLabelText(inviteCopy.legacy.joinRequest)).toBeNull();
  });

  it('asks for their login on the server, and reminds the host what theirs is', async () => {
    // QA T-23: the placeholder said `bob` while the broker wanted the server login.
    renderInvite();
    const username = await screen.findByLabelText('Username');
    expect(username).toHaveAttribute('placeholder', 'their login on hpc.example.edu');
    expect(username).toHaveAccessibleDescription(
      'The name they sign in to hpc.example.edu with; yours is @alice.'
    );
  });

  it('keeps the reminder beside a refusal, which it describes the field with first', async () => {
    renderInvite('unknown_account: There is no account @Bob on this server. Check the spelling.');
    await invite('Bob');
    const username = screen.getByLabelText('Username');
    await waitFor(() =>
      expect(username).toHaveAccessibleDescription(
        `${inviteCopy.refusal.noAccount('Bob')} The name they sign in to hpc.example.edu with; yours is @alice.`
      )
    );
  });

  it('starts again with an empty field after Invite another', async () => {
    const { crew } = renderInvite();
    await invite('bob');
    const another = await screen.findByRole('button', { name: inviteCopy.inviteAnother });
    fireEvent.click(another);
    const username = await screen.findByLabelText('Username');
    expect(username).toHaveValue('');
    await waitFor(() => expect(username).toHaveFocus());
    expect(screen.queryByRole('button', { name: 'Copy invitation message' })).toBeNull();
    await invite('carol');
    expect(requestsFor(crew, 'enrollment.invite')).toEqual([
      { username: 'bob' },
      { username: 'carol' },
    ]);
  });

  // QA Q2-24: the field shows its own @, so a typed one read as "@ @crew_frank".
  it('drops a typed or pasted leading @ from the field, value and display alike', async () => {
    const { crew } = renderInvite();
    const username = await screen.findByLabelText('Username');
    fireEvent.change(username, { target: { value: '@' } });
    expect(username).toHaveValue('');
    fireEvent.change(username, { target: { value: '@crew_frank' } });
    expect(username).toHaveValue('crew_frank');
    fireEvent.change(username, { target: { value: ' @@bob' } });
    expect(username).toHaveValue('bob');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Invite' }));
    });
    expect(requestsFor(crew, 'enrollment.invite')).toEqual([{ username: 'bob' }]);
    // Only a LEADING @: anything else is the broker's to refuse, in its own words.
    expect(withoutLeadingAt('bo@b')).toBe('bo@b');
  });

  it('shows the install commands a line each, scrolling sideways instead of breaking a word', async () => {
    renderInvite();
    await invite('bob');
    const dialog = await screen.findByRole('dialog', { name: 'Invite people to lab' });
    fireEvent.click(within(dialog).getByRole('button', { name: inviteCopy.installed('Bob') }));
    const commands = await within(dialog).findByRole('button', {
      name: `Copy ${inviteCopy.installCommandsLabel}`,
    });
    const field = commands.closest('[data-slot="copy-field"]')!;
    expect(field).toHaveAttribute('data-multiline', 'true');
    expect(field.querySelector('.biorouter-copy-field-value')).toHaveClass('crew-command-lines');

    // jsdom loads no stylesheet: the rule is read at the source. It must out-rank the copy
    // field's own multi-line `pre-wrap` (three selectors deep in main.css) to apply at all.
    const css = readFileSync(join(__dirname, 'dialogs.css'), 'utf8').replace(
      /\/\*[\s\S]*?\*\//g,
      ' '
    );
    const rule = /([^{}]*\.crew-command-lines)\s*\{([^}]*)\}/.exec(css);
    expect(rule, 'the .crew-command-lines rule').not.toBeNull();
    expect(rule![2]).toMatch(/white-space:\s*pre;/);
    expect(rule![2]).toMatch(/overflow-x:\s*auto;/);
    expect(rule![2]).toMatch(/overflow-wrap:\s*normal;/);
    const specificity = (rule![1].match(/\.[\w-]+|\[[^\]]+\]/g) ?? []).length;
    expect(specificity).toBeGreaterThan(3);
  });

  it('gives install commands that run as written, with no placeholder path', () => {
    // QA T-44: `/path/to/biorouter-crew` is not something a person can run.
    expect(inviteCopy.installCommands).not.toMatch(/\/path\/to/);
    expect(inviteCopy.installCommands).toContain('"$HOME/.local/bin/biorouter-crew"');
  });

  it('invites by username and renders the broker’s answer, then the message to send', async () => {
    const { crew } = renderInvite();
    await invite('@bob');
    expect(requestsFor(crew, 'enrollment.invite')).toEqual([{ username: 'bob' }]);

    const dialog = await screen.findByRole('dialog', { name: 'Invite people to lab' });
    expect(
      within(dialog).getByText((_, node) =>
        Boolean(
          node?.tagName === 'P' &&
          node.textContent === '@bob · Bob Lee (name on the server account) · invited'
        )
      )
    ).toBeInTheDocument();
    expect(within(dialog).getByText(inviteCopy.sendInvitation('Bob'))).toBeInTheDocument();
    expect(
      await within(dialog).findByRole('button', { name: 'Copy invitation message' })
    ).toBeInTheDocument();
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/invitation?invitee=bob',
      'GET',
      undefined,
      undefined
    );
    expect(dialog).toHaveTextContent(LINE);
    expect(
      within(dialog).getByRole('button', { name: inviteCopy.installed('Bob') })
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(inviteCopy.nextStep('Bob'), { exact: false })
    ).toBeInTheDocument();
    await waitFor(() => expect(within(dialog).getByRole('button', { name: 'Done' })).toHaveFocus());
  });

  it('names the server as the person does, never by its address (QA Q4-34)', async () => {
    const labelled = {
      ...connection,
      ssh_target: 'crew_iris@52.33.141.141',
      server_label: 'lab-server',
    } as CrewConnection;
    renderInvite(undefined, { connections: [labelled] });
    const username = await screen.findByLabelText('Username');
    expect(username).toHaveAttribute('placeholder', 'their login on lab-server');
    expect(username).toHaveAccessibleDescription(
      'The name they sign in to lab-server with; yours is @alice.'
    );
    fireEvent.click(screen.getByRole('button', { name: inviteCopy.legacy.toggle }));
    expect(
      await screen.findByLabelText(inviteCopy.legacy.userId('lab-server'))
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('52.33.141.141');

    await invite('bob');
    const dialog = await screen.findByRole('dialog', { name: 'Invite people to lab' });
    fireEvent.click(within(dialog).getByRole('button', { name: inviteCopy.installed('Bob') }));
    expect(
      await within(dialog).findByText(inviteCopy.installLead('bob', 'lab-server'))
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('52.33.141.141');
  });

  it('says when the invitation expires, after what to do next (QA Q4-36)', async () => {
    renderInvite();
    await invite('bob');
    const dialog = await screen.findByRole('dialog', { name: 'Invite people to lab' });
    const next = await within(dialog).findByText(inviteCopy.nextStep('Bob'), { exact: false });
    expect(next).toHaveTextContent(
      `${inviteCopy.nextStep('Bob')} This invitation ${expiresText(EXPIRES_AT)}.`
    );
    // The broker's unit is seconds: read as milliseconds, a day from now would be January 1970.
    expect(next.textContent).not.toMatch(/expired/);
  });

  it('says an invitation that has already run out is expired, not a time in the past', async () => {
    renderInvite(undefined, { expiresAt: Math.floor(Date.now() / 1000) - 60 });
    await invite('bob');
    const next = await screen.findByText(inviteCopy.nextStep('Bob'), { exact: false });
    expect(next).toHaveTextContent(`This invitation ${expiryCopy.expired}.`);
  });

  it('keeps what only IT can answer collapsed, in the joiner’s words (QA Q4-37)', async () => {
    renderInvite();
    // Nothing asks the host which Biorouter the joiner runs.
    expect(inviteCopy.legacy.toggle).toBe('Other ways to invite (older Biorouter)');
    const other = await screen.findByRole('button', { name: inviteCopy.legacy.toggle });
    expect(other).toHaveAttribute('aria-expanded', 'false');
    await invite('bob');
    const dialog = await screen.findByRole('dialog', { name: 'Invite people to lab' });
    // What the joiner's Crew says when it isn't set up for them, as a heading to open if needed.
    expect(inviteCopy.installed('Bob')).toBe('If Bob sees “Crew isn’t set up”');
    const disclosure = within(dialog).getByRole('button', { name: inviteCopy.installed('Bob') });
    expect(disclosure).toHaveAttribute('aria-expanded', 'false');
    expect(within(dialog).queryByText(/~\/\.local\/bin/)).toBeNull();
    expect(dialog.textContent).not.toMatch(/Is Crew installed|older version/);
    fireEvent.click(disclosure);
    // One sentence, then the commands to copy.
    expect(
      await within(dialog).findByText(inviteCopy.installLead('bob', 'hpc.example.edu'))
    ).toBeInTheDocument();
    expect(inviteCopy.installLead('bob', 'hpc.example.edu')).toBe(
      'Send this to whoever runs hpc.example.edu, to run in @bob’s account:'
    );
    expect(
      within(dialog).getByRole('button', { name: `Copy ${inviteCopy.installCommandsLabel}` })
    ).toBeInTheDocument();
  });

  it('says a stale background service plainly when the message cannot be built', async () => {
    mocks.crewHttp.mockRejectedValue(new CrewHttpError('', 404));
    renderInvite();
    await invite('bob');
    expect(await screen.findByText(STALE_DAEMON_MESSAGE)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
  });

  it('offers the account’s exact spelling rather than accepting a near miss', async () => {
    renderInvite('identity_ambiguous: This server spells the account @bob. Invite @bob.');
    await invite('Bob');
    expect(await screen.findByText(inviteCopy.refusal.canonical('bob'))).toBeInTheDocument();
    expect(screen.getByLabelText('Username')).toHaveAttribute('aria-invalid', 'true');
  });

  it('reads an older daemon’s refusal envelope the same way, and never shows its JSON', async () => {
    const view = renderInvite(
      `Crew broker refused request: ${JSON.stringify({
        code: 'identity_ambiguous',
        message: 'identity_ambiguous: @Al is an alias on this server. Invite @alice.',
      })}`
    );
    await invite('Al');
    expect(await screen.findByText(inviteCopy.refusal.canonical('alice'))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('Crew broker refused request');
    view.unmount();

    renderInvite(
      `Crew broker refused request: ${JSON.stringify({
        code: 'identity_conflict',
        message:
          'identity_conflict: Another account on this server is already invited as @bob. Cancel that invitation first.',
      })}`
    );
    await invite('Bob');
    expect(
      await screen.findByText(
        'Another account on this server is already invited as @bob. Cancel that invitation first.'
      )
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('{');
  });

  it('says there is no such account, and offers Add device for an existing member', async () => {
    const missing = renderInvite(
      'unknown_account: There is no account @zed on this server. Check the spelling.'
    );
    await invite('zed');
    expect(await screen.findByText(inviteCopy.refusal.noAccount('zed'))).toBeInTheDocument();
    missing.unmount();

    const member = renderInvite(
      'already_member: @bob is already a member. Choose Add device to add another computer for them.'
    );
    await invite('bob');
    expect(
      await screen.findByText(inviteCopy.refusal.alreadyMember('bob', 'lab'))
    ).toBeInTheDocument();
    const addDevice = screen.getByRole('switch', { name: inviteCopy.addDevice('bob') });
    fireEvent.click(addDevice);
    member.crew.request.mockClear();
    await invite('bob');
    expect(requestsFor(member.crew, 'enrollment.invite')).toEqual([
      { username: 'bob', add_device: true },
    ]);
  });

  it('keeps the older token path under its own disclosure, and shows the token masked', async () => {
    const { crew } = renderInvite();
    fireEvent.click(await screen.findByRole('button', { name: inviteCopy.legacy.toggle }));
    const request = await screen.findByLabelText(inviteCopy.legacy.joinRequest);
    fireEvent.change(request, { target: { value: 'Crew join request\nUsername: @dana\nno key' } });
    expect(request).toBeInvalid();

    const key = 'ef'.repeat(32);
    fireEvent.change(request, {
      target: { value: `Crew join request\nUsername: @dana\nDevice key: ${key}` },
    });
    expect(request).toBeValid();
    expect(screen.getByText('id -u dana')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(inviteCopy.legacy.userId('hpc.example.edu')), {
      target: { value: '1050' },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: inviteCopy.legacy.submit }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'enrollment.invite')).toEqual([{ uid: 1050, public_key: key }])
    );
    expect(
      await screen.findByRole('button', { name: 'Copy invitation token' })
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('token-secret-value');
    expect(screen.getByText(inviteCopy.legacy.sendToken('@dana'))).toBeInTheDocument();
  });
});
