import { StatusDot, type StatusDotTone } from '../ui/status-dot';
import './onboarding.css';

export type PillTone = 'success' | 'warning' | 'info';

/**
 * `info` is a state that is not a fault (an agent signed in with an API key): the neutral dot, not
 * the warning one and not a second hue. The status dot has no info tone, and a blue dot beside
 * green and amber ones read as a third kind of verdict.
 */
const PILL_DOT: Record<PillTone, StatusDotTone> = {
  success: 'success',
  warning: 'warning',
  info: 'neutral',
};

/**
 * A status word with its dot: the shared `StatusDot` (8px, `aria-hidden` beside its word), the
 * same dot and gap as a catalog row's "Configured" (FR-S-10). Every first-run status line uses it:
 * the coding agents' auth state, Llama Server's and Ollama's.
 */
export function StatusPill({ tone, children }: { tone: PillTone; children: React.ReactNode }) {
  return (
    <span className="br-status-pill text-supporting text-text-muted">
      <StatusDot tone={PILL_DOT[tone]} />
      {children}
    </span>
  );
}
