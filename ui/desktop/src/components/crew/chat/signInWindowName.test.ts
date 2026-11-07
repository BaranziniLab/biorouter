import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readSignInWindowName, useSignInWindowName } from './signInWindowName';

const mocks = vi.hoisted(() => ({
  readSetup: vi.fn(),
  crewHttp: vi.fn(),
}));

vi.mock('../onboarding/setup', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  readSetup: mocks.readSetup,
}));
vi.mock('../crewApi', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  crewHttp: mocks.crewHttp,
}));

beforeEach(() => {
  mocks.readSetup.mockReset();
  mocks.crewHttp.mockReset();
});

describe('the Sign in window’s name for a chat card', () => {
  it('names a setup job by the SSH login the daemon has for it', async () => {
    mocks.readSetup.mockResolvedValue({ server: { login: 'you\u202E@hpc.example.edu' } });
    expect(await readSignInWindowName({ kind: 'setup', jobId: 'job-1' })).toBe(
      'you@hpc.example.edu'
    );
    expect(mocks.readSetup).toHaveBeenCalledWith('job-1', undefined);
    mocks.readSetup.mockResolvedValue({ server: { login: '', user: 'you' } });
    expect(await readSignInWindowName({ kind: 'setup', jobId: 'job-1' })).toBe('you');
  });

  it('names a saved connection the way the Crew view’s Sign in window does', async () => {
    mocks.crewHttp.mockResolvedValue({
      connections: [
        { id: 'conn-2', name: 'Other', ssh_target: 'me@10.0.0.8', server_label: 'other server' },
        { id: 'conn-1', name: 'Lab', ssh_target: 'me@10.0.0.7', server_label: 'lab server' },
      ],
    });
    expect(await readSignInWindowName({ kind: 'connection', id: 'conn-1' })).toBe('lab server');
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections', 'GET', undefined, undefined);
    // A connection the daemon does not list is named by nothing.
    expect(await readSignInWindowName({ kind: 'connection', id: 'gone' })).toBe('');
  });

  it('reads nothing for a card that cannot open the window, and answers null when it fails', async () => {
    const off = renderHook(() => useSignInWindowName({ kind: 'setup', jobId: 'job-1' }, false));
    expect(off.result.current).toBeNull();
    expect(mocks.readSetup).not.toHaveBeenCalled();

    mocks.readSetup.mockRejectedValue(new Error('gone'));
    const failed = renderHook(() => useSignInWindowName({ kind: 'setup', jobId: 'job-1' }, true));
    await waitFor(() => expect(mocks.readSetup).toHaveBeenCalled());
    expect(failed.result.current).toBeNull();

    mocks.readSetup.mockResolvedValue({ server: { login: 'you@hpc.example.edu' } });
    const read = renderHook(() => useSignInWindowName({ kind: 'setup', jobId: 'job-2' }, true));
    await waitFor(() => expect(read.result.current).toBe('you@hpc.example.edu'));
  });
});
