import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, fireEvent } from '@testing-library/react';

/**
 * A chat that failed to start says so in a toast that ends "Your message was
 * kept." On a fresh tab's composer — the surface people start most of their
 * chats from — that was false, then half true, and these tests pin down the
 * half that was still false: THE RETRY.
 *
 * Measured in the dev app on 1.90.4 (2026-09-12) with `VERSA_AZURE_API_KEY`
 * unreadable, so `POST /agent/start` answers 400 from a fresh tab:
 *
 *   Enter, nothing on screen    -> a NEW toast node renders -> message KEPT (2/2)
 *   Enter again, toast still up -> no new node (deduped)    -> message LOST (3/3)
 *
 * The toast is what decided it. Error toasts are `autoClose: false` with a
 * `toastId` derived from their own words, so a second identical failure renders
 * nothing at all — and without that render the composer's rebuild lands in a
 * LATER TASK than the give-back instead of inside the same one. #303 gave the
 * message a lifetime of exactly one task (`setTimeout(..., 0)`), so the retry —
 * the press a person actually makes — lost it every time, under a toast still
 * reading "kept".
 *
 * The property that ends that class of bug is not "the rebuild is fast enough".
 * It is: THE MESSAGE IS STILL THERE FOR A COMPOSER THAT IS BUILT LATER, and is
 * spent by events, not by time. So these tests rebuild the composer after the
 * failure has fully settled — which the app does anyway, twice per failure,
 * measured at +19 ms and +21 ms in #303 and at +38 ms / +43 ms here — and ask
 * for the message then. Everything else is the real thing: `BaseChat`'s own
 * exported `handleCreateSessionError`, and the real `ChatInput`.
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
import { handleCreateSessionError, messageOwedToComposer, type KeptMessage } from './BaseChat';
import { ChatState } from '../types/chatState';
import { toastError } from '../toasts';

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

const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));

type StartOutcome = { ok: true; id: string } | { ok: false; err: unknown };

/**
 * `BaseChat`'s pre-session surface, reduced to the four things this defect
 * lives in and nothing else:
 *
 *   1. it owns the composer's replacement. `isCreatingSession` flipping moves
 *      the composer between two SLOTS of the same parent (the centred empty
 *      state and the bar under a transcript), which is a different position in
 *      the children array, so React tears the old one down and mounts a new one;
 *   2. it owns whatever a failed start gives back, and outlives the composer;
 *   3. it offers that back only for the chat it was typed into, and only when
 *      it is not at that moment trying to send it;
 *   4. its chat id is `''` until a chat exists.
 *
 * `composerGeneration` is a rebuild the test can ask for, to stand in for the
 * later of the two rebuilds every failure causes in the app. It changes nothing
 * about the surface's own behaviour.
 */
