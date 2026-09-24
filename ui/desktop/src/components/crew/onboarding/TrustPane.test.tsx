import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { LastConnectFailure } from '../state/types';
import { hostCopy, trustCopy } from './copy';
import { resetJoinContextForTests, updateJoinContext } from './joinContext';
import { TrustPane } from './TrustPane';
import { fakeConnection, makeCrew, renderWithCrew } from './testCrew';

const dock = vi.hoisted(() => ({ props: [] as Record<string, unknown>[] }));
vi.mock('../../InAppTerminalDock', () => ({
  default: (props: Record<string, unknown>) => {
    dock.props.push(props);
    return <div data-testid="in-app-terminal-dock" />;
  },
}));

const OFFERED = 'SHA256:' + 'A'.repeat(43);
const KNOWN = 'SHA256:' + 'B'.repeat(43);

function renderPane(failure: LastConnectFailure) {
  const crew = makeCrew({
    connectionId: 'conn-1',
    connection: fakeConnection({ status: 'disconnected' }),
    lastConnectFailure: failure,
    screen: 'trust',
  });
  renderWithCrew(<TrustPane />, crew);
  return crew;
}

/** Nothing on a trust pane may accept a key or hand out a command that removes one. */
function expectNoAcceptOrRemoval() {
  for (const button of screen.queryAllByRole('button')) {
    expect(button.textContent ?? '').not.toMatch(/accept|trust (this|it)|continue anyway|remove/i);
  }
  expect(document.body.textContent).not.toMatch(/ssh-keygen|known_hosts -R|StrictHostKeyChecking/);
}

beforeEach(() => {
  dock.props.length = 0;
  resetJoinContextForTests();
});

describe('TrustPane', () => {
  it('shows an unknown host key with the fingerprint it offered and Try again', async () => {
    const crew = renderPane({
      kind: 'host_key_unknown',
      message: 'Host key verification failed.',
      code: 'crew_ssh_host_key_unknown',
      detail: `Offered host key fingerprint: ${OFFERED}\n\nHost key verification failed.`,
    });

    expect(
      screen.getByRole('heading', { name: trustCopy.unknownTitle('hpc.ucsf.edu') })
    ).toBeInTheDocument();
    expect(screen.getByText(trustCopy.unknownBody)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: `Copy ${trustCopy.offeredLabel}` }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(OFFERED));

    fireEvent.click(screen.getByRole('button', { name: trustCopy.tryAgain }));
    expect(crew.connect).toHaveBeenCalledWith({ userInitiated: true });
    expect(crew.registerErrorSlot).toHaveBeenCalledWith('connect');
    expectNoAcceptOrRemoval();
  });

  it('explains how to verify, and offers a terminal that runs nothing for the person', () => {
    renderPane({ kind: 'host_key_unknown', message: 'Host key verification failed.' });
    expect(screen.queryByText(trustCopy.steps('hpc.ucsf.edu')[0])).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: trustCopy.howToVerify }));
    for (const step of trustCopy.steps('hpc.ucsf.edu')) {
      expect(screen.getByText(step)).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole('button', { name: hostCopy.openTerminal }));
    expect(screen.getByTestId('in-app-terminal-dock')).toBeInTheDocument();
    // The dock is opened as a plain shell: no command, no chat to take Run requests from.
    expect(dock.props[0]).not.toHaveProperty('dockKey');
    expect(Object.keys(dock.props[0]).sort()).toEqual(['onClose', 'onEmptied', 'open']);
    expectNoAcceptOrRemoval();
  });

  it('shows a changed host key with both fingerprints and only "Copy details for IT"', async () => {
    renderPane({
      kind: 'host_key_changed',
      message: 'REMOTE HOST IDENTIFICATION HAS CHANGED',
      code: 'crew_ssh_host_key_changed',
      detail: [
        `New host key fingerprint (offered by the server): ${OFFERED}`,
        `Previously known host key fingerprint: ${KNOWN}`,
        '',
        '@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@',
      ].join('\n'),
    });

    const pane = screen.getByTestId('crew-trust-changed');
    expect(pane).toHaveAttribute('data-tone', 'danger');
    expect(
      within(pane).getByRole('heading', { name: trustCopy.changedTitle('hpc.ucsf.edu') })
    ).toBeInTheDocument();
    expect(within(pane).getByText(OFFERED)).toBeInTheDocument();
    expect(within(pane).getByText(KNOWN)).toBeInTheDocument();
    expect(
      within(pane)
        .getAllByRole('button')
        .map((button) => button.textContent)
    ).toEqual([trustCopy.copyForIt]);

    fireEvent.click(screen.getByRole('button', { name: trustCopy.copyForIt }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalled());
    const copied = vi.mocked(navigator.clipboard.writeText).mock.calls.slice(-1)[0]?.[0] ?? '';
    expect(copied).toContain('Server: hpc.ucsf.edu');
    expect(copied).toContain(OFFERED);
    expect(copied).toContain(KNOWN);
    expectNoAcceptOrRemoval();
  });

  it('shows a different workspace key with Copy details and Connection settings', () => {
    updateJoinContext('conn-1', { hostUsername: 'alice', hostDisplayName: 'Alice Chen' });
    const crew = renderPane({
      kind: 'workspace_identity_mismatch',
      message: 'workspace identity mismatch',
      code: 'crew_workspace_identity_mismatch',
    });
    expect(screen.getByRole('heading', { name: trustCopy.workspaceTitle })).toBeInTheDocument();
    expect(screen.getByText(trustCopy.workspaceBody('Alice Chen (@alice)'))).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: trustCopy.connectionSettings }));
    expect(crew.openDialog).toHaveBeenCalledWith({
      kind: 'connection-settings',
      connectionId: 'conn-1',
    });
    expect(screen.getByRole('button', { name: trustCopy.copyDetails })).toBeInTheDocument();
    expectNoAcceptOrRemoval();
  });

  it('draws nothing for a failure that is not a trust failure', () => {
    renderPane({ kind: 'unreachable', message: 'Could not resolve hostname' });
    expect(screen.queryByRole('heading')).toBeNull();
  });
});
