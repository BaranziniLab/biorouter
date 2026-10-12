import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import ProgressiveMessageList from './ProgressiveMessageList';
import { ChatState } from '../types/chatState';
import type { Message } from '../api';
import { transcriptCopy } from './transcript/copy';

// The transcript is no longer a display surface for artifacts — it can only hand
// one to the panel — so `onOpenArtifact` is required all the way down the chain.
const noopOpenArtifact = vi.fn();

const assistantWithToolCall: Message = {
  id: 'assistant-1',
  role: 'assistant',
  created: 1_700_000_000,
  metadata: { userVisible: true, agentVisible: true },
  content: [
    {
      type: 'toolRequest',
      id: 'tool-1',
      toolCall: {
        status: 'success',
        value: { name: 'developer__shell', arguments: { command: 'ls' } },
      },
    },
  ],
} as Message;

const toolResponseOnlyUserMessage: Message = {
  id: 'user-1',
  role: 'user',
  created: 1_700_000_001,
  metadata: { userVisible: true, agentVisible: true },
  content: [
    {
      type: 'toolResponse',
      id: 'tool-1',
      toolResult: { status: 'success', value: { content: [] } },
    },
  ],
} as Message;

const messages = [assistantWithToolCall, toolResponseOnlyUserMessage];

const liveProps = {
  messages,
  chat: { sessionId: 'live-session' },
  toolCallNotifications: new Map(),
  isUserMessage: (m: Message) => m.role !== 'assistant',
  onRunInTerminal: null,
};

