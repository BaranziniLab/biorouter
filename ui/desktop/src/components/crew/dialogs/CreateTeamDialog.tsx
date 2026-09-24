import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { toastSuccess } from '../../../toasts';
import { isRecord, optionalText } from '../api/parse';
import { unexpectedCrewResponse } from '../api/errors';
import { personLabel, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { addPeopleCopy, createTeamCopy as copy } from './copy';
import { ErrorNote, Field, helpId, labelId, useCustomValidity, useDialogError } from './fields';
import { teamNameProblem } from './nameRules';
import { PersonPicker } from './PersonPicker';
import { isNameRefusal, nameRefusalText, refusalText } from './refusals';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:create-team';
const CREATE_KEY = 'mutate:team.create';
const INVITE_KEY = 'mutate:invitation.create';

interface CreatedTeam {
  id: string;
  name: string;
}

/** `team.create` answers `{team, channel}`; the team's ID is what the next step invites to. */
function createdTeamFrom(value: unknown, typed: string): CreatedTeam {
  const team = isRecord(value) && isRecord(value.team) ? value.team : null;
  const id = team ? optionalText(team.id) : undefined;
  if (!team || !id) throw unexpectedCrewResponse('a new team');
  return { id, name: teamName({ id, name: optionalText(team.name) ?? typed }) };
}

export interface CreateTeamDialogProps {
  onClose(): void;
}

/**
 * Create team, then an optional "Add people to {team}" step (ui-redesign-spec, "Dialog
 * inventory"). The name is unique in the workspace (S2): a taken name is refused in the broker's
 * one wording, which reads the same whether or not the team holding it is visible. When it is
 * done the new team is selected, so its #general is where the person lands.
 */
export function CreateTeamDialog({ onClose }: CreateTeamDialogProps) {
  const { crew, dir, workspace } = useDialogView();
  const formId = React.useId();
  const nameId = `${formId}-name`;
  const personId = `${formId}-person`;
  const [name, setName] = React.useState('');
  const [team, setTeam] = React.useState<CreatedTeam | null>(null);
  const [principalId, setPrincipalId] = React.useState<string | null>(null);
  const error = useDialogError(SOURCE);
  const nameRef = useCustomValidity<HTMLInputElement>(teamNameProblem(name));
  const creating = crew.isPending(CREATE_KEY);
  const inviting = crew.isPending(INVITE_KEY);
  const candidates = dir.people.filter((person) => !person.isYou && !person.isFormer && person.id);

  const finish = (created: CreatedTeam) => {
    crew.selectTeam(created.id);
    onClose();
  };

  const create = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const typed = name.trim();
    void crew
      .act(SOURCE, CREATE_KEY, async () =>
        createdTeamFrom(
          await crew.request('team.create', { name: typed }, { mutation: true }),
          typed
        )
      )
      .then((created) => {
        if (!created) return;
        if (candidates.length === 0) finish(created);
        else setTeam(created);
      });
  };

  const add = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const person = candidates.find((item) => item.id === principalId);
    if (!team || !person?.id) return;
    void crew
      .act(SOURCE, INVITE_KEY, async () => {
        await crew.request(
          'invitation.create',
          {
            kind: 'team',
            target_id: team.id,
            principal_id: person.id,
            expected_username: person.username,
          },
          { mutation: true }
        );
        return true as const;
      })
      .then((done) => {
        if (done !== true) return;
        toastSuccess({ msg: addPeopleCopy.sent(personLabel(person, 'inline', dir)) });
        finish(team);
      });
  };

  if (team) {
    return (
      <ModalShell
        open
        onOpenChange={(open) => !open && finish(team)}
        size="sm"
        purpose={inviting ? 'required' : 'form'}
        title={copy.addTitle(team.name)}
        footer={
          <>
            <Button
              type="button"
              variant="outline"
              disabled={inviting}
              onClick={() => finish(team)}
            >
              {copy.skip}
            </Button>
            <Button type="submit" form={formId} disabled={inviting || !principalId}>
              {copy.add}
            </Button>
          </>
        }
      >
        <form id={formId} onSubmit={add} className="flex flex-col gap-3 pb-1">
          <Field id={personId} label={addPeopleCopy.person}>
            <PersonPicker
              id={personId}
              labelledBy={labelId(personId)}
              label={addPeopleCopy.person}
              candidates={candidates}
              value={principalId}
              onChange={setPrincipalId}
              dir={dir}
            />
          </Field>
          {error ? <ErrorNote text={refusalText(error)} /> : null}
        </form>
      </ModalShell>
    );
  }

  const nameError = error && isNameRefusal(error) ? nameRefusalText(error, 'team') : null;
  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={creating ? 'required' : 'form'}
      title={copy.title}
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
      <form id={formId} onSubmit={create} className="flex flex-col gap-3 pb-1">
        <Field
          id={nameId}
          label={copy.name}
          helper={copy.helper(workspace)}
          error={nameError ?? teamNameProblem(name) ?? undefined}
        >
          <Input
            id={nameId}
            ref={nameRef}
            required
            autoComplete="off"
            placeholder={copy.placeholder}
            aria-invalid={nameError || teamNameProblem(name) ? true : undefined}
            aria-describedby={helpId(nameId)}
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              if (error) crew.dismissError();
            }}
          />
        </Field>
        {error && !nameError ? <ErrorNote text={refusalText(error)} /> : null}
      </form>
    </ModalShell>
  );
}
