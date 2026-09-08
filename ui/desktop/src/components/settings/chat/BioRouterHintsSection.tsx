import { useState } from 'react';
import { Button } from '../../ui/button';
import { FolderKey } from '../../icons/app-icons';
import { BioRouterHintsModal } from './BioRouterHintsModal';

export const BioRouterHintsSection = () => {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const directory = window.appConfig?.get('BIOROUTER_WORKING_DIR') as string;

  return (
    <>
      <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-text-default">
        <div className="min-w-0 flex-1">
          <p className="text-label text-text-default">Project hints (.biorouterhints)</p>
          <p className="mt-0.5 max-w-md text-supporting text-text-muted">
            Configure your project's .biorouterhints file to provide additional context to Biorouter
          </p>
        </div>
        {/* No `className`. `flex` was flipping the cva base's `inline-flex`
            through tailwind-merge, and `items-center gap-2` restated what that
            base already emits. */}
        <Button onClick={() => setIsModalOpen(true)} variant="secondary">
          <FolderKey size={16} />
          Configure
        </Button>
      </div>
      {isModalOpen && (
        <BioRouterHintsModal directory={directory} setIsBioRouterHintsModalOpen={setIsModalOpen} />
      )}
    </>
  );
};
