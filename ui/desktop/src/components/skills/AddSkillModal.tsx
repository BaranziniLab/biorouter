import { useState, useRef, DragEvent } from 'react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import { MODAL_SIZE } from '../ModalShell';
import { toastSuccess, toastError } from '../../toasts';
import { Dialog, DialogContent, DialogTitle } from '../ui/dialog';
import { installSkillPackage, previewSkillPackage } from '../../api';
import type { ImportPreview, ImportRequest, ImportResult } from '../../api';

interface Props {
  onClose: () => void;
  onSaved: () => void;
}

/**
 * Add Skill.
 *
 * ⚠ **Nothing here parses an archive.** The modal used to read a `.md` in the
 * renderer and hand a `.zip` to a depth-counting daemon parser, and it had no
 * way at all to take a repository URL — so a user with
 * `https://github.com/heygen-com/hyperframes` had to ask the agent, which
 * improvised with shell commands and produced twenty unrelated top-level skills
 * (#115). Every source now goes to the one import pipeline, which reads the
 * package's own manifest and keeps a coordinated repository together.
 *
 * The preview shown below is the daemon's, not a second interpretation of the
 * same bytes.
 */
export default function AddSkillModal({ onClose, onSaved }: Props) {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [planId, setPlanId] = useState<string | null>(null);
  const [sourceLabel, setSourceLabel] = useState<string>('');
  const [url, setUrl] = useState('');
  const [busy, setBusy] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const errorText = (err: unknown) =>
    err instanceof Error ? err.message : typeof err === 'string' ? err : 'the request failed.';

  const runPreview = async (request: ImportRequest, label: string) => {
    setBusy(true);
    try {
      const response = await previewSkillPackage<true>({ body: request, throwOnError: true });
      const result = response.data as ImportResult;
      setError(null);
      setSourceLabel(label);
      setPreview(result.preview);
      setPlanId(result.status === 'needsChoice' ? result.planId : null);
    } catch (err) {
      setError(errorText(err));
      setPreview(null);
      setPlanId(null);
    } finally {
      setBusy(false);
    }
  };

  const previewFile = async (file: File) => {
    const filePath = window.electron.getPathForFile(file);
    // See `BrxtInstallModal`: an empty path means this surface cannot supply
    // one. Sending the bare name would have the daemon read whatever matching
    // archive sat in its own working directory.
    if (!filePath) {
      setError(
        'Biorouter is running on another machine, so it cannot read a file you ' +
          'drop here. Copy the skill onto that machine and add it with ' +
          '`biorouter skill install <path>`, or paste a repository URL above.'
      );
      setPreview(null);
      return;
    }
    await runPreview({ filePath }, file.name);
  };

  const handleDrop = async (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragging(false);
    const file = e.dataTransfer.files[0];
    if (file) await previewFile(file);
  };

  const handleBrowse = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) await previewFile(file);
    e.target.value = '';
  };

  const previewUrl = async () => {
    const trimmed = url.trim();
    if (!trimmed) return;
    await runPreview({ url: trimmed }, trimmed);
  };

  const install = async (choice?: 'bundle' | 'individual') => {
    if (!preview || busy) return;
    setBusy(true);
    try {
      // Installing by `planId` rather than by source is what makes the preview
      // binding: it installs the archive that was previewed, not whatever the
      // branch points at now.
      const body: ImportRequest = planId ? { planId } : { url: url.trim() || null };
      if (choice) body.choice = choice;
      const response = await installSkillPackage<true>({ body, throwOnError: true });
      const result = response.data as ImportResult;
      if (result.status === 'needsChoice') {
        // The daemon still wants an answer; keep the fresh plan id.
        setPlanId(result.planId);
        setPreview(result.preview);
        setBusy(false);
        return;
      }
      const count = result.installed.reduce((total, one) => total + one.skills.length, 0);
      toastSuccess({
        title: result.installed[0]?.displayName ?? preview.displayName,
        msg:
          result.installed.length === 1 && result.installed[0].kind === 'bundle'
            ? `Installed ${count} skill${count === 1 ? '' : 's'}`
            : `Installed ${result.installed.length} skill${result.installed.length === 1 ? '' : 's'}`,
      });
      onSaved();
      onClose();
    } catch (err) {
      toastError({ title: 'Install failed', msg: errorText(err) });
      setBusy(false);
    }
  };

  const ambiguity = preview?.ambiguity ?? null;

  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent
        aria-describedby={undefined}
        dismissible={!busy}
        // `MODAL_SIZE.lg`, not the 520px literal this carried: L is the rung for
        // "anything with a list", and the preview below is one. The `w-[520px]`
        // that came with it is gone too — `DialogContent`'s own `w-full` plus
        // the rung's cap is what every other dialog in the app is sized by.
        className={`flex max-h-[80vh] flex-col gap-0 overflow-hidden p-0 ${MODAL_SIZE.lg}`}
      >
        <div className="px-6 pt-5 pb-4 pr-14 border-b border-border-subtle">
          <DialogTitle>Add Skill</DialogTitle>
        </div>

        <div className="p-6 flex flex-col gap-4 overflow-y-auto">
          <div className="flex flex-col gap-1.5">
            <label htmlFor="skill-source-url" className="text-label text-text-default">
              From a repository
            </label>
            <div className="flex gap-2">
              <Input
                id="skill-source-url"
                type="text"
                placeholder="https://github.com/owner/repo"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void previewUrl();
                }}
                // No height here: `Input` is already the 32px md rung, the same
                // box the "Look up" Button beside it takes. The `h-9` this
                // carried made the field 36px next to a 32px button.
                className="flex-1"
                disabled={busy}
              />
              <Button
                variant="outline"
                onClick={() => void previewUrl()}
                disabled={busy || !url.trim()}
              >
                Look up
              </Button>
            </div>
            <p className="text-supporting text-text-muted">
              A repository holding several skills stays one package, with its own name and entry
              point.
            </p>
          </div>

          <div
            onDragOver={(e) => {
              e.preventDefault();
              setIsDragging(true);
            }}
            onDragLeave={() => setIsDragging(false)}
            onDrop={handleDrop}
            onClick={() => fileInputRef.current?.click()}
            // ⚠ **Not `biorouter-modal-panel`, and that is the whole point.**
            // That class is UNLAYERED in main.css, so its `background` and
            // `border` beat any Tailwind utility in `@layer utilities` whatever
            // the specificity — which meant every background and border class
            // this element used to carry beside it was a no-op, and the dropzone
            // never changed appearance on drag, on error, or on hover. The
            // panel's own two values are spelled out here instead, as utilities,
            // so the state can actually move them.
            //
            // Hover/press is `tint-interactive`, never `hover:bg-overlay-hover`:
            // that sets a background-COLOUR, which REPLACES an opaque ground
            // rather than compositing over it, so the zone got LIGHTER under the
            // pointer (main.css, "the interaction tints").
            className={[
              'rounded-container border bg-background-muted p-8 text-center cursor-pointer select-none tint-interactive transition-colors',
              isDragging
                ? 'border-border-accent'
                : error
                  ? 'border-border-danger'
                  : 'border-border-subtle',
            ].join(' ')}
          >
            <p className="text-label text-text-default mb-1">Or drop a skill file here</p>
            <p className="text-supporting text-text-muted">
              Accepts <code>.zip</code>
            </p>
          </div>
          <input
            ref={fileInputRef}
            type="file"
            accept=".zip"
            className="hidden"
            onChange={handleBrowse}
          />

          {/* One note (V4). Both of these were hand-rolled prose boxes — the
              error on a hand-mixed `bg-background-danger/10`, the question on a
              flat surface step. Tone is a `--wash-*` now, derived per family and
              per mode, so each reads correctly under all three themes in both
              modes with no `.dark` fork. */}
          {error && (
            <Note tone="danger" role="alert">
              {error}
            </Note>
          )}

          {preview && <PreviewCard preview={preview} sourceLabel={sourceLabel} />}

          {ambiguity && <Note tone="info">{ambiguity.reason}</Note>}
        </div>

        <div className="px-6 py-4 border-t border-border-subtle flex justify-end gap-2">
          <Button variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          {ambiguity ? (
            <>
              <Button variant="outline" onClick={() => void install('individual')} disabled={busy}>
                Install separately
              </Button>
              <Button variant="default" onClick={() => void install('bundle')} disabled={busy}>
                Install as one bundle
              </Button>
            </>
          ) : (
            <Button variant="default" onClick={() => void install()} disabled={!preview || busy}>
              {busy ? 'Installing…' : installLabel(preview)}
            </Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function installLabel(preview: ImportPreview | null): string {
  if (!preview) return 'Install';
  if (preview.kind === 'single') return 'Install Skill';
  return `Install ${preview.components.length} skills`;
}

function PreviewCard({ preview, sourceLabel }: { preview: ImportPreview; sourceLabel: string }) {
  const entryPoint = preview.entryPoint;
  return (
    <div className="biorouter-modal-panel rounded-element px-4 py-3">
      <p className="text-label">
        {preview.displayName}
        {preview.version && (
          <span className="ml-2 text-supporting text-text-subtle">{preview.version}</span>
        )}
        {preview.kind === 'bundle' && (
          <span className="ml-2 text-supporting text-text-subtle">
            {preview.components.length} skill{preview.components.length === 1 ? '' : 's'}
          </span>
        )}
      </p>
      {entryPoint && (
        <p className="text-supporting text-text-muted mt-0.5">entry point: {entryPoint}</p>
      )}
      <div className="mt-1.5 max-h-[140px] overflow-y-auto">
        {preview.components.map((component) => (
          <p key={component.name} className="text-supporting text-text-muted">
            {component.entryPoint ? '→' : '·'} {component.name}
            {component.group && <span className="text-text-subtle"> [{component.group}]</span>}
            {component.description && (
              <span className="text-text-subtle">: {component.description}</span>
            )}
          </p>
        ))}
      </div>
      <p className="text-supporting text-text-subtle mt-1.5 font-mono">
        {preview.fileCount} file{preview.fileCount !== 1 ? 's' : ''} · from {sourceLabel}
      </p>
    </div>
  );
}
