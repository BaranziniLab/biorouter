import { useCallback, useMemo, useState } from 'react';
import { Switch } from '../../ui/switch';
import { useConfig, FixedExtensionEntry } from '../../ConfigContext';
import { nameToKey } from '../extensions/utils';
import { toggleExtensionDefault } from '../extensions';
import { CAPABILITIES, CapabilityMeta } from './capabilities';

interface CapabilityItemProps {
  meta: CapabilityMeta;
  entry?: FixedExtensionEntry;
  onToggle: (entry: FixedExtensionEntry) => Promise<void>;
}

function CapabilityItem({ meta, entry, onToggle }: CapabilityItemProps) {
  const [isToggling, setIsToggling] = useState(false);
  const enabled = entry ? entry.enabled : meta.defaultEnabled;

  const handleToggle = async () => {
    if (!entry || isToggling) return;
    setIsToggling(true);
    try {
      await onToggle(entry);
    } finally {
      setIsToggling(false);
    }
  };

  // ⚠ **No state-dependent fill.** This row used to paint
  // `bg-background-medium/70` while its switch was ON, which turned a hairline
  // list into a striped one and said the state a second time in a weaker
  // language. It was also inverted: `.biorouter-settings-row:hover` is
  // unlayered and beats a `@layer utilities` background, so pointing at an ON
  // row dropped its fill from 70% to 38% and the row visibly LIGHTENED. The
  // switch states the state.
  return (
    <div className="biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-text-default">
      <div className="min-w-0 flex-1">
        <p className="text-label text-text-default">{meta.label}</p>
        <p className="mt-0.5 max-w-md text-supporting text-text-muted">{meta.description}</p>
      </div>

      <div className="flex flex-shrink-0 items-center">
        <Switch
          checked={enabled}
          onCheckedChange={handleToggle}
          disabled={!entry || isToggling}
          variant="mono"
          aria-label={`Toggle ${meta.label} capability`}
        />
      </div>
    </div>
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
  // `.biorouter-settings-list` this section mounts into, so they abut, the
  // hairline is their only separator, and `:last-child` selects the real last
  // row instead of the last row of a nested box.
  return (
    <>
      {CAPABILITIES.map((meta) => (
        <CapabilityItem
          key={meta.key}
          meta={meta}
          entry={entriesByKey.get(meta.key)}
          onToggle={handleToggle}
        />
      ))}
    </>
  );
};
