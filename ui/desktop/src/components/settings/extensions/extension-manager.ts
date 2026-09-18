import type { ExtensionConfig } from '../../../api/types.gen';
import { toastService } from '../../../toasts';

interface DeleteExtensionProps {
  name: string;
  removeFromConfig: (name: string) => Promise<void>;
  extensionConfig?: ExtensionConfig;
}

/**
 * Deletes an extension from config (will no longer be loaded in new sessions)
 */
export async function deleteExtension({ name, removeFromConfig }: DeleteExtensionProps) {
  try {
    await removeFromConfig(name);
    toastService.success({
      title: name,
      msg: 'Extension removed. Saved credentials were retained and may be reused on reinstall.',
    });
  } catch (error) {
    console.error('Failed to remove extension from config:', error);
    throw error;
  }
}

interface ToggleExtensionDefaultProps {
  toggle: 'toggleOn' | 'toggleOff';
  extensionConfig: ExtensionConfig;
  addToConfig: (name: string, extensionConfig: ExtensionConfig, enabled: boolean) => Promise<void>;
  itemKind?: 'capability' | 'extension';
}

export async function toggleExtensionDefault({
  toggle,
  extensionConfig,
  addToConfig,
  itemKind = 'extension',
}: ToggleExtensionDefaultProps) {
  const enabled = toggle === 'toggleOn';
  const itemLabel = itemKind === 'capability' ? 'Capability' : 'Extension';

  try {
    await addToConfig(extensionConfig.name, extensionConfig, enabled);
    toastService.success({
      title: extensionConfig.name,
      msg: enabled ? `${itemLabel} enabled for new chats` : `${itemLabel} disabled for new chats`,
    });
  } catch (error) {
    console.error(`Failed to update ${itemKind} default in config:`, error);
    toastService.error({
      title: extensionConfig.name,
      msg: `Failed to update ${itemKind} default`,
    });
    throw error;
  }
}

interface ActivateExtensionDefaultProps {
  addToConfig: (name: string, extensionConfig: ExtensionConfig, enabled: boolean) => Promise<void>;
  extensionConfig: ExtensionConfig;
}

export async function activateExtensionDefault({
  addToConfig,
  extensionConfig,
}: ActivateExtensionDefaultProps): Promise<void> {
  try {
    await addToConfig(extensionConfig.name, extensionConfig, true);
    toastService.success({
      title: extensionConfig.name,
      msg: 'Extension added as default',
    });
  } catch (error) {
    console.error('Failed to add extension to config:', error);
    toastService.error({
      title: extensionConfig.name,
      msg: 'Failed to add extension',
    });
    throw error;
  }
}
