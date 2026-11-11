import { Session, startAgent, ExtensionConfig } from './api';
import type { setViewType } from './hooks/useNavigation';
import {
  getExtensionConfigsWithOverrides,
  clearExtensionOverrides,
  hasExtensionOverrides,
} from './store/extensionOverrides';
import type { FixedExtensionEntry } from './components/ConfigContext';
import { userActionHeaders } from './utils/userAction';
import {
  PublicChatDeclinedError,
  decisionSettingsToWire,
  parsePublicDecisionConfirmation,
  type DecisionSettings,
  type PublicDecisionConfirmation,
} from './utils/sessionDecisions';

export { PublicChatDeclinedError };

/**
 * Asks the person whether a chat whose decision models make it public should start.
 * `true` starts it with `acceptPublicDecisionModels`.
 */
export type ConfirmPublicDecisionModels = (
  confirmation: PublicDecisionConfirmation
) => Promise<boolean>;

export function resumeSession(session: Session, setView: setViewType) {
  setView('pair', {
    disableAnimation: true,
    resumeSessionId: session.id,
  });
}

export async function createSession(
  workingDir: string,
  options?: {
    workflowId?: string;
    workflowDeeplink?: string;
    extensionConfigs?: ExtensionConfig[];
    allExtensions?: FixedExtensionEntry[];
    /**
     * The chat's own model, bound by the start itself. Sent with a draft's decision
     * settings, so what they do to the chat's tier is worked out against the model
     * the chat will run on, not the app-wide one the start would otherwise bind.
     */
    provider?: string;
    model?: string;
    /** DR-33 (design §4): decision settings saved before the first message, in the same request. */
    decisionSettings?: DecisionSettings | null;
    /** The person already saw "This chat will be public: …" for these settings and accepted it. */
    acceptPublicDecisionModels?: boolean;
    /**
     * Asked when the daemon answers that the start would make the chat public and
     * the request did not say the person accepted it (409
     * `needsPublicDecisionConfirmation`; a workflow's own decision models ask the
     * same). Yes starts again with the acceptance; no, or no asker, throws
     * {@link PublicChatDeclinedError} (the daemon discarded the chat when it asked).
     */
    confirmPublicDecisionModels?: ConfirmPublicDecisionModels;
  }
): Promise<Session> {
  const body: {
    working_dir: string;
    workflow_id?: string;
    workflow_deeplink?: string;
    extension_overrides?: ExtensionConfig[];
    provider?: string;
    model?: string;
    decision_settings?: Record<string, unknown>;
    accept_public_decision_models?: boolean;
  } = {
    working_dir: workingDir,
  };
  if (options?.provider && options.model) {
    body.provider = options.provider;
    body.model = options.model;
  }
  if (options?.decisionSettings) {
    // The parsers' own wire writer, which mirrors the Rust enums.
    body.decision_settings = decisionSettingsToWire(options.decisionSettings);
  }
  if (options?.acceptPublicDecisionModels) body.accept_public_decision_models = true;

  if (options?.workflowId) {
    body.workflow_id = options.workflowId;
  } else if (options?.workflowDeeplink) {
    body.workflow_deeplink = options.workflowDeeplink;
  }

  if (options?.extensionConfigs && options.extensionConfigs.length > 0) {
    body.extension_overrides = options.extensionConfigs;
  } else if (options?.allExtensions) {
    const extensionConfigs = getExtensionConfigsWithOverrides(options.allExtensions);
    if (extensionConfigs.length > 0) {
      body.extension_overrides = extensionConfigs;
    }
    if (hasExtensionOverrides()) {
      clearExtensionOverrides();
    }
  }

  const start = async (sent: typeof body) =>
    (
      await startAgent({
        // `decision_settings` is the parsers' wire shape, newer than the generated body type.
        body: sent as Parameters<typeof startAgent>[0]['body'],
        headers: await userActionHeaders(),
        throwOnError: true,
      })
    ).data;

  try {
    return await start(body);
  } catch (error) {
    const confirmation = body.accept_public_decision_models
      ? null
      : parsePublicDecisionConfirmation(error);
    if (!confirmation) throw error;
    const accepted = options?.confirmPublicDecisionModels
      ? await options.confirmPublicDecisionModels(confirmation)
      : false;
    if (!accepted) throw new PublicChatDeclinedError();
    return await start({ ...body, accept_public_decision_models: true });
  }
}

export async function startNewSession(
  workingDir: string,
  initialText: string | undefined,
  setView: setViewType,
  options?: {
    workflowId?: string;
    workflowDeeplink?: string;
    allExtensions?: FixedExtensionEntry[];
  }
): Promise<Session> {
  const session = await createSession(workingDir, options);

  setView('pair', {
    disableAnimation: true,
    initialMessage: initialText,
    resumeSessionId: session.id,
  });

  return session;
}
