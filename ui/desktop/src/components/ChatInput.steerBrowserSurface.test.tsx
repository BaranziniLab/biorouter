import React from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

/**
 * SD-8 — the queue's "Add now" on a `biorouter serve` session.
 *
 * Measured on a real serve host, 2026-09-12 (daemon at 127.0.0.1:18783, seeded
 * `versa_azure`/`gpt-5.5`): with a turn in flight and one message queued,
 * clicking "Add now" posted `/interrupt` and took
 *
 *     403 {"message":"This daemon was started without a user-action key, … Stop
 *          the turn and send the message instead, or use the desktop app."}
 *
 * The queue row's text was byte-identical before and after the click —
 * `"Next\n\nSTEER TEST alpha\n\nAdd now\n→\nStop & send"` both times — with no
 * toast and no inline message anywhere on the page. The only trace was a
 * `console.warn`. A second click posted `/interrupt` AGAIN and took the same
 * 403, so the control was not merely inert: it re-asked, forever, a daemon that
 * can never say yes, and stayed silent each time.
 *
 * The refusal is permanent and knowable in advance, which is why these tests
 * ask about the state BEFORE the click. `userActionHeaders()` emits
 * `X-User-Action` on the desktop only (`utils/userAction.surface.test.ts` pins
 * both halves), and `reply.rs`'s `steer_refusal` admits `Proven` and nothing
 * else — so on a browser surface every `/interrupt` is refused by every daemon,
 * whether or not it holds a key.
 *
 * These drive the REAL composer and the REAL queue, so the button, the note and
 * the Cmd/Ctrl+Enter chord are checked against the one predicate rather than
 * three that agree today. What they cannot say is how any of it LOOKS: jsdom
 * has no layout engine and does not run Tailwind.
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
import { BROWSER_SURFACE_MARKER } from '../utils/surface';
import { resetComposerQueuesForTests } from '../utils/composerQueues';

const QUEUED_TEXT = 'summarise the second table too';
const ADD_NOW = 'Add this message to the current turn';
const STOP_AND_SEND = 'Stop the current turn and send this message as a new turn';

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

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

const servedInABrowser = () => {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
};

const composer = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;

const queueLength = () => {
  const expander = screen.queryByRole('button', { name: /queued\. Expand queue\./i });
  if (expander) {
    return Number(/^(\d+) message/.exec(expander.getAttribute('aria-label') ?? '')?.[1] ?? 0);
  }
  return screen.queryAllByLabelText('Drag to reorder').length;
};

const pressSteerChord = () =>
  fireEvent.keyDown(composer(), { key: 'Enter', metaKey: true, ctrlKey: true });

function renderComposer(onSteer: (text: string) => Promise<boolean>) {
  render(
    <ChatInput
      sessionId="session-under-test"
      handleSubmit={vi.fn()}
      chatState={ChatState.Streaming}
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
}

/** Type a message while the turn is running, so it lands in the queue. */
async function queueOneMessage(text = QUEUED_TEXT) {
  fireEvent.change(composer(), { target: { value: text } });
  fireEvent.submit(composer().closest('form')!);
  await waitFor(() => expect(queueLength()).toBe(1));
}

