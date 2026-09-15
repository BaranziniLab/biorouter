import React from 'react';
import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { render, screen, fireEvent, waitFor, act, within } from '@testing-library/react';

/**
 * A message queued behind a running turn belongs to the CHAT, not to the
 * composer instance that happened to be on screen when it was typed.
 *
 * Only the active tab of a pane mounts a `BaseChat`, so switching tabs unmounts
 * the composer, and splitting or collapsing a pane rebuilds it in one commit.
 * Measured on 1.90.4 (main 1038a113) in the dev app, against Versa GPT-5.5: a
 * message showing as "Next" behind a running turn was gone after clicking
 * another tab and back, the turn ended, and it was never sent nor put back in
 * the composer. The unmount recovery kept only offers already in flight.
 *
 * These tests drive the REAL composer and remount it the ways the shell does:
 * unmount + mount (a tab switch), and a key change inside one render (a pane
 * rebuild, where React renders the replacement before the old one's cleanup).
 * They vary only the chat state the composer mounts into. jsdom has no layout,
 * so nothing here says how the queue LOOKS.
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
// The real queue is a drag-and-drop list; these tests need only WHICH messages
// it holds, and whether it says it is paused.
vi.mock('./MessageQueue', () => ({
  default: ({
    queuedMessages,
    onRemoveMessage,
    onStopAndSend,
    isPaused,
  }: {
    queuedMessages: Array<{ id: string; content: string }>;
    onRemoveMessage?: (id: string) => void;
    onStopAndSend?: (id: string) => void;
    isPaused?: boolean;
  }) => (
    <ul data-testid="queue" data-paused={isPaused ? 'true' : 'false'}>
      {queuedMessages.map((msg) => (
        <li key={msg.id}>
          <span data-testid="queued-content">{msg.content}</span>
          <button type="button" onClick={() => onRemoveMessage?.(msg.id)}>
            Remove {msg.id}
          </button>
          <button type="button" onClick={() => onStopAndSend?.(msg.id)}>
            Stop and send {msg.id}
          </button>
        </li>
      ))}
    </ul>
  ),
}));
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
import { toastWarning } from '../toasts';
import { readComposerDraft, resetComposerDraftsForTests } from '../utils/composerDrafts';

const QUEUED = 'QUEUED: one sentence summary please';
const SECOND = 'and then a table of the dates';

type SubmitFn = (e: React.FormEvent) => void | Promise<boolean | void>;
type SubmitMock = Mock<SubmitFn>;

beforeEach(() => {
  vi.clearAllMocks();
  resetComposerDraftsForTests();
  // What the composer keeps between instances is renderer memory shared by the
  // whole file. Each test uses its own chat ids instead of a reset hook, so a
  // queue that leaked from one chat to another would show up here, not be wiped.
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
const queued = () => screen.queryAllByTestId('queued-content').map((el) => el.textContent);
const submittedTexts = (handleSubmit: SubmitMock) =>
  handleSubmit.mock.calls.map((call) => (call[0] as unknown as CustomEvent).detail.value as string);

type ComposerProps = {
  sessionId: string | null;
  chatState: ChatState;
  handleSubmit: SubmitMock;
  draftKey?: string;
  onStop?: (continuationPending?: boolean) => boolean | void | Promise<boolean | void>;
};

const noStop = () => undefined;

function Composer({
  sessionId,
  chatState,
  handleSubmit,
  draftKey,
  onStop = noStop,
}: ComposerProps) {
  return (
    <ChatInput
      sessionId={sessionId}
      handleSubmit={handleSubmit}
      chatState={chatState}
      onStop={onStop}
      draftKey={draftKey}
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

/** A composer as a tab mounts it. `rebuild` replaces it within one render. */
function mountComposer(initial: ComposerProps) {
  let props = initial;
  let instance = 0;
  const view = render(<Composer key={instance} {...props} />);
  return {
    set(next: Partial<ComposerProps>) {
      props = { ...props, ...next };
      view.rerender(<Composer key={instance} {...props} />);
    },
    /** A pane rebuild: the replacement renders in the same commit that unmounts this one. */
    rebuild() {
      instance += 1;
      view.rerender(<Composer key={instance} {...props} />);
    },
    unmount: view.unmount,
  };
}

async function queueMessage(text: string) {
  fireEvent.change(composer(), { target: { value: text } });
  fireEvent.submit(composer().closest('form')!);
  await waitFor(() => expect(queued()).toContain(text));
}

