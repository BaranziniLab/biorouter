import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  type FocusEvent,
  type KeyboardEvent,
} from 'react';
import { Check, Copy, LoaderCircle } from '../../icons/app-icons';
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
import { connectErrorText } from '../channel/ConnectionBar';
import type { CrewConnection } from '../crewApi';
import { groupedFingerprint, useWorkspaceKeyFingerprint } from '../dialogs/fingerprint';
import { connectionNames, PersonName } from '../identity';
import { CONNECT_FAILURE_CODES } from '../state/connectFailure';
import { useCrew } from '../state/CrewControllerContext';
import { crewStatusCopy } from '../state/copy';
import { CONNECTION_STATUS, type ConnectionStatusKey } from '../state/crewStatus';
import { sidebarCopy } from './copy';
import { useMenuCopyItem, type MenuCopyItem, type MenuCopyState } from './menuCopy';
import { useSidebarAnnounce, writeClipboard } from './SidebarAnnouncer';
import { serverLabel, usePendingHost, useSidebarView } from './sidebarView';
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
 * Whether the menu offers Reconnect (Q3-57): only while the connection is not connected and
 * verified — which covers a join waiting for the host, whose status is "Not joined yet". Beside
 * "Connected · identity verified" it could only restart a healthy connection, and a keyboard user
 * pressed it by accident. Every other state keeps it, disabled while a connect already runs.
 */
