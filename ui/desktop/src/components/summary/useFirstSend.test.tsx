import { act, render, renderHook } from '@testing-library/react';
import { useLayoutEffect, useRef, useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../../api';
import { ChatState } from '../../types/chatState';
import { summaryFigures, summaryTokensValue } from '../BaseChat';
import { ChatSummary } from '../ChatSummary';
import ProgressiveMessageList from '../ProgressiveMessageList';
import { COMPOSER_COPY } from '../composer/copy';
import { summaryCopy } from './copy';
import { FirstSendTranscript } from './FirstSendTranscript';
import {
  composerFollowsUp,
  FIRST_SEND_ROW_ID,
  firstSendMessage,
  useFirstSend,
} from './useFirstSend';

/**
 * Vision V2-R8-2: one send in a new chat at 1440, timed from Enter. At 81 ms the
 * composer was cleared; the rail read "Tokens 0" from 160 ms, nothing from
 * 470 ms (the load) and "N/A" at 806 ms; the message reached the transcript at
 * 753 ms, so for 0.67 s it was in neither place.
 *
 * BaseChat cannot be mounted in jsdom (`BaseChat.privacy.test.tsx`), so the
 * harness below joins the real hook, the real `summaryFigures`, the real
 * `ChatSummary`, `FirstSendTranscript` and `ProgressiveMessageList` the way
 * BaseChat does (`BaseChat.summaryRail.test.ts` pins that wiring at its source)
 * and plays the first send against a fake daemon: `/agent/start` answers after
 * 500 ms, the load takes 280 ms, the ledger answers 50 ms after the session.
 * The first case is the instrument: the wiring before the fix, given the same
 * steps, shows the gap and the "0".
 *
 * WS-SUMMARY-4: with the row drawn, the composer's placeholder still followed
 * the store's count, so it asked "Ask Biorouter anything…" under the drawn
 * message until the store took it. The harness words it the way ChatInput does
 * (`followUp ?? messagesLength > 0`); `placeholderFromStore` is that wiring.
 */

const SEND = 'List 5 tiny to-do items for watering a plant.';
const CHAT = '20261012_1';

interface Snapshot {
  at: number;
  transcript: string;
  tokens: string;
  toolCalls: string;
  placeholder: string;
}

function tokensValue(container: HTMLElement): string {
  const label = [...container.querySelectorAll('dt')].find((dt) =>
    dt.textContent?.includes(summaryCopy.tokens)
  );
  return label?.nextElementSibling?.textContent ?? '';
}

function toolCallsValue(container: HTMLElement): string {
  const label = [...container.querySelectorAll('dt')].find(
    (dt) => dt.textContent === summaryCopy.toolCalls
  );
  return label?.nextElementSibling?.textContent ?? '';
}

interface HarnessApi {
  send: (text: string) => Promise<void>;
}

/**
 * BaseChat's first send, step by step. `legacy` is the wiring before V2-R8-2:
 * no provisional row, every figure blank while loading and the Tokens of a
 * chat with no session read from the counters of no chat.
 */
function Harness({
  legacy,
  placeholderFromStore = false,
  api,
  log,
  startedAt,
}: {
  legacy: boolean;
  placeholderFromStore?: boolean;
  api: { current: HarnessApi | null };
  log: Snapshot[];
  startedAt: { current: number };
}) {
  const [sessionId, setSessionId] = useState('');
  const [session, setSession] = useState<{ id: string } | undefined>();
  const [chatState, setChatState] = useState(ChatState.Idle);
  const [messages, setMessages] = useState<Message[]>([]);
  const [ledgerFor, setLedgerFor] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const firstSend = useFirstSend({ sessionId, messageCount: messages.length, failed: false });

  // `useCostTracking`: settled with no session, else once this chat's ledger answered.
  const ledgerSettled = !session || ledgerFor === session.id;
  const chatLoading = chatState === ChatState.LoadingConversation;
  // A new chat's counters: live zeros and no persisted row, so no billed figure.
  const billed = session ? null : 0;
  const figures = legacy
    ? {
        toolCalls: chatLoading ? null : '0',
        billedTokens: summaryTokensValue({ loading: chatLoading, ledgerSettled, billed }),
        artifacts: chatLoading ? null : '0',
        codeDelta: chatLoading ? null : { added: 0, removed: 0 },
      }
    : summaryFigures({
        loading: chatLoading,
        newChat: firstSend.creating,
        hasSession: !!session,
        ledgerSettled,
        billed,
        toolCalls: 0,
        artifacts: 0,
        codeDelta: { added: 0, removed: 0 },
      });

  // ChatInput's placeholder, `followUp ?? messagesLength > 0`. A new chat's row
  // knows no messages, so `messagesLength` is the store's count.
  const followUp =
    legacy || placeholderFromStore
      ? undefined
      : composerFollowsUp({
          storedMessages: messages.length,
          firstSendShown: !!firstSend.message,
          knownMessages: 0,
        });
  const placeholder =
    (followUp ?? messages.length > 0)
      ? COMPOSER_COPY.placeholderFollowUp
      : COMPOSER_COPY.placeholderEmpty;

  api.current = {
    // handleFormSubmit's pre-session branch, then the route change.
    send: async (text: string) => {
      if (!legacy) firstSend.begin(text);
      setChatState(ChatState.Idle);
      const created = await new Promise<{ id: string }>((resolve) =>
        setTimeout(() => resolve({ id: CHAT }), 500)
      );
      if (!legacy) firstSend.created(created.id);
      setSessionId(created.id);
      // The chat's store: the load starts a frame later and takes 280 ms.
      setTimeout(() => setChatState(ChatState.LoadingConversation), 16);
      setTimeout(() => {
        setSession({ id: created.id });
        setChatState(ChatState.Idle);
        // The ledger answers 50 ms after the session.
        setTimeout(() => setLedgerFor(created.id), 50);
        // The auto-submit: the store appends the message once it is loaded.
        void Promise.resolve().then(() => {
          setMessages([
            {
              id: 'stored-1',
              role: 'user',
              created: Math.floor(Date.now() / 1000),
              content: [{ type: 'text', text }],
              metadata: { userVisible: true, agentVisible: true },
            },
          ]);
          setChatState(ChatState.Thinking);
        });
      }, 296);
    },
  };

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const transcript = container.querySelector('[data-testid="transcript"]');
    log.push({
      at: Date.now() - startedAt.current,
      transcript: transcript?.textContent ?? '',
      tokens: tokensValue(container),
      toolCalls: toolCallsValue(container),
      placeholder: container.querySelector('textarea')?.placeholder ?? '',
    });
  });

  const transcriptList = (
    <ProgressiveMessageList
      messages={messages}
      chat={{ sessionId }}
      isUserMessage={(m) => m.role === 'user'}
      isStreamingMessage={chatState !== ChatState.Idle}
      chatState={chatState}
      onMessageUpdate={() => undefined}
      onOpenArtifact={() => undefined}
      onRunInTerminal={null}
    />
  );
  const showsTranscript = messages.length > 0 || (!legacy && !!firstSend.message);
  return (
    <div ref={containerRef}>
      <div data-testid="transcript">
        {showsTranscript &&
          (!legacy && firstSend.message ? (
            <FirstSendTranscript
              message={firstSend.message}
              sessionId={sessionId}
              onOpenArtifact={() => undefined}
            />
          ) : (
            transcriptList
          ))}
      </div>
      <textarea aria-label="Message" placeholder={placeholder} readOnly />
      <ChatSummary
        {...figures}
        todos={{ items: [], loading: false, error: false, refresh: () => undefined }}
        hasWorkflow={false}
        onWorkflow={() => undefined}
        onDiagnostics={() => undefined}
      />
    </div>
  );
}

