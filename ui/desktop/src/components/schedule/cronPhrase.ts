import cronstrue from 'cronstrue';

/**
 * A cron rule as one plain phrase: "Every day at 3:00 AM", "Every weekday at 9:30 AM",
 * "Every 15 minutes". The ONE definition every surface reads (the Scheduler's row and detail
 * facts, the schedule dialogs' live line, a Workflows row's schedule badge help), because a
 * schedule that reads "At 03:00 AM" in one place and "Every day at 3:00 AM" in the next is two
 * answers to one question (vision round 1, V3-15).
 *
 * `cronstrue` is the fallback, not the voice. It zero-pads the hour ("At 03:00 AM") and drops
 * the frequency ("every day" trails, or is missing), so the common shapes (every N minutes,
 * hourly, daily, on some weekdays, monthly, yearly) are phrased here and anything else goes to
 * `cronstrue` with its padded hour trimmed. The raw expression belongs in a tooltip, never on a
 * row (principle 10).
 *
 * The daemon writes six fields (seconds first, `CronPicker`'s output); a five-field rule is
 * read the same way with the seconds taken as 0. English on purpose, like `cronstrue`.
 */

const DAY_NAMES = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const DAY_ALIASES: Record<string, number> = {
  SUN: 0,
  MON: 1,
  TUE: 2,
  WED: 3,
  THU: 4,
  FRI: 5,
  SAT: 6,
};
const MONTH_NAMES = [
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
];

const isNumber = (value: string) => /^\d+$/.test(value);

function fieldsOf(cron: string): string[] | null {
  const parts = cron.trim().split(/\s+/);
  if (parts.length === 5) return ['0', ...parts];
  if (parts.length === 6) return parts;
  return null;
}

/** 14, 0 → "2:00 PM"; 3, 5 → "3:05 AM"; 0, 0 → "12:00 AM". The hour is never padded. */
export function clockWithMinutes(hour: number, minute: number): string {
  const suffix = hour < 12 ? 'AM' : 'PM';
  const h12 = hour % 12 === 0 ? 12 : hour % 12;
  return `${h12}:${String(minute).padStart(2, '0')} ${suffix}`;
}

function ordinal(n: number): string {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  switch (n % 10) {
    case 1:
      return `${n}st`;
    case 2:
      return `${n}nd`;
    case 3:
      return `${n}rd`;
    default:
      return `${n}th`;
  }
}

/** "Monday", "Monday and Friday", "Monday, Wednesday and Friday". */
function joinList(items: string[]): string {
  if (items.length <= 1) return items.join('');
  return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

function dayToken(token: string): number | null {
  if (/^[0-7]$/.test(token)) return Number(token) % 7;
  const named = DAY_ALIASES[token.toUpperCase()];
  return named === undefined ? null : named;
}

/** The weekdays a day-of-week field names (0 is Sunday; 7 reads as 0), sorted, or null. */
function weekdaysOf(field: string): number[] | null {
  const days = new Set<number>();
  for (const part of field.split(',')) {
    const range = part.split('-');
    if (range.length === 1) {
      const day = dayToken(part);
      if (day === null) return null;
      days.add(day);
    } else if (range.length === 2) {
      const from = dayToken(range[0]);
      const to = dayToken(range[1]);
      if (from === null || to === null || from > to) return null;
      for (let day = from; day <= to; day += 1) days.add(day);
    } else {
      return null;
    }
  }
  return [...days].sort((a, b) => a - b);
}

function everyDays(days: number[]): string | null {
  if (days.length === 7) return 'Every day';
  if (days.length === 5 && days.join() === '1,2,3,4,5') return 'Every weekday';
  if (days.length === 0) return null;
  return `Every ${joinList(days.map((day) => DAY_NAMES[day]))}`;
}

function structured(parts: string[]): string | null {
  const [second, minute, hour, dom, month, dow] = parts;
  const anyDate = dom === '*' && month === '*' && dow === '*';

  // The picker's "minute" and "hour" periods carry a second or a minute of their own.
  if (minute === '*' && hour === '*' && anyDate) {
    if (second === '0') return 'Every minute';
    return isNumber(second) ? `Every minute at second ${Number(second)}` : null;
  }

  // Everything below is a rule on a whole minute.
  if (second !== '0') return null;

  const everyMinutes = /^\*\/(\d+)$/.exec(minute);
  if (everyMinutes && hour === '*' && anyDate) {
    return Number(everyMinutes[1]) === 1 ? 'Every minute' : `Every ${everyMinutes[1]} minutes`;
  }

  if (!isNumber(minute)) return null;
  const at = Number(minute);
  const past = at === 0 ? '' : ` at :${String(at).padStart(2, '0')}`;

  if (hour === '*' && anyDate) return `Every hour${past}`;
  const everyHours = /^\*\/(\d+)$/.exec(hour);
  if (everyHours && anyDate) {
    return Number(everyHours[1]) === 1
      ? `Every hour${past}`
      : `Every ${everyHours[1]} hours${past}`;
  }

  if (!isNumber(hour) || Number(hour) > 23 || at > 59) return null;
  const time = clockWithMinutes(Number(hour), at);

  if (anyDate) return `Every day at ${time}`;
  if (dom === '*' && month === '*') {
    const days = weekdaysOf(dow);
    const phrase = days ? everyDays(days) : null;
    return phrase ? `${phrase} at ${time}` : null;
  }
  if (isNumber(dom) && month === '*' && dow === '*') {
    return `On the ${ordinal(Number(dom))} of every month at ${time}`;
  }
  if (isNumber(dom) && isNumber(month) && dow === '*') {
    const name = MONTH_NAMES[Number(month) - 1];
    return name ? `Every year on ${name} ${Number(dom)} at ${time}` : null;
  }
  return null;
}

/** `cronstrue`'s sentence with the hour's leading zero trimmed ("At 03:00 AM" → "At 3:00 AM"). */
function describeWithCronstrue(cron: string): string {
  return cronstrue.toString(cron).replace(/\b0(\d):(\d\d)\b/g, '$1:$2');
}

/**
 * The rule as a phrase, or the expression itself when it cannot be read. Throws for nothing.
 */
export function cronPhrase(cron: string): string {
  const parts = fieldsOf(cron);
  if (parts) {
    const phrase = structured(parts);
    if (phrase) return phrase;
  }
  try {
    return describeWithCronstrue(cron);
  } catch {
    return cron;
  }
}

/**
 * Like `cronPhrase`, but a rule this cannot read throws, for the schedule picker, which turns
 * a throw into an error line and a disabled Save rather than saving a rule nobody can read.
 */
export function cronPhraseOrThrow(cron: string): string {
  const parts = fieldsOf(cron);
  const phrase = parts ? structured(parts) : null;
  return phrase ?? describeWithCronstrue(cron);
}
