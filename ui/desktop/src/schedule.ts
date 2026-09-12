import {
  listSchedules as apiListSchedules,
  createSchedule as apiCreateSchedule,
  deleteSchedule as apiDeleteSchedule,
  pauseSchedule as apiPauseSchedule,
  unpauseSchedule as apiUnpauseSchedule,
  updateSchedule as apiUpdateSchedule,
  sessionsHandler as apiGetScheduleSessions,
  runNowHandler as apiRunScheduleNow,
  killRunningJob as apiKillRunningJob,
  inspectRunningJob as apiInspectRunningJob,
  SessionDisplayInfo,
} from './api';
import { userActionHeaders } from './utils/userAction';

export interface ScheduledJob {
  id: string;
  source: string;
  cron: string;
  last_run?: string | null;
  currently_running?: boolean;
  paused?: boolean;
  current_session_id?: string | null;
  process_start_time?: string | null;
  /**
   * The last run's failure, or null once a run succeeds (issue #56).
   *
   * A cron tick that returned `Err` used to leave nothing behind but a log
   * line, and a scheduled run mints a fresh session each time — so a job that
   * had been failing since the day it was created had no surface anywhere the
   * user could see. Rendered by `SchedulesView` and `ScheduleDetailView`.
   */
  last_error?: string | null;
}

export interface ScheduleSession {
  id: string;
  name: string;
  createdAt: string; // ISO 8601 date string
  workingDir: string;
  scheduleId: string;
  messageCount: number;
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  accumulatedTotalTokens: number;
  accumulatedInputTokens: number;
  accumulatedOutputTokens: number;
}

/**
 * The sentence the daemon sent back, out of a failed generated-client call.
 *
 * Without `throwOnError` the @hey-api client resolves rather than throwing, with
 * `data` undefined and `error` holding the PARSED body — a `{ message }` object
 * for the routes that answer with `ErrorResponse` (`routes/errors.rs`), a bare
 * string for the ones that answer `(StatusCode, String)`, and `undefined` for a
 * status with no body at all.
 *
 * Every one of those shapes used to collapse into "Unexpected response format",
 * which reported a validation problem the user could fix as a transport fault
 * they could not — the whole of finding F4. Returns `null` when there really is
 * nothing to show, so the caller can say something honest instead of inventing
 * a reason.
 */
export function serverErrorText(error: unknown): string | null {
  if (typeof error === 'string') {
    return error.trim() || null;
  }
  if (error && typeof error === 'object') {
    const { message } = error as { message?: unknown };
    if (typeof message === 'string') {
      return message.trim() || null;
    }
  }
  return null;
}

/**
 * `prefix`, plus whatever the daemon was willing to say about why.
 *
 * A body-less failure names its status rather than its shape: "the server
 * answered 500" is a report the user can act on (retry, check the daemon);
 * "Unexpected response format" sends them looking for a bug in the client.
 */
function failureMessage(
  prefix: string,
  response: { error?: unknown; response?: { status?: number } } | undefined
): string {
  const text = serverErrorText(response?.error);
  if (text) {
    return `${prefix}: ${text}`;
  }
  const status = response?.response?.status;
  return status
    ? `${prefix}: the server answered ${status} with no explanation.`
    : `${prefix}: no response from the server.`;
}

export async function listSchedules(): Promise<ScheduledJob[]> {
  try {
    const response = await apiListSchedules<true>();
    if (response && response.data && Array.isArray(response.data.jobs)) {
      return response.data.jobs as ScheduledJob[];
    }
    console.error('Failed response from apiListSchedules', response);
    throw new Error(failureMessage('Failed to list schedules', response));
  } catch (error) {
    console.error('Error listing schedules:', error);
    throw error;
  }
}

export async function createSchedule(request: {
  id: string;
  workflow_source: string;
  cron: string;
  execution_mode?: string;
}): Promise<ScheduledJob> {
  try {
    const response = await apiCreateSchedule<true>({ body: request });
    if (response && response.data) {
      return response.data as ScheduledJob;
    }
    console.error('Failed response from apiCreateSchedule', response);
    throw new Error(failureMessage('Failed to create schedule', response));
  } catch (error) {
    console.error('Error creating schedule:', error);
    throw error;
  }
}

export async function deleteSchedule(id: string): Promise<void> {
  try {
    await apiDeleteSchedule<true>({ path: { id } });
  } catch (error) {
    console.error(`Error deleting schedule ${id}:`, error);
    throw error;
  }
}

export async function getScheduleSessions(
  scheduleId: string,
  limit: number
): Promise<Array<SessionDisplayInfo>> {
  // With the user's proof: a schedule's private runs are omitted from a caller
  // without it (issue #56, QA 2026-09-10 M1).
  const response = await apiGetScheduleSessions<true>({
    path: { id: scheduleId },
    query: { limit },
    headers: await userActionHeaders(),
    throwOnError: true,
  });

  return response.data;
}

export async function runScheduleNow(scheduleId: string): Promise<string> {
  try {
    const response = await apiRunScheduleNow<true>({
      path: { id: scheduleId },
    });

    if (response && response.data && response.data.session_id) {
      return response.data.session_id;
    }
    console.error('Failed response from apiRunScheduleNow', response);
    throw new Error(failureMessage('Failed to run schedule now', response));
  } catch (error) {
    console.error(`Error running schedule ${scheduleId} now:`, error);
    throw error;
  }
}

export async function pauseSchedule(scheduleId: string): Promise<void> {
  try {
    await apiPauseSchedule<true>({
      path: { id: scheduleId },
    });
  } catch (error) {
    console.error(`Error pausing schedule ${scheduleId}:`, error);
    throw error;
  }
}

export async function unpauseSchedule(scheduleId: string): Promise<void> {
  try {
    await apiUnpauseSchedule<true>({
      path: { id: scheduleId },
    });
  } catch (error) {
    console.error(`Error unpausing schedule ${scheduleId}:`, error);
    throw error;
  }
}

export async function updateSchedule(scheduleId: string, cron: string): Promise<ScheduledJob> {
  try {
    const response = await apiUpdateSchedule<true>({
      path: { id: scheduleId },
      body: { cron },
    });

    if (response && response.data) {
      return response.data as ScheduledJob;
    }
    console.error('Failed response from apiUpdateSchedule', response);
    throw new Error(failureMessage('Failed to update schedule', response));
  } catch (error) {
    console.error(`Error updating schedule ${scheduleId}:`, error);
    throw error;
  }
}

export interface KillJobResponse {
  message: string;
}

export interface InspectJobResponse {
  sessionId?: string | null;
  processStartTime?: string | null;
  runningDurationSeconds?: number | null;
}

export async function killRunningJob(scheduleId: string): Promise<KillJobResponse> {
  try {
    const response = await apiKillRunningJob<true>({
      path: { id: scheduleId },
    });

    if (response && response.data) {
      return response.data as KillJobResponse;
    }
    console.error('Failed response from apiKillRunningJob', response);
    throw new Error(failureMessage('Failed to kill running job', response));
  } catch (error) {
    console.error(`Error killing running job ${scheduleId}:`, error);
    throw error;
  }
}

export async function inspectRunningJob(scheduleId: string): Promise<InspectJobResponse> {
  try {
    const response = await apiInspectRunningJob<true>({
      path: { id: scheduleId },
    });

    if (response && response.data) {
      return response.data as InspectJobResponse;
    }
    console.error('Failed response from apiInspectRunningJob', response);
    throw new Error(failureMessage('Failed to inspect running job', response));
  } catch (error) {
    console.error(`Error inspecting running job ${scheduleId}:`, error);
    throw error;
  }
}
