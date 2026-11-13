import { describe, expect, it } from 'vitest';
import type { Message } from '../api';
import { failedTurnPrompt, opensTurn, promptToSendAgain } from './failedTurn';

function row(
  id: string,
  role: 'user' | 'assistant',
  content: Message['content'],
  metadata: Message['metadata'] = { userVisible: true, agentVisible: true }
): Message {
  return { id, role, created: 1, content, metadata };
}

const text = (value: string): Message['content'] => [{ type: 'text', text: value }];
const toolResult: Message['content'] = [
  {
    type: 'toolResponse',
    id: 'call-1',
    toolResult: { status: 'success', value: { content: [], is_error: false } },
  } as Message['content'][number],
];

describe('failedTurnPrompt', () => {
  it('finds the prompt behind tool results, the notice and a marker', () => {
    const prompt = row('prompt', 'user', text('list the claims'));
    const messages = [
      row('earlier', 'user', text('an earlier turn')),
      row('answer', 'assistant', text('done')),
      prompt,
      row('call', 'assistant', text('reading')),
      row('result', 'user', toolResult),
      row('notice', 'assistant', text('Ran into this error: Server error: 502.')),
      row('hint', 'user', text('model-only note'), { userVisible: false, agentVisible: true }),
      row('marker', 'assistant', text('Monitor checked 3 times, quiet')),
    ];
    expect(opensTurn(messages[4])).toBe(false);
    expect(opensTurn(messages[6])).toBe(false);
    expect(failedTurnPrompt(messages)).toBe(prompt);
  });

  it('sends nothing for words the person did not write', () => {
    const injected = row('injected', 'user', text('from another agent'), {
      userVisible: true,
      agentVisible: true,
      provenance: { kind: 'agent_injection' },
    });
    expect(failedTurnPrompt([injected, row('notice', 'assistant', text('failed'))])).toBeNull();
    expect(failedTurnPrompt([row('notice', 'assistant', text('failed'))])).toBeNull();
  });

  it('sends the same words under a new id', () => {
    const prompt = row('prompt', 'user', text('list the claims'));
    const again = promptToSendAgain(prompt);
    expect(again.id).not.toBe(prompt.id);
    expect(again.role).toBe('user');
    expect(again.content).toEqual(prompt.content);
    expect(again.metadata).toEqual({ userVisible: true, agentVisible: true });
  });
});
