//! What a search walks and what a full history drops, as pure functions over the parsed
//! entries, so the store's file handling and these rules are tested apart.

use chrono::{DateTime, Utc};

use super::schema::{
    entry_line, header_line, is_entry_id, HistoryEntry, MAX_ENTRIES, MAX_FILE_BYTES,
    PRUNE_TO_PERCENT,
};

/// Which creation times a search keeps: at or after `after`, and before `before`. Either
/// end may be open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimeWindow {
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
}

impl TimeWindow {
    /// Every entry.
    pub fn all() -> Self {
        Self::default()
    }

    pub fn holds(&self, at: DateTime<Utc>) -> bool {
        self.after.is_none_or(|after| at >= after) && self.before.is_none_or(|before| at < before)
    }
}

/// Where the next page starts: just after the entry named here, in the newest first
/// order. It names only an entry the caller was shown, so it tells them nothing new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCursor {
    pub at: DateTime<Utc>,
    pub id: String,
}

impl HistoryCursor {
    /// The cursor as one opaque word, for a route or a tool to hand back.
    pub fn to_token(&self) -> String {
        format!("{}:{}", self.at.timestamp(), self.id)
    }

    /// A word [`Self::to_token`] made; `None` for anything else.
    pub fn from_token(token: &str) -> Option<Self> {
        let (seconds, id) = token.split_once(':')?;
        let at = DateTime::from_timestamp(seconds.parse().ok()?, 0)?;
        is_entry_id(id).then(|| Self {
            at,
            id: id.to_string(),
        })
    }

    fn after(entry: &HistoryEntry) -> Self {
        Self {
            at: entry.created_at,
            id: entry.id.clone(),
        }
    }
}

/// One page of a search, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    /// Set when more matches the caller may see follow this page.
    pub next_cursor: Option<HistoryCursor>,
    /// How many entries the caller may see match, over every page. Never a count of
    /// anything withheld.
    pub total: usize,
}

/// Walk `entries` (oldest first, as the file holds them) newest first, and keep a match
/// only when `visible` admits it, it lies in `window` and its folded text holds `query`
/// (already folded). Each test runs before a match counts toward `limit`, so entries the
/// caller may not see never use up a page.
///
/// A cursor resumes after the entry it names. When that entry has gone (deleted or
/// pruned), the page resumes at the first entry created before it.
pub(crate) fn walk(
    entries: &[HistoryEntry],
    folded: &[String],
    query: Option<&str>,
    window: TimeWindow,
    cursor: Option<&HistoryCursor>,
    limit: usize,
    visible: &dyn Fn(&HistoryEntry) -> bool,
) -> HistoryPage {
    let resume_below = cursor.map(|cursor| {
        entries
            .iter()
            .rposition(|entry| entry.id == cursor.id)
            .map_or(Resume::Before(cursor.at), Resume::At)
    });
    let mut page = Vec::new();
    let mut total = 0;
    let mut more = false;
    for (index, entry) in entries.iter().enumerate().rev() {
        if !window.holds(entry.created_at) || !visible(entry) {
            continue;
        }
        if let Some(query) = query {
            if !folded[index].contains(query) {
                continue;
            }
        }
        total += 1;
        let past_cursor = match resume_below {
            None => true,
            Some(Resume::At(at)) => index < at,
            Some(Resume::Before(at)) => entry.created_at < at,
        };
        if !past_cursor {
            continue;
        }
        if page.len() < limit {
            page.push(entry.clone());
        } else {
            more = true;
        }
    }
    let next_cursor = if more {
        page.last().map(HistoryCursor::after)
    } else {
        None
    };
    HistoryPage {
        entries: page,
        next_cursor,
        total,
    }
}

#[derive(Debug, Clone, Copy)]
enum Resume {
    /// Below this file position.
    At(usize),
    /// Created before this time.
    Before(DateTime<Utc>),
}

/// The file `entries` make, header first.
pub(crate) fn file_bytes(entries: &[HistoryEntry]) -> Vec<u8> {
    let mut bytes = header_line().into_bytes();
    for entry in entries {
        bytes.extend_from_slice(entry_line(entry).as_bytes());
    }
    bytes
}

