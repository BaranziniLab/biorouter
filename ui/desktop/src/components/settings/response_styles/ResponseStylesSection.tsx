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

function readStoredStyle(): ResponseStyleKey {
  try {
    const saved = localStorage.getItem(RESPONSE_STYLE_STORAGE_KEY);
    if (isResponseStyle(saved)) return saved;
  } catch {
    /* storage closed */
  }
  return DEFAULT_STYLE;
}

/**
 * Settings > Chat > Display > Tool call details: Expanded or Collapsed (spec §3.13). It used to
 * be "Response styles", two radio rows whose labels ("Detailed", "Concise") promised more than
 * the setting does: it only decides whether tool calls start open. The stored keys stay
 * `detailed` and `concise`.
 */
export function ToolCallDetailsRow() {
  // Read before the first paint: a stored "Expanded" must not draw "Collapsed" first and then
  // slide the segmented thumb across on every visit.
  const [currentStyle, setCurrentStyle] = useState<ResponseStyleKey>(readStoredStyle);

  useEffect(() => {
    // Collapsed is the default for new users, and it is written down so the transcript, which
    // reads the same key, agrees with what this row shows.
    try {
      if (!isResponseStyle(localStorage.getItem(RESPONSE_STYLE_STORAGE_KEY))) {
        localStorage.setItem(RESPONSE_STYLE_STORAGE_KEY, DEFAULT_STYLE);
      }
    } catch {
      /* storage closed: the default still shows, and nothing persists */
    }
  }, []);

  const handleStyleChange = (newStyle: ResponseStyleKey) => {
    setCurrentStyle(newStyle);
    try {
      localStorage.setItem(RESPONSE_STYLE_STORAGE_KEY, newStyle);
    } catch {
      /* storage closed: the transcript falls back to its default */
    }
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
