// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/**
 * The message editor never writes message text to the log (DD-69, P-2 in
 * docs/history/local-dictation-2026-11/implementation-plan.md).
 *
 * `window.electron.logInfo` lands in the packaged app's log file. The editor
 * used to log the whole message when editing began and again on every change,
 * and dictated text reaches the editor through that change handler, so a
 * dictated message would have gone to disk. Both calls are gone; this test
 * reads every remaining `logInfo` call in `UserMessage.tsx` and refuses any
 * whose argument could carry the text: a template may interpolate only the
 * names below, and anything that is not a plain literal or such a template is
 * refused because nothing here can tell what it holds.
 */
const ALLOWED_INTERPOLATIONS = new Set([
  'message.id',
  'newEditingState',
  // `handleKeyDown` logs each key name with its Meta and Ctrl state. Dictated
  // text never passes through it; DD-69 leaves it, and the lead author may
  // remove it under DD-93.
  'e.key',
  'e.metaKey',
  'e.ctrlKey',
]);

/** The argument text of every `logInfo(...)` call, read with quote and bracket awareness. */
function logInfoArguments(text: string): string[] {
  const found: string[] = [];
  const marker = /\blogInfo\s*\(/g;
  for (let match = marker.exec(text); match; match = marker.exec(text)) {
    const start = match.index + match[0].length;
    const stack: string[] = ['('];
    let i = start;
    for (; i < text.length && stack.length > 0; i++) {
      const c = text[i];
      const top = stack[stack.length - 1];
      if (top === "'" || top === '"') {
        if (c === '\\') i++;
        else if (c === top) stack.pop();
      } else if (top === '`') {
        if (c === '\\') i++;
        else if (c === '`') stack.pop();
        else if (c === '$' && text[i + 1] === '{') {
          stack.push('{');
          i++;
        }
      } else if (c === "'" || c === '"' || c === '`' || c === '(' || c === '{') {
        stack.push(c);
      } else if (c === ')' || c === '}') {
        stack.pop();
      }
    }
    if (stack.length > 0) throw new Error(`unclosed logInfo( call at offset ${match.index}`);
    found.push(
      text
        .slice(start, i - 1)
        .trim()
        .replace(/,$/, '')
        .trim()
    );
  }
  return found;
}

/** Why a `logInfo` argument might carry message text, or null when it cannot. */
function contentRisk(argument: string): string | null {
  if (/^'(?:[^'\\\n]|\\.)*'$/.test(argument) || /^"(?:[^"\\\n]|\\.)*"$/.test(argument)) {
    return null;
  }
  const template = /^`((?:[^`\\]|\\.)*)`$/s.exec(argument);
  if (!template) return `not a literal, so its contents are unknown: ${argument}`;
  for (const [, expression] of template[1].matchAll(/\$\{([^}]*)\}/g)) {
    if (!ALLOWED_INTERPOLATIONS.has(expression.trim())) {
      return `interpolates ${expression.trim()}: ${argument}`;
    }
  }
  return null;
}

function contentBearingLogs(text: string): string[] {
  return logInfoArguments(text)
    .map(contentRisk)
    .filter((risk): risk is string => risk !== null);
}

describe('message editor logging', () => {
  it('logs no message text from UserMessage.tsx', () => {
    const source = readFileSync(resolve(__dirname, 'UserMessage.tsx'), 'utf8');
    expect(contentBearingLogs(source)).toEqual([]);
  });

  it('refuses the two calls DD-69 deleted, and any other way to carry text', () => {
    const removed = [
      'window.electron.logInfo(`Entering edit mode with content: ${displayText}`);',
      'window.electron.logInfo(`Content changed: ${newContent}`);',
      'window.electron.logInfo("Saving: " + editContent);',
      'window.electron.logInfo(editContent);',
      'window.electron.logInfo(`Saved ${editContent.slice(0, 10)}`);',
      'window.electron.logInfo(\n  `Long ${message.id} and ${value}`\n);',
    ];
    for (const call of removed) {
      expect(contentBearingLogs(call), call).toHaveLength(1);
    }
  });

  it('accepts literals and templates over the allowed names', () => {
    const kept = [
      "window.electron.logInfo('Cancel clicked - reverting to original content');",
      'window.electron.logInfo(`Edit state toggled: ${newEditingState} for message: ${message.id}`);',
      'window.electron.logInfo(\n  `Key pressed: ${e.key}, metaKey: ${e.metaKey}, ctrlKey: ${e.ctrlKey}`\n);',
      'window.electron.logInfo("Cmd+Enter (detected)");',
    ];
    for (const call of kept) {
      expect(logInfoArguments(call), call).toHaveLength(1);
      expect(contentBearingLogs(call), call).toEqual([]);
    }
  });
});
