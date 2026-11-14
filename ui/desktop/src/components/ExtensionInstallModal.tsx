import { useState, useCallback, useEffect, useRef } from 'react';
import { IpcRendererEvent } from 'electron';
import { ModalShell } from './ModalShell';
import { Button } from './ui/button';
import { InfoTip } from './ui/info-tip';
import { extractExtensionName } from './settings/extensions/utils';
import { addExtensionFromDeepLink } from './settings/extensions/deeplink';
import type { ExtensionConfig } from '../api/types.gen';
import { View, ViewOptions } from '../utils/navigationUtils';
import { useConfig } from './ConfigContext';
import { toastService } from '../toasts';
import { DependencyErrorBanner } from './DependencyErrorBanner';

type ModalType = 'blocked' | 'untrusted' | 'trusted';

interface ExtensionInfo {
  name: string;
  command?: string;
  remoteUrl?: string;
  link: string;
}

interface ExtensionModalState {
  isOpen: boolean;
  modalType: ModalType;
  extensionInfo: ExtensionInfo | null;
  isPending: boolean;
  error: string | null;
}

interface ExtensionModalConfig {
  title: string;
  /** The one line the decision needs: what this install does, or why it is refused. */
  message: string;
  /** What will run: the command, or the remote URL, as a value to read character by character. */
  source: { label: 'Command' | 'URL'; value: string };
  /** The advice behind the line, at most two sentences, in an InfoTip beside it. */
  help?: string;
  confirmLabel: string;
  cancelLabel: string;
  showSingleButton: boolean;
  isBlocked: boolean;
}

interface ExtensionInstallModalProps {
  addExtension?: (name: string, config: ExtensionConfig, enabled: boolean) => Promise<void>;
  setView: (view: View, options?: ViewOptions) => void;
}

function extractCommand(link: string): string {
  const url = new URL(link);

  // For remote extensions (SSE or Streaming HTTP), return the URL
  const remoteUrl = url.searchParams.get('url');
  if (remoteUrl) {
    return remoteUrl;
  }

  // For stdio extensions, return the command
  const cmd = url.searchParams.get('cmd') || 'Unknown Command';
  const args = url.searchParams.getAll('arg').map(decodeURIComponent);
  return `${cmd} ${args.join(' ')}`.trim();
}

function extractRemoteUrl(link: string): string | null {
  const url = new URL(link);
  return url.searchParams.get('url');
}

