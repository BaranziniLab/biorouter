import { useRef, useState } from 'react';
import { AlertCircle, Bot, MessageSquare } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import { accessStatusTone, splitAccessRows, type AccessRow } from './accessRows';
import { accessCopy } from './copy';
import { InlineConfirm, RevokeResultNote } from './RevokeControls';
import type { GrantsStatus, RevokeOutcome } from './useCrewGrants';
import '../crew-app.css';

export interface AccessListProps {
  rows: readonly AccessRow[];
  status: GrantsStatus;
  /** The list failure to show, with Retry. */
  error?: string | null;
  onRetryLoad(): void;
  /** The empty-state sentence ("None of your chats can post in #methods yet."). */
  emptyText: string;
  /** Open the chat or the task's conversation. */
  onOpen(row: AccessRow): void;
  /** Revoke a chat's grant (or retry a revoke that stopped only on this device). Never throws. */
  onRevoke(row: AccessRow): Promise<RevokeOutcome>;
  /** Stop a task through the existing cancel route. Never throws. */
  onStop(row: AccessRow): Promise<void>;
  /** Layout only. */
  className?: string;
}

type Confirming = { key: string; action: 'revoke' | 'stop' } | null;

/**
 * The list of chats and tasks that can read or post (ui-redesign-spec, "Revoke", "Access rows"):
 * the Access tab's, and Workspace settings → Agent access's.
 *
 * A row: the chat's title (or "Your task · 1:16 PM · Please work out…", so two tasks can be told
 * apart), `#destination (+n)`, a status badge ("Ended" once a task's access ended with it),
 * **Open**, and a visible **Revoke** on an active chat row or **Stop** on a running task row. There is no `⋯`: its
 * only item was "Copy session ID", a machine ID with no use to the person reading the list (live QA
 * round 1, T-55). Revoked, expired and ended rows collapse under "Show past access (n)". Revoke and
 * Stop each ask inline first; what a revoke came to is said once, above the list.
 *
 * The row's line wraps (`.crew-access-row-*` in `crew-app.css`, authored rather than utilities):
 * the badge and the actions share one end group, which moves to a line of its own whenever the
 * title and destination could not keep about 160px, as in the 360px details pane.
 */
