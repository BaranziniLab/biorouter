// `src/routes/` is compiled TWICE — once here and once by `main.rs`, which
// re-declares the module tree (see `routes::secret_matches` for why that
// constrains what routes may name). `auth` is lib-only, so its process globals
// exist exactly once; a route that needs one therefore has to name the lib by
// crate name, which resolves in the binary compilation and — with this line —
// in the lib's own. Both spellings then reach the same static, which is the
// whole point: `commands::agent` installs the user-action digest through
// `biorouter_server::auth`, and `routes::agent` reads it through the same path.
extern crate self as biorouter_server;

pub mod auth;
pub mod configuration;
pub mod error;
// SD-12: what this daemon was LAUNCHED with. Lib-only for the same reason
// `auth` is — it holds a process global, and `routes::agent` is compiled twice.
pub mod launch;
pub mod openapi;
pub mod routes;
pub mod state;
pub mod tunnel;
pub mod turn_stream;
pub mod workspace;

/// Redirects the data/config/state dirs at a throwaway root before `main`, so
/// tests never open the developer's real `sessions.db`.
#[cfg(test)]
mod test_sandbox;

// Re-export commonly used items
pub use openapi::*;
pub use state::*;
