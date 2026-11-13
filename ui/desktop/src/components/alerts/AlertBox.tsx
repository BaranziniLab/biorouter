import { useState, useEffect } from 'react';
import { Pencil, Save } from '../icons/app-icons';
import { cn } from '../../utils';
import { Alert, AlertType } from './types';
import { NotificationSurface, type NotificationStatus } from './NotificationSurface';
import { useConfig } from '../ConfigContext';
import { InfoTip } from '../ui/info-tip';
import { toastError } from '../../toasts';
import {
  AUTO_COMPACT_MAX_TOKENS_KEY,
  effectiveAutoCompactTrigger,
  savedAutoCompactCeiling,
} from '../../utils/autoCompactTrigger';

interface AlertBoxProps {
  alert: Alert;
  className?: string;
  compactButtonEnabled?: boolean;
}

const AUTO_COMPACT_THRESHOLD_KEY = 'BIOROUTER_AUTO_COMPACT_THRESHOLD';

const ALERT_STATUS: Record<AlertType, NotificationStatus> = {
  [AlertType.Error]: 'error',
  [AlertType.Warning]: 'warning',
  [AlertType.Info]: 'info',
};

/** What an alert's info icon is about: its glyph reads "About this warning". */
const ALERT_HELP_LABEL: Record<AlertType, string> = {
  [AlertType.Error]: 'this error',
  [AlertType.Warning]: 'this warning',
  [AlertType.Info]: 'this notice',
};

// Progress-mode (context-window meter) keeps its solid-fill container — it is a
// distinct indicator widget, not a status notification. The simple message mode
// below renders through the shared NotificationSurface.
const alertStyles: Record<AlertType, string> = {
  // The danger fill is the secondary ink (design.md 9.14, red is rare), light
  // grey in dark mode, so the ink on it is `--text-on-status`, never white.
  [AlertType.Error]: 'bg-background-danger text-text-on-status',
  [AlertType.Warning]: 'bg-background-warning text-neutral-900',
  [AlertType.Info]: 'dark:bg-white dark:text-black bg-black text-white',
};

const formatTokenCount = (count: number): string => {
  if (count >= 1000000) {
    const millions = count / 1000000;
    return millions % 1 === 0 ? `${millions.toFixed(0)}M` : `${millions.toFixed(1)}M`;
  } else if (count >= 1000) {
    const thousands = count / 1000;
    return thousands % 1 === 0 ? `${thousands.toFixed(0)}k` : `${thousands.toFixed(1)}k`;
  }
  return count.toString();
};

