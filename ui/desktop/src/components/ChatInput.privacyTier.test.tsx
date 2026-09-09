import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';

/**
 * #56 — the composer's session privacy tier must describe THIS chat, and the
 * most recently *issued* read of it.
 *
 * Both consumers of `sessionPrivacyTier` treat `private` as a licence to
 * restrict: `ModelsBottomBar` paints a Private dot and `SwitchModelModal` greys
 * out every public model. A tier left over from a previous chat is therefore
 * not a cosmetic lag — it is a false claim in the direction that both walls a
 * working model and asserts a confidentiality guarantee the chat does not have.
 *
 * Three distinct failure modes:
 *   1. rebinding to another chat, where the old tier survives until (and, on a
 *      failed read, past) the new one resolves;
 *   2. two overlapping reads for the SAME chat — the initial fetch and the
 *      `message-stream-finished` refresh — where the slower one wins because it
 *      landed last;
 *   3. v1.89.0 F-12 — the daemon RAISES the tier when it starts a turn, after
 *      the bind read and before the turn ends, so a chat that becomes private on
 *      its first message reads `public` for the whole of that turn.
 *   4. The chat stream now carries the POST-ratchet tier in the reply stream's
 *      own first frames, so the composer has a fresher answer than any of its
 *      reads. It must prefer that one — and must still work for the callers
 *      that thread nothing, which is why the reads above are kept.
 */

// Heavy children that are irrelevant here. The two tier consumers are replaced
// by probes so the test asserts on what the composer actually hands down,
// rather than on a badge's rendering.
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
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({
  default: ({ privacyTier }: { privacyTier?: string }) => (
    <div data-testid="tier-probe">{privacyTier ?? 'unresolved'}</div>
  ),
}));
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
  getSession: vi.fn(),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { getSession } from '../api';

const getSessionMock = vi.mocked(getSession);

beforeEach(() => {
  vi.clearAllMocks();
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

function chatInput(
  sessionId: string | null,
  chatState: ChatState = ChatState.Idle,
  messagesLength = 0,
  sessionRowPrivacyTier?: 'public' | 'private'
) {
  return (
    <ChatInput
      sessionId={sessionId}
      sessionRowPrivacyTier={sessionRowPrivacyTier}
      handleSubmit={vi.fn()}
      chatState={chatState}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={messagesLength}
      disableAnimation={false}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
}

function renderChatInput(
  sessionId: string | null,
  chatState: ChatState = ChatState.Idle,
  messagesLength = 0,
  sessionRowPrivacyTier?: 'public' | 'private'
) {
  return render(chatInput(sessionId, chatState, messagesLength, sessionRowPrivacyTier));
}

// A promise whose resolution this test controls, so the order responses LAND
// can be made to differ from the order the requests were ISSUED.
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('ChatInput session privacy tier (#56)', () => {
  it('drops the previous chat`s tier the moment the session rebinds', async () => {
    // BaseChat is keyed by TAB, not by session, so a tab that navigates from a
    // private chat to another chat re-renders this same ChatInput instance with
    // a new id. The old tier must not survive that.
    const second = deferred<{ data: { privacy_tier: string } }>();
    getSessionMock
      .mockResolvedValueOnce({ data: { privacy_tier: 'private' } } as never)
      .mockResolvedValueOnce({ data: { privacy_tier: 'private' } } as never)
      .mockReturnValue(second.promise as never);

    const { rerender } = renderChatInput('chat-a');
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));

    rerender(chatInput('chat-b'));

    // chat-b's read has not landed (and may never land, if it throws). Until it
    // does, the composer knows nothing about chat-b — and "nothing" is the only
    // honest answer. Rendering chat-a's `private` here is what greys out every
    // public model in a chat that permits them.
    expect(screen.getByTestId('tier-probe')).toHaveTextContent('unresolved');

    await act(async () => {
      second.resolve({ data: { privacy_tier: 'public' } });
      await second.promise;
    });
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('public'));
  });

  it('keeps the newest issued read when an older one lands after it', async () => {
    // The mount fetch and the `message-stream-finished` refresh share no
    // ordering. Without a generation check the LAST TO LAND wins, so a slow
    // first read can overwrite the fresher answer that already arrived.
    const slowFirst = deferred<{ data: { privacy_tier: string } }>();
    getSessionMock
      // The working-directory effect's own read — settled, and irrelevant.
      .mockResolvedValueOnce({ data: { working_dir: '/w' } } as never)
      .mockReturnValueOnce(slowFirst.promise as never)
      .mockResolvedValue({ data: { privacy_tier: 'public' } } as never);

    renderChatInput('chat-a');
    await waitFor(() => expect(getSessionMock).toHaveBeenCalled());

    // The refresh is issued second and lands first.
    await act(async () => {
      window.dispatchEvent(new Event('message-stream-finished'));
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('public'));

    // The mount read finally lands, carrying the OLDER answer.
    await act(async () => {
      slowFirst.resolve({ data: { privacy_tier: 'private' } });
      await slowFirst.promise;
    });

    expect(screen.getByTestId('tier-probe')).toHaveTextContent('public');
  });

  /**
   * v1.89.0 F-12. The bind read is not enough on the path that creates the chat:
   * a session is born `public` and the daemon RAISES it as it starts the turn,
   * so the composer's answer is stale the moment it is fetched — and the tab's
   * id does not change on that path, so nothing remounts to force a re-read.
   *
   * The chat is genuinely private for the whole of that turn while the extension
   * menu says "Unavailable in this chat (public model)" under every private
   * extension. Turn-END is where the old code refreshed, which is precisely too
   * late.
   */
  it('re-reads once the running turn streams its first message back', async () => {
    getSessionMock
      // The working-directory effect's own read — settled, and irrelevant.
      .mockResolvedValueOnce({ data: { working_dir: '/w' } } as never)
      // Bind read — the daemon has not classified this turn yet.
      .mockResolvedValueOnce({ data: { privacy_tier: 'public' } } as never)
      .mockResolvedValue({ data: { privacy_tier: 'private' } } as never);

    // The chat is created and a turn is already in flight — the composer mounts
    // INTO a running turn, which is what the Home-composer submit does, so
    // there is no idle→busy transition to observe.
    const { rerender } = renderChatInput('chat-a', ChatState.Streaming, 1);
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('public'));

    // The daemon streams the assistant's message back. That is emitted after the
    // turn started, hence after the raise was written.
    rerender(chatInput('chat-a', ChatState.Streaming, 2));

    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));
  });

  it('re-reads at most once per turn', async () => {
    // `getSession` carries the whole transcript, and the ratchet fires once at
    // the start of a turn — so a tool-heavy turn must not re-fetch the
    // conversation on every message it appends.
    getSessionMock
      .mockResolvedValueOnce({ data: { working_dir: '/w' } } as never)
      .mockResolvedValue({ data: { privacy_tier: 'public' } } as never);

    const { rerender } = renderChatInput('chat-a', ChatState.Streaming, 1);
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('public'));
    const afterBind = getSessionMock.mock.calls.length;

    rerender(chatInput('chat-a', ChatState.Streaming, 2));
    await waitFor(() => expect(getSessionMock.mock.calls.length).toBe(afterBind + 1));

    for (const n of [3, 4, 5, 6]) {
      rerender(chatInput('chat-a', ChatState.Thinking, n));
      rerender(chatInput('chat-a', ChatState.Streaming, n));
    }
    await act(async () => {
      await Promise.resolve();
    });

    expect(getSessionMock.mock.calls.length).toBe(afterBind + 1);
  });

  it('stops re-reading once the answer is private', async () => {
    // The ratchet only raises within a chat, so no further read during the turn
    // can change the answer.
    getSessionMock.mockResolvedValue({ data: { privacy_tier: 'private' } } as never);

    const { rerender } = renderChatInput('chat-a', ChatState.Streaming, 1);
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));
    const afterBind = getSessionMock.mock.calls.length;

    rerender(chatInput('chat-a', ChatState.Streaming, 2));
    await act(async () => {
      await Promise.resolve();
    });

    expect(getSessionMock.mock.calls.length).toBe(afterBind);
  });

  it('does not let a turn in the previous chat answer for this one', async () => {
    // The turn watcher shares the bind read's generation counter, so a read
    // issued for chat-a cannot land on chat-b — the failure the whole
    // `unresolved` posture exists to prevent, arriving through the new door.
    const slowA = deferred<{ data: { privacy_tier: string } }>();
    getSessionMock
      // The working-directory effect's own read — settled, and irrelevant.
      .mockResolvedValueOnce({ data: { working_dir: '/w' } } as never)
      .mockReturnValueOnce(slowA.promise as never)
      // chat-b's own reads never land, so `unresolved` is the only honest answer.
      .mockReturnValue(new Promise(() => {}) as never);

    const { rerender } = renderChatInput('chat-a');
    rerender(chatInput('chat-b'));
    expect(screen.getByTestId('tier-probe')).toHaveTextContent('unresolved');

    await act(async () => {
      slowA.resolve({ data: { privacy_tier: 'private' } });
      await slowA.promise;
    });

    expect(screen.getByTestId('tier-probe')).toHaveTextContent('unresolved');
  });
});

