//! How well chat suggestions suit one person, measured by that person (owner, 2026-11-13).
//!
//! Word suggestions from chats replaced a one time precision gate (M-21, which needed the
//! owner's labels on 200 of the owner's chats) with each person's own feedback. Every
//! **Keep** or **Dismiss** of a suggestion the chat extractor made is recorded here, on this
//! computer, in the Dictionary store and nowhere else. Once [`MIN_DECISIONS`] of the latest
//! [`RECENT_DECISIONS`] are in, a person who dismisses most suggestions is offered fewer: a
//! word must be found in more chats before it is listed ([`ChatLearning::required_chats`]),
//! and the Dictionary page offers, in one sentence, to turn suggestions off.
//!
//! Only the latest decisions count, so a person whose suggestions get better (because
//! fewer, stricter ones are offered) gets the usual threshold back.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How many of the person's decisions are needed before their precision changes anything.
pub const MIN_DECISIONS: usize = 20;
/// How many of the person's latest decisions are kept and counted.
pub const RECENT_DECISIONS: usize = 50;
/// The chats a word needs before it is suggested, while most suggestions are kept.
pub const BASE_REQUIRED_CHATS: u32 = 2;
/// The most chats a word ever needs, when nearly every suggestion is dismissed.
pub const MAX_REQUIRED_CHATS: u32 = 4;

/// The person's decisions on chat suggestions. Every field is left out of the store while
/// it holds its default, so a store whose owner never reviewed a chat suggestion is what
/// an older build wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatLearning {
    /// The latest decisions, oldest first: `true` for **Keep**, `false` for **Dismiss**.
    /// At most [`RECENT_DECISIONS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<bool>,
    /// Every suggestion the person kept, ever.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub kept: u32,
    /// Every suggestion the person dismissed, ever.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dismissed: u32,
    /// The person's total decisions ([`ChatLearning::decisions`]) when they last chose to
    /// keep suggestions on: turning them on, or answering the offer to turn them off with
    /// **Keep suggesting**. The offer comes back only after [`MIN_DECISIONS`] more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chose_on_at: Option<u32>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl ChatLearning {
    /// Record one decision on a chat suggestion.
    pub fn record(&mut self, kept: bool) {
        if kept {
            self.kept = self.kept.saturating_add(1);
        } else {
            self.dismissed = self.dismissed.saturating_add(1);
        }
        self.recent.push(kept);
        if self.recent.len() > RECENT_DECISIONS {
            let extra = self.recent.len() - RECENT_DECISIONS;
            self.recent.drain(..extra);
        }
    }

    /// Every decision the person made, ever.
    pub fn decisions(&self) -> u32 {
        self.kept.saturating_add(self.dismissed)
    }

    /// The latest decisions that were **Keep**.
    pub fn recent_kept(&self) -> usize {
        self.recent.iter().filter(|kept| **kept).count()
    }

    /// How many chats a word must be found in before it is suggested: two, until at least
    /// [`MIN_DECISIONS`] recent decisions show that most suggestions were dismissed; then
    /// three while at least a quarter are kept, and four below that.
    pub fn required_chats(&self) -> u32 {
        let total = self.recent.len();
        if total < MIN_DECISIONS {
            return BASE_REQUIRED_CHATS;
        }
        let kept = self.recent_kept();
        if kept * 2 >= total {
            BASE_REQUIRED_CHATS
        } else if kept * 4 >= total {
            BASE_REQUIRED_CHATS + 1
        } else {
            MAX_REQUIRED_CHATS
        }
    }

    /// Whether suggestions are being held back because most were dismissed.
    pub fn backing_off(&self) -> bool {
        self.required_chats() > BASE_REQUIRED_CHATS
    }

    /// Whether the person should be offered, once, to turn suggestions off: they are being
    /// held back, and the person has not chosen to keep them on within the last
    /// [`MIN_DECISIONS`] decisions.
    pub fn offers_turning_off(&self) -> bool {
        self.backing_off()
            && self
                .chose_on_at
                .is_none_or(|at| self.decisions().saturating_sub(at) >= MIN_DECISIONS as u32)
    }

    /// The person chose to keep suggestions on (turned them on, or answered the offer to
    /// turn them off with **Keep suggesting**).
    pub fn choose_on(&mut self) {
        self.chose_on_at = Some(self.decisions());
    }
}

