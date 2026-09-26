import { useId, useState, type FormEvent } from 'react';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { Separator } from '../../ui/separator';
import { INSTITUTION_ID_PATTERN, institutionId, personLabel } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { sidebarCopy } from './copy';
import { keepNamesWhole, useSidebarView, type VerifiedPrivacy } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy.privacy;

/** The pending-action key a connection privacy change runs under. */
export const PRIVACY_UPDATE_KEY = 'connection.update';

/**
 * The privacy popover (ui-redesign-spec, wireframe "Privacy popover", copy deck "Privacy
 * popover"): the badge, what the chip means in one sentence, the three facts behind it, then ONE
 * short 12px note (Q2-44, Q3-54, Q4-51) — why the mode is what it is, then, for a member, "Only the
 * host can change {workspace}. Only people {host} lets in can see it.", and for the host "Only
 * people you let in can see {workspace}.": who can see the workspace at all, because "Private" is
 * about models, never about people. It was two paragraphs, which read as more than it says. The
 * why is always shown, so a joiner who sets Public in a Private-for-everyone workspace sees why
 * nothing changed.
 *
 * The workspace's name never breaks inside a sentence here (Q4-51: "…can see chen-" / "lab."):
 * every sentence that names it goes through `keepNamesWhole`.
 *
 * It is named by its title row — `Privacy: Private · UCSF`, the chip's own name — which stays
 * at the top through the institution step, so the popover never loses its name (T-38).
 *
 * The two changes it offers are deliberately asymmetric ("Privacy and institution"):
 *
 * - **Make my connection public…** is offered only where it changes something: a workspace that
 *   allows Public (Q3-54). In a Private-for-everyone workspace its own description said the models
 *   that can read it "stay the same" — a control that changes nothing, offered first — so there
 *   **Privacy…** is the one action (the settings tab still holds the connection's own mode). Where
 *   it is offered it exposes data, so it is a secondary link in the same ink as Privacy…, never
 *   the popover's most prominent control (Q2-44), and it only opens the typed confirmation (the
 *   workspace name as the phrase). The line under it, which is also its description, says what it
 *   changes: only this connection, and which models could then read what — checked against the
 *   broker, which refuses a public model a Restricted channel but never a person, so nobody loses
 *   a channel. The dialog, not this popover, sends the change.
 * - **Make private** is one click: the full-body PATCH (L18) and a refresh. A connection with no
 *   institution cannot be Private (the daemon refuses the save), so the popover first asks for
 *   one, in place, as a required field. Like the downgrade it is offered only in a workspace that
 *   allows Public: in a Private-for-everyone one the chip already reads Private, and "Make private"
 *   beside it would change nothing there either.
 *
 * "Privacy…" is the link to the rest, in Workspace settings. React authorizes nothing: the daemon
 * decides the save, and the chip changes only when the observer verifies the new mode.
 */
