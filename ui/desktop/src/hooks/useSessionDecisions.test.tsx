import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../api';
import type { DecisionSettings, SessionDecisionSummary } from '../utils/sessionDecisions';
import { DECISION_POLL_MS, useSessionDecisions } from './useSessionDecisions';

const mocks = vi.hoisted(() => ({ summary: vi.fn(), settings: vi.fn() }));
vi.mock('../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../utils/sessionDecisions')>()),
  fetchDecisionSummary: mocks.summary,
  fetchDecisionSettings: mocks.settings,
}));

const summary = (total: number): SessionDecisionSummary => ({
  total,
  aggregates:
    total > 0
      ? [
          {
            origin: 'tool',
            originId: 'c1',
            label: 'decide',
            count: total,
            passed: 0,
            failed: 0,
            errors: 0,
            lastAt: '2026-11-04T10:00:00Z',
            lastAnswer: 'yes',
            lastPassProbability: 0.9,
            model: null,
          },
        ]
      : [],
  recentTurns: [],
});
const noMonitors: DecisionSettings = { defaultModel: null, monitors: [] };
const withMonitor: DecisionSettings = {
  defaultModel: null,
  monitors: [
    {
      id: 'form',
      name: 'Form reached',
      question: { instructions: 'q', kind: { type: 'binary' }, threshold: null },
      model: null,
      timing: 'post_iteration',
      window: { entries: 24, maxChars: 6000, toolCalls: true },
      threshold: 0.85,
      option: null,
      consecutive: 1,
      rearmBelow: null,
      action: 'stop',
      onError: 'notify',
      maxEvalsPerTurn: 50,
      enabled: true,
    },
  ],
};

function decided(id: string, name = 'decisions__decide'): Message[] {
  return [
    {
      role: 'assistant',
      created: 0,
      metadata: { agentVisible: true, userVisible: true },
      content: [
        { type: 'toolRequest', id, toolCall: { status: 'success', value: { name } } },
        { type: 'toolResponse', id, toolResult: { status: 'success', value: { content: [] } } },
      ],
    },
  ] as Message[];
}

async function advance(ms: number) {
  for (let elapsed = 0; elapsed < ms; elapsed += 50) {
    await act(() => vi.advanceTimersByTimeAsync(Math.min(50, ms - elapsed)));
  }
}

