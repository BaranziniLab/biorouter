import { useMemo } from 'react';
import { LoaderCircle } from '../../icons/app-icons';
import {
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '../../ui/dropdown-menu';
import { StatusDot, type StatusDotTone } from '../../ui/status-dot';
import type { CrewConnection } from '../crewApi';
import { connectionNames, PersonName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { crewStatusCopy } from '../state/copy';
import { CONNECTION_STATUS } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { useSidebarView } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy.workspaceMenu;

/** A saved connection's dot in the Switch section, from the only state the daemon keeps for it. */
function savedStatusTone(status: CrewConnection['status']): StatusDotTone {
  switch (status) {
    case 'connected':
      return 'success';
    case 'authentication_required':
      return 'warning';
    case 'error':
      return 'danger';
    default:
      return 'idle';
  }
}

/** The word beside that dot, for a screen reader (colour is never the only signal). */
function savedStatusWord(status: CrewConnection['status']): string {
  switch (status) {
    case 'connected':
      return crewStatusCopy.connected;
    case 'authentication_required':
      return crewStatusCopy.signInNeeded;
    case 'error':
      return crewStatusCopy.cantConnect;
    default:
      return crewStatusCopy.offline;
  }
}

/**
 * The workspace menu (ui-redesign-spec, wireframe "Workspace menu"; "The workspace menu and the
 * You menu"). A header that is not focusable, then the workspace's own dialogs, then the manual
 * connection tools, then Switch workspace and Add a workspace.
 *
 * - Items that open a dialog end in "…" and open it through the controller's intents, so this
 *   area never imports another.
 * - Reconnect, Sign in… and Disconnect are always listed as manual tools; when the status needs
 *   one it is also the main area's one action.
 * - Switching is a `menuitemradio` that does exactly what the old `<select>` did. With one saved
 *   connection the Switch section is omitted, but Add a workspace stays.
 * - The header's status line is the only other place "Connected · identity verified" is visible,
 *   and it mounts only while the menu is open (so the status row's `sr-only` copy stays the one
 *   text node the regression tests find at rest).
 * - The workspace's own items need its verified snapshot, and are disabled while the sidebar
 *   shows only the last verified copy. React authorizes nothing: the daemon and broker decide
 *   every action these open.
 */
export function WorkspaceMenu({ title }: { title: string }) {
  const crew = useCrew();
  const { dir, verified } = useSidebarView(crew);
  const {
    connection,
    connections,
    connectionId,
    status,
    signIn,
    isPending,
    isHost,
    lastConnectFailure,
  } = crew;
  const labels = useMemo(() => connectionNames(connections), [connections]);
  if (!connection) return null;

  const presentation = status ? CONNECTION_STATUS[status] : null;
  const connecting = isPending('connect') || isPending('sign-in') || signIn.open;
  const lastError = lastConnectFailure?.message || connection.last_error || '';
  const snapshotReady = verified && Boolean(crew.snapshot);

  return (
    <DropdownMenuContent align="start" className="w-72" data-crew-menu="workspace">
      <div className="crew-sidebar-menu-header" data-crew-menu-header="">
        <span className="crew-sidebar-truncate text-label text-text-default">{title}</span>
        {dir.host && (
          <span className="crew-sidebar-truncate text-supporting text-text-muted">
            {copy.hostedBy} <PersonName person={dir.host} context="inline" dir={dir} />
          </span>
        )}
        <span className="crew-sidebar-truncate text-supporting text-text-muted">
          {copy.signedInAs} <span className="font-mono">{connection.ssh_target}</span>
        </span>
        {presentation && (
          <span className="flex min-w-0 items-center gap-1.5 text-supporting text-text-muted">
            {presentation.spinner ? (
              <LoaderCircle className="crew-sidebar-spinner" aria-hidden="true" />
            ) : (
              <StatusDot tone={presentation.tone} />
            )}
            <span className="crew-sidebar-truncate">
              {presentation.srText ?? presentation.word}
            </span>
          </span>
        )}
        {lastError && status !== 'connected' && (
          <span className="crew-sidebar-clamp text-supporting text-text-muted">{lastError}</span>
        )}
      </div>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        {isHost && (
          <DropdownMenuItem
            disabled={!snapshotReady}
            onSelect={() => crew.openDialog({ kind: 'invite-people' })}
          >
            {copy.invite(title)}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          disabled={!snapshotReady}
          onSelect={() => crew.openDialog({ kind: 'workspace-settings', tab: 'people' })}
        >
          {copy.people}
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={!snapshotReady}
          onSelect={() => crew.openDialog({ kind: 'workspace-settings', tab: 'privacy' })}
        >
          {copy.privacy}
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={!snapshotReady}
          onSelect={() => crew.openDialog({ kind: 'workspace-settings', tab: 'agent-access' })}
        >
          {copy.access}
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={!snapshotReady}
          onSelect={() => crew.openDialog({ kind: 'create-team' })}
        >
          {copy.createTeam}
        </DropdownMenuItem>
      </DropdownMenuGroup>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuItem
          disabled={connecting || isPending('disconnect')}
          onSelect={() => void crew.connect({ userInitiated: true })}
        >
          {copy.reconnect}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => crew.openSignIn()}>{copy.signIn}</DropdownMenuItem>
        <DropdownMenuItem
          disabled={isPending('connect') || isPending('disconnect')}
          onSelect={() => void crew.disconnect()}
        >
          {copy.disconnect}
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => crew.openDialog({ kind: 'connection-settings', connectionId })}
        >
          {copy.settings}
        </DropdownMenuItem>
      </DropdownMenuGroup>
      <DropdownMenuSeparator />
      {connections.length >= 2 && (
        <>
          <DropdownMenuLabel>{copy.switchWorkspace}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={connectionId}
            onValueChange={(id) => {
              if (id !== connectionId) crew.selectConnection(id);
            }}
          >
            {connections.map((item) => {
              const current = item.id === connectionId;
              const tone =
                current && presentation ? presentation.tone : savedStatusTone(item.status);
              const word =
                current && presentation ? presentation.word : savedStatusWord(item.status);
              return (
                <DropdownMenuRadioItem key={item.id} value={item.id} disabled={crew.busy}>
                  <StatusDot tone={tone} />
                  <span className="crew-sidebar-truncate">{labels.get(item.id) ?? ''}</span>
                  <span className="sr-only">, {word}</span>
                </DropdownMenuRadioItem>
              );
            })}
          </DropdownMenuRadioGroup>
        </>
      )}
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>{copy.add}</DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          <DropdownMenuItem onSelect={() => crew.openDialog({ kind: 'join' })}>
            {copy.addJoin}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => crew.openDialog({ kind: 'host' })}>
            {copy.addHost}
          </DropdownMenuItem>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    </DropdownMenuContent>
  );
}
