import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { FileText, KeyRound, Lock } from '../../icons/app-icons';
import type { CrewConnection, CrewDevice } from '../crewApi';
import { useInitialFocus } from '../onboarding/fields';
import type { ErrorSource } from '../state/types';
import { keysCopy as copy } from './copy';
import { DialogErrorNote } from './fields';
import { groupedFingerprint, useWorkspaceKeyFingerprint } from './fingerprint';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:keys';
const KEY = 'credentials';

type CredentialAction = 'init' | 'unlock' | 'lock';
/**
 * `file` is the plain-file store, and only a development profile reports it: the daemon chooses it
 * when the keyring is disabled AND `BIOROUTER_DEV_PROFILE_ROOT` is an absolute path
 * (`file_credentials_enabled`, crew/mod.rs). It is no fallback for a machine without a keyring.
 * Said as such, "(development profile)", never as a keychain it is not using (QA T-49).
 */
type CredentialBackend = 'keyring' | 'encrypted_vault' | 'file';
interface CredentialStatus {
  backend: CredentialBackend;
  initialized: boolean;
  locked: boolean;
}

/**
 * Every storage backend the daemon reports. The main process validates the same list before it
 * answers (`crew:credentials` in `main.ts`); a test holds the two together, because the day they
 * drifted (`file`, QA Q2-02) the main process refused the answer and the dialog never loaded.
 */
export const CREDENTIAL_BACKENDS: readonly CredentialBackend[] = [
  'keyring',
  'encrypted_vault',
  'file',
];

/** How long "Checking where your keys are stored…" may stand before it says it couldn't. */
export const STATUS_PATIENCE_MS = 5000;

/**
 * Electron's wrapper around an error the main process threw: `Error invoking remote method
 * 'crew:credentials': Error: …`. Machinery, never words for a person.
 */
const IPC_WRAPPER = /^Error invoking remote method '[^']*':\s*(?:[A-Za-z]*Error:\s*)?/;

/**
 * An action's failure in words: the main process's own sentence without Electron's wrapper, and
 * the plain status sentence for a status read that failed after the action (QA Q2-02).
 */
export function keysErrorText(message: string): string {
  const inner = message.replace(IPC_WRAPPER, '').trim();
  if (!inner) return copy.failed;
  if (/credential status/i.test(inner)) return copy.statusFailed;
  return inner;
}

/**
 * The storage status the main process reports, or null when the answer is not one: a dialog that
 * has been cancelled (`{cancelled: true}`) changes nothing, and anything else is not trusted.
 */
function statusFrom(value: unknown): CredentialStatus | null {
  if (typeof value !== 'object' || value === null || 'cancelled' in value) return null;
  const status = value as Partial<CredentialStatus>;
  return status.backend !== undefined &&
    CREDENTIAL_BACKENDS.includes(status.backend) &&
    typeof status.initialized === 'boolean' &&
    typeof status.locked === 'boolean'
    ? { backend: status.backend, initialized: status.initialized, locked: status.locked }
    : null;
}

