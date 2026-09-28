import type * as Api from '../../api/types.gen';
import { crewHttp } from './crewApi';
import { withFileWindow } from './files/fileWindows';
import { unwrapIpcError } from '../../utils/ipcError';

export type TransferDirection = Api.Direction;
/**
 * A transfer as the daemon records it (`Receipt`), without the daemon's own bookkeeping for
 * resuming and cleaning up, of which only `destination_identity` is read here. `pause_reason` is
 * `server_storage` for a transfer paused because the workspace server could not save it
 * (T3-BE-14); optional, as a daemon from before it does not send it.
 */
export type CrewTransfer = Partial<Pick<Api.Receipt, 'pause_reason'>> &
  Pick<
    Api.Receipt,
    | 'id'
    | 'request_id'
    | 'connection_id'
    | 'channel_id'
    | 'direction'
    | 'name'
    | 'size'
    | 'sha256'
    | 'offset'
    | 'blob_id'
    | 'state'
    | 'error'
    | 'destination_identity'
  >;
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
export interface FileCapability {
  capability_id: string;
  name: string;
  size?: number | null;
}
/**
 * The secure picker in the main process. A refusal comes back as the main process's own sentence:
 * Electron wraps an error thrown across `invoke` in "Error invoking remote method …", and that
 * wrapper is taken off here, once, so every surface shows what the drop path shows (FILES-F6).
 */
export async function chooseTransferFile(
  request: FileSelectionRequest
): Promise<FileCapability | null> {
  const picker = window.electron.crewSelectTransferFile;
  if (!picker)
    throw new Error(
      'This desktop build does not provide the secure Crew file picker. Update the desktop app before transferring local files.'
    );
  try {
    // Counted open until it answers, so a card told to finish it first hears when it closed.
    return await withFileWindow(() =>
      picker({
        expectedMode: request.expected_mode,
        purpose: request.purpose,
        direction: request.direction,
        connectionId: request.connection_id,
        channelId: request.channel_id,
        blobId: request.blob_id,
        transferId: request.transfer_id,
        suggestedName: request.suggestedName,
      })
    );
  } catch (error) {
    throw unwrapIpcError(error, 'The file window could not open.');
  }
}
export async function listTransfers(
  connectionId: string,
  channelId?: string
): Promise<CrewTransfer[]> {
  const query = new URLSearchParams({ connection_id: connectionId });
  if (channelId) query.set('channel_id', channelId);
  return (await crewHttp<{ transfers: CrewTransfer[] }>(`/transfers?${query}`)).transfers;
}
/**
 * Start a transfer. The file capability comes from the secure picker the main process shows, or,
 * for a file dropped or pasted into Crew (D-DROP), from the native Share / Cancel confirmation
 * the main process showed for it (`chosen`): either way from the main process, never from a path
 * the renderer names. The daemon binds the capability to the connection and channel it was
 * registered for, so `request` must name the same ones.
 */
export async function beginTransfer(
  request: FileSelectionRequest,
  chosen?: FileCapability
): Promise<CrewTransfer | null> {
  const file = chosen ?? (await chooseTransferFile(request));
  if (!file) return null;
  return crewHttp<CrewTransfer>('/transfers', 'POST', {
    request_id: crypto.randomUUID(),
    connection_id: request.connection_id,
    channel_id: request.channel_id,
    direction: request.direction,
    file_capability: file.capability_id,
    blob_id: request.blob_id,
  } satisfies Api.StartRequest);
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

/** Receipt states in which the daemon is still running the transfer and will not forget it. */
const RUNNING_STATES: readonly string[] = [
  'starting',
  'uploading',
  'publishing',
  'pause_requested',
];

/** How a cancel ended: the receipt is gone, or the upload finished before it could stop. */
export type CancelUploadOutcome = 'cancelled' | 'finished';

export interface CancelUploadOptions {
  /** How long to wait between looks while the upload stops. */
  intervalMs?: number;
  /** How many looks before giving up. */
  attempts?: number;
  wait?: (ms: number) => Promise<void>;
}

/**
 * Cancel an upload (FILES-F7): pause it if it is moving, wait until the daemon has stopped it,
 * then forget its receipt. What was already sent stays on the server as an unfinished part,
 * which the workspace removes a day after its last piece and counts toward its file space until
 * then; nothing can delete it sooner. An upload that finished before it could stop is left as it
 * is (`finished`): it is a whole file now, and the draft's own × is the way to take it out.
 */
export async function cancelUpload(
  id: string,
  {
    intervalMs = 250,
    attempts = 40,
    wait = (ms) => new Promise((r) => setTimeout(r, ms)),
  }: CancelUploadOptions = {}
): Promise<CancelUploadOutcome> {
  const path = `/transfers/${encodeURIComponent(id)}` as const;
  let receipt = await crewHttp<CrewTransfer>(path);
  if (receipt.direction !== 'upload') throw new Error('Only an upload can be cancelled.');
  if (RUNNING_STATES.includes(receipt.state)) await pauseTransfer(id);
  for (let look = 0; RUNNING_STATES.includes(receipt.state); look += 1) {
    if (look >= attempts)
      throw new Error('Crew couldn’t stop that upload yet. Try again in a moment.');
    await wait(intervalMs);
    receipt = await crewHttp<CrewTransfer>(path);
  }
  if (receipt.state === 'completed') return 'finished';
  await crewHttp(path, 'DELETE');
  return 'cancelled';
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
