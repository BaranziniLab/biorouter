import React from 'react';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';

const EnvironmentBadge: React.FC = () => {
  const isAlpha = process.env.ALPHA;
  const isDevelopment = import.meta.env.DEV;

  // Don't show badge in production
  if (!isDevelopment && !isAlpha) {
    return null;
  }

  // A coloured dot has no visible label, so this text IS the control's name.
  // Naming it "Alpha" told the user only what they could already guess from the
  // colour, so it says what the build means for them instead.
  const tooltipText = isAlpha
    ? 'Alpha build: experimental features are on'
    : 'Development build: running from source, not a release';
  const bgColor = isAlpha ? 'bg-background-info' : 'bg-background-warning';

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className={`${bgColor} w-3 h-3 rounded-full cursor-default`}
          data-testid="environment-badge"
          role="img"
          aria-label={tooltipText}
        />
      </TooltipTrigger>
      <TooltipContent side="right">{tooltipText}</TooltipContent>
    </Tooltip>
  );
};

export default EnvironmentBadge;
