import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ExtensionCredentialsDialog from './ExtensionCredentialsDialog';
import { EXTENSION_CREDENTIALS_COPY as COPY } from '../copy';

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
    // No em dash in the copy (standing constraint): "Kept: used by …".
    await screen.findByText(COPY.kept('Extension: other; Provider: synthetic'));
    expect(
      screen.getByText('Kept: used by Extension: other; Provider: synthetic')
    ).toBeInTheDocument();
    expect(screen.getByText(COPY.noValue)).toBeInTheDocument();
    // The consequence stays on screen at the moment of decision.
    expect(screen.getByText(COPY.restart)).toBeInTheDocument();
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
    // One restart line at a time: the instruction before, the status after.
    expect(screen.queryByText(COPY.restart)).toBeNull();
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

describe('the saved-credentials dialog chrome', () => {
  it('is a medium dialog whose subtitle says what it does, and keeps the long help in an InfoTip', async () => {
    render(<ExtensionCredentialsDialog name="fixture" onClose={vi.fn()} onDeleted={vi.fn()} />);
    const dialog = await screen.findByRole('dialog', { name: COPY.title('fixture') });

    expect(dialog).toHaveAttribute('data-size', 'md');
    expect(dialog).toHaveAccessibleDescription(COPY.subtitle('fixture'));
    // The paragraph that used to sit above the list is behind the glyph, and still in the page.
    expect(screen.getByText(COPY.help)).toHaveClass('sr-only');
    expect(dialog.querySelector('.br-info-tip')).not.toBeNull();
  });

  it('shows the dialog description without a dash anywhere in its copy', () => {
    const strings = [
      COPY.subtitle('x'),
      COPY.help,
      COPY.kept('y'),
      COPY.deleted,
      COPY.deletedHelp,
      COPY.deleteFailed,
    ];
    for (const text of strings) expect(text).not.toMatch(/[\u2013\u2014]/);
  });
});