/** "Added September 24, 2026 · with an invitation": when and how a device joined, as known. */
function deviceMeta(device: CrewDevice): string {
  const date = addedOn(device);
  const via = device.added_via ? copy.addedVia[device.added_via] : undefined;
  return [date ? copy.deviceAdded(date) : null, via].filter(Boolean).join(' · ');
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
 * send them to review it: each device's fingerprint, and under it, on its own left-aligned line,
 * when and how it was added (QA Q3-41). When this computer is the only device, that line sits under
 * its fingerprint above and nothing repeats the fingerprint. The dialog opens on Done: it is read,
 * not filled in, and the first key must not copy anything. Done keeps that focus while the menu
 * that opened the dialog finishes closing (`useInitialFocus`): opened with the pointer from the You
 * menu, focus used to drop to `<body>` a quarter of a second later (QA Q4-33).
 */
export function KeysDialog({ onClose }: KeysDialogProps) {
  const { crew, snapshot } = useDialogView();
  const [status, setStatus] = React.useState<CredentialStatus | null>(null);
  // Reading the status is not an action of the person's, so it never goes through `act`: its
  // failure is the header's plain sentence with a Retry, never an error note (QA Q2-02).
  const [read, setRead] = React.useState<'checking' | 'failed' | 'done'>('checking');
  const [slow, setSlow] = React.useState(false);
  const reads = React.useRef(0);
  const pending = crew.isPending(KEY);
  const act = crew.act;
  const connection = crew.connections.find((item) => item.id === crew.connectionId) ?? null;
  const devices = snapshot?.actor.devices ?? [];
  const thisDevice = useDeviceFingerprint(connection);
  // The account's one device is this computer: its fingerprint is already in the box above.
  const onlyThisDevice =
    thisDevice !== null && devices.length === 1 && devices[0].fingerprint === thisDevice;
  const doneRef = React.useRef<HTMLButtonElement>(null);
  useInitialFocus(doneRef, true);

  const readStatus = React.useCallback(async () => {
    const generation = ++reads.current;
    setRead('checking');
    setSlow(false);
    let next: CredentialStatus | null = null;
    try {
      next = statusFrom(await window.electron.crewCredentials('status'));
    } catch {
      next = null;
    }
    // A later read (Retry) or a closed dialog owns the answer now.
    if (generation !== reads.current) return;
    if (next) setStatus(next);
    setRead(next ? 'done' : 'failed');
  }, []);

  React.useEffect(() => {
    void readStatus();
    return () => {
      reads.current += 1;
    };
  }, [readStatus]);

  // No answer yet after a few seconds says so, rather than "Checking…" for ever. A late answer
  // still replaces it.
  React.useEffect(() => {
    if (read !== 'checking') return;
    const timer = window.setTimeout(() => setSlow(true), STATUS_PATIENCE_MS);
    return () => window.clearTimeout(timer);
  }, [read]);

  const run = React.useCallback(
    (action: CredentialAction) =>
      act(SOURCE, KEY, async () => {
        const next = statusFrom(await window.electron.crewCredentials(action));
        if (next) {
          setStatus(next);
          setRead('done');
        }
      }),
    [act]
  );

  const vault = status?.backend === 'encrypted_vault';
  const file = status?.backend === 'file';
  const unknown = !status && (read === 'failed' || slow);
  const StoreIcon = vault ? Lock : file ? FileText : KeyRound;
  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose="info"
      title={copy.title}
      footer={
        <Button ref={doneRef} autoFocus onClick={onClose}>
          {copy.done}
        </Button>
      }
    >
      <div className="flex flex-col gap-4 pb-1">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <p role="status" className="flex min-w-0 items-center gap-2 text-body text-text-default">
            <StoreIcon aria-hidden className="h-icon-row w-icon-row shrink-0 text-text-muted" />
            <span>
              {status
                ? vault
                  ? copy.vault
                  : file
                    ? copy.file
                    : copy.keychain
                : unknown
                  ? copy.statusFailed
                  : copy.checking}
            </span>
            {vault ? (
              <Badge tone="neutral" variant="badge">
                {status?.locked ? copy.locked : copy.unlocked}
              </Badge>
            ) : null}
          </p>
          {unknown ? (
            <Button variant="ghost" size="sm" onClick={() => void readStatus()}>
              {copy.retry}
            </Button>
          ) : null}
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
            {onlyThisDevice && deviceMeta(devices[0]) ? (
              <p className="text-supporting text-text-muted" data-crew-device-meta="">
                {deviceMeta(devices[0])}
              </p>
            ) : null}
          </div>
        ) : null}

        {devices.length > 0 && !onlyThisDevice ? (
          <section className="flex min-w-0 flex-col gap-1.5" aria-label={copy.devices}>
            <h3 className="text-caps text-text-muted">{copy.devices}</h3>
            <ul className="biorouter-settings-list">
              {devices.map((device) => {
                const meta = deviceMeta(device);
                return (
                  <li
                    key={device.fingerprint}
                    className="biorouter-settings-row flex min-w-0 flex-col items-start gap-0.5 px-3 py-2"
                  >
                    <span className="flex min-w-0 items-center gap-2">
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
                    {/* Its own line, left-aligned: it wrapped right-aligned beside the
                        fingerprint (QA Q3-41). */}
                    {meta ? (
                      <span className="text-supporting text-text-muted" data-crew-device-meta="">
                        {meta}
                      </span>
                    ) : null}
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

        <DialogErrorNote source={SOURCE} render={keysErrorText} />
      </div>
    </ModalShell>
  );
}
