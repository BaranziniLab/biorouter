import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Avatar } from '../../ui/avatar';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Note } from '../../ui/note';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { AlertTriangle, MoreHorizontal } from '../../icons/app-icons';
import type { PendingJoin } from '../crewApi';
import {
  InstitutionName,
  joinerPerson,
  PersonName,
  personLabel,
  type CrewPerson,
} from '../identity';
import { sidebarCopy } from '../sidebar/copy';
import { useFocusReturn } from '../state/focusReturn';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ConfirmIntent, ErrorSource, WorkspaceSettingsTab } from '../state/types';
import { copyText } from './clipboard';
import { CrewConfirmation } from './confirmations';
import { workspaceSettingsCopy as copy } from './copy';
import { DialogErrorNote, useDismissOwnError } from './fields';
import { MakePrivateDialog } from './MakePrivateDialog';
import { peopleInOrder } from './people';
import { uniqueNamesSupported, useDialogView, type DialogView } from './workspace';
import './dialogs.css';

const SOURCE: ErrorSource = 'dialog:workspace-settings';

export interface WorkspaceSettingsDialogProps {
  /** The tab to open on; General when absent (or when Agent access has no content). */
  tab?: WorkspaceSettingsTab;
  /**
   * The Agent access tab's content: the workspace-wide list of chats and tasks that can post,
   * owned by the access area. Without it the tab is not offered.
   */
  agentAccess?: React.ReactNode;
  onClose(): void;
}

/**
 * Workspace settings (ui-redesign-spec, "Dialog inventory"): General · People · Privacy · Agent
 * access, the one home for what the workspace menu's People…, Privacy… and Chats with access… open.
 *
 * Every control here sends a request the broker or daemon decides; host-only controls are shown
 * only to the host because they are the only person the broker would accept them from, not because
 * this dialog grants anything. Changes that expose data or cannot be undone go through the
 * confirmations of the copy deck, opened over this dialog so Cancel returns here — and focus to
 * the control that opened them.
 *
 * The dialog is as tall as its tallest own tab whichever is showing (QA T-30): General, People and
 * Privacy stay mounted, stacked in one grid cell, and only the selected one is visible and in the
 * accessibility tree, so switching tabs never moves the footer. The tab list is named, and
 * Shift+Tab from it leaves the list (to the dialog's last control, as a focus trap wraps) instead
 * of Radix's roving group handing focus straight back to the selected tab (QA T-39).
 */
