import { useState } from 'react';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { InfoTip } from '../ui/info-tip';
import { Textarea } from '../ui/textarea';
import { ModalShell } from '../ModalShell';
import { parseSkillFrontmatter, toSlug, BIOROUTER_SKILLS_DIR } from './skillUtils';
import { toastSuccess, toastError } from '../../toasts';
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
}

export default function CustomSkillModal({ onClose, onSaved }: Props) {
  const [content, setContent] = useState(TEMPLATE);
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const handleSave = async () => {
    if (isSaving) return;
    const parsed = parseSkillFrontmatter(content);
    if (!parsed) {
      setError(CUSTOM_SKILL_COPY.invalid);
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
      // Typed instructions survive a stray backdrop click; nothing dismisses a
      // write in flight.
      purpose={isSaving ? 'required' : 'form'}
      title={CUSTOM_SKILL_COPY.title}
      subtitle={
        <span className="inline-flex flex-wrap items-center gap-x-1">
          <span>{CUSTOM_SKILL_COPY.subtitle}</span>
          <InfoTip label={CUSTOM_SKILL_COPY.title.toLowerCase()} help={CUSTOM_SKILL_COPY.help} />
        </span>
      }
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
          the one text-field skin. */}
      <Textarea
        aria-label={CUSTOM_SKILL_COPY.editorLabel}
        rows={14}
        className="resize-none p-3 font-mono text-code"
        value={content}
        onChange={(e) => setContent(e.target.value)}
        spellCheck={false}
      />
      {error && (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      )}
    </ModalShell>
  );
}
