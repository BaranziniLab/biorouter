import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
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
 * itself once the connection was back. A status, not an alert — nothing is wrong any more.
 */
export function RevocationConfirmedNote({ className }: { className?: string }) {
  return (
    <Note
      tone="success"
      icon={CheckCircle2}
      role="status"
      className={className}
      testId="crew-access-confirmed"
    >
      {accessCopy.confirmed}
    </Note>
  );
}

/**
 * Whether a revoke this view saw waiting for the workspace has since been confirmed (F3): true
 * from the moment `unconfirmed` turns false while `confirmed` holds, for the same `key` (a grant's
 * run). A new key starts over, so a revoke confirmed at once (a 200) never shows it.
 */
export function useConfirmedAfterWait(
  key: string | null,
  unconfirmed: boolean,
  confirmed: boolean
): boolean {
  const [waitedFor, setWaitedFor] = useState<string | null>(null);
  useEffect(() => {
    if (key && unconfirmed) setWaitedFor(key);
  }, [key, unconfirmed]);
  return Boolean(key) && waitedFor === key && !unconfirmed && confirmed;
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
