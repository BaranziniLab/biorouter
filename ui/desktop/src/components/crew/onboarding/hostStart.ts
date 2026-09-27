import { crewHttp } from '../crewApi';
import { outdatedDaemonResponse, unexpectedCrewResponse } from '../api/errors';
import { isRecord } from '../api/parse';

/**
 * "Start it for me" (D-HOST): the Host dialog's second step asks the daemon to run the start
 * commands it shows, instead of the person running them in a terminal and pasting the output.
 *
 * The renderer never sends a command. It names this computer's pending host setup (the
 * preparation the daemon made at Continue), the workspace name and the SSH login and route the
 * person typed; the daemon builds the command itself from the name and the hosting key it holds,
 * character for character what `hostStartCommands` shows (`biorouter::crew::host_start`, whose test
 * reads `joinText.ts`). Every call goes through `crewHttp`, so each one carries the proof that a
 * person asked, and the daemon refuses any that does not.
 */

/** What starting a run sends: exactly the fields the route accepts (it refuses any other). */
export interface HostStartInput {
  preparation_id: string;
  workspace_name: string;
  ssh_target: string;
  port: number | null;
  identity_file: string | null;
  proxy_jump: string | null;
}

export type HostStartState = 'running' | 'finished' | 'failed';

/** What the output says, read as a paste is read. */
export type HostStartResult =
  /** The `brcrew1:` line (or status JSON) to preview and pin, exactly as a paste would give it. */
  | { kind: 'found'; text: string }
  /** `starting`, `not_installed`, `server_error` (with `detail`) or `unreadable`. */
  | { kind: 'problem'; problem: string; detail: string | null };

export interface HostStartRun {
  jobId: string;
  /** The exact command text the daemon runs. */
  command: string;
  state: HostStartState;
  /** Everything printed so far, stdout and stderr in arrival order, control characters removed. */
  output: string;
  result: HostStartResult | null;
  /** Once `failed`: a typed code (`crew_ssh_auth_required`, …) and the daemon's sentence. */
  error: { code: string; message: string } | null;
}

/** How often a running start is read again. */
export const HOST_START_POLL_MS = 500;

const STATES: readonly string[] = ['running', 'finished', 'failed'];

function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function resultFrom(value: unknown): HostStartResult | null {
  if (!isRecord(value)) return null;
  if (value.kind === 'found') {
    const found = text(value.text);
    return found ? { kind: 'found', text: found } : null;
  }
  if (value.kind === 'problem') {
    const problem = text(value.problem);
    return problem ? { kind: 'problem', problem, detail: text(value.detail) } : null;
  }
  return null;
}

/**
 * A run as the daemon answered it, or a refusal: a body that is not a run is never read as one. An
 * older daemon that answers a route it does not have with a web page is reported as outdated.
 */
export function hostStartRunFrom(value: unknown): HostStartRun {
  if (!isRecord(value)) throw outdatedDaemonResponse();
  const jobId = text(value.job_id);
  const command = text(value.command);
  const state = text(value.state);
  if (!jobId || command === null || !state || !STATES.includes(state)) {
    throw unexpectedCrewResponse('a host start');
  }
  const error =
    isRecord(value.error) && text(value.error.code) && text(value.error.message)
      ? { code: value.error.code as string, message: value.error.message as string }
      : null;
  return {
    jobId,
    command,
    state: state as HostStartState,
    output: text(value.output) ?? '',
    result: resultFrom(value.result),
    error,
  };
}

/** Start the host setup's commands on the server. Answers the run already under way, if one is. */
export async function startHostRun(input: HostStartInput): Promise<HostStartRun> {
  const body: HostStartInput = {
    preparation_id: input.preparation_id,
    workspace_name: input.workspace_name,
    ssh_target: input.ssh_target,
    port: input.port,
    identity_file: input.identity_file,
    proxy_jump: input.proxy_jump,
  };
  return hostStartRunFrom(await crewHttp<unknown>('/host/start', 'POST', body));
}

/** Where a run stands: its output so far and, once done, what it read. */
export async function readHostRun(jobId: string, signal?: AbortSignal): Promise<HostStartRun> {
  return hostStartRunFrom(
    await crewHttp<unknown>(`/host/start/${encodeURIComponent(jobId)}`, 'GET', undefined, signal)
  );
}

/** Stop a run. Stopping a finished run changes nothing. */
export async function stopHostRun(jobId: string): Promise<void> {
  await crewHttp<unknown>(`/host/start/${encodeURIComponent(jobId)}`, 'DELETE');
}
