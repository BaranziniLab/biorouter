import { useId, useMemo, useState } from 'react';
import { MessageSquare, MoreHorizontal } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { StatusDot, type StatusDotTone } from '../../ui/status-dot';
import type { ObservedRun } from '../crewApi';
import { cn } from '../../../utils';
import { useCrew } from '../state/CrewControllerContext';
import { runStatusPresentation, type RunTone } from '../state/crewStatus';
import {
  accessRows,
  accessStatusTone,
  channelLabels,
  isFinishedRun,
  type AccessRow,
} from './accessRows';
import { accessCopy } from './copy';
import { isUnconfirmedRevocation } from './useCrewGrants';
import { useWorkspaceGrants } from './useWorkspaceGrants';

export interface AgentsSectionProps {
  /**
   * Scroll to and highlight a task's row in the timeline. The section selects the task's channel
   * first; the highlight belongs to whoever renders the timeline, so the layout wires it here.
   */
  onShowTask?(run: ObservedRun): void;
  /** Layout only. */
  className?: string;
}

const RUN_DOT: Record<RunTone, StatusDotTone> = {
  running: 'neutral',
  warning: 'warning',
  danger: 'danger',
  muted: 'idle',
};

/**
 * The Crew sidebar's Agents section (ui-redesign-spec, "The Crew sidebar"): at rest, your tasks that
 * are running, waiting for approval or unconfirmed — with a "Needs you" badge when one waits for
 * your approval — and the chats connected to this workspace. A task row jumps to and highlights the
 * task in its channel; a chat row opens that chat's Chat access pane. The header's `⋯` also shows
 * revoked chats and finished tasks.
 *
 * It renders nothing when it has no row to show, so the sidebar can place it unconditionally. Its
 * rows wear the Crew sidebar's own section and row classes (`crew-sidebar-*`, in the sidebar's
 * stylesheet), because it only ever renders inside that sidebar and must read as one of its lists.
 */
export function AgentsSection({ onShowTask, className }: AgentsSectionProps) {
  const { snapshot, runs, teamId, channelId, selectTeam, selectChannel, openPane } = useCrew();
  const grants = useWorkspaceGrants();
  const [showAll, setShowAll] = useState(false);
  const headingId = useId();
  const labels = useMemo(() => channelLabels(snapshot), [snapshot]);

  const tasks = useMemo(
    () => runs.filter((run) => showAll || !isFinishedRun(run)),
    [runs, showAll]
  );
  const chatRows = useMemo(
    () =>
      accessRows(grants.grants, { snapshot, runs, isUnconfirmed: isUnconfirmedRevocation }).filter(
        (row) => row.kind === 'chat'
      ),
    [grants.grants, snapshot, runs]
  );
  const chats = showAll
    ? chatRows
    : chatRows.filter((row) => row.status === 'active' || row.status === 'unconfirmed');
  const hasRowsAtRest =
    runs.some((run) => !isFinishedRun(run)) ||
    chatRows.some((row) => row.status === 'active' || row.status === 'unconfirmed');

  if (!hasRowsAtRest && !(showAll && (tasks.length > 0 || chats.length > 0))) return null;

  const showTask = (run: ObservedRun) => {
    if (run.channel_id !== channelId) {
      const target = snapshot?.channels.find((item) => item.id === run.channel_id);
      if (target) {
        if (target.team_id !== teamId) selectTeam(target.team_id);
        selectChannel(target.id);
      }
    }
    onShowTask?.(run);
  };

  const openChatAccess = (row: AccessRow) =>
    openPane({ mode: 'chat-access', sessionId: row.sessionId });

  return (
    <div
      className={cn('crew-sidebar-section', className)}
      data-crew-attention="agents"
      data-testid="crew-agents-section"
    >
      <div className="flex min-w-0 items-center justify-between pr-1">
        <h2 id={headingId} className="crew-sidebar-section-label text-caps">
          {accessCopy.agents}
        </h2>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              shape="round"
              className="no-drag"
              aria-label={accessCopy.agentsOptions}
            >
              <MoreHorizontal aria-hidden />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuCheckboxItem
              checked={showAll}
              onCheckedChange={(checked) => setShowAll(checked === true)}
            >
              {accessCopy.agentsShowAll}
            </DropdownMenuCheckboxItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <ul role="list" aria-labelledby={headingId} className="crew-sidebar-list">
        {tasks.map((run) => {
          const presentation = runStatusPresentation(run.status);
          const channel = labels.get(run.channel_id) ?? accessCopy.unknownChannel;
          return (
            <li key={`task:${run.run_id}`}>
              <button
                type="button"
                className="crew-sidebar-row no-drag"
                onClick={() => showTask(run)}
                data-testid="crew-agents-task"
                data-run-status={run.status}
              >
                <StatusDot
                  tone={RUN_DOT[presentation.tone]}
                  live={presentation.tone === 'running'}
                />
                <span className="crew-sidebar-row-name">
                  <bdi>{channel}</bdi>
                  {accessCopy.agentsSeparator}
                  {presentation.word}
                </span>
                {run.status === 'waiting_for_approval' ? (
                  <Badge tone="warning">{accessCopy.needsYou}</Badge>
                ) : null}
              </button>
            </li>
          );
        })}
        {chats.map((row) => (
          <li key={`chat:${row.key}`}>
            <button
              type="button"
              className="crew-sidebar-row no-drag"
              onClick={() => openChatAccess(row)}
              data-testid="crew-agents-chat"
              data-access-status={row.status}
            >
              <MessageSquare className="crew-sidebar-row-icon" aria-hidden />
              <span className="crew-sidebar-row-name">
                <bdi className="text-text-default">{row.title}</bdi>
                <span className="text-text-muted">
                  {accessCopy.agentsSeparator}
                  <bdi>{row.destination}</bdi>
                </span>
              </span>
              {row.status !== 'active' ? (
                <Badge tone={accessStatusTone(row.status)}>{row.statusLabel}</Badge>
              ) : null}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
