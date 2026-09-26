import { useCallback, useEffect, useState } from 'react';

type CrewCredentialStatus = {
  backend: 'keyring' | 'encrypted_vault';
  initialized: boolean;
  locked: boolean;
};
type CrewCredentialAnswer = CrewCredentialStatus | { cancelled: true };

function credentialsApi(): ((action: 'status' | 'unlock') => Promise<CrewCredentialAnswer>) | null {
  // Absent outside Electron (a browser session, a unit test): then there is no vault to report.
  const api = (window as { electron?: { crewCredentials?: unknown } }).electron?.crewCredentials;
  return typeof api === 'function'
    ? (api as (action: 'status' | 'unlock') => Promise<CrewCredentialAnswer>)
    : null;
}

function isLocked(answer: CrewCredentialAnswer): boolean | null {
  if ('cancelled' in answer) return null;
  return answer.backend === 'encrypted_vault' && answer.locked;
}

/**
 * Whether this profile's Crew vault is locked, from the main process (`crew:credentials`).
 *
 * The keychain backend is never "locked". The status is re-read when the connection changes and
 * whenever `recheck` changes (the bar passes the current error, since a locked vault is the usual
 * reason a Crew action suddenly fails). `unlock()` asks the main process, which prompts for the
 * passphrase natively; it resolves `true` once the vault is unlocked and throws on failure.
 */
export function useVaultStatus(connectionId: string, recheck: unknown) {
  const [locked, setLocked] = useState(false);
  const [unlocking, setUnlocking] = useState(false);

  useEffect(() => {
    const api = credentialsApi();
    if (!api) return;
    let active = true;
    api('status')
      .then((answer) => {
        const next = isLocked(answer);
        if (active && next !== null) setLocked(next);
      })
      .catch(() => {
        // An unreadable status is not a locked vault; the action that needs it will say so.
      });
    return () => {
      active = false;
    };
  }, [connectionId, recheck]);

  const unlock = useCallback(async () => {
    const api = credentialsApi();
    if (!api) return false;
    setUnlocking(true);
    try {
      const next = isLocked(await api('unlock'));
      if (next === null) return false;
      setLocked(next);
      return !next;
    } finally {
      setUnlocking(false);
    }
  }, []);

  return { locked, unlocking, unlock };
}
