import { useCallback, useEffect, useMemo, useState } from 'react';
import { Button } from '../ui/button';
import { Package } from '../icons/app-icons';
import { BrxtInstallModal } from '../BrxtInstallModal';
import {
  loadRegistry,
  rankExtensions,
  effectivePrivacy,
  catalogFreshnessLine,
  type BaamRegistry,
  type RegistryExtension,
} from './registry';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';
import { PrivacyBadge } from '../ui/PrivacyBadge';

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
      const dl = await window.electron.downloadRegistryAsset(ext.download);
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
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex max-h-[86vh] w-[720px] max-w-[92vw] flex-col gap-0 overflow-hidden p-0 sm:max-w-[92vw] lg:max-w-[720px]">
        {/* Header */}
        <div className="px-6 pt-5 pb-4 pr-14 border-b border-border-subtle">
          <div>
            <DialogTitle>Browse extensions</DialogTitle>
            <DialogDescription className="text-xs text-text-muted mt-0.5">
              Install MCP extensions from the Biorouter marketplace. Add one at a time. Most need
              credentials configured during install.
              {/* §10.2: a last-good catalogue is dated rather than dismissed as
                  "offline" — the entries are real, they are just not current. */}
              {catalogFreshnessLine({ live, fetchedAt }) && (
                <span className="text-text-subtle">
                  {' '}
                  · {catalogFreshnessLine({ live, fetchedAt })}
                </span>
              )}
            </DialogDescription>
          </div>
        </div>

        {/* Search */}
        <div className="px-6 pt-4 pb-3 border-b border-border-subtle">
          <input
            type="text"
            autoFocus
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search extensions by name, description, or tag…"
            className="biorouter-modal-panel w-full rounded-lg px-3 py-2 text-sm"
          />
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto px-6 py-3">
          {loadError && (
            <p className="text-sm text-text-danger text-center mt-10">
              Could not load the marketplace catalog.
            </p>
          )}
          {!registry && !loadError && (
            <p className="text-sm text-text-muted text-center mt-10 animate-pulse">
              Loading catalog…
            </p>
          )}
          {registry && filtered.length === 0 && (
            <p className="text-sm text-text-muted text-center mt-10">
              No extensions match your search.
            </p>
          )}
          <div className="flex flex-col gap-2">
            {filtered.map((ext) => {
              const installed = isInstalled(ext);
              return (
                <div
                  key={ext.id}
                  className="biorouter-modal-row flex items-start gap-3 rounded-lg px-3 py-3 hover:bg-background-default transition-colors"
                >
                  <div className="flex-shrink-0 mt-0.5 w-9 h-9 rounded-lg bg-background-default/80 flex items-center justify-center">
                    <Package className="w-5 h-5 text-text-muted" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-sm font-semibold text-text-default">{ext.name}</span>
                      {/* The union rule, not `ext.privacy`: a downgraded document
                          must not be able to un-badge an entry here either. */}
                      {registry && (
                        <PrivacyBadge
                          tier={effectivePrivacy(registry, ext.extension_name ?? ext.name)}
                        />
                      )}
                      <span className="text-[11px] text-text-subtle">
                        {[ext.organization, ext.version].filter(Boolean).join(' · ')}
                      </span>
                    </div>
                    <p className="text-xs text-text-muted mt-0.5 leading-relaxed">
                      {ext.description}
                    </p>
                    {ext.tags.length > 0 && (
                      <div className="flex gap-1 flex-wrap mt-1.5">
                        {ext.tags.map((t) => (
                          <span
                            key={t}
                            className="text-[10px] text-text-subtle bg-background-medium rounded px-1.5 py-0.5"
                          >
                            {t}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                  <div className="flex-shrink-0 self-center flex items-center gap-2">
                    {installed ? (
                      <>
                        <span className="text-[10px] uppercase tracking-wide text-background-info bg-background-info/10 rounded px-2 py-1">
                          Installed
                        </span>
                        {/* Issue #116: an installed row is a destination, not a
                            dead end — its credentials are the thing a user most
                            often comes back here to change. */}
                        {onConfigureInstalled && (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => onConfigureInstalled(ext)}
                          >
                            Configure
                          </Button>
                        )}
                      </>
                    ) : (
                      <Button size="sm" variant="outline" onClick={() => handleAdd(ext)}>
                        Add
                      </Button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        {/* Footer */}
        <div className="px-6 py-4 border-t border-border-subtle flex items-center justify-end">
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
