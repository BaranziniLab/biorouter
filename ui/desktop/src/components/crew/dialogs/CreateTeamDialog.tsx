import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { toastSuccess } from '../../../toasts';
import { isRecord, optionalText } from '../api/parse';
import { unexpectedCrewResponse } from '../api/errors';
import { nameKey, personLabel, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { addPeopleCopy, createTeamCopy as copy } from './copy';
import {
  DebouncedAnnouncement,
  ErrorNote,
  Field,
  helpId,
  labelId,
  useCustomValidity,
  useDialogError,
} from './fields';
import { teamNameProblem } from './nameRules';
import { directAddResultFrom, directAddSupported } from './people';
import { PersonPicker } from './PersonPicker';
import { directAddRefusalText, isNameRefusal, nameRefusalText, refusalText } from './refusals';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:create-team';
const CREATE_KEY = 'mutate:team.create';
const INVITE_KEY = 'mutate:invitation.create';
const ADD_KEY = 'mutate:team.add_member';

interface CreatedTeam {
  id: string;
  name: string;
  /** Its #general, as `#slug`: a person added to the team can see it. */
  general: string;
}

/** `team.create` answers `{team, channel}`; the team's ID is what the next step adds people to. */
function createdTeamFrom(value: unknown, typed: string): CreatedTeam {
  const team = isRecord(value) && isRecord(value.team) ? value.team : null;
  const id = team ? optionalText(team.id) : undefined;
  if (!team || !id) throw unexpectedCrewResponse('a new team');
  const channel = isRecord(value) && isRecord(value.channel) ? value.channel : null;
  const general = channel ? optionalText(channel.name) : undefined;
  return {
    id,
    name: teamName({ id, name: optionalText(team.name) ?? typed }),
    general: `#${(general ?? 'general').replace(/^#+/, '')}`,
  };
}

/**
 * The name field's example: never the name of a team this workspace already has, which read as a
 * suggestion to make a duplicate ("e.g. Analysis Lab" beside the lab's own Analysis Lab, QA Q3-38).
 * Compared the way the broker compares names, so "imaging-group" takes it too.
 */
export function teamExamplePlaceholder(
  teams: readonly { name?: string | null; handle?: string | null }[]
): string {
  const example = nameKey(copy.placeholder.replace(/^e\.g\. /, ''));
  const taken = teams.some((team) =>
    [team.name, team.handle].some((name) => typeof name === 'string' && nameKey(name) === example)
  );
  return taken ? copy.placeholderTaken : copy.placeholder;
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
  const { crew, dir, snapshot, workspace } = useDialogView();
  const formId = React.useId();
  const nameId = `${formId}-name`;
  const personId = `${formId}-person`;
  const [name, setName] = React.useState('');
  const [team, setTeam] = React.useState<CreatedTeam | null>(null);
  const [principalId, setPrincipalId] = React.useState<string | null>(null);
  const error = useDialogError(SOURCE);
  const nameRef = useCustomValidity<HTMLInputElement>(teamNameProblem(name));
  const creating = crew.isPending(CREATE_KEY);
  // A broker that adds members directly puts them in the new team; an older one invites them.
  const directAdd = directAddSupported(crew.capabilities);
  const inviting = crew.isPending(directAdd ? ADD_KEY : INVITE_KEY);
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
    const label = personLabel(person, 'inline', dir);
    void crew
      .act(SOURCE, directAdd ? ADD_KEY : INVITE_KEY, async () => {
        if (!directAdd) {
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
          return addPeopleCopy.sent(label);
        }
        // A new team has only its #general, which comes with the team: no channels to list.
        const result = directAddResultFrom(
          await crew.request(
            'team.add_member',
            { team_id: team.id, principal_id: person.id, expected_username: person.username },
            { mutation: true }
          )
        );
        return result.alreadyMember
          ? addPeopleCopy.alreadyIn(label, team.name)
          : addPeopleCopy.added(label, team.general);
      })
      .then((said) => {
        if (said === undefined) return;
        toastSuccess({ msg: said });
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
              variant="secondary"
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
              // The name field that had focus is gone with the first step.
              autoFocus
              id={personId}
              labelledBy={labelId(personId)}
              label={addPeopleCopy.person}
              candidates={candidates}
              value={principalId}
              onChange={setPrincipalId}
              dir={dir}
            />
          </Field>
          {error ? (
            <ErrorNote text={directAdd ? directAddRefusalText(error) : refusalText(error)} />
          ) : null}
        </form>
      </ModalShell>
    );
  }

  const nameError = error && isNameRefusal(error) ? nameRefusalText(error, 'team') : null;
  const fieldError = nameError ?? teamNameProblem(name);
  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={creating ? 'required' : 'form'}
      title={copy.title}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onClose} disabled={creating}>
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
          error={fieldError ?? undefined}
        >
          <Input
            id={nameId}
            ref={nameRef}
            required
            autoComplete="off"
            placeholder={teamExamplePlaceholder(snapshot?.teams ?? [])}
            aria-invalid={fieldError ? true : undefined}
            // The helper ("Team names are unique in …"), or the error that replaces it.
            aria-describedby={helpId(nameId)}
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              if (error) crew.dismissError();
            }}
          />
        </Field>
        <DebouncedAnnouncement text={fieldError} />
        {error && !nameError ? <ErrorNote text={refusalText(error)} /> : null}
      </form>
    </ModalShell>
  );
}
