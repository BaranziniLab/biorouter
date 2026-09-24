import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { isRecord, optionalText } from '../api/parse';
import { unexpectedCrewResponse } from '../api/errors';
import { teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { createChannelCopy as copy, nameRuleCopy } from './copy';
import {
  AdornedInput,
  ErrorNote,
  Field,
  helpId,
  RadioRows,
  useCustomValidity,
  useDialogError,
} from './fields';
import { channelSlugPreview, channelSlugProblem } from './nameRules';
import { isNameRefusal, nameRefusalText, refusalText } from './refusals';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:create-channel';
const KEY = 'mutate:channel.create';

type Classification = 'restricted' | 'public_safe';

export interface CreateChannelDialogProps {
  teamId: string;
  onClose(): void;
}

/**
 * Create channel (ui-redesign-spec, "Dialog inventory", "Progressive disclosure").
 *
 * - Every open starts clean — name empty, content Restricted — because the dialog's state is its
 *   own and it is mounted per open (L14: the old panel kept the last channel's classification).
 * - The name previews the exact slug the broker will store ("Will be created as #…").
 * - Content is visible, not behind Advanced: it cannot be changed after the channel exists.
 * - A taken name is refused in the broker's one wording (S2), and the consequence line says that
 *   the refusal itself tells team members a name exists.
 */
export function CreateChannelDialog({ teamId, onClose }: CreateChannelDialogProps) {
  const { crew, snapshot } = useDialogView();
  const formId = React.useId();
  const nameId = `${formId}-name`;
  const [name, setName] = React.useState('');
  const [classification, setClassification] = React.useState<Classification>('restricted');
  const [touched, setTouched] = React.useState(false);
  const error = useDialogError(SOURCE);
  const team = snapshot?.teams.find((item) => item.id === teamId) ?? null;
  const slug = channelSlugPreview(name);
  const problem = name.trim() ? channelSlugProblem(slug) : null;
  const nameRef = useCustomValidity<HTMLInputElement>(problem);
  const creating = crew.isPending(KEY);
  useCloseWhenMissing(snapshot !== null && team === null, onClose);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void crew
      .act(SOURCE, KEY, async () => {
        const created = await crew.request(
          'channel.create',
          { team_id: teamId, name: slug, classification },
          { mutation: true }
        );
        const id = isRecord(created) ? optionalText(created.id) : undefined;
        if (!id) throw unexpectedCrewResponse('a new channel');
        return id;
      })
      .then((channelId) => {
        if (!channelId) return;
        if (teamId !== crew.teamId) crew.selectTeam(teamId);
        crew.selectChannel(channelId);
        onClose();
      });
  };

  const nameError = error && isNameRefusal(error) ? nameRefusalText(error, 'channel') : null;
  const fieldError = nameError ?? (touched ? problem : null);
  const helper = slug && !problem ? copy.preview(slug) : undefined;

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={creating ? 'required' : 'form'}
      title={copy.title}
      subtitle={team ? copy.inTeam(teamName(team)) : undefined}
      footer={
        <>
          <Button type="button" variant="outline" onClick={onClose} disabled={creating}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={creating}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-4 pb-1">
        <Field id={nameId} label={copy.name} helper={helper} error={fieldError ?? undefined}>
          <AdornedInput
            adornment="#"
            id={nameId}
            ref={nameRef}
            required
            autoComplete="off"
            spellCheck={false}
            placeholder={copy.placeholder}
            aria-invalid={fieldError ? true : undefined}
            aria-describedby={helper || fieldError ? helpId(nameId) : undefined}
            value={name}
            onBlur={() => setTouched(name.trim().length > 0)}
            onInvalid={() => setTouched(true)}
            onChange={(event) => {
              setName(event.target.value);
              if (error) crew.dismissError();
            }}
          />
        </Field>
        <RadioRows<Classification>
          label={copy.content}
          name={`${formId}-content`}
          value={classification}
          onChange={setClassification}
          options={[
            { value: 'restricted', label: copy.restricted, detail: copy.restrictedDetail },
            { value: 'public_safe', label: copy.publicSafe, detail: copy.publicSafeDetail },
          ]}
        />
        <p className="text-supporting text-text-muted">{nameRuleCopy.consequence}</p>
        {error && !nameError ? <ErrorNote text={refusalText(error)} /> : null}
      </form>
    </ModalShell>
  );
}
