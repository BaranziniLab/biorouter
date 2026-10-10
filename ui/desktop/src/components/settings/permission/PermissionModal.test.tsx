import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PermissionModal from './PermissionModal';

const { getTools, upsertPermissions } = vi.hoisted(() => ({
  getTools: vi.fn(),
  upsertPermissions: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../../api')>();
  return { ...actual, getTools, upsertPermissions };
});

describe('PermissionModal', () => {
  beforeEach(() => {
    getTools.mockReset();
    upsertPermissions.mockReset();
  });

  it('loads the configured extension independently of an active chat', async () => {
    getTools.mockResolvedValue({
      data: [
        {
          name: 'extensionmanager__search_available_extensions',
          description: 'Find extensions that can help with a task',
          parameters: [],
          permission: 'ask_before',
        },
      ],
    });

    render(
      <PermissionModal
        extensionName="Extension Manager"
        extensionLabel="Extension Manager"
        onClose={vi.fn()}
      />
    );

    expect(await screen.findByText('Search Available Extensions')).toBeInTheDocument();
    // The description is the row's help: the select hears it, and it is not a visible paragraph.
    const select = screen.getByRole('button', { name: /Search Available Extensions Ask before/ });
    expect(select).toHaveAccessibleDescription('Find extensions that can help with a task');
    expect(getTools).toHaveBeenCalledWith({
      query: { extension_name: 'Extension Manager', session_id: '' },
    });
  });

  it('shows a completed empty state instead of an endless loading indicator', async () => {
    getTools.mockResolvedValue({ data: [] });

    render(<PermissionModal extensionName="empty" onClose={vi.fn()} />);

    expect(await screen.findByText('No configurable tools')).toBeInTheDocument();
    expect(screen.queryByText('Loading tools…')).not.toBeInTheDocument();
  });

  it('renders camel-case tool identifiers as readable words', async () => {
    getTools.mockResolvedValue({
      data: [
        {
          name: 'skills__installMarketplaceSkill',
          description: 'Install a trusted skill',
          parameters: [],
          permission: 'ask_before',
        },
      ],
    });

    render(<PermissionModal extensionName="skills" onClose={vi.fn()} />);

    expect(await screen.findByText('Install Marketplace Skill')).toBeInTheDocument();
  });

  it('shows an actionable error state and can retry', async () => {
    getTools
      .mockResolvedValueOnce({ error: { message: 'failed' } })
      .mockResolvedValueOnce({ data: [] });

    render(<PermissionModal extensionName="broken" onClose={vi.fn()} />);

    expect(await screen.findByText('Tools could not be loaded')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    await waitFor(() => expect(getTools).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('No configurable tools')).toBeInTheDocument();
  });

  it('saves a changed rule through the one Save changes button', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    getTools.mockResolvedValue({
      data: [
        {
          name: 'developer__shell',
          description: 'Run a shell command.',
          parameters: [],
          permission: 'ask_before',
        },
      ],
    });
    upsertPermissions.mockResolvedValue({ data: {} });
    render(<PermissionModal extensionName="developer" onClose={onClose} />);

    const save = await screen.findByRole('button', { name: 'Save changes' });
    expect(save).toBeDisabled();
    await user.click(screen.getByRole('button', { name: /Shell Ask before/ }));
    await user.click(await screen.findByRole('menuitemradio', { name: 'Always allow' }));
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() =>
      expect(upsertPermissions).toHaveBeenCalledWith({
        body: { tool_permissions: [{ tool_name: 'developer__shell', permission: 'always_allow' }] },
      })
    );
    expect(onClose).toHaveBeenCalled();
  });
});
