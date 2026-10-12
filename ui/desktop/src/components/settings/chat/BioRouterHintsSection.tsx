import { useState } from 'react';
import { Button } from '../../ui/button';
import { SettingRow } from '../../ui/setting-row';
import { BioRouterHintsModal } from './BioRouterHintsModal';
import { projectCopy } from './copy';

/**
 * Settings > Chat > Project > Project hints. The filename lives in the InfoTip, not the label,
 * and the action is a plain `Edit…` on the row's 28px rung (spec §3.13).
 */
export const ProjectHintsRow = () => {
  const [isModalOpen, setIsModalOpen] = useState(false);
  const directory = window.appConfig?.get('BIOROUTER_WORKING_DIR') as string;

  return (
    <>
      <SettingRow label={projectCopy.hints} help={projectCopy.hintsHelp}>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          aria-label={projectCopy.editHints}
          onClick={() => setIsModalOpen(true)}
        >
          {projectCopy.edit}
        </Button>
      </SettingRow>
      {isModalOpen && (
        <BioRouterHintsModal directory={directory} setIsBioRouterHintsModalOpen={setIsModalOpen} />
      )}
    </>
  );
};
