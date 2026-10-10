import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { chatSectionsCopy } from './copy';

describe('Capabilities settings copy', () => {
  /**
   * The capability switches are defaults for NEW chats, and a person must not read them as
   * changing the chat they have open. The sentence moved from a paragraph under the header into
   * the header's InfoTip (spec §3.13); its two load-bearing clauses did not change.
   */
  it('states that capability switches are defaults for new chats', () => {
    expect(chatSectionsCopy.capabilitiesHelp).toContain('new chats start with');
    expect(chatSectionsCopy.capabilitiesHelp).toContain('Existing chats keep their current');
  });

  it('is the help the Capabilities section actually renders', () => {
    const source = readFileSync(join(__dirname, 'ChatSettingsSection.tsx'), 'utf8');
    expect(source).toContain('help={chatSectionsCopy.capabilitiesHelp}');
  });
});
