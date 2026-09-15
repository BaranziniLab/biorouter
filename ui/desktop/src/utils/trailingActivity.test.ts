import { describe, expect, it } from 'vitest';
import { Message } from '../api';
import { ChatState } from '../types/chatState';
import { deriveTrailingActivity } from './trailingActivity';

const LAST_MESSAGE_AT = 1_700_000_000_000;
const TURN_STARTED_AT = 1_699_999_990_000;

function message(role: 'user' | 'assistant', content: Message['content']): Message {
  return {
    id: `msg-${role}-${content.length}`,
    role,
    created: Math.floor(LAST_MESSAGE_AT / 1000),
    content,
    metadata: { userVisible: true, agentVisible: true },
  } as Message;
}

const toolRequest = (id: string) =>
  ({
    type: 'toolRequest',
    id,
    toolCall: {
      status: 'success',
      value: { name: 'developer__shell', arguments: { command: 'ls' } },
    },
  }) as Message['content'][number];

const toolResponse = (id: string) =>
  ({
    type: 'toolResponse',
    id,
    toolResult: { status: 'success', value: { content: [] } },
  }) as Message['content'][number];

const text = (value: string) => ({ type: 'text', text: value }) as Message['content'][number];

const toolResponseOnlyMessage = message('user', [toolResponse('t1')]);

const live = {
  isTurnActive: true,
  chatState: ChatState.Streaming,
  turnStartedAt: TURN_STARTED_AT,
  lastMessageAt: LAST_MESSAGE_AT,
};

describe('deriveTrailingActivity', () => {
  it('reports the model round-trip after a tool response is the last message', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [toolRequest('t1')]), toolResponseOnlyMessage],
    });

    expect(activity).not.toBeNull();
    expect(activity?.phase).toBe('thinking');
    expect(activity?.label).toBe('Working on the result');
    expect(activity?.since).toBe(LAST_MESSAGE_AT);
  });

  it('returns null for every fixture once the turn is not active', () => {
    // The historical-session guarantee, asserted at the unit level: whatever the
    // transcript looks like, an inactive turn produces no live indicator.
    const fixtures: Message[][] = [
      [message('assistant', [toolRequest('t1')]), toolResponseOnlyMessage],
      [message('assistant', [toolRequest('t1'), toolRequest('t2')])],
      [message('user', [text('hello')])],
      [message('assistant', [])],
    ];

    for (const messages of fixtures) {
      expect(deriveTrailingActivity({ ...live, isTurnActive: false, messages })).toBeNull();
    }
  });

  it('counts outstanding tool calls and suppresses its own clock while they run', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [
        message('assistant', [
          toolRequest('t1'),
          toolRequest('t2'),
          toolRequest('t3'),
          toolResponse('t1'),
        ]),
      ],
    });

    expect(activity?.phase).toBe('running');
    expect(activity?.label).toBe('Running 2 tools');
    // Exactly one clock on screen at a time — the tool cards own it here.
    expect(activity?.since).toBeUndefined();
  });

  it('uses the singular label for a single outstanding tool call', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [toolRequest('t1')])],
    });

    expect(activity?.label).toBe('Running the tool');
  });

  it('returns null while assistant prose is streaming, since the text is the feedback', () => {
    expect(
      deriveTrailingActivity({
        ...live,
        messages: [message('assistant', [text('Here is what I found')])],
      })
    ).toBeNull();
  });

  it('returns null while waiting for user input so the confirmation card is not double-narrated', () => {
    expect(
      deriveTrailingActivity({
        ...live,
        chatState: ChatState.WaitingForUserInput,
        messages: [toolResponseOnlyMessage],
      })
    ).toBeNull();
  });

  it('returns null while a conversation is loading', () => {
    expect(
      deriveTrailingActivity({
        ...live,
        chatState: ChatState.LoadingConversation,
        messages: [toolResponseOnlyMessage],
      })
    ).toBeNull();
  });

  it('returns null when the last message asks for tool confirmation', () => {
    // ⚠ This fixture used to be `{ type: 'toolConfirmationRequest' }`, a variant
    // NOTHING in `crates/` constructs — so the test passed against a predicate
    // that never fired in production, and a real approval card sat under a
    // running "Thinking" clock for its whole TTL. The shape below is what the
    // daemon actually sends.
    const confirmation = message('assistant', [
      {
        type: 'actionRequired',
        data: {
          actionType: 'toolConfirmation',
          id: 'confirm-1',
          toolName: 'developer__shell',
          arguments: {},
        },
      } as unknown as Message['content'][number],
    ]);

    expect(deriveTrailingActivity({ ...live, messages: [confirmation] })).toBeNull();
  });

  it('narrates compaction with its own label', () => {
    const activity = deriveTrailingActivity({
      ...live,
      chatState: ChatState.Compacting,
      messages: [toolResponseOnlyMessage],
    });

    expect(activity?.phase).toBe('compacting');
    expect(activity?.label).toBe('Compacting the chat');
  });

  it('falls back to the turn start when no message event has landed yet', () => {
    const activity = deriveTrailingActivity({
      ...live,
      lastMessageAt: undefined,
      messages: [message('user', [text('do the thing')])],
    });

    expect(activity?.phase).toBe('thinking');
    expect(activity?.since).toBe(TURN_STARTED_AT);
  });

  it('never fabricates a clock origin when both timestamps are missing', () => {
    // The reload-mid-turn case: show the pulse, but no invented number.
    const activity = deriveTrailingActivity({
      ...live,
      turnStartedAt: undefined,
      lastMessageAt: undefined,
      messages: [toolResponseOnlyMessage],
    });

    expect(activity).not.toBeNull();
    expect(activity?.since).toBeUndefined();
  });

  it('returns null for an empty transcript', () => {
    expect(deriveTrailingActivity({ ...live, messages: [] })).toBeNull();
  });

  it('gives each gap in a sequential tool chain a fresh clock origin', () => {
    const first = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [toolRequest('t1')]), toolResponseOnlyMessage],
    });

    const laterMessageAt = LAST_MESSAGE_AT + 4000;
    const second = deriveTrailingActivity({
      ...live,
      lastMessageAt: laterMessageAt,
      messages: [
        message('assistant', [toolRequest('t1')]),
        toolResponseOnlyMessage,
        message('assistant', [toolRequest('t2')]),
        message('user', [toolResponse('t2')]),
      ],
    });

    expect(first?.since).toBe(LAST_MESSAGE_AT);
    expect(second?.since).toBe(laterMessageAt);
  });
});

