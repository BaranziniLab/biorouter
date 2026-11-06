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
    expect(ask).toHaveTextContent(
      '3 people are connected. Their running agent commands will stop.'
    );
    expect(mocks.startSetup).toHaveBeenCalledTimes(1);
    fireEvent.click(within(ask).getByRole('button', { name: openCopy.updateAnyway }));
    await waitFor(() => expect(mocks.startSetup).toHaveBeenCalledTimes(2));
    expect(mocks.startSetup.mock.calls[1][0]).toEqual({
      target: { kind: 'server', ssh_target: LOGIN, preparation_id: 'prep-1' },
      intent: 'install',
      restart_workspaces: true,
    });
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
