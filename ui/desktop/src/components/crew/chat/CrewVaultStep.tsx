import { useEffect, useState } from 'react';
import type { CrewStepRequest } from '../../../api';
import { failureSentence } from '../../../utils/ipcError';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import { sanitizeDisplayText } from '../identity/displayText';
import { crewVaultStepCopy as copy } from './CrewVaultStep.copy';
import { CrewStepFrame, dismissStep } from './CrewStepFrame';

type Vault = Extract<CrewStepRequest, { kind: 'vault' }>;

type VaultAnswer =
  | { backend: 'keyring' | 'encrypted_vault' | 'file'; initialized: boolean; locked: boolean }
  | { cancelled: true };

/**
 * The main process's vault IPC (`crew:credentials`, `main.ts`), or `null`
 * outside Electron (a browser session, a unit test). It takes the action and
 * nothing else: the passphrase is asked in the OS-native prompt main opens with
 * its own fixed words, posted to the daemon with the person's proof by main,
 * and never handed to this renderer.
 */
function vaultApi(): ((action: 'unlock' | 'init') => Promise<VaultAnswer>) | null {
  const api = (window as { electron?: { crewCredentials?: unknown } }).electron?.crewCredentials;
  return typeof api === 'function'
    ? (api as (action: 'unlock' | 'init') => Promise<VaultAnswer>)
    : null;
}

type State =
  | { kind: 'idle' }
  | { kind: 'waiting' }
  | { kind: 'closed' }
  | { kind: 'done' }
  | { kind: 'failed'; message: string }
  | { kind: 'dismissed' };

/**
 * A chat's vault card: unlock or set up Crew's vault on this computer (the
 * owner's D1 ruling on AO-12). Its button opens the OS-native passphrase prompt
 * through the main process, which never hands the passphrase to the renderer,
 * so the card holds no input. The card is never answered as done: the call that
 * parked it reads the vault and takes the card down itself once it is open.
 * "Not now" dismisses it, which tells the chat the vault is still locked.
 */
export default function CrewVaultStep({
  sessionId,
  id,
  step,
  onHolding,
}: {
  sessionId: string;
  id: string;
  step: Vault;
  /** Told whether the passphrase prompt is open, so the card stays until it answers. */
  onHolding?: (holding: boolean) => void;
}) {
  // The daemon's action, read as one of two literals: nothing else reaches the IPC.
  const init = step.action === 'init';
  const action = init ? 'init' : 'unlock';
  const title = init ? copy.initTitle : copy.unlockTitle;
  const api = vaultApi();
  const [state, setState] = useState<State>({ kind: 'idle' });
  const prompting = state.kind === 'waiting';
  useEffect(() => {
    onHolding?.(prompting);
  }, [prompting, onHolding]);

  if (!api) {
    // The warning stays in view; where to do it instead folds (owner, 2026-11-13).
    return (
      <CrewStepFrame kind="vault" title={title}>
        <p className="text-supporting text-text-muted">{copy.fallbackWarning}</p>
        <Disclosure label={init ? copy.initHowLabel : copy.unlockHowLabel}>
          <p className="text-supporting text-text-muted">{init ? copy.initHow : copy.unlockHow}</p>
        </Disclosure>
      </CrewStepFrame>
    );
  }

  const failed = init ? copy.failedInit : copy.failedUnlock;
  const open = async () => {
    setState({ kind: 'waiting' });
    try {
      const answer = await api(action);
      if ('cancelled' in answer) setState({ kind: 'closed' });
      else if (answer.initialized && !answer.locked) setState({ kind: 'done' });
      else setState({ kind: 'failed', message: failed });
    } catch (error) {
      // Main's own sentence (a refused passphrase, a missing zenity, a prompt already open),
      // without Electron's wrapper.
      setState({
        kind: 'failed',
        message: sanitizeDisplayText(failureSentence(error, failed)) || failed,
      });
    }
  };
  const notNow = async () => {
    setState({ kind: 'dismissed' });
    await dismissStep(sessionId, id).catch(() => false);
  };

  const busy = state.kind === 'waiting';
  const finished = state.kind === 'done' || state.kind === 'dismissed';
  const status =
    state.kind === 'waiting'
      ? copy.waiting
      : state.kind === 'closed'
        ? init
          ? copy.closedInit
          : copy.closedUnlock
        : state.kind === 'done'
          ? init
            ? copy.setUp
            : copy.unlocked
          : null;
  // Where the passphrase goes stays in view; what the vault is and what the button opens wait
  // behind the title's info icon (owner, 2026-11-13).
  return (
    <CrewStepFrame
      kind="vault"
      title={title}
      titleHelp={{ label: copy.aboutLabel, help: init ? copy.initAbout : copy.unlockAbout }}
    >
      <p className="text-supporting text-text-muted">{copy.passphraseGoes}</p>
      {status && (
        <p role="status" className="text-supporting text-text-default">
          {status}
        </p>
      )}
      {state.kind === 'failed' && (
        <p role="alert" className="text-supporting text-text-danger">
          {state.message}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <Button variant="ghost" size="sm" disabled={busy || finished} onClick={notNow}>
          {copy.notNow}
        </Button>
        <Button size="sm" disabled={busy || finished} onClick={open}>
          {init ? copy.initButton : copy.unlockButton}
        </Button>
      </div>
    </CrewStepFrame>
  );
}
