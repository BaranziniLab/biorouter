import { useCallback, useEffect, useRef, useState } from 'react';
import { crewRequest } from './crewApi';
import {
  readPendingTransfers,
  savePendingTransfer,
  removePendingTransfer,
  onPendingTransfersChanged,
  forgetAllPendingTransfers,
  type PendingCrewTransfer,
} from './crewTransfers';

interface CrewBlob {
  id: string;
  owner_id: string;
  channel_id: string;
  name: string;
  media_type: string;
  size: number;
  sha256: string;
  offset: number;
  complete: boolean;
}
const MAX_FILE_SIZE = 64 * 1024 * 1024;
const hex = (bytes: Uint8Array) =>
  Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
const digest = async (bytes: ArrayBuffer) =>
  hex(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)));

export function CrewUpload({
  connectionId,
  channelId,
  disabled,
  onReady,
  onRemoteReference,
}: {
  connectionId: string;
  channelId: string;
  disabled: boolean;
  onReady: (blob: { id: string; name: string }) => void;
  onRemoteReference: () => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<PendingCrewTransfer[]>([]);
  const [resumeKey, setResumeKey] = useState('');
  const [metadataError, setMetadataError] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const alive = useRef(true);
  const uploading = useRef(false);
  const readyCallback = useRef(onReady);
  readyCallback.current = onReady;
  const reload = useCallback(() => {
    setPending(readPendingTransfers(connectionId, channelId));
    setMetadataError(false);
  }, [connectionId, channelId]);
  useEffect(() => {
    alive.current = true;
    try {
      reload();
    } catch (error) {
      setMetadataError(true);
      setStatus(error instanceof Error ? error.message : 'Could not read pending upload metadata.');
    }
    const unsubscribe = onPendingTransfersChanged(() => {
      try {
        reload();
      } catch {
        setMetadataError(true);
      }
    });
    return () => {
      alive.current = false;
      unsubscribe();
    };
  }, [reload]);

  const recoverCompleted = async (item: PendingCrewTransfer) => {
    if (!item.blobId) {
      setResumeKey(item.beginKey);
      fileInput.current?.click();
      return;
    }
    setBusy(true);
    try {
      const blob = await crewRequest<CrewBlob>(connectionId, 'blob.status', {
        blob_id: item.blobId,
      });
      if (!alive.current) return;
      if (blob.sha256 !== item.sha256 || blob.size !== item.size || blob.channel_id !== channelId)
        throw new Error(
          'Saved transfer does not match the server record. Forget it and start again.'
        );
      if (blob.complete) {
        readyCallback.current({ id: blob.id, name: blob.name });
        setStatus('Completed upload restored to the composer. Send a message to share it.');
      } else {
        setResumeKey(item.beginKey);
        setStatus(
          'Select the original file. Its size and SHA-256 must match before upload resumes.'
        );
      }
    } catch (error) {
      if (alive.current)
        setStatus(error instanceof Error ? error.message : 'Could not resume upload.');
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  const upload = async (selected: File, selectedResumeKey = resumeKey) => {
    if (uploading.current || disabled) return;
    if (selected.size > MAX_FILE_SIZE) {
      setStatus(
        'This file is above the 64 MiB desktop transfer limit. Keep it on the cluster and choose “Share remote reference” below. No file bytes have been uploaded.'
      );
      return;
    }
    uploading.current = true;
    setFile(selected);
    setBusy(true);
    setStatus('Verifying file size and SHA-256…');
    try {
      const sha256 = await digest(await selected.arrayBuffer());
      if (!alive.current) return;
      const stored = readPendingTransfers(connectionId, channelId);
      let item = selectedResumeKey
        ? stored.find((entry) => entry.beginKey === selectedResumeKey)
        : stored.find((entry) => entry.sha256 === sha256 && entry.size === selected.size);
      if (selectedResumeKey && !item)
        throw new Error('The saved transfer is no longer available. Start a new upload.');
      if (item && (item.sha256 !== sha256 || item.size !== selected.size))
        throw new Error(
          'This is a different file. Select the original file with the matching size and SHA-256, or start a new upload.'
        );
      item ??= {
        connectionId,
        channelId,
        sha256,
        size: selected.size,
        beginKey: crypto.randomUUID(),
        updatedAt: Date.now(),
      };
      // Persist the idempotency key before admitting the first remote write.
      savePendingTransfer(item);
      reload();
      let blob = item.blobId
        ? await crewRequest<CrewBlob>(connectionId, 'blob.status', { blob_id: item.blobId })
        : await crewRequest<CrewBlob>(
            connectionId,
            'blob.begin',
            {
              channel_id: channelId,
              name: selected.name,
              media_type: selected.type || 'application/octet-stream',
              size: selected.size,
              sha256,
              idempotency_key: item.beginKey,
            },
            true
          );
      if (
        blob.sha256 !== sha256 ||
        blob.size !== selected.size ||
        blob.channel_id !== channelId ||
        blob.offset < 0 ||
        blob.offset > selected.size
      )
        throw new Error('The server transfer record does not match the selected file.');
      item = { ...item, blobId: blob.id, updatedAt: Date.now() };
      savePendingTransfer(item);
      while (blob.offset < selected.size) {
        if (!alive.current) return;
        const offset: number = blob.offset;
        const data = new Uint8Array(
          await selected.slice(offset, offset + 128 * 1024).arrayBuffer()
        );
        blob = await crewRequest<CrewBlob>(
          connectionId,
          'blob.chunk',
          { blob_id: blob.id, offset, data_hex: hex(data) },
          true
        );
        if (blob.offset <= offset || blob.offset > selected.size)
          throw new Error('The server returned an invalid upload offset.');
        if (alive.current)
          setStatus(
            `Uploading ${selected.name} · ${Math.round((blob.offset / selected.size) * 100)}%`
          );
      }
      if (!blob.complete)
        blob = await crewRequest<CrewBlob>(
          connectionId,
          'blob.finish',
          { blob_id: blob.id, idempotency_key: `${item.beginKey}-finish` },
          true
        );
      if (!blob.complete) throw new Error('The server did not confirm the file commit.');
      if (alive.current) {
        readyCallback.current({ id: blob.id, name: blob.name });
        setStatus('Upload verified. Send a message to share it.');
        setFile(null);
        setResumeKey('');
        reload();
      }
    } catch (error) {
      if (alive.current) {
        setStatus(
          error instanceof Error ? error.message : 'Upload interrupted. Reconnect and resume.'
        );
        try {
          reload();
        } catch {
          /* The original storage failure is already visible. */
        }
      }
    } finally {
      uploading.current = false;
      if (alive.current) setBusy(false);
    }
  };
  return (
    <div
      className="crew-upload"
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault();
        if (!disabled && !busy && event.dataTransfer.files[0]) {
          setResumeKey('');
          void upload(event.dataTransfer.files[0], '');
        }
      }}
    >
      <input
        ref={fileInput}
        aria-label="Choose file for Crew upload"
        type="file"
        hidden
        onChange={(event) => {
          const selected = event.target.files?.[0];
          if (selected) void upload(selected);
          event.target.value = '';
        }}
      />
      <button
        type="button"
        className="crew-button"
        disabled={disabled || busy}
        onClick={() => {
          setResumeKey('');
          fileInput.current?.click();
        }}
      >
        {busy ? 'Uploading…' : 'Attach file'}
      </button>
      <button
        type="button"
        className="crew-button"
        disabled={disabled || busy}
        onClick={onRemoteReference}
      >
        Share remote reference
      </button>
      {status && (
        <span role="status" className="crew-small">
          {status}
        </span>
      )}
      {resumeKey && !busy && (
        <button
          type="button"
          className="crew-button"
          disabled={disabled}
          onClick={() => fileInput.current?.click()}
        >
          Choose original file to resume
        </button>
      )}
      {file && !busy && (
        <button
          type="button"
          className="crew-button"
          disabled={disabled}
          onClick={() => void upload(file)}
        >
          Resume selected file
        </button>
      )}
      {metadataError && (
        <button
          type="button"
          className="crew-button"
          disabled={busy}
          onClick={() => {
            try {
              forgetAllPendingTransfers();
              setStatus('Saved transfer records cleared. Remote files were not deleted.');
            } catch (error) {
              setStatus(
                error instanceof Error ? error.message : 'Could not clear transfer metadata.'
              );
            }
          }}
        >
          Reset saved transfer records in this profile
        </button>
      )}
      {pending.length > 0 && (
        <details className="crew-pending">
          <summary>
            {pending.length} saved transfer{pending.length === 1 ? '' : 's'} · resume or restore
          </summary>
          <p className="crew-small">
            Only transfer IDs, size, and SHA-256 are saved in this app profile. File bytes, local
            paths, credentials, and conversation history are not saved here.
          </p>
          {pending.map((item) => (
            <div className="crew-inline" key={item.beginKey}>
              <span className="crew-small">
                {item.size.toLocaleString()} bytes · SHA-256 {item.sha256.slice(0, 12)}…
              </span>
              <button
                type="button"
                className="crew-button"
                disabled={disabled || busy}
                onClick={() => void recoverCompleted(item)}
              >
                Resume / restore
              </button>
              <button
                type="button"
                className="crew-button"
                disabled={busy}
                onClick={() => {
                  try {
                    removePendingTransfer(item.beginKey);
                    reload();
                    if (resumeKey === item.beginKey) setResumeKey('');
                  } catch (error) {
                    setStatus(
                      error instanceof Error ? error.message : 'Could not forget transfer.'
                    );
                  }
                }}
              >
                Forget local record
              </button>
            </div>
          ))}
        </details>
      )}
    </div>
  );
}

export function CrewAttachment({ connectionId, blobId }: { connectionId: string; blobId: string }) {
  const [metadata, setMetadata] = useState<CrewBlob | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState('');
  const url = useRef('');
  const mounted = useRef(true);
  const partial = useRef<{ parts: Uint8Array<ArrayBuffer>[]; offset: number }>({
    parts: [],
    offset: 0,
  });
  useEffect(() => {
    let active = true;
    mounted.current = true;
    void crewRequest<CrewBlob>(connectionId, 'blob.status', { blob_id: blobId })
      .then((result) => {
        if (active) setMetadata(result);
      })
      .catch((error: Error) => {
        if (active) setStatus(error.message);
      });
    return () => {
      active = false;
      mounted.current = false;
      partial.current = { parts: [], offset: 0 };
      URL.revokeObjectURL(url.current);
    };
  }, [connectionId, blobId]);
  const download = async (showPreview: boolean) => {
    setBusy(true);
    setStatus('Downloading…');
    try {
      const parts = partial.current.parts;
      let offset = partial.current.offset;
      let blob: CrewBlob;
      do {
        const part = await crewRequest<{
          blob: CrewBlob;
          data_hex: string;
          next_offset: number;
          complete: boolean;
        }>(connectionId, 'blob.read', { blob_id: blobId, offset });
        if (!mounted.current) return;
        blob = part.blob;
        if (blob.size > MAX_FILE_SIZE)
          throw new Error('This file exceeds the desktop in-memory download limit of 64 MiB.');
        if (!/^(?:[0-9a-f]{2})*$/i.test(part.data_hex))
          throw new Error('The server returned invalid file data.');
        const bytes = new Uint8Array(part.data_hex.length / 2);
        for (let i = 0; i < bytes.length; i += 1)
          bytes[i] = parseInt(part.data_hex.slice(i * 2, i * 2 + 2), 16);
        if (
          part.next_offset !== offset + bytes.length ||
          (bytes.length === 0 && offset < blob.size)
        )
          throw new Error('The server returned an invalid download offset.');
        parts.push(bytes);
        offset = part.next_offset;
        partial.current = { parts, offset };
        setStatus(`Downloading · ${offset} / ${blob.size} bytes`);
      } while (offset < blob.size);
      const output = new Blob(parts, { type: blob.media_type });
      if (offset !== blob.size || (await digest(await output.arrayBuffer())) !== blob.sha256) {
        partial.current = { parts: [], offset: 0 };
        throw new Error(
          'File integrity check failed. The download was discarded; retry starts from the beginning.'
        );
      }
      if (!mounted.current) return;
      partial.current = { parts: [], offset: 0 };
      const objectUrl = URL.createObjectURL(output);
      URL.revokeObjectURL(url.current);
      url.current = objectUrl;
      if (
        showPreview &&
        ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(blob.media_type)
      )
        setPreview(objectUrl);
      else {
        const anchor = document.createElement('a');
        anchor.href = objectUrl;
        anchor.download = Array.from(blob.name, (character) =>
          character === '/' || character === '\\' || character.codePointAt(0)! < 32
            ? '_'
            : character
        ).join('');
        anchor.click();
      }
      setStatus('SHA-256 verified');
    } catch (error) {
      if (mounted.current) setStatus(error instanceof Error ? error.message : 'Download failed.');
    } finally {
      if (mounted.current) setBusy(false);
    }
  };
  return (
    <div className="crew-attachment">
      <div className="crew-inline">
        <span>
          {metadata?.name || 'Attachment'}
          {metadata && <small> · {metadata.size.toLocaleString()} bytes</small>}
        </span>
        <button className="crew-button" disabled={busy} onClick={() => void download(false)}>
          Download
        </button>
        {metadata &&
          ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(metadata.media_type) && (
            <button className="crew-button" disabled={busy} onClick={() => void download(true)}>
              Preview image
            </button>
          )}
      </div>
      {status && (
        <p role="status" className="crew-small">
          {status}
        </p>
      )}
      {preview && (
        <img
          src={preview}
          alt={metadata?.name || 'Shared image'}
          style={{ maxWidth: '100%', maxHeight: 350, objectFit: 'contain' }}
        />
      )}
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
