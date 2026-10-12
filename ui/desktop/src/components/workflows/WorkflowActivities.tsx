import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import MarkdownContent from '../MarkdownContent';
import { substituteParameters } from '../../utils/providerUtils';

interface WorkflowActivitiesProps {
  append: (text: string) => void;
  activities: string[] | null;
  title?: string;
  parameterValues?: Record<string, string>;
}

/** A starter longer than this is cut, with the whole prompt in its tooltip. */
const STARTER_MAX_CHARS = 60;

/**
 * A workflow chat's opening: the workflow's own message, if it has one, as
 * prose in the column, then its starter prompts as one row of secondary
 * buttons. Each starter is a real button (it used to be a clickable card no
 * keyboard could reach), and a cut one shows the whole prompt in its tooltip
 * rather than in a `title`. The group enters once with the shared `.br-enter`.
 */
export default function WorkflowActivities({
  append,
  activities,
  parameterValues = {},
}: WorkflowActivitiesProps) {
  const pills = activities || [];

  // Find any pill that starts with "message:"
  const messagePillIndex = pills.findIndex((pill) => pill.toLowerCase().startsWith('message:'));

  // Extract the message pill and the remaining pills
  const messagePill = messagePillIndex >= 0 ? pills[messagePillIndex] : null;
  const remainingPills =
    messagePillIndex >= 0
      ? [...pills.slice(0, messagePillIndex), ...pills.slice(messagePillIndex + 1)]
      : pills;

  // If we have activities or instructions (workflow mode), show a simplified version without greeting
  if (activities && activities.length > 0) {
    return (
      <div className="br-enter flex flex-col gap-4">
        {messagePill && (
          <MarkdownContent
            content={substituteParameters(
              messagePill.replace(/^message:/i, '').trim(),
              parameterValues
            )}
            className="text-body"
          />
        )}

        {remainingPills.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {remainingPills.map((content, index) => {
              const prompt = substituteParameters(content, parameterValues);
              const cut = prompt.length > STARTER_MAX_CHARS;
              const button = (
                <Button
                  key={index}
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => append(prompt)}
                >
                  {cut ? `${prompt.slice(0, STARTER_MAX_CHARS)}…` : prompt}
                </Button>
              );
              if (!cut) return button;
              return (
                <Tooltip key={index}>
                  <TooltipTrigger asChild>{button}</TooltipTrigger>
                  <TooltipContent className="max-w-[320px] whitespace-normal">
                    {prompt}
                  </TooltipContent>
                </Tooltip>
              );
            })}
          </div>
        )}
      </div>
    );
  }

  return null;
}
