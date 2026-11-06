import { useEffect, useRef, useState } from 'react';
import { Eye, X } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import { crewErrorCode } from '../api/errors';
import { revokePersonRead } from '../account/api';
import { readsCopy } from '../account/copy';
import { calendarDate } from '../channel/presentation';
import type { ChatCrewAccess, ChatPersonRead } from './chatCrewAccess';

/** The daemon's answer when there was no consent to take back. */
const CREW_NOT_FOUND = 'crew_not_found';

/** Where focus goes when the last Revoke and its note are gone: the chat's own composer. */
const COMPOSER = 'textarea[data-testid="chat-input"]';

/**
 * What a chat with no Crew grant shows above its composer when it may read workspaces as the
 * person (account ownership AO-12's read consent): one slim row per workspace, "Reads lab as you",
 * with **Revoke**. The consent lasts until it is taken back, so the chat itself says it holds one
 * and where the control that ends it is, as the grant's bar does (`ChatCrewAccessBar`); before,
 * only the `/crew` pane listed it, under its own Allow (live QA).
 *
 * Revoke needs no question: nothing is lost, and the next read asks again. What it came to is said
 * in a note with Dismiss, and focus never falls to the page: it goes to the next Revoke, else the
 * note's Dismiss, and from Dismiss to the composer.
 */
export function ChatReadsBar({
  access,
  className,
}: {
  access: Pick<ChatCrewAccess, 'sessionId' | 'reads' | 'refetch'>;
  className?: string;
}) {
  const { sessionId, refetch } = access;
  const [gone, setGone] = useState<string[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [said, setSaid] = useState<{ text: string; tone: 'info' | 'danger' } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const [focusNext, setFocusNext] = useState(0);
  const reads = (access.reads ?? []).filter((read) => !gone.includes(read.connectionId));

  // A revoke took its button away: the next Revoke, else the note's Dismiss (once drawn).
  useEffect(() => {
    if (focusNext === 0) return;
    const next =
      root.current?.querySelector<HTMLElement>('[data-crew-read-revoke]') ??
      root.current?.querySelector<HTMLElement>('[data-crew-read-dismiss]');
    next?.focus();
  }, [focusNext]);

  if (!sessionId || (reads.length === 0 && !said)) return null;

  const revoke = async (read: ChatPersonRead) => {
    if (busy) return;
    setBusy(read.connectionId);
    setSaid(null);
    try {
      await revokePersonRead(sessionId, read.connectionId);
      setGone((current) => [...current, read.connectionId]);
      setSaid({ text: readsCopy.revoked(read.workspace), tone: 'info' });
    } catch (failure) {
      if (crewErrorCode(failure) === CREW_NOT_FOUND) {
        setGone((current) => [...current, read.connectionId]);
        setSaid({ text: readsCopy.gone, tone: 'info' });
      } else {
        setSaid({ text: readsCopy.failed, tone: 'danger' });
      }
    } finally {
      setBusy(null);
      setFocusNext((count) => count + 1);
      refetch();
    }
  };

  const dismiss = () => {
    setSaid(null);
    window.setTimeout(() => document.querySelector<HTMLElement>(COMPOSER)?.focus(), 0);
  };

  return (
    <div
      ref={root}
      className={cn('flex flex-col gap-2', className)}
      data-testid="crew-chat-reads-bar"
    >
      {said ? (
        <Note
          tone={said.tone}
          role={said.tone === 'danger' ? 'alert' : 'status'}
          action={
            <Button
              type="button"
              variant="ghost"
              shape="round"
              size="xs"
              aria-label={readsCopy.dismiss}
              className="size-5"
              data-crew-read-dismiss=""
              onClick={dismiss}
            >
              <X aria-hidden="true" />
            </Button>
          }
        >
          {said.text}
        </Note>
      ) : null}
      {reads.map((read) => (
        <div key={read.connectionId} className="flex flex-wrap items-center gap-2">
          <Badge
            variant="chip"
            title={
              read.grantedAt !== null ? readsCopy.since(calendarDate(read.grantedAt)) : undefined
            }
          >
            <Eye className="h-3.5 w-3.5" aria-hidden />
            <bdi>{readsCopy.reads(read.workspace)}</bdi>
          </Badge>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            aria-label={readsCopy.revokeLabel(read.workspace)}
            disabled={busy !== null}
            data-crew-read-revoke=""
            onClick={() => void revoke(read)}
          >
            {readsCopy.revoke}
          </Button>
        </div>
      ))}
    </div>
  );
}
