// @vitest-environment node

/**
 * The desktop's copy of the extension rename table must equal the table the
 * daemon compiles in (`crates/biorouter/src/config/extension_renames.json`).
 * A drift would install an old-named bundle under the old id on the desktop
 * while the daemon resolves it to the new one.
 */
import { describe, expect, it } from 'vitest';
import * as fs from 'fs';
import {
  EXTENSION_RENAMES,
  allExtensionKeys,
  currentExtensionKey,
  isRenamedAway,
  sameExtension,
} from './extensionRenames';

const TABLE = new URL(
  '../../../../crates/biorouter/src/config/extension_renames.json',
  import.meta.url
);

describe('extension renames', () => {
  it('mirrors the table the daemon compiles in', () => {
    const table = JSON.parse(fs.readFileSync(TABLE, 'utf8')) as {
      version: number;
      renames: Array<{ from: string; to: string }>;
    };
    expect(table.version).toBe(1);
    expect(EXTENSION_RENAMES.map(({ from, to }) => ({ from, to }))).toEqual(
      table.renames.map(({ from, to }) => ({ from, to }))
    );
  });

  it('resolves an old id to the current one, and never the other way', () => {
    expect(currentExtensionKey('gptragent')).toBe('gptr');
    expect(currentExtensionKey(' GptrAgent ')).toBe('gptr');
    expect(currentExtensionKey('gptr')).toBe('gptr');
    expect(currentExtensionKey('spokeagent')).toBe('spokeagent');
    expect(allExtensionKeys('gptr')).toEqual(['gptr', 'gptragent']);
    expect(allExtensionKeys('gptragent')).toEqual(['gptr', 'gptragent']);
    expect(isRenamedAway('gptragent')).toBe(true);
    expect(isRenamedAway('gptr')).toBe(false);
    expect(sameExtension('gptragent', 'gptr')).toBe(true);
    expect(sameExtension('gptragent', 'spokeagent')).toBe(false);
  });
});
