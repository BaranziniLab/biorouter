//! The Unicode `Default_Ignorable_Code_Point` property, as a generated table.
//!
//! Default-ignorable code points render as nothing at all: zero-width spaces and joiners,
//! bidirectional controls, variation selectors, Hangul fillers, tag characters. A name that
//! contains one can look identical to a name that does not, so the naming rules refuse them in
//! new names and strip them before computing a name key (`names.rs`).
//!
//! **Generated, do not edit by hand.** Source: `DerivedCoreProperties.txt` from the Unicode
//! Character Database, version 17.0.0 (the version `unicode-normalization` and
//! `unicode-properties` carry in `Cargo.lock`), downloaded from
//! <https://www.unicode.org/Public/17.0.0/ucd/DerivedCoreProperties.txt>,
//! SHA-256 `24c7fed1195c482faaefd5c1e7eb821c5ee1fb6de07ecdbaa64b56a99da22c08`.
//! The file lists 27 `Default_Ignorable_Code_Point` entries (4,174 code points); adjacent
//! entries are merged into the 17 ranges below. To regenerate for a new Unicode version,
//! download that version's file, update the URL, hash and version constant, and replace
//! the table with the output of:
//!
//! ```text
//! python3 - <<'EOF'
//! ranges = []
//! for line in open('DerivedCoreProperties.txt'):
//!     fields = [x.strip() for x in line.split('#')[0].split(';')]
//!     if len(fields) != 2 or fields[1] != 'Default_Ignorable_Code_Point':
//!         continue
//!     a, _, b = fields[0].partition('..')
//!     ranges.append((int(a, 16), int(b or a, 16)))
//! merged = []
//! for a, b in sorted(ranges):
//!     if merged and a <= merged[-1][1] + 1:
//!         merged[-1] = (merged[-1][0], max(merged[-1][1], b))
//!     else:
//!         merged.append((a, b))
//! for a, b in merged:
//!     print(f"    ('\\u{{{a:X}}}', '\\u{{{b:X}}}'),")
//! EOF
//! ```

/// The Unicode version the table below was generated from.
pub const DEFAULT_IGNORABLE_UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0);

/// Inclusive, sorted, non-overlapping, non-adjacent ranges of `Default_Ignorable_Code_Point`.
const DEFAULT_IGNORABLE: &[(char, char)] = &[
    ('\u{AD}', '\u{AD}'),
    ('\u{34F}', '\u{34F}'),
    ('\u{61C}', '\u{61C}'),
    ('\u{115F}', '\u{1160}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{180B}', '\u{180F}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{206F}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFA0}', '\u{FFA0}'),
    ('\u{FFF0}', '\u{FFF8}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0000}', '\u{E0FFF}'),
];

/// Whether `c` has the Unicode `Default_Ignorable_Code_Point` property.
pub fn is_default_ignorable(c: char) -> bool {
    DEFAULT_IGNORABLE
        .binary_search_by(|&(low, high)| {
            if high < c {
                std::cmp::Ordering::Less
            } else if low > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_disjoint_and_merged() {
        for pair in DEFAULT_IGNORABLE.windows(2) {
            let (_, previous_high) = pair[0];
            let (next_low, _) = pair[1];
            assert!(
                (previous_high as u32) + 1 < next_low as u32,
                "{pair:?} overlaps or should have been merged"
            );
        }
        for &(low, high) in DEFAULT_IGNORABLE {
            assert!(low <= high);
        }
        let total: u32 = DEFAULT_IGNORABLE
            .iter()
            .map(|&(low, high)| high as u32 - low as u32 + 1)
            .sum();
        assert_eq!(
            total, 4_174,
            "the Unicode 17.0.0 property has 4,174 code points"
        );
    }

    #[test]
    fn named_members_and_neighbors() {
        for c in [
            '\u{AD}',
            '\u{34F}',
            '\u{61C}',
            '\u{115F}',
            '\u{1160}',
            '\u{180E}',
            '\u{200B}',
            '\u{200C}',
            '\u{200D}',
            '\u{200E}',
            '\u{200F}',
            '\u{202A}',
            '\u{202E}',
            '\u{2060}',
            '\u{2065}',
            '\u{2066}',
            '\u{2069}',
            '\u{206F}',
            '\u{3164}',
            '\u{FE00}',
            '\u{FE0F}',
            '\u{FEFF}',
            '\u{FFA0}',
            '\u{1D173}',
            '\u{E0001}',
            '\u{E007F}',
            '\u{E0100}',
            '\u{E01EF}',
            '\u{E0FFF}',
        ] {
            assert!(is_default_ignorable(c), "U+{:04X}", c as u32);
        }
        for c in [
            'a',
            ' ',
            '\u{AC}',
            '\u{AE}',
            '\u{200A}',
            '\u{2010}',
            '\u{2029}',
            '\u{2070}',
            '\u{3163}',
            '\u{3165}',
            '\u{FDFF}',
            '\u{FE10}',
            '\u{FFF9}',
            '\u{E1000}',
            '李',
        ] {
            assert!(!is_default_ignorable(c), "U+{:04X}", c as u32);
        }
    }
}
