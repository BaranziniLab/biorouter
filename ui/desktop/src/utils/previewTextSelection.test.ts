import { describe, expect, it } from 'vitest';
import {
  PREVIEW_SELECTION_MESSAGE_TYPE,
  previewSelectionFromMessage,
} from './previewTextSelection';

describe('sandbox preview selection reporting', () => {
  it('accepts only the known descendant preview frame, never a foreign window', () => {
    const root = document.createElement('div');
    const frame = document.createElement('iframe');
    frame.name = 'biorouter-artifact-preview';
    root.append(frame);
    document.body.append(root);
    const data = { type: PREVIEW_SELECTION_MESSAGE_TYPE, text: 'exact\n"text"', length: 12 };
    data.length = data.text.length;
    expect(
      previewSelectionFromMessage(
        root,
        new MessageEvent('message', { source: frame.contentWindow, data })
      )
    ).toBe(data.text);
    expect(
      previewSelectionFromMessage(root, new MessageEvent('message', { source: window, data }))
    ).toBeNull();
    expect(
      previewSelectionFromMessage(
        root,
        new MessageEvent('message', { source: frame.contentWindow, data: { ...data, length: 1 } })
      )
    ).toBeNull();
    const oldWindow = frame.contentWindow;
    frame.remove();
    expect(
      previewSelectionFromMessage(root, new MessageEvent('message', { source: oldWindow, data }))
    ).toBeNull();
    root.remove();
  });
  it('reports oversize visibly, without silently clipping text', () => {
    const root = document.createElement('div');
    const frame = document.createElement('iframe');
    frame.name = 'biorouter-artifact-preview';
    root.append(frame);
    document.body.append(root);
    const data = { type: PREVIEW_SELECTION_MESSAGE_TYPE, text: null, length: 16001 };
    expect(
      previewSelectionFromMessage(
        root,
        new MessageEvent('message', { source: frame.contentWindow, data })
      )
    ).toEqual({ error: 'Select at most 16,000 characters; this selection has 16,001.' });
    root.remove();
  });
});
