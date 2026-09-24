import { timelineCopy } from './copy';

/**
 * The unread rule: a 1px `--accent-bar` line with "New" at its right, in accent
 * ink. Never danger: unread is live state, not a failure. Where it goes is
 * decided once, when the channel opens (`newLineBeforeId`).
 */
export function NewDivider() {
  return (
    <div className="crew-new-divider" role="separator" aria-label={timelineCopy.newLineLabel}>
      <span aria-hidden="true" className="crew-new-label text-chip text-text-accent">
        {timelineCopy.newLine}
      </span>
    </div>
  );
}
