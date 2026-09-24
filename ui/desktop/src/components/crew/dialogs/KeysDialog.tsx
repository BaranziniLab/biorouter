import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { FileText, KeyRound, Lock } from '../../icons/app-icons';
import type { CrewConnection, CrewDevice } from '../crewApi';
import type { ErrorSource } from '../state/types';
import { keysCopy as copy } from './copy';
import { DialogErrorNote } from './fields';
import { groupedFingerprint, useWorkspaceKeyFingerprint } from './fingerprint';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:keys';
const KEY = 'credentials';

type CredentialAction = 'status' | 'init' | 'unlock' | 'lock';
/**
 * `file` is a development profile's plain-file store (`BIOROUTER_DEV_PROFILE_ROOT` with the
 * keyring disabled): said as such, never as a keychain it is not using (QA T-49).
 */
type CredentialBackend = 'keyring' | 'encrypted_vault' | 'file';
interface CredentialStatus {
  backend: CredentialBackend;
  initialized: boolean;
  locked: boolean;
}

const BACKENDS: readonly CredentialBackend[] = ['keyring', 'encrypted_vault', 'file'];

/**
 * The storage status the main process reports, or null when the answer is not one: a dialog that
 * has been cancelled (`{cancelled: true}`) changes nothing, and anything else is not trusted.
 */
function statusFrom(value: unknown): CredentialStatus | null {
  if (typeof value !== 'object' || value === null || 'cancelled' in value) return null;
  const status = value as Partial<CredentialStatus>;
  return status.backend !== undefined &&
    BACKENDS.includes(status.backend) &&
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

const HEX_DIGEST = /^[0-9a-f]{64}$/i;

/**
 * This device's fingerprint, grouped as the broker projects every device's (`grouped_fingerprint`
 * of the device ID, which is SHA-256 of the device's public key) — so it matches its own row in
 * "Devices on your account". Read from the saved device ID when that is the digest, else computed
 * from the key. Never the 64-hex key itself (QA T-33).
 */
function useDeviceFingerprint(connection: CrewConnection | null): string | null {
  const deviceId = connection?.device_id ?? '';
  const computed = useWorkspaceKeyFingerprint(
    HEX_DIGEST.test(deviceId) ? null : (connection?.public_key ?? null)
  );
  const digest = HEX_DIGEST.test(deviceId) ? deviceId : computed;
  return digest ? groupedFingerprint(digest) : null;
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
  const thisDevice = useDeviceFingerprint(connection);

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
  const file = status?.backend === 'file';
  const StoreIcon = vault ? Lock : file ? FileText : KeyRound;
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
            <StoreIcon aria-hidden className="h-icon-row w-icon-row shrink-0 text-text-muted" />
            <span>
              {!status ? copy.checking : vault ? copy.vault : file ? copy.file : copy.keychain}
            </span>
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

        {thisDevice ? (
          <div className="flex min-w-0 flex-col gap-1.5">
            <p className="text-label text-text-default">{copy.deviceKey}</p>
            <CopyField value={thisDevice} label={copy.deviceKeyLabel} />
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
                    <span className="flex shrink-0 items-center gap-2">
                      <span
                        className="whitespace-nowrap font-mono text-supporting text-text-default"
                        translate="no"
                      >
                        {device.fingerprint}
                      </span>
                      {thisDevice && device.fingerprint === thisDevice ? (
                        <Badge tone="neutral" variant="badge">
                          {copy.thisDevice}
                        </Badge>
                      ) : null}
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
