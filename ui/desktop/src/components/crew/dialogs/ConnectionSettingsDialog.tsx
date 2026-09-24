import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { Switch } from '../../ui/switch';
import type { CrewConnection } from '../crewApi';
import { isInstitutionId } from '../identity';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ErrorSource, SaveConnectionInput } from '../state/types';
import { MakeConnectionPublicDialog, CrewConfirmation } from './confirmations';
import { connectionSettingsCopy as copy } from './copy';
import { DialogErrorNote, Field, helpId, RadioRows } from './fields';
import { groupedFingerprint, useWorkspaceKeyFingerprint } from './fingerprint';
import { ABSOLUTE_PATH_PATTERN, INSTITUTION_FIELD_PATTERN } from './nameRules';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:connection-settings';
const CONFIRM_SOURCE: ErrorSource = 'dialog:confirm';
const SAVE_KEY = 'connection.update';
const DEFAULT_PORT = 22;

/** The editable part of a saved connection, as the form holds it. */
interface ConnectionForm {
  name: string;
  ssh_target: string;
  port: string;
  identity_file: string;
  proxy_jump: string;
  remote_root: string;
  remote_execution: boolean;
  mode: 'private' | 'public';
  /** Kept while the field is hidden, so toggling the mode never loses what was typed. */
  institution: string;
}

type AdvancedField = 'port' | 'remote_root';

function formFrom(connection: CrewConnection): ConnectionForm {
  return {
    name: connection.name,
    ssh_target: connection.ssh_target,
    port: connection.port ? String(connection.port) : '',
    identity_file: connection.identity_file ?? '',
    proxy_jump: connection.proxy_jump ?? '',
    remote_root: connection.remote_root ?? '',
    remote_execution: connection.remote_execution,
    mode: connection.mode,
    institution: connection.institution_id ?? '',
  };
}

/** Advanced opens by itself when the saved record already uses anything inside it. */
export function advancedInUse(connection: CrewConnection): boolean {
  return Boolean(
    (connection.port && connection.port !== DEFAULT_PORT) ||
    connection.identity_file ||
    connection.proxy_jump ||
    connection.remote_root ||
    connection.remote_execution
  );
}

function portProblem(port: string): boolean {
  if (!port.trim()) return false;
  const value = Number(port);
  return !Number.isInteger(value) || value < 1 || value > 65535;
}

/** The first Advanced field the browser would refuse, checked while Advanced is closed. */
function hiddenProblem(form: ConnectionForm): AdvancedField | null {
  if (portProblem(form.port)) return 'port';
  if (form.remote_root.trim() && !form.remote_root.trim().startsWith('/')) return 'remote_root';
  return null;
}

/**
 * The institution to save. Private requires the typed value (the field is required and
 * pattern-checked). Public hides the field: an emptied value clears it, a well-formed one is kept,
 * and anything else keeps the saved value rather than sending text the person can no longer see.
 */
function institutionToSave(form: ConnectionForm, saved: CrewConnection): string | null {
  const typed = form.institution.trim();
  if (form.mode === 'private') return typed || null;
  if (!typed) return null;
  return isInstitutionId(typed) ? typed : (saved.institution_id ?? null);
}

/** The full body the PATCH sends (L18: the daemon replaces the whole record). */
function connectionSettingsBody(saved: CrewConnection, form: ConnectionForm): SaveConnectionInput {
  const root = form.remote_root.trim();
  return {
    ...connectionUpdateBody(saved),
    name: form.name.trim(),
    ssh_target: form.ssh_target.trim(),
    port: form.port.trim() ? Number(form.port) : undefined,
    identity_file: form.identity_file.trim() || undefined,
    proxy_jump: form.proxy_jump.trim() || undefined,
    remote_root: root || undefined,
    remote_execution: root ? form.remote_execution : false,
    mode: form.mode,
    institution_id: institutionToSave(form, saved),
  };
}

