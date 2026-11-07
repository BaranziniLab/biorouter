import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  CREW_CHANGED_EVENT,
  changeMayConcernChat,
  changeOfKind,
  changeTouches,
  changedConnections,
  crewChangeFeedStateForTests,
  dispatchCrewChanged,
  onCrewChanged,
  setCrewChangeFeedForTests,
  subscribeToCrewChanges,
  type CrewChange,
  type CrewChangesDelta,
} from './useCrewChanges';

/**
 * Crew's change feed in the renderer (WI-15): one poller per window, short polls, a baseline that
 * reports nothing, and one debounced call per burst for each surface.
 */

function change(overrides: Partial<CrewChange> = {}): CrewChange {
  return {
    revision: 1,
    kind: 'route',
    connection_id: null,
    channel_id: null,
    session_id: null,
    run_id: null,
    job_id: null,
    ...overrides,
  };
}

function delta(revision: number, changes: CrewChange[], truncated = false): CrewChangesDelta {
  return { revision, changes, truncated };
}

/** A poll that answers `answers` in turn, then the last one forever; and a sleep that yields. */
function scripted(answers: (CrewChangesDelta | Error | undefined)[]) {
  const asked: (number | undefined)[] = [];
  let next = 0;
  const poll = vi.fn(async (since: number | undefined) => {
    asked.push(since);
    const answer = answers[Math.min(next, answers.length - 1)];
    next += 1;
    if (answer instanceof Error) throw answer;
    return answer;
  });
  const sleeps: number[] = [];
  const sleep = vi.fn(async (ms: number) => {
    sleeps.push(ms);
    await Promise.resolve();
  });
  return { poll, sleep, asked, sleeps };
}

async function settle(rounds = 20) {
  for (let i = 0; i < rounds; i++) await Promise.resolve();
}

afterEach(() => {
  setCrewChangeFeedForTests(null);
  vi.useRealTimers();
});

describe('subscribeToCrewChanges', () => {
  it('learns where the feed stands first, and reports only what changes after', async () => {
    const seen: CrewChangesDelta[] = [];
    const dispatched: CrewChangesDelta[] = [];
    const onEvent = (event: Event) =>
      dispatched.push((event as CustomEvent<CrewChangesDelta>).detail);
    window.addEventListener(CREW_CHANGED_EVENT, onEvent);
    const later = delta(8, [change({ revision: 8, kind: 'channels_create' })]);
    const { poll, sleep, asked } = scripted([
      delta(7, [change({ revision: 7 })]),
      later,
      delta(8, []),
    ]);
    const stop = subscribeToCrewChanges({ poll, sleep, onChange: (d) => seen.push(d) });
    await settle();
    stop();
    window.removeEventListener(CREW_CHANGED_EVENT, onEvent);
    // The baseline is asked with no `since`, and what it lists is not news.
    expect(asked[0]).toBeUndefined();
    expect(asked[1]).toBe(7);
    expect(seen[0]).toEqual(later);
    expect(dispatched[0]).toEqual(later);
    // Caught up: an answer with nothing new is not a change.
    expect(seen).toHaveLength(1);
    expect(asked.slice(2).every((since) => since === 8)).toBe(true);
  });

  it('reports a truncated answer, which is how a restarted daemon is noticed', async () => {
    const seen: CrewChangesDelta[] = [];
    const restarted = delta(0, [], true);
    const { poll, sleep } = scripted([delta(40, []), restarted, delta(0, [])]);
    const stop = subscribeToCrewChanges({ poll, sleep, onChange: (d) => seen.push(d) });
    await settle();
    stop();
    expect(seen).toEqual([restarted]);
  });

  it('asks nothing while the window is hidden', async () => {
    const { poll, sleep } = scripted([delta(1, [])]);
    let shown = false;
    const stop = subscribeToCrewChanges({ poll, sleep, visible: () => shown });
    await settle();
    expect(poll).not.toHaveBeenCalled();
    shown = true;
    await settle();
    stop();
    expect(poll).toHaveBeenCalled();
  });

  it('waits before asking again after a failure', async () => {
    const { poll, sleep, sleeps } = scripted([new Error('daemon restarting'), delta(1, [])]);
    const stop = subscribeToCrewChanges({ poll, sleep, retryDelayMs: 4321, intervalMs: 10 });
    await settle();
    stop();
    expect(sleeps[0]).toBe(4321);
    expect(poll.mock.calls.length).toBeGreaterThan(1);
  });

  it('asks with timeout 0 between short sleeps, never parked', async () => {
    const { poll, sleep, sleeps } = scripted([delta(1, [])]);
    const stop = subscribeToCrewChanges({ poll, sleep, intervalMs: 1500 });
    await settle();
    stop();
    expect(sleeps.length).toBeGreaterThan(0);
    expect(sleeps.every((ms) => ms === 1500)).toBe(true);
  });
});

