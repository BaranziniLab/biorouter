import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { channelSlug, isMachineIdShaped, sanitizeDisplayText, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { renameCopy as copy } from './copy';
import {
  AdornedInput,
  ErrorNote,
  Field,
  helpId,
  useCustomValidity,
  useDialogError,
} from './fields';
import {
  channelSlugPreview,
  channelSlugProblem,
  teamNameProblem,
  WORKSPACE_NAME_PATTERN,
  workspaceNameProblem,
} from './nameRules';
import { createChannelCopy } from './copy';
import { isNameRefusal, nameRefusalText, refusalText } from './refusals';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:rename';

export interface RenameDialogProps {
  target: 'team' | 'channel' | 'workspace';
  targetId: string;
  onClose(): void;
}

/**
 * Rename a team, a channel or the workspace (naming slice S2). Offered only where the broker
 * speaks the unique-name rules; a rename keeps the object's ID, so history, invitations and grants
 * are unaffected. A taken name is refused in the broker's one wording.
 */
export function RenameDialog({ target, targetId, onClose }: RenameDialogProps) {
  const { crew, snapshot } = useDialogView();
  const formId = React.useId();
  const nameId = `${formId}-name`;
  const team =
    target === 'team' ? (snapshot?.teams.find((item) => item.id === targetId) ?? null) : null;
  const channel =
    target === 'channel' ? (snapshot?.channels.find((item) => item.id === targetId) ?? null) : null;
  const workspaceName = sanitizeDisplayText(snapshot?.workspace.name);
  const initial =
    target === 'team'
      ? team
        ? teamName(team)
        : ''
      : target === 'channel'
        ? channel
          ? channelSlug(channel)
          : ''
        : workspaceName && !isMachineIdShaped(workspaceName)
          ? workspaceName
          : '';
  const [name, setName] = React.useState(initial);
  const [touched, setTouched] = React.useState(false);
  const error = useDialogError(SOURCE);
  const missing =
    snapshot !== null &&
    ((target === 'team' && !team) ||
      (target === 'channel' && !channel) ||
      (target === 'workspace' && snapshot.workspace.id !== targetId));
  useCloseWhenMissing(missing, onClose);

  const slug = target === 'channel' ? channelSlugPreview(name) : '';
  const problem = !name.trim()
    ? null
    : target === 'channel'
      ? channelSlugProblem(slug)
      : target === 'team'
        ? teamNameProblem(name)
        : workspaceNameProblem(name.trim());
  const nameRef = useCustomValidity<HTMLInputElement>(problem);
  const method =
    target === 'team'
      ? 'team.rename'
      : target === 'channel'
        ? 'channel.rename'
        : 'workspace.rename';
  const key = `mutate:${method}`;
  const pending = crew.isPending(key);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const params =
      target === 'team'
        ? { team_id: targetId, name: name.trim() }
        : target === 'channel'
          ? { channel_id: targetId, name: slug }
          : { name: name.trim() };
    void crew
      .act(SOURCE, key, async () => {
        await crew.mutate(method, params);
        return true as const;
      })
      .then((done) => done === true && onClose());
  };

  const nameError = error && isNameRefusal(error) ? nameRefusalText(error, target) : null;
  const fieldError = nameError ?? (touched ? problem : null);
  const helper =
    target === 'channel' && slug && !problem ? createChannelCopy.preview(slug) : undefined;
  const title =
    target === 'team'
      ? copy.titleTeam
      : target === 'channel'
        ? copy.titleChannel
        : copy.titleWorkspace;
  const inputProps = {
    id: nameId,
    ref: nameRef,
    required: true,
    autoComplete: 'off',
    spellCheck: false,
    'aria-invalid': fieldError ? true : undefined,
    'aria-describedby': helper || fieldError ? helpId(nameId) : undefined,
    value: name,
    onBlur: () => setTouched(name.trim().length > 0),
    onInvalid: () => setTouched(true),
    onChange: (event: React.ChangeEvent<HTMLInputElement>) => {
      setName(event.target.value);
      if (error) crew.dismissError();
    },
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={pending ? 'required' : 'form'}
      title={title}
      footer={
        <>
          <Button type="button" variant="outline" onClick={onClose} disabled={pending}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={pending}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        <Field id={nameId} label={copy.name} helper={helper} error={fieldError ?? undefined}>
          {target === 'channel' ? (
            <AdornedInput adornment="#" {...inputProps} />
          ) : target === 'workspace' ? (
            <Input {...inputProps} pattern={WORKSPACE_NAME_PATTERN} maxLength={40} translate="no" />
          ) : (
            <Input {...inputProps} />
          )}
        </Field>
        {error && !nameError ? <ErrorNote text={refusalText(error)} /> : null}
      </form>
    </ModalShell>
  );
}