function advancedSummary(form: ConnectionForm): string {
  const parts = [
    copy.summaryPort(form.port.trim() || String(DEFAULT_PORT)),
    form.identity_file.trim() ? copy.summaryIdentity : copy.summarySshDefaults,
  ];
  if (form.proxy_jump.trim()) parts.push(copy.summaryJump(form.proxy_jump.trim()));
  if (form.remote_root.trim()) parts.push(copy.summaryFolder(form.remote_root.trim()));
  return copy.advancedSummary(parts);
}

export interface ConnectionSettingsDialogProps {
  connectionId: string;
  /** Called when the dialog is dismissed, saved, or its connection is removed. */
  onClose(): void;
}

/**
 * Connection settings (ui-redesign-spec, "Dialog inventory", "Progressive disclosure"): editing a
 * saved connection, titled as an edit and saved with **Save connection** (L1, pinned).
 *
 * - A real `<form>` with a `type="submit"` button and no `noValidate`, so the browser's own
 *   validation refuses an incomplete connection before any request (C8). Institution is required
 *   exactly when the connection is Private, and survives toggling the mode.
 * - Advanced holds the optional fields and opens by itself when the record already uses one; a
 *   submit that would fail on a hidden field opens it and focuses that field.
 * - Saving a Private connection as Public first asks for the workspace's name
 *   (`DangerousConfirmDialog`); Cancel there sends nothing. The PATCH carries the whole record.
 * - The workspace's identity (ID, fingerprint, socket, host user ID, device and cluster IDs) is
 *   read-only here: it is what the connection was pinned to, not a preference.
 */
export function ConnectionSettingsDialog({ connectionId, onClose }: ConnectionSettingsDialogProps) {
  const { crew } = useDialogView(connectionId);
  const saved = crew.connections.find((item) => item.id === connectionId) ?? null;
  useCloseWhenMissing(saved === null, onClose);
  if (!saved) return null;
  return <ConnectionSettingsForm saved={saved} onClose={onClose} />;
}

