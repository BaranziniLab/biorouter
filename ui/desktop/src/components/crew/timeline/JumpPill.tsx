import { Button } from '../../ui/button';
import { ArrowDown } from '../../icons/app-icons';
import { timelineCopy } from './copy';

/**
 * The one pill 12px above the composer.
 *
 * - `history`: an older page is on screen. "Viewing earlier messages" (pinned)
 *   and "Jump to latest", which leaves the page and reloads the live tail.
 *   Live message frames are ignored while a page is shown, so this pill is the
 *   reminder that the channel may have moved on.
 * - `live`: the reader is scrolled up and something arrived below. "↓ Jump to
 *   latest" scrolls down; it is the only scroll this timeline animates.
 */
export function JumpPill({
  mode,
  disabled,
  onJump,
}: {
  mode: 'history' | 'live';
  disabled?: boolean;
  onJump(): void;
}) {
  if (mode === 'history') {
    return (
      <div className="crew-jump-pill" data-mode="history">
        <span className="crew-jump-pill-text text-secondary text-text-muted">
          {timelineCopy.viewingEarlier}
        </span>
        <Button type="button" variant="ghost" size="sm" disabled={disabled} onClick={onJump}>
          {timelineCopy.jumpToLatest}
        </Button>
      </div>
    );
  }
  return (
    <div className="crew-jump-pill" data-mode="live">
      <Button type="button" variant="ghost" size="sm" disabled={disabled} onClick={onJump}>
        <ArrowDown aria-hidden />
        {timelineCopy.jumpToLatest}
      </Button>
    </div>
  );
}
