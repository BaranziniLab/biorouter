import {
  Check,
  CircleDotDashed,
  CircleHelp,
  CircleIcon,
  MoreHorizontal,
  PauseCircle,
  StopCircle,
  X,
} from '../icons/app-icons';
import type { SubagentRunStatus } from '../../utils/sessionSubagents';

type Glyph = typeof CircleIcon;

/**
 * A run's state, told apart by SHAPE (never by colour alone), shared by the chat
 * summary's roster and History's run rows so the two cannot disagree. Keyed by
 * status, so a status the daemon adds fails the build here.
 *
 * Cancelling and incomplete share the stop shape on purpose: the shape says "stop"
 * and the word says whether it has finished; the two never sit in one group. A
 * stopped run never shows the running ring (live QA 2026-11-10, C-1).
 *
 * Inks, for the surface's CSS: running wears the accent (and turns slowly), blocked
 * the warning ink, failed the danger ink; the rest are muted.
 */
export const SUBAGENT_STATUS_GLYPHS: Record<SubagentRunStatus, Glyph> = {
  queued: CircleIcon,
  running: CircleDotDashed,
  cancelling: StopCircle,
  completed: Check,
  blocked: CircleHelp,
  incomplete: StopCircle,
  failed: X,
  interrupted: PauseCircle,
  unknown: MoreHorizontal,
};

/** The word each status glyph stands for, spoken ahead of the name and shown as text. */
export const SUBAGENT_STATUS_WORDS: Record<SubagentRunStatus, string> = {
  queued: 'Waiting to start',
  running: 'Running',
  cancelling: 'Stopping',
  completed: 'Completed',
  blocked: 'Needs an answer',
  incomplete: 'Stopped',
  failed: 'Failed',
  interrupted: 'Interrupted',
  unknown: 'Status unknown',
};
