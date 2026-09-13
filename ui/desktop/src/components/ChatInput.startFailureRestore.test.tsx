import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';

/**
 * A chat that failed to start says so in a toast that ends "Your message was
 * kept." — and on the surface where people start most of their chats, a fresh
 * tab's composer, the message was NOT kept.
 *
 * Measured in the dev app on 1.90.4 (2026-09-12) with `POST /agent/start`
 * answering 500: the toast appeared, the composer stayed on "New chat", and the
 * box was empty. The give-back is a `restore-chat-input` window event, which
 * reaches only a composer that is listening at that instant — and this one is
 * not, because the same failure replaces it. `BaseChat` renders its composer in
 * two places (the centred empty state and the bar under a transcript) and
 * `isCreatingSession` moves it between them, so flipping that flag back on
 * failure REMOUNTS the composer; the instance that took the message was
 * discarded one task later, never having painted it.
 *
 * The remount is what these tests reproduce: the composer that submits is
 * unmounted and its replacement mounts. Nothing here mocks the give-back — it
 * is `BaseChat`'s own exported `handleCreateSessionError` writing to the real
 * `ChatInput`.
 */

vi.mock('../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastWarning: vi.fn(),
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
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: null,
    currentProvider: null,
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({ default: () => null }));
vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  BottomMenuExtensionSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({ BottomMenuSkillSelection: () => null }));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  BottomMenuKnowledgeSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuReasoningEffort', () => ({
  BottomMenuReasoningEffort: () => null,
}));
vi.mock('./bottom_menu/CostTracker', () => ({ CostTracker: () => null }));
vi.mock('./MessageQueue', () => ({ default: () => null }));
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
import { handleCreateSessionError } from './BaseChat';
import { ChatState } from '../types/chatState';

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    appConfig: { get: () => '/w' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

/** A fresh tab's composer: no session yet, which is why `sessionId` is ''. */
const renderComposer = (sessionId = '') =>
  render(
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
      messagesLength={0}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );

const composerText = () => (screen.getByRole('textbox') as HTMLTextAreaElement).value;

const startFails = (sessionId: string | null, text: string) =>
  act(() => {
    handleCreateSessionError(new Error('HTTP 500 Internal Server Error'), {
      textValue: text,
      attachments: [],
      sessionId,
    });
  });

describe('a chat that failed to start, on the composer the user is looking at', () => {
  it('gives the message back to the composer that replaces the one that submitted', () => {
    const first = renderComposer();
    startFails('', 'analyze my cohort');
    // The composer that submitted does take it — that part was never broken.
    expect(composerText()).toBe('analyze my cohort');

    // ...and is then thrown away, which is what the failure does on this
    // surface. Its replacement is what the person is actually looking at.
    first.unmount();
    renderComposer();

    expect(composerText()).toBe('analyze my cohort');
  });

  it('gives it back even when no composer was listening at the moment it failed', () => {
    // The same remount, one task earlier: nothing is mounted when the start
    // fails. The event alone reaches nobody at all here.
    startFails('', 'keep me');
    renderComposer();

    expect(composerText()).toBe('keep me');
  });

  it('survives the SECOND rebuild too, so the last composer standing has it', () => {
    // Measured in the dev app: the failure rebuilds the composer twice, at
    // +19ms and +21ms. A give-back the first replacement consumed left the
    // second — the one actually on screen — empty, which is this same bug
    // wearing a different hat.
    const first = renderComposer();
    startFails('', 'twice rebuilt');
    first.unmount();

    const second = renderComposer();
    expect(composerText()).toBe('twice rebuilt');
    second.unmount();

    renderComposer();
    expect(composerText()).toBe('twice rebuilt');
  });

  it('leaves a composer for a different chat alone', () => {
    startFails('', 'meant for the new tab');
    renderComposer('session-7');

    expect(composerText()).toBe('');
  });

  it('does not resurrect the message in some later "New chat"', async () => {
    startFails('', 'said once');
    // Nobody came back for it within the task the give-back lives for.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    renderComposer();

    expect(composerText()).toBe('');
  });
});
