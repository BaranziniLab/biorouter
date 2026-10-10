import { useCallback, useEffect, useRef, useState } from 'react';
import { FixedExtensionEntry, useConfig } from '../../ConfigContext';
import { ChevronRight } from '../../icons/app-icons';
import PermissionModal from './PermissionModal';
import { Button } from '../../ui/button';
import { ModalShell } from '../../ModalShell';
import { getFriendlyTitle } from '../extensions/subcomponents/ExtensionList';
import { nameToKey } from '../extensions/utils';
import { permissionDialogCopy } from '../chat/copy';

export function getConfigurableExtensions(extensions: FixedExtensionEntry[]) {
  return extensions
    .filter((extension) => extension.enabled && nameToKey(extension.name) !== 'platform')
    .sort((a, b) => getFriendlyTitle(a).localeCompare(getFriendlyTitle(b)));
}

/**
 * One enabled extension: a row that opens its tool rules. A name and at most one truncated line
 * of description (principle 4), then a chevron; no icon tile.
 */
function RuleItem({ extension }: { extension: FixedExtensionEntry }) {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const title = getFriendlyTitle(extension);
  const description = 'description' in extension ? extension.description || '' : '';

  return (
    <>
      <button
        type="button"
        className="biorouter-settings-row flex w-full min-w-0 items-center gap-3 px-3 py-2.5 text-left"
        onClick={() => setIsModalOpen(true)}
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate text-label text-text-default">{title}</span>
          {description && (
            <span className="block truncate text-supporting text-text-muted">{description}</span>
          )}
        </span>
        <ChevronRight aria-hidden="true" className="size-4 shrink-0 text-text-muted" />
      </button>
      {isModalOpen && (
        <PermissionModal
          onClose={() => setIsModalOpen(false)}
          extensionName={extension.name}
          extensionLabel={title}
        />
      )}
    </>
  );
}

interface PermissionRulesModalProps {
  isOpen: boolean;
  onClose: () => void;
}

/**
 * Settings > Chat > Approvals > Tool permissions > Edit…: the enabled extensions, each opening
 * its own tool rules. `ModalShell` `lg` with a scrolling body (spec §3.13).
 */
export default function PermissionRulesModal({ isOpen, onClose }: PermissionRulesModalProps) {
  const { getExtensions } = useConfig();
  const getExtensionsRef = useRef(getExtensions);
  const [extensions, setExtensions] = useState<FixedExtensionEntry[]>([]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');

  useEffect(() => {
    getExtensionsRef.current = getExtensions;
  }, [getExtensions]);

  const fetchExtensions = useCallback(async () => {
    setStatus('loading');
    try {
      const extensionsList = await getExtensionsRef.current(true);
      setExtensions(getConfigurableExtensions(extensionsList));
      setStatus('ready');
    } catch (error) {
      console.error('Failed to load extensions for permission settings:', error);
      setStatus('error');
    }
  }, []);

  useEffect(() => {
    if (isOpen) void fetchExtensions();
  }, [fetchExtensions, isOpen]);

  return (
    <ModalShell
      open={isOpen}
      onOpenChange={(open) => !open && onClose()}
      size="lg"
      scrollBody
      title={permissionDialogCopy.rulesTitle}
      subtitle={permissionDialogCopy.rulesSubtitle}
    >
      <div className="py-3">
        {status === 'loading' && (
          <p className="py-8 text-center text-supporting text-text-muted">
            {permissionDialogCopy.loadingExtensions}
          </p>
        )}

        {status === 'error' && (
          <div className="flex flex-col items-center gap-3 py-8 text-center">
            <p className="text-supporting text-text-muted">
              {permissionDialogCopy.extensionsFailed}
            </p>
            <Button variant="secondary" size="sm" onClick={fetchExtensions}>
              {permissionDialogCopy.tryAgain}
            </Button>
          </div>
        )}

        {status === 'ready' && extensions.length === 0 && (
          <p className="py-8 text-center text-supporting text-text-muted">
            {permissionDialogCopy.noExtensions}
          </p>
        )}

        {status === 'ready' && extensions.length > 0 && (
          <div
            className="biorouter-settings-list"
            role="group"
            aria-label={permissionDialogCopy.rulesListLabel}
          >
            {extensions.map((extension) => (
              <RuleItem key={nameToKey(extension.name)} extension={extension} />
            ))}
          </div>
        )}
      </div>
    </ModalShell>
  );
}
