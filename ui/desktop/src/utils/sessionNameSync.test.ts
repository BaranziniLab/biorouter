import { describe, it, expect, vi, beforeEach } from 'vitest';

// Mock the API surface BEFORE importing the module under test so the
// `updateSessionName` reference is captured.
vi.mock('../api', () => ({
  updateSessionName: vi.fn(async () => ({ data: {} })),
}));

// The proof the desktop sends. Since issue #56's QA sweep (2026-09-10) the
// daemon answers a request without it as a public model — private chats and
// knowledge bases omitted or refused — so each call here must carry it.
vi.mock('./userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

import { updateSessionName } from '../api';
import {
  announceSessionName,
  cacheGet,
  cacheSet,
  cacheUpdateName,
  DEFAULT_SESSION_NAME,
  isDefaultSessionName,
  renameSession,
  subscribeSessionNameChanges,
} from './sessionNameSync';

const makeSession = (overrides: Record<string, unknown> = {}) =>
  ({
    id: 's1',
    name: DEFAULT_SESSION_NAME,
    user_set_name: false,
    ...overrides,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  }) as any;

describe('isDefaultSessionName', () => {
  it('flags the current "New chat" placeholder', () => {
    expect(isDefaultSessionName('New chat')).toBe(true);
  });

  it('keeps recognizing the legacy "New Session" placeholder', () => {
    expect(isDefaultSessionName('New Session')).toBe(true);
    expect(isDefaultSessionName('CLI Session')).toBe(true);
  });
  it('flags legacy numbered placeholders so old data still reads as default', () => {
    expect(isDefaultSessionName('New session 154')).toBe(true);
    expect(isDefaultSessionName('Session 5')).toBe(true);
  });
  it('flags empty/null/undefined as default', () => {
    expect(isDefaultSessionName('')).toBe(true);
    expect(isDefaultSessionName(null)).toBe(true);
    expect(isDefaultSessionName(undefined)).toBe(true);
  });
  it('does not flag user-authored or LLM names', () => {
    expect(isDefaultSessionName('Q1 Planning')).toBe(false);
    expect(isDefaultSessionName('Discussion about React performance')).toBe(false);
    // Boundary: 'Session' alone (no number) is a real name.
    expect(isDefaultSessionName('Session')).toBe(false);
    // 'A new session' is a real label — must not match the default regex.
    expect(isDefaultSessionName('A new session today')).toBe(false);
  });

  it('match is case-insensitive (defensive against backend variants)', () => {
    expect(isDefaultSessionName('NEW SESSION')).toBe(true);
    expect(isDefaultSessionName('new session')).toBe(true);
  });
});

describe('cache', () => {
  beforeEach(() => {
    // Hack: clear by overwriting every known key. The cache itself is a
    // module singleton, so we touch the keys we set in tests.
    for (const k of ['s1', 's2', 's3'])
      cacheSet(k, { messages: [], session: makeSession({ id: k }) });
    for (const k of ['s1', 's2', 's3']) cacheUpdateName(k, 'reset', false);
  });

  it('cacheGet returns undefined for an unknown key', () => {
    expect(cacheGet('unknown-key-x')).toBeUndefined();
  });

  it('cacheUpdateName patches the session record in place', () => {
    cacheSet('s1', { messages: [], session: makeSession({ name: 'old' }) });
    cacheUpdateName('s1', 'new', true);
    const entry = cacheGet('s1');
    expect(entry?.session.name).toBe('new');
    expect(entry?.session.user_set_name).toBe(true);
  });

  it('cacheUpdateName is a no-op when the session is not cached', () => {
    expect(() => cacheUpdateName('not-in-cache', 'x', true)).not.toThrow();
    expect(cacheGet('not-in-cache')).toBeUndefined();
  });
});

describe('subscribeSessionNameChanges + announceSessionName', () => {
  it('fans out to all subscribers', () => {
    const a = vi.fn();
    const b = vi.fn();
    const unsubA = subscribeSessionNameChanges(a);
    const unsubB = subscribeSessionNameChanges(b);
    announceSessionName({ sessionId: 's1', name: 'A', userSetName: true, origin: 'user' });
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    unsubA();
    unsubB();
  });

  it('updates the cache as a side effect of the announce', () => {
    cacheSet('s1', { messages: [], session: makeSession({ name: 'before' }) });
    announceSessionName({ sessionId: 's1', name: 'after', userSetName: true, origin: 'user' });
    expect(cacheGet('s1')?.session.name).toBe('after');
    expect(cacheGet('s1')?.session.user_set_name).toBe(true);
  });

  it('unsubscribed listeners stop firing', () => {
    const a = vi.fn();
    const unsub = subscribeSessionNameChanges(a);
    unsub();
    announceSessionName({ sessionId: 's1', name: 'A', userSetName: true, origin: 'user' });
    expect(a).not.toHaveBeenCalled();
  });
});

describe('renameSession', () => {
  beforeEach(() => {
    vi.mocked(updateSessionName).mockClear();
    vi.mocked(updateSessionName).mockResolvedValue({ data: {} } as never);
  });

  it('calls updateSessionName with the trimmed name', async () => {
    await renameSession('s1', '  Q1 Plans  ');
    expect(updateSessionName).toHaveBeenCalledWith({
      path: { session_id: 's1' },
      body: { name: 'Q1 Plans' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });

  it('rejects empty names without hitting the network', async () => {
    await expect(renameSession('s1', '   ')).rejects.toThrow();
    expect(updateSessionName).not.toHaveBeenCalled();
  });

  it("broadcasts userSetName=true for 'user' origin", async () => {
    const listener = vi.fn();
    const unsub = subscribeSessionNameChanges(listener);
    await renameSession('s1', 'Mine', 'user');
    expect(listener).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'Mine', userSetName: true, origin: 'user' })
    );
    unsub();
  });

  it("broadcasts userSetName=false for 'llm' origin (disambiguation rewrites)", async () => {
    const listener = vi.fn();
    const unsub = subscribeSessionNameChanges(listener);
    await renameSession('s1', 'Discussion about React', 'llm');
    expect(listener).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'Discussion about React',
        userSetName: false,
        origin: 'llm',
      })
    );
    unsub();
  });

  it('rethrows when the backend call fails', async () => {
    vi.mocked(updateSessionName).mockRejectedValueOnce(new Error('http 500'));
    await expect(renameSession('s1', 'X', 'user')).rejects.toThrow('http 500');
  });
});
