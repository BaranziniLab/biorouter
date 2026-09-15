import { describe, expect, it } from 'vitest';
import type { Message } from '../api';
import { withSteerRecoveries } from './steerRecovery';
const message = (id: string): Message => ({
  id,
  role: 'user',
  created: 1,
  content: [{ type: 'text', text: id }],
  metadata: { userVisible: true, agentVisible: true },
});
describe('steer recovery display', () => {
  it('keeps the recovery at its original place without changing canonical history', () => {
    const original = [message('before'), message('later')];
    const recovery = { message: message('uncertain'), afterMessageId: 'before', index: 1 };
    expect(withSteerRecoveries(original, [recovery]).map((m) => m.id)).toEqual([
      'before',
      'uncertain',
      'later',
    ]);
    expect(original.map((m) => m.id)).toEqual(['before', 'later']);
    expect(
      withSteerRecoveries([original[0], recovery.message, original[1]], [recovery])
    ).toHaveLength(3);
  });
});