describe('ProgressiveMessageList trailing activity indicator', () => {
  it('never renders the trailing activity indicator on a replayed historical session', () => {
    // Props copied from SessionHistoryView: no isStreamingMessage, no chatState,
    // no timestamps. This is the load-bearing guarantee — a saved transcript
    // must never sprout a running clock.
    render(
      <ProgressiveMessageList
        messages={messages}
        onRunInTerminal={null}
        chat={{ sessionId: 'session-preview' }}
        toolCallNotifications={new Map()}
        isUserMessage={(m: Message) => m.role !== 'assistant'}
        batchSize={15}
        batchDelay={30}
        showLoadingThreshold={30}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.queryByTestId('turn-activity-indicator')).toBeNull();
  });

  it('renders the indicator under the tool block while a live turn awaits the model', () => {
    render(
      <ProgressiveMessageList
        {...liveProps}
        isStreamingMessage
        chatState={ChatState.Streaming}
        lastMessageAt={Date.now()}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.getByTestId('turn-activity-indicator')).toBeInTheDocument();
    expect(screen.getAllByText(transcriptCopy.activity.thinking).length).toBeGreaterThan(0);
  });

  it('removes the indicator the moment the turn goes idle', () => {
    const { rerender } = render(
      <ProgressiveMessageList
        {...liveProps}
        isStreamingMessage
        chatState={ChatState.Streaming}
        lastMessageAt={Date.now()}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(screen.getByTestId('turn-activity-indicator')).toBeInTheDocument();

    rerender(
      <ProgressiveMessageList
        {...liveProps}
        isStreamingMessage={false}
        chatState={ChatState.Idle}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.queryByTestId('turn-activity-indicator')).toBeNull();
  });

  it('places the indicator AFTER the last rendered message, not above the list', () => {
    // "Trailing underneath the tool call section" is the whole requirement; a
    // refactor that moved it above the list would otherwise pass silently.
    const { container } = render(
      <ProgressiveMessageList
        {...liveProps}
        isStreamingMessage
        chatState={ChatState.Streaming}
        lastMessageAt={Date.now()}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    const children: Element[] = Array.from(
      container.firstElementChild?.parentElement?.children ?? []
    );
    const lastMessageIndex = children.reduce(
      (last, el, i) => (el.getAttribute('data-testid') === 'message-container' ? i : last),
      -1
    );
    const indicatorIndex = children.findIndex(
      (el: Element) => el.getAttribute('data-testid') === 'trailing-activity-container'
    );

    expect(lastMessageIndex).toBeGreaterThanOrEqual(0);
    expect(indicatorIndex).toBeGreaterThan(lastMessageIndex);
  });

  it('never adds a "still working" nudge to a long wait, on any surface', () => {
    // The 45 s nudge is gone (spec 3.6). `canStopTurn` is still accepted from
    // BaseChat and changes nothing.
    const longAgo = Date.now() - 46_000;
    const { rerender } = render(
      <ProgressiveMessageList
        {...liveProps}
        isStreamingMessage
        chatState={ChatState.Streaming}
        lastMessageAt={longAgo}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(screen.queryByText(/Still working/)).toBeNull();

    rerender(
      <ProgressiveMessageList
        {...liveProps}
        isStreamingMessage
        chatState={ChatState.Streaming}
        lastMessageAt={longAgo}
        canStopTurn={false}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(screen.queryByText(/Still working/)).toBeNull();
    expect(screen.getByTestId('turn-activity-indicator')).toBeInTheDocument();
  });

  it('shows no indicator while the assistant is streaming visible prose', () => {
    const prose: Message = {
      id: 'assistant-2',
      role: 'assistant',
      created: 1_700_000_002,
      metadata: { userVisible: true, agentVisible: true },
      content: [{ type: 'text', text: 'Here is what I found' }],
    } as Message;

    render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[assistantWithToolCall, toolResponseOnlyUserMessage, prose]}
        isStreamingMessage
        chatState={ChatState.Streaming}
        lastMessageAt={Date.now()}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    expect(screen.queryByTestId('turn-activity-indicator')).toBeNull();
  });
});

describe('ProgressiveMessageList render stability', () => {
  // NOTE ON TEST STRENGTH — read before trusting this.
  //
  // "Maximum update depth exceeded" was reproduced LIVE in the Electron GUI on
  // 2026-07-18 and confirmed PRE-EXISTING (it reproduced with the trailing
  // indicator commit reverted). The fix removes `renderedCount` and
  // `onRenderingComplete` from the batching effect's dependency array; the
  // effect only ever WRITES `renderedCount`, through a functional updater.
  //
  // This test does NOT gate that fix. It was checked against the unfixed
  // component and still passed: jsdom + Testing Library's act() batching do not
  // reproduce the nested-update cascade, which needs the real scheduler. It is
  // kept as a smoke test that streaming re-renders stay clean, not as proof.
  // The load-bearing evidence is the live GUI check recorded in
  // docs/history/streaming-tool-call-ui-2026-07/streaming-implementation-status.md §10.
  it('renders cleanly while messages stream in (smoke, not a depth-loop gate)', () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    const grow = (n: number): Message[] =>
      Array.from({ length: n }, (_, i) => ({
        id: `m-${i}`,
        role: i % 2 === 0 ? 'assistant' : 'user',
        created: 1_700_000_000 + i,
        metadata: { userVisible: true, agentVisible: true },
        content: [{ type: 'text', text: `message ${i}` }],
      })) as Message[];

    // A fresh callback identity per render, exactly as BaseChat's
    // useCallback(..., [messages.length]) produces during a stream.
    const renderWith = (n: number) => (
      <ProgressiveMessageList
        {...liveProps}
        messages={grow(n)}
        isStreamingMessage
        chatState={ChatState.Streaming}
        onRenderingComplete={() => {}}
        batchSize={15}
        batchDelay={5}
        showLoadingThreshold={30}
        onOpenArtifact={noopOpenArtifact}
      />
    );

    const { rerender } = render(renderWith(1));
    for (let n = 2; n <= 40; n++) rerender(renderWith(n));

    const depthErrors = errorSpy.mock.calls
      .map((args) => args.map(String).join(' '))
      .filter((text) => /Maximum update depth exceeded/i.test(text));

    errorSpy.mockRestore();
    expect(depthErrors).toEqual([]);
  });
});

const text = (id: string, role: 'user' | 'assistant', words: string): Message =>
  ({
    id,
    role,
    created: 1_700_000_100,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text: words }],
  }) as Message;