export function offersReconnect(status: ConnectionStatusKey | null): boolean {
  return status !== 'connected';
}

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
 * - Disconnect and Connection settings are always listed as manual tools. **Reconnect is listed
 *   only while the connection is not connected and verified** (Q3-57, {@link offersReconnect}),
 *   and **Sign in… only while sign-in is needed** (T-40): offered under "Signed in as @alice", it
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
 *   away for that reason), and in the menu it would sit unexplained beside "Your code doesn't
 *   change."; before verification there is nothing for it to confirm. It carries a small Copy
 *   (Q4-49), as host step 3 does — step 3 says "Your workspace menu shows it too", and selecting
 *   text inside a menu is not something a person tries — see {@link FingerprintCopy}. It answers
 *   with every sidebar menu copy's timing (Q3-57, `useMenuCopyItem`): "Copied", then the menu
 *   closes; a refused copy reads "Couldn't copy" and the menu stays. So the menu's open state is
 *   the switcher's, beside the Copy's, as the You row holds its menu's.
 * - That Copy is never where the keyboard lands (Q4-49 round 2). It is the menu's first item in
 *   DOM order, because it is drawn on the header's fingerprint line, so Radix's first-item rule
 *   (a keyboard open, Home, PageUp, ArrowDown from the menu itself) would put a person on "Copy
 *   workspace fingerprint" ahead of every action. Those land on the first action instead
 *   ({@link useFirstStopSkipsHeaderCopy}); ArrowUp from there reaches the Copy, in the order it is
 *   drawn.
 * - While a join waits for the host, Reconnect and Disconnect each say what they do and that the
 *   code already sent survives it (Q2-43, Q3-47): "Try the connection again. Your code doesn't
 *   change." and "Stop waiting for now. {host} can still let you in with the same code." The code
 *   is computed from this computer's saved device key and the pinned workspace key
 *   (`device_code_of`), which neither action touches.
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
export function WorkspaceMenu({
  title,
  fingerprintCopy,
}: {
  title: string;
  /** The Copy's state, owned by the switcher with the menu's open state so a copy can close it. */
  fingerprintCopy?: MenuCopyItem;
}) {
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
  const { host } = usePendingHost(crew);
  const headerId = useId();
  const reasonId = useId();
  const keptId = useId();
  const fingerprintHex = useWorkspaceKeyFingerprint(connection?.workspace_public_key);
  const copyLabelId = useId();
  const { announce } = useSidebarAnnounce();
  const ownCopy = useMenuCopyItem(keepOpen);
  const copyItem = fingerprintCopy ?? ownCopy;
  const firstStop = useFirstStopSkipsHeaderCopy();
  if (!connection) return null;

  const presentation = status ? CONNECTION_STATUS[status] : null;
  const connecting = isPending('connect') || isPending('sign-in') || signIn.open;
  const snapshotReady = verified && Boolean(crew.snapshot);
  const reason = unavailableReason(status, snapshotReady);
  const server = serverLabel(connection);
  // Never the transport's words (NEW-1): the daemon's saved `last_error` for an SSH drop is its
  // record ("Crew SSH failure [ssh_eof; …]"), read here by its typed code, or plainly.
  const savedCode = connection.last_error_code;
  const lastError = lastConnectFailure
    ? connectErrorText(lastConnectFailure.kind, lastConnectFailure.message, server)
    : connection.last_error
      ? connectErrorText(
          savedCode && Object.prototype.hasOwnProperty.call(CONNECT_FAILURE_CODES, savedCode)
            ? CONNECT_FAILURE_CODES[savedCode]
            : undefined,
          connection.last_error,
          server
        )
      : '';
  const me = verified ? dir.me : null;
  const fingerprint =
    fingerprintHex && status === 'connected' ? groupedFingerprint(fingerprintHex) : '';
  const joining = status === 'not-joined';

  const copyFingerprint = async (value: string) => {
    const copied = await copyItem.run(() => writeClipboard(value));
    announce(copied ? sidebarCopy.clipboard.copied : sidebarCopy.clipboard.failed);
  };

  return (
    <DropdownMenuContent
      align="start"
      className="w-72"
      data-crew-menu="workspace"
      aria-describedby={reason ? `${headerId} ${reasonId}` : headerId}
      ref={firstStop.ref}
      onFocus={firstStop.onFocus}
      onKeyDownCapture={firstStop.onKeyDownCapture}
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
        {fingerprint && fingerprintHex && (
          <span
            className="crew-sidebar-menu-fingerprint text-supporting text-text-muted"
            data-crew-menu-fingerprint=""
          >
            <span className="crew-sidebar-truncate">
              {copy.fingerprint}{' '}
              <bdi className="font-mono" translate="no">
                {fingerprint}
              </bdi>
            </span>
            <FingerprintCopy
              labelId={copyLabelId}
              state={copyItem.state}
              onCopy={() => void copyFingerprint(fingerprintHex)}
            />
          </span>
        )}
        {lastError && status !== 'connected' && (
          <span className="crew-sidebar-clamp text-supporting text-text-muted">{lastError}</span>
        )}
      </div>
      {fingerprint && fingerprintHex && (
        // The Copy's name, outside the header: the header is the menu's description, and a name
        // inside it would add "Copy workspace fingerprint" to what the menu is described as.
        <span id={copyLabelId} hidden>
          {fingerprintCopyLabel(copyItem.state)}
        </span>
      )}
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
        {offersReconnect(status) && (
          <DropdownMenuItem
            disabled={connecting || isPending('disconnect')}
            aria-describedby={joining ? `${keptId}-reconnect` : undefined}
            className={joining ? 'flex-col items-start gap-0' : undefined}
            onSelect={() => void crew.connect({ userInitiated: true })}
          >
            {copy.reconnect}
            {joining && <KeptNote id={`${keptId}-reconnect`} text={copy.joinCodeKept.reconnect} />}
          </DropdownMenuItem>
        )}
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
          {joining && (
            <KeptNote id={`${keptId}-disconnect`} text={copy.joinCodeKept.disconnect(host)} />
          )}
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

/** A workspace menu rendered without its switcher has no menu state to close; the Copy still answers. */
const keepOpen = () => {};

/** The Copy's accessible name: what it copies, then what became of the copy. */
function fingerprintCopyLabel(state: MenuCopyState): string {
  return state === 'copied'
    ? copy.copiedFingerprint
    : state === 'failed'
      ? copy.copyFingerprintFailed
      : copy.copyFingerprintLabel;
}

/** Keys that send Radix to a menu's first item from one of its items. */
const FIRST_FROM_ITEM: ReadonlySet<string> = new Set(['Home', 'PageUp']);
/** …and from the menu itself, which holds focus after a pointer opened it. */
const FIRST_FROM_MENU: ReadonlySet<string> = new Set(['Home', 'PageUp', 'ArrowDown']);

/**
 * Where the menu's first stop goes instead of the header's Copy: the first enabled item after it,
 * in this menu (a submenu's items are portalled elsewhere). `null` when the Copy is not shown —
 * then nothing here intervenes and Radix's own rule stands.
 */
function firstActionAfterHeaderCopy(content: HTMLElement | null): HTMLElement | null {
  if (!content?.querySelector('[data-crew-copy-fingerprint]')) return null;
  const items = Array.from(content.querySelectorAll<HTMLElement>('[role^="menuitem"]'));
  return (
    items.find(
      (item) =>
        item.closest('[data-radix-menu-content]') === content &&
        !item.hasAttribute('data-crew-copy-fingerprint') &&
        !item.hasAttribute('data-disabled')
    ) ?? null
  );
}

/**
 * Keeps the header's fingerprint Copy from being the menu's first stop (Q4-49 round 2).
 *
 * Radix sends focus to a menu's first item in DOM order when the keyboard opens it (its entry
 * focus), and on Home or PageUp, or ArrowDown while the menu itself holds focus. Here that item is
 * the Copy on the header's fingerprint line, so Enter on the switcher put a keyboard or
 * screen-reader user on "Copy workspace fingerprint" instead of "Invite people to {workspace}…"
 * or "People…". Each of those goes to the first action instead; ArrowUp from it still reaches the
 * Copy, where it is drawn. The Copy keeps its place in the DOM, so the order the arrows walk is
 * the order a person sees.
 *
 * - The keys are taken in the capture phase, before the item's roving handler (which moves focus
 *   on a timer) or the menu's own sees them.
 * - The entry focus cannot be cancelled from here — `DropdownMenuContent` does not take Radix's
 *   `onEntryFocus` — so it is recognised as it lands: focus reaching the Copy from the menu itself,
 *   while the keyboard is in use, before any key has gone down inside this menu. That is Radix's
 *   own condition for an entry focus: `Menu` focuses the first item only while a key was pressed
 *   since the last pointer down or move (its `isUsingKeyboardRef`, tracked here the same way). The
 *   pointer resting on the Copy is not it (the pointer moved), and neither is typeahead (a key went
 *   down in the menu). Focus moves on within the same task, before anything is painted or read.
 */
function useFirstStopSkipsHeaderCopy() {
  const usingKeyboard = useRef(false);
  const keyInMenu = useRef(false);
  useEffect(() => {
    const onPointer = () => {
      usingKeyboard.current = false;
    };
    const onKeyDown = () => {
      usingKeyboard.current = true;
      document.addEventListener('pointerdown', onPointer, { capture: true, once: true });
      document.addEventListener('pointermove', onPointer, { capture: true, once: true });
    };
    document.addEventListener('keydown', onKeyDown, { capture: true });
    return () => {
      document.removeEventListener('keydown', onKeyDown, { capture: true });
      document.removeEventListener('pointerdown', onPointer, { capture: true });
      document.removeEventListener('pointermove', onPointer, { capture: true });
    };
  }, []);

  // The content mounts afresh each time the menu opens: no key has gone down in it yet.
  const ref = useCallback((node: HTMLDivElement | null) => {
    if (node) keyInMenu.current = false;
  }, []);

  const onFocus = (event: FocusEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    if (!target.hasAttribute('data-crew-copy-fingerprint')) return;
    if (event.relatedTarget !== event.currentTarget) return;
    if (!usingKeyboard.current || keyInMenu.current) return;
    firstActionAfterHeaderCopy(event.currentTarget)?.focus({ preventScroll: true });
  };

  const onKeyDownCapture = (event: KeyboardEvent<HTMLDivElement>) => {
    const menu = event.currentTarget;
    const target = event.target as HTMLElement;
    // A key in the Add a workspace submenu reaches here through the React tree; it is its own.
    if (target.closest('[data-radix-menu-content]') !== menu) return;
    keyInMenu.current = true;
    if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    const keys = target === menu ? FIRST_FROM_MENU : FIRST_FROM_ITEM;
    if (!keys.has(event.key)) return;
    const action = firstActionAfterHeaderCopy(menu);
    if (!action) return;
    event.preventDefault();
    action.focus();
  };

  return { ref, onFocus, onKeyDownCapture };
}

/**
 * The small Copy beside the fingerprint (Q4-49): a real menu item, so the arrow keys reach it
 * (static text in a menu is skipped, and a plain button there could not be focused at all), drawn
 * as a compact control on the fingerprint's own line. It copies the whole fingerprint, the value
 * host step 3's Copy copies; the line shows its first 16 digits, grouped. Its name comes from a
 * label outside the header (`labelId`), and its visible word is hidden from the accessibility tree,
 * so the menu's description — the header — does not end in "Copy". Selecting it does not close
 * the menu there and then: the item answers first, and `useMenuCopyItem` closes it.
 */
function FingerprintCopy({
  labelId,
  state,
  onCopy,
}: {
  labelId: string;
  state: MenuCopyState;
  onCopy(): void;
}) {
  return (
    <DropdownMenuItem
      className="crew-sidebar-menu-copy"
      aria-labelledby={labelId}
      data-crew-copy-fingerprint=""
      data-crew-copy-state={state}
      onSelect={(event) => {
        // Not yet: the item shows whether the copy landed, then a landed copy closes the menu.
        event.preventDefault();
        onCopy();
      }}
    >
      {state === 'copied' ? (
        <Check className="crew-sidebar-menu-copy-icon" aria-hidden="true" />
      ) : (
        <Copy className="crew-sidebar-menu-copy-icon" aria-hidden="true" />
      )}
      <span aria-hidden="true">
        {state === 'copied'
          ? copy.copiedFingerprint
          : state === 'failed'
            ? copy.copyFingerprintFailed
            : copy.copyFingerprint}
      </span>
    </DropdownMenuItem>
  );
}

/**
 * What a connection tool does to a join that waits (Q2-43, Q3-47), under the tool. Hidden from the
 * item's name, so the item is still called "Reconnect", and read as its description.
 */
function KeptNote({ id, text }: { id: string; text: string }) {
  return (
    <span
      id={id}
      aria-hidden="true"
      className="text-supporting text-text-muted"
      data-crew-join-code-kept=""
    >
      {text}
    </span>
  );
}
