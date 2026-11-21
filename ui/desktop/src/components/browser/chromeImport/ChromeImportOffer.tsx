import { Button } from '../../ui/button';
import { Globe } from '../../icons/app-icons';
import { CHROME_IMPORT_COPY as COPY } from './copy';
import { chromeImportBridge, refreshChromeImportStatus } from './useChromeImportStatus';

/**
 * The first use card (BB-21): the first time the Biorouter Browser opens a page the person asked
 * for, and Chrome is on this computer, the page slot asks once whether to bring Chrome's logins
 * and bookmarks. **Not now** dismisses it for good; Settings and the ⋯ menu still offer it.
 *
 * It sits in the page slot while the native page is hidden (the caller counts it as covering),
 * on the panel's own preview state style.
 */
export function ChromeImportOffer({
  onImport,
  onNotNow,
}: {
  onImport: () => void;
  onNotNow: () => void;
}) {
  return (
    <div
      data-testid="chrome-import-offer"
      className="br-preview-state absolute inset-0 bg-background-default"
      role="region"
      aria-label={COPY.offerTitle}
    >
      <Globe className="h-5 w-5 text-text-muted" aria-hidden="true" />
      <div className="br-preview-state__line text-body text-text-default">{COPY.offerTitle}</div>
      <div className="mt-2 flex flex-wrap items-center justify-center gap-2">
        <Button type="button" size="sm" onClick={onImport} data-testid="chrome-import-offer-import">
          {COPY.open}
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="chrome-import-offer-not-now"
          onClick={() => {
            onNotNow();
            void chromeImportBridge()
              ?.dismissOffer()
              .then(() => refreshChromeImportStatus());
          }}
        >
          {COPY.notNow}
        </Button>
      </div>
    </div>
  );
}
