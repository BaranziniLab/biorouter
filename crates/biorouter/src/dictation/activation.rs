//! The person-owned choice that enables dictionary suggestions in new chats.

use crate::config::extensions::is_extension_enabled;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static ACP_SUGGEST_ONLY: AtomicBool = AtomicBool::new(false);
static PROCESS_SURFACE: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Unspecified,
    Cli,
    Acp,
}

pub fn set_process_surface(surface: Surface) {
    let value = match surface {
        Surface::Unspecified => 0,
        Surface::Cli => 1,
        Surface::Acp => 2,
    };
    let _ = PROCESS_SURFACE.compare_exchange(0, value, Ordering::Relaxed, Ordering::Relaxed);
    if surface == Surface::Acp {
        force_suggest_only_for_process();
    }
}

pub fn process_surface() -> Surface {
    match PROCESS_SURFACE.load(Ordering::Relaxed) {
        1 => Surface::Cli,
        2 => Surface::Acp,
        _ => Surface::Unspecified,
    }
}

pub fn force_suggest_only_for_process() {
    ACP_SUGGEST_ONLY.store(true, Ordering::Relaxed);
}

pub fn suggest_only_for_process() -> bool {
    ACP_SUGGEST_ONLY.load(Ordering::Relaxed)
}

pub const EXTENSION_KEY: &str = "dictionary";
pub const ACTIVATION_VERSION_KEY: &str = "dictation_dictionary_activation_version";
pub const ENABLE_REFUSAL: &str = "The Dictionary capability is turned on by you, with Suggest dictionary words from my chats on the Dictionary page.";

pub fn is_dictionary_name(name: &str) -> bool {
    crate::config::extensions::name_to_key(name) == EXTENSION_KEY
}

pub fn suggest_from_chats() -> bool {
    is_extension_enabled(EXTENSION_KEY)
}

pub fn activate(suggest: bool) -> Result<()> {
    crate::config::extensions::set_dictionary_activation(suggest, ACTIVATION_VERSION_KEY, false)
        .map_err(Into::into)
}

pub fn activate_first_install(suggest: bool) -> Result<()> {
    crate::config::extensions::set_dictionary_activation(suggest, ACTIVATION_VERSION_KEY, true)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The activation marker as `config.yaml` stores it, or `None` before any activation.
    fn stored_marker() -> Option<u8> {
        crate::config::Config::global()
            .get_param::<u8>(ACTIVATION_VERSION_KEY)
            .ok()
    }

    #[test]
    fn dictionary_names_are_matched_the_way_extension_keys_are() {
        for name in ["dictionary", "Dictionary", " DICTIONARY "] {
            assert!(is_dictionary_name(name), "{name:?}");
        }
        for name in ["dictionary-notes", "dictionaries", "memory", ""] {
            assert!(!is_dictionary_name(name), "{name:?}");
        }
    }

    /// The capability is off in a fresh configuration; a first install with the box
    /// ticked turns it on and writes the marker, and the box applies once: a later first
    /// install with the box unticked, or one after the person turned it off, changes
    /// nothing. The person's own switch turns it off and on.
    ///
    /// In a process of its own, because `Config::global()` is that process's own fresh
    /// configuration, which nothing else in the run reads.
    #[test]
    fn a_ticked_first_install_turns_the_capability_on_once() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        assert!(!suggest_from_chats(), "a fresh configuration has it off");
        assert_eq!(stored_marker(), None);

        activate_first_install(true).unwrap();
        assert!(suggest_from_chats());
        assert_eq!(stored_marker(), Some(1));

        activate_first_install(false).unwrap();
        assert!(
            suggest_from_chats(),
            "a second first install turned the capability off"
        );

        activate(false).unwrap();
        assert!(!suggest_from_chats());
        activate_first_install(true).unwrap();
        assert!(
            !suggest_from_chats(),
            "a later first install overrode the person's switch"
        );

        activate(true).unwrap();
        assert!(suggest_from_chats());
        assert_eq!(stored_marker(), Some(1));
    }

    /// An unticked box on the first install leaves the capability off and still writes the
    /// marker, so a later ticked install does not turn it on; only the person's switch does.
    #[test]
    fn an_unticked_first_install_leaves_the_capability_off_for_good() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        assert!(!suggest_from_chats());
        activate_first_install(false).unwrap();
        assert!(!suggest_from_chats());
        assert_eq!(stored_marker(), Some(1));

        activate_first_install(true).unwrap();
        assert!(
            !suggest_from_chats(),
            "the install box turned the capability on a second time"
        );

        activate(true).unwrap();
        assert!(suggest_from_chats());
    }

    /// ACP stdio forces suggestions for the whole process, and the first surface a process
    /// names is the one it keeps.
    #[test]
    fn an_acp_process_suggests_only_and_keeps_its_first_surface() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        assert_eq!(process_surface(), Surface::Unspecified);
        assert!(!suggest_only_for_process());

        set_process_surface(Surface::Acp);
        assert_eq!(process_surface(), Surface::Acp);
        assert!(suggest_only_for_process());

        set_process_surface(Surface::Cli);
        assert_eq!(process_surface(), Surface::Acp);
        assert!(suggest_only_for_process());
    }

    #[test]
    fn a_cli_process_is_not_forced_to_suggest() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        set_process_surface(Surface::Cli);
        assert_eq!(process_surface(), Surface::Cli);
        assert!(!suggest_only_for_process());
    }
}