describe('onCrewChanged', () => {
  it('calls once for a burst, with every change in it', () => {
    vi.useFakeTimers();
    const calls: CrewChangesDelta[] = [];
    const stop = onCrewChanged((d) => calls.push(d));
    dispatchCrewChanged(delta(1, [change({ revision: 1, kind: 'send' })]));
    dispatchCrewChanged(delta(2, [change({ revision: 2, kind: 'grants_revoke' })]));
    vi.advanceTimersByTime(100);
    expect(calls).toHaveLength(0);
    vi.advanceTimersByTime(300);
    expect(calls).toHaveLength(1);
    expect(calls[0].revision).toBe(2);
    expect(calls[0].changes.map((c) => c.kind)).toEqual(['send', 'grants_revoke']);
    stop();
    dispatchCrewChanged(delta(3, [change({ revision: 3 })]));
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
  });

  it('ignores an event that is not a delta', () => {
    vi.useFakeTimers();
    const listener = vi.fn();
    const stop = onCrewChanged(listener);
    window.dispatchEvent(new CustomEvent(CREW_CHANGED_EVENT, { detail: { nope: true } }));
    vi.advanceTimersByTime(1000);
    stop();
    expect(listener).not.toHaveBeenCalled();
  });

  it('runs one poller for the window however many surfaces listen, and stops with the last', async () => {
    const { poll, sleep } = scripted([delta(1, [])]);
    setCrewChangeFeedForTests({ poll, sleep });
    const first = onCrewChanged(() => undefined);
    const second = onCrewChanged(() => undefined);
    expect(crewChangeFeedStateForTests()).toEqual({ holders: 2, running: true });
    first();
    expect(crewChangeFeedStateForTests()).toEqual({ holders: 1, running: true });
    // Releasing twice changes nothing.
    first();
    expect(crewChangeFeedStateForTests().holders).toBe(1);
    second();
    expect(crewChangeFeedStateForTests()).toEqual({ holders: 0, running: false });
    await settle();
  });

  it('starts no poller in a test that did not ask for one', () => {
    const stop = onCrewChanged(() => undefined);
    expect(crewChangeFeedStateForTests()).toEqual({ holders: 1, running: false });
    stop();
  });
});

describe('reading a delta', () => {
  it('says which connections a change may concern', () => {
    const on = (id: string | null) => delta(1, [change({ connection_id: id })]);
    expect(changeTouches(on('conn-1'), ['conn-1'])).toBe(true);
    expect(changeTouches(on('conn-9'), ['conn-1'])).toBe(false);
    // A change that names no connection may be any.
    expect(changeTouches(on(null), ['conn-1'])).toBe(true);
    expect(changeTouches(delta(1, [], true), ['conn-1'])).toBe(true);
  });

  it('says which chats a change may concern', () => {
    const naming = (id: string | null) => delta(1, [change({ session_id: id })]);
    expect(changeMayConcernChat(naming('chat-1'), 'chat-1')).toBe(true);
    expect(changeMayConcernChat(naming('chat-2'), 'chat-1')).toBe(false);
    // A change that names no chat (a connection connected or removed) may concern any.
    expect(changeMayConcernChat(naming(null), 'chat-1')).toBe(true);
    expect(changeMayConcernChat(delta(1, [], true), 'chat-1')).toBe(true);
  });

  it('finds a kind, and lists the connections named, newest last', () => {
    const d = delta(3, [
      change({ kind: 'send', connection_id: 'a' }),
      change({ kind: 'defaults_set' }),
      change({ kind: 'setup_finished', connection_id: 'b' }),
      change({ kind: 'send', connection_id: 'a' }),
    ]);
    expect(changeOfKind(d, ['defaults_set', 'defaults_clear'])).toBe(true);
    expect(changeOfKind(d, ['rejoin'])).toBe(false);
    expect(changeOfKind(delta(1, [], true), ['rejoin'])).toBe(true);
    expect(changedConnections(d)).toEqual(['b', 'a']);
  });
});
