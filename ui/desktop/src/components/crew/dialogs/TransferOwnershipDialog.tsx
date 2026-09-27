import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { toastSuccess } from '../../../toasts';
import { channelName, personLabel } from '../identity';
import type { ErrorSource } from '../state/types';
import { transferCopy as copy } from './copy';
import { DialogErrorNote, Field, helpId, labelId } from './fields';
import { ownershipCandidates } from './people';
import { PersonPicker } from './PersonPicker';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:transfer-ownership';
const KEY = 'mutate:channel.transfer';

export interface TransferOwnershipDialogProps {
  channelId: string;
  /** Preselect the new owner (for "Make owner…" on a member row). */
  successorId?: string;
  onClose(): void;
}

/**
 * Transfer ownership of a channel (ui-redesign-spec, "Dialog inventory"): an OFFER the other
 * person must accept, sent as `channel.transfer` with their `expected_username`. Only the channel's
 * active members are offered.
 */
export function TransferOwnershipDialog({
  channelId,
  successorId,
  onClose,
}: TransferOwnershipDialogProps) {
  const { crew, snapshot, dir } = useDialogView();
  const formId = React.useId();
  const ownerId = `${formId}-owner`;
  const candidates = ownershipCandidates(snapshot, dir, channelId);
  const [principalId, setPrincipalId] = React.useState<string | null>(() =>
    successorId && candidates.some((person) => person.id === successorId) ? successorId : null
  );
  const channel = snapshot?.channels.find((item) => item.id === channelId) ?? null;
  const sending = crew.isPending(KEY);
  useCloseWhenMissing(snapshot !== null && channel === null, onClose);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const person = candidates.find((item) => item.id === principalId);
    if (!person?.id) return;
    const label = personLabel(person, 'inline', dir);
    void crew
      .act(SOURCE, KEY, async () => {
        await crew.mutate('channel.transfer', {
          channel_id: channelId,
          successor_id: person.id,
          expected_username: person.username,
        });
        return true as const;
      })
      .then((done) => {
        if (done !== true) return;
        toastSuccess({ msg: copy.offered(label) });
        onClose();
      });
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={sending ? 'required' : 'form'}
      title={copy.title(channelName(channel))}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onClose} disabled={sending}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={sending || !principalId}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        {candidates.length > 0 ? (
          <Field id={ownerId} label={copy.owner} helper={copy.helper}>
            <PersonPicker
              id={ownerId}
              labelledBy={labelId(ownerId)}
              label={copy.owner}
              candidates={candidates}
              value={principalId}
              onChange={setPrincipalId}
              dir={dir}
              aria-describedby={helpId(ownerId)}
            />
          </Field>
        ) : (
          <Note tone="neutral" role="status">
            {copy.noOne}
          </Note>
        )}
        <DialogErrorNote source={SOURCE} />
      </form>
    </ModalShell>
  );
}
