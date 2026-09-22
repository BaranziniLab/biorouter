import { useCallback, useEffect, useRef, useState } from 'react';
import { crewRequest } from './crewApi';
import {
  beginTransfer,
  forgetTransfer,
  listTransfers,
  pauseTransfer,
  previewAttachment,
  resumeTransfer,
  type CrewTransfer,
} from './crewTransfers';

interface CrewBlob {
  id: string;
  channel_id: string;
  name: string;
  size: number;
  sha256: string;
  complete: boolean;
  media_type: string;
}
const activeStates = ['starting', 'uploading', 'downloading', 'publishing', 'pause_requested'];

function TransferRows({
  transfers,
  onRestore,
  onChange,
}: {
  transfers: CrewTransfer[];
  onRestore?: (transfer: CrewTransfer) => Promise<void>;
  onChange: () => Promise<void>;
}) {
  const [error, setError] = useState('');
  const act = async (operation: () => Promise<unknown>) => {
    setError('');
    try {
      await operation();
      await onChange();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Transfer operation failed');
    }
  };
  return (
    <div className="crew-pending">
      {transfers.map((transfer) => (
        <div key={transfer.id}>
          <p role="status" className="crew-small">
            {transfer.name} · {transfer.state} · {transfer.offset.toLocaleString()} /{' '}
            {transfer.size.toLocaleString()} bytes
          </p>
          {transfer.error && <p role="alert">{transfer.error}</p>}
          {activeStates.includes(transfer.state) ? (
            <button
              className="crew-button"
              onClick={() => void act(() => pauseTransfer(transfer.id))}
            >
              Pause
            </button>
          ) : (
            <>
              {transfer.state === 'completed' && onRestore && (
                <button className="crew-button" onClick={() => void act(() => onRestore(transfer))}>
                  Restore to composer
                </button>
              )}
              {!['completed', 'publication_unconfirmed'].includes(transfer.state) && (
                <button
                  className="crew-button"
                  onClick={() => void act(() => resumeTransfer(transfer))}
                >
                  Select file and resume
                </button>
              )}
              <button
                className="crew-button"
                onClick={() => void act(() => forgetTransfer(transfer.id))}
              >
                Forget receipt
              </button>
            </>
          )}
        </div>
      ))}
      {error && <p role="alert">{error}</p>}
      {transfers.length > 0 && (
        <p className="crew-small">
          Receipts contain transfer metadata, not file bytes or local paths. Forgetting a receipt
          does not delete remote attachments or published files. Unfinished downloads require
          selecting their original destination so the daemon can remove its partial before
          forgetting. Completed downloads are saved to the destination you selected.
        </p>
      )}
    </div>
  );
}

