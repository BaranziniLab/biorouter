import { useState } from 'react';
import { AlertTriangle, Fingerprint as FingerprintIcon } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { connectionServer, personFromProjection, personLabel } from '../identity';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import { trustCopy } from './copy';
import { useJoinContext } from './joinContext';
import { hostKeyFingerprints } from './joinText';
import { EmbeddedTerminal, CopyTextButton, SetupCard, SetupScreen, TerminalToggle } from './parts';

/**
 * The tier-3 trust panes: the server or the workspace could not be verified, so Crew will not
 * connect. They are chosen only from the daemon's typed codes, never from words in an error.
 *
 * - **A new host key** shows the fingerprint the server offered and how to verify it out of band.
 *   Crew never accepts a key: there is no accept control anywhere, and "Open a terminal here" is
 *   only the person's own shell for comparing and adding the key they verified.
 * - **A changed host key** offers exactly one thing, the details for IT. No accept, no removal
 *   command: softening out-of-band trust at the moment it matters most is what this refuses.
 * - **A different workspace key** than the one pinned: copy the details, or open the settings.
 *
 * Each pane is the `connect` error's home while it is shown, so the failure it explains is not
 * repeated in the connection bar.
 */
export function TrustPane() {
  const { lastConnectFailure } = useCrew();
  useCrewErrorSlot('connect');
  switch (lastConnectFailure?.kind) {
    case 'host_key_unknown':
      return <UnknownHostKey />;
    case 'host_key_changed':
      return <ChangedHostKey />;
    case 'workspace_identity_mismatch':
      return <WorkspaceMismatch />;
    default:
      return null;
  }
}

function useHost() {
  const { connection } = useCrew();
  return connectionServer(connection) || connection?.name || '';
}

function UnknownHostKey() {
  const { lastConnectFailure, connect, isPending } = useCrew();
  const host = useHost();
  const [terminal, setTerminal] = useState(false);
  const { offered } = hostKeyFingerprints(lastConnectFailure?.detail);
  return (
    <SetupScreen>
      <SetupCard
        icon={FingerprintIcon}
        title={trustCopy.unknownTitle(host)}
        testId="crew-trust-unknown"
      >
        <p className="text-body text-text-default">{trustCopy.unknownBody}</p>
        {offered ? (
          <div className="crew-onboard-field">
            <span className="text-supporting text-text-muted">{trustCopy.offered}</span>
            <CopyField value={offered} label={trustCopy.offeredLabel} truncate="middle" />
          </div>
        ) : null}
        <Disclosure label={trustCopy.howToVerify}>
          <div className="crew-onboard-stack">
            <ol className="crew-onboard-list text-body text-text-default">
              {trustCopy.steps(host).map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
            <div className="crew-onboard-row">
              <TerminalToggle open={terminal} onToggle={() => setTerminal((open) => !open)} />
            </div>
            {terminal ? <EmbeddedTerminal onClose={() => setTerminal(false)} /> : null}
          </div>
        </Disclosure>
        <div className="crew-onboard-actions">
          <Button
            type="button"
            disabled={isPending('connect')}
            onClick={() => void connect({ userInitiated: true })}
          >
            {trustCopy.tryAgain}
          </Button>
        </div>
      </SetupCard>
    </SetupScreen>
  );
}

function ChangedHostKey() {
  const { lastConnectFailure } = useCrew();
  const host = useHost();
  const { offered, known } = hostKeyFingerprints(lastConnectFailure?.detail);
  const details = [
    ...trustCopy.detailsHeader(host, trustCopy.changedProblem),
    '',
    lastConnectFailure?.detail || lastConnectFailure?.message || '',
  ]
    .join('\n')
    .trim();
  return (
    <SetupScreen>
      <SetupCard
        icon={AlertTriangle}
        tone="danger"
        title={trustCopy.changedTitle(host)}
        testId="crew-trust-changed"
      >
        <p className="text-body text-text-default">{trustCopy.changedBody}</p>
        {/* Shown to read and compare, not handed on one by one: the one action is the details
            for IT, which carry both. */}
        {known ? <Fingerprint label={trustCopy.previous} value={known} /> : null}
        {offered ? <Fingerprint label={trustCopy.newKey} value={offered} /> : null}
        <div className="crew-onboard-actions">
          <CopyTextButton text={details} label={trustCopy.copyForIt} />
        </div>
      </SetupCard>
    </SetupScreen>
  );
}

function WorkspaceMismatch() {
  const { lastConnectFailure, connectionId, openDialog } = useCrew();
  const host = useHost();
  const context = useJoinContext(connectionId);
  const hostPerson = context.hostUsername
    ? personFromProjection({
        username: context.hostUsername,
        display_name: context.hostDisplayName,
      })
    : null;
  const details = [
    ...trustCopy.detailsHeader(host, trustCopy.workspaceProblem),
    '',
    lastConnectFailure?.detail || lastConnectFailure?.message || '',
  ]
    .join('\n')
    .trim();
  return (
    <SetupScreen>
      <SetupCard
        icon={AlertTriangle}
        tone="danger"
        title={trustCopy.workspaceTitle}
        testId="crew-trust-workspace"
      >
        <p className="text-body text-text-default">
          {trustCopy.workspaceBody(
            hostPerson ? personLabel(hostPerson, 'inline') : trustCopy.yourHost
          )}
        </p>
        <div className="crew-onboard-actions">
          <CopyTextButton text={details} label={trustCopy.copyDetails} />
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => openDialog({ kind: 'connection-settings', connectionId })}
          >
            {trustCopy.connectionSettings}
          </Button>
        </div>
      </SetupCard>
    </SetupScreen>
  );
}

function Fingerprint({ label, value }: { label: string; value: string }) {
  return (
    <div className="crew-onboard-field">
      <span className="text-supporting text-text-muted">{label}</span>
      <code className="font-mono text-code break-all text-text-default" translate="no">
        {value}
      </code>
    </div>
  );
}
