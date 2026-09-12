import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import ProgressiveMessageList from './ProgressiveMessageList';
import { ChatState } from '../types/chatState';
import type { Message } from '../api';

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
    expect(screen.getAllByText('Working on the result').length).toBeGreaterThan(0);
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

  it("hands the indicator whether this tab can stop the turn, so the nudge can't lie", () => {
    // Past the 45 s nudge threshold on a live turn. Desktop (the default)
    // points at the composer; a subagent's tab in a browser has none (SD-8).
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
    expect(screen.getByText(/stop the turn from the composer/)).toBeInTheDocument();

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
    expect(screen.getByText('Still working.')).toBeInTheDocument();
    expect(screen.queryByText(/stop the turn from the composer/)).toBeNull();
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