const toolCall = (id: string, toolId: string): Message =>
  ({
    id,
    role: 'assistant',
    created: 1_700_000_100,
    metadata: { userVisible: true, agentVisible: true },
    content: [
      {
        type: 'toolRequest',
        id: toolId,
        toolCall: { status: 'success', value: { name: 'developer__shell', arguments: {} } },
      },
    ],
  }) as Message;

const toolResult = (id: string, toolId: string): Message =>
  ({
    id,
    role: 'user',
    created: 1_700_000_100,
    metadata: { userVisible: true, agentVisible: true },
    content: [
      {
        type: 'toolResponse',
        id: toolId,
        toolResult: { status: 'success', value: { content: [] } },
      },
    ],
  }) as Message;

const rows = (container: HTMLElement) =>
  Array.from(container.querySelectorAll<HTMLElement>('[data-testid="message-container"]'));

describe('ProgressiveMessageList rhythm (spec 3.6)', () => {
  it('puts 24px between turns, 8px between blocks and 4px between tool rows', () => {
    const { container } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[
          text('u1', 'user', 'question'),
          toolCall('a1', 't1'),
          toolResult('r1', 't1'),
          toolCall('a2', 't2'),
          toolResult('r2', 't2'),
          text('a3', 'assistant', 'answer'),
          text('u2', 'user', 'follow-up'),
        ]}
        isUserMessage={(m: Message) => m.role === 'user'}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    const gaps = rows(container).map((row) => row.className.match(/\bmt-\d\b/)?.[0]);
    // question, tool, (result), tool, (result), answer, follow-up
    expect(gaps).toEqual(['mt-0', 'mt-6', 'mt-0', 'mt-1', 'mt-0', 'mt-2', 'mt-6']);
    // The row names its gap for the floating actions' rise (message-meta.css).
    expect(rows(container).map((row) => row.dataset.gap)).toEqual([
      'first',
      'turn',
      'none',
      'tool',
      'none',
      'block',
      'turn',
    ]);
    // The first row leaves room above it for the 26px rise.
    expect(rows(container)[0]).toHaveClass('pt-7');
  });

  // Vision V2-03: the stream stores an empty text message before each of the
  // model's next calls (chats 20260621_46 and _47). Each sat in the list as a
  // zero-height row with a block's 8px, so two tool rows in separate messages
  // stood 16px apart. A message that draws nothing takes no gap, and the tool
  // row after it measures from the tool row before it: 4px.
  it('keeps tool rows 4px apart across the empty messages the stream leaves between calls', () => {
    const { container } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[
          text('u1', 'user', 'question'),
          toolCall('a1', 't1'),
          toolResult('r1', 't1'),
          text('e1', 'assistant', ''),
          toolCall('a2', 't2'),
          toolResult('r2', 't2'),
          text('e2', 'assistant', '<info-msg>model only</info-msg>'),
          toolCall('a3', 't3'),
          toolResult('r3', 't3'),
          text('e3', 'assistant', '   '),
          text('a4', 'assistant', 'answer'),
        ]}
        isUserMessage={(m: Message) => m.role === 'user'}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    const gaps = rows(container).map((row) => row.className.match(/\bmt-\d\b/)?.[0]);
    // question, tool, (result), (empty), tool, (result), (empty), tool, (result), (empty), answer
    expect(gaps).toEqual([
      'mt-0',
      'mt-6',
      'mt-0',
      'mt-0',
      'mt-1',
      'mt-0',
      'mt-0',
      'mt-1',
      'mt-0',
      'mt-0',
      'mt-2',
    ]);
  });

  // Vision V2-12, round 3: an answer 8px under a tool row floats its actions
  // 18px into that row. The list marks the row the cluster reaches, so its last
  // tool row ends the cluster's width short (`message-meta.css`).
  it('marks the row an answer floats its actions into, and no other', () => {
    const { container } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[
          text('u1', 'user', 'question'),
          toolCall('a1', 't1'),
          toolResult('r1', 't1'),
          text('e1', 'assistant', ''),
          text('a2', 'assistant', 'answer'),
          text('u2', 'user', 'follow-up'),
          text('a3', 'assistant', 'answer under a question'),
        ]}
        isUserMessage={(m: Message) => m.role === 'user'}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    // The tool row, past the tool result and the empty message between them;
    // an answer under a question reaches only the bubble's padding.
    expect(rows(container).map((row) => row.hasAttribute('data-actions-below'))).toEqual([
      false,
      true,
      false,
      false,
      false,
      false,
      false,
    ]);
  });

  it('marks nothing when the answer floats no actions into the row above', () => {
    const { container } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[
          text('u1', 'user', 'question'),
          toolCall('a1', 't1'),
          toolResult('r1', 't1'),
          // A thought draws its own short line between the cluster and the tool row.
          text('a2', 'assistant', '<think>check the output</think>answer'),
          toolCall('a3', 't3'),
          toolResult('r3', 't3'),
          // The daemon's stored notice draws as the error row, with no actions.
          text('a4', 'assistant', 'Ran into this error: Server error: 500\n\nTry again.'),
          toolCall('a5', 't5'),
          toolResult('r5', 't5'),
          // More of the turn: a tool call carries no cluster.
          toolCall('a6', 't6'),
        ]}
        isUserMessage={(m: Message) => m.role === 'user'}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(container.querySelector('[data-actions-below]')).toBeNull();
  });

  it('measures the trailing activity line from the last row that shows something', () => {
    const { getByTestId } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[
          text('u1', 'user', 'question'),
          toolCall('a1', 't1'),
          text('e1', 'assistant', ''),
        ]}
        isUserMessage={(m: Message) => m.role === 'user'}
        isStreamingMessage
        chatState={ChatState.Streaming}
        turnStartedAt={Date.now()}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(getByTestId('trailing-activity-container').className).toMatch(/\bmt-1\b/);
  });

  it('draws a saved chat at rest: no entrance on rows that were there at mount', () => {
    const { container } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[text('u1', 'user', 'question'), text('a1', 'assistant', 'answer')]}
        chatState={ChatState.Idle}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(container.querySelector('.br-enter')).toBeNull();
    // ...and so are their tool rows (WS-TOOLS' `[data-motion-still]` rest).
    expect(rows(container).every((row) => row.hasAttribute('data-motion-still'))).toBe(true);
  });

  it('plays the entrance only on a row appended live', () => {
    const before = [text('u1', 'user', 'question'), text('a1', 'assistant', 'answer')];
    const { container, rerender } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={before}
        chatState={ChatState.Idle}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    rerender(
      <ProgressiveMessageList
        {...liveProps}
        messages={[...before, text('u2', 'user', 'follow-up')]}
        isStreamingMessage
        chatState={ChatState.Thinking}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    const entered = rows(container).filter((row) => row.classList.contains('br-enter'));
    expect(entered).toHaveLength(1);
    expect(entered[0]).toHaveTextContent('follow-up');
    // The live row lets its tool rows rise; the rows that were there stay still.
    const moving = rows(container).filter((row) => !row.hasAttribute('data-motion-still'));
    expect(moving).toEqual(entered);
  });

  it('draws a chat that arrives while loading at rest, and a replay never animates', () => {
    const history = [text('u1', 'user', 'question'), text('a1', 'assistant', 'answer')];
    const { container, rerender } = render(
      <ProgressiveMessageList
        {...liveProps}
        messages={[]}
        chatState={ChatState.LoadingConversation}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    rerender(
      <ProgressiveMessageList
        {...liveProps}
        messages={history}
        chatState={ChatState.LoadingConversation}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(container.querySelector('.br-enter')).toBeNull();

    // A read-only replay passes no chat state at all.
    const replay = render(
      <ProgressiveMessageList {...liveProps} messages={[]} onOpenArtifact={noopOpenArtifact} />
    );
    replay.rerender(
      <ProgressiveMessageList {...liveProps} messages={history} onOpenArtifact={noopOpenArtifact} />
    );
    expect(replay.container.querySelector('.br-enter')).toBeNull();
  });

  it('says one quiet line while a long chat renders in batches', () => {
    render(
      <ProgressiveMessageList
        {...liveProps}
        messages={Array.from({ length: 60 }, (_, i) =>
          text(`m${i}`, i % 2 ? 'assistant' : 'user', `m${i}`)
        )}
        showLoadingThreshold={50}
        onOpenArtifact={noopOpenArtifact}
      />
    );
    expect(screen.getByText(transcriptCopy.activity.loadingMessages)).toBeInTheDocument();
    expect(screen.queryByText(/Cmd\/Ctrl\+F/)).toBeNull();
  });
});

describe('ProgressiveMessageList opens a long live chat at its end (vision V2-R7-2)', () => {
  // A long chat used to draw its OLDEST batch first and append below, so a tab
  // switch painted its first question, then its middle, then its end. A live
  // chat now draws its newest batch first and adds older batches above.
  const chat60 = Array.from({ length: 60 }, (_, i) =>
    text(`m${i}`, i % 2 ? 'assistant' : 'user', `message number ${i}`)
  );
  const shown = (container: HTMLElement) =>
    rows(container).map((row) => row.textContent?.match(/message number (\d+)/)?.[1]);
  const long = {
    ...liveProps,
    batchSize: 20,
    batchDelay: 20,
    showLoadingThreshold: 50,
    onOpenArtifact: noopOpenArtifact,
  };

  afterEach(() => {
    vi.useRealTimers();
  });

  it('draws the newest batch in its first commit, with the loading line above it', () => {
    vi.useFakeTimers();
    const { container } = render(
      <ProgressiveMessageList {...long} messages={chat60} chatState={ChatState.Idle} />
    );
    expect(shown(container)).toEqual(Array.from({ length: 20 }, (_, i) => String(40 + i)));
    const line = screen.getByTestId('transcript-loading-line');
    // Above every row: that is where the batches are being added.
    expect(line.compareDocumentPosition(rows(container)[0])).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });

  it('adds older batches above until the whole chat is drawn, then settles once', () => {
    vi.useFakeTimers();
    const settled = vi.fn();
    const { container } = render(
      <ProgressiveMessageList
        {...long}
        messages={chat60}
        chatState={ChatState.Idle}
        onRenderingComplete={settled}
      />
    );
    act(() => vi.advanceTimersByTime(20));
    expect(shown(container)[0]).toBe('20');
    expect(shown(container)[shown(container).length - 1]).toBe('59');
    act(() => vi.advanceTimersByTime(20));
    expect(shown(container)).toEqual(Array.from({ length: 60 }, (_, i) => String(i)));
    expect(screen.queryByTestId('transcript-loading-line')).toBeNull();
    expect(settled).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(50));
    expect(settled).toHaveBeenCalledTimes(1);
  });

  it('settles only after the last batch, when the messages arrive inside the empty list’s 50 ms', () => {
    // The empty first render arms the settle timer; messages that land before
    // it fires start batches, and that timer must not fire into the load with
    // 40 of 60 rows drawn (WS-SUMMARY-2-FYI: it ran with 20 drawn, then at 60).
    vi.useFakeTimers();
    const drawnAtSettle: number[] = [];
    const settled = vi.fn(() => drawnAtSettle.push(rows(document.body).length));
    const props = {
      ...long,
      chatState: ChatState.LoadingConversation,
      onRenderingComplete: settled,
    };
    const { container, rerender } = render(<ProgressiveMessageList {...props} messages={[]} />);
    act(() => vi.advanceTimersByTime(10));
    rerender(<ProgressiveMessageList {...props} messages={chat60} />);
    expect(shown(container)).toHaveLength(20);
    // One frame of 5 ms at a time: each timer's state update commits (and its
    // effects run) before the next timer is due, as they do in the app.
    for (let elapsed = 0; elapsed < 200; elapsed += 5) act(() => vi.advanceTimersByTime(5));
    expect(shown(container)).toHaveLength(60);
    expect(drawnAtSettle).toEqual([60]);
  });

  it('keeps the old order on a saved transcript, which reads top down', () => {
    vi.useFakeTimers();
    const { container } = render(<ProgressiveMessageList {...long} messages={chat60} />);
    expect(shown(container)).toEqual(Array.from({ length: 20 }, (_, i) => String(i)));
    const line = screen.getByTestId('transcript-loading-line');
    expect(line.compareDocumentPosition(rows(container)[rows(container).length - 1])).toBe(
      Node.DOCUMENT_POSITION_PRECEDING
    );
    act(() => vi.advanceTimersByTime(20));
    act(() => vi.advanceTimersByTime(20));
    expect(shown(container)).toEqual(Array.from({ length: 60 }, (_, i) => String(i)));
  });

  it('starts from the end when the messages arrive after the list mounted', () => {
    vi.useFakeTimers();
    const { container, rerender } = render(
      <ProgressiveMessageList {...long} messages={[]} chatState={ChatState.LoadingConversation} />
    );
    rerender(
      <ProgressiveMessageList
        {...long}
        messages={chat60}
        chatState={ChatState.LoadingConversation}
      />
    );
    // Not all 60 in one commit, and not the first 20.
    expect(shown(container)).toEqual(Array.from({ length: 20 }, (_, i) => String(40 + i)));
  });

  it('draws a message streamed in during the load at once, and takes no drawn row away', () => {
    vi.useFakeTimers();
    const { container, rerender } = render(
      <ProgressiveMessageList {...long} messages={chat60} chatState={ChatState.Streaming} />
    );
    rerender(
      <ProgressiveMessageList
        {...long}
        messages={[...chat60, text('m60', 'assistant', 'message number 60')]}
        chatState={ChatState.Streaming}
      />
    );
    expect(shown(container)).toEqual(Array.from({ length: 21 }, (_, i) => String(40 + i)));
  });

  it('hands a custom renderer each message with its place in the whole chat', () => {
    vi.useFakeTimers();
    const renderMessage = vi.fn((message: Message, index: number) => (
      <p key={message.id ?? index}>{index}</p>
    ));
    render(
      <ProgressiveMessageList
        {...long}
        messages={chat60}
        chatState={ChatState.Idle}
        renderMessage={renderMessage}
      />
    );
    const indices = renderMessage.mock.calls.map(([message, index]) => [message.id, index]);
    expect(indices[0]).toEqual(['m40', 40]);
    expect(indices[indices.length - 1]).toEqual(['m59', 59]);
  });

  it('draws the rest at once for find', () => {
    vi.useFakeTimers();
    const electron = (window as unknown as { electron?: unknown }).electron;
    (window as unknown as { electron: { platform: string } }).electron = { platform: 'darwin' };
    try {
      const { container } = render(
        <ProgressiveMessageList {...long} messages={chat60} chatState={ChatState.Idle} />
      );
      fireEvent.keyDown(window, { key: 'f', metaKey: true });
      expect(shown(container)).toHaveLength(60);
      expect(screen.queryByTestId('transcript-loading-line')).toBeNull();
    } finally {
      (window as unknown as { electron?: unknown }).electron = electron;
    }
  });
});
