import { memo, useMemo } from 'react';
import type { Message } from '../../api';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import ProgressiveMessageList from '../ProgressiveMessageList';
import { ChatState } from '../../types/chatState';

const isUserRow = (message: Message) => message.role === 'user';

/**
 * A new chat's first message while the chat is created and loaded
 * (`useFirstSend`, vision V2-R8-2), drawn by the transcript's own list, so it
 * is exactly the row the stored message is drawn as.
 *
 * BaseChat swaps this for its transcript list when the store holds the
 * message. The two are different elements, so the stored list mounts: rows a
 * list mounts with are drawn at rest (`useLiveRows`), so the row does not play
 * its entrance a second time. No edit, send-again or activity line: there is
 * no chat to edit in and no turn running yet.
 */
export const FirstSendTranscript = memo(function FirstSendTranscript({
  message,
  sessionId,
  onOpenArtifact,
  workingDir,
}: {
  message: Message;
  sessionId: string;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
}) {
  const messages = useMemo(() => [message], [message]);
  return (
    <ProgressiveMessageList
      messages={messages}
      chat={{ sessionId }}
      isUserMessage={isUserRow}
      chatState={ChatState.Idle}
      onOpenArtifact={onOpenArtifact}
      onRunInTerminal={null}
      workingDir={workingDir}
    />
  );
});
