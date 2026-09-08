import { safeJsonParse } from './utils/conversionUtils';
import { Message, MessageMetadata } from './api';

export interface SharedSessionDetails {
  share_token: string;
  created_at: number;
  base_url: string;
  description: string;
  working_dir: string;
  messages: Message[];
  message_count: number;
  total_tokens: number | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function normalizeSharedMessage(message: Record<string, unknown>): Message {
  const metadata = isRecord(message.metadata) ? message.metadata : {};
  return {
    ...(message as unknown as Message),
    // Only the two CONTAINER fields are filled in, because they are the only
    // ones a consumer dereferences. A missing scalar (`created`, `role`, `id`)
    // degrades a label rather than throwing — `formatMessageTimestamp` already
    // takes an optional — and inventing one would disguise a malformed payload
    // instead of surviving it.
    content: Array.isArray(message.content) ? (message.content as Message['content']) : [],
    metadata: {
      ...(metadata as Partial<MessageMetadata>),
      // A shared transcript is what the sharer chose to publish, so a message
      // arriving without a visibility flag is SHOWN, and hidden only when the
      // payload says so explicitly. Defaulting to `false` would trade the error
      // boundary for a silently truncated transcript, which is worse: the
      // reader cannot tell that anything is missing.
      userVisible: metadata.userVisible !== false,
      agentVisible: metadata.agentVisible !== false,
    },
  };
}

/**
 * Fill in the shape the generated `Message` type promises, for the one
 * `Message[]` in the renderer that does not come from the local daemon.
 *
 * `Message` declares `content` and `metadata` as REQUIRED, so every consumer
 * downstream — `getTextContent`, `isUserMessage`, `localOrigin` in
 * `artifacts/artifactFileProvenance.ts`, both transcript renderers — reads them
 * without a guard, and is right to. But `fetchSharedSessionDetails` reads this
 * payload from an operator-configured remote `base_url` and `safeJsonParse`
 * *asserts* the type rather than checking it, so a server on an older schema
 * (or any malformed response) hands the renderer a message that violates it.
 * Measured: a transcript whose messages carried no `metadata` replaced the
 * whole page with the app's error boundary ("Cannot read properties of
 * undefined (reading 'provenance')").
 *
 * Filling the shape once, here, is what keeps that promise true for every
 * consumer. Guarding each consumer instead is an open-ended list that the next
 * consumer written against the type would not join.
 */
export function normalizeSharedMessages(messages: unknown): Message[] {
  if (!Array.isArray(messages)) return [];
  return messages.filter(isRecord).map(normalizeSharedMessage);
}

/**
 * Fetches details for a specific shared session
 * @param baseUrl The base URL for session sharing API
 * @param shareToken The share token of the session to fetch
 * @returns Promise with shared session details
 */
export async function fetchSharedSessionDetails(
  baseUrl: string,
  shareToken: string
): Promise<SharedSessionDetails> {
  try {
    const response = await fetch(`${baseUrl}/sessions/share/${shareToken}`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json',
        // Origin: 'http://localhost:5173', // required to bypass Cloudflare security filter
      },
      credentials: 'include',
    });

    if (!response.ok) {
      throw new Error(`Could not load the shared chat: ${response.status} ${response.statusText}`);
    }

    const data = await safeJsonParse<SharedSessionDetails>(
      response,
      'Could not read the shared chat'
    );

    if (baseUrl != data.base_url) {
      throw new Error(`Base URL mismatch for the shared chat: ${baseUrl} != ${data.base_url}`);
    }

    return {
      share_token: data.share_token,
      created_at: data.created_at,
      base_url: data.base_url,
      description: data.description,
      working_dir: data.working_dir,
      messages: normalizeSharedMessages(data.messages),
      message_count: data.message_count,
      total_tokens: data.total_tokens,
    };
  } catch (error) {
    console.error('Error fetching shared session:', error);
    throw error;
  }
}

/**
 * Creates a new shared session
 * @param baseUrl The base URL for session sharing API
 * @param workingDir The working directory for the shared session
 * @param messages The messages to include in the shared session
 * @param description Description for the shared session
 * @param totalTokens Total token count for the session, or null if not available
 * @param userName The user name for who is sharing the session
 * @returns Promise with the share token
 */
export async function createSharedSession(
  baseUrl: string,
  workingDir: string,
  messages: Message[],
  description: string,
  totalTokens: number | null
): Promise<string> {
  try {
    const response = await fetch(`${baseUrl}/sessions/share`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        working_dir: workingDir,
        messages,
        description: description,
        base_url: baseUrl,
        total_tokens: totalTokens ?? null,
      }),
    });

    if (!response.ok) {
      if (response.status === 302) {
        throw new Error(
          `Could not create the share link. Check that you are on the VPN. ${response.status} ${response.statusText}`
        );
      }
      throw new Error(`Could not create the share link: ${response.status} ${response.statusText}`);
    }

    const data = await safeJsonParse<{ share_token: string }>(
      response,
      'Could not read the share-link response'
    );
    return data.share_token;
  } catch (error) {
    console.error('Error creating shared session:', error);
    throw error;
  }
}
