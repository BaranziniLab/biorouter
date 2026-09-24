import { useEffect, useId, useRef, useState } from 'react';
import { Badge } from '../../ui/badge';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '../../ui/dropdown-menu';
import { PersonName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { useSidebarAnnounce, writeClipboard } from './SidebarAnnouncer';
import { useSidebarView } from './sidebarView';
import { unavailableReason } from './WorkspaceMenu';
import './crew-sidebar.css';

const copy = sidebarCopy.you;

/** How long "Copied" / "Couldn't copy" replaces the item's label before it reads as before. */
export const COPY_FEEDBACK_MS = 1500;

type CopyState = 'idle' | 'copied' | 'failed';

/**
 * The You menu (ui-redesign-spec, "The workspace menu and the You menu"), opening upward from the
 * footer: my name with `@username` (the identity `header` context) over the SSH login — and, in a
 * dev profile, the neutral "Profile: {name}" badge, which lives here rather than on the row so
 * the row keeps its width for the name (T-71) — then Edit profile…, Keys and security… and Copy
 * my username. It mounts only while open, so the You row's login stays the one text node at rest.
 *
 * Edit profile and Copy my username need a person to act as, so they wait for the verified
 * snapshot, and say so: a note above them names what they wait for, and it is the menu's
 * description too (T-71). Keys and security is this computer's own storage and is always
 * available.
 *
 * "Copy my username" copies the bare username — what a host types into Invite people's `@`
 * field — and answers ON THE ITEM: the menu stays open and the item reads "Copied" (or
 * "Couldn't copy") for a moment, and the same result is spoken. A copy result is never sent to
 * the channel's connection bar, which is for the connection.
 */
export function YouMenu({ profile = null }: { profile?: string | null }) {
  const crew = useCrew();
  const { dir, verified } = useSidebarView(crew);
  const { announce } = useSidebarAnnounce();
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const timer = useRef<number | null>(null);
  const reasonId = useId();
  const me = dir.me;
  const connection = crew.connection;

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );

  if (!connection) return null;

  const canEdit = verified && Boolean(me);
  const reason = canEdit ? null : unavailableReason(crew.status, false);

  const copyUsername = async () => {
    if (!me) return;
    const copied = await writeClipboard(me.username);
    setCopyState(copied ? 'copied' : 'failed');
    announce(
      copied
        ? copy.announceCopiedUsername(me.username)
        : copy.announceCopyUsernameFailed(me.username)
    );
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setCopyState('idle');
    }, COPY_FEEDBACK_MS);
  };

  return (
    <DropdownMenuContent
      side="top"
      align="start"
      className="w-64"
      data-crew-menu="you"
      aria-describedby={reason ? reasonId : undefined}
    >
      <div className="crew-sidebar-menu-header" data-crew-menu-header="">
        {me && (
          <span className="crew-sidebar-truncate">
            <PersonName person={me} context="header" dir={dir} />
          </span>
        )}
        <span
          className="crew-sidebar-truncate font-mono text-supporting text-text-muted"
          translate="no"
        >
          {connection.ssh_target}
        </span>
        {profile && (
          <Badge tone="neutral" className="min-w-0 self-start" data-crew-dev-profile="">
            <span className="crew-sidebar-truncate">{copy.devProfile(profile)}</span>
          </Badge>
        )}
      </div>
      <DropdownMenuSeparator />
      {reason && (
        <p
          id={reasonId}
          className="crew-sidebar-menu-note text-supporting text-text-muted"
          data-crew-menu-note=""
        >
          {reason}
        </p>
      )}
      <DropdownMenuItem
        disabled={!canEdit}
        aria-describedby={canEdit ? undefined : reasonId}
        onSelect={() => crew.openDialog({ kind: 'edit-profile' })}
      >
        {copy.editProfile}
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={() => crew.openDialog({ kind: 'keys' })}>
        {copy.keys}
      </DropdownMenuItem>
      <DropdownMenuItem
        disabled={!me}
        aria-describedby={me ? undefined : reasonId}
        data-crew-copy-state={copyState}
        onSelect={(event) => {
          // Stay open: the item itself shows whether the copy landed.
          event.preventDefault();
          void copyUsername();
        }}
      >
        {copyState === 'copied'
          ? copy.copiedUsername
          : copyState === 'failed'
            ? copy.copyUsernameFailed
            : copy.copyUsername}
      </DropdownMenuItem>
    </DropdownMenuContent>
  );
}
