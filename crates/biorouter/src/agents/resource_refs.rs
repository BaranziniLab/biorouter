//! Explicit resource references — the skills, extensions and knowledge bases a
//! user attaches to a message before sending it.
//!
//! # The canonical form is a tag
//!
//! ```text
//! <biorouter-ref type="skill" name="single-cell RNA &quot;QC&quot;">
//! <biorouter-ref type="extension" name="Chat Recall">
//! <biorouter-ref type="knowledge_base" id="soul" label="Soul &amp; Body">
//! ```
//!
//! Everything a composer emits uses this form. It exists because the compact
//! `/skill:name` markers cannot carry a name with a space in it: the extractor
//! for those splits the message on whitespace, so `/skill:my skill` reaches the
//! resolver as `my` and resolves to nothing (issue #65). Normalising the name
//! away — the fix that worked for `/ext:` in #60, where every consumer
//! re-normalises anyway — cannot transfer, because `skills_extension`'s
//! `loadSkill` looks the name up *exactly*: `myskill` is not `my skill`, so
//! that fix would trade a truncation for a quieter failure.
//!
//! The tag delimits the value explicitly instead of by whitespace, so any name
//! survives it.
//!
//! # Escaping — the contract other implementations must match
//!
//! Attribute values are escaped with a **fixed six-entry table** of XML
//! character references, applied to every occurrence regardless of position:
//!
//! | character | escaped as |
//! |-----------|------------|
//! | `&`       | `&amp;`    |
//! | `"`       | `&quot;`   |
//! | `<`       | `&lt;`     |
//! | `>`       | `&gt;`     |
//! | `\n`      | `&#10;`    |
//! | `\r`      | `&#13;`    |
//!
//! XML entities were chosen over percent-encoding or backslash escapes because
//! the syntax is already XML-shaped: `&quot;` is what a reader — human or
//! machine — expects to find inside `name="…"`, and it keeps the tag parseable
//! by a real XML parser should anyone ever want to use one.
//!
//! The table is **closed**. There is no general numeric-character-reference
//! support and no `&apos;`/`&nbsp;`: `&#65;` decodes to the literal text
//! `&#65;`, not to `A`. A closed table is a map an implementation in another
//! language can copy verbatim, which is the point — [`encode_ref_value`] and
//! [`decode_ref_value`] are exact inverses of each other and every emitter must
//! agree with them.
//!
//! Two rules make or break an implementation:
//!
//! * **Encoding must escape `&` first.** Written as a chain of replacements,
//!   `"<"` → `"&lt;"` → (`&` pass) → `"&amp;lt;"` corrupts every other escape.
//!   The implementation here iterates characters once and maps each
//!   independently, which is order-free by construction; a chained
//!   `String.replace` port must do `&` before all others.
//! * **Decoding must resolve `&amp;` last** for the mirror-image reason —
//!   otherwise a name containing the literal text `&quot;`, which encodes to
//!   `&amp;quot;`, decodes back to `"`. [`decode_ref_value`] scans left to
//!   right and consumes a whole entity at a time, which again removes the
//!   ordering question; a chained port must do `&amp;` last.
//!
//! `'` is deliberately *not* escaped, and single-quoted attribute values are
//! *not* accepted, so there is exactly one quoting style to agree on.
//!
//! # What a producer must guarantee
//!
//! A value is scanned to the next raw `"`, and a tag is abandoned at the next
//! raw `<`. Both are unreachable in correctly encoded output, so this costs
//! nothing — but a hand-written tag that skips the encoding will truncate.
//!
//! # Compatibility: the compact markers still parse
//!
//! `/skill:`, `/ext:`, `/kb:`, `/skill(…)`, `kb_id:` and the quoted legacy
//! phrases all still resolve. They appear in persisted sessions, saved
//! workflows, skill documents and the CLI completer, so removing them would
//! break messages already on disk. They keep their whitespace limitation — that
//! is now an acceptable tradeoff rather than a bug, because a producer that
//! needs to carry a name the compact form cannot represent has the tag to reach
//! for, and `reference_marker` picks between the two automatically.
//!
//! Those compatibility extractors scan raw message text with no delimiters of
//! their own, so they have always been able to fire on prose that merely looks
//! like a reference — `kb_id:` in a sentence selects a knowledge base. That
//! predates the tag and is left alone deliberately; `inline_marker_round_trips`
//! at least refuses to *choose* the compact form for a value that would trip
//! one.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnowledgeBaseRef {
    pub id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ResourceRefs {
    pub skills: Vec<String>,
    pub extensions: Vec<String>,
    pub knowledge_bases: Vec<KnowledgeBaseRef>,
}

impl ResourceRefs {
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.extensions.is_empty() && self.knowledge_bases.is_empty()
    }
}

/// The element name every reference tag uses.
pub const REF_TAG_NAME: &str = "biorouter-ref";

/// The escape table. Closed by design — see the module docs.
///
/// Order is irrelevant to both [`encode_ref_value`] and [`decode_ref_value`]:
/// each maps one character or one whole entity at a time rather than sweeping
/// the string once per rule, so neither can corrupt an escape it already
/// produced. No two entities share a first character after the `&`, so the
/// decoder's match is unambiguous.
const REF_ENTITIES: &[(char, &str)] = &[
    ('&', "&amp;"),
    ('"', "&quot;"),
    ('<', "&lt;"),
    ('>', "&gt;"),
    ('\n', "&#10;"),
    ('\r', "&#13;"),
];

/// Which kind of resource a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Skill,
    Extension,
    KnowledgeBase,
}

impl RefKind {
    /// The `type="…"` keyword. A closed set, so it is never escaped.
    pub fn tag_type(self) -> &'static str {
        match self {
            RefKind::Skill => "skill",
            RefKind::Extension => "extension",
            RefKind::KnowledgeBase => "knowledge_base",
        }
    }

    /// The attribute the resource's identity travels in. Knowledge bases are
    /// named by `id` because that is what `kb_search` takes; skills and
    /// extensions by `name`.
    pub fn value_attr(self) -> &'static str {
        match self {
            RefKind::KnowledgeBase => "id",
            _ => "name",
        }
    }
}

