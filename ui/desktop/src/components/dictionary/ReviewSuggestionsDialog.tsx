import { useId, useMemo, useState } from 'react';
import type { DictionaryEntryView } from '../../api';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { Note } from '../ui/note';
import { DICTIONARY_LABELS, dictionaryMessage } from './copy';
import { groupOf, lineTwo } from './rows';
import { acceptTerms, deleteTerms, dismissTerms, keepTerms, type Outcome } from './useDictionary';

/**
 * The ids a choice acts on, split by what each row is: a suggestion is
 * accepted or dismissed (`/dictionary/suggestions/*`); an agent's row not yet
 * kept is kept (`/dictionary/entries/keep`) or removed, which writes the
 * person's tombstone (`/dictionary/entries/delete`).
 */
export function splitReview(rows: readonly DictionaryEntryView[]): {
  suggestions: string[];
  agentRows: string[];
} {
  const suggestions: string[] = [];
  const agentRows: string[] = [];
  for (const row of rows) {
    if (groupOf(row) === 'suggested') suggestions.push(row.id);
    else if (groupOf(row) === 'agent') agentRows.push(row.id);
  }
  return { suggestions, agentRows };
}

/**
 * **Review all…** ("Dialogs" in `docs/dictation/dictionary.md`): a checklist of
 * the suggestions and the agent's rows not yet kept, with **Select all** above
 * it and **Accept selected** and **Dismiss selected**. No row is ticked at
 * open, so neither button does anything until the person chooses rows: a
 * dismissed suggestion is never suggested again, and a removed agent row writes
 * the person's tombstone, so one press must never act on rows nobody chose. An
 * accept that would pass the active cap fails as a whole (the daemon writes
 * nothing), and its sentence shows here.
 */
export default function ReviewSuggestionsDialog({
  rows,
  onClose,
}: {
  rows: readonly DictionaryEntryView[];
  onClose: () => void;
}) {
  const listId = useId();
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chosen = useMemo(() => rows.filter((row) => selected.has(row.id)), [rows, selected]);
  const allChosen = chosen.length === rows.length && rows.length > 0;

  const run = async (accept: boolean) => {
    const { suggestions, agentRows } = splitReview(chosen);
    setBusy(true);
    setError(null);
    try {
      const outcomes: Outcome<unknown>[] = [];
      if (suggestions.length > 0) {
        outcomes.push(await (accept ? acceptTerms(suggestions) : dismissTerms(suggestions)));
      }
      if (agentRows.length > 0) {
        outcomes.push(await (accept ? keepTerms(agentRows) : deleteTerms(agentRows)));
      }
      const failed = outcomes.find((outcome) => !outcome.ok);
      if (failed && !failed.ok) setError(failed.error.message);
      else onClose();
    } finally {
      setBusy(false);
    }
  };

  const toggle = (id: string, on: boolean) =>
    setSelected((current) => {
      const next = new Set(current);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  return (
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      size="md"
      purpose={busy ? 'required' : 'form'}
      title={DICTIONARY_LABELS.review.title}
      subtitle={dictionaryMessage('MSG-REVIEW-HELP')}
      anchor="top"
      scrollBody
      bodyClassName="flex flex-col gap-2 py-4"
      testId="dictionary-review-dialog"
      footer={
        <>
          <Button
            type="button"
            variant="secondary"
            onClick={() => void run(false)}
            disabled={busy || chosen.length === 0}
            data-testid="dictionary-review-dismiss"
          >
            {DICTIONARY_LABELS.review.dismissSelected}
          </Button>
          <Button
            type="button"
            onClick={() => void run(true)}
            disabled={busy || chosen.length === 0}
            aria-busy={busy || undefined}
            data-testid="dictionary-review-accept"
          >
            {DICTIONARY_LABELS.review.acceptSelected}
          </Button>
        </>
      }
    >
      <label className="flex items-center gap-2 px-3 text-label text-text-default">
        <Checkbox
          checked={allChosen}
          indeterminate={!allChosen && chosen.length > 0}
          disabled={busy}
          aria-controls={listId}
          onChange={(event) =>
            setSelected(event.target.checked ? new Set(rows.map((row) => row.id)) : new Set())
          }
          data-testid="dictionary-review-all"
        />
        {DICTIONARY_LABELS.review.selectAll}
      </label>
      <ul id={listId} className="biorouter-list-shell br-rows-flat">
        {rows.map((row) => (
          <li key={row.id} className="biorouter-list-row px-3 py-2.5">
            <label className="flex min-w-0 items-center gap-3">
              <Checkbox
                checked={selected.has(row.id)}
                disabled={busy}
                onChange={(event) => toggle(row.id, event.target.checked)}
                data-testid="dictionary-review-row"
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-label text-text-default">{row.written}</span>
                <span className="block truncate text-supporting text-text-muted">
                  {lineTwo(row)}
                </span>
              </span>
            </label>
          </li>
        ))}
      </ul>
      {error ? (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      ) : null}
    </ModalShell>
  );
}
