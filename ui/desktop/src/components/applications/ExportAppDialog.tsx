import { useEffect, useRef, useState } from 'react';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import CustomRadio from '../ui/CustomRadio';
import { InfoTip } from '../ui/info-tip';
import { ModalShell } from '../ModalShell';
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
import { appsCopy } from './copy';

const copy = appsCopy.exportDialog;

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
 * One included item: the checkbox and its name, nothing else (spec 3.10: "the item rows are
 * checkbox plus name"). What a group of items does on the other machine is said once, in the
 * group's InfoTip, not repeated under every skill and every extension. The whole row is the
 * label, so a click anywhere on it toggles the box (the app's `Checkbox` is an sr-only input
 * whose square only toggles inside a `<label>`).
 */
function IncludeRow({
  id,
  checked,
  onChange,
  name,
  disabled,
}: {
  id: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  name: string;
  disabled?: boolean;
}) {
  return (
    <label htmlFor={id} className="flex min-h-8 min-w-0 cursor-pointer items-center gap-2">
      <Checkbox
        id={id}
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="min-w-0 truncate text-body text-text-default">{name}</span>
    </label>
  );
}

/** A group of included items under a caps section label with one InfoTip. */
function IncludeGroup({
  title,
  info,
  children,
}: {
  title: string;
  info: string;
  children: React.ReactNode;
}) {
  return (
    <div role="group" aria-label={title} className="flex flex-col gap-1">
      <div className="flex items-center gap-1">
        <span className="text-caps text-text-muted">{title}</span>
        <InfoTip label={title} help={info} />
      </div>
      {children}
    </div>
  );
}

/** The dialog's three faces: the choice, the export running, and the harness's findings. */
type Phase =
  | { kind: 'choose' }
  | { kind: 'running' }
  | { kind: 'lint'; message: string; findings: string[]; request: AppExportRequest };

/** How many findings are listed before "N more". */
const SHOWN_FINDINGS = 8;

/**
 * The Built apps export dialog (Apps SDK v2, design §3.9): launcher or full export, with every
 * item the app references pre-checked. Confirm runs the export through `onExport` and the dialog
 * stays open, busy, until it answers; Cancel sends nothing.
 *
 * In full mode all three lists are sent, an unchecked item left out and an empty list meaning
 * "none": a list with nothing checked used to be dropped, and the daemon read a missing list as
 * "everything the app references", so a knowledge base the person had unchecked shipped anyway.
 *
 * When the harness lint blocks the export, the findings are shown here and the person may export
 * anyway. That is the only way `allowLintErrors` is ever set.
 *
 * `ModalShell size="md"` (the forms rung). Radio rows appear here because a dialog is the one
 * place principle 5 allows them, each with one consequence line. "Credentials are never
 * included." stays visible: a safety fact is never hidden behind help.
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
      result = { status: 'failed', message: appsCopy.couldNotExport };
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
            // ⚠ All three, always: `[]` means none, and a missing key means everything.
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
  const handleOpenChange = (open: boolean) => {
    if (!open) onClose();
  };

  if (phase.kind === 'lint') {
    const findings = phase.findings.length > 0 ? phase.findings : [phase.message];
    return (
      <ModalShell
        open
        onOpenChange={handleOpenChange}
        size="md"
        purpose="form"
        title={copy.lintTitle(app.title)}
        subtitle={copy.lintSubtitle}
        footer={
          <>
            <Button variant="secondary" onClick={onClose}>
              {copy.cancel}
            </Button>
            <Button onClick={() => void run(phase.request)}>{copy.exportAnyway}</Button>
          </>
        }
      >
        <div className="flex flex-col gap-3 py-1">
          <p className="text-body text-text-default">{copy.lintHelp}</p>
          <ul
            aria-label={copy.problemsFound}
            className="flex max-h-60 flex-col gap-1 overflow-y-auto"
          >
            {findings.slice(0, SHOWN_FINDINGS).map((finding, index) => (
              <li key={index} className="break-words text-supporting text-text-default">
                {finding}
              </li>
            ))}
            {findings.length > SHOWN_FINDINGS && (
              <li className="text-supporting text-text-muted">
                {copy.moreProblems(findings.length - SHOWN_FINDINGS)}
              </li>
            )}
          </ul>
        </div>
      </ModalShell>
    );
  }

  return (
    <ModalShell
      open
      onOpenChange={handleOpenChange}
      size="md"
      purpose="form"
      title={copy.title(app.title)}
      subtitle={browser ? copy.browserSubtitle : copy.credentials}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {copy.cancel}
          </Button>
          <Button onClick={confirm} disabled={running}>
            {running ? copy.exporting : copy.export}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4 py-1">
        <div role="radiogroup" aria-label={copy.mode} className="flex flex-col">
          <CustomRadio
            id="export-mode-launcher"
            name="export-mode"
            value="launcher"
            checked={mode === 'launcher'}
            onChange={() => setMode('launcher')}
            label={copy.launcher}
            secondaryLabel={copy.launcherHint}
          />
          <CustomRadio
            id="export-mode-full"
            name="export-mode"
            value="full"
            checked={mode === 'full'}
            onChange={() => setMode('full')}
            label={copy.full}
            secondaryLabel={copy.fullHint}
          />
        </div>

        {mode === 'full' && (
          <div className="flex flex-col gap-3">
            {knowledgeBases.length > 0 && (
              <IncludeGroup
                title={knowledgeBases.length > 1 ? copy.knowledgeBases : copy.knowledgeBase}
                info={copy.knowledgeBaseInfo}
              >
                {knowledgeBases.map((kb) => (
                  <IncludeRow
                    key={kb}
                    id={`export-include-kb-${kb}`}
                    checked={selectedKbs.has(kb)}
                    disabled={running}
                    onChange={(checked) => setSelectedKbs((prev) => toggle(prev, kb, checked))}
                    name={kb}
                  />
                ))}
              </IncludeGroup>
            )}
            {skills.length > 0 && (
              <IncludeGroup title={copy.skills} info={copy.skillsInfo}>
                {skills.map((skill) => (
                  <IncludeRow
                    key={skill}
                    id={`export-include-skill-${skill}`}
                    checked={selectedSkills.has(skill)}
                    disabled={running}
                    onChange={(checked) =>
                      setSelectedSkills((prev) => toggle(prev, skill, checked))
                    }
                    name={skill}
                  />
                ))}
              </IncludeGroup>
            )}
            {extensions.length > 0 && (
              <IncludeGroup title={copy.extensions} info={copy.extensionsInfo}>
                {extensions.map((ext) => (
                  <IncludeRow
                    key={ext}
                    id={`export-include-extension-${ext}`}
                    checked={selectedExtensions.has(ext)}
                    disabled={running}
                    onChange={(checked) =>
                      setSelectedExtensions((prev) => toggle(prev, ext, checked))
                    }
                    name={ext}
                  />
                ))}
              </IncludeGroup>
            )}
            {knowledgeBases.length === 0 && skills.length === 0 && extensions.length === 0 && (
              <p className="text-supporting text-text-muted">{copy.nothingToInclude}</p>
            )}
            {/* Not offered in a browser: the daemon there was built for the server, not for
                whoever downloads the zip. */}
            {!browser && (
              <div className="flex items-center gap-1">
                <IncludeRow
                  id="export-bundle-daemon"
                  checked={bundleDaemon}
                  disabled={running}
                  onChange={setBundleDaemon}
                  name={copy.includeBiorouter}
                />
                <InfoTip label={copy.includeBiorouter} help={copy.includeBiorouterInfo} />
              </div>
            )}
          </div>
        )}
      </div>
    </ModalShell>
  );
}
