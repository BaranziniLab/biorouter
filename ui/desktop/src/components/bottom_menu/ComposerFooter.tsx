import './pickers.css';
import type { ModelCostRow, SessionCosts } from '../../hooks/useCostTracking';
import { COST_TRACKING_ENABLED } from '../../updates';
import { cn } from '../../utils';
import { ContextWindowIndicator } from '../ContextWindowIndicator';
import { CostTracker } from './CostTracker';
import { DirSwitcher } from './DirSwitcher';
import { FolderAccess } from './FolderAccess';

/**
 * The composer's footer line (spec 3.7): the working folder on the left, the
 * context ring and the chat's cost on the right, in 12px muted ink under the
 * card. On Home (`sessionId === null`) only the folder shows, and a decision
 * chat (`showContext={false}`) shows no ring.
 *
 * ⚠ The props are the wave-0 contract (WS-PICKERS). `ChatInput` places this
 * under the composer card and owns nothing inside it.
 */
export interface ComposerFooterProps {
  /** The focused chat, or `null` on Home and in a chat that has not started. */
  sessionId: string | null;
  /**
   * The folder the agent works in (full path; the footer shows its basename),
   * or `null` while an existing chat's folder is not known yet. The folder is
   * then left out rather than named by a default the chat does not use
   * (V2-R7-3); the line keeps its height.
   */
  workingDir: string | null;
  /** #44: the folder is choosable only until the chat's first message. */
  workingDirLocked: boolean;
  onWorkingDirChange: (newDir: string) => void;
  /** A chat restarts its agent to change folder; these bracket that restart. */
  onRestartStart?: () => void;
  onRestartEnd?: () => void;
  /**
   * Whether the chat has a context ring (default true). A chat whose model is a
   * decision model has none: each message is asked on its own, so there is no
   * running context to measure and nothing to compact. The cost stays.
   */
  showContext?: boolean;
  /** Price the chat's usage ledger only (a decision chat): see `CostTracker`. */
  costFromLedgerOnly?: boolean;
  /** Context used so far, and the model's window. */
  totalTokens?: number;
  tokenLimit: number;
  isTokenLimitLoaded: boolean;
  /**
   * The model the ring must measure for now (`contextWindowModelKey`: the chat's
   * binding, else the app's selection), or `null` while the chat loads and its
   * binding is not known. The ring draws a window only when `tokenLimitModel`
   * names this same model, so the new-chat model's window never stands in for
   * the chat's (V2-R4-1). Omitted, a loaded window is taken to be the chat's.
   */
  boundModel?: string | null;
  /** The model `tokenLimit` was looked up for, spelled like `boundModel`. */
  tokenLimitModel?: string | null;
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
  showContext = true,
  costFromLedgerOnly = false,
  totalTokens,
  tokenLimit,
  isTokenLimitLoaded,
  boundModel,
  tokenLimitModel,
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
        {workingDir !== null && (
          <DirSwitcher
            sessionId={sessionId ?? undefined}
            locked={workingDirLocked}
            workingDir={workingDir}
            onWorkingDirChange={onWorkingDirChange}
            onRestartStart={onRestartStart}
            onRestartEnd={onRestartEnd}
          />
        )}
        {sessionId !== null && <FolderAccess key={sessionId} sessionId={sessionId} />}
      </div>
      {inChat && (
        <div className="br-footline__side" data-side="end">
          {showContext && (
            <ContextWindowIndicator
              sessionId={sessionId}
              totalTokens={totalTokens}
              tokenLimit={tokenLimit}
              isTokenLimitLoaded={isTokenLimitLoaded}
              boundModel={boundModel}
              tokenLimitModel={tokenLimitModel}
              showRemainingPercent
              onCompact={onCompact}
            />
          )}
          {COST_TRACKING_ENABLED && (
            <CostTracker
              inputTokens={inputTokens}
              outputTokens={outputTokens}
              sessionCosts={sessionCosts}
              modelCostRows={modelCostRows}
              ledgerOnly={costFromLedgerOnly}
            />
          )}
        </div>
      )}
    </div>
  );
}
