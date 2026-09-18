import {
  CodingAgentBody,
  StatusPill,
  pillFor,
  useCodingAgents,
} from '../../../onboarding/codingAgentControls';
import type {
  CodingAgentAvailability,
  CodingAgentKind,
} from '../../../onboarding/codingAgentStatus';
import { Button } from '../../../ui/button';

const SETUP_DOCS: Record<CodingAgentKind, string> = {
  claude_code: 'https://code.claude.com/docs/en/setup',
  codex: 'https://developers.openai.com/codex/cli',
};

export default function CodingAgentSetupRecovery({
  kind,
  onRetry,
  isRetrying,
  initialAgents,
}: {
  kind: CodingAgentKind;
  onRetry: () => void;
  isRetrying: boolean;
  initialAgents?: CodingAgentAvailability[];
}) {
  const controls = useCodingAgents(() => {}, undefined, initialAgents);
  const agent = controls.agents?.find((item) => item.kind === kind);
  const name = kind === 'claude_code' ? 'Claude Code' : 'Codex';

  return (
    <div className="space-y-4 text-sm text-text-default" data-testid="coding-agent-setup-recovery">
      <p className="text-text-muted leading-relaxed">
        {name} uses its installed command-line app and your subscription sign-in.
        {kind === 'claude_code'
          ? ' To use an Anthropic API key instead, choose Anthropic under API providers.'
          : ' To use an OpenAI API key instead, choose OpenAI under API providers.'}
      </p>
      {controls.isChecking ? (
        <p role="status">Checking installation and sign-in…</p>
      ) : controls.loadError || !agent ? (
        <div className="space-y-2" role="status">
          <p>
            Biorouter could not check {name}. Check that Biorouter is connected, then try again.
          </p>
          <Button
            variant="outline"
            disabled={controls.isRechecking}
            onClick={() => void controls.refresh(false)}
          >
            {controls.isRechecking ? 'Checking…' : 'Check again'}
          </Button>
        </div>
      ) : (
        <>
          <StatusPill tone={pillFor(agent.auth).tone}>{pillFor(agent.auth).label}</StatusPill>
          <CodingAgentBody
            agent={agent}
            controls={controls}
            onRetryConfiguration={onRetry}
            isRetrying={isRetrying}
          />
        </>
      )}
      <a
        href={SETUP_DOCS[kind]}
        target="_blank"
        rel="noopener noreferrer"
        className="text-sm underline underline-offset-2"
      >
        Open official {name} setup instructions
      </a>
    </div>
  );
}
