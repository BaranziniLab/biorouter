import React from 'react';
import { cn } from '../utils';

export type ToolCallStatus = 'pending' | 'loading' | 'success' | 'error';

interface ToolCallStatusIndicatorProps {
  status: ToolCallStatus;
  className?: string;
}

export const ToolCallStatusIndicator: React.FC<ToolCallStatusIndicatorProps> = ({
  status,
  className,
}) => {
  return (
    <span role="img" className={cn('sr-only', className)} aria-label={`Tool status: ${status}`} />
  );
};

/**
 * Keeps status accessible while the visible tool icon stays undecorated.
 */
interface ToolIconWithStatusProps {
  ToolIcon: React.ComponentType<{ className?: string }>;
  status: ToolCallStatus;
  className?: string;
}

export const ToolIconWithStatus: React.FC<ToolIconWithStatusProps> = ({
  ToolIcon,
  status,
  className,
}) => {
  return (
    <span className={cn('inline-flex shrink-0', className)}>
      <ToolIcon className="h-4 w-4 flex-shrink-0" />
      <ToolCallStatusIndicator status={status} />
    </span>
  );
};
