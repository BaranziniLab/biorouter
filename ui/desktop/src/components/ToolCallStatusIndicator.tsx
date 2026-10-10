/**
 * A tool call's status, as the row reports it to a screen reader.
 *
 * The visible glyph stays undecorated (Codex draws no status badge either): the
 * verb carries the state ("Running", "Ran", "Failed to", "Stopped"), and
 * `TranscriptRow` renders `Tool status: {status}` as a visually hidden image
 * beside the glyph, with the glyph's kind stamped as `data-tool-glyph`.
 */
export type ToolCallStatus = 'pending' | 'loading' | 'success' | 'error';
