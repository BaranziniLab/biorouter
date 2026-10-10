import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

// #39 — the pre-session working-directory wiring contract. With no sessionId,
// a DirSwitcher change has nothing to persist to; the ONLY thing keeping the
// user's choice alive is ChatInput forwarding it through its
// onWorkingDirChange prop (Hub threads it into createSession; BaseChat keeps
// it as pendingWorkingDir for the pre-session createSession). This test pins
// that contract at the ChatInput level.

// Heavy or context-hungry children that are irrelevant to the wiring under
// test. DirSwitcher itself stays REAL — its no-session branch is the contract.
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: null,
    currentProvider: null,
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
// The pickers are WS-PICKERS' modules, mocked by path. The footer is real:
// the folder chip it holds is what this suite drives.
vi.mock('./bottom_menu/ToolsChip', () => ({ ToolsChip: () => null }));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/CostTracker', () => ({
  CostTracker: () => null,
}));
vi.mock('./MessageQueue', () => ({
  default: () => null,
}));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { updateWorkingDir } from '../api';

const DEFAULT_DIR = '/default/workdir';
const CHOSEN_DIR = '/Users/wgu/Desktop/data';

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    appConfig: {
      get: (key: string) => (key === 'BIOROUTER_WORKING_DIR' ? DEFAULT_DIR : undefined),
    },
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: false, filePaths: [CHOSEN_DIR] })),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

function renderChatInput(
  onWorkingDirChange: (dir: string) => void,
  {
    sessionId = null as string | null,
    messagesLength = 0,
    workingDirLocked = undefined as boolean | undefined,
  } = {}
) {
  return render(
    <ChatInput
      sessionId={sessionId}
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={messagesLength}
      workingDirLocked={workingDirLocked}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={onWorkingDirChange}
    />
  );
}

describe('ChatInput pre-session working-directory wiring (#39)', () => {
  it('forwards a DirSwitcher change to onWorkingDirChange when there is no session', async () => {
    const onWorkingDirChange = vi.fn();
    renderChatInput(onWorkingDirChange);

    // The folder chip names the app default before any choice is made: the
    // footer shows the folder's name, and the chooser's name carries the path.
    const chip = screen.getByRole('button', { name: `Working folder: ${DEFAULT_DIR}` });
    expect(chip).toHaveTextContent('workdir');
    fireEvent.click(chip);

    await waitFor(() => expect(onWorkingDirChange).toHaveBeenCalledWith(CHOSEN_DIR));

    // No session — nothing may be persisted server-side yet.
    expect(updateWorkingDir).not.toHaveBeenCalled();
    // The chip reflects the choice locally (ChatInput's own state).
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: `Working folder: ${CHOSEN_DIR}` })
      ).toHaveTextContent('data')
    );
  });
});

// #44 — once the chat has messages, ChatInput must render the folder chip
// locked: a read-only basename label (full path on hover) with no chooser.
describe('ChatInput working-directory lock once the chat has messages (#44)', () => {
  it('renders the chip as a read-only basename label when messagesLength > 0', async () => {
    renderChatInput(vi.fn(), { sessionId: 'session-1', messagesLength: 2 });

    // Basename of the app-default dir ('/default/workdir' -> 'workdir'),
    // never the interactive full-path chooser button.
    const label = await screen.findByTestId('dir-switcher-locked');
    expect(label).toHaveTextContent('workdir');
    expect(screen.queryByText(DEFAULT_DIR)).not.toBeInTheDocument();

    fireEvent.click(label);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(updateWorkingDir).not.toHaveBeenCalled();
  });

  it('keeps the chip interactive for a session with zero messages', () => {
    renderChatInput(vi.fn(), { sessionId: 'session-1', messagesLength: 0 });

    expect(screen.getByRole('button', { name: `Working folder: ${DEFAULT_DIR}` })).toBeEnabled();
    expect(screen.queryByTestId('dir-switcher-locked')).not.toBeInTheDocument();
  });
});

// #44 — the authoritative lock prop. `messagesLength` alone is 0 while a
// resumed transcript hydrates and >0 after a failed optimistic first submit,
// so BaseChat derives `workingDirLocked` from session metadata
// (deriveWorkingDirLocked) and, when provided, it must win over the
// messagesLength fallback in BOTH directions.
describe('ChatInput authoritative working-dir lock (#44)', () => {
  it('locks from first paint while a resumed transcript is still hydrating', async () => {
    // The store has not loaded the messages yet (messagesLength 0), but the
    // session metadata says the chat is non-empty.
    renderChatInput(vi.fn(), {
      sessionId: 'session-1',
      messagesLength: 0,
      workingDirLocked: true,
    });

    expect(await screen.findByTestId('dir-switcher-locked')).toBeInTheDocument();
  });

  it('unlocks after a failed first submit whose optimistic message never reached the server', () => {
    // The transcript retains the unsent message (messagesLength 1), but the
    // server still reports an empty session.
    renderChatInput(vi.fn(), {
      sessionId: 'session-1',
      messagesLength: 1,
      workingDirLocked: false,
    });

    expect(screen.queryByTestId('dir-switcher-locked')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: `Working folder: ${DEFAULT_DIR}` })).toBeEnabled();
  });
});
