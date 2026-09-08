import React, { useState, useEffect } from 'react';
import cronstrue from 'cronstrue';
import { ScheduledJob } from '../../schedule';
import { errorMessage } from '../../utils/conversionUtils';
import { Select } from '../ui/Select';

type Period = 'minute' | 'hour' | 'day' | 'week' | 'month' | 'year';

type ParsedCron = {
  period: Period;
  second: string;
  minute: string;
  hour: string;
  dayOfMonth: string;
  month: string;
  dayOfWeek: string;
};

interface CronPickerProps {
  schedule: ScheduledJob | null;
  onChange: (cron: string) => void;
  isValid: (valid: boolean) => void;
}

const parseCron = (cron: string): ParsedCron => {
  const parts = cron.split(' ');
  if (parts.length === 5) {
    parts.unshift('0');
  }
  if (parts.length !== 6) {
    return {
      period: 'day',
      second: '0',
      minute: '0',
      hour: '14',
      dayOfMonth: '*',
      month: '*',
      dayOfWeek: '*',
    };
  }

  const [second, minute, hour, dayOfMonth, month, dayOfWeek] = parts;

  if (month !== '*' && dayOfMonth !== '*') {
    return { period: 'year', second, minute, hour, dayOfMonth, month, dayOfWeek };
  }
  if (dayOfMonth !== '*') {
    return { period: 'month', second, minute, hour, dayOfMonth, month, dayOfWeek };
  }
  if (dayOfWeek !== '*') {
    return { period: 'week', second, minute, hour, dayOfMonth, month, dayOfWeek };
  }
  if (hour !== '*') {
    return { period: 'day', second, minute, hour, dayOfMonth, month, dayOfWeek };
  }
  if (minute !== '*') {
    return { period: 'hour', second, minute, hour, dayOfMonth, month, dayOfWeek };
  }
  return { period: 'minute', second, minute, hour, dayOfMonth, month, dayOfWeek };
};

const to24Hour = (hour12: number, isPM: boolean): number => {
  if (hour12 === 12) {
    return isPM ? 12 : 0;
  }
  return isPM ? hour12 + 12 : hour12;
};

const to12Hour = (hour24: number): { hour: number; isPM: boolean } => {
  if (hour24 === 0) {
    return { hour: 12, isPM: false };
  }
  if (hour24 === 12) {
    return { hour: 12, isPM: true };
  }
  if (hour24 > 12) {
    return { hour: hour24 - 12, isPM: true };
  }
  return { hour: hour24, isPM: false };
};

type Option = { value: string; label: string };

const PERIOD_OPTIONS: Option[] = [
  { value: 'minute', label: 'Minute' },
  { value: 'hour', label: 'Hour' },
  { value: 'day', label: 'Day' },
  { value: 'week', label: 'Week' },
  { value: 'month', label: 'Month' },
  { value: 'year', label: 'Year' },
];

const MONTH_OPTIONS: Option[] = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
].map((label, index) => ({ value: String(index + 1), label }));

const DAY_OF_WEEK_OPTIONS: Option[] = [
  'Sunday',
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
].map((label, index) => ({ value: String(index), label }));

const DAY_OF_MONTH_OPTIONS: Option[] = Array.from({ length: 31 }, (_, index) => ({
  value: String(index + 1),
  label: String(index + 1),
}));

const HOUR_OPTIONS: Option[] = Array.from({ length: 12 }, (_, index) => ({
  value: String(index + 1),
  label: String(index + 1),
}));

/**
 * Minutes and seconds are the same 0–59 ladder, and both are read back as a
 * clock face — so they are zero-padded in the label and bare in the value, the
 * way the cron expression itself is written.
 */
const SIXTY_OPTIONS: Option[] = Array.from({ length: 60 }, (_, index) => ({
  value: String(index),
  label: String(index).padStart(2, '0'),
}));

const MERIDIEM_OPTIONS: Option[] = [
  { value: 'AM', label: 'AM' },
  { value: 'PM', label: 'PM' },
];

