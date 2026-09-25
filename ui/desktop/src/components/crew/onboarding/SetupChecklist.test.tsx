import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewController } from '../state/types';
import { checklistCopy } from './copy';
import { nameOfferDismissedKey, readJoinContext, resetJoinContextForTests } from './joinContext';
import { SetupChecklist } from './SetupChecklist';
import { fakeConnection, fakeSnapshot, makeCrew, renderWithCrew } from './testCrew';

const connection = fakeConnection({ id: 'conn-host', ssh_target: 'henry@lab-server' });

/** Henry, just after Create: his display name is still his username. */
function hostSnapshot(nickname = 'crew_henry') {
  return fakeSnapshot({
    workspace: {
      id: 'workspace-1',
      host_uid: 1000,
      mode: 'private',
      institution_id: 'ucsf',
      policy_epoch: 1,
      host_principal_id: 'p-henry',
      name: 'ito-lab',
    },
    actor: { id: 'p-henry', uid: 1000, username: 'crew_henry', nickname },
    principals: [{ id: 'p-henry', uid: 1000, username: 'crew_henry', nickname }],
  });
}

function hostCrew(overrides: Partial<CrewController> = {}) {
  return makeCrew({
    connectionId: 'conn-host',
    connection,
    connections: [connection],
    snapshot: hostSnapshot(),
    observedPrivacy: {
      connectionId: 'conn-host',
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    isHost: true,
    request: vi.fn(async (method: string) =>
      method === 'profile.suggest' ? { full_name: 'Henry Ito' } : {}
    ) as CrewController['request'],
    ...overrides,
  });
}

beforeEach(() => {
  resetJoinContextForTests();
  localStorage.clear();
});

describe('SetupChecklist: the host’s name (Q3-51)', () => {
  it('offers the server-account name first, before Invite, and applies it only on Use', async () => {
    const crew = hostCrew();
    const view = renderWithCrew(<SetupChecklist force />, crew);

    // The host never joins, so this is the offer the join flow gives everyone else: asked before
    // the first invitation goes out naming him only @crew_henry.
    const row = await screen.findByText(checklistCopy.name('Henry Ito'));
    expect(row).toHaveTextContent('Your name: Use “Henry Ito”?');
    const rows = screen.getAllByRole('listitem');
    expect(rows[0]).toContainElement(row);
    expect(rows[rows.length - 1]).toHaveTextContent(checklistCopy.invite);
    expect(crew.request).toHaveBeenCalledWith('profile.suggest', {}, expect.anything());
    // Offered, never applied silently (naming D2).
    expect(crew.mutate).not.toHaveBeenCalled();

    fireEvent.click(within(rows[0]).getByRole('button', { name: checklistCopy.useName }));
    await waitFor(() =>
      expect(crew.mutate).toHaveBeenCalledWith('profile.update', {
        nickname: 'Henry Ito',
        avatar: null,
      })
    );
    // Answered on this computer: the composer's note won't ask again.
    await waitFor(() => expect(readJoinContext('conn-host').suggestName).toBe(false));
    expect(localStorage.getItem(nameOfferDismissedKey('conn-host'))).not.toBeNull();

    // The workspace now names him: the row stays, ticked, with his name.
    view.update({ snapshot: hostSnapshot('Henry Ito') });
    const first = screen.getAllByRole('listitem')[0];
    expect(first).toHaveAttribute('data-done', 'true');
    expect(first).toHaveTextContent(checklistCopy.nameSet('Henry Ito'));
    expect(within(first).queryByRole('button')).toBeNull();
  });

  it('opens Edit profile from Edit…, and counts that as the answer', async () => {
    const crew = hostCrew();
    renderWithCrew(<SetupChecklist force />, crew);
    await screen.findByText(checklistCopy.name('Henry Ito'));
    fireEvent.click(screen.getByRole('button', { name: checklistCopy.editName }));
    expect(crew.openDialog).toHaveBeenCalledWith({ kind: 'edit-profile' });
    expect(crew.mutate).not.toHaveBeenCalled();
    expect(readJoinContext('conn-host').suggestName).toBe(false);
    // Still unnamed, but answered: the row goes rather than asking twice.
    await waitFor(() => expect(screen.queryByText(checklistCopy.name('Henry Ito'))).toBeNull());
    expect(screen.getAllByRole('listitem')).toHaveLength(3);
  });

  it('keeps the offer open when Use fails, to try again', async () => {
    const crew = hostCrew({
      mutate: vi.fn().mockRejectedValue(new Error('offline')) as CrewController['mutate'],
    });
    renderWithCrew(<SetupChecklist force />, crew);
    fireEvent.click(await screen.findByRole('button', { name: checklistCopy.useName }));
    await waitFor(() => expect(crew.mutate).toHaveBeenCalled());
    await act(async () => {
      await Promise.resolve();
    });
    expect(readJoinContext('conn-host').suggestName).toBe(true);
    expect(screen.getByText(checklistCopy.name('Henry Ito'))).toBeInTheDocument();
  });

  it('has no name row for a host who already has a name, or when the server offers none', async () => {
    const named = hostCrew({ snapshot: hostSnapshot('Henry Ito') });
    const view = renderWithCrew(<SetupChecklist force />, named);
    expect(screen.getAllByRole('listitem')).toHaveLength(3);
    expect(named.request).not.toHaveBeenCalled();
    view.unmount();

    const nothing = hostCrew({
      request: vi.fn().mockResolvedValue({ full_name: null }) as CrewController['request'],
    });
    renderWithCrew(<SetupChecklist force />, nothing);
    await waitFor(() => expect(nothing.request).toHaveBeenCalled());
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getAllByRole('listitem')).toHaveLength(3);
    expect(document.body.textContent).not.toMatch(/Your name/);
  });

  it('does not ask again once the person answered the offer on this computer', async () => {
    localStorage.setItem(nameOfferDismissedKey('conn-host'), '1');
    const crew = hostCrew();
    renderWithCrew(<SetupChecklist force />, crew);
    await act(async () => {
      await Promise.resolve();
    });
    expect(crew.request).not.toHaveBeenCalled();
    expect(screen.getAllByRole('listitem')).toHaveLength(3);
  });

  it('asks nothing for a member, or for the compact line a channel keeps', async () => {
    const member = hostCrew({ isHost: false });
    const view = renderWithCrew(<SetupChecklist force />, member);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByRole('listitem')).toBeNull();
    expect(member.request).not.toHaveBeenCalled();
    view.unmount();

    const compact = hostCrew();
    renderWithCrew(<SetupChecklist compact />, compact);
    await act(async () => {
      await Promise.resolve();
    });
    expect(compact.request).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toMatch(/Your name/);
  });
});
