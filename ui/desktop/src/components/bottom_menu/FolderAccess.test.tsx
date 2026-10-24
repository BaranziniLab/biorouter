import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { FolderAccess } from './FolderAccess';

const mocks = vi.hoisted(() => ({ list: vi.fn(), revoke: vi.fn(), browser: false }));
vi.mock('../../api', () => ({ listFolderAccess: mocks.list, revokeFolderAccess: mocks.revoke }));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'desktop-human-proof' }),
}));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

beforeEach(() => {
  mocks.browser = false;
  mocks.list.mockReset().mockResolvedValue({
    data: [{ id: 'grant', folder: '/synthetic/results', access: 'read-only', descendants: true }],
  });
  mocks.revoke.mockReset().mockResolvedValue({ data: { revoked: true } });
});

describe('chat folder access management', () => {
  it('loads only when opened and revokes only the selected grant in its chat', async () => {
    render(<FolderAccess sessionId="chat-a" />);
    expect(mocks.list).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Folder access' }));
    expect(await screen.findByText('/synthetic/results')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledWith({
      query: { session_id: 'chat-a' },
      headers: { 'X-User-Action': 'desktop-human-proof' },
    });
    mocks.list.mockResolvedValueOnce({ data: [] });
    fireEvent.click(screen.getByRole('button', { name: 'Revoke' }));
    await waitFor(() =>
      expect(mocks.revoke).toHaveBeenCalledWith({
        body: { sessionId: 'chat-a', grantId: 'grant' },
        headers: { 'X-User-Action': 'desktop-human-proof' },
      })
    );
    expect(await screen.findByText('No additional folders are authorized.')).toBeInTheDocument();
  });

  it('keeps a failed revocation visible instead of pretending it succeeded', async () => {
    mocks.revoke.mockResolvedValueOnce({ error: 'refused' });
    render(<FolderAccess sessionId="chat-a" />);
    fireEvent.click(screen.getByRole('button', { name: 'Folder access' }));
    await screen.findByText('/synthetic/results');
    fireEvent.click(screen.getByRole('button', { name: 'Revoke' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Access could not be revoked');
    expect(screen.getByText('/synthetic/results')).toBeInTheDocument();
  });

  it('labels the browser key separately from its help and clears it after revocation', async () => {
    mocks.browser = true;
    render(<FolderAccess sessionId="chat-a" />);
    fireEvent.click(screen.getByRole('button', { name: 'Folder access' }));
    await screen.findByText('/synthetic/results');
    const key = screen.getByLabelText('Folder approval key');
    expect(key).toHaveAccessibleName('Folder approval key');
    expect(key).toHaveAccessibleDescription(/Use the separate key configured with/);
    expect(screen.getByRole('button', { name: 'Revoke' })).toBeDisabled();
    fireEvent.change(key, { target: { value: 'synthetic-folder-key' } });
    fireEvent.click(screen.getByRole('button', { name: 'Revoke' }));
    await waitFor(() => expect(mocks.revoke).toHaveBeenCalledTimes(1));
    expect(mocks.revoke.mock.calls[0][0].headers).toHaveProperty(
      'X-Folder-Access-Key',
      'synthetic-folder-key'
    );
    await waitFor(() => expect(key).toHaveValue(''));
  });
});
