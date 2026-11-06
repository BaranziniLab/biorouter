/**
 * The first monitor's way in. The summary's Decisions section is drawn only once
 * a chat has a decision call or a monitor, so the composer's `+` menu carries a
 * "Decision monitors…" row that opens the chat's Monitors dialog. A chat that has
 * not been created yet has nothing to attach a monitor to, so the row is absent.
 */
import React from 'react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'versa_azure',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
vi.mock('./bottom_menu/ToolsChip', () => ({
  ToolsChip: () => <button data-testid="tools-chip-stub">Tools</button>,
}));
vi.mock('./bottom_menu/ModelEffortChip', () => ({
  ModelEffortChip: () => <button data-testid="model-chip-stub">model</button>,
}));
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: () => <div data-testid="composer-footer" />,
}));
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
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { COMPOSER_COPY } from './composer/copy';

beforeEach(() => {
  vi.clearAllMocks();
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

function renderComposer(sessionId: string | null, onOpenDecisionMonitors?: () => void) {
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
      messagesLength={0}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
      onOpenDecisionMonitors={onOpenDecisionMonitors}
    />
  );
}

async function openPlusMenu() {
  const user = userEvent.setup();
  await user.click(screen.getByTestId('composer-plus'));
  // The menu is open once its first row is there.
  await screen.findByRole('menuitem', { name: COMPOSER_COPY.plus.attachFile });
  return user;
}

describe("the composer's + menu: decision monitors", () => {
  it("opens the chat's Monitors dialog from a chat that exists", async () => {
    const onOpen = vi.fn();
    renderComposer('chat-1', onOpen);
    const user = await openPlusMenu();
    const row = screen.getByRole('menuitem', { name: COMPOSER_COPY.plus.decisionMonitors });
    await user.click(row);
    expect(onOpen).toHaveBeenCalledTimes(1);
    // Choosing it closes the menu.
    expect(screen.queryByRole('menuitem', { name: COMPOSER_COPY.plus.attachFile })).toBeNull();
  });

  it('is absent while the chat has not been created', async () => {
    const onOpen = vi.fn();
    renderComposer(null, onOpen);
    await openPlusMenu();
    expect(
      screen.queryByRole('menuitem', { name: COMPOSER_COPY.plus.decisionMonitors })
    ).toBeNull();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('is absent where nothing can open the dialog (Home)', async () => {
    renderComposer('chat-1');
    await openPlusMenu();
    expect(
      screen.queryByRole('menuitem', { name: COMPOSER_COPY.plus.decisionMonitors })
    ).toBeNull();
  });

  /**
   * BaseChat owns the Monitors dialog (mounted at chat level so the summary
   * popover closing cannot take it down), so the composer's row must reach that
   * one dialog, and only once the chat exists. jsdom cannot mount BaseChat whole,
   * so the wiring is pinned at its source.
   */
  it("is wired to BaseChat's one Monitors dialog, only for a chat that exists", () => {
    const source = readFileSync(join(__dirname, 'BaseChat.tsx'), 'utf8');
    expect(source).toMatch(/onOpenDecisionMonitors=\{sessionId \? openMonitors : undefined\}/);
    expect(source).toMatch(
      /const openMonitors = useCallback\(\(\) => \{[^}]*setMonitorsOpen\(true\)/
    );
  });
});
