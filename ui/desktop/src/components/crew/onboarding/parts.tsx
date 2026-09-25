import { useEffect, useId, useRef, useState, type ComponentType, type ReactNode } from 'react';
import type { LucideProps } from 'lucide-react';
import InAppTerminalDock from '../../InAppTerminalDock';
import { Check, Copy, Terminal } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField, COPY_FIELD_FEEDBACK_MS } from '../../ui/copy-field';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { InstitutionName, type KnownInstitution } from '../identity';
import { hostCopy, trustCopy } from './copy';
import './onboarding.css';

/**
 * The small building blocks the onboarding screens share. None of them decides anything; they
 * give every setup card, trust pane and join state the same shape.
 */

/** One centred main-area screen. */
export function SetupScreen({
  children,
  label,
}: {
  children: ReactNode;
  /** The region's accessible name, when the card's own heading is not enough. */
  label?: string;
}) {
  return (
    <div className="crew-onboard-screen" aria-label={label} role={label ? 'region' : undefined}>
      {children}
    </div>
  );
}

/** A setup card: an icon, a heading, then the card's content. */
export function SetupCard({
  icon: Icon,
  title,
  tone = 'neutral',
  children,
  testId,
}: {
  icon?: ComponentType<LucideProps>;
  title: ReactNode;
  tone?: 'neutral' | 'danger';
  children?: ReactNode;
  testId?: string;
}) {
  const titleId = useId();
  return (
    <section
      className="crew-onboard-card crew-crossfade-item"
      data-state="open"
      data-tone={tone}
      aria-labelledby={titleId}
      data-testid={testId}
    >
      <div className="crew-onboard-card-head">
        {Icon ? (
          <span className="crew-onboard-card-icon" aria-hidden="true">
            <Icon className="h-4 w-4" />
          </span>
        ) : null}
        <h2 id={titleId} className="crew-onboard-title min-w-0 text-subheading text-text-default">
          {title}
        </h2>
      </div>
      {children}
    </section>
  );
}

/** The one spinner. It is decoration: the words beside it carry the state. */
export function Spinner() {
  return <span className="crew-onboard-spinner" aria-hidden="true" />;
}

/** "🔒 Private · ucsf" or "Public": a workspace's or a connection's privacy, never animated. */
export function PrivacyLabel({
  mode,
  institutionId,
  known,
}: {
  mode: 'private' | 'public';
  institutionId?: string | null;
  /** The institutions configured providers publish names for, so `ucsf` reads as UCSF (Q2-38). */
  known?: readonly KnownInstitution[] | null;
}) {
  return (
    <span className="crew-onboard-privacy" data-privacy={mode}>
      {/* The broker enforces Crew's mode on its own, independent of this machine's master switch,
          so "(enforcement off)" would be false here. */}
      <PrivacyBadge tier={mode} enforcementOff={false} />
      {mode === 'private' && institutionId ? (
        <>
          <span aria-hidden="true" className="text-text-muted">
            ·
          </span>
          <InstitutionName id={institutionId} known={known} className="text-label" />
        </>
      ) : null}
    </span>
  );
}

/**
 * "Open a terminal here": the app's own terminal dock, embedded below the command a person has to
 * run — exactly as the coding-agent setup card embeds it. Nothing is typed or run for them; the
 * dock is only a shell they can use without leaving the app.
 */
export function TerminalToggle({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return (
    <Button type="button" variant="ghost" size="sm" onClick={onToggle} aria-expanded={open}>
      <Terminal aria-hidden />
      {open ? hostCopy.hideTerminal : hostCopy.openTerminal}
    </Button>
  );
}

export function EmbeddedTerminal({ onClose }: { onClose: () => void }) {
  return (
    <div className="crew-onboard-terminal" data-testid="crew-onboard-terminal">
      <div className="crew-onboard-terminal-mount">
        <InAppTerminalDock open onClose={onClose} onEmptied={onClose} />
      </div>
    </div>
  );
}

/**
 * A button that copies a block of text a person hands to someone else (the details for IT) without
 * showing it at rest. It confirms itself in place, with no toast; if the clipboard refuses, the
 * text appears in a `CopyField` so it can still be selected and copied by hand.
 */
export function CopyTextButton({
  text,
  label,
  variant = 'outline',
}: {
  text: string;
  label: string;
  variant?: 'outline' | 'default' | 'secondary';
}) {
  const [feedback, setFeedback] = useState<'idle' | 'copied' | 'failed'>('idle');
  const [announcement, setAnnouncement] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    []
  );
  const copy = async () => {
    let next: 'copied' | 'failed' = 'copied';
    try {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
      await navigator.clipboard.writeText(text);
    } catch {
      next = 'failed';
    }
    if (timer.current) clearTimeout(timer.current);
    setFeedback(next);
    setAnnouncement(next === 'copied' ? trustCopy.copied : trustCopy.copyFailed);
    if (next === 'copied')
      timer.current = setTimeout(() => {
        timer.current = null;
        setFeedback('idle');
        setAnnouncement('');
      }, COPY_FIELD_FEEDBACK_MS);
  };
  return (
    <div className="crew-onboard-stack">
      <div className="crew-onboard-row">
        <Button type="button" variant={variant} size="sm" onClick={() => void copy()}>
          {feedback === 'copied' ? (
            <Check aria-hidden className="biorouter-check-settled" />
          ) : (
            <Copy aria-hidden />
          )}
          {label}
        </Button>
        <span className="sr-only" aria-live="polite" aria-atomic="true">
          {announcement}
        </span>
        {feedback === 'copied' ? (
          <span className="text-supporting text-text-muted" aria-hidden="true">
            {trustCopy.copied}
          </span>
        ) : null}
      </div>
      {feedback === 'failed' ? (
        <CopyField value={text} label={trustCopy.detailsFallbackLabel} multiline />
      ) : null}
    </div>
  );
}