async function playFirstSend(legacy: boolean, placeholderFromStore = false) {
  const api: { current: HarnessApi | null } = { current: null };
  const log: Snapshot[] = [];
  const startedAt = { current: Date.now() };
  const view = render(
    <Harness
      legacy={legacy}
      placeholderFromStore={placeholderFromStore}
      api={api}
      log={log}
      startedAt={startedAt}
    />
  );
  const before = log.length;
  startedAt.current = Date.now();
  await act(async () => {
    void api.current!.send(SEND);
  });
  const afterSend = log.length;
  await playFrames(1200);
  return {
    view,
    log,
    beforeSend: log[before - 1],
    sinceSend: log.slice(before),
    firstAfterSend: log[afterSend - 1],
  };
}

/** Time passes a frame at a time, each frame its own commit, as in the app. */
async function playFrames(ms: number) {
  for (let elapsed = 0; elapsed < ms; elapsed += 4) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4);
    });
  }
}

/** The distinct values in order, so "0, 0, empty, empty, N/A" reads "0, empty, N/A". */
function changes(values: string[]): string[] {
  return values.filter((value, index) => index === 0 || value !== values[index - 1]);
}

beforeEach(() => {
  vi.useFakeTimers();
  // A fixed clock, so the send and the stored row fall in the same minute.
  vi.setSystemTime(new Date(2026, 9, 11, 19, 39, 0));
});
afterEach(() => {
  vi.useRealTimers();
});

