//! Register `biorouter_mcp::test_tracing`'s inert dispatchers before any test
//! in this binary runs, so a thread-local capture cannot miss its own events
//! (the mechanism is in that module). The capture it protects here is
//! `workspace::turn`'s `provider_abort_keeps_private_details_off_diagnostics_but_on_bus`.

static REGISTERED_BEFORE_MAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

#[ctor::ctor]
fn register_inert_tracing_dispatchers_before_main() {
    biorouter_mcp::test_tracing::register_inert_dispatchers();
    let _ = REGISTERED_BEFORE_MAIN.set(biorouter_mcp::test_tracing::inert_dispatchers_registered());
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_inert_dispatchers_are_registered_before_main() {
        assert_eq!(
            super::REGISTERED_BEFORE_MAIN.get(),
            Some(&true),
            "the ctor did not register the inert tracing dispatchers before main, so a \
             capture in this binary can miss its own event whenever a sibling test \
             thread reaches that callsite first (biorouter_mcp::test_tracing)."
        );
    }
}