export function WorkspaceSettingsDialog({
  tab,
  agentAccess,
  onClose,
}: WorkspaceSettingsDialogProps) {
  const view = useDialogView();
  const { crew, workspace } = view;
  const offered: WorkspaceSettingsTab[] = ['general', 'people', 'privacy'];
  if (agentAccess !== undefined) offered.push('agent-access');
  const [current, setCurrent] = React.useState<WorkspaceSettingsTab>(
    tab && offered.includes(tab) ? tab : 'general'
  );
  const [confirm, setConfirm] = React.useState<ConfirmIntent | null>(null);
  const [makingPrivate, setMakingPrivate] = React.useState(false);
  const dismissOwnError = useDismissOwnError(SOURCE, 'dialog:confirm');
  const nestedFocus = useFocusReturn();

  const ask = (intent: ConfirmIntent) => {
    dismissOwnError();
    nestedFocus.remember();
    setConfirm(intent);
  };
  const panel = (value: WorkspaceSettingsTab) => ({
    value,
    forceMount: true as const,
    className: 'crew-settings-panel',
    // Mounted but not selected: out of sight (CSS), of the tab order and of the a11y tree.
    ...(current === value ? {} : { 'aria-hidden': true, inert: true }),
  });

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="lg"
      purpose="info"
      scrollBody
      title={copy.title(workspace)}
      footer={<Button onClick={onClose}>{copy.done}</Button>}
    >
      <Tabs
        value={current}
        onValueChange={(value) => setCurrent(value as WorkspaceSettingsTab)}
        className="py-3"
      >
        <TabsList aria-label={copy.tabsLabel} onKeyDown={leaveTabListBackward}>
          <TabsTrigger value="general">{copy.tabs.general}</TabsTrigger>
          <TabsTrigger value="people">{copy.tabs.people}</TabsTrigger>
          <TabsTrigger value="privacy">{copy.tabs.privacy}</TabsTrigger>
          {agentAccess !== undefined ? (
            <TabsTrigger value="agent-access">{copy.tabs.agentAccess}</TabsTrigger>
          ) : null}
        </TabsList>
        <div className="crew-settings-panels">
          <TabsContent {...panel('general')}>
            <GeneralTab view={view} />
          </TabsContent>
          <TabsContent {...panel('people')}>
            <PeopleTab view={view} onConfirm={ask} />
          </TabsContent>
          <TabsContent {...panel('privacy')}>
            <PrivacyTab
              view={view}
              onConfirm={ask}
              onMakePrivate={() => {
                nestedFocus.remember();
                setMakingPrivate(true);
              }}
            />
          </TabsContent>
          {/* The access area's content mounts only while it is selected: it is not ours to run
              hidden. It shares the cell, so the dialog is at least as tall as the tallest of ours.
              It opens with the same caps label as every other tab; the access area's own heading
              repeated the tab's name, so `dialogs.css` hides it here (QA Q2-29, Q2-66). */}
          {agentAccess !== undefined ? (
            <TabsContent
              value="agent-access"
              className="crew-settings-panel"
              data-crew-tab="agent-access"
            >
              <TabLabel title={copy.tabs.agentAccess} />
              {agentAccess}
            </TabsContent>
          ) : null}
        </div>
      </Tabs>
      <DialogErrorNote source={SOURCE} className="mb-2" />

      {confirm ? (
        <CrewConfirmation
          confirm={confirm}
          onClose={() => {
            setConfirm(null);
            nestedFocus.restore();
          }}
        />
      ) : null}
      {makingPrivate && crew.connection ? (
        <MakePrivateDialog
          connection={savedConnection(view) ?? crew.connection}
          onClose={() => {
            setMakingPrivate(false);
            nestedFocus.restore();
          }}
        />
      ) : null}
    </ModalShell>
  );
}

/**
 * Shift+Tab inside the tab list. Radix's roving group answers it by making the list untabbable for
 * a moment so the browser can move focus backward — but nothing in this dialog comes before the
 * list, so the dialog's focus trap caught the escaping focus and put it straight back on the
 * selected tab, and Shift+Tab looped there forever. Move it where a focus trap wraps to instead:
 * the dialog's last control (its ×).
 */
function leaveTabListBackward(event: React.KeyboardEvent<HTMLElement>) {
  if (event.key !== 'Tab' || !event.shiftKey || event.defaultPrevented) return;
  const dialog = event.currentTarget.closest('[role="dialog"], [role="alertdialog"]');
  if (!dialog) return;
  const list = event.currentTarget;
  const before = tabbables(dialog).filter(
    (element) =>
      !list.contains(element) &&
      element.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING
  );
  // Something before the list takes focus the ordinary way; only a list that is first wraps.
  if (before.length > 0) return;
  const last = tabbables(dialog)
    .filter((element) => !list.contains(element))
    .pop();
  if (!last) return;
  event.preventDefault();
  last.focus();
}

const TABBABLE =
  'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

/** The dialog's keyboard stops, in document order: enabled, not hidden, not inert. */
function tabbables(root: Element): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(TABBABLE)).filter(
    (element) =>
      !element.hasAttribute('disabled') &&
      element.getAttribute('tabindex') !== '-1' &&
      element.closest('[inert], [aria-hidden="true"], [hidden]') === null
  );
}

function savedConnection({ crew }: DialogView) {
  return crew.connections.find((item) => item.id === crew.connectionId) ?? null;
}

/** A label and its value, one settings row. */
function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-4 px-3 py-2.5">
      <span className="text-label text-text-default">{label}</span>
      <div className="flex min-w-0 items-center gap-2 text-body text-text-muted">{children}</div>
    </div>
  );
}

