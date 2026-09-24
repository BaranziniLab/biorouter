import { useEffect, useState } from 'react';
import { CopyField } from '../../ui/copy-field';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Link } from '../../icons/app-icons';
import { crewRequest } from '../crewApi';
import { filesCopy } from './copy';
import './files.css';

/** `reference.get`: a path on the server, shared by name only. */
export interface CrewReference {
  path: string;
  label: string;
  verified: boolean;
}

const failureText = (failure: unknown) =>
  failure instanceof Error && failure.message ? failure.message : filesCopy.detailsFailed;

/**
 * A server path shared in a message: the `Link` glyph, its label, the path in a compact
 * `CopyField` (the whole path is copied, however it is truncated), and a muted "Not uploaded"
 * whose tooltip says what sharing a path does not do. Crew never checks the path or grants
 * access to it, so nothing here claims it does.
 */
export function ServerPathRow({
  connectionId,
  referenceId,
}: {
  connectionId: string;
  referenceId: string;
}) {
  const [reference, setReference] = useState<CrewReference | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    setReference(null);
    setError('');
    void crewRequest<CrewReference>(connectionId, 'reference.get', { reference_id: referenceId })
      .then((item) => {
        if (active) setReference(item);
      })
      .catch((failure: unknown) => {
        if (active) setError(failureText(failure));
      });
    return () => {
      active = false;
    };
  }, [connectionId, referenceId]);

  const label = reference?.label || reference?.path || filesCopy.serverPathLoading;
  return (
    <div className="crew-server-path">
      <div className="crew-server-path-row">
        <Link className="crew-attachment-icon" aria-hidden />
        <span className="crew-attachment-name">{label}</span>
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="crew-server-path-note" tabIndex={0}>
              {filesCopy.notUploaded}
            </span>
          </TooltipTrigger>
          <TooltipContent>{filesCopy.notUploadedHelp}</TooltipContent>
        </Tooltip>
      </div>
      {reference ? (
        <CopyField
          value={reference.path}
          label={filesCopy.serverPath}
          truncate="middle"
          className="crew-server-path-field"
        />
      ) : null}
      {error ? <p className="crew-file-row-error">{error}</p> : null}
    </div>
  );
}
