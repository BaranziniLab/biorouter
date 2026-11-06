//! Local dictation in the host process: the one [`service::DictationService`] per
//! `biorouterd` or interactive `biorouter`, which owns the supervisor of the engine's
//! worker (`docs/dictation/engine-and-models.md`, "Supervisor and service API").
//!
//! The engine itself never runs here (DD-4): it runs in a worker process that the
//! service starts from this binary with a hidden first argument. A build without the
//! `local-dictation` feature still has this module; its status answers `no_engine` and
//! every request answers [`MSG_NO_ENGINE`].

pub mod service;

pub use biorouter_dictation::audio::{PcmSlice, Span};
pub use biorouter_dictation::protocol::{Hello, Loaded, TranscribeResult};
pub use biorouter_dictation::supervisor::LoadSpec;
pub use service::{
    service, DictationError, DictationService, EngineBuild, EngineState, EngineStatus,
    TranscribeRequest, WorkerProbe,
};

/// MSG-NO-ENGINE.
pub const MSG_NO_ENGINE: &str = "This build of Biorouter has no local dictation engine.";
