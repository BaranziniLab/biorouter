import { useId, useState, type FormEvent } from 'react';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { Separator } from '../../ui/separator';
import { INSTITUTION_ID_PATTERN, institutionId } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { sidebarCopy } from './copy';
import { useSidebarView, type VerifiedPrivacy } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy.privacy;

/** The pending-action key a connection privacy change runs under. */
export const PRIVACY_UPDATE_KEY = 'connection.update';

/**
 * The privacy popover (ui-redesign-spec, wireframe "Privacy popover", copy deck "Privacy
 * popover"): what the chip means in one sentence, the three facts behind it, and the "why" line,
 * always shown, so a joiner who sets Public in a Private-for-everyone workspace sees why nothing
 * changed.
 *
 * The two changes it offers are deliberately asymmetric ("Privacy and institution"):
 *
 * - **Make public…** exposes data, so it only opens the typed confirmation (the workspace name as
 *   the phrase). The dialog, not this popover, sends the change.
 * - **Make private** is one click: the full-body PATCH (L18) and a refresh. A connection with no
 *   institution cannot be Private (the daemon refuses the save), so the popover first asks for
 *   one, in place, as a required field.
 *
 * React authorizes nothing: the daemon decides the save, and the chip changes only when the
 * observer verifies the new mode.
 */
export function PrivacyPopover({
  privacy,
  onClose,
}: {
  privacy: VerifiedPrivacy;
  onClose(): void;
}) {
  const crew = useCrew();
  const { title } = useSidebarView(crew);
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

  if (askInstitution) {
    return <InstitutionStep onCancel={() => setAskInstitution(false)} onSubmit={makePrivate} />;
  }

  const showHostOnly = !crew.isHost && (privacy.why === 'workspace' || privacy.why === 'both');

  return (
    <div className="flex flex-col gap-3 p-1" data-crew-privacy-popover="">
      <div className="flex min-w-0 items-center gap-1">
        <PrivacyBadge tier={privacy.effective} enforcementOff={false} />
        {institution && (
          <span className="crew-sidebar-chip-text">
            {' · '}
            <bdi translate="no" className="crew-sidebar-truncate">
              {institution}
            </bdi>
          </span>
        )}
      </div>
      <p className="text-secondary text-text-default">
        {privacy.effective === 'private' ? copy.private(title, institution) : copy.public(title)}
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
        {copy.why[privacy.why]}
      </p>
      {showHostOnly && <p className="text-supporting text-text-muted">{copy.hostOnly}</p>}
      <div className="flex items-center justify-between gap-2">
        {privacy.connectionMode === 'private' ? (
          <Button variant="outline" size="sm" disabled={pending} onClick={onMakePublic}>
            {copy.makePublic}
          </Button>
        ) : (
          <Button variant="outline" size="sm" disabled={pending} onClick={onMakePrivate}>
            {copy.makePrivate}
          </Button>
        )}
        <Button variant="ghost" size="sm" onClick={onMore}>
          {copy.more}
        </Button>
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
    <form className="flex flex-col gap-2 p-1" onSubmit={submit}>
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
