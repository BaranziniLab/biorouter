import { useRef, useState } from 'react';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { Field } from '../ui/field';
import { Textarea } from '../ui/textarea';
import { ModalShell } from '../ModalShell';
import { focusFieldOnOpen } from '../baam/focusFieldOnOpen';
import { parseSkillFrontmatter, toSlug, BIOROUTER_SKILLS_DIR } from './skillUtils';
import { toastSuccess, toastError } from '../../toasts';
import type { CatalogSkill } from '../../api';
import { CUSTOM_SKILL_COPY } from './copy';

const TEMPLATE = `---
name: example-skill
description: A brief description of what this skill does and when to use it.
---

# Instructions

Describe the step-by-step instructions for Biorouter to follow when this skill is activated.

## Steps

1. First, do X
2. Then, do Y
3. Finally, do Z

## Notes

- Keep instructions clear and specific
- Include any constraints or edge cases
`;

interface Props {
  onClose: () => void;
  onSaved: () => void;
  /**
   * Every installed skill, from the Skills view's catalog, or `null` while it has not been read.
   * A save is refused, with nothing written, when its name is taken (`takenNameRefusal`).
   */
  skills: readonly CatalogSkill[] | null;
}

/**
 * Why `name` cannot be saved, or `null` when nothing holds it.
 *
 * ⚠ A save is a plain `writeFile` to `<Biorouter skills>/<slug>/SKILL.md`, which replaces what is
 * there, and `refuse_shipped` guards only the package importer. So the check is here, before the
 * write: saving `office-word` overwrote the shipped file until the seeder restored it, and the
 * person's text was lost without a word (rq2-247). Two ways a name is taken:
 * - the same frontmatter name anywhere, because discovery keys on it and one would shadow the
 *   other;
 * - the same top folder in Biorouter's own root, a single skill's or a bundle's, because the
 *   write lands in it.
 * Shipped wins the wording when any holder is shipped.
 */
function takenNameRefusal(name: string, skills: readonly CatalogSkill[] | null): string | null {
  if (skills === null) return CUSTOM_SKILL_COPY.cannotCheck;
  const slug = toSlug(name);
  const holders = skills.filter(
    (skill) =>
      skill.name === name ||
      toSlug(skill.name) === slug ||
      (skill.source.kind === 'biorouter' && skill.slug.split('/')[0].toLowerCase() === slug)
  );
  if (holders.length === 0) return null;
  return holders.some((skill) => skill.builtin)
    ? CUSTOM_SKILL_COPY.shipped(name)
    : CUSTOM_SKILL_COPY.exists(name);
}

export default function CustomSkillModal({ onClose, onSaved, skills }: Props) {
  const [content, setContent] = useState(TEMPLATE);
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const editorRef = useRef<HTMLTextAreaElement>(null);

  const handleSave = async () => {
    if (isSaving) return;
    const parsed = parseSkillFrontmatter(content);
    if (!parsed) {
      setError(CUSTOM_SKILL_COPY.invalid);
      return;
    }
    const refusal = takenNameRefusal(parsed.name, skills);
    if (refusal) {
      setError(refusal);
      return;
    }
    setError(null);
    setIsSaving(true);

    const slug = toSlug(parsed.name);
    const destFolder = `${BIOROUTER_SKILLS_DIR}/${slug}`;
    try {
      await window.electron.ensureDirectory(destFolder);
      const ok = await window.electron.writeFile(`${destFolder}/SKILL.md`, content);
      if (ok) {
        toastSuccess({ title: parsed.name, msg: CUSTOM_SKILL_COPY.saved });
        onSaved();
        onClose();
      } else {
        toastError({
          title: CUSTOM_SKILL_COPY.saveFailed,
          msg: CUSTOM_SKILL_COPY.couldNotWrite(`${destFolder}/SKILL.md`),
        });
      }
    } catch (error) {
      toastError({
        title: CUSTOM_SKILL_COPY.saveFailed,
        msg:
          error instanceof Error
            ? error.message
            : CUSTOM_SKILL_COPY.couldNotWrite(`${destFolder}/SKILL.md`),
      });
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open && !isSaving) onClose();
      }}
      size="lg"
      // First focus lands in the editor, not on the help glyph beside its label. Not `autoFocus`:
      // with focus already inside, Radix never announces the open, the dialog never records
      // which control opened it, and Escape leaves focus on the page (see `focusFieldOnOpen`).
      onOpenAutoFocus={focusFieldOnOpen(editorRef)}
      // Typed instructions survive a stray backdrop click; nothing dismisses a
      // write in flight.
      purpose={isSaving ? 'required' : 'form'}
      title={CUSTOM_SKILL_COPY.title}
      // The dialog grows by one note when a save is refused; anchored at the top
      // it grows downward, so the editor and the buttons do not move under the pointer.
      anchor="top"
      scrollBody
      bodyClassName="flex flex-col gap-3 py-4"
      footer={
        <>
          <Button variant="secondary" onClick={onClose} disabled={isSaving}>
            {CUSTOM_SKILL_COPY.cancel}
          </Button>
          <Button variant="default" onClick={handleSave} disabled={isSaving}>
            {isSaving ? CUSTOM_SKILL_COPY.saving : CUSTOM_SKILL_COPY.save}
          </Button>
        </>
      }
    >
      {/* The SKILL.md source is code, so the mono face earns its place here, on
          the one text-field skin. The label is the only visible text: what the
          file needs and where it is saved are the InfoTip beside it, and the
          error below says it again when a save is refused. */}
      <Field
        id="custom-skill-md"
        label={CUSTOM_SKILL_COPY.editorLabel}
        info={CUSTOM_SKILL_COPY.help}
      >
        <Textarea
          ref={editorRef}
          rows={14}
          className="p-3 font-mono text-code"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          spellCheck={false}
        />
      </Field>
      {error && (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      )}
    </ModalShell>
  );
}
