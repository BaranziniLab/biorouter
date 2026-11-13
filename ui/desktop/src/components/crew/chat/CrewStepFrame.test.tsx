import { beforeEach, describe, expect, it, vi } from 'vitest';
import { dismissStep } from './CrewStepFrame';

const mocks = vi.hoisted(() => ({
  dismissCrewStep: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  dismissCrewStep: mocks.dismissCrewStep,
}));
vi.mock('../../../utils/userAction', () => ({ userActionHeaders: mocks.userActionHeaders }));

beforeEach(() => {
  mocks.dismissCrewStep.mockReset().mockResolvedValue({ data: { status: 'dismissed' } });
  mocks.userActionHeaders.mockReset().mockResolvedValue({ 'X-User-Action': 'proof-of-user' });
});

describe('dismissing a Crew step card', () => {
  /**
   * The route names a chat, so the daemon asks whether this caller reaches it: without the
   * person's proof, a private chat's card is refused and stays waiting.
   */
  it('carries the person’s proof, as every call that names a chat does', async () => {
    expect(await dismissStep('chat-1', 'card-1')).toBe(true);
    expect(mocks.userActionHeaders).toHaveBeenCalledTimes(1);
    expect(mocks.dismissCrewStep).toHaveBeenCalledWith({
      headers: { 'X-User-Action': 'proof-of-user' },
      body: { id: 'card-1', session_id: 'chat-1', outcome: 'dismiss' },
    });
  });

  it('sends what a browser surface states instead, and nothing else', async () => {
    mocks.userActionHeaders.mockResolvedValueOnce({ 'X-Caller-Provider': 'synthetic-provider' });
    await dismissStep('chat-2', 'card-2');
    expect(mocks.dismissCrewStep).toHaveBeenCalledWith({
      headers: { 'X-Caller-Provider': 'synthetic-provider' },
      body: { id: 'card-2', session_id: 'chat-2', outcome: 'dismiss' },
    });
  });

  it('answers false when the daemon refused or no longer had the card', async () => {
    mocks.dismissCrewStep.mockResolvedValueOnce({ error: { status: 'unknown' } });
    expect(await dismissStep('chat-1', 'gone')).toBe(false);
    mocks.dismissCrewStep.mockResolvedValueOnce({ error: 'That chat is private.' });
    expect(await dismissStep('chat-1', 'private')).toBe(false);
  });
});
