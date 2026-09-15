import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

/**
 * Cmd/Ctrl+Enter means ONE thing: "send what I have into the running turn."
 *
 * BR-61 bound the chord to the composer's text. It did nothing at all when the
 * composer was empty — which is exactly the state a user is in when the thing
 * they want to send is already sitting in the queue, one click away on "Add
 * now". So the empty-composer branch became that button's keyboard equivalent
 * rather than a second binding: the chord still has a single meaning, and which
 * message it picks up follows from what the user can see (composer text if
 * there is any, otherwise the front of the queue).
 *
 * These tests drive the REAL composer and the REAL queue, so the button and the
 * shortcut are checked against one shared eligibility predicate rather than two
 * that agree today. What they cannot say anything about is how any of it LOOKS:
 * jsdom has no layout engine and does not run Tailwind.
 */

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
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
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
vi.mock('../toasts', () => ({
  toastWarning: vi.fn(),
  toastError: vi.fn(),
  toastInfo: vi.fn(),
  toastSuccess: vi.fn(),
  toastLoading: vi.fn(),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { resetComposerQueuesForTests } from '../utils/composerQueues';

const QUEUED_TEXT = 'summarise the second table too';
const COMPOSER_TEXT = 'plot the residuals instead';

beforeEach(() => {
  vi.clearAllMocks();
  // Every test here mounts the same chat, and a composer unmounted at the end of
  // one test parks its queue for that chat, so the next would claim it.
  resetComposerQueuesForTests();
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
      readTempImageAsBase64: vi.fn(async () => ({ data: 'cGl4ZWxz', mimeType: 'image/png' })),
      deleteTempFile: vi.fn(),
    },
  });
});

const composer = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;

/**
 * How many messages the queue is currently holding.
 *
 * Collapsed it announces its own count; expanded it drops that label, so the
 * rows are counted by their drag handles. Reading only the collapsed label
 * would silently report an EXPANDED queue as empty.
 */
const queueLength = () => {
  const expander = screen.queryByRole('button', { name: /queued\. Expand queue\./i });
  if (expander) {
    return Number(/^(\d+) message/.exec(expander.getAttribute('aria-label') ?? '')?.[1] ?? 0);
  }
  return screen.queryAllByLabelText('Drag to reorder').length;
};

/** The chord, as the composer's own key handler sees it. */
const pressSteerChord = () =>
  fireEvent.keyDown(composer(), { key: 'Enter', metaKey: true, ctrlKey: true });

function renderComposer({
  onSteer,
  chatState = ChatState.Streaming,
}: {
  onSteer?: (text: string) => Promise<boolean>;
  chatState?: ChatState;
}) {
  const props = (state: ChatState) => (
    <ChatInput
      sessionId="session-under-test"
      handleSubmit={vi.fn()}
      chatState={state}
      onStop={vi.fn()}
      onSteer={onSteer}
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

/** Type a message while the turn is running, so it lands in the queue. */
async function queueOneMessage(text = QUEUED_TEXT) {
  fireEvent.change(composer(), { target: { value: text } });
  fireEvent.submit(composer().closest('form')!);
  await waitFor(() => expect(queueLength()).toBe(1));
}

describe('Cmd/Ctrl+Enter steers, and picks up whichever message the user can see', () => {
  it('steers the composer text when the composer has text', async () => {
    const onSteer = vi.fn(async () => true);
    renderComposer({ onSteer });
    await queueOneMessage();

    fireEvent.change(composer(), { target: { value: COMPOSER_TEXT } });
    pressSteerChord();

    await waitFor(() => expect(onSteer).toHaveBeenCalledWith(COMPOSER_TEXT));
    // The composer's own text wins outright: the queue is not touched, so the
    // chord can never send two messages or reorder what is waiting.
    expect(onSteer).toHaveBeenCalledTimes(1);
    expect(queueLength()).toBe(1);
    expect(composer().value).toBe('');
  });

  it('steers the front of the queue when the composer is empty', async () => {
    const onSteer = vi.fn(async () => true);
    renderComposer({ onSteer });
    await queueOneMessage();
    expect(composer().value).toBe('');

    pressSteerChord();

    await waitFor(() => expect(onSteer).toHaveBeenCalledWith(QUEUED_TEXT));
    // Steered, so it leaves the queue — the same hand-off the "Add now" button
    // performs, not a copy that would then be sent twice.
    await waitFor(() => expect(queueLength()).toBe(0));
  });

  it('takes the FRONT of the queue, not the last thing queued', async () => {
    const onSteer = vi.fn(async () => true);
    renderComposer({ onSteer });
    await queueOneMessage('first in line');
    fireEvent.change(composer(), { target: { value: 'second in line' } });
    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(queueLength()).toBe(2));

    pressSteerChord();

    await waitFor(() => expect(onSteer).toHaveBeenCalledWith('first in line'));
    await waitFor(() => expect(queueLength()).toBe(1));
  });

  it('does nothing when the composer is empty and the queue is empty', async () => {
    const onSteer = vi.fn(async () => true);
    renderComposer({ onSteer });
    expect(queueLength()).toBe(0);

    pressSteerChord();

    // A no-op, and specifically not a fall-through into an ordinary send: an
    // empty composer has nothing to send, and inventing a turn here would be a
    // keystroke doing something the user cannot see coming.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(onSteer).not.toHaveBeenCalled();
    expect(queueLength()).toBe(0);
  });

  it('does nothing once the turn has ended, even with the queue still full', async () => {
    const onSteer = vi.fn(async () => true);
    const { setChatState } = renderComposer({ onSteer });
    await queueOneMessage();

    // There is nothing to steer INTO once the turn is over. `canSteer` goes
    // false, the queue's "Add now" button disappears with it, and the chord has
    // to disappear too — the queue drains on its own from here.
    setChatState(ChatState.Idle);
    await waitFor(() =>
      expect(
        screen.queryByRole('button', { name: 'Add this message to the current turn' })
      ).toBeNull()
    );

    pressSteerChord();

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(onSteer).not.toHaveBeenCalled();
  });

  it('does not steer a queued message whose editor is open', async () => {
    const user = userEvent.setup();
    const onSteer = vi.fn(async () => true);
    renderComposer({ onSteer });
    await queueOneMessage();

    // The row's own "Add now" is disabled while its editor is open. The user can
    // click back into the composer with that editor still open, so the chord has
    // to refuse too — otherwise it steers the row out from under the edit.
    await user.click(screen.getByRole('button', { name: /queued\. Expand queue\./i }));
    await user.click(screen.getByText(QUEUED_TEXT));
    expect(
      screen.getByRole('button', { name: 'Add this message to the current turn' })
    ).toBeDisabled();

    pressSteerChord();

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(onSteer).not.toHaveBeenCalled();
    expect(queueLength()).toBe(1);
  });
});
