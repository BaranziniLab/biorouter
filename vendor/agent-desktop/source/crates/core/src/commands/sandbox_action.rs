#[derive(Debug, Clone)]
pub enum SandboxAction {
    Install {
        provider: Option<String>,
    },
    Create {
        name: String,
        provider: String,
        set: Vec<String>,
    },
    Start {
        name: String,
        set: Vec<String>,
    },
    Stop {
        name: String,
    },
    Status {
        name: String,
    },
    List,
    Rm {
        name: String,
    },
    Exec {
        name: String,
        args: Vec<String>,
    },
}