beforeEach(() => {
  mocks.summary.mockReset().mockResolvedValue({ data: summary(0), missing: false });
  mocks.settings.mockReset().mockResolvedValue({ data: noMonitors, missing: false });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useSessionDecisions', () => {
  it('reads nothing while the summary is hidden and no turn runs', async () => {
    renderHook(() => useSessionDecisions('a', [], { visible: false, turnRunning: false }));
    await act(() => new Promise((resolve) => setTimeout(resolve, 120)));
    expect(mocks.summary).not.toHaveBeenCalled();
    expect(mocks.settings).not.toHaveBeenCalled();
  });

  it('takes a saved value as the chat’s settings even while it reads nothing', async () => {
    const { result } = renderHook(() =>
      useSessionDecisions('a', [], { visible: false, turnRunning: false })
    );
    act(() => result.current.applySettings(withMonitor));
    expect(result.current.data?.settings).toEqual(withMonitor);
    expect(mocks.settings).not.toHaveBeenCalled();
  });

  it('reads the summary and the settings once the summary is shown', async () => {
    mocks.summary.mockResolvedValue({ data: summary(5000), missing: false });
    const { result } = renderHook(() =>
      useSessionDecisions('a', [], { visible: true, turnRunning: false })
    );
    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.data?.summary?.total).toBe(5000));
    expect(mocks.summary).toHaveBeenCalledWith('a', expect.any(AbortSignal));
    expect(mocks.settings).toHaveBeenCalledWith('a', expect.any(AbortSignal));
    expect(result.current.loading).toBe(false);
  });

  it('re-reads when a decide call answers, and not for prose', async () => {
    const { result, rerender } = renderHook(
      ({ messages }) => useSessionDecisions('a', messages, { visible: true, turnRunning: false }),
      { initialProps: { messages: [] as Message[] } }
    );
    await waitFor(() => expect(result.current.data).not.toBeNull());
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    rerender({
      messages: [
        { role: 'assistant', created: 0, content: [{ type: 'text', text: 'Hi' }] },
      ] as unknown as Message[],
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 120)));
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    rerender({ messages: decided('one') });
    await waitFor(() => expect(mocks.summary).toHaveBeenCalledTimes(2));
    rerender({
      messages: [...decided('one'), ...decided('two', 'workspace__workspace_orchestrate')],
    });
    await waitFor(() => expect(mocks.summary).toHaveBeenCalledTimes(3));
  });

  it('polls the summary and the settings every 3s while a turn runs in a chat with a monitor', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    mocks.settings.mockResolvedValue({ data: withMonitor, missing: false });
    const { result, rerender } = renderHook(
      ({ turnRunning }) => useSessionDecisions('a', [], { visible: false, turnRunning }),
      { initialProps: { turnRunning: true } }
    );
    await advance(100);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.settings).toHaveBeenCalledTimes(1);
    // Someone else changes the monitor (the CLI, another window, a parent's
    // save): nothing lands in this chat's transcript, and the next poll shows it.
    const moved: DecisionSettings = {
      ...withMonitor,
      monitors: [{ ...withMonitor.monitors[0], threshold: 0.6 }],
    };
    mocks.settings.mockResolvedValue({ data: moved, missing: false });
    await advance(DECISION_POLL_MS * 2 + 100);
    expect(mocks.summary).toHaveBeenCalledTimes(3);
    expect(mocks.settings).toHaveBeenCalledTimes(3);
    expect(result.current.data?.settings?.monitors[0].threshold).toBe(0.6);
    // The turn ends with the summary hidden: the polling stops.
    rerender({ turnRunning: false });
    await advance(DECISION_POLL_MS * 4);
    expect(mocks.summary).toHaveBeenCalledTimes(3);
    expect(mocks.settings).toHaveBeenCalledTimes(3);
  });

  it('polls only the settings while a turn runs in a chat that never used decision models', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    renderHook(() => useSessionDecisions('a', [], { visible: true, turnRunning: true }));
    await advance(100);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.settings).toHaveBeenCalledTimes(1);
    await advance(DECISION_POLL_MS * 4);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.settings).toHaveBeenCalledTimes(5);
  });

  it('shows a monitor attached from elsewhere mid-turn, then polls the summary too', async () => {
    // A subagent's tab: no monitors when it opens, then its parent's agent
    // (`workspace_monitor`) or a parent's save puts one on it while it runs.
    vi.useFakeTimers({ shouldAdvanceTime: false });
    const { result } = renderHook(() =>
      useSessionDecisions('child', [], { visible: true, turnRunning: true })
    );
    await advance(100);
    expect(result.current.data?.settings?.monitors).toEqual([]);
    mocks.settings.mockResolvedValue({ data: withMonitor, missing: false });
    await advance(DECISION_POLL_MS);
    expect(result.current.data?.settings).toEqual(withMonitor);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    // The chat now uses decision models: the summary is polled from here on.
    mocks.summary.mockResolvedValue({ data: summary(2), missing: false });
    await advance(DECISION_POLL_MS);
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    expect(result.current.data?.summary?.total).toBe(2);
  });

  it('stops polling a running turn on a daemon without the routes', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    mocks.summary.mockResolvedValue({ data: null, missing: true });
    mocks.settings.mockResolvedValue({ data: null, missing: true });
    renderHook(() => useSessionDecisions('a', [], { visible: true, turnRunning: true }));
    await advance(100);
    await advance(DECISION_POLL_MS * 4);
    expect(mocks.summary).toHaveBeenCalledTimes(1);
    expect(mocks.settings).toHaveBeenCalledTimes(1);
  });

  it('keeps the last good answer when a read fails, and reads again on refresh', async () => {
    mocks.summary.mockResolvedValueOnce({ data: summary(3), missing: false });
    const { result } = renderHook(() =>
      useSessionDecisions('a', [], { visible: true, turnRunning: false })
    );
    await waitFor(() => expect(result.current.data?.summary?.total).toBe(3));
    mocks.summary.mockRejectedValueOnce(new Error('offline'));
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.error).toBe(true));
    expect(result.current.data?.summary?.total).toBe(3);
    expect(mocks.settings).toHaveBeenCalledTimes(2);
  });

  it('reads a daemon without the routes as nothing to draw', async () => {
    mocks.summary.mockResolvedValue({ data: null, missing: true });
    mocks.settings.mockResolvedValue({ data: null, missing: true });
    const { result } = renderHook(() =>
      useSessionDecisions('a', [], { visible: true, turnRunning: false })
    );
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.data).toBeNull();
    expect(result.current.error).toBe(false);
  });
});