/**
 * One control in the cron sentence.
 *
 * `Select`'s own container is `w-full`, so the sentence's rhythm is set by the
 * width of the box each control sits in rather than by a per-control override —
 * which is what keeps the trigger identical, class for class, to the `Input`
 * beside it in the dialog (§3.2).
 *
 * ⚠ **The menu is portalled, and it has to be.** The picker lives inside
 * `ModalShell`, whose `DialogContent` is `overflow-hidden` and whose scrolling
 * body is `overflow-y-auto`; an absolutely-positioned menu inside either is cut
 * off at the box's edge no matter how high it is stacked. Measured: the period
 * select showed one option of six. `menuPortalTarget` escapes both clips and
 * `Select`'s own `menuPortal` z-index puts the escaped menu back above the
 * dialog; `menuPlacement="auto"` still flips it upward near the window's bottom.
 */
function CronSelect({
  label,
  value,
  options,
  onChange,
  width,
}: {
  label: string;
  value: string;
  options: Option[];
  onChange: (value: string) => void;
  width: string;
}) {
  const selected = options.find((option) => option.value === value) ?? null;
  return (
    <div className={width}>
      <Select
        aria-label={label}
        value={selected}
        options={options}
        isSearchable={false}
        menuPlacement="auto"
        menuPortalTarget={typeof document === 'undefined' ? undefined : document.body}
        onChange={(newValue: unknown) => {
          const option = newValue as Option | null;
          if (option) onChange(option.value);
        }}
      />
    </div>
  );
}

/**
 * The schedule, read as one sentence of controls — "Every [Day] at [3] : [00]
 * [PM]" — rather than as a stack of labelled boxes.
 *
 * Two things changed together and both matter. The controls are the shared
 * `Select` (§3.2: the trigger IS the `Input` chrome), so the picker stops
 * carrying its own `border … rounded-md` recipe; and every `<input
 * type="number">` is gone, because a spinner box is the "tablet" the flat
 * redesign is removing — an hour, a minute, a second and a day-of-month are all
 * closed ladders, so each is a list to pick from rather than a field to type an
 * out-of-range value into.
 *
 * The words between the controls are `text-label` — the one role every control's
 * text shares — so the sentence reads at a single weight instead of alternating
 * between prose and chrome.
 */
