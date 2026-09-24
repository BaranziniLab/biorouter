import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import type { CrewConnection } from '../crewApi';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ErrorSource } from '../state/types';
import { connectionSettingsCopy, confirmCopy, makePrivateCopy } from './copy';
import { DialogErrorNote, Field, helpId } from './fields';
import { INSTITUTION_FIELD_PATTERN } from './nameRules';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:confirm';
const KEY = 'connection.update';

export interface MakePrivateDialogProps {
  connection: CrewConnection;
  onClose(): void;
}

/**
 * Public → Private for a connection that has no institution yet (ui-redesign-spec, "Privacy and
 * institution"). Going private is one click everywhere else; a Private connection must name its
 * institution, so this one small form asks for it first rather than dead-ending in a refusal. The
 * PATCH carries the whole record (L18).
 */
export function MakePrivateDialog({ connection, onClose }: MakePrivateDialogProps) {
  const { crew, workspace } = useDialogView(connection.id);
  const formId = React.useId();
  const fieldId = `${formId}-institution`;
  const [institution, setInstitution] = React.useState('');
  const [invalid, setInvalid] = React.useState(false);
  const saving = crew.isPending(KEY);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void crew
      .act(SOURCE, KEY, async () => {
        await crew.updateConnection(connection.id, {
          ...connectionUpdateBody(connection),
          mode: 'private',
          institution_id: institution.trim(),
        });
        if (connection.id === crew.connectionId) await crew.refresh();
        return true as const;
      })
      .then((done) => done === true && onClose());
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={saving ? 'required' : 'form'}
      title={makePrivateCopy.title(workspace)}
      footer={
        <>
          <Button type="button" variant="outline" onClick={onClose} disabled={saving}>
            {confirmCopy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={saving}>
            {makePrivateCopy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        <Field
          id={fieldId}
          label={connectionSettingsCopy.institution}
          error={invalid ? connectionSettingsCopy.institutionPattern : undefined}
          helper={institution ? undefined : connectionSettingsCopy.institutionHelper}
        >
          <Input
            id={fieldId}
            required
            maxLength={64}
            pattern={INSTITUTION_FIELD_PATTERN}
            autoComplete="off"
            spellCheck={false}
            translate="no"
            placeholder={connectionSettingsCopy.institutionPlaceholder}
            aria-invalid={invalid || undefined}
            aria-describedby={invalid || !institution ? helpId(fieldId) : undefined}
            value={institution}
            onInvalid={(event) => setInvalid(event.currentTarget.validity.patternMismatch)}
            onChange={(event) => {
              setInvalid(false);
              setInstitution(event.target.value);
            }}
          />
        </Field>
        <DialogErrorNote source={SOURCE} />
      </form>
    </ModalShell>
  );
}
