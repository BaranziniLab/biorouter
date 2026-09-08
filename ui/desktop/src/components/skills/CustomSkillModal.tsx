import { useState } from 'react';
import { Button } from '../ui/button';
import { Note } from '../ui/note';
import { MODAL_SIZE } from '../ModalShell';
import { parseSkillFrontmatter, toSlug, BIOROUTER_SKILLS_DIR } from './skillUtils';
import { toastSuccess, toastError } from '../../toasts';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';

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
      setError('File must have valid YAML frontmatter with "name" and "description" fields.');
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
        toastSuccess({ title: parsed.name, msg: 'Skill saved to Biorouter Skills' });
        onSaved();
        onClose();
      } else {
        toastError({ title: 'Save failed', msg: `Could not write to ${destFolder}/SKILL.md` });
      }
    } catch (error) {
      toastError({
        title: 'Save failed',
        msg: error instanceof Error ? error.message : `Could not write to ${destFolder}/SKILL.md`,
      });
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && !isSaving && onClose()}>
      <DialogContent
        dismissible={!isSaving}
        // `MODAL_SIZE.lg` IS 640px — the three-rung ladder, not the two
        // breakpoint-forked literals plus a `w-[640px]` this was built from.
        // `DialogContent` already carries `w-full max-w-[calc(100%-2rem)]`, so
        // the narrow-window behaviour those spelled out is the default.
        className={`flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0 ${MODAL_SIZE.lg}`}
      >
        <div className="px-6 pt-5 pb-4 pr-14 border-b border-border-subtle">
          <DialogTitle>Add custom skill</DialogTitle>
        </div>

        <div className="p-6 flex flex-col gap-3 flex-1 overflow-hidden">
          <DialogDescription className="text-supporting text-text-muted">
            Edit the YAML frontmatter (<code>name</code> and <code>description</code> required),
            then write your skill instructions below. A folder named after the skill will be created
            in Biorouter Skills with a <code>SKILL.md</code> inside.
          </DialogDescription>
          <textarea
            className="biorouter-modal-panel flex-1 min-h-[300px] font-mono text-code rounded-element p-3 resize-none"
            value={content}
            onChange={(e) => setContent(e.target.value)}
            spellCheck={false}
          />
          {error && (
            <Note tone="danger" role="alert">
              {error}
            </Note>
          )}
        </div>

        <div className="px-6 py-4 border-t border-border-subtle flex justify-end gap-2">
          <Button variant="outline" onClick={onClose} disabled={isSaving}>
            Cancel
          </Button>
          <Button variant="default" onClick={handleSave} disabled={isSaving}>
            {isSaving ? 'Saving…' : 'Save skill'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
