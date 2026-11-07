import { useId, useRef, useState } from 'react';
import type {
  DictionaryAddRequest,
  DictionaryAuthority,
  DictionaryEditRequest,
  DictionaryEntryView,
  DictionaryMatch,
} from '../../api';
import { ModalShell } from '../ModalShell';
import { focusFieldOnOpen } from '../baam/focusFieldOnOpen';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import CustomRadio from '../ui/CustomRadio';
import { Disclosure } from '../ui/disclosure';
import { Field } from '../ui/field';
import { InfoTip } from '../ui/info-tip';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { DICTIONARY_LABELS, dictionaryMessage } from './copy';
import { publicizeWarning } from './rows';
import type { TermOrigin } from './termOrigin';
import { addTerm, editTerm, setTermTier, type Outcome } from './useDictionary';

/** A term takes at most eight spoken forms (MSG-TERM-FORMS). */
export const MAX_SPOKEN_FORMS = 8;

export type TermDialogMode =
  | {
      kind: 'add';
      /** The word or selection the context menu passed. */
      text?: string;
      /** Where the add started; only the context menu door resolves one (O-36). */
      origin?: TermOrigin;
      /** **Only in private chats** at open: ticked for a private chat or an unresolved transcript. */
      startPrivate?: boolean;
    }
  | { kind: 'edit'; entry: DictionaryEntryView };

export interface TermDialogProps {
  mode: TermDialogMode;
  /** `GET /dictionary/settings`'s `authority`; `null` before it has loaded. */
  authority: DictionaryAuthority | null;
  onClose: () => void;
}

