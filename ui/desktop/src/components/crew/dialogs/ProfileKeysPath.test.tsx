import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { keysCopy, profileCopy, sharePathCopy } from './copy';
import {
  alice,
  bob,
  connection,
  installResizeObserverStub,
  makeSnapshot,
  renderWithCrew,
  requestsFor,
} from './dialogsTestHarness';
import { EditProfileDialog } from './EditProfileDialog';
import { KeysDialog } from './KeysDialog';
import { SharePathDialog } from './SharePathDialog';

installResizeObserverStub();

afterEach(() => vi.clearAllMocks());

describe('EditProfileDialog', () => {
  it('offers the name on the server account without applying it, and saves on Save profile', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(<EditProfileDialog onClose={onClose} />, {
      request: (method) => (method === 'profile.suggest' ? { full_name: 'Alice M. Chen' } : {}),
    });
    const name = await screen.findByLabelText('Display name');
    await waitFor(() => expect(name).toHaveFocus());
    expect(name).toHaveValue('Alice Chen');
    expect(screen.getByText('Your username:')).toHaveTextContent('Your username: @alice');

    // She chose a name already, so the suggestion is one click away, not applied.
    const offer = await screen.findByRole('button', {
      name: profileCopy.suggestion('Alice M. Chen'),
    });
    expect(name).toHaveValue('Alice Chen');
    expect(requestsFor(crew, 'profile.update')).toEqual([]);
    fireEvent.click(offer);
    expect(name).toHaveValue('Alice M. Chen');

    fireEvent.change(screen.getByLabelText('Initials (optional)'), { target: { value: 'AC' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save profile' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'profile.update')).toEqual([
        { nickname: 'Alice M. Chen', avatar: 'AC' },
      ])
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('prefills the suggestion for someone who never chose a name', async () => {
    const plainBob = { ...bob, nickname: 'bob' };
    renderWithCrew(<EditProfileDialog onClose={vi.fn()} />, {
      snapshot: makeSnapshot({ actor: plainBob, principals: [alice, plainBob] }),
      request: (method) => (method === 'profile.suggest' ? { full_name: 'Bob Lee' } : {}),
    });
    await waitFor(() => expect(screen.getByLabelText('Display name')).toHaveValue('Bob Lee'));
  });

  it('offers nothing when the broker has no suggestion method', async () => {
    renderWithCrew(<EditProfileDialog onClose={vi.fn()} />, {
      request: (method) => {
        if (method === 'profile.suggest') throw new Error('unsupported: method unavailable');
        return {};
      },
    });
    expect(await screen.findByLabelText('Display name')).toHaveValue('Alice Chen');
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
    expect(screen.queryByRole('button', { name: /^Use “/ })).toBeNull();
  });

  it('refuses @ or # in a display name before sending', async () => {
    const { crew } = renderWithCrew(<EditProfileDialog onClose={vi.fn()} />);
    const name = await screen.findByLabelText('Display name');
    fireEvent.change(name, { target: { value: 'Alice (＠bob)' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save profile' }));
    expect(name).toBeInvalid();
    expect(screen.getByText(profileCopy.handleMark)).toBeInTheDocument();
    expect(requestsFor(crew, 'profile.update')).toEqual([]);
  });
});

describe('KeysDialog', () => {
  const credentials = vi.fn();
  beforeEach(() => {
    credentials.mockReset();
    (window as unknown as { electron: Record<string, unknown> }).electron = {
      ...(window as unknown as { electron?: Record<string, unknown> }).electron,
      crewCredentials: credentials,
    };
  });

  it('loads the storage status on open and shows this device’s key', async () => {
    credentials.mockResolvedValue({ backend: 'keyring', initialized: true, locked: false });
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(await screen.findByText(keysCopy.keychain)).toBeInTheDocument();
    expect(credentials).toHaveBeenCalledWith('status');
    expect(screen.getByRole('button', { name: 'Copy this device’s key' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Refresh credential status' })).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: keysCopy.vaultToggle }));
    expect(await screen.findByText(keysCopy.vaultNote)).toBeInTheDocument();
    credentials.mockResolvedValue({ cancelled: true });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: keysCopy.setUpVault }));
    });
    expect(credentials).toHaveBeenLastCalledWith('init');
    // A cancelled native prompt changes nothing.
    expect(screen.getByText(keysCopy.keychain)).toBeInTheDocument();
  });

  it('locks and unlocks a vault through the same IPC', async () => {
    credentials.mockResolvedValueOnce({
      backend: 'encrypted_vault',
      initialized: true,
      locked: true,
    });
    renderWithCrew(<KeysDialog onClose={vi.fn()} />);
    expect(await screen.findByText(keysCopy.vault)).toBeInTheDocument();
    expect(screen.getByText(keysCopy.locked)).toBeInTheDocument();
    credentials.mockResolvedValueOnce({
      backend: 'encrypted_vault',
      initialized: true,
      locked: false,
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Unlock' }));
    });
    expect(credentials).toHaveBeenLastCalledWith('unlock');
    expect(await screen.findByText(keysCopy.unlocked)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Lock' })).toBeInTheDocument();
  });

  it('shows a failure in the dialog and lists the account’s devices', async () => {
    credentials.mockRejectedValue(new Error('The vault passphrase was not accepted.'));
    renderWithCrew(<KeysDialog onClose={vi.fn()} />, {
      snapshot: makeSnapshot({
        actor: {
          ...alice,
          devices: [
            { fingerprint: '3F2A 9C1E 77B0 D4E1', added_at: 1_700_000_000, added_via: 'bootstrap' },
          ],
        },
      }),
    });
    expect(await screen.findByText('The vault passphrase was not accepted.')).toBeInTheDocument();
    const devices = screen.getByRole('region', { name: keysCopy.devices });
    expect(within(devices).getByText('3F2A 9C1E 77B0 D4E1')).toBeInTheDocument();
    expect(devices).toHaveTextContent(keysCopy.addedVia.bootstrap);
  });
});

describe('SharePathDialog', () => {
  it('requires an absolute path and adds the reference to the message', async () => {
    const onClose = vi.fn();
    const { crew } = renderWithCrew(<SharePathDialog onClose={onClose} />, {
      request: (method) => (method === 'reference.create' ? { id: 'ref-1' } : {}),
    });
    expect(
      await screen.findByRole('dialog', { name: sharePathCopy.title('hpc.example.edu') })
    ).toBeInTheDocument();
    const path = screen.getByLabelText('Path');
    await waitFor(() => expect(path).toHaveFocus());
    fireEvent.change(path, { target: { value: 'data/run.h5ad' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add to message' }));
    expect(path).toBeInvalid();
    expect(requestsFor(crew, 'reference.create')).toEqual([]);

    fireEvent.change(path, { target: { value: '/home/alice/project/run.h5ad' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add to message' }));
    });
    await waitFor(() =>
      expect(requestsFor(crew, 'reference.create')).toEqual([
        {
          channel_id: 'channel-general',
          path: '/home/alice/project/run.h5ad',
          label: 'run.h5ad',
        },
      ])
    );
    expect(crew.addReference).toHaveBeenCalledWith({ id: 'ref-1', label: 'run.h5ad' });
    expect(onClose).toHaveBeenCalled();
    expect(connection.id).toBe('conn-1');
  });

  it('keeps the label optional, behind Advanced', async () => {
    const { crew } = renderWithCrew(<SharePathDialog onClose={vi.fn()} />, {
      request: () => ({ id: 'ref-2' }),
    });
    expect(screen.queryByLabelText('Label (optional)')).toBeNull();
    expect(await screen.findByText(sharePathCopy.labelSummary)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Path'), { target: { value: '/data/big.bam' } });
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    fireEvent.change(await screen.findByLabelText('Label (optional)'), {
      target: { value: 'Aligned reads' },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Add to message' }));
    });
    await waitFor(() =>
      expect(crew.addReference).toHaveBeenCalledWith({ id: 'ref-2', label: 'Aligned reads' })
    );
  });
});
