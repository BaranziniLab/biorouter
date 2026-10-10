import React from 'react';
import { Button } from '../../../../ui/button';
import clsx from 'clsx';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../../../ui/Tooltip';
import { catalogCopy } from '../../copy';

interface ActionButtonProps extends React.ComponentProps<typeof Button> {
  /** Icon component to render, e.g. `RefreshCw` from lucide-react */
  icon?: React.ComponentType<React.SVGProps<globalThis.SVGSVGElement>>;
  /** Tooltip text to show; optional if you want no tooltip. */
  tooltip?: React.ReactNode;
  /** Additional classes for styling. */
  className?: string;
  /** Text to display next to the icon */
  text?: string;
  /** Additional class for the icon specifically */
  iconClassName?: string;
}

export function ActionButton({
  icon: Icon,
  size = 'sm',
  variant = 'secondary',
  tooltip,
  className,
  text,
  iconClassName,
  ...props
}: ActionButtonProps) {
  const ButtonElement = (
    <Button
      size={size}
      variant={variant}
      shape={text ? 'pill' : 'round'}
      className={className}
      {...props}
    >
      {Icon && <Icon className={clsx('!size-4', iconClassName)} />}
      {text && <span>{text}</span>}
    </Button>
  );

  if (tooltip) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>{ButtonElement}</TooltipTrigger>
        <TooltipContent side="top">{tooltip}</TooltipContent>
      </Tooltip>
    );
  }

  return ButtonElement;
}

/** A row's "Configure": a secondary `sm` text button. The row names it with `aria-label`. */
export function ConfigureSettingsButton({ tooltip, className, text, ...props }: ActionButtonProps) {
  return (
    <ActionButton
      tooltip={tooltip}
      variant="secondary"
      text={text ?? catalogCopy.configure}
      className={clsx('cursor-pointer', className)}
      {...props}
    />
  );
}

/** First run's "Launch" on a configured provider. */
export function RocketButton({ tooltip, className, text, ...props }: ActionButtonProps) {
  return (
    <ActionButton
      data-testid="provider-launch-button"
      tooltip={tooltip}
      variant="secondary"
      text={text ?? catalogCopy.launch}
      className={clsx('cursor-pointer', className)}
      {...props}
    />
  );
}