/**
 * BR-61 — the soft interrupt used to be invisible. `steer()` posts the text and
 * the composer empties, but the agent only injects it at its next loop
 * boundary, which can be a whole tool call away. Between those two moments the
 * transcript said nothing at all, so a working steer was indistinguishable from
 * a dropped one.
 */
describe('pending steer', () => {
  const PENDING = { text: 'actually, use R', since: LAST_MESSAGE_AT + 500 };

  it('announces the steer, carrying the user’s own words', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [text('Let me start by loading the CSV.')])],
      pendingSteer: PENDING,
    });

    expect(activity?.phase).toBe('steering');
    expect(activity?.steerText).toBe('actually, use R');
    expect(activity?.since).toBe(PENDING.since);
  });

  it('outranks streaming prose, which otherwise renders no indicator at all', () => {
    const streaming = {
      ...live,
      messages: [message('assistant', [text('Loading the CSV now…')])],
    };
    // Precondition: this is exactly the case that returns null today.
    expect(deriveTrailingActivity(streaming)).toBeNull();
    expect(deriveTrailingActivity({ ...streaming, pendingSteer: PENDING })?.phase).toBe('steering');
  });

  it('outranks a tool run, so the press is acknowledged mid-tool', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [toolRequest('t1')])],
      pendingSteer: PENDING,
    });

    expect(activity?.phase).toBe('steering');
  });

  it('names the tool a steer is waiting behind', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [toolRequest('t1')])],
      pendingSteer: PENDING,
    });

    expect(activity?.label).toBe('Your message will be added when developer__shell finishes');
  });

  it('uses the tool the daemon said the steer waits behind', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [text('Delegating now.')])],
      pendingSteer: { ...PENDING, waitingOn: { reason: 'tool', toolName: 'workspace__watch' } },
    });

    expect(activity?.phase).toBe('steering');
    expect(activity?.label).toBe('Your message will be added when workspace__watch finishes');
  });

  /**
   * D3 — live, 2026-09-13: "Steering the current turn · 4m 1s" plus "Still
   * working…" sat under an unanswered Run Shell? card. The turn was waiting on
   * the PERSON, and the steer outranked the card that said so.
   */
  it('yields to a turn that is waiting for the user, with no clock', () => {
    const activity = deriveTrailingActivity({
      ...live,
      chatState: ChatState.WaitingForUserInput,
      messages: [message('assistant', [toolRequest('t1')])],
      pendingSteer: PENDING,
    });

    expect(activity?.phase).not.toBe('steering');
    expect(activity?.phase).toBe('steerQueued');
    expect(activity?.since).toBeUndefined();
    expect(activity?.label).toBe('Your message will be added after you answer the card');
  });

  it('yields to an approval card on screen, with no clock', () => {
    const confirmation = message('assistant', [
      {
        type: 'actionRequired',
        data: {
          actionType: 'toolConfirmation',
          id: 'confirm-1',
          toolName: 'developer__shell',
          arguments: {},
        },
      } as unknown as Message['content'][number],
    ]);
    const activity = deriveTrailingActivity({
      ...live,
      messages: [confirmation],
      pendingSteer: PENDING,
    });

    expect(activity?.phase).toBe('steerQueued');
    expect(activity?.since).toBeUndefined();
    expect(activity?.steerText).toBe('actually, use R');
  });

  it('still never shows on a historical replay', () => {
    expect(
      deriveTrailingActivity({
        ...live,
        isTurnActive: false,
        messages: [message('assistant', [toolRequest('t1')])],
        pendingSteer: PENDING,
      })
    ).toBeNull();
  });

  it('falls back to the ordinary phases once the steer is retired', () => {
    const activity = deriveTrailingActivity({
      ...live,
      messages: [message('assistant', [toolRequest('t1')]), toolResponseOnlyMessage],
      pendingSteer: undefined,
    });

    expect(activity?.phase).toBe('thinking');
    expect(activity?.steerText).toBeUndefined();
  });
});
