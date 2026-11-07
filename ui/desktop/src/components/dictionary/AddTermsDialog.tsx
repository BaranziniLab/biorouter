import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { DictionaryAuthority, DictionaryImportAnswer } from '../../api';
import { ModalShell } from '../ModalShell';
import { focusFieldOnOpen } from '../baam/focusFieldOnOpen';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { Field } from '../ui/field';
import { Note } from '../ui/note';
import { Textarea } from '../ui/textarea';
import { DICTIONARY_LABELS, dictionaryMessage, validationSentence } from './copy';
import {
  MAX_IMPORT_ROWS,
  fileOverImportBytes,
  importFormatFor,
  overImportBytes,
  parseTermList,
} from './parseTermList';
import { applyImport, planImport } from './useDictionary';

/** How long typing settles before the daemon is asked for a dry run. */
export const PLAN_DEBOUNCE_MS = 300;

/** The text a file holds, and its name, which names its format. */
export interface AddTermsSource {
  content: string;
  fileName: string | null;
}

type Plan =
  | { state: 'idle' }
  | { state: 'checking' }
  | { state: 'ready'; answer: DictionaryImportAnswer }
  | { state: 'refused'; message: string };

/** "12 new · 3 already in your dictionary · 1 skipped" (MSG-IMPORT-PLAN). */
export function planLine(answer: DictionaryImportAnswer): string {
  return dictionaryMessage('MSG-IMPORT-PLAN', {
    new: String(answer.new),
    existing: String(answer.merged + answer.duplicate),
    skipped: String(answer.invalid.length),
  });
}

/** One line per skipped row, with its reason (MSG-IMPORT-SKIPPED). */
export function skippedLines(answer: DictionaryImportAnswer): string[] {
  return answer.invalid.map((row) =>
    dictionaryMessage('MSG-IMPORT-SKIPPED', {
      line: String(row.line),
      sentence: validationSentence(row.code) ?? row.code,
    })
  );
}

/** Whether applying the plan would change anything. */
export function planChanges(answer: DictionaryImportAnswer): boolean {
  return answer.new + answer.merged > 0;
}

/**
 * **Paste a list…** and **Import a file…** (`AddTermsDialog` in "Dialogs",
 * `docs/dictation/dictionary.md`): paste or drop a `.txt`, `.csv`, `.tsv` or
 * `.json` file, read the daemon's dry run ("12 new · 3 already in your
 * dictionary · 1 skipped", each skipped line with its reason), choose **Only in
 * private chats** (disabled before the click with `authority.add_private.reason`
 * when the caller has no private standing), and add. It merges and never
 * deletes. A file over the daemon's 1 MiB is refused by its size and never
 * read, and text over it shows MSG-DICTIONARY-TOO-LARGE before it is parsed.
 */
