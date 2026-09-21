import { useId, useState } from 'react';

const PREVIEW_LINES = 6;
const PREVIEW_CHARACTERS = 600;

export function ToolContentPreview({
  text,
  children,
}: {
  text: string;
  children: (visibleText: string, truncated: boolean) => React.ReactNode;
}) {
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  const preview = text
    .split('\n', PREVIEW_LINES)
    .join('\n')
    .slice(0, PREVIEW_CHARACTERS)
    .replace(/[\uD800-\uDBFF]$/, '');
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
