export interface PendingCrewTransfer {
  connectionId: string;
  channelId: string;
  sha256: string;
  size: number;
  beginKey: string;
  blobId?: string;
  updatedAt: number;
}

const KEY = 'biorouter:crew:pending-transfers:v1';
const MAX_PENDING = 32;
const CHANGE_EVENT = 'biorouter:crew-transfers-changed';

export function onPendingTransfersChanged(callback: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, callback);
  return () => window.removeEventListener(CHANGE_EVENT, callback);
}

export function forgetAllPendingTransfers(): void {
  window.localStorage.removeItem(KEY);
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

export function readPendingTransfers(
  connectionId: string,
  channelId: string
): PendingCrewTransfer[] {
  return readAll().filter(
    (item) => item.connectionId === connectionId && item.channelId === channelId
  );
}

function readAll(): PendingCrewTransfer[] {
  const value: unknown = JSON.parse(window.localStorage.getItem(KEY) || '[]');
  if (!Array.isArray(value) || value.length > MAX_PENDING)
    throw new Error(
      'The saved Crew transfer list is invalid. Remove pending transfers in Crew before starting another upload.'
    );
  const transfers: PendingCrewTransfer[] = [];
  for (const [index, item] of value.entries()) {
    const valid =
      item !== null &&
      typeof item === 'object' &&
      typeof item.connectionId === 'string' &&
      item.connectionId.length <= 128 &&
      typeof item.channelId === 'string' &&
      item.channelId.length <= 128 &&
      typeof item.sha256 === 'string' &&
      /^[a-f0-9]{64}$/.test(item.sha256) &&
      Number.isSafeInteger(item.size) &&
      item.size >= 0 &&
      item.size <= 64 * 1024 * 1024 &&
      typeof item.beginKey === 'string' &&
      /^[a-zA-Z0-9-]{1,128}$/.test(item.beginKey) &&
      (item.blobId === undefined ||
        (typeof item.blobId === 'string' && item.blobId.length <= 128)) &&
      Number.isSafeInteger(item.updatedAt);
    if (
      !valid ||
      Object.keys(item).some(
        (key) =>
          ![
            'connectionId',
            'channelId',
            'sha256',
            'size',
            'beginKey',
            'blobId',
            'updatedAt',
          ].includes(key)
      )
    ) {
      throw new Error(
        `Saved Crew transfer ${index + 1} has an invalid metadata shape. Reset saved transfer records in this profile to continue.`
      );
    }
    transfers.push(item as PendingCrewTransfer);
  }
  return transfers;
}

export function savePendingTransfer(transfer: PendingCrewTransfer): void {
  const entries = readAll().filter((item) => item.beginKey !== transfer.beginKey);
  if (entries.length >= MAX_PENDING)
    throw new Error(
      '32 uploads are pending. Resume or forget a pending upload before starting another.'
    );
  entries.push(transfer);
  window.localStorage.setItem(KEY, JSON.stringify(entries));
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

export function removePendingTransfer(beginKey: string): void {
  window.localStorage.setItem(
    KEY,
    JSON.stringify(readAll().filter((item) => item.beginKey !== beginKey))
  );
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

export function clearPublishedTransfers(connectionId: string, blobIds: string[]): void {
  window.localStorage.setItem(
    KEY,
    JSON.stringify(
      readAll().filter(
        (item) =>
          item.connectionId !== connectionId || !item.blobId || !blobIds.includes(item.blobId)
      )
    )
  );
  window.dispatchEvent(new Event(CHANGE_EVENT));
}
