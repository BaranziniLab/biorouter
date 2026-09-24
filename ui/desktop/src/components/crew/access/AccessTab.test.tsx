import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { useCrew } from '../state/CrewControllerContext';
import { AccessTab } from './AccessTab';
import { accessCopy } from './copy';
import {
  callsTo,
  grantRow,
  installDaemon,
  renderWithController,
  type DaemonFixture,
} from './testing';
import { forgetUnconfirmedRevocations } from './useCrewGrants';
import { WorkspaceAgentAccess } from './WorkspaceAgentAccess';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
  navigate: vi.fn(),
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
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mocks.navigate };
});

const past = () => Math.floor(Date.now() / 1000) - 60;

/** The grants a workspace holds: an active chat, a running task, a revoked and an expired chat. */
function workspaceGrants(state: { revoked?: string[] } = {}) {
  const revoked = new Set(state.revoked ?? []);
  return () => [
    grantRow({ expired: revoked.has('agent-1') }),
    grantRow({
      session_id: 'task-1',
      run_id: 'run-2',
      kind: 'task',
      session_name: 'Crew task',
      source_channels: ['channel-1', 'channel-2'],
    }),
    grantRow({ session_id: 'old-chat', session_name: 'Old chat', expired: true }),
    grantRow({ session_id: 'late-chat', session_name: 'Late chat', expires_at: past() }),
    grantRow({
      session_id: 'methods-chat',
      session_name: 'Methods chat',
      channel_id: 'channel-2',
      source_channels: ['channel-2'],
    }),
  ];
}

const RUNS = [
  { run_id: 'run-2', channel_id: 'channel-1', session_id: 'task-1', status: 'running' },
];

function TabLayout() {
  const { channel } = useCrew();
  return channel ? <AccessTab /> : null;
}

function WorkspaceLayout() {
  const { snapshot } = useCrew();
  return snapshot ? <WorkspaceAgentAccess /> : null;
}

function setup(fixture: Partial<DaemonFixture>, layout = TabLayout) {
  installDaemon(mocks, { grants: workspaceGrants(), runs: RUNS, ...fixture });
  renderWithController(layout, '/crew');
}

const rowFor = async (title: string) => {
  const rows = await screen.findAllByTestId('crew-access-row');
  const row = rows.find((item) => within(item).queryByText(title));
  if (!row) throw new Error(`No access row for ${title}`);
  return row;
};

