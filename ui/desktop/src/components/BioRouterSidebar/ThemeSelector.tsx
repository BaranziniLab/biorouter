import React from 'react';
import { Monitor, Moon, Sun } from '../icons/app-icons';
import { Button } from '../ui/button';
import { cn } from '../../utils';
import { useTheme } from '../../contexts/ThemeContext';

interface ThemeSelectorProps {
  className?: string;
  horizontal?: boolean;
}

/**
 * Light / dark / system — one of the two theme axes, and the sibling of
 * {@link ThemeFamilySelector}. The two render the same segmented shape
 * deliberately, so keep them in step.
 *
 * ⚠ **Selection is achromatic** (astryx A-06): the active arm is
 * `tint-selected tint-interactive`, a neutral wash the app composites the same
 * way everywhere, not `bg-background-accent` + `border-border-accent`. Three
 * coral-filled buttons above three coral-filled family buttons made the
 * Appearance section the loudest thing in Settings, and the accent is reserved
 * for CTAs. The composed pair is also what lets the ACTIVE arm answer the
 * pointer — `tint-interactive` alone would lighten a selected row, so the old
 * `hover:!bg-background-accent` override existed to freeze it, which meant the
 * selected button silently stopped responding to hover at all.
 *
 * `aria-pressed` carries the state. It had none: the selection was colour and
 * nothing else, so a screen reader was told three identical buttons.
 */
const ThemeSelector: React.FC<ThemeSelectorProps> = ({ className = '', horizontal = false }) => {
  const { userThemePreference, setUserThemePreference } = useTheme();

  const options = [
    { value: 'light' as const, label: 'Light', Icon: Sun, testId: 'light-mode-button' },
    { value: 'dark' as const, label: 'Dark', Icon: Moon, testId: 'dark-mode-button' },
    { value: 'system' as const, label: 'System', Icon: Monitor, testId: 'system-mode-button' },
  ];

  return (
    <div className={`${!horizontal ? 'px-1 py-2 space-y-2' : ''} ${className}`}>
      <div
        className={`${horizontal ? 'flex' : 'grid grid-cols-3'} gap-1 ${!horizontal ? 'px-3' : ''}`}
      >
        {options.map(({ value, label, Icon, testId }) => {
          const active = userThemePreference === value;
          return (
            <Button
              key={value}
              data-testid={testId}
              onClick={() => setUserThemePreference(value)}
              aria-pressed={active}
              variant="ghost"
              size="sm"
              className={cn(
                'border border-border-default transition-colors',
                active
                  ? 'tint-selected tint-interactive font-medium text-text-default'
                  : 'text-text-muted hover:text-text-default'
              )}
            >
              <Icon className="h-4 w-4" />
              <span>{label}</span>
            </Button>
          );
        })}
      </div>
    </div>
  );
};

export default ThemeSelector;