export const AlertBox = ({ alert, className }: AlertBoxProps) => {
  // Read *and* write through the context. Writing straight to `upsertConfig`
  // leaves the context's cached config serving the pre-write value to every
  // other consumer of this key, with no signal that it has gone stale (#52).
  const { read, upsert } = useConfig();
  const [isEditingThreshold, setIsEditingThreshold] = useState(false);
  const [loadedThreshold, setLoadedThreshold] = useState<number>(0.8);
  const [thresholdValue, setThresholdValue] = useState(80);
  const [isSaving, setIsSaving] = useState(false);
  /** The absolute token ceiling (`BIOROUTER_AUTO_COMPACT_MAX_TOKENS`), `null` for none. */
  const [ceiling, setCeiling] = useState<number | null>(savedAutoCompactCeiling(undefined));

  useEffect(() => {
    const loadThreshold = async () => {
      try {
        const threshold = await read(AUTO_COMPACT_THRESHOLD_KEY, false);
        if (threshold !== undefined && threshold !== null && typeof threshold === 'number') {
          setLoadedThreshold(threshold);
          setThresholdValue(Math.max(1, Math.round(threshold * 100)));
        }
      } catch (err) {
        console.error('Error fetching auto-compact threshold:', err);
      }
    };

    loadThreshold();
  }, [read]);

  useEffect(() => {
    let cancelled = false;
    read(AUTO_COMPACT_MAX_TOKENS_KEY, false)
      .then((v) => {
        if (!cancelled) setCeiling(savedAutoCompactCeiling(v));
      })
      .catch(() => {
        /* the daemon's default stands */
      });
    return () => {
      cancelled = true;
    };
  }, [read]);

  const currentThreshold = loadedThreshold;
  // Where it really fires: the lower of the threshold's share and the token
  // ceiling (a window over 250k tokens compacts at 200k by default).
  const trigger = alert.progress
    ? effectiveAutoCompactTrigger(
        currentThreshold > 0 && currentThreshold < 1 ? currentThreshold * 100 : null,
        alert.progress.total,
        ceiling
      )
    : null;
  const markerShare = trigger ? trigger.pct / 100 : currentThreshold;

  const handleSaveThreshold = async () => {
    if (isSaving) return; // Prevent double-clicks

    let validThreshold = Math.max(1, Math.min(100, thresholdValue));
    if (validThreshold !== thresholdValue) {
      setThresholdValue(validThreshold);
    }

    setIsSaving(true);
    try {
      const newThreshold = validThreshold / 100; // Convert percentage to decimal

      await upsert(AUTO_COMPACT_THRESHOLD_KEY, newThreshold, false);

      setIsEditingThreshold(false);
      setLoadedThreshold(newThreshold);

      // Notify parent component of the threshold change
      if (alert.onThresholdChange) {
        alert.onThresholdChange(newThreshold);
      }
    } catch (error) {
      console.error('Error saving threshold:', error);
      toastError({
        title: 'Failed to save threshold',
        msg: error instanceof Error ? error.message : 'Unknown error',
      });
    } finally {
      setIsSaving(false);
    }
  };

  // Simple message alert → shared NotificationSurface: neutral surface, tinted
  // status chip, top-aligned icon. Rendered full-bleed (no border/radius) and on
  // the popover's own ground, not the banner's fill, so it sits flush in the
  // alert popover's stacked tiles (a fill there would be a box in a box).
  if (!alert.progress) {
    return (
      <NotificationSurface
        status={ALERT_STATUS[alert.type]}
        className={cn('rounded-none border-0 bg-background-default px-3 py-3', className)}
        message={
          // The message is the one visible line; why it matters is the info icon beside it
          // (owner, 2026-11-13; design.md 9.16).
          <span className="whitespace-pre-line">
            {alert.message}
            {alert.help ? (
              <>
                {' '}
                <InfoTip label={ALERT_HELP_LABEL[alert.type]} help={alert.help} />
              </>
            ) : null}
          </span>
        }
        actions={
          alert.action ? (
            <a
              role="button"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                alert.action?.onClick();
              }}
              className="cursor-pointer text-left text-secondary text-text-default underline hover:opacity-80"
            >
              {alert.action.text}
            </a>
          ) : undefined
        }
      />
    );
  }

  // Context-window progress meter — a distinct indicator widget, kept as-is.
  return (
    <div
      className={cn('flex flex-col gap-2 px-3 py-3', alertStyles[alert.type], className)}
      onMouseDown={(e) => {
        // Prevent popover from closing when clicking inside the alert box
        if (isEditingThreshold) {
          e.stopPropagation();
        }
      }}
    >
      <div className="flex flex-col gap-2">
        <span className="text-supporting">{alert.message}</span>

        {/* Auto-compact threshold indicator with edit */}
        <div className="flex items-center justify-center gap-1 min-h-[20px]">
          {isEditingThreshold ? (
            <>
              <span className="text-supporting opacity-70">Auto compact at</span>
              <input
                type="number"
                min="1"
                max="100"
                step="1"
                value={thresholdValue}
                onChange={(e) => {
                  const val = parseInt(e.target.value, 10);
                  // Allow empty input for easier editing
                  if (e.target.value === '') {
                    setThresholdValue(1);
                  } else if (!isNaN(val)) {
                    // Clamp value between 1 and 100
                    setThresholdValue(Math.max(1, Math.min(100, val)));
                  }
                }}
                onBlur={(e) => {
                  // On blur, ensure we have a valid value
                  const val = parseInt(e.target.value, 10);
                  if (isNaN(val) || val < 1) {
                    setThresholdValue(1);
                  } else if (val > 100) {
                    setThresholdValue(100);
                  }
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    handleSaveThreshold();
                  } else if (e.key === 'Escape') {
                    setIsEditingThreshold(false);
                    const resetValue = Math.round(currentThreshold * 100);
                    setThresholdValue(Math.max(1, resetValue));
                  }
                }}
                onFocus={(e) => {
                  // Select all text on focus for easier editing
                  e.target.select();
                }}
                onClick={(e) => {
                  // Prevent issues with text selection
                  e.stopPropagation();
                }}
                className="w-12 px-1 text-supporting bg-white/10 border border-current/30 rounded text-center focus:bg-white/20 focus:border-current/50 transition-colors"
                disabled={isSaving}
                autoFocus
              />
              <span className="text-supporting opacity-70">%</span>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  handleSaveThreshold();
                }}
                disabled={isSaving}
                className="p-1 hover:opacity-60 transition-opacity cursor-pointer relative z-50"
                style={{ minWidth: '20px', minHeight: '20px', pointerEvents: 'auto' }}
              >
                <Save className="w-3.5 h-3.5" />
              </button>
            </>
          ) : (
            <>
              <span className="text-supporting opacity-70">
                {trigger?.byCeiling
                  ? `Auto compact at ${formatTokenCount(trigger.tokens)} tokens`
                  : `Auto compact at ${Math.round(currentThreshold * 100)}%`}
              </span>
              <button
                type="button"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setIsEditingThreshold(true);
                }}
                className="p-1 hover:opacity-60 transition-opacity cursor-pointer relative z-10"
                style={{ minWidth: '20px', minHeight: '20px' }}
              >
                <Pencil className="w-3.5 h-3.5 opacity-70" />
              </button>
            </>
          )}
        </div>

        <div className="flex justify-between w-full relative">
          {(() => {
            let closestDotIndex = -1;
            if (markerShare !== undefined && markerShare > 0 && markerShare <= 1) {
              let minDistance = Infinity;
              for (let j = 0; j < 30; j++) {
                const dotPos = j / 29;
                const distance = Math.abs(dotPos - markerShare);
                if (distance < minDistance) {
                  minDistance = distance;
                  closestDotIndex = j;
                }
              }
            }

            return [...Array(30)].map((_, i) => {
              const progress = alert.progress!.current / alert.progress!.total;
              const progressPercentage = Math.round(progress * 100);
              const dotPosition = i / 29; // 0 to 1 range for 30 dots
              const isActive = dotPosition <= progress;
              const isThresholdDot = i === closestDotIndex;

              const getProgressColor = () => {
                if (progressPercentage <= 50) {
                  return 'bg-background-success';
                } else if (progressPercentage <= 75) {
                  return 'bg-background-warning';
                } else if (progressPercentage <= 90) {
                  return 'bg-background-warning';
                } else {
                  return 'bg-background-danger';
                }
              };

              const progressColor = getProgressColor();
              const inactiveColor = 'bg-background-strong';

              return (
                <div
                  key={i}
                  className={cn(
                    'rounded-full transition-colors relative',
                    isThresholdDot
                      ? 'h-[6px] w-[6px] -mt-[2px]' // Make threshold dot twice as large
                      : 'h-[2px] w-[2px]',
                    isActive ? progressColor : inactiveColor
                  )}
                />
              );
            });
          })()}
        </div>
        <div className="flex justify-between items-baseline text-supporting">
          <div className="flex gap-1 items-baseline">
            <span className={'dark:text-black/60 text-white/60'}>
              {formatTokenCount(alert.progress!.current)}
            </span>
            <span className={'dark:text-black/40 text-white/40'}>
              {Math.round((alert.progress!.current / alert.progress!.total) * 100)}%
            </span>
          </div>
          <span className={'dark:text-black/60 text-white/60'}>
            {formatTokenCount(alert.progress!.total)}
          </span>
        </div>
        {alert.showCompactButton && alert.onCompact && (
          <button
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              alert.onCompact!();
            }}
            disabled={alert.compactButtonDisabled}
            className={cn(
              'flex items-center gap-1.5 text-supporting mt-1',
              alert.compactButtonDisabled
                ? 'opacity-50 cursor-not-allowed'
                : 'hover:opacity-80 cursor-pointer'
            )}
          >
            {alert.compactIcon}
            <span>Compact now</span>
          </button>
        )}
      </div>
    </div>
  );
};
