//! Human-readable names: normalization, keys and the per-kind validators.
//!
//! One definition shared by the broker, which enforces these rules on create and rename, and
//! the desktop daemon, which resolves names against a snapshot and previews them. See
//! `docs/research/biorouter-crew/naming-design.md` ("Display-name validation", "Normalization
//! and keys", "Validation per kind" and "Confusable characters").
//!
//! - [`clean`] is the stored form of a display name or team name.
//! - [`name_key`] and [`skeleton_key`] decide when two names are "the same name"
//!   ([`names_collide`]: either key equal). Keys are computed, never stored, so a Unicode data
//!   update never rewrites a journal.
//! - The validators apply only to new creates and renames. Legacy names stay readable; the
//!   `sanitize_*` functions turn them into something safe to display.
//!
//! Names confer no authority. Every ACL, membership and owner field stays an ID; a name is for
//! display and lookup only.

use std::fmt;

use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};
use unicode_security::{GeneralSecurityProfile, RestrictionLevel, RestrictionLevelDetection};

pub use crate::default_ignorable::{is_default_ignorable, DEFAULT_IGNORABLE_UNICODE_VERSION};

/// Display names and team names: at most this many Unicode scalar values after [`clean`].
pub const DISPLAY_NAME_MAX_CHARS: usize = 64;
/// Display names and team names: at most this many UTF-8 bytes after [`clean`].
pub const DISPLAY_NAME_MAX_BYTES: usize = 120;
/// Team names: at most this many Unicode scalar values after [`clean`].
pub const TEAM_NAME_MAX_CHARS: usize = 64;
/// Team names: at most this many UTF-8 bytes after [`clean`].
pub const TEAM_NAME_MAX_BYTES: usize = 120;
/// Channel names: at most this many Unicode scalar values after canonicalization.
pub const CHANNEL_NAME_MAX_CHARS: usize = 80;
/// Channel names: at most this many UTF-8 bytes after canonicalization.
pub const CHANNEL_NAME_MAX_BYTES: usize = 120;
/// Workspace names: at most this many ASCII characters.
pub const WORKSPACE_NAME_MAX_CHARS: usize = 40;
/// Usernames typed for an invitation: at most this many bytes.
pub const USERNAME_MAX_BYTES: usize = 256;
/// The channel every team is created with. Reserved for that channel.
pub const RESERVED_CHANNEL_NAME: &str = "general";
/// What a legacy team whose name sanitizes to nothing is displayed as.
pub const UNTITLED_TEAM: &str = "Untitled team";
/// What a legacy channel whose name sanitizes to nothing is displayed as.
pub const UNTITLED_CHANNEL: &str = "untitled";

/// The ASCII punctuation a team name may contain besides letters, marks and numbers.
const TEAM_PUNCTUATION: [char; 9] = [' ', '-', '_', '.', '\'', '&', '(', ')', '+'];
/// Characters reserved for selectors (`@person`, `#channel`, `team/channel`, `host:port`),
/// refused in team and channel names after NFKC.
const SELECTOR_CHARACTERS: [char; 4] = ['@', '#', '/', ':'];
/// The selector characters that mark a person or a channel, refused after NFKC **and** after the
/// confusable skeleton, so fullwidth `＠` and small `﹫` cannot pose as a mention. `/` and `:`
/// are not checked by skeleton: common letters are their confusables (KATAKANA LETTER NO `ノ` is
/// `/`, DEVANAGARI SIGN VISARGA `ः` is `:`), and a lookalike cannot change how a selector
/// parses, which is all those two are reserved for.
const MENTION_CHARACTERS: [char; 2] = ['@', '#'];

/// Which kind of name a [`NameError`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    DisplayName,
    Team,
    Channel,
    Workspace,
}

impl NameKind {
    fn label(self) -> &'static str {
        match self {
            Self::DisplayName => "Display name",
            Self::Team => "Team name",
            Self::Channel => "Channel name",
            Self::Workspace => "Workspace name",
        }
    }
}

