import { ChatState } from '../types/chatState';
import { Spinner } from './ui/spinner';
import { transcriptCopy } from './transcript/copy';

interface LoadingBioRouterProps {
  message?: string;
  chatState?: ChatState;
}

const activityCopy = transcriptCopy.activity;

const STATE_MESSAGES: Record<ChatState, string> = {
  [ChatState.LoadingConversation]: activityCopy.loadingChat,
  [ChatState.Thinking]: activityCopy.thinkingShort,
  [ChatState.Streaming]: activityCopy.working,
  [ChatState.WaitingForUserInput]: activityCopy.waiting,
  [ChatState.Compacting]: activityCopy.compactingShort,
  [ChatState.Idle]: activityCopy.working,
  [ChatState.RestartingAgent]: activityCopy.restarting,
};

/**
 * A quiet loading line: the one spinner and a short state word in 12px muted
 * ink ("Loading chat…", "Loading messages…"). It used to draw the working ring
 * and glow, a third "working" loop beside the composer's edge and the tool
 * pulse; a load is not the agent working, so it takes the plain spinner.
 */
const LoadingBioRouter = ({ message, chatState = ChatState.Idle }: LoadingBioRouterProps) => {
  const displayMessage = message || STATE_MESSAGES[chatState];

  return (
    <div className="w-full">
      <div
        data-testid="loading-biorouter"
        aria-live="polite"
        className="inline-flex items-center gap-2 text-supporting text-text-muted"
      >
        <Spinner size={14} />
        <span>{displayMessage}</span>
      </div>
    </div>
  );
};

export default LoadingBioRouter;
