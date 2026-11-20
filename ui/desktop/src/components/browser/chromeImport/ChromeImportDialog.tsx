import { useEffect, useId, useMemo, useState } from 'react';
import type {
  ChromeImportProfile,
  ChromeImportSites,
  ChromeImportSummary,
} from '../../../browser/types';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import CustomRadio from '../../ui/CustomRadio';
import { Disclosure } from '../../ui/disclosure';
import { FilterInput } from '../../ui/filter-input';
import { InfoTip } from '../../ui/info-tip';
import { Note } from '../../ui/note';
import { Spinner } from '../../ui/spinner';
import { CHROME_IMPORT_COPY as COPY, loginsNote, summaryParts } from './copy';
import {
  chromeImportBridge,
  refreshChromeImportStatus,
  useChromeImportStatus,
} from './useChromeImportStatus';

type Choice = 'logins' | 'bookmarks' | 'history' | 'settings';
const CHOICES: Array<{ id: Choice; label: string; help?: string }> = [
  { id: 'logins', label: COPY.logins, help: COPY.loginsHelp },
  { id: 'bookmarks', label: COPY.bookmarks },
  { id: 'history', label: COPY.history, help: COPY.historyHelp },
  { id: 'settings', label: COPY.settings, help: COPY.settingsHelp },
];
const SITE_ROWS_SHOWN = 200;

type Step =
  | { kind: 'choose' }
  | { kind: 'importing' }
  | { kind: 'summary'; summary: ChromeImportSummary }
  | { kind: 'removed' };

/**
 * Import from Chrome (BB-21): choose a profile, what to import and, behind a disclosure, which
 * sites' logins; then the operating system's prompt on macOS (BB-23); then a summary with
 * **Remove imported data** (BB-25).
 *
 * Opened only by the person: from the browser's first use card, its ⋯ menu, or Settings. The
 * dialog talks to main over the `browserImport` bridge, which answers only the app's own windows
 * and returns counts and names, never a value.
 *
 * ⚠ A caller inside the browser panel must hide the native page while this is open: the page is
 * a native view that paints above every DOM layer, this dialog included.
 */