/// Why a name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameProblem {
    /// Nothing is left after cleaning.
    Empty,
    /// Over the kind's scalar-value or byte limit.
    TooLong,
    /// No visible letter or number.
    NoLetterOrDigit,
    /// A control, format, private-use, unassigned or default-ignorable character.
    InvisibleCharacter,
    /// A selector character (`@`, `#`, and for teams and channels `/` and `:`), including a
    /// lookalike of one such as fullwidth `＠`.
    ReservedCharacter,
    /// A character outside the kind's allowed set, or one Unicode does not allow in
    /// identifiers (UTS #39 `Identifier_Status`).
    DisallowedCharacter,
    /// A generic combining mark that did not compose with the character before it.
    UnattachedMark,
    /// A channel name that starts with something other than a letter or number.
    MustStartWithLetterOrDigit,
    /// Scripts mixed beyond UTS #39 Highly Restrictive (`Аnalysis` with a Cyrillic `А`).
    MixedScripts,
    /// The name parses as a UUID or 64 hexadecimal characters.
    LooksLikeId,
    /// A display name equal (or confusable) to another person's username.
    ClaimsUsername,
}

/// A refused name. [`fmt::Display`] is a plain sentence fit to show a person; [`Self::wire`]
/// adds the broker's `code: ` prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameError {
    pub kind: NameKind,
    pub problem: NameProblem,
}

impl NameError {
    fn new(kind: NameKind, problem: NameProblem) -> Self {
        Self { kind, problem }
    }

    /// The machine code for every refused name.
    pub fn code(&self) -> &'static str {
        "name_invalid"
    }

    /// `name_invalid: <sentence>`, the broker's error convention.
    pub fn wire(&self) -> String {
        format!("{}: {}", self.code(), self)
    }

    /// The sentence shown to a person.
    pub fn message(&self) -> String {
        let label = self.kind.label();
        match (self.kind, self.problem) {
            (_, NameProblem::Empty) => format!("{label} can't be empty."),
            (_, NameProblem::TooLong) => format!("{label} is too long. Choose a shorter name."),
            (_, NameProblem::NoLetterOrDigit) => {
                format!("{label} needs at least one letter or number.")
            }
            (_, NameProblem::InvisibleCharacter) => {
                format!("{label} can't contain invisible, control or formatting characters.")
            }
            (NameKind::DisplayName, NameProblem::ReservedCharacter) => {
                "Display name can't contain @ or #.".to_string()
            }
            (_, NameProblem::ReservedCharacter) => {
                format!("{label} can't contain @, #, / or :.")
            }
            (NameKind::Team, NameProblem::DisallowedCharacter) => {
                "Team name can use letters, numbers, spaces and - _ . ' & ( ) + only.".to_string()
            }
            (NameKind::Channel, NameProblem::DisallowedCharacter) => {
                "Channel name can use lowercase letters, numbers, hyphens and underscores only."
                    .to_string()
            }
            (NameKind::Workspace, NameProblem::DisallowedCharacter) => {
                "Workspace name can use lowercase letters a-z, numbers and hyphens, and must \
                 start and end with a letter or number."
                    .to_string()
            }
            (_, NameProblem::DisallowedCharacter) => {
                format!("{label} contains a character that isn't allowed.")
            }
            (_, NameProblem::UnattachedMark) => {
                format!("{label} can't contain a separate accent or combining mark.")
            }
            (_, NameProblem::MustStartWithLetterOrDigit) => {
                format!("{label} must start with a letter or number.")
            }
            (_, NameProblem::MixedScripts) => format!(
                "{label} can't mix writing systems that look alike, such as Latin and Cyrillic \
                 letters."
            ),
            (_, NameProblem::LooksLikeId) => format!("{label} can't look like an ID."),
            (_, NameProblem::ClaimsUsername) => {
                "That name is another member's username. Choose a different name.".to_string()
            }
        }
    }
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for NameError {}

/// `NFC → trim → collapse runs of White_Space to one U+0020`: the stored form of a display name
/// or team name.
pub fn clean(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    let mut out = String::with_capacity(nfc.len());
    let mut pending_space = false;
    for c in nfc.trim().chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    out
}

/// Remove every `Default_Ignorable_Code_Point` character.
pub fn strip_ignorable(s: &str) -> String {
    s.chars().filter(|c| !is_default_ignorable(*c)).collect()
}

