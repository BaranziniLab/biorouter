import OnboardingSectionLabel from './OnboardingSectionLabel';

type Chrome = 'card' | 'bare';

interface OnboardingCardShellProps {
  /**
   * `'card'` — the standalone onboarding card, with its own border, tier label,
   * heading and blurb. `'bare'` — the same body with none of that, for the
   * provider catalog, whose accordion row already carries the provider's name,
   * whose panel heading already carries the tier label, and inside which a second
   * bordered box would be a card within a card.
   */
  chrome: Chrome;
  /** The `aria-labelledby` target; only meaningful in `'card'` chrome. */
  titleId: string;
  category: 'institutional' | 'local' | 'commercial';
  label: string;
  title: string;
  description: string;
  children: React.ReactNode;
}

/**
 * The one shell every onboarding setup card wears, so a card and the catalog row
 * that hosts the same body cannot drift apart.
 *
 * ⚠ **The body is never duplicated — only the chrome around it is switched.**
 * The alternative considered and rejected was copying each card's body into the
 * catalog: these bodies hold live state machines (a download in flight, a
 * credential probe, a validated key) and a copy would be two implementations of
 * each, diverging on the first fix that landed on one screen.
 */
export default function OnboardingCardShell({
  chrome,
  titleId,
  category,
  label,
  title,
  description,
  children,
}: OnboardingCardShellProps) {
  if (chrome === 'bare') {
    return <div className="min-w-0">{children}</div>;
  }
  return (
    <section
      aria-labelledby={titleId}
      className="min-w-0 overflow-hidden rounded-xl border border-border-subtle bg-background-card p-5 sm:p-6"
    >
      <OnboardingSectionLabel category={category} label={label} />
      <h2 id={titleId} className="mt-2 text-base font-medium text-text-default">
        {title}
      </h2>
      <p className="text-sm text-text-muted mt-1 mb-5 leading-relaxed">{description}</p>
      {children}
    </section>
  );
}

export type { Chrome as OnboardingCardChrome };