/// Escape `value` for an attribute of a reference tag.
///
/// The inverse of [`decode_ref_value`]; see the module docs for the table and
/// for why the two must not be implemented as chained replacements.
pub fn encode_ref_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match REF_ENTITIES.iter().find(|(escaped, _)| *escaped == ch) {
            Some((_, entity)) => out.push_str(entity),
            None => out.push(ch),
        }
    }
    out
}

/// Unescape an attribute value from a reference tag.
///
/// Anything that is not one of the six entities in the table is passed through
/// untouched, including a lone `&`, an unknown entity (`&nbsp;`) and a numeric
/// reference outside the table (`&#65;`). Dropping them would silently mangle
/// names, and guessing at them would make this disagree with the encoder.
pub fn decode_ref_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(amp) = rest.find('&') {
        out.push_str(slice_to(rest, amp));
        let tail = slice_from(rest, amp);
        match REF_ENTITIES
            .iter()
            .find(|(_, entity)| tail.starts_with(*entity))
        {
            Some((decoded, entity)) => {
                out.push(*decoded);
                rest = slice_from(tail, entity.len());
            }
            // Not an entity we emit: keep the `&` verbatim and carry on from
            // the next character, so `&amp;` inside `&amp;amp;` still decodes
            // exactly once.
            None => {
                out.push('&');
                rest = slice_from(tail, '&'.len_utf8());
            }
        }
    }

    out.push_str(rest);
    out
}

/// The canonical tag naming `value`.
pub fn ref_tag(kind: RefKind, value: &str) -> String {
    format!(
        "<{REF_TAG_NAME} type=\"{}\" {}=\"{}\">",
        kind.tag_type(),
        kind.value_attr(),
        encode_ref_value(value)
    )
}

/// The canonical tag naming `value`, carrying a display string for the chip a
/// UI renders in its place.
///
/// Only a knowledge base's label is read back out (it has somewhere to go — see
/// `KnowledgeBaseRef::label`); on the other kinds it is presentation-only and
/// the parser ignores it, which is exactly the tolerance a composer needs to
/// add attributes without coordinating with this module.
pub fn labelled_ref_tag(kind: RefKind, value: &str, label: &str) -> String {
    format!(
        "<{REF_TAG_NAME} type=\"{}\" {}=\"{}\" label=\"{}\">",
        kind.tag_type(),
        kind.value_attr(),
        encode_ref_value(value),
        encode_ref_value(label)
    )
}

/// The compact marker for `value` — `/skill:rna-qc`, `/ext:developer`,
/// `/kb:soul`.
///
/// Readable, but only able to carry a value that contains no whitespace and no
/// trailing `,` `.` `;`. Use [`reference_marker`], which checks.
pub fn inline_marker(kind: RefKind, value: &str) -> String {
    let prefix = match kind {
        RefKind::Skill => "/skill:",
        RefKind::Extension => "/ext:",
        RefKind::KnowledgeBase => "/kb:",
    };
    format!("{prefix}{value}")
}

/// Whether [`inline_marker`] can carry `value` without losing anything.
///
/// Answered by running the real extractor over the marker and comparing, rather
/// than by restating the extractor's rules — a restatement is a second copy
/// that drifts, and the drift is silent by construction: the marker still looks
/// fine, it just names a different resource.
///
/// The marker must yield that one reference and *nothing else*. A compact
/// marker is raw text in the message, so the compatibility extractors read it
/// too: a skill named `kb_id:x` would produce a valid-looking `/skill:kb_id:x`
/// that also silently selects knowledge base `x`. Requiring a clean read costs
/// nothing and sends anything ambiguous to the tag.
pub fn inline_marker_round_trips(kind: RefKind, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let refs = extract_resource_refs(&inline_marker(kind, value));
    let total = refs.skills.len() + refs.extensions.len() + refs.knowledge_bases.len();
    total == 1 && select_refs(refs, kind) == [value]
}

/// The reference to insert into a message for `value`: the compact marker when
/// it provably survives, the canonical tag otherwise.
///
/// The tag is always correct and is what the desktop composer emits, because it
/// renders the tag as a chip and the user never sees the markup. A terminal has
/// no chip to render into, so the CLI keeps the readable form for the names it
/// can represent — which is almost all of them — and reaches for the tag exactly
/// when the alternative is the silent truncation of issue #65. Both front-ends
/// therefore agree on the guarantee that matters: whatever the user picked is
/// what the agent resolves.
///
/// Flip this to always return [`ref_tag`] once the CLI can render a chip of its
/// own; nothing else needs to change.
pub fn reference_marker(kind: RefKind, value: &str) -> String {
    if inline_marker_round_trips(kind, value) {
        inline_marker(kind, value)
    } else {
        ref_tag(kind, value)
    }
}

/// What the agent will extract from `text` for one kind of resource.
///
/// The narrow public view over the extractor. A front-end uses it to assert
/// that what it inserts into a message is what the agent reads back out, which
/// is the only definition of "the interfaces agree" that survives contact with
/// a name nobody anticipated.
pub fn extracted_refs(kind: RefKind, text: &str) -> Vec<String> {
    select_refs(extract_resource_refs(text), kind)
}

fn select_refs(refs: ResourceRefs, kind: RefKind) -> Vec<String> {
    match kind {
        RefKind::Skill => refs.skills,
        RefKind::Extension => refs.extensions,
        RefKind::KnowledgeBase => refs
            .knowledge_bases
            .into_iter()
            .map(|base| base.id)
            .collect(),
    }
}

