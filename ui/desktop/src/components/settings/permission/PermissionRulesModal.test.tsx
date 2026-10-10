import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import PermissionRulesModal from './PermissionRulesModal';
import { permissionDialogCopy } from '../chat/copy';

const { getExtensions } = vi.hoisted(() => ({ getExtensions: vi.fn() }));

vi.mock('../../ConfigContext', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../ConfigContext')>();
  return { ...actual, useConfig: () => ({ getExtensions }) };
});

vi.mock('./PermissionModal', () => ({
  default: ({ extensionName }: { extensionName: string }) => (
    <div data-testid="permission-modal">{extensionName}</div>
  ),
}));

describe('PermissionRulesModal', () => {
  beforeEach(() => {
    getExtensions.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('lists enabled real extensions without fabricating a platform extension', async () => {
    getExtensions.mockResolvedValue([
      {
        type: 'builtin',
        name: 'developer',
        display_name: 'Developer',
        description: 'A very long description that must remain inside the permission row.',
        enabled: true,
      },
      {
        type: 'builtin',
        name: 'platform',
        display_name: 'Platform',
        description: 'Synthetic internal grouping',
        enabled: true,
      },
      {
        type: 'builtin',
        name: 'disabled',
        display_name: 'Disabled',
        description: 'Disabled extension',
        enabled: false,
      },
    ]);

    render(<PermissionRulesModal isOpen onClose={vi.fn()} />);

    const developer = await screen.findByRole('button', { name: /Developer/ });
    // One truncated line of description, so a long one cannot grow the row (principle 4).
    expect(
      screen.getByText('A very long description that must remain inside the permission row.')
    ).toHaveClass('truncate');
    expect(screen.queryByText('Synthetic internal grouping')).not.toBeInTheDocument();
    expect(screen.queryByText('Disabled extension')).not.toBeInTheDocument();

    fireEvent.click(developer);
    expect(screen.getByTestId('permission-modal')).toHaveTextContent('developer');
  });

  it('shows a retryable error instead of leaving the extension list loading forever', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    getExtensions.mockRejectedValue(new Error('unavailable'));

    render(<PermissionRulesModal isOpen onClose={vi.fn()} />);

    expect(await screen.findByText(permissionDialogCopy.extensionsFailed)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: permissionDialogCopy.tryAgain })).toBeInTheDocument();
    expect(error).toHaveBeenCalledWith(
      'Failed to load extensions for permission settings:',
      expect.objectContaining({ message: 'unavailable' })
    );
  });

  it('is one dialog titled Tool permissions, with no icon tile', async () => {
    getExtensions.mockResolvedValue([]);
    render(<PermissionRulesModal isOpen onClose={vi.fn()} />);
    const dialog = await screen.findByRole('dialog', { name: permissionDialogCopy.rulesTitle });
    expect(await screen.findByText(permissionDialogCopy.noExtensions)).toBeInTheDocument();
    expect(dialog.querySelector('svg.h-5')).toBeNull();
  });
});
