import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { AlertCircle, AlertTriangle, CheckCircle2 } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import { accessCopy } from './copy';
import type { RevokeOutcome } from './useCrewGrants';

/**
 * The inline two-step confirmation (ui-redesign-spec, "Dialogs and confirmations": "Revoke chat
 * access — inline two-step"). No modal over the pane: the question replaces the control that asked
 * it, focus lands on the safe choice, and Escape keeps things as they were.
 */
export function InlineConfirm({
  question,
  detail,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onCancel,
  pending = false,
  className,
}: {
  question: string;
  detail?: string;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm(): void;
  onCancel(): void;
  pending?: boolean;
  className?: string;
}) {
  const questionId = useId();
  const keep = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    keep.current?.focus();
  }, []);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Escape') return;
    event.stopPropagation();
    onCancel();
  };
  return (
    <div
      role="group"
      aria-labelledby={questionId}
      className={cn('flex flex-col gap-2', className)}
      onKeyDown={onKeyDown}
      data-testid="crew-access-inline-confirm"
    >
      <p id={questionId} className="text-label text-text-default">
        {question}
      </p>
      {detail ? <p className="text-supporting text-text-muted">{detail}</p> : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="destructive"
          size="sm"
          disabled={pending}
          onClick={onConfirm}
        >
          {confirmLabel}
        </Button>
        <Button ref={keep} type="button" variant="ghost" size="sm" onClick={onCancel}>
          {cancelLabel}
        </Button>
      </div>
    </div>
  );
}

/**
 * The workspace confirmed a revoke this view saw waiting for it (F3): the daemon asked again by
 * itself once the connection was back. A status, not an alert — nothing is wrong any more. With
 * `onDismiss` it carries its own Dismiss: it stays until the person dismisses it or leaves the
 * surface that shows it (final polish NEW-4).
 */
export function RevocationConfirmedNote({
  className,
  onDismiss,
}: {
  className?: string;
  onDismiss?: () => void;
}) {
  return (
    <Note
      tone="success"
      icon={CheckCircle2}
      role="status"
      className={className}
      testId="crew-access-confirmed"
      action={
        onDismiss ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label={accessCopy.confirmedDismissName}
            onClick={onDismiss}
          >
            {accessCopy.confirmedDismiss}
          </Button>
        ) : undefined
      }
    >
      {accessCopy.confirmed}
    </Note>
  );
}

/**
 * What a view saw of each revoke, per scope: `waited` once it saw the revoke waiting for the
 * workspace, `dismissed` once the person dismissed its "Confirmed." note. Keyed by the scope object
 * the caller names — the Chat access pane's intent — so it outlives a remount of the same surface,
 * and goes with it: a closed pane, or one opened anew, is a new intent and starts over.
 */
const confirmationMarks = new WeakMap<object, Map<string, 'waited' | 'dismissed'>>();

function marksFor(scope: object): Map<string, 'waited' | 'dismissed'> {
  let marks = confirmationMarks.get(scope);
  if (!marks) {
    marks = new Map();
    confirmationMarks.set(scope, marks);
  }
  return marks;
}

export interface ConfirmedAfterWait {
  /** Show "Confirmed.": this view saw the revoke waiting, and the daemon now says confirmed. */
  shown: boolean;
  /** The person dismissed the note: it does not come back for this revoke in this scope. */
  dismiss(): void;
}

/**
 * Whether a revoke this view saw waiting for the workspace has since been confirmed (F3): shown
 * from the moment `unconfirmed` turns false while `confirmed` holds, for the same `key` (a grant's
 * run), until the person dismisses it. A new key starts over, so a revoke confirmed at once (a 200)
 * never shows it.
 *
 * `scope` is what the memory lives as long as (final polish NEW-4). Live, "Confirmed." was on
 * screen for seven seconds: when Crew reconnected, the offline screen gave way to the channel, the
 * pane's body mounted again, and component state forgot it had ever waited. The Chat access pane
 * passes its pane intent, which lives across that remount and ends when the person closes the pane
 * or opens another. Without a scope the memory is this mount's own.
 */
export function useConfirmedAfterWait(
  key: string | null,
  unconfirmed: boolean,
  confirmed: boolean,
  scope?: object | null
): ConfirmedAfterWait {
  const [own] = useState<object>(() => ({}));
  const holder = scope ?? own;
  const [, changed] = useState(0);
  useEffect(() => {
    if (!key || !unconfirmed) return;
    const marks = marksFor(holder);
    if (marks.get(key) === 'waited') return;
    marks.set(key, 'waited');
    changed((value) => value + 1);
  }, [key, unconfirmed, holder]);
  const dismiss = useCallback(() => {
    if (!key) return;
    marksFor(holder).set(key, 'dismissed');
    changed((value) => value + 1);
  }, [key, holder]);
  const mark = key ? confirmationMarks.get(holder)?.get(key) : undefined;
  return { shown: Boolean(key) && mark === 'waited' && !unconfirmed && confirmed, dismiss };
}

/**
 * What a revoke came to, said once, where the person asked for it. Success is shown only for a
 * confirmed revoke (a 200); a 503 is "stopped on this device"; every other answer is "not revoked"
 * followed by the daemon's own words, and both failures offer Retry.
 */
export function RevokeResultNote({
  outcome,
  chat,
  onRetry,
  retrying = false,
  successActions,
  confirmation = 'offline',
  className,
}: {
  outcome: RevokeOutcome;
  /** The chat's title for the success sentence, or `null`. */
  chat: string | null;
  onRetry(): void;
  retrying?: boolean;
  /** Controls after a confirmed revoke (Open chat · Done). */
  successActions?: ReactNode;
  /**
   * For a revoke that stopped only on this device: whether its connection is back, so the daemon
   * is asking the workspace again by itself (`confirming`, F3), or not yet (`offline`). Retry
   * stays either way: it asks now.
   */
  confirmation?: 'confirming' | 'offline';
  className?: string;
}) {
  if (outcome.kind === 'revoked') {
    return (
      <Note
        tone="success"
        icon={CheckCircle2}
        role="status"
        className={className}
        testId="crew-access-revoked"
      >
        <p>{accessCopy.revoked(chat)}</p>
        {successActions ? <div className="mt-2 flex flex-wrap gap-2">{successActions}</div> : null}
      </Note>
    );
  }
  const retry = (
    <Button type="button" variant="ghost" size="sm" disabled={retrying} onClick={onRetry}>
      {accessCopy.retry}
    </Button>
  );
  if (outcome.kind === 'unconfirmed') {
    return (
      <Note
        tone="warning"
        icon={AlertTriangle}
        role="alert"
        action={retry}
        className={className}
        testId="crew-access-unconfirmed"
      >
        <span data-confirmation={confirmation}>
          {confirmation === 'confirming' ? accessCopy.confirming : accessCopy.unconfirmed}
        </span>
      </Note>
    );
  }
  return (
    <Note
      tone="danger"
      icon={AlertCircle}
      role="alert"
      action={retry}
      className={className}
      testId="crew-access-not-revoked"
    >
      <span>{accessCopy.notRevoked}</span> <span>{outcome.message}</span>
    </Note>
  );
}