pub(crate) fn extract_resource_refs(text: &str) -> ResourceRefs {
    let unquoted = without_quoted_source_data(text);
    let text = unquoted.as_str();
    let mut refs = ResourceRefs::default();

    extract_tag_refs(text, &mut refs);

    extract_legacy_resource_phrases(text, "skill", &mut refs.skills);
    extract_legacy_resource_phrases(text, "extension", &mut refs.extensions);
    extract_inline_refs(text, "skill:", &mut refs.skills);
    extract_inline_refs(text, "ext:", &mut refs.extensions);
    extract_inline_kb_refs(text, &mut refs.knowledge_bases);
    extract_function_refs(text, "/skill(", ")", &mut refs.skills);
    extract_function_refs(text, "/ext(", ")", &mut refs.extensions);
    extract_function_kb_refs(text, &mut refs.knowledge_bases);
    extract_legacy_kb_refs(text, &mut refs.knowledge_bases);

    dedup(&mut refs.skills);
    dedup(&mut refs.extensions);
    dedup_kbs(&mut refs.knowledge_bases);

    refs
}

// Quoted document/response text is context, never an explicit capability selection.
// Validate the same bounded data envelope the composer emits; malformed markup
// remains ordinary text so it cannot hide an actual user-selected reference.
fn without_quoted_source_data(text: &str) -> String {
    const OPEN: &str = "<biorouter-quote>";
    const CLOSE: &str = "</biorouter-quote>";
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((prefix, body)) = rest.split_once(OPEN) {
        output.push_str(prefix);
        let Some((raw, remaining)) = body.split_once(CLOSE) else {
            output.push_str(OPEN);
            output.push_str(body);
            return output;
        };
        let raw = if let Some((malformed_prefix, candidate)) = raw.rsplit_once(OPEN) {
            output.push_str(OPEN);
            output.push_str(malformed_prefix);
            candidate
        } else {
            raw
        };
        let valid = raw.len() <= 200_000
            && !raw.contains(['<', '>'])
            && serde_json::from_str::<serde_json::Value>(raw)
                .ok()
                .is_some_and(|value| {
                    value
                        .get("source")
                        .is_some_and(serde_json::Value::is_string)
                        && value
                            .get("text")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|text| text.encode_utf16().count() <= 16_000)
                        && ["locator", "revision"]
                            .iter()
                            .all(|key| value.get(key).is_none_or(serde_json::Value::is_string))
                });
        if valid {
            output.push(' ');
        } else {
            output.push_str(OPEN);
            output.push_str(raw);
            output.push_str(CLOSE);
        }
        rest = remaining;
    }
    output.push_str(rest);
    output
}

// A `/ext:` reference is resolved to a concrete extension by
// `extension_manager::resolve_bundled_extension`, which keys off the extension
// id *and* the registry that owns it. This module only extracts the raw
// reference text from the message.

/// One parsed tag's attributes, in source order, values still escaped.
type TagAttrs = Vec<(String, String)>;

/// Pull every `<biorouter-ref …>` out of `text`.
///
/// Tags are honoured wherever they appear, **including inside a code fence**.
/// Skipping fenced regions would mean carrying a Markdown parser here for a
/// message that is not necessarily Markdown, and it would single out this one
/// syntax: `/skill:`, `kb_id:` and the quoted legacy phrases have never been
/// fence-aware either. The cost of honouring one is bounded and visible — the
/// user sees the resource announced in the reply — whereas dropping a chip a
/// user deliberately placed inside a fenced block is silent.
fn extract_tag_refs(text: &str, refs: &mut ResourceRefs) {
    for attrs in parse_ref_tags(text) {
        // The raw value is trimmed *before* decoding, so surrounding slop in a
        // hand-written tag is forgiven while an encoded `&#10;` at either end
        // is not mistaken for slop and survives.
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.trim())
        };

        // `type` is compared raw: it is a closed keyword set, so a keyword
        // never needs escaping and decoding here would only invent a second
        // spelling for each type.
        match attr("type").unwrap_or_default() {
            "skill" => push_decoded(&mut refs.skills, attr(RefKind::Skill.value_attr())),
            "extension" => {
                push_decoded(&mut refs.extensions, attr(RefKind::Extension.value_attr()))
            }
            "knowledge_base" => {
                let id = attr(RefKind::KnowledgeBase.value_attr())
                    .map(decode_ref_value)
                    .unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                refs.knowledge_bases.push(KnowledgeBaseRef {
                    id,
                    label: attr("label")
                        .map(decode_ref_value)
                        .filter(|label| !label.is_empty()),
                });
            }
            // An unknown `type` is ignored rather than guessed at, so a newer
            // composer can add one without this build mis-filing it.
            _ => {}
        }
    }
}

/// Scan `text` for reference tags and return each one's attributes.
///
/// Deliberately permissive about shape — attributes in any order, extra
/// attributes, a valueless attribute, `/>` or `>`, whitespace anywhere — because
/// the alternative is losing a reference the user explicitly attached over a
/// cosmetic difference in how some emitter serialises it. It is strict about
/// exactly two things: the value's quoting (double quotes, entity-escaped) and
/// that the tag is closed.
fn parse_ref_tags(text: &str) -> Vec<TagAttrs> {
    let mut tags = Vec::new();
    let mut idx = 0usize;

    while let Some(offset) = text.get(idx..).and_then(|rest| rest.find('<')) {
        let start = idx + offset;
        // Advance past this `<` before doing anything else. Every failure below
        // resumes from here, which is what stops a malformed tag from either
        // spinning forever or eating the tags that follow it.
        idx = start + '<'.len_utf8();

        let Some(after_name) = text.get(idx..).and_then(|r| r.strip_prefix(REF_TAG_NAME)) else {
            continue;
        };
        // `<biorouter-reference …>` is a different element: the name has to end
        // here, not merely start here.
        if !after_name.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            continue;
        }

        // A tag never spans a raw `<`. A correctly encoded value carries `&lt;`
        // instead, so a raw one means this tag is malformed and another is
        // starting — and bounding the scan is also what keeps a single
        // unterminated `<biorouter-ref` from costing a scan of the whole
        // message.
        let bounded = match after_name.find('<') {
            Some(end) => slice_to(after_name, end),
            None => after_name,
        };

        let Some((attrs, consumed)) = parse_tag_attrs(bounded) else {
            continue;
        };
        tags.push(attrs);
        idx += REF_TAG_NAME.len() + consumed;
    }

    tags
}

