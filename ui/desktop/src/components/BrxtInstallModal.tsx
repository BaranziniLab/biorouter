// ui/desktop/src/components/BrxtInstallModal.tsx
import { useState, useCallback, useRef, useEffect } from 'react';
import { Package } from './icons/app-icons';
import { ModalShell } from './ModalShell';
import { Button } from './ui/button';
import { Disclosure } from './ui/disclosure';
import { Field } from './ui/field';
import { InfoTip } from './ui/info-tip';
import { Input } from './ui/input';
import { PrivacyBadge } from './ui/PrivacyBadge';
import { Spinner } from './ui/spinner';
import { BrxtEnvVar, BrxtManifest } from '../types/brxt';
import { useConfig } from './ConfigContext';
import { activateExtensionDefault } from './settings/extensions';
import { classifyExtension } from './settings/extensions/extensionPrivacy';
import { upsertConfig } from '../api';
import { userActionHeaders } from '../utils/userAction';
import { toastService } from '../toasts';
import { DependencyErrorBanner } from './DependencyErrorBanner';
import {
  marketplacePhase,
  primaryInstallAction,
  resultingPrivacyTier,
  type BrxtInstallOrigin,
} from './brxtInstallFlow';
import { BRXT_INSTALL_COPY as COPY } from './brxtInstallCopy';
import './brxt-install.css';

interface EnvEntry {
  key: string;
  value: string;
  secret: boolean;
  required: boolean;
  description: string;
  auto_propagate: boolean;
}

/**
 * Issue #116. The two entry modes share this component's validation and install
 * internals and nothing else, so the first step is named for what it *is* in
 * each — picking a file, or reviewing the extension the user already picked —
 * rather than for the local route's drop zone.
 */
type Step = 'select' | 'configure';

/**
 * A label has to point at its input. These fields carry a credential and are
 * the one part of this modal a screen-reader user must be able to navigate
 * unambiguously, and an unassociated `<label>` gives them nothing to land on.
 */
const envFieldId = (key: string) => `brxt-env-${key}`;

interface Props {
  onClose: () => void;
  onInstalled: () => void;
  preloadedFilePath?: string;
  /**
   * Where this install came from. Defaults to the local-file route.
   *
   * ⚠ **Not derivable from `preloadedFilePath`.** Finder hands a double-clicked
   * `.brxt` to the app over IPC and `ExtensionsView` preloads its path, so a
   * preloaded path means "a file is already chosen", never "this came from the
   * marketplace". See `brxtInstallFlow.ts`.
   */
  origin?: BrxtInstallOrigin;
}

const NO_LOCAL_PATH_MESSAGE = COPY.noLocalPath;