/// The key two names are compared by:
/// `strip_ignorable(NFKC(s)) → to_lowercase → NFKC → separators to '-' → collapse → trim '-'`.
///
/// Separators are `' '`, `'-'`, `'_'`, `'.'` and, as a superset of the design's list, every
/// other `White_Space` character that survives NFKC (a tab in a legacy name). `to_lowercase` is
/// Rust's default mapping, not full case folding, so `ß` and `ss` stay distinct.
pub fn name_key(s: &str) -> String {
    let compatible: String = s.nfkc().collect();
    let lowered: String = strip_ignorable(&compatible).to_lowercase().nfkc().collect();
    join_separated(&strip_ignorable(&lowered), is_key_separator)
}

/// [`name_key`] of the UTS #39 confusable skeleton, so `anaIysis` (capital I) and `analysis`,
/// or `rn` and `m`, share a key.
///
/// The skeleton is taken before lowercasing, as the design specifies, because the confusable is
/// capital `I` against lowercase `l`. The price is that this key alone is not case-insensitive
/// for a capital `I`: `ANALYSIS` and `analysis` share a [`name_key`] but not a skeleton key.
/// No single key can capture both relations (it would have to merge `i` and `l` everywhere), so
/// "the same name" is [`names_collide`]: either key equal.
pub fn skeleton_key(s: &str) -> String {
    let compatible: String = s.nfkc().collect();
    let skeleton: String = unicode_security::skeleton(&strip_ignorable(&compatible)).collect();
    name_key(&skeleton)
}

/// Whether two names count as the same name: equal [`name_key`] (case, width, separators and
/// invisible characters ignored) or equal [`skeleton_key`] (lookalike characters). Uniqueness
/// checks and the display-name collision directory use this rather than either key alone.
pub fn names_collide(a: &str, b: &str) -> bool {
    name_key(a) == name_key(b) || skeleton_key(a) == skeleton_key(b)
}

/// Text that is **always** an ID and never a name: anything that parses as a UUID (any of its
/// textual forms) or is exactly 64 hexadecimal characters.
pub fn is_uuid_shaped(s: &str) -> bool {
    uuid::Uuid::try_parse(s).is_ok() || (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether the letters, marks and digits of `s` are at most UTS #39 **Highly Restrictive**:
/// one script plus Common and Inherited, or Latin with Han and Hiragana and Katakana, Latin with
/// Han and Bopomofo, or Latin with Han and Hangul. ASCII punctuation and spaces are not scored;
/// the per-kind character rules decide those.
pub fn restriction_level_ok(s: &str) -> bool {
    let scored: String = s
        .chars()
        .filter(|c| !c.is_ascii() || c.is_ascii_alphanumeric())
        .collect();
    scored
        .as_str()
        .check_restriction_level(RestrictionLevel::HighlyRestrictive)
}

/// Whether `name` is shaped like a Unix account name a person can type after `@`: 1-256 bytes;
/// no NUL, control character, whitespace, `/` or `:`; and not all digits (glibc's
/// `getpwnam("1001")` looks up a name while `getent passwd 1001` looks up a UID). An `@` inside
/// the name (an SSSD name such as `bob@ad.ucsf.edu`) is allowed; strip a leading selector `@`
/// before calling this.
pub fn valid_username(name: &str) -> bool {
    (1..=USERNAME_MAX_BYTES).contains(&name.len())
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '/' || c == ':')
        && !name.bytes().all(|b| b.is_ascii_digit())
}

/// The context-free display-name rules. Returns the cleaned name to store.
///
/// After [`clean`]: 1-64 scalar values and at most 120 bytes; no character of category Cc, Cf,
/// Co, Cs, Cn, Zl or Zp and no default-ignorable character; no `@` or `#` after NFKC or after
/// the confusable skeleton (so fullwidth `＠` and small `﹫` are refused); at least one visible
/// letter or number. Mixed scripts are allowed: real names mix scripts (`李明 Li Ming`) and
/// display names confer no authority.
pub fn validate_display_name(raw: &str) -> Result<String, NameError> {
    let kind = NameKind::DisplayName;
    let name = clean(raw);
    if name.is_empty() {
        return Err(NameError::new(kind, NameProblem::Empty));
    }
    if name.chars().any(is_invisible) {
        return Err(NameError::new(kind, NameProblem::InvisibleCharacter));
    }
    if name.chars().count() > DISPLAY_NAME_MAX_CHARS || name.len() > DISPLAY_NAME_MAX_BYTES {
        return Err(NameError::new(kind, NameProblem::TooLong));
    }
    if contains_reserved(&name, &MENTION_CHARACTERS, &MENTION_CHARACTERS) {
        return Err(NameError::new(kind, NameProblem::ReservedCharacter));
    }
    if !name.chars().any(is_visible_base) {
        return Err(NameError::new(kind, NameProblem::NoLetterOrDigit));
    }
    Ok(name)
}

/// Whether [`validate_display_name`] accepts `raw`.
pub fn display_name_valid(raw: &str) -> bool {
    validate_display_name(raw).is_ok()
}

/// Whether a display name reads as **another** person's username: its [`name_key`] or
/// [`skeleton_key`] equals that of a username in `other_usernames` (active or former
/// principals, and the host). A person's own username is always allowed, since it is the
/// default display name.
pub fn display_name_claims_username<'a, I>(
    name: &str,
    own_username: &str,
    other_usernames: I,
) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    let key = name_key(name);
    if key == name_key(own_username) {
        return false;
    }
    let skeleton = skeleton_key(name);
    other_usernames
        .into_iter()
        .filter(|other| *other != own_username)
        .any(|other| name_key(other) == key || skeleton_key(other) == skeleton)
}

