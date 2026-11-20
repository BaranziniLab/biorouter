import { useState } from 'react';
import type { ChromeImportSummary } from '../../../browser/types';
import { isBrowserSurface } from '../../../utils/surface';
import { ChromeImportDialog } from '../../browser/chromeImport/ChromeImportDialog';
import {
  CHROME_IMPORT_COPY as COPY,
  summaryDetails,
  summaryLine,
} from '../../browser/chromeImport/copy';
import {
  chromeImportBridge,
  refreshChromeImportStatus,
  useChromeImportStatus,
} from '../../browser/chromeImport/useChromeImportStatus';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import { SETTINGS_SECTION_IDS } from '../settingsSections';

/**
 * Settings > App > Browser (BB-21, BB-25): one line per Chrome import, what it brought in the
 * line's InfoTip, with **Remove imported data**, and **Import from Chrome…**. Desktop only: a
 * `biorouter serve` page has no Biorouter Browser, and main answers the import only to the app's
 * own windows.
 */
export default function BrowserSettingsSection() {
  const status = useChromeImportStatus();
  const [open, setOpen] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  // L4: what Remove imported data would take, waiting for the person's yes.
  const [confirmRemove, setConfirmRemove] = useState<ChromeImportSummary | null>(null);

  if (isBrowserSurface() || !chromeImportBridge()) return null;

  const remove = async (summary: ChromeImportSummary) => {
    const bridge = chromeImportBridge();
    if (!bridge) return;
    setRemoving(summary.id);
    try {
      await bridge.remove(summary.id);
      await refreshChromeImportStatus();
    } finally {
      setRemoving(null);
    }
  };

  return (
    <SettingSection
      id={SETTINGS_SECTION_IDS.browser}
      title={COPY.section}
      help={COPY.sectionHelp}
      data-testid="settings-browser"
    >
      {(status?.imports ?? []).map((summary) => (
        <SettingRow
          key={summary.id}
          label={summaryLine(summary)}
          help={summaryDetails(summary)}
          data-testid="settings-browser-import"
        >
          <Button
            type="button"
            variant="secondary"
            size="sm"
            aria-label={COPY.removeNamed(summary.profileName)}
            disabled={removing !== null}
            onClick={() => setConfirmRemove(summary)}
          >
            {removing === summary.id ? COPY.removing : COPY.remove}
          </Button>
        </SettingRow>
      ))}
      <SettingRow
        label={COPY.importRow}
        help={status && !status.available ? COPY.notFound : COPY.importRowHelp}
      >
        <Button
          type="button"
          variant="secondary"
          size="sm"
          aria-label={COPY.open}
          disabled={!status?.available}
          onClick={() => setOpen(true)}
          data-testid="settings-browser-import-open"
        >
          {COPY.open}
        </Button>
      </SettingRow>
      <ChromeImportDialog open={open} onOpenChange={setOpen} />
      <ConfirmationModal
        isOpen={confirmRemove !== null}
        title={COPY.confirmRemoveTitle(confirmRemove?.profileName ?? '')}
        message={COPY.confirmRemoveMessage}
        confirmLabel={COPY.confirmRemove}
        cancelLabel={COPY.cancel}
        // Not red: it takes only what the import brought, and importing again restores it.
        confirmVariant="default"
        onConfirm={() => {
          const summary = confirmRemove;
          setConfirmRemove(null);
          if (summary) void remove(summary);
        }}
        onCancel={() => setConfirmRemove(null)}
      />
    </SettingSection>
  );
}
