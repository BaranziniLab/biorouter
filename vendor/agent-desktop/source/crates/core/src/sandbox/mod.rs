//! Local sandbox domain: names, records, provider scripts, their files, and the reply protocol.

pub(crate) mod exec_passthrough;
pub(crate) mod provider_dirs;
pub(crate) mod provider_files;
pub(crate) mod provider_protocol;
pub(crate) mod provider_reply;
pub(crate) mod provider_run;
pub(crate) mod provider_script_output;
pub(crate) mod provider_script_request;
pub(crate) mod provider_settings;
pub(crate) mod provider_spec;
pub(crate) mod provider_table;
#[cfg(test)]
pub(crate) mod provider_test_support;
pub(crate) mod sandbox_entry;
pub(crate) mod sandbox_name;
pub(crate) mod sandbox_record;
pub(crate) mod sandbox_state;
pub(crate) mod sandbox_store;
