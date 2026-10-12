import { useCallback, useMemo, useState } from 'react';
import { Switch, type SwitchProps } from '../../ui/switch';
import { SettingRow, settingLabelId } from '../../ui/setting-row';
import {
  CopilotSetupPanel,
  CopilotSetupTrigger,
  useCopilotSetup,
} from '../../copilot/CopilotSetup';
import { useConfig, FixedExtensionEntry } from '../../ConfigContext';
import { nameToKey } from '../extensions/utils';
import { toggleExtensionDefault } from '../extensions';
import { CAPABILITIES, CapabilityMeta } from './capabilities';

/** The extension key Biorouter Copilot kept for config compatibility. */
const COPILOT_KEY = 'computercontroller';

interface CapabilityItemProps {
  meta: CapabilityMeta;
  entry?: FixedExtensionEntry;
  onToggle: (entry: FixedExtensionEntry) => Promise<void>;
}

/**
 * A switch with something before it. `SettingRow` hands its one child the id, the label's
 * `aria-labelledby` and the help's `aria-describedby`; this passes all three to the switch and
 * leaves the extra control (Copilot's "Check setup") outside that naming.
 */
function SwitchWithLead({ lead, ...switchProps }: SwitchProps & { lead: React.ReactNode }) {
  return (
    <div className="flex items-center gap-3">
      {lead}
      <Switch {...switchProps} />
    </div>
  );
}

function useCapabilityToggle(
  entry: FixedExtensionEntry | undefined,
  onToggle: CapabilityItemProps['onToggle']
) {
  const [isToggling, setIsToggling] = useState(false);
  const handleToggle = async () => {
    if (!entry || isToggling) return;
    setIsToggling(true);
    try {
      await onToggle(entry);
    } finally {
      setIsToggling(false);
    }
  };
  return { isToggling, handleToggle };
}

// ⚠ **No state-dependent fill.** A capability row used to paint
// `bg-background-medium/70` while its switch was ON, which turned a ruled
// list into a striped one and said the state a second time in a weaker
// language. It was also inverted: `.biorouter-settings-row:hover` is
// unlayered and beats a `@layer utilities` background, so pointing at an ON
// row dropped its fill from 70% to 38% and the row visibly LIGHTENED. The
// switch states the state.
//
// The row is a `SettingRow`: the capability's name, an InfoTip carrying what it
// does, and the switch, which the row names (`aria-labelledby`) and describes
// (`aria-describedby`), so the name never reads "Toggle …".
function CapabilityItem({ meta, entry, onToggle }: CapabilityItemProps) {
  const { isToggling, handleToggle } = useCapabilityToggle(entry, onToggle);
  const enabled = entry ? entry.enabled : meta.defaultEnabled;

  return (
    <SettingRow label={meta.label} help={meta.description} controlId={`capability-${meta.key}`}>
      <Switch checked={enabled} onCheckedChange={handleToggle} disabled={!entry || isToggling} />
    </SettingRow>
  );
}

/**
 * Biorouter Copilot's row. "Check setup" is the row's trailing button and its result is a block
 * AFTER the row, never inside it: a result of several lines used to swell the row and take its
 * hover wash, with the switch centred against a paragraph.
 */
function CopilotCapabilityItem({ meta, entry, onToggle }: CapabilityItemProps) {
  const { isToggling, handleToggle } = useCapabilityToggle(entry, onToggle);
  const enabled = entry ? entry.enabled : meta.defaultEnabled;
  const setup = useCopilotSetup();
  const controlId = `capability-${meta.key}`;

  return (
    <>
      <SettingRow label={meta.label} help={meta.description} controlId={controlId}>
        <SwitchWithLead
          lead={<CopilotSetupTrigger setup={setup} describedBy={settingLabelId(controlId)} />}
          checked={enabled}
          onCheckedChange={handleToggle}
          disabled={!entry || isToggling}
        />
      </SettingRow>
      <CopilotSetupPanel setup={setup} />
    </>
  );
}

export const CapabilitiesSection = () => {
  const { extensionsList, addExtension, getExtensions } = useConfig();

  const entriesByKey = useMemo(() => {
    const map = new Map<string, FixedExtensionEntry>();
    for (const ext of extensionsList) {
      map.set(nameToKey(ext.name), ext);
    }
    return map;
  }, [extensionsList]);

  const handleToggle = useCallback(
    async (entry: FixedExtensionEntry) => {
      await toggleExtensionDefault({
        toggle: entry.enabled ? 'toggleOff' : 'toggleOn',
        extensionConfig: entry,
        addToConfig: addExtension,
        itemKind: 'capability',
      });
      await getExtensions(true);
    },
    [addExtension, getExtensions]
  );

  // A fragment, not a `space-y-1` wrapper: the rows belong directly to the
  // `.biorouter-settings-list` this section mounts into, so they abut and
  // `:last-child` selects the real last row instead of the last row of a nested
  // box. Nothing here draws a line: Settings switches the row's rule off by
  // scope (R-27), and a row is told apart by its rhythm and its hover fill.
  return (
    <>
      {CAPABILITIES.map((meta) => {
        const Item = meta.key === COPILOT_KEY ? CopilotCapabilityItem : CapabilityItem;
        return (
          <Item
            key={meta.key}
            meta={meta}
            entry={entriesByKey.get(meta.key)}
            onToggle={handleToggle}
          />
        );
      })}
    </>
  );
};
