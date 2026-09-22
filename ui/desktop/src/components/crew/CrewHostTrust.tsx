export default function CrewHostTrust() {
  const profileRoot = window.appConfig?.get('BIOROUTER_DEV_PROFILE_ROOT');
  return (
    <details className="crew-trust">
      <summary>New host or changed host key? Verify SSH trust</summary>
      <ol>
        <li>
          Obtain the SSH host’s public key or SHA-256 fingerprint through your institution’s trusted
          directory, administrator, or cloud control plane. Verify every jump host as well.
        </li>
        <li>
          Use your existing SSH trust setup to compare the fingerprint. Import the verified full
          host public key into the known-hosts file used by this connection. A fingerprint alone is
          not a known-hosts entry.
        </li>
        <li>
          Return to Crew and choose Reconnect, then Authenticate if the host requests a password or
          MFA.
        </li>
      </ol>
      <p>
        {typeof profileRoot === 'string' && profileRoot ? (
          <>
            This isolated profile uses <code>{profileRoot}/home/.ssh/known_hosts</code>. Its fixture
            keys must be provisioned separately from your personal SSH setup.
          </>
        ) : (
          <>
            Crew uses your existing OpenSSH configuration and known-hosts settings, normally{' '}
            <code>~/.ssh/known_hosts</code>.
          </>
        )}
      </p>
      <p>
        A changed key requires independent re-verification. Crew never accepts an unknown host
        automatically. The workspace public key in your invitation is a separate broker identity
        pin.
      </p>
    </details>
  );
}
