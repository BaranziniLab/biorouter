import { useMemo, useState } from 'react';
import type { ChangeKind, HistoryEntry } from '../../../api/types.gen';
import { AlertCircle, History } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import { Sheet, SheetContent, SheetHeader, SheetTitle } from '../../ui/sheet';
import { useKnowledge } from '../KnowledgeContext';
import { useHistory } from '../hooks/useHistory';
import { ChangeKindChip } from './ChangeKindChip';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { toastError } from '../../../toasts';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPreview: (sha: string) => void;
  onRestored: () => void;
}

const ALL_KINDS: ChangeKind[] = ['ingest', 'link', 'flag', 'query', 'lint', 'restore', 'manual'];

function relativeTime(iso: string): string {
  const t = new Date(iso).getTime();
  const diff = Date.now() - t;
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(iso).toLocaleDateString();
}

/**
 * What changed in this knowledge base, and when (ui-spec §4.10).
 *
 * Four corrections over the drawer this replaces:
 *
 * - **Width is `--knowledge-rail-detail`**, so the drawer and the workspace's
 *   detail rail are the same object at the same width instead of two arbitrary
 *   numbers (420 here, 340 there).
 * - **`SheetTitle` keeps `text-subheading`.** The `text-label` override made
 *   this the app's second overlay title size, on a screen that can open both.
 * - **Kind filters are real toggle buttons**, through `Badge asChild` so the
 *   chip IS the button and takes the global focus fill. They used to be ~20px
 *   lowercase words with no background at all in the unselected state, so the
 *   filter row read as a run of prose.
 * - **`tint-interactive` comes off the entry rows.** The targets are the two
 *   buttons inside them; a row that lights up under the pointer and does nothing
 *   when clicked is a promise the surface does not keep. The row keeps
 *   `.biorouter-list-row` for its hairline.
 */
export function ChangeLogDrawer({ open, onOpenChange, onPreview, onRestored }: Props) {
  const { primaryKbId, triggerGraphRefresh } = useKnowledge();
  // Read on every open, not once at mount: the drawer stays mounted, hidden,
  // for as long as the Knowledge view is, and a digest lands while it is shut.
  const { history, loading, error, restore } = useHistory(primaryKbId, open);
  const [activeKinds, setActiveKinds] = useState<Set<ChangeKind>>(new Set(ALL_KINDS));
  const [restoring, setRestoring] = useState<string | null>(null);
  const [entryToRestore, setEntryToRestore] = useState<HistoryEntry | null>(null);

  const filtered = useMemo(
    () => history.filter((h) => activeKinds.has(h.kind)),
    [history, activeKinds]
  );

  function toggleKind(k: ChangeKind) {
    setActiveKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  async function confirmRestore() {
    const entry = entryToRestore;
    if (!entry || restoring) return;
    setRestoring(entry.commit_sha);
    try {
      await restore(entry.commit_sha);
      triggerGraphRefresh();
      onRestored();
    } catch (err) {
      toastError({
        title: 'Restore failed',
        msg: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setRestoring(null);
      setEntryToRestore(null);
    }
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        // This drawer's header is a bare title on an `h-row` bar — there is no
        // explanatory line on screen to link, so it takes Radix's own explicit
        // opt-out. Without it Radix leaves `aria-describedby` pointing at an id
        // nothing claims and warns on every open. See `ui/sheet.tsx` for why the
        // choice cannot be defaulted in the primitive.
        aria-describedby={undefined}
        className="flex w-knowledge-rail-detail flex-col gap-0 p-0 sm:max-w-knowledge-rail-detail"
      >
        <SheetHeader className="h-row flex-none flex-row items-center justify-between border-b border-border-subtle px-4 py-0">
          <SheetTitle>Change log</SheetTitle>
        </SheetHeader>

        <div className="flex flex-none flex-wrap gap-2 border-b border-border-subtle px-4 py-2">
          {ALL_KINDS.map((k) => {
            const on = activeKinds.has(k);
            return (
              <Badge
                key={k}
                variant="chip"
                asChild
                uppercase
                className={
                  on
                    ? 'tint-selected tint-interactive text-text-default'
                    : 'tint-interactive text-text-muted'
                }
              >
                <button type="button" aria-pressed={on} onClick={() => toggleKind(k)}>
                  {k}
                </button>
              </Badge>
            );
          })}
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {loading && (
            <EmptyState
              compact
              icon={History}
              title="Loading history"
              description="Reading this knowledge base's commits."
            />
          )}
          {error && (
            <EmptyState
              compact
              icon={AlertCircle}
              title="Could not load the history"
              description={error}
            />
          )}
          {!loading && !error && filtered.length === 0 && (
            <EmptyState
              compact
              icon={History}
              title="No changes yet"
              description="Digesting a source records a commit here."
            />
          )}
          {!loading &&
            !error &&
            filtered.map((entry) => (
              <div key={entry.commit_sha} className="biorouter-list-row flex flex-col px-4 py-3">
                <div className="mb-1 flex items-center gap-2">
                  <ChangeKindChip kind={entry.kind} />
                  <span className="text-supporting font-mono text-text-muted">
                    {entry.commit_sha.slice(0, 7)}
                  </span>
                  <span className="ml-auto text-supporting text-text-muted">
                    {relativeTime(entry.timestamp)}
                  </span>
                </div>
                <div className="mb-2 text-body text-text-default">{entry.summary}</div>
                <div className="flex items-center gap-2">
                  {/* `sm` (28px), not `xs`. The compact tier's contract is
                      glyph-only — "a control carrying a label never uses it". */}
                  <Button variant="ghost" size="sm" onClick={() => onPreview(entry.commit_sha)}>
                    Preview
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setEntryToRestore(entry)}
                    disabled={restoring !== null}
                  >
                    {restoring === entry.commit_sha ? 'Restoring…' : 'Restore'}
                  </Button>
                </div>
              </div>
            ))}
        </div>
      </SheetContent>
      <ConfirmationModal
        isOpen={entryToRestore !== null}
        title="Restore knowledge base?"
        message={`Restore to ${entryToRestore?.commit_sha.slice(0, 7) ?? ''}? A new revert commit will be created.`}
        confirmLabel="Restore"
        cancelLabel="Cancel"
        isSubmitting={restoring !== null}
        onConfirm={() => void confirmRestore()}
        onCancel={() => setEntryToRestore(null)}
      />
    </Sheet>
  );
}