/// [`validate_display_name`] plus the rule that a display name may not be another person's
/// username. Used by `profile.update` and for the `profile.suggest` output.
pub fn validate_display_name_for<'a, I>(
    raw: &str,
    own_username: &str,
    other_usernames: I,
) -> Result<String, NameError>
where
    I: IntoIterator<Item = &'a str>,
{
    let name = validate_display_name(raw)?;
    if display_name_claims_username(&name, own_username, other_usernames) {
        return Err(NameError::new(
            NameKind::DisplayName,
            NameProblem::ClaimsUsername,
        ));
    }
    Ok(name)
}

/// The projection sanitizer for a stored (possibly legacy) nickname: strips the characters the
/// validator refuses, then falls back to the username when the result is empty, too long, has
/// no letter or number, or reads as another person's username. Never fails.
pub fn sanitize_display_name<'a, I>(nickname: &str, username: &str, other_usernames: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let stripped: String = nickname
        .chars()
        .filter(|c| {
            !is_invisible(*c)
                && !contains_reserved(&c.to_string(), &MENTION_CHARACTERS, &MENTION_CHARACTERS)
        })
        .collect();
    match validate_display_name_for(&stripped, username, other_usernames) {
        Ok(name) => name,
        Err(_) => username.to_string(),
    }
}

/// The team-name rules for create and rename. Returns the cleaned name to store.
///
/// After [`clean`]: 1-64 scalar values and at most 120 bytes; at least one letter or number;
/// only letters, marks and numbers that Unicode allows in identifiers (UTS #39
/// `Identifier_Status=Allowed`), the space and `- _ . ' & ( ) +`; no default-ignorable
/// character; no generic combining mark left uncomposed by NFC; no `@`, `#`, `/` or `:`; a
/// [`name_key`] that is not UUID-shaped; and at most Highly Restrictive script mixing.
pub fn validate_team_name(raw: &str) -> Result<String, NameError> {
    let kind = NameKind::Team;
    let name = clean(raw);
    if name.is_empty() {
        return Err(NameError::new(kind, NameProblem::Empty));
    }
    if name.chars().any(is_invisible) {
        return Err(NameError::new(kind, NameProblem::InvisibleCharacter));
    }
    if name.chars().count() > TEAM_NAME_MAX_CHARS || name.len() > TEAM_NAME_MAX_BYTES {
        return Err(NameError::new(kind, NameProblem::TooLong));
    }
    if contains_reserved(&name, &SELECTOR_CHARACTERS, &MENTION_CHARACTERS) {
        return Err(NameError::new(kind, NameProblem::ReservedCharacter));
    }
    if name.chars().any(is_unattached_mark) {
        return Err(NameError::new(kind, NameProblem::UnattachedMark));
    }
    if name.chars().any(|c| !team_character_allowed(c)) {
        return Err(NameError::new(kind, NameProblem::DisallowedCharacter));
    }
    let key = name_key(&name);
    if key.is_empty() || !name.chars().any(is_visible_base) {
        return Err(NameError::new(kind, NameProblem::NoLetterOrDigit));
    }
    if is_uuid_shaped(&key) || is_uuid_shaped(&name) {
        return Err(NameError::new(kind, NameProblem::LooksLikeId));
    }
    if !restriction_level_ok(&name) {
        return Err(NameError::new(kind, NameProblem::MixedScripts));
    }
    Ok(name)
}

