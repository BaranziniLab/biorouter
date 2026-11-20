import { useState } from 'react';
import { SettingRow } from '../../ui/setting-row';
import { SegmentedControl } from '../../ui/segmented-control';
import { isBrowserSurface } from '../../../utils/surface';
import { readLinkTarget, writeLinkTarget, type LinkTarget } from '../../artifacts/linkPreference';
import { displayCopy } from './copy';

/**
 * Settings > Chat > Display > Open links in (BB-02): the Biorouter Browser in the chat's panel,
 * or the default browser. Not shown on a `biorouter serve` page, which has no built-in browser:
 * its links always open in a new tab of the person's own browser.
 */
export function OpenLinksInRow() {
  const [target, setTarget] = useState<LinkTarget>(readLinkTarget);
  if (isBrowserSurface()) return null;
  return (
    <SettingRow label={displayCopy.openLinksIn} help={displayCopy.openLinksInHelp}>
      <SegmentedControl<LinkTarget>
        options={displayCopy.openLinksOptions.map((option) => ({
          value: option.key as LinkTarget,
          label: option.label,
          testId: `open-links-in-${option.key}`,
        }))}
        value={target}
        onValueChange={(next) => {
          setTarget(next);
          writeLinkTarget(next);
        }}
      />
    </SettingRow>
  );
}
