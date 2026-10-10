import { useCallback, useEffect, useMemo, useState } from 'react';
import { Button } from '../../ui/button';
import { SettingRow } from '../../ui/setting-row';
import { ModalShell } from '../../ModalShell';
import { getTools, PermissionLevel, ToolInfo, upsertPermissions } from '../../../api';
import { toolIdentifierToTitleCase } from '../../../utils';
import { SettingSelect } from '../SettingSelect';
import { permissionDialogCopy } from '../chat/copy';

const permissionOptions = permissionDialogCopy.levels.map((level) => ({
  value: level.value as PermissionLevel,
  label: level.label,
}));

function getFirstSentence(text: string): string {
  const trimmed = text.trim();
  const match = trimmed.match(/^([^.?!]+[.?!])/);
  return match ? match[0] : trimmed;
}

function getToolLabel(name: string): string {
  const nameParts = name.split('__');
  const toolName = nameParts[nameParts.length - 1] || name;
  return toolIdentifierToTitleCase(toolName);
}

interface PermissionModalProps {
  extensionName: string;
  extensionLabel?: string;
  onClose: () => void;
}

/**
 * One extension's tool rules: a `SettingRow` per tool, its first sentence as the InfoTip, and
 * one select per row (Always allow, Ask before, Never allow). `ModalShell` `lg`, a scrolling
 * body, and the dialog's one committing action, Save changes (spec §3.13).
 */
export default function PermissionModal({
  extensionName,
  extensionLabel = extensionName,
  onClose,
}: PermissionModalProps) {
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [updatedPermissions, setUpdatedPermissions] = useState<Record<string, PermissionLevel>>({});
  const [loadStatus, setLoadStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saving' | 'error'>('idle');

  const hasChanges = useMemo(
    () =>
      Object.entries(updatedPermissions).some(
        ([toolName, permission]) =>
          permission !== tools.find((tool) => tool.name === toolName)?.permission
      ),
    [updatedPermissions, tools]
  );

  const fetchTools = useCallback(async () => {
    setLoadStatus('loading');
    setTools([]);
    setUpdatedPermissions({});
    try {
      const response = await getTools({
        query: { extension_name: extensionName, session_id: '' },
      });
      if (response.error) {
        setLoadStatus('error');
        return;
      }

      setTools(response.data || []);
      setLoadStatus('ready');
    } catch (error) {
      console.error('Error fetching extension tools:', error);
      setLoadStatus('error');
    }
  }, [extensionName]);

  useEffect(() => {
    void fetchTools();
  }, [fetchTools]);

  const handleSettingChange = (toolName: string, newPermission: PermissionLevel) => {
    setSaveStatus('idle');
    setUpdatedPermissions((previous) => ({
      ...previous,
      [toolName]: newPermission,
    }));
  };

  const handleSave = async () => {
    const toolPermissions = Object.entries(updatedPermissions)
      .filter(
        ([toolName, permission]) =>
          permission !== tools.find((tool) => tool.name === toolName)?.permission
      )
      .map(([toolName, permission]) => ({
        tool_name: toolName,
        permission,
      }));

    if (toolPermissions.length === 0) {
      onClose();
      return;
    }

    setSaveStatus('saving');
    try {
      const response = await upsertPermissions({
        body: { tool_permissions: toolPermissions },
      });
      if (response.error) {
        setSaveStatus('error');
        return;
      }
      onClose();
    } catch (error) {
      console.error('Error saving permissions:', error);
      setSaveStatus('error');
    }
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="lg"
      purpose={saveStatus === 'saving' ? 'required' : 'form'}
      scrollBody
      title={extensionLabel}
      subtitle={permissionDialogCopy.toolsSubtitle}
      footer={
        <>
          {saveStatus === 'error' && (
            <p className="mr-auto text-supporting text-text-danger" role="alert">
              {permissionDialogCopy.saveFailed}
            </p>
          )}
          <Button variant="outline" onClick={onClose} disabled={saveStatus === 'saving'}>
            {permissionDialogCopy.cancel}
          </Button>
          <Button
            disabled={!hasChanges || loadStatus !== 'ready' || saveStatus === 'saving'}
            onClick={handleSave}
          >
            {saveStatus === 'saving' ? permissionDialogCopy.saving : permissionDialogCopy.save}
          </Button>
        </>
      }
    >
      <div className="py-3">
        {loadStatus === 'loading' && (
          <p className="py-8 text-center text-supporting text-text-muted">
            {permissionDialogCopy.loadingTools}
          </p>
        )}

        {loadStatus === 'error' && (
          <div className="flex flex-col items-center gap-3 py-8 text-center">
            <div>
              <p className="text-label text-text-default">{permissionDialogCopy.toolsFailed}</p>
              <p className="mt-1 text-supporting text-text-muted">
                {permissionDialogCopy.toolsFailedHelp}
              </p>
            </div>
            <Button variant="secondary" size="sm" onClick={fetchTools}>
              {permissionDialogCopy.tryAgain}
            </Button>
          </div>
        )}

        {loadStatus === 'ready' && tools.length === 0 && (
          <div className="py-8 text-center">
            <p className="text-label text-text-default">{permissionDialogCopy.noTools}</p>
            <p className="mt-1 text-supporting text-text-muted">
              {permissionDialogCopy.noToolsHelp}
            </p>
          </div>
        )}

        {loadStatus === 'ready' && tools.length > 0 && (
          <div className="biorouter-settings-list">
            {tools.map((tool) => {
              const selectedPermission = updatedPermissions[tool.name] || tool.permission;
              return (
                <SettingRow
                  key={tool.name}
                  label={getToolLabel(tool.name)}
                  help={tool.description ? getFirstSentence(tool.description) : undefined}
                >
                  <SettingSelect<PermissionLevel>
                    options={permissionOptions}
                    value={selectedPermission ?? 'ask_before'}
                    onValueChange={(next) => handleSettingChange(tool.name, next)}
                    triggerClassName="w-36"
                    contentClassName="min-w-36"
                  />
                </SettingRow>
              );
            })}
          </div>
        )}
      </div>
    </ModalShell>
  );
}
