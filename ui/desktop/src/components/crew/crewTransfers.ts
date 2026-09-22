import { crewHttp } from './crewApi';

export type TransferDirection = 'upload' | 'download';
export interface CrewTransfer {
  id: string;
  request_id: string;
  connection_id: string;
  channel_id: string;
  direction: TransferDirection;
  name: string;
  size: number;
  sha256: string;
  offset: number;
  blob_id: string | null;
  state: string;
  error: string | null;
  destination_identity?: string | null;
}
export interface FileSelectionRequest {
  expected_mode?: 'private' | 'public';
  purpose?: 'transfer' | 'cleanup';
  connection_id: string;
  channel_id: string;
  direction: TransferDirection;
  blob_id?: string;
  transfer_id?: string;
  suggestedName?: string;
}
interface FileCapability {
  capability_id: string;
  name: string;
  size?: number | null;
}
export async function chooseTransferFile(
  request: FileSelectionRequest
): Promise<FileCapability | null> {
  const picker = window.electron.crewSelectTransferFile;
  if (!picker)
    throw new Error(
      'This desktop build does not provide the secure Crew file picker. Update the desktop app before transferring local files.'
    );
  return picker({
    expectedMode: request.expected_mode,
    purpose: request.purpose,
    direction: request.direction,
    connectionId: request.connection_id,
    channelId: request.channel_id,
    blobId: request.blob_id,
    transferId: request.transfer_id,
    suggestedName: request.suggestedName,
  });
}
export async function listTransfers(
  connectionId: string,
  channelId?: string
): Promise<CrewTransfer[]> {
  const query = new URLSearchParams({ connection_id: connectionId });
  if (channelId) query.set('channel_id', channelId);
  return (await crewHttp<{ transfers: CrewTransfer[] }>(`/transfers?${query}`)).transfers;
}
export async function beginTransfer(request: FileSelectionRequest): Promise<CrewTransfer | null> {
  const file = await chooseTransferFile(request);
  if (!file) return null;
  return crewHttp<CrewTransfer>('/transfers', 'POST', {
    request_id: crypto.randomUUID(),
    connection_id: request.connection_id,
    channel_id: request.channel_id,
    direction: request.direction,
    file_capability: file.capability_id,
    blob_id: request.blob_id,
  });
}
export async function resumeTransfer(transfer: CrewTransfer): Promise<CrewTransfer | null> {
  const file = await chooseTransferFile({
    connection_id: transfer.connection_id,
    channel_id: transfer.channel_id,
    direction: transfer.direction,
    transfer_id: transfer.id,
    blob_id: transfer.blob_id ?? undefined,
    suggestedName: transfer.name,
  });
  if (!file) return null;
  return crewHttp<CrewTransfer>(`/transfers/${encodeURIComponent(transfer.id)}/resume`, 'POST', {
    file_capability: file.capability_id,
  });
}
export function pauseTransfer(id: string): Promise<CrewTransfer> {
  return crewHttp(`/transfers/${encodeURIComponent(id)}/pause`, 'POST', {});
}
export async function forgetTransfer(id: string): Promise<unknown> {
  const transfer = await crewHttp<CrewTransfer>(`/transfers/${encodeURIComponent(id)}`);
  if (
    transfer.direction === 'download' &&
    transfer.state !== 'completed' &&
    transfer.destination_identity
  ) {
    const file = await chooseTransferFile({
      connection_id: transfer.connection_id,
      channel_id: transfer.channel_id,
      direction: 'download',
      purpose: 'cleanup',
      transfer_id: transfer.id,
      blob_id: transfer.blob_id ?? undefined,
      suggestedName: transfer.name,
    });
    if (!file) return null;
    return crewHttp(`/transfers/${encodeURIComponent(id)}`, 'DELETE', {
      file_capability: file.capability_id,
    });
  }
  return crewHttp(`/transfers/${encodeURIComponent(id)}`, 'DELETE');
}
export async function clearPublishedTransfers(
  connectionId: string,
  blobIds: string[]
): Promise<void> {
  const transfers = await listTransfers(connectionId);
  for (const transfer of transfers) {
    if (
      transfer.direction === 'upload' &&
      transfer.state === 'completed' &&
      transfer.blob_id &&
      blobIds.includes(transfer.blob_id)
    )
      await forgetTransfer(transfer.id);
  }
}

export async function previewAttachment(
  connectionId: string,
  channelId: string,
  blobId: string
): Promise<Blob> {
  const { client } = await import('../../api/client.gen');
  const { userActionHeaders } = await import('../../utils/userAction');
  const config = client.getConfig();
  const headers = new Headers(config.headers as HeadersInit);
  headers.set('X-Secret-Key', await window.electron.getSecretKey());
  Object.entries(await userActionHeaders()).forEach(([key, value]) => headers.set(key, value));
  headers.set('Content-Type', 'application/json');
  const response = await fetch(`${config.baseUrl ?? ''}/crew/transfers/preview`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ connection_id: connectionId, channel_id: channelId, blob_id: blobId }),
    cache: 'no-store',
  });
  if (!response.ok) {
    const error = await response.json().catch(() => null);
    throw new Error(error?.error || 'Image preview was refused');
  }
  return response.blob();
}