function ConnectionSettingsForm({ saved, onClose }: { saved: CrewConnection; onClose(): void }) {
  const { crew, workspace, phrase } = useDialogView(saved.id);
  const formId = React.useId();
  const ids = {
    name: `${formId}-name`,
    login: `${formId}-login`,
    institution: `${formId}-institution`,
    port: `${formId}-port`,
    identity: `${formId}-identity`,
    jump: `${formId}-jump`,
    root: `${formId}-root`,
    execution: `${formId}-execution`,
  };
  const [form, setForm] = React.useState<ConnectionForm>(() => formFrom(saved));
  const [advancedOpen, setAdvancedOpen] = React.useState(() => advancedInUse(saved));
  const [focusField, setFocusField] = React.useState<AdvancedField | null>(null);
  const [institutionInvalid, setInstitutionInvalid] = React.useState(false);
  const [confirmBody, setConfirmBody] = React.useState<SaveConnectionInput | null>(null);
  const [removing, setRemoving] = React.useState(false);
  const saving = crew.isPending(SAVE_KEY);
  const fingerprint = useWorkspaceKeyFingerprint(saved.workspace_public_key);

  const update = <K extends keyof ConnectionForm>(key: K, value: ConnectionForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  // A hidden field that would fail: open Advanced, then focus the field once it has mounted.
  React.useEffect(() => {
    if (!focusField || !advancedOpen) return;
    const node = document.getElementById(focusField === 'port' ? ids.port : ids.root);
    if (node instanceof HTMLInputElement) {
      node.focus();
      node.reportValidity();
      setFocusField(null);
    }
  }, [focusField, advancedOpen, ids.port, ids.root]);

  const save = (body: SaveConnectionInput, source: ErrorSource) =>
    crew
      .act(source, SAVE_KEY, async () => {
        await crew.updateConnection(saved.id, body);
        // A privacy change takes effect through a fresh observation of this connection.
        const privacyChanged =
          body.mode !== saved.mode ||
          (body.institution_id ?? null) !== (saved.institution_id ?? null);
        if (privacyChanged && saved.id === crew.connectionId) await crew.refresh();
        return true as const;
      })
      .then((done) => {
        if (done === true) onClose();
      });

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const problem = advancedOpen ? null : hiddenProblem(form);
    if (problem) {
      setAdvancedOpen(true);
      setFocusField(problem);
      return;
    }
    const body = connectionSettingsBody(saved, form);
    // Exposing a Private connection asks for the workspace's name first, from every path.
    if (saved.mode === 'private' && body.mode === 'public') {
      crew.dismissError();
      setConfirmBody(body);
      return;
    }
    void save(body, SOURCE);
  };

  const root = form.remote_root.trim();
  const institutionError = institutionInvalid ? copy.institutionPattern : undefined;
  const institutionHelper = form.institution ? undefined : copy.institutionHelper;

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose={saving ? 'required' : 'form'}
      scrollBody
      title={copy.title}
      footer={
        <>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="mr-auto text-text-danger"
            disabled={saving}
            onClick={() => {
              crew.dismissError();
              setRemoving(true);
            }}
          >
            {copy.remove(workspace)}
          </Button>
          <Button type="button" variant="outline" onClick={onClose} disabled={saving}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={saving}>
            {copy.save}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-4 pb-1">
        <Field id={ids.name} label={copy.name}>
          <Input
            id={ids.name}
            required
            maxLength={255}
            value={form.name}
            onChange={(event) => update('name', event.target.value)}
          />
        </Field>
        <Field id={ids.login} label={copy.login}>
          <Input
            id={ids.login}
            required
            autoComplete="off"
            spellCheck={false}
            placeholder={copy.loginPlaceholder}
            value={form.ssh_target}
            onChange={(event) => update('ssh_target', event.target.value)}
          />
        </Field>
        <RadioRows<'private' | 'public'>
          label={copy.privacy}
          name={`${formId}-mode`}
          value={form.mode}
          onChange={(mode) => update('mode', mode)}
          options={[
            { value: 'private', label: copy.private, detail: copy.privateDetail },
            { value: 'public', label: copy.public, detail: copy.publicDetail },
          ]}
        />
        {form.mode === 'private' ? (
          <Field
            id={ids.institution}
            label={copy.institution}
            helper={institutionHelper}
            error={institutionError}
          >
            <Input
              id={ids.institution}
              required
              maxLength={64}
              pattern={INSTITUTION_FIELD_PATTERN}
              autoComplete="off"
              spellCheck={false}
              translate="no"
              placeholder={copy.institutionPlaceholder}
              aria-invalid={institutionInvalid || undefined}
              aria-describedby={
                institutionError || institutionHelper ? helpId(ids.institution) : undefined
              }
              value={form.institution}
              onInvalid={(event) =>
                setInstitutionInvalid(event.currentTarget.validity.patternMismatch)
              }
              onChange={(event) => {
                setInstitutionInvalid(false);
                update('institution', event.target.value);
              }}
            />
          </Field>
        ) : null}

        <Disclosure
          label={copy.advanced}
          summary={advancedSummary(form)}
          open={advancedOpen}
          onOpenChange={setAdvancedOpen}
        >
          <div className="flex flex-col gap-4 pt-2">
            <Field
              id={ids.port}
              label={copy.port}
              error={portProblem(form.port) ? copy.portRange : undefined}
            >
              <Input
                id={ids.port}
                type="number"
                inputMode="numeric"
                min={1}
                max={65535}
                step={1}
                placeholder={String(DEFAULT_PORT)}
                aria-invalid={portProblem(form.port) || undefined}
                aria-describedby={portProblem(form.port) ? helpId(ids.port) : undefined}
                value={form.port}
                onChange={(event) => update('port', event.target.value)}
              />
            </Field>
            <Field id={ids.identity} label={copy.identityFile}>
              <Input
                id={ids.identity}
                autoComplete="off"
                spellCheck={false}
                placeholder={copy.identityFilePlaceholder}
                value={form.identity_file}
                onChange={(event) => update('identity_file', event.target.value)}
              />
            </Field>
            <Field id={ids.jump} label={copy.jumpHosts}>
              <Input
                id={ids.jump}
                autoComplete="off"
                spellCheck={false}
                placeholder={copy.jumpHostsPlaceholder}
                value={form.proxy_jump}
                onChange={(event) => update('proxy_jump', event.target.value)}
              />
            </Field>
            <Field id={ids.root} label={copy.remoteRoot}>
              <Input
                id={ids.root}
                autoComplete="off"
                spellCheck={false}
                pattern={ABSOLUTE_PATH_PATTERN}
                title={copy.remoteRootPattern}
                placeholder={copy.remoteRootPlaceholder}
                value={form.remote_root}
                onChange={(event) => update('remote_root', event.target.value)}
              />
            </Field>
            <div className="flex min-w-0 items-center justify-between gap-3">
              <label htmlFor={ids.execution} className="text-label text-text-default">
                {copy.remoteExecution}
                {!root ? (
                  <span className="mt-0.5 block text-supporting text-text-muted">
                    {copy.remoteExecutionNeedsFolder}
                  </span>
                ) : null}
              </label>
              <Switch
                id={ids.execution}
                checked={Boolean(root) && form.remote_execution}
                disabled={!root}
                onCheckedChange={(checked) => update('remote_execution', checked)}
              />
            </div>
            <WorkspaceDetails connection={saved} fingerprint={fingerprint} />
          </div>
        </Disclosure>

        <DialogErrorNote source={SOURCE} />
      </form>

      {confirmBody ? (
        <MakeConnectionPublicDialog
          workspace={workspace}
          phrase={phrase}
          busy={saving}
          onCancel={() => {
            crew.dismissError();
            setConfirmBody(null);
          }}
          onConfirm={() => void save(confirmBody, CONFIRM_SOURCE)}
        />
      ) : null}
      {removing ? (
        <CrewConfirmation
          confirm={{ action: 'remove-connection', connectionId: saved.id }}
          onClose={() => setRemoving(false)}
        />
      ) : null}
    </ModalShell>
  );
}

