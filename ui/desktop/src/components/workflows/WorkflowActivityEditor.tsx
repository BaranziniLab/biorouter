import { useState, useEffect } from 'react';
import { X } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Field } from '../ui/field';
import { InfoTip } from '../ui/info-tip';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { workflowFormCopy } from './copy';

const copy = workflowFormCopy.activities;

/** Longer starter labels are cut on their row; the whole label is the row's tooltip. */
const CHIP_MAX_CHARS = 100;

/**
 * The workflow's opening: an intro message (`message:` activity) and starter buttons (every
 * other activity). Two fields under one group label, each explained by an InfoTip rather than
 * a paragraph, on the shared field skin (`Input`, `Textarea`, `Field`).
 */
export default function WorkflowActivityEditor({
  activities = [],
  setActivities,
  onBlur,
}: {
  activities?: string[];
  setActivities: (prev: string[]) => void;
  onBlur?: () => void;
}) {
  const [newActivity, setNewActivity] = useState('');
  const [messageContent, setMessageContent] = useState('');

  // Extract message content from activities on component mount and when activities change
  useEffect(() => {
    const messageActivity = activities.find((activity) =>
      activity.toLowerCase().startsWith('message:')
    );
    if (messageActivity) {
      setMessageContent(messageActivity.replace(/^message:/i, ''));
    }
  }, [activities]);

  // Get activities that are not messages
  const nonMessageActivities = activities.filter(
    (activity) => !activity.toLowerCase().startsWith('message:')
  );

  const handleAddActivity = () => {
    if (newActivity.trim()) {
      setActivities([...activities, newActivity.trim()]);
      setNewActivity('');
      // Trigger parameter extraction after adding activity
      if (onBlur) {
        onBlur();
      }
    }
  };

  const handleRemoveActivity = (activity: string) => {
    setActivities(activities.filter((a) => a !== activity));
  };

  const handleMessageChange = (value: string) => {
    setMessageContent(value);

    // Update activities array - remove existing message and add new one if not empty
    const otherActivities = activities.filter(
      (activity) => !activity.toLowerCase().startsWith('message:')
    );

    if (value.length > 0) {
      setActivities([`message:${value}`, ...otherActivities]);
    } else {
      setActivities(otherActivities);
    }
  };

  return (
    <div className="flex flex-col gap-3" role="group" aria-labelledby="workflow-activities-label">
      <div className="flex min-w-0 items-center">
        <span id="workflow-activities-label" className="text-label text-text-default">
          {copy.title}
        </span>
        <InfoTip label={copy.title} help={copy.info} />
      </div>

      <Field id="workflow-intro-message" label={copy.message} info={copy.messageInfo}>
        <Textarea
          value={messageContent}
          onChange={(e) => handleMessageChange(e.target.value)}
          onBlur={onBlur}
          placeholder={copy.messagePlaceholder}
          rows={3}
          className="resize-none"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck="false"
        />
      </Field>

      <div className="flex flex-col gap-1.5">
        <div className="flex min-w-0 items-center">
          <label htmlFor="workflow-starter-button" className="text-label text-text-default">
            {copy.buttons}
          </label>
          <InfoTip label={copy.buttons} help={copy.buttonsInfo} />
        </div>

        {/* Rows, not chips: each starter button is one line with its own Remove. */}
        {nonMessageActivities.length > 0 && (
          <ul className="flex flex-col">
            {nonMessageActivities.map((activity, index) => {
              const cut = activity.length > CHIP_MAX_CHARS;
              return (
                <li
                  key={index}
                  className="flex min-h-8 min-w-0 items-center gap-2 rounded-element pl-2 hover:bg-overlay-hover"
                  title={cut ? activity : undefined}
                >
                  <span className="min-w-0 flex-1 truncate text-body text-text-default">
                    {cut ? `${activity.slice(0, CHIP_MAX_CHARS)}…` : activity}
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    shape="round"
                    size="sm"
                    className="text-text-muted"
                    onClick={() => handleRemoveActivity(activity)}
                    aria-label={copy.remove(activity)}
                  >
                    <X aria-hidden />
                  </Button>
                </li>
              );
            })}
          </ul>
        )}

        <div className="flex gap-2">
          <Input
            id="workflow-starter-button"
            type="text"
            value={newActivity}
            onChange={(e) => setNewActivity(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                handleAddActivity();
              }
            }}
            onBlur={onBlur}
            placeholder={copy.buttonPlaceholder}
          />
          <Button
            type="button"
            variant="secondary"
            onClick={handleAddActivity}
            disabled={!newActivity.trim()}
          >
            {copy.add}
          </Button>
        </div>
      </div>
    </div>
  );
}