export function CrewUpload({
  expectedMode,
  connectionId,
  channelId,
  disabled,
  onReady,
  onRemoteReference,
}: {
  connectionId: string;
  channelId: string;
  expectedMode: 'private' | 'public' | undefined;
  disabled: boolean;
  onReady: (blob: { id: string; name: string }) => void;
  onRemoteReference: () => void;
}) {
  const [transfers, setTransfers] = useState<CrewTransfer[]>([]);
  const [error, setError] = useState('');
  const [choosing, setChoosing] = useState(false);
  const watched = useRef('');
  const generation = useRef(0);
  const ready = useRef(onReady);
  ready.current = onReady;
  const refresh = useCallback(async () => {
    const current = generation.current;
    const next = (await listTransfers(connectionId, channelId)).filter(
      (item) => item.direction === 'upload'
    );
    if (current !== generation.current) return;
    setTransfers(next);
    const completed = next.find(
      (item) => item.id === watched.current && item.state === 'completed'
    );
    if (completed?.blob_id) {
      watched.current = '';
      ready.current({ id: completed.blob_id, name: completed.name });
    }
  }, [connectionId, channelId]);
  useEffect(() => {
    let active = true;
    watched.current = '';
    setTransfers([]);
    const tick = () =>
      void refresh().catch((failure: Error) => {
        if (active) setError(failure.message);
      });
    tick();
    const timer = window.setInterval(tick, 2000);
    return () => {
      active = false;
      generation.current += 1;
      window.clearInterval(timer);
    };
  }, [refresh]);
  const choose = async () => {
    const current = generation.current;
    setChoosing(true);
    setError('');
    try {
      if (!expectedMode)
        throw new Error('Refresh the workspace to verify connection privacy before uploading.');
      const transfer = await beginTransfer({
        expected_mode: expectedMode,
        connection_id: connectionId,
        channel_id: channelId,
        direction: 'upload',
      });
      if (current !== generation.current) return;
      if (transfer) watched.current = transfer.id;
      await refresh();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Upload could not start');
    } finally {
      setChoosing(false);
    }
  };
  const restore = async (transfer: CrewTransfer) => {
    const current = generation.current;
    const blob = await crewRequest<CrewBlob>(connectionId, 'blob.status', {
      blob_id: transfer.blob_id,
    });
    if (
      !blob.complete ||
      blob.channel_id !== channelId ||
      blob.sha256 !== transfer.sha256 ||
      blob.size !== transfer.size
    )
      throw new Error('Attachment no longer matches the completed transfer');
    if (current === generation.current) ready.current({ id: blob.id, name: blob.name });
  };
  return (
    <div className="crew-upload">
      <button className="crew-button" disabled={disabled || choosing} onClick={() => void choose()}>
        Choose file to upload
      </button>
      <button className="crew-button" disabled={disabled} onClick={onRemoteReference}>
        Share remote reference
      </button>
      <p className="crew-small">
        Files up to 1 GiB are streamed by the daemon. Uploads continue while this panel is closed;
        pause them below. A completed upload is shared when you send its message.
      </p>
      <TransferRows transfers={transfers} onRestore={restore} onChange={refresh} />
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

export function CrewAttachment({ connectionId, blobId }: { connectionId: string; blobId: string }) {
  const [metadata, setMetadata] = useState<CrewBlob | null>(null);
  const [transfers, setTransfers] = useState<CrewTransfer[]>([]);
  const [error, setError] = useState('');
  const [choosing, setChoosing] = useState(false);
  const [preview, setPreview] = useState('');
  const previewUrl = useRef('');
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const current = generation.current;
    const next = await listTransfers(connectionId);
    if (current !== generation.current) return;
    setTransfers(next.filter((item) => item.direction === 'download' && item.blob_id === blobId));
  }, [connectionId, blobId]);
  useEffect(() => {
    let active = true;
    setMetadata(null);
    setTransfers([]);
    setPreview('');
    void crewRequest<CrewBlob>(connectionId, 'blob.status', { blob_id: blobId })
      .then((blob) => {
        if (active) setMetadata(blob);
      })
      .catch((failure: Error) => {
        if (active) setError(failure.message);
      });
    const tick = () =>
      void refresh().catch((failure: Error) => {
        if (active) setError(failure.message);
      });
    tick();
    const timer = window.setInterval(tick, 2000);
    return () => {
      active = false;
      generation.current += 1;
      URL.revokeObjectURL(previewUrl.current);
      previewUrl.current = '';
      window.clearInterval(timer);
    };
  }, [connectionId, blobId, refresh]);
  const download = async () => {
    if (!metadata) return;
    setChoosing(true);
    setError('');
    try {
      await beginTransfer({
        connection_id: connectionId,
        channel_id: metadata.channel_id,
        direction: 'download',
        blob_id: blobId,
        suggestedName: metadata.name,
      });
      await refresh();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'Download could not start');
    } finally {
      setChoosing(false);
    }
  };
  const showPreview = async () => {
    if (!metadata) return;
    const current = generation.current;
    setChoosing(true);
    setError('');
    try {
      const image = await previewAttachment(connectionId, metadata.channel_id, blobId);
      if (current !== generation.current) return;
      URL.revokeObjectURL(previewUrl.current);
      previewUrl.current = URL.createObjectURL(image);
      setPreview(previewUrl.current);
    } catch (failure) {
      if (current === generation.current)
        setError(failure instanceof Error ? failure.message : 'Preview failed');
    } finally {
      if (current === generation.current) setChoosing(false);
    }
  };
  return (
    <div className="crew-attachment">
      <span>
        {metadata?.name ?? 'Attachment'}{' '}
        {metadata && <small> · {metadata.size.toLocaleString()} bytes</small>}
      </span>
      <button
        className="crew-button"
        disabled={!metadata || choosing}
        onClick={() => void download()}
      >
        Save attachment…
      </button>
      {metadata &&
        ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(metadata.media_type) && (
          <button className="crew-button" disabled={choosing} onClick={() => void showPreview()}>
            Preview image
          </button>
        )}
      <TransferRows transfers={transfers} onChange={refresh} />
      {preview && (
        <img
          src={preview}
          alt={metadata?.name || 'Shared image'}
          style={{ maxWidth: '100%', maxHeight: 350, objectFit: 'contain' }}
        />
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

export function CrewRemoteReference({
  connectionId,
  referenceId,
}: {
  connectionId: string;
  referenceId: string;
}) {
  const [reference, setReference] = useState<{
    path: string;
    label: string;
    verified: boolean;
  } | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    void crewRequest<{ path: string; label: string; verified: boolean }>(
      connectionId,
      'reference.get',
      { reference_id: referenceId }
    )
      .then((item) => {
        if (active) setReference(item);
      })
      .catch((failure: Error) => {
        if (active) setError(failure.message);
      });
    return () => {
      active = false;
    };
  }, [connectionId, referenceId]);
  return (
    <div className="crew-attachment">
      <strong>Remote reference · {reference?.label || 'Loading…'}</strong>
      {reference && (
        <>
          <p className="crew-message-body">{reference.path}</p>
          <p className="crew-small">
            Reference only · not uploaded · existence and access not verified
          </p>
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
