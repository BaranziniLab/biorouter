//! Single-component glob matching for the secret guard.
//!
//! Two jobs, one small engine: matching a shell glob against the names a
//! directory really holds (`cat ~/.aws/cred*`), and matching one component of
//! a deny pattern against a literal name (`find ~ -name 'cred*'` against
//! `credentials`). Neither crosses a `/`, which is what keeps this simple
//! enough to trust.
//!
//! Matching is case-insensitive throughout. On the two filesystems where it
//! matters (APFS and NTFS) `~/.AWS/Credentials` opens the real file, and on a
//! case-sensitive one the only cost is refusing a name that differs from a
//! secret's by case — an acceptable price for a deny list.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    Lit(char),
    /// `?`
    Any,
    /// `*`
    Star,
    /// `[…]`: inclusive ranges, and whether the class is negated.
    Class(Vec<(char, char)>, bool),
}

/// Parse a glob. `glob_active(i)` says whether the character at index `i` may
/// act as glob syntax; a quoted `*` is a literal star.
pub(crate) fn parse(chars: &[(char, bool)]) -> Vec<Tok> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let (c, active) = chars[i];
        match c {
            '*' if active => {
                // `**` inside one component is just `*`.
                if out.last() != Some(&Tok::Star) {
                    out.push(Tok::Star);
                }
                i += 1;
            }
            '?' if active => {
                out.push(Tok::Any);
                i += 1;
            }
            '[' if active => match parse_class(chars, i) {
                Some((tok, next)) => {
                    out.push(tok);
                    i = next;
                }
                None => {
                    out.push(Tok::Lit('['));
                    i += 1;
                }
            },
            _ => {
                out.push(Tok::Lit(c));
                i += 1;
            }
        }
    }
    out
}

/// A pattern whose every character may act as glob syntax (deny patterns).
pub(crate) fn parse_pattern(pattern: &str) -> Vec<Tok> {
    let chars: Vec<(char, bool)> = pattern.chars().map(|c| (c, true)).collect();
    parse(&chars)
}

fn parse_class(chars: &[(char, bool)], open: usize) -> Option<(Tok, usize)> {
    let mut i = open + 1;
    let mut negated = false;
    if matches!(chars.get(i), Some(('!' | '^', _))) {
        negated = true;
        i += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let (c, _) = chars[i];
        if c == ']' && !first {
            return Some((Tok::Class(ranges, negated), i + 1));
        }
        first = false;
        if matches!(chars.get(i + 1), Some(('-', _)))
            && chars.get(i + 2).is_some_and(|(e, _)| *e != ']')
        {
            let (end, _) = chars[i + 2];
            ranges.push((c, end));
            i += 3;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
    None
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn class_contains(ranges: &[(char, char)], negated: bool, c: char) -> bool {
    let c = fold(c);
    let hit = ranges.iter().any(|(lo, hi)| {
        let (lo, hi) = (fold(*lo), fold(*hi));
        (lo..=hi).contains(&c)
    });
    hit != negated
}

/// Does `name` match `glob`?
///
/// `leading_dot_needs_literal` is the shell rule: a name starting with `.`
/// is only matched by a glob that starts with a literal `.`, so `*` never
/// reaches `.env`. Gitignore patterns do not have that rule.
pub(crate) fn matches(glob: &[Tok], name: &str, leading_dot_needs_literal: bool) -> bool {
    let name: Vec<char> = name.chars().collect();
    if leading_dot_needs_literal
        && name.first() == Some(&'.')
        && !matches!(glob.first(), Some(Tok::Lit('.')))
    {
        return false;
    }
    let mut memo = vec![vec![None; name.len() + 1]; glob.len() + 1];
    match_at(glob, &name, 0, 0, &mut memo)
}

fn match_at(
    glob: &[Tok],
    name: &[char],
    g: usize,
    n: usize,
    memo: &mut Vec<Vec<Option<bool>>>,
) -> bool {
    if let Some(done) = memo[g][n] {
        return done;
    }
    let result = match glob.get(g) {
        None => n == name.len(),
        Some(Tok::Star) => {
            match_at(glob, name, g + 1, n, memo)
                || (n < name.len() && match_at(glob, name, g, n + 1, memo))
        }
        Some(tok) => {
            n < name.len() && tok_accepts(tok, name[n]) && match_at(glob, name, g + 1, n + 1, memo)
        }
    };
    memo[g][n] = Some(result);
    result
}

fn tok_accepts(tok: &Tok, c: char) -> bool {
    match tok {
        Tok::Lit(l) => fold(*l) == fold(c),
        Tok::Any | Tok::Star => true,
        Tok::Class(ranges, negated) => class_contains(ranges, *negated, c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(glob: &str) -> Vec<Tok> {
        parse_pattern(glob)
    }

    #[test]
    fn shell_globs_match_like_a_shell() {
        assert!(matches(&shell("cred*"), "credentials", true));
        assert!(matches(&shell("*.pem"), "deploy.pem", true));
        assert!(matches(&shell("credential?"), "credentials", true));
        assert!(matches(&shell("cr[a-f]dentials"), "credentials", true));
        assert!(matches(&shell("cr[!x]dentials"), "credentials", true));
        assert!(!matches(&shell("cr[!e]dentials"), "credentials", true));
        assert!(!matches(&shell("*.pem"), "deploy.pub", true));
        // The shell's leading-dot rule.
        assert!(!matches(&shell("*"), ".env", true));
        assert!(matches(&shell(".*"), ".env", true));
        assert!(matches(&shell("*"), ".env", false));
        // Case-insensitive, on purpose.
        assert!(matches(&shell("CRED*"), "credentials", true));
    }

    #[test]
    fn quoted_glob_characters_are_literal() {
        let quoted: Vec<(char, bool)> = "cred*".chars().map(|c| (c, c != '*')).collect();
        let glob = parse(&quoted);
        assert!(!matches(&glob, "credentials", true));
        assert!(matches(&glob, "cred*", true));
    }

    #[test]
    fn pathological_globs_stay_bounded() {
        let glob = shell(&"*a".repeat(40));
        let name = "a".repeat(200);
        assert!(matches(&glob, &name, true));
        assert!(!matches(&glob, &"b".repeat(200), true));
    }
}
