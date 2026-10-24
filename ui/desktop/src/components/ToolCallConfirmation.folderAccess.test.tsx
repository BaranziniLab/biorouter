import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import ToolConfirmation from './ToolCallConfirmation';
import type { ActionRequired } from '../api';

const mocks = vi.hoisted(() => ({ confirm: vi.fn(), browser: true }));
vi.mock('../api', () => ({ confirmToolAction: mocks.confirm }));
vi.mock('../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-Caller-Provider': 'synthetic-provider' }),
}));
vi.mock('./settings/permission/PermissionModal', () => ({ default: () => null }));
let sequence = 0;

function show(folder = true) {
  const action = {
    type: 'actionRequired',
    data: {
      actionType: 'toolConfirmation',
      id: `folder-browser-${sequence++}`,
      toolName: folder ? 'filesystem__folder_access' : 'skills__removeSkillPackage',
      arguments: {},
      prompt: 'External folder access',
      preview: folder
        ? {
            kind: 'folderAccess',
            folder: '/synthetic/results',
            access: 'read-only',
            files: ['/synthetic/results/completed.csv'],
            descendants: true,
            onceDescendants: false,
            actionId: 'action',
          }
        : undefined,
    },
  } as ActionRequired & { type: 'actionRequired' };
  return render(
    <ToolConfirmation
      sessionId="synthetic-chat"
      isCancelledMessage={false}
      isClicked={false}
      actionRequiredContent={action}
    />
  );
}

beforeEach(() => {
  mocks.browser = true;
  mocks.confirm.mockReset().mockResolvedValue({ data: { status: 'delivered' } });
});

describe('folder-scoped browser approval', () => {
  it('uses a transient scoped key and sends a session decision once', async () => {
    show();
    const key = screen.getByLabelText('Folder approval key');
    expect(key).toHaveAccessibleName('Folder approval key');
    expect(key).toHaveAccessibleDescription(/Use the separate key set with/);
    const allow = screen.getByRole('button', { name: 'Allow for this session' });
    expect(allow).toBeDisabled();
    fireEvent.change(key, { target: { value: 'human-held-folder-key' } });
    fireEvent.click(allow);
    fireEvent.click(allow);
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
    expect(mocks.confirm.mock.calls[0][0]).toMatchObject({
      headers: {
        'X-Folder-Access-Key': 'human-held-folder-key',
        'X-Caller-Provider': 'synthetic-provider',
      },
      body: { sessionId: 'synthetic-chat', action: 'allow_session' },
    });
    expect(mocks.confirm.mock.calls[0][0].headers).not.toHaveProperty('X-User-Action');
    await waitFor(() => expect(screen.queryByLabelText('Folder approval key')).toBeNull());
    expect(screen.queryByRole('button', { name: 'Change' })).toBeNull();
  });

  it('clears a refused key and keeps the pending card retryable', async () => {
    mocks.confirm.mockResolvedValueOnce({
      error: { reason: 'unproven', error: 'Enter the separate folder approval key.' },
    });
    show();
    fireEvent.change(screen.getByLabelText('Folder approval key'), {
      target: { value: 'wrong-key' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Allow once' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Enter the separate folder approval key.'
    );
    expect(screen.getByLabelText('Folder approval key')).toHaveValue('');
    fireEvent.change(screen.getByLabelText('Folder approval key'), {
      target: { value: 'human-held-folder-key' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Allow once' }));
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(2));
  });

  it('can decline without a key and cannot use the folder channel for unrelated protected actions', async () => {
    const card = show();
    fireEvent.click(screen.getByRole('button', { name: 'Deny' }));
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
    expect(mocks.confirm.mock.calls[0][0].headers).not.toHaveProperty('X-Folder-Access-Key');
    card.unmount();
    show(false);
    expect(screen.queryByLabelText('Folder approval key')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Allow once' })).toBeNull();
  });
});
