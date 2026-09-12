import React, { useCallback, useEffect, useState } from 'react';
import { ScrollArea } from '../../ui/scroll-area';
import BackButton from '../../ui/BackButton';
import { FixedExtensionEntry, useConfig } from '../../ConfigContext';
import { ChevronRight } from '../../icons/app-icons';
import PermissionModal from './PermissionModal';
import { Button } from '../../ui/button';
import { getFriendlyTitle } from '../extensions/subcomponents/ExtensionList';
import { getConfigurableExtensions } from './PermissionRulesModal';

function RuleItem({ extension }: { extension: FixedExtensionEntry }) {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const title = getFriendlyTitle(extension);
  const description = 'description' in extension ? extension.description || '' : '';

  return (
    <>
      <Button
        className="h-auto min-h-14 w-full justify-between gap-4 px-3 py-3 text-left whitespace-normal"
        onClick={() => setIsModalOpen(true)}
        variant="secondary"
        size="lg"
      >
        <div className="min-w-0 flex-1">
          <h3 className="font-semibold text-text-default break-words [overflow-wrap:anywhere]">
            {title}
          </h3>
          <p className="mt-1 text-xs text-text-muted break-words [overflow-wrap:anywhere]">
            {description}
          </p>
        </div>
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

function RulesSection({ title, rules }: { title: string; rules: React.ReactNode }) {
  return (
    <div className="space-y-4">
      <h2 className="text-base font-semibold text-text-default">{title}</h2>
      {rules}
    </div>
  );
}

export default function PermissionSettingsView({ onClose }: { onClose: () => void }) {
  const { getExtensions } = useConfig();
  const [extensions, setExtensions] = useState<FixedExtensionEntry[]>([]);

  const fetchExtensions = useCallback(async () => {
    const extensionsList = await getExtensions(true);
    setExtensions(getConfigurableExtensions(extensionsList));
  }, [getExtensions]);

  useEffect(() => {
    fetchExtensions();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="bg-background-muted h-screen w-full animate-[appear_200ms_var(--ease-out)_forwards]">
      <ScrollArea className="h-full w-full">
        <div className="flex flex-col pb-24">
          <div className="px-8 pt-6 pb-4">
            <BackButton onClick={() => onClose()} className="mb-4" />
            <div className="rounded-element bg-background-inverse w-16 h-16 flex items-center justify-center mb-4">
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="24"
                height="24"
                viewBox="0 0 24 24"
                className="stroke-text-inverse fill-background-inverse"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="m15.5 7.5 2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L19 4" />
                <path d="m21 2-9.6 9.6" />
                <circle cx="7.5" cy="15.5" r="5.5" />
              </svg>
            </div>
            <h1 className="text-title text-text-default mt-4">Permission rules</h1>
            <p className="text-text-muted">
              Hidden instructions that will be passed to the provider to help direct and add context
              to your responses.
            </p>
          </div>

          {/* Content Area */}
          <div className="flex-1 pt-[20px]">
            <div className="space-y-8 px-8">
              {/* Extension Rules Section */}
              <RulesSection
                title="Extension rules"
                rules={
                  <>
                    {extensions.map((extension) => (
                      <RuleItem key={extension.name} extension={extension} />
                    ))}
                  </>
                }
              />
            </div>
          </div>
        </div>
      </ScrollArea>
    </div>
  );
}
