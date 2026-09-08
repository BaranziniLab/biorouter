import { useState } from 'react';
import { Button } from '../ui/button';
// V8 — the app's OWN checkbox, not `@radix-ui/themes`'. It is built exactly
// like the `CustomRadio` it sits under (a `peer sr-only` input driving styled
// siblings, 22px inside a 24px target), so the two controls in this dialog share
// an optical axis and one theme. The Radix Themes control is a different design
// system's box: it reads its own accent, not the family's, and nothing around it
// can reach inside.
import { Checkbox } from '../ui/Checkbox';
import CustomRadio from '../ui/CustomRadio';
import { MODAL_SIZE } from '../ModalShell';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import type { AppManifest, ExportMode, ExportOptions } from './appManagement';

interface ExportAppDialogProps {
  app: AppManifest;
  onConfirm: (options: ExportOptions) => void;
  onCancel: () => void;
}

/**
 * A compact labelled checkbox with a one-line explanation underneath.
 *
 * V2 — **no `min-h-*`**: `.biorouter-list-row` already carries
 * `min-height: var(--row-height)`, and the `min-h-10` this replaced was a
 * second spelling of the same 40px that could drift from the token.
 *
 * The 4px horizontal inset is deliberately NOT the settings row's `px-3`.
 * These rows sit directly under the `CustomRadio`s above them, which carry no
 * horizontal padding at all, so `px-3` would indent every checkbox past the
 * radio ring it is meant to line up with.
 */
function IncludeCheckbox({
  id,
  checked,
  onChange,
  label,
  hint,
}: {
  id: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  hint: string;
}) {
  return (
    <div className="biorouter-list-row flex items-start gap-2 rounded-none px-1 py-2">
      <Checkbox
        id={id}
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
        className="mt-0.5"
      />
      <label htmlFor={id} className="min-w-0 cursor-pointer">
        <span className="block text-label text-text-default">{label}</span>
        <span className="block text-supporting text-text-muted">{hint}</span>
      </label>
    </div>
  );
}

/**
 * The Applications-panel export dialog (Apps SDK v2, design §3.9): launcher vs
 * full export, with per-item payload toggles pre-checked from what the app's
 * agent config actually references. Confirm hands the chosen options back to
 * the caller, which performs the request; Cancel sends nothing.
 */
export default function ExportAppDialog({ app, onConfirm, onCancel }: ExportAppDialogProps) {
  const kb = app.agent?.knowledge_base || undefined;
  const skills = app.agent?.skills ?? [];
  const extensions = app.agent?.extensions ?? [];

  const [mode, setMode] = useState<ExportMode>('launcher');
  const [bundleDaemon, setBundleDaemon] = useState(false);
  // Pre-checked from the manifest: everything the agent references travels by
  // default; the user opts out item by item.
  const [includeKb, setIncludeKb] = useState(kb !== undefined);
  const [selectedSkills, setSelectedSkills] = useState<ReadonlySet<string>>(() => new Set(skills));
  const [selectedExtensions, setSelectedExtensions] = useState<ReadonlySet<string>>(
    () => new Set(extensions)
  );

  const toggle = (set: ReadonlySet<string>, item: string, checked: boolean): Set<string> => {
    const next = new Set(set);
    if (checked) next.add(item);
    else next.delete(item);
    return next;
  };

  const confirm = () => {
    const include =
      mode === 'full'
        ? {
            knowledge_bases: kb && includeKb ? [kb] : undefined,
            skills: skills.filter((s) => selectedSkills.has(s)),
            extensions: extensions.filter((e) => selectedExtensions.has(e)),
          }
        : {};
    onConfirm({ mode, bundleDaemon: mode === 'full' && bundleDaemon, include });
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      {/* V8 — a dialog's width comes off the `MODAL_SIZE` ladder, never from a
          Tailwind `max-w-*` fork. `md` is the forms rung, and this dialog is a
          form: a mode choice plus a column of include toggles. The `sm:max-w-md`
          it replaces was 448px, a fifteenth width nobody chose. */}
      <DialogContent className={MODAL_SIZE.md}>
        <DialogHeader>
          <DialogTitle>Export “{app.title}”</DialogTitle>
          <DialogDescription>
            Choose what travels with the exported folder. Credentials never travel. The recipient is
            prompted on first run.
          </DialogDescription>
        </DialogHeader>

        <div role="radiogroup" aria-label="Export mode" className="flex flex-col">
          <CustomRadio
            id="export-mode-launcher"
            name="export-mode"
            value="launcher"
            checked={mode === 'launcher'}
            onChange={() => setMode('launcher')}
            label="Launcher (app + scripts)"
            secondaryLabel="Smallest folder. Uses the knowledge bases, skills, and extensions already on the target machine."
          />
          <CustomRadio
            id="export-mode-full"
            name="export-mode"
            value="full"
            checked={mode === 'full'}
            onChange={() => setMode('full')}
            label="Full (bundle server-side content)"
            secondaryLabel="Carries the items below so a fresh machine can run the app."
          />
        </div>

        {mode === 'full' && (
          <div className="flex flex-col border-y border-border-subtle">
            {kb && (
              <IncludeCheckbox
                id="export-include-kb"
                checked={includeKb}
                onChange={setIncludeKb}
                label={`Knowledge base: ${kb}`}
                hint="Bundled as a .brkb and imported into the recipient's store on first run."
              />
            )}
            {skills.map((skill) => (
              <IncludeCheckbox
                key={skill}
                id={`export-include-skill-${skill}`}
                checked={selectedSkills.has(skill)}
                onChange={(checked) => setSelectedSkills((prev) => toggle(prev, skill, checked))}
                label={`Skill: ${skill}`}
                hint="Staged in the export and installed into the recipient's skills on first run."
              />
            ))}
            {extensions.map((ext) => (
              <IncludeCheckbox
                key={ext}
                id={`export-include-extension-${ext}`}
                checked={selectedExtensions.has(ext)}
                onChange={(checked) => setSelectedExtensions((prev) => toggle(prev, ext, checked))}
                label={`Extension: ${ext}`}
                hint="Bundled or pinned for first-run install; built-ins already travel with the daemon."
              />
            ))}
            <IncludeCheckbox
              id="export-bundle-daemon"
              checked={bundleDaemon}
              onChange={setBundleDaemon}
              label="Bundle daemon binary (~110 MB)"
              hint="Skips the biorouterd download on the recipient's machine."
            />
            <p className="text-supporting text-text-muted mt-1">
              Full exports can be large. A bundled knowledge base carries its sources.
            </p>
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={confirm}>Export</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
