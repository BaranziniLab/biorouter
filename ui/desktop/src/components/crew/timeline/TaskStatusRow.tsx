import { useCallback, useId, useState, type AnimationEvent } from 'react';
import { useNavigate } from 'react-router-dom';
import { Avatar } from '../../ui/avatar';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Bot, MoreHorizontal } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { identityCopy } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { runStatusPresentation, type RunStatusPresentation } from '../state/crewStatus';
import { timelineCopy } from './copy';
import type { TimelineTask } from './groupMessages';
import { useMenuCopy, useTimelineCopy } from './TimelineCopy';
import { useTimeline } from './TimelineContext';

/**
 * The viewer's own agent task, as a line in the log — not a card (design.md
 * D-17): a square agent tile, "Your agent · {status word}", the task's first
 * line muted beneath, the one inline action, a visible Stop while the task can
 * be stopped, and ⋯: Show in chat history, Copy error (when there is one), a
 * separator, then Copy task ID — the person's actions first, the machine string
 * last (Q2-62). A copy answers in the menu ("Copied", then it closes).
 *
 * It sits under the task's result, or under the task's post until a result
 * lands, never between the two (`groupMessages`, Q2-62).
 *
 * - **Open and Review go to THIS task's own conversation**, `run.session_id`
 *   (baseline critique F-8: the old card opened a different chat). Crew
 *   approves nothing; a waiting approval is answered in that conversation.
 * - **Stop confirms first**: it opens the `stop-task` confirmation, which calls
 *   the existing cancel route. "Try stopping again" retries a stop the person
 *   already confirmed, so it replaces Stop rather than sitting beside it.
 * - Status words and tones come from `runStatusPresentation`, so a status this
 *   renderer does not know yet still reads as words, never as a code.
 * - The run ID is a React key and a clipboard value only; it is never rendered.
 */
export function TaskStatusRow({ task }: { task: TimelineTask }) {
  const { run } = task;
  const crew = useCrew();
  const navigate = useNavigate();
  const copy = useTimelineCopy();
  const { readOnly, setActiveRow, registerTaskRow, highlightedRunId, onHighlightEnd } =
    useTimeline();
  const [menuOpen, setMenuOpen] = useState(false);
  const menuCopy = useMenuCopy<'error' | 'id'>(copy, setMenuOpen);
  const labelId = useId();
  const presentation = runStatusPresentation(run.status);
  const stopping = crew.isPending('run.cancel');
  const highlighted = highlightedRunId === run.run_id;

  const register = useCallback(
    (element: HTMLDivElement | null) => registerTaskRow(run.run_id, element),
    [registerTaskRow, run.run_id]
  );
  const openConversation = () =>
    navigate(`/pair?resumeSessionId=${encodeURIComponent(run.session_id)}`);
  const confirmStop = () =>
    crew.openDialog({ kind: 'confirm', confirm: { action: 'stop-task', runId: run.run_id } });
  const endHighlight = (event: AnimationEvent<HTMLDivElement>) => {
    if (event.target === event.currentTarget && highlighted) onHighlightEnd(run.run_id);
  };

  return (
    <div
      ref={register}
      className={cn('crew-task-row', highlighted && 'crew-highlight')}
      role="group"
      aria-labelledby={labelId}
      data-crew-row=""
      tabIndex={-1}
      onFocus={() => setActiveRow(`task-${run.run_id}`)}
      onAnimationEnd={endHighlight}
    >
      <div className="crew-message-gutter">
        <Avatar size={32} shape="square" icon={<Bot aria-hidden />} />
      </div>
      <div className="crew-task-main">
        <p id={labelId} className="crew-task-status text-label">
          {identityCopy.yourAgent}
          {identityCopy.separator}
          <StatusWord presentation={presentation} />
        </p>
        {task.title && (
          <p className="crew-task-title text-secondary text-text-muted">{task.title}</p>
        )}
      </div>
      <div className="crew-task-actions">
        {presentation.action === 'open' && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label={timelineCopy.taskOpenLabel}
            disabled={readOnly}
            onClick={openConversation}
          >
            {timelineCopy.taskOpen}
          </Button>
        )}
        {presentation.action === 'review' && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            aria-label={timelineCopy.taskReviewLabel}
            disabled={readOnly}
            onClick={openConversation}
          >
            {timelineCopy.taskReview}
          </Button>
        )}
        {presentation.action === 'stop-again' && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={readOnly || stopping}
            onClick={() => void crew.cancelRun(run.run_id)}
          >
            {timelineCopy.taskStopAgain}
          </Button>
        )}
        {presentation.stoppable && presentation.action !== 'stop-again' && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="text-text-danger"
            aria-label={timelineCopy.taskStopLabel}
            disabled={readOnly || stopping}
            onClick={confirmStop}
          >
            {timelineCopy.taskStop}
          </Button>
        )}
        <DropdownMenu open={menuOpen} onOpenChange={menuCopy.onOpenChange}>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  shape="round"
                  aria-label={timelineCopy.taskMoreActions}
                  className="text-text-muted"
                >
                  <MoreHorizontal aria-hidden />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent>{timelineCopy.taskMoreActions}</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end">
            <DropdownMenuItem disabled={readOnly} onSelect={() => navigate('/sessions')}>
              {timelineCopy.taskOpenHistory}
            </DropdownMenuItem>
            {run.error && (
              <DropdownMenuItem
                data-crew-copy-state={menuCopy.state('error')}
                onSelect={menuCopy.select('error', run.error)}
              >
                {menuCopy.label('error', timelineCopy.taskCopyError)}
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              data-crew-copy-state={menuCopy.state('id')}
              onSelect={menuCopy.select('id', run.run_id)}
            >
              {menuCopy.label('id', timelineCopy.taskCopyId)}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}

/**
 * The status word in its tone: a gentle pulse while it runs (static under
 * reduced motion), a warning badge when it needs the person, danger ink when it
 * could not finish, muted otherwise.
 */
function StatusWord({ presentation }: { presentation: RunStatusPresentation }) {
  switch (presentation.tone) {
    case 'running':
      return <span className="crew-task-running">{presentation.word}</span>;
    case 'warning':
      return (
        <Badge tone="warning" className="crew-task-badge">
          {presentation.word}
        </Badge>
      );
    case 'danger':
      return <span className="text-text-danger">{presentation.word}</span>;
    default:
      return <span className="text-text-muted">{presentation.word}</span>;
  }
}