describe('a new chat’s first send (V2-R8-2)', () => {
  it('instrument: before the fix the message left the screen and Tokens read 0, nothing, N/A', async () => {
    const { sinceSend } = await playFirstSend(true);
    const missing = sinceSend.filter((snapshot) => !snapshot.transcript.includes(SEND));
    // From the send until the store held the message, the text was nowhere.
    expect(missing.length).toBeGreaterThan(0);
    expect(missing[missing.length - 1].at).toBeGreaterThanOrEqual(500);
    expect(changes(sinceSend.map((snapshot) => snapshot.tokens))).toEqual(['0', '', 'N/A']);
  });

  it('shows the message in the first commit after the send and in every commit after it', async () => {
    const { sinceSend, firstAfterSend } = await playFirstSend(false);
    expect(firstAfterSend.transcript).toContain(SEND);
    expect(sinceSend.length).toBeGreaterThan(4);
    for (const snapshot of sinceSend) expect(snapshot.transcript).toContain(SEND);
  });

  it('states Tokens once, from empty to its answer, never "0" first', async () => {
    const { sinceSend } = await playFirstSend(false);
    expect(changes(sinceSend.map((snapshot) => snapshot.tokens))).toEqual(['', 'N/A']);
    // The counts of a chat that is new and empty are its zeros, held through the load.
    expect(changes(sinceSend.map((snapshot) => snapshot.toolCalls))).toEqual(['0']);
  });

  it('instrument: a placeholder that follows the store asks the open question under the drawn message', async () => {
    const { sinceSend } = await playFirstSend(false, true);
    const contradicted = sinceSend.filter(
      (snapshot) =>
        snapshot.transcript.includes(SEND) &&
        snapshot.placeholder === COMPOSER_COPY.placeholderEmpty
    );
    expect(contradicted.length).toBeGreaterThan(0);
    expect(contradicted[contradicted.length - 1].at).toBeGreaterThanOrEqual(500);
  });

  it('says "Ask a follow-up" from the commit that draws the message, and in every commit after it', async () => {
    const { beforeSend, sinceSend, firstAfterSend } = await playFirstSend(false);
    expect(beforeSend.placeholder).toBe(COMPOSER_COPY.placeholderEmpty);
    expect(firstAfterSend.transcript).toContain(SEND);
    expect(changes(sinceSend.map((snapshot) => snapshot.placeholder))).toEqual([
      COMPOSER_COPY.placeholderFollowUp,
    ]);
  });

  it('puts the stored row exactly where the first-send row was, at rest', async () => {
    const api: { current: HarnessApi | null } = { current: null };
    const log: Snapshot[] = [];
    const view = render(
      <Harness legacy={false} api={api} log={log} startedAt={{ current: Date.now() }} />
    );
    await act(async () => {
      void api.current!.send(SEND);
    });
    const row = () =>
      view.container.querySelector<HTMLElement>(
        '[data-testid="transcript"] [data-testid="message-container"]'
      )!;
    // React's generated ids differ between the two mounts and mean nothing on screen.
    const markup = () => row().outerHTML.replace(/ id="_r_\w+_"/g, '');
    const provisional = markup();
    expect(row()).toHaveAttribute('data-role', 'user');
    await playFrames(1200);
    // The store's row: the same markup (the list keys rows by id, not into the
    // DOM), and no entrance, since its list mounted with it.
    expect(row().textContent).toContain(SEND);
    expect(markup()).toBe(provisional);
    expect(row()).not.toHaveClass('br-enter');
    expect(view.container.querySelectorAll('[data-testid="message-container"]')).toHaveLength(1);
  });
});

