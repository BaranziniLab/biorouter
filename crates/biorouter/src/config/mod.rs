pub mod base;
pub mod biorouter_mode;
pub mod declarative_providers;
mod experiments;
pub mod extensions;
pub mod paths;
pub mod permission;
pub mod search_path;
pub mod signup_openrouter;
pub mod signup_tetrate;
pub mod usage;

pub use crate::agents::ExtensionConfig;
pub use base::{with_config_overrides, Config, ConfigError, ConfigWriteFailure};
pub use biorouter_mode::BioRouterMode;
pub use declarative_providers::DeclarativeProviderConfig;
pub use experiments::ExperimentManager;
pub use extensions::{
    extension_entry_is_persisted, get_all_extension_names, get_all_extensions,
    get_enabled_extensions, get_extension_by_name, get_extension_entry_by_name, get_warnings,
    is_extension_enabled, persisted_extension_names, remove_extension,
    resolve_extensions_for_new_session, set_extension, set_extension_enabled, ExtensionEntry,
};
pub use permission::PermissionManager;
pub use signup_openrouter::configure_openrouter;
pub use signup_tetrate::configure_tetrate;
pub use usage::{percent_of, UsageLimits};

pub use extensions::DEFAULT_DISPLAY_NAME;
pub use extensions::DEFAULT_EXTENSION;
pub use extensions::DEFAULT_EXTENSION_DESCRIPTION;
pub use extensions::DEFAULT_EXTENSION_TIMEOUT;
