/**
 * The composer with no provider bound — the honest other half of letting a user
 * into the app before setup ("Explore Biorouter first →" in `ProviderGuard`).
 *
 * ⚠ **The hint and the send guard are one decision, and must stay one.** A hint
 * over a composer that still submits produces the daemon's own error toast,
 * written for a developer, at the moment a first-run user is least equipped to
 * read it — which is precisely the failure the first-run wall existed to avoid,
 * reintroduced by the escape from it.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

// Heavy children that are irrelevant here — the same scaffold the sibling
// ChatInput suites use, so this one exercises the real composer rather than a
// stub of it.
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
const model = vi.hoisted(() => ({
  currentProvider: null as string | null,
  modelConfigStatus: 'ready' as 'loading' | 'ready',
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: null,
    currentProvider: model.currentProvider,
    modelConfigStatus: model.modelConfigStatus,
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({ default: () => null }));
vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  BottomMenuExtensionSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({
  BottomMenuSkillSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  BottomMenuKnowledgeSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuReasoningEffort', () => ({
  BottomMenuReasoningEffort: () => null,
}));
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
  getSession: vi.fn(async () => ({ data: { privacy_tier: 'public' } })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';

beforeEach(() => {
  vi.clearAllMocks();
  model.currentProvider = null;
  model.modelConfigStatus = 'ready';
  Object.assign(window, {
    appConfig: { get: () => '/default/workdir' },
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

const setView = vi.fn();
const handleSubmit = vi.fn();

function renderComposer() {
  return render(
    <ChatInput
      sessionId="chat-a"
      handleSubmit={handleSubmit}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue="hello"
      setView={setView}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
}

describe('the composer with no model configured', () => {
  it('says why, above the input, with the way out', async () => {
    renderComposer();
    const hint = await screen.findByTestId('composer-no-model-hint');
    expect(hint).toHaveTextContent('No model yet');
    fireEvent.click(screen.getByTestId('composer-no-model-action'));
    expect(setView).toHaveBeenCalledWith('ConfigureProviders');
  });

  /**
   * ⚠ Both halves: the button is disabled AND the Enter path refuses. Guarding
   * only the button leaves the keyboard route — which is how the composer is
   * actually used — sending into a daemon with nothing bound.
   */
  it('refuses to send, by button and by Enter alike', async () => {
    renderComposer();
    await screen.findByTestId('composer-no-model-hint');

    const textarea = screen.getByRole('textbox');
    fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter' });
    expect(handleSubmit).not.toHaveBeenCalled();

    const send = document.querySelector('button[type="submit"]');
    expect(send).toBeDisabled();
  });

  it('leaves a configured composer alone', async () => {
    model.currentProvider = 'versa_azure';
    renderComposer();
    await waitFor(() => expect(screen.queryByTestId('composer-no-model-hint')).toBeNull());
    const textarea = screen.getByRole('textbox');
    fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter' });
    expect(handleSubmit).toHaveBeenCalled();
  });

  /**
   * ⚠ The state a `!currentProvider` check gets wrong: until the config is read,
   * every install looks unconfigured. A hint there — and a disabled Send —
   * would greet a perfectly working Versa user on every launch.
   */
  it('says nothing while the config is still loading', async () => {
    model.modelConfigStatus = 'loading';
    renderComposer();
    await waitFor(() => expect(screen.queryByTestId('composer-no-model-hint')).toBeNull());
  });
});
