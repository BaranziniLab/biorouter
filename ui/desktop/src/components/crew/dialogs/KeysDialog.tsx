import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { KeyRound, Lock } from '../../icons/app-icons';
import type { CrewDevice } from '../crewApi';
import type { ErrorSource } from '../state/types';
import { keysCopy as copy } from './copy';
import { DialogErrorNote } from './fields';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:keys';
const KEY = 'credentials';

type CredentialAction = 'status' | 'init' | 'unlock' | 'lock';
interface CredentialStatus {
  backend: 'keyring' | 'encrypted_vault';
  initialized: boolean;
  locked: boolean;
}

/**
 * The storage status the main process reports, or null when the answer is not one: a dialog that
 * has been cancelled (`{cancelled: true}`) changes nothing, and anything else is not trusted.
 */
function statusFrom(value: unknown): CredentialStatus | null {
  if (typeof value !== 'object' || value === null || 'cancelled' in value) return null;
  const status = value as Partial<CredentialStatus>;
  return (status.backend === 'keyring' || status.backend === 'encrypted_vault') &&
    typeof status.initialized === 'boolean' &&
    typeof status.locked === 'boolean'
    ? { backend: status.backend, initialized: status.initialized, locked: status.locked }
    : null;
}

function addedOn(device: CrewDevice): string | null {
  if (typeof device.added_at !== 'number' || !Number.isFinite(device.added_at)) return null;
  return new Date(device.added_at * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  });
}

export interface KeysDialogProps {
  onClose(): void;
}

/**
 * Keys and security (ui-redesign-spec, "Dialog inventory"), replacing the old `CrewCredentials`
 * block with the same IPC (`window.electron.crewCredentials`). Status loads when the dialog opens,
 * so there is no refresh button. Unlocking and setting up a vault prompt for the passphrase in the
 * main process's own native dialog; nothing secret passes through this renderer.
 *
 * It also lists the devices on the person's account, so "A new device was added…" has somewhere to
 * send them to review it.
 */
export function KeysDialog({ onClose }: KeysDialogProps) {
  const { crew, snapshot } = useDialogView();
  const [status, setStatus] = React.useState<CredentialStatus | null>(null);
  const pending = crew.isPending(KEY);
  const act = crew.act;
  const connection = crew.connections.find((item) => item.id === crew.connectionId) ?? null;
  const devices = snapshot?.actor.devices ?? [];

  const run = React.useCallback(
    (action: CredentialAction) =>
      act(
        SOURCE,
        KEY,
        async () => {
          const next = statusFrom(await window.electron.crewCredentials(action));
          if (next) setStatus(next);
        },
        // Reading the status is not an action of the person's: it must not clear an error
        // another surface is showing.
        { preserveError: action === 'status' }
      ),
    [act]
  );

  React.useEffect(() => {
    void run('status');
  }, [run]);

  const vault = status?.backend === 'encrypted_vault';
  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose="info"
      title={copy.title}
      footer={<Button onClick={onClose}>{copy.done}</Button>}
    >
      <div className="flex flex-col gap-4 pb-1">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <p role="status" className="flex min-w-0 items-center gap-2 text-body text-text-default">
            {vault ? (
              <Lock aria-hidden className="h-icon-row w-icon-row shrink-0 text-text-muted" />
            ) : (
              <KeyRound aria-hidden className="h-icon-row w-icon-row shrink-0 text-text-muted" />
            )}
            <span>{!status ? copy.checking : vault ? copy.vault : copy.keychain}</span>
            {vault ? (
              <Badge tone="neutral" variant="badge">
                {status?.locked ? copy.locked : copy.unlocked}
              </Badge>
            ) : null}
          </p>
          {vault ? (
            <Button
              variant="secondary"
              size="sm"
              disabled={pending}
              onClick={() => void run(status?.locked ? 'unlock' : 'lock')}
            >
              {status?.locked ? copy.unlock : copy.lock}
            </Button>
          ) : null}
        </div>

        {connection?.public_key ? (
          <div className="flex min-w-0 flex-col gap-1.5">
            <p className="text-label text-text-default">{copy.deviceKey}</p>
            <CopyField
              value={connection.public_key}
              label={copy.deviceKeyLabel}
              truncate="middle"
            />
          </div>
        ) : null}

        {devices.length > 0 ? (
          <section className="flex min-w-0 flex-col gap-1.5" aria-label={copy.devices}>
            <h3 className="text-caps text-text-muted">{copy.devices}</h3>
            <ul className="biorouter-settings-list">
              {devices.map((device) => {
                const date = addedOn(device);
                const via = device.added_via ? copy.addedVia[device.added_via] : undefined;
                return (
                  <li
                    key={device.fingerprint}
                    className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2"
                  >
                    <span
                      className="shrink-0 whitespace-nowrap font-mono text-supporting text-text-default"
                      translate="no"
                    >
                      {device.fingerprint}
                    </span>
                    <span className="min-w-0 text-right text-supporting text-text-muted">
                      {[date ? copy.deviceAdded(date) : null, via].filter(Boolean).join(' · ')}
                    </span>
                  </li>
                );
              })}
            </ul>
          </section>
        ) : null}

        {status?.backend === 'keyring' ? (
          <Disclosure label={copy.vaultToggle}>
            <div className="flex flex-col items-start gap-2 pt-2">
              <p className="text-supporting text-text-muted">{copy.vaultNote}</p>
              <Button
                variant="secondary"
                size="sm"
                disabled={pending}
                onClick={() => void run('init')}
              >
                {copy.setUpVault}
              </Button>
            </div>
          </Disclosure>
        ) : null}

        <DialogErrorNote source={SOURCE} />
      </div>
    </ModalShell>
  );
}
