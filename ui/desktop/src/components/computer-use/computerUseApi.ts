import { client } from '../../api/client.gen';
import { userActionHeaders } from '../../utils/userAction';

export interface ComputerUseStatus {
  session_id: string;
  provider: string;
  model: string;
  destination: string;
  target: string;
  disclosure: string;
  state: 'approval_required' | 'active' | 'stopped' | 'busy';
  challenge_id: string;
  public_model: boolean;
  handoff_required: boolean;
  requested?: boolean;
  enabled?: boolean;
  runtime?: ComputerUseRuntime;
}

export async function computerUseStatus(sessionId: string): Promise<ComputerUseStatus> {
  const response = await client.get<{ 200: ComputerUseStatus }, unknown, true>({
    url: '/agent/computer_use/status',
    query: { session_id: sessionId },
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  return response.data;
}

export async function computerUseDecision(
  status: ComputerUseStatus,
  action: 'consent' | 'revoke',
  approvalKey: string
): Promise<ComputerUseStatus> {
  const headers = await userActionHeaders();
  const response = await client.post<{ 200: ComputerUseStatus }, unknown, true>({
    url: `/agent/computer_use/${action}`,
    headers: {
      'Content-Type': 'application/json',
      ...headers,
      ...(action === 'consent' && approvalKey ? { 'X-Computer-Use-Key': approvalKey } : {}),
    },
    body: {
      session_id: status.session_id,
      ...(action === 'consent' ? { challenge_id: status.challenge_id } : {}),
    },
    throwOnError: true,
  });
  return response.data;
}

export interface ComputerUseRuntime {
  status: string;
  runtime_version?: string;
  target?: string;
  executable?: string;
  development_override?: boolean;
  permissions: string | { accessibility?: boolean | null; screen_recording?: boolean | null };
  desktop_available?: boolean;
  capture_available?: boolean;
  message?: string;
  host?: string;
  error?: string;
}

export async function computerUseSetup(): Promise<ComputerUseRuntime> {
  const response = await client.get<{ 200: ComputerUseRuntime }, unknown, true>({
    url: '/agent/computer_use/setup',
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  return response.data;
}
