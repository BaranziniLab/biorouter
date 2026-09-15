import React from 'react';
import { describe, it, expect, vi, beforeEach, type Mock } from 'vitest';
import { render, screen, fireEvent, waitFor, act, within } from '@testing-library/react';

/**
 * A message typed while a turn is running is QUEUED, and the queue drains when
 * the turn ends. The drain used to fire the submit and forget it: it dequeued
 * whether or not the submit had taken the message, and the store's submit
 * refuses silently while the finishing turn's `submitInFlight` latch is still
 * held (see chatStreamStore.submitVerdict.test.tsx). The user's text vanished
 * with no error, every time.
 *
 * These tests drive the REAL composer and vary one thing: what the submit
 * answers. What they can and cannot show:
 *
 *  - They CAN show ownership of the message across a refusal (how many times
 *    it is offered, whether it comes back to the queue, whether the composer
 *    gets its text back), because that is state this component owns.
 *  - They CANNOT show the ordering that produces the refusal in the real app
 *    (React's effect scheduling against the store's promise chain). The store
 *    test above covers that half, in the store, with no fake ordering.
 *  - jsdom has no layout engine and does not run Tailwind, so nothing here says
 *    anything about how the queue chip or the toast LOOK.
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
// The real queue UI is a drag-and-drop list; what these tests need from it is
// only WHICH messages are in the queue, so it stands in as a plain list.
vi.mock('./MessageQueue', () => ({
  default: ({
    queuedMessages,
    onStopAndSend,
    onRemoveMessage,
    onClearQueue,
  }: {
    queuedMessages: Array<{ id: string; content: string }>;
    onStopAndSend?: (id: string) => void;
    onRemoveMessage?: (id: string) => void;
    onClearQueue?: () => void;
  }) => (
    <ul data-testid="queue">
      {queuedMessages.map((msg) => (
        <li key={msg.id} data-testid="queued-item">
          <span data-testid="queued-content">{msg.content}</span>
          <button type="button" onClick={() => onStopAndSend?.(msg.id)}>
            Stop and send {msg.id}
          </button>
          <button type="button" onClick={() => onRemoveMessage?.(msg.id)}>
            Remove {msg.id}
          </button>
        </li>
      ))}
      <button type="button" onClick={onClearQueue}>
        Clear queue
      </button>
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
import type { DroppedFile } from '../hooks/useFileDrop';
import {
  resetAnnotationChannelForTests,
  sendArtifactAnnotation,
  type ArtifactAnnotation,
} from '../utils/annotationChannel';
import { resetComposerQueuesForTests } from '../utils/composerQueues';

const QUEUED_TEXT = 'summarise the second table too';
const DIRECT_TEXT = 'plot the residuals';

/** The composer's own prop signature, so a mock cannot drift from it. */
type SubmitFn = (e: React.FormEvent) => void | Promise<boolean | void>;
type SubmitMock = Mock<SubmitFn>;
type StopFn = (continuationPending?: boolean) => boolean | void | Promise<boolean | void>;

