#![recursion_limit = "256"]

pub mod server;

// Every `keyring::Entry` the lib test binary builds uses process memory, never the
// OS credential store (`biorouter::config::test_binary_credentials`).
#[cfg(test)]
biorouter::keep_this_test_binary_off_the_os_credential_store!();
