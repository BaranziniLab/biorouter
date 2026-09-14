import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  BACKEND_DISCONNECTED_TITLE,
  START_CHAT_FAILED_TITLE,
  appendSentence,
  isStartRefusedForWantOfProof,
  startChatFailureNotice,
} from './startChatFailure';

/**
 * The body the 2026-09-10 QA run captured from `POST /agent/start` on a
 * `biorouter serve` daemon with `versa_azure` configured (finding F1), verbatim.
 * Under `throwOnError` the generated client throws the parsed body, so this
 * object IS what a caller's `catch` receives.
 */
const QA_REFUSAL = {
  message:
    "Switching this chat to a private model is the user's decision, not yours. The request to " +
    "switch it to 'versa_azure' did not come from the model picker, so the chat is unchanged and " +
    'still on its current model. Do not retry; the same call will be refused again. If this task ' +
    'genuinely needs a private model, stop and ask the user to switch this chat to a private ' +
    'model first, in the desktop app under Settings > Models, or with the model chip in the ' +
    'composer.',
};

describe('startChatFailureNotice', () => {
  it('words the privacy refusal for a person and keeps the daemon text behind Copy error', () => {
    const notice = startChatFailureNotice(QA_REFUSAL, { kept: true });
    expect(notice.title).toBe(START_CHAT_FAILED_TITLE);
    // Every instruction in the refusal is addressed to a model, and the last
    // one names a control the browser surface deliberately disables (SD-1).
    for (const forAModel of [
      'Do not retry',
      'not yours',
      'model picker',
      'stop and ask the user',
      'model chip',
    ]) {
      expect(notice.msg).not.toContain(forAModel);
    }
    expect(notice.msg).toContain('Your message was kept.');
    expect(notice.traceback).toBe(QA_REFUSAL.message);
  });

  it('only claims the message was kept when the caller put it back', () => {
    expect(startChatFailureNotice(QA_REFUSAL, { kept: false }).msg).not.toContain('kept');
  });

  it('shows a failure already written for a person as it came', () => {
    // The A/B leg of the QA run: the same sandbox with a public provider and no key.
    const body = {
      message:
        'Failed to configure the selected provider for the new chat: Configuration value not ' +
        'found: OPENAI_API_KEY',
    };
    expect(startChatFailureNotice(body, { kept: false })).toEqual({
      title: START_CHAT_FAILED_TITLE,
      msg: body.message,
      // Always copyable: the troubleshooting guide sends people to "Copy error".
      traceback: body.message,
    });
    // Punctuated as prose. This read "…OPENAI_API_KEY Your message was kept."
    // — two sentences run together — and the traceback keeps the daemon's own
    // text untouched.
    const kept = startChatFailureNotice(body, { kept: true });
    expect(kept.msg).toBe(`${body.message}. Your message was kept.`);
    expect(kept.traceback).toBe(body.message);
  });

  it('ends the daemon text as a sentence before saying the message was kept', () => {
    // The exact 400 measured in the dev app on 1.90.4, which ran on as
    // "…with a host Your message was kept."
    const measured = {
      message:
        'Failed to configure the selected provider for the new chat: provider endpoint must be ' +
        'an HTTPS URL with a host',
    };
    expect(startChatFailureNotice(measured, { kept: true }).msg).toBe(
      'Failed to configure the selected provider for the new chat: provider endpoint must be an ' +
        'HTTPS URL with a host. Your message was kept.'
    );
    // One that already ends a sentence gets no second period. (The credential
    // refusal measured beside it ends "…signs in with the API key.")
    const finished = { message: 'Could not read the key. Answer the prompt with “Always Allow”.' };
    expect(startChatFailureNotice(finished, { kept: true }).msg).toBe(
      'Could not read the key. Answer the prompt with “Always Allow”. Your message was kept.'
    );
  });

  it('says the backend is unreachable when the request never got an answer', () => {
    const notice = startChatFailureNotice(new TypeError('Failed to fetch'), { kept: true });
    expect(notice.title).toBe(BACKEND_DISCONNECTED_TITLE);
    expect(notice.msg).toContain('could not reach its backend');
    expect(notice.msg).toContain('Your message was kept');
  });

  it('recognizes the refusal by its marker, in the shapes the daemon sends, and nothing else', () => {
    expect(isStartRefusedForWantOfProof(QA_REFUSAL)).toBe(true);
    expect(isStartRefusedForWantOfProof(QA_REFUSAL.message)).toBe(true);
    // A thrown Error that happens to carry the words is a bug, not a policy.
    expect(isStartRefusedForWantOfProof(new Error(QA_REFUSAL.message))).toBe(false);
    expect(isStartRefusedForWantOfProof({ message: 'Failed to create session: disk full' })).toBe(
      false
    );
    expect(isStartRefusedForWantOfProof(null)).toBe(false);
    expect(isStartRefusedForWantOfProof(undefined)).toBe(false);
  });
});

