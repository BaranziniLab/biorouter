import type { ExtensionConfig, FixedExtensionEntry } from '../components/ConfigContext';
import { Workflow, updateAgentProvider, updateFromSession } from '../api';
import { userActionHeaders } from './userAction';

// Helper function to substitute parameters in text
export const substituteParameters = (text: string, params: Record<string, string>): string => {
  let substitutedText = text;

  for (const key in params) {
    // Escape special characters in the key (parameter) and match optional whitespace
    const regex = new RegExp(`{{\\s*${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*}}`, 'g');
    substitutedText = substitutedText.replace(regex, params[key]);
  }

  return substitutedText;
};

export const initializeSystem = async (
  sessionId: string,
  provider: string,
  model: string,
  _options?: {
    getExtensions?: (b: boolean) => Promise<FixedExtensionEntry[]>;
    addExtension?: (name: string, config: ExtensionConfig, enabled: boolean) => Promise<void>;
    workflowParameters?: Record<string, string> | null;
    workflow?: Workflow;
  }
) => {
  try {
    console.log(
      'initializing agent with provider',
      provider,
      'model',
      model,
      'sessionId',
      sessionId
    );
    await updateAgentProvider({
      body: {
        session_id: sessionId,
        provider,
        model,
      },
      // Issue #56 DR-16: the third `updateAgentProvider` call site, and the one
      // the first pass missed. It has no live caller today (only `App.test.tsx`
      // mocks `initializeSystem`), so this is not a fix to a regression — it is
      // a trap disarmed: revived without the header, this would bind a session
      // from the renderer and be refused by the daemon as though a model had
      // made the call.
      headers: await userActionHeaders(),
      throwOnError: true,
    });

    if (!sessionId) {
      console.log('This will not end well');
    }
    await updateFromSession({
      body: {
        session_id: sessionId,
      },
      headers: await userActionHeaders(),
      throwOnError: true,
    });
  } catch (error) {
    console.error('Failed to initialize agent:', error);
    throw error;
  }
};
