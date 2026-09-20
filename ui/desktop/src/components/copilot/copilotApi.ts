import { z } from 'zod';
import {
  computerUseConsent as approveCopilot,
  computerUseRevoke as revokeCopilot,
  computerUseSetup as readCopilotSetup,
  computerUseStatus as readCopilotStatus,
} from '../../api/sdk.gen';
import type { ComputerUseStatus as ApiCopilotStatus } from '../../api/types.gen';
import { userActionHeaders } from '../../utils/userAction';

const optionalText = z
  .string()
  .nullish()
  .transform((value) => value ?? undefined);
const optionalBoolean = z
  .boolean()
  .nullish()
  .transform((value) => value ?? undefined);
const runtimeSchema = z.object({
  status: z.string(),
  runtime_version: optionalText,
  target: optionalText,
  executable: optionalText,
  development_override: optionalBoolean,
  permissions: z.union([
    z.string(),
    z.object({ accessibility: z.boolean().nullish(), screen_recording: z.boolean().nullish() }),
  ]),
  desktop_available: optionalBoolean,
  capture_available: optionalBoolean,
  message: optionalText,
  host: optionalText,
  error: optionalText,
});

export type CopilotRuntime = z.infer<typeof runtimeSchema>;
export type CopilotStatus = Omit<ApiCopilotStatus, 'runtime'> & {
  runtime?: CopilotRuntime;
};

function parseRuntime(value: unknown): CopilotRuntime {
  const parsed = runtimeSchema.safeParse(value);
  if (parsed.success) return parsed.data;
  return {
    status: 'probe_failed',
    permissions: 'unknown',
    message:
      'The backend returned an invalid readiness response. Update or repair Biorouter and check again.',
  };
}

function statusForDisplay(status: ApiCopilotStatus): CopilotStatus {
  return { ...status, runtime: parseRuntime(status.runtime) };
}

/**
 * Thrown when `GET /agent/computer_use/status` refuses in a way that RE-ASKING
 * cannot change.
 *
 * The route answers 409 for a session whose mode forbids Biorouter Copilot outright
 * ("Chat mode does not run Biorouter Copilot tools"). That is a statement about the
 * chat, not a transient fault, and the difference matters twice over: a caller
 * that treats it as retryable shows the user a permanent error they cannot act
 * on, and it keeps polling a probe that re-spawns the native helper.
 */
export class CopilotNotApplicable extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CopilotNotApplicable';
  }
}

export async function copilotStatus(sessionId: string): Promise<CopilotStatus> {
  // Deliberately NOT `throwOnError`: the status code is the signal, and
  // throwing discards it. Every other helper here keeps `throwOnError` because
  // a failed decision or probe really is retryable.
  const result = await readCopilotStatus({
    query: { session_id: sessionId },
    headers: await userActionHeaders(),
  });
  if (result.error !== undefined || result.data === undefined) {
    const detail = result.error as { message?: string } | undefined;
    const message = detail?.message ?? 'Biorouter Copilot status unavailable';
    if (result.response?.status === 409) throw new CopilotNotApplicable(message);
    throw new Error(message);
  }
  return statusForDisplay(result.data);
}

export async function copilotDecision(
  status: CopilotStatus,
  action: 'consent' | 'revoke',
  approvalKey: string
): Promise<CopilotStatus> {
  const headers = await userActionHeaders();
  const response =
    action === 'consent'
      ? await approveCopilot({
          headers: { ...headers, ...(approvalKey ? { 'X-Computer-Use-Key': approvalKey } : {}) },
          body: { session_id: status.session_id, challenge_id: status.challenge_id },
          throwOnError: true,
        })
      : await revokeCopilot({
          headers,
          body: { session_id: status.session_id },
          throwOnError: true,
        });
  return statusForDisplay(response.data);
}

export async function copilotSetup(): Promise<CopilotRuntime> {
  const response = await readCopilotSetup({
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  return parseRuntime(response.data);
}