export const CronPicker: React.FC<CronPickerProps> = ({ schedule, onChange, isValid }) => {
  const [period, setPeriod] = useState<Period>('day');
  const [second, setSecond] = useState('0');
  const [minute, setMinute] = useState('0');
  const [hour12, setHour12] = useState(2);
  const [isPM, setIsPM] = useState(true);
  const [dayOfWeek, setDayOfWeek] = useState('1');
  const [dayOfMonth, setDayOfMonth] = useState('1');
  const [month, setMonth] = useState('1');
  const [readableCron, setReadableCron] = useState('');

  useEffect(() => {
    const parsed = parseCron(schedule?.cron || '');
    setPeriod(parsed.period);
    setSecond(parsed.second === '*' ? '0' : parsed.second);
    setMinute(parsed.minute === '*' ? '0' : parsed.minute);
    const hour24 = parsed.hour === '*' ? 14 : parseInt(parsed.hour, 10);
    const { hour, isPM: pm } = to12Hour(hour24);
    setHour12(hour);
    setIsPM(pm);
    setDayOfWeek(parsed.dayOfWeek === '*' ? '1' : parsed.dayOfWeek);
    setDayOfMonth(parsed.dayOfMonth === '*' ? '1' : parsed.dayOfMonth);
    setMonth(parsed.month === '*' ? '1' : parsed.month);
  }, [schedule]);

  useEffect(() => {
    const hour24 = to24Hour(hour12, isPM);
    let cron: string;

    switch (period) {
      case 'minute':
        cron = `${second} * * * * *`;
        break;
      case 'hour':
        cron = `${second} ${minute} * * * *`;
        break;
      case 'day':
        cron = `${second} ${minute} ${hour24} * * *`;
        break;
      case 'week':
        cron = `${second} ${minute} ${hour24} * * ${dayOfWeek}`;
        break;
      case 'month':
        cron = `${second} ${minute} ${hour24} ${dayOfMonth} * *`;
        break;
      case 'year':
        cron = `${second} ${minute} ${hour24} ${dayOfMonth} ${month} *`;
        break;
      default:
        cron = '0 0 0 * * *';
    }
    onChange(cron);
    if (cron) {
      const cronWithoutSeconds = cron.split(' ').slice(1).join(' ');
      try {
        setReadableCron(cronstrue.toString(cronWithoutSeconds));
        isValid(true);
      } catch (e) {
        isValid(false);
        setReadableCron('error: ' + errorMessage(e));
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [period, second, minute, hour12, isPM, dayOfWeek, dayOfMonth, month]);

  const showClock =
    period === 'day' || period === 'week' || period === 'month' || period === 'year';

  return (
    <div className="flex flex-col gap-3">
      {/* One wrapping sentence, not one row per clause — and each CLAUSE is its
          own flex item, so a fold happens between clauses instead of inside
          one. Without the grouping, "Every [Week] on [Monday] at [2] :" filled
          the first line and stranded the colon at its end with "[00] [PM]"
          beneath: a time broken across two lines by the box it sits in. */}
      <div className="flex flex-wrap items-center gap-2 text-label text-text-default">
        <span className="flex items-center gap-2">
          Every
          <CronSelect
            label="Repeat every"
            value={period}
            options={PERIOD_OPTIONS}
            onChange={(value) => setPeriod(value as Period)}
            width="w-28"
          />
        </span>

        {period === 'year' && (
          <span className="flex items-center gap-2">
            in
            <CronSelect
              label="Month"
              value={month}
              options={MONTH_OPTIONS}
              onChange={setMonth}
              width="w-36"
            />
          </span>
        )}

        {(period === 'month' || period === 'year') && (
          <span className="flex items-center gap-2">
            on day
            <CronSelect
              label="Day of the month"
              value={dayOfMonth}
              options={DAY_OF_MONTH_OPTIONS}
              onChange={setDayOfMonth}
              width="w-20"
            />
          </span>
        )}

        {period === 'week' && (
          <span className="flex items-center gap-2">
            on
            <CronSelect
              label="Day of the week"
              value={dayOfWeek}
              options={DAY_OF_WEEK_OPTIONS}
              onChange={setDayOfWeek}
              width="w-32"
            />
          </span>
        )}

        {showClock && (
          <span className="flex items-center gap-2">
            at
            <CronSelect
              label="Hour"
              value={String(hour12)}
              options={HOUR_OPTIONS}
              onChange={(value) => setHour12(parseInt(value, 10))}
              width="w-16"
            />
            :
            <CronSelect
              label="Minute"
              value={minute}
              options={SIXTY_OPTIONS}
              onChange={setMinute}
              width="w-16"
            />
            <CronSelect
              label="AM or PM"
              value={isPM ? 'PM' : 'AM'}
              options={MERIDIEM_OPTIONS}
              onChange={(value) => setIsPM(value === 'PM')}
              width="w-[4.5rem]"
            />
          </span>
        )}

        {period === 'hour' && (
          <span className="flex items-center gap-2">
            at minute
            <CronSelect
              label="Minute"
              value={minute}
              options={SIXTY_OPTIONS}
              onChange={setMinute}
              width="w-20"
            />
          </span>
        )}

        {period === 'minute' && (
          <span className="flex items-center gap-2">
            at second
            <CronSelect
              label="Second"
              value={second}
              options={SIXTY_OPTIONS}
              onChange={setSecond}
              width="w-20"
            />
          </span>
        )}
      </div>

      <p className="text-supporting text-text-muted">{readableCron}</p>
    </div>
  );
};
