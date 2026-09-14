import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * Every Schedules call the renderer makes carries the person's proof.
 *
 * Issue #56: the daemon answers a schedule request without `X-User-Action` as a
 * public model's. It redacts the chats a schedule names from the list, refuses
 * to stop or inspect a run in a private chat, and refuses to create, run,
 * re-time, pause, resume or delete a schedule whose work is private — work on a
 * private model, or for a private chat. The desktop is the person at the
 * keyboard, but the daemon only believes that when the request says so.
 *
 * ⚠ **Measured on `main` (1038a113) before this test existed:** `killRunningJob`,
 * `inspectRunningJob` and `listSchedules` were already gated by the daemon and
 * sent nothing, so Stop on a private chat's scheduled run was refused in the
 * desktop app itself. A missing proof here is not an error the view can tell
 * from a real one, which is why it is pinned per call rather than left to review.
 */

const PROOF = { 'X-User-Action': 'renderer-proof-of-user' };

const mocks = vi.hoisted(() => ({
  userActionHeaders: vi.fn(),
  api: {
    listSchedules: vi.fn(),
    createSchedule: vi.fn(),
    deleteSchedule: vi.fn(),
    pauseSchedule: vi.fn(),
    unpauseSchedule: vi.fn(),
    updateSchedule: vi.fn(),
    sessionsHandler: vi.fn(),
    runNowHandler: vi.fn(),
    killRunningJob: vi.fn(),
    inspectRunningJob: vi.fn(),
  },
}));

vi.mock('./api', () => mocks.api);
vi.mock('./utils/userAction', () => ({ userActionHeaders: mocks.userActionHeaders }));

const schedule = await import('./schedule');

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'error').mockImplementation(() => {});
  mocks.userActionHeaders.mockResolvedValue(PROOF);
  for (const fn of Object.values(mocks.api)) {
    fn.mockResolvedValue({ data: {} });
  }
  mocks.api.listSchedules.mockResolvedValue({ data: { jobs: [] } });
  mocks.api.runNowHandler.mockResolvedValue({ data: { session_id: 'run-1' } });
  mocks.api.sessionsHandler.mockResolvedValue({ data: [] });
});

/** `[what the renderer calls, the generated function it must reach, with the proof]`. */
const CALLS: [string, () => Promise<unknown>, keyof typeof mocks.api][] = [
  ['listSchedules', () => schedule.listSchedules(), 'listSchedules'],
  [
    'createSchedule',
    () =>
      schedule.createSchedule({ id: 'nightly', workflow_source: '/wf.yaml', cron: '0 0 1 * * *' }),
    'createSchedule',
  ],
  ['deleteSchedule', () => schedule.deleteSchedule('nightly'), 'deleteSchedule'],
  ['getScheduleSessions', () => schedule.getScheduleSessions('nightly', 5), 'sessionsHandler'],
  ['runScheduleNow', () => schedule.runScheduleNow('nightly'), 'runNowHandler'],
  ['pauseSchedule', () => schedule.pauseSchedule('nightly'), 'pauseSchedule'],
  ['unpauseSchedule', () => schedule.unpauseSchedule('nightly'), 'unpauseSchedule'],
  ['updateSchedule', () => schedule.updateSchedule('nightly', '0 0 2 * * *'), 'updateSchedule'],
  ['killRunningJob', () => schedule.killRunningJob('nightly'), 'killRunningJob'],
  ['inspectRunningJob', () => schedule.inspectRunningJob('nightly'), 'inspectRunningJob'],
];

describe("the Schedules calls carry the person's proof", () => {
  it.each(CALLS)('%s', async (_name, invoke, generated) => {
    await invoke();
    expect(mocks.api[generated]).toHaveBeenCalledTimes(1);
    expect(mocks.api[generated].mock.calls[0][0]).toEqual(
      expect.objectContaining({ headers: PROOF })
    );
  });

  it('covers every generated Schedules function the module imports', () => {
    const exercised = new Set(CALLS.map(([, , generated]) => generated));
    expect([...exercised].sort()).toEqual(Object.keys(mocks.api).sort());
  });

  /**
   * The generated client RESOLVES a failed request, and these three answer 204
   * with no body — so each used to report a refusal as success, and the view
   * toasted "paused" over a schedule that was not.
   */
  it.each([
    ['deleteSchedule', () => schedule.deleteSchedule('nightly'), 'deleteSchedule'],
    ['pauseSchedule', () => schedule.pauseSchedule('nightly'), 'pauseSchedule'],
    ['unpauseSchedule', () => schedule.unpauseSchedule('nightly'), 'unpauseSchedule'],
  ] as const)('%s reports a refusal instead of success', async (_name, invoke, generated) => {
    const refusal = "That schedule's work is private, or there is no schedule with that id.";
    mocks.api[generated].mockResolvedValue({
      error: refusal,
      response: { ok: false, status: 403 },
    });
    await expect(invoke()).rejects.toThrow(refusal);
  });

  /** A surface with no bridge sends nothing rather than something invented. */
  it('sends no proof it does not have', async () => {
    mocks.userActionHeaders.mockResolvedValue({});
    await schedule.pauseSchedule('nightly');
    expect(mocks.api.pauseSchedule.mock.calls[0][0]).toEqual(
      expect.objectContaining({ headers: {} })
    );
  });
});
