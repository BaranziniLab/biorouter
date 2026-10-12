import { useEffect, useState, useCallback } from 'react';
import { useConfig } from '../../ConfigContext';
import { Button } from '../../ui/button';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import PermissionRulesModal from '../permission/PermissionRulesModal';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { approvalsCopy } from '../chat/copy';
import { recall, useLastKnown } from '../lastKnown';
import { ApprovalModeSelect } from './ApprovalModeSelect';
import { MaxTurnsRow } from './ConversationLimitsDropdown';
import {
  APPROVAL_MODE_KEY,
  DEFAULT_MODE,
  MAX_TURNS_KEY,
  approvalModeFromConfig,
  maxTurnsFromConfig,
} from './storedSettings';

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
  // `undefined` until the daemon has answered, and only the first time this window asks: each
  // visit then starts from what the last answer was and replaces it when the new one lands (V4-R7-2).
  // The select stays blank and disabled while nothing is known: showing "Autonomous" first and
  // correcting it a moment later told a person who chose Manual that nothing would ask them,
  // which is the one setting that must never read wrong. A remembered answer is not that guess.
  const [currentMode, setCurrentMode] = useLastKnown<string>('approvalMode');
  // `undefined` until a read has answered; `null` once it has and nothing is saved, so the field
  // shows the agent's default rather than a number it does not use. The field is blank and
  // disabled until it is known, not typed into (or read) as the default for the moment before
  // the answer.
  const [maxTurns, setMaxTurns] = useLastKnown<number | null>('maxTurns');
  const [permissionsOpen, setPermissionsOpen] = useState(false);
  const { read, upsert } = useConfig();

  const handleModeChange = async (newMode: string) => {
    try {
      await upsert(APPROVAL_MODE_KEY, newMode, false);
      setCurrentMode(newMode);
    } catch (error) {
      console.error('Error updating biorouter mode:', error);
    }
  };

  const fetchCurrentMode = useCallback(async () => {
    try {
      setCurrentMode(approvalModeFromConfig(await read(APPROVAL_MODE_KEY, false)));
    } catch (error) {
      console.error('Error fetching current mode:', error);
      // A failed read keeps what the last one said, and with none the daemon's default.
      if (recall('approvalMode') === undefined) setCurrentMode(DEFAULT_MODE);
    }
  }, [read, setCurrentMode]);

  const fetchMaxTurns = useCallback(async () => {
    try {
      setMaxTurns(maxTurnsFromConfig(await read(MAX_TURNS_KEY, false)));
    } catch (error) {
      console.error('Error fetching max turns:', error);
      // As above: the default shows, and the field is usable, only when nothing was ever read.
      if (recall('maxTurns') === undefined) setMaxTurns(null);
    }
  }, [read, setMaxTurns]);

  const handleMaxTurnsChange = async (value: number) => {
    try {
      await upsert(MAX_TURNS_KEY, value, false);
      setMaxTurns(value);
    } catch (error) {
      console.error('Error updating max turns:', error);
    }
  };

  useEffect(() => {
    fetchCurrentMode();
    fetchMaxTurns();
  }, [fetchCurrentMode, fetchMaxTurns]);

  const permissionsApply =
    currentMode !== undefined && MODES_WITH_TOOL_PERMISSIONS.has(currentMode);

  return (
    <SettingSection id={SETTINGS_SECTION_IDS.approvals} title={approvalsCopy.section}>
      <SettingRow label={approvalsCopy.mode} help={approvalsCopy.modeHelp}>
        <ApprovalModeSelect value={currentMode ?? null} onValueChange={handleModeChange} />
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
        maxTurns={maxTurns ?? null}
        onMaxTurnsChange={handleMaxTurnsChange}
        loaded={maxTurns !== undefined}
      />

      <PermissionRulesModal isOpen={permissionsOpen} onClose={() => setPermissionsOpen(false)} />
    </SettingSection>
  );
};
