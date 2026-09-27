import { useEffect, useLayoutEffect, useRef, useState } from 'react';
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
 *
 * `tabIndex`: in the timeline, the row's stops (the note and Copy) are Tab stops only while their
 * message is the active row (Q3-05); in the Files tab they always are. `CopyField` takes no
 * `tabIndex`, so every stop in the row is given the row's after each render.
 */
export function ServerPathRow({
  connectionId,
  referenceId,
  tabIndex,
}: {
  connectionId: string;
  referenceId: string;
  /** The row's Tab stop; absent, its controls are ordinary Tab stops. */
  tabIndex?: number;
}) {
  const [reference, setReference] = useState<CrewReference | null>(null);
  const [error, setError] = useState('');
  const root = useRef<HTMLDivElement>(null);
  // No dependency list: the row's stops can change on any render (the field mounts once the
  // reference loads), and setting a tab index that already holds the value costs nothing.
  // Only what was a stop to begin with is moved, and it is marked the first time it is seen, so
  // an element that is never a stop (`tabindex="-1"` of its own) never becomes one.
  useLayoutEffect(() => {
    if (tabIndex === undefined || !root.current) return;
    for (const control of root.current.querySelectorAll<HTMLElement>('button, [tabindex]')) {
      if (control.dataset.crewRowStop === undefined) {
        control.dataset.crewRowStop = control.tabIndex >= 0 ? 'true' : 'false';
      }
      if (control.dataset.crewRowStop === 'true') control.tabIndex = tabIndex;
    }
  });
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
    <div ref={root} className="crew-server-path">
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
