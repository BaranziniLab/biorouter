import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
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
import { pastAccessStorageKey, readPastAccess, rememberPastAccess } from './pastAccess';
import {
  announceGrantsChanged,
  forgetUnconfirmedRevocations,
  UNCONFIRMED_REVOKE_WATCH_MS,
} from './useCrewGrants';
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

/** The channel's Access tab and Workspace settings' Agent access, side by side. */
function BothLayout() {
  const { channel } = useCrew();
  return channel ? (
    <>
      <AccessTab />
      <WorkspaceAgentAccess />
    </>
  ) : null;
}

/** The rows under one surface's "Show past access (n)", opened. */
async function pastRows(surface: HTMLElement, count: number) {
  fireEvent.click(await within(surface).findByRole('button', { name: accessCopy.showOld(count) }));
  const list = within(surface).getByRole('list', { name: accessCopy.oldListName });
  return within(list).getAllByTestId('crew-access-row');
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
    // Past access is remembered in localStorage (Q4-12): each test starts with none.
    window.localStorage.clear();
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
    // Q3-30: one name for rows that read Ended, Revoked or Expired.
    expect(accessCopy.showOld(2)).toBe('Show past access (2)');
    fireEvent.click(screen.getByRole('button', { name: accessCopy.showOld(2) }));
    expect(screen.getByRole('list', { name: 'Past access' })).toBeInTheDocument();
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

    // Its connection is up: the daemon is confirming it by itself (F3), not waiting on a person.
    expect(await screen.findByText(accessCopy.confirming)).toBeInTheDocument();
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

  it('follows a 503 to “Confirmed” once the daemon confirms it with the workspace by itself (F3)', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const state = { revoked: [] as string[], standing: 'unconfirmed' };
      setup({
        grants: () =>
          workspaceGrants(state)().map((grant) =>
            grant.session_id === 'agent-1' && grant.expired
              ? { ...grant, revocation: state.standing }
              : grant
          ),
        revoke: (sessionId) => {
          state.revoked.push(sessionId);
          throw new CrewHttpError('Stopped here.', 503, 'crew_revocation_unconfirmed');
        },
      });
      const chat = await rowFor('Plot review');
      fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
      fireEvent.click(within(chat).getByRole('button', { name: accessCopy.confirmRevoke }));
      expect(await screen.findByText(accessCopy.confirming)).toBeInTheDocument();

      // The daemon's own retry reached the workspace; no one clicks anything.
      state.standing = 'confirmed';
      await act(async () => {
        await vi.advanceTimersByTimeAsync(UNCONFIRMED_REVOKE_WATCH_MS);
      });
      expect(await screen.findByText(accessCopy.confirmed)).toBeInTheDocument();
      expect(screen.queryByText(accessCopy.confirming)).toBeNull();
      expect(screen.queryByTestId('crew-access-unconfirmed')).toBeNull();

      // It stays until the person dismisses it (NEW-4), then goes for good.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2 * UNCONFIRMED_REVOKE_WATCH_MS);
      });
      const note = screen.getByTestId('crew-access-confirmed');
      fireEvent.click(within(note).getByRole('button', { name: accessCopy.confirmedDismissName }));
      await waitFor(() => expect(screen.queryByTestId('crew-access-confirmed')).toBeNull());
    } finally {
      vi.useRealTimers();
    }
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
   * The 360px pane squeezed a one-line row until the title and destination were zero wide and its
   * actions were clipped. The row now wraps: the text block keeps a floor, and the badge and the actions
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
    // One wording for an active grant, with its end time: never "Expires …" in one place and
    // "Active" in another (T-55).
    expect(end.firstElementChild?.textContent).toMatch(/^Active · ends \S/);
    expect(end.lastElementChild).toBe(actions);

    // Accessible names and their order, all in the end group.
    const names = within(actions as HTMLElement)
      .getAllByRole('button')
      .map((button) => button.getAttribute('aria-label'));
    expect(names).toEqual([
      accessCopy.openName('Plot review'),
      accessCopy.revokeRowName('Plot review'),
    ]);
    expect(within(chat).getAllByRole('button')).toHaveLength(2);

    const task = await rowFor(accessCopy.yourTask);
    const taskActions = task.querySelector('.crew-access-row-end > .crew-access-row-actions');
    if (!taskActions) throw new Error('the task row has no end group');
    expect(
      within(taskActions as HTMLElement)
        .getAllByRole('button')
        .map((button) => button.getAttribute('aria-label'))
    ).toEqual([accessCopy.openTaskName, accessCopy.stopRowName]);

    // While a revoke is confirmed inline, the badge stays in the end group and the actions leave.
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    expect(chat.querySelector('.crew-access-row-actions')).toBeNull();
    expect(chat.querySelector('.crew-access-row-end')?.children).toHaveLength(1);
  });

  /**
   * T-55: the only item a row's `⋯` held was "Copy session ID" — a machine ID of no use to the
   * person reading the list — so a row offers no menu at all, current or revoked.
   */
  it('offers no ⋯ menu whose only item would be a session ID', async () => {
    setup({});
    const chat = await rowFor('Plot review');
    expect(within(chat).queryByRole('button', { name: /^More actions/ })).toBeNull();
    expect(
      within(await rowFor(accessCopy.yourTask)).queryByRole('button', { name: /^More/ })
    ).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: accessCopy.showOld(2) }));
    const old = await rowFor('Old chat');
    expect(
      within(old)
        .getAllByRole('button')
        .map((button) => button.getAttribute('aria-label'))
    ).toEqual([accessCopy.openName('Old chat')]);
    expect(screen.queryByText(/session ID/i)).toBeNull();
    expect(screen.queryByRole('menuitem')).toBeNull();
  });

  /**
   * Q3-30 (live QA round 3): the details pane's Agent access tab opened on a heading that said
   * "Agent access" again. The tab names its panel; the list has no heading of its own.
   */
  it('has no heading of its own: the Agent access tab names it', async () => {
    setup({});
    const tab = await screen.findByTestId('crew-access-tab');
    await rowFor('Plot review');
    expect(within(tab).queryByRole('heading')).toBeNull();
    expect(screen.queryByRole('region', { name: 'Agent access' })).toBeNull();
    expect(tab).not.toHaveTextContent(/^Agent access/);
    // The name every surface uses for this list is still the tab's and Workspace settings'.
    expect(accessCopy.tabTitle).toBe('Agent access');
  });

  it('keeps the heading in Workspace settings, whose panel hides it and is named by it', async () => {
    setup({}, WorkspaceLayout);
    expect(await screen.findByRole('region', { name: 'Agent access' })).toBeInTheDocument();
  });

  /**
   * Q4-15 (live QA round 4): the confirmation said the chat would stop reading and posting here,
   * not that the whole chat stops.
   */
  it('says the chat stops until access is granted again, in the revoke question', async () => {
    setup({});
    const chat = await rowFor('Plot review');
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    const confirm = within(chat).getByRole('group', {
      name: 'Stop “Plot review” reading and posting in #general?',
    });
    expect(confirm).toHaveTextContent(accessCopy.confirmStops);
  });

  /**
   * Q4-12 (live QA round 4): the daemon lists one grant per chat, so after a revoke and a new
   * grant "Show past access" no longer listed the revoked one (Jack J6, Gina F9). This device
   * remembers what it saw revoked.
   */
  it('keeps a revoked grant under past access after the chat is granted again', async () => {
    const state = { run: 'run-1', revoked: false };
    setup({
      grants: () => [
        grantRow({ run_id: state.run, expired: state.revoked }),
        ...workspaceGrants()().slice(1),
      ],
      revoke: (sessionId) => {
        state.revoked = true;
        return {
          revoked: true,
          remote_revocation_confirmed: true,
          session_id: sessionId,
          run_id: 'run-1',
        };
      },
    });
    const chat = await rowFor('Plot review');
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    fireEvent.click(within(chat).getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await screen.findByText(accessCopy.revoked('Plot review'))).toBeInTheDocument();
    // The daemon still lists the revoked run: one row for it, not two.
    expect(await screen.findByRole('button', { name: accessCopy.showOld(3) })).toBeInTheDocument();
    expect(readPastAccess('conn-1')).toEqual([
      expect.objectContaining({
        session_id: 'agent-1',
        run_id: 'run-1',
        session_name: 'Plot review',
        channel_id: 'channel-1',
      }),
    ]);

    // Granted again: the daemon's list now holds the new run in the chat's one row.
    state.run = 'run-2';
    state.revoked = false;
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'agent-1', change: 'granted' })
    );
    await waitFor(async () =>
      expect(await rowFor('Plot review')).toHaveAttribute('data-access-status', 'active')
    );
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.showOld(3) }));
    const pastList = screen.getByRole('list', { name: accessCopy.oldListName });
    const revoked = within(pastList)
      .getAllByTestId('crew-access-row')
      .find((row) => within(row).queryByText('Plot review'));
    expect(revoked, 'the remembered Plot review row').toBeDefined();
    expect(revoked).toHaveAttribute('data-access-status', 'revoked');
    expect(revoked).toHaveTextContent(accessCopy.status.revoked);
    expect(revoked).toHaveTextContent('#general');
    expect(within(revoked as HTMLElement).queryByRole('button', { name: /^Revoke/ })).toBeNull();
    // Display only: nothing but the page reads the record, and it holds no more than it shows.
    expect(window.localStorage.getItem(pastAccessStorageKey('conn-1'))).not.toMatch(/secret|key/i);
  });

  /**
   * Q4-29 (live QA round 4): the pane's tabs did not share a left edge — About's words start at
   * the tab panel's own padding, Agent access's 12px further in (Carol R4-6). jsdom lays nothing
   * out, so the classes that put the words on that edge are what is asserted.
   */
  it('starts its words at the tab panel’s own edge, as About does', async () => {
    setup({});
    const chat = await rowFor('Plot review');
    expect(chat).toHaveClass('px-1');
    expect(chat).not.toHaveClass('px-3');
    // The row steps out by the 4px it pads back in, as About's rows do: only its hover wash
    // reaches past the edge.
    expect(chat.parentElement).toHaveClass('biorouter-list-shell', '-mx-1');
  });

  it('keeps Workspace settings’ inset rows', async () => {
    setup({}, WorkspaceLayout);
    const chat = await rowFor('Plot review');
    expect(chat).toHaveClass('px-3');
    expect(chat.parentElement).not.toHaveClass('-mx-1');
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
    // Q3-29: the list holds this computer's own chats, so it says whose — never that no chat or
    // agent at all can post, which is false wherever another person's agent does.
    expect(
      await screen.findByText('None of your chats can post in #general yet.')
    ).toBeInTheDocument();
    expect(screen.getByText(accessCopy.empty('#general'))).toBeInTheDocument();
    expect(screen.queryByText(/No chats or agents/)).toBeNull();
    const how = screen.getByText((_, element) =>
      Boolean(
        element?.tagName === 'P' &&
        element.textContent === 'To connect one, open that chat and type /crew.'
      )
    );
    // The command is drawn as something to type.
    expect(within(how).getByText('/crew').tagName).toBe('CODE');
    // Q4-29: the empty state starts at the panel's edge too.
    expect(how.parentElement).not.toHaveClass('px-3');
  });
});

