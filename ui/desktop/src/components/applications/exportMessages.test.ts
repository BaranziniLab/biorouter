import { describe, expect, it } from 'vitest';
import type { AppExportResult } from '../../utils/appExportRequest';
import { OPEN_IN_BROWSER_COPY } from '../../utils/openInBrowser';
import { exportedMessage, exportNotesMessage } from './appManagement';
import { appsCopy } from './copy';

type Exported = Extract<AppExportResult, { status: 'exported' }>;

const exported = (target: string, mode: 'launcher' | 'full', kbs = 0, skills = 0): Exported => ({
  status: 'exported',
  target,
  legacy: false,
  summary: {
    mode,
    files: 3,
    kbs: Array.from({ length: kbs }, (_, n) => `kb-${n}`),
    skills: Array.from({ length: skills }, (_, n) => `skill-${n}`),
    extensions: [],
    notes: [],
    lintWarnings: [],
    skipped: [],
  },
});

/** The export's toasts read from `appsCopy`, and say each thing once, as a sentence. */
describe('exportedMessage', () => {
  it('names the folder a launcher export went to', () => {
    expect(exportedMessage(exported('/Users/me/Desktop/atlas', 'launcher'))).toBe(
      appsCopy.exportedTo('/Users/me/Desktop/atlas')
    );
    expect(appsCopy.exportedTo('/x')).toBe('Exported to /x.');
  });

  it('names the zip a browser downloaded', () => {
    expect(exportedMessage(exported('atlas.zip', 'launcher'))).toBe('Downloaded atlas.zip.');
  });

  it('counts what a full export carries, one and many', () => {
    expect(exportedMessage(exported('/x', 'full', 1, 0))).toBe(
      'Exported to /x. It carries 1 knowledge base and 0 skills.'
    );
    expect(exportedMessage(exported('/x', 'full', 2, 1))).toBe(
      'Exported to /x. It carries 2 knowledge bases and 1 skill.'
    );
  });
});

describe('exportNotesMessage', () => {
  it('opens a daemon clause with a capital and stops it, keeping its words', () => {
    expect(exportNotesMessage(["skipped knowledge base 'ms-cohort': it is private"])).toBe(
      "Skipped knowledge base 'ms-cohort': it is private."
    );
  });

  it('keeps two notes apart: they used to run together into one unpunctuated line', () => {
    expect(
      exportNotesMessage([
        'the app has no built bundle yet; the daemon that opens it builds one',
        'exported despite 2 harness error(s); the app may not work until they are fixed',
      ])
    ).toBe(
      'The app has no built bundle yet; the daemon that opens it builds one. ' +
        'Exported despite 2 harness error(s); the app may not work until they are fixed.'
    );
  });

  it('leaves a sentence, and a note that opens on a quoted name, as written', () => {
    const sentence =
      'The background service is older than this app, so the export carries its text files only.';
    expect(exportNotesMessage([sentence])).toBe(sentence);
    expect(exportNotesMessage(["'trials' is not a knowledge base id; skipped"])).toBe(
      "'trials' is not a knowledge base id; skipped."
    );
  });

  it('says nothing for no notes, or only blank ones', () => {
    expect(exportNotesMessage([])).toBe('');
    expect(exportNotesMessage(['  ', ''])).toBe('');
  });
});

describe('opening an app in the browser', () => {
  it('fails in the same words from a chat card or the preview as from Built apps’ Launch', () => {
    expect(OPEN_IN_BROWSER_COPY.failed).toBe(appsCopy.couldNotOpen);
  });
});