/** One form per line, trimmed, blanks dropped. */
export function spokenFormsOf(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/**
 * The body of `POST /dictionary/entries` for an add. The origin fields name the
 * chat the add started in, which the daemon reads only to stamp a private row's
 * institutions (O-36): a resolved chat is `origin_session_id`, a transcript whose
 * chat could not be resolved is `origin_unresolved: true`, and an add outside any
 * transcript sends neither.
 */
export function addRequestOf(fields: {
  written: string;
  spoken: string;
  note: string;
  private: boolean;
  match: DictionaryMatch | null;
  always: boolean;
  origin?: TermOrigin;
}): DictionaryAddRequest {
  const note = fields.note.trim();
  return {
    written: fields.written.trim(),
    spoken_forms: spokenFormsOf(fields.spoken),
    ...(note ? { note } : {}),
    private: fields.private,
    ...(fields.match ? { match: fields.match } : {}),
    ...(fields.always ? { bias: 'always' as const } : {}),
    ...(fields.origin?.kind === 'chat' ? { origin_session_id: fields.origin.sessionId } : {}),
    ...(fields.origin?.kind === 'unresolved' ? { origin_unresolved: true } : {}),
  };
}

/** The `PATCH` for an edit: only the fields that changed; an emptied note is `null`. */
export function editPatchOf(
  entry: DictionaryEntryView,
  fields: { written: string; spoken: string; note: string; match: DictionaryMatch; always: boolean }
): DictionaryEditRequest {
  const patch: DictionaryEditRequest = {};
  const written = fields.written.trim();
  if (written !== entry.written) patch.written = written;
  const forms = spokenFormsOf(fields.spoken);
  if (forms.join('\n') !== entry.spoken_forms.join('\n')) patch.spoken_forms = forms;
  const note = fields.note.trim();
  if (note !== (entry.note ?? '')) patch.note = note ? note : null;
  if (fields.match !== entry.match) patch.match = fields.match;
  const bias = fields.always ? 'always' : entry.bias === 'always' ? 'auto' : entry.bias;
  if (bias !== entry.bias) patch.bias = bias;
  return patch;
}

const MATCHES: DictionaryMatch[] = ['exact', 'fuzzy', 'prompt_only'];

/**
 * Add or edit one term (`TermDialog` in "Dialogs", `docs/dictation/dictionary.md`).
 *
 * **Term**; **Spoken as**, one per line, at most eight; **Note**; **Only in
 * private chats**, ticked at open when the add starts from a private chat or a
 * transcript whose chat cannot be resolved (O-6) and disabled before the click
 * with `authority.add_private.reason` when the caller has no private standing
 * (a box ticked at open can still be unticked, which adds the word publicly
 * after MSG-PUBLICIZE-WARN, and while it stays ticked **Add term** is disabled
 * with that reason); and a **More options** disclosure with **Match** and **Always use in
 * dictation**. On an edit, unticking a private term is a publicize: it asks
 * MSG-PUBLICIZE-WARN (or MSG-PUBLICIZE-WARN-NO-CHAT) before it saves. The
 * daemon's refusal sentence shows under **Term**, and MSG-SAME-LETTERS, which an
 * add answers beside a new row, shows there as a note before the dialog closes.
 */
export default function TermDialog({ mode, authority, onClose }: TermDialogProps) {
  const termId = useId();
  const spokenId = useId();
  const noteId = useId();
  const privateId = useId();
  const termRef = useRef<HTMLInputElement>(null);
  const editing = mode.kind === 'edit' ? mode.entry : null;

  const [written, setWritten] = useState(
    editing?.written ?? (mode.kind === 'add' ? (mode.text ?? '') : '')
  );
  const [spoken, setSpoken] = useState(editing ? editing.spoken_forms.join('\n') : '');
  const [note, setNote] = useState(editing?.note ?? '');
  const initialPrivate = editing
    ? editing.tier === 'private'
    : mode.kind === 'add' && mode.startPrivate === true;
  const [onlyPrivate, setOnlyPrivate] = useState(initialPrivate);
  const [match, setMatch] = useState<DictionaryMatch | null>(editing?.match ?? null);
  const [always, setAlways] = useState(editing?.bias === 'always');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sameLetters, setSameLetters] = useState<string | null>(null);
  const [confirmPublicize, setConfirmPublicize] = useState(false);

  const formsCount = spokenFormsOf(spoken).length;
  const tooManyForms = formsCount > MAX_SPOKEN_FORMS;
  const canAddPrivate = authority?.add_private.allowed ?? true;
  const privateReason = canAddPrivate ? null : (authority?.add_private.reason ?? null);
  const canChangeTier = authority?.tier_change.allowed ?? true;
  // On an edit the box is a tier change; it can be used only where tier changes are admitted.
  // On an add without private standing it cannot be ticked, but one ticked at open (a word from
  // a private chat or an unresolved transcript) can be unticked: the person may add it as a
  // public term after the warning below, and a box locked ticked would leave no add at all.
  const privateDisabled =
    busy || sameLetters !== null || (editing ? !canChangeTier : !canAddPrivate && !initialPrivate);
  // A private add this caller may not make is refused before the click, with the reason.
  const privateAddRefused = !editing && onlyPrivate && !canAddPrivate;
  const publicizing = editing?.tier === 'private' && !onlyPrivate;
  // A new term from a private chat, unticked by the person: say what that does, as publicize would.
  const addFromPrivateUnticked = !editing && initialPrivate && !onlyPrivate;

  const title = editing ? DICTIONARY_LABELS.term.editTitle : DICTIONARY_LABELS.term.addTitle;
  const done = sameLetters !== null;

  const finish = <T,>(outcome: Outcome<T>): boolean => {
    if (outcome.ok) return true;
    setError(outcome.error.message);
    return false;
  };

  const save = async (confirmed = false) => {
    if (done) {
      onClose();
      return;
    }
    if (!written.trim() || tooManyForms || privateAddRefused) return;
    if (publicizing && !confirmed) {
      setConfirmPublicize(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (!editing) {
        const origin = mode.kind === 'add' ? mode.origin : undefined;
        const outcome = await addTerm(
          addRequestOf({ written, spoken, note, private: onlyPrivate, match, always, origin })
        );
        if (!finish(outcome) || !outcome.ok) return;
        if (outcome.data.warning) {
          setSameLetters(outcome.data.warning);
          return;
        }
        onClose();
        return;
      }
      const patch = editPatchOf(editing, {
        written,
        spoken,
        note,
        match: match ?? editing.match,
        always,
      });
      if (Object.keys(patch).length > 0) {
        if (!finish(await editTerm(editing.id, editing.rev, patch))) return;
      }
      const wantTier = onlyPrivate ? 'private' : 'public';
      if (wantTier !== editing.tier) {
        if (!finish(await setTermTier(editing.id, wantTier))) return;
      }
      onClose();
    } finally {
      setBusy(false);
    }
  };

  const privateHelp = addFromPrivateUnticked
    ? dictionaryMessage('MSG-PUBLICIZE-WARN')
    : privateReason && !editing
      ? privateReason
      : null;
  const privateHelpId = `${privateId}-help`;

  return (
    <>
      <ModalShell
        open
        onOpenChange={(open) => {
          if (!open && !busy) onClose();
        }}
        size="md"
        purpose={busy ? 'required' : 'form'}
        title={title}
        anchor="top"
        scrollBody
        onOpenAutoFocus={focusFieldOnOpen(termRef)}
        bodyClassName="flex flex-col gap-3 py-4"
        testId="dictionary-term-dialog"
        footer={
          <>
            <Button type="button" variant="secondary" onClick={onClose} disabled={busy}>
              {DICTIONARY_LABELS.cancel}
            </Button>
            <Button
              type="button"
              onClick={() => void save()}
              disabled={busy || (!done && (!written.trim() || tooManyForms || privateAddRefused))}
              aria-busy={busy || undefined}
              aria-describedby={privateAddRefused && privateHelp ? privateHelpId : undefined}
              data-testid="dictionary-term-save"
            >
              {done
                ? DICTIONARY_LABELS.done
                : busy
                  ? editing
                    ? DICTIONARY_LABELS.term.saving
                    : DICTIONARY_LABELS.term.adding
                  : editing
                    ? DICTIONARY_LABELS.save
                    : DICTIONARY_LABELS.term.add}
            </Button>
          </>
        }
      >
        <form
          className="contents"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <Field
            id={termId}
            label={DICTIONARY_LABELS.term.term}
            required
            error={error ?? undefined}
            helper={
              sameLetters ? (
                <span data-testid="dictionary-term-same-letters">{sameLetters}</span>
              ) : undefined
            }
          >
            <Input
              ref={termRef}
              value={written}
              maxLength={256}
              readOnly={done}
              onChange={(event) => {
                setWritten(event.target.value);
                setError(null);
              }}
              data-testid="dictionary-term-written"
            />
          </Field>
          <Field
            id={spokenId}
            label={DICTIONARY_LABELS.term.spokenAs}
            info={dictionaryMessage('MSG-SPOKEN-FORMS-HELP')}
            error={tooManyForms ? dictionaryMessage('MSG-TERM-FORMS') : undefined}
          >
            <Textarea
              rows={3}
              value={spoken}
              readOnly={done}
              onChange={(event) => setSpoken(event.target.value)}
              data-testid="dictionary-term-spoken"
            />
          </Field>
          <Field id={noteId} label={DICTIONARY_LABELS.term.note}>
            <Input
              value={note}
              maxLength={280}
              readOnly={done}
              onChange={(event) => setNote(event.target.value)}
              data-testid="dictionary-term-note"
            />
          </Field>
          <div className="flex flex-col gap-1">
            <label
              htmlFor={privateId}
              className="flex items-center gap-2 text-body text-text-default"
            >
              <Checkbox
                id={privateId}
                checked={onlyPrivate}
                disabled={privateDisabled}
                onChange={(event) => setOnlyPrivate(event.target.checked)}
                aria-describedby={privateHelp ? privateHelpId : undefined}
                data-testid="dictionary-term-private"
              />
              {DICTIONARY_LABELS.term.onlyPrivate}
            </label>
            {privateHelp ? (
              <p
                id={privateHelpId}
                className={
                  addFromPrivateUnticked
                    ? 'pl-8 text-supporting text-text-warning'
                    : 'pl-8 text-supporting text-text-muted'
                }
                data-testid="dictionary-term-private-help"
              >
                {privateHelp}
              </p>
            ) : null}
          </div>
          <Disclosure label={DICTIONARY_LABELS.term.moreOptions}>
            <div className="flex flex-col gap-3 pt-2">
              <fieldset className="flex flex-col gap-1" disabled={done}>
                <legend className="mb-1 flex items-center text-label text-text-default">
                  {DICTIONARY_LABELS.term.match}
                  <InfoTip
                    label={DICTIONARY_LABELS.term.match}
                    help={dictionaryMessage('MSG-MATCH-HELP')}
                  />
                </legend>
                {MATCHES.map((value) => (
                  <CustomRadio
                    key={value}
                    id={`${termId}-match-${value}`}
                    name={`${termId}-match`}
                    value={value}
                    checked={match === value}
                    onChange={() => setMatch(value)}
                    label={DICTIONARY_LABELS.term.matchOptions[value]}
                  />
                ))}
              </fieldset>
              <label className="flex items-center gap-2 text-body text-text-default">
                <Checkbox
                  checked={always}
                  disabled={done}
                  onChange={(event) => setAlways(event.target.checked)}
                  data-testid="dictionary-term-always"
                />
                {DICTIONARY_LABELS.term.always}
              </label>
            </div>
          </Disclosure>
          {/* Enter in a field saves, as every form in the app does. */}
          <button type="submit" hidden aria-hidden tabIndex={-1} />
        </form>
      </ModalShell>
      {editing ? (
        <ConfirmationModal
          isOpen={confirmPublicize}
          title={DICTIONARY_LABELS.publicize.title}
          message={dictionaryMessage(publicizeWarning(editing))}
          confirmLabel={DICTIONARY_LABELS.publicize.confirm}
          cancelLabel={DICTIONARY_LABELS.cancel}
          confirmVariant="default"
          onConfirm={() => {
            setConfirmPublicize(false);
            void save(true);
          }}
          onCancel={() => setConfirmPublicize(false)}
        />
      ) : null}
    </>
  );
}