/// Parse the attribute list that follows the tag name, up to and including the
/// closing `>`.
///
/// Returns the attributes and how many bytes were consumed, or `None` if the
/// tag never closes — an unterminated tag is dropped rather than honoured,
/// because a truncated message may have been cut mid-attribute and acting on
/// half a tag is how the wrong resource gets loaded.
fn parse_tag_attrs(input: &str) -> Option<(TagAttrs, usize)> {
    let mut attrs: TagAttrs = Vec::new();
    let mut pos = 0usize;

    loop {
        pos += leading_whitespace_len(input.get(pos..)?);
        let rest = input.get(pos..)?;

        if let Some(width) = rest
            .starts_with("/>")
            .then_some(2)
            .or_else(|| rest.starts_with('>').then_some(1))
        {
            return Some((attrs, pos + width));
        }
        if rest.is_empty() {
            return None;
        }

        let name_len = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(rest.len());
        if name_len == 0 {
            // A stray `=` or `/` where an attribute name belongs: not a shape
            // we understand, so leave the whole tag alone.
            return None;
        }
        let name = slice_to(rest, name_len).to_string();
        pos += name_len;

        pos += leading_whitespace_len(input.get(pos..)?);
        let Some(after_eq) = input.get(pos..)?.strip_prefix('=') else {
            // A valueless attribute (`<biorouter-ref … hidden>`). Record it
            // empty and keep going rather than discarding an otherwise good
            // tag over a decoration.
            attrs.push((name, String::new()));
            continue;
        };
        pos += '='.len_utf8();
        pos += leading_whitespace_len(after_eq);

        // Double quotes only. Accepting `'` too would need `'` in the escape
        // table to be safe, and one quoting style is one fewer way for an
        // emitter to drift from this parser.
        let value = input.get(pos..)?.strip_prefix('"')?;
        pos += '"'.len_utf8();
        // Scanning to the next raw `"` is correct *because* of the escaping: an
        // escaped quote is `&quot;`, which contains no quote at all, so the
        // first one found is always the closing one.
        let end = value.find('"')?;
        attrs.push((name, slice_to(value, end).to_string()));
        pos += end + '"'.len_utf8();
    }
}

fn leading_whitespace_len(input: &str) -> usize {
    input.len() - input.trim_start().len()
}

fn push_decoded(out: &mut Vec<String>, raw: Option<&str>) {
    let Some(raw) = raw else { return };
    let value = decode_ref_value(raw);
    if !value.is_empty() {
        out.push(value);
    }
}

fn extract_function_refs(text: &str, prefix: &str, suffix: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(start) = rest.find(prefix) {
        let value_start = start + prefix.len();
        let value_rest = slice_from(rest, value_start);
        let Some(end) = value_rest.find(suffix) else {
            break;
        };
        push_trimmed(out, slice_to(value_rest, end));
        rest = slice_from(value_rest, end + suffix.len());
    }
}

fn extract_function_kb_refs(text: &str, out: &mut Vec<KnowledgeBaseRef>) {
    let mut refs = Vec::new();
    extract_function_refs(text, "/kb(", ")", &mut refs);
    for id in refs {
        out.push(KnowledgeBaseRef { id, label: None });
    }
}

fn extract_legacy_resource_phrases(text: &str, kind: &str, out: &mut Vec<String>) {
    let marker = format!("\" {kind}");
    let mut rest = text;
    while let Some(marker_start) = rest.find(&marker) {
        let before_marker = slice_to(rest, marker_start);
        let Some(open_quote) = before_marker.rfind('"') else {
            rest = slice_from(rest, marker_start + marker.len());
            continue;
        };
        push_trimmed(out, slice_from(before_marker, open_quote + 1));
        rest = slice_from(rest, marker_start + marker.len());
    }
}

fn extract_inline_refs(text: &str, prefix: &str, out: &mut Vec<String>) {
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| c == ',' || c == '.' || c == ';');
        let value = trimmed.strip_prefix(&format!("/{prefix}"));
        if let Some(value) = value {
            push_trimmed(out, value);
        }
    }
}

fn extract_inline_kb_refs(text: &str, out: &mut Vec<KnowledgeBaseRef>) {
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| c == ',' || c == '.' || c == ';');
        let value = trimmed.strip_prefix("/kb:");
        if let Some(value) = value {
            let id = value.trim();
            if !id.is_empty() {
                out.push(KnowledgeBaseRef {
                    id: id.to_string(),
                    label: None,
                });
            }
        }
    }
}

fn extract_legacy_kb_refs(text: &str, out: &mut Vec<KnowledgeBaseRef>) {
    let mut rest = text;
    while let Some(kb_start) = rest.find("kb_id:") {
        let after = slice_from(rest, kb_start + "kb_id:".len()).trim_start();
        let after = after.strip_prefix('"').unwrap_or(after);
        let after = after.strip_prefix('`').unwrap_or(after);
        let id_len = after
            .find(|c: char| c == ')' || c == '"' || c == '`' || c == ',' || c.is_whitespace())
            .unwrap_or(after.len());
        let id = slice_to(after, id_len).trim();
        if !id.is_empty() {
            out.push(KnowledgeBaseRef {
                id: id.to_string(),
                label: None,
            });
        }
        rest = slice_from(after, id_len);
    }

    let mut phrase_rest = text;
    while let Some(start) = phrase_rest.find("focus the \"") {
        let value_start = start + "focus the \"".len();
        let value_rest = slice_from(phrase_rest, value_start);
        let Some(end) = value_rest.find("\" knowledge base") else {
            break;
        };
        let id = slice_to(value_rest, end).trim();
        if !id.is_empty() {
            out.push(KnowledgeBaseRef {
                id: id.to_string(),
                label: None,
            });
        }
        phrase_rest = slice_from(value_rest, end);
    }
}

