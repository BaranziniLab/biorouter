import { useEffect, useState, useCallback } from 'react';
import { all_biorouter_modes, ModeSelectionItem } from './ModeSelectionItem';
import { useConfig } from '../../ConfigContext';
import { ConversationLimitsDropdown } from './ConversationLimitsDropdown';

export const ModeSection = () => {
  const [currentMode, setCurrentMode] = useState('auto');
  const [maxTurns, setMaxTurns] = useState<number>(1000);
  const { read, upsert } = useConfig();

  const handleModeChange = async (newMode: string) => {
    try {
      await upsert('BIOROUTER_MODE', newMode, false);
      setCurrentMode(newMode);
    } catch (error) {
      console.error('Error updating biorouter mode:', error);
      throw new Error(`Failed to store new biorouter mode: ${newMode}`);
    }
  };

  const fetchCurrentMode = useCallback(async () => {
    try {
      const mode = (await read('BIOROUTER_MODE', false)) as string;
      if (mode) {
        setCurrentMode(mode);
      }
    } catch (error) {
      console.error('Error fetching current mode:', error);
    }
  }, [read]);

  const fetchMaxTurns = useCallback(async () => {
    try {
      const turns = (await read('BIOROUTER_MAX_TURNS', false)) as number;
      if (turns) {
        setMaxTurns(turns);
      }
    } catch (error) {
      console.error('Error fetching max turns:', error);
    }
  }, [read]);

  const handleMaxTurnsChange = async (value: number) => {
    try {
      await upsert('BIOROUTER_MAX_TURNS', value, false);
      setMaxTurns(value);
    } catch (error) {
      console.error('Error updating max turns:', error);
    }
  };

  useEffect(() => {
    fetchCurrentMode();
    fetchMaxTurns();
  }, [fetchCurrentMode, fetchMaxTurns]);

  // ⚠ This section owns its `.biorouter-settings-list` rather than being mounted
  // inside one, and the reason is `role="radiogroup"`: the role has to sit on
  // the element that actually contains the radios. Every other Chat section
  // contributes a fragment of rows to the list its parent provides — they have
  // no semantics of their own to declare. `space-y-1` is gone either way: rows
  // abut inside the list and the hairline is the only separator.
  return (
    <div className="biorouter-settings-list" role="radiogroup" aria-label="Biorouter mode">
      {all_biorouter_modes.map((mode) => (
        <ModeSelectionItem
          key={mode.key}
          mode={mode}
          currentMode={currentMode}
          showDescription={true}
          handleModeChange={handleModeChange}
        />
      ))}

      <ConversationLimitsDropdown maxTurns={maxTurns} onMaxTurnsChange={handleMaxTurnsChange} />
    </div>
  );
};
