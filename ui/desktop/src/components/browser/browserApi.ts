import { browserUseConsent, browserUseRevoke, browserUseStatus } from '../../api/sdk.gen';
import type { ActLifetime, BrowserGrant, BrowserUseStatus } from '../../api/types.gen';
import { userActionHeaders } from '../../utils/userAction';

/**
 * The Biorouter Browser's consent routes (BB-17), as the approval card uses them.
 *
 * Every call carries the user-action proof: the consent route requires it, so neither a model
 * nor the plain API secret can allow the browser.
 */

export type BrowserStatus = BrowserUseStatus;

/** A 409 from the status poll: this chat's mode never runs the browser, so polling stops. */
export class BrowserNotApplicable extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'BrowserNotApplicable';
  }
}

export async function browserStatus(sessionId: string): Promise<BrowserStatus> {
  const result = await browserUseStatus({
    query: { session_id: sessionId },
    headers: await userActionHeaders(),
  });
  if (result.error !== undefined || result.data === undefined) {
    const detail = result.error as { message?: string } | undefined;
    const message = detail?.message ?? 'Biorouter Browser status unavailable';
    if (result.response?.status === 409) throw new BrowserNotApplicable(message);
    throw new Error(message);
  }
  return result.data;
}

export async function allowBrowser(
  status: BrowserStatus,
  grant: BrowserGrant,
  lifetime: ActLifetime
): Promise<BrowserStatus> {
  const response = await browserUseConsent({
    headers: await userActionHeaders(),
    body: {
      session_id: status.session_id,
      challenge_id: status.challenge_id,
      grant,
      lifetime,
    },
    throwOnError: true,
  });
  return response.data;
}

/** Stop and Cancel: the one Stop for both agent capabilities (BB-18). */
export async function stopBrowser(sessionId: string): Promise<BrowserStatus> {
  const response = await browserUseRevoke({
    headers: await userActionHeaders(),
    body: { session_id: sessionId },
    throwOnError: true,
  });
  return response.data;
}
