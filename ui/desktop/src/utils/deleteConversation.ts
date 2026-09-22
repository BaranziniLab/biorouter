import { deleteSession } from '../api';
import { userActionHeaders } from './userAction';
import { notifySessionListChanged, updateCachedSessionList } from './sessionListCache';

export async function deleteConversation(sessionId: string): Promise<void> {
  await deleteSession({
    path: { session_id: sessionId },
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  updateCachedSessionList((sessions) => sessions.filter((session) => session.id !== sessionId));
  notifySessionListChanged({ removed: sessionId });
}
