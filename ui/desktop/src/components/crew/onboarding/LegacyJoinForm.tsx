import { useId, useState, type FormEvent } from 'react';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Note } from '../../ui/note';
import { SecretInput } from '../../ui/secret-input';
import { useCrew } from '../state/CrewControllerContext';
import { failureMessage } from '../state/observationFailure';
import { crewActionCopy } from '../state/copy';
import { joinStateCopy, legacyJoinCopy } from './copy';
import { updateJoinContext } from './joinContext';
import { useMounted } from './fields';

/**
 * The invitation-token path (`auth.enroll`), for a workspace whose server cannot join by
 * invitation code, or behind "Having trouble joining?". The person sends their host a join
 * request — their username and this computer's public device key, never a secret — and pastes back
 * the token the host's older invitation form produced. The broker decides; this only carries it.
 *
 * `host` makes it the fallback (Q2-35): it opens on the condition ("If Alice asks for it, send
 * this instead:"), keeps the 64-character key folded behind "Show the join request for Alice", and
 * says the token field is only for a token the host sent. Its button is "Join with a token": the
 * person already pressed Join, and a second "Join workspace" read as doing that again (Q3-48).
 * Without `host`, this is the only way in, and the request is shown outright.
 */
export function LegacyJoinForm({
  workspace,
  username,
  host,
}: {
  workspace: string;
  username: string | null;
  /**
   * The host, as the join card's sentences name them ("Alice", `@alice`, or "your host"): marks
   * this as the fallback.
   */
  host?: string;
}) {
  const crew = useCrew();
  const mounted = useMounted();
  const [token, setToken] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [requestShown, setRequestShown] = useState(false);
  const requestId = useId();
  const tokenHelperId = useId();
  const fallback = host !== undefined;
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
      {publicKey && fallback ? (
        <>
          <p className="text-body text-text-default">{joinStateCopy.otherBody(host)}</p>
          <div className="crew-onboard-row">
            <Button
              type="button"
              variant="link"
              className="h-auto p-0"
              aria-expanded={requestShown}
              aria-controls={requestShown ? requestId : undefined}
              onClick={() => setRequestShown((shown) => !shown)}
            >
              {requestShown ? legacyJoinCopy.hideRequest : legacyJoinCopy.showRequest(host)}
            </Button>
          </div>
          {requestShown ? (
            <div id={requestId}>
              <CopyField
                multiline
                label={legacyJoinCopy.requestLabel}
                value={legacyJoinCopy.request(workspace, username, publicKey)}
              />
            </div>
          ) : null}
        </>
      ) : publicKey ? (
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
          aria-describedby={fallback ? tokenHelperId : undefined}
          onChange={(event) => setToken(event.target.value)}
        />
        {fallback ? (
          <p id={tokenHelperId} className="text-supporting text-text-muted">
            {legacyJoinCopy.tokenHelper(host)}
          </p>
        ) : null}
        <div className="crew-onboard-actions">
          <Button type="submit" disabled={pending || !token.trim()}>
            {pending
              ? legacyJoinCopy.submitting
              : fallback
                ? legacyJoinCopy.submitToken
                : legacyJoinCopy.submit}
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