export function ExtensionInstallModal({ addExtension, setView }: ExtensionInstallModalProps) {
  const { getExtensions } = useConfig();
  const getExtensionsRef = useRef(getExtensions);
  const processingLinkRef = useRef<string | null>(null);

  useEffect(() => {
    getExtensionsRef.current = getExtensions;
  }, [getExtensions]);

  const [modalState, setModalState] = useState<ExtensionModalState>({
    isOpen: false,
    modalType: 'trusted',
    extensionInfo: null,
    isPending: false,
    error: null,
  });

  const [pendingLink, setPendingLink] = useState<string | null>(null);

  const determineModalType = async (
    command: string,
    _remoteUrl: string | null
  ): Promise<ModalType> => {
    try {
      const config = window.electron.getConfig();
      const ALLOWLIST_WARNING_MODE = config.BIOROUTER_ALLOWLIST_WARNING === true;

      if (ALLOWLIST_WARNING_MODE) {
        return 'untrusted';
      }

      const allowedCommands = await window.electron.getAllowedExtensions();

      if (!allowedCommands || allowedCommands.length === 0) {
        return 'trusted';
      }

      const isCommandAllowed = allowedCommands.some((allowedCmd: string) =>
        command.startsWith(allowedCmd)
      );

      return isCommandAllowed ? 'trusted' : 'blocked';
    } catch (error) {
      console.error('Error checking allowlist:', error);
      return 'trusted';
    }
  };

  const generateModalConfig = (
    modalType: ModalType,
    extensionInfo: ExtensionInfo
  ): ExtensionModalConfig => {
    const { name, command, remoteUrl } = extensionInfo;
    // A remote extension names its URL, a local one the command it runs.
    const source: ExtensionModalConfig['source'] = remoteUrl
      ? { label: 'URL', value: remoteUrl }
      : { label: 'Command', value: command ?? '' };

    switch (modalType) {
      // One confirmation recipe (Astryx §3.6): the title is the verb phrase the
      // primary button performs, and the buttons name that action — never
      // "Yes"/"No", which force the reader back up to the title to find out
      // what they are agreeing to.
      // Owner, 2026-11-13: the line a person decides on stays visible (the refusal's reason, the
      // consequence of installing), the command or URL sits under it as data, and the advice
      // behind it is an InfoTip beside the line.
      case 'blocked':
        return {
          title: `Cannot install ${name}`,
          message:
            'This extension command is not in the allowed list, so its installation is blocked.',
          source,
          help: 'Contact your administrator to request approval for this extension.',
          confirmLabel: 'Got it',
          cancelLabel: '',
          showSingleButton: true,
          isBlocked: true,
        };

      case 'untrusted': {
        return {
          title: `Install untrusted extension ${name}?`,
          message:
            'This extension command is not in the allowed list. Once installed it can read your chats.',
          source,
          help: 'Installing extensions from untrusted sources may pose security risks. Contact your administrator if you are unsure about this.',
          confirmLabel: 'Install anyway',
          cancelLabel: 'Cancel',
          showSingleButton: false,
          isBlocked: false,
        };
      }

      case 'trusted':
      default:
        return {
          title: `Install ${name}?`,
          message: 'This extension will be added to Biorouter and enabled for new chats.',
          source,
          confirmLabel: 'Install',
          cancelLabel: 'Cancel',
          showSingleButton: false,
          isBlocked: false,
        };
    }
  };

  const handleExtensionRequest = useCallback(async (link: string): Promise<void> => {
    if (processingLinkRef.current === link) {
      console.log(`Skipping duplicate extension request (already processing): ${link}`);
      return;
    }
    processingLinkRef.current = link;

    try {
      console.log(`Processing extension request: ${link}`);

      const command = extractCommand(link);
      const remoteUrl = extractRemoteUrl(link);
      const extName = extractExtensionName(link);
      const extensionsList = await getExtensionsRef.current(true);

      if (extensionsList?.find((ext) => ext.name === extName)) {
        console.log(`Extension Already Installed: ${extName}`);

        // The name is the title, as every other extension toast has it; the message does not
        // say it a second time.
        toastService.success({
          title: extName,
          msg: 'Already installed. Start a new chat to use it.',
        });
        return;
      }
      console.log('Extension not found, continuing to show modal');

      const extensionInfo: ExtensionInfo = {
        name: extName,
        command: command,
        remoteUrl: remoteUrl || undefined,
        link: link,
      };

      const modalType = await determineModalType(command, remoteUrl);

      setModalState({
        isOpen: true,
        modalType,
        extensionInfo,
        isPending: false,
        error: null,
      });

      setPendingLink(modalType === 'blocked' ? null : link);

      window.electron.logInfo(`Extension modal opened: ${modalType} for ${extName}`);
    } catch (error) {
      console.error('Error processing extension request:', error);
      setModalState((prev) => ({
        ...prev,
        error: error instanceof Error ? error.message : 'Unknown error',
      }));
    } finally {
      processingLinkRef.current = null;
    }
  }, []);

  const dismissModal = useCallback(() => {
    setModalState({
      isOpen: false,
      modalType: 'trusted',
      extensionInfo: null,
      isPending: false,
      error: null,
    });
    setPendingLink(null);
  }, []);

  const confirmInstall = useCallback(async (): Promise<void> => {
    if (!pendingLink) {
      return;
    }

    setModalState((prev) => ({ ...prev, isPending: true }));

    try {
      console.log(`Confirming installation of extension from: ${pendingLink}`);

      if (addExtension) {
        await addExtensionFromDeepLink(
          pendingLink,
          addExtension,
          (view: string, options?: ViewOptions) => {
            console.log('Extension installation completed, navigating to:', view, options);
            setView(view as View, options);
          }
        );
      } else {
        throw new Error('addExtension function not provided to component');
      }

      // Only dismiss modal after successful installation
      dismissModal();
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Installation failed';
      console.error('Extension installation failed:', error);

      setModalState((prev) => ({
        ...prev,
        error: errorMessage,
        isPending: false,
      }));
    }
  }, [pendingLink, dismissModal, addExtension, setView]);

  useEffect(() => {
    console.log('Setting up extension install modal handler');

    const handleAddExtension = async (_event: IpcRendererEvent, ...args: unknown[]) => {
      const link = args[0] as string;
      await handleExtensionRequest(link);
    };

    return window.electron.on('add-extension', handleAddExtension);
  }, [handleExtensionRequest]);

  const getModalConfig = (): ExtensionModalConfig | null => {
    if (!modalState.extensionInfo) return null;
    return generateModalConfig(modalState.modalType, modalState.extensionInfo);
  };

  const config = getModalConfig();
  if (!config) return null;

  const getConfirmButtonVariant = () => {
    switch (modalState.modalType) {
      case 'blocked':
        return 'secondary';
      case 'untrusted':
        return 'destructive';
      case 'trusted':
      default:
        return 'default';
    }
  };

  const getTitleClassName = () => {
    switch (modalState.modalType) {
      case 'blocked':
        return 'text-text-danger';
      case 'untrusted':
        return 'text-text-warning';
      case 'trusted':
      default:
        return '';
    }
  };

  return (
    <ModalShell
      open={modalState.isOpen}
      onOpenChange={(open) => !open && dismissModal()}
      // A confirmation is width S. While the install is in flight the dialog
      // becomes `required` so no dismissal can orphan it half-done.
      size="sm"
      purpose={modalState.isPending ? 'required' : 'info'}
      title={config.title}
      titleClassName={getTitleClassName()}
      footer={
        config.showSingleButton ? (
          <Button
            onClick={dismissModal}
            disabled={modalState.isPending}
            variant={getConfirmButtonVariant()}
          >
            {config.confirmLabel}
          </Button>
        ) : (
          <>
            <Button variant="secondary" onClick={dismissModal} disabled={modalState.isPending}>
              {config.cancelLabel}
            </Button>
            <Button
              onClick={confirmInstall}
              disabled={modalState.isPending}
              variant={getConfirmButtonVariant()}
            >
              {modalState.isPending ? 'Installing…' : config.confirmLabel}
            </Button>
          </>
        )
      }
    >
      <div className="flex min-w-0 flex-col gap-3 text-left">
        <div className="flex min-w-0 items-center">
          <p className="min-w-0 text-body text-text-default">{config.message}</p>
          {config.help && <InfoTip label="installing this extension" help={config.help} />}
        </div>
        {/* What will run is data, read character by character: mono at the code size, under a
            sans label (design.md D-33, D-42). */}
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-supporting text-text-muted">{config.source.label}</span>
          <code className="min-w-0 font-mono text-code text-text-default [overflow-wrap:anywhere]">
            {config.source.value}
          </code>
        </div>
      </div>
      {/* The install failure was recorded but never shown: the dialog simply
          stayed open with no explanation. Now it also carries the way out — a
          deeplink install fails on the extension's own runtime prerequisites
          (uv, npx, a missing interpreter) more often than on anything Biorouter
          controls. */}
      {modalState.error && (
        <DependencyErrorBanner
          className="mt-3"
          error={modalState.error}
          failure={{
            kind: 'extension',
            name: extractExtensionName(pendingLink ?? '') || 'extension',
            displayName: config.title,
          }}
        />
      )}
    </ModalShell>
  );
}
