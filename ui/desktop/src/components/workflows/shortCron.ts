import { cronPhrase } from '../schedule/cronPhrase';

/**
 * A workflow's schedule as a badge ("Daily 2 PM"), and as a sentence for the badge's help
 * ("Every day at 2:00 PM"). Spec 3.10: line 1 of a Workflows row carries "a short cron";
 * the full rule is one hover or Tab away, never a second line.
 *
 * The daemon writes six fields (seconds first, `CronPicker`'s output); a five-field rule is
 * read the same way with the seconds taken as 0. Anything this does not recognise reads
 * "Scheduled", and the sentence still says exactly when.
 *
 * English on purpose: the sentence beside it comes from `cronPhrase` (the Scheduler's own), which
 * is English, and a badge that disagreed with its own help in language would be two answers to
 * one question.
 */

const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_NAMES = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

const FALLBACK = 'Scheduled';

function fields(cron: string): string[] | null {
  const parts = cron.trim().split(/\s+/);
  if (parts.length === 5) return ['0', ...parts];
  if (parts.length === 6) return parts;
  return null;
}

const isNumber = (value: string) => /^\d+$/.test(value);

/** 14, 0 → "2 PM"; 9, 30 → "9:30 AM"; 0, 0 → "12 AM". */
export function clockTime(hour: number, minute: number): string {
  const suffix = hour < 12 ? 'AM' : 'PM';
  const h12 = hour % 12 === 0 ? 12 : hour % 12;
  return minute === 0 ? `${h12} ${suffix}` : `${h12}:${String(minute).padStart(2, '0')} ${suffix}`;
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

function dayList(dow: string): string | null {
  if (dow === '1-5' || dow === 'MON-FRI') return 'Weekdays';
  if (dow === '0,6' || dow === '6,0' || dow === 'SAT,SUN') return 'Weekends';
  const days = dow.split(',');
  if (!days.every((d) => /^[0-7]$/.test(d))) return null;
  return days.map((d) => DAY_NAMES[Number(d) % 7]).join(', ');
}

/** The badge text: "Daily 2 PM", "Mon 9:30 AM", "Hourly", "Every 15 min", or "Scheduled". */
export function shortCron(cron: string | null | undefined): string | null {
  if (!cron) return null;
  const parts = fields(cron);
  if (!parts) return FALLBACK;
  const [, minute, hour, dom, month, dow] = parts;

  // Every N minutes, every minute, every N hours.
  const everyMinutes = /^\*\/(\d+)$/.exec(minute);
  if (everyMinutes && hour === '*' && dom === '*' && month === '*' && dow === '*') {
    return `Every ${everyMinutes[1]} min`;
  }
  if (minute === '*' && hour === '*' && dom === '*' && month === '*' && dow === '*') {
    return 'Every minute';
  }
  const everyHours = /^\*\/(\d+)$/.exec(hour);
  if (isNumber(minute) && everyHours && dom === '*' && month === '*' && dow === '*') {
    return `Every ${everyHours[1]} h`;
  }
  if (isNumber(minute) && hour === '*' && dom === '*' && month === '*' && dow === '*') {
    return Number(minute) === 0 ? 'Hourly' : `Hourly at :${minute.padStart(2, '0')}`;
  }

  if (!isNumber(minute) || !isNumber(hour)) return FALLBACK;
  const at = clockTime(Number(hour), Number(minute));

  if (dom === '*' && month === '*' && dow === '*') return `Daily ${at}`;
  if (dom === '*' && month === '*') {
    const days = dayList(dow);
    return days ? `${days} ${at}` : FALLBACK;
  }
  if (isNumber(dom) && month === '*' && dow === '*') return `Monthly ${ordinal(Number(dom))} ${at}`;
  if (isNumber(dom) && isNumber(month) && dow === '*') {
    const name = MONTH_NAMES[Number(month) - 1];
    return name ? `Yearly ${name} ${Number(dom)} ${at}` : FALLBACK;
  }
  return FALLBACK;
}

/** The whole rule as a sentence, for the badge's help. The expression itself if unreadable. */
export const cronSentence = cronPhrase;
