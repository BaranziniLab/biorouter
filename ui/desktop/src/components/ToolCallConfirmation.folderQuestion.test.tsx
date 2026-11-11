/**
 * The folder-access card's one line (R2-1) and its answered row (R2-3), live QA 2026-11-10.
 *
 * The card used to open with the daemon's prompt ("External folder access Allow read-only access
 * to /private/tmp/qa-r2-notes for developer__text_editor? …") and then ask the same question
 * again, never saying which subagent asked; once answered it kept that whole warning below the
 * outcome.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import ToolConfirmation from './ToolCallConfirmation';
import { FolderRequesterContext, accessVerb, folderAsRequested } from './folderAccessCopy';
import type { ActionRequired } from '../api';

const mocks = vi.hoisted(() => ({ confirm: vi.fn() }));
vi.mock('../api', () => ({ confirmToolAction: mocks.confirm }));
vi.mock('../utils/surface', () => ({ isBrowserSurface: () => false }));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));
vi.mock('./settings/permission/PermissionModal', () => ({ default: () => null }));

const PROMPT =
  'External folder access\nAllow read-only access to /private/tmp/qa-notes for developer__text_editor? ' +
  'Allow once covers only the listed files in this operation. A session grant includes descendants. ' +
  'Revoking the grant or deleting this chat ends it.\nFiles:\n' +
  '/private/tmp/qa-notes/note2.txt (requested as /tmp/qa-notes/note2.txt)';

let sequence = 0;

function show({
  requester = null,
  requestedBy,
  folder = '/private/tmp/qa-notes',
  files = ['/private/tmp/qa-notes/note2.txt (requested as /tmp/qa-notes/note2.txt)'],
}: {
  requester?: string | null;
  requestedBy?: string;
  folder?: string;
  files?: string[];
} = {}) {
  const action = {
    type: 'actionRequired',
    data: {
      actionType: 'toolConfirmation',
      id: `folder-question-${sequence++}`,
      toolName: 'filesystem__folder_access',
      arguments: {},
      prompt: PROMPT,
      preview: {
        kind: 'folderAccess',
        folder,
        access: 'read-only',
        files,
        descendants: true,
        onceDescendants: false,
        actionId: 'action',
        ...(requestedBy ? { requestedBy } : {}),
      },
    },
  } as ActionRequired & { type: 'actionRequired' };
  return render(
    <FolderRequesterContext.Provider value={requester}>
      <ToolConfirmation
        sessionId="synthetic-chat"
        isCancelledMessage={false}
        isClicked={false}
        actionRequiredContent={action}
      />
    </FolderRequesterContext.Provider>
  );
}

beforeEach(() => {
  mocks.confirm.mockReset().mockResolvedValue({ data: { status: 'delivered' } });
  Object.defineProperty(window, 'appConfig', {
    configurable: true,
    value: {
      get: (key: string) =>
        ({ BIOROUTER_HOME_DIR: '/Users/someone', BIOROUTER_LOCAL_BACKEND: true })[key],
    },
  });
});

describe('the folder-access card asks one question', () => {
  it('names the subagent that asks and the folder as it was asked for, with no tool id', () => {
    const { container } = show({ requester: 'notes-reader' });
    expect(screen.getByTestId('folder-access-question')).toHaveTextContent(
      "Subagent 'notes-reader' asks to read /tmp/qa-notes"
    );
    // The daemon's prompt is the terminal's wording of the same question: not drawn.
    expect(container).not.toHaveTextContent('developer__text_editor');
    expect(container).not.toHaveTextContent('External folder access');
    expect(screen.queryByTestId('tool-security-finding')).toBeNull();
    // Asked once.
    expect(container.textContent?.match(/asks to read/g)).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Allow for this session' })).toBeInTheDocument();
  });

  it('takes the subagent the daemon names on the card first (a card shown in the parent)', () => {
    show({ requestedBy: 'folder-reader' });
    expect(screen.getByTestId('folder-access-question')).toHaveTextContent(
      "Subagent 'folder-reader' asks to read /tmp/qa-notes"
    );
  });

  it('writes a folder under home with `~`, in the chat’s own voice when no subagent asks', () => {
    const { container } = show({
      folder: '/Users/someone/qa-notes',
      files: ['/Users/someone/qa-notes/a.txt'],
    });
    expect(screen.getByTestId('folder-access-question')).toHaveTextContent(
      'Allow this chat to read ~/qa-notes?'
    );
    expect(screen.getByTestId('folder-access-question').className).not.toMatch(/font-mono/);
    expect(container.querySelector('.font-mono')).toBeNull();
  });

  it('collapses to its outcome once answered, without the warning', async () => {
    const { container } = show({ requester: 'notes-reader' });
    fireEvent.click(screen.getByRole('button', { name: 'Allow for this session' }));
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Allow once' })).toBeNull());
    expect(container).toHaveTextContent('allowed for this session');
    expect(screen.queryByTestId('tool-security-finding')).toBeNull();
    expect(container).not.toHaveTextContent('External folder access');
    expect(container).not.toHaveTextContent('A session grant includes');
    expect(screen.queryByTestId('folder-access-preview')).toBeNull();
  });
});

describe('folderAsRequested', () => {
  it('reads the asked-for folder off a file the daemon resolved', () => {
    expect(
      folderAsRequested('/private/tmp/qa', [
        '/private/tmp/qa/a/b.txt (requested as /tmp/qa/a/b.txt)',
      ])
    ).toBe('/tmp/qa');
  });

  it('keeps the resolved folder when nothing lines up', () => {
    expect(folderAsRequested('/private/tmp/qa', ['/private/tmp/qa/b.txt'])).toBe('/private/tmp/qa');
    expect(
      folderAsRequested('/private/tmp/qa', [
        '/private/tmp/qa/b.txt (requested as /elsewhere/c.txt)',
      ])
    ).toBe('/private/tmp/qa');
    expect(
      folderAsRequested('/private/tmp/qa', ['/private/tmp/qaz/b.txt (requested as /tmp/qaz/b.txt)'])
    ).toBe('/private/tmp/qa');
  });

  it('says what each access lets the asker do', () => {
    expect(accessVerb('read-only')).toBe('read');
    expect(accessVerb('Read-only')).toBe('read');
    expect(accessVerb('read/write')).toBe('read and change');
  });
});