fn slice_from(value: &str, start: usize) -> &str {
    value.get(start..).unwrap_or("")
}

fn slice_to(value: &str, end: usize) -> &str {
    value.get(..end).unwrap_or(value)
}

fn push_trimmed(out: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        out.push(value.to_string());
    }
}

fn dedup(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn dedup_kbs(values: &mut Vec<KnowledgeBaseRef>) {
    let mut seen = std::collections::HashSet::new();
    values.retain(|value| seen.insert(value.id.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_machine_readable_refs() {
        let refs = extract_resource_refs(
            r#"<biorouter-ref type="skill" name="rna-qc"> <biorouter-ref type="extension" name="developer"> <biorouter-ref type="knowledge_base" id="soul">"#,
        );

        assert_eq!(refs.skills, vec!["rna-qc"]);
        assert_eq!(refs.extensions, vec!["developer"]);
        assert_eq!(
            refs.knowledge_bases,
            vec![KnowledgeBaseRef {
                id: "soul".to_string(),
                label: None
            }]
        );
    }

    #[test]
    fn extracts_legacy_inserted_phrases() {
        let refs = extract_resource_refs(
            r#"Use the "literature-review" skill for this request, Use the "pubmed" extension for this request, Using the Knowledge capability, focus knowledge base "Soul" (kb_id: soul) for this request"#,
        );

        assert_eq!(refs.skills, vec!["literature-review"]);
        assert_eq!(refs.extensions, vec!["pubmed"]);
        assert_eq!(refs.knowledge_bases[0].id, "soul");
    }

    #[test]
    fn extracts_inline_refs() {
        let refs = extract_resource_refs(
            "/skill:rna-qc /ext:developer /kb:soul /skill(rna-qc-v2) /ext(agentdrafter) /kb(project-notes)",
        );

        assert_eq!(refs.skills, vec!["rna-qc", "rna-qc-v2"]);
        assert_eq!(refs.extensions, vec!["developer", "agentdrafter"]);
        assert_eq!(refs.knowledge_bases[0].id, "soul");
        assert_eq!(refs.knowledge_bases[1].id, "project-notes");
    }

    // Alias coverage (`agentdrafter` -> `agent_drafter`, `autovisualizer` ->
    // `autovisualiser`) moved to
    // `extension_manager::tests::resolves_bundled_extension_spelling_aliases`,
    // which owns reference resolution now.

    /// The corpus every escaping test runs over: one entry per character class
    /// that has ever broken an escaping scheme, plus the two literal entity
    /// spellings that catch a wrong `&` ordering.
    ///
    /// Shared verbatim with the renderer's port — see
    /// [`the_shared_corpus_binds_both_implementations`], which pins this list
    /// and this encoder to `resource_ref_corpus.json`.
    const HOSTILE_NAMES: &[&str] = &[
        "rna-qc",
        "my skill",
        r#"say "hi""#,
        "tom & jerry",
        "a &amp; b",
        r#"a &quot; b"#,
        "&",
        "&&&",
        "<script>alert(1)</script>",
        "a < b > c",
        "commas, everywhere,",
        "line one\nline two",
        "carriage\r\nreturn",
        "naïve café — 生物路由器 🧬",
        r#"<biorouter-ref type="skill" name="nested">"#,
        "",
        "   ",
        "&#10;",
        "&#65;",
        "it's\ta — b",
        r#"single-cell "QC" & prep <v2>"#,
    ];

    /// The renderer emits these tags and this parser reads them, so the two
    /// encoders have to produce the same bytes for the same name. They cannot
    /// share code, so they share a corpus: `resource_ref_corpus.json` lists
    /// every hostile name with its exact encoding, and both test suites assert
    /// against it.
    ///
    /// Without this the divergence is invisible — each side keeps passing its
    /// own tests while the tag the composer writes names a resource this parser
    /// never finds.
    #[test]
    fn the_shared_corpus_binds_both_implementations() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("resource_ref_corpus.json"))
                .expect("the shared corpus is valid JSON");
        let pairs = corpus["pairs"]
            .as_array()
            .expect("the shared corpus has a `pairs` array");

        let raw_names: Vec<&str> = pairs
            .iter()
            .map(|pair| pair[0].as_str().expect("every pair starts with a raw name"))
            .collect();
        assert_eq!(
            raw_names, HOSTILE_NAMES,
            "the shared corpus and this crate's corpus have drifted apart"
        );

        for pair in pairs {
            let (raw, encoded) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
            assert_eq!(
                encode_ref_value(raw),
                encoded,
                "this encoder disagrees with the shared corpus on `{raw}`"
            );
            assert_eq!(
                decode_ref_value(encoded),
                raw,
                "this decoder disagrees with the shared corpus on `{encoded}`"
            );
        }
    }

    /// The property the whole scheme rests on.
    #[test]
    fn escaping_round_trips_every_hostile_name() {
        for name in HOSTILE_NAMES {
            assert_eq!(
                decode_ref_value(&encode_ref_value(name)),
                *name,
                "round trip lost `{name}` (encoded as `{}`)",
                encode_ref_value(name)
            );
        }
    }

    /// The ampersand ordering, from both sides. A chained-replacement encoder
    /// that escapes `&` last produces `&amp;lt;` for `<`; a chained decoder
    /// that resolves `&amp;` first turns the encoding of the literal text
    /// `&quot;` into a bare `"`.
    #[test]
    fn the_ampersand_is_escaped_first_and_decoded_last() {
        assert_eq!(encode_ref_value("<"), "&lt;");
        assert_eq!(encode_ref_value("&"), "&amp;");
        assert_eq!(encode_ref_value("&lt;"), "&amp;lt;");
        assert_eq!(encode_ref_value(r#"&quot;"#), "&amp;quot;");

        assert_eq!(decode_ref_value("&amp;lt;"), "&lt;");
        assert_eq!(decode_ref_value("&amp;quot;"), r#"&quot;"#);
        assert_eq!(decode_ref_value("&amp;amp;"), "&amp;");
    }

    /// Position-independent: every occurrence is escaped, not just the first,
    /// and no character outside the table is touched.
    #[test]
    fn escaping_covers_the_whole_table_and_nothing_else() {
        assert_eq!(
            encode_ref_value("a&b\"c<d>e\nf\rg"),
            "a&amp;b&quot;c&lt;d&gt;e&#10;f&#13;g"
        );
        // `'`, tab and non-ASCII are deliberately left alone.
        assert_eq!(encode_ref_value("it's\ta — b"), "it's\ta — b");
    }

    /// An entity we do not emit must survive decoding verbatim rather than
    /// being dropped or guessed at.
    #[test]
    fn an_unknown_or_malformed_entity_is_left_alone() {
        for input in [
            "caf&eacute;",
            "&#65;",
            "&#x41;",
            "&apos;",
            "&nbsp;",
            "AT&T",
            "a & b",
            "&amp",
            "&",
            "&;",
            "&quot",
        ] {
            assert_eq!(decode_ref_value(input), input, "mangled `{input}`");
        }

        // ...and a known entity next to an unknown one still decodes.
        assert_eq!(decode_ref_value("&nbsp;&amp;&nbsp;"), "&nbsp;&&nbsp;");
    }

    #[test]
    fn tag_builders_escape_the_values_they_embed() {
        assert_eq!(
            ref_tag(RefKind::Skill, r#"say "hi" & bye"#),
            r#"<biorouter-ref type="skill" name="say &quot;hi&quot; &amp; bye">"#
        );
        assert_eq!(
            ref_tag(RefKind::Extension, "Chat Recall"),
            r#"<biorouter-ref type="extension" name="Chat Recall">"#
        );
        assert_eq!(
            labelled_ref_tag(RefKind::KnowledgeBase, "soul", "Soul & Body"),
            r#"<biorouter-ref type="knowledge_base" id="soul" label="Soul &amp; Body">"#
        );
    }

    /// Issue #65. A name may contain any character, including the `"` that
    /// delimits the attribute it travels in, so the tag carries values
    /// entity-escaped and the parser has to decode them.
    #[test]
    fn tag_values_are_entity_decoded() {
        let refs = extract_resource_refs(
            r#"<biorouter-ref type="skill" name="say &quot;hi&quot; &amp; bye"> <biorouter-ref type="extension" name="Chat Recall"> <biorouter-ref type="knowledge_base" id="a &lt;b&gt; c">"#,
        );

        assert_eq!(refs.skills, vec![r#"say "hi" & bye"#]);
        assert_eq!(refs.extensions, vec!["Chat Recall"]);
        assert_eq!(refs.knowledge_bases[0].id, "a <b> c");
    }

    /// A tag that never closes its attribute must be abandoned at the next `<`,
    /// not scanned onwards until it finds some unrelated quote — which is how
    /// the old first-quote scan turned one broken tag into a bogus reference
    /// *and* ate the good tag that followed it.
    #[test]
    fn a_malformed_tag_does_not_swallow_the_next_one() {
        let refs = extract_resource_refs(
            r#"<biorouter-ref type="skill" name="broken <biorouter-ref type="skill" name="good">"#,
        );

        assert_eq!(refs.skills, vec!["good"]);
    }

    /// The composer will add presentation-only attributes (a chip label, an
    /// icon) and a serializer is free to reorder them. Neither may cost the
    /// reference.
    #[test]
    fn tag_attributes_may_appear_in_any_order_with_extras() {
        let refs =
            extract_resource_refs(r#"<biorouter-ref name="rna-qc" label="RNA QC" type="skill" />"#);

        assert_eq!(refs.skills, vec!["rna-qc"]);
    }

    /// `KnowledgeBaseRef::label` exists for the chip's display string; the tag
    /// is where it comes from.
    #[test]
    fn knowledge_base_tag_carries_its_label() {
        let refs = extract_resource_refs(
            r#"<biorouter-ref type="knowledge_base" id="soul" label="Soul &amp; Body">"#,
        );

        assert_eq!(
            refs.knowledge_bases,
            vec![KnowledgeBaseRef {
                id: "soul".to_string(),
                label: Some("Soul & Body".to_string()),
            }]
        );
    }

    /// The end-to-end contract: whatever a composer puts into `ref_tag`, the
    /// extractor hands back byte for byte. This is the guarantee the compact
    /// markers cannot make, and the reason the tag exists.
    #[test]
    fn every_hostile_name_survives_a_round_trip_through_a_tag() {
        for name in HOSTILE_NAMES.iter().filter(|n| !n.trim().is_empty()) {
            let message = format!("please use {} on this", ref_tag(RefKind::Skill, name));
            assert_eq!(
                extract_resource_refs(&message).skills,
                vec![name.to_string()],
                "lost `{name}` in `{message}`"
            );

            let message = format!("{} thanks", ref_tag(RefKind::Extension, name));
            assert_eq!(
                extract_resource_refs(&message).extensions,
                vec![name.to_string()],
                "lost `{name}`"
            );

            let message = ref_tag(RefKind::KnowledgeBase, name);
            assert_eq!(
                extract_resource_refs(&message)
                    .knowledge_bases
                    .into_iter()
                    .map(|kb| kb.id)
                    .collect::<Vec<_>>(),
                vec![name.to_string()],
                "lost `{name}`"
            );
        }
    }

    /// Chips sit flush against each other in a composer, so the tags they
    /// serialise to have no separator between them.
    #[test]
    fn adjacent_tags_with_no_separator_are_all_found() {
        let refs = extract_resource_refs(&format!(
            "{}{}{}",
            ref_tag(RefKind::Skill, "a"),
            ref_tag(RefKind::Skill, "b"),
            ref_tag(RefKind::Extension, "c"),
        ));

        assert_eq!(refs.skills, vec!["a", "b"]);
        assert_eq!(refs.extensions, vec!["c"]);
    }

    /// An element whose name merely *starts* with ours is not ours.
    #[test]
    fn a_longer_element_name_is_not_a_reference_tag() {
        let refs = extract_resource_refs(
            r#"<biorouter-reference type="skill" name="nope"> <biorouter-refs type="skill" name="also-nope">"#,
        );

        assert!(refs.is_empty(), "matched a prefix: {refs:?}");
    }

    /// Malformed input must terminate, must not consume the message, and must
    /// not invent a reference. The last case is the adversarial one: a tag with
    /// no `>` at all, followed by text containing quotes.
    #[test]
    fn malformed_tags_are_dropped_without_hanging() {
        for text in [
            r#"<biorouter-ref type="skill" name="oops"#,
            r#"<biorouter-ref type="skill" name="oops">"#.trim_end_matches('>'),
            r#"<biorouter-ref type="skill" name=oops>"#,
            r#"<biorouter-ref type="skill" name='oops'>"#,
            r#"<biorouter-ref type="skill" name="oops but the message rambles on with "quotes" in it"#,
            r#"<biorouter-ref"#,
            "<biorouter-ref ",
            r#"<biorouter-ref = "x">"#,
            "<<<<<biorouter-ref<<<<",
        ] {
            let refs = extract_resource_refs(text);
            assert!(
                refs.skills.is_empty() && refs.extensions.is_empty(),
                "`{text}` produced {refs:?}"
            );
        }
    }

    /// A broken tag must not cost the good ones on either side of it.
    #[test]
    fn a_good_tag_survives_a_broken_neighbour() {
        let refs = extract_resource_refs(&format!(
            r#"{} <biorouter-ref type="skill" name="broken {}"#,
            ref_tag(RefKind::Skill, "before"),
            ref_tag(RefKind::Skill, "after"),
        ));

        assert_eq!(refs.skills, vec!["before", "after"]);
    }

    /// Deliberate: a tag inside a code fence still counts. See `extract_tag_refs`
    /// for why fence-awareness is not worth a Markdown parser here.
    #[test]
    fn a_tag_inside_a_code_fence_is_still_honoured() {
        let refs = extract_resource_refs(&format!(
            "here is the syntax:\n```\n{}\n```\n",
            ref_tag(RefKind::Skill, "rna-qc")
        ));

        assert_eq!(refs.skills, vec!["rna-qc"]);
    }

    /// Shape tolerance a serialiser may impose, none of which may cost the
    /// reference.
    #[test]
    fn tag_shape_variations_are_tolerated() {
        for text in [
            r#"<biorouter-ref type="skill" name="rna-qc">"#,
            r#"<biorouter-ref type="skill" name="rna-qc"/>"#,
            r#"<biorouter-ref type="skill" name="rna-qc" />"#,
            r#"<biorouter-ref    type = "skill"    name = "rna-qc"   >"#,
            r#"<biorouter-ref name="rna-qc" type="skill">"#,
            r#"<biorouter-ref type="skill" name="rna-qc" data-chip>"#,
            r#"<biorouter-ref type="skill" name="rna-qc" label="RNA QC" icon="beaker">"#,
            "<biorouter-ref\n  type=\"skill\"\n  name=\"rna-qc\"\n>",
        ] {
            assert_eq!(
                extract_resource_refs(text).skills,
                vec!["rna-qc"],
                "rejected `{text}`"
            );
        }
    }

    /// A `type` this build does not know is ignored, not guessed at, so a newer
    /// composer can add one without an older backend mis-filing it.
    #[test]
    fn an_unknown_tag_type_is_ignored() {
        let refs = extract_resource_refs(
            r#"<biorouter-ref type="workflow" name="nightly"> <biorouter-ref name="orphan">"#,
        );

        assert!(refs.is_empty(), "{refs:?}");
    }

    /// Issue #65's compatibility half: the compact markers predate the tag and
    /// still appear in persisted sessions, saved workflows and skill documents,
    /// so they must keep resolving — whitespace limitation and all.
    #[test]
    fn compact_markers_still_resolve_alongside_tags() {
        let refs = extract_resource_refs(&format!(
            "/skill:rna-qc /ext:developer /kb:soul /skill(rna-qc-v2) {}",
            ref_tag(RefKind::Skill, "my skill"),
        ));

        // Tags are collected before the compact forms, so they lead — the
        // ordering `explicit_resource_context` renders them in.
        assert_eq!(refs.skills, vec!["my skill", "rna-qc", "rna-qc-v2"]);
        assert_eq!(refs.extensions, vec!["developer"]);
        assert_eq!(refs.knowledge_bases[0].id, "soul");
    }

    /// `reference_marker` is the one rule both front-ends follow. Whatever it
    /// returns must come back out of the extractor as the exact value, for
    /// every name in the hostile corpus — that is the guarantee, and the
    /// compact/tag choice is an implementation detail beneath it.
    #[test]
    fn reference_marker_always_round_trips() {
        for kind in [RefKind::Skill, RefKind::Extension, RefKind::KnowledgeBase] {
            for name in HOSTILE_NAMES.iter().filter(|n| !n.trim().is_empty()) {
                let marker = reference_marker(kind, name);
                assert_eq!(
                    extracted_refs(kind, &marker),
                    vec![name.to_string()],
                    "`{marker}` does not read back as `{name}`"
                );
            }
        }
    }

    /// ...and it reaches for the tag only when it has to, so a terminal keeps
    /// showing the readable form for the names that can carry it.
    #[test]
    fn reference_marker_prefers_the_compact_form_when_it_survives() {
        for (kind, name, expected) in [
            (RefKind::Skill, "rna-qc", "/skill:rna-qc"),
            (RefKind::Skill, "my_skill-v2", "/skill:my_skill-v2"),
            (RefKind::Extension, "developer", "/ext:developer"),
            (RefKind::KnowledgeBase, "soul", "/kb:soul"),
        ] {
            assert_eq!(reference_marker(kind, name), expected);
            assert!(inline_marker_round_trips(kind, name));
        }

        for (kind, name) in [
            (RefKind::Skill, "my skill"),
            (RefKind::Extension, "Chat Recall"),
            (RefKind::KnowledgeBase, "two words"),
            // Trailing `.` `,` `;` are stripped by the compact extractor, so a
            // name that ends in one cannot use it either.
            (RefKind::Skill, "step 1."),
            (RefKind::Skill, "v1,"),
        ] {
            assert!(
                !inline_marker_round_trips(kind, name),
                "`{name}` was thought safe for the compact marker"
            );
            assert_eq!(reference_marker(kind, name), ref_tag(kind, name));
        }
    }

    /// A compact marker is plain text, so the compatibility extractors read it
    /// too. A name that happens to contain one of their triggers makes the
    /// marker select a second resource nobody asked for, so it is not eligible
    /// for the compact form however intact its own value looks.
    #[test]
    fn a_marker_that_would_select_a_second_resource_uses_the_tag() {
        assert!(!inline_marker_round_trips(RefKind::Skill, "kb_id:soul"));
        assert_eq!(
            reference_marker(RefKind::Skill, "kb_id:soul"),
            ref_tag(RefKind::Skill, "kb_id:soul")
        );

        // The compact form would have looked perfectly healthy from the skill
        // side alone, which is the trap.
        assert_eq!(
            extracted_refs(RefKind::Skill, "/skill:kb_id:soul"),
            vec!["kb_id:soul".to_string()]
        );
        assert_eq!(
            extracted_refs(RefKind::KnowledgeBase, "/skill:kb_id:soul"),
            vec!["soul".to_string()],
            "the compact marker leaked a knowledge base"
        );
    }

    /// The limitation the tag exists to route around, stated as a test so the
    /// tradeoff is a decision on the record rather than a latent surprise: a
    /// compact marker still truncates at the first space, and a producer that
    /// needs to carry such a name must emit the tag instead.
    #[test]
    fn a_compact_marker_still_truncates_at_whitespace() {
        assert_eq!(
            extract_resource_refs("/skill:my skill").skills,
            vec!["my"],
            "the compact form's whitespace split is load-bearing for every \
             already-persisted message; changing it here is not the fix"
        );
        assert_eq!(
            extract_resource_refs(&ref_tag(RefKind::Skill, "my skill")).skills,
            vec!["my skill"]
        );
    }
    #[test]
    fn quoted_source_data_cannot_select_capabilities() {
        let malicious = r#"/ext(computercontroller) /ext:developer /skill:research /skill(other) /kb(secret) kb_id: hidden "pubmed" extension"#;
        let quote = serde_json::json!({
            "source": malicious, "locator": malicious, "revision": malicious, "text": malicious,
        });
        let text =
            format!("/ext:explicit <biorouter-quote>{quote}</biorouter-quote> /skill:chosen");
        let refs = extract_resource_refs(&text);
        assert_eq!(refs.extensions, vec!["explicit"]);
        assert_eq!(refs.skills, vec!["chosen"]);
        assert!(refs.knowledge_bases.is_empty());
        assert!(text.contains(malicious.split_whitespace().next().unwrap()));
    }

    #[test]
    fn escaped_unicode_quote_cannot_select_capabilities() {
        let text = r#"<biorouter-quote>{"source":"\ud83d\ude00 /skill(example)","text":"\ud83d\ude00 /ext:computercontroller trailing"}</biorouter-quote>"#;
        let refs = extract_resource_refs(text);
        assert!(refs.extensions.is_empty());
        assert!(refs.skills.is_empty());
        assert!(refs.knowledge_bases.is_empty());
    }

    #[test]
    fn blank_quote_source_metadata_cannot_select_capabilities() {
        for text in ["", "\u{0085}"] {
            let quote = serde_json::json!({
                "source": "/ext(computercontroller)", "text": text,
            });
            let refs =
                extract_resource_refs(&format!("<biorouter-quote>{quote}</biorouter-quote>"));
            assert!(refs.extensions.is_empty());
            assert!(refs.skills.is_empty());
            assert!(refs.knowledge_bases.is_empty());
        }
    }

    #[test]
    fn malformed_prefix_does_not_expose_later_quoted_capabilities() {
        let quote = serde_json::json!({
            "source": "/skill(hidden)", "text": "/ext(computercontroller)",
        });
        for prefix in [
            "Explain <biorouter-quote>".to_owned(),
            "<biorouter-quote>{invalid".to_owned(),
            "<biorouter-quote>".repeat(1_000),
        ] {
            let refs = extract_resource_refs(&format!(
                "/ext:explicit {prefix} <biorouter-quote>{quote}</biorouter-quote> /skill:chosen"
            ));
            assert_eq!(refs.extensions, vec!["explicit"]);
            assert_eq!(refs.skills, vec!["chosen"]);
            assert!(refs.knowledge_bases.is_empty());
        }
    }

    #[test]
    fn malformed_quote_does_not_hide_resource_selections() {
        let refs =
            extract_resource_refs("<biorouter-quote>invalid /ext:developer </biorouter-quote>");
        assert_eq!(refs.extensions, vec!["developer"]);
    }
}
