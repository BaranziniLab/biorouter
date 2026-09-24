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
import { AlertTriangle, MoreHorizontal } from '../../icons/app-icons';
import type { PendingJoin } from '../crewApi';
import {
  InstitutionName,
  joinerPerson,
  PersonName,
  personLabel,
  type CrewPerson,
} from '../identity';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ConfirmIntent, ErrorSource, WorkspaceSettingsTab } from '../state/types';
import { copyText } from './clipboard';
import { CrewConfirmation } from './confirmations';
import { workspaceSettingsCopy as copy } from './copy';
import { DialogErrorNote, useDismissOwnError } from './fields';
import { MakePrivateDialog } from './MakePrivateDialog';
import { uniqueNamesSupported, useDialogView, type DialogView } from './workspace';

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
 * confirmations of the copy deck, opened over this dialog so Cancel returns here.
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

  const ask = (intent: ConfirmIntent) => {
    dismissOwnError();
    setConfirm(intent);
  };

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
        className="pb-2"
      >
        <TabsList>
          <TabsTrigger value="general">{copy.tabs.general}</TabsTrigger>
          <TabsTrigger value="people">{copy.tabs.people}</TabsTrigger>
          <TabsTrigger value="privacy">{copy.tabs.privacy}</TabsTrigger>
          {agentAccess !== undefined ? (
            <TabsTrigger value="agent-access">{copy.tabs.agentAccess}</TabsTrigger>
          ) : null}
        </TabsList>
        <TabsContent value="general">
          <GeneralTab view={view} />
        </TabsContent>
        <TabsContent value="people">
          <PeopleTab view={view} onConfirm={ask} />
        </TabsContent>
        <TabsContent value="privacy">
          <PrivacyTab view={view} onConfirm={ask} onMakePrivate={() => setMakingPrivate(true)} />
        </TabsContent>
        {agentAccess !== undefined ? (
          <TabsContent value="agent-access">{agentAccess}</TabsContent>
        ) : null}
      </Tabs>
      <DialogErrorNote source={SOURCE} className="mb-2" />

      {confirm ? <CrewConfirmation confirm={confirm} onClose={() => setConfirm(null)} /> : null}
      {makingPrivate && crew.connection ? (
        <MakePrivateDialog
          connection={savedConnection(view) ?? crew.connection}
          onClose={() => setMakingPrivate(false)}
        />
      ) : null}
    </ModalShell>
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

function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  const headingId = React.useId();
  return (
    <section className="biorouter-settings-section" aria-labelledby={headingId}>
      <div className="biorouter-settings-section-header flex min-w-0 items-center justify-between gap-3">
        <h3 id={headingId} className="text-caps text-text-muted">
          {title}
        </h3>
        {action}
      </div>
      <div className="biorouter-settings-list">{children}</div>
    </section>
  );
}

function GeneralTab({ view }: { view: DialogView }) {
  const { crew, dir, snapshot, server } = view;
  const canRename = dir.viewerIsHost && uniqueNamesSupported(snapshot);
  return (
    <div className="flex flex-col">
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
  const people = dir.people;
  const others = people.filter((person) => !person.isYou);

  return (
    <div className="flex flex-col">
      {waiting.length > 0 ? (
        <Section title={copy.waiting}>
          {waiting.map((join) => (
            <WaitingRow key={join.username} join={join} view={view} />
          ))}
        </Section>
      ) : null}
      <Section
        title={copy.members}
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
      >
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
    <div className="biorouter-settings-row flex min-w-0 items-center gap-3 px-3 py-2">
      <Avatar
        size={24}
        fallback={person.avatar}
        name={person.displayName}
        username={person.username}
      />
      <div className="min-w-0 flex-1 truncate">
        <PersonName person={person} context="header" dir={dir} you={person.isYou} />
      </div>
      {person.isHost ? (
        <Badge tone="neutral" variant="badge">
          {copy.host}
        </Badge>
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
    </div>
  );
}

function WaitingRow({ join, view }: { join: PendingJoin; view: DialogView }) {
  const { crew } = view;
  const [confirming, setConfirming] = React.useState(false);
  const person = joinerPerson(join.username, join.full_name);
  const key = `mutate:enrollment.cancel:${join.username}`;
  const pending = crew.isPending(key);
  const mismatched = (join.mismatched_attempts ?? 0) > 0;
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
    <div className="biorouter-settings-row flex min-w-0 flex-col gap-1.5 px-3 py-2">
      <div className="flex min-w-0 items-center gap-3">
        <div className="min-w-0 flex-1 truncate">
          <PersonName person={person} context="joiner" />
        </div>
        {confirming ? (
          <div className="flex items-center gap-2">
            <span className="text-supporting text-text-muted">
              {copy.cancelInvitationConfirm(join.username)}
            </span>
            <Button variant="destructive" size="sm" disabled={pending} onClick={cancel}>
              {copy.cancelInvitation}
            </Button>
            <Button
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
    </div>
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
  const { crew, dir, snapshot } = view;
  const connection = crew.connection;
  const saved = savedConnection(view);
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

  return (
    <div className="flex flex-col">
      <div className="biorouter-settings-list">
        <Row label={copy.yourConnection}>
          {connection ? (
            <>
              <PrivacyBadge tier={connection.mode} enforcementOff={false} />
              {connection.mode === 'private' ? (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!saved}
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
