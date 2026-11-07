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
import { focusIsLost } from '../state/focusReturn';
import type { ChatCrewAccess, ChatPersonRead } from './chatCrewAccess';

/** The daemon's answer when there was no consent to take back. */
const CREW_NOT_FOUND = 'crew_not_found';

/** Where focus goes when the last Revoke and its note are gone: the chat's own composer. */
const COMPOSER = 'textarea[data-testid="chat-input"]';

/** A consent this bar took back: which one, by its workspace and when it was given. */
interface Gone {
  connectionId: string;
  grantedAt: number | null;
}

/**
 * Whether `read` is a consent this bar took back. Only the same consent counts: one given again
 * later carries a later `grantedAt`, and a list that no longer named the workspace already dropped
 * the mark (`useEffect` below), so a lasting consent granted again in this chat always shows. A
 * list read before the revoke landed still names the old consent, and that one stays hidden.
 */
function isGone(gone: readonly Gone[], read: ChatPersonRead): boolean {
  return gone.some(
    (mark) => mark.connectionId === read.connectionId && mark.grantedAt === read.grantedAt
  );
}

/** What the bar said last: a revoke's outcome names the workspace it was about. */
interface Said {
  text: string;
  tone: 'info' | 'danger';
  /** The connection a "no longer reads" note is about; null for a failure, which stays. */
  about: string | null;
}

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
 *
 * A consent given again in the same open chat shows again, and the "no longer reads" note about
 * that workspace goes: it would contradict the row (live QA g2-05). Only the consent this bar took
 * back stays hidden, so a list read before the revoke landed cannot bring it back.
 */
export function ChatReadsBar({
  access,
  className,
}: {
  access: Pick<ChatCrewAccess, 'sessionId' | 'reads' | 'refetch'>;
  className?: string;
}) {
  const { sessionId, refetch } = access;
  const [gone, setGone] = useState<Gone[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [said, setSaid] = useState<Said | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const [focusNext, setFocusNext] = useState(0);
  const listed = access.reads;
  const reads = (listed ?? []).filter((read) => !isGone(gone, read));

  // The daemon's list no longer names a workspace whose consent was taken back: the revoke is in
  // it, and a consent given there from now on is a new one, whatever its time.
  useEffect(() => {
    if (!listed) return;
    setGone((current) => {
      const kept = current.filter((mark) =>
        listed.some((read) => read.connectionId === mark.connectionId)
      );
      return kept.length === current.length ? current : kept;
    });
  }, [listed]);

  // A consent shown again for the workspace the note said this chat no longer reads: the note is
  // stale, so it goes in the same render the row comes back in, never beside it. Focus on its
  // Dismiss moves to the row's Revoke rather than to the page.
  const shownAgain = said?.about != null && reads.some((read) => read.connectionId === said.about);
  const note = shownAgain ? null : said;
  const dismissFocused = useRef(false);
  useEffect(() => {
    if (!shownAgain) return;
    const rescue = dismissFocused.current && focusIsLost();
    dismissFocused.current = false;
    setSaid(null);
    if (rescue) setFocusNext((count) => count + 1);
  }, [shownAgain]);

  // A revoke took its button away: the next Revoke, else the note's Dismiss (once drawn).
  useEffect(() => {
    if (focusNext === 0) return;
    const next =
      root.current?.querySelector<HTMLElement>('[data-crew-read-revoke]') ??
      root.current?.querySelector<HTMLElement>('[data-crew-read-dismiss]');
    next?.focus();
  }, [focusNext]);

  if (!sessionId || (reads.length === 0 && !note)) return null;

  const revoke = async (read: ChatPersonRead) => {
    if (busy) return;
    setBusy(read.connectionId);
    setSaid(null);
    const mark: Gone = { connectionId: read.connectionId, grantedAt: read.grantedAt };
    try {
      await revokePersonRead(sessionId, read.connectionId);
      setGone((current) => [...current, mark]);
      setSaid({ text: readsCopy.revoked(read.workspace), tone: 'info', about: read.connectionId });
    } catch (failure) {
      if (crewErrorCode(failure) === CREW_NOT_FOUND) {
        setGone((current) => [...current, mark]);
        setSaid({ text: readsCopy.gone, tone: 'info', about: read.connectionId });
      } else {
        setSaid({ text: readsCopy.failed, tone: 'danger', about: null });
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
      {note ? (
        <Note
          tone={note.tone}
          role={note.tone === 'danger' ? 'alert' : 'status'}
          action={
            <Button
              type="button"
              variant="ghost"
              shape="round"
              size="xs"
              aria-label={readsCopy.dismiss}
              className="size-5"
              data-crew-read-dismiss=""
              onFocus={() => {
                dismissFocused.current = true;
              }}
              onBlur={() => {
                dismissFocused.current = false;
              }}
              onClick={dismiss}
            >
              <X aria-hidden="true" />
            </Button>
          }
        >
          {note.text}
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