function Surface({
  start,
  composerGeneration = 0,
  boundSessionId = '',
}: {
  start: () => StartOutcome;
  composerGeneration?: number;
  /** The chat this tab is bound to from outside; `''` is a tab with no chat. */
  boundSessionId?: string;
}) {
  const [createdSessionId, setCreatedSessionId] = React.useState<string | null>(null);
  const sessionId = createdSessionId ?? boundSessionId;
  const [isCreatingSession, setIsCreatingSession] = React.useState(false);
  const [keptMessage, setKeptMessage] = React.useState<KeptMessage>(null);

  const handleFormSubmit = async (e: React.FormEvent): Promise<boolean> => {
    const textValue = (e as unknown as CustomEvent).detail?.value ?? '';
    setIsCreatingSession(true);
    // The real one awaits `createSession`; the outcome always arrives in a
    // later microtask, never inline, which is why the composer has already
    // cleared and moved by the time it lands.
    await Promise.resolve();
    const outcome = start();
    if (outcome.ok) {
      setKeptMessage(null);
      setCreatedSessionId(outcome.id);
      return true;
    }
    setIsCreatingSession(false);
    handleCreateSessionError(outcome.err, {
      textValue,
      attachments: [],
      sessionId,
      keep: setKeptMessage,
    });
    return true;
  };

  const composer = (
    <ChatInput
      key={composerGeneration}
      sessionId={sessionId}
      handleSubmit={handleFormSubmit}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      // `BaseChat`'s own rule, not a copy of it.
      keptMessage={messageOwedToComposer(keptMessage, { sessionId, isCreatingSession })}
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

  const isCleanConversation = !sessionId && !isCreatingSession;
  return (
    <>
      {isCleanConversation ? <div data-slot="empty-state">{composer}</div> : null}
      {!isCleanConversation && <div data-slot="under-transcript">{composer}</div>}
    </>
  );
}

const textarea = () => screen.getByRole('textbox') as HTMLTextAreaElement;
const composerText = () => textarea().value;

/**
 * Type and press Enter, through the real composer, which clears itself — then
 * let the surface settle: the start resolves on a microtask and the rebuild it
 * causes is committed after that.
 */
const send = async (text: string) => {
  await act(async () => {
    fireEvent.change(textarea(), { target: { value: text } });
  });
  await act(async () => {
    fireEvent.keyDown(textarea(), { key: 'Enter', code: 'Enter' });
    await nextTask();
    await nextTask();
  });
};

const failing = (err = new Error('HTTP 400 Bad Request')): (() => StartOutcome) => {
  return () => ({ ok: false, err });
};

describe('a chat that failed to start, on the composer the user is looking at', () => {
  it('gives the message back to the composer that replaces the one that submitted', async () => {
    render(<Surface start={failing()} />);

    await send('analyze my cohort');

    expect(composerText()).toBe('analyze my cohort');
    expect(toastError).toHaveBeenCalledTimes(1);
  });

  it('still has it for a composer built LATER than any timer could have waited', async () => {
    // The lifetime, stated as the thing that broke. #303's copy was deleted one
    // task after the give-back, so a composer rebuilt after that — which is
    // every rebuild the app makes once the failure renders no toast — found
    // nothing. Nothing here is spent by a clock, so a rebuild is just a rebuild.
    const { rerender } = render(<Surface start={failing()} composerGeneration={0} />);
    await send('built later');
    expect(composerText()).toBe('built later');

    await act(async () => {
      await nextTask();
      rerender(<Surface start={failing()} composerGeneration={1} />);
    });

    expect(composerText()).toBe('built later');
  });

  it('keeps it on the RETRY, the press a person actually makes', async () => {
    // The reported release blocker: same failure, same words on screen, and the
    // only difference is that react-toastify renders nothing the second time.
    // Measured 3 of 3 retries losing the message, under a toast reading "Your
    // message was kept."
    const { rerender } = render(<Surface start={failing()} composerGeneration={0} />);

    await send('analyze my cohort');
    expect(composerText()).toBe('analyze my cohort');

    await send('analyze my cohort');
    expect(composerText()).toBe('analyze my cohort');

    // ...and the composer the retry rebuilds after that has it too.
    await act(async () => {
      await nextTask();
      rerender(<Surface start={failing()} composerGeneration={1} />);
    });
    expect(composerText()).toBe('analyze my cohort');

    // A third press changes nothing: being read does not spend it.
    await send('analyze my cohort');
    expect(composerText()).toBe('analyze my cohort');
  });

  it('gives back the EDITED text on a retry, not the first attempt', async () => {
    render(<Surface start={failing()} />);

    await send('frist draft');
    await send('first draft');

    expect(composerText()).toBe('first draft');
  });

  it('clears the composer when the start SUCCEEDS after a failure', async () => {
    // A start that worked hands the message to the new chat as route cargo. It
    // must not ALSO be sitting in the composer of the chat that just started —
    // including in the composer the in-flight attempt rebuilds on its way up.
    let succeed = false;
    const { rerender } = render(
      <Surface
        start={() => (succeed ? { ok: true, id: 'sess-9' } : { ok: false, err: new Error('nope') })}
        composerGeneration={0}
      />
    );

    await send('kept for now');
    expect(composerText()).toBe('kept for now');

    succeed = true;
    await send('kept for now');
    expect(composerText()).toBe('');

    // Nor does it come back on the next rebuild of that chat's composer.
    await act(async () => {
      await nextTask();
      rerender(<Surface start={() => ({ ok: true, id: 'sess-9' })} composerGeneration={1} />);
    });
    expect(composerText()).toBe('');
  });

  it('never shows it in a chat that is not the one it was typed into', async () => {
    // The same surface, rebound to an existing chat (what clicking another chat
    // in this tab does). The message belonged to the chat that never existed.
    const { rerender } = render(<Surface start={failing()} />);
    await send('meant for the new tab');
    expect(composerText()).toBe('meant for the new tab');

    await act(async () => {
      rerender(<Surface start={failing()} boundSessionId="session-7" />);
    });

    expect(composerText()).toBe('');
  });

  it('restores nothing after a reload', async () => {
    // A reload is a new renderer: no surface, no state, nothing owed. Modelled
    // as unmounting everything and mounting a fresh surface — which is exactly
    // what a module-level store would NOT have been caught by, and is why the
    // copy lives in the component.
    const first = render(<Surface start={failing()} />);
    await send('said once');
    expect(composerText()).toBe('said once');

    await act(async () => {
      first.unmount();
      render(<Surface start={failing()} />);
    });

    expect(composerText()).toBe('');
  });

  it('reports nothing when the send was never attempted', async () => {
    // The F3 model check refusing before anything is tried is a deliberate
    // cancel: the composer keeps its own text and NOTHING speaks.
    render(<Surface start={() => ({ ok: true, id: 'sess-1' })} />);

    await act(async () => {
      fireEvent.change(textarea(), { target: { value: 'never sent' } });
    });

    expect(toastError).not.toHaveBeenCalled();
  });
});
