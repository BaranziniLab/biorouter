/**
 * R1-6: in a decision chat, a message sent moments after the question was typed
 * waits for the question to be stored. Without the wait the message reached the
 * daemon first, and the chat had no question to answer it with.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
const model = vi.hoisted(() => ({ provider: 'ollama_decision', name: 'tev1:0.8b' }));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: model.name,
    currentProvider: model.provider,
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
vi.mock('./bottom_menu/ModelEffortChip', () => ({
  ModelEffortChip: () => <button data-testid="model-chip-stub">model</button>,
}));
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: () => <div data-testid="composer-footer" />,
}));
vi.mock('./MessageQueue', () => ({ default: () => null }));
// Records whether the `/` and `@` list is open, which is all these tests read of it.
const mentionList = vi.hoisted(() => ({ open: false, slash: false }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(
    (props: { isOpen: boolean; isSlashCommand: boolean }, _ref) => {
      mentionList.open = props.isOpen;
      mentionList.slash = props.isSlashCommand;
      return null;
    }
  );
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
const save = vi.hoisted(() => ({
  answer: null as null | ((value: { ok: true }) => void),
  calls: [] as unknown[],
}));
vi.mock('./decisions/decisionChatApi', () => ({
  fetchDecisionChatQuestion: vi.fn(async () => null),
  saveDecisionChatQuestion: vi.fn((_session: string, question: unknown) => {
    save.calls.push(question);
    return new Promise((done) => (save.answer = done));
  }),
}));
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';

beforeEach(() => {
  vi.clearAllMocks();
  save.answer = null;
  save.calls = [];
  mentionList.open = false;
  mentionList.slash = false;
  model.provider = 'ollama_decision';
  model.name = 'tev1:0.8b';
  Object.assign(window, {
    appConfig: { get: () => '/Users/wgu/Desktop' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(),
      on: vi.fn(),
      off: vi.fn(),
      saveDataUrlToTemp: vi.fn(),
      readTempImageAsBase64: vi.fn(),
      deleteTempFile: vi.fn(),
    },
  });
});

function renderComposer(handleSubmit: () => void) {
  return render(
    <ChatInput
      sessionId="chat-1"
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
}

describe('a quick send in a decision chat (R1-6)', () => {
  it('waits for the question typed just before it to be stored', async () => {
    const handleSubmit = vi.fn();
    renderComposer(handleSubmit);
    const question = await screen.findByTestId('decision-composer-question');
    await waitFor(() => expect(question).toBeEnabled());
    fireEvent.change(question, { target: { value: 'Does this note mention a fever?' } });
    const text = screen.getByTestId('chat-input') as HTMLTextAreaElement;
    fireEvent.change(text, { target: { value: 'Temp 39.2 overnight.', selectionStart: 20 } });
    fireEvent.keyDown(text, { key: 'Enter' });

    // The question's save started at the send, and the message waits for it.
    await waitFor(() => expect(save.calls).toHaveLength(1));
    await act(() => Promise.resolve());
    expect(handleSubmit).not.toHaveBeenCalled();
    // The box still clears at once.
    expect(text).toHaveValue('');

    await act(async () => save.answer?.({ ok: true }));
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    const event = handleSubmit.mock.calls[0][0] as CustomEvent<{ value: string }>;
    expect(event.detail.value).toBe('Temp 39.2 overnight.');
  });
});
