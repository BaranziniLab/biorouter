import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { isRecord } from '../api/parse';
import { displayNameIsUsername, usableName } from '../identity';
import type { ErrorSource } from '../state/types';
import { profileCopy as copy } from './copy';
import { DialogErrorNote, Field, helpId, useCustomValidity } from './fields';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:edit-profile';
const KEY = 'mutate:profile.update';

/** A display name may not carry `@` or `#`, nor anything whose compatibility form is one. */
function displayNameProblem(name: string): string | null {
  return Array.from(name).some((char) => /[@#]/.test(char.normalize('NFKC')))
    ? copy.handleMark
    : null;
}

export interface EditProfileDialogProps {
  onClose(): void;
}

/**
 * Edit profile (ui-redesign-spec, "Dialog inventory"; naming design, "Where display names come
 * from"). The name on the server account comes from `profile.suggest` and is OFFERED — prefilled
 * only while the person has never chosen a name, otherwise one click away — never applied without
 * **Save profile**. The username is shown read-only: it is the account, not a preference.
 */
export function EditProfileDialog({ onClose }: EditProfileDialogProps) {
  const { crew, dir } = useDialogView();
  const me = dir.me;
  const formId = React.useId();
  const nameId = `${formId}-name`;
  const initialsId = `${formId}-initials`;
  const chosen = me && !displayNameIsUsername(me.displayName, me.username) ? me.displayName : '';
  const [name, setName] = React.useState(chosen || me?.username || '');
  const [initials, setInitials] = React.useState(me?.avatar ?? '');
  const [edited, setEdited] = React.useState(false);
  const [suggestion, setSuggestion] = React.useState<string | null>(null);
  const problem = displayNameProblem(name);
  const nameRef = useCustomValidity<HTMLInputElement>(problem);
  const saving = crew.isPending(KEY);
  const request = crew.request;

  // One lookup when the dialog opens. A broker without `profile.suggest` simply offers nothing.
  React.useEffect(() => {
    let live = true;
    request<unknown>('profile.suggest', {})
      .then((answer) => {
        const offered = isRecord(answer) ? usableName(answer.full_name) : '';
        if (live && offered) setSuggestion(offered);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [request]);

  // Never chosen a name and not typing: the name on the server account is the natural prefill.
  React.useEffect(() => {
    if (suggestion && !chosen && !edited) setName(suggestion);
  }, [suggestion, chosen, edited]);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void crew
      .act(SOURCE, KEY, async () => {
        await crew.mutate('profile.update', {
          nickname: name.trim(),
          avatar: initials.trim() || null,
        });
        return true as const;
      })
      .then((done) => done === true && onClose());
  };

  const offer = suggestion && suggestion !== name.trim() ? suggestion : null;

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={saving ? 'required' : 'form'}
      title={copy.title}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onClose} disabled={saving}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={saving || !me}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-4 pb-1">
        <Field id={nameId} label={copy.displayName} error={problem ?? undefined}>
          <Input
            id={nameId}
            ref={nameRef}
            required
            autoComplete="name"
            aria-invalid={problem ? true : undefined}
            aria-describedby={problem ? helpId(nameId) : undefined}
            value={name}
            onChange={(event) => {
              setEdited(true);
              setName(event.target.value);
            }}
          />
          {offer ? (
            <p className="flex flex-wrap items-center gap-1.5 text-supporting text-text-muted">
              <Button
                type="button"
                variant="link"
                className="h-auto p-0 text-supporting"
                onClick={() => {
                  setEdited(true);
                  setName(offer);
                }}
              >
                {copy.suggestion(offer)}
              </Button>
              <span>· {copy.suggestionDetail}</span>
            </p>
          ) : null}
        </Field>
        <Field id={initialsId} label={copy.initials}>
          <Input
            id={initialsId}
            maxLength={12}
            autoComplete="off"
            value={initials}
            onChange={(event) => setInitials(event.target.value)}
          />
        </Field>
        {me ? (
          <p className="text-supporting text-text-muted">
            {copy.usernameLead}{' '}
            <bdi className="font-mono" translate="no">
              @{me.username}
            </bdi>
          </p>
        ) : null}
        <DialogErrorNote source={SOURCE} />
      </form>
    </ModalShell>
  );
}
