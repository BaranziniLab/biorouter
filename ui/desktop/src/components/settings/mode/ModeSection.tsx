import { useEffect, useState, useCallback } from 'react';
import { useConfig } from '../../ConfigContext';
import { Button } from '../../ui/button';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import PermissionRulesModal from '../permission/PermissionRulesModal';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { approvalsCopy } from '../chat/copy';
import { ApprovalModeSelect } from './ApprovalModeSelect';
import { MaxTurnsRow } from './ConversationLimitsDropdown';

/** The modes whose approvals the per-tool permissions shape. */
const MODES_WITH_TOOL_PERMISSIONS = new Set(['approve', 'smart_approve']);

/**
 * Settings > Chat > Approvals (renamed from "Mode", which collided with the light and dark
 * theme). Three rows, one control each (spec §3.13):
 * - Approval mode: a select menu, each mode's one-line description inside its item;
 * - Tool permissions: `Edit…`, enabled in Manual and Smart, the two modes it shapes;
 * - Max turns: a number field, always shown (it used to hide behind a "Chat limits"
 *   disclosure that held exactly one field).
 */
export const ModeSection = () => {
  const [currentMode, setCurrentMode] = useState('auto');
  // `null` until a value is known to be saved; the field then shows the
  // agent's default rather than a number it does not use.
  const [maxTurns, setMaxTurns] = useState<number | null>(null);
  const [permissionsOpen, setPermissionsOpen] = useState(false);
  const { read, upsert } = useConfig();

  const handleModeChange = async (newMode: string) => {
    try {
      await upsert('BIOROUTER_MODE', newMode, false);
      setCurrentMode(newMode);
    } catch (error) {
      console.error('Error updating biorouter mode:', error);
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
      const turns = await read('BIOROUTER_MAX_TURNS', false);
      // Every saved number is shown, 0 and negatives included. `if (turns)`
      // hid a saved 0 (the value that stops every new chat) behind the
      // default, so the field looked fine while nothing worked.
      const stored = typeof turns === 'string' && turns.trim() !== '' ? Number(turns) : turns;
      if (typeof stored === 'number' && Number.isFinite(stored)) {
        setMaxTurns(stored);
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

  const permissionsApply = MODES_WITH_TOOL_PERMISSIONS.has(currentMode);

  return (
    <SettingSection id={SETTINGS_SECTION_IDS.approvals} title={approvalsCopy.section}>
      <SettingRow label={approvalsCopy.mode} help={approvalsCopy.modeHelp}>
        <ApprovalModeSelect value={currentMode} onValueChange={handleModeChange} />
      </SettingRow>

      <SettingRow label={approvalsCopy.toolPermissions} help={approvalsCopy.toolPermissionsHelp}>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={!permissionsApply}
          aria-label={approvalsCopy.editToolPermissions}
          onClick={() => setPermissionsOpen(true)}
        >
          {approvalsCopy.edit}
        </Button>
      </SettingRow>

      <MaxTurnsRow maxTurns={maxTurns} onMaxTurnsChange={handleMaxTurnsChange} />

      <PermissionRulesModal isOpen={permissionsOpen} onClose={() => setPermissionsOpen(false)} />
    </SettingSection>
  );
};
