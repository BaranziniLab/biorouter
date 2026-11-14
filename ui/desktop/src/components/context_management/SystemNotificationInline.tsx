import React from 'react';
import { Message, SystemNotificationContent } from '../../api';
import { ChatTurnStopped } from '../conversation/ChatTurnStopped';
import { isTurnStoppedNotice } from '../conversation/turnStoppedNotice';
import { usageLimitNoticeKind } from '../conversation/usageLimitNotice';
import { Disclosure } from '../ui/disclosure';
import { InfoTip } from '../ui/info-tip';
import { foldForHelp } from '../../utils/proseFold';

/** The info icon's name beside a folded notice: "About this notice". */
const NOTICE_TIP_NAME = 'this notice';

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
  // notice reads as a line of the chat rather than a footnote. A usage-limit
  // notice draws the same line and is marked with its kind, so tests and
  // styling can find it without matching the text.
  //
  // A notice that runs past one line keeps its first sentence on the row (the
  // reason: "Monthly usage limit reached: …", "Nothing ran: …") and folds what
  // follows, the guidance, behind the info icon beside it, or under Details when
  // it is longer than a tip holds (owner, 2026-11-13: no solid chunk of text).
  const { lead, tip, details } = foldForHelp(systemNotification.msg);
  const row = (
    <div
      className="flex min-h-7 items-center gap-1 text-left text-secondary text-text-muted"
      data-usage-limit-notice={usageLimitNoticeKind(message) ?? undefined}
    >
      {lead}
      {tip ? <InfoTip label={NOTICE_TIP_NAME} help={tip} /> : null}
    </div>
  );
  if (!details) return row;
  return (
    <div className="flex flex-col">
      {row}
      <Disclosure label="Details">
        <p className="text-secondary text-text-muted">{details}</p>
      </Disclosure>
    </div>
  );
};
