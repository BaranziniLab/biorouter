import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react';

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
// The chat's row as `GET /sessions/{id}` answers it: its folder is SESSION_DIR,
// which is never the app default, so a chip that fell back to the default
// cannot pass for one that read the row.
vi.mock('../api', () => ({
  getSession: vi.fn(async ({ path }: { path: { session_id: string } }) => ({
    data: { id: path.session_id, working_dir: '/Users/wgu/Projects/kdps' },
  })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { getSession, updateWorkingDir, type Session } from '../api';
import { clearSessionListCache, updateCachedSessionList } from '../utils/sessionListCache';

const DEFAULT_DIR = '/default/workdir';
const CHOSEN_DIR = '/Users/wgu/Desktop/data';
const SESSION_DIR = '/Users/wgu/Projects/kdps';

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  // A test's held read must not outlive it: `clearAllMocks` keeps implementations.
  vi.mocked(getSession).mockImplementation((async ({ path }: { path: { session_id: string } }) => ({
    data: { id: path.session_id, working_dir: SESSION_DIR },
  })) as never);
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

function chatInput(
  onWorkingDirChange: (dir: string) => void,
  {
    sessionId = null as string | null,
    messagesLength = 0,
    workingDirLocked = undefined as boolean | undefined,
  } = {}
) {
  return (
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
      toolCount={0}
      onWorkingDirChange={onWorkingDirChange}
    />
  );
}

function renderChatInput(
  onWorkingDirChange: (dir: string) => void,
  options: Parameters<typeof chatInput>[1] = {}
) {
  return render(chatInput(onWorkingDirChange, options));
}

/**
 * Holds every read of one chat's row until the test answers it. The composer
 * reads the row twice (its folder, and its privacy tier), so holding only the
 * next call could hold the wrong one.
 */
function holdRowRead(sessionId: string) {
  let answer: ((dir: string) => void) | undefined;
  const held = new Promise<string>((resolve) => {
    answer = resolve;
  });
  vi.mocked(getSession).mockImplementation((async ({ path }: { path: { session_id: string } }) => {
    const dir = path.session_id === sessionId ? await held : SESSION_DIR;
    return { data: { id: path.session_id, working_dir: dir } };
  }) as never);
  return (dir: string) => act(async () => answer?.(dir));
}

/** Every folder name the footer has shown, in order ('' for none). */
function recordFolders(): string[] {
  const footer = screen.getByTestId('composer-footer');
  const seen: string[] = [];
  const note = () => {
    const shown = footer.querySelector('.br-footline__name')?.textContent ?? '';
    if (seen[seen.length - 1] !== shown) seen.push(shown);
  };
  note();
  new MutationObserver(note).observe(footer, {
    childList: true,
    subtree: true,
    characterData: true,
  });
  return seen;
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

    // Basename of the chat's own folder ('/Users/wgu/Projects/kdps' -> 'kdps'),
    // never the interactive full-path chooser button.
    const label = await screen.findByTestId('dir-switcher-locked');
    expect(label).toHaveTextContent('kdps');
    expect(screen.queryByText(SESSION_DIR)).not.toBeInTheDocument();

    fireEvent.click(label);
    expect(window.electron.directoryChooser).not.toHaveBeenCalled();
    expect(updateWorkingDir).not.toHaveBeenCalled();
  });

  it('keeps the chip interactive for a session with zero messages', async () => {
    renderChatInput(vi.fn(), { sessionId: 'session-1', messagesLength: 0 });

    expect(
      await screen.findByRole('button', { name: `Working folder: ${SESSION_DIR}` })
    ).toBeEnabled();
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

  it('unlocks after a failed first submit whose optimistic message never reached the server', async () => {
    // The transcript retains the unsent message (messagesLength 1), but the
    // server still reports an empty session.
    renderChatInput(vi.fn(), {
      sessionId: 'session-1',
      messagesLength: 1,
      workingDirLocked: false,
    });

    expect(
      await screen.findByRole('button', { name: `Working folder: ${SESSION_DIR}` })
    ).toBeEnabled();
    expect(screen.queryByTestId('dir-switcher-locked')).not.toBeInTheDocument();
  });
});

// V2-R7-3 — the folder states the chat's own working directory or nothing. It
// read the app default ("home") on every chat switch until the row arrived:
// 137 to 360 ms on most chats, 3.2 s on a long one.
describe('ChatInput folder for a chat that exists (V2-R7-3)', () => {
  it('names no folder until the chat’s row answers, then the chat’s folder once', async () => {
    const answerRow = holdRowRead('session-1');
    renderChatInput(vi.fn(), { sessionId: 'session-1', messagesLength: 2 });
    const folders = recordFolders();

    expect(screen.queryByTestId('dir-switcher-locked')).not.toBeInTheDocument();
    expect(screen.queryByTestId('dir-switcher')).not.toBeInTheDocument();
    expect(screen.getByTestId('composer-footer')).not.toHaveTextContent('workdir');

    await answerRow(SESSION_DIR);
    expect(await screen.findByTestId('dir-switcher-locked')).toHaveTextContent('kdps');
    expect(folders).toEqual(['', 'kdps']);
  });

  it('names the folder in the first frame when the session listing holds the chat', async () => {
    updateCachedSessionList([{ id: 'session-1', working_dir: SESSION_DIR } as Session]);
    const answerRow = holdRowRead('session-1');
    renderChatInput(vi.fn(), { sessionId: 'session-1', messagesLength: 2 });
    const folders = recordFolders();

    expect(screen.getByTestId('dir-switcher-locked')).toHaveTextContent('kdps');
    await answerRow(SESSION_DIR);
    expect(folders).toEqual(['kdps']);
  });

  it('never names the previous chat’s folder under the next one', async () => {
    const view = renderChatInput(vi.fn(), { sessionId: 'chat-a', messagesLength: 2 });
    expect(await screen.findByTestId('dir-switcher-locked')).toHaveTextContent('kdps');
    const folders = recordFolders();

    // The tab keeps its mount and binds another chat, whose row is slow.
    const answerRow = holdRowRead('chat-b');
    view.rerender(chatInput(vi.fn(), { sessionId: 'chat-b', messagesLength: 2 }));
    await act(async () => {});
    expect(screen.queryByTestId('dir-switcher-locked')).not.toBeInTheDocument();

    await answerRow('/Users/wgu/Downloads');
    expect(screen.getByTestId('dir-switcher-locked')).toHaveTextContent('Downloads');
    expect(folders).toEqual(['kdps', '', 'Downloads']);
  });

  it('keeps the folder a new tab showed when its first message creates the chat', async () => {
    const onWorkingDirChange = vi.fn();
    const view = renderChatInput(onWorkingDirChange);
    fireEvent.click(screen.getByRole('button', { name: `Working folder: ${DEFAULT_DIR}` }));
    await waitFor(() => expect(onWorkingDirChange).toHaveBeenCalledWith(CHOSEN_DIR));
    await screen.findByRole('button', { name: `Working folder: ${CHOSEN_DIR}` });
    const folders = recordFolders();

    // The chat is created in the chosen folder (`pendingWorkingDir`), and the
    // tab keeps its mount: the folder stays while the new row is read.
    const answerRow = holdRowRead('session-new');
    view.rerender(chatInput(onWorkingDirChange, { sessionId: 'session-new', messagesLength: 1 }));
    await act(async () => {});
    expect(screen.getByTestId('composer-footer')).toHaveTextContent('data');

    await answerRow(CHOSEN_DIR);
    expect(folders).toEqual(['data']);
  });
});
