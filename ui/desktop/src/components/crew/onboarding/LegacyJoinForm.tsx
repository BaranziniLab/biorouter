import { useState, type FormEvent } from 'react';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Note } from '../../ui/note';
import { SecretInput } from '../../ui/secret-input';
import { useCrew } from '../state/CrewControllerContext';
import { failureMessage } from '../state/observationFailure';
import { crewActionCopy } from '../state/copy';
import { legacyJoinCopy } from './copy';
import { updateJoinContext } from './joinContext';
import { useMounted } from './fields';

/**
 * The invitation-token path (`auth.enroll`), for a workspace whose server cannot join by
 * invitation code, or behind "Other ways to join". The person sends their host a join request —
 * their username and this computer's public device key, never a secret — and pastes back the
 * token the host's older invitation form produced. The broker decides; this only carries it.
 */
export function LegacyJoinForm({
  workspace,
  username,
}: {
  workspace: string;
  username: string | null;
}) {
  const crew = useCrew();
  const mounted = useMounted();
  const [token, setToken] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const publicKey = crew.connection?.public_key ?? '';
  const connectionId = crew.connectionId;

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending || !token.trim()) return;
    setPending(true);
    setError(null);
    try {
      await crew.request(
        'auth.enroll',
        { invitation: token.trim(), public_key: publicKey },
        { mutation: true }
      );
      if (!mounted.current) return;
      setToken('');
      updateJoinContext(connectionId, { joining: false });
      crew.setJoinStatus('joined');
      await crew.refresh();
    } catch (failure) {
      if (mounted.current) setError(failureMessage(failure, crewActionCopy.actionFallback));
    } finally {
      if (mounted.current) setPending(false);
    }
  };

  return (
    <div className="crew-onboard-stack" data-testid="crew-legacy-join">
      {publicKey ? (
        <>
          <p className="text-body text-text-default">{legacyJoinCopy.sendRequest}</p>
          <CopyField
            multiline
            label={legacyJoinCopy.requestLabel}
            value={legacyJoinCopy.request(workspace, username, publicKey)}
          />
        </>
      ) : null}
      <form className="crew-onboard-stack" onSubmit={(event) => void submit(event)}>
        <SecretInput
          aria-label={legacyJoinCopy.tokenName}
          revealLabel={legacyJoinCopy.tokenPlaceholder.toLowerCase()}
          placeholder={legacyJoinCopy.tokenPlaceholder}
          required
          disabled={pending}
          value={token}
          onChange={(event) => setToken(event.target.value)}
        />
        <div className="crew-onboard-actions">
          <Button type="submit" disabled={pending || !token.trim()}>
            {pending ? legacyJoinCopy.submitting : legacyJoinCopy.submit}
          </Button>
        </div>
      </form>
      {error ? (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      ) : null}
    </div>
  );
}
