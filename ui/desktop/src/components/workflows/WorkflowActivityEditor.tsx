import { useState, useEffect } from 'react';
import { Button } from '../ui/button';

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
    <div>
      <label htmlFor="activities" className="block text-label text-text-default mb-1">
        Activities
      </label>
      <p className="text-supporting text-text-muted mb-4">
        The top-line prompts and activity buttons that will display in the workflow chat window.
      </p>

      {/* Message Field */}
      <div className="mb-4">
        <label htmlFor="message" className="block text-caps text-text-muted mb-1">
          Message
        </label>
        <p className="text-supporting text-text-muted mb-2">
          Appears at the top of the workflow. Supports markdown formatting.
        </p>
        <textarea
          id="message"
          value={messageContent}
          onChange={(e) => handleMessageChange(e.target.value)}
          onBlur={onBlur}
          className="w-full px-3 py-2 text-body border border-border-subtle rounded-element bg-background-default text-text-default placeholder:text-text-muted focus:border-border-strong transition-colors resize-none"
          placeholder="Enter a user facing introduction message for your workflow (supports **bold**, *italic*, `code`, etc.)"
          rows={3}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck="false"
        />
      </div>

      {/* Activity Buttons */}
      <div className="space-y-3">
        <div>
          <label className="block text-caps text-text-muted mb-1">Activity Buttons</label>
          <p className="text-supporting text-text-muted mb-3">
            Clickable buttons that appear below the message to help users interact with your
            workflow.
          </p>
        </div>

        {nonMessageActivities.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {nonMessageActivities.map((activity, index) => (
              <div
                key={index}
                className="inline-flex items-center gap-1.5 bg-background-medium border border-border-subtle rounded-inner px-3 py-1.5 text-body text-text-default"
                title={activity.length > 100 ? activity : undefined}
              >
                <span>{activity.length > 100 ? activity.slice(0, 100) + '…' : activity}</span>
                <button
                  type="button"
                  onClick={() => handleRemoveActivity(activity)}
                  className="text-text-muted hover:text-text-danger transition-colors leading-none"
                  aria-label={`Remove ${activity}`}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}

        <div className="flex gap-2">
          <input
            type="text"
            value={newActivity}
            onChange={(e) => setNewActivity(e.target.value)}
            onKeyPress={(e) => e.key === 'Enter' && handleAddActivity()}
            onBlur={onBlur}
            className="flex-1 h-9 px-3 text-body border border-border-subtle rounded-element bg-background-default text-text-default placeholder:text-text-muted focus:border-border-strong transition-colors"
            placeholder="Add activity button label…"
          />
          <Button
            type="button"
            onClick={handleAddActivity}
            disabled={!newActivity.trim()}
            variant="outline"
            size="sm"
          >
            Add
          </Button>
        </div>
      </div>
    </div>
  );
}
