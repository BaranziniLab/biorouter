import { useId, type ReactNode } from 'react';
import { Button } from '../../ui/button';
import { cn } from '../../../utils';
import { channelName, PersonName, teamName } from '../identity';
import { aboutCopy } from './copy';
import { copyText, usePanePresentation } from './presentation';

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
 * ownership is offered), who made it, its team, Copy channel ID — the one place this tab holds an
 * ID, behind a copy — and the owner's danger zone. Copy channel ID sits above the danger zone,
 * never in it: a harmless copy under "Archive channel…" read as dangerous (T-67). The owner's
 * actions open the same dialog intents as the channel menu; the broker decides.
 */
export function AboutTab({ canRename = false, className }: AboutTabProps) {
  const { crew, channel, team, dir, isOwner } = usePanePresentation();
  const dangerId = useId();
  if (!channel) return null;
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
        <Row label={aboutCopy.createdBy}>
          <PersonName person={channel.created_by} dir={dir} context="inline" />
        </Row>
        <Row label={aboutCopy.team}>{teamName(team)}</Row>
      </div>

      <div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="crew-pane-flush text-text-muted"
          onClick={() => void copyText(channel.id)}
        >
          {aboutCopy.copyId}
        </Button>
      </div>

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