/**
 * The caps label every tab opens with — GENERAL, PEOPLE, PRIVACY, AGENT ACCESS — so no tab starts
 * with a different kind of heading, or with none (QA Q2-29).
 */
function TabLabel({ title, id, action }: { title: string; id?: string; action?: React.ReactNode }) {
  return (
    <div className="biorouter-settings-section-header flex min-w-0 items-center justify-between gap-3">
      <h3 id={id} className="text-caps text-text-muted">
        {title}
      </h3>
      {action}
    </div>
  );
}

/** A group of people rows under its own caps label: a real list, so it is announced as one. */
function Section({ title, children }: { title: string; children: React.ReactNode }) {
  const headingId = React.useId();
  return (
    <section className="biorouter-settings-section" aria-labelledby={headingId}>
      <div className="biorouter-settings-section-header flex min-w-0 items-center justify-between gap-3">
        <h4 id={headingId} className="text-caps text-text-muted">
          {title}
        </h4>
      </div>
      <ul role="list" className="biorouter-settings-list">
        {children}
      </ul>
    </section>
  );
}

function GeneralTab({ view }: { view: DialogView }) {
  const { crew, dir, snapshot, server } = view;
  const canRename = dir.viewerIsHost && uniqueNamesSupported(snapshot, crew.capabilities);
  return (
    <div className="flex flex-col">
      <TabLabel title={copy.tabs.general} />
      <div className="biorouter-settings-list">
        <Row label={copy.hostedBy}>
          <PersonName person={dir.host} context="inline" dir={dir} />
        </Row>
        <Row label={copy.server}>
          <span className="truncate" translate="no">
            {server}
          </span>
        </Row>
      </div>
      {canRename && snapshot ? (
        <div className="biorouter-settings-control-strip mt-3">
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              crew.openDialog({
                kind: 'rename',
                target: 'workspace',
                targetId: snapshot.workspace.id,
              })
            }
          >
            {copy.rename}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function PeopleTab({
  view,
  onConfirm,
}: {
  view: DialogView;
  onConfirm(intent: ConfirmIntent): void;
}) {
  const { crew, dir, snapshot, workspace } = view;
  const isHost = dir.viewerIsHost;
  const waiting = isHost ? (snapshot?.pending_joins ?? []) : [];
  const people = React.useMemo(() => peopleInOrder(dir.people), [dir.people]);
  const others = people.filter((person) => !person.isYou);

  return (
    <div className="flex flex-col">
      <TabLabel
        title={copy.tabs.people}
        action={
          isHost ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => crew.openDialog({ kind: 'invite-people' })}
            >
              {copy.invite}
            </Button>
          ) : undefined
        }
      />
      {waiting.length > 0 ? (
        <Section title={copy.waiting}>
          {waiting.map((join) => (
            <WaitingRow key={join.username} join={join} view={view} />
          ))}
        </Section>
      ) : null}
      <Section title={copy.members}>
        {people.map((person) => (
          <MemberRow
            key={person.id ?? person.username}
            person={person}
            view={view}
            canRemove={isHost && !person.isYou && !person.isHost}
            onRemove={() =>
              person.id && onConfirm({ action: 'remove-person', principalId: person.id })
            }
          />
        ))}
      </Section>
      {others.length === 0 ? (
        <Note tone="neutral" className="mt-2">
          {copy.noMembers(workspace)}
        </Note>
      ) : null}
    </div>
  );
}

function MemberRow({
  person,
  view,
  canRemove,
  onRemove,
}: {
  person: CrewPerson;
  view: DialogView;
  canRemove: boolean;
  onRemove(): void;
}) {
  const { dir, workspace } = view;
  return (
    <li className="biorouter-settings-row flex min-w-0 items-center gap-3 px-3 py-2">
      <Avatar
        size={24}
        fallback={person.avatar}
        name={person.displayName}
        username={person.username}
      />
      <div className="min-w-0 flex-1 truncate text-label">
        <PersonName person={person} context="header" dir={dir} you={person.isYou} />
      </div>
      {person.isHost ? (
        // What "Host" means, on hover and on focus (QA Q2-69): the badge takes a tab stop only
        // for the one row that has it.
        <Tooltip>
          <TooltipTrigger asChild>
            <span tabIndex={0} className="inline-flex rounded-element">
              <Badge tone="neutral" variant="badge">
                {copy.host}
              </Badge>
            </span>
          </TooltipTrigger>
          <TooltipContent>{copy.hostTooltip(workspace)}</TooltipContent>
        </Tooltip>
      ) : null}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            shape="round"
            aria-label={copy.memberOptions(personLabel(person, 'inline', dir))}
          >
            <MoreHorizontal aria-hidden />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => void copyText(person.username)}>
            {copy.copyUsername}
          </DropdownMenuItem>
          {person.id ? (
            <DropdownMenuItem onSelect={() => void copyText(person.id!)}>
              {copy.copyPersonId}
            </DropdownMenuItem>
          ) : null}
          {canRemove ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem variant="destructive" onSelect={onRemove}>
                {copy.removeFrom(workspace)}
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </li>
  );
}

function WaitingRow({ join, view }: { join: PendingJoin; view: DialogView }) {
  const { crew } = view;
  const [confirming, setConfirming] = React.useState(false);
  const person = joinerPerson(join.username, join.full_name);
  const key = `mutate:enrollment.cancel:${join.username}`;
  const pending = crew.isPending(key);
  const mismatched = (join.mismatched_attempts ?? 0) > 0;
  // The broker's word, not the local clock: an expired join can only be invited again.
  const expired = join.expired === true;
  const dismissOwnError = useDismissOwnError(SOURCE);

  const cancel = () =>
    void crew
      .act(SOURCE, key, async () => {
        await crew.request('enrollment.cancel', { username: join.username }, { mutation: true });
        return true as const;
      })
      .then((done) => {
        if (done === true) setConfirming(false);
      });

  return (
    <li className="biorouter-settings-row flex min-w-0 flex-col gap-1.5 px-3 py-2">
      <div className="flex min-w-0 items-center gap-3">
        <div className="min-w-0 flex-1 truncate text-label">
          <PersonName person={person} context="joiner" />
        </div>
        {expired ? (
          <div className="flex items-center gap-2 text-supporting text-text-muted">
            <span>{sidebarCopy.waiting.expired}</span>
            <span aria-hidden="true">{` ${sidebarCopy.waiting.separator} `}</span>
            <Button
              variant="secondary"
              size="sm"
              aria-label={sidebarCopy.waiting.inviteAgainLabel(join.username)}
              onClick={() => crew.openDialog({ kind: 'invite-people' })}
            >
              {sidebarCopy.waiting.inviteAgain}
            </Button>
          </div>
        ) : confirming ? (
          <div className="flex items-center gap-2">
            <span className="text-supporting text-text-muted">
              {copy.cancelInvitationConfirm(join.username)}
            </span>
            <Button variant="destructive" size="sm" disabled={pending} onClick={cancel}>
              {copy.cancelInvitation}
            </Button>
            <Button
              // A destructive confirmation holds focus on the safe answer.
              autoFocus
              variant="ghost"
              size="sm"
              disabled={pending}
              onClick={() => setConfirming(false)}
            >
              {copy.keepInvitation}
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              aria-label={copy.cancelInvitationLabel(join.username)}
              onClick={() => {
                dismissOwnError();
                setConfirming(true);
              }}
            >
              {copy.cancelInvitation}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              aria-label={copy.letInLabel(join.username)}
              onClick={() => crew.openDialog({ kind: 'let-in', username: join.username })}
            >
              {copy.letIn}
            </Button>
          </div>
        )}
      </div>
      {mismatched ? (
        <p className="flex items-center gap-1.5 text-supporting text-text-warning">
          <AlertTriangle aria-hidden className="h-icon-row w-icon-row shrink-0" />
          {copy.otherDevice(join.username)}
        </p>
      ) : null}
    </li>
  );
}

function PrivacyTab({
  view,
  onConfirm,
  onMakePrivate,
}: {
  view: DialogView;
  onConfirm(intent: ConfirmIntent): void;
  onMakePrivate(): void;
}) {
  const { crew, dir, snapshot, workspace } = view;
  const connection = crew.connection;
  const saved = savedConnection(view);
  const effectId = React.useId();
  const isHost = dir.viewerIsHost;
  const workspaceMode = snapshot?.workspace.mode ?? null;
  const workspaceInstitution = snapshot?.workspace.institution_id ?? null;
  const connectionInstitution = saved?.institution_id ?? connection?.institution_id ?? null;
  const verified = crew.snapshot !== null && crew.observedPrivacy !== null;
  const makePrivateKey = 'connection.update';

  const makePrivate = () => {
    if (!saved) return;
    // Without an institution a Private connection cannot be saved: ask for it first.
    if (!saved.institution_id) {
      onMakePrivate();
      return;
    }
    void crew.act(SOURCE, makePrivateKey, async () => {
      await crew.updateConnection(saved.id, {
        ...connectionUpdateBody(saved),
        mode: 'private',
      });
      await crew.refresh();
    });
  };

  // The popover's one line saying what "Make my connection public…" changes, under the same words.
  const publicEffect =
    connection?.mode === 'private' && workspaceMode
      ? sidebarCopy.privacy.makePublicEffect(workspace, workspaceMode)
      : null;

  return (
    <div className="flex flex-col">
      <TabLabel title={copy.tabs.privacy} />
      <div className="biorouter-settings-list">
        <Row label={copy.yourConnection}>
          {connection ? (
            <>
              <PrivacyBadge tier={connection.mode} enforcementOff={false} />
              {connection.mode === 'private' && connectionInstitution ? (
                <span>
                  {'· '}
                  <InstitutionName id={connectionInstitution} />
                </span>
              ) : null}
              {connection.mode === 'private' ? (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!saved}
                  aria-describedby={publicEffect ? effectId : undefined}
                  onClick={() =>
                    saved && onConfirm({ action: 'make-connection-public', connectionId: saved.id })
                  }
                >
                  {copy.makePublic}
                </Button>
              ) : (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!saved || crew.isPending(makePrivateKey)}
                  onClick={makePrivate}
                >
                  {copy.makePrivate}
                </Button>
              )}
            </>
          ) : null}
        </Row>
        <Row label={copy.workspace}>
          {workspaceMode ? (
            <span>{workspaceMode === 'private' ? copy.privateForEveryone : copy.allowsPublic}</span>
          ) : null}
          {isHost && workspaceMode ? (
            <Button
              variant="secondary"
              size="sm"
              disabled={!verified}
              onClick={() =>
                onConfirm(
                  workspaceMode === 'private'
                    ? { action: 'allow-workspace-public' }
                    : { action: 'make-workspace-private' }
                )
              }
            >
              {workspaceMode === 'private' ? copy.allowPublic : copy.makePrivateForEveryone}
            </Button>
          ) : null}
        </Row>
        <Row label={copy.institution}>
          {workspaceInstitution ? (
            <InstitutionName id={workspaceInstitution} />
          ) : (
            <span>{copy.notSet}</span>
          )}
          {!workspaceInstitution && isHost && connectionInstitution ? (
            <Button
              variant="secondary"
              size="sm"
              disabled={!verified}
              onClick={() =>
                onConfirm({ action: 'set-institution', institutionId: connectionInstitution })
              }
            >
              {copy.setInstitution(connectionInstitution)}
            </Button>
          ) : null}
        </Row>
      </div>
      {publicEffect ? (
        <p id={effectId} className="mt-2 px-3 text-supporting text-text-muted">
          {publicEffect}
        </p>
      ) : null}
      {!workspaceInstitution && isHost && !connectionInstitution ? (
        <p className="mt-2 px-3 text-supporting text-text-muted">
          {copy.institutionNeedsConnection}
        </p>
      ) : null}
      {!isHost ? (
        <p className="mt-2 px-3 text-supporting text-text-muted">{copy.hostOnly}</p>
      ) : null}
    </div>
  );
}
