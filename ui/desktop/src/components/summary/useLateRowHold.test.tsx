import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ChatTurnErrorData } from '../../types/turnError';
import { LATE_ROW_GRACE_MS, lateRowHolds, useLateRowHold } from './useLateRowHold';

/**
 * Which turn error rows are held below the fold instead of followed (vision
 * V2-R9-2). The geometry, on the real scroll area in Chromium, is in
 * `lateRowHold.browser.test.tsx`; this is the decision, commit by commit.
 */
const restoreFailure: ChatTurnErrorData = {
  message: 'Could not create provider: Configuration value not found: AZURE_OPENAI_ENDPOINT',
  code: 'provider_restore_failed',
  scope: 'session',
  retryable: false,
};
const turnFailure: ChatTurnErrorData = {
  message: 'Rate limit exceeded',
  code: 'provider_failure',
  scope: 'provider',
  retryable: true,
};

describe('lateRowHolds', () => {
  const base = { openOutcome: true, scope: 'session' as const, shownAt: 1000 };

  it('holds the open’s own restore failure once the transcript has been on screen', () => {
    expect(lateRowHolds({ ...base, now: 1000 + LATE_ROW_GRACE_MS })).toBe(true);
    expect(lateRowHolds({ ...base, now: 7500 })).toBe(true);
  });

  it('follows it while the open is still settling', () => {
    expect(lateRowHolds({ ...base, now: 1000 + LATE_ROW_GRACE_MS - 1 })).toBe(false);
    expect(lateRowHolds({ ...base, now: 1030 })).toBe(false);
  });

  it('follows a row drawn in the transcript’s first frame', () => {
    expect(lateRowHolds({ ...base, shownAt: null, now: 9000 })).toBe(false);
  });

  it('follows a turn’s failure and any other scope', () => {
    expect(lateRowHolds({ ...base, openOutcome: false, now: 9000 })).toBe(false);
    for (const scope of ['provider', 'transport', 'inference', 'internal'] as const) {
      expect(lateRowHolds({ ...base, scope, now: 9000 })).toBe(false);
    }
  });
});

interface Props {
  sessionId: string;
  transcriptShown: boolean;
  row: ChatTurnErrorData | null;
  turnRunning: boolean;
}

describe('useLateRowHold', () => {
  let clock = 0;
  beforeEach(() => {
    clock = 0;
    vi.spyOn(performance, 'now').mockImplementation(() => clock);
  });
  afterEach(() => vi.restoreAllMocks());

  const mount = (initial: Partial<Props> = {}) => {
    const follow = { holdForLateRow: vi.fn(), releaseHold: vi.fn() };
    const initialProps: Props = {
      sessionId: 's1',
      transcriptShown: false,
      row: null,
      turnRunning: false,
      ...initial,
    };
    const view = renderHook((props: Props) => useLateRowHold({ ...props, follow }), {
      initialProps,
    });
    let props = initialProps;
    const commit = (next: Partial<Props>, at?: number) => {
      if (at !== undefined) clock = at;
      props = { ...props, ...next };
      view.rerender(props);
    };
    return { follow, commit };
  };

  it('holds the restore failure that lands 6.5 s after a long chat is drawn (the measured case)', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 6000);
    // The agent's load, its own `agentReady` and any re-render in between.
    commit({}, 9000);
    commit({ row: restoreFailure }, 12_500);
    expect(follow.holdForLateRow).toHaveBeenCalledTimes(1);
  });

  it('follows a restore failure that lands a few frames after the transcript', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 6000);
    commit({ row: restoreFailure }, 6030);
    expect(follow.holdForLateRow).not.toHaveBeenCalled();
    expect(follow.releaseHold).toHaveBeenCalled();
  });

  it('follows a restore failure drawn with the transcript', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true, row: restoreFailure }, 6000);
    commit({}, 20_000);
    expect(follow.holdForLateRow).not.toHaveBeenCalled();
  });

  it('follows a turn’s failure, whatever its scope, once a turn has run', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 1000);
    commit({ turnRunning: true }, 5000);
    // The failure and the end of the turn land in one commit (finishCurrentStream).
    commit({ turnRunning: false, row: { ...restoreFailure, code: 'session_failure' } }, 9000);
    expect(follow.holdForLateRow).not.toHaveBeenCalled();
  });

  it('follows a late row of any other scope', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 1000);
    commit({ row: turnFailure }, 9000);
    expect(follow.holdForLateRow).not.toHaveBeenCalled();
  });

  it('holds only the first row: a Retry’s answer is the person’s', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 1000);
    commit({ row: restoreFailure }, 9000);
    expect(follow.holdForLateRow).toHaveBeenCalledTimes(1);
    commit({ row: null }, 20_000);
    commit({ row: { ...restoreFailure } }, 20_400);
    expect(follow.holdForLateRow).toHaveBeenCalledTimes(1);
  });

  it('releases the hold when the row goes away', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 1000);
    commit({ row: restoreFailure }, 9000);
    follow.releaseHold.mockClear();
    commit({ row: null }, 9500);
    expect(follow.releaseHold).toHaveBeenCalledTimes(1);
  });

  it('decides once per row, not on every commit that keeps it', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 1000);
    commit({ row: restoreFailure }, 9000);
    commit({}, 9100);
    commit({ turnRunning: false }, 9200);
    expect(follow.holdForLateRow).toHaveBeenCalledTimes(1);
    expect(follow.releaseHold).not.toHaveBeenCalled();
  });

  it('starts over for another chat', () => {
    const { follow, commit } = mount();
    commit({ transcriptShown: true }, 1000);
    commit({ turnRunning: true }, 2000);
    commit({ turnRunning: false }, 3000);
    // Another chat in the same BaseChat: its own open, its own first frame.
    commit({ sessionId: 's2', transcriptShown: false }, 4000);
    expect(follow.releaseHold).toHaveBeenCalledTimes(1);
    commit({ transcriptShown: true }, 5000);
    commit({ row: restoreFailure }, 9000);
    expect(follow.holdForLateRow).toHaveBeenCalledTimes(1);
  });
});
