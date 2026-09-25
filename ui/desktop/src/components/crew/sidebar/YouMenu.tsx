import { useId } from 'react';
import { Badge } from '../../ui/badge';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '../../ui/dropdown-menu';
import { PersonName, type CrewPerson } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { MENU_COPY_FAILED_MS, useMenuCopyItem, type MenuCopyItem } from './menuCopy';
import { useSidebarAnnounce, writeClipboard } from './SidebarAnnouncer';
import {
  knownUsername,
  loginLabel,
  serverLabel,
  usePendingHost,
  useSidebarView,
  type LabelledConnection,
} from './sidebarView';
import { unavailableReason } from './WorkspaceMenu';
import './crew-sidebar.css';

const copy = sidebarCopy.you;

/** How long "Couldn't copy" replaces the item's label before it reads as before. */
export const COPY_FEEDBACK_MS = MENU_COPY_FAILED_MS;

/**
 * The line under my name, in the You row and the You menu's header (Q4-50): "on lab-server" once a
 * snapshot names me — where I am, in the sans supporting style, since the name above already says
 * who — and, before that, the SSH login as the placeholder, in monospace, as its own text node
 * (`crew_alice@lab-server`; the regression tests find `fixture`). The server is the person's own
 * name for it (D-ALIAS) either way.
 */
export function YouPlace({
  me,
  connection,
}: {
  me: CrewPerson | null | undefined;
  connection: LabelledConnection;
}) {
  const server = serverLabel(connection);
  if (me && server) {
    return (
      <span
        className="crew-sidebar-truncate text-supporting text-text-muted"
        data-crew-you-place=""
      >
        {copy.on} <bdi translate="no">{server}</bdi>
      </span>
    );
  }
  return (
    <span
      className="crew-sidebar-truncate font-mono text-supporting text-text-muted"
      translate="no"
      data-crew-you-login=""
    >
      {loginLabel(connection)}
    </span>
  );
}

/** A You menu rendered without its row has no menu state to close; the item still answers. */
const keepOpen = () => {};

/**
 * The You menu (ui-redesign-spec, "The workspace menu and the You menu"), opening upward from the
 * footer: my name with `@username` (the identity `header` context) over where I am — "on
 * lab-server", or the SSH login before a snapshot names me ({@link YouPlace}, the row's own line)
 * — and, in a dev profile, the neutral "Profile: {name}" badge, which lives here rather than on the
 * row so the row keeps its width for the name (T-71) — then Edit profile…, Keys and security… and
 * Copy my username. It mounts only while open, so the You row's line stays the one at rest.
 *
 * Edit profile needs a person to act as, so it waits for the verified snapshot, and says so: a
 * note above it names what it waits for, and it is the menu's description too (T-71). Copy my
 * username needs only the username, so it is available whenever anything names it — the verified
 * directory, the saved login or the join (Q2-43: a joiner's own `crew_frank` was greyed out while
 * it was shown right above). Keys and security is this computer's own storage and is always
 * available.
 *
 * "Copy my username" copies the bare username — what a host types into Invite people's `@`
 * field — and answers ON THE ITEM, with every sidebar menu's timing (Q3-57, `useMenuCopyItem`):
 * the item reads "Copied" and the menu closes a moment later, still saying so; a refused copy
 * reads "Couldn't copy" and the menu stays. It used to stay open after "Copied" and flip back,
 * unlike the team and message menus. The same result is spoken. A copy result is never sent to
 * the channel's connection bar, which is for the connection.
 *
 * `usernameCopy` is the item's state, owned by `YouRow` with the menu's open state, so the item
 * can close the menu it sits in.
 */
export function YouMenu({
  profile = null,
  usernameCopy,
}: {
  profile?: string | null;
  usernameCopy?: MenuCopyItem;
}) {
  const crew = useCrew();
  const { dir, verified } = useSidebarView(crew);
  const { announce } = useSidebarAnnounce();
  const ownCopy = useMenuCopyItem(keepOpen);
  const copyItem = usernameCopy ?? ownCopy;
  const copyState = copyItem.state;
  const reasonId = useId();
  const { username: remembered } = usePendingHost(crew);
  const me = dir.me;
  const connection = crew.connection;
  const username = knownUsername(me, connection, remembered);

  if (!connection) return null;

  const canEdit = verified && Boolean(me);
  const reason = canEdit ? null : unavailableReason(crew.status, false);

  const copyUsername = async () => {
    if (!username) return;
    const copied = await copyItem.run(() => writeClipboard(username));
    announce(
      copied ? copy.announceCopiedUsername(username) : copy.announceCopyUsernameFailed(username)
    );
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
        <YouPlace me={me} connection={connection} />
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
        disabled={!username}
        aria-describedby={username ? undefined : reasonId}
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
