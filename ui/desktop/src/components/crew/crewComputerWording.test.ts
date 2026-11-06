import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import * as access from './access/copy';
import * as account from './account/copy';
import * as auth from './auth/copy';
import * as channel from './channel/copy';
import * as channelHeader from './channel/headerCopy';
import * as composer from './composer/copy';
import * as dialogs from './dialogs/copy';
import * as files from './files/copy';
import * as identity from './identity/copy';
import * as layout from './layout/copy';
import * as onboarding from './onboarding/copy';
import * as pane from './pane/copy';
import * as sidebar from './sidebar/copy';
import * as sidebarMenu from './sidebar/menuCopy';
import * as state from './state/copy';
import * as timeline from './timeline/copy';

/**
 * A person's machine is a "computer" in everything Crew's desktop says, as the command line and the
 * chat card say it ("New computer on your account: …", "Computers enrolled as you …"): the live QA
 * of account ownership found the desktop alone saying "device". "Device" stays only in code and in
 * the names of things the protocol hands around: a computer's device key, its device ID, and the
 * device code a joining computer shows.
 */

const MODULES: Record<string, unknown> = {
  access,
  account,
  auth,
  channel,
  channelHeader,
  composer,
  dialogs,
  files,
  identity,
  layout,
  onboarding,
  pane,
  sidebar,
  sidebarMenu,
  state,
  timeline,
};

/** The protocol's names, which keep the word. */
const PROTOCOL_NAMES = /\bdevice (?:keys?|codes?|ID)\b/gi;

/** Every string a copy module can say: its constants, and each function's answers. */
function sayings(value: unknown, path: string, out: [string, string][], depth = 0): void {
  if (depth > 8) return;
  if (typeof value === 'string') {
    out.push([path, value]);
  } else if (typeof value === 'function') {
    // Both sides of a function's conditions: names given, and none.
    for (const fill of ['X', '', null]) {
      const args = Array.from({ length: Math.max(value.length, 1) }, () => fill);
      try {
        sayings((value as (...args: unknown[]) => unknown)(...args), `${path}()`, out, depth + 1);
      } catch {
        // One that needs other arguments says nothing here.
      }
    }
  } else if (Array.isArray(value)) {
    value.forEach((item, index) => sayings(item, `${path}[${index}]`, out, depth + 1));
  } else if (value !== null && typeof value === 'object') {
    for (const [key, item] of Object.entries(value)) {
      // A hook (`useMenuCopyItem`) runs only inside a component, and says nothing of its own.
      if (/^use[A-Z]/.test(key)) continue;
      sayings(item, `${path}.${key}`, out, depth + 1);
    }
  }
}

describe('a person’s machine is a computer in Crew’s desktop', () => {
  it('says "device" nowhere but in the protocol’s names', () => {
    const said: [string, string][] = [];
    for (const [name, module] of Object.entries(MODULES)) sayings(module, name, said);
    expect(said.length).toBeGreaterThan(500);
    const offenders = said
      .filter(([, text]) => /\bdevices?\b/i.test(text.replace(PROTOCOL_NAMES, '')))
      .map(([path, text]) => `${path}: ${text}`);
    expect(offenders).toEqual([]);
  });

  it('names the account’s computers as the command line does', () => {
    expect(account.newDevicesCopy.one('AAAA BBBB CCCC DDDD', 'November 6')).toBe(
      'New computer on your account: AAAA BBBB CCCC DDDD, added November 6.'
    );
    expect(dialogs.keysCopy.devices).toBe('Computers enrolled as you');
    expect(account.deviceCopy.removeLabel('AAAA')).toBe('Remove computer AAAA');
    expect(account.deviceCopy.confirmAction).toBe('Remove computer');
  });

  it('says how a computer was added in the notice’s words and in Keys’, as the command line does', () => {
    const via = account.addedViaText('account');
    expect(via).toBe('by signing in to the server');
    // `biorouter crew status`: "New computer on your account: {fingerprint}, added {date} by
    // signing in to the server."
    expect(account.newDevicesCopy.one('AAAA BBBB CCCC DDDD', 'November 6, 2026', via)).toBe(
      'New computer on your account: AAAA BBBB CCCC DDDD, added November 6, 2026 by signing in to the server.'
    );
    expect(account.newDevicesCopy.one('AAAA', null, via)).toBe(
      'New computer on your account: AAAA, added by signing in to the server.'
    );
    // Keys says the same clause for each way a computer is added.
    expect(dialogs.keysCopy.addedVia).toBe(account.addedViaCopy);
    for (const code of ['bootstrap', 'token', 'invitation_code', 'account'])
      expect(account.addedViaText(code)).toBe(dialogs.keysCopy.addedVia[code]);
    // One this build does not know has no clause, never the raw code.
    expect(account.addedViaText('carrier_pigeon')).toBeNull();
    expect(account.addedViaText('toString')).toBeNull();
    expect(account.addedViaText(undefined)).toBeNull();
  });
});

/** Every Crew source file a person's words can sit in, tests left out. */
function crewSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'test' ? [] : crewSources(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [path] : [];
  });
}

/**
 * The words a source file says that sound like a sentence: string literals and JSX text holding a
 * space, with comments left out. A route (`/devices/acknowledge`) or an ID (`${formId}-device`)
 * has no space, so it is never read as words, and text between an arrow or a type's `>` and the
 * next `<` is code, not JSX text.
 */
function sourceWords(source: string): string[] {
  const code = source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  const literals = [...code.matchAll(/'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)"|`([^`]*)`/g)].map(
    (match) => match[1] ?? match[2] ?? match[3]
  );
  const jsxText = [...code.matchAll(/(?<![=-])>([^<>{}]+)<(?=\/|[A-Za-z])/g)]
    .map((match) => match[1])
    .filter((text) => /[A-Za-z]/.test(text) && !/[;=&|()]/.test(text));
  return [...literals, ...jsxText].filter((text) => /\s/.test(text.trim()));
}

const saysDevice = (text: string) => /\bdevices?\b/i.test(text.replace(PROTOCOL_NAMES, ''));

describe('a person’s machine is a computer in every Crew component', () => {
  it('finds "device" in a sentence written into a component, and passes a route or an ID', () => {
    const bad = `<p>Remove this device</p>; const label = 'A new device was added';`;
    expect(sourceWords(bad).filter(saysDevice)).toEqual([
      'A new device was added',
      'Remove this device',
    ]);
    const good =
      'crewHttp(`/connections/${id}/devices/acknowledge`); const id = `${formId}-device`; // a device\n' +
      "/* the device */ const code = 'Enter the device code';";
    expect(sourceWords(good).filter(saysDevice)).toEqual([]);
  });

  it('says "device" in no component’s own words, only in the protocol’s names', () => {
    const files = crewSources(__dirname);
    expect(files.length).toBeGreaterThan(100);
    const said = files.flatMap((path) =>
      sourceWords(readFileSync(path, 'utf8')).map(
        (text) => [path.slice(__dirname.length + 1), text] as const
      )
    );
    // It reads the components' own words, not only the copy modules'.
    expect(said.length).toBeGreaterThan(1000);
    expect(said.some(([path]) => path === join('dialogs', 'KeysDialog.tsx'))).toBe(true);
    const offenders = said
      .filter(([, text]) => saysDevice(text))
      .map(([path, text]) => `${path}: ${text}`);
    expect(offenders).toEqual([]);
  });
});
