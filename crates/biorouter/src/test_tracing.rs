//! Keep this test binary's scoped tracing captures from missing events that
//! another test thread's callsite registration would otherwise hide.
//!
//! The mechanism, and why this has to run before `main`, is in
//! [`biorouter_mcp::test_tracing`]. The captures it protects here are the ones
//! that install a thread-local subscriber and assert on what it received:
//! `providers::utils`'s `private_error_logging_*` traces,
//! `agents::phase_timing`'s drop emission, `privacy::master_switch`'s load
//! warning, `slash_commands`' and `workflow::local_workflows`' warnings.

/// Whether the ctor registered the inert dispatchers, read as the ctor
/// returned — before any test could run.
static REGISTERED_BEFORE_MAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

#[ctor::ctor]
fn register_inert_tracing_dispatchers_before_main() {
    biorouter_mcp::test_tracing::register_inert_dispatchers();
    let _ = REGISTERED_BEFORE_MAIN.set(biorouter_mcp::test_tracing::inert_dispatchers_registered());
}

#[cfg(test)]
mod tests {
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A callsite nothing else in the binary reaches, so the thread below is
    /// the first to register it — the event every capture here depends on.
    fn emit_the_probe_warning() {
        tracing::warn!("test_tracing probe 7c1e");
    }

    /// The ctor's work is what every capture in this binary depends on, and
    /// its absence is invisible to them until a sibling wins a race, so it is
    /// asserted directly.
    #[test]
    fn the_inert_dispatchers_are_registered_before_main() {
        assert_eq!(
            super::REGISTERED_BEFORE_MAIN.get(),
            Some(&true),
            "the ctor did not register the inert tracing dispatchers before main. Without \
             them, a sibling test thread that is first to reach a callsite while one \
             capture's subscriber is the only one registered caches Interest::never for \
             it, and that capture then misses its own event (see \
             biorouter_mcp::test_tracing)."
        );
    }

    /// The race itself, forced: another thread, with no subscriber of its own,
    /// is the first to reach the callsite while this test's thread-local
    /// capture is live. That thread's event is not this capture's to see; the
    /// one emitted on this thread afterwards must be.
    ///
    /// Without the ctor's dispatchers, run alone, this fails every time — it
    /// is `providers::utils`' "the control warning must be captured" failure
    /// with the race decided in advance.
    #[test]
    fn a_capture_sees_a_callsite_another_thread_reached_first() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            std::thread::spawn(emit_the_probe_warning)
                .join()
                .expect("the probe thread exits cleanly");
            emit_the_probe_warning();
        });

        let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            text.matches("test_tracing probe 7c1e").count(),
            1,
            "the capture must see exactly its own thread's event, got: {text:?}. None \
             means the callsite's interest was cached as never by the thread that \
             reached it first."
        );
    }
}
