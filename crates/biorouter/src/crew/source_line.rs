//! The daemon's provenance line on what an agent posts (Q3-02, W2-DMN-12).
//!
//! A task's result and a connected chat's post both end with a line the daemon writes from what
//! the broker returned to the run's reads, never from the model's words. It lived in the task
//! route alone, so a chat's `run.project` post carried none, and a "Source:" line the model
//! wrote (or was steered into writing) was the last thing in it and looked like the daemon's.

/// The broker's limit on one message body, in bytes (`biorouter-crew`'s `message too long`).
pub const MAX_POSTED_BYTES: usize = 65_536;

/// Said where a reply was cut to fit [`MAX_POSTED_BYTES`].
pub const SHORTENED_NOTE: &str = "(This reply was shortened to fit the channel.)";

/// The line a task's result ends with when the run read no shared file.
pub const NO_FILE_READ_FOR_RESULT: &str = "No shared file was read for this result.";

/// The line a connected chat's post ends with when the chat read no shared file since its last
/// post (W2-DMN-12).
pub const NO_FILE_READ_FOR_POST: &str = "No shared file was read for this post.";

/// The result as posted (Q3-02): the agent's reply exactly as written, then the daemon's own
/// line, always. When the run read shared files it names them (``Source: `gina-assay.csv`,
/// shared by Gina Rossi (@crew_gina).``); when it read none it says so (`no_file`). The
/// line is built from what the broker returned to the run's requests, never from the model's
/// words, so it is true whatever the reply says.
///
/// The line's place is what marks it as the daemon's: it is always there and always last, so
/// a "Source:" line the model wrote (or was steered into writing) is never the last thing in
/// the post, even on a run that read nothing. Nothing is removed from the reply: a closing
/// "Sources:" list the model wrote may be data (a specimen's source), and removing text by its
/// shape both deleted answers and was defeated by an invisible last line. The one thing the
/// channel draws after the body is its footnotes, so a footnote definition in the reply is
/// escaped ([`without_footnote_definitions`]) and shows where it was written. Its line endings
/// are written as `\n` first ([`with_newline_endings`]): the channel's Markdown also ends a line
/// at `\r\n` and at a lone `\r`, so a definition after a bare `\r` was a line the parser saw
/// and the escaping did not. What the reply can still change is how the line looks, not where
/// it is: an unclosed code fence draws it in that code block, and an unclosed raw-HTML block
/// (which this channel shows as text) as plain text with its backticks; either way it is still
/// the last thing drawn, in the daemon's words. A reply too long to post with the line is cut,
/// and says so, rather than failing to post.
pub fn with_source_line(response: String, source: Option<String>, no_file: &str) -> String {
    let reply = without_footnote_definitions(with_newline_endings(&response).trim_end());
    let reply = reply.trim_end();
    let line = format!("\n\n{}", source.as_deref().unwrap_or(no_file));
    if reply.len() + line.len() <= MAX_POSTED_BYTES {
        return format!("{reply}{line}");
    }
    let mut cut = MAX_POSTED_BYTES
        .saturating_sub(line.len() + SHORTENED_NOTE.len() + 2)
        .min(reply.len());
    while !reply.is_char_boundary(cut) {
        cut -= 1;
    }
    let kept = reply.get(..cut).unwrap_or_default().trim_end();
    format!("{kept}\n\n{SHORTENED_NOTE}{line}")
}

/// `reply` with each of the line endings CommonMark reads (`\r\n`, and `\r` alone) written as
/// `\n`, so every line the channel's Markdown sees is a line [`without_footnote_definitions`]
/// sees. The channel draws the three alike, so nothing drawn changes. Nothing else ends a line
/// there: U+2028, U+2029, NEL, VT and FF leave `[^1]:` inside its paragraph (measured against
/// the channel's react-markdown + remark-gfm + remark-breaks, the stack
/// `ui/desktop/src/components/crew/daemonSourceLine.render.test.tsx` renders).
fn with_newline_endings(reply: &str) -> String {
    reply.replace("\r\n", "\n").replace('\r', "\n")
}

