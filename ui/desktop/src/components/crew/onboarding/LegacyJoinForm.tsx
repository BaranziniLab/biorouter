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

/** One token submit, and what the form around it shows while it runs. */
export interface TokenJoin {
  token: string;
  setToken: (token: string) => void;
  pending: boolean;
  error: string | null;
  submit: (event: FormEvent<HTMLFormElement>) => Promise<void>;
  /** This computer's public device key, which the join request carries and the submit sends. */
  publicKey: string;
}

/**
 * The one submit of a token from the host's older invitation form (`auth.enroll`, with this
 * computer's public device key), shared by the token-only path below and the token field behind
 * "Having trouble joining?" (`JoinStatusCard`). The broker decides; this only carries it. On
 * success the join is over: the join context stops saying "joining" and the workspace refreshes.
 */
export function useTokenJoin(): TokenJoin {
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

  return { token, setToken, pending, error, submit, publicKey };
}

/**
 * The invitation-token path, for a workspace whose server cannot join by invitation code: the only
 * way in there. The person sends their host the join request — their username and this computer's
 * public device key, never a secret — shown outright, and pastes back the token the host's older
 * invitation form produced. The fallback behind "Having trouble joining?" is `JoinStatusCard`'s
 * own, over the same submit (`useTokenJoin`).
 */
export function LegacyJoinForm({
  workspace,
  username,
}: {
  workspace: string;
  username: string | null;
}) {
  const { token, setToken, pending, error, submit, publicKey } = useTokenJoin();

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