/// The digest a suggestion keeps for a chat it was found in: the first 16 hexadecimal
/// digits of the SHA-256 of the chat's id, so the store counts chats without listing them.
pub fn chat_digest(session_id: &str) -> String {
    let hash = Sha256::digest(session_id.as_bytes());
    hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(kept: usize, dismissed: usize) -> ChatLearning {
        let mut learning = ChatLearning::default();
        for _ in 0..kept {
            learning.record(true);
        }
        for _ in 0..dismissed {
            learning.record(false);
        }
        learning
    }

    #[test]
    fn fewer_than_the_minimum_decisions_change_nothing() {
        let learning = with(0, MIN_DECISIONS - 1);
        assert_eq!(learning.required_chats(), BASE_REQUIRED_CHATS);
        assert!(!learning.backing_off());
        assert!(!learning.offers_turning_off());
    }

    #[test]
    fn most_dismissed_raises_the_chats_a_word_needs() {
        assert_eq!(
            with(10, 10).required_chats(),
            2,
            "half kept is not most dismissed"
        );
        assert_eq!(with(9, 11).required_chats(), 3);
        assert_eq!(with(5, 15).required_chats(), 3, "a quarter kept");
        assert_eq!(with(4, 16).required_chats(), 4);
        assert_eq!(with(0, 50).required_chats(), MAX_REQUIRED_CHATS);
        assert!(with(9, 11).offers_turning_off());
    }

    #[test]
    fn only_the_latest_decisions_count_so_the_threshold_comes_back() {
        let mut learning = with(0, RECENT_DECISIONS);
        assert_eq!(learning.required_chats(), MAX_REQUIRED_CHATS);
        for _ in 0..RECENT_DECISIONS / 2 {
            learning.record(true);
        }
        assert_eq!(learning.recent.len(), RECENT_DECISIONS);
        assert_eq!(learning.required_chats(), BASE_REQUIRED_CHATS);
        assert_eq!(learning.kept, (RECENT_DECISIONS / 2) as u32);
        assert_eq!(learning.dismissed, RECENT_DECISIONS as u32);
    }

    #[test]
    fn choosing_to_keep_suggesting_quiets_the_offer_for_a_while() {
        let mut learning = with(2, 20);
        assert!(learning.offers_turning_off());
        learning.choose_on();
        assert!(!learning.offers_turning_off());
        assert!(learning.backing_off(), "the threshold still holds");
        for _ in 0..MIN_DECISIONS - 1 {
            learning.record(false);
        }
        assert!(!learning.offers_turning_off());
        learning.record(false);
        assert!(
            learning.offers_turning_off(),
            "it asks again after more decisions"
        );
    }

    #[test]
    fn a_default_record_serializes_to_an_empty_object_and_unknown_keys_are_refused() {
        assert_eq!(
            serde_json::to_string(&ChatLearning::default()).unwrap(),
            "{}"
        );
        let learning = with(1, 1);
        let text = serde_json::to_string(&learning).unwrap();
        assert_eq!(text, r#"{"recent":[true,false],"kept":1,"dismissed":1}"#);
        assert_eq!(
            serde_json::from_str::<ChatLearning>(&text).unwrap(),
            learning
        );
        assert!(serde_json::from_str::<ChatLearning>(r#"{"precision":1}"#).is_err());
    }

    #[test]
    fn a_chat_digest_is_short_stable_and_names_no_chat() {
        let digest = chat_digest("20261113_42");
        assert_eq!(digest.len(), 16);
        assert_eq!(digest, chat_digest("20261113_42"));
        assert_ne!(digest, chat_digest("20261113_43"));
        assert!(!digest.contains("20261113"));
    }
}
