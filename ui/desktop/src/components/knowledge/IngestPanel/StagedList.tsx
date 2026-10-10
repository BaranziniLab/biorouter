import { ClipboardList, FileText, Globe, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { knowledgeCopy } from '../copy';
import type { StagedSource } from '../hooks/useStagedSources';

interface Props {
  items: StagedSource[];
  onRemove: (id: string) => void;
  onClear: () => void;
}

/**
 * The staging queue (ui-spec §4.4, states 1 and 2).
 *
 * Rows sit FLUSH in a `.biorouter-list-shell` — the `gap-1.5` between them is
 * gone, so each row's bottom hairline DIVIDES rather than floats. Icons are
 * `--icon-row` (16px) throughout, replacing a 14px/12px mix, and the remove
 * control is a real `--control-compact` ghost button rather than a 12×12
 * pointer target.
 *
 * `Clear`, capital C. The section said `clear all` here and `Clear` in
 * `IngestWarnings`, six components apart in the same rail.
 */
export function StagedList({ items, onRemove, onClear }: Props) {
  if (items.length === 0) {
    // ⚠ **NOTHING, and that is the fix** (R-06). This was an `EmptyState`
    // reading "Nothing staged / Drop files above, paste text, or choose a
    // folder" — measured at **236px** in the 300px rail, which is LARGER than a
    // five-item staged list at 232px. The rail was at its tallest when it had
    // the least to say, and the text pointed at the dropzone sitting directly
    // above it. An empty list is nothing; its neighbour already says what to do.
    return null;
  }

  return (
    <div className="flex flex-col">
      {/* The count joins the caps run, "STAGED 3" (Crew's "6 MEMBERS"), rather
          than sitting in a badge of its own. */}
      <div className="mb-1 flex items-center justify-between gap-2">
        <span className="text-caps tabular-nums text-text-muted">
          {knowledgeCopy.ingest.stagedLabel} {items.length}
        </span>
        <Button type="button" variant="ghost" size="sm" onClick={onClear}>
          Clear
        </Button>
      </div>
      <div className="biorouter-list-shell">
        {items.map((s) => {
          const Icon =
            s.kind === 'file' || s.kind === 'path'
              ? FileText
              : s.kind === 'url'
                ? Globe
                : ClipboardList;
          const label =
            s.kind === 'file'
              ? s.label || s.file.name
              : s.kind === 'path'
                ? s.label
                : s.kind === 'url'
                  ? s.url
                  : s.title || s.text.substring(0, 60);

          return (
            <div
              key={s.id}
              data-testid="knowledge-staged-item"
              data-label={label}
              className="biorouter-list-row flex items-center gap-2"
            >
              <Icon
                className="h-icon-row w-icon-row flex-shrink-0 text-text-muted"
                aria-hidden="true"
              />
              <div className="min-w-0 flex-1">
                <div className="truncate text-label">{label}</div>
                {s.status !== 'pending' && (
                  <div
                    className={`text-supporting ${s.status === 'error' ? 'text-text-danger' : 'text-text-muted'}`}
                  >
                    {s.status}
                    {s.error ? `: ${s.error}` : ''}
                  </div>
                )}
              </div>
              <Button
                type="button"
                variant="ghost"
                shape="round"
                size="sm"
                onClick={() => onRemove(s.id)}
                aria-label={`Remove ${label} from the staging queue`}
              >
                <X aria-hidden="true" />
              </Button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