/** Let every pending macrotask (the bounded re-offer timers) run out. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
}

describe('a queued message across a tab switch', () => {
  it('is back in the queue when the chat comes back mid-turn, and is sent once when the turn ends', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    const first = mountComposer({
      sessionId: 'chat-mid-turn',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);

    act(() => first.unmount());
    const back = mountComposer({
      sessionId: 'chat-mid-turn',
      chatState: ChatState.Streaming,
      handleSubmit,
    });

    expect(queued()).toEqual([QUEUED]);
    expect(handleSubmit).not.toHaveBeenCalled();

    back.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED]);
    expect(queued()).toEqual([]);
  });

  it('is sent once when the chat comes back after its turn ended while no composer was mounted', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    const first = mountComposer({
      sessionId: 'chat-ended-away',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);
    await queueMessage(SECOND);

    act(() => first.unmount());
    // The turn it was waiting for ended while the tab was not showing. Nothing
    // can send then: the submit path is the mounted composer's.
    await settle();
    expect(handleSubmit).not.toHaveBeenCalled();

    const back = mountComposer({
      sessionId: 'chat-ended-away',
      chatState: ChatState.Idle,
      handleSubmit,
    });

    // Exactly the drain the turn's end would have made: the head goes, the rest
    // waits for the turn the head starts.
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED]);
    expect(queued()).toEqual([SECOND]);

    back.set({ chatState: ChatState.Streaming });
    back.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(2));
    await settle();
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED, SECOND]);
    expect(queued()).toEqual([]);
  });

  it('survives a pane rebuild, where the replacement renders before the old composer is gone', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    const pane = mountComposer({
      sessionId: 'chat-split',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);

    act(() => pane.rebuild());

    expect(queued()).toEqual([QUEUED]);
    pane.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(queued()).toEqual([]);
  });

  it("is never another chat's: a different chat's composer neither shows nor sends it", async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    const mine = mountComposer({
      sessionId: 'chat-mine',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);
    act(() => mine.unmount());

    const other = mountComposer({
      sessionId: 'chat-other',
      chatState: ChatState.Idle,
      handleSubmit,
    });
    await settle();
    expect(queued()).toEqual([]);
    expect(handleSubmit).not.toHaveBeenCalled();
    act(() => other.unmount());

    // ...and a new chat's composer, which has no chat yet, does not get it either.
    const fresh = mountComposer({ sessionId: null, chatState: ChatState.Idle, handleSubmit });
    await settle();
    expect(queued()).toEqual([]);
    expect(handleSubmit).not.toHaveBeenCalled();
    act(() => fresh.unmount());

    mountComposer({ sessionId: 'chat-mine', chatState: ChatState.Streaming, handleSubmit });
    expect(queued()).toEqual([QUEUED]);
  });

  it('does not queue again, or send again, a message already handed to its own turn', async () => {
    // The drain's submit resolves when the turn it started ENDS, so for that
    // whole turn the message is in flight. A remount must not show it as the
    // next message, and the turn's end must not send it a second time.
    let endTurn: (accepted: boolean) => void = () => {};
    const handleSubmit = vi.fn<SubmitFn>(
      () =>
        new Promise<boolean>((resolve) => {
          endTurn = resolve;
        })
    );
    const first = mountComposer({
      sessionId: 'chat-in-flight',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);
    first.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    // The queued message's own turn is now running.
    first.set({ chatState: ChatState.Streaming });

    act(() => first.unmount());
    const back = mountComposer({
      sessionId: 'chat-in-flight',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    expect(queued()).toEqual([]);

    back.set({ chatState: ChatState.Idle });
    await act(async () => endTurn(true));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(queued()).toEqual([]);
  });

  it('keeps a message refused mid-drain when the tab is switched before its next attempt', async () => {
    // The first drain attempt of a turn is routinely refused (the finishing
    // turn's submit latch), and the re-offer is a timer. A switch in between
    // cancels the timer; the message must not go with it.
    let refuseFirst: (accepted: boolean) => void = () => {};
    const handleSubmit: SubmitMock = vi.fn<SubmitFn>(() =>
      handleSubmit.mock.calls.length === 1
        ? new Promise<boolean>((resolve) => {
            refuseFirst = resolve;
          })
        : Promise.resolve(true)
    );
    const first = mountComposer({
      sessionId: 'chat-mid-drain',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);
    first.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));

    // The refusal lands (microtasks) and arms the re-offer; the switch comes
    // before that timer (a macrotask) can run.
    await act(async () => {
      refuseFirst(false);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      first.unmount();
    });
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);

    mountComposer({ sessionId: 'chat-mid-drain', chatState: ChatState.Idle, handleSubmit });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(2));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(2);
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED, QUEUED]);
    expect(queued()).toEqual([]);
  });

  it('hands a message its gone composer could not send to the composer now mounted for the chat', async () => {
    let answer: (accepted: boolean) => void = () => {};
    const slowSubmit = vi.fn<SubmitFn>(
      () =>
        new Promise<boolean>((resolve) => {
          answer = resolve;
        })
    );
    const first = mountComposer({
      sessionId: 'chat-late-no',
      chatState: ChatState.Streaming,
      handleSubmit: slowSubmit,
    });
    await queueMessage(QUEUED);
    first.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(slowSubmit).toHaveBeenCalledTimes(1));
    act(() => first.unmount());

    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    mountComposer({ sessionId: 'chat-late-no', chatState: ChatState.Idle, handleSubmit });
    await settle();
    // Still the first submit's: not shown, not sent by the new composer.
    expect(queued()).toEqual([]);
    expect(handleSubmit).not.toHaveBeenCalled();

    await act(async () => answer(false));
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED]);
    expect(queued()).toEqual([]);
  });

  it('does not bring back a Stop & send interrupted by a tab switch as a paused queue', async () => {
    // Stop & send pauses the queue while its stop is on the wire, so the turn's
    // own end cannot drain the row as well. That pause is the control's, not the
    // person's: a switch in the middle must not leave the message parked behind
    // a "Queue paused" nobody chose.
    let finishStop: (stopped: boolean) => void = () => {};
    const onStop = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          finishStop = resolve;
        })
    );
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    const first = mountComposer({
      sessionId: 'chat-stop-and-send',
      chatState: ChatState.Streaming,
      handleSubmit,
      onStop,
    });
    await queueMessage(QUEUED);
    fireEvent.click(screen.getByRole('button', { name: /^stop and send /i }));
    expect(onStop).toHaveBeenCalledTimes(1);

    act(() => first.unmount());
    // The stop lands after the composer that asked for it is gone: it sends nothing.
    await act(async () => finishStop(true));
    await settle();
    expect(handleSubmit).not.toHaveBeenCalled();

    mountComposer({ sessionId: 'chat-stop-and-send', chatState: ChatState.Idle, handleSubmit });
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED]);
    expect(queued()).toEqual([]);
  });

  it('keeps a queue left idle after a refusal as rows, and does not send it by itself on return', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => false);
    const first = mountComposer({
      sessionId: 'chat-refused',
      chatState: ChatState.Streaming,
      handleSubmit,
    });
    await queueMessage(QUEUED);
    first.set({ chatState: ChatState.Idle });
    await waitFor(() => expect(toastWarning).toHaveBeenCalledTimes(1));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(3);
    expect(queued()).toEqual([QUEUED]);

    act(() => first.unmount());
    mountComposer({ sessionId: 'chat-refused', chatState: ChatState.Idle, handleSubmit });
    await settle();

    // Visible and re-sendable, as it was before the switch; not a fourth attempt
    // nobody asked for, which an "idle and non-empty, so send" rule would make
    // on every mount.
    expect(queued()).toEqual([QUEUED]);
    expect(handleSubmit).toHaveBeenCalledTimes(3);
    fireEvent.click(within(screen.getByTestId('queue')).getByRole('button', { name: /^remove /i }));
    expect(queued()).toEqual([]);
  });
});

describe('a composer with no chat', () => {
  it("hands a queued message back to its own draft when it goes away, never to a chat's queue", async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    // Home while its start is in flight shows a loading state, so a second
    // message queues behind it. Home has no chat to send it to later.
    const home = mountComposer({
      sessionId: null,
      draftKey: 'home',
      chatState: ChatState.LoadingConversation,
      handleSubmit,
    });
    await queueMessage(QUEUED);

    act(() => home.unmount());

    expect(readComposerDraft('home')?.text).toBe(QUEUED);
    expect(handleSubmit).not.toHaveBeenCalled();
  });
});
