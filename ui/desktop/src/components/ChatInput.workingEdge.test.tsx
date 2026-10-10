import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';

/**
 * The composer's working edge — the HOOK half.
 *
 * While a turn runs, the composer card carries `data-working="true"`, and the
 * authored CSS beside the focus rule in `main.css` turns that into a travelling
 * segment of the brand accent on the card's own 1px border. It replaced a row
 * above the composer whose breathing dot duplicated `TurnActivityIndicator`'s,
 * sat 8px out of line with it, and cost a ~34px layout shift on every Send.
 *
 * ⚠ **What this file can and cannot show.** jsdom has no layout engine, never
 * runs Tailwind, does not evaluate `:has()`, does not resolve `color-mix()` and
 * does not animate `@property`-registered custom properties. So nothing here
 * says anything about how the edge LOOKS — reading `borderColor` would return
 * the resting value in every case and pass whether the CSS exists or not.
 *
 * The split is deliberate:
 *  - THIS file pins the attribute: does it appear for exactly the states a turn
 *    is running in, and vanish when idle?
 *  - `styles/composerWorkingEdge.test.ts` pins the declarations it keys.
 *  - A real browser was used for the appearance; see that file's header.
 */

vi.mock('./ConfigContext', () => ({
  usePrivacyTiersEnabled: () => false,
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
// The pickers and the footer are WS-PICKERS' modules, mocked by path; their
// insides are tested where they live.
vi.mock('./bottom_menu/ToolsChip', () => ({ ToolsChip: () => null }));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/ComposerFooter', () => ({ ComposerFooter: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../toasts', () => ({
  toastWarning: vi.fn(),
  toastError: vi.fn(),
  toastInfo: vi.fn(),
  toastSuccess: vi.fn(),
  toastLoading: vi.fn(),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { ChatTabStrip } from './chatGroups/ChatTabStrip';

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    appConfig: {
      get: (key: string) => (key === 'BIOROUTER_WORKING_DIR' ? '/tmp/workdir' : undefined),
    },
    electron: {
      directoryChooser: vi.fn(async () => ({ canceled: true, filePaths: [] })),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

function renderComposer(chatState: ChatState) {
  const props = (state: ChatState) => (
    <ChatInput
      sessionId="session-under-test"
      handleSubmit={vi.fn()}
      chatState={state}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={2}
      disableAnimation
      sessionCosts={undefined}
      toolCount={0}
    />
  );
  const view = render(props(chatState));
  return { setChatState: (state: ChatState) => view.rerender(props(state)) };
}

/**
 * The card is found by the class the CSS keys off, not by a test id — so if the
 * class is ever renamed, this fails alongside the stylesheet rather than
 * quietly passing against an element the CSS no longer matches.
 */
const card = () => document.querySelector('.biorouter-composer-card');

describe('the composer working edge attribute', () => {
  it('is absent when the chat is idle', () => {
    renderComposer(ChatState.Idle);
    expect(card()).not.toBeNull();
    // Absent, not "false" — `[data-working]` is the whole test in CSS and e2e.
    expect(card()!.hasAttribute('data-working')).toBe(false);
  });

  /** The same live-turn states used by the running-tab registry. */
  const RUNNING = [
    ChatState.Thinking,
    ChatState.Streaming,
    ChatState.WaitingForUserInput,
    ChatState.Compacting,
    ChatState.RestartingAgent,
  ];

  it.each(RUNNING)('is set while the chat state is %s', (state) => {
    renderComposer(state);
    expect(card()!.getAttribute('data-working')).toBe('true');
  });

  it('does not report conversation loading as live work', () => {
    renderComposer(ChatState.LoadingConversation);
    expect(card()!.hasAttribute('data-working')).toBe(false);
  });

  it('appears when a turn starts and clears when it ends', () => {
    const { setChatState } = renderComposer(ChatState.Idle);
    expect(card()!.hasAttribute('data-working')).toBe(false);

    setChatState(ChatState.Streaming);
    expect(card()!.getAttribute('data-working')).toBe('true');

    setChatState(ChatState.Idle);
    expect(card()!.hasAttribute('data-working')).toBe(false);
  });

  /**
   * The edge rides the border of the ONE element that draws it. If the
   * attribute ever landed on the shell or on an inner row instead, the CSS
   * would match nothing and the indicator would silently disappear — with every
   * source-level assertion still green.
   */
  it('sits on the element that carries the border, not a wrapper', () => {
    renderComposer(ChatState.Streaming);
    const marked = document.querySelectorAll('[data-working="true"]');
    expect(marked).toHaveLength(1);
    expect(marked[0].classList.contains('biorouter-composer-card')).toBe(true);
  });

  /**
   * The card is `position: relative` in its own class list, and the sweep is an
   * absolutely-positioned `::after` on it. Without a positioned ancestor the
   * pseudo-element would escape to the nearest one and draw a ring around some
   * other box entirely.
   */
  it('keeps the positioning context the sweep is drawn against', () => {
    renderComposer(ChatState.Streaming);
    expect(card()!.classList.contains('relative')).toBe(true);
  });

  it('still renders the composer normally while working', () => {
    renderComposer(ChatState.Streaming);
    // The edge is decoration on an otherwise untouched composer: no layout
    // change, no disabled input, nothing removed.
    expect(screen.getByTestId('chat-input')).toBeTruthy();
  });

  it('shows the shared running tab marker and working composer for a delegated child', () => {
    renderComposer(ChatState.Streaming);
    render(
      <ChatTabStrip
        tabs={[
          {
            tabId: 'child-tab',
            sessionId: 'child-session',
            title: 'Delegated child',
            userSetName: false,
          },
        ]}
        activeTabId="child-tab"
        runningSessionIds={['child-session']}
        tabAnnotations={{
          'child-session': { badge: 'subagent', parentSessionId: 'parent-session' },
        }}
        onSelect={vi.fn()}
        onClose={vi.fn()}
        onReorder={vi.fn()}
        reserveTitlebar={false}
        isCompactSidebarOverlayOpen={false}
      />
    );

    expect(screen.getByTestId('chat-tab-running-child-tab')).toBeTruthy();
    expect(card()!.getAttribute('data-working')).toBe('true');
    expect(screen.getByRole('button', { name: 'Stop response' })).toBeTruthy();
  });
});