/**
 * Q2-09 and Q2-74 (live QA round 2): two finished tasks were listed as "Your task · #general ·
 * Revoked" twice over — a task that did its work read as revoked, and the rows could not be told
 * apart. They read "Ended", with when each started and its first words.
 */
describe('the Access tab’s finished tasks', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    // Past access is remembered in localStorage (Q4-12): each test starts with none.
    window.localStorage.clear();
  });

  const hourAgo = () => Math.floor(Date.now() / 1000) - 3600;

  it('reads each ended task as Ended, told apart by its time and first words', async () => {
    setup({
      runs: [],
      grants: () => [
        grantRow({
          session_id: 'task-sums',
          run_id: 'run-sums',
          kind: 'task',
          expired: true,
          session_name: 'Crew · #general · Please work out the sum and the average of each…',
          expires_at: hourAgo() + 600,
        }),
        grantRow({
          session_id: 'task-plot',
          run_id: 'run-plot',
          kind: 'task',
          expired: true,
          session_name: 'Crew · #general · Plot the growth curves',
          expires_at: hourAgo() + 1800,
        }),
      ],
    });
    fireEvent.click(await screen.findByRole('button', { name: accessCopy.showOld(2) }));
    const rows = await screen.findAllByTestId('crew-access-row');
    const texts = rows.map((row) => row.textContent ?? '');
    expect(texts).toHaveLength(2);
    expect(texts[0]).not.toBe(texts[1]);
    const sums = rows.find((row) => row.textContent?.includes('Please work out…'));
    const plot = rows.find((row) => row.textContent?.includes('Plot the growth…'));
    expect(sums, 'the sums task').toBeDefined();
    expect(plot, 'the plot task').toBeDefined();
    for (const row of [sums, plot]) {
      if (!row) continue;
      expect(row).toHaveTextContent(/^Your task · \S+.* · /);
      expect(row).toHaveTextContent(accessCopy.status.ended);
      expect(row).not.toHaveTextContent(accessCopy.status.revoked);
      expect(row).not.toHaveTextContent(/run-|task-/);
    }
  });
});