beforeEach(() => {
  vi.clearAllMocks();
  resetAnnotationChannelForTests();
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
const queued = () => screen.queryAllByTestId('queued-content').map((el) => el.textContent);
const submittedTexts = (handleSubmit: SubmitMock) =>
  handleSubmit.mock.calls.map((call) => (call[0] as unknown as CustomEvent).detail.value as string);

/** How many offers actually landed a turn, as opposed to being refused. */
async function acceptedCount(handleSubmit: SubmitMock): Promise<number> {
  const verdicts = await Promise.all(
    handleSubmit.mock.results.map((result) => Promise.resolve(result.value as boolean | undefined))
  );
  return verdicts.filter((verdict) => verdict !== false).length;
}

function renderComposer(
  handleSubmit: SubmitMock,
  chatState: ChatState,
  onStop: StopFn = vi.fn(),
  onAbandonContinuation: () => void | Promise<void> = vi.fn(),
  droppedFiles: DroppedFile[] = []
) {
  const props = (state: ChatState) => (
    <ChatInput
      sessionId="session-under-test"
      handleSubmit={handleSubmit}
      chatState={state}
      onStop={onStop}
      onAbandonContinuation={onAbandonContinuation}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={droppedFiles}
      onFilesProcessed={vi.fn()}
      messagesLength={2}
      disableAnimation
      sessionCosts={undefined}
      toolCount={0}
    />
  );
  const view = render(props(chatState));
  return {
    setChatState: (state: ChatState) => view.rerender(props(state)),
    unmount: view.unmount,
  };
}

function annotation(imagePath: string): ArtifactAnnotation {
  return {
    sessionId: 'session-under-test',
    imagePath,
    sourceTitle: 'Preview',
    sourceLocator: '/Users/example/source.pdf',
    region: { x: 1, y: 2, width: 30, height: 40, surfaceWidth: 300, surfaceHeight: 400 },
    width: 30,
    height: 40,
  };
}

async function queueAnnotation(imagePath: string) {
  act(() => sendArtifactAnnotation(annotation(imagePath)));
  await waitFor(() =>
    expect(composer().value).toContain('[Selected region from the preview panel]')
  );
  fireEvent.submit(composer().closest('form')!);
  await waitFor(() => expect(queued()).toHaveLength(1));
}

/** Type a message while the turn is running, so it lands in the queue. */
async function queueOneMessage(text = QUEUED_TEXT) {
  fireEvent.change(composer(), { target: { value: text } });
  fireEvent.submit(composer().closest('form')!);
  await waitFor(() => expect(queued()).toEqual([text]));
}

/** Let every pending macrotask (the bounded re-offer timers) run out. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
}

describe('draining the message queue', () => {
  it('sends an accepted message exactly once and clears it from the queue', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    const { setChatState } = renderComposer(handleSubmit, ChatState.Streaming);
    await queueOneMessage();

    setChatState(ChatState.Idle);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    // Exactly one: the re-offer must never turn an accepted message into two
    // turns, which is the failure the store's own re-entrancy latch exists to
    // prevent. "At least one" would not catch that.
    expect(handleSubmit).toHaveBeenCalledTimes(1);
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED_TEXT]);
    expect(queued()).toEqual([]);
  });

  it('keeps a refused message and re-offers it, so it is sent exactly once', async () => {
    // The real shape of the bug: the first offer lands while the finishing
    // turn's submit latch is still held, the next one does not.
    let offers = 0;
    const handleSubmit = vi.fn<SubmitFn>(async () => (offers++ === 0 ? false : true));
    const { setChatState } = renderComposer(handleSubmit, ChatState.Streaming);
    await queueOneMessage();

    setChatState(ChatState.Idle);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(2));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(2);
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED_TEXT, QUEUED_TEXT]);
    // Two offers, ONE send: the refused offer sent nothing at all.
    expect(await acceptedCount(handleSubmit)).toBe(1);
    expect(queued()).toEqual([]);
    expect(toastWarning).not.toHaveBeenCalled();
  });

  it('puts a message refused past the retry bound back in the queue and says so', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => false);
    const { setChatState } = renderComposer(handleSubmit, ChatState.Streaming);
    await queueOneMessage();

    setChatState(ChatState.Idle);

    await waitFor(() => expect(toastWarning).toHaveBeenCalledTimes(1));
    await settle();
    // Bounded, not a spin: three offers and no more.
    expect(handleSubmit).toHaveBeenCalledTimes(3);
    // And the message is still the user's, visible in the queue rather than
    // dropped on the floor.
    expect(queued()).toEqual([QUEUED_TEXT]);
  });

  it('offers a refused message only while it is OUT of the queue', async () => {
    // While the re-offer is in flight the message must not also be sitting in
    // the queue, where a second drain edge or a Stop-and-send would pick up the
    // same message and send it a second time.
    let release: (accepted: boolean) => void = () => {};
    const handleSubmit = vi.fn<SubmitFn>(
      () =>
        new Promise<boolean>((resolve) => {
          release = resolve;
        })
    );
    const { setChatState } = renderComposer(handleSubmit, ChatState.Streaming);
    await queueOneMessage();

    setChatState(ChatState.Idle);
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));

    expect(queued()).toEqual([]);
    await act(async () => release(true));
    expect(queued()).toEqual([]);
  });

  it('recovers an in-flight refused offer after the composer unmounts', async () => {
    let resolveOffer: (accepted: boolean) => void = () => {};
    const handleSubmit = vi.fn<SubmitFn>(
      () =>
        new Promise<boolean>((resolve) => {
          resolveOffer = resolve;
        })
    );
    const first = renderComposer(handleSubmit, ChatState.Streaming);
    await queueOneMessage();
    act(() => first.setChatState(ChatState.Idle));
    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));

    act(() => first.unmount());
    await act(async () => resolveOffer(false));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);

    renderComposer(
      vi.fn<SubmitFn>(async () => true),
      ChatState.Streaming
    );
    expect(queued()).toEqual([QUEUED_TEXT]);
    fireEvent.click(within(screen.getByTestId('queue')).getByRole('button', { name: /^remove /i }));
    expect(queued()).toEqual([]);
  });
});

describe('discarding queued preview captures', () => {
  it('removing a queued annotation deletes only renderer-owned temp images', async () => {
    const originalUserFile = '/Users/example/original-image.png';
    const stagedDrop = '/private/tmp/staged-drop.png';
    const capture = '/private/tmp/preview-capture.png';
    const droppedImage: DroppedFile = {
      id: 'drop-1',
      path: originalUserFile,
      sourcePath: originalUserFile,
      stagedPath: stagedDrop,
      name: 'original-image.png',
      type: 'image/png',
      isImage: true,
      canUploadAsImage: true,
      isLoading: false,
    };
    renderComposer(
      vi.fn<SubmitFn>(async () => true),
      ChatState.Streaming,
      vi.fn(),
      vi.fn(),
      [droppedImage]
    );
    await queueAnnotation(capture);

    fireEvent.click(within(screen.getByTestId('queue')).getByRole('button', { name: /^remove /i }));

    expect(window.electron.deleteTempFile).toHaveBeenCalledWith(capture);
    expect(window.electron.deleteTempFile).toHaveBeenCalledWith(stagedDrop);
    expect(window.electron.deleteTempFile).not.toHaveBeenCalledWith(originalUserFile);
    expect(queued()).toEqual([]);
  });

  it('clearing the queue deletes every owned annotation capture', async () => {
    const firstCapture = '/private/tmp/preview-capture-1.png';
    const secondCapture = '/private/tmp/preview-capture-2.png';
    renderComposer(
      vi.fn<SubmitFn>(async () => true),
      ChatState.Streaming
    );
    await queueAnnotation(firstCapture);
    act(() => sendArtifactAnnotation(annotation(secondCapture)));
    await waitFor(() =>
      expect(composer().value).toContain('[Selected region from the preview panel]')
    );
    fireEvent.submit(composer().closest('form')!);
    await waitFor(() => expect(queued()).toHaveLength(2));

    fireEvent.click(screen.getByRole('button', { name: 'Clear queue' }));

    expect(window.electron.deleteTempFile).toHaveBeenCalledWith(firstCapture);
    expect(window.electron.deleteTempFile).toHaveBeenCalledWith(secondCapture);
    expect(queued()).toEqual([]);
  });
});

describe('stopping the current turn and sending a queued message', () => {
  it('sends directly from an idle paused queue without issuing a generationless Stop', async () => {
    let accepted = false;
    const handleSubmit = vi.fn<SubmitFn>(async () => accepted);
    const onStop = vi.fn<StopFn>();
    const { setChatState } = renderComposer(handleSubmit, ChatState.Streaming, onStop);
    await queueOneMessage();

    // Exhaust the automatic drain so the row is visible while idle, matching a
    // queue the user deliberately left paused after an interruption.
    setChatState(ChatState.Idle);
    await waitFor(() => expect(toastWarning).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(queued()).toEqual([QUEUED_TEXT]));
    handleSubmit.mockClear();
    onStop.mockClear();
    accepted = true;

    fireEvent.click(screen.getByRole('button', { name: /stop and send/i }));

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    expect(onStop).not.toHaveBeenCalled();
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED_TEXT]);
    expect(queued()).toEqual([]);
  });

  it('keeps the row and sends nothing until Stop reaches its completion barrier', async () => {
    let finishStop: (value: boolean) => void = () => {};
    const onStop = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          finishStop = resolve;
        })
    );
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    renderComposer(handleSubmit, ChatState.Streaming, onStop);
    await queueOneMessage();

    fireEvent.click(screen.getByRole('button', { name: /stop and send/i }));

    expect(onStop).toHaveBeenCalledTimes(1);
    expect(onStop).toHaveBeenCalledWith(true);
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(queued()).toEqual([QUEUED_TEXT]);

    await act(async () => finishStop(true));

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    expect(submittedTexts(handleSubmit)).toEqual([QUEUED_TEXT]);
    expect(queued()).toEqual([]);
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
  });

  it('leaves the row queued and warns when Stop reports failure', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    renderComposer(handleSubmit, ChatState.Streaming, async () => false);
    await queueOneMessage();

    fireEvent.click(screen.getByRole('button', { name: /stop and send/i }));

    await waitFor(() => expect(toastWarning).toHaveBeenCalledTimes(1));
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(queued()).toEqual([QUEUED_TEXT]);
    expect(toastWarning).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Message still queued' })
    );
  });

  it('leaves the row queued and warns when Stop rejects', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    renderComposer(handleSubmit, ChatState.Streaming, async () => {
      throw new Error('cancel unavailable');
    });
    await queueOneMessage();

    fireEvent.click(screen.getByRole('button', { name: /stop and send/i }));

    await waitFor(() => expect(toastWarning).toHaveBeenCalledTimes(1));
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(queued()).toEqual([QUEUED_TEXT]);
  });

  it('abandons a delayed continuation acknowledgement after its queued owner is removed', async () => {
    let finishStop: (value: boolean) => void = () => {};
    const onStop = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          finishStop = resolve;
        })
    );
    const abandon = vi.fn(async () => undefined);
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    renderComposer(handleSubmit, ChatState.Streaming, onStop, abandon);
    await queueOneMessage();

    fireEvent.click(screen.getByRole('button', { name: /stop and send/i }));
    fireEvent.click(screen.getByRole('button', { name: /^remove /i }));
    expect(abandon).toHaveBeenCalledTimes(1);

    await act(async () => finishStop(true));
    await waitFor(() => expect(abandon).toHaveBeenCalledTimes(2));
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(queued()).toEqual([]);
  });

  it('abandons the admitted continuation when its refused queued replacement is removed', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => false);
    const abandon = vi.fn(async () => undefined);
    renderComposer(handleSubmit, ChatState.Streaming, async () => true, abandon);
    await queueOneMessage();

    fireEvent.click(screen.getByRole('button', { name: /stop and send/i }));
    await settle();
    await waitFor(() => expect(queued()).toEqual([QUEUED_TEXT]));
    expect(abandon).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: /^remove /i }));
    expect(abandon).toHaveBeenCalledTimes(1);
    expect(queued()).toEqual([]);
  });
});

describe('a direct send that is refused', () => {
  it('puts the text back in the composer', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => false);
    renderComposer(handleSubmit, ChatState.Idle);

    fireEvent.change(composer(), { target: { value: DIRECT_TEXT } });
    fireEvent.submit(composer().closest('form')!);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    // The composer clears itself synchronously on submit, so this is a restore,
    // not a "never cleared".
    await waitFor(() => expect(composer().value).toBe(DIRECT_TEXT));
    // One send attempt only: restoring the text must not also re-send it.
    expect(handleSubmit).toHaveBeenCalledTimes(1);
  });

  it('control: an accepted send leaves the composer empty', async () => {
    const handleSubmit = vi.fn<SubmitFn>(async () => true);
    renderComposer(handleSubmit, ChatState.Idle);

    fireEvent.change(composer(), { target: { value: DIRECT_TEXT } });
    fireEvent.submit(composer().closest('form')!);

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));
    await settle();
    expect(composer().value).toBe('');
  });
});