/// A team's handle: its [`name_key`] (`Analysis Lab` → `analysis-lab`).
pub fn team_handle(name: &str) -> String {
    name_key(name)
}

/// The display form of a stored (possibly legacy) team name: invisible characters removed and
/// [`clean`]ed, or [`UNTITLED_TEAM`] when nothing visible is left.
pub fn sanitize_team_name(name: &str) -> String {
    let visible = clean(
        &name
            .chars()
            .filter(|c| !is_invisible(*c))
            .collect::<String>(),
    );
    if visible.chars().any(is_visible_base) {
        visible
    } else {
        UNTITLED_TEAM.to_string()
    }
}

/// Canonicalize a channel name for create and rename, then validate it. Returns the slug to
/// store (`Data Analysis` → `data-analysis`).
///
/// Canonicalization: NFKC, trim, drop one leading `#` (so a person can type `#methods`),
/// lowercase, NFKC, whitespace and `.` to `-`, collapse `-` runs, trim `-`. The result must
/// then be 1-80 scalar values and at most 120 bytes; every character a lowercase-stable letter,
/// a script-specific combining mark, a decimal digit, `-` or `_`; start with a letter or digit;
/// every character `Identifier_Status=Allowed`; contain no default-ignorable character; not be
/// UUID-shaped; and be at most Highly Restrictive. Canonicalizing instead of refusing keeps
/// older clients that send `"Data Analysis"` working and lets a client preview the exact slug.
/// [`RESERVED_CHANNEL_NAME`] is valid here; reserving it is the broker's rule.
pub fn canonical_channel_name(raw: &str) -> Result<String, NameError> {
    let kind = NameKind::Channel;
    let compatible: String = raw.nfkc().collect();
    let trimmed = compatible.trim();
    let trimmed = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let lowered: String = trimmed.to_lowercase().nfkc().collect();
    let slug = join_separated(&lowered, |c| c.is_whitespace() || c == '.' || c == '-');
    if slug.is_empty() {
        return Err(NameError::new(kind, NameProblem::Empty));
    }
    if slug.chars().any(is_invisible) {
        return Err(NameError::new(kind, NameProblem::InvisibleCharacter));
    }
    if slug.chars().count() > CHANNEL_NAME_MAX_CHARS || slug.len() > CHANNEL_NAME_MAX_BYTES {
        return Err(NameError::new(kind, NameProblem::TooLong));
    }
    if contains_reserved(&slug, &SELECTOR_CHARACTERS, &MENTION_CHARACTERS) {
        return Err(NameError::new(kind, NameProblem::ReservedCharacter));
    }
    if slug.chars().any(is_unattached_mark) {
        return Err(NameError::new(kind, NameProblem::UnattachedMark));
    }
    if slug.chars().any(|c| !channel_character_allowed(c)) {
        return Err(NameError::new(kind, NameProblem::DisallowedCharacter));
    }
    if !slug.chars().next().is_some_and(|c| {
        c.general_category_group() == GeneralCategoryGroup::Letter
            || c.general_category() == GeneralCategory::DecimalNumber
    }) {
        return Err(NameError::new(
            kind,
            NameProblem::MustStartWithLetterOrDigit,
        ));
    }
    if is_uuid_shaped(&slug) || is_uuid_shaped(&name_key(&slug)) {
        return Err(NameError::new(kind, NameProblem::LooksLikeId));
    }
    if !restriction_level_ok(&slug) {
        return Err(NameError::new(kind, NameProblem::MixedScripts));
    }
    Ok(slug)
}

/// The display form of a stored (possibly legacy) channel name: invisible characters removed
/// and whitespace collapsed, or [`UNTITLED_CHANNEL`] when nothing visible is left.
pub fn sanitize_channel_name(name: &str) -> String {
    let visible = clean(
        &name
            .chars()
            .filter(|c| !is_invisible(*c))
            .collect::<String>(),
    );
    if visible.chars().any(is_visible_base) {
        visible
    } else {
        UNTITLED_CHANNEL.to_string()
    }
}

