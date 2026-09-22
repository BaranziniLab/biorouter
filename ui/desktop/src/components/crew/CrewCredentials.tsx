import { useEffect, useState } from 'react';

type Status = { backend: 'keyring' | 'encrypted_vault'; initialized: boolean; locked: boolean };

export default function CrewCredentials() {
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const act = async (action: 'status' | 'init' | 'unlock' | 'lock') => {
    setBusy(true);
    setError('');
    try {
      const result = await window.electron.crewCredentials(action);
      if (!('cancelled' in result)) setStatus(result);
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Crew credential action failed.');
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    void act('status');
  }, []);
  return (
    <section aria-label="Crew credentials" className="crew-credentials">
      <strong>Crew credentials</strong>
      <p className="crew-small">
        {!status
          ? 'Checking credential storage…'
          : status.backend === 'keyring'
            ? 'Using the operating system keyring (default).'
            : `Encrypted profile vault is ${status.locked ? 'locked' : 'unlocked'}.`}
      </p>
      {status?.backend === 'keyring' && (
        <>
          <p className="crew-small">
            An encrypted vault can be initialized only in a fresh Crew profile. Existing identities
            are not migrated. Its passphrase is separate from the daemon approval secret.
          </p>
          <button className="crew-button" disabled={busy} onClick={() => void act('init')}>
            Initialize encrypted vault
          </button>
        </>
      )}
      {status?.backend === 'encrypted_vault' && (
        <button
          className="crew-button"
          disabled={busy}
          onClick={() => void act(status.locked ? 'unlock' : 'lock')}
        >
          {status.locked ? 'Unlock vault' : 'Lock vault'}
        </button>
      )}
      <button className="crew-button" disabled={busy} onClick={() => void act('status')}>
        Refresh credential status
      </button>
      {error && (
        <p role="alert" className="crew-small">
          {error}
        </p>
      )}
    </section>
  );
}
