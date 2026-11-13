import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { openCopy, setupCopy, trustCopy, welcomeCopy } from './copy';
import { readJoinContext, resetJoinContextForTests } from './joinContext';
import { OpenWorkspaceDialog } from './OpenWorkspaceDialog';
import type { SetupInput, SetupRun, SetupWorkspace } from './setup';
import { fakeConnection, makeCrew, renderWithCrew } from './testCrew';
import { Welcome } from './Welcome';

/**
 * Open a workspace on a server (account ownership, "Desktop"; AO-6, AO-8, AO-9): the login, a check
 * job on the server target with the device it prepared, the workspaces found with their host,
 * role and state, and Open, Start and Update Crew.
 */

const mocks = vi.hoisted(() => ({
  startSetup: vi.fn(),
  readSetup: vi.fn(),
  stopSetup: vi.fn(),
  saveFromInvitation: vi.fn(),
  setupSupport: {
    supported: true as boolean | null,
    artifact: { version: '1.95.0' } as unknown,
  },
}));

vi.mock('./setup', async () => {
  const actual = await vi.importActual<typeof import('./setup')>('./setup');
  return {
    ...actual,
    startSetup: mocks.startSetup,
    readSetup: mocks.readSetup,
    stopSetup: mocks.stopSetup,
  };
});
vi.mock('./SetupProgress', async () => {
  const actual = await vi.importActual<typeof import('./SetupProgress')>('./SetupProgress');
  return { ...actual, useSetupArtifact: () => mocks.setupSupport };
});
vi.mock('../api/join', async () => {
  const actual = await vi.importActual<typeof import('../api/join')>('../api/join');
  return { ...actual, saveFromInvitation: mocks.saveFromInvitation };
});
vi.mock('../auth/SignInDialog', () => ({ SignInDialog: () => null }));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
beforeAll(() => vi.stubGlobal('ResizeObserver', ResizeObserverStub));
afterAll(() => vi.unstubAllGlobals());

const LOGIN = 'wanjun@leo.ucsf.edu';

function entry(overrides: Partial<SetupWorkspace> = {}): SetupWorkspace {
  return {
    workspaceId: '11111111-2222-4333-8444-555555555555',
    name: 'baranzini-lab',
    role: 'member',
    state: 'running',
    hostUsername: 'wanjun',
    hostUid: 1000,
    fingerprint: 'abcd',
    mode: 'private',
    institutionId: 'ucsf',
    version: '1.95.0',
    binaryCurrent: true,
    othersConnected: null,
    invitation: 'brcrew1:abc',
    savedConnectionId: null,
    thisComputer: 'not_saved',
    error: null,
    ...overrides,
  };
}

function finished(
  workspaces: SetupWorkspace[],
  outcome: Partial<SetupRun['outcome']> = {}
): SetupRun {
  return {
    jobId: 'job-1',
    intent: 'check',
    state: 'finished',
    stage: 'done',
    stages: [
      { name: 'check', state: 'done', detail: null },
      { name: 'workspaces', state: 'done', detail: null },
      { name: 'done', state: 'done', detail: null },
    ],
    server: { login: LOGIN, user: 'wanjun' },
    artifact: null,
    outcome: {
      installedVersion: null,
      reused: true,
      previousKept: false,
      replacedVersion: null,
      host: null,
      hostCommand: null,
      connectionId: null,
      workFolder: null,
      remoteAccess: null,
      contextDraft: null,
      ...outcome,
    },
    error: null,
    output: '',
    workspaces,
    workspacesNote: null,
  };
}

function renderDialog(crewOverrides = {}) {
  const crew = makeCrew({
    ui: { dialog: { kind: 'open-workspace' }, pane: null },
    prepareHostingDevice: vi.fn().mockResolvedValue({
      device_id: 'device-new',
      preparation_id: 'prep-1',
      public_key: 'e'.repeat(64),
    }),
    ...crewOverrides,
  });
  const rendered = renderWithCrew(<OpenWorkspaceDialog />, crew);
  return { ...rendered, crew };
}

async function look(login = LOGIN) {
  const dialog = await screen.findByRole('dialog', { name: openCopy.title });
  fireEvent.change(within(dialog).getByLabelText(openCopy.login), { target: { value: login } });
  fireEvent.click(within(dialog).getByRole('button', { name: openCopy.look }));
  return dialog;
}

/**
 * What a row's Update Crew would do (`restartsOnUpdate`): behind the info icon beside it, and the
 * button's own description, never a line under the row (design.md 9.16).
 */
function expectUpdateHelp(scope: HTMLElement, workspace: string, help: string) {
  const label = openCopy.updateLabel(workspace);
  expect(within(scope).getByRole('button', { name: label })).toHaveAccessibleDescription(help);
  expect(within(scope).getByRole('button', { name: `About ${label}` })).toHaveAccessibleDescription(
    help
  );
  expect(within(scope).queryByText(help, { selector: 'p' })).toBeNull();
}