export default function AddTermsDialog({
  initial,
  authority,
  onClose,
}: {
  initial?: AddTermsSource;
  authority: DictionaryAuthority | null;
  onClose: () => void;
}) {
  const fieldId = useId();
  const privateId = useId();
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const [content, setContent] = useState(initial?.content ?? '');
  const [fileName, setFileName] = useState<string | null>(initial?.fileName ?? null);
  const [onlyPrivate, setOnlyPrivate] = useState(false);
  const [plan, setPlan] = useState<Plan>({ state: 'idle' });
  const [busy, setBusy] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  // A file that was not taken (too large, or unreadable); the text in the field stays as it was.
  const [fileRefusal, setFileRefusal] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const generation = useRef(0);

  const canAddPrivate = authority?.add_private.allowed ?? true;
  const privateReason = canAddPrivate ? null : (authority?.add_private.reason ?? null);
  // Text over the daemon's 1 MiB is refused before anything walks it: no format sniffing, no
  // parse and no request, so a paste far over the limit costs one length check per change.
  const tooLarge = useMemo(() => overImportBytes(content), [content]);
  const format = useMemo(
    () => importFormatFor(fileName, tooLarge ? '' : content),
    [fileName, content, tooLarge]
  );

  useEffect(() => {
    const issued = ++generation.current;
    if (tooLarge) {
      setPlan({ state: 'refused', message: dictionaryMessage('MSG-DICTIONARY-TOO-LARGE') });
      return;
    }
    if (!content.trim()) {
      setPlan({ state: 'idle' });
      return;
    }
    const parsed = parseTermList(content, format);
    if (parsed.ok && parsed.terms.length > MAX_IMPORT_ROWS) {
      setPlan({ state: 'refused', message: dictionaryMessage('MSG-DICTIONARY-TOO-LARGE') });
      return;
    }
    setPlan({ state: 'checking' });
    const timer = setTimeout(() => {
      void planImport({ format, content, private: onlyPrivate }).then((outcome) => {
        // A plan for text the person has since changed is dropped.
        if (issued !== generation.current) return;
        setPlan(
          outcome.ok
            ? { state: 'ready', answer: outcome.data }
            : { state: 'refused', message: outcome.error.message }
        );
      });
    }, PLAN_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [content, format, onlyPrivate, tooLarge]);

  const takeFile = async (file: File | undefined | null) => {
    if (!file) return;
    // Its size is in bytes already, so a file over the limit is never read.
    if (fileOverImportBytes(file)) {
      setFileRefusal(dictionaryMessage('MSG-DICTIONARY-TOO-LARGE'));
      return;
    }
    try {
      const text = await file.text();
      setContent(text);
      setFileName(file.name);
      setApplyError(null);
      setFileRefusal(null);
    } catch {
      setFileRefusal(dictionaryMessage('MSG-IMPORT-ROW-UNREADABLE'));
    }
  };

  const apply = async () => {
    if (plan.state !== 'ready' || !planChanges(plan.answer)) return;
    setBusy(true);
    setApplyError(null);
    try {
      const outcome = await applyImport({ format, content, private: onlyPrivate });
      if (outcome.ok) onClose();
      else setApplyError(outcome.error.message);
    } finally {
      setBusy(false);
    }
  };

  const ready = plan.state === 'ready' && planChanges(plan.answer);

  return (
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      size="lg"
      purpose={busy ? 'required' : 'form'}
      title={DICTIONARY_LABELS.addTerms.title}
      anchor="top"
      scrollBody
      onOpenAutoFocus={focusFieldOnOpen(fieldRef)}
      bodyClassName="flex flex-col gap-3 py-4"
      testId="dictionary-add-terms-dialog"
      footer={
        <>
          <Button
            type="button"
            variant="secondary"
            className="mr-auto"
            onClick={() => fileRef.current?.click()}
            disabled={busy}
          >
            {DICTIONARY_LABELS.addTerms.chooseFile}
          </Button>
          <Button type="button" variant="secondary" onClick={onClose} disabled={busy}>
            {DICTIONARY_LABELS.cancel}
          </Button>
          <Button
            type="button"
            onClick={() => void apply()}
            disabled={busy || !ready}
            aria-busy={busy || undefined}
            data-testid="dictionary-add-terms-apply"
          >
            {busy ? DICTIONARY_LABELS.addTerms.applying : DICTIONARY_LABELS.addTerms.apply}
          </Button>
        </>
      }
    >
      <input
        ref={fileRef}
        type="file"
        accept=".txt,.csv,.tsv,.json,text/plain,text/csv,text/tab-separated-values,application/json"
        hidden
        onChange={(event) => {
          void takeFile(event.target.files?.[0]);
          event.target.value = '';
        }}
        data-testid="dictionary-add-terms-file"
      />
      <Field
        id={fieldId}
        label={DICTIONARY_LABELS.addTerms.field}
        info={dictionaryMessage('MSG-ADD-TERMS-HELP')}
      >
        <Textarea
          ref={fieldRef}
          rows={8}
          value={content}
          spellCheck={false}
          className="br-dictionary-drop font-mono text-code"
          data-dragging={dragging ? 'true' : undefined}
          onChange={(event) => {
            setContent(event.target.value);
            setFileRefusal(null);
          }}
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes('Files')) return;
            event.preventDefault();
            setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={(event) => {
            setDragging(false);
            const file = event.dataTransfer.files?.[0];
            if (!file) return;
            event.preventDefault();
            void takeFile(file);
          }}
          data-testid="dictionary-add-terms-content"
        />
      </Field>
      <div className="flex flex-col gap-1">
        <label htmlFor={privateId} className="flex items-center gap-2 text-body text-text-default">
          <Checkbox
            id={privateId}
            checked={onlyPrivate}
            disabled={busy || !canAddPrivate}
            onChange={(event) => setOnlyPrivate(event.target.checked)}
            aria-describedby={privateReason ? `${privateId}-help` : undefined}
            data-testid="dictionary-add-terms-private"
          />
          {DICTIONARY_LABELS.term.onlyPrivate}
        </label>
        {privateReason ? (
          <p id={`${privateId}-help`} className="pl-8 text-supporting text-text-muted">
            {privateReason}
          </p>
        ) : null}
      </div>
      <div
        aria-live="polite"
        className="flex flex-col gap-1"
        data-testid="dictionary-add-terms-plan"
      >
        {fileRefusal ? (
          <Note tone="danger" role="alert" testId="dictionary-add-terms-file-refused">
            {fileRefusal}
          </Note>
        ) : null}
        {plan.state === 'checking' ? (
          <p className="text-supporting text-text-muted">{DICTIONARY_LABELS.addTerms.checking}</p>
        ) : null}
        {plan.state === 'ready' ? (
          <>
            <p className="text-body text-text-default">{planLine(plan.answer)}</p>
            {skippedLines(plan.answer).map((line) => (
              <p key={line} className="text-supporting text-text-muted">
                {line}
              </p>
            ))}
          </>
        ) : null}
        {plan.state === 'refused' ? (
          <Note tone="danger" role="alert">
            {plan.message}
          </Note>
        ) : null}
      </div>
      {applyError ? (
        <Note tone="danger" role="alert">
          {applyError}
        </Note>
      ) : null}
    </ModalShell>
  );
}
