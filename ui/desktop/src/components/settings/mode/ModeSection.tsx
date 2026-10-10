import { useEffect, useState, useCallback } from 'react';
import { useConfig } from '../../ConfigContext';
import { Button } from '../../ui/button';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import PermissionRulesModal from '../permission/PermissionRulesModal';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { approvalsCopy } from '../chat/copy';
import { ApprovalModeSelect } from './ApprovalModeSelect';
import { MaxTurnsRow } from './ConversationLimitsDropdown';

/** What `BIOROUTER_MODE` means when nothing is saved: the daemon's own default. */
const DEFAULT_MODE = 'auto';

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
  // `null` until the daemon has answered. The select stays blank and disabled until then: showing
  // "Autonomous" first and correcting it a moment later told a person who chose Manual that
  // nothing would ask them, which is the one setting that must never read wrong.
  const [currentMode, setCurrentMode] = useState<string | null>(null);
  // `null` until a value is known to be saved; the field then shows the
  // agent's default rather than a number it does not use.
  const [maxTurns, setMaxTurns] = useState<number | null>(null);
  // Whether the saved Max turns has been read, saved or not, so the field is not typed into (or
  // read) as the default for the moment before the answer.
  const [maxTurnsRead, setMaxTurnsRead] = useState(false);
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
      const mode = await read('BIOROUTER_MODE', false);
      // Nothing saved is the daemon's default, Autonomous.
      setCurrentMode(typeof mode === 'string' && mode ? mode : DEFAULT_MODE);
    } catch (error) {
      console.error('Error fetching current mode:', error);
      setCurrentMode(DEFAULT_MODE);
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
    } finally {
      setMaxTurnsRead(true);
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

  const permissionsApply = currentMode !== null && MODES_WITH_TOOL_PERMISSIONS.has(currentMode);

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

      <MaxTurnsRow
        maxTurns={maxTurns}
        onMaxTurnsChange={handleMaxTurnsChange}
        loaded={maxTurnsRead}
      />

      <PermissionRulesModal isOpen={permissionsOpen} onClose={() => setPermissionsOpen(false)} />
    </SettingSection>
  );
};
