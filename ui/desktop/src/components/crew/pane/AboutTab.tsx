import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import { cn } from '../../../utils';
import { channelName, PersonName, teamName } from '../identity';
import { aboutCopy } from './copy';
import { COPY_FEEDBACK_MS, copyText, usePanePresentation } from './presentation';

export interface AboutTabProps {
  /** Offer Rename… to the owner (the broker advertises `unique_names_v1`, naming slice S2). */
  canRename?: boolean;
  className?: string;
}

function Row({
  label,
  children,
  action,
}: {
  label: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="biorouter-settings-row flex min-w-0 items-start justify-between gap-3 px-1 py-2.5">
      <dl className="min-w-0 flex-1">
        <dt className="text-supporting text-text-muted">{label}</dt>
        <dd className="mt-0.5 min-w-0 break-words text-label text-text-default">{children}</dd>
      </dl>
      {action ? <div className="flex shrink-0 items-center">{action}</div> : null}
    </div>
  );
}

/**
 * The details pane's About tab (ui-redesign-spec, "The details pane"): the channel's name, who can
 * read it ("Private models only" for a Restricted channel, T-67), who owns it (and to whom
 * ownership is offered), who made it — every person here in the one authority form, "Iris Wong
 * (@crew_iris)" (Q4-21) — its team, Copy channel ID — the one place this tab holds an
 * ID, behind a copy — and the owner's danger zone. Copy channel ID is a machine ID, for a support
 * request, so it waits behind a quiet "IDs for support" disclosure rather than sitting among the
 * everyday rows (Q3-26), as Connection settings keeps its own. The disclosure sits above the
 * danger zone, never in it: a harmless copy under "Archive channel…" read as dangerous (T-67). The
 * copy answers on itself: "Copied" (or "Couldn't copy") for a moment, also spoken (Q2-34). The
 * owner's actions open the same dialog intents as the channel menu; the broker decides.
 */
export function AboutTab({ canRename = false, className }: AboutTabProps) {
  const { crew, channel, team, dir, isOwner } = usePanePresentation();
  const dangerId = useId();
  const [copyOutcome, setCopyOutcome] = useState<'copied' | 'failed' | null>(null);
  const timer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );
  if (!channel) return null;

  const copyId = async () => {
    const copied = await copyText(channel.id);
    setCopyOutcome(copied ? 'copied' : 'failed');
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setCopyOutcome(null);
    }, COPY_FEEDBACK_MS);
  };
  const copyWords =
    copyOutcome === 'copied'
      ? aboutCopy.copied
      : copyOutcome === 'failed'
        ? aboutCopy.copyFailed
        : null;
  const ownerTools = isOwner && !channel.archived;
  const restricted = channel.classification !== 'public_safe';

  return (
    <div className={cn('flex flex-col gap-4', className)}>
      <div className="biorouter-settings-list crew-pane-rows">
        <Row
          label={aboutCopy.name}
          action={
            ownerTools && canRename ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                aria-label={aboutCopy.renameName}
                onClick={() =>
                  crew.openDialog({ kind: 'rename', target: 'channel', targetId: channel.id })
                }
              >
                {aboutCopy.rename}
              </Button>
            ) : undefined
          }
        >
          {channelName(channel)}
        </Row>
        <Row label={aboutCopy.whoCanRead}>
          <span>{restricted ? aboutCopy.restricted : aboutCopy.publicSafe}</span>
          <span className="block text-supporting text-text-muted">
            {restricted ? aboutCopy.restrictedHint : aboutCopy.publicSafeHint}
          </span>
        </Row>
        <Row
          label={aboutCopy.owner}
          action={
            ownerTools ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() =>
                  crew.openDialog({ kind: 'transfer-ownership', channelId: channel.id })
                }
              >
                {aboutCopy.transfer}
              </Button>
            ) : undefined
          }
        >
          <PersonName person={channel.owner_id} dir={dir} context="authority" />
          {channel.pending_owner && (
            <span className="block text-supporting text-text-muted">
              {aboutCopy.offeredTo}{' '}
              <PersonName person={channel.pending_owner} dir={dir} context="authority" />
              {' · '}
              {aboutCopy.waiting}
            </span>
          )}
        </Row>
        {/* Owner's form, the authority point's (Q4-21): one person read "Iris Wong @crew_iris"
            as Owner and "Iris Wong (@crew_iris)" as Created by on the next row. Both are now
            `personLabel(…, 'authority')`, "Iris Wong (@crew_iris)", drawn as the Members rows
            draw it. */}
        <Row label={aboutCopy.createdBy}>
          <PersonName person={channel.created_by} dir={dir} context="authority" />
        </Row>
        <Row label={aboutCopy.team}>{teamName(team)}</Row>
      </div>

      <Disclosure label={aboutCopy.idsForSupport}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="crew-pane-flush text-text-muted"
          data-crew-copy-state={copyOutcome ?? undefined}
          onClick={() => void copyId()}
        >
          {copyWords ?? aboutCopy.copyId}
        </Button>
        <span className="sr-only" aria-live="polite" aria-atomic="true">
          {copyWords ?? ''}
        </span>
      </Disclosure>

      {ownerTools && (
        <section aria-labelledby={dangerId} className="flex flex-col gap-2">
          <h3 id={dangerId} className="text-caps text-text-muted">
            {aboutCopy.dangerZone}
          </h3>
          <div>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              onClick={() =>
                crew.openDialog({
                  kind: 'confirm',
                  confirm: { action: 'archive-channel', channelId: channel.id },
                })
              }
            >
              {aboutCopy.archive}
            </Button>
          </div>
        </section>
      )}
    </div>
  );
}
