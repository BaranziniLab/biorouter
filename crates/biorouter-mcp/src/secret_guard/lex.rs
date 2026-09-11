//! Shell lexing for the secret guard (H1).
//!
//! The guard used to split a command on whitespace and test each token as a
//! literal path. That is not how a shell reads a command: `~` and `$HOME` are
//! expanded, globs are matched against the directory, quotes are removed and
//! the pieces concatenated, and `cd` moves the directory every later relative
//! path is resolved against. Each of those was a working bypass, measured on
//! 2026-09-10 (QA-C H1). This module tokenizes a command the way a POSIX shell
//! does, closely enough to know *which* characters the shell will expand;
//! [`super::expand`] and [`super::resolve`] do the rest.
//!
//! It is deliberately not a parser. It never decides that a command is
//! malformed and never has to be right about control flow: the resolver treats
//! every command it sees as one that may run, so being generous here only ever
//! adds refusals. An unterminated quote simply runs to the end of the input.

/// Stands for a value the guard cannot know — a command substitution's output,
/// an unset variable — inside text that is lexed again as a nested script
/// (`bash -c "$CMD"`). A private-use code point: it cannot occur in a real
/// argument unless someone writes it deliberately, and if they do it only makes
/// the guard more conservative.
pub(crate) const UNKNOWN: char = '\u{E000}';

/// One piece of a shell word, before expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece {
    /// Literal text. `quoted` means it came from quotes or a backslash escape,
    /// so the shell will neither glob it nor expand a `~` in it.
    Lit { text: String, quoted: bool },
    /// `$NAME`, `${NAME}`, `${NAME:-word}`, `$1`, `$@`, …
    Param {
        name: String,
        op: ParamOp,
        quoted: bool,
    },
    /// `$(…)`, `` `…` ``, `$((…))`, `<(…)`, `>(…)`: the value is unknown, and
    /// the text is itself a command that has to be scanned.
    Subst { source: String },
    /// See [`UNKNOWN`].
    Unknown,
}

/// What a `${…}` expansion does beyond naming its variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParamOp {
    Plain,
    /// `${X:-w}`, `${X-w}`, `${X:=w}`, `${X=w}`: the value of `X`, or `w`.
    OrDefault(Vec<Piece>),
    /// `${X:+w}`, `${X+w}`: `w`, or nothing.
    IfSet(Vec<Piece>),
    /// Everything else (`${#X}`, `${X#p}`, `${X/a/b}`, `${!X}`, `${X[i]}`):
    /// treated as unknowable rather than emulated.
    Opaque,
}

