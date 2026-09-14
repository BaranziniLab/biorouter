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

  return <div className="text-xs text-text-muted py-2 text-left">{systemNotification.msg}</div>;
};
