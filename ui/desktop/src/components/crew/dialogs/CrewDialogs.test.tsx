import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DialogIntent } from '../state/types';
import { CrewDialogs, dialogKey, HOSTED_DIALOG_KINDS } from './CrewDialogs';
import { installResizeObserverStub, makeSnapshot, renderWithCrew } from './dialogsTestHarness';

installResizeObserverStub();

afterEach(() => vi.clearAllMocks());

/** Each hosted dialog, and the control its first focus lands on. */
const FIRST_FIELDS: [DialogIntent, string][] = [
  [{ kind: 'connection-settings', connectionId: 'conn-1' }, 'Connection name'],
  [{ kind: 'invite-people' }, 'Username'],
  [{ kind: 'let-in', username: 'eve' }, 'Code from @eve'],
  [{ kind: 'create-team' }, 'Name'],
  [{ kind: 'create-channel', teamId: 'team-1' }, 'Name'],
  [{ kind: 'rename', target: 'team', targetId: 'team-1' }, 'Name'],
  [{ kind: 'edit-profile' }, 'Display name'],
  [{ kind: 'share-path' }, 'Path'],
];

describe('CrewDialogs', () => {
  it.each(FIRST_FIELDS)('opens %j on its first field', async (intent, label) => {
    renderWithCrew(<CrewDialogs />, {
      dialog: intent,
      snapshot: makeSnapshot({ pending_joins: [{ username: 'eve' }] }),
    });
    const field = await screen.findByLabelText(label);
    await waitFor(() => expect(field).toHaveFocus());
  });

  it('opens a picker dialog on its picker', async () => {
    renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'add-people', target: 'channel', targetId: 'channel-general' },
    });
    const picker = await screen.findByRole('button', { name: /^Person/ });
    await waitFor(() => expect(picker).toHaveFocus());
  });

  it('opens a destructive confirmation on Cancel', async () => {
    renderWithCrew(<CrewDialogs />, {
      dialog: {
        kind: 'confirm',
        confirm: { action: 'archive-channel', channelId: 'channel-general' },
      },
    });
    const cancel = await screen.findByRole('button', { name: 'Cancel' });
    await waitFor(() => expect(cancel).toHaveFocus());
  });

  it('closes through the controller, and renders nothing for dialogs other areas own', async () => {
    Object.assign(window, {
      electron: { ...window.electron, crewCredentials: vi.fn(async () => ({ cancelled: true })) },
    });
    const { crew } = renderWithCrew(<CrewDialogs />, { dialog: { kind: 'keys' } });
    expect(await screen.findByRole('dialog', { name: 'Keys and security' })).toBeInTheDocument();
    act(() => crew.current().closeDialog());
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    for (const intent of [{ kind: 'join' }, { kind: 'host' }] as DialogIntent[]) {
      act(() => crew.current().openDialog(intent));
      expect(screen.queryByRole('dialog')).toBeNull();
    }
  });

  it('mounts a fresh dialog for a different intent of the same kind', async () => {
    const snapshot = makeSnapshot({
      pending_joins: [{ username: 'eve' }, { username: 'frank' }],
    });
    const { crew } = renderWithCrew(<CrewDialogs />, {
      dialog: { kind: 'let-in', username: 'eve' },
      snapshot,
    });
    expect(await screen.findByRole('dialog', { name: 'Let @eve into lab' })).toBeInTheDocument();
    act(() => crew.current().openDialog({ kind: 'let-in', username: 'frank' }));
    expect(await screen.findByRole('dialog', { name: 'Let @frank into lab' })).toBeInTheDocument();
    expect(dialogKey({ kind: 'let-in', username: 'eve' })).not.toBe(
      dialogKey({ kind: 'let-in', username: 'frank' })
    );
  });

  it('hosts every dialog kind but join, host and sign-in', () => {
    expect([...HOSTED_DIALOG_KINDS].sort()).toEqual(
      [
        'add-people',
        'confirm',
        'connection-settings',
        'create-channel',
        'create-team',
        'edit-profile',
        'invite-people',
        'keys',
        'let-in',
        'rename',
        'share-path',
        'transfer-ownership',
        'workspace-settings',
      ].sort()
    );
  });
});
