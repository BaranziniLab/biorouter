import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * FR-R4-4: the day-one notice's count starts at boot, and the first-run page waits for its answer.
 *
 * On an upgraded install the first-run page settled in about a second and the notice rose over it
 * a second later (5,542 chats to count), while a person was reading the list or moving to a row.
 * The gate decided only after its own fetch, started when the gate mounted. The count is now one
 * module-level decision, started by `renderer.tsx` as soon as the daemon is reachable, and
 * `ProviderGuard` holds its skeleton for the answer so the page and the notice arrive together.
 *
 * What this file pins: the count runs once however often it is started; the answer is the same
 * show or no-show rule as before; the hold can delay a first draw and nothing else; and the boot
 * really calls it (a source guard, because every behavioural test here would keep passing if
 * `renderer.tsx` stopped, which is the state Task 38 first shipped in, one level down).
 */
const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  listSessions: mocks.listSessions,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: mocks.userActionHeaders,
}));

import {
  acknowledgeFirstRunNotice,
  FIRST_RUN_NOTICE_ACK_KEY,
  FIRST_RUN_NOTICE_WAIT_MS,
  resetFirstRunNoticeDecisionForTests,
  startFirstRunNoticeCount,
  useFirstRunNoticeDecision,
  useFirstRunNoticeHold,
} from './firstRunNoticeDecision';
import type { Session } from '../../api';

function row(id: string, tier: 'private' | 'public', provider: string | null, reason?: string) {
  return {
    id,
    privacy_tier: tier,
    privacy_reason: reason ?? null,
    provider_name: provider,
  } as unknown as Session;
}

const BACKFILLED = [
  row('s1', 'private', 'versa_azure', 'backfill:versa_azure'),
  row('s2', 'public', 'anthropic'),
];
const NOT_DUE = [row('s1', 'private', 'ollama', 'turn:ollama'), row('s2', 'public', 'anthropic')];

/** A count the test settles by hand, so "in flight" is a state it can stand in. */
function deferredCount() {
  let settle!: (sessions: Session[]) => void;
  let fail!: (error: unknown) => void;
  mocks.listSessions.mockReturnValue(
    new Promise((resolve, reject) => {
      settle = (sessions) => resolve({ data: { sessions } });
      fail = reject;
    })
  );
  return { settle, fail };
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  resetFirstRunNoticeDecisionForTests();
  mocks.userActionHeaders.mockResolvedValue({ 'X-User-Action': 'proof' });
  mocks.listSessions.mockResolvedValue({ data: { sessions: BACKFILLED } });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('the count', () => {
  it('is idle until somebody asks, and asks once however often it is started', async () => {
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    expect(result.current.status).toBe('idle');

    await act(async () => {
      startFirstRunNoticeCount();
      startFirstRunNoticeCount();
      startFirstRunNoticeCount();
    });
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe('show');
  });

  it('asks as the person, for the History population', async () => {
    await act(async () => startFirstRunNoticeCount());
    expect(mocks.listSessions).toHaveBeenCalledWith(
      expect.objectContaining({
        headers: { 'X-User-Action': 'proof' },
        query: { include_subagents: false },
      })
    );
  });

  it('is pending while the daemon counts, then shows the counts it read', async () => {
    const count = deferredCount();
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    await act(async () => startFirstRunNoticeCount());
    expect(result.current.status).toBe('pending');

    await act(async () => count.settle(BACKFILLED));
    expect(result.current).toMatchObject({
      status: 'show',
      counts: { privateVisible: 1, totalVisible: 2, backfilledVisible: 1 },
    });
  });

  it('answers none when the migration marked nothing', async () => {
    mocks.listSessions.mockResolvedValue({ data: { sessions: NOT_DUE } });
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    await act(async () => startFirstRunNoticeCount());
    expect(result.current.status).toBe('none');
  });

  it('does no work at all on an acknowledged machine', async () => {
    localStorage.setItem(FIRST_RUN_NOTICE_ACK_KEY, '1');
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    await act(async () => startFirstRunNoticeCount());
    expect(result.current.status).toBe('none');
    expect(mocks.listSessions).not.toHaveBeenCalled();
    expect(mocks.userActionHeaders).not.toHaveBeenCalled();
  });

  it('answers none and records nothing when the daemon does not answer', async () => {
    const count = deferredCount();
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    await act(async () => startFirstRunNoticeCount());
    await act(async () => count.fail(new Error('daemon down')));
    expect(result.current.status).toBe('none');
    // The next launch asks again: the one-time notice is not burned on a launch that could not show it.
    expect(localStorage.getItem(FIRST_RUN_NOTICE_ACK_KEY)).toBeNull();
  });

  it('remembers the acknowledgement and stops showing', async () => {
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    await act(async () => startFirstRunNoticeCount());
    expect(result.current.status).toBe('show');
    act(() => acknowledgeFirstRunNotice());
    expect(result.current.status).toBe('none');
    expect(localStorage.getItem(FIRST_RUN_NOTICE_ACK_KEY)).toBe('1');
  });

  it('drops a count that a new launch outlived', async () => {
    const count = deferredCount();
    const { result } = renderHook(() => useFirstRunNoticeDecision());
    await act(async () => startFirstRunNoticeCount());
    act(() => resetFirstRunNoticeDecisionForTests());
    await act(async () => count.settle(BACKFILLED));
    expect(result.current.status).toBe('idle');
  });
});

describe('the first-run page hold', () => {
  // The boot starts the count before the first-run page can draw, so each case below starts it first.
  it('holds while the count is in flight and lets go the moment it answers', async () => {
    const count = deferredCount();
    await act(async () => startFirstRunNoticeCount());
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    expect(result.current).toBe(true);

    await act(async () => count.settle(BACKFILLED));
    expect(result.current).toBe(false);
  });

  it('lets go on a no-show answer too, so the page never waits on a notice that is not coming', async () => {
    const count = deferredCount();
    await act(async () => startFirstRunNoticeCount());
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    expect(result.current).toBe(true);
    await act(async () => count.settle(NOT_DUE));
    expect(result.current).toBe(false);
  });

  it('lets go when the daemon does not answer', async () => {
    const count = deferredCount();
    await act(async () => startFirstRunNoticeCount());
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    expect(result.current).toBe(true);
    await act(async () => count.fail(new Error('daemon down')));
    expect(result.current).toBe(false);
  });

  it('holds nothing while nobody is counting', () => {
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    expect(result.current).toBe(false);
    expect(mocks.listSessions).not.toHaveBeenCalled();
  });

  it('holds nothing while the page is not drawing its catalog', async () => {
    deferredCount();
    const { result } = renderHook(() => useFirstRunNoticeHold(false));
    await act(async () => startFirstRunNoticeCount());
    expect(result.current).toBe(false);
  });

  it('gives up after its budget, so a slow daemon never keeps the first screen a skeleton', async () => {
    vi.useFakeTimers();
    deferredCount();
    await act(async () => startFirstRunNoticeCount());
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    expect(result.current).toBe(true);

    act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_WAIT_MS - 1));
    expect(result.current).toBe(true);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe(false);
  });

  it('starts its budget when the page starts drawing, not before', async () => {
    vi.useFakeTimers();
    deferredCount();
    await act(async () => startFirstRunNoticeCount());
    // The page is still reading the provider: the budget has not begun.
    const { result, rerender } = renderHook(({ on }) => useFirstRunNoticeHold(on), {
      initialProps: { on: false },
    });
    act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_WAIT_MS * 2));
    rerender({ on: true });
    expect(result.current).toBe(true);
    act(() => vi.advanceTimersByTime(FIRST_RUN_NOTICE_WAIT_MS));
    expect(result.current).toBe(false);
  });

  // The hold can delay the first draw and nothing else. Once the catalog is drawn, a skeleton under
  // the pointer is exactly the thing this finding is about.
  it('never takes a drawn page back to a skeleton', async () => {
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    // Nobody was counting when the page drew, so the hold is spent for good...
    expect(result.current).toBe(false);
    // ...and a count that starts afterwards (the gate's fallback start) does not revive it.
    deferredCount();
    await act(async () => startFirstRunNoticeCount());
    expect(result.current).toBe(false);
  });

  it('is spent once the answer has landed, even if a new launch starts a new count', async () => {
    const first = deferredCount();
    await act(async () => startFirstRunNoticeCount());
    const { result } = renderHook(() => useFirstRunNoticeHold(true));
    expect(result.current).toBe(true);
    await act(async () => first.settle(BACKFILLED));
    expect(result.current).toBe(false);

    act(() => resetFirstRunNoticeDecisionForTests());
    deferredCount();
    await act(async () => startFirstRunNoticeCount());
    expect(result.current).toBe(false);
  });
});

