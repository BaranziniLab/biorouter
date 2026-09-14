import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * The row-change feed's own poll carries the person's proof.
 *
 * Issue #56: `GET /sessions/changes` now reports a change only for a chat its
 * caller could open, and answers a request without `X-User-Action` as a public
 * model's. The poll is the desktop's only ear on a row another PROCESS rewrote,
 * so a poll without the proof would silently drop exactly the changes that
 * matter most — a private chat's model switch or tier raise — and the composer
 * would keep stating the binding it replaced. Nothing would error.
 *
 * A separate file from `sessionMetaSubscription.test.ts` because that suite
 * injects its own `poll`, and mocking the generated client there would change
 * what every one of its tests is measuring.
 */

const mocks = vi.hoisted(() => ({
  sessionChanges: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../api', () => ({ sessionChanges: mocks.sessionChanges }));
vi.mock('./userAction', () => ({ userActionHeaders: mocks.userActionHeaders }));

const { subscribeToSessionMeta } = await import('./sessionMetaSubscription');

beforeEach(() => {
  vi.clearAllMocks();
});

describe("the row-change poll carries the person's proof", () => {
  it('sends userActionHeaders() on the generated client call', async () => {
    mocks.userActionHeaders.mockResolvedValue({ 'X-User-Action': 'renderer-proof' });
    let answered = false;
    mocks.sessionChanges.mockImplementation(async () => {
      if (answered) {
        // Park, so the loop does not spin past the one call under test.
        await new Promise(() => {});
      }
      answered = true;
      return { data: { revision: 1, changes: [], truncated: false } };
    });

    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['20260914_1'],
      onSessionChanged: () => {},
      sleep: () => Promise.resolve(),
    });
    await vi.waitFor(() => expect(mocks.sessionChanges).toHaveBeenCalled());
    stop();

    expect(mocks.sessionChanges.mock.calls[0][0]).toEqual(
      expect.objectContaining({
        query: { since: 0, ids: '20260914_1' },
        headers: { 'X-User-Action': 'renderer-proof' },
      })
    );
  });
});
