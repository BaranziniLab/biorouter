import { useState } from 'react';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import CustomRadio from '../ui/CustomRadio';
import { InfoTip } from '../ui/info-tip';
import { ModalShell } from '../ModalShell';
import type { AppManifest, ExportMode, ExportOptions } from './appManagement';
import { appsCopy } from './copy';

const copy = appsCopy.exportDialog;

interface ExportAppDialogProps {
  app: AppManifest;
  onConfirm: (options: ExportOptions) => void;
  onCancel: () => void;
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
}: {
  id: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  name: string;
}) {
  return (
    <label htmlFor={id} className="flex min-h-8 min-w-0 cursor-pointer items-center gap-2">
      <Checkbox id={id} checked={checked} onChange={(event) => onChange(event.target.checked)} />
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
 * The Built apps export dialog (Apps SDK v2, design §3.9): launcher or full export, with the
 * items the app's agent references pre-checked. Confirm hands the chosen options back to the
 * caller, which performs the request; Cancel sends nothing.
 *
 * `ModalShell size="md"` (the forms rung). Radio rows appear here because a dialog is the one
 * place principle 5 allows them, each with one consequence line. "Credentials are never
 * included." stays visible: a safety fact is never hidden behind help.
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
    <ModalShell
      open
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      size="md"
      purpose="form"
      title={copy.title(app.title)}
      subtitle={copy.credentials}
      footer={
        <>
          <Button variant="secondary" onClick={onCancel}>
            {copy.cancel}
          </Button>
          <Button onClick={confirm}>{copy.export}</Button>
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
            {kb && (
              <IncludeGroup title={copy.knowledgeBase} info={copy.knowledgeBaseInfo}>
                <IncludeRow
                  id="export-include-kb"
                  checked={includeKb}
                  onChange={setIncludeKb}
                  name={kb}
                />
              </IncludeGroup>
            )}
            {skills.length > 0 && (
              <IncludeGroup title={copy.skills} info={copy.skillsInfo}>
                {skills.map((skill) => (
                  <IncludeRow
                    key={skill}
                    id={`export-include-skill-${skill}`}
                    checked={selectedSkills.has(skill)}
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
                    onChange={(checked) =>
                      setSelectedExtensions((prev) => toggle(prev, ext, checked))
                    }
                    name={ext}
                  />
                ))}
              </IncludeGroup>
            )}
            <div className="flex items-center gap-1">
              <IncludeRow
                id="export-bundle-daemon"
                checked={bundleDaemon}
                onChange={setBundleDaemon}
                name={copy.includeBiorouter}
              />
              <InfoTip label={copy.includeBiorouter} help={copy.includeBiorouterInfo} />
            </div>
          </div>
        )}
      </div>
    </ModalShell>
  );
}
