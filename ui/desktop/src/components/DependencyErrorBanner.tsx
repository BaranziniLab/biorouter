/**
 * DependencyErrorBanner.tsx
 *
 * An install/setup failure, plus the one action that can actually resolve it.
 *
 * Every one of these banners used to be an error string in a red box — accurate,
 * and a dead end. Biorouter ships an agent with shell access that is good at
 * exactly this class of problem, so the error now carries a button that opens a
 * NEW chat briefed with the failure, the command behind it and the machine's
 * environment. New window, not the current chat: the user hit this mid-task.
 */
import { Button } from './ui/button';
import { launchDependencyDebugSession } from '../utils/launchDependencyDebug';
import type { DependencyFailure } from '../utils/dependencyDebugPrompt';

interface DependencyErrorBannerProps {
  /** The message already being shown to the user. */
  error: string;
  /** Everything else known about the failure. `error` is filled in from above. */
  failure: Omit<DependencyFailure, 'environment' | 'error'>;
  /** Suppress the action where a debugging session cannot help (e.g. a refusal). */
  hideDebugAction?: boolean;
  className?: string;
}

export function DependencyErrorBanner({
  error,
  failure,
  hideDebugAction,
  className = '',
}: DependencyErrorBannerProps) {
  return (
    <div
      className={`rounded-element border border-border-danger/40 bg-background-danger/10 p-3 ${className}`}
    >
      <div className="flex min-w-0 items-start justify-between gap-3">
        {/* min-w-0 so a long single-token error (a path, a URL) wraps inside the
            banner instead of pushing the button out of the modal. */}
        <p className="min-w-0 break-words text-body text-text-danger">{error}</p>
        {!hideDebugAction && (
          <Button
            variant="secondary"
            size="sm"
            className="shrink-0"
            onClick={() => void launchDependencyDebugSession({ ...failure, error })}
            title="Open a new chat with this error and let Biorouter work it out"
          >
            Debug with Biorouter
          </Button>
        )}
      </div>
    </div>
  );
}