describe('the boot', () => {
  // The one assertion a render cannot make. Every test above would keep passing if `renderer.tsx`
  // stopped starting the count, and the notice would be back to arriving a second after the page.
  it('starts the count once the daemon client is configured, and not for the launcher', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    // `process.cwd()` is `ui/desktop`, where vitest.config lives.
    const renderer = readFileSync(resolve(process.cwd(), 'src/renderer.tsx'), 'utf8');
    const code = renderer.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^[ \t]*\/\/.*$/gm, '');

    // Anti-vacuity: this is the boot function, not a stub left behind.
    expect(code).toContain('client.setConfig(daemonClientConfig(');
    expect(code).toContain("from './components/privacy/firstRunNoticeDecision'");

    const configured = code.indexOf('client.setConfig(daemonClientConfig(');
    const started = code.indexOf('startFirstRunNoticeCount();');
    const rendered = code.indexOf('ReactDOM.createRoot(');
    expect(started).toBeGreaterThan(configured);
    expect(started).toBeLessThan(rendered);

    // Inside the `!isLauncher` branch: the launcher has no daemon to ask.
    const branch = code.slice(code.indexOf('if (!isLauncher) {'), rendered);
    expect(branch).toContain('startFirstRunNoticeCount();');
    expect(code.split('startFirstRunNoticeCount();').length - 1).toBe(1);
  });

  it('keeps the entry chunk free of the notice dialog', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const source = readFileSync(
      resolve(process.cwd(), 'src/components/privacy/firstRunNoticeDecision.ts'),
      'utf8'
    );
    const imports = [...source.matchAll(/^import .* from '([^']+)';$/gm)].map((m) => m[1]);
    // The boot imports this module before the app has loaded. If it imported the notice component
    // (or its shell, dialog or button) all of that would move out of the lazy app chunk.
    expect(imports).toEqual(
      expect.arrayContaining(['react', '../../api', '../../utils/userAction', './noticeCounts'])
    );
    expect(imports.some((from) => /FirstRunPrivacyNotice|ModalShell|ui\//.test(from))).toBe(false);
  });
});
