/**
 * Hub Component
 *
 * The Hub is the main landing page and entry point for the Biorouter Desktop application.
 * It serves as the welcome screen where users can start new conversations.
 *
 * Key Responsibilities:
 * - Displays SessionInsights (greeting + usage heatmap)
 * - Provides a ChatInput for users to start new conversations
 * - Creates a new session and navigates to Pair with the session ID
 * - Shows loading state while session is being created
 *
 * Navigation Flow:
 * Hub (input submission) → Create Session → Pair (with session ID and initial message)
 */

import { useState } from 'react';
import { SessionInsights } from './sessions/SessionsInsights';
import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import 'react-toastify/dist/ReactToastify.css';
import { View, ViewOptions } from '../utils/navigationUtils';
import { useConfig } from './ConfigContext';
import {
  getExtensionConfigsWithOverrides,
  clearExtensionOverrides,
} from '../store/extensionOverrides';
import { getInitialWorkingDir } from '../utils/workingDir';
import { createSession } from '../sessions';
import LoadingBioRouter from './LoadingBioRouter';
import type { UserAttachment } from '../types/message';
import { useConfirmNewChatModel } from './privacy/useConfirmNewChatModel';

export default function Hub({
  setView,
}: {
  setView: (view: View, viewOptions?: ViewOptions) => void;
}) {
  const { extensionsList } = useConfig();
  const [workingDir, setWorkingDir] = useState(getInitialWorkingDir());
  const [isCreatingSession, setIsCreatingSession] = useState(false);
  const confirmNewChatModel = useConfirmNewChatModel();

  const handleSubmit = async (e: React.FormEvent): Promise<boolean | void> => {
    const customEvent = e as unknown as CustomEvent;
    const combinedTextFromInput = customEvent.detail?.value || '';
    const attachments = (customEvent.detail?.attachments ?? []) as UserAttachment[];
    const hasAttachments = attachments.length > 0;

    if ((combinedTextFromInput.trim() || hasAttachments) && !isCreatingSession) {
      // F3. Before anything is consumed — the extension overrides below are
      // cleared as they are read — so a refused send leaves nothing behind but
      // the text, which `ChatInput` puts back when this resolves `false`.
      if (!(await confirmNewChatModel())) return false;

      const extensionConfigs = getExtensionConfigsWithOverrides(extensionsList);
      clearExtensionOverrides();
      setIsCreatingSession(true);

      try {
        const session = await createSession(workingDir, {
          extensionConfigs,
          allExtensions: extensionConfigs.length > 0 ? undefined : extensionsList,
        });

        setView('pair', {
          resumeSessionId: session.id,
          initialMessage: combinedTextFromInput,
          initialAttachments: attachments,
        });
      } catch (error) {
        console.error('Failed to create session:', error);
        setIsCreatingSession(false);
      }

      e.preventDefault();
    }
  };

  return (
    <div className="biorouter-home flex h-full min-h-0 flex-col bg-background-canvas">
      {/* overflow-y-auto is the last-resort guarantee: if a squeeze (tiny
          window + tall composer) leaves less than the heatmap's minimum box,
          the content scrolls instead of clipping or overlapping. */}
      <div className="biorouter-home-content min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
        <SessionInsights />
      </div>

      <div className="biorouter-home-composer shrink-0 px-4 pb-6 sm:px-6">
        <div className="biorouter-composer-view-transition mx-auto w-full max-w-[760px]">
          {isCreatingSession && (
            <div className="pointer-events-none mb-2.5 pl-2">
              <LoadingBioRouter chatState={ChatState.LoadingConversation} />
            </div>
          )}
          <ChatInput
            sessionId={null}
            handleSubmit={handleSubmit}
            chatState={isCreatingSession ? ChatState.LoadingConversation : ChatState.Idle}
            onStop={() => {}}
            initialValue=""
            setView={setView}
            totalTokens={0}
            accumulatedInputTokens={0}
            accumulatedOutputTokens={0}
            droppedFiles={[]}
            onFilesProcessed={() => {}}
            messagesLength={0}
            disableAnimation={false}
            sessionCosts={undefined}
            toolCount={0}
            onWorkingDirChange={setWorkingDir}
          />
        </div>
      </div>
    </div>
  );
}
