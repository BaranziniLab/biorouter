import { useCallback, useEffect, useRef, useState } from 'react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';
import { FixedExtensionEntry, useConfig } from '../../ConfigContext';
import { ChevronRight, SlidersHorizontal } from '../../icons/app-icons';
import PermissionModal from './PermissionModal';
import { Button } from '../../ui/button';
import { getFriendlyTitle } from '../extensions/subcomponents/ExtensionList';
import { nameToKey } from '../extensions/utils';

export function getConfigurableExtensions(extensions: FixedExtensionEntry[]) {
  return extensions
    .filter((extension) => extension.enabled && nameToKey(extension.name) !== 'platform')
    .sort((a, b) => getFriendlyTitle(a).localeCompare(getFriendlyTitle(b)));
}

function RuleItem({ extension }: { extension: FixedExtensionEntry }) {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const title = getFriendlyTitle(extension);
  const description = 'description' in extension ? extension.description || '' : '';

  return (
    <>
      <Button
        className="biorouter-settings-row h-auto min-h-14 w-full justify-between gap-4 px-3 py-3 text-left whitespace-normal"
        onClick={() => setIsModalOpen(true)}
        variant="ghost"
        size="lg"
      >
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium text-text-default break-words [overflow-wrap:anywhere]">
            {title}
          </span>
          {description && (
            <span className="mt-0.5 block text-xs font-normal leading-5 text-text-muted break-words [overflow-wrap:anywhere]">
              {description}
            </span>
          )}
        </span>
        <ChevronRight className="h-4 w-4 flex-shrink-0" />
      </Button>
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
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[min(760px,calc(100vh-2rem))] p-0 flex flex-col overflow-hidden sm:max-w-[720px]">
        <DialogHeader className="flex-shrink-0 border-b border-border-subtle px-5 pb-5 pt-5 sm:px-6">
          <div className="flex min-w-0 items-start gap-3 pr-6">
            <div className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-element bg-background-medium text-text-default">
              <SlidersHorizontal className="h-5 w-5" />
            </div>
            <div className="min-w-0 pt-0.5">
              <DialogTitle className="text-base font-semibold text-text-default">
                Tool permissions
              </DialogTitle>
              <DialogDescription className="mt-1 text-sm leading-5 text-text-muted break-words">
                Choose how Manual and Smart modes handle tools from each enabled extension.
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 sm:px-6">
          {status === 'loading' && (
            <div className="flex min-h-36 items-center justify-center text-sm text-text-muted">
              Loading enabled extensions…
            </div>
          )}

          {status === 'error' && (
            <div className="flex min-h-36 flex-col items-center justify-center gap-3 text-center">
              <p className="text-sm text-text-muted">Enabled extensions could not be loaded.</p>
              <Button variant="outline" size="sm" onClick={fetchExtensions}>
                Try again
              </Button>
            </div>
          )}

          {status === 'ready' && extensions.length === 0 && (
            <div className="flex min-h-36 items-center justify-center text-center text-sm text-text-muted">
              No enabled extensions have configurable tool permissions.
            </div>
          )}

          {status === 'ready' && extensions.length > 0 && (
            <div className="biorouter-settings-list" aria-label="Enabled extension permissions">
              {extensions.map((extension) => (
                <RuleItem key={nameToKey(extension.name)} extension={extension} />
              ))}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
