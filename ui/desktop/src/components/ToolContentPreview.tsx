import { useId, useState } from 'react';

const PREVIEW_LINES = 6;
const PREVIEW_CHARACTERS = 600;

/** The first `PREVIEW_LINES` lines, capped, without splitting a surrogate pair. */
function headPreview(text: string): string {
  return text
    .split('\n', PREVIEW_LINES)
    .join('\n')
    .slice(0, PREVIEW_CHARACTERS)
    .replace(/[\uD800-\uDBFF]$/, '');
}

/**
 * The LAST `PREVIEW_LINES` lines, capped the same way.
 *
 * Trimmed from the front, so the cut is at the start and the newest line is
 * always whole — the opposite end from `headPreview`, for the same reason it
 * exists. The low surrogate of a pair split at the front is dropped, the mirror
 * of the high-surrogate trim above.
 */
function tailPreview(text: string): string {
  const lines = text.split('\n');
  const last = lines.slice(Math.max(0, lines.length - PREVIEW_LINES)).join('\n');
  return last.length <= PREVIEW_CHARACTERS
    ? last
    : last.slice(last.length - PREVIEW_CHARACTERS).replace(/^[\uDC00-\uDFFF]/, '');
}

export function ToolContentPreview({
  text,
  tail = false,
  children,
}: {
  text: string;
  /**
   * Preview the END of the text rather than its beginning.
   *
   * For arguments and results the first lines are the interesting ones. For
   * LOGS they are the least interesting: a tool that has been running for a
   * minute has its newest output at the bottom, and a head preview shows the
   * same six lines from the first second, forever, while the work scrolls past
   * unseen.
   */
  tail?: boolean;
  children: (visibleText: string, truncated: boolean) => React.ReactNode;
}) {
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  const preview = tail ? tailPreview(text) : headPreview(text);
  const long = preview.length < text.length;
  return (
    <div className="min-w-0 max-w-full">
      <div id={id}>{children(expanded || !long ? text : preview, long && !expanded)}</div>
      {long && (
        <button
          type="button"
          className="br-tool-more mt-1 text-xs text-text-muted hover:text-text-default focus-visible:text-text-default"
          aria-expanded={expanded}
          aria-controls={id}
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </div>
  );
}