describe('ChatInput prefers the chat stream`s tier over its own read', () => {
  /**
   * F-12's own premise was that "the composer cannot see the ratchet directly —
   * no event announces it". One does now: the reply stream states the
   * post-ratchet classification in its first frames and the chat stream patches
   * its cached row from it. That answer is strictly fresher than this
   * component's read, which cannot fire until the transcript has grown.
   */
  it('takes the row`s tier even while its own read still says public', async () => {
    getSessionMock.mockResolvedValue({ data: { privacy_tier: 'public' } } as never);

    renderChatInput('chat-stream-wins', ChatState.Idle, 0, 'private');

    // Let the component's own read land, so this is a genuine disagreement
    // rather than a race the row happened to win.
    await waitFor(() => expect(getSessionMock).toHaveBeenCalled());
    expect(screen.getByTestId('tier-probe')).toHaveTextContent('private');
  });

  /**
   * The fallback is not decoration. A transcript surface, an observer tab, or
   * any caller that does not thread the row has only this component's own read
   * — and `undefined` from the row must mean "I know nothing", never "public".
   */
  it('falls back to its own read when the row carries nothing', async () => {
    getSessionMock.mockResolvedValue({ data: { privacy_tier: 'private' } } as never);

    renderChatInput('chat-no-row', ChatState.Idle, 0, undefined);

    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));
  });

  /**
   * A rebind must not leave the previous chat's row tier asserting about this
   * one — the same failure mode as the first test in the suite above, on the
   * new input. `BaseChat` guards it by only passing the row when its id matches
   * the session, and the composer must honour whatever it is handed.
   */
  it('follows the row it is given when the chat rebinds', async () => {
    getSessionMock.mockResolvedValue({ data: { privacy_tier: 'public' } } as never);

    const { rerender } = renderChatInput('chat-x', ChatState.Idle, 0, 'private');
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('private'));

    rerender(chatInput('chat-y', ChatState.Idle, 0, undefined));
    await waitFor(() => expect(screen.getByTestId('tier-probe')).toHaveTextContent('public'));
  });
});
