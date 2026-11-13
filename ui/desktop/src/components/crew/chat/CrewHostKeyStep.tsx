import { useEffect, useState } from 'react';
import type { CrewStepRequest } from '../../../api';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { InfoTip } from '../../ui/info-tip';
import { sanitizeDisplayText } from '../identity/displayText';
import { TrustHostKey } from '../hostKeys/TrustHostKey';
import { trustCopy } from '../onboarding/copy';
import { CopyTextButton, EmbeddedTerminal, TerminalToggle } from '../onboarding/parts';
import { crewHostKeyStepCopy } from './CrewHostKeyStep.copy';
import { CrewStepFrame, dismissStep } from './CrewStepFrame';

type HostKey = Extract<CrewStepRequest, { kind: 'hostKey' }>;

/** A SHA-256 host-key fingerprint, and nothing else. */
const FINGERPRINT = /^SHA256:[A-Za-z0-9+/=]{32,64}$/;

/** A daemon value, shown only when it is a fingerprint and nothing else. */
function fingerprintOf(value: string | null | undefined): string | null {
  const text = sanitizeDisplayText(value);
  return FINGERPRINT.test(text) ? text : null;
}

/**
 * A chat's host-key card (WI-2, and the owner's D2 ruling on AO-12): the
 * server offered a key this computer does not trust, so Crew refused to sign
 * in there. Everything on it is the daemon's: the host and the fingerprints it
 * read. The model can never trust a key; only the person can, here.
 *
 * - **An unknown key** shows what the Crew view's own help shows
 *   (`SetupHostKeyHelp`: the fingerprint and how to verify it, in the same
 *   words, except a last step that sends the person back to this chat rather
 *   than to a Try again button the card does not have), Trust this key when
 *   the card shows a fingerprint (`TrustHostKey`: the person's proof, and the
 *   daemon reads the key again and writes it only when its fingerprint is the
 *   one shown), an "Open a terminal here" button that opens the app's own
 *   terminal in the person's shell (never the model's, and never a landing zone
 *   for a transcript's Run button), and Not now. Once the daemon sees the key
 *   in the known hosts file the chat goes on by itself: it retries the step
 *   once.
 * - **A changed key** offers exactly one thing, the details for IT, as the
 *   Crew view's trust pane does. It is never trusted.
 */
export default function CrewHostKeyStep({
  sessionId,
  id,
  step,
  onHolding,
}: {
  sessionId: string;
  id: string;
  step: HostKey;
  /** Told whether the card's terminal is open, so the card stays while the person uses it. */
  onHolding?: (holding: boolean) => void;
}) {
  const host = sanitizeDisplayText(step.host);
  const offered = fingerprintOf(step.offered);
  if (step.changed) {
    return <ChangedHostKey host={host} offered={offered} known={fingerprintOf(step.known)} />;
  }
  return (
    <UnknownHostKey
      sessionId={sessionId}
      id={id}
      host={host}
      port={step.port}
      login={sanitizeDisplayText(step.login)}
      offered={offered}
      onHolding={onHolding}
    />
  );
}

function UnknownHostKey({
  sessionId,
  id,
  host,
  port,
  login,
  offered,
  onHolding,
}: {
  sessionId: string;
  id: string;
  host: string;
  port: number;
  login: string;
  offered: string | null;
  onHolding?: (holding: boolean) => void;
}) {
  const [terminal, setTerminal] = useState(false);
  const [ended, setEnded] = useState<'dismissed' | 'ended' | null>(null);
  useEffect(() => {
    onHolding?.(terminal);
  }, [terminal, onHolding]);
  const notNow = async () => {
    const dismissed = await dismissStep(sessionId, id).catch(() => false);
    setEnded(dismissed ? 'dismissed' : 'ended');
  };
  return (
    <CrewStepFrame kind="hostKey" title={crewHostKeyStepCopy.unknownTitle(host)}>
      <div className="crew-onboard-stack" data-testid="crew-setup-host-key-unknown">
        <p className="text-body text-text-default">{trustCopy.unknownBody}</p>
        {offered ? (
          <div className="crew-onboard-field">
            <span className="text-supporting text-text-muted">{crewHostKeyStepCopy.read}</span>
            <CopyField value={offered} label={trustCopy.offeredLabel} truncate="middle" />
          </div>
        ) : null}
        {/* The person's own act, with their proof: the daemon re-reads the key and the card
            comes down once the key is in, then the chat tries the step again. */}
        {offered && !ended ? (
          <TrustHostKey target={{ host, port, login }} fingerprint={offered} />
        ) : null}
        <Disclosure label={trustCopy.howToVerify}>
          <ol className="crew-onboard-list text-body text-text-default">
            {crewHostKeyStepCopy.steps(host).map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ol>
        </Disclosure>
      </div>
      {ended ? (
        <p className="text-supporting text-text-muted">
          {ended === 'dismissed' ? crewHostKeyStepCopy.dismissed : crewHostKeyStepCopy.ended}
        </p>
      ) : (
        <>
          <p className="text-supporting text-text-muted">{crewHostKeyStepCopy.goesOn}</p>
          <div className="crew-onboard-row">
            <TerminalToggle open={terminal} onToggle={() => setTerminal((open) => !open)} />
            <Button type="button" variant="ghost" size="sm" onClick={() => void notNow()}>
              {crewHostKeyStepCopy.notNow}
            </Button>
          </div>
        </>
      )}
      {terminal ? <EmbeddedTerminal onClose={() => setTerminal(false)} /> : null}
    </CrewStepFrame>
  );
}

function ChangedHostKey({
  host,
  offered,
  known,
}: {
  host: string;
  offered: string | null;
  known: string | null;
}) {
  const details = [
    ...trustCopy.detailsHeader(host, trustCopy.changedProblem),
    ...(known ? [`${trustCopy.previous}: ${known}`] : []),
    ...(offered ? [`${trustCopy.newKey}: ${offered}`] : []),
  ].join('\n');
  return (
    <CrewStepFrame kind="hostKey" title={crewHostKeyStepCopy.changedTitle(host)}>
      <div className="crew-onboard-stack" data-testid="crew-step-host-key-changed">
        <p className="text-body text-text-default">
          <span>{trustCopy.changedBody}</span>{' '}
          <InfoTip label={trustCopy.changedWhyLabel} help={trustCopy.changedWhy} />
        </p>
        {/* Shown to read and compare, not handed on one by one: the one action is the
            details for IT, which carry both. */}
        {known ? <Fingerprint label={trustCopy.previous} value={known} /> : null}
        {offered ? <Fingerprint label={trustCopy.newKey} value={offered} /> : null}
        <div className="crew-onboard-actions">
          <CopyTextButton text={details} label={trustCopy.copyForIt} />
        </div>
      </div>
    </CrewStepFrame>
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
