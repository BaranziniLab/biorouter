import type { CrewTransfer } from '../crewTransfers';
import type { CrewController } from '../state/types';
import type { CrewBlob } from './AttachmentCard';
import { filesCopy } from './copy';

/**
 * A finished upload this composer did not start, re-read from the workspace before it joins a
 * draft: its shared file must be complete, in this channel, and exactly these bytes. "Attach" under
 * Files › "Uploaded, not sent" asks this; so does a composer chip that came back after an app
 * relaunch (RES2-N9), whose upload no watch in this window started. Throws
 * `filesCopy.attachMismatch` when the file no longer matches.
 */
export async function checkedUpload(
  request: CrewController['request'],
  transfer: CrewTransfer,
  channelId: string
): Promise<{ id: string; name: string }> {
  const blob = await request<CrewBlob>('blob.status', { blob_id: transfer.blob_id });
  if (
    !blob.complete ||
    blob.channel_id !== channelId ||
    blob.sha256 !== transfer.sha256 ||
    blob.size !== transfer.size
  )
    throw new Error(filesCopy.attachMismatch);
  return { id: blob.id, name: blob.name };
}
