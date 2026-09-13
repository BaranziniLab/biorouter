// Every string on the Browse skills install path that counts something.
//
// ⚠ **A row is not a skill.** A marketplace row is either one skill or a
// package of several ("3 skills · auto-applied"), and the copy here used to
// count rows as skills: selecting the 3-skill `primer-design` package read
// "Install 1 skill" and then "1 skill installed", and a package plus one skill
// said "2 skills installed" for 15. The units are now kept apart:
//
// - **Selection counts rows.** `Select all (N)`, the "N selected" footer and the
//   section headings all count checkboxes, because checking boxes is what they
//   describe — "Select all (129)" checks 129 boxes.
// - **The install button counts skills**, from what each selected row says it
//   holds (`registrySkillCount`). It is the one place the catalogue's own claim
//   is all there is: nothing has been downloaded yet.
// - **The success toast counts skills the daemon installed**, not what the
//   catalogue claimed, and names what landed — a package with its count — so
//   the number can be matched against the Skills list it points at.
// - **A failure names the row that failed.** The importer stages a package and
//   swaps it in whole, so nothing of a failed row lands: counting its skills as
//   "failed" would count skills that were never there.
//
// Pure functions, pinned in `installCopy.test.ts`, so a count cannot drift back
// to rows without a test saying so.

import type { RegistrySkill } from './registry';

const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? '' : 's'}`;

/**
 * How many skills a marketplace row installs, as the row says.
 *
 * The registry carries no count field: a package's is the leading number of its
 * `type` phrase ("14 skills · auto-applied", "13 skills · TDD, debugging, …"),
 * which is also the text the user reads on that row. A row whose phrase names
 * no count ("User-invocable · /r-scripting") is one skill.
 */
export function registrySkillCount(skill: Pick<RegistrySkill, 'type'>): number {
  const match = /^\s*(\d+)\s+skills?\b/i.exec(skill.type ?? '');
  const count = match ? Number(match[1]) : 1;
  return count > 0 ? count : 1;
}

/**
 * The install button's label, given the number of SKILLS the selection holds.
 *
 * ⚠ The count goes INSIDE the conditional along with its trailing space, not
 * beside it. The original wrote `` `Install ${n > 0 ? n : ''} skill…` `` — an
 * empty substitution between two literal spaces — so the button read
 * **"Install  skills"** with a double space in the state it spends most of its
 * life in: nothing selected, and therefore disabled and in front of the user
 * from the moment the dialog opens.
 *
 * Three shapes: 0 → "Install skills" (plural, because it is an invitation, not
 * a count), 1 → "Install 1 skill", n → "Install n skills".
 */
export function installButtonLabel(skillCount: number): string {
  const count = skillCount > 0 ? `${skillCount} ` : '';
  return `Install ${count}skill${skillCount !== 1 ? 's' : ''}`;
}

/** What one install put on disk, as the daemon reported it. */
export interface LandedInstall {
  /** The name the Skills list shows it under. */
  name: string;
  skills: number;
  isPackage: boolean;
}

/** How many landed installs the toast names before it says "and N more". */
export const NAMED_INSTALLS = 3;

/** "a", "a and b", "a, b and c", "a, b, c and 4 more". */
function listed(names: string[]): string {
  if (names.length <= NAMED_INSTALLS) {
    return names.length <= 1
      ? (names[0] ?? '')
      : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  }
  return `${names.slice(0, NAMED_INSTALLS).join(', ')} and ${names.length - NAMED_INSTALLS} more`;
}

export function installedToast(landed: readonly LandedInstall[]): { title: string; msg: string } {
  const total = landed.reduce((sum, one) => sum + one.skills, 0);
  const names = landed.map((one) =>
    one.isPackage ? `${one.name} (${plural(one.skills, 'skill')})` : one.name
  );
  return {
    title: `${plural(total, 'skill')} installed`,
    msg:
      names.length > 0
        ? `Added to Biorouter Skills: ${listed(names)}`
        : 'Added to Biorouter Skills',
  };
}

export interface FailedInstall {
  /** The row's name, as the marketplace list shows it. */
  name: string;
  error: string;
}

export function failedToast(failures: readonly FailedInstall[]): { title: string; msg: string } {
  const [first] = failures;
  if (!first) return { title: 'Nothing was installed', msg: '' };
  if (failures.length === 1) {
    return { title: `${first.name} was not installed`, msg: first.error };
  }
  return {
    title: `${failures.length} selections were not installed`,
    msg: `${first.name}: ${first.error} (and ${failures.length - 1} more)`,
  };
}

/** The footer while installs run — one download at a time, named. */
export function installProgressLabel(name: string, position: number, total: number): string {
  return total > 1 ? `Installing ${name} (${position} of ${total})…` : `Installing ${name}…`;
}
