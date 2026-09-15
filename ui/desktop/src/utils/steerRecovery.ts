import type { Message } from '../api';

export interface SteerRecovery {
  message: Message;
  afterMessageId?: string;
  index: number;
}

/** Recovery rows belong to the display only, never to server edit/writeback history. */
export function withSteerRecoveries(messages: Message[], recoveries: SteerRecovery[] = []): Message[] {
  const result = [...messages];
  for (const recovery of recoveries) {
    if (result.some((message) => message.id === recovery.message.id)) continue;
    const anchor = recovery.afterMessageId ? result.findIndex((message) => message.id === recovery.afterMessageId) : -1;
    result.splice(anchor >= 0 ? anchor + 1 : Math.min(recovery.index, result.length), 0, recovery.message);
  }
  return result;
}
