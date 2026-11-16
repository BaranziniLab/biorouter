import React from 'react';
import { describe, expect, it, vi, beforeEach, type Mock } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

// A message that starts with an absolute path is not a command (W1-03): `/Users/wgu/x` opened the
// command list with the query "Users/wgu/x", which matched nothing, and Enter on that list did
// nothing at all, so the message could not be sent without quoting the path.

vi.mock('../toasts', () => ({
  toastWarning: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastInfo: vi.fn(),
  toastService: { error: vi.fn(), configure: vi.fn() },
}));
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: 'gpt-5.5', provider: 'versa_azure' })),
    currentModel: 'gpt-5.5',
    currentProvider: 'versa_azure',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
vi.mock('./bottom_menu/toolsLists', () => ({
  useToolsLists: () => ({ shown: null, clearQueries: () => {} }),
  ToolsListPanel: () => null,
}));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/ComposerFooter', () => ({ ComposerFooter: () => null }));
vi.mock('./MessageQueue', () => ({
  default: () => null,
  canSteerMessage: () => false,
}));

// The popover is replaced by a stand-in that shows whether it is open, for which trigger and
// query, and answers the composer's two questions (what it lists, pick a row) the way the real
// one does. Its insides (matching and loading) are tested where they live.
const popover = vi.hoisted(() => ({
  rows: [] as Array<{ name: string }>,
  selectFile: vi.fn(),
}));
vi.mock('./MentionPopover', async () => {
  const actual = await vi.importActual<typeof import('./MentionPopover')>('./MentionPopover');
  const MentionPopoverMock = React.forwardRef(
    (props: { isOpen: boolean; isSlashCommand: boolean; query: string }, ref) => {
      React.useImperativeHandle(ref, () => ({
        getDisplayFiles: () => popover.rows,
        selectFile: popover.selectFile,
      }));
      return props.isOpen ? (
        <div
          data-testid="mention-popover"
          data-slash={String(props.isSlashCommand)}
          data-query={props.query}
        />
      ) : null;
    }
  );
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { ...actual, default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { resetComposerDraftsForTests } from '../utils/composerDrafts';

beforeEach(() => {
  vi.clearAllMocks();
  popover.rows = [];
  resetComposerDraftsForTests();
  Object.assign(window, {
    appConfig: {
      get: (key: string) =>
        key === 'BIOROUTER_WORKING_DIR' ? '/tmp/biorouter-workdir' : undefined,
    },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      deleteTempFile: vi.fn(),
    },
  });
});

type SubmitFn = (event: React.FormEvent) => void | Promise<boolean | void>;
type SubmitMock = Mock<SubmitFn>;

const renderComposer = () => {
  const handleSubmit = vi.fn<SubmitFn>(async () => true);
  render(
    <ChatInput
      sessionId="session-42"
      handleSubmit={handleSubmit}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
  return handleSubmit;
};

const composer = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;
const list = () => screen.queryByTestId('mention-popover');
const submitted = (handleSubmit: SubmitMock) => {
  const calls = handleSubmit.mock.calls;
  return (calls[calls.length - 1][0] as unknown as CustomEvent).detail.value as string;
};

/** Type into the textarea the way the browser reports it: value and caret. */
const typeProse = (value: string, caret = value.length) => {
  fireEvent.change(composer(), { target: { value, selectionStart: caret, selectionEnd: caret } });
};
const pressEnter = () => fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter' });

describe('a path at the start of a message', () => {
  it('a message that starts with an absolute path sends on Enter', async () => {
    const handleSubmit = renderComposer();
    typeProse('/Users/wgu/x');
    expect(list()).toBeNull();

    pressEnter();
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('/Users/wgu/x');
    expect(popover.selectFile).not.toHaveBeenCalled();
  });

  it('a path after other words opens no list either', () => {
    renderComposer();
    typeProse('please read /Users/wgu/notes.md');
    expect(list()).toBeNull();
  });

  it('/compact still opens the command list, and Enter picks its row', () => {
    const handleSubmit = renderComposer();
    popover.rows = [{ name: 'compact' }];
    typeProse('/compact');
    expect(list()).toHaveAttribute('data-slash', 'true');
    expect(list()).toHaveAttribute('data-query', 'compact');

    pressEnter();
    expect(popover.selectFile).toHaveBeenCalledWith(0);
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  it('an @ mention may still name a folder path', () => {
    renderComposer();
    typeProse('@src/components');
    expect(list()).toHaveAttribute('data-slash', 'false');
    expect(list()).toHaveAttribute('data-query', 'src/components');
  });
});

describe('Enter on a list with nothing in it', () => {
  it('closes the list and sends what was typed', async () => {
    const handleSubmit = renderComposer();
    typeProse('/nosuchcommand');
    expect(list()).toHaveAttribute('data-query', 'nosuchcommand');

    pressEnter();
    await waitFor(() => expect(handleSubmit).toHaveBeenCalled());
    expect(submitted(handleSubmit)).toBe('/nosuchcommand');
    expect(popover.selectFile).not.toHaveBeenCalled();
    expect(list()).toBeNull();
  });

  it('Shift+Enter still adds a line instead of sending', () => {
    const handleSubmit = renderComposer();
    typeProse('/nosuchcommand');
    fireEvent.keyDown(composer(), { key: 'Enter', code: 'Enter', shiftKey: true });
    expect(handleSubmit).not.toHaveBeenCalled();
  });
});