/**
 * Every surface that starts a chat reports a failure through the notice.
 *
 * Seven surfaces started a chat when this was written, and before it six of
 * them handled a failure four different ways: a console line (the Home composer
 * the QA run hit, the launcher), an unhandled rejection (both "Ask Biorouter"
 * buttons), a retry loop (a window opened for a workflow), and a list-load
 * error over a list that had loaded (Workflows). An eighth surface must not get
 * to pick a fifth.
 */
describe('every surface that starts a chat', () => {
  const SRC = join(__dirname, '..');
  const STARTS_A_CHAT = /\b(?:createSession|startNewSession|startAgent)\(/;
  // `sessions.ts` DEFINES the first two and wraps the third; it reports nothing
  // because it has no one to report to.
  const DEFINITIONS = new Set(['sessions.ts']);

  const sourceFiles = (dir: string): string[] =>
    readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        // The generated client is not a surface.
        return name === 'api' || name === 'node_modules' ? [] : sourceFiles(path);
      }
      return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
    });

  // Prose about a call is not a call: `navigationUtils.ts` names
  // `startNewSession()` in a comment and starts nothing.
  const withoutComments = (source: string) =>
    source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|\s)\/\/.*$/gm, '$1');

  const surfaces = sourceFiles(SRC)
    .map((path) => ({
      rel: relative(SRC, path).split(sep).join('/'),
      source: withoutComments(readFileSync(path, 'utf8')),
    }))
    .filter(({ rel, source }) => !DEFINITIONS.has(rel) && STARTS_A_CHAT.test(source));

  it('finds the surfaces it is guarding, so the scan is not vacuous', () => {
    const found = surfaces.map(({ rel }) => rel);
    for (const known of [
      'App.tsx',
      'toasts.tsx',
      'utils/launcherMessage.ts',
      'components/Hub.tsx',
      'components/BaseChat.tsx',
      'components/GroupedExtensionLoadingToast.tsx',
      'components/workflows/WorkflowsView.tsx',
    ]) {
      expect(found).toContain(known);
    }
  });

  it.each(surfaces.map(({ rel, source }) => [rel, source] as const))(
    '%s reports a failed start in the shared words',
    (_rel, source) => {
      expect(source).toMatch(/startChatFailureNotice\(|handleCreateSessionError\(/);
    }
  );
});

describe('appendSentence', () => {
  const KEPT = 'Your message was kept.';
  it.each([
    // [daemon text, toast text]
    ['no period', 'no period. Your message was kept.'],
    ['a sentence.', 'a sentence. Your message was kept.'],
    ['a question?', 'a question? Your message was kept.'],
    ['an exclamation!', 'an exclamation! Your message was kept.'],
    ['trailing ellipsis…', 'trailing ellipsis… Your message was kept.'],
    ['trailing whitespace.\n\n', 'trailing whitespace. Your message was kept.'],
    // A finished sentence closed inside a quote or bracket is finished.
    ['he said "stop."', 'he said "stop." Your message was kept.'],
    ['(see the log.)', '(see the log.) Your message was kept.'],
    ['it said ‘done.’', 'it said ‘done.’ Your message was kept.'],
    // A quoted VALUE is not a sentence end: the period goes after the quote.
    ['invalid value "abc"', 'invalid value "abc". Your message was kept.'],
    ["unknown provider 'x'", "unknown provider 'x'. Your message was kept."],
    ['(see the log)', '(see the log). Your message was kept.'],
    // A code span is the daemon's, character for character: never a period
    // inside it, even when the code itself ends in one.
    ['set `api_version`', 'set `api_version`. Your message was kept.'],
    ['ends in code `a.b.`', 'ends in code `a.b.`. Your message was kept.'],
    // A dangling separator is a template with nothing substituted.
    ['for the new chat:', 'for the new chat. Your message was kept.'],
    ['', 'Your message was kept.'],
  ])('%j', (daemon, toast) => {
    expect(appendSentence(daemon, KEPT)).toBe(toast);
  });
});
