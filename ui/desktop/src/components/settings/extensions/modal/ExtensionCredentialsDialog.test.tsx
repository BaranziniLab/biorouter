import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ExtensionCredentialsDialog from './ExtensionCredentialsDialog';

const api = vi.hoisted(() => ({ get: vi.fn(), purge: vi.fn(), proof: vi.fn() }));
vi.mock('../../../../api', () => ({
  getExtensionCredentials: api.get,
  purgeExtensionCredentials: api.purge,
}));
vi.mock('../../../../utils/userAction', () => ({ userActionHeaders: api.proof }));

const credentials = [
  { key: 'SYNTHETIC_ONLY', stored: true, used_by: [] },
  { key: 'SHARED', stored: true, used_by: ['Extension: other', 'Provider: synthetic'] },
  { key: 'MISSING', stored: false, used_by: [] },
];

beforeEach(() => {
  vi.resetAllMocks();
  api.proof.mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
  api.get.mockResolvedValue({ data: credentials });
  api.purge.mockResolvedValue({
    data: credentials.map((item) =>
      item.key === 'SYNTHETIC_ONLY' ? { ...item, stored: false } : item
    ),
  });
});

describe('extension credential deletion', () => {
  it('reviews retention and deletes only saved unshared keys with user proof', async () => {
    const user = userEvent.setup();
    const onDeleted = vi.fn();
    render(<ExtensionCredentialsDialog name="fixture" onClose={vi.fn()} onDeleted={onDeleted} />);
    await screen.findByText('Retained — Extension: other; Provider: synthetic');
    expect(screen.getByText('No saved value')).toBeInTheDocument();
    expect(screen.getByText(/Restart Biorouter/)).toBeInTheDocument();
    expect(api.purge).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Delete 1 saved credential' }));
    expect(api.purge).toHaveBeenCalledWith({
      path: { name: 'fixture' },
      body: { keys: ['SYNTHETIC_ONLY'] },
      headers: { 'X-User-Action': 'synthetic-proof' },
      throwOnError: true,
    });
    expect(onDeleted).toHaveBeenCalledWith(['SYNTHETIC_ONLY']);
    await screen.findByText(/Saved credentials deleted/);
    expect(screen.getByRole('button', { name: 'Delete 0 saved credentials' })).toBeDisabled();
  });

  it('keeps the review open and reports failed deletion without claiming success', async () => {
    api.purge.mockRejectedValue(new Error('references changed'));
    const onDeleted = vi.fn();
    render(<ExtensionCredentialsDialog name="fixture" onClose={vi.fn()} onDeleted={onDeleted} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Delete 1 saved credential' }));
    await screen.findByRole('alert');
    expect(onDeleted).not.toHaveBeenCalled();
    expect(screen.queryByText(/Saved credentials deleted/)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Delete 1 saved credential' })).toBeDisabled();
  });

  it('cannot delete when references cannot be loaded', async () => {
    api.get.mockRejectedValue(new Error('invalid config'));
    render(<ExtensionCredentialsDialog name="fixture" onClose={vi.fn()} onDeleted={vi.fn()} />);
    await screen.findByRole('alert');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Delete 0 saved credentials' })).toBeDisabled()
    );
    expect(api.purge).not.toHaveBeenCalled();
  });
});