/// How many of the oldest entries a history of these line lengths drops once it has
/// passed a cap: none while both caps hold, otherwise enough that what is left is at
/// [`PRUNE_TO_PERCENT`] of both. The newest entry is always kept.
pub(crate) fn prune_count(header_len: u64, line_lens: &[u64]) -> usize {
    let total: u64 = header_len + line_lens.iter().sum::<u64>();
    if line_lens.len() <= MAX_ENTRIES && total <= MAX_FILE_BYTES {
        return 0;
    }
    let keep_entries = MAX_ENTRIES * PRUNE_TO_PERCENT as usize / 100;
    let keep_bytes = MAX_FILE_BYTES * PRUNE_TO_PERCENT / 100;
    let mut dropped = 0;
    let mut bytes = total;
    while dropped + 1 < line_lens.len()
        && (line_lens.len() - dropped > keep_entries || bytes > keep_bytes)
    {
        bytes -= line_lens[dropped];
        dropped += 1;
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::schema::{new_entry_id, Surface};
    use crate::store::EntryTier;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_795_707_552 + seconds, 0).unwrap()
    }

    fn entry(seconds: i64, text: &str, tier: EntryTier) -> HistoryEntry {
        HistoryEntry {
            id: new_entry_id(),
            created_at: at(seconds),
            text: text.to_string(),
            tier,
            institutions: None,
            surface: Surface::Clipboard,
            duration_ms: 1_000,
            model: "whisper-small.en-q8_0".to_string(),
            private_terms: false,
            session_id: None,
            session_incarnation: None,
            truncated: None,
        }
    }

    fn folded(entries: &[HistoryEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| crate::store::fold_key(&e.text))
            .collect()
    }

    fn public_only(entry: &HistoryEntry) -> bool {
        entry.tier == EntryTier::Public
    }

    #[test]
    fn a_window_is_closed_below_and_open_above() {
        let window = TimeWindow {
            after: Some(at(10)),
            before: Some(at(20)),
        };
        assert!(!window.holds(at(9)));
        assert!(window.holds(at(10)));
        assert!(window.holds(at(19)));
        assert!(!window.holds(at(20)));
        assert!(TimeWindow::all().holds(at(-1_000_000)));
    }

    #[test]
    fn the_cursor_token_round_trips_and_refuses_anything_else() {
        let cursor = HistoryCursor {
            at: at(5),
            id: new_entry_id(),
        };
        assert_eq!(
            HistoryCursor::from_token(&cursor.to_token()),
            Some(cursor.clone())
        );
        for bad in ["", "12", "x:dch_", "12:dct_1", &format!("y:{}", cursor.id)] {
            assert_eq!(HistoryCursor::from_token(bad), None, "{bad}");
        }
    }

    /// The private entries are the newer ones, and still the public caller's page is full
    /// and its total counts only what it may see.
    #[test]
    fn private_entries_never_use_up_a_public_page() {
        let mut entries: Vec<HistoryEntry> = (0..10)
            .map(|i| entry(i, &format!("public note {i}"), EntryTier::Public))
            .collect();
        entries.extend((10..70).map(|i| entry(i, "PRIVATE SENTINEL", EntryTier::Private)));
        let keys = folded(&entries);
        let page = walk(
            &entries,
            &keys,
            None,
            TimeWindow::all(),
            None,
            10,
            &public_only,
        );
        assert_eq!(page.entries.len(), 10);
        assert!(page.entries.iter().all(|e| e.tier == EntryTier::Public));
        assert_eq!(page.total, 10);
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.entries[0].text, "public note 9", "newest first");
    }

    #[test]
    fn pages_follow_the_cursor_and_skip_what_is_hidden() {
        let entries: Vec<HistoryEntry> = (0..25)
            .map(|i| {
                let tier = if i % 2 == 0 {
                    EntryTier::Public
                } else {
                    EntryTier::Private
                };
                entry(i, &format!("note {i}"), tier)
            })
            .collect();
        let keys = folded(&entries);
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = walk(
                &entries,
                &keys,
                None,
                TimeWindow::all(),
                cursor.as_ref(),
                4,
                &public_only,
            );
            assert_eq!(page.total, 13);
            seen.extend(page.entries.iter().map(|e| e.text.clone()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let want: Vec<String> = (0..25)
            .rev()
            .filter(|i| i % 2 == 0)
            .map(|i| format!("note {i}"))
            .collect();
        assert_eq!(seen, want);
        // A cursor whose entry has gone resumes at the first entry created before it.
        let gone = HistoryCursor {
            at: at(12),
            id: new_entry_id(),
        };
        let page = walk(
            &entries,
            &keys,
            None,
            TimeWindow::all(),
            Some(&gone),
            2,
            &public_only,
        );
        assert_eq!(
            page.entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            ["note 10", "note 8"]
        );
    }

    #[test]
    fn a_query_matches_the_fold_key_and_the_window_narrows() {
        let entries = vec![
            entry(0, "HIF-2alpha binds EPAS 1", EntryTier::Public),
            entry(1, "nothing here", EntryTier::Public),
            entry(2, "hif2α again", EntryTier::Public),
        ];
        let keys = folded(&entries);
        let query = crate::store::fold_key("hif 2 alpha");
        let page = walk(
            &entries,
            &keys,
            Some(&query),
            TimeWindow::all(),
            None,
            10,
            &|_| true,
        );
        assert_eq!(page.total, 2);
        let window = TimeWindow {
            after: Some(at(1)),
            before: None,
        };
        let page = walk(&entries, &keys, Some(&query), window, None, 10, &|_| true);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].text, "hif2α again");
    }

    #[test]
    fn a_full_history_drops_to_ninety_percent_of_both_caps() {
        let small = vec![100u64; MAX_ENTRIES];
        assert_eq!(prune_count(50, &small), 0, "at the cap nothing goes");
        let over = vec![100u64; MAX_ENTRIES + 1];
        assert_eq!(prune_count(50, &over), MAX_ENTRIES + 1 - 4_500);
        let big = vec![40_000u64; 220];
        let dropped = prune_count(50, &big);
        let kept: u64 = 50 + big[dropped..].iter().sum::<u64>();
        assert!(kept <= MAX_FILE_BYTES * 9 / 10);
        assert!(
            kept + 40_000 > MAX_FILE_BYTES * 9 / 10,
            "no more than needed"
        );
        assert_eq!(prune_count(50, &[MAX_FILE_BYTES]), 0, "the newest stays");
    }
}