export function ChromeImportDialog({
  open,
  onOpenChange,
  onImported,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** After an import that changed something (a tab reloads to pick up its logins). */
  onImported?: (summary: ChromeImportSummary) => void;
}) {
  const status = useChromeImportStatus();
  const idPrefix = useId();
  const [profiles, setProfiles] = useState<ChromeImportProfile[]>([]);
  const [profileId, setProfileId] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Record<Choice, boolean>>({
    logins: true,
    bookmarks: true,
    history: true,
    settings: true,
  });
  const [plan, setPlan] = useState<ChromeImportSites | null>(null);
  const [selectedSites, setSelectedSites] = useState<Set<string> | null>(null);
  const [filter, setFilter] = useState('');
  const [step, setStep] = useState<Step>({ kind: 'choose' });
  const [error, setError] = useState<string | null>(null);
  // What Remove imported data would take, waiting for the person's yes, as in Settings (L4).
  const [confirmRemove, setConfirmRemove] = useState<ChromeImportSummary | null>(null);
  const loginsSupported = status?.loginsSupported ?? true;

  useEffect(() => {
    if (!open) return;
    setStep({ kind: 'choose' });
    setError(null);
    setConfirmRemove(null);
    setFilter('');
    const bridge = chromeImportBridge();
    if (!bridge) return;
    void refreshChromeImportStatus();
    void bridge.profiles().then((found) => {
      setProfiles(found);
      setProfileId((current) =>
        current && found.some((profile) => profile.id === current)
          ? current
          : (found.find((profile) => profile.lastUsed)?.id ?? found[0]?.id ?? null)
      );
    });
  }, [open]);

  // The site list comes from the unencrypted host column: no key is asked for here.
  useEffect(() => {
    setPlan(null);
    setSelectedSites(null);
    const bridge = chromeImportBridge();
    if (!open || !profileId || !bridge || !loginsSupported) return;
    let cancelled = false;
    void bridge.sites(profileId).then((next) => {
      if (!cancelled) setPlan(next);
    });
    return () => {
      cancelled = true;
    };
  }, [open, profileId, loginsSupported]);

  const allSites = useMemo(
    () => (plan?.readable ? plan.sites.map((site) => site.site) : []),
    [plan]
  );
  const effectiveSites = selectedSites ?? new Set(allSites);
  const visibleSites = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const sites = plan?.readable ? plan.sites : [];
    return (needle ? sites.filter((site) => site.site.includes(needle)) : sites).slice(
      0,
      SITE_ROWS_SHOWN
    );
  }, [plan, filter]);

  const logins = chosen.logins && loginsSupported;
  const anything = logins || chosen.bookmarks || chosen.history || chosen.settings;
  const noSites =
    logins && plan?.readable === true && allSites.length > 0 && effectiveSites.size === 0;
  const busy = step.kind === 'importing';

  const runImport = async () => {
    const bridge = chromeImportBridge();
    if (!bridge || !profileId) return;
    if (!anything) {
      setError(COPY.nothingChosen);
      return;
    }
    if (noSites) {
      setError(COPY.noneSelected);
      return;
    }
    setError(null);
    setStep({ kind: 'importing' });
    const everySite = selectedSites === null || effectiveSites.size === allSites.length;
    const result = await bridge.run({
      profileId,
      logins,
      bookmarks: chosen.bookmarks,
      history: chosen.history,
      settings: chosen.settings,
      sites: logins && !everySite ? [...effectiveSites] : null,
    });
    await refreshChromeImportStatus();
    if (!result.ok) {
      // Cancel in the app's own confirmation is the person's answer, not a failure.
      setError(result.cancelled ? null : result.error);
      setStep({ kind: 'choose' });
      return;
    }
    setStep({ kind: 'summary', summary: result.summary });
    onImported?.(result.summary);
  };

  const removeImport = async (summary: ChromeImportSummary) => {
    const bridge = chromeImportBridge();
    if (!bridge) return;
    setStep({ kind: 'importing' });
    await bridge.remove(summary.id);
    await refreshChromeImportStatus();
    setStep({ kind: 'removed' });
    onImported?.(summary);
  };

  const toggleSite = (site: string) => {
    const next = new Set(effectiveSites);
    if (next.has(site)) next.delete(site);
    else next.add(site);
    setSelectedSites(next);
  };

  const close = () => !busy && onOpenChange(false);

  const footer =
    step.kind === 'summary' ? (
      <>
        <Button type="button" variant="secondary" onClick={() => setConfirmRemove(step.summary)}>
          {COPY.remove}
        </Button>
        <Button type="button" onClick={close} data-testid="chrome-import-done">
          {COPY.done}
        </Button>
      </>
    ) : step.kind === 'removed' ? (
      <Button type="button" onClick={close}>
        {COPY.done}
      </Button>
    ) : (
      <>
        <Button type="button" variant="secondary" disabled={busy} onClick={close}>
          {COPY.cancel}
        </Button>
        <Button
          type="button"
          disabled={busy || !profileId || !anything}
          onClick={() => void runImport()}
          data-testid="chrome-import-run"
        >
          {busy ? COPY.importing : COPY.import}
        </Button>
      </>
    );

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => (next ? onOpenChange(true) : close())}
      size="md"
      purpose={busy ? 'required' : 'form'}
      title={step.kind === 'summary' ? COPY.summaryTitle : COPY.title}
      subtitle={step.kind === 'choose' ? COPY.subtitle : undefined}
      footer={footer}
      scrollBody
      bodyClassName="py-3"
      testId="chrome-import-dialog"
    >
      {step.kind === 'importing' && (
        <div className="flex items-center gap-2 py-2 text-body text-text-muted">
          <Spinner size={16} />
          <span>{COPY.importing}</span>
        </div>
      )}

      {step.kind === 'summary' && (
        <div data-testid="chrome-import-summary" className="flex flex-col gap-2">
          <ul className="flex flex-col gap-1 text-body text-text-default">
            {summaryParts(step.summary).map((part) => (
              <li key={part}>{part[0].toUpperCase() + part.slice(1)}</li>
            ))}
          </ul>
          {loginsNote(step.summary) && <Note tone="neutral">{loginsNote(step.summary)}</Note>}
        </div>
      )}

      {step.kind === 'removed' && <p className="text-body text-text-default">{COPY.removed}</p>}

      {step.kind === 'choose' && (
        <div className="flex flex-col gap-4">
          {profiles.length > 1 && (
            <fieldset className="flex flex-col gap-1">
              <legend className="px-3 pb-1 text-label text-text-muted">{COPY.profile}</legend>
              {profiles.map((profile) => (
                <CustomRadio
                  key={profile.id}
                  id={`${idPrefix}-profile-${profile.id}`}
                  name={`${idPrefix}-profile`}
                  value={profile.id}
                  checked={profileId === profile.id}
                  onChange={() => setProfileId(profile.id)}
                  label={profile.name}
                  secondaryLabel={profile.email ?? undefined}
                  // On the checkbox rows' 12px inset, so radios and boxes share one edge.
                  className="px-3"
                />
              ))}
            </fieldset>
          )}
          {profiles.length === 1 && (
            <p className="px-3 text-body text-text-default">
              {COPY.profile}: {profiles[0].name}
              {profiles[0].email && <span className="text-text-muted"> ({profiles[0].email})</span>}
            </p>
          )}

          <ul className="biorouter-settings-list br-rows-flat">
            {CHOICES.map((choice) => {
              const id = `${idPrefix}-choice-${choice.id}`;
              const disabled = busy || (choice.id === 'logins' && !loginsSupported);
              return (
                <li
                  key={choice.id}
                  // The checkbox row recipe Reset data uses: 40px, the box 16px in.
                  className="biorouter-settings-row br-reset-option flex min-w-0 items-center gap-2"
                  data-testid={`chrome-import-choice-${choice.id}`}
                >
                  <Checkbox
                    id={id}
                    checked={choice.id === 'logins' ? logins : chosen[choice.id]}
                    disabled={disabled}
                    onChange={() =>
                      setChosen((current) => ({ ...current, [choice.id]: !current[choice.id] }))
                    }
                  />
                  <label htmlFor={id} className="min-w-0 truncate text-label text-text-default">
                    {choice.label}
                  </label>
                  {choice.help && <InfoTip label={choice.label} help={choice.help} />}
                </li>
              );
            })}
          </ul>

          {!loginsSupported && <Note tone="neutral">{COPY.windowsLogins}</Note>}

          <p className="flex items-center gap-1 pl-4 text-supporting text-text-muted">
            {COPY.notImported}
            <InfoTip label={COPY.notImportedLabel} help={COPY.notImportedHelp} />
          </p>

          {logins && plan && !plan.readable && <Note tone="neutral">{COPY.sitesUnreadable}</Note>}

          {logins && plan?.readable && allSites.length > 0 && (
            <Disclosure
              label={COPY.sites}
              summary={COPY.sitesSummary(effectiveSites.size, allSites.length)}
            >
              <div className="flex flex-col gap-2 pt-2" data-testid="chrome-import-sites">
                <div className="flex items-center gap-2">
                  <FilterInput
                    value={filter}
                    onValueChange={setFilter}
                    label={COPY.sitesFilter}
                    findShortcut={false}
                    className="min-w-0 flex-1"
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setSelectedSites(new Set(allSites))}
                  >
                    {COPY.selectAll}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setSelectedSites(new Set())}
                  >
                    {COPY.selectNone}
                  </Button>
                </div>
                <ul className="biorouter-settings-list br-rows-flat max-h-56 overflow-y-auto">
                  {visibleSites.map((site) => {
                    const id = `${idPrefix}-site-${site.site}`;
                    return (
                      <li
                        key={site.site}
                        className="biorouter-settings-row br-reset-option flex items-center gap-2"
                      >
                        <Checkbox
                          id={id}
                          checked={effectiveSites.has(site.site)}
                          onChange={() => toggleSite(site.site)}
                        />
                        <label
                          htmlFor={id}
                          className="min-w-0 truncate text-label text-text-default"
                        >
                          {site.site}
                        </label>
                        <span className="ml-auto shrink-0 text-supporting tabular-nums text-text-muted">
                          {COPY.cookieCount(site.count)}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </div>
            </Disclosure>
          )}

          {logins && status?.asksForKeyAccess && (
            <p
              className="pl-4 text-supporting text-text-muted"
              data-testid="chrome-import-mac-prompt"
            >
              {COPY.macPrompt}
            </p>
          )}

          {error && (
            <Note tone="neutral" role="alert">
              {error}
            </Note>
          )}
        </div>
      )}

      {/* A dialog nested in this one, so it stacks above it and Escape closes it first. */}
      <ConfirmationModal
        isOpen={confirmRemove !== null}
        title={COPY.confirmRemoveTitle(confirmRemove?.profileName ?? '')}
        message={COPY.confirmRemoveMessage}
        confirmLabel={COPY.confirmRemove}
        cancelLabel={COPY.cancel}
        // Not red, as in Settings: it takes only what the import brought, and importing again
        // restores it.
        confirmVariant="default"
        onConfirm={() => {
          const summary = confirmRemove;
          setConfirmRemove(null);
          if (summary) void removeImport(summary);
        }}
        onCancel={() => setConfirmRemove(null)}
      />
    </ModalShell>
  );
}
