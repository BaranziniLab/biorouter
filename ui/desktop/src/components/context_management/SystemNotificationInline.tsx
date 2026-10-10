import React from 'react';
import { Message, SystemNotificationContent } from '../../api';
import { ChatTurnStopped } from '../conversation/ChatTurnStopped';
import { isTurnStoppedNotice } from '../conversation/turnStoppedNotice';

interface SystemNotificationInlineProps {
  message: Message;
}

export const SystemNotificationInline: React.FC<SystemNotificationInlineProps> = ({ message }) => {
  // Item 7: a stopped turn's stored notice is the same line a confirmed Stop
  // draws live, so a reload, another window and History read it the same way.
  if (isTurnStoppedNotice(message)) {
    return <ChatTurnStopped inTranscript />;
  }

  const systemNotification = message.content.find(
    (content): content is SystemNotificationContent & { type: 'systemNotification' } =>
      content.type === 'systemNotification' && content.notificationType === 'inlineMessage'
  );

  if (!systemNotification?.msg) {
    return null;
  }

  // The transcript row's type and rhythm (a 28px line in muted 13px ink), so a
  // notice reads as a line of the chat rather than a footnote.
  return (
    <div className="flex min-h-7 items-center text-left text-secondary text-text-muted">
      {systemNotification.msg}
    </div>
  );
};
