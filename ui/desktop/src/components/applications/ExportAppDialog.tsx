import { useEffect, useRef, useState } from 'react';
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
import {
  exportableExtensions,
  referencedKnowledgeBases,
  referencedSkills,
  type AppManifest,
} from './appManagement';
import type {
  AppExportMode,
  AppExportRequest,
  AppExportResult,
} from '../../utils/appExportRequest';

interface ExportAppDialogProps {
  app: AppManifest;
  /** Runs the export and answers how it went. The dialog stays open, busy, until it does. */
  onExport: (request: AppExportRequest) => Promise<AppExportResult>;
  onClose: () => void;
  /**
   * A `biorouter serve` browser: the export downloads as a zip, and the daemon's own binary is
   * not offered, since it was built for the server rather than for whoever downloads it.
   */
  browser?: boolean;
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
  disabled,
}: {
  id: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  hint: string;
  disabled?: boolean;
}) {
  return (
    <div className="biorouter-list-row flex items-start gap-2 rounded-none px-1 py-2">
      <Checkbox
        id={id}
        checked={checked}
        disabled={disabled}
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

/** The dialog's three faces: the choice, the export running, and the harness's findings. */
type Phase =
  | { kind: 'choose' }
  | { kind: 'running' }
  | { kind: 'lint'; message: string; findings: string[]; request: AppExportRequest };

/** How many findings are listed before "and N more". */
const SHOWN_FINDINGS = 8;

/**
 * The Applications-panel export dialog (Apps SDK v2, design §3.9): launcher vs
 * full export, with per-item payload toggles pre-checked from everything the
 * app references. Confirm runs the export through `onExport`; Cancel sends
 * nothing.
 *
 * In full mode all three lists are sent, an unchecked item left out and an
 * empty list meaning "none": a list with nothing checked used to be dropped, and
 * the daemon read a missing list as "everything the app references", so a
 * knowledge base the person had unchecked shipped anyway.
 *
 * When the harness lint blocks the export, the findings are shown here and the
 * person may export anyway. That is the only way `allowLintErrors` is ever set.
 */
export default function ExportAppDialog({
  app,
  onExport,
  onClose,
  browser = false,
}: ExportAppDialogProps) {
  const knowledgeBases = referencedKnowledgeBases(app);
  const skills = referencedSkills(app);
  const extensions = exportableExtensions(app);

  const [mode, setMode] = useState<AppExportMode>('launcher');
  const [bundleDaemon, setBundleDaemon] = useState(false);
  // Pre-checked from the manifest: everything the agent references travels by
  // default; the user opts out item by item.
  const [selectedKbs, setSelectedKbs] = useState<ReadonlySet<string>>(
    () => new Set(knowledgeBases)
  );
  const [selectedSkills, setSelectedSkills] = useState<ReadonlySet<string>>(() => new Set(skills));
  const [selectedExtensions, setSelectedExtensions] = useState<ReadonlySet<string>>(
    () => new Set(extensions)
  );
  const [phase, setPhase] = useState<Phase>({ kind: 'choose' });
  const mounted = useRef(true);
  useEffect(
    () => () => {
      mounted.current = false;
    },
    []
  );

  const toggle = (set: ReadonlySet<string>, item: string, checked: boolean): Set<string> => {
    const next = new Set(set);
    if (checked) next.add(item);
    else next.delete(item);
    return next;
  };

  const run = async (request: AppExportRequest) => {
    setPhase({ kind: 'running' });
    let result: AppExportResult;
    try {
      result = await onExport(request);
    } catch {
      result = { status: 'failed', message: 'Could not export the app.' };
    }
    if (!mounted.current) return;
    if (result.status === 'lint') {
      setPhase({
        kind: 'lint',
        message: result.message,
        findings: result.findings,
        request: { ...request, allowLintErrors: true, retry: result.retry || undefined },
      });
      return;
    }
    onClose();
  };

  const confirm = () => {
    void run({
      mode,
      bundleDaemon: !browser && mode === 'full' && bundleDaemon,
      allowLintErrors: false,
      ...(mode === 'full'
        ? {
            include: {
              knowledge_bases: knowledgeBases.filter((kb) => selectedKbs.has(kb)),
              skills: skills.filter((s) => selectedSkills.has(s)),
              extensions: extensions.filter((e) => selectedExtensions.has(e)),
            },
          }
        : {}),
    });
  };

  const running = phase.kind === 'running';

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {/* V8 — a dialog's width comes off the `MODAL_SIZE` ladder, never from a
          Tailwind `max-w-*` fork. `md` is the forms rung, and this dialog is a
          form: a mode choice plus a column of include toggles. The `sm:max-w-md`
          it replaces was 448px, a fifteenth width nobody chose. */}
      <DialogContent className={MODAL_SIZE.md}>
        {phase.kind === 'lint' ? (
          <>
            <DialogHeader>
              <DialogTitle>Export “{app.title}” anyway?</DialogTitle>
              <DialogDescription>
                The app check found problems that can keep the exported app from working. Fix them
                in the chat that built it, or export it as it is.
              </DialogDescription>
            </DialogHeader>
            <ul
              aria-label="Problems found"
              className="flex max-h-60 flex-col gap-1 overflow-y-auto border-y border-border-subtle py-2"
            >
              {(phase.findings.length > 0 ? phase.findings : [phase.message])
                .slice(0, SHOWN_FINDINGS)
                .map((finding, index) => (
                  <li key={index} className="text-supporting text-text-default">
                    {finding}
                  </li>
                ))}
              {phase.findings.length > SHOWN_FINDINGS && (
                <li className="text-supporting text-text-muted">
                  and {phase.findings.length - SHOWN_FINDINGS} more
                </li>
              )}
            </ul>
            <DialogFooter>
              <Button variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button onClick={() => void run(phase.request)}>Export anyway</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Export “{app.title}”</DialogTitle>
              <DialogDescription>
                {browser
                  ? 'Downloads the app as a .zip of a folder that runs on another computer. '
                  : 'Writes a folder that runs on another computer. '}
                Credentials never travel. The recipient is prompted on first run.
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
                {knowledgeBases.map((kb) => (
                  <IncludeCheckbox
                    key={kb}
                    id={`export-include-kb-${kb}`}
                    checked={selectedKbs.has(kb)}
                    disabled={running}
                    onChange={(checked) => setSelectedKbs((prev) => toggle(prev, kb, checked))}
                    label={`Knowledge base: ${kb}`}
                    hint="Bundled as a .brkb with its sources. A private base is included only when you could open it."
                  />
                ))}
                {skills.map((skill) => (
                  <IncludeCheckbox
                    key={skill}
                    id={`export-include-skill-${skill}`}
                    checked={selectedSkills.has(skill)}
                    disabled={running}
                    onChange={(checked) =>
                      setSelectedSkills((prev) => toggle(prev, skill, checked))
                    }
                    label={`Skill: ${skill}`}
                    hint="Staged in the export and installed into the recipient's skills on first run."
                  />
                ))}
                {extensions.map((ext) => (
                  <IncludeCheckbox
                    key={ext}
                    id={`export-include-extension-${ext}`}
                    checked={selectedExtensions.has(ext)}
                    disabled={running}
                    onChange={(checked) =>
                      setSelectedExtensions((prev) => toggle(prev, ext, checked))
                    }
                    label={`Extension: ${ext}`}
                    hint="Recorded in export.json for the recipient to install from the marketplace."
                  />
                ))}
                {knowledgeBases.length === 0 && skills.length === 0 && extensions.length === 0 && (
                  <p className="py-2 text-supporting text-text-muted">
                    This app references no knowledge bases, skills, or extensions.
                  </p>
                )}
                {!browser && (
                  <IncludeCheckbox
                    id="export-bundle-daemon"
                    checked={bundleDaemon}
                    disabled={running}
                    onChange={setBundleDaemon}
                    label="Bundle daemon binary (~110 MB)"
                    hint="Runs on a computer like this one with no Biorouter installed."
                  />
                )}
                <p className="text-supporting text-text-muted mt-1">
                  Full exports can be large. A bundled knowledge base carries its sources.
                </p>
              </div>
            )}

            <DialogFooter>
              <Button variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button onClick={confirm} disabled={running} aria-busy={running || undefined}>
                {running ? 'Exporting…' : 'Export'}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
