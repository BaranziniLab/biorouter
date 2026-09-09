import { describe, expect, it, vi, beforeEach } from 'vitest';

const mocks = vi.hoisted(() => ({
  createSchedule: vi.fn(),
}));

vi.mock('./api', () => ({
  listSchedules: vi.fn(),
  createSchedule: mocks.createSchedule,
  deleteSchedule: vi.fn(),
  pauseSchedule: vi.fn(),
  unpauseSchedule: vi.fn(),
  updateSchedule: vi.fn(),
  sessionsHandler: vi.fn(),
  runNowHandler: vi.fn(),
  killRunningJob: vi.fn(),
  inspectRunningJob: vi.fn(),
}));

const { createSchedule, serverErrorText } = await import('./schedule');

const request = { id: 'probe', workflow_source: '/tmp/wf.yaml', cron: '0 0 14 * * *' };

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

describe('serverErrorText', () => {
  /**
   * `routes/errors.rs` — the shape most of the daemon answers errors in, and
   * the one `POST /schedule/create` now uses.
   */
  it('reads the message out of an ErrorResponse body', () => {
    expect(serverErrorText({ message: "schedule id 'x y' may only contain letters" })).toBe(
      "schedule id 'x y' may only contain letters"
    );
  });

  /** The other shape in `routes/schedule.rs`: `Err((StatusCode, String))`. */
  it('takes a plain-text body as the message', () => {
    expect(serverErrorText('Schedule ‘nightly’ was not found.')).toBe(
      'Schedule ‘nightly’ was not found.'
    );
  });

  /**
   * A body-less status is the case that produced the bug. Reporting `null` is
   * what lets the caller say "the server answered 400" rather than inventing a
   * reason or blaming the response format.
   */
  it('reports nothing rather than something empty', () => {
    expect(serverErrorText(undefined)).toBeNull();
    expect(serverErrorText(null)).toBeNull();
    expect(serverErrorText('')).toBeNull();
    expect(serverErrorText('   ')).toBeNull();
    expect(serverErrorText({})).toBeNull();
    expect(serverErrorText({ message: '  ' })).toBeNull();
    expect(serverErrorText({ message: 42 })).toBeNull();
  });
});

describe('createSchedule error reporting', () => {
  /**
   * Finding F4, end to end on the client half. The daemon refuses the name and
   * says why; this used to throw that away and report "Unexpected response
   * format" — a transport-shaped message for a validation problem.
   */
  it('surfaces the daemon reason instead of "Unexpected response format"', async () => {
    mocks.createSchedule.mockResolvedValue({
      data: undefined,
      error: {
        message:
          "Invalid job ID: schedule id '<img src=x onerror=alert(1)>' may only contain letters, digits, '-' and '_'",
      },
      response: { status: 400 },
    });

    await expect(createSchedule(request)).rejects.toThrow(
      "Failed to create schedule: Invalid job ID: schedule id '<img src=x onerror=alert(1)>' may only contain letters, digits, '-' and '_'"
    );
  });

  /** A route that still answers without a body names its status, not its shape. */
  it('names the status when the daemon sent no body', async () => {
    mocks.createSchedule.mockResolvedValue({
      data: undefined,
      error: undefined,
      response: { status: 500 },
    });

    await expect(createSchedule(request)).rejects.toThrow(
      'Failed to create schedule: the server answered 500 with no explanation.'
    );
  });

  it('passes a successful create straight through', async () => {
    mocks.createSchedule.mockResolvedValue({ data: { id: 'probe' } });
    await expect(createSchedule(request)).resolves.toEqual({ id: 'probe' });
  });
});