pub(crate) type Word = Vec<Piece>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token {
    Word(Word),
    /// `;`, `&`, `&&`, `||`, `|`, `|&`, `;;`, `(`, `)` or a newline.
    Op(&'static str),
    /// A redirection. The next [`Token::Word`] is its target, except for a
    /// here-document, whose delimiter is consumed here and whose body — read
    /// from the lines after the command — is attached as `body`.
    Redirect {
        op: String,
        body: Option<String>,
    },
}

/// Lex a whole command line (or script).
pub(crate) fn lex(input: &str) -> Vec<Token> {
    Lexer::new(input, false).run()
}

/// Lex the word inside `${X:-word}`, where blanks and operators are literal.
fn lex_param_word(input: &str) -> Word {
    let mut lexer = Lexer::new(input, true);
    lexer.read_word().unwrap_or_default()
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    out: Vec<Token>,
    /// Here-documents whose bodies start after the next newline:
    /// `(index of the Redirect token, delimiter, strip leading tabs)`.
    pending: Vec<(usize, String, bool)>,
    /// Inside `${X:-…}`: blanks and operators do not end the word.
    param_word: bool,
}

impl Lexer {
    fn new(input: &str, param_word: bool) -> Self {
        Self {
            chars: input.chars().collect(),
            i: 0,
            out: Vec::new(),
            pending: Vec::new(),
            param_word,
        }
    }

    fn cur(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.i + n).copied()
    }

    fn run(mut self) -> Vec<Token> {
        while let Some(c) = self.cur() {
            match c {
                ' ' | '\t' | '\r' => self.i += 1,
                '\\' if self.peek(1) == Some('\n') => self.i += 2,
                '\n' => {
                    self.i += 1;
                    self.out.push(Token::Op("\n"));
                    self.read_heredoc_bodies();
                }
                '#' => {
                    while self.cur().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                ';' => {
                    if self.peek(1) == Some(';') {
                        self.i += 2;
                        if self.cur() == Some('&') {
                            self.i += 1;
                        }
                        self.out.push(Token::Op(";;"));
                    } else if self.peek(1) == Some('&') {
                        self.i += 2;
                        self.out.push(Token::Op(";;"));
                    } else {
                        self.i += 1;
                        self.out.push(Token::Op(";"));
                    }
                }
                '&' => match self.peek(1) {
                    Some('&') => {
                        self.i += 2;
                        self.out.push(Token::Op("&&"));
                    }
                    Some('>') => self.read_redirect(),
                    _ => {
                        self.i += 1;
                        self.out.push(Token::Op("&"));
                    }
                },
                '|' => match self.peek(1) {
                    Some('|') => {
                        self.i += 2;
                        self.out.push(Token::Op("||"));
                    }
                    Some('&') => {
                        self.i += 2;
                        self.out.push(Token::Op("|&"));
                    }
                    _ => {
                        self.i += 1;
                        self.out.push(Token::Op("|"));
                    }
                },
                '(' => {
                    self.i += 1;
                    self.out.push(Token::Op("("));
                }
                ')' => {
                    self.i += 1;
                    self.out.push(Token::Op(")"));
                }
                '<' | '>' if self.peek(1) == Some('(') => self.push_word(),
                '<' | '>' => self.read_redirect(),
                c if c.is_ascii_digit() && self.fd_redirect_ahead() => {
                    while self.cur().is_some_and(|c| c.is_ascii_digit()) {
                        self.i += 1;
                    }
                    self.read_redirect();
                }
                _ => self.push_word(),
            }
        }
        self.out
    }

    /// `2>`, `10<&`: digits immediately followed by a redirection operator.
    fn fd_redirect_ahead(&self) -> bool {
        let mut j = self.i;
        while self.chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
        }
        matches!(self.chars.get(j), Some('<' | '>'))
    }

    fn read_redirect(&mut self) {
        let rest: String = self.chars[self.i..].iter().take(3).collect();
        let op = [
            "&>>", "<<<", "<<-", "&>", "<<", "<>", "<&", ">>", ">|", ">&", "<", ">",
        ]
        .into_iter()
        .find(|op| rest.starts_with(op))
        .unwrap_or(">");
        self.i += op.chars().count();

        if op == "<<" || op == "<<-" {
            while matches!(self.cur(), Some(' ' | '\t')) {
                self.i += 1;
            }
            let delimiter = self
                .read_word()
                .map(|word| {
                    word.iter()
                        .map(|piece| match piece {
                            Piece::Lit { text, .. } => text.clone(),
                            Piece::Param { name, .. } => format!("${name}"),
                            _ => String::new(),
                        })
                        .collect::<String>()
                })
                .unwrap_or_default();
            self.out.push(Token::Redirect {
                op: op.to_string(),
                body: None,
            });
            self.pending
                .push((self.out.len() - 1, delimiter, op == "<<-"));
            return;
        }
        self.out.push(Token::Redirect {
            op: op.to_string(),
            body: None,
        });
    }

    fn read_heredoc_bodies(&mut self) {
        for (index, delimiter, strip_tabs) in std::mem::take(&mut self.pending) {
            let mut body = String::new();
            loop {
                if self.i >= self.chars.len() {
                    break;
                }
                let start = self.i;
                while self.cur().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                let line: String = self.chars[start..self.i].iter().collect();
                if self.cur() == Some('\n') {
                    self.i += 1;
                }
                let compared = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if compared == delimiter {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            if let Some(Token::Redirect { body: slot, .. }) = self.out.get_mut(index) {
                *slot = Some(body);
            }
        }
    }

    fn push_word(&mut self) {
        match self.read_word() {
            Some(word) => self.out.push(Token::Word(word)),
            // A character that can neither start a word nor an operator; skip
            // it rather than loop on it.
            None => self.i += 1,
        }
    }

    /// Read one word starting at the cursor. `None` when nothing was consumed.
    fn read_word(&mut self) -> Option<Word> {
        let start = self.i;
        let mut pieces: Word = Vec::new();
        let mut lit = String::new();

        fn flush(lit: &mut String, pieces: &mut Word) {
            if !lit.is_empty() {
                pieces.push(Piece::Lit {
                    text: std::mem::take(lit),
                    quoted: false,
                });
            }
        }

        while let Some(c) = self.cur() {
            match c {
                ' ' | '\t' | '\r' | '\n' | ';' | '&' | '|' | ')' if !self.param_word => break,
                '(' if !self.param_word => {
                    if lit.is_empty() && pieces.is_empty() {
                        break;
                    }
                    // Mid-word parentheses are a zsh glob group or qualifier
                    // (`(a|b)`, `*(.)`) or a bash extglob (`@(a|b)`). Keep them
                    // in the word as glob syntax; `expand` decides what they mean.
                    let group = self.take_balanced_parens();
                    lit.push_str(&group);
                }
                '<' | '>' if !self.param_word => {
                    if self.peek(1) == Some('(') {
                        flush(&mut lit, &mut pieces);
                        self.i += 2;
                        let source = self.take_until_close_paren();
                        pieces.push(Piece::Subst { source });
                    } else {
                        break;
                    }
                }
                '\\' => match self.peek(1) {
                    Some('\n') => self.i += 2,
                    Some(next) => {
                        flush(&mut lit, &mut pieces);
                        pieces.push(Piece::Lit {
                            text: next.to_string(),
                            quoted: true,
                        });
                        self.i += 2;
                    }
                    None => {
                        lit.push('\\');
                        self.i += 1;
                    }
                },
                '\'' => {
                    flush(&mut lit, &mut pieces);
                    self.i += 1;
                    let text = self.take_until('\'');
                    pieces.push(Piece::Lit { text, quoted: true });
                }
                '"' => {
                    flush(&mut lit, &mut pieces);
                    self.i += 1;
                    self.read_double_quoted(&mut pieces);
                }
                '$' => {
                    flush(&mut lit, &mut pieces);
                    self.read_dollar(&mut pieces, false);
                }
                '`' => {
                    flush(&mut lit, &mut pieces);
                    self.i += 1;
                    let source = self.take_backtick();
                    pieces.push(Piece::Subst { source });
                }
                UNKNOWN => {
                    flush(&mut lit, &mut pieces);
                    pieces.push(Piece::Unknown);
                    self.i += 1;
                }
                _ => {
                    lit.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut lit, &mut pieces);
        (self.i > start).then_some(pieces)
    }

    fn read_double_quoted(&mut self, pieces: &mut Word) {
        let mut text = String::new();
        fn flush(text: &mut String, pieces: &mut Word) {
            if !text.is_empty() {
                pieces.push(Piece::Lit {
                    text: std::mem::take(text),
                    quoted: true,
                });
            }
        }
        // An empty `""` is still an argument.
        let mut produced = false;
        while let Some(c) = self.cur() {
            match c {
                '"' => {
                    self.i += 1;
                    break;
                }
                '\\' => match self.peek(1) {
                    Some('\n') => self.i += 2,
                    Some(next @ ('$' | '`' | '"' | '\\')) => {
                        text.push(next);
                        self.i += 2;
                    }
                    _ => {
                        text.push('\\');
                        self.i += 1;
                    }
                },
                '$' => {
                    flush(&mut text, pieces);
                    produced = true;
                    self.read_dollar(pieces, true);
                }
                '`' => {
                    flush(&mut text, pieces);
                    produced = true;
                    self.i += 1;
                    let source = self.take_backtick();
                    pieces.push(Piece::Subst { source });
                }
                UNKNOWN => {
                    flush(&mut text, pieces);
                    produced = true;
                    pieces.push(Piece::Unknown);
                    self.i += 1;
                }
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        if text.is_empty() && !produced {
            pieces.push(Piece::Lit {
                text: String::new(),
                quoted: true,
            });
        } else {
            flush(&mut text, pieces);
        }
    }

    /// At a `$`. Pushes the expansion (or a literal `$`) and advances.
    fn read_dollar(&mut self, pieces: &mut Word, quoted: bool) {
        self.i += 1;
        match self.cur() {
            Some('(') => {
                // `$((` is arithmetic; its body may still hold a `$(…)`, so it
                // is scanned like any other substitution.
                self.i += 1;
                let source = self.take_until_close_paren();
                pieces.push(Piece::Subst { source });
            }
            Some('{') => {
                self.i += 1;
                let body = self.take_until_close_brace();
                pieces.push(parse_param_body(&body, quoted));
            }
            Some('\'') if !quoted => {
                self.i += 1;
                let text = self.take_ansi_c();
                pieces.push(Piece::Lit { text, quoted: true });
            }
            Some('"') if !quoted => {
                self.i += 1;
                self.read_double_quoted(pieces);
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let mut name = String::new();
                while let Some(c) = self
                    .cur()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    name.push(c);
                    self.i += 1;
                }
                pieces.push(Piece::Param {
                    name,
                    op: ParamOp::Plain,
                    quoted,
                });
            }
            Some(c) if c.is_ascii_digit() || "@*#?-$!".contains(c) => {
                self.i += 1;
                pieces.push(Piece::Param {
                    name: c.to_string(),
                    op: ParamOp::Plain,
                    quoted,
                });
            }
            _ => pieces.push(Piece::Lit {
                text: "$".to_string(),
                quoted,
            }),
        }
    }

    fn take_until(&mut self, end: char) -> String {
        let mut out = String::new();
        while let Some(c) = self.cur() {
            self.i += 1;
            if c == end {
                break;
            }
            out.push(c);
        }
        out
    }

    /// The body of `` `…` ``, with the escapes a backquote honours removed.
    fn take_backtick(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.cur() {
            self.i += 1;
            match c {
                '`' => break,
                '\\' => match self.cur() {
                    Some(next @ ('`' | '\\' | '$')) => {
                        out.push(next);
                        self.i += 1;
                    }
                    _ => out.push('\\'),
                },
                _ => out.push(c),
            }
        }
        out
    }

    /// After an opening `(`: the text up to its matching `)`, which is consumed.
    /// Quotes and escapes are honoured so a `)` inside a string does not close
    /// it; the text itself is returned verbatim for a nested lex.
    fn take_until_close_paren(&mut self) -> String {
        let start = self.i;
        let mut depth = 1usize;
        while let Some(c) = self.cur() {
            match c {
                '\\' => self.i += 2,
                '\'' => {
                    self.i += 1;
                    self.take_until('\'');
                }
                '"' => {
                    self.i += 1;
                    self.skip_double_quoted();
                }
                '(' => {
                    depth += 1;
                    self.i += 1;
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let source: String = self.chars[start..self.i].iter().collect();
                        self.i += 1;
                        return source;
                    }
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
        self.i = self.i.min(self.chars.len());
        self.chars[start..self.i].iter().collect()
    }

    /// A mid-word `( … )` group, parentheses included, verbatim.
    fn take_balanced_parens(&mut self) -> String {
        let start = self.i;
        self.i += 1;
        self.take_until_close_paren();
        self.i = self.i.min(self.chars.len());
        self.chars[start..self.i].iter().collect()
    }

    /// After `${`: the text up to the matching `}`, which is consumed.
    fn take_until_close_brace(&mut self) -> String {
        let start = self.i;
        let mut depth = 1usize;
        while let Some(c) = self.cur() {
            match c {
                '\\' => self.i += 2,
                '\'' => {
                    self.i += 1;
                    self.take_until('\'');
                }
                '"' => {
                    self.i += 1;
                    self.skip_double_quoted();
                }
                '{' => {
                    depth += 1;
                    self.i += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let body: String = self.chars[start..self.i].iter().collect();
                        self.i += 1;
                        return body;
                    }
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
        self.i = self.i.min(self.chars.len());
        self.chars[start..self.i].iter().collect()
    }

    fn skip_double_quoted(&mut self) {
        while let Some(c) = self.cur() {
            match c {
                '\\' => self.i += 2,
                '"' => {
                    self.i += 1;
                    return;
                }
                _ => self.i += 1,
            }
        }
    }

    /// The body of `$'…'`, with its C escapes decoded.
    fn take_ansi_c(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.cur() {
            self.i += 1;
            match c {
                '\'' => break,
                '\\' => {
                    let Some(e) = self.cur() else {
                        out.push('\\');
                        break;
                    };
                    self.i += 1;
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'a' => out.push('\u{7}'),
                        'b' => out.push('\u{8}'),
                        'e' | 'E' => out.push('\u{1b}'),
                        'f' => out.push('\u{c}'),
                        'v' => out.push('\u{b}'),
                        '\\' | '\'' | '"' | '?' => out.push(e),
                        'x' => {
                            let value = self.take_radix(16, 2);
                            push_code(&mut out, value, 'x');
                        }
                        'u' => {
                            let value = self.take_radix(16, 4);
                            push_code(&mut out, value, 'u');
                        }
                        'U' => {
                            let value = self.take_radix(16, 8);
                            push_code(&mut out, value, 'U');
                        }
                        'c' => {
                            if let Some(ctrl) = self.cur() {
                                self.i += 1;
                                out.push(char::from((ctrl as u8 & 0x1f) as char as u8));
                            }
                        }
                        '0'..='7' => {
                            self.i -= 1;
                            let value = self.take_radix(8, 3);
                            push_code(&mut out, value, '0');
                        }
                        other => {
                            out.push('\\');
                            out.push(other);
                        }
                    }
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn take_radix(&mut self, radix: u32, max: usize) -> Option<u32> {
        let mut value: Option<u32> = None;
        for _ in 0..max {
            match self.cur().and_then(|c| c.to_digit(radix)) {
                Some(d) => {
                    value = Some(value.unwrap_or(0) * radix + d);
                    self.i += 1;
                }
                None => break,
            }
        }
        value
    }
}

fn push_code(out: &mut String, value: Option<u32>, escape: char) {
    match value.and_then(char::from_u32) {
        Some(c) => out.push(c),
        None => {
            out.push('\\');
            out.push(escape);
        }
    }
}

/// Interpret the text between `${` and `}`.
fn parse_param_body(body: &str, quoted: bool) -> Piece {
    let opaque = |name: &str| Piece::Param {
        name: name.to_string(),
        op: ParamOp::Opaque,
        quoted,
    };
    if (body.starts_with('#') && body.len() > 1) || body.starts_with('!') {
        return opaque(body.trim_start_matches(['#', '!']));
    }
    let chars: Vec<char> = body.chars().collect();
    let mut end = 0;
    if chars
        .first()
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
    {
        while chars
            .get(end)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
        {
            end += 1;
        }
    } else if chars.first().is_some_and(char::is_ascii_digit) {
        while chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
    } else if chars.first().is_some_and(|c| "@*#?-$!".contains(*c)) {
        end = 1;
    }
    let name: String = chars[..end].iter().collect();
    let rest: String = chars[end..].iter().collect();
    if name.is_empty() {
        return opaque(body);
    }
    let op = if rest.is_empty() {
        ParamOp::Plain
    } else if let Some(word) = rest
        .strip_prefix(":-")
        .or_else(|| rest.strip_prefix(":="))
        .or_else(|| rest.strip_prefix('-'))
        .or_else(|| rest.strip_prefix('='))
    {
        ParamOp::OrDefault(lex_param_word(word))
    } else if let Some(word) = rest.strip_prefix(":+").or_else(|| rest.strip_prefix('+')) {
        ParamOp::IfSet(lex_param_word(word))
    } else {
        ParamOp::Opaque
    };
    Piece::Param { name, op, quoted }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(text: &str, quoted: bool) -> Piece {
        Piece::Lit {
            text: text.to_string(),
            quoted,
        }
    }

    fn words(input: &str) -> Vec<Word> {
        lex(input)
            .into_iter()
            .filter_map(|t| match t {
                Token::Word(w) => Some(w),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn quotes_and_escapes_keep_their_quotedness() {
        let w = words(r#"cat ~/.aws/cred""entials 'a b' c\d"#);
        assert_eq!(w[0], vec![lit("cat", false)]);
        assert_eq!(
            w[1],
            vec![
                lit("~/.aws/cred", false),
                lit("", true),
                lit("entials", false)
            ]
        );
        assert_eq!(w[2], vec![lit("a b", true)]);
        assert_eq!(w[3], vec![lit("c", false), lit("d", true)]);
    }

    #[test]
    fn dollar_forms() {
        let w = words(r#"$HOME ${HOME} "${X:-~/.aws}" $(echo hi) `id` $'\x63' $1"#);
        assert!(
            matches!(&w[0][0], Piece::Param { name, op: ParamOp::Plain, quoted: false } if name == "HOME")
        );
        assert!(
            matches!(&w[1][0], Piece::Param { name, op: ParamOp::Plain, .. } if name == "HOME")
        );
        match &w[2][0] {
            Piece::Param {
                name,
                op: ParamOp::OrDefault(default),
                quoted: true,
            } => {
                assert_eq!(name, "X");
                assert_eq!(default, &vec![lit("~/.aws", false)]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            w[3],
            vec![Piece::Subst {
                source: "echo hi".into()
            }]
        );
        assert_eq!(
            w[4],
            vec![Piece::Subst {
                source: "id".into()
            }]
        );
        assert_eq!(w[5], vec![lit("c", true)]);
        assert!(matches!(&w[6][0], Piece::Param { name, .. } if name == "1"));
    }

    #[test]
    fn operators_and_redirects_split_commands() {
        let tokens = lex("cd ~/.aws && head credentials; cat <x 2>/dev/null | wc");
        let ops: Vec<&str> = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Op(op) => Some(*op),
                _ => None,
            })
            .collect();
        assert_eq!(ops, vec!["&&", ";", "|"]);
        let redirects: Vec<&str> = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Redirect { op, .. } => Some(op.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(redirects, vec!["<", ">"]);
    }

    #[test]
    fn heredoc_body_is_attached_to_its_redirect() {
        let tokens = lex("bash <<'EOF'\ncat ~/.aws/credentials\nEOF\necho done");
        let body = tokens.iter().find_map(|t| match t {
            Token::Redirect { op, body } if op == "<<" => body.clone(),
            _ => None,
        });
        assert_eq!(body.as_deref(), Some("cat ~/.aws/credentials\n"));
        // The delimiter is not a word; the command after the body still is.
        let w = words("bash <<'EOF'\ncat ~/.aws/credentials\nEOF\necho done");
        assert_eq!(w.len(), 3, "{w:?}");
    }

    #[test]
    fn unterminated_input_does_not_hang_or_panic() {
        for input in [
            "cat '~/.aws/cred",
            "cat \"$(echo",
            "cat ${HOME",
            "echo `x",
            "cat $'\\x",
            "a <<",
            "(((",
            "\\",
        ] {
            let _ = lex(input);
        }
    }
}
