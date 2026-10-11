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
 * One enabled extension: a 40px row that opens its tool rules. The name and a chevron, nothing
 * else. The daemon's description of the extension used to sit under the name, and it is raw data
 * (em dashes, "BioRouter", whole sentences that cut off mid-word), twelve of them in a column
 * (vision V4-09). The Extensions page holds it; this list only needs to say which one opens.
 *
 * The row is `.br-setting-row`'s box (10px, a 20px line, 9px and the 1px hairline room, so 40px
 * exactly) around a button's worth of content, the same row a `SettingRow` is.
 */
function RuleItem({ extension }: { extension: FixedExtensionEntry }) {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const title = getFriendlyTitle(extension);

  return (
    <>
      <button
        type="button"
        className="biorouter-settings-row br-setting-row w-full text-left"
        onClick={() => setIsModalOpen(true)}
      >
        <span className="br-setting-row-text">
          <span className="br-setting-row-label text-label text-text-default">{title}</span>
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
            className="biorouter-settings-list br-rows-flat"
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
