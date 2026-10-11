import { useEffect, useMemo, useRef, useState } from 'react';
import { AlertTriangle } from '../../icons/app-icons';
import type { KbFormat } from '../../../api/types.gen';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Field } from '../../ui/field';
import { InfoTip } from '../../ui/info-tip';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import CustomRadio from '../../ui/CustomRadio';
import { ModalShell } from '../../ModalShell';
import { knowledgeCopy } from '../copy';
import { previewKbId } from '../kbFormat';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Prefill the name field — the manager's search box, when the user typed one. */
  initialName?: string;
  /** Create it. Resolves with the id the DAEMON assigned, which may differ. */
  onCreate: (name: string, format: KbFormat) => Promise<string | undefined>;
}

/**
 * One row of the format radiogroup: the name, its short code and one line saying what it is
 * for. The facts that used to sit under each option (three sentences each, six on screen at
 * once) are the option's InfoTip now (spec 3.12).
 *
 * The slots map onto `CustomRadio`'s own `label` and `secondaryLabel`. The InfoTip is the radio
 * label's SIBLING, never inside it: a click on the glyph inside a `<label>` would also pick the
 * radio (spec 2.6, InfoTip "Never").
 *
 * ⚠ **The badge carries the SHORT CODE and the label carries what it is.** Taken literally the
 * two are the same word, so the row rendered as `OKF ⟨OKF⟩`, a stutter that teaches nothing. The
 * badge is the same mark the band and the manager rows show, so the user learns the mapping at
 * the moment of choosing.
 */
const FORMATS: {
  value: KbFormat;
  code: string;
  copy: { name: string; line: string; info: string };
}[] = [
  { value: 'okf', code: 'OKF', copy: knowledgeCopy.format.okf },
  { value: 'biookf', code: 'BioOKF', copy: knowledgeCopy.format.biookf },
];

/**
 * The format chooser (ui-spec §4.3) — the surface that makes OKF versus BioOKF
 * a decision the user makes rather than one the daemon defaults.
 *
 * ⚠ **The id line is a PREVIEW and the copy says so.** Slug derivation and
 * collision handling are owned by `create_base_as` on the daemon, not by the
 * renderer, so a client-derived id the server then alters is a real footgun. The
 * created base's actual id is echoed back through `onCreate`'s resolution.
 *
 * ⚠ **The irreversibility banner is not decoration.** `kb_migrate_format` is
 * deferred by DR-22, so the choice really is permanent in this build and the UI
 * is obliged to say so. When migration ships the banner is deleted and nothing
 * else in this surface changes.
 *
 * ⚠ **A name problem belongs to the Name field, and it REPLACES the folder line.**
 * "Enter a name." used to be a detached 14px paragraph at the foot of the dialog: no
 * danger edge, no `aria-invalid`, focus left on the button, and the paragraph's own
 * height pushed Cancel and Create 36px down from under the pointer (V3-R4-3). It is the
 * shared `Field`'s error now (12px, danger ink, the danger edge, `aria-invalid`, linked
 * by `aria-describedby`), in the slot the folder preview holds, so the dialog does not
 * grow, and focus goes to the field so the person can type. A failure the daemon reports
 * is not about the field and stays a danger `Note` at the foot, as in the schedule
 * dialog.
 *
 * This is also the surface a future `Convert format` action reuses. Do not build
 * a second one.
 */
export function KbFormatChooser({ open, onOpenChange, initialName = '', onCreate }: Props) {
  const [name, setName] = useState(initialName);
  const [format, setFormat] = useState<KbFormat>('okf');
  const [busy, setBusy] = useState(false);
  // What is wrong with the Name the person typed, caught before anything is sent.
  const [nameError, setNameError] = useState<string | null>(null);
  // A failure the create request came back with: not about the field, so not on it.
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setName(initialName);
    setFormat('okf');
    setBusy(false);
    setNameError(null);
    setError(null);
  }, [open, initialName]);

  const id = useMemo(() => previewKbId(name), [name]);

  async function submit() {
    const trimmed = name.trim();
    const problem = !trimmed
      ? knowledgeCopy.format.nameRequired
      : !previewKbId(trimmed)
        ? knowledgeCopy.format.nameNeedsLetters
        : null;
    if (problem) {
      setNameError(problem);
      setError(null);
      // Create was pressed from the button or from the field; either way the next thing to do
      // is type, so the caret goes to the field the message is about.
      nameRef.current?.focus();
      return;
    }
    setBusy(true);
    setNameError(null);
    setError(null);
    try {
      await onCreate(trimmed, format);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!next && busy) return;
        onOpenChange(next);
      }}
      size="md"
      purpose={busy ? 'required' : 'form'}
      title={knowledgeCopy.format.title}
      footer={
        <>
          <Button
            type="button"
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={busy}
          >
            Cancel
          </Button>
          <Button
            type="button"
            data-testid="knowledge-format-submit"
            onClick={() => void submit()}
            disabled={busy}
          >
            {busy ? 'Creating…' : 'Create knowledge base'}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4 pb-1">
        {/* The id is a PREVIEW: the daemon owns slugs and collisions, so the
            helper line says where the folder will probably go and the InfoTip
            says that it can change. A name problem takes this line's place (a
            Field shows its error where it shows its helper), so the dialog never
            grows under the pointer. */}
        <Field
          id="kb-format-name"
          label={knowledgeCopy.format.nameLabel}
          required
          error={nameError ? <span role="alert">{nameError}</span> : undefined}
          helper={
            <span className="flex min-w-0 items-center gap-1">
              <span className="shrink-0">{knowledgeCopy.format.folderLabel}</span>
              <span className="min-w-0 truncate font-mono">
                {knowledgeCopy.format.folder(id || '…')}
              </span>
              <InfoTip
                label={knowledgeCopy.format.folderInfoLabel}
                help={knowledgeCopy.format.folderInfo}
              />
            </span>
          }
        >
          <Input
            ref={nameRef}
            data-testid="knowledge-format-name"
            type="text"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setNameError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                void submit();
              }
            }}
            placeholder="Knowledge base name"
            autoFocus
          />
        </Field>

        <div role="radiogroup" aria-label="Knowledge base format" className="flex flex-col">
          {FORMATS.map((entry) => (
            <div key={entry.value} className="flex items-center gap-1">
              <CustomRadio
                id={`kb-format-${entry.value}`}
                name="kb-format"
                value={entry.value}
                checked={format === entry.value}
                onChange={() => setFormat(entry.value)}
                className="min-w-0 flex-1"
                label={
                  <span className="flex items-center gap-2">
                    {entry.copy.name}
                    <Badge>{entry.code}</Badge>
                  </span>
                }
                secondaryLabel={entry.copy.line}
              />
              <InfoTip label={entry.copy.name} help={entry.copy.info} side="left" />
            </div>
          ))}
        </div>

        {/* The irreversibility notice is not decoration: `kb_migrate_format` is
            deferred by DR-22, so the choice is permanent in this build. One line;
            when migration ships, this Note is deleted. */}
        <Note tone="warning" icon={AlertTriangle}>
          {knowledgeCopy.format.permanent}
        </Note>

        {error && (
          <Note tone="danger" role="alert">
            {error}
          </Note>
        )}
      </div>
    </ModalShell>
  );
}
