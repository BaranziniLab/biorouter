import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '../../ui/dropdown-menu';
import { PersonName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { useSidebarCopy } from './SidebarAnnouncer';
import { useSidebarView } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy.you;

/**
 * The You menu (ui-redesign-spec, "The workspace menu and the You menu"), opening upward from the
 * footer: my name with `@username` (the identity `header` context) over the SSH login, then Edit
 * profile…, Keys and security… and Copy my username. It mounts only while open, so the You row's
 * login stays the one text node at rest.
 *
 * Edit profile and Copy my username need a person to act as, so they wait for the verified
 * snapshot; Keys and security is this computer's own storage and is always available. "Copy my
 * username" copies the bare username — what a host types into Invite people's `@` field.
 */
export function YouMenu() {
  const crew = useCrew();
  const { dir, verified } = useSidebarView(crew);
  const copyText = useSidebarCopy();
  const me = dir.me;
  const connection = crew.connection;
  if (!connection) return null;

  return (
    <DropdownMenuContent side="top" align="start" className="w-64" data-crew-menu="you">
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
      </div>
      <DropdownMenuSeparator />
      <DropdownMenuItem
        disabled={!verified || !me}
        onSelect={() => crew.openDialog({ kind: 'edit-profile' })}
      >
        {copy.editProfile}
      </DropdownMenuItem>
      <DropdownMenuItem onSelect={() => crew.openDialog({ kind: 'keys' })}>
        {copy.keys}
      </DropdownMenuItem>
      <DropdownMenuItem
        disabled={!me}
        onSelect={() => {
          if (me) void copyText(me.username);
        }}
      >
        {copy.copyUsername}
      </DropdownMenuItem>
    </DropdownMenuContent>
  );
}