export function BrxtInstallModal({ onClose, onInstalled, preloadedFilePath, origin }: Props) {
  const installOrigin: BrxtInstallOrigin = origin ?? { kind: 'local-file' };
  const isMarketplace = installOrigin.kind === 'marketplace';
  const registrySource =
    installOrigin.kind === 'marketplace' ? installOrigin.registrySource : undefined;

  const [step, setStep] = useState<Step>('select');
  const [filePath, setFilePath] = useState<string | null>(null);
  const [manifest, setManifest] = useState<BrxtManifest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [envEntries, setEnvEntries] = useState<EnvEntry[]>([]);
  const [skillsPreview, setSkillsPreview] = useState<
    Array<{ slug: string; name: string; description: string }>
  >([]);
  const [showOptional, setShowOptional] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { addExtension } = useConfig();

  const processFile = useCallback(async (fp: string) => {
    // An empty path means the surface could not supply one -- a browser tab
    // cannot know where a dropped file lives, and the daemon that would open it
    // is on another machine. Refusing here is the point: passing the bare file
    // name on would make the daemon resolve it against ITS working directory
    // and possibly validate some unrelated bundle.
    if (!fp) {
      setError(NO_LOCAL_PATH_MESSAGE);
      return;
    }
    setError(null);
    setManifest(null);
    setSkillsPreview([]);
    setIsValidating(true);
    setFilePath(fp);

    try {
      const result = await window.electron.validateBrxtBundle(fp);
      if ('error' in result) {
        setError(result.error);
      } else {
        setManifest(result.manifest);
        setSkillsPreview(result.skillsPreview);
        setEnvEntries(
          result.manifest.env_vars.map((v: BrxtEnvVar) => ({
            key: v.key,
            value: v.auto_propagate && v.default ? v.default : '',
            secret: v.secret,
            required: v.required,
            description: v.description,
            auto_propagate: v.auto_propagate,
          }))
        );
      }
    } catch (error) {
      setError(error instanceof Error ? error.message : COPY.readFailed);
    } finally {
      setIsValidating(false);
    }
  }, []);

  useEffect(() => {
    if (preloadedFilePath) {
      processFile(preloadedFilePath);
    }
  }, [preloadedFilePath, processFile]);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragging(false);
      // A bundle is already being read: the zone stays on screen (and keeps focus) but is not
      // taking another one.
      if (isValidating) return;
      const file = e.dataTransfer.files[0];
      if (!file) return;
      if (!file.name.endsWith('.brxt')) {
        setError(COPY.dropBrxt);
        return;
      }
      processFile(window.electron.getPathForFile(file));
    },
    [processFile, isValidating]
  );

  const handleFileInput = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (file) processFile(window.electron.getPathForFile(file));
      // reset so same file can be re-selected after an error
      e.target.value = '';
    },
    [processFile]
  );

  const setEnvValue = (key: string, value: string) =>
    setEnvEntries((prev) => prev.map((e) => (e.key === key ? { ...e, value } : e)));

  /**
   * Issue #116. One decision, read by both the button's label and its click, so
   * a screen that says "Install extension" cannot open a configuration step and
   * a screen that says "Next: configure" cannot install.
   */
  const primaryAction = primaryInstallAction(manifest);

  const handleNext = () => {
    if (primaryAction.kind === 'install') {
      handleInstall();
    } else {
      setStep('configure');
    }
  };

  const handleInstall = async () => {
    if (!filePath || !manifest || isInstalling) return;
    setIsInstalling(true);
    setError(null);

    try {
      const result = await window.electron.installBrxtBundle(
        filePath,
        manifest.name,
        registrySource
      );

      if ('error' in result) {
        setError(result.error);
        return;
      }

      const { installDir } = result;

      // Store secrets in Biorouter's keyring; track which ones succeeded
      const secretEnvKeys: string[] = [];
      for (const entry of envEntries.filter((e) => e.secret && e.value.trim())) {
        const res = await upsertConfig({
          body: { is_secret: true, key: entry.key, value: entry.value },
          // Issue #56 DR-16: the key here is whatever the extension's manifest
          // declared, so it can collide with a capability key — an extension
          // that talks to Ollama would plausibly declare OLLAMA_HOST. The guard
          // is on the key name and does not look at `is_secret`, so without this
          // the user clicking Install would be refused as though a model had
          // made the call. This IS the user, so it carries the proof.
          headers: await userActionHeaders(),
        }).catch(() => null);
        if (res && !res.error) {
          secretEnvKeys.push(entry.key);
        }
      }

      // Non-secret values go in envs; secrets that failed keyring storage are fallback-included
      const envs: Record<string, string> = {};
      envEntries.forEach(({ key, value, secret }) => {
        if (!value.trim()) return;
        if (!secret || !secretEnvKeys.includes(key)) envs[key] = value;
      });

      const extensionConfig = {
        name: manifest.name,
        description: manifest.description,
        type: 'stdio' as const,
        cmd: 'uv',
        args: ['run', '--directory', installDir, manifest.entry_point],
        envs,
        env_keys: secretEnvKeys,
        timeout: 300,
      };

      try {
        await activateExtensionDefault({ addToConfig: addExtension, extensionConfig });
      } catch {
        setError(COPY.registerFailed);
        return;
      }
      toastService.success({
        title: manifest.display_name,
        msg: COPY.installedToast,
      });
      onInstalled();
      onClose();
    } catch (error) {
      setError(error instanceof Error ? error.message : COPY.installFailed);
    } finally {
      setIsInstalling(false);
    }
  };

  /**
   * Issue #56 §13.5. The badge this install is going to produce, said out loud
   * before the user commits.
   *
   * ⚠ **The disclosure is about the RESULT, not about the route.** §13.5's
   * sentence is written for the file-drop case and is true there — an extension
   * installed from a file is Public under R11(ii), because the install records
   * no provenance for the daemon to treat as private. But this component is not
   * only the file-drop case:
   *
   *   - `BrowseExtensionsModal` renders THIS component for a marketplace
   *     install, so a row badged Private led straight into a confirmation that
   *     said "always Public";
   *   - and the task's own Step 3 records that a bundle merely NAMED
   *     `ucsfomopagent` inherits the private badge — "fail-closed, and fine",
   *     which it only is if the last screen before Install did not promise the
   *     opposite.
   *
   * Three screens with two answers is worse than either answer alone, so once a
   * manifest is in hand the modal states the tier the install will actually
   * produce, resolved through `classifyExtension` — the same union the Settings
   * card, the Browse row and the composer all read, so they cannot disagree.
   *
   * Issue #116 adds the marketplace's own answer to the same union, for the
   * window where the bundle is still downloading and there is no manifest to
   * classify: the row the user clicked already rendered a badge, and the modal
   * that opens on top of it must not contradict the row it came from. Private
   * from either source wins — see `resultingPrivacyTier`.
   *
   * Before a file is chosen on the LOCAL route there is no name to classify and
   * the route IS the only fact available, so §13.5's sentence stands verbatim:
   * a user who has not yet picked a bundle should already know what dropping
   * one in here means.
   *
   * One sentence, one element: the assertions match on the normalised text of a
   * single node, and splitting a phrase into a nested `<strong>` would take it
   * out of that node.
   */
  const resultingTier = resultingPrivacyTier(
    manifest ? classifyExtension(manifest.name) : null,
    installOrigin.kind === 'marketplace' ? installOrigin.entry.privacyTier : undefined
  );
  const publicNotice = isMarketplace ? COPY.tierPublicMarketplace : COPY.tierPublicFile;
  const badgeNotice = resultingTier === 'private' ? COPY.tierPrivate : publicNotice;

  const requiredVars = envEntries.filter((e) => e.required);
  const optionalVars = envEntries.filter((e) => !e.required);
  const requiredMissing = requiredVars.some((e) => !e.value.trim());

  /**
   * Issue #116. The marketplace route's own progress. `downloading` is the
   * parent's — `BrowseExtensionsModal` opens this modal the instant Add is
   * clicked, so the download runs *inside* the install rather than in front of
   * it, and a failure lands on a screen that can offer Retry.
   */
  const phase = marketplacePhase({
    downloading: installOrigin.kind === 'marketplace' ? installOrigin.downloading : false,
    downloadError: installOrigin.kind === 'marketplace' ? installOrigin.downloadError : null,
    isValidating,
    validationError: error,
    hasManifest: manifest !== null,
  });
  const downloadError =
    installOrigin.kind === 'marketplace' ? (installOrigin.downloadError ?? null) : null;
  const isBusy =
    isValidating ||
    isInstalling ||
    (installOrigin.kind === 'marketplace' && installOrigin.downloading);

  const marketplaceName = isMarketplace
    ? (manifest?.display_name ??
      (installOrigin.kind === 'marketplace' ? installOrigin.entry.name : ''))
    : '';
  const marketplaceMeta =
    installOrigin.kind === 'marketplace'
      ? [installOrigin.entry.organization, manifest ? `v${manifest.version}` : undefined]
          .filter(Boolean)
          .join(' · ')
      : '';

  // §13.5: the badge this install will produce and the one sentence that says what it means,
  // as text and not as help. No panel around them.
  const privacyLine = (
    <div className="br-brxt-tier">
      <PrivacyBadge tier={resultingTier} />
      <p className="text-supporting text-text-muted">{badgeNotice}</p>
    </div>
  );

  const manifestSummary = manifest && !error && (
    <section className="br-brxt-summary">
      <p className="text-supporting text-text-muted">
        {isMarketplace ? COPY.fromMarketplace : COPY.detectedFromBundle}
      </p>
      <p className="text-label text-text-default">{manifest.display_name}</p>
      <p className="text-supporting text-text-muted">
        v{manifest.version}
        {manifest.tools_count ? ` · ${COPY.tools(manifest.tools_count)}` : ''}
        {skillsPreview.length > 0 ? ` · ${COPY.skills(skillsPreview.length)}` : ''}
        {' · '}
        {COPY.requiredVars(requiredVars.length, optionalVars.length)}
      </p>
      <p className="br-brxt-summary-desc text-body text-text-default">{manifest.description}</p>
      {skillsPreview.length > 0 && (
        <div className="br-brxt-skills">
          <p className="text-supporting text-text-muted">{COPY.skillsIncluded}</p>
          {skillsPreview.map((skill) => (
            <p key={skill.slug} className="text-supporting text-text-muted">
              · <span className="text-text-default">{skill.name}</span>: {skill.description}
            </p>
          ))}
        </div>
      )}
    </section>
  );

  const dependencyBanner = error && (
    <DependencyErrorBanner
      error={error}
      failure={{
        kind: 'extension',
        name: manifest?.name ?? 'extension',
        displayName: manifest?.display_name ?? manifest?.name,
        command: 'uv sync',
      }}
    />
  );

  // The titles say what the dialog is doing; the second line, where there is one, is a fact (where
  // it came from), never an instruction. "Install a Biorouter extension bundle (.brxt file)" and
  // "Fill in required credentials…" repeated what the drop zone and the asterisks already say.
  let title: React.ReactNode;
  let subtitle: React.ReactNode;
  if (step === 'configure') {
    title = COPY.configureTitle(manifest?.display_name ?? '');
  } else if (isMarketplace) {
    title = COPY.installTitle(marketplaceName);
    // Who published it and which version: facts. Where it came from is said once, by the
    // summary's own first line (and by the tier sentence above it), not again under the title.
    subtitle = marketplaceMeta || undefined;
  } else {
    title = COPY.addTitle;
  }

  /**
   * Issue #116. On the marketplace route Cancel, Back and the × are all one
   * thing — returning to the list the user came from — so there is one control
   * and it says so. Offering a second "Cancel" beside it would imply the two
   * differ.
   */
  const backToMarketplace = (
    <Button variant="secondary" onClick={onClose} disabled={isInstalling}>
      {COPY.backToMarketplace}
    </Button>
  );

  let footer: React.ReactNode;
  if (step === 'select' && isMarketplace) {
    footer = (
      <>
        {backToMarketplace}
        {phase === 'error' ? (
          <Button
            disabled={isBusy}
            onClick={() => {
              setError(null);
              if (downloadError) {
                if (installOrigin.kind === 'marketplace') installOrigin.onRetry();
              } else if (preloadedFilePath) {
                processFile(preloadedFilePath);
              }
            }}
          >
            {COPY.retry}
          </Button>
        ) : (
          <Button disabled={!manifest || isBusy} onClick={handleNext}>
            {isInstalling ? COPY.installing : primaryAction.label}
          </Button>
        )}
      </>
    );
  } else if (step === 'select') {
    footer = (
      <>
        <Button variant="secondary" onClick={onClose} disabled={isBusy}>
          {COPY.cancel}
        </Button>
        <Button disabled={!manifest || !!error || isBusy} onClick={handleNext}>
          {isInstalling ? COPY.installing : primaryAction.label}
        </Button>
      </>
    );
  } else {
    footer = (
      <>
        <Button
          variant="secondary"
          disabled={isBusy}
          onClick={() => {
            setStep('select');
            setError(null);
          }}
        >
          {COPY.back}
        </Button>
        {isMarketplace ? (
          backToMarketplace
        ) : (
          <Button variant="secondary" onClick={onClose} disabled={isBusy}>
            {COPY.cancel}
          </Button>
        )}
        <Button disabled={requiredMissing || isInstalling} onClick={handleInstall}>
          {isInstalling ? COPY.installing : COPY.installExtension}
        </Button>
      </>
    );
  }

  /** One credential field: a real label, a masked input for a secret, the manifest's own words as the hint. */
  const envField = (entry: EnvEntry) => (
    <Field
      key={entry.key}
      id={envFieldId(entry.key)}
      label={
        entry.required ? (
          <>
            {entry.key}{' '}
            <span className="br-brxt-required-mark" aria-hidden="true">
              {COPY.requiredMark}
            </span>
          </>
        ) : (
          entry.key
        )
      }
      required={entry.required}
    >
      <Input
        type={entry.secret ? 'password' : 'text'}
        // The manifest's description of the variable is the hint inside the empty field: third-party
        // text of any length, so not a line under every field and not a second labelled control.
        placeholder={entry.description}
        value={entry.value}
        onChange={(e) => setEnvValue(entry.key, e.target.value)}
        autoComplete="off"
      />
    </Field>
  );

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      // A form: Escape and the × still leave, but a misclick on the scrim can
      // no longer throw away typed credentials. While a bundle is being read or
      // installed it is `required` — nothing may interrupt it.
      size="md"
      purpose={isBusy ? 'required' : 'form'}
      // The body changes shape as the person goes (a drop zone, a summary, a form, an error
      // banner); anchored at the top it grows downward, so the controls stay under the pointer.
      anchor="top"
      scrollBody
      title={title}
      subtitle={subtitle}
      footer={footer}
      bodyClassName="flex flex-col gap-4 py-4"
    >
      {step === 'select' && isMarketplace && (
        /* Issue #116. The marketplace route renders NO local-file controls: no
           drop zone, no file input, no way to pick a file. The user chose this
           extension on the previous screen and Biorouter is fetching it — there
           is nothing for them to supply. */
        <>
          {privacyLine}

          {phase === 'downloading' && (
            <p role="status" className="br-brxt-status text-body text-text-muted">
              <Spinner />
              {COPY.downloading(marketplaceName)}
            </p>
          )}

          {phase === 'validating' && (
            <p role="status" className="br-brxt-status text-body text-text-muted">
              <Spinner />
              {COPY.reading}
            </p>
          )}

          {phase === 'error' &&
            (downloadError ? (
              <DependencyErrorBanner
                error={downloadError}
                hideDebugAction
                failure={{
                  kind: 'extension',
                  name:
                    installOrigin.kind === 'marketplace' ? installOrigin.entry.name : 'extension',
                  command: 'download',
                }}
              />
            ) : (
              dependencyBanner
            ))}

          {phase === 'ready' && manifestSummary}
        </>
      )}

      {step === 'select' && !isMarketplace && (
        <>
          {/* §13.5: the tier this install will actually produce, stated before
              a bundle has even been chosen — someone who has not yet picked one
              should already know what dropping it in here means. */}
          {privacyLine}

          {/* Knowledge's drop zone recipe (`knowledge/IngestPanel/Dropzone.tsx`), as Add skill
              draws it: a glyph plate over one label line on the muted ground, and a real button,
              so the keyboard reaches the file chooser it opens. Hover is `tint-interactive`,
              never `hover:bg-overlay-hover`, which would REPLACE an opaque ground rather than
              composite over it. Not `biorouter-modal-panel`: that class is unlayered, so its
              background and border beat any utility and the zone never changed on drag or error. */}
          <button
            type="button"
            // Not `disabled` while a bundle is read: a disabled button cannot hold focus, and a
            // dialog opened with a file already chosen (Finder hands one over) then had nowhere
            // to put first focus but the dialog's own box, drawn with the browser's blue ring.
            // The label says what the zone is doing, so it still contains the words on screen.
            aria-label={
              isValidating ? COPY.reading : manifest ? COPY.dropZoneAnotherName : COPY.dropZoneName
            }
            aria-disabled={isValidating || undefined}
            onDragOver={(e) => {
              e.preventDefault();
              setIsDragging(true);
            }}
            onDragLeave={() => setIsDragging(false)}
            onDrop={handleDrop}
            onClick={async () => {
              if (isValidating) return;
              // The native picker returns a real path (and the Playwright hook can answer it
              // without a dialog); a surface without it falls back to the file input.
              if (typeof window.electron?.openBrxtFilePicker !== 'function') {
                fileInputRef.current?.click();
                return;
              }
              const fp = await window.electron.openBrxtFilePicker();
              if (fp) processFile(fp);
            }}
            className={[
              'biorouter-focus-surface flex w-full select-none flex-col items-center gap-2 rounded-container border px-4 py-6 text-center transition-colors',
              isValidating ? 'cursor-progress' : 'cursor-pointer',
              isDragging
                ? 'border-border-strong bg-background-medium'
                : error
                  ? 'border-border-danger bg-background-muted tint-interactive'
                  : 'border-border-subtle bg-background-muted tint-interactive',
            ].join(' ')}
          >
            {isValidating ? (
              <span className="flex items-center gap-2 text-body text-text-muted">
                <Spinner />
                {COPY.reading}
              </span>
            ) : (
              <>
                <span
                  aria-hidden="true"
                  className={`flex h-8 w-8 items-center justify-center rounded-element border border-border-subtle ${isDragging ? 'bg-background-strong text-text-default' : 'bg-background-muted text-text-muted'}`}
                >
                  <Package className="h-4 w-4" />
                </span>
                <span className="text-label text-text-default">
                  {manifest ? COPY.dropZoneAnother : COPY.dropZone}
                </span>
              </>
            )}
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept=".brxt"
            className="hidden"
            onChange={handleFileInput}
          />

          {/* Error banner. A `.brxt` install fails most often on `uv sync` —
              a Python dependency that will not build on this machine — which is
              precisely the kind of thing a session with a shell can work out. */}
          {dependencyBanner}

          {/* What the bundle says it is */}
          {manifestSummary}
        </>
      )}

      {step === 'configure' && manifest && (
        <>
          <div className="br-brxt-fields">
            {requiredVars.map(envField)}

            {/* Issue #116. When an extension declares only optional variables the
                first step's button says so, and this step must not then hide them
                behind a disclosure the user has to find. They are open, under a
                title whose help says they can be left as they are. */}
            {optionalVars.length > 0 && requiredVars.length === 0 && (
              <>
                <div className="br-brxt-group-title">
                  <h3 className="text-label text-text-default">{COPY.optionalSettings}</h3>
                  <InfoTip
                    label={COPY.optionalSettings}
                    help={COPY.optionalHelp(manifest.display_name, optionalVars.length)}
                  />
                </div>
                {optionalVars.map(envField)}
              </>
            )}

            {optionalVars.length > 0 && requiredVars.length > 0 && (
              <Disclosure
                label={COPY.optionalVariables(optionalVars.length)}
                summary={COPY.prefilled}
                open={showOptional}
                onOpenChange={setShowOptional}
              >
                <div className="br-brxt-fields">{optionalVars.map(envField)}</div>
              </Disclosure>
            )}
          </div>

          {dependencyBanner}

          {/* §13.5: the resulting badge, above the Install button — the footer
              sits directly below this body, so it is the last thing read before
              the button that commits the install. */}
          {privacyLine}
        </>
      )}
    </ModalShell>
  );
}
