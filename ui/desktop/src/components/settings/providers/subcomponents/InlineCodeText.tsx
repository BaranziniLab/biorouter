/**
 * Text the daemon wrote with Markdown backticks around a command (`claude`), drawn as inline code
 * instead of with the literal marks (V4-26). A provider's description comes from the daemon, so
 * the renderer cannot rewrite the sentence; it can only draw its machine string the way a machine
 * string is drawn everywhere else (mono, principle 3). Only a closed pair of backticks on one line
 * counts: a stray one stays as text, and nothing else is interpreted, so the text can never
 * become markup.
 */
export function InlineCodeText({ text }: { text: string }) {
  // `split` with one capture group alternates plain text (even) and the captured code (odd).
  const parts = text.split(/`([^`\n]+)`/);
  return (
    <>
      {parts.map((part, index) =>
        index % 2 === 1 ? (
          <code key={index} className="font-mono">
            {part}
          </code>
        ) : (
          part
        )
      )}
    </>
  );
}
