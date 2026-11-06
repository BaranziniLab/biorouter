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
});
