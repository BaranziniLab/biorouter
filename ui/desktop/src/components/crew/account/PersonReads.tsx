import { useEffect, useMemo, useState } from 'react';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { crewErrorCode } from '../api/errors';
import { connectionNames, identityCopy } from '../identity';
import { calendarDate } from '../channel/presentation';
import { useCrew } from '../state/CrewControllerContext';
import { listPersonReads, revokePersonRead, type CrewPersonRead } from './api';
import { readsCopy } from './copy';

/** The daemon's answer when there was no consent to take back. */
const CREW_NOT_FOUND = 'crew_not_found';

/**
 * The workspaces a chat may read as the person (account ownership AO-12's read consent), in the
 * chat access pane: "Reads lab as you", each with Revoke. A chat without Crew access reads only
 * after the person allowed it once per workspace; this is where that permission is taken back.
 * Nothing is shown while there is none. Lists that cannot be read are left out quietly: the
 * consent is kept on this computer, and `biorouter crew grants list` shows the same.
 */
export function PersonReads({ sessionId }: { sessionId: string | null }) {
  const { connections } = useCrew();
  const ids = useMemo(() => connections.map((item) => item.id), [connections]);
  const idsKey = ids.join('\n');
  const names = useMemo(() => connectionNames(connections), [connections]);
  const [reads, setReads] = useState<CrewPersonRead[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [said, setSaid] = useState<{ text: string; tone: 'info' | 'danger' } | null>(null);

  useEffect(() => {
    setSaid(null);
    if (!sessionId || !idsKey) {
      setReads([]);
      return;
    }
    const controller = new AbortController();
    void Promise.allSettled(
      idsKey.split('\n').map((id) => listPersonReads(id, controller.signal))
    ).then((results) => {
      if (controller.signal.aborted) return;
      setReads(
        results
          .flatMap((result) => (result.status === 'fulfilled' ? result.value : []))
          .filter((read) => read.sessionId === sessionId)
      );
    });
    return () => controller.abort();
  }, [sessionId, idsKey]);

  if (!sessionId || (reads.length === 0 && !said)) return null;
  const workspaceOf = (read: CrewPersonRead) =>
    names.get(read.connectionId) || identityCopy.unnamedWorkspace;

  const revoke = async (read: CrewPersonRead) => {
    if (busy) return;
    setBusy(read.connectionId);
    setSaid(null);
    const workspace = workspaceOf(read);
    try {
      await revokePersonRead(read.sessionId, read.connectionId);
      setReads((current) => current.filter((item) => item.connectionId !== read.connectionId));
      setSaid({ text: readsCopy.revoked(workspace), tone: 'info' });
    } catch (failure) {
      if (crewErrorCode(failure) === CREW_NOT_FOUND) {
        setReads((current) => current.filter((item) => item.connectionId !== read.connectionId));
        setSaid({ text: readsCopy.gone, tone: 'info' });
      } else {
        setSaid({ text: readsCopy.failed, tone: 'danger' });
      }
    } finally {
      setBusy(null);
    }
  };

  return (
    <section
      className="flex min-w-0 flex-col gap-1.5 pt-3"
      aria-label={readsCopy.heading}
      data-testid="crew-person-reads"
    >
      {reads.length > 0 ? (
        <>
          <h3 className="text-caps text-text-muted">{readsCopy.heading}</h3>
          <ul className="biorouter-settings-list">
            {reads.map((read) => {
              const workspace = workspaceOf(read);
              return (
                <li
                  key={read.connectionId}
                  className="biorouter-settings-row flex min-w-0 items-center justify-between gap-2 px-3 py-2"
                >
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className="min-w-0 break-words text-secondary text-text-default">
                      {readsCopy.reads(workspace)}
                    </span>
                    {read.grantedAt !== null ? (
                      <span className="text-supporting text-text-muted">
                        {readsCopy.since(calendarDate(read.grantedAt))}
                      </span>
                    ) : null}
                  </span>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    aria-label={readsCopy.revokeLabel(workspace)}
                    disabled={busy !== null}
                    onClick={() => void revoke(read)}
                  >
                    {readsCopy.revoke}
                  </Button>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
      {said ? (
        <Note tone={said.tone} role={said.tone === 'danger' ? 'alert' : 'status'}>
          {said.text}
        </Note>
      ) : null}
    </section>
  );
}
