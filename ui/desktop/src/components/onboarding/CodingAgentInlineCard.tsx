import OnboardingSectionLabel from './OnboardingSectionLabel';
import {
  CodingAgentBody,
  CodingAgentLoadError,
  CodingAgentProvenance,
  StatusPill,
  pillFor,
  useCodingAgents,
} from './codingAgentControls';
import type { CodingAgentAvailability } from './codingAgentStatus';

interface CodingAgentInlineCardProps {
  /**
   * The provider is configured and selected; the host should now let the user pick
   * a model. Mirrors `InstitutionalSetupCard`'s contract (and
   * `ProviderGuard.handleProviderReady`), NOT LlamaServerInlineCard's
   * zero-argument one — these providers expose several models and the card has no
   * business choosing one.
   */
  onSuccess: (providerId: string) => void;
}

/**
 * The standalone onboarding card for the two coding-agent providers.
 *
 * ⚠ **Every moving part lives in `codingAgentControls.tsx`**, which the provider
 * catalog's Public tab also renders. This file is the card's *shell* and nothing
 * else — if you are about to add a state, a sentence or a button here, it belongs
 * next door, or the catalog will quietly not have it.
 */
export {
  CODING_AGENT_TERMINAL_HEIGHT_PX,
  CODING_AGENT_TERMINAL_RESERVE_PX,
} from './codingAgentControls';

export default function CodingAgentInlineCard({ onSuccess }: CodingAgentInlineCardProps) {
  const controls = useCodingAgents(onSuccess);

  const renderAgent = (agent: CodingAgentAvailability) => {
    const pill = pillFor(agent.auth);
    return (
      <div
        key={agent.providerId}
        className="min-w-0 space-y-2 rounded-lg border border-border-subtle bg-background-default p-3 sm:p-4"
        data-testid={`coding-agent-row-${agent.providerId}`}
      >
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
          <p className="text-sm font-medium text-text-default">{agent.displayName}</p>
          <StatusPill tone={pill.tone}>{pill.label}</StatusPill>
        </div>
        <CodingAgentProvenance agent={agent} />
        <CodingAgentBody agent={agent} controls={controls} />
      </div>
    );
  };

  return (
    <section
      aria-labelledby="coding-agent-setup-title"
      className="min-w-0 overflow-hidden rounded-xl border border-border-subtle bg-background-card p-5 sm:p-6"
    >
      <OnboardingSectionLabel category="commercial" label="Commercial · Existing subscription" />
      <h2 id="coding-agent-setup-title" className="mt-2 text-base font-medium text-text-default">
        Bring your own subscription
      </h2>
      <p className="text-sm text-text-muted mt-1 mb-5 leading-relaxed">
        Already pay for a coding-agent plan? Biorouter drives the vendor&apos;s own CLI, already
        installed and signed in on this machine, so turns bill to that plan instead of an API key.
      </p>

      {controls.isChecking ? (
        <div className="flex items-center gap-2 text-xs text-text-muted">
          <div className="w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
          <span>Checking for installed coding agents…</span>
        </div>
      ) : controls.loadError && !controls.agents ? (
        <CodingAgentLoadError message={controls.loadError} controls={controls} />
      ) : (
        <div className="space-y-3">{(controls.agents ?? []).map(renderAgent)}</div>
      )}
    </section>
  );
}
