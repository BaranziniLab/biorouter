import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ComputerUseNotApplicable,
  computerUseDecision,
  computerUseSetup,
  computerUseStatus,
} from './computerUseApi';

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  setup: vi.fn(),
  consent: vi.fn(),
  revoke: vi.fn(),
}));
vi.mock('../../api/sdk.gen', () => ({
  computerUseStatus: mocks.status,
  computerUseSetup: mocks.setup,
  computerUseConsent: mocks.consent,
  computerUseRevoke: mocks.revoke,
}));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-Caller-Provider': 'private-provider' }),
}));
const status = {
  session_id: 'task-a',
  provider: 'provider',
  model: 'model',
  destination: 'destination',
  target: 'host',
  disclosure: 'Allow this request',
  state: 'approval_required',
  challenge_id: 'nonce',
  public_model: false,
  handoff_required: false,
  requested: true,
  enabled: true,
  runtime: { status: 'probe_pending', permissions: 'unknown' },
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.status.mockResolvedValue({ data: status });
  mocks.consent.mockResolvedValue({ data: { ...status, state: 'active', requested: false } });
  mocks.revoke.mockResolvedValue({ data: { ...status, state: 'stopped', requested: false } });
});

describe('generated Computer Use API adapter', () => {
  it('scopes status to the selected chat and never attaches the approval passphrase to reads or Stop', async () => {
    const current = await computerUseStatus('task-a');
    expect(mocks.status).toHaveBeenCalledWith(
      expect.objectContaining({
        query: { session_id: 'task-a' },
        headers: { 'X-Caller-Provider': 'private-provider' },
      })
    );
    await computerUseDecision(current, 'revoke', 'secret-passphrase');
    expect(mocks.revoke).toHaveBeenCalledWith(
      expect.objectContaining({
        body: { session_id: 'task-a' },
        headers: { 'X-Caller-Provider': 'private-provider' },
      })
    );
    expect(mocks.consent).not.toHaveBeenCalled();
  });

  it('sends the reviewed challenge and separate human proof only for an explicit grant', async () => {
    const current = await computerUseStatus('task-a');
    await computerUseDecision(current, 'consent', 'secret-passphrase');
    expect(mocks.consent).toHaveBeenCalledWith(
      expect.objectContaining({
        body: { session_id: 'task-a', challenge_id: 'nonce' },
        headers: {
          'X-Caller-Provider': 'private-provider',
          'X-Computer-Use-Key': 'secret-passphrase',
        },
      })
    );
  });

  it('accepts native permission objects, including unreported checks, without inventing readiness', async () => {
    mocks.setup.mockResolvedValue({
      data: {
        status: 'os_permission_required',
        permissions: { accessibility: true, screen_recording: null },
        message: null,
      },
    });
    expect(await computerUseSetup()).toMatchObject({
      status: 'os_permission_required',
      permissions: { accessibility: true, screen_recording: null },
    });
    mocks.setup.mockResolvedValue({ data: { unexpected: true } });
    expect(await computerUseSetup()).toMatchObject({
      status: 'probe_failed',
      permissions: 'unknown',
    });
  });
});

describe('a refusal that re-asking cannot change', () => {
  it('maps a 409 to ComputerUseNotApplicable so the caller can stop asking', async () => {
    // The route answers 409 for a session whose mode forbids Computer Use.
    // Losing the STATUS here is what made the panel show a permanent error and
    // keep polling a probe that re-spawns the native helper.
    mocks.status.mockResolvedValue({
      error: { message: 'Chat mode does not run Computer Use tools' },
      response: { status: 409 },
    });
    await expect(computerUseStatus('task-a')).rejects.toBeInstanceOf(ComputerUseNotApplicable);
    await expect(computerUseStatus('task-a')).rejects.toThrow(
      'Chat mode does not run Computer Use tools'
    );
  });

  it('leaves every other failure an ordinary, retryable Error', async () => {
    // The discriminating control. A 500 or a 403 IS worth retrying, and must
    // not be silently turned into "this chat has no Computer Use".
    for (const code of [403, 500, 503]) {
      mocks.status.mockResolvedValue({
        error: { message: 'upstream unavailable' },
        response: { status: code },
      });
      const failure = await computerUseStatus('task-a').catch((error: unknown) => error);
      expect(failure).toBeInstanceOf(Error);
      expect(failure).not.toBeInstanceOf(ComputerUseNotApplicable);
    }
  });
});
