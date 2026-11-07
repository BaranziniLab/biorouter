import { useEffect, useState } from 'react';
import { crewHostKeysRead, crewHostKeysTrust } from '../../../api';
import type { HostKeyRead, HostKeyTrusted } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import { Button } from '../../ui/button';
import { sanitizeDisplayText } from '../identity/displayText';
import type { HostKeyLookup } from './hostKeyLookup';
import { trustHostKeyCopy } from './trustHostKey.copy';

/** A SHA-256 host-key fingerprint, and nothing else. */
const FINGERPRINT = /^SHA256:[A-Za-z0-9+/=]{32,64}$/;
/**
 * The code the daemon refuses a trust with when a key is already known for the server, or the
 * key it reads again is not the one shown: nothing a second press could fix.
 */
const CHANGED_CODE = 'crew_ssh_host_key_changed';

/** Which server's key: the name OpenSSH looked it up by, and the login the refused step used. */
export interface HostKeyTarget extends HostKeyLookup {
  login?: string | null;
}

interface Refusal {
  code?: string;
  error?: string;
}

function refusalOf(error: unknown): Refusal {
  return error && typeof error === 'object' ? (error as Refusal) : {};
}

function bodyOf(target: HostKeyTarget) {
  const login = sanitizeDisplayText(target.login);
  return { host: target.host, port: target.port, ...(login ? { login } : {}) };
}

/**
 * "Trust this key" for an unknown host key (the owner's D2 ruling on AO-12), the same in Crew's
 * trust panes and a chat's host-key card: the fixed warning, and a button that posts
 * `POST /crew/host-keys/trust` with the person's proof (`userActionHeaders()`). The daemon reads
 * the key again and writes it only when no key is known for the server and its fingerprint is
 * the one shown here. A changed key never gets this control: its panes offer only the details
 * for IT. Renders nothing for a value that is not a fingerprint.
 */
export function TrustHostKey({
  target,
  fingerprint,
  trustedNote,
  onTrusted,
}: {
  target: HostKeyTarget;
  /** The fingerprint shown above it: the daemon's value. */
  fingerprint: string;
  /** Said after "Trusted.": what happens next on this surface. */
  trustedNote?: string;
  onTrusted?: (trusted: HostKeyTrusted) => void;
}) {
  const [state, setState] = useState<'idle' | 'trusting' | 'trusted' | 'refused'>('idle');
  const [message, setMessage] = useState('');
  const [settled, setSettled] = useState(false);
  if (!FINGERPRINT.test(fingerprint)) return null;
  const trust = async () => {
    setState('trusting');
    try {
      const response = await crewHostKeysTrust({
        headers: await userActionHeaders(),
        body: { ...bodyOf(target), fingerprint },
      });
      const trusted = response.data as HostKeyTrusted | undefined;
      if (trusted) {
        setState('trusted');
        setMessage(
          trusted.already_trusted ? trustHostKeyCopy.alreadyTrusted : trustHostKeyCopy.trusted
        );
        onTrusted?.(trusted);
        return;
      }
      const refusal = refusalOf(response.error);
      setState('refused');
      setMessage(sanitizeDisplayText(refusal.error) || trustHostKeyCopy.refused);
      setSettled(refusal.code === CHANGED_CODE);
    } catch {
      setState('refused');
      setMessage(trustHostKeyCopy.refused);
    }
  };
  return (
    <div className="crew-onboard-stack" data-testid="crew-trust-host-key">
      <p className="text-supporting text-text-muted">{trustHostKeyCopy.warning}</p>
      {state === 'trusted' ? (
        <p className="text-supporting text-text-default" role="status">
          {trustedNote ? `${message} ${trustedNote}` : message}
        </p>
      ) : null}
      {state === 'refused' ? (
        <p className="text-supporting text-text-default" role="alert">
          {message}
        </p>
      ) : null}
      {state !== 'trusted' && !settled ? (
        <div className="crew-onboard-row">
          <Button
            type="button"
            size="sm"
            disabled={state === 'trusting'}
            onClick={() => void trust()}
          >
            {state === 'trusting' ? trustHostKeyCopy.trusting : trustHostKeyCopy.trust}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/** Where a pane's fingerprint stands: given, being read, read, or not readable. */
export interface HostKeyFingerprint {
  fingerprint: string | null;
  reading: boolean;
  /** Why none could be read, as the daemon said it. */
  message: string | null;
}

/**
 * The fingerprint a trust pane shows: the one the refusal printed (`given`), else the one this
 * computer reads from the server once (`POST /crew/host-keys/read`, with the person's proof).
 * OpenSSH prints none for an unknown key under strict checking, so without this read Crew's
 * panes could never offer Trust this key. `target` is `null` where no read may be made (a route
 * through a jump host, or a refusal that named no host).
 */
export function useHostKeyFingerprint(
  target: HostKeyTarget | null,
  given: string | null
): HostKeyFingerprint {
  const [read, setRead] = useState<HostKeyFingerprint>({
    fingerprint: null,
    reading: false,
    message: null,
  });
  const key = target ? `${target.host}\n${target.port}\n${target.login ?? ''}` : '';
  useEffect(() => {
    if (given || !target) return undefined;
    let cancelled = false;
    setRead({ fingerprint: null, reading: true, message: null });
    void (async () => {
      try {
        const response = await crewHostKeysRead({
          headers: await userActionHeaders(),
          body: bodyOf(target),
        });
        if (cancelled) return;
        const found = (response.data as HostKeyRead | undefined)?.fingerprint;
        const fingerprint = typeof found === 'string' && FINGERPRINT.test(found) ? found : null;
        setRead({
          fingerprint,
          reading: false,
          message: fingerprint
            ? null
            : sanitizeDisplayText(refusalOf(response.error).error) || trustHostKeyCopy.unreadable,
        });
      } catch {
        if (!cancelled) {
          setRead({ fingerprint: null, reading: false, message: trustHostKeyCopy.unreadable });
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // The target is read by value: `key` names it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [given, key]);
  if (given) return { fingerprint: given, reading: false, message: null };
  if (!target) return { fingerprint: null, reading: false, message: null };
  return read;
}
