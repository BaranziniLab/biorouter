import { z } from 'zod';
import {
  computerUseConsent as approveComputerUse,
  computerUseRevoke as revokeComputerUse,
  computerUseSetup as readComputerUseSetup,
  computerUseStatus as readComputerUseStatus,
} from '../../api/sdk.gen';
import type { ComputerUseStatus as ApiComputerUseStatus } from '../../api/types.gen';
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

export type ComputerUseRuntime = z.infer<typeof runtimeSchema>;
export type ComputerUseStatus = Omit<ApiComputerUseStatus, 'runtime'> & {
  runtime?: ComputerUseRuntime;
};

function parseRuntime(value: unknown): ComputerUseRuntime {
  const parsed = runtimeSchema.safeParse(value);
  if (parsed.success) return parsed.data;
  return {
    status: 'probe_failed',
    permissions: 'unknown',
    message:
      'The backend returned an invalid readiness response. Update or repair Biorouter and check again.',
  };
}

function statusForDisplay(status: ApiComputerUseStatus): ComputerUseStatus {
  return { ...status, runtime: parseRuntime(status.runtime) };
}

export async function computerUseStatus(sessionId: string): Promise<ComputerUseStatus> {
  const response = await readComputerUseStatus({
    query: { session_id: sessionId },
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  return statusForDisplay(response.data);
}

export async function computerUseDecision(
  status: ComputerUseStatus,
  action: 'consent' | 'revoke',
  approvalKey: string
): Promise<ComputerUseStatus> {
  const headers = await userActionHeaders();
  const response =
    action === 'consent'
      ? await approveComputerUse({
          headers: { ...headers, ...(approvalKey ? { 'X-Computer-Use-Key': approvalKey } : {}) },
          body: { session_id: status.session_id, challenge_id: status.challenge_id },
          throwOnError: true,
        })
      : await revokeComputerUse({
          headers,
          body: { session_id: status.session_id },
          throwOnError: true,
        });
  return statusForDisplay(response.data);
}

export async function computerUseSetup(): Promise<ComputerUseRuntime> {
  const response = await readComputerUseSetup({
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  return parseRuntime(response.data);
}