describe('the queue explains the steer it cannot perform, on a browser surface', () => {
  /**
   * ⚠ **Fails against today's code**: the button is rendered exactly as on the
   * desktop and there is no note to find, which is the measured defect.
   */
  it('offers the note instead of the button, and names the control that does work', async () => {
    servedInABrowser();
    const onSteer = vi.fn(async () => true);
    renderComposer(onSteer);
    await queueOneMessage();

    expect(screen.queryByRole('button', { name: ADD_NOW })).toBeNull();

    const note = await screen.findByTestId('steer-unavailable-note');
    // The daemon's own useful half — "stop the turn and send the message
    // instead" — pointed at the button sitting beside the missing one, rather
    // than at a desktop application the reader may not have.
    expect(note.textContent).toMatch(/Stop & send/);
    expect(screen.getByRole('button', { name: STOP_AND_SEND })).toBeInTheDocument();
  });

  /**
   * The defect's second half, and the one that made the control look like a
   * working button: a click that neither acts nor speaks, repeatable forever.
   *
   * ⚠ Fails against today's code, where the button exists to be clicked.
   */
  it('leaves no "Add now" to click a second time', async () => {
    servedInABrowser();
    const onSteer = vi.fn(async () => true);
    renderComposer(onSteer);
    await queueOneMessage();

    expect(screen.queryAllByRole('button', { name: ADD_NOW })).toHaveLength(0);
    expect(onSteer).not.toHaveBeenCalled();
  });

  /** ⚠ Fails against today's code: the expanded row carries its own button. */
  it('says it once for the whole queue, not once per row', async () => {
    servedInABrowser();
    const user = userEvent.setup();
    renderComposer(vi.fn(async () => true));
    await queueOneMessage('first in line');
    fireEvent.change(composer(), { target: { value: 'second in line' } });
    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(queueLength()).toBe(2));

    await user.click(screen.getByRole('button', { name: /queued\. Expand queue\./i }));

    expect(screen.queryAllByRole('button', { name: ADD_NOW })).toHaveLength(0);
    expect(screen.getAllByTestId('steer-unavailable-note')).toHaveLength(1);
  });

  /**
   * The chord is the same capability bound to a key, and the tooltip on Send
   * advertises it as "adds it to the running turn". Left alone it would queue
   * the message while claiming to have steered — the same silent lie in a
   * second place.
   *
   * ⚠ Fails against today's code, which calls `onSteer` and then falls back.
   */
  it('does not steer on Cmd/Ctrl+Enter, and still never drops the words', async () => {
    servedInABrowser();
    const onSteer = vi.fn(async () => true);
    renderComposer(onSteer);

    fireEvent.change(composer(), { target: { value: 'plot the residuals instead' } });
    pressSteerChord();

    await waitFor(() => expect(queueLength()).toBe(1));
    expect(onSteer).not.toHaveBeenCalled();
  });

  /**
   * The note has nothing to explain when no turn is running: "Add now" is
   * absent there on every surface, and a note under an idle agent would be the
   * app apologising for a control the user was never offered.
   */
  it('says nothing while the queue waits on an idle agent', async () => {
    servedInABrowser();
    render(
      <ChatInput
        sessionId="session-under-test"
        handleSubmit={vi.fn()}
        chatState={ChatState.Idle}
        onStop={vi.fn()}
        onSteer={vi.fn(async () => true)}
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

    expect(screen.queryByTestId('steer-unavailable-note')).toBeNull();
  });
});

describe('the desktop is untouched', () => {
  /**
   * SD-8 is about the browser. Steering works on the desktop — the renderer
   * mints and attaches `X-User-Action` — so a note there would be telling the
   * user that a control they can watch working does not work.
   *
   * This passes BEFORE and AFTER the change, deliberately: it is the assertion
   * that the fix stayed on its own side of the line.
   */
  it('keeps the button and shows no note when no browser marker is set', async () => {
    const onSteer = vi.fn(async () => true);
    renderComposer(onSteer);
    await queueOneMessage();

    expect(screen.getByRole('button', { name: ADD_NOW })).toBeInTheDocument();
    expect(screen.queryByTestId('steer-unavailable-note')).toBeNull();
  });

  /** The chord still steers, and still takes the composer's text first. */
  it('still steers on Cmd/Ctrl+Enter', async () => {
    const onSteer = vi.fn(async () => true);
    renderComposer(onSteer);
    await queueOneMessage();

    fireEvent.change(composer(), { target: { value: 'plot the residuals instead' } });
    pressSteerChord();

    await waitFor(() => expect(onSteer).toHaveBeenCalledWith('plot the residuals instead'));
    expect(queueLength()).toBe(1);
  });
});
