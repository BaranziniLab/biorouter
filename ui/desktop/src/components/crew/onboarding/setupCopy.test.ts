import { describe, expect, it } from 'vitest';
import { connectionSettingsCopy, inviteCopy } from '../dialogs/copy';
import * as onboardingCopy from './copy';
import {
  FULL_ACCESS_IS_YOU,
  fullAccessHost,
  hostCopy,
  joinCopy,
  notSetUpCopy,
  setupCopy,
} from './copy';

/**
 * Every refusal the daemon's setup answers (`docs/crew/design/open-workspaces-and-automatic-setup.md`
 * OW-9, and the setup service's code table). A code missing here would fall back to the daemon's
 * own sentence, which is fine for a person but means this deck drifted from the contract.
 */
const SETUP_CODES = [
  'crew_setup_busy',
  'crew_setup_not_found',
  'crew_setup_cancelled',
  'crew_setup_timed_out',
  'crew_setup_sign_in_expired',
  'crew_setup_not_waiting_for_sign_in',
  'crew_setup_artifact_changed',
  'crew_setup_server_unreadable',
  'crew_setup_start_failed',
  'crew_install_artifact_unavailable',
  'crew_install_unsupported_platform',
  'crew_install_checksum_mismatch',
  'crew_install_checksum_tool_missing',
  'crew_install_permission_denied',
  'crew_install_unexpected_target',
  'crew_install_dependency_missing',
  'crew_install_no_space',
  'crew_install_transfer_incomplete',
  'crew_install_busy',
  'crew_install_outcome_unknown',
  'crew_install_incompatible',
  'crew_host_storage_unsupported',
  'crew_host_pidfd_unavailable',
  'crew_host_machine_id_unavailable',
  'crew_host_home_unsafe',
];

describe('setupCopy (OW-9)', () => {
  it('names the plain steps exactly as the contract does', () => {
    expect(setupCopy.stages).toEqual({
      checking: 'Checking server',
      'signing-in': 'Signing in',
      installing: 'Installing Crew in your account',
      // Account ownership AO-8's two new stages, in the contract's words.
      finding: 'Looking for workspaces',
      updating: 'Updating running workspaces',
      starting: 'Starting workspace',
      connecting: 'Connecting',
      ready: 'Ready',
    });
    expect(hostCopy.setUpAndStart).toBe('Set up and start Crew');
    expect(notSetUpCopy.setUp).toBe('Set up Crew for my account');
  });

  it.each(SETUP_CODES)('says what %s means, in one or two plain sentences', (code) => {
    const sentence = setupCopy.error(code, 'hpc.ucsf.edu');
    expect(sentence).toBeTruthy();
    expect(sentence).toMatch(/[.]$/);
    expect(sentence).not.toMatch(/undefined|null|crew_|BRCREW|\bstderr\b/);
    // The house style: typographic apostrophes, and no dashes standing in for punctuation.
    expect(sentence).not.toMatch(/'| — | – /);
  });

  it('leaves a code it does not know, and the SSH codes, to the daemon’s own sentence', () => {
    expect(setupCopy.error('crew_ssh_key_refused', 'hpc')).toBeNull();
    expect(setupCopy.error('something_new', 'hpc')).toBeNull();
  });

  it('carries the one warning for As me on the server, and the host’s extra sentence', () => {
    expect(joinCopy.fullWarning('wanjun', 'leo.ucsf.edu')).toBe(
      'Your agent will run commands as wanjun on leo.ucsf.edu: it can read and change anything your account can, use the network, and start programs that keep running after access ends. Other people’s messages and files in channels it reads can try to steer it. Use a private model you trust. Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark.'
    );
    // Under account ownership a process running as the host's account is the host (finding 16).
    expect(joinCopy.fullWarningHost('lab')).toMatch(
      /channels you’re not in\. It can also act as the host of lab\.$/
    );
    // The command line's and the chat cards' words, the same on every desktop surface: the Join
    // and Host dialogs, Connection settings, and Not set up's member setup.
    expect(FULL_ACCESS_IS_YOU).toBe(
      'Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark.'
    );
    expect(fullAccessHost('lab')).toBe(
      'You host lab from this account, so your agent can also read the workspace’s own files on the server, including channels you’re not in. It can also act as the host of lab.'
    );
    expect(joinCopy.fullWarning(null, 'leo').endsWith(` ${FULL_ACCESS_IS_YOU}`)).toBe(true);
    expect(
      connectionSettingsCopy.fullWarning('@alice', 'leo').endsWith(` ${FULL_ACCESS_IS_YOU}`)
    ).toBe(true);
    expect(joinCopy.fullWarningHost('lab')).toBe(fullAccessHost('lab'));
    expect(connectionSettingsCopy.fullWarningHost('lab')).toBe(fullAccessHost('lab'));
    expect(joinCopy.agentLegend('leo')).toBe('What can your agent do on leo?');
    expect([joinCopy.accessOff, joinCopy.accessFolder, joinCopy.accessFull]).toEqual([
      'Off',
      'In a folder',
      'As me on the server',
    ]);
  });

  it('tells the host the joiner’s Biorouter sets Crew up', () => {
    expect(inviteCopy.setsUp('Bob')).toBe('Bob’s Biorouter sets Crew up in their account.');
  });

  it('keeps no placeholder install path anywhere in the onboarding deck', () => {
    expect('INSTALL_COMMANDS' in onboardingCopy).toBe(false);
    expect(JSON.stringify(onboardingCopy)).not.toMatch(/replace\/with|\/path\/to/);
  });
});