/// `reply` with every line that could open a GFM footnote definition (`[^label]:`, after any
/// indentation, `>` and list markers) escaped as `\[^label]:`, so the channel draws it as the
/// text it is, where it is, instead of in a footnote section after the daemon's line. Nothing is
/// removed; outside code the backslash does not show. A code line that starts with `[^…]:`
/// gains a visible backslash, which is the price of not parsing Markdown here. Lines end at
/// `\n` only: the caller writes every other line ending as `\n` first
/// ([`with_newline_endings`]).
fn without_footnote_definitions(reply: &str) -> String {
    let mut escaped = String::with_capacity(reply.len());
    for (n, line) in reply.split('\n').enumerate() {
        if n > 0 {
            escaped.push('\n');
        }
        match footnote_definition_at(line).and_then(|at| line.split_at_checked(at)) {
            Some((prefix, definition)) => {
                escaped.push_str(prefix);
                escaped.push('\\');
                escaped.push_str(definition);
            }
            None => escaped.push_str(line),
        }
    }
    escaped
}

/// Where the `[` of a footnote definition would open on `line`: a `[^` past whitespace, `>`
/// and list markers (`-`, `*`, `+`, `1.`, `1)`). Wider than GFM's rule on purpose (a label
/// holding an escaped `]`, say): escaping a `[` that opened nothing changes nothing drawn.
fn footnote_definition_at(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut at = 0;
    loop {
        while at < bytes.len() && matches!(bytes[at], b' ' | b'\t' | b'>' | b'-' | b'*' | b'+') {
            at += 1;
        }
        let digits = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 0 && matches!(bytes.get(at + digits), Some(b'.' | b')')) {
            at += digits + 1;
            continue;
        }
        break;
    }
    line.get(at..)?.starts_with("[^").then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The posted bodies the desktop draws through the channel's own Markdown stack
    /// (`daemonSourceLine.render.test.tsx`): the file holds exactly what [`with_source_line`]
    /// posts for each reply, a task's result or (`kind: "post"`) a connected chat's own post.
    /// After changing it, rewrite the file with `BIOROUTER_WRITE_SOURCE_LINE_CASES=1` and run
    /// the render test again.
    #[test]
    fn the_desktop_render_cases_are_what_the_daemon_posts() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/crew/daemonSourceLine.cases.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let mut fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
        let rewrite = std::env::var_os("BIOROUTER_WRITE_SOURCE_LINE_CASES").is_some();
        let cases = fixture["cases"].as_array_mut().expect("cases");
        assert!(cases.iter().any(|case| case["kind"] == "post"));
        for case in cases {
            let reply = case["reply"].as_str().expect("reply").to_owned();
            let source = case["source"].as_str().map(str::to_owned);
            let no_file = if case["kind"] == "post" {
                NO_FILE_READ_FOR_POST
            } else {
                NO_FILE_READ_FOR_RESULT
            };
            let line = source.clone().unwrap_or_else(|| no_file.to_owned());
            let posted = with_source_line(reply, source, no_file);
            if rewrite {
                case["posted"] = json!(posted);
                case["line"] = json!(line);
            } else {
                assert_eq!(case["posted"], json!(posted), "{}", case["name"]);
                assert_eq!(case["line"], json!(line), "{}", case["name"]);
            }
        }
        if rewrite {
            let mut text = serde_json::to_string_pretty(&fixture).unwrap();
            text.push('\n');
            std::fs::write(&path, text).unwrap();
        }
    }

    /// W2-DMN-12: a chat post that read nothing says so in its own words, last, below a
    /// "Source:" line the model wrote.
    #[test]
    fn a_chat_post_that_read_nothing_says_so_last() {
        assert_eq!(
            with_source_line(
                "Done.\n\nSource: `FAKE.csv`".into(),
                None,
                NO_FILE_READ_FOR_POST
            ),
            "Done.\n\nSource: `FAKE.csv`\n\nNo shared file was read for this post."
        );
    }
}