describe('useFirstSend', () => {
  const props = { sessionId: '', messageCount: 0, failed: false };

  it('holds the message from begin, through the chat’s creation, to the store taking it', () => {
    const { result, rerender } = renderHook((p) => useFirstSend(p), { initialProps: props });
    expect(result.current).toMatchObject({ creating: false, message: null });

    act(() => result.current.begin(SEND));
    expect(result.current.creating).toBe(true);
    expect(result.current.message).toMatchObject({
      id: FIRST_SEND_ROW_ID,
      role: 'user',
      content: [{ type: 'text', text: SEND }],
    });
    const message = result.current.message;

    act(() => result.current.created(CHAT));
    rerender({ ...props, sessionId: CHAT });
    expect(result.current.creating).toBe(true);
    // One row object across renders, so the list it is drawn by does not re-render.
    expect(result.current.message).toBe(message);

    rerender({ ...props, sessionId: CHAT, messageCount: 1 });
    expect(result.current).toMatchObject({ creating: false, message: null });
    // Spent: a chat whose messages are later taken away never shows it again.
    rerender({ ...props, sessionId: CHAT, messageCount: 0 });
    expect(result.current).toMatchObject({ creating: false, message: null });
  });

  it('is not this chat’s once the tab shows another chat', () => {
    const { result, rerender } = renderHook((p) => useFirstSend(p), { initialProps: props });
    act(() => result.current.begin(SEND));
    act(() => result.current.created(CHAT));
    rerender({ ...props, sessionId: 'another-chat' });
    expect(result.current).toMatchObject({ creating: false, message: null });
  });

  it('withholds the row over a turn error or a failed load, and ends when the text goes back', () => {
    const { result, rerender } = renderHook((p) => useFirstSend(p), { initialProps: props });
    act(() => result.current.begin(SEND));
    rerender({ ...props, failed: true });
    expect(result.current).toMatchObject({ creating: true, message: null });
    rerender(props);
    expect(result.current.message).not.toBeNull();
    act(() => result.current.end());
    expect(result.current).toMatchObject({ creating: false, message: null });
  });

  it('draws no row for a message of attachments alone, and still knows the chat is new', () => {
    const { result } = renderHook((p) => useFirstSend(p), { initialProps: props });
    act(() => result.current.begin('  '));
    expect(result.current).toMatchObject({ creating: true, message: null });
  });

  it('builds a text row with nothing the store reads images for', () => {
    expect(firstSendMessage('hello', 1)).toEqual({
      id: FIRST_SEND_ROW_ID,
      role: 'user',
      created: 1,
      content: [{ type: 'text', text: 'hello' }],
      metadata: { userVisible: true, agentVisible: true },
    });
  });
});

describe('composerFollowsUp', () => {
  const none = { storedMessages: 0, firstSendShown: false, knownMessages: 0 };

  it('asks the open question only when the chat shows no message and expects none', () => {
    expect(composerFollowsUp(none)).toBe(false);
    expect(composerFollowsUp({ ...none, storedMessages: 1 })).toBe(true);
    expect(composerFollowsUp({ ...none, firstSendShown: true })).toBe(true);
    expect(composerFollowsUp({ ...none, knownMessages: 12 })).toBe(true);
  });
});

describe('summaryFigures', () => {
  const base = {
    loading: false,
    newChat: false,
    hasSession: true,
    ledgerSettled: true,
    billed: 2_200_000,
    toolCalls: 6,
    artifacts: 1,
    codeDelta: { added: 3, removed: 1 },
  };

  it('is every figure once the chat is in', () => {
    expect(summaryFigures(base)).toEqual({
      toolCalls: '6',
      billedTokens: '2.2M',
      artifacts: '1',
      codeDelta: { added: 3, removed: 1 },
    });
  });

  it('is nothing while a chat loads, and no Tokens before its ledger answers', () => {
    expect(summaryFigures({ ...base, loading: true, hasSession: false })).toEqual({
      toolCalls: null,
      billedTokens: null,
      artifacts: null,
      codeDelta: null,
    });
    expect(summaryFigures({ ...base, ledgerSettled: false }).billedTokens).toBeNull();
  });

  it('gives a chat created here its zeros through the load, and Tokens only from its own ledger', () => {
    const creating = {
      ...base,
      newChat: true,
      hasSession: false,
      billed: 0,
      toolCalls: 0,
      artifacts: 0,
      codeDelta: { added: 0, removed: 0 },
    };
    for (const loading of [false, true]) {
      expect(summaryFigures({ ...creating, loading })).toEqual({
        toolCalls: '0',
        billedTokens: null,
        artifacts: '0',
        codeDelta: { added: 0, removed: 0 },
      });
    }
    expect(
      summaryFigures({ ...creating, hasSession: true, ledgerSettled: true, billed: null })
        .billedTokens
    ).toBe('N/A');
  });
});
