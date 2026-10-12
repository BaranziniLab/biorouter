import { useEffect, useRef, useState } from 'react';
import { AlertTriangle } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import CustomRadio from '../ui/CustomRadio';
import { InfoTip } from '../ui/info-tip';
import { Note } from '../ui/note';
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

/**
 * What the harness lint answered: its findings, and the request that exports anyway (the same
 * choice, `allowLintErrors` set, and the folder the first attempt already picked).
 */
type LintAnswer = { message: string; findings: string[]; request: AppExportRequest };

/**
 * The Built apps export dialog (Apps SDK v2, design §3.9): launcher or full export, with every
 * item the app references pre-checked. Confirm runs the export through `onExport` and the dialog
 * stays open, busy, until it answers; Cancel sends nothing.
 *
 * In full mode all three lists are sent, an unchecked item left out and an empty list meaning
 * "none": a list with nothing checked used to be dropped, and the daemon read a missing list as
 * "everything the app references", so a knowledge base the person had unchecked shipped anyway.
 *
 * When the harness lint blocks the export, the dialog turns to a second face that shows the
 * findings and lets the person export anyway. That is the only way `allowLintErrors` is ever set.
 * The face stays while "Export anyway" runs: it used to fall back to the first face, so the
 * findings the person had just answered were swapped for the mode choice under the pointer.
 *
 * `ModalShell size="md"` (the forms rung), both faces with a scrolling body: a full export of an
 * app that reaches four knowledge bases, a skill and an extension is taller than the body a
 * 600px window leaves, and a body that does not scroll is clipped (the shell hides its overflow),
 * so the last items and the "Include Biorouter" row could not be reached. Radio rows appear here
 * because a dialog is the one place principle 5 allows them, each with one consequence line.
 * "Credentials are never included." stays visible: a safety fact is never hidden behind help.
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
  const [running, setRunning] = useState(false);
  const [lint, setLint] = useState<LintAnswer | null>(null);
  const mounted = useRef(true);
  useEffect(
    () => () => {
      mounted.current = false;
    },
    []
  );

  // The findings take focus when they arrive. The button that asked turned disabled while the
  // export ran, which drops focus to <body> in Chromium, so a keyboard or screen reader user would
  // otherwise land nowhere, and hear nothing of what the check found. The list is named, so
  // reaching it reads "Problems found" and its length (the non-private model disclosure's
  // pattern: focus the words, not the button that acts on them).
  const findingsRef = useRef<HTMLUListElement>(null);
  useEffect(() => {
    if (lint) findingsRef.current?.focus();
  }, [lint]);

  const toggle = (set: ReadonlySet<string>, item: string, checked: boolean): Set<string> => {
    const next = new Set(set);
    if (checked) next.add(item);
    else next.delete(item);
    return next;
  };

  const run = async (request: AppExportRequest) => {
    setRunning(true);
    let result: AppExportResult;
    try {
      result = await onExport(request);
    } catch {
      result = { status: 'failed', message: appsCopy.couldNotExport };
    }
    if (!mounted.current) return;
    setRunning(false);
    if (result.status === 'lint') {
      setLint({
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

  const handleOpenChange = (open: boolean) => {
    if (!open) onClose();
  };

  if (lint) {
    const findings = lint.findings.length > 0 ? lint.findings : [lint.message];
    return (
      <ModalShell
        open
        onOpenChange={handleOpenChange}
        size="md"
        purpose="form"
        scrollBody
        title={copy.lintTitle(app.title)}
        // The consequence is the subtitle, visible at the moment of decision (principle 2); what
        // to do about it is advice, so it waits behind the glyph.
        titleInfo={<InfoTip label={copy.problemsFound} help={copy.lintHelp} />}
        subtitle={copy.lintSubtitle}
        footer={
          <>
            <Button variant="secondary" onClick={onClose}>
              {copy.cancel}
            </Button>
            <Button onClick={() => void run(lint.request)} disabled={running}>
              {running ? copy.exporting : copy.exportAnyway}
            </Button>
          </>
        }
      >
        {/* One warning Note holds the list. Its own ceiling (eight lines, then "Show more")
            replaces the hand-rolled "N more" that left the rest of the findings unreadable, and
            the shell's body is the one scroller: no box scrolls inside the dialog. */}
        <div className="py-4">
          <Note tone="warning" icon={AlertTriangle} testId="export-lint-findings">
            <ul
              ref={findingsRef}
              tabIndex={-1}
              aria-label={copy.problemsFound}
              className="flex flex-col gap-1 outline-none"
            >
              {findings.map((finding, index) => (
                <li key={index} className="break-words">
                  {finding}
                </li>
              ))}
            </ul>
          </Note>
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
      scrollBody
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
      <div className="flex flex-col gap-4 py-4">
        <div role="radiogroup" aria-label={copy.mode} className="flex flex-col">
          <CustomRadio
            id="export-mode-launcher"
            name="export-mode"
            value="launcher"
            checked={mode === 'launcher'}
            disabled={running}
            onChange={() => setMode('launcher')}
            label={copy.launcher}
            secondaryLabel={copy.launcherHint}
          />
          <CustomRadio
            id="export-mode-full"
            name="export-mode"
            value="full"
            checked={mode === 'full'}
            disabled={running}
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
            {/* An empty-store line: the in-place prose block, as Settings' are. */}
            {knowledgeBases.length === 0 && skills.length === 0 && extensions.length === 0 && (
              <Note testId="export-nothing-to-include">{copy.nothingToInclude}</Note>
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
