import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderDetails } from '../../../api';
import { CREW_TASK_MODEL_KEY } from '../state/crewTaskModel';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { useConfiguredModels } from './useConfiguredModels';

/**
 * The Crew task model follows Crew's change feed (WI-15): a chat's `crew__change defaults_set`
 * writes `CREW_TASK_MODEL` in the daemon, which no window announces, and the chip that names the
 * model a task starts on must show the new one without a reload.
 */

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  read: vi.fn(),
  crewModel: null as unknown,
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders, read: mocks.read }),
}));

const versa = {
  name: 'versa_azure',
  is_configured: true,
  metadata: { display_name: 'Versa', known_models: [{ name: 'gpt-5.5' }] },
} as unknown as ProviderDetails;

const taskModelReads = () =>
  mocks.read.mock.calls.filter(([key]) => key === CREW_TASK_MODEL_KEY).length;

const change = (kind: string, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind,
      connection_id: null,
      channel_id: null,
      session_id: 'chat-1',
      run_id: null,
      job_id: null,
    },
  ],
});

const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.crewModel = { provider: 'versa_azure', model: 'gpt-5' };
  mocks.getProviders.mockResolvedValue([versa]);
  mocks.read.mockImplementation(async (key: string) => {
    if (key === CREW_TASK_MODEL_KEY) return mocks.crewModel;
    if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
    if (key === 'BIOROUTER_MODEL') return 'gpt-5.5';
    return null;
  });
});

describe('useConfiguredModels and the change feed', () => {
  it('shows the task model a chat set, with one read for a burst', async () => {
    const { result } = renderHook(() => useConfiguredModels());
    await waitFor(() =>
      expect(result.current.taskDefault).toEqual({ provider: 'versa_azure', model: 'gpt-5' })
    );
    expect(taskModelReads()).toBe(1);

    mocks.crewModel = { provider: 'versa_azure', model: 'gpt-5.5' };
    dispatchCrewChanged(change('defaults_set', 1));
    dispatchCrewChanged(change('defaults_set', 2));
    await waitFor(() =>
      expect(result.current.taskDefault).toEqual({ provider: 'versa_azure', model: 'gpt-5.5' })
    );
    expect(result.current.taskDefaultSource).toBe('crew');
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(taskModelReads()).toBe(2);
  });

  it('falls back to the app model when a chat clears it', async () => {
    const { result } = renderHook(() => useConfiguredModels());
    await waitFor(() => expect(result.current.taskDefaultSource).toBe('crew'));
    mocks.crewModel = null;
    dispatchCrewChanged(change('defaults_clear'));
    await waitFor(() => expect(result.current.taskDefaultSource).toBe('app'));
  });

  it('reads nothing again for a change that is not about the task model', async () => {
    const { result } = renderHook(() => useConfiguredModels());
    await waitFor(() => expect(result.current.taskDefault).not.toBeNull());
    dispatchCrewChanged(change('send'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(taskModelReads()).toBe(1);
  });

  it('reads nothing before a surface asked for the models', async () => {
    renderHook(() => useConfiguredModels({ enabled: false }));
    dispatchCrewChanged(change('defaults_set'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(taskModelReads()).toBe(0);
  });
});