/** The pinned identity of the workspace, read-only, each value one click from the clipboard. */
function WorkspaceDetails({
  connection,
  fingerprint,
}: {
  connection: CrewConnection;
  fingerprint: string | null;
}) {
  const rows: { label: string; copyLabel: string; value: string; display?: string }[] = [
    { label: copy.workspaceId, copyLabel: 'workspace ID', value: connection.workspace_id },
    fingerprint
      ? {
          label: copy.fingerprint,
          copyLabel: 'fingerprint',
          value: fingerprint,
          display: groupedFingerprint(fingerprint),
        }
      : {
          label: copy.workspaceKey,
          copyLabel: 'workspace key',
          value: connection.workspace_public_key,
        },
    { label: copy.socketPath, copyLabel: 'socket path', value: connection.socket_path },
    { label: copy.hostUserId, copyLabel: 'host user ID', value: String(connection.owner_uid) },
    { label: copy.deviceId, copyLabel: 'device ID', value: connection.device_id },
    {
      label: copy.clusterId,
      copyLabel: 'cluster ID',
      value: connection.cluster_connection_id,
    },
  ].filter((row) => typeof row.value === 'string' && row.value.length > 0);

  return (
    <section className="flex min-w-0 flex-col gap-2" aria-label={copy.workspaceDetails}>
      <h3 className="text-caps text-text-muted">{copy.workspaceDetails}</h3>
      {rows.map((row) => (
        <div key={row.label} className="flex min-w-0 flex-col gap-1">
          <span className="text-supporting text-text-muted">{row.label}</span>
          <CopyField
            value={row.value}
            display={row.display}
            label={row.copyLabel}
            // Long machine strings keep their tail visible; a grouped fingerprint fits whole.
            truncate={(row.display ?? row.value).length > 24 ? 'middle' : undefined}
          />
        </div>
      ))}
    </section>
  );
}