beforeEach(() => {
  mocks.startSetup.mockReset();
  mocks.readSetup.mockReset();
  mocks.stopSetup.mockReset();
  mocks.saveFromInvitation.mockReset();
  resetJoinContextForTests();
});

describe('Open a workspace on a server', () => {
  it('is offered on the welcome screen', () => {
    const crew = makeCrew();
    renderWithCrew(<Welcome />, crew);
    fireEvent.click(screen.getByRole('button', { name: welcomeCopy.open }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'open-workspace' });
  });

  it('checks the server with a device it prepared, then lists the workspaces with their host', async () => {
    mocks.startSetup.mockResolvedValue(
      finished([
        entry(),
        entry({
          workspaceId: '22222222-2222-4333-8444-555555555555',
          name: 'old-lab',
          role: 'removed',
          hostUsername: 'alice',
        }),
      ])
    );
    const { crew } = renderDialog();
    const dialog = await look();
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(1));
    expect(crew.prepareHostingDevice).toHaveBeenCalledTimes(1);
    expect(mocks.startSetup.mock.calls[0][0]).toEqual<SetupInput>({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'check',
    });
    const found = await within(dialog).findByTestId('crew-open-found');
    // AO-6's trust rule: every workspace offered names its host.
    expect(within(found).getByText(openCopy.hostedBy('baranzini-lab', 'wanjun'))).toBeVisible();
    expect(within(found).getByText(/You’re a member · Running · Crew 1\.95\.0/)).toBeVisible();
    expect(
      within(found).getByRole('button', {
        name: openCopy.openLabel(openCopy.hostedBy('baranzini-lab', 'wanjun')),
      })
    ).toBeVisible();
    // A removed account reads AO-2's sentence and has nothing to open.
    expect(within(found).getByText(openCopy.removed('old-lab'))).toBeVisible();
    expect(
      within(found).queryByRole('button', {
        name: openCopy.openLabel(openCopy.hostedBy('old-lab', 'alice')),
      })
    ).toBeNull();
    expect(within(dialog).getByText(setupCopy.stages.finding)).toBeVisible();
  });

  it('opens one by saving its invitation with the typed login and the prepared device, then connects', async () => {
    mocks.startSetup.mockResolvedValue(finished([entry()]));
    const saved = fakeConnection({
      id: 'conn-new',
      status: 'disconnected',
      account_identity: true,
    });
    mocks.saveFromInvitation.mockResolvedValue(saved);
    const rendered = renderDialog();
    const dialog = await look();
    const open = await within(dialog).findByRole('button', {
      name: openCopy.openLabel(openCopy.hostedBy('baranzini-lab', 'wanjun')),
    });
    fireEvent.click(open);
    await waitFor(() => expect(mocks.saveFromInvitation).toHaveBeenCalledTimes(1));
    expect(mocks.saveFromInvitation).toHaveBeenCalledWith('brcrew1:abc', {
      username: 'wanjun',
      mode: 'private',
      institution_id: 'ucsf',
      advanced: { ssh_target: LOGIN, preparation_id: 'prep-1' },
    });
    await waitFor(() => expect(rendered.crew.selectConnection).toHaveBeenCalledWith('conn-new'));
    // The person asked for it: the join screen joins by account at once.
    expect(readJoinContext('conn-new')).toMatchObject({
      joining: true,
      workspaceName: 'baranzini-lab',
    });
    rendered.update({ connectionId: 'conn-new', connection: saved, connections: [saved] });
    await waitFor(() =>
      expect(rendered.crew.connect).toHaveBeenCalledWith({ userInitiated: true })
    );
    await waitFor(() => expect(rendered.crew.closeDialog).toHaveBeenCalled());
  });

  it('starts a stopped workspace the account hosts', async () => {
    mocks.startSetup.mockResolvedValue(finished([entry({ role: 'host', state: 'stopped' })]));
    renderDialog();
    const dialog = await look();
    const start = await within(dialog).findByRole('button', {
      name: openCopy.startLabel('baranzini-lab'),
    });
    fireEvent.click(start);
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      // Starting one workspace restarts no other.
      restart_workspaces: false,
      start_workspaces: ['11111111-2222-4333-8444-555555555555'],
    });
  });

  it('asks before updating Crew while other people are connected', async () => {
    mocks.startSetup.mockResolvedValue(
      finished([
        entry({ role: 'host', binaryCurrent: false, version: '1.94.1', othersConnected: 3 }),
      ])
    );
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('baranzini-lab') })
    );
    const ask = await within(dialog).findByTestId('crew-open-update-confirm');
    // The people besides you, as the command line and the chat count them.
    expect(ask).toHaveTextContent(
      '3 other people are connected. Their running agent commands will stop.'
    );
    expect(mocks.startSetup).toHaveBeenCalledTimes(1);
    fireEvent.click(within(ask).getByRole('button', { name: openCopy.updateAnyway }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    // The consent is for this workspace alone: its ID, and no restart of any other.
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      restart_workspaces: false,
      restart_workspace_ids: ['11111111-2222-4333-8444-555555555555'],
    });
  });

  it('asks first when the server cannot say who is connected, as a Crew from before account ownership', async () => {
    // The leo case: an older broker names no version and no count (findings 1 and 3).
    mocks.startSetup.mockResolvedValue(
      finished([
        entry({ role: 'host', version: null, binaryCurrent: false, othersConnected: null }),
      ])
    );
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('baranzini-lab') })
    );
    const ask = await within(dialog).findByTestId('crew-open-update-confirm');
    expect(ask).toHaveTextContent(
      'Other people may be connected. Their running agent commands will stop.'
    );
    expect(mocks.startSetup).toHaveBeenCalledTimes(1);
    fireEvent.click(within(ask).getByRole('button', { name: openCopy.updateAnyway }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      restart_workspaces: false,
      restart_workspace_ids: ['11111111-2222-4333-8444-555555555555'],
    });
  });

  it('restarts only the workspace whose Update Crew was confirmed (findings 7, 14 and 18)', async () => {
    const labA = entry({
      role: 'host',
      name: 'lab-a',
      binaryCurrent: false,
      version: '1.94.1',
      othersConnected: 1,
    });
    const labB = entry({
      workspaceId: '22222222-2222-4333-8444-555555555555',
      role: 'host',
      name: 'lab-b',
      binaryCurrent: false,
      version: '1.94.1',
      othersConnected: 5,
    });
    mocks.startSetup.mockResolvedValue(finished([labA, labB]));
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('lab-a') })
    );
    const ask = await within(dialog).findByTestId('crew-open-update-confirm');
    expect(ask).toHaveTextContent('1 other person is connected.');
    fireEvent.click(within(ask).getByRole('button', { name: openCopy.updateAnyway }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    const sent = mocks.startSetup.mock.calls[1][0] as SetupInput;
    expect(sent.restart_workspace_ids).toEqual([labA.workspaceId]);
    expect(sent.restart_workspaces).toBe(false);
  });

  it('says why an update left a workspace running, and offers no update that cannot help', async () => {
    const custom = entry({
      role: 'host',
      name: 'tmp-lab',
      binaryCurrent: false,
      othersConnected: 0,
    });
    const busy = entry({
      workspaceId: '22222222-2222-4333-8444-555555555555',
      role: 'host',
      name: 'busy-lab',
      binaryCurrent: false,
      othersConnected: null,
    });
    const quiet = entry({
      workspaceId: '33333333-2222-4333-8444-555555555555',
      role: 'host',
      name: 'quiet-lab',
      binaryCurrent: false,
      othersConnected: 0,
    });
    mocks.startSetup.mockResolvedValueOnce(finished([custom, busy, quiet])).mockResolvedValueOnce({
      ...finished([custom, busy, quiet], {
        restarted: [
          {
            workspaceId: quiet.workspaceId,
            name: 'quiet-lab',
            fromVersion: '1.94.1',
            toVersion: '1.95.0',
          },
        ],
        outdated: [
          {
            workspaceId: custom.workspaceId,
            name: 'tmp-lab',
            runningVersion: '1.94.1',
            installedVersion: '1.95.0',
            othersConnected: 0,
            reason: 'custom_state_dir',
          },
          {
            workspaceId: busy.workspaceId,
            name: 'busy-lab',
            runningVersion: '1.94.1',
            installedVersion: '1.95.0',
            othersConnected: null,
            reason: 'others_connected',
          },
        ],
      }),
      intent: 'install',
    });
    renderDialog();
    const dialog = await look();
    // The update of one workspace, which left the others running.
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('quiet-lab') })
    );
    expect(
      await within(dialog).findByText(openCopy.restarted('quiet-lab', '1.95.0'))
    ).toBeVisible();
    const found = within(dialog).getByTestId('crew-open-found');
    expect(
      within(found).getByText(openCopy.leftRunning.custom_state_dir('leo.ucsf.edu'))
    ).toBeVisible();
    expect(
      within(found).queryByRole('button', { name: openCopy.updateLabel('tmp-lab') })
    ).toBeNull();
    // Only the reason for tmp-lab: "older" would offer what cannot help. busy-lab says the
    // versions the job read: what it runs, and the Crew in the account.
    expect(within(found).getAllByText(/^Runs /)).toHaveLength(1);
    expect(within(found).getByText(openCopy.outdated('1.94.1', '1.95.0'))).toBeVisible();
    expect(openCopy.outdated('1.94.1', '1.95.0')).toBe('Runs Crew 1.94.1, not 1.95.0.');
    // Nobody asked to restart busy-lab: the update was quiet-lab's. It says what Update Crew would
    // do, never that it wasn't restarted.
    expectUpdateHelp(found, 'busy-lab', openCopy.restartsOnUpdate(null));
    expect(within(found).queryByText(/wasn’t restarted/)).toBeNull();
    expect(
      within(found).getByRole('button', { name: openCopy.updateLabel('busy-lab') })
    ).toBeVisible();
  });

  it('says a restart was left only for the row the update was asked to restart (rows decide one by one)', async () => {
    const asked = entry({
      role: 'host',
      name: 'asked-lab',
      binaryCurrent: false,
      othersConnected: 2,
    });
    const other = entry({
      workspaceId: '22222222-2222-4333-8444-555555555555',
      role: 'host',
      name: 'other-lab',
      binaryCurrent: false,
      othersConnected: 3,
    });
    const left = (workspace: SetupWorkspace, othersConnected: number) => ({
      workspaceId: workspace.workspaceId,
      name: workspace.name,
      runningVersion: '1.94.1',
      installedVersion: '1.95.0',
      othersConnected,
      reason: 'others_connected' as const,
    });
    // The update asked for asked-lab alone; by then more people had connected, so the job left
    // it running, and it left other-lab as it found it.
    mocks.startSetup.mockResolvedValueOnce(finished([asked, other])).mockResolvedValueOnce({
      ...finished([asked, other], { outdated: [left(asked, 2), left(other, 3)] }),
      intent: 'install',
    });
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('asked-lab') })
    );
    const ask = await within(dialog).findByTestId('crew-open-update-confirm');
    fireEvent.click(within(ask).getByRole('button', { name: openCopy.updateAnyway }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    expect((mocks.startSetup.mock.calls[1][0] as SetupInput).restart_workspace_ids).toEqual([
      asked.workspaceId,
    ]);
    const found = await within(dialog).findByTestId('crew-open-found');
    const row = (name: string) =>
      Array.from(found.querySelectorAll<HTMLElement>('[data-crew-found-row]')).find((item) =>
        item.textContent?.includes(name)
      ) as HTMLElement;
    await waitFor(() =>
      expect(
        within(row('asked-lab')).getByText(openCopy.leftRunning.others_connected(2))
      ).toBeVisible()
    );
    // other-lab was only looked at: what Update Crew would do, and no restart claimed.
    expectUpdateHelp(row('other-lab'), 'other-lab', openCopy.restartsOnUpdate(3));
    expect(within(row('other-lab')).queryByText(/wasn’t restarted/)).toBeNull();
    // asked-lab says only its reason; the way on is Update Crew itself, with the same help
    // behind its info icon as every row (the reason's old "Choose Update Crew…" sentence).
    expect(
      within(row('asked-lab')).queryByText(openCopy.restartsOnUpdate(2), { selector: 'p' })
    ).toBeNull();
    expectUpdateHelp(row('asked-lab'), 'asked-lab', openCopy.restartsOnUpdate(2));
    expect(within(row('asked-lab')).queryByText(/Choose Update Crew/)).toBeNull();
  });

  it('starting one workspace leaves every other row reading as looked at', async () => {
    const stopped = entry({ role: 'host', name: 'stopped-lab', state: 'stopped' });
    const busy = entry({
      workspaceId: '22222222-2222-4333-8444-555555555555',
      role: 'host',
      name: 'busy-lab',
      binaryCurrent: false,
      othersConnected: null,
    });
    const leftBusy = {
      workspaceId: busy.workspaceId,
      name: 'busy-lab',
      runningVersion: '1.94.1',
      installedVersion: '1.95.0',
      othersConnected: null,
      reason: 'others_connected' as const,
    };
    mocks.startSetup
      .mockResolvedValueOnce(finished([stopped, busy], { outdated: [leftBusy] }))
      .mockResolvedValueOnce({
        ...finished([stopped, busy], {
          restarted: [
            {
              workspaceId: stopped.workspaceId,
              name: 'stopped-lab',
              fromVersion: null,
              toVersion: '1.95.0',
            },
          ],
          outdated: [leftBusy],
        }),
        intent: 'install',
      });
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.startLabel('stopped-lab') })
    );
    expect(
      await within(dialog).findByText(openCopy.started('stopped-lab', '1.95.0'))
    ).toBeVisible();
    const found = within(dialog).getByTestId('crew-open-found');
    expectUpdateHelp(found, 'busy-lab', openCopy.restartsOnUpdate(null));
    expect(within(found).queryByText(/wasn’t restarted/)).toBeNull();
  });

  it('after a look, says what Update Crew would do, never that a restart was tried (live QA finding 6)', async () => {
    const old = entry({
      role: 'host',
      name: 'old-lab',
      version: null,
      binaryCurrent: false,
      othersConnected: null,
    });
    mocks.startSetup.mockResolvedValue(
      finished([old], {
        outdated: [
          {
            workspaceId: old.workspaceId,
            name: 'old-lab',
            runningVersion: null,
            installedVersion: '1.95.0',
            othersConnected: null,
            reason: 'others_connected',
          },
        ],
      })
    );
    renderDialog();
    const dialog = await look();
    const found = await within(dialog).findByTestId('crew-open-found');
    expect(within(found).getByText(openCopy.outdated(null, '1.95.0'))).toBeVisible();
    expectUpdateHelp(found, 'old-lab', openCopy.restartsOnUpdate(null));
    expect(openCopy.restartsOnUpdate(null)).toBe(
      'Update Crew restarts it. Other people may be connected. Their running agent commands will stop.'
    );
    expect(within(found).queryByText(openCopy.leftRunning.others_connected(null))).toBeNull();
    expect(within(found).queryByText(/wasn’t restarted/)).toBeNull();
    expect(
      within(found).getByRole('button', { name: openCopy.updateLabel('old-lab') })
    ).toBeVisible();
  });

  it('says the versions and what Update Crew does on every row a look found older, whatever its reason', async () => {
    // Nobody else connected: the daemon's reason is not_running_installed (a check restarts
    // nothing). The other row is older than this computer's Crew, which the job did not list.
    const quiet = entry({
      role: 'host',
      name: 'quiet-lab',
      version: '1.94.1',
      binaryCurrent: false,
      othersConnected: 0,
    });
    const behind = entry({
      workspaceId: '22222222-2222-4333-8444-555555555555',
      role: 'host',
      name: 'behind-lab',
      version: '1.94.0',
      binaryCurrent: true,
      othersConnected: 1,
    });
    mocks.startSetup.mockResolvedValue(
      finished([quiet, behind], {
        outdated: [
          {
            workspaceId: quiet.workspaceId,
            name: 'quiet-lab',
            runningVersion: '1.94.1',
            installedVersion: '1.94.2',
            othersConnected: 0,
            reason: 'not_running_installed',
          },
        ],
      })
    );
    renderDialog();
    const dialog = await look();
    const found = await within(dialog).findByTestId('crew-open-found');
    const row = (name: string) =>
      Array.from(found.querySelectorAll<HTMLElement>('[data-crew-found-row]')).find((item) =>
        item.textContent?.includes(name)
      ) as HTMLElement;
    // The Crew installed in the account, as the job read it.
    expect(within(row('quiet-lab')).getByText('Runs Crew 1.94.1, not 1.94.2.')).toBeVisible();
    expectUpdateHelp(row('quiet-lab'), 'quiet-lab', 'Update Crew restarts it.');
    // Not listed by the job: the Crew this computer carries, and the count the server gave.
    expect(within(row('behind-lab')).getByText('Runs Crew 1.94.0, not 1.95.0.')).toBeVisible();
    expectUpdateHelp(
      row('behind-lab'),
      'behind-lab',
      'Update Crew restarts it. 1 other person is connected. Their running agent commands will stop.'
    );
    expect(within(found).queryByText(/wasn’t restarted/)).toBeNull();
  });

  it('says the people connected besides you, with "other", wherever it counts them', () => {
    expect(openCopy.othersConnected(1)).toBe(
      '1 other person is connected. Their running agent commands will stop.'
    );
    expect(openCopy.othersConnected(4)).toBe(
      '4 other people are connected. Their running agent commands will stop.'
    );
    expect(openCopy.othersConnected(null)).toBe(
      'Other people may be connected. Their running agent commands will stop.'
    );
    // Only the reason: Update Crew beside it is the way on (design.md 9.16).
    expect(openCopy.leftRunning.others_connected(1)).toBe(
      'It wasn’t restarted, because 1 other person is connected.'
    );
    expect(openCopy.leftRunning.others_connected(2)).toBe(
      'It wasn’t restarted, because 2 other people are connected.'
    );
    expect(openCopy.leftRunning.others_connected(null)).toBe(
      'It wasn’t restarted, because other people may be connected.'
    );
    expect(openCopy.outdated('1.95.0', '1.95.0')).toBe(
      'Runs Crew 1.95.0, not the Crew installed in your account.'
    );
  });

  it('offers Update Crew when the Crew in your account is too old to list workspaces, then lists them (live QA finding 2)', async () => {
    const said =
      'Crew 1.93.0 in your account on leo.ucsf.edu is too old to list workspaces. Update it with biorouter crew setup --server wanjun@leo.ucsf.edu.';
    const tooOld: SetupRun = {
      ...finished([]),
      stages: [
        { name: 'check', state: 'done', detail: null },
        { name: 'sign_in', state: 'pending', detail: null },
        { name: 'workspaces', state: 'skipped', detail: said },
        { name: 'done', state: 'done', detail: null },
      ],
      workspacesNote: said,
      workspacesUnlisted: 'too_old',
      installedVersion: '1.93.0',
    };
    const old = entry({
      role: 'host',
      name: 'old-lab',
      version: null,
      binaryCurrent: false,
      othersConnected: null,
    });
    const listed: SetupRun = {
      ...finished([old], {
        installedVersion: '1.95.0',
        reused: false,
        previousKept: true,
        replacedVersion: '1.93.0',
        outdated: [
          {
            workspaceId: old.workspaceId,
            name: 'old-lab',
            runningVersion: null,
            installedVersion: '1.95.0',
            othersConnected: null,
            reason: 'others_connected',
          },
        ],
      }),
      intent: 'install',
      stages: [
        { name: 'check', state: 'done', detail: null },
        { name: 'install', state: 'done', detail: null },
        { name: 'workspaces', state: 'done', detail: null },
        { name: 'restart', state: 'done', detail: null },
        { name: 'done', state: 'done', detail: null },
      ],
    };
    mocks.startSetup.mockResolvedValueOnce(tooOld).mockResolvedValueOnce(listed);
    renderDialog();
    const dialog = await look();
    const unlisted = await within(dialog).findByTestId('crew-open-unlisted');
    expect(unlisted).toHaveTextContent(openCopy.tooOld('1.93.0', 'leo.ucsf.edu'));
    expect(openCopy.tooOld('1.93.0', 'leo.ucsf.edu')).toBe(
      'Crew 1.93.0 in your account on leo.ucsf.edu is too old to list workspaces.'
    );
    // The button is the instruction; the note does not say it again (design.md 9.16).
    expect(unlisted).not.toHaveTextContent(/Choose /);
    // The dialog's own way on, not the terminal command the daemon's sentence gives.
    expect(within(dialog).queryByText(/biorouter crew setup/)).toBeNull();
    // The listing failed: the steps end at Looking for workspaces, and never say Ready.
    const steps = within(dialog).getByRole('list', { name: setupCopy.stepsLabel });
    expect(within(steps).getByText(setupCopy.stages.finding).closest('li')).toHaveAttribute(
      'data-state',
      'failed'
    );
    expect(within(steps).queryByText(setupCopy.stages.ready)).toBeNull();

    fireEvent.click(within(unlisted).getByRole('button', { name: openCopy.update }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    // It updates the Crew in the account and restarts no workspace.
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      restart_workspaces: false,
    });
    // The same job looked again with the Crew it installed.
    const found = await within(dialog).findByTestId('crew-open-found');
    expect(within(found).getByText(openCopy.hostedByYou('old-lab'))).toBeVisible();
    // Nothing was asked to restart, so the row says what Update Crew would do.
    expectUpdateHelp(found, 'old-lab', openCopy.restartsOnUpdate(null));
    expect(within(dialog).queryByTestId('crew-open-unlisted')).toBeNull();
    const after = within(dialog).getByRole('list', { name: setupCopy.stepsLabel });
    expect(within(after).getByText(setupCopy.stages.ready)).toBeVisible();
    expect(within(after).getByText(setupCopy.updated('1.93.0', '1.95.0'))).toBeVisible();
  });

  it('offers Install Crew when Crew isn’t installed in your account', async () => {
    const said =
      'Crew isn’t installed in your account on leo.ucsf.edu, so Biorouter can’t look for workspaces there.';
    mocks.startSetup.mockResolvedValue({
      ...finished([]),
      workspacesNote: said,
      workspacesUnlisted: 'not_installed',
    });
    renderDialog();
    const dialog = await look();
    const unlisted = await within(dialog).findByTestId('crew-open-unlisted');
    expect(unlisted).toHaveTextContent(openCopy.notInstalled('leo.ucsf.edu'));
    expect(openCopy.notInstalled('leo.ucsf.edu')).toBe(said);
    expect(unlisted).not.toHaveTextContent(/Choose /);
    fireEvent.click(within(unlisted).getByRole('button', { name: openCopy.install }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      restart_workspaces: false,
    });
  });

  it('only says why nothing was listed when this computer has no Crew to install', async () => {
    const carried = mocks.setupSupport.artifact;
    mocks.setupSupport.artifact = null;
    try {
      const said = 'Crew 1.93.0 in your account on leo.ucsf.edu is too old to list workspaces.';
      mocks.startSetup.mockResolvedValue({
        ...finished([]),
        workspacesNote: said,
        workspacesUnlisted: 'too_old',
        installedVersion: '1.93.0',
      });
      renderDialog();
      const dialog = await look();
      expect(await within(dialog).findByTestId('crew-setup-workspaces-note')).toHaveTextContent(
        setupCopy.workspacesNote(said)
      );
      expect(within(dialog).queryByTestId('crew-open-unlisted')).toBeNull();
      expect(within(dialog).queryByRole('button', { name: openCopy.update })).toBeNull();
    } finally {
      mocks.setupSupport.artifact = carried;
    }
  });

  it('offers Update Crew exactly when the daemon says too_old, comparing no versions itself', async () => {
    // The daemon decides what is too old to list workspaces. A version this dialog would read as
    // newer than this computer's, or one it cannot read at all, changes nothing.
    for (const installedVersion of ['1.96.0', '1.96.0-rc.1', 'crew', null]) {
      mocks.startSetup.mockReset();
      const said = `Crew ${installedVersion ?? 'x'} in your account on leo.ucsf.edu is too old to list workspaces.`;
      mocks.startSetup.mockResolvedValue({
        ...finished([]),
        workspacesNote: said,
        workspacesUnlisted: 'too_old',
        installedVersion,
      });
      const { unmount } = renderDialog();
      const dialog = await look();
      const unlisted = await within(dialog).findByTestId('crew-open-unlisted');
      expect(unlisted).toHaveTextContent(openCopy.tooOld(installedVersion, 'leo.ucsf.edu'));
      expect(within(unlisted).getByRole('button', { name: openCopy.update })).toBeEnabled();
      unmount();
    }
    // Any other reason the job could not list them offers no install.
    for (const workspacesUnlisted of ['failed', 'timed_out'] as const) {
      mocks.startSetup.mockReset();
      const said = 'Crew on leo.ucsf.edu couldn’t list its workspaces, so they aren’t shown.';
      mocks.startSetup.mockResolvedValue({
        ...finished([]),
        workspacesNote: said,
        workspacesUnlisted,
        installedVersion: '1.93.0',
      });
      const { unmount } = renderDialog();
      const dialog = await look();
      expect(await within(dialog).findByTestId('crew-setup-workspaces-note')).toHaveTextContent(
        setupCopy.workspacesNote(said)
      );
      expect(within(dialog).queryByTestId('crew-open-unlisted')).toBeNull();
      expect(within(dialog).queryByRole('button', { name: openCopy.update })).toBeNull();
      unmount();
    }
  });

  it('updates at once when nobody else is connected', async () => {
    mocks.startSetup.mockResolvedValue(
      finished([entry({ role: 'host', binaryCurrent: false, othersConnected: 0 })])
    );
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('baranzini-lab') })
    );
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      restart_workspaces: false,
      restart_workspace_ids: ['11111111-2222-4333-8444-555555555555'],
    });
    expect(within(dialog).queryByTestId('crew-open-update-confirm')).toBeNull();
  });

  it('counts a running broker that names no version as older (AO-8)', async () => {
    mocks.startSetup.mockResolvedValue(
      finished([entry({ role: 'host', version: null, binaryCurrent: true })])
    );
    renderDialog();
    const dialog = await look();
    expect(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('baranzini-lab') })
    ).toBeVisible();
  });

  it('says what the update restarted, and offers no update for it again', async () => {
    const outdated = entry({ role: 'host', binaryCurrent: false, othersConnected: 0 });
    mocks.startSetup.mockResolvedValueOnce(finished([outdated])).mockResolvedValueOnce({
      ...finished([outdated], {
        restarted: [
          {
            workspaceId: outdated.workspaceId,
            name: 'baranzini-lab',
            fromVersion: '1.94.1',
            toVersion: '1.95.0',
          },
        ],
      }),
      intent: 'install',
    });
    renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', { name: openCopy.updateLabel('baranzini-lab') })
    );
    expect(
      await within(dialog).findByText(openCopy.restarted('baranzini-lab', '1.95.0'))
    ).toBeVisible();
    expect(
      within(dialog).queryByRole('button', { name: openCopy.updateLabel('baranzini-lab') })
    ).toBeNull();
  });

  it('says how to verify a server whose host key is unknown, and never accepts one', async () => {
    mocks.startSetup.mockResolvedValue({
      ...finished([]),
      state: 'failed',
      stage: 'check',
      error: {
        code: 'crew_ssh_host_key_unknown',
        message: 'Crew couldn’t verify leo.ucsf.edu.',
        detail: 'ED25519 key fingerprint is SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG.',
      },
      output: 'ED25519 key fingerprint is SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG.',
    });
    renderDialog();
    const dialog = await look();
    const help = await within(dialog).findByTestId('crew-setup-host-key-unknown');
    expect(help).toHaveTextContent(trustCopy.unknownBody);
    expect(within(help).getByText(trustCopy.howToVerify)).toBeVisible();
    expect(within(dialog).queryByRole('button', { name: /accept|trust/i })).toBeNull();
  });

  it('looks at once with the login a surface handed it (Update Crew, a name in use)', async () => {
    mocks.startSetup.mockResolvedValue(finished([entry()]));
    renderDialog({
      ui: {
        dialog: {
          kind: 'open-workspace',
          route: { ssh_target: LOGIN, port: 2222, identity_file: null, proxy_jump: null },
          check: true,
        },
        pane: null,
      },
    });
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(1));
    expect(mocks.startSetup.mock.calls[0][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, port: 2222, preparation_id: 'prep-1' },
      intent: 'check',
    });
  });

  it('lists two hosts that present one workspace ID as two rows (AO-6)', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    try {
      mocks.startSetup.mockResolvedValue(
        finished([
          entry({ hostUsername: 'wanjun', hostUid: 1000 }),
          entry({ hostUsername: 'alice', hostUid: 1001, fingerprint: 'ffff' }),
        ])
      );
      renderDialog();
      const dialog = await look();
      const found = await within(dialog).findByTestId('crew-open-found');
      expect(within(found).getAllByRole('listitem')).toHaveLength(2);
      expect(within(found).getByText(openCopy.hostedBy('baranzini-lab', 'alice'))).toBeVisible();
      // One name, two hosts: the fact in view, the advice behind its info icon (design.md 9.16).
      expect(
        within(dialog).getByText(openCopy.sameName('baranzini-lab'), { selector: 'span' })
      ).toBeVisible();
      expect(
        within(dialog).getByRole('button', { name: `About ${openCopy.sameNameMoreLabel}` })
      ).toHaveAccessibleDescription(openCopy.sameNameMore);
      // At rest the advice is only the info icon's hidden description, not a line of the note.
      expect(within(dialog).getByText(openCopy.sameNameMore)).toHaveAttribute(
        'data-slot',
        'info-tip-description'
      );
      expect(openCopy.sameNameMore).toBe('Check the host before you open one.');
      const keyWarnings = errors.mock.calls.filter((call) => String(call[0]).includes('same key'));
      expect(keyWarnings).toEqual([]);
    } finally {
      errors.mockRestore();
    }
  });

  it('says one line where a person reads, with the rest behind its info icon (design.md 9.16)', async () => {
    mocks.startSetup.mockResolvedValue(
      finished([entry({ hostUsername: 'alice', hostUid: 1001, invitation: null })])
    );
    renderDialog();
    const dialog = await screen.findByRole('dialog', { name: openCopy.title });
    // The intro: one line, and that the workspaces need no invitation behind its icon.
    const intro = within(dialog).getByTestId('crew-open-intro');
    expect(within(intro).getByText(openCopy.intro)).toBeVisible();
    expect(
      within(intro).getByRole('button', { name: `About ${openCopy.introMoreLabel}` })
    ).toHaveAccessibleDescription('You need no invitation or code for those.');
    expect(within(dialog).getByText(openCopy.introMore)).toHaveAttribute(
      'data-slot',
      'info-tip-description'
    );
    // A row with nothing to open it from: why, in view; the way on behind its icon.
    fireEvent.change(within(dialog).getByLabelText(openCopy.login), { target: { value: LOGIN } });
    fireEvent.click(within(dialog).getByRole('button', { name: openCopy.look }));
    const found = await within(dialog).findByTestId('crew-open-found');
    expect(within(found).getByText(openCopy.cantOpen)).toBeVisible();
    const name = 'baranzini-lab';
    expect(
      within(found).getByRole('button', { name: `About ${openCopy.cantOpenMoreLabel(name)}` })
    ).toHaveAccessibleDescription('Update Crew there, then look again.');
    expect(within(found).getByText(openCopy.cantOpenMore)).toHaveAttribute(
      'data-slot',
      'info-tip-description'
    );
  });

  it('says an empty list in one line, and how to join another’s behind its info icon', async () => {
    mocks.startSetup.mockResolvedValue(finished([]));
    renderDialog();
    const dialog = await look();
    const none = await within(dialog).findByText(openCopy.none('leo.ucsf.edu'));
    expect(none).toBeVisible();
    expect(openCopy.none('leo.ucsf.edu')).toBe('There are no workspaces for you on leo.ucsf.edu.');
    expect(
      within(dialog).getByRole('button', { name: `About ${openCopy.noneMoreLabel}` })
    ).toHaveAccessibleDescription('To join someone else’s, ask them to invite you.');
    expect(within(dialog).getByText(openCopy.noneMore)).toHaveAttribute(
      'data-slot',
      'info-tip-description'
    );
  });

  it('says "hosted by you" once, and shows the login as typed', async () => {
    mocks.startSetup.mockResolvedValue(finished([entry({ role: 'host' })]));
    renderDialog();
    const dialog = await look('Leo-WJ-Crew');
    const found = await within(dialog).findByTestId('crew-open-found');
    expect(within(found).getByText(openCopy.hostedByYou('baranzini-lab'))).toBeVisible();
    expect(within(found).queryByText(new RegExp(openCopy.role.host))).toBeNull();
    const heading = within(found).getByRole('heading', { level: 3 });
    expect(heading).toHaveTextContent('Workspaces on Leo-WJ-Crew');
    expect(heading.className).not.toMatch(/text-caps/);
  });

  it('puts focus on the first workspace found once the look ends', async () => {
    mocks.startSetup.mockResolvedValue(finished([entry()]));
    renderDialog();
    const dialog = await screen.findByRole('dialog', { name: openCopy.title });
    fireEvent.change(within(dialog).getByLabelText(openCopy.login), { target: { value: LOGIN } });
    const submit = within(dialog).getByRole('button', { name: openCopy.look });
    submit.focus();
    fireEvent.click(submit);
    const open = await within(dialog).findByRole('button', {
      name: openCopy.openLabel(openCopy.hostedBy('baranzini-lab', 'wanjun')),
    });
    await waitFor(() => expect(document.activeElement).toBe(open));
  });

  it('keeps the dialog when a daemon refuses the save, saying why', async () => {
    mocks.startSetup.mockResolvedValue(finished([entry()]));
    mocks.saveFromInvitation.mockRejectedValue(
      new CrewHttpError('This invitation needs an institution.', 400, 'crew_invitation_invalid')
    );
    const { crew } = renderDialog();
    const dialog = await look();
    fireEvent.click(
      await within(dialog).findByRole('button', {
        name: openCopy.openLabel(openCopy.hostedBy('baranzini-lab', 'wanjun')),
      })
    );
    await waitFor(() => expect(mocks.saveFromInvitation).toHaveBeenCalledTimes(1));
    expect(crew.selectConnection).not.toHaveBeenCalled();
    expect(crew.closeDialog).not.toHaveBeenCalled();
  });
});
