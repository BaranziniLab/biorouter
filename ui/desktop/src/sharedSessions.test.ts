/**
 * The shared-transcript fetch is the ONE place a `Message[]` enters the
 * renderer without having come from the local daemon's typed client: it is read
 * from an operator-configured remote `base_url` and `safeJsonParse` *asserts*
 * the `SharedSessionDetails` type rather than checking it.
 *
 * Every transcript consumer is compiled against a generated `Message` that
 * declares `content` and `metadata` as required, and reads them without a
 * guard. These specs pin the repair that keeps that promise true: a server on
 * an older schema costs the reader a degraded transcript, never the app's error
 * boundary.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchSharedSessionDetails, normalizeSharedMessages } from './sharedSessions';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('normalizeSharedMessages', () => {
  it('fills in the container fields a consumer dereferences', () => {
    const [message] = normalizeSharedMessages([{ id: 'shapeless', role: 'assistant', created: 7 }]);
    expect(message.content).toEqual([]);
    expect(message.metadata).toEqual({ userVisible: true, agentVisible: true });
    // Scalars are left exactly as they arrived: a missing one degrades a label
    // rather than throwing, so inventing one would disguise a bad payload.
    expect(message.id).toBe('shapeless');
    expect(message.created).toBe(7);
  });

  it('shows a message with no visibility flag and hides one told to hide', () => {
    const [absent, explicit] = normalizeSharedMessages([
      { role: 'assistant', content: [], metadata: {} },
      { role: 'assistant', content: [], metadata: { userVisible: false, agentVisible: false } },
    ]);
    expect(absent.metadata.userVisible).toBe(true);
    expect(explicit.metadata.userVisible).toBe(false);
    expect(explicit.metadata.agentVisible).toBe(false);
  });

  it('preserves the fields the transcript actually renders', () => {
    const [message] = normalizeSharedMessages([
      {
        role: 'assistant',
        created: 1,
        content: [{ type: 'text', text: 'hello' }],
        metadata: {
          userVisible: true,
          agentVisible: true,
          provenance: { kind: 'agent_injection', fromSessionId: 'other' },
        },
      },
    ]);
    expect(message.content).toEqual([{ type: 'text', text: 'hello' }]);
    expect(message.metadata.provenance).toEqual({
      kind: 'agent_injection',
      fromSessionId: 'other',
    });
  });

  it('drops entries that are not messages at all, and a non-array payload', () => {
    expect(normalizeSharedMessages([null, 'nope', 42, ['also-not']])).toEqual([]);
    expect(normalizeSharedMessages(undefined)).toEqual([]);
    expect(normalizeSharedMessages({ messages: [] })).toEqual([]);
  });
});

describe('fetchSharedSessionDetails', () => {
  it('normalizes the messages it hands to the transcript', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({
          share_token: 'token-1',
          created_at: 1,
          base_url: 'https://share.test',
          description: 'Older schema',
          working_dir: '/tmp/project',
          // A server that predates `metadata` — the payload that took the
          // reader to the error boundary.
          messages: [{ role: 'assistant', created: 1, content: [{ type: 'text', text: 'hi' }] }],
          message_count: 1,
          total_tokens: null,
        }),
      })
    );

    const details = await fetchSharedSessionDetails('https://share.test', 'token-1');
    expect(details.messages).toHaveLength(1);
    expect(details.messages[0].metadata).toEqual({ userVisible: true, agentVisible: true });
  });
});
