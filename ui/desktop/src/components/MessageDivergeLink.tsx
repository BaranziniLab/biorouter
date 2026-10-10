import { useState } from 'react';
import { MoreHorizontal } from './icons/app-icons';
import { useDiverge } from '../hooks/useDiverge';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './ui/dropdown-menu';
import { TranscriptIconButton } from './transcript/TranscriptIconButton';
import { transcriptCopy } from './transcript/copy';

interface MessageDivergeLinkProps {
  /** The session this message belongs to — the chat that will be branched. */
  sessionId: string;
  /** `created` timestamp (ms) of this assistant message. The branch is trimmed
   * to end exactly at this answer. */
  truncateAfterMs?: number;
  /** Durable id of this assistant message. Preferred over the timestamp when
   * resolving the exact branch point. */
  truncateAfterId?: string;
}

/**
 * The ⋯ on a finished assistant message, holding its second-tier action:
 * "Diverge from here" (Crew's pattern: the cluster shows the one everyday action,
 * Copy, and everything else goes behind ⋯).
 *
 * Diverging branches the chat into a brand-new session that inherits the full
 * history up to this answer, so the new chat resumes from exactly here while
 * the original stays untouched. The branch opens in a new Biorouter window.
 */
export default function MessageDivergeLink({
  sessionId,
  truncateAfterMs,
  truncateAfterId,
}: MessageDivergeLinkProps) {
  const [busy, setBusy] = useState(false);
  const { diverge } = useDiverge();

  const handleDiverge = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await diverge(sessionId, truncateAfterMs, truncateAfterId);
    } finally {
      setBusy(false);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <TranscriptIconButton
          label={transcriptCopy.message.more}
          icon={<MoreHorizontal aria-hidden="true" />}
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem disabled={busy} onSelect={() => void handleDiverge()}>
          {busy ? transcriptCopy.message.diverging : transcriptCopy.message.divergeFromHere}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
