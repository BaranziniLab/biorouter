import { act, renderHook } from '@testing-library/react';
import { StrictMode, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelUsageRow, Session } from '../api';
import {
  LATE_USAGE_READS_MS,
  RETURN_READ_GAP_MS,
  ledgerTokenCount,
  useCostTracking,
} from './useCostTracking';

/**
 * 2026-11-13, a Claude Haiku 5.5 chat on Versa Bedrock: after a long-context turn the daemon
 * billed two eager-compaction side calls (about $0.19) AFTER the turn's last frame, and the
 * compaction failed, so nothing else changed. No counter the hook watches moved and the chat's row
 * was not stamped, so the footer's cost stayed at the turn's $0.127 and the summary's Tokens at
 * 255k until a reload, while `GET /sessions/{id}/usage` was right throughout. The hook now reads
 * the ledger again on a short back-off after a turn ends, and once when the window comes back.
 */
const mocks = vi.hoisted(() => ({
  getSessionUsage: vi.fn(),
  pricing: vi.fn(),
  headers: vi.fn(),
}));
vi.mock('../api', () => ({ getSessionUsage: mocks.getSessionUsage }));
vi.mock('../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

/** The turn's own requests: what the ledger holds as the turn ends. */
const turnRow: ModelUsageRow = {
  provider: 'versa_bedrock',
  modelId: 'claude-haiku-5-5',
  inputTokens: 250_000,
  outputTokens: 5_000,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  totalTokens: 255_000,
  turns: 1,
  cost: 0.127,
};
/** The same row once the two compaction side calls are billed. */
const withSideCalls: ModelUsageRow = {
  ...turnRow,
  inputTokens: 250_000 + 250_000,
  outputTokens: 5_000 + 2_000,
  totalTokens: 507_000,
  turns: 3,
  cost: 0.127 + 0.19,
};

/** What the daemon's ledger holds right now. */
let ledgerNow: ModelUsageRow[] = [];

const chat = (id: string) => ({ id, accumulated_total_tokens: 0 }) as unknown as Session;

const reads = () => mocks.getSessionUsage.mock.calls.length;

/** Lets every pending promise chain (the proof, the request, the pricing) run to its end. */
async function settle() {
  await act(async () => {
    for (let hop = 0; hop < 20; hop += 1) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
  await settle();
}

/** The hook as BaseChat drives it: the chat's row, and whether a turn runs in it. */
function renderChat(
  initial: { session: Session; turnRunning: boolean },
  wrapper?: ({ children }: { children: ReactNode }) => ReactNode
) {
  return renderHook(
    ({ session, turnRunning }: { session: Session; turnRunning: boolean }) =>
      useCostTracking({ session, turnRunning }),
    { initialProps: initial, wrapper }
  );
}

/** Open a chat with a turn running, then let the turn end with its own usage in the ledger. */
async function turnThatEnds(id = 'a') {
  ledgerNow = [turnRow];
  const view = renderChat({ session: chat(id), turnRunning: true });
  await settle();
  view.rerender({ session: chat(id), turnRunning: false });
  await settle();
  expect(view.result.current.modelRows[0]?.totalCost).toBeCloseTo(0.127);
  return view;
}

beforeEach(() => {
  vi.useFakeTimers();
  mocks.getSessionUsage.mockReset();
  mocks.getSessionUsage.mockImplementation(async () => ({ data: { models: ledgerNow } }));
  mocks.pricing.mockReset().mockResolvedValue(null);
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
  ledgerNow = [];
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useCostTracking: side calls billed after the turn reach the figures', () => {
  it('picks up a side call billed 30 s after the turn ends, with no reload and nothing moved', async () => {
    const { result } = await turnThatEnds();
    const before = reads();

    // The compaction is still running at the first re-read.
    await advance(10_000);
    expect(reads()).toBe(before + 1);
    expect(result.current.modelRows[0]?.totalCost).toBeCloseTo(0.127);

    ledgerNow = [withSideCalls];
    await advance(20_000);
    expect(reads()).toBe(before + 2);
    expect(result.current.modelRows[0]?.totalCost).toBeCloseTo(0.317);
    expect(result.current.modelRows[0]?.inputTokens).toBe(500_000);
    expect(result.current.sessionCosts['versa_bedrock/claude-haiku-5-5']?.totalCost).toBeCloseTo(
      0.317
    );

    // Every read carries the person's proof: a private chat's usage is refused without it.
    for (const [options] of mocks.getSessionUsage.mock.calls) {
      expect(options).toMatchObject({
        path: { session_id: 'a' },
        headers: { 'X-User-Action': 'synthetic-proof' },
      });
    }
  });

  it('reads a handful of times after a turn, then stops', async () => {
    await turnThatEnds();
    const before = reads();
    expect(LATE_USAGE_READS_MS.length).toBeLessThanOrEqual(6);

    // One read at each step of the back-off, measured from the turn's end.
    let elapsed = 0;
    for (const [step, delay] of LATE_USAGE_READS_MS.entries()) {
      await advance(delay - 1 - elapsed);
      expect(reads()).toBe(before + step);
      await advance(1);
      expect(reads()).toBe(before + step + 1);
      elapsed = delay;
    }

    // The schedule has ended: an hour of an open, idle chat asks for nothing more.
    await advance(60 * 60_000);
    expect(reads()).toBe(before + LATE_USAGE_READS_MS.length);
  });

  it('reads nothing on a timer while a turn runs', async () => {
    ledgerNow = [turnRow];
    renderChat({ session: chat('a'), turnRunning: true });
    await settle();
    expect(reads()).toBe(1);

    await advance(60 * 60_000);
    expect(reads()).toBe(1);

    // A return to the window mid-turn asks for nothing either: the turn's counters do.
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    await settle();
    expect(reads()).toBe(1);
  });

  it('reads nothing on a timer in a chat opened idle, where no turn has ended', async () => {
    ledgerNow = [turnRow];
    renderChat({ session: chat('a'), turnRunning: false });
    await settle();
    await advance(60 * 60_000);
    expect(reads()).toBe(1);
  });

  it('cancels what is left of the schedule when a new turn starts', async () => {
    const { rerender } = await turnThatEnds();
    const before = reads();
    await advance(10_000);
    expect(reads()).toBe(before + 1);

    rerender({ session: chat('a'), turnRunning: true });
    await settle();
    await advance(60 * 60_000);
    expect(reads()).toBe(before + 1);

    // That turn's end starts a schedule of its own, from its own end.
    rerender({ session: chat('a'), turnRunning: false });
    await settle();
    await advance(LATE_USAGE_READS_MS[0] - 1);
    expect(reads()).toBe(before + 1);
    await advance(1);
    expect(reads()).toBe(before + 2);
  });

  it('cancels the schedule on an unmount and on a switch to another chat', async () => {
    const first = await turnThatEnds('a');
    const before = reads();
    first.unmount();
    await advance(60 * 60_000);
    expect(reads()).toBe(before);

    const second = await turnThatEnds('b');
    const afterOpen = reads();
    second.rerender({ session: chat('c'), turnRunning: false });
    await settle();
    // The switch reads chat c once, as any open does, and nothing on b's schedule.
    expect(reads()).toBe(afterOpen + 1);
    await advance(60 * 60_000);
    expect(reads()).toBe(afterOpen + 1);
    for (const [options] of mocks.getSessionUsage.mock.calls.slice(afterOpen)) {
      expect(options).toMatchObject({ path: { session_id: 'c' } });
    }
  });

  it('keeps its schedule under StrictMode’s double run of effects', async () => {
    ledgerNow = [turnRow];
    const strict = ({ children }: { children: ReactNode }) => <StrictMode>{children}</StrictMode>;
    const { result, rerender } = renderChat({ session: chat('a'), turnRunning: true }, strict);
    await settle();
    rerender({ session: chat('a'), turnRunning: false });
    await settle();
    const before = reads();

    ledgerNow = [withSideCalls];
    await advance(LATE_USAGE_READS_MS[0]);
    expect(reads()).toBe(before + 1);
    expect(result.current.modelRows[0]?.totalCost).toBeCloseTo(0.317);
  });

  it('never lowers the figures on a read that counts fewer tokens than it holds', async () => {
    const { result } = await turnThatEnds();
    ledgerNow = [withSideCalls];
    await advance(LATE_USAGE_READS_MS[0]);
    expect(result.current.modelRows[0]?.totalCost).toBeCloseTo(0.317);

    // A stale answer, from before the side calls were written.
    ledgerNow = [turnRow];
    await advance(LATE_USAGE_READS_MS[1] - LATE_USAGE_READS_MS[0]);
    expect(result.current.modelRows[0]?.totalCost).toBeCloseTo(0.317);
    expect(ledgerTokenCount(result.current.modelRows)).toBe(507_000);
  });
});

describe('useCostTracking: a return to the window reads the ledger once', () => {
  it('reads once on focus and visibility together, again after the gap, with the proof', async () => {
    ledgerNow = [turnRow];
    const { result } = renderChat({ session: chat('a'), turnRunning: false });
    await settle();
    expect(reads()).toBe(1);

    ledgerNow = [withSideCalls];
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
      window.dispatchEvent(new Event('focus'));
    });
    await settle();
    expect(reads()).toBe(2);
    expect(result.current.modelRows[0]?.totalCost).toBeCloseTo(0.317);
    expect(mocks.getSessionUsage.mock.calls[1][0]).toMatchObject({
      path: { session_id: 'a' },
      headers: { 'X-User-Action': 'synthetic-proof' },
    });

    await advance(RETURN_READ_GAP_MS);
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    await settle();
    expect(reads()).toBe(3);
  });

  it('reads nothing when the window is hidden, nor after the chat is closed', async () => {
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    ledgerNow = [turnRow];
    const { unmount } = renderChat({ session: chat('a'), turnRunning: false });
    await settle();
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await settle();
    expect(reads()).toBe(1);

    visibility.mockRestore();
    unmount();
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    await settle();
    expect(reads()).toBe(1);
  });
});
