use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxState {
    Unavailable,
    Missing,
    Ready,
    Working,
    NeedsSetup,
    Running,
    Stopped,
    Broken,
}

impl SandboxState {
    pub(crate) fn from_wire(value: &str) -> Option<Self> {
        match value {
            "unavailable" => Some(Self::Unavailable),
            "missing" => Some(Self::Missing),
            "ready" => Some(Self::Ready),
            "working" => Some(Self::Working),
            "needs_setup" => Some(Self::NeedsSetup),
            "running" => Some(Self::Running),
            "stopped" => Some(Self::Stopped),
            "broken" => Some(Self::Broken),
            _ => None,
        }
    }
}