describe('Workspace settings → Agent access', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    // Past access is remembered in localStorage (Q4-12): each test starts with none.
    window.localStorage.clear();
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

  /**
   * Q4-12 (live QA round 4), in Workspace settings too: its list is built on the same rows as the
   * Access tab, so a chat revoked and granted again keeps its revoked row here as well.
   */
  it('keeps a revoked grant under past access after the chat is granted again', async () => {
    const state = { run: 'run-1', revoked: false };
    setup(
      {
        grants: () => [
          grantRow({ run_id: state.run, expired: state.revoked }),
          ...workspaceGrants()().slice(1),
        ],
        revoke: (sessionId) => {
          state.revoked = true;
          return {
            revoked: true,
            remote_revocation_confirmed: true,
            session_id: sessionId,
            run_id: 'run-1',
          };
        },
      },
      WorkspaceLayout
    );
    const chat = await rowFor('Plot review');
    fireEvent.click(within(chat).getByRole('button', { name: 'Revoke access for Plot review' }));
    fireEvent.click(within(chat).getByRole('button', { name: accessCopy.confirmRevoke }));
    expect(await screen.findByText(accessCopy.revoked('Plot review'))).toBeInTheDocument();
    // The daemon still lists the revoked run: one row for it, not two.
    expect(await screen.findByRole('button', { name: accessCopy.showOld(3) })).toBeInTheDocument();
    expect(readPastAccess('conn-1')).toEqual([
      expect.objectContaining({ session_id: 'agent-1', run_id: 'run-1', channel_id: 'channel-1' }),
    ]);

    state.run = 'run-2';
    state.revoked = false;
    act(() =>
      announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'agent-1', change: 'granted' })
    );
    await waitFor(async () =>
      expect(await rowFor('Plot review')).toHaveAttribute('data-access-status', 'active')
    );
    const rows = await pastRows(screen.getByTestId('crew-workspace-agent-access'), 3);
    const revoked = rows.find((row) => within(row).queryByText('Plot review'));
    expect(revoked, 'the remembered Plot review row').toBeDefined();
    expect(revoked).toHaveAttribute('data-access-status', 'revoked');
    expect(revoked).toHaveTextContent(accessCopy.status.revoked);
    expect(revoked).toHaveTextContent('#general');
    expect(within(revoked as HTMLElement).queryByRole('button', { name: /^Revoke/ })).toBeNull();
  });

  /**
   * The finding behind round 2 of the Q4-12 fix: the channel's tab showed a remembered revoke and
   * Workspace settings did not, so the two lists disagreed about the same chat. Workspace settings
   * has no channel filter, so it also holds the remembered revokes of other channels.
   */
  it('agrees with the channel’s Access tab about remembered revokes', async () => {
    const revokedAt = Date.now() - 60_000;
    // Revoked on this device earlier, then granted again: the daemon lists the newer runs.
    rememberPastAccess('conn-1', {
      session_id: 'agent-1',
      run_id: 'run-0',
      session_name: 'Plot review',
      channel_id: 'channel-1',
      revoked_at: revokedAt,
    });
    rememberPastAccess('conn-1', {
      session_id: 'methods-chat',
      run_id: 'run-9',
      session_name: 'Methods chat',
      channel_id: 'channel-2',
      source_channels: ['channel-2'],
      revoked_at: revokedAt,
    });
    setup({}, BothLayout);
    const tab = await screen.findByTestId('crew-access-tab');
    const workspace = screen.getByTestId('crew-workspace-agent-access');

    const inTab = await pastRows(tab, 3);
    const inWorkspace = await pastRows(workspace, 4);
    const remembered = (rows: HTMLElement[], title: string) =>
      rows.find(
        (row) =>
          within(row).queryByText(title) && row.getAttribute('data-access-status') === 'revoked'
      );
    for (const rows of [inTab, inWorkspace]) {
      const plot = remembered(rows, 'Plot review');
      expect(plot, 'the remembered Plot review row').toBeDefined();
      expect(plot).toHaveTextContent('#general');
    }
    // Every row the channel's tab holds under past access, Workspace settings holds too.
    const titles = (rows: HTMLElement[]) => rows.map((row) => row.textContent);
    expect(titles(inWorkspace)).toEqual(expect.arrayContaining(titles(inTab)));
    // #methods' remembered revoke is outside this channel, and only there.
    expect(remembered(inTab, 'Methods chat')).toBeUndefined();
    expect(remembered(inWorkspace, 'Methods chat')).toHaveTextContent('#methods');
  });

  it('names the workspace when nothing has access', async () => {
    setup({ grants: () => [] }, WorkspaceLayout);
    expect(await screen.findByText(accessCopy.emptyWorkspace('lab'))).toBeInTheDocument();
    expect(screen.getByText('None of your chats can post in lab yet.')).toBeInTheDocument();
  });
});
