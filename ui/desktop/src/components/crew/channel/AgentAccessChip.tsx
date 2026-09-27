import { Bot } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { cn } from '../../../utils';
import { channelCopy } from './copy';

export interface AgentAccessChipProps {
  /** Chats with an active grant that posts in this channel. */
  chats: number;
  /** The viewer's own tasks that may still post in this channel. */
  tasks: number;
  /** Open the details pane on the Access tab. */
  onOpen(): void;
  /** Layout only. */
  className?: string;
}

/**
 * "2 chats" — who, besides people, can post in this channel right now (ui-redesign-spec, "The
 * channel header and channel menu"). It is one of the four resting homes of revoke: it opens the
 * Access tab, where each grant can be revoked. It renders nothing when nobody can post, so its
 * presence is itself the signal.
 *
 * The counts come in as props: grants are listed by the Access area, and tasks from the
 * controller's owner-scoped runs. The accessible name is the visible words followed by what they
 * mean, so the visible label is always part of it (WCAG 2.5.3).
 */
export function AgentAccessChip({ chats, tasks, onOpen, className }: AgentAccessChipProps) {
  const safeChats = Math.max(0, Math.floor(chats));
  const safeTasks = Math.max(0, Math.floor(tasks));
  if (safeChats + safeTasks === 0) return null;
  const visible = channelCopy.accessChip(safeChats, safeTasks);
  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      aria-label={channelCopy.accessChipName(visible)}
      className={cn('no-drag tabular-nums', className)}
      onClick={onOpen}
    >
      <Bot aria-hidden="true" />
      {visible}
    </Button>
  );
}
