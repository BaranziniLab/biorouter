import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';

/**
 * Which composer takes the caret when several mount at once.
 *
 * Every split pane's composer mounts in the same commit when /pair is rebuilt
 * (coming back from Settings or Home), and each one used to focus itself. The
 * last to mount won, and because a focus inside a pane makes that pane the
 * focused one, it won the focus as well. Measured in the dev app: coming back
 * with New chat resumed the draft showing in the left pane, and the right pane's
 * chat ended up focused, with the caret in its composer.
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
    currentModelSupportsVision: true,
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

function Pane({ label, autoFocus }: { label: string; autoFocus?: boolean }) {
  return (
    <section aria-label={label}>
      <ChatInput
        sessionId=""
        handleSubmit={async () => true}
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
        autoFocus={autoFocus}
      />
    </section>
  );
}

const box = (label: string) =>
  within(screen.getByRole('region', { name: label })).getByRole('textbox');

describe('a composer takes the caret when it mounts only if asked to', () => {
  it('takes it by default, as every single composer always has', () => {
    render(<Pane label="only" />);
    expect(document.activeElement).toBe(box('only'));
  });

  it('in a split, the focused pane keeps the caret although another pane mounts after it', () => {
    render(
      <>
        <Pane label="focused" autoFocus />
        <Pane label="background" autoFocus={false} />
      </>
    );
    expect(document.activeElement).toBe(box('focused'));
  });

  it('a later change to the prop does not move the caret', () => {
    const view = render(
      <>
        <Pane label="focused" autoFocus />
        <Pane label="background" autoFocus={false} />
      </>
    );
    view.rerender(
      <>
        <Pane label="focused" autoFocus={false} />
        <Pane label="background" autoFocus />
      </>
    );
    expect(document.activeElement).toBe(box('focused'));
  });
});