describe('the Access tab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('lists this channel’s chats with Revoke and tasks with Stop, and folds revoked and expired rows', async () => {
    setup({});
    const chat = await rowFor('Plot review');
    expect(chat).toHaveTextContent('#general');
    expect(
      within(chat).getByRole('button', { name: 'Revoke access for Plot review' })
    ).toBeInTheDocument();
    expect(within(chat).queryByRole('button', { name: accessCopy.stopRowName })).toBeNull();

    const task = await rowFor(accessCopy.yourTask);
    expect(task).toHaveTextContent('#general');
    // The +1 is spoken as words.
    expect(task).toHaveTextContent('+1');
    expect(within(task).getByText(accessCopy.moreSourcesName(1))).toBeInTheDocument();
    expect(within(task).getByRole('button', { name: accessCopy.stopRowName })).toBeInTheDocument();
    expect(within(task).queryByRole('button', { name: /^Revoke/ })).toBeNull();

    // Another channel's chat is not in this channel's list.
    expect(screen.queryByText('Methods chat')).toBeNull();
    // Revoked and expired rows wait behind their disclosure.
    expect(screen.queryByText('Old chat')).toBeNull();
    expect(screen.queryByText('Late chat')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: accessCopy.showOld(2) }));
    const old = await rowFor('Old chat');
    expect(old).toHaveTextContent(accessCopy.status.revoked);
    expect(within(old).queryByRole('button', { name: /^Revoke/ })).toBeNull();
    expect(await rowFor('Late chat')).toHaveTextContent(accessCopy.status.expired);
  });

  it('never shows a machine ID in a row', async () => {
    setup({});
    await rowFor('Plot review');
    const list = screen.getByTestId('crew-access-list');
    expect(list.textContent).not.toMatch(/agent-1|task-1|run-|channel-|conn-1/);
  });

  it('revokes a chat after an inline confirmation and refetches the list', async () => {
    const state = { revoked: [] as string[] };
    setup({
      grants: () => workspaceGrants(state)(),
      revoke: (sessionId) => {
        state.revoked.push(sessionId);
        return { revoked: true, remote_revocation_confirmed: true };
      },
    });
    const chat = await rowFor('Plot review');
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    const confirm = within(chat).getByRole('group', {
      name: 'Stop “Plot review” reading and posting in #general?',
    });
    fireEvent.click(within(confirm).getByRole('button', { name: accessCopy.confirmRevoke }));

    await waitFor(() =>
      expect(mocks.crewHttp).toHaveBeenCalledWith(
        '/connections/conn-1/sessions/agent-1/revoke',
        'POST'
      )
    );
    expect(await screen.findByText(accessCopy.revoked('Plot review'))).toBeInTheDocument();
    // The row moved behind the disclosure: three revoked or expired now.
    expect(await screen.findByRole('button', { name: accessCopy.showOld(3) })).toBeInTheDocument();
  });

  it('says a 503 stopped only on this device and keeps the row in view with Retry', async () => {
    const state = { revoked: [] as string[] };
    setup({
      grants: () => workspaceGrants(state)(),
      revoke: (sessionId) => {
        state.revoked.push(sessionId);
        throw new CrewHttpError('Stopped here.', 503, 'crew_revocation_unconfirmed');
      },
    });
    const chat = await rowFor('Plot review');
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    fireEvent.click(within(chat).getByRole('button', { name: accessCopy.confirmRevoke }));

    expect(await screen.findByText(accessCopy.unconfirmed)).toBeInTheDocument();
    expect(screen.queryByText(/Access revoked/)).toBeNull();
    const stopped = await waitFor(async () => {
      const row = await rowFor('Plot review');
      expect(row).toHaveAttribute('data-access-status', 'unconfirmed');
      return row;
    });
    expect(stopped).toHaveTextContent(accessCopy.status.unconfirmed);
    expect(
      within(stopped).getByRole('button', { name: 'Retry revoking Plot review' })
    ).toBeInTheDocument();
  });

  it('says a refused revoke was not revoked, in the daemon’s words', async () => {
    setup({
      revoke: () => {
        throw new CrewHttpError('No Crew grant for this session', 404, 'crew_grant_not_found');
      },
    });
    const chat = await rowFor('Plot review');
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    fireEvent.click(within(chat).getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      `${accessCopy.notRevoked} No Crew grant for this session`
    );
    expect(screen.queryByText(/Access revoked/)).toBeNull();
  });

  it('stops a task through the existing cancel route after asking', async () => {
    setup({});
    const task = await rowFor(accessCopy.yourTask);
    fireEvent.click(within(task).getByRole('button', { name: accessCopy.stopRowName }));
    const confirm = within(task).getByRole('group', { name: accessCopy.stopConfirm });
    expect(confirm).toHaveTextContent(accessCopy.stopConfirmBody);
    expect(callsTo(mocks, '/connections/conn-1/runs/run-2/cancel', 'POST')).toHaveLength(0);
    fireEvent.click(within(confirm).getByRole('button', { name: accessCopy.stopConfirmAction }));
    await waitFor(() =>
      expect(callsTo(mocks, '/connections/conn-1/runs/run-2/cancel', 'POST')).toHaveLength(1)
    );
    expect(callsTo(mocks, '/connections/conn-1/sessions/task-1/revoke', 'POST')).toHaveLength(0);
  });

  it('opens a row’s conversation', async () => {
    setup({});
    fireEvent.click(
      within(await rowFor('Plot review')).getByRole('button', { name: 'Open Plot review' })
    );
    expect(mocks.navigate).toHaveBeenCalledWith('/pair?resumeSessionId=agent-1');
    fireEvent.click(
      within(await rowFor(accessCopy.yourTask)).getByRole('button', {
        name: accessCopy.openTaskName,
      })
    );
    expect(mocks.navigate).toHaveBeenCalledWith('/pair?resumeSessionId=task-1');
  });

  /**
   * The 360px pane squeezed a one-line row until the title and destination were zero wide and `⋯`
   * was clipped. The row now wraps: the text block keeps a floor, and the badge and the actions
   * move as ONE end group onto their own line. That only works if they are siblings in the right
   * containers (flex-wrap acts on direct children), so the structure is what is asserted here.
   * jsdom computes no layout and loads no stylesheet, so a width check here would prove nothing;
   * the wrapping itself is `.crew-access-row-*` in `crew-app.css`.
   */
  it('keeps a row’s badge and actions together in one end group that can wrap below the text', async () => {
    setup({});
    const chat = await rowFor('Plot review');
    const line = chat.querySelector('.crew-access-row-line');
    const text = chat.querySelector('.crew-access-row-text');
    const end = chat.querySelector('.crew-access-row-end');
    const actions = chat.querySelector('.crew-access-row-actions');
    if (!line || !text || !end || !actions) throw new Error('an access row part is missing');

    // The line's direct children: the icon, the text block, then the end group, last.
    expect(text.parentElement).toBe(line);
    expect(end.parentElement).toBe(line);
    expect(line.lastElementChild).toBe(end);
    expect(text).toHaveTextContent('Plot review');
    expect(text).toHaveTextContent('#general');
    expect(end).not.toHaveTextContent('Plot review');

    // The end group holds the badge first, then the actions, so they wrap as one.
    expect(end.children).toHaveLength(2);
    expect(end.firstElementChild?.textContent).toMatch(/^(?:Active|Expires )/);
    expect(end.lastElementChild).toBe(actions);

    // Accessible names and their order are unchanged, `⋯` included, and all in the end group.
    const names = within(actions as HTMLElement)
      .getAllByRole('button')
      .map((button) => button.getAttribute('aria-label'));
    expect(names).toEqual([
      accessCopy.openName('Plot review'),
      accessCopy.revokeRowName('Plot review'),
      accessCopy.rowMore('Plot review'),
    ]);
    expect(within(chat).getAllByRole('button')).toHaveLength(3);

    const task = await rowFor(accessCopy.yourTask);
    const taskActions = task.querySelector('.crew-access-row-end > .crew-access-row-actions');
    if (!taskActions) throw new Error('the task row has no end group');
    expect(
      within(taskActions as HTMLElement)
        .getAllByRole('button')
        .map((button) => button.getAttribute('aria-label'))
    ).toEqual([
      accessCopy.openTaskName,
      accessCopy.stopRowName,
      accessCopy.rowMore(accessCopy.yourTask),
    ]);

    // While a revoke is confirmed inline, the badge stays in the end group and the actions leave.
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    expect(chat.querySelector('.crew-access-row-actions')).toBeNull();
    expect(chat.querySelector('.crew-access-row-end')?.children).toHaveLength(1);
  });

  it('copies a session ID from the row’s menu, and only there', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    setup({});
    const chat = await rowFor('Plot review');
    const more = within(chat).getByRole('button', { name: 'More actions for Plot review' });
    fireEvent.pointerDown(more, { button: 0, ctrlKey: false });
    fireEvent.click(await screen.findByRole('menuitem', { name: accessCopy.copySessionId }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('agent-1'));
    expect(await screen.findByText(accessCopy.copiedSessionId)).toBeInTheDocument();
  });

  it('shows an empty channel, and a failed list with Retry', async () => {
    let fail = true;
    setup({
      grants: () => [],
      listFailure: () => (fail ? new CrewHttpError('boom', 500) : null),
    });
    expect(await screen.findByText(accessCopy.listFailed)).toBeInTheDocument();
    fail = false;
    fireEvent.click(screen.getByRole('button', { name: accessCopy.listRetryName }));
    expect(await screen.findByText(accessCopy.empty('#general'))).toBeInTheDocument();
    expect(screen.getByText(accessCopy.emptyHow)).toBeInTheDocument();
  });
});

describe('Workspace settings → Agent access', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('lists every channel’s chats and tasks', async () => {
    setup({}, WorkspaceLayout);
    const methods = await rowFor('Methods chat');
    expect(methods).toHaveTextContent('#methods');
    expect(
      within(methods).getByRole('button', { name: 'Revoke access for Methods chat' })
    ).toBeInTheDocument();
    expect(await rowFor('Plot review')).toBeInTheDocument();
    expect(await rowFor(accessCopy.yourTask)).toBeInTheDocument();
  });

  it('names the workspace when nothing has access', async () => {
    setup({ grants: () => [] }, WorkspaceLayout);
    expect(await screen.findByText(accessCopy.emptyWorkspace('lab'))).toBeInTheDocument();
  });
});
