//! Content-only adapter preserving mpatch 0.2.0 matching semantics.
// The matcher below is adapted from mpatch 0.2.0; its MIT notice follows.
/*

Copyright 2025 Romelium

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

*/
use similar::TextDiff;

pub(super) fn apply_content(
    patch: &mpatch::Patch,
    content: Option<&str>,
) -> Option<(String, bool)> {
    if content.is_none()
        && patch
            .hunks
            .first()
            .is_none_or(|hunk| !hunk.get_match_block().is_empty())
    {
        return None;
    }
    let mut lines: Vec<String> = content
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    let mut complete = true;
    for hunk in &patch.hunks {
        if !hunk.has_changes() {
            continue;
        }
        let matches = hunk.get_match_block();
        match find_hunk_location(&matches, &lines, 0.7) {
            Some(start) => {
                lines.splice(
                    start..start + matches.len(),
                    hunk.get_replace_block().into_iter().map(String::from),
                );
            }
            None => complete = false,
        }
    }
    let mut updated = lines.join("\n");
    if patch.ends_with_newline && !updated.is_empty() {
        updated.push('\n');
    }
    Some((updated, complete))
}

fn find_hunk_location(
    match_block: &[&str],
    target_lines: &[String],
    fuzz_threshold: f32,
) -> Option<usize> {
    if match_block.is_empty() {
        return target_lines.is_empty().then_some(0);
    }
    if match_block.len() > target_lines.len() {
        return None;
    }
    let exact: Vec<_> = target_lines
        .windows(match_block.len())
        .enumerate()
        .filter(|(_, window)| *window == match_block)
        .map(|(index, _)| index)
        .collect();
    if !exact.is_empty() {
        return (exact.len() == 1).then_some(exact[0]);
    }
    let stripped: Vec<_> = match_block.iter().map(|line| line.trim_end()).collect();
    let whitespace: Vec<_> = target_lines
        .windows(match_block.len())
        .enumerate()
        .filter(|(_, window)| {
            window
                .iter()
                .map(|line| line.trim_end())
                .eq(stripped.iter().copied())
        })
        .map(|(index, _)| index)
        .collect();
    if !whitespace.is_empty() {
        return (whitespace.len() == 1).then_some(whitespace[0]);
    }
    if fuzz_threshold <= 0.0 {
        return None;
    }
    let mut best_ratio = -1.0;
    let mut potential = Vec::new();
    for (index, window) in target_lines.windows(match_block.len()).enumerate() {
        let window_content = window.join("\n");
        let match_content = match_block.join("\n");
        let ratio = f64::from(TextDiff::from_chars(&window_content, &match_content).ratio());
        if ratio > best_ratio {
            best_ratio = ratio;
            potential.clear();
            potential.push(index);
        } else if (ratio - best_ratio).abs() < 1e-9 {
            potential.push(index);
        }
    }
    if best_ratio >= f64::from(fuzz_threshold) && potential.len() == 1 {
        Some(potential[0])
    } else {
        None
    }
}
