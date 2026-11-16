pub mod bench_config;
pub mod bench_session;
pub mod bench_work_dir;
pub mod error_capture;
pub mod eval_suites;
pub mod reporting;
pub mod runners;
pub mod utilities;

// Every `keyring::Entry` the lib test binary builds uses process memory, never the
// OS credential store (`biorouter::config::test_binary_credentials`).
#[cfg(test)]
biorouter::keep_this_test_binary_off_the_os_credential_store!();
