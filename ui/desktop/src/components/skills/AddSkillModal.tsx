import { useState, useRef, DragEvent } from 'react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import { InfoTip, useInfoTipId } from '../ui/info-tip';
import { Upload } from '../icons/app-icons';
import { ModalShell } from '../ModalShell';
import { toastSuccess, toastError } from '../../toasts';
import { ADD_SKILL_COPY } from './copy';
import { installSkillPackage, previewSkillPackage } from '../../api';
import type { ImportPreview, ImportRequest, ImportResult } from '../../api';

interface Props {
  onClose: () => void;
  onSaved: () => void;
}

/**
 * Add skill.
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
  const repositoryHelpId = useInfoTipId();

  const errorText = (err: unknown) =>
    err instanceof Error
      ? err.message
      : typeof err === 'string'
        ? err
        : ADD_SKILL_COPY.requestFailed;

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
      setError(ADD_SKILL_COPY.remoteDaemon);
      setPreview(null);
      return;
    }
    await runPreview({ filePath }, file.name);
  };

  const handleDrop = async (e: DragEvent<HTMLButtonElement>) => {
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
        msg: ADD_SKILL_COPY.installed(
          result.installed.length === 1 && result.installed[0].kind === 'bundle'
            ? count
            : result.installed.length
        ),
      });
      onSaved();
      onClose();
    } catch (err) {
      toastError({ title: ADD_SKILL_COPY.installFailed, msg: errorText(err) });
      setBusy(false);
    }
  };

  const ambiguity = preview?.ambiguity ?? null;

  return (
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      size="lg"
      // A typed URL survives a stray backdrop click; nothing is dismissible
      // while a preview or an install is in flight.
      purpose={busy ? 'required' : 'form'}
      title={ADD_SKILL_COPY.title}
      scrollBody
      bodyClassName="flex flex-col gap-4 py-4"
      footer={
        <>
          <Button variant="secondary" onClick={onClose} disabled={busy}>
            {ADD_SKILL_COPY.cancel}
          </Button>
          {ambiguity ? (
            <>
              <Button
                variant="secondary"
                onClick={() => void install('individual')}
                disabled={busy}
              >
                {ADD_SKILL_COPY.installSeparately}
              </Button>
              <Button variant="default" onClick={() => void install('bundle')} disabled={busy}>
                {ADD_SKILL_COPY.installBundle}
              </Button>
            </>
          ) : (
            <Button variant="default" onClick={() => void install()} disabled={!preview || busy}>
              {busy ? ADD_SKILL_COPY.installing : installLabel(preview)}
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-1">
          <label htmlFor="skill-source-url" className="text-label text-text-default">
            {ADD_SKILL_COPY.repositoryLabel}
          </label>
          <InfoTip
            id={repositoryHelpId}
            label={ADD_SKILL_COPY.repositoryLabel.toLowerCase()}
            help={ADD_SKILL_COPY.repositoryHelp}
          />
        </div>
        <div className="flex gap-2">
          <Input
            id="skill-source-url"
            // First focus lands on the field, not on the help glyph beside its
            // label (Radix keeps a focus already inside the dialog).
            autoFocus
            type="text"
            placeholder={ADD_SKILL_COPY.repositoryPlaceholder}
            aria-describedby={repositoryHelpId}
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void previewUrl();
            }}
            // No height here: `Input` is already the 32px md rung, the same
            // box the "Look up" Button beside it takes.
            className="flex-1"
            disabled={busy}
          />
          <Button
            variant="secondary"
            onClick={() => void previewUrl()}
            disabled={busy || !url.trim()}
          >
            {ADD_SKILL_COPY.lookUp}
          </Button>
        </div>
      </div>

      {/* Knowledge's drop zone recipe (`knowledge/IngestPanel/Dropzone.tsx`): a
          32px glyph plate over one label line, on the muted ground. A real
          button, so the keyboard reaches the file chooser it opens.

          ⚠ **Not `biorouter-modal-panel`.** That class is UNLAYERED in
          main.css, so its `background` and `border` beat any utility whatever
          the specificity, and the zone never changed on drag, on error or on
          hover. Hover is `tint-interactive`, never `hover:bg-overlay-hover`,
          which REPLACES an opaque ground rather than compositing over it. */}
      <button
        type="button"
        aria-label={ADD_SKILL_COPY.dropZoneName}
        disabled={busy}
        onDragOver={(e) => {
          e.preventDefault();
          setIsDragging(true);
        }}
        onDragLeave={() => setIsDragging(false)}
        onDrop={handleDrop}
        onClick={() => fileInputRef.current?.click()}
        className={[
          'biorouter-focus-surface flex w-full cursor-pointer select-none flex-col items-center gap-2 rounded-container border px-4 py-6 text-center transition-colors',
          isDragging
            ? 'border-border-strong bg-background-medium'
            : error
              ? 'border-border-danger bg-background-muted tint-interactive'
              : 'border-border-subtle bg-background-muted tint-interactive',
        ].join(' ')}
      >
        <span
          aria-hidden="true"
          className={`flex h-8 w-8 items-center justify-center rounded-element border border-border-subtle ${isDragging ? 'bg-background-strong text-text-default' : 'bg-background-muted text-text-muted'}`}
        >
          <Upload className="h-4 w-4" />
        </span>
        <span className="text-label text-text-default">{ADD_SKILL_COPY.dropZone}</span>
      </button>
      <input
        ref={fileInputRef}
        type="file"
        accept=".zip"
        className="hidden"
        onChange={handleBrowse}
      />

      {/* One note (V4). Tone is a `--wash-*`, derived per family and per mode. */}
      {error && (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      )}

      {preview && <PreviewSummary preview={preview} sourceLabel={sourceLabel} />}

      {ambiguity && <Note tone="info">{ambiguity.reason}</Note>}
    </ModalShell>
  );
}

function installLabel(preview: ImportPreview | null): string {
  if (!preview) return ADD_SKILL_COPY.install;
  if (preview.kind === 'single') return ADD_SKILL_COPY.installSkill;
  return ADD_SKILL_COPY.installSkills(preview.components.length);
}

/**
 * What the daemon says will be installed. Flat, under a hairline: the dialog is
 * already the box, so the preview is a section of it rather than a card inside
 * it.
 */
function PreviewSummary({ preview, sourceLabel }: { preview: ImportPreview; sourceLabel: string }) {
  const entryPoint = preview.entryPoint;
  const facts = [
    preview.kind === 'bundle'
      ? `${preview.components.length} skill${preview.components.length === 1 ? '' : 's'}`
      : null,
    preview.version,
    ADD_SKILL_COPY.fileCount(preview.fileCount),
    sourceLabel,
  ].filter(Boolean);
  return (
    <section className="flex flex-col gap-2 border-t border-border-subtle pt-3">
      <div className="min-w-0">
        <p className="truncate text-label text-text-default">{preview.displayName}</p>
        <p className="truncate text-supporting text-text-muted">{facts.join(' · ')}</p>
        {entryPoint && (
          <p className="text-supporting text-text-muted">{ADD_SKILL_COPY.entryPoint(entryPoint)}</p>
        )}
      </div>
      <ul className="flex max-h-[140px] flex-col gap-0.5 overflow-y-auto">
        {preview.components.map((component) => (
          <li key={component.name} className="truncate text-supporting">
            <span className="text-text-default">{component.name}</span>
            {component.group && <span className="text-text-muted"> · {component.group}</span>}
            {component.description && (
              <span className="text-text-muted"> · {component.description}</span>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
