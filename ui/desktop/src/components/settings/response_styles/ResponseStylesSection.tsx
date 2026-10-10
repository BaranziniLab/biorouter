import { useEffect, useState } from 'react';
import { SegmentedControl } from '../../ui/segmented-control';
import { SettingRow } from '../../ui/setting-row';
import { displayCopy } from '../chat/copy';

type ResponseStyleKey = (typeof displayCopy.toolCallOptions)[number]['key'];

/** Where the choice is stored; the transcript reads it. The keys never change. */
export const RESPONSE_STYLE_STORAGE_KEY = 'response_style';
const DEFAULT_STYLE: ResponseStyleKey = 'concise';

function isResponseStyle(value: string | null): value is ResponseStyleKey {
  return displayCopy.toolCallOptions.some((option) => option.key === value);
}

/**
 * Settings > Chat > Display > Tool call details: Expanded or Collapsed (spec §3.13). It used to
 * be "Response styles", two radio rows whose labels ("Detailed", "Concise") promised more than
 * the setting does: it only decides whether tool calls start open. The stored keys stay
 * `detailed` and `concise`.
 */
export function ToolCallDetailsRow() {
  const [currentStyle, setCurrentStyle] = useState<ResponseStyleKey>(DEFAULT_STYLE);

  useEffect(() => {
    const savedStyle = localStorage.getItem(RESPONSE_STYLE_STORAGE_KEY);
    if (isResponseStyle(savedStyle)) {
      setCurrentStyle(savedStyle);
    } else {
      // Collapsed is the default for new users.
      localStorage.setItem(RESPONSE_STYLE_STORAGE_KEY, DEFAULT_STYLE);
      setCurrentStyle(DEFAULT_STYLE);
    }
  }, []);

  const handleStyleChange = (newStyle: ResponseStyleKey) => {
    setCurrentStyle(newStyle);
    localStorage.setItem(RESPONSE_STYLE_STORAGE_KEY, newStyle);
    // Tell the transcript, which listens for this event.
    window.dispatchEvent(new CustomEvent('responseStyleChanged'));
  };

  return (
    <SettingRow label={displayCopy.toolCallDetails} help={displayCopy.toolCallDetailsHelp}>
      <SegmentedControl<ResponseStyleKey>
        options={displayCopy.toolCallOptions.map((option) => ({
          value: option.key,
          label: option.label,
          testId: `tool-call-details-${option.key}`,
        }))}
        value={currentStyle}
        onValueChange={handleStyleChange}
      />
    </SettingRow>
  );
}