/// The workspace-name rule: 1-40 lowercase ASCII letters, digits and hyphens, starting and
/// ending with a letter or digit (`^[a-z0-9](?:[a-z0-9-]{0,38}[a-z0-9])?$`), and not
/// UUID-shaped. A workspace name travels in remote SSH commands, so it is plain ASCII and is
/// never normalized: the caller passes exactly what will be stored.
pub fn validate_workspace_name(name: &str) -> Result<(), NameError> {
    let kind = NameKind::Workspace;
    if name.is_empty() {
        return Err(NameError::new(kind, NameProblem::Empty));
    }
    if name.len() > WORKSPACE_NAME_MAX_CHARS {
        return Err(NameError::new(kind, NameProblem::TooLong));
    }
    let edge = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = name.as_bytes();
    if !bytes.iter().all(|&b| edge(b) || b == b'-')
        || !bytes.first().copied().is_some_and(edge)
        || !bytes.last().copied().is_some_and(edge)
    {
        return Err(NameError::new(kind, NameProblem::DisallowedCharacter));
    }
    if is_uuid_shaped(name) {
        return Err(NameError::new(kind, NameProblem::LooksLikeId));
    }
    Ok(())
}

/// Whether [`validate_workspace_name`] accepts `name`.
pub fn workspace_name_valid(name: &str) -> bool {
    validate_workspace_name(name).is_ok()
}

/// A character that renders as nothing or as a control: general category Cc, Cf, Co, Cs, Cn,
/// Zl or Zp, or `Default_Ignorable_Code_Point`.
fn is_invisible(c: char) -> bool {
    matches!(
        c.general_category(),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::PrivateUse
            | GeneralCategory::Surrogate
            | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
    ) || is_default_ignorable(c)
}

/// A visible base character: category L or N and not default-ignorable (U+3164 HANGUL FILLER
/// is a letter that renders as nothing).
fn is_visible_base(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
    ) && !is_default_ignorable(c)
}

/// A generic combining mark (`Script=Inherited`, such as U+0301 or U+0336) that is still
/// present after NFC, so it did not compose with the character before it: `q\u{301}` or a
/// stack of overlays. Marks that belong to one script (Devanagari vowel signs, Thai tone marks)
/// are how that script is written and are not refused here. Arabic harakat are
/// `Script=Inherited`, so a vowelled Arabic team or channel name is refused; names are
/// normally written without them.
fn is_unattached_mark(c: char) -> bool {
    c.general_category_group() == GeneralCategoryGroup::Mark && c.script() == Script::Inherited
}

/// Whether `s` contains one of `after_nfkc` after NFKC, or one of `after_skeleton` in its
/// confusable skeleton.
fn contains_reserved(s: &str, after_nfkc: &[char], after_skeleton: &[char]) -> bool {
    let compatible: String = s.nfkc().collect();
    compatible.contains(after_nfkc)
        || unicode_security::skeleton(&compatible).any(|c| after_skeleton.contains(&c))
}

fn team_character_allowed(c: char) -> bool {
    TEAM_PUNCTUATION.contains(&c)
        || (matches!(
            c.general_category_group(),
            GeneralCategoryGroup::Letter
                | GeneralCategoryGroup::Mark
                | GeneralCategoryGroup::Number
        ) && c.identifier_allowed())
}

fn channel_character_allowed(c: char) -> bool {
    if c == '-' || c == '_' {
        return true;
    }
    let allowed_shape = match c.general_category_group() {
        GeneralCategoryGroup::Letter => c.to_lowercase().eq(std::iter::once(c)),
        GeneralCategoryGroup::Mark => c.script() != Script::Inherited,
        _ => c.general_category() == GeneralCategory::DecimalNumber,
    };
    allowed_shape && c.identifier_allowed()
}

fn is_key_separator(c: char) -> bool {
    matches!(c, ' ' | '-' | '_' | '.') || c.is_whitespace()
}

/// Replace every run of `separator` characters with one `-`, dropping leading and trailing
/// runs.
fn join_separated(s: &str, separator: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending = false;
    for c in s.chars() {
        if separator(c) {
            pending = true;
            continue;
        }
        if pending && !out.is_empty() {
            out.push('-');
        }
        pending = false;
        out.push(c);
    }
    out
}
