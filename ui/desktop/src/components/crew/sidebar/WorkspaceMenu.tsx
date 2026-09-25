import { useId, useMemo } from 'react';
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
import { groupedFingerprint, useWorkspaceKeyFingerprint } from '../dialogs/fingerprint';
import { connectionNames, PersonName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { crewStatusCopy } from '../state/copy';
import { CONNECTION_STATUS, type ConnectionStatusKey } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { serverLabel, useSidebarView } from './sidebarView';
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
 * Statuses in which the connection itself is up and only verification is outstanding. Telling
 * that person "once you're connected" contradicts the status row beside it.
 */
const CONNECTION_UP: ReadonlySet<string> = new Set<ConnectionStatusKey>([
  'connected',
  'checking',
  'updating',
  'updates-unavailable',
]);

/**
 * Why the workspace's own items are disabled, or `null` when they are not (T-40, T-71). A joiner
 * the host has not let in yet waits for that; a person whose connection is up waits for it to be
 * verified; anyone else waits to be connected.
 */
export function unavailableReason(status: string | null, ready: boolean): string | null {
  if (ready) return null;
  if (status === 'not-joined') return sidebarCopy.unavailable.notJoined;
  return status !== null && CONNECTION_UP.has(status)
    ? sidebarCopy.unavailable.notVerified
    : sidebarCopy.unavailable.notConnected;
}

/**
 * The workspace menu (ui-redesign-spec, wireframe "Workspace menu"; "The workspace menu and the
 * You menu"). A header that is not focusable, then the workspace's own dialogs, then the manual
 * connection tools, then Switch workspace and Add a workspace.
 *
 * - Items that open a dialog end in "…" and open it through the controller's intents, so this
 *   area never imports another.
 * - Reconnect, Disconnect and Connection settings are always listed as manual tools. **Sign in…
 *   is listed only while sign-in is needed** (T-40): offered under "Signed in as @alice", it
 *   read as a second, unexplained Reconnect. When a Reconnect needs credentials, the sign-in
 *   dialog opens by itself.
 * - The header names the PERSON and the server — "Signed in as @alice on lab-server" — never the
 *   SSH login alone. The server is the person's own name for it (D-ALIAS): the daemon's
 *   `server_label`, their SSH alias when one maps to the address, else the host; the raw address
 *   stays in Connection settings. Before this connection's identity is verified there is no
 *   person to name, so it states the server only.
 * - After the status line, the workspace key's fingerprint, grouped as the Join dialog shows it
 *   (`Fingerprint 6682 327B A040 C709`), so a host asked "does it match?" finds it beside
 *   "identity verified" instead of under Connection settings' IDs for support (Q2-04). Shown
 *   ONLY in that status — "Connected · identity verified" is the line it explains. A joiner the
 *   host has not let in yet took the fingerprint for the code to send (the Join dialog folds it
 *   away for that reason), and in the menu it would sit unexplained beside "Your join code stays
 *   the same."; before verification there is nothing for it to confirm.
 * - While a join waits for the host, Reconnect and Disconnect each say "Your join code stays the
 *   same." (Q2-43): the code is computed from this computer's saved device key and the pinned
 *   workspace key (`device_code_of`), which neither action touches.
 * - The header's text is the menu's `aria-describedby`: a screen reader's menu navigation skips
 *   static text inside `role="menu"`, so without it the header was unreachable.
 * - A disabled item says why: one note above the workspace's own items, which are disabled until
 *   its snapshot is verified, also in the menu's description.
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
  const headerId = useId();
  const reasonId = useId();
  const keptId = useId();
  const fingerprintHex = useWorkspaceKeyFingerprint(connection?.workspace_public_key);
  if (!connection) return null;

  const presentation = status ? CONNECTION_STATUS[status] : null;
  const connecting = isPending('connect') || isPending('sign-in') || signIn.open;
  const lastError = lastConnectFailure?.message || connection.last_error || '';
  const snapshotReady = verified && Boolean(crew.snapshot);
  const reason = unavailableReason(status, snapshotReady);
  const server = serverLabel(connection);
  const me = verified ? dir.me : null;
  const fingerprint =
    fingerprintHex && status === 'connected' ? groupedFingerprint(fingerprintHex) : '';
  const joining = status === 'not-joined';

  return (
    <DropdownMenuContent
      align="start"
      className="w-72"
      data-crew-menu="workspace"
      aria-describedby={reason ? `${headerId} ${reasonId}` : headerId}
    >
      <div id={headerId} className="crew-sidebar-menu-header" data-crew-menu-header="">
        <span className="crew-sidebar-truncate text-label text-text-default">{title}</span>
        {dir.host && (
          <span className="crew-sidebar-truncate text-supporting text-text-muted">
            {copy.hostedBy} <PersonName person={dir.host} context="inline" dir={dir} />
          </span>
        )}
        <span
          className="crew-sidebar-truncate text-supporting text-text-muted"
          data-crew-signed-in=""
        >
          {me ? (
            <>
              {copy.signedInAs} <bdi className="font-mono" translate="no">{`@${me.username}`}</bdi>{' '}
              {copy.signedInOn}{' '}
              <bdi className="font-mono" translate="no">
                {server}
              </bdi>
            </>
          ) : (
            <>
              {copy.server}{' '}
              <bdi className="font-mono" translate="no">
                {server}
              </bdi>
            </>
          )}
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
        {fingerprint && (
          <span
            className="crew-sidebar-truncate text-supporting text-text-muted"
            data-crew-menu-fingerprint=""
          >
            {copy.fingerprint}{' '}
            <bdi className="font-mono" translate="no">
              {fingerprint}
            </bdi>
          </span>
        )}
        {lastError && status !== 'connected' && (
          <span className="crew-sidebar-clamp text-supporting text-text-muted">{lastError}</span>
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
      <DropdownMenuGroup aria-describedby={reason ? reasonId : undefined}>
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
          aria-describedby={joining ? `${keptId}-reconnect` : undefined}
          className={joining ? 'flex-col items-start gap-0' : undefined}
          onSelect={() => void crew.connect({ userInitiated: true })}
        >
          {copy.reconnect}
          {joining && <KeptNote id={`${keptId}-reconnect`} />}
        </DropdownMenuItem>
        {status === 'sign-in-needed' && (
          <DropdownMenuItem onSelect={() => crew.openSignIn()}>{copy.signIn}</DropdownMenuItem>
        )}
        <DropdownMenuItem
          disabled={isPending('connect') || isPending('disconnect')}
          aria-describedby={joining ? `${keptId}-disconnect` : undefined}
          className={joining ? 'flex-col items-start gap-0' : undefined}
          onSelect={() => void crew.disconnect()}
        >
          {copy.disconnect}
          {joining && <KeptNote id={`${keptId}-disconnect`} />}
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

/**
 * "Your join code stays the same." under a connection tool while a join waits (Q2-43). Hidden from
 * the item's name, so the item is still called "Reconnect", and read as its description.
 */
function KeptNote({ id }: { id: string }) {
  return (
    <span
      id={id}
      aria-hidden="true"
      className="text-supporting text-text-muted"
      data-crew-join-code-kept=""
    >
      {copy.joinCodeKept}
    </span>
  );
}
