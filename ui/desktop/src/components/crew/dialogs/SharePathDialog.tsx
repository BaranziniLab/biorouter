import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { isRecord, optionalText } from '../api/parse';
import { unexpectedCrewResponse } from '../api/errors';
import { sanitizeDisplayText } from '../identity';
import type { ErrorSource } from '../state/types';
import { sharePathCopy as copy } from './copy';
import { DialogErrorNote, Field, helpId } from './fields';
import { ABSOLUTE_PATH_PATTERN } from './nameRules';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:share-path';
const KEY = 'mutate:reference.create';

/** The last path segment, which is what a reference is called unless the person names it. */
export function pathLabel(path: string): string {
  const trimmed = path.trim().replace(/\/+$/, '');
  const last = trimmed.slice(trimmed.lastIndexOf('/') + 1);
  return last || path.trim();
}

/**
 * The account name before the `@` in a saved SSH target (`alice@hpc.example.edu` → `alice`), when
 * it reads as one; null otherwise, so a placeholder never shows something that is not a login.
 */
export function sshLogin(sshTarget: string | null | undefined): string | null {
  const target = sanitizeDisplayText(sshTarget);
  const at = target.lastIndexOf('@');
  const login = at > 0 ? target.slice(0, at) : '';
  return /^[A-Za-z0-9._-]{1,64}$/.test(login) ? login : null;
}

export interface SharePathDialogProps {
  onClose(): void;
}

/**
 * Share a path on the server (ui-redesign-spec, "Composer and files"): a reference to a file that
 * stays where it is, added to the message being written. Crew shares the path only — it does not
 * check the file exists or grant anyone access to it (the reference chip's tooltip says so).
 *
 * The title and helper say when to use it — a file already on the server, such as a large dataset,
 * rather than one from this computer — and the placeholder starts in the person's own home there
 * (QA Q2-32).
 */
export function SharePathDialog({ onClose }: SharePathDialogProps) {
  const { crew, server } = useDialogView();
  const saved = crew.connections.find((item) => item.id === crew.connectionId) ?? null;
  const login = sshLogin(saved?.ssh_target);
  const formId = React.useId();
  const pathId = `${formId}-path`;
  const labelFieldId = `${formId}-label`;
  const [path, setPath] = React.useState('');
  const [label, setLabel] = React.useState('');
  const [invalid, setInvalid] = React.useState(false);
  const pending = crew.isPending(KEY);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const target = path.trim();
    const name = label.trim() || pathLabel(target);
    void crew
      .act(SOURCE, KEY, async () => {
        const created = await crew.request(
          'reference.create',
          { channel_id: crew.channelId, path: target, label: name },
          { mutation: true }
        );
        const id = isRecord(created) ? optionalText(created.id) : undefined;
        if (!id) throw unexpectedCrewResponse('a shared path');
        return id;
      })
      .then((id) => {
        if (!id) return;
        crew.addReference({ id, label: name });
        onClose();
      });
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={pending ? 'required' : 'form'}
      title={copy.title(server)}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onClose} disabled={pending}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={pending || !crew.channelId}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        <Field
          id={pathId}
          label={copy.path}
          helper={copy.helper}
          error={invalid ? copy.pattern : undefined}
        >
          <Input
            id={pathId}
            required
            pattern={ABSOLUTE_PATH_PATTERN}
            autoComplete="off"
            spellCheck={false}
            translate="no"
            placeholder={copy.placeholder(login)}
            className="font-mono"
            aria-invalid={invalid || undefined}
            // The helper saying when to use this, or the error that replaces it.
            aria-describedby={helpId(pathId)}
            value={path}
            onInvalid={(event) => setInvalid(event.currentTarget.validity.patternMismatch)}
            onChange={(event) => {
              setInvalid(false);
              setPath(event.target.value);
            }}
          />
        </Field>
        <Disclosure label={copy.advanced} summary={copy.labelSummary}>
          <div className="pt-2">
            <Field id={labelFieldId} label={copy.label}>
              <Input
                id={labelFieldId}
                maxLength={255}
                autoComplete="off"
                placeholder={path.trim() ? pathLabel(path) : undefined}
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </Field>
          </div>
        </Disclosure>
        <DialogErrorNote source={SOURCE} />
      </form>
    </ModalShell>
  );
}
