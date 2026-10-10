import { useCallback, useEffect, useMemo, useState } from 'react';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { BrxtInstallModal } from '../BrxtInstallModal';
import {
  loadRegistry,
  rankExtensions,
  effectivePrivacy,
  type BaamRegistry,
  type RegistryExtension,
} from './registry';
import { MarketplaceDialog, MarketplaceRow } from './MarketplaceDialog';
import { BROWSE_EXTENSIONS_COPY, MARKETPLACE_COPY } from './copy';
import { PrivacyBadge } from '../ui/PrivacyBadge';
import { readRegistryDownload } from '../../utils/registryDownloadResult';

interface Props {
  onClose: () => void;
  onInstalled: () => void;
  /** Lowercased names of extensions already configured. */
  installedNames: Set<string>;
  /**
   * Issue #116. Open an already-installed extension's configuration. Optional:
   * a surface that has nowhere to send the user keeps the inert "Installed"
   * badge rather than offering a control that goes nowhere.
   */
  onConfigureInstalled?: (extension: RegistryExtension) => void;
}

/**
 * Issue #116. The marketplace install, as this modal owns it.
 *
 * The download used to run *before* the installer opened, which is why a
 * failure could only become a toast and why the installer was handed a path
 * with no memory of where it came from. Opening the installer first — with the
 * registry entry, and `downloading: true` — puts the whole marketplace install
 * on one surface: progress, failure, Retry, and Back to this list.
 */
interface PendingInstall {
  entry: RegistryExtension;
  path?: string;
  error?: string;
  downloading: boolean;
}

export default function BrowseExtensionsModal({
  onClose,
  onInstalled,
  installedNames,
  onConfigureInstalled,
}: Props) {
  const [registry, setRegistry] = useState<BaamRegistry | null>(null);
  const [live, setLive] = useState(false);
  const [fetchedAt, setFetchedAt] = useState<string | undefined>(undefined);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState('');
  const [pending, setPending] = useState<PendingInstall | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadRegistry()
      .then(({ registry, live, fetchedAt }) => {
        if (cancelled) return;
        setRegistry(registry);
        setLive(live);
        setFetchedAt(fetchedAt);
      })
      .catch(() => !cancelled && setLoadError(true));
    return () => {
      cancelled = true;
    };
  }, []);

  const isInstalled = (e: RegistryExtension) =>
    installedNames.has(e.name.toLowerCase()) || installedNames.has(e.id.toLowerCase());

  /** Best match first under a query; registry order when there is none. */
  const filtered = useMemo(() => {
    if (!registry) return [];
    return rankExtensions(registry.extensions, search).hits.map((hit) => hit.entry);
  }, [registry, search]);

  /**
   * Fetch the bundle for an entry the installer is already showing. Every exit
   * clears `downloading`, so the installer can never be left claiming progress
   * that stopped.
   */
  const download = useCallback(async (ext: RegistryExtension) => {
    try {
      // Read by value, not by key: `biorouter serve` answers with both keys and
      // one of them null. See `utils/registryDownloadResult`.
      const dl = readRegistryDownload(
        await window.electron.downloadRegistryAsset(ext.download),
        `Could not download ${ext.name}`
      );
      setPending((prev) => {
        // A Back-to-marketplace click during the download wins: resolving into
        // a cleared (or re-targeted) slot would reopen an installer the user
        // just dismissed.
        if (!prev || prev.entry.id !== ext.id) return prev;
        return 'error' in dl
          ? { ...prev, downloading: false, error: dl.error }
          : { ...prev, downloading: false, path: dl.path, error: undefined };
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Download failed';
      setPending((prev) =>
        prev && prev.entry.id === ext.id ? { ...prev, downloading: false, error: message } : prev
      );
    }
  }, []);

  const handleAdd = (ext: RegistryExtension) => {
    if (pending) return;
    setPending({ entry: ext, downloading: true });
    void download(ext);
  };

  const handleRetry = useCallback(() => {
    setPending((prev) => (prev ? { ...prev, downloading: true, error: undefined } : prev));
    if (pending) void download(pending.entry);
  }, [download, pending]);

  const closePending = useCallback(() => setPending(null), []);

  // While a marketplace install is in flight, show it on top of the browse
  // list. One extension is installed at a time.
  if (pending) {
    return (
      <BrxtInstallModal
        preloadedFilePath={pending.path}
        origin={{
          kind: 'marketplace',
          // Issue #56 Task 43 (DR-23): the registry `id` exists only here, and
          // the install that has to record it happens one component away.
          registrySource: { registryId: pending.entry.id, sourceUrl: pending.entry.download },
          entry: {
            name: pending.entry.name,
            organization: pending.entry.organization,
            version: pending.entry.version,
            description: pending.entry.description,
            // The badge this row rendered. The installer must not contradict
            // the row the user clicked while the bundle is still downloading
            // and there is no manifest to classify.
            privacyTier: registry
              ? effectivePrivacy(registry, pending.entry.extension_name ?? pending.entry.name)
              : undefined,
          },
          downloading: pending.downloading,
          downloadError: pending.error ?? null,
          onRetry: handleRetry,
        }}
        onClose={closePending}
        onInstalled={() => {
          closePending();
          onInstalled();
          onClose();
        }}
      />
    );
  }

  return (
    <MarketplaceDialog
      title={BROWSE_EXTENSIONS_COPY.title}
      help={BROWSE_EXTENSIONS_COPY.help}
      live={live}
      fetchedAt={fetchedAt}
      search={search}
      onSearchChange={setSearch}
      searchLabel={BROWSE_EXTENSIONS_COPY.searchLabel}
      status={loadError ? 'error' : registry ? 'ready' : 'loading'}
      empty={filtered.length === 0}
      emptyText={BROWSE_EXTENSIONS_COPY.empty}
      onClose={onClose}
    >
      <div className="biorouter-list-shell">
        {filtered.map((ext) => {
          const installed = isInstalled(ext);
          return (
            <MarketplaceRow
              key={ext.id}
              title={ext.name}
              badges={
                <>
                  {/* The union rule, not `ext.privacy`: a downgraded document
                      must not be able to un-badge an entry here either. */}
                  {registry && (
                    <PrivacyBadge
                      tier={effectivePrivacy(registry, ext.extension_name ?? ext.name)}
                    />
                  )}
                  {installed && <Badge tone="neutral">{MARKETPLACE_COPY.installed}</Badge>}
                </>
              }
              meta={[ext.organization, ext.version].filter(Boolean).join(' · ')}
              description={ext.description}
              trailing={
                installed ? (
                  // Issue #116: an installed row is a destination, not a dead
                  // end — its credentials are the thing a user most often comes
                  // back here to change.
                  onConfigureInstalled && (
                    <Button size="sm" variant="secondary" onClick={() => onConfigureInstalled(ext)}>
                      {BROWSE_EXTENSIONS_COPY.configure}
                    </Button>
                  )
                ) : (
                  <Button size="sm" variant="secondary" onClick={() => handleAdd(ext)}>
                    {BROWSE_EXTENSIONS_COPY.add}
                  </Button>
                )
              }
            />
          );
        })}
      </div>
    </MarketplaceDialog>
  );
}