export function PrivacyPopover({
  privacy,
  titleId,
  onClose,
}: {
  privacy: VerifiedPrivacy;
  /** The id the popover's `aria-labelledby` names: this component renders that title. */
  titleId: string;
  onClose(): void;
}) {
  const crew = useCrew();
  const { title, dir } = useSidebarView(crew);
  const [askInstitution, setAskInstitution] = useState(false);
  const pending = crew.isPending(PRIVACY_UPDATE_KEY);
  const institution = privacy.effective === 'private' ? privacy.institution : null;

  const makePrivate = (institutionValue?: string) => {
    const connection = crew.connection;
    if (!connection) return;
    const body = connectionUpdateBody(connection);
    onClose();
    void crew.act('global', PRIVACY_UPDATE_KEY, async () => {
      await crew.updateConnection(crew.connectionId, {
        ...body,
        mode: 'private',
        institution_id: institutionValue ?? body.institution_id,
      });
      await crew.refresh();
    });
  };

  const onMakePrivate = () => {
    if (institutionId(crew.connection?.institution_id) === null) setAskInstitution(true);
    else makePrivate();
  };

  const onMakePublic = () => {
    onClose();
    crew.openDialog({
      kind: 'confirm',
      confirm: { action: 'make-connection-public', connectionId: crew.connectionId },
    });
  };

  const onMore = () => {
    onClose();
    crew.openDialog({ kind: 'workspace-settings', tab: 'privacy' });
  };

  const heading = (
    <div id={titleId} className="flex min-w-0 items-center gap-1" data-crew-privacy-title="">
      <span className="sr-only">{copy.titlePrefix}</span>{' '}
      <span className="crew-sidebar-chip-badge">
        <PrivacyBadge tier={privacy.effective} enforcementOff={false} />
        {institution && (
          <span className="crew-sidebar-chip-institution">
            {' · '}
            <bdi translate="no" className="crew-sidebar-truncate">
              {institution}
            </bdi>
          </span>
        )}
      </span>
    </div>
  );

  if (askInstitution) {
    return (
      <div className="flex flex-col gap-2 p-1" data-crew-privacy-popover="">
        {heading}
        <InstitutionStep onCancel={() => setAskInstitution(false)} onSubmit={makePrivate} />
      </div>
    );
  }

  const host = crew.isHost ? null : dir.host ? personLabel(dir.host, 'inline', dir) : null;
  const hostOnly = crew.isHost ? null : copy.hostOnly(title);
  // "it" only straight after "Only the host can change {workspace}.", whose one noun it points
  // back to; after the host's why, which may name the connection too, the workspace is named.
  const audience =
    crew.isHost || host ? (hostOnly ? copy.audienceIt(host) : copy.audience(title, host)) : null;
  const names = [title];
  // Either change moves which models may read this person's channels only where the workspace
  // allows Public; in a Private-for-everyone one neither changes anything there, so Privacy… is
  // the one action (Q3-54).
  const workspaceAllowsPublic = privacy.workspaceMode === 'public';
  const offerPublic = workspaceAllowsPublic && privacy.connectionMode === 'private';
  const offerPrivate = workspaceAllowsPublic && privacy.connectionMode === 'public';
  const effectId = `${titleId}-make-public`;

  return (
    <div className="flex flex-col gap-3 p-1" data-crew-privacy-popover="">
      {heading}
      <p className="text-secondary text-text-default">
        {keepNamesWhole(
          privacy.effective === 'private' ? copy.private(title, institution) : copy.public(title),
          names
        )}
      </p>
      <Separator className="bg-border-subtle" />
      <dl className="crew-sidebar-privacy-facts text-secondary">
        <dt className="text-text-muted">{copy.rows.connection}</dt>
        <dd>{privacy.connectionMode === 'private' ? copy.values.private : copy.values.public}</dd>
        <dt className="text-text-muted">{copy.rows.workspace}</dt>
        <dd>
          {privacy.workspaceMode === 'private'
            ? copy.values.workspacePrivate
            : copy.values.workspacePublic}
        </dd>
        <dt className="text-text-muted">{copy.rows.institution}</dt>
        <dd className="min-w-0">
          {privacy.institution ? (
            <bdi translate="no">{privacy.institution}</bdi>
          ) : (
            copy.values.notSet
          )}
        </dd>
      </dl>
      <Separator className="bg-border-subtle" />
      <p className="text-supporting text-text-muted" data-crew-privacy-why={privacy.why}>
        {keepNamesWhole(copy.why[privacy.why](title), names)}
        {hostOnly && <> {keepNamesWhole(hostOnly, names)}</>}
        {audience && (
          <>
            {' '}
            <span data-crew-privacy-audience="">{keepNamesWhole(audience, names)}</span>
          </>
        )}
      </p>
      <div className="flex flex-col items-start gap-1">
        <div className="flex w-full flex-wrap items-center justify-between gap-2">
          {offerPublic ? (
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-supporting"
              disabled={pending}
              aria-describedby={effectId}
              data-crew-privacy-downgrade=""
              onClick={onMakePublic}
            >
              {copy.makePublic}
            </Button>
          ) : offerPrivate ? (
            <Button variant="outline" size="sm" disabled={pending} onClick={onMakePrivate}>
              {copy.makePrivate}
            </Button>
          ) : null}
          <Button variant="link" size="sm" className="h-auto p-0 text-supporting" onClick={onMore}>
            {copy.more}
          </Button>
        </div>
        {offerPublic && (
          <p id={effectId} className="text-supporting text-text-muted" data-crew-privacy-effect="">
            {keepNamesWhole(copy.makePublicEffect(title, privacy.workspaceMode), names)}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * Making a connection Private without an institution: the daemon refuses a Private save that has
 * none, so the popover asks for it first — required, with the same pattern the daemon and the
 * broker judge an institution ID by.
 */
function InstitutionStep({
  onCancel,
  onSubmit,
}: {
  onCancel(): void;
  onSubmit(institution: string): void;
}) {
  const [value, setValue] = useState('');
  const id = useId();
  const helpId = `${id}-help`;
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const trimmed = value.trim();
    if (trimmed) onSubmit(trimmed);
  };
  return (
    <form className="flex flex-col gap-2" onSubmit={submit}>
      <label className="text-label" htmlFor={id}>
        {copy.institutionField}
      </label>
      <Input
        id={id}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        required
        pattern={INSTITUTION_ID_PATTERN}
        maxLength={64}
        autoComplete="off"
        spellCheck={false}
        placeholder={copy.institutionPlaceholder}
        aria-describedby={value ? undefined : helpId}
        autoFocus
      />
      {!value && (
        <p id={helpId} className="text-supporting text-text-muted">
          {copy.institutionHelp}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          {copy.cancel}
        </Button>
        <Button type="submit" size="sm">
          {copy.makePrivate}
        </Button>
      </div>
    </form>
  );
}
