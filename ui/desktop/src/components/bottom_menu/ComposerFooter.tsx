import './pickers.css';
import type { ModelCostRow, SessionCosts } from '../../hooks/useCostTracking';
import { COST_TRACKING_ENABLED } from '../../updates';
import { cn } from '../../utils';
import { ContextWindowIndicator } from '../ContextWindowIndicator';
import { CostTracker } from './CostTracker';
import { DirSwitcher } from './DirSwitcher';

/**
 * The composer's footer line (spec 3.7): the working folder on the left, the
 * context ring and the chat's cost on the right, in 12px muted ink under the
 * card. On Home (`sessionId === null`) only the folder shows.
 *
 * ⚠ The props are the wave-0 contract (WS-PICKERS). `ChatInput` places this
 * under the composer card and owns nothing inside it.
 */
export interface ComposerFooterProps {
  /** The focused chat, or `null` on Home and in a chat that has not started. */
  sessionId: string | null;
  /** The folder the agent works in (full path; the footer shows its basename). */
  workingDir: string;
  /** #44: the folder is choosable only until the chat's first message. */
  workingDirLocked: boolean;
  onWorkingDirChange: (newDir: string) => void;
  /** A chat restarts its agent to change folder; these bracket that restart. */
  onRestartStart?: () => void;
  onRestartEnd?: () => void;
  /** Context used so far, and the model's window. */
  totalTokens?: number;
  tokenLimit: number;
  isTokenLimitLoaded: boolean;
  /** Runs the manual compaction (`ChatInput` submits its compact trigger). */
  onCompact: () => void;
  /** The chat's cost inputs, as `ChatInput` receives them from `BaseChat`. */
  inputTokens?: number;
  outputTokens?: number;
  sessionCosts?: SessionCosts;
  modelCostRows?: ModelCostRow[];
  /** Layout only (the inset that lines the folder up with the card's text). */
  className?: string;
}

export function ComposerFooter({
  sessionId,
  workingDir,
  workingDirLocked,
  onWorkingDirChange,
  onRestartStart,
  onRestartEnd,
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  onCompact,
  inputTokens,
  outputTokens,
  sessionCosts,
  modelCostRows,
  className,
}: ComposerFooterProps) {
  const inChat = sessionId !== null;
  return (
    <div data-testid="composer-footer" className={cn('br-footline', className)}>
      <div className="br-footline__side">
        <DirSwitcher
          sessionId={sessionId ?? undefined}
          locked={workingDirLocked}
          workingDir={workingDir}
          onWorkingDirChange={onWorkingDirChange}
          onRestartStart={onRestartStart}
          onRestartEnd={onRestartEnd}
        />
      </div>
      {inChat && (
        <div className="br-footline__side" data-side="end">
          <ContextWindowIndicator
            totalTokens={totalTokens}
            tokenLimit={tokenLimit}
            isTokenLimitLoaded={isTokenLimitLoaded}
            showRemainingPercent
            onCompact={onCompact}
          />
          {COST_TRACKING_ENABLED && (
            <CostTracker
              inputTokens={inputTokens}
              outputTokens={outputTokens}
              sessionCosts={sessionCosts}
              modelCostRows={modelCostRows}
            />
          )}
        </div>
      )}
    </div>
  );
}
