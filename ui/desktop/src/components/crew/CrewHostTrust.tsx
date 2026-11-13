import { CopyField } from '../ui/copy-field';
import { signInCopy } from './auth/copy';
import './auth/auth.css';

/** The known hosts file this computer's SSH reads for Crew connections. */
export function knownHostsPath(profileRoot: unknown): string {
  return typeof profileRoot === 'string' && profileRoot
    ? `${profileRoot.replace(/\/+$/, '')}/home/.ssh/known_hosts`
    : '~/.ssh/known_hosts';
}

/**
 * The body of "Trouble signing in?" under the sign-in terminal: use the same credentials as for
 * the server, add a jump host in Connection settings if IT gave one, and the known hosts file
 * Crew checks servers against: an isolated development profile's own file when one is set.
 *
 * Verifying a new or changed host key lives on the trust panes (`TrustPane`, `SetupHostKeyHelp`)
 * and a chat's host-key card. Only those offer Trust this key, and only for an unknown key whose
 * fingerprint they show (`TrustHostKey`); a sign-in terminal has no fingerprint to compare, so
 * nothing here trusts a key, and a changed key is never trusted anywhere.
 */
export default function CrewHostTrust() {
  const profileRoot = window.appConfig?.get('BIOROUTER_DEV_PROFILE_ROOT');
  return (
    <div className="crew-signin-help text-body text-text-default">
      <p>{signInCopy.sameCredentials}</p>
      <p>{signInCopy.jumpHost}</p>
      <p>{signInCopy.knownHosts}</p>
      <CopyField value={knownHostsPath(profileRoot)} label={signInCopy.knownHostsLabel} />
    </div>
  );
}
