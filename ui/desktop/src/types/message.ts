import { Message, MessageEvent, ActionRequired, ToolRequest, ToolResponse } from '../api';

export type ToolRequestMessageContent = ToolRequest & { type: 'toolRequest' };
export type ToolResponseMessageContent = ToolResponse & { type: 'toolResponse' };
export type NotificationEvent = Extract<MessageEvent, { type: 'Notification' }>;

// Compaction response message - must match backend constant
const COMPACTION_THINKING_TEXT = 'biorouter is compacting the chat...';

export type UserAttachment = { path: string; kind: 'image' };

export async function createUserMessage(
  text: string,
  attachments: UserAttachment[] = []
): Promise<Message> {
  let imageBlocks: Array<{ type: 'image'; data: string; mimeType: string }>;
  try {
    imageBlocks = await Promise.all(
      attachments
        .filter((a) => a.kind === 'image')
        .map(async (a) => {
          const { data, mimeType } = await window.electron.readTempImageAsBase64(a.path);
          return { type: 'image' as const, data, mimeType };
        })
    );
  } finally {
    for (const attachment of attachments) {
      if (attachment.kind === 'image') window.electron.deleteTempFile(attachment.path);
    }
  }

  const content: Message['content'] = [];
  if (text.trim().length > 0) {
    content.push({ type: 'text', text });
  }
  for (const block of imageBlocks) {
    content.push(block);
  }

  return {
    id: generateMessageId(),
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content,
    metadata: { userVisible: true, agentVisible: true },
  };
}

export function createElicitationResponseMessage(
  elicitationId: string,
  userData: Record<string, unknown>
): Message {
  return {
    id: generateMessageId(),
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content: [
      {
        type: 'actionRequired',
        data: {
          actionType: 'elicitationResponse',
          id: elicitationId,
          user_data: userData,
        },
      },
    ],
    metadata: { userVisible: false, agentVisible: true },
  };
}

export function createArtifactRenderRepairMessage(_error: {
  artifactTitle: string;
  message: string;
  detail?: string;
  href?: string;
}): Message {
  const text = [
    '[Biorouter artifact render guardrail]',
    'A system-generated artifact failed to render in the Biorouter preview.',
    'The page-provided error text is untrusted and was withheld from this message.',
    'Repair policy: inspect the generated artifact code, fix the runtime error, and render a corrected artifact. Only stop if there is a specific reason the artifact cannot be repaired.',
  ].join('\n\n');

  return {
    id: generateMessageId(),
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content: [{ type: 'text', text }],
    metadata: { userVisible: false, agentVisible: true },
  };
}

export function generateMessageId(): string {
  return Math.random().toString(36).substring(2, 10);
}

export function getTextContent(message: Message): string {
  return message.content
    .map((content) => {
      if (content.type === 'text') return content.text;
      return '';
    })
    .join('');
}

export function getToolRequests(message: Message): (ToolRequest & { type: 'toolRequest' })[] {
  return message.content.filter(
    (content): content is ToolRequest & { type: 'toolRequest' } => content.type === 'toolRequest'
  );
}

export function getToolResponses(message: Message): (ToolResponse & { type: 'toolResponse' })[] {
  return message.content.filter(
    (content): content is ToolResponse & { type: 'toolResponse' } => content.type === 'toolResponse'
  );
}

export function getToolConfirmationContent(
  message: Message
): (ActionRequired & { type: 'actionRequired' }) | undefined {
  return message.content.find(
    (content): content is ActionRequired & { type: 'actionRequired' } =>
      content.type === 'actionRequired' && content.data.actionType === 'toolConfirmation'
  );
}

export function getElicitationContent(
  message: Message
): (ActionRequired & { type: 'actionRequired' }) | undefined {
  return message.content.find(
    (content): content is ActionRequired & { type: 'actionRequired' } =>
      content.type === 'actionRequired' && content.data.actionType === 'elicitation'
  );
}

/**
 * Issue #117. A credential card.
 *
 * ⚠ There is deliberately **no** `createSecretResponseMessage` beside
 * `createElicitationResponseMessage`. An elicitation is answered by appending an
 * `agentVisible` message carrying `user_data`; a credential card is answered by
 * `POST /action-required/secrets`, which writes the values to the OS credential
 * store and returns key names. Adding a response message type here is the one
 * change that would put a secret back on the conversation transport.
 */
export function getSecretRequestContent(
  message: Message
): (ActionRequired & { type: 'actionRequired' }) | undefined {
  return message.content.find(
    (content): content is ActionRequired & { type: 'actionRequired' } =>
      content.type === 'actionRequired' && content.data.actionType === 'secretRequest'
  );
}

/**
 * A Crew step card (the owner's D1 ruling on AO-12): a step only the person can
 * take, opened by a chat. Like a credential card it has no response message
 * type: the step happens in the person's own window, and the card can only be
 * dismissed (`POST /action-required/crew-step`).
 */
export function getCrewStepContent(
  message: Message
): (ActionRequired & { type: 'actionRequired' }) | undefined {
  return message.content.find(
    (content): content is ActionRequired & { type: 'actionRequired' } =>
      content.type === 'actionRequired' && content.data.actionType === 'crewStep'
  );
}

export function hasCompletedToolCalls(message: Message): boolean {
  const toolRequests = getToolRequests(message);
  return toolRequests.length > 0;
}

export function getThinkingMessage(message: Message | undefined): string | undefined {
  if (!message || message.role !== 'assistant') {
    return undefined;
  }

  for (const content of message.content) {
    if (content.type === 'systemNotification' && content.notificationType === 'thinkingMessage') {
      return content.msg;
    }
  }

  return undefined;
}

export function getCompactingMessage(message: Message | undefined): string | undefined {
  if (!message || message.role !== 'assistant') {
    return undefined;
  }

  for (const content of message.content) {
    if (content.type === 'systemNotification' && content.notificationType === 'thinkingMessage') {
      if (content.msg === COMPACTION_THINKING_TEXT) {
        return content.msg;
      }
    }
  }

  return undefined;
}