export function AccessList({
  rows,
  status,
  error,
  onRetryLoad,
  emptyText,
  onOpen,
  onRevoke,
  onStop,
  className,
}: AccessListProps) {
  const [confirming, setConfirming] = useState<Confirming>(null);
  const [pendingKey, setPendingKey] = useState<string | null>(null);
  const [result, setResult] = useState<{ row: AccessRow; outcome: RevokeOutcome } | null>(null);
  const triggers = useRef(new Map<string, HTMLButtonElement>());
  const { current, old } = splitAccessRows(rows);

  const restoreFocus = (key: string) =>
    window.setTimeout(() => triggers.current.get(key)?.focus(), 0);

  const revoke = async (row: AccessRow) => {
    setConfirming(null);
    setPendingKey(row.key);
    const outcome = await onRevoke(row);
    setPendingKey(null);
    setResult({ row, outcome });
  };

  const stop = async (row: AccessRow) => {
    setConfirming(null);
    setPendingKey(row.key);
    await onStop(row);
    setPendingKey(null);
  };

  const renderRow = (row: AccessRow) => {
    const Icon = row.kind === 'task' ? Bot : MessageSquare;
    const busy = pendingKey === row.key;
    const confirmingThis = confirming?.key === row.key ? confirming.action : null;
    return (
      <li
        key={row.key}
        className="biorouter-list-row flex flex-col gap-2 px-3 py-2"
        data-testid="crew-access-row"
        data-access-status={row.status}
        data-access-kind={row.kind}
      >
        <div className="crew-access-row-line">
          <Icon className="h-4 w-4 shrink-0 text-text-muted" aria-hidden />
          <div className="crew-access-row-text">
            <p className="truncate text-label text-text-default">
              <bdi>{row.title}</bdi>
              {row.detail ? (
                <span className="text-text-muted">
                  {accessCopy.agentsSeparator}
                  <bdi>{row.detail}</bdi>
                </span>
              ) : null}
            </p>
            <p className="truncate text-supporting text-text-muted">
              <bdi>{row.destination}</bdi>
              {row.extraSources > 0 ? (
                <>
                  {' '}
                  <span aria-hidden>{accessCopy.moreSources(row.extraSources)}</span>
                  <span className="sr-only">{accessCopy.moreSourcesName(row.extraSources)}</span>
                </>
              ) : null}
            </p>
          </div>
          <div className="crew-access-row-end">
            <Badge tone={accessStatusTone(row.status)}>{row.statusLabel}</Badge>
            {confirmingThis ? null : (
              <div className="crew-access-row-actions">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  aria-label={
                    row.kind === 'task' ? accessCopy.openTaskName : accessCopy.openName(row.title)
                  }
                  onClick={() => onOpen(row)}
                >
                  {accessCopy.open}
                </Button>
                {row.canRevoke ? (
                  <Button
                    ref={(node) => {
                      if (node) triggers.current.set(row.key, node);
                    }}
                    type="button"
                    variant="destructive"
                    size="sm"
                    disabled={busy}
                    aria-label={accessCopy.revokeRowName(row.title)}
                    onClick={() => setConfirming({ key: row.key, action: 'revoke' })}
                  >
                    {accessCopy.revokeRow}
                  </Button>
                ) : null}
                {row.canRetry ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    aria-label={accessCopy.retryRowName(
                      row.detail
                        ? `${row.title}${accessCopy.agentsSeparator}${row.detail}`
                        : row.title
                    )}
                    onClick={() => void revoke(row)}
                  >
                    {accessCopy.retry}
                  </Button>
                ) : null}
                {row.canStop ? (
                  <Button
                    ref={(node) => {
                      if (node) triggers.current.set(row.key, node);
                    }}
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    aria-label={accessCopy.stopRowName}
                    onClick={() => setConfirming({ key: row.key, action: 'stop' })}
                  >
                    {accessCopy.stopRow}
                  </Button>
                ) : null}
              </div>
            )}
          </div>
        </div>
        {confirmingThis === 'revoke' ? (
          <InlineConfirm
            question={accessCopy.confirm(row.chatTitle, row.destination)}
            confirmLabel={accessCopy.confirmRevoke}
            cancelLabel={accessCopy.confirmKeep}
            pending={busy}
            onConfirm={() => void revoke(row)}
            onCancel={() => {
              setConfirming(null);
              restoreFocus(row.key);
            }}
          />
        ) : null}
        {confirmingThis === 'stop' ? (
          <InlineConfirm
            question={accessCopy.stopConfirm}
            detail={accessCopy.stopConfirmBody}
            confirmLabel={accessCopy.stopConfirmAction}
            cancelLabel={accessCopy.stopKeep}
            pending={busy}
            onConfirm={() => void stop(row)}
            onCancel={() => {
              setConfirming(null);
              restoreFocus(row.key);
            }}
          />
        ) : null}
      </li>
    );
  };

  let body;
  if (status === 'loading' || status === 'idle') {
    body = (
      <p role="status" className="px-3 py-2 text-supporting text-text-muted">
        {accessCopy.listLoading}
      </p>
    );
  } else if (status === 'failed') {
    body = null;
  } else if (rows.length === 0) {
    body = <EmptyAccess text={emptyText} />;
  } else {
    body = (
      <>
        {current.length > 0 ? (
          <ul className="biorouter-list-shell">{current.map(renderRow)}</ul>
        ) : (
          <EmptyAccess text={emptyText} />
        )}
        {old.length > 0 ? (
          <Disclosure label={accessCopy.showOld(old.length)}>
            <ul className="biorouter-list-shell" aria-label={accessCopy.oldListName}>
              {old.map(renderRow)}
            </ul>
          </Disclosure>
        ) : null}
      </>
    );
  }

  return (
    <div className={cn('flex flex-col gap-2', className)} data-testid="crew-access-list">
      {error ? (
        <Note
          tone="warning"
          icon={AlertCircle}
          role="alert"
          action={
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={accessCopy.listRetryName}
              onClick={onRetryLoad}
            >
              {accessCopy.retry}
            </Button>
          }
        >
          {error}
        </Note>
      ) : null}
      {result ? (
        <RevokeResultNote
          outcome={result.outcome}
          chat={result.row.chatTitle}
          retrying={pendingKey === result.row.key}
          onRetry={() => void revoke(result.row)}
          successActions={
            <Button type="button" variant="ghost" size="sm" onClick={() => setResult(null)}>
              {accessCopy.done}
            </Button>
          }
        />
      ) : null}
      {body}
    </div>
  );
}

const CREW_COMMAND = '/crew';

/**
 * "None of your chats can post in #methods yet." and how to connect one, with the command drawn as
 * code: it is something to type, not a word in the sentence (Q3-29).
 */
function EmptyAccess({ text }: { text: string }) {
  const [before, ...rest] = accessCopy.emptyHow.split(CREW_COMMAND);
  return (
    <div className="flex flex-col gap-1 px-3 py-2">
      <p className="text-secondary text-text-default">{text}</p>
      <p className="text-supporting text-text-muted">
        {before}
        {rest.length > 0 ? (
          <>
            <code className="font-mono" translate="no">
              {CREW_COMMAND}
            </code>
            {rest.join(CREW_COMMAND)}
          </>
        ) : null}
      </p>
    </div>
  );
}
